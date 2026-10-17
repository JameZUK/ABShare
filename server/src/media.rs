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

impl Media {
    pub fn new(ff: Ffmpeg, data_dir: PathBuf, roots: HashMap<String, PathBuf>) -> Self {
        Self {
            ff,
            data_dir,
            roots,
            fg: Semaphore::new(1),
            prefetch: Semaphore::new(1),
            copies: Semaphore::new(1),
            locks: Mutex::default(),
            copying: Mutex::default(),
            seeks: Mutex::default(),
            seek_counter: AtomicU64::new(0),
            silent: Mutex::default(),
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
