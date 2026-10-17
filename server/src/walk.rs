//! Walking a library root (SPEC §4.2 step 1): no symlinks, no other filesystems, no hidden
//! entries, UTF-8 names only, bounded depth and size. Records metadata only; never opens
//! audio files.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub const MAX_DEPTH: usize = 8;
pub const MAX_FILES: usize = 200_000;
const MAX_SIDECAR_BYTES: u64 = 64 * 1024;

const AUDIO: &[&str] = &["m4b", "m4a", "mp4", "mp3", "ogg", "oga", "opus", "flac", "wav", "aac", "aax", "aa"];
const IMAGE: &[&str] = &["jpg", "jpeg", "png"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub rel: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub ext: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirExtras {
    /// `.txt` files (relative paths) under 64 KiB.
    pub texts: Vec<String>,
    /// Image files (relative paths), candidates for a cover.
    pub images: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Walk {
    pub audio: Vec<Found>,
    pub extras: BTreeMap<String, DirExtras>,
    /// Entries skipped, with the reason: shown to admins.
    pub skipped: Vec<(String, &'static str)>,
}

pub fn walk(root: &Path) -> Result<Walk, String> {
    let root_meta = std::fs::metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let dev = root_meta.dev();
    let mut w = Walk::default();
    let mut stack: Vec<(std::path::PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel_dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                w.skipped.push((rel_dir.clone(), "unreadable directory"));
                continue;
            }
        };
        for entry in entries.flatten() {
            let name_os = entry.file_name();
            let Some(name) = name_os.to_str() else {
                w.skipped.push((format!("{rel_dir}/<non-UTF-8 name>"), "name is not valid UTF-8"));
                continue;
            };
            let rel = if rel_dir.is_empty() { name.to_owned() } else { format!("{rel_dir}/{name}") };
            if name.starts_with('.') {
                continue;
            }
            if name.chars().any(char::is_control) {
                w.skipped.push((rel, "control characters in name"));
                continue;
            }
            // symlink_metadata: never follow links.
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            let ft = meta.file_type();
            if ft.is_symlink() {
                w.skipped.push((rel, "symbolic link (not followed)"));
                continue;
            }
            if meta.dev() != dev {
                w.skipped.push((rel, "on another filesystem"));
                continue;
            }
            if ft.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    w.skipped.push((rel, "deeper than 8 levels"));
                } else {
                    stack.push((entry.path(), rel, depth + 1));
                }
                continue;
            }
            if !ft.is_file() {
                w.skipped.push((rel, "not a regular file"));
                continue;
            }
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            if AUDIO.contains(&ext.as_str()) {
                if w.audio.len() >= MAX_FILES {
                    return Err(format!("more than {MAX_FILES} audio files under {}", root.display()));
                }
                w.audio.push(Found {
                    rel,
                    size: meta.size(),
                    mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
                    ext,
                });
            } else if ext == "txt" && meta.size() <= MAX_SIDECAR_BYTES {
                w.extras.entry(rel_dir.clone()).or_default().texts.push(rel);
            } else if IMAGE.contains(&ext.as_str()) {
                w.extras.entry(rel_dir.clone()).or_default().images.push(rel);
            }
        }
    }
    w.audio.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("abshare-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn audio_sidecars_hidden_and_links_are_handled() {
        let d = tmp("basic");
        std::fs::create_dir_all(d.join("Book [B0AAAAAAA1]/CD1")).expect("mkdir");
        std::fs::write(d.join("Book [B0AAAAAAA1]/CD1/01.mp3"), b"x").expect("w");
        std::fs::write(d.join("Book [B0AAAAAAA1]/cover.jpg"), b"x").expect("w");
        std::fs::write(d.join("Book [B0AAAAAAA1]/notes.txt"), b"Read by Someone").expect("w");
        std::fs::write(d.join("loose.m4b"), b"x").expect("w");
        std::fs::write(d.join(".hidden.mp3"), b"x").expect("w");
        std::fs::write(d.join("readme.pdf"), b"x").expect("w");
        std::os::unix::fs::symlink("/etc/passwd", d.join("escape.mp3")).expect("link");
        std::os::unix::fs::symlink("/etc", d.join("etcdir")).expect("link");
        let w = walk(&d).expect("walk");
        let rels: Vec<&str> = w.audio.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, ["Book [B0AAAAAAA1]/CD1/01.mp3", "loose.m4b"]);
        assert_eq!(w.extras["Book [B0AAAAAAA1]"].images, ["Book [B0AAAAAAA1]/cover.jpg"]);
        assert_eq!(w.extras["Book [B0AAAAAAA1]"].texts, ["Book [B0AAAAAAA1]/notes.txt"]);
        let skipped: Vec<&str> = w.skipped.iter().map(|(p, _)| p.as_str()).collect();
        assert!(skipped.contains(&"escape.mp3") && skipped.contains(&"etcdir"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn depth_is_bounded() {
        let d = tmp("deep");
        let mut p = d.clone();
        for i in 0..11 {
            p = p.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&p).expect("mkdir");
        std::fs::write(p.join("deep.mp3"), b"x").expect("w");
        let w = walk(&d).expect("walk");
        assert!(w.audio.is_empty());
        assert!(w.skipped.iter().any(|(_, why)| *why == "deeper than 8 levels"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unicode_colons_and_brackets_are_kept_as_they_are() {
        let d = tmp("names");
        let name = "Writer’s Tale: Book 1 (Opens 10∕31!) [B0BBBBBBB2]";
        std::fs::create_dir_all(d.join(name)).expect("mkdir");
        std::fs::write(d.join(name).join("x.m4b"), b"x").expect("w");
        let w = walk(&d).expect("walk");
        assert_eq!(w.audio[0].rel, format!("{name}/x.m4b"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
