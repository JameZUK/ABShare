//! Covers (SPEC §4.6): the source image is extracted by jailed ffmpeg (embedded) or read
//! from a sidecar, then decoded in Rust with limits (JPEG and PNG only) and re-encoded as
//! JPEG thumbnails. Re-encoding strips all metadata and malformed-image tricks. The decoding
//! itself is in `covers/render.rs`, which is fuzzed.

mod render;

use crate::config::Ffmpeg;
use crate::tools::{Limits, demuxer_for, run_jailed};
pub use render::{SIZES, render};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_SOURCE: usize = 20 * 1024 * 1024;

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

/// Makes the thumbnails for one book. Returns the cover hash (part of each thumbnail's
/// immutable URL). Thumbnails that already exist for the same source are reused.
pub async fn make(ff: &Ffmpeg, data_dir: &Path, book_id: &str, src: Source) -> Result<String, String> {
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
    let dir = data_dir.join("covers").join(book_id);
    if dir.join(format!("{hash}-320.jpg")).exists() {
        return Ok(hash);
    }
    // On a blocking thread because rendering is slow, not to contain it: a release build
    // aborts on panic, so a panic in the decoder is never seen here as an error. What
    // protects the scanner is that `render` is fuzzed and refuses large images before
    // decoding them. The join fails only when the task is cancelled (or, in a build that
    // unwinds, such as the tests, when it panicked).
    let thumbs = tokio::task::spawn_blocking(move || render(&bytes))
        .await
        .map_err(|_| "cover rendering did not finish".to_string())??;
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    for (size, jpeg) in thumbs {
        let path = dir.join(format!("{hash}-{size}.jpg"));
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, &jpeg).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
    }
    // Drop thumbnails of earlier covers for this book.
    if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if !e.file_name().to_string_lossy().starts_with(&hash) {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    Ok(hash)
}

async fn extract(ff: &Ffmpeg, data_dir: &Path, file: &Path, ext: &str) -> Result<Vec<u8>, String> {
    let demuxer = demuxer_for(ext).ok_or("unsupported container")?;
    let mut rnd = [0u8; 8];
    getrandom::fill(&mut rnd).map_err(|e| e.to_string())?;
    let job =
        data_dir.join("jobs").join(format!("cover-{}", rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    tokio::fs::create_dir_all(&job).await.map_err(|e| e.to_string())?;
    let out = job.join("cover.img");
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
    let result = run_jailed(ff, &[file], Some(&job), &ff.ffmpeg, &args, &limits).await;
    let bytes = tokio::fs::read(&out).await;
    let _ = tokio::fs::remove_dir_all(&job).await;
    let r = result?;
    if !r.success {
        return Err(format!("cover extraction failed: {}", r.stderr.lines().next().unwrap_or("")));
    }
    let bytes = bytes.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SOURCE {
        return Err("embedded cover too large".into());
    }
    Ok(bytes)
}
