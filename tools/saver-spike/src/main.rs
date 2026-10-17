//! Phase 0(f) spike (SPEC D23): serves ONE book as a data-saver virtual MP3, plus the
//! original, to test seeking, lock-screen playback and segment cost on real devices.
//!
//! This is a test tool. It has no authentication and must only be reachable on the LAN
//! (behind a source-address allow-list). It does not ship.
//!
//! Every ffmpeg/ffprobe run goes through `abshare-jail`.

use abshare_core::range::{self, RangeReply};
use abshare_core::saver::{Layout, Profile, SAMPLE_RATE, SegmentWindow};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Semaphore;

struct Opts {
    listen: String,
    source: PathBuf,
    cache: PathBuf,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    jail: PathBuf,
    libs: Vec<PathBuf>,
}

fn parse_opts() -> Result<Opts, String> {
    let mut o = Opts {
        listen: "127.0.0.1:8743".into(),
        source: PathBuf::new(),
        cache: PathBuf::new(),
        ffmpeg: "/usr/bin/ffmpeg".into(),
        ffprobe: "/usr/bin/ffprobe".into(),
        jail: "/usr/local/bin/abshare-jail".into(),
        libs: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut v = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--listen" => o.listen = v()?,
            "--source" => o.source = v()?.into(),
            "--cache" => o.cache = v()?.into(),
            "--ffmpeg" => o.ffmpeg = v()?.into(),
            "--ffprobe" => o.ffprobe = v()?.into(),
            "--jail" => o.jail = v()?.into(),
            "--lib" => o.libs.push(v()?.into()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if o.source.as_os_str().is_empty() || o.cache.as_os_str().is_empty() {
        return Err("usage: abshare-saver-spike --source FILE --cache DIR [--listen ADDR] [--lib DIR]...".into());
    }
    Ok(o)
}

struct SegStat {
    index: u64,
    millis: u128,
    prefetch: bool,
}

struct App {
    o: Opts,
    layout: Layout,
    demuxer: &'static str,
    original_len: u64,
    etag: String,
    silent_frame: Vec<u8>,
    /// A listener waiting on a segment never queues behind prefetch (SPEC §5.4 fast lane).
    jobs: Semaphore,
    prefetch_jobs: Semaphore,
    locks: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>,
    stats: Mutex<Vec<SegStat>>,
}

type Shared = Arc<App>;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("abshare-saver-spike: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let o = parse_opts()?;
    tokio::fs::create_dir_all(&o.cache).await.map_err(|e| format!("cache dir: {e}"))?;
    let meta = tokio::fs::metadata(&o.source).await.map_err(|e| format!("source: {e}"))?;
    let (duration, format) = probe(&o).await?;
    let demuxer = match format.split(',').next() {
        Some("mov") => "mov",
        Some("mp3") => "mp3",
        other => return Err(format!("unsupported container {other:?}")),
    };
    let samples = (duration * f64::from(SAMPLE_RATE)).round() as u64;
    let layout = Profile::DEFAULT.layout(samples).map_err(|e| e.to_string())?;
    let silent_frame = silent_frame(&o).await?;
    let etag = format!("\"spike-{}-{}\"", meta.len(), layout.total_bytes());
    eprintln!(
        "source {:.1} h, {demuxer}; virtual MP3 {} bytes in {} segments",
        duration / 3600.0,
        layout.total_bytes(),
        layout.segment_count()
    );
    let listen = o.listen.clone();
    let app = Arc::new(App {
        o,
        layout,
        demuxer,
        original_len: meta.len(),
        etag,
        silent_frame,
        jobs: Semaphore::new(1),
        prefetch_jobs: Semaphore::new(1),
        locks: Mutex::default(),
        stats: Mutex::default(),
    });
    let router = Router::new()
        .route("/", get(page))
        .route("/app.js", get(script))
        .route("/app.css", get(style))
        .route("/media", get(media))
        .route("/original", get(original))
        .route("/stats", get(stats))
        .route("/log", post(client_log))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&listen).await.map_err(|e| format!("bind {listen}: {e}"))?;
    eprintln!("listening on {listen} (LAN-only test tool, no auth)");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

/// Runs a program under the jail with a time limit; returns stdout.
async fn jailed(o: &Opts, jail_args: &[String], program: &Path, args: &[String]) -> Result<Vec<u8>, String> {
    let mut cmd = tokio::process::Command::new(&o.jail);
    cmd.args(jail_args);
    for l in &o.libs {
        cmd.arg("--lib").arg(l);
    }
    cmd.arg("--").arg(program).args(args);
    cmd.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("spawn jail: {e}"))?;
    let out = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output())
        .await
        .map_err(|_| "ffmpeg timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr[..out.stderr.len().min(4096)]).into_owned();
        return Err(format!("{} failed ({}): {err}", program.display(), out.status));
    }
    Ok(out.stdout)
}

fn s(x: impl ToString) -> String {
    x.to_string()
}

async fn probe(o: &Opts) -> Result<(f64, String), String> {
    let src = o.source.to_string_lossy().into_owned();
    let args =
        ["-v", "error", "-show_entries", "format=duration,format_name", "-of", "default=noprint_wrappers=1", &src]
            .map(s);
    let out = jailed(o, &[s("--read"), src.clone(), s("--mem-mib"), s(512)], &o.ffprobe, &args).await?;
    let text = String::from_utf8_lossy(&out);
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::to_owned);
    let duration = field("duration=").and_then(|d| d.parse::<f64>().ok()).ok_or("no duration")?;
    let format = field("format_name=").ok_or("no format")?;
    Ok((duration, format))
}

fn mp3_args() -> Vec<String> {
    [
        "-ac",
        "1",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "32k",
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
    .map(s)
    .to_vec()
}

/// One silent frame of the profile, used to pad a short last segment.
async fn silent_frame(o: &Opts) -> Result<Vec<u8>, String> {
    let job = o.cache.join("job-silent");
    let _ = tokio::fs::remove_dir_all(&job).await;
    tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
    let out = job.join("out.mp3");
    let mut args =
        ["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=24000:cl=mono", "-t", "2"]
            .map(s)
            .to_vec();
    args.extend(mp3_args());
    args.push(out.to_string_lossy().into_owned());
    jailed(o, &[s("--write"), job.to_string_lossy().into_owned()], &o.ffmpeg, &args).await?;
    let bytes = tokio::fs::read(&out).await.map_err(|e| e.to_string())?;
    let _ = tokio::fs::remove_dir_all(&job).await;
    let fb = Profile::DEFAULT.bitrate().frame_bytes() as usize;
    let frame = bytes.get(20 * fb..21 * fb).ok_or("silence too short")?.to_vec();
    if frame[..3] != Profile::DEFAULT.bitrate().header_prefix() {
        return Err("silent frame has an unexpected header".into());
    }
    Ok(frame)
}

/// Exact decimal seconds for a sample count on the alignment grid.
fn secs(samples: u64) -> String {
    let rate = u64::from(SAMPLE_RATE);
    format!("{}.{:06}", samples / rate, (samples % rate) * 1_000_000 / rate)
}

impl App {
    fn seg_path(&self, i: u64) -> PathBuf {
        self.o.cache.join(format!("seg-{i:06}.mp3"))
    }

    async fn segment(self: &Arc<Self>, i: u64, prefetch: bool) -> Result<Bytes, String> {
        let path = self.seg_path(i);
        if let Ok(b) = tokio::fs::read(&path).await {
            return Ok(b.into());
        }
        let lock = {
            let mut m = self.locks.lock().map_err(|_| "poisoned")?;
            m.entry(i).or_default().clone()
        };
        let _guard = lock.lock().await;
        if let Ok(b) = tokio::fs::read(&path).await {
            return Ok(b.into());
        }
        let lane = if prefetch { &self.prefetch_jobs } else { &self.jobs };
        let _permit = lane.acquire().await.map_err(|e| e.to_string())?;
        let started = Instant::now();
        let window = self.layout.window(i).ok_or("no such segment")?;
        let kept = self.encode(&window).await?;
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, &kept).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
        if let Ok(mut st) = self.stats.lock() {
            st.push(SegStat { index: i, millis: started.elapsed().as_millis(), prefetch });
        }
        Ok(kept.into())
    }

    async fn encode(&self, w: &SegmentWindow) -> Result<Vec<u8>, String> {
        let job = self.o.cache.join(format!("job-{:06}", w.index));
        let _ = tokio::fs::remove_dir_all(&job).await;
        tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
        let out = job.join("out.mp3");
        let src = self.o.source.to_string_lossy().into_owned();
        let mut args =
            ["-nostdin", "-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file"].map(s).to_vec();
        if self.demuxer == "mov" {
            args.extend(["-enable_drefs", "0"].map(s));
        }
        args.extend([s("-f"), s(self.demuxer)]);
        if w.seek {
            args.extend([s("-ss"), format!("{:.6}", w.seek_seconds()), s("-copyts")]);
        }
        let trim = match w.input_end {
            Some(end) => format!("atrim=start={}:end={}", secs(w.input_start), secs(end)),
            None => format!("atrim=start={}", secs(w.input_start)),
        };
        args.extend([
            s("-i"),
            src.clone(),
            s("-map"),
            s("0:a:0"),
            s("-vn"),
            s("-map_metadata"),
            s("-1"),
            s("-af"),
            format!("aresample={SAMPLE_RATE},asettb=1/{SAMPLE_RATE},{trim},asetpts=PTS-STARTPTS"),
        ]);
        args.extend(mp3_args());
        args.push(out.to_string_lossy().into_owned());
        let jail_args = [
            s("--read"),
            src,
            s("--write"),
            job.to_string_lossy().into_owned(),
            s("--mem-mib"),
            s(2048),
            s("--cpu-secs"),
            s(120),
            s("--fsize-mib"),
            s(64),
        ];
        let result = jailed(&self.o, &jail_args, &self.o.ffmpeg, &args).await;
        let bytes = tokio::fs::read(&out).await;
        let _ = tokio::fs::remove_dir_all(&job).await;
        result?;
        let bytes = bytes.map_err(|e| e.to_string())?;
        let acc = self.layout.check_segment(w, &bytes).map_err(|e| format!("segment {}: {e}", w.index))?;
        let mut kept = bytes[acc.start as usize..acc.end as usize].to_vec();
        for _ in 0..acc.pad_frames {
            kept.extend_from_slice(&self.silent_frame);
        }
        Ok(kept)
    }

    fn prefetch(self: &Arc<Self>, after: u64) {
        for i in after + 1..=after + 2 {
            if i < self.layout.segment_count() && !self.seg_path(i).exists() {
                let app = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = app.segment(i, true).await {
                        eprintln!("prefetch {i}: {e}");
                    }
                });
            }
        }
    }
}

fn base_headers(h: &mut HeaderMap) {
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
}

fn range_header(h: &HeaderMap) -> Option<&str> {
    h.get(header::RANGE).and_then(|v| v.to_str().ok())
}

fn unsatisfiable(len: u64) -> Response {
    let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("bytes */{len}")) {
        r.headers_mut().insert(header::CONTENT_RANGE, v);
    }
    r
}

fn ranged(
    len: u64,
    req: &HeaderMap,
    ctype: &'static str,
    etag: &str,
) -> Result<(StatusCode, HeaderMap, u64, u64), Box<Response>> {
    let (status, start, end) = match range::parse(range_header(req), len) {
        RangeReply::Full => (StatusCode::OK, 0, len - 1),
        RangeReply::Partial { start, end } => (StatusCode::PARTIAL_CONTENT, start, end),
        RangeReply::Unsatisfiable => return Err(Box::new(unsatisfiable(len))),
    };
    let mut h = HeaderMap::new();
    base_headers(&mut h);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let v =
        |s: String| HeaderValue::from_str(&s).map_err(|_| Box::new(StatusCode::INTERNAL_SERVER_ERROR.into_response()));
    h.insert(header::CONTENT_LENGTH, v((end - start + 1).to_string())?);
    h.insert(header::ETAG, v(etag.to_owned())?);
    if status == StatusCode::PARTIAL_CONTENT {
        h.insert(header::CONTENT_RANGE, v(format!("bytes {start}-{end}/{len}"))?);
    }
    Ok((status, h, start, end))
}

async fn media(State(app): State<Shared>, req: HeaderMap) -> Response {
    let len = app.layout.total_bytes();
    let (status, headers, start, end) = match ranged(len, &req, "audio/mpeg", &app.etag) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let segs = match app.layout.segments_for(start, end) {
        Ok(s) => s,
        Err(_) => return unsatisfiable(len),
    };
    let first = *segs.start();
    let stream = futures_util::stream::unfold((app, segs), move |(app, mut segs)| async move {
        let i = segs.next()?;
        let off = app.layout.segment_offset(i);
        let item = match app.segment(i, false).await {
            Ok(bytes) => {
                let lo = start.saturating_sub(off) as usize;
                let hi = ((end + 1 - off) as usize).min(bytes.len());
                app.prefetch(i);
                Ok(bytes.slice(lo..hi))
            }
            Err(e) => {
                eprintln!("segment {i}: {e}");
                Err(std::io::Error::other(e))
            }
        };
        Some((item, (app, segs)))
    });
    eprintln!("media {start}-{end} (segment {first}+)");
    (status, headers, Body::from_stream(stream)).into_response()
}

async fn original(State(app): State<Shared>, req: HeaderMap) -> Response {
    let ctype = if app.demuxer == "mov" { "audio/mp4" } else { "audio/mpeg" };
    let etag = format!("\"orig-{}\"", app.original_len);
    let (status, headers, start, end) = match ranged(app.original_len, &req, ctype, &etag) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let mut f = match tokio::fs::File::open(&app.o.source).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(f.take(end - start + 1)));
    (status, headers, body).into_response()
}

async fn stats(State(app): State<Shared>) -> Response {
    let st = app.stats.lock().map(|s| {
        let rows: Vec<String> = s
            .iter()
            .map(|x| format!("{{\"segment\":{},\"ms\":{},\"prefetch\":{}}}", x.index, x.millis, x.prefetch))
            .collect();
        rows.join(",")
    });
    let body = format!(
        "{{\"total_bytes\":{},\"segments\":{},\"jobs\":[{}]}}",
        app.layout.total_bytes(),
        app.layout.segment_count(),
        st.unwrap_or_default()
    );
    let mut h = HeaderMap::new();
    base_headers(&mut h);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    (h, body).into_response()
}

async fn client_log(State(app): State<Shared>, body: Bytes) -> StatusCode {
    let text: String = String::from_utf8_lossy(&body[..body.len().min(4096)])
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = format!("{stamp} {}\n", text.replace('\n', " | "));
    let path = app.o.cache.join("client.log");
    let res = async {
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
        f.write_all(line.as_bytes()).await
    }
    .await;
    if res.is_ok() { StatusCode::NO_CONTENT } else { StatusCode::INTERNAL_SERVER_ERROR }
}

fn static_file(body: &'static str, ctype: &'static str) -> Response {
    let mut h = HeaderMap::new();
    base_headers(&mut h);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; media-src 'self'; connect-src 'self'; \
             img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    (h, body).into_response()
}

async fn page() -> Response {
    static_file(include_str!("../static/index.html"), "text/html; charset=utf-8")
}

async fn script() -> Response {
    static_file(include_str!("../static/app.js"), "text/javascript; charset=utf-8")
}

async fn style() -> Response {
    static_file(include_str!("../static/app.css"), "text/css; charset=utf-8")
}

#[cfg(test)]
mod tests {
    use super::secs;

    #[test]
    fn seconds_are_exact_on_the_alignment_grid() {
        assert_eq!(secs(0), "0.000000");
        assert_eq!(secs(2_880), "0.120000");
        assert_eq!(secs(1_440_000 - 25_920), "58.920000");
        assert_eq!(secs(24_000 * 3_600 + 480), "3600.020000");
    }
}
