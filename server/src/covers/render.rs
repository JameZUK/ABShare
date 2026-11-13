//! Decoding a cover image and rendering its thumbnails. Pure: bytes in, JPEG bytes out.
//!
//! The image is untrusted. In the server this code runs only in the `render-cover` command
//! (`child.rs`), which the scanner runs inside the jail: a panic, or a decoder that runs away
//! with memory or time, ends that one process and costs one book its cover. It is still held
//! to never panicking and to bounded memory: this file depends on nothing but the `image`
//! crate, so the fuzz crate builds it as it is (`fuzz/fuzz_targets/cover_render.rs`), and the
//! size is checked from the header before anything is decoded.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::Cursor;

pub const SIZES: [u32; 4] = [128, 320, 640, 1280];
/// The longest side accepted, in pixels.
pub const MAX_SIDE: u32 = 6_000;
/// The most pixels accepted (4,000 × 4,000): far more than a cover needs, and it bounds the
/// decoded image to 128 MiB even at 16 bits a channel with alpha.
pub const MAX_PIXELS: u64 = 16_000_000;
/// The most the decoder may allocate for one image.
const MAX_ALLOC: u64 = 192 * 1024 * 1024;

/// Whether an image of this size is accepted at all.
fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("cover has no pixels".into());
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "cover too large: {width} x {height} pixels (at most {MAX_SIDE} a side and {MAX_PIXELS} in all)"
        ));
    }
    Ok(())
}

/// Decodes (JPEG/PNG only, with size and memory limits) and renders every thumbnail.
pub fn render(bytes: &[u8]) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let open = || ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string());
    let header = open()?;
    match header.format() {
        Some(ImageFormat::Jpeg | ImageFormat::Png) => {}
        other => return Err(format!("cover format {other:?} not accepted")),
    }
    // The size comes from the header alone: an image that is too large is refused before
    // any of its pixels are decompressed.
    let (width, height) = header.into_dimensions().map_err(|e| format!("cover header: {e}"))?;
    check_size(width, height)?;

    let mut reader = open()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("cover decode: {e}"))?;
    // What was decoded is what the header promised.
    check_size(img.width(), img.height())?;
    // Eight bits a channel and no alpha from here on: all a JPEG thumbnail keeps.
    let img = DynamicImage::ImageRgb8(img.into_rgb8());

    // The full-size image is resampled once, for the largest thumbnail; the smaller ones are
    // made from that. A cover no larger than a thumbnail is not scaled up.
    let largest = SIZES.iter().copied().max().unwrap_or(0);
    let base =
        if img.width().max(img.height()) > largest { img.resize(largest, largest, FilterType::Lanczos3) } else { img };
    let mut out = Vec::new();
    for size in SIZES {
        let scaled;
        let thumb = if base.width().max(base.height()) > size {
            scaled = base.resize(size, size, FilterType::Lanczos3);
            &scaled
        } else {
            &base
        };
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 85).encode_image(thumb).map_err(|e| e.to_string())?;
        out.push((size, jpeg));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use image::codecs::png::PngEncoder;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 128]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).expect("png");
        out
    }

    /// A real PNG of one flat grey: a few kilobytes that inflate to `w` × `h` pixels.
    fn flat_png(w: u32, h: u32) -> Vec<u8> {
        let pixels = vec![0x80u8; w as usize * h as usize];
        let mut out = Vec::new();
        PngEncoder::new(&mut out).write_image(&pixels, w, h, image::ExtendedColorType::L8).expect("png");
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
    fn the_shape_is_kept_and_jpeg_sources_are_accepted() {
        let src = image::RgbImage::from_fn(3000, 1500, |x, _| image::Rgb([(x % 255) as u8, 10, 200]));
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&src).expect("jpeg");
        let t = render(&jpeg).expect("render");
        for (size, thumb) in &t {
            let d = image::load_from_memory_with_format(thumb, ImageFormat::Jpeg).expect("jpeg");
            assert_eq!((d.width(), d.height()), (*size, *size / 2));
        }
    }

    #[test]
    fn deep_colour_and_transparency_are_flattened() {
        let src = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(400, 300, |x, y| {
            image::Rgba([(x * 100) as u16, (y * 100) as u16, 40_000, 30_000])
        });
        let mut png = Vec::new();
        src.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).expect("png");
        let t = render(&png).expect("render");
        let d = image::load_from_memory_with_format(&t[1].1, ImageFormat::Jpeg).expect("jpeg");
        assert_eq!((d.width(), d.height()), (320, 240));
        assert_eq!(d.color(), image::ColorType::Rgb8);
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
        // A PNG cut short anywhere never panics, and is an error wherever pixels are missing
        // (only the closing chunk may be absent).
        let whole = png(64, 64);
        for cut in 0..whole.len() {
            let r = render(&whole[..cut]);
            assert!(r.is_err() || cut >= whole.len() - 12, "cut at {cut}");
        }
    }

    #[test]
    fn a_decompression_bomb_header_is_refused() {
        // A PNG header claiming 100000 x 100000 pixels.
        let mut p = png(1, 1);
        p[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        p[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(render(&p).is_err());
    }

    #[test]
    fn the_size_limits_are_exact() {
        assert!(check_size(4_000, 4_000).is_ok());
        assert!(check_size(MAX_SIDE, 2_666).is_ok());
        assert!(check_size(1, 1).is_ok());
        for (w, h) in [(4_001, 4_000), (4_000, 4_001), (MAX_SIDE + 1, 1), (1, MAX_SIDE + 1), (MAX_SIDE, 2_667)] {
            assert!(check_size(w, h).is_err(), "{w} x {h}");
        }
        assert!(check_size(0, 10).is_err() && check_size(10, 0).is_err());
        assert!(check_size(u32::MAX, u32::MAX).is_err(), "no overflow");
    }

    /// The file up to the first few bytes of its pixel data: the whole header, no pixels.
    fn header_only(png: &[u8]) -> &[u8] {
        let idat = png.windows(4).position(|w| w == b"IDAT").expect("IDAT");
        &png[..idat + 12]
    }

    /// Review M5: real decompression bombs, small files that inflate to more pixels than a
    /// cover may have. They are refused from the header, before anything is inflated: the
    /// same files cut off where their pixels begin are refused for their size too, not for
    /// being cut off. (A 10,000 × 10,000 image used to be accepted and decoded.)
    #[test]
    fn a_decompression_bomb_just_over_the_limit_is_refused_before_decoding() {
        for (w, h) in [(4_001, 4_000), (MAX_SIDE + 1, 16), (16, MAX_SIDE + 1), (10_000, 10_000)] {
            let bomb = flat_png(w, h);
            assert!(bomb.len() < 1024 * 1024, "{w} x {h} is {} bytes", bomb.len());
            let e = render(&bomb).expect_err("refused");
            assert!(e.starts_with("cover too large"), "{w} x {h}: {e}");
            let e = render(header_only(&bomb)).expect_err("refused");
            assert!(e.starts_with("cover too large"), "{w} x {h}, header only: {e}");
        }
        // The header check is what refuses them: a file of an allowed size cut off in the
        // same place gets as far as the decoder.
        let e = render(header_only(&flat_png(64, 64))).expect_err("cut short");
        assert!(e.starts_with("cover decode"), "{e}");
    }

    /// The largest image accepted really is rendered, in bounded memory and time.
    #[test]
    fn a_cover_at_the_size_limit_is_rendered() {
        let started = std::time::Instant::now();
        let t = render(&flat_png(4_000, 4_000)).expect("render");
        assert_eq!(t.len(), SIZES.len());
        let d = image::load_from_memory_with_format(&t[3].1, ImageFormat::Jpeg).expect("jpeg");
        assert_eq!((d.width(), d.height()), (1280, 1280));
        assert!(started.elapsed() < std::time::Duration::from_secs(120), "took {:?}", started.elapsed());
    }
}
