//! Covers (SPEC §4.6): the source image is extracted by jailed ffmpeg (embedded) or read
//! from a sidecar, then decoded in Rust with limits (JPEG and PNG only) and re-encoded as
//! JPEG thumbnails. Re-encoding strips all metadata and malformed-image tricks.

use crate::config::Ffmpeg;
use crate::tools::{Limits, demuxer_for, run_jailed};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SIZES: [u32; 4] = [128, 320, 640, 1280];
const MAX_SOURCE: usize = 20 * 1024 * 1024;

/// Whether the thumbnails for `hash` are on disk.
pub fn exists(data_dir: &Path, book_id: &str, hash: &str) -> bool {
    SIZES.iter().all(|s| data_dir.join("covers").join(book_id).join(format!("{hash}-{s}.jpg")).exists())
}

pub enum Source {
    Embedded { file: PathBuf, ext: String },
    Sidecar(PathBuf),
}

/// Makes the thumbnails for one book. Returns the cover hash (part of each thumbnail's
/// immutable URL). Thumbnails that already exist for the same source are reused.
pub async fn make(ff: &Ffmpeg, data_dir: &Path, book_id: &str, src: Source) -> Result<String, String> {
    let bytes = match src {
        Source::Sidecar(p) => {
            let meta = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() as usize > MAX_SOURCE {
                return Err("cover image missing or too large".into());
            }
            tokio::fs::read(&p).await.map_err(|e| e.to_string())?
        }
        Source::Embedded { file, ext } => extract(ff, data_dir, &file, &ext).await?,
    };
    let hash: String = Sha256::digest(&bytes).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let dir = data_dir.join("covers").join(book_id);
    if dir.join(format!("{hash}-320.jpg")).exists() {
        return Ok(hash);
    }
    let thumbs = tokio::task::spawn_blocking(move || render(&bytes))
        .await
        .map_err(|_| "cover decoder crashed".to_string())??;
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

/// Decodes (JPEG/PNG only, with size and memory limits) and renders every thumbnail.
pub fn render(bytes: &[u8]) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    match reader.format() {
        Some(ImageFormat::Jpeg | ImageFormat::Png) => {}
        other => return Err(format!("cover format {other:?} not accepted")),
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(10_000);
    limits.max_image_height = Some(10_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("cover decode: {e}"))?;
    let mut out = Vec::new();
    for size in SIZES {
        let thumb = if img.width().max(img.height()) > size {
            img.resize(size, size, FilterType::Lanczos3)
        } else {
            img.clone()
        };
        let rgb = thumb.to_rgb8();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 85).encode_image(&rgb).map_err(|e| e.to_string())?;
        out.push((size, jpeg));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 128]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).expect("png");
        out
    }

    #[test]
    fn thumbnails_are_rendered_at_every_size_as_jpeg() {
        let t = render(&png(2400, 2400)).expect("render");
        assert_eq!(t.iter().map(|(s, _)| *s).collect::<Vec<_>>(), SIZES);
        for (size, jpeg) in &t {
            let d = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg).expect("jpeg");
            assert_eq!(d.width().max(d.height()), *size);
        }
    }

    #[test]
    fn small_covers_are_not_upscaled() {
        let t = render(&png(100, 80)).expect("render");
        let d = image::load_from_memory(&t[3].1).expect("jpeg");
        assert_eq!((d.width(), d.height()), (100, 80));
    }

    #[test]
    fn other_formats_and_garbage_are_refused() {
        assert!(render(b"GIF89a\x01\x00\x01\x00").is_err());
        assert!(render(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
        assert!(render(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]).is_err());
        assert!(render(&[]).is_err());
    }

    #[test]
    fn a_decompression_bomb_header_is_refused() {
        // A PNG header claiming 100000 x 100000 pixels.
        let mut p = png(1, 1);
        p[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        p[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(render(&p).is_err());
    }
}
