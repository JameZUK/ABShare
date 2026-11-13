//! Covers (SPEC §4.6): the source image is extracted by jailed ffmpeg (embedded) or read
//! from a sidecar, then decoded with limits (JPEG and PNG only) and re-encoded as JPEG
//! thumbnails. Re-encoding strips all metadata and malformed-image tricks.
//!
//! The scanner never decodes an image itself. The decoder (`covers/render.rs`, which is
//! fuzzed) runs only in the `render-cover` command of this same binary (`covers/child.rs`),
//! and the scanner runs that command in `abshare-jail`, as it runs ffmpeg: it can read and
//! write one job directory and nothing else, under memory, CPU, file-size and wall-clock
//! limits. A decoder that panics, hangs or is taken over costs one book its cover, which is
//! recorded as a problem for that book; the scan goes on. To the scanner the image, and the
//! thumbnails the command hands back, are bytes it hashes, measures and copies.

mod child;

use crate::config::Ffmpeg;
use crate::tools::{JAIL_REFUSED, JailError, JobDir, Limits, demuxer_for, run_jailed, run_jailed_classified};
pub use child::SIZES;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_SOURCE: usize = child::MAX_SOURCE as usize;
/// The largest thumbnail accepted back from the renderer. One of 1,280 pixels a side is a
/// few hundred kilobytes, and can't pass two megabytes.
const MAX_THUMB: u64 = 4 * 1024 * 1024;
/// The marker left in a book's cover directory when its cover could not be rendered.
const FAILED: &str = "failed";

/// The jailed renderer's limits, sized for the largest image it accepts (16 megapixels, at
/// most 128 MiB decoded). Measured on such an image at 16 bits a channel with transparency:
/// it needs between 192 and 256 MiB of address space, and about a second of CPU in a release
/// build (14 s unoptimised, as in the tests). So: twice the memory, and time to spare.
fn render_limits() -> Limits {
    Limits { mem_mib: 512, cpu_secs: 60, fsize_mib: 4, wall: Duration::from_secs(90), stdout_cap: 1024 }
}

/// Runs the cover renderer if the process was started as `abshare render-cover …`; `None`
/// for every other command. Called first thing in `main`: the command needs no configuration.
pub fn render_command(args: impl Iterator<Item = std::ffi::OsString>) -> Option<std::process::ExitCode> {
    child::command(args)
}

/// The binary that holds the `render-cover` command: this one, resolved once.
pub fn renderer() -> Result<&'static Path, String> {
    static EXE: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        #[cfg(not(test))]
        let exe = std::env::current_exe().map_err(|e| format!("cannot find the running abshare binary: {e}"));
        // The unit tests are their own binary; `abshare` is built beside the jail (for the
        // tests in `server/tests`).
        #[cfg(test)]
        let exe = Ok(std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"))
            .join(if cfg!(debug_assertions) { "debug" } else { "release" })
            .join("abshare"));
        exe
    })
    .as_deref()
    .map_err(Clone::clone)
}

/// Whether the thumbnails for `hash` are on disk.
pub fn exists(data_dir: &Path, book_id: &str, hash: &str) -> bool {
    SIZES.iter().all(|s| data_dir.join("covers").join(book_id).join(format!("{hash}-{s}.jpg")).exists())
}

pub enum Source {
    Embedded {
        file: PathBuf,
        ext: String,
    },
    /// An image file beside the book, opened safely inside its library root.
    Sidecar {
        root: PathBuf,
        rel: String,
    },
}

/// Why a cover was not rendered.
#[derive(Debug, PartialEq, Eq)]
enum Failed {
    /// The renderer ran and refused the image, hit a limit, was killed, or handed back
    /// something that is not a thumbnail. The same image would fail the same way again.
    Cover(String),
    /// The renderer never ran (the jail could not start it): nothing was learnt about the image.
    Jail(String),
}

/// Makes the thumbnails for one book. Returns the cover hash (part of each thumbnail's
/// immutable URL). Thumbnails that already exist for the same source are reused.
///
/// `fingerprint` identifies the source (file, size, modification time). A cover the renderer
/// failed on is remembered under it, so a source that has not changed is not rendered again
/// on every scan: the same error is returned at once.
pub async fn make(
    ff: &Ffmpeg,
    data_dir: &Path,
    book_id: &str,
    src: Source,
    fingerprint: &str,
) -> Result<String, String> {
    make_with(ff, renderer()?, &render_limits(), data_dir, book_id, src, fingerprint).await
}

async fn make_with(
    ff: &Ffmpeg,
    renderer: &Path,
    limits: &Limits,
    data_dir: &Path,
    book_id: &str,
    src: Source,
    fingerprint: &str,
) -> Result<String, String> {
    let dir = data_dir.join("covers").join(book_id);
    if let Some(why) = failed_before(&dir, fingerprint) {
        return Err(why);
    }
    let bytes = match src {
        Source::Sidecar { root, rel } => tokio::task::spawn_blocking(move || {
            crate::media::read_inside(&root, &rel, MAX_SOURCE as u64)
                .map_err(|_| "cover image missing or too large".to_owned())
        })
        .await
        .map_err(|e| e.to_string())??,
        Source::Embedded { file, ext } => extract(ff, data_dir, &file, &ext).await?,
    };
    let hash: String = Sha256::digest(&bytes).iter().take(8).map(|b| format!("{b:02x}")).collect();
    if dir.join(format!("{hash}-320.jpg")).exists() {
        let _ = tokio::fs::remove_file(dir.join(FAILED)).await;
        return Ok(hash);
    }
    let thumbs = match render_jailed(ff, renderer, limits, data_dir, &bytes).await {
        Ok(thumbs) => thumbs,
        Err(Failed::Cover(why)) => {
            remember_failure(&dir, fingerprint, &why);
            return Err(why);
        }
        Err(Failed::Jail(why)) => return Err(why),
    };
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    for (size, jpeg) in thumbs {
        let path = dir.join(format!("{hash}-{size}.jpg"));
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, &jpeg).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
    }
    // Drop thumbnails of earlier covers for this book (and the mark of an earlier failure).
    if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if !e.file_name().to_string_lossy().starts_with(&hash) {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    Ok(hash)
}

/// The first line (that says anything) of a program's diagnostics as one short line of
/// text: it ends up in the problem list, and comes from a process that handled untrusted bytes.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// Renders the thumbnails of `bytes` in the jail: the image is written to a job directory,
/// `renderer render-cover` runs with access to that directory alone, and its output is read
/// back as untrusted. The job directory is removed however this ends.
async fn render_jailed(
    ff: &Ffmpeg,
    renderer: &Path,
    limits: &Limits,
    data_dir: &Path,
    bytes: &[u8],
) -> Result<Vec<(u32, Vec<u8>)>, Failed> {
    let job = JobDir::create(data_dir, "cover").map_err(Failed::Jail)?;
    let source = job.path().join("source");
    tokio::fs::write(&source, bytes).await.map_err(|e| Failed::Jail(format!("cover job: {e}")))?;
    let args =
        [child::COMMAND.to_owned(), source.to_string_lossy().into_owned(), job.path().to_string_lossy().into_owned()];
    // No `--read`: the image is in the job directory, the one place the renderer may touch.
    match run_jailed_classified(ff, &[], Some(job.path()), renderer, &args, limits).await {
        Err(JailError::Start(e)) => Err(Failed::Jail(e)),
        Err(JailError::TimedOut(_)) => {
            Err(Failed::Cover(format!("cover rendering stopped: it took more than {} s", limits.wall.as_secs_f32())))
        }
        Err(JailError::Output(_)) => Err(Failed::Cover("cover rendering stopped: the renderer misbehaved".into())),
        Ok(o) if o.code == Some(JAIL_REFUSED) => {
            Err(Failed::Jail(format!("the jail refused to run the cover renderer: {}", first_line(&o.stderr))))
        }
        Ok(o) if !o.success => Err(Failed::Cover(match o.code {
            Some(1) if !first_line(&o.stderr).is_empty() => first_line(&o.stderr),
            Some(code) => format!("cover rendering failed (status {code}): {}", first_line(&o.stderr)),
            None => "cover rendering stopped: the renderer crashed or passed a limit".into(),
        })),
        Ok(_) => collect(job.path()).map_err(Failed::Cover),
    }
}

/// Reads the thumbnails the renderer left in `dir`. They are untrusted too: each expected
/// file must be a regular file of a sane size that begins and ends as a JPEG does. Anything
/// else in the directory is ignored.
fn collect(dir: &Path) -> Result<Vec<(u32, Vec<u8>)>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut out = Vec::new();
    for size in SIZES {
        let refused = |why: &str| format!("cover renderer output refused: the {size} px thumbnail {why}");
        // Never through a link, and never blocking on something that is not a file.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(dir.join(child::file_name(size)))
            .map_err(|_| refused("is missing"))?;
        let meta = file.metadata().map_err(|_| refused("can't be read"))?;
        if !meta.is_file() {
            return Err(refused("is not a regular file"));
        }
        let mut jpeg = Vec::new();
        let read = file.take(MAX_THUMB + 1).read_to_end(&mut jpeg);
        if meta.len() > MAX_THUMB || jpeg.len() as u64 > MAX_THUMB {
            return Err(refused("is too large"));
        }
        read.map_err(|_| refused("can't be read"))?;
        if !jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) || !jpeg.ends_with(&[0xFF, 0xD9]) {
            return Err(refused("is not a JPEG"));
        }
        out.push((size, jpeg));
    }
    Ok(out)
}

/// The first line of a failure marker: the fingerprint, hashed (a file name may hold a line
/// break).
fn marker_key(fingerprint: &str) -> String {
    Sha256::digest(fingerprint.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The error remembered for this book's cover, if the source is the one that failed.
fn failed_before(dir: &Path, fingerprint: &str) -> Option<String> {
    if fingerprint.is_empty() {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(dir.join(FAILED)).ok()?.take(4096).read_to_string(&mut text).ok()?;
    let (key, why) = text.split_once('\n')?;
    (key == marker_key(fingerprint)).then(|| first_line(why))
}

/// Remembers that the cover from this source can't be rendered. Best effort: if it can't be
/// written, the cover is tried again on the next scan.
fn remember_failure(dir: &Path, fingerprint: &str, why: &str) {
    if fingerprint.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(dir.join(FAILED), format!("{}\n{}\n", marker_key(fingerprint), first_line(why))));
}

/// A two-by-two PNG, for [`self_test`].
const SAMPLE_PNG: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a, 0x73, 0x00, 0x00, 0x00, 0x10, 0x49,
    0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xd0, 0x88, 0xea, 0x01, 0x22, 0x06, 0x08, 0x05, 0x00, 0x1c, 0x06, 0x04, 0x39,
    0x38, 0xfb, 0xfb, 0xd3, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// For `abshare doctor`: renders a tiny built-in image exactly as a scan would (this binary,
/// in the jail, with the scan's limits, in a job directory that is removed again).
pub async fn self_test(ff: &Ffmpeg, data_dir: &Path) -> Result<String, String> {
    match render_jailed(ff, renderer()?, &render_limits(), data_dir, &SAMPLE_PNG).await {
        Ok(thumbs) => Ok(format!("a test image rendered in the jail ({} thumbnails)", thumbs.len())),
        Err(Failed::Cover(e) | Failed::Jail(e)) => Err(e),
    }
}

async fn extract(ff: &Ffmpeg, data_dir: &Path, file: &Path, ext: &str) -> Result<Vec<u8>, String> {
    let demuxer = demuxer_for(ext).ok_or("unsupported container")?;
    let job = JobDir::create(data_dir, "cover")?;
    let out = job.path().join("cover.img");
    let args: Vec<String> = [
        "-nostdin",
        "-hide_banner",
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-f",
        demuxer,
        "-i",
        &file.to_string_lossy(),
        "-map",
        "0:v:0",
        "-c",
        "copy",
        "-frames:v",
        "1",
        "-f",
        "image2",
        "-update",
        "1",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let limits = Limits { mem_mib: 2048, cpu_secs: 30, fsize_mib: 20, wall: Duration::from_secs(60), stdout_cap: 1024 };
    let r = run_jailed(ff, &[file], Some(job.path()), &ff.ffmpeg, &args, &limits).await?;
    if !r.success {
        return Err(format!("cover extraction failed: {}", first_line(&r.stderr)));
    }
    // Capped while reading, whatever size the file claims.
    let mut bytes = Vec::new();
    let file = tokio::fs::File::open(&out).await.map_err(|e| e.to_string())?;
    tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::AsyncReadExt::take(file, MAX_SOURCE as u64 + 1), &mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SOURCE {
        return Err("embedded cover too large".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageFormat;
    use std::io::Cursor;
    use std::time::Instant;

    const BOOK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// A data directory and a library of one folder holding `cover` as `cover.png`.
    struct Site {
        dir: PathBuf,
    }

    impl Site {
        fn new(name: &str, cover: &[u8]) -> Self {
            let dir = std::env::temp_dir().join(format!("abshare-covers-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("data")).expect("mkdir");
            std::fs::create_dir_all(dir.join("lib/Book")).expect("mkdir");
            std::fs::write(dir.join("lib/Book/cover.png"), cover).expect("write");
            Self { dir }
        }

        fn data(&self) -> PathBuf {
            self.dir.join("data")
        }

        fn source(&self) -> Source {
            Source::Sidecar { root: self.dir.join("lib"), rel: "Book/cover.png".into() }
        }

        fn covers(&self) -> PathBuf {
            self.data().join("covers").join(BOOK)
        }

        /// The job directories left behind.
        fn jobs(&self) -> Vec<PathBuf> {
            std::fs::read_dir(self.data().join("jobs"))
                .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
                .unwrap_or_default()
        }

        async fn make(
            &self,
            ff: &Ffmpeg,
            renderer: &Path,
            limits: &Limits,
            fingerprint: &str,
        ) -> Result<String, String> {
            make_with(ff, renderer, limits, &self.data(), BOOK, self.source(), fingerprint).await
        }
    }

    impl Drop for Site {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The real jail and the built `abshare` binary, or `None` where the jail can't run. With
    /// ABSHARE_REQUIRE_LANDLOCK set, not being able to run is a failure instead of a skip.
    async fn jail() -> Option<(Ffmpeg, &'static Path)> {
        let ff = crate::tools::test_ffmpeg().await?;
        let renderer = renderer().expect("renderer");
        if !renderer.exists() {
            assert!(
                std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_none(),
                "ABSHARE_REQUIRE_LANDLOCK is set but {} is not built",
                renderer.display()
            );
            return None;
        }
        Some((ff, renderer))
    }

    macro_rules! need_jail {
        () => {
            match jail().await {
                Some(j) => j,
                None => {
                    eprintln!("skipped: the jail cannot run here");
                    return;
                }
            }
        };
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 128]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).expect("png");
        out
    }

    /// One flat grey: a few kilobytes that inflate to `w` × `h` pixels.
    fn flat_png(w: u32, h: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let pixels = vec![0x80u8; w as usize * h as usize];
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, w, h, image::ExtendedColorType::L8)
            .expect("png");
        out
    }

    #[tokio::test]
    async fn a_cover_is_rendered_in_the_jail_at_every_size() {
        let (ff, renderer) = need_jail!();
        let site = Site::new("ok", &png(2400, 1200));
        let hash = site.make(&ff, renderer, &render_limits(), "sidecar:Book/cover.png:1:1").await.expect("cover");
        assert_eq!(hash.len(), 16);
        assert!(exists(&site.data(), BOOK, &hash));
        for size in SIZES {
            let jpeg = std::fs::read(site.covers().join(format!("{hash}-{size}.jpg"))).expect("thumbnail");
            let d = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg).expect("jpeg");
            assert_eq!((d.width(), d.height()), (size, size / 2));
        }
        assert_eq!(std::fs::read_dir(site.covers()).expect("dir").count(), SIZES.len(), "thumbnails and nothing else");
        assert!(site.jobs().is_empty(), "the job directory is removed: {:?}", site.jobs());
    }

    /// The limits the scanner sets are enough for the largest image the renderer accepts,
    /// in its most expensive form: 16 megapixels, 16 bits a channel, with transparency.
    #[tokio::test]
    async fn the_largest_cover_accepted_fits_the_limits() {
        let (ff, renderer) = need_jail!();
        let src = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(4_000, 4_000, |x, y| {
            image::Rgba([(x * 16) as u16, (y * 16) as u16, 40_000, 60_000])
        });
        let mut big = Vec::new();
        src.write_to(&mut Cursor::new(&mut big), ImageFormat::Png).expect("png");
        drop(src);
        let site = Site::new("largest", &[]);
        let thumbs = render_jailed(&ff, renderer, &render_limits(), &site.data(), &big).await.expect("render");
        assert_eq!(thumbs.iter().map(|(s, _)| *s).collect::<Vec<_>>(), SIZES);
        let d = image::load_from_memory_with_format(&thumbs[3].1, ImageFormat::Jpeg).expect("jpeg");
        assert_eq!((d.width(), d.height()), (1280, 1280));
        assert!(site.jobs().is_empty());
    }

    /// Hostile images make the renderer exit with an error. That is a problem for the book,
    /// in words, and nothing is left behind.
    #[tokio::test]
    async fn a_hostile_cover_is_a_problem_and_leaves_no_job_directory() {
        let (ff, renderer) = need_jail!();
        let mut truncated = png(64, 64);
        truncated.truncate(200);
        let cases: [(&str, Vec<u8>, &str); 4] = [
            ("garbage", b"\xff\xd8\xff\xe0 not a picture".to_vec(), "cover"),
            ("bomb", flat_png(10_000, 10_000), "cover too large"),
            ("truncated", truncated, "cover decode"),
            ("gif", b"GIF89a\x01\x00\x01\x00".to_vec(), "cover format"),
        ];
        for (name, bytes, want) in cases {
            let site = Site::new(name, &bytes);
            let e = site.make(&ff, renderer, &render_limits(), "fp").await.expect_err(name);
            assert!(e.starts_with(want), "{name}: {e}");
            assert!(site.jobs().is_empty(), "{name}: {:?}", site.jobs());
            assert!(
                !site.covers().join("x-320.jpg").exists()
                    && std::fs::read_dir(site.covers()).expect("dir").count() == 1
            );
        }
    }

    /// A renderer killed by a limit (here, too little memory even to decode) is a cover
    /// problem too, and the job directory still goes.
    #[tokio::test]
    async fn a_renderer_that_passes_a_limit_is_a_problem_and_leaves_no_job_directory() {
        let (ff, renderer) = need_jail!();
        let site = Site::new("limit", &[]);
        let tight = Limits { mem_mib: 16, ..render_limits() };
        let r = render_jailed(&ff, renderer, &tight, &site.data(), &png(2400, 2400)).await;
        assert!(matches!(r, Err(Failed::Cover(_))), "{r:?}");
        assert!(site.jobs().is_empty(), "{:?}", site.jobs());
        // The same image under the real limits is fine: it was the limit.
        assert!(render_jailed(&ff, renderer, &render_limits(), &site.data(), &png(2400, 2400)).await.is_ok());
    }

    #[tokio::test]
    async fn the_wall_clock_limit_stops_a_slow_render() {
        let (ff, renderer) = need_jail!();
        let site = Site::new("wall", &[]);
        let short = Limits { wall: Duration::from_millis(20), ..render_limits() };
        let started = Instant::now();
        let r = render_jailed(&ff, renderer, &short, &site.data(), &flat_png(4_000, 4_000)).await;
        assert!(matches!(&r, Err(Failed::Cover(e)) if e.contains("took more than")), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
        assert!(site.jobs().is_empty(), "{:?}", site.jobs());
    }

    /// The scanner does not decode: with a "renderer" that writes nothing (the jail's probe),
    /// a perfectly good image gets no cover. Were the image decoded in the scanner's own
    /// process again, this would succeed.
    #[tokio::test]
    async fn without_the_jailed_renderer_there_is_no_cover() {
        let (ff, _) = need_jail!();
        let probe = ff.jail.with_file_name("abshare-jail-probe");
        if !probe.exists() {
            assert!(std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_none(), "{} is not built", probe.display());
            return;
        }
        let site = Site::new("probe", &png(300, 300));
        let e = site.make(&ff, &probe, &render_limits(), "fp").await.expect_err("no cover");
        assert!(e.starts_with("cover rendering failed (status 4)"), "{e}");
        assert!(site.jobs().is_empty());
        // And a renderer that does not exist: the jail refuses, which is not held against
        // the image.
        let e = site.make(&ff, Path::new("/nonexistent/abshare"), &render_limits(), "other").await.expect_err("none");
        assert!(e.starts_with("the jail refused to run the cover renderer"), "{e}");
        assert!(site.jobs().is_empty());
    }

    /// The same structurally: image bytes are decoded only in `covers/render.rs`, which only
    /// the `render-cover` command (`covers/child.rs`) can reach.
    #[test]
    fn only_the_render_command_can_decode_an_image() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).expect("read_dir").map(|e| e.expect("entry").path()) {
                if e.is_dir() {
                    walk(&e, out);
                } else if e.extension().is_some_and(|x| x == "rs") {
                    out.push(e);
                }
            }
        }
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        assert!(files.len() > 10);
        // The code of a file, without its unit tests (which make and check images).
        let code = |p: &Path| {
            let text = std::fs::read_to_string(p).expect("read");
            text.split("#[cfg(test)]\nmod tests").next().unwrap_or("").to_owned()
        };
        for f in &files {
            let rel = f.strip_prefix(&src).expect("inside src").to_string_lossy().into_owned();
            if rel == "covers/render.rs" || rel.ends_with("tests.rs") {
                continue;
            }
            let text = code(f);
            for needle in ["image::", "use image", "load_from_memory"] {
                assert!(!text.contains(needle), "{rel} mentions `{needle}`: images are decoded only in the jail");
            }
            if rel != "covers/child.rs" {
                for needle in ["mod render", "render::render"] {
                    assert!(!text.contains(needle), "{rel} mentions `{needle}`: only the command has the decoder");
                }
            }
        }
        // The decoder is a private module of the command, and only the sizes leave it.
        let child = code(&src.join("covers/child.rs"));
        assert!(child.contains("\nmod render;\n") && !child.contains("pub mod render"));
        let exported: Vec<&str> = child.lines().filter(|l| l.contains("use render::")).collect();
        assert_eq!(exported, ["pub use render::SIZES;"]);
        // The scanner's side of the module names the command, not the decoder.
        let covers = code(&src.join("covers.rs"));
        assert!(covers.contains("child::COMMAND") && covers.contains("run_jailed_classified("));
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("abshare-covers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    /// What the renderer hands back is checked before it is kept.
    #[test]
    fn output_that_is_not_a_jpeg_is_refused() {
        let d = dir("collect");
        let jpeg = b"\xff\xd8\xff\xe0 pretend \xff\xd9".to_vec();
        let fill = |d: &Path| {
            for s in SIZES {
                std::fs::write(d.join(child::file_name(s)), &jpeg).expect("write");
            }
        };
        fill(&d);
        // Other files are ignored.
        std::fs::write(d.join("source"), b"anything").expect("write");
        std::fs::write(d.join("9999.jpg"), b"anything").expect("write");
        let got = collect(&d).expect("four thumbnails");
        assert_eq!(got.iter().map(|(s, _)| *s).collect::<Vec<_>>(), SIZES);
        assert!(got.iter().all(|(_, j)| *j == jpeg));

        let bad: [(&str, Vec<u8>); 6] = [
            ("a PNG", SAMPLE_PNG.to_vec()),
            ("HTML", b"<!doctype html><script>alert(1)</script>".to_vec()),
            ("nothing", Vec::new()),
            ("a JPEG start only", b"\xff\xd8\xff\xe0 no end".to_vec()),
            ("a JPEG end only", b"no start \xff\xd9".to_vec()),
            ("too large", [&b"\xff\xd8\xff"[..], &vec![0u8; MAX_THUMB as usize][..], &b"\xff\xd9"[..]].concat()),
        ];
        for (what, bytes) in &bad {
            fill(&d);
            std::fs::write(d.join("640.jpg"), bytes).expect("write");
            let e = collect(&d).expect_err(what);
            assert!(e.starts_with("cover renderer output refused: the 640 px thumbnail"), "{what}: {e}");
        }
        // Missing, a directory, and a link (even to a good thumbnail).
        fill(&d);
        std::fs::remove_file(d.join("128.jpg")).expect("rm");
        assert!(collect(&d).is_err_and(|e| e.contains("128 px thumbnail is missing")));
        std::fs::create_dir(d.join("128.jpg")).expect("mkdir");
        assert!(collect(&d).is_err_and(|e| e.contains("not a regular file")));
        std::fs::remove_dir(d.join("128.jpg")).expect("rmdir");
        std::os::unix::fs::symlink(d.join("320.jpg"), d.join("128.jpg")).expect("symlink");
        assert!(collect(&d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A cover that failed is not rendered again while its source is unchanged, and is
    /// tried again when the source changes. No jail is needed to see it: the "renderer" here
    /// would be refused if it were ever run.
    #[tokio::test]
    async fn a_failed_cover_is_remembered_until_its_source_changes() {
        let site = Site::new("remember", &png(64, 64));
        let covers = site.covers();
        assert_eq!(failed_before(&covers, "sidecar:Book/cover.png:10:20"), None);
        remember_failure(&covers, "sidecar:Book/cover.png:10:20", "cover decode: bad\x1b[31m line\nsecond line");
        assert_eq!(
            failed_before(&covers, "sidecar:Book/cover.png:10:20").as_deref(),
            Some("cover decode: bad[31m line")
        );
        assert_eq!(failed_before(&covers, "sidecar:Book/cover.png:11:20"), None, "another size");
        assert_eq!(failed_before(&covers, ""), None);

        // `make` answers from the mark: nothing is started (this jail does not exist).
        let none = Ffmpeg {
            ffmpeg: "/nonexistent/ffmpeg".into(),
            ffprobe: "/nonexistent/ffprobe".into(),
            jail: "/nonexistent/abshare-jail".into(),
            libs: Vec::new(),
            jobs: 1,
        };
        let limits = render_limits();
        let nowhere = Path::new("/nonexistent/abshare");
        let e = site.make(&none, nowhere, &limits, "sidecar:Book/cover.png:10:20").await.expect_err("remembered");
        assert_eq!(e, "cover decode: bad[31m line");
        // A changed source is tried, and a jail that can't start is not remembered.
        let e = site.make(&none, nowhere, &limits, "sidecar:Book/cover.png:11:20").await.expect_err("tried");
        assert!(e.starts_with("cannot start /nonexistent/abshare-jail"), "{e}");
        assert_eq!(failed_before(&covers, "sidecar:Book/cover.png:11:20"), None);
        assert!(site.jobs().is_empty(), "{:?}", site.jobs());
    }

    /// Through the jail: the failure is recorded by `make` itself, and a later good cover
    /// clears it.
    #[tokio::test]
    async fn a_failed_cover_is_not_rendered_again_on_the_next_scan() {
        let (ff, renderer) = need_jail!();
        let site = Site::new("again", b"\x89PNG\r\n\x1a\n nothing more");
        let limits = render_limits();
        let first = site.make(&ff, renderer, &limits, "v1").await.expect_err("bad image");
        assert!(site.covers().join(FAILED).is_file());
        // The second scan: the renderer is not run (this one does not exist) and the same
        // problem is reported.
        let second = site.make(&ff, Path::new("/nonexistent/abshare"), &limits, "v1").await.expect_err("remembered");
        assert_eq!(first, second);
        // The image is replaced: it is rendered, and the mark goes.
        std::fs::write(site.dir.join("lib/Book/cover.png"), png(200, 200)).expect("write");
        let hash = site.make(&ff, renderer, &limits, "v2").await.expect("cover");
        assert!(exists(&site.data(), BOOK, &hash) && !site.covers().join(FAILED).exists());
    }

    #[tokio::test]
    async fn the_self_test_renders_the_built_in_image() {
        let (ff, _) = need_jail!();
        let site = Site::new("selftest", &[]);
        let said = self_test(&ff, &site.data()).await.expect("self-test");
        assert_eq!(said, "a test image rendered in the jail (4 thumbnails)");
        assert!(site.jobs().is_empty());
    }

    #[test]
    fn diagnostics_are_cut_to_one_short_line_of_text() {
        assert_eq!(first_line("cover decode: bad\nmore"), "cover decode: bad");
        assert_eq!(first_line("a\tb\x07c\u{1b}[0m"), "abc[0m");
        assert_eq!(first_line(&"x".repeat(5000)).len(), 200);
        assert_eq!(first_line(""), "");
    }
}
