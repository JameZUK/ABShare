//! `abshare render-cover INPUT OUTPUT-DIR`: the cover renderer, a hidden command of the
//! server binary that the scanner runs only inside `abshare-jail`.
//!
//! It reads one image file, decodes it with the limits in `render.rs`, and writes one JPEG
//! for each thumbnail size into the output directory, under fixed names. It does nothing
//! else: no configuration, no database, no network, no async runtime. A cover that is refused
//! is one line on standard error and exit status 1.
//!
//! The decoder (`render`) is private to this module, so nothing else in the server can call
//! it: image bytes are decoded only in this command (and in the fuzz target, which builds
//! `render.rs` by itself).

// Beside this file, not in `child/`: the fuzz crate includes the same file by path.
#[path = "render.rs"]
mod render;

pub use render::SIZES;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

/// The command's name, the first argument of the server binary.
pub const COMMAND: &str = "render-cover";
/// The largest source image read, in bytes.
pub const MAX_SOURCE: u64 = 20 * 1024 * 1024;

/// The name of the thumbnail of this size in the output directory.
pub fn file_name(size: u32) -> String {
    format!("{size}.jpg")
}

/// Runs the command if `args` (the process's own, program name first) ask for it; `None`
/// when they ask for something else.
pub fn command(mut args: impl Iterator<Item = OsString>) -> Option<ExitCode> {
    let _program = args.next();
    if args.next()? != COMMAND {
        return None;
    }
    let (Some(input), Some(out), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: abshare {COMMAND} INPUT OUTPUT-DIR");
        return Some(ExitCode::from(2));
    };
    Some(match run(Path::new(&input), Path::new(&out)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    })
}

fn run(input: &Path, out: &Path) -> Result<(), String> {
    let mut bytes = Vec::new();
    std::fs::File::open(input)
        .and_then(|f| f.take(MAX_SOURCE + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("cover source: {e}"))?;
    if bytes.len() as u64 > MAX_SOURCE {
        return Err("cover source too large".into());
    }
    for (size, jpeg) in render::render(&bytes)? {
        // A new file each time: nothing already in the directory is written through.
        std::fs::File::create_new(out.join(file_name(size)))
            .and_then(|mut f| f.write_all(&jpeg))
            .map_err(|e| format!("cover output: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("abshare-render-cover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 120, 200]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).expect("png");
        out
    }

    fn args(a: &[&str]) -> impl Iterator<Item = OsString> {
        a.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn only_its_own_name_selects_the_command() {
        for other in [&["abshare"][..], &["abshare", "serve", "--config", "x"], &["abshare", "scan", COMMAND], &[]] {
            assert!(command(args(other)).is_none(), "{other:?}");
        }
        // Selected, whatever follows (here: too few and too many paths, so nothing is read).
        assert!(command(args(&["abshare", COMMAND])).is_some());
        assert!(command(args(&["abshare", COMMAND, "/nonexistent/a", "/nonexistent/b", "extra"])).is_some());
    }

    #[test]
    fn one_jpeg_for_each_size_under_fixed_names() {
        let d = dir("ok");
        std::fs::write(d.join("source"), png(300, 150)).expect("write");
        run(&d.join("source"), &d).expect("render");
        let mut names: Vec<String> = std::fs::read_dir(&d)
            .expect("dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into())
            .collect();
        names.sort();
        assert_eq!(names, ["128.jpg", "1280.jpg", "320.jpg", "640.jpg", "source"]);
        for size in SIZES {
            let jpeg = std::fs::read(d.join(file_name(size))).expect("read");
            assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) && jpeg.ends_with(&[0xFF, 0xD9]), "{size}");
        }
        // A second run does not write through what is there.
        assert!(run(&d.join("source"), &d).is_err_and(|e| e.starts_with("cover output")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_bad_or_missing_source_is_an_error_and_writes_nothing() {
        let d = dir("bad");
        assert!(run(&d.join("missing"), &d).is_err_and(|e| e.starts_with("cover source")));
        std::fs::write(d.join("source"), b"GIF89a\x01\x00\x01\x00").expect("write");
        assert!(run(&d.join("source"), &d).is_err_and(|e| e.starts_with("cover format")));
        std::fs::write(d.join("source"), vec![0u8; MAX_SOURCE as usize + 1]).expect("write");
        assert_eq!(run(&d.join("source"), &d).err().as_deref(), Some("cover source too large"));
        assert_eq!(std::fs::read_dir(&d).expect("dir").count(), 1, "only the source");
        let _ = std::fs::remove_dir_all(&d);
    }
}
