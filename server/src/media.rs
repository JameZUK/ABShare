//! Media (SPEC §5.2, §5.4, D7, D23): originals with Range, playable copies for books that need
//! one, data-saver streams made on demand in segments, and covers. All ffmpeg work is jailed.

use crate::config::Ffmpeg;
use crate::tools::{Limits, demuxer_for, run_jailed};
use abshare_core::saver::{Bitrate, Layout, Profile, SAMPLE_RATE, SegmentWindow, samples_from_ms};
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Most segments one data-saver response may make the server encode (D23).
pub const MAX_SEGMENTS_PER_RESPONSE: u64 = 10;

pub struct Media {
    pub ff: Ffmpeg,
    pub data_dir: PathBuf,
    /// Library name → root, from the config.
    pub roots: HashMap<String, PathBuf>,
    fg: Semaphore,
    prefetch: Semaphore,
    copies: Semaphore,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    copying: Mutex<HashSet<String>>,
    /// Latest seek per device: a queued encode superseded by a newer request is dropped.
    seeks: Mutex<HashMap<i64, u64>>,
    seek_counter: AtomicU64,
    silent: Mutex<HashMap<u64, Vec<u8>>>,
    /// Cache key (`saver/<book>` or `copies/<book>.m4b`) → when it was last served.
    used: Mutex<HashMap<String, i64>>,
    /// Read-ahead tasks running now (bounded, so download loops can't queue unbounded work).
    read_aheads: Arc<AtomicU64>,
    /// Whether the jail starts here (checked once, on first ask).
    sandbox: tokio::sync::OnceCell<bool>,
}

/// What the media layer needs to know about a book's audio.
#[derive(Clone, Debug)]
pub struct Source {
    pub book_id: String,
    /// Absolute paths of the parts in play order (already checked to be inside the root).
    pub parts: Vec<PathBuf>,
    pub duration_ms: u64,
    pub needs_copy: bool,
}

pub fn bitrate_for(kbps: u64) -> Option<Bitrate> {
    match kbps {
        32 => Some(Bitrate::K32),
        40 => Some(Bitrate::K40),
        48 => Some(Bitrate::K48),
        _ => None,
    }
}

fn s(x: impl ToString) -> String {
    x.to_string()
}

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Exact decimal seconds for a sample count on the alignment grid.
fn secs(samples: u64) -> String {
    let rate = u64::from(SAMPLE_RATE);
    format!("{}.{:06}", samples / rate, (samples % rate) * 1_000_000 / rate)
}

/// Opens a library file for reading without following symlinks, refusing anything that
/// isn't a regular file inside `root`.
pub fn open_inside(root: &Path, path: &Path) -> Result<std::fs::File, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let canon = path.canonicalize().map_err(|e| e.to_string())?;
    if !canon.starts_with(&root) {
        return Err("path outside the library".into());
    }
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&canon)
        .map_err(|e| e.to_string())?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("not a regular file".into());
    }
    let _ = m.ino();
    Ok(f)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Bytes used by the transcode cache (data-saver segments and playable copies).
pub fn cache_usage(data_dir: &Path) -> u64 {
    let mut total = 0;
    for sub in ["saver", "copies"] {
        let Ok(rd) = std::fs::read_dir(data_dir.join(sub)) else { continue };
        for ent in rd.flatten() {
            let Ok(m) = ent.metadata() else { continue };
            total += if m.is_dir() { dir_usage(&ent.path()).0 } else { m.len() };
        }
    }
    total
}

/// Total size and newest modification time of the files under `dir` (one level of
/// subdirectories: `saver/<book>/k<kbps>/<segment>.mp3`).
fn dir_usage(dir: &Path) -> (u64, i64) {
    let (mut bytes, mut newest) = (0, 0);
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for ent in rd.flatten() {
            let Ok(m) = ent.metadata() else { continue };
            if m.is_dir() && depth < 2 {
                stack.push((ent.path(), depth + 1));
            } else if m.is_file() {
                bytes += m.len();
                newest = newest.max(m.mtime());
            }
        }
    }
    (bytes, newest)
}

impl Media {
    pub fn new(ff: Ffmpeg, data_dir: PathBuf, roots: HashMap<String, PathBuf>) -> Self {
        let lanes = ff.jobs.clamp(1, 8);
        Self {
            ff,
            data_dir,
            roots,
            fg: Semaphore::new(1),
            // Background work (read-ahead, downloads) may use `jobs` cores; playback has its own lane.
            prefetch: Semaphore::new(lanes),
            copies: Semaphore::new(1),
            locks: Mutex::default(),
            copying: Mutex::default(),
            seeks: Mutex::default(),
            seek_counter: AtomicU64::new(0),
            silent: Mutex::default(),
            used: Mutex::default(),
            sandbox: tokio::sync::OnceCell::new(),
            read_aheads: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn copy_path(&self, book: &str) -> PathBuf {
        self.data_dir.join("copies").join(format!("{book}.m4b"))
    }

    /// Whether a book can be played now (originals always; copies once built).
    pub fn ready(&self, src: &Source) -> bool {
        !src.needs_copy || self.copy_path(&src.book_id).exists()
    }

    /// The file to play or to transcode from: the original single part, or the copy.
    pub fn play_file(&self, src: &Source) -> Option<PathBuf> {
        if src.needs_copy {
            let p = self.copy_path(&src.book_id);
            p.exists().then_some(p)
        } else {
            src.parts.first().cloned()
        }
    }

    /// Starts building a playable copy in the background (once per book).
    pub fn ensure_copy(self: &Arc<Self>, src: Source) {
        if !src.needs_copy || self.copy_path(&src.book_id).exists() {
            return;
        }
        {
            let Ok(mut c) = self.copying.lock() else { return };
            if !c.insert(src.book_id.clone()) {
                return;
            }
        }
        let me = self.clone();
        tokio::spawn(async move {
            let id = src.book_id.clone();
            if let Err(e) = me.build_copy(&src).await {
                eprintln!("abshare: playable copy for {id} failed: {e}");
            }
            if let Ok(mut c) = me.copying.lock() {
                c.remove(&id);
            }
        });
    }

    async fn build_copy(&self, src: &Source) -> Result<(), String> {
        let _permit = self.copies.acquire().await.map_err(|e| e.to_string())?;
        let job = self.data_dir.join("jobs").join(format!("copy-{}", src.book_id));
        let _ = tokio::fs::remove_dir_all(&job).await;
        tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
        tokio::fs::create_dir_all(self.data_dir.join("copies")).await.map_err(|e| e.to_string())?;
        let out = job.join("copy.m4b");
        let mut args =
            ["-nostdin", "-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file"].map(s).to_vec();
        for p in &src.parts {
            let d = demuxer_for(&ext(p)).ok_or("unsupported part")?;
            if d == "mov" {
                args.extend(["-enable_drefs", "0"].map(s));
            }
            args.extend([s("-f"), s(d), s("-i"), p.to_string_lossy().into_owned()]);
        }
        // The concat filter over explicit inputs: never the concat demuxer or protocol.
        let n = src.parts.len();
        let inputs: String = (0..n).map(|i| format!("[{i}:a:0]")).collect();
        args.extend([
            s("-filter_complex"),
            format!("{inputs}concat=n={n}:v=0:a=1,aresample=44100[a]"),
            s("-map"),
            s("[a]"),
            s("-map_metadata"),
            s("-1"),
            s("-c:a"),
            s("aac"),
            s("-b:a"),
            s("96k"),
            s("-movflags"),
            s("+faststart"),
            s("-f"),
            s("mp4"),
            s("-y"),
            out.to_string_lossy().into_owned(),
        ]);
        let hours = src.duration_ms / 3_600_000 + 1;
        let limits = Limits {
            mem_mib: 2048,
            cpu_secs: 600 + hours * 120,
            fsize_mib: 64 + hours * 64,
            wall: Duration::from_secs(900 + hours * 180),
            stdout_cap: 1024,
        };
        let reads: Vec<&Path> = src.parts.iter().map(PathBuf::as_path).collect();
        let r = run_jailed(&self.ff, &reads, Some(&job), &self.ff.ffmpeg, &args, &limits).await?;
        if !r.success || !r.stderr.trim().is_empty() {
            let _ = tokio::fs::remove_dir_all(&job).await;
            return Err(format!("ffmpeg: {}", r.stderr.lines().next().unwrap_or("failed")));
        }
        tokio::fs::rename(&out, self.copy_path(&src.book_id)).await.map_err(|e| e.to_string())?;
        let _ = tokio::fs::remove_dir_all(&job).await;
        Ok(())
    }

    /// Whether ffmpeg can run in its sandbox here: a jailed `ffmpeg -version`, run once.
    pub async fn sandbox_works(&self) -> bool {
        *self
            .sandbox
            .get_or_init(|| async {
                let limits = Limits {
                    mem_mib: 2048,
                    cpu_secs: 10,
                    fsize_mib: 1,
                    wall: Duration::from_secs(20),
                    // `-version` prints about 2 KB: the cap must allow it, or a working
                    // sandbox reads as broken.
                    stdout_cap: 16 * 1024,
                };
                let args = [s("-hide_banner"), s("-version")];
                run_jailed(&self.ff, &[], None, &self.ff.ffmpeg, &args, &limits).await.is_ok_and(|o| o.success)
            })
            .await
    }

    // ------------------------------------------------------------ cache limit

    /// Notes that a book's cached data-saver segments or playable copy were just served.
    pub fn mark_used(&self, book: &str, copy: bool) {
        let key = if copy { format!("copies/{book}.m4b") } else { format!("saver/{book}") };
        if let Ok(mut u) = self.used.lock() {
            if u.len() > 50_000 {
                u.clear();
            }
            u.insert(key, unix_now());
        }
    }

    /// Deletes the least recently used data-saver segments and playable copies until the
    /// cache fits in `limit` bytes (core::cache::evict). Anything served in the last 30
    /// minutes stays. Returns the bytes freed.
    pub async fn sweep(&self, limit: u64) -> Result<u64, String> {
        let now = unix_now();
        let used = self.used.lock().map(|u| u.clone()).unwrap_or_default();
        let mut entries = Vec::new();
        let saver = self.data_dir.join("saver");
        let copies = self.data_dir.join("copies");
        let (saver2, copies2) = (saver.clone(), copies.clone());
        let found = tokio::task::spawn_blocking(move || -> Vec<(String, u64, i64)> {
            let mut out = Vec::new();
            for (dir, is_copy) in [(saver2, false), (copies2, true)] {
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                for ent in rd.flatten() {
                    let name = ent.file_name().to_string_lossy().into_owned();
                    let book = name.trim_end_matches(".m4b");
                    if !abshare_core::ids::is_valid_id(book) {
                        continue;
                    }
                    let (bytes, mtime) = if is_copy {
                        ent.metadata().map_or((0, 0), |m| (m.len(), m.mtime()))
                    } else {
                        dir_usage(&ent.path())
                    };
                    let key = if is_copy { format!("copies/{name}") } else { format!("saver/{name}") };
                    out.push((key, bytes, mtime));
                }
            }
            out
        })
        .await
        .map_err(|e| e.to_string())?;
        for (key, bytes, mtime) in found {
            let last_used = used.get(&key).copied().unwrap_or(0).max(mtime);
            entries.push(abshare_core::cache::Entry { pinned: now - last_used < 30 * 60, key, bytes, last_used });
        }
        let mut freed = 0;
        for key in abshare_core::cache::evict(&entries, limit) {
            let bytes = entries.iter().find(|e| e.key == key).map_or(0, |e| e.bytes);
            let path = self.data_dir.join(&key);
            let r = if key.starts_with("copies/") {
                tokio::fs::remove_file(&path).await
            } else {
                tokio::fs::remove_dir_all(&path).await
            };
            if r.is_ok() {
                freed += bytes;
                if let Ok(mut u) = self.used.lock() {
                    u.remove(&key);
                }
            }
        }
        let _ = (saver, copies);
        Ok(freed)
    }

    // ------------------------------------------------------------ data saver

    pub fn layout(&self, duration_ms: u64, bitrate: Bitrate) -> Option<Layout> {
        Profile::new(bitrate, 2_500).ok()?.layout(samples_from_ms(duration_ms)).ok()
    }

    /// Registers a ranged request from `device`; returns its generation.
    pub fn new_seek(&self, device: i64) -> u64 {
        let g = self.seek_counter.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut m) = self.seeks.lock() {
            if m.len() > 10_000 {
                m.clear();
            }
            m.insert(device, g);
        }
        g
    }

    fn superseded(&self, device: i64, generation: u64) -> bool {
        self.seeks.lock().ok().and_then(|m| m.get(&device).copied()).is_some_and(|g| g != generation)
    }

    fn seg_dir(&self, book: &str, kbps: u64) -> PathBuf {
        self.data_dir.join("saver").join(book).join(format!("k{kbps}"))
    }

    /// One segment (cached on disk). `seek` = Some((device, generation)) for the first segment
    /// of a listener's request, which is dropped if a newer request from that device came in.
    pub async fn segment(
        self: &Arc<Self>,
        file: &Path,
        book: &str,
        layout: &Layout,
        index: u64,
        prefetch: bool,
        seek: Option<(i64, u64)>,
    ) -> Result<bytes::Bytes, String> {
        let bitrate = layout.profile().bitrate();
        let kbps = bitrate.bits_per_second() / 1000;
        let dir = self.seg_dir(book, kbps);
        let path = dir.join(format!("{index:06}.mp3"));
        if let Ok(b) = tokio::fs::read(&path).await {
            return Ok(b.into());
        }
        let key = format!("{book}/{kbps}/{index}");
        let lock = { self.locks.lock().map_err(|_| "poisoned")?.entry(key.clone()).or_default().clone() };
        let _g = lock.lock().await;
        if let Ok(b) = tokio::fs::read(&path).await {
            return Ok(b.into());
        }
        let lane = if prefetch { &self.prefetch } else { &self.fg };
        let _p = lane.acquire().await.map_err(|e| e.to_string())?;
        if let Some((device, g)) = seek
            && self.superseded(device, g)
        {
            return Err("superseded".into());
        }
        let w = layout.window(index).ok_or("no such segment")?;
        let kept = self.encode(file, &dir, layout, &w).await?;
        tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, &kept).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
        if let Ok(mut l) = self.locks.lock() {
            l.remove(&key);
        }
        Ok(kept.into())
    }

    /// Encodes the uncached segments among `first..first + count` with as few ffmpeg runs as
    /// possible (one per run of consecutive missing segments) and caches them. For downloads,
    /// which read the book in order: one start-up per span instead of one per segment.
    pub async fn fill_span(
        self: &Arc<Self>,
        file: &Path,
        book: &str,
        layout: &Layout,
        first: u64,
        count: u64,
    ) -> Result<(), String> {
        let kbps = layout.profile().bitrate().bits_per_second() / 1000;
        let dir = self.seg_dir(book, kbps);
        // One fill per span start at a time: a download's next request waits for the
        // read-ahead already encoding it instead of encoding it again.
        let key = format!("span:{book}/{kbps}/{first}");
        let lock = { self.locks.lock().map_err(|_| "poisoned")?.entry(key.clone()).or_default().clone() };
        let _g = lock.lock().await;
        let r = self.fill_span_locked(file, book, layout, &dir, first, count).await;
        if let Ok(mut l) = self.locks.lock() {
            l.remove(&key);
        }
        r
    }

    async fn fill_span_locked(
        self: &Arc<Self>,
        file: &Path,
        book: &str,
        layout: &Layout,
        dir: &Path,
        first: u64,
        count: u64,
    ) -> Result<(), String> {
        let end = first.saturating_add(count).min(layout.segment_count());
        let missing: Vec<u64> = (first..end).filter(|i| !dir.join(format!("{i:06}.mp3")).exists()).collect();
        let mut i = 0;
        while i < missing.len() {
            let mut j = i;
            while j + 1 < missing.len() && missing[j + 1] == missing[j] + 1 {
                j += 1;
            }
            let (a, n) = (missing[i], (j - i + 1) as u64);
            if n == 1 {
                self.segment(file, book, layout, a, true, None).await?;
            } else {
                let _p = self.prefetch.acquire().await.map_err(|e| e.to_string())?;
                let w = layout.span_window(a, n).ok_or("no such span")?;
                let kept = self.encode(file, dir, layout, &w).await?;
                tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
                let fb = layout.profile().bitrate().frame_bytes() as usize;
                let mut at = 0;
                for idx in a..a + n {
                    let len = layout.segment_frames(idx) as usize * fb;
                    let chunk = kept.get(at..at + len).ok_or("span shorter than its segments")?;
                    at += len;
                    let path = dir.join(format!("{idx:06}.mp3"));
                    // A unique temporary name: a concurrent single-segment encode may race.
                    let tmp = dir.join(format!(
                        ".{idx:06}.{}.tmp",
                        crate::auth::crypto::random::<4>().iter().map(|b| format!("{b:02x}")).collect::<String>()
                    ));
                    tokio::fs::write(&tmp, chunk).await.map_err(|e| e.to_string())?;
                    tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
                }
            }
            i = j + 1;
        }
        Ok(())
    }

    /// Starts encoding the span after a download's current one in the background, so it is
    /// ready (or nearly) when the next piece is asked for.
    pub fn read_ahead(self: &Arc<Self>, file: PathBuf, book: String, layout: Layout, first: u64, count: u64) {
        const MAX_READ_AHEADS: u64 = 4;
        if first >= layout.segment_count() {
            return;
        }
        if self.read_aheads.fetch_add(1, Ordering::SeqCst) >= MAX_READ_AHEADS {
            self.read_aheads.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let _ = me.fill_span(&file, &book, &layout, first, count).await;
            me.read_aheads.fetch_sub(1, Ordering::SeqCst);
        });
    }

    pub fn prefetch(self: &Arc<Self>, file: PathBuf, book: String, layout: Layout, after: u64) {
        for i in after + 1..=after + 2 {
            if i >= layout.segment_count() {
                break;
            }
            let kbps = layout.profile().bitrate().bits_per_second() / 1000;
            if self.seg_dir(&book, kbps).join(format!("{i:06}.mp3")).exists() {
                continue;
            }
            let (me, file, book) = (self.clone(), file.clone(), book.clone());
            tokio::spawn(async move {
                let _ = me.segment(&file, &book, &layout, i, true, None).await;
            });
        }
    }

    fn mp3_args(bitrate: Bitrate) -> Vec<String> {
        let mut v = ["-ac", "1", "-c:a", "libmp3lame", "-b:a"].map(s).to_vec();
        v.push(format!("{}k", bitrate.bits_per_second() / 1000));
        v.extend(
            [
                "-abr",
                "0",
                "-reservoir",
                "0",
                "-write_xing",
                "0",
                "-id3v2_version",
                "0",
                "-write_id3v1",
                "0",
                "-threads",
                "1",
                "-f",
                "mp3",
                "-y",
            ]
            .map(s),
        );
        v
    }

    async fn silent_frame(&self, bitrate: Bitrate) -> Result<Vec<u8>, String> {
        let kbps = bitrate.bits_per_second() / 1000;
        if let Some(f) = self.silent.lock().ok().and_then(|m| m.get(&kbps).cloned()) {
            return Ok(f);
        }
        let job = self.data_dir.join("jobs").join(format!("silent-{kbps}"));
        let _ = tokio::fs::remove_dir_all(&job).await;
        tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
        let out = job.join("s.mp3");
        let mut args = [
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=24000:cl=mono",
            "-t",
            "2",
        ]
        .map(s)
        .to_vec();
        args.extend(Self::mp3_args(bitrate));
        args.push(out.to_string_lossy().into_owned());
        let limits =
            Limits { mem_mib: 2048, cpu_secs: 30, fsize_mib: 4, wall: Duration::from_secs(60), stdout_cap: 1024 };
        run_jailed(&self.ff, &[], Some(&job), &self.ff.ffmpeg, &args, &limits).await?;
        let bytes = tokio::fs::read(&out).await.map_err(|e| e.to_string())?;
        let _ = tokio::fs::remove_dir_all(&job).await;
        let fb = bitrate.frame_bytes() as usize;
        let frame = bytes.get(20 * fb..21 * fb).ok_or("silence too short")?.to_vec();
        if let Ok(mut m) = self.silent.lock() {
            m.insert(kbps, frame.clone());
        }
        Ok(frame)
    }

    async fn encode(&self, file: &Path, dir: &Path, layout: &Layout, w: &SegmentWindow) -> Result<Vec<u8>, String> {
        let job = self.data_dir.join("jobs").join(format!(
            "seg-{}",
            crate::auth::crypto::random::<6>().iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
        let _ = dir;
        let out = job.join("out.mp3");
        let demuxer = demuxer_for(&ext(file)).ok_or("unsupported source")?;
        let mut args =
            ["-nostdin", "-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file"].map(s).to_vec();
        if demuxer == "mov" {
            args.extend(["-enable_drefs", "0"].map(s));
        }
        args.extend([s("-f"), s(demuxer)]);
        if w.seek {
            args.extend([s("-ss"), format!("{:.6}", w.seek_seconds()), s("-copyts")]);
        }
        let trim = match w.input_end {
            Some(end) => format!("atrim=start={}:end={}", secs(w.input_start), secs(end)),
            None => format!("atrim=start={}", secs(w.input_start)),
        };
        args.extend([
            s("-i"),
            file.to_string_lossy().into_owned(),
            s("-map"),
            s("0:a:0"),
            s("-vn"),
            s("-map_metadata"),
            s("-1"),
            s("-af"),
            format!("aresample={SAMPLE_RATE},asettb=1/{SAMPLE_RATE},{trim},asetpts=PTS-STARTPTS"),
        ]);
        let bitrate = layout.profile().bitrate();
        args.extend(Self::mp3_args(bitrate));
        args.push(out.to_string_lossy().into_owned());
        let limits =
            Limits { mem_mib: 2048, cpu_secs: 120, fsize_mib: 64, wall: Duration::from_secs(180), stdout_cap: 1024 };
        let r = run_jailed(&self.ff, &[file], Some(&job), &self.ff.ffmpeg, &args, &limits).await;
        let bytes = tokio::fs::read(&out).await;
        let _ = tokio::fs::remove_dir_all(&job).await;
        let r = r?;
        if !r.success {
            return Err(format!("ffmpeg: {}", r.stderr.lines().next().unwrap_or("failed")));
        }
        let bytes = bytes.map_err(|e| e.to_string())?;
        let acc = layout.check_segment(w, &bytes).map_err(|e| format!("segment {}: {e}", w.index))?;
        let mut kept = bytes[acc.start as usize..acc.end as usize].to_vec();
        if acc.pad_frames > 0 {
            let silent = self.silent_frame(bitrate).await?;
            for _ in 0..acc.pad_frames {
                kept.extend_from_slice(&silent);
            }
        }
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic mono "book" of `secs` seconds, made with the system ffmpeg (not jailed:
    /// test set-up, not untrusted input).
    fn tone(dir: &Path, secs: u32) -> PathBuf {
        let p = dir.join("book.mp3");
        let ok = std::process::Command::new("/usr/bin/ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=f=330:sample_rate=22050:d={secs}"))
            .args(["-c:a", "libmp3lame", "-b:a", "64k", "-y"])
            .arg(&p)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg could not make the test tone");
        p
    }

    #[tokio::test]
    async fn the_cache_sweep_evicts_least_recently_used_and_spares_what_is_playing() {
        let base = std::env::temp_dir().join(format!("abshare-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ff = Ffmpeg { ffmpeg: "/x".into(), ffprobe: "/x".into(), jail: "/x".into(), libs: vec![], jobs: 1 };
        let m = Media::new(ff, base.clone(), HashMap::new());
        let (old, mid, new, copy) = ("o".repeat(26), "m".repeat(26), "n".repeat(26), "c".repeat(26));
        let day = std::time::Duration::from_secs(86_400);
        let ago = |d: u32| std::time::SystemTime::now() - day * d;
        for (book, age) in [(&old, 3), (&mid, 2), (&new, 1)] {
            let dir = base.join("saver").join(book).join("k48");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let f = dir.join("000000.mp3");
            std::fs::write(&f, vec![0u8; 1000]).expect("write");
            std::fs::File::options().write(true).open(&f).expect("open").set_modified(ago(age)).expect("mtime");
        }
        std::fs::create_dir_all(base.join("copies")).expect("mkdir");
        let c = base.join("copies").join(format!("{copy}.m4b"));
        std::fs::write(&c, vec![0u8; 1000]).expect("write");
        std::fs::File::options().write(true).open(&c).expect("open").set_modified(ago(10)).expect("mtime");
        // Not ours: never touched.
        std::fs::write(base.join("saver").join("notes.txt"), b"x").expect("write");
        // Under the limit: nothing goes.
        assert_eq!(m.sweep(10_000).await.expect("sweep"), 0);
        // The copy is the oldest but is being listened to right now.
        m.mark_used(&copy, true);
        // 4000 bytes over a 2500 limit: evict to <= 2250, oldest unpinned first.
        assert_eq!(m.sweep(2_500).await.expect("sweep"), 2_000);
        assert!(!base.join("saver").join(&old).exists());
        assert!(!base.join("saver").join(&mid).exists());
        assert!(base.join("saver").join(&new).exists());
        assert!(c.exists(), "a copy in use is never evicted");
        assert!(base.join("saver").join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn the_sandbox_self_check_passes_where_the_jail_runs() {
        let Some(ff) = crate::tools::test_ffmpeg().await else {
            eprintln!("skipped: the jailed ffmpeg cannot run here");
            return;
        };
        let m = Media::new(ff, std::env::temp_dir(), HashMap::new());
        assert!(m.sandbox_works().await, "a working jail must not be reported as broken");
        let broken = Ffmpeg {
            ffmpeg: "/usr/bin/ffmpeg".into(),
            ffprobe: "/x".into(),
            jail: "/nonexistent/jail".into(),
            libs: vec![],
            jobs: 1,
        };
        assert!(!Media::new(broken, std::env::temp_dir(), HashMap::new()).sandbox_works().await);
    }

    #[tokio::test]
    async fn a_batch_encoded_span_matches_the_layout_of_single_segments() {
        let Some(ff) = crate::tools::test_ffmpeg().await else {
            eprintln!("skipped: the jailed ffmpeg cannot run here");
            return;
        };
        let base = std::env::temp_dir().join(format!("abshare-span-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (lib, one, many) = (base.join("lib"), base.join("one"), base.join("many"));
        for d in [&lib, &one, &many] {
            std::fs::create_dir_all(d.join("jobs")).expect("mkdir");
        }
        // 3.5 minutes: three full 60 s segments and a short last one.
        let file = tone(&lib, 210);
        let layout =
            Profile::new(Bitrate::K48, 2_500).expect("profile").layout(samples_from_ms(210_000)).expect("layout");
        assert_eq!(layout.segment_count(), 4);
        let ff2 = Ffmpeg {
            ffmpeg: ff.ffmpeg.clone(),
            ffprobe: ff.ffprobe.clone(),
            jail: ff.jail.clone(),
            libs: ff.libs.clone(),
            jobs: 1,
        };
        let a = Arc::new(Media::new(ff, one.clone(), HashMap::new()));
        let b = Arc::new(Media::new(ff2, many.clone(), HashMap::new()));
        let book = "aaaaaaaaaaaaaaaaaaaaaaaaaa";
        b.fill_span(&file, book, &layout, 0, 4).await.expect("span");
        let fb = Bitrate::K48.frame_bytes() as usize;
        for i in 0..4 {
            let single = a.segment(&file, book, &layout, i, false, None).await.expect("segment");
            let batched = std::fs::read(b.seg_dir(book, 48).join(format!("{i:06}.mp3"))).expect("cached");
            assert_eq!(batched.len(), single.len(), "segment {i}");
            assert_eq!(batched.len(), layout.segment_frames(i) as usize * fb, "segment {i}");
            assert!(batched.chunks_exact(fb).all(|f| f[..3] == Bitrate::K48.header_prefix()), "segment {i} frames");
        }
        // Already cached: nothing to do, and no error.
        b.fill_span(&file, book, &layout, 0, 4).await.expect("cached span");
        // The whole virtual file is exactly the layout's length.
        let total: usize = (0..4)
            .map(|i| std::fs::metadata(b.seg_dir(book, 48).join(format!("{i:06}.mp3"))).map_or(0, |m| m.len() as usize))
            .sum();
        assert_eq!(total as u64, layout.total_bytes());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn files_outside_the_root_or_through_symlinks_are_refused() {
        let base = std::env::temp_dir().join(format!("abshare-media-{}", std::process::id()));
        let root = base.join("lib");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("a.m4b"), b"x").expect("w");
        std::fs::write(base.join("secret"), b"x").expect("w");
        std::os::unix::fs::symlink(base.join("secret"), root.join("link.m4b")).expect("link");
        assert!(open_inside(&root, &root.join("a.m4b")).is_ok());
        assert!(open_inside(&root, &root.join("../secret")).is_err());
        assert!(open_inside(&root, &root.join("link.m4b")).is_err());
        assert!(open_inside(&root, &root).is_err(), "a directory");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bitrates_are_an_allow_list() {
        assert!(bitrate_for(48).is_some() && bitrate_for(40).is_some() && bitrate_for(32).is_some());
        assert!(bitrate_for(320).is_none() && bitrate_for(0).is_none());
    }
}
