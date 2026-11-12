//! Walking a library root (SPEC §4.2 step 1): no symlinks, no other filesystems, no hidden
//! entries, UTF-8 names only, bounded depth and size. Records metadata only; never opens
//! audio files.
//!
//! Every directory is opened relative to a descriptor for the root, with no symbolic link
//! allowed anywhere in its path, and then listed and examined through that descriptor. A
//! directory swapped for a link after it was seen therefore can't bring names or sizes from
//! outside the root into the catalogue (review L17).

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, ResolveFlags};
use std::collections::BTreeMap;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

pub const MAX_DEPTH: usize = 8;
pub const MAX_FILES: usize = 200_000;
/// Most directory entries of any kind looked at in one walk: sidecars, images, skipped and
/// hidden entries count too, so a library of junk can't grow the walk without bound.
pub const MAX_ENTRIES: usize = 2_000_000;
/// Most skipped entries reported by name; the rest are only counted.
pub const MAX_SKIPPED: usize = 1_000;
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
    /// Entries skipped, with the reason: shown to admins. At most [`MAX_SKIPPED`].
    pub skipped: Vec<(String, &'static str)>,
    /// How many more entries were skipped than `skipped` lists.
    pub skipped_more: usize,
}

#[derive(Clone, Copy)]
struct Bounds {
    files: usize,
    entries: usize,
    skipped: usize,
}

const BOUNDS: Bounds = Bounds { files: MAX_FILES, entries: MAX_ENTRIES, skipped: MAX_SKIPPED };

struct State {
    w: Walk,
    /// Directories still to list: path relative to the root, and depth.
    stack: Vec<(String, usize)>,
    /// Entries looked at so far.
    entries: usize,
    /// The root's filesystem.
    dev: u64,
    bounds: Bounds,
}

impl State {
    fn skip(&mut self, rel: String, why: &'static str) {
        if self.w.skipped.len() < self.bounds.skipped {
            self.w.skipped.push((rel, why));
        } else {
            self.w.skipped_more += 1;
        }
    }
}

pub fn walk(root: &Path) -> Result<Walk, String> {
    walk_bounded(root, BOUNDS)
}

fn walk_bounded(root: &Path, bounds: Bounds) -> Result<Walk, String> {
    let (root_fd, dev) = open_root(root)?;
    let mut st = State { w: Walk::default(), stack: vec![(String::new(), 0)], entries: 0, dev, bounds };
    while let Some((rel_dir, depth)) = st.stack.pop() {
        list(root, &root_fd, &rel_dir, depth, &mut st)?;
    }
    st.w.audio.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(st.w)
}

fn open_root(root: &Path) -> Result<(OwnedFd, u64), String> {
    let err = |e: rustix::io::Errno| format!("{}: {e}", root.display());
    let fd =
        rustix::fs::open(root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty()).map_err(err)?;
    let dev = rustix::fs::fstat(&fd).map_err(err)?.st_dev;
    Ok((fd, dev))
}

/// Opens the directory `rel` inside the root for listing: resolved beneath the root's
/// descriptor in one system call that refuses a symbolic link in any component, so what is
/// opened is the directory inside the root or nothing. The filesystem is checked on the
/// opened descriptor, not on the path.
fn open_dir(root_fd: BorrowedFd<'_>, rel: &str, dev: u64) -> Result<OwnedFd, &'static str> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let how = ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS;
    let fd = rustix::fs::openat2(root_fd, rel, flags, Mode::empty(), how).map_err(|e| {
        if e == rustix::io::Errno::LOOP || e == rustix::io::Errno::NOTDIR || e == rustix::io::Errno::XDEV {
            "no longer a directory inside the library (not followed)"
        } else {
            "unreadable directory"
        }
    })?;
    match rustix::fs::fstat(&fd) {
        Ok(st) if st.st_dev == dev => Ok(fd),
        Ok(_) => Err("on another filesystem"),
        Err(_) => Err("unreadable directory"),
    }
}

/// Lists one directory (`rel_dir`, "" for the root itself) into `st`.
fn list(root: &Path, root_fd: &OwnedFd, rel_dir: &str, depth: usize, st: &mut State) -> Result<(), String> {
    let opened;
    let dir_fd = if rel_dir.is_empty() {
        root_fd.as_fd()
    } else {
        match open_dir(root_fd.as_fd(), rel_dir, st.dev) {
            Ok(fd) => {
                opened = fd;
                opened.as_fd()
            }
            Err(why) => {
                st.skip(rel_dir.to_owned(), why);
                return Ok(());
            }
        }
    };
    let Ok(entries) = Dir::read_from(dir_fd) else {
        st.skip(rel_dir.to_owned(), "unreadable directory");
        return Ok(());
    };
    for entry in entries {
        // A failed read ends this directory's listing; what was read so far is kept.
        let Ok(entry) = entry else { break };
        let name_c = entry.file_name();
        if matches!(name_c.to_bytes(), b"." | b"..") {
            continue;
        }
        st.entries += 1;
        if st.entries > st.bounds.entries {
            return Err(format!("more than {} entries under {}", st.bounds.entries, root.display()));
        }
        let Ok(name) = name_c.to_str() else {
            st.skip(format!("{rel_dir}/<non-UTF-8 name>"), "name is not valid UTF-8");
            continue;
        };
        let rel = if rel_dir.is_empty() { name.to_owned() } else { format!("{rel_dir}/{name}") };
        if name.starts_with('.') {
            continue;
        }
        if name.chars().any(char::is_control) {
            st.skip(rel, "control characters in name");
            continue;
        }
        // Looked up in the open directory, never following a link.
        let Ok(meta) = rustix::fs::statat(dir_fd, name_c, AtFlags::SYMLINK_NOFOLLOW) else { continue };
        let ft = FileType::from_raw_mode(meta.st_mode);
        if ft.is_symlink() {
            st.skip(rel, "symbolic link (not followed)");
            continue;
        }
        if meta.st_dev != st.dev {
            st.skip(rel, "on another filesystem");
            continue;
        }
        if ft.is_dir() {
            if depth + 1 > MAX_DEPTH {
                st.skip(rel, "deeper than 8 levels");
            } else {
                st.stack.push((rel, depth + 1));
            }
            continue;
        }
        if !ft.is_file() {
            st.skip(rel, "not a regular file");
            continue;
        }
        let size = u64::try_from(meta.st_size).unwrap_or(0);
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if AUDIO.contains(&ext.as_str()) {
            if st.w.audio.len() >= st.bounds.files {
                return Err(format!("more than {} audio files under {}", st.bounds.files, root.display()));
            }
            let mtime_ns = meta
                .st_mtime
                .saturating_mul(1_000_000_000)
                .saturating_add(i64::try_from(meta.st_mtime_nsec).unwrap_or(0));
            st.w.audio.push(Found { rel, size, mtime_ns, ext });
        } else if ext == "txt" && size <= MAX_SIDECAR_BYTES {
            st.w.extras.entry(rel_dir.to_owned()).or_default().texts.push(rel);
        } else if IMAGE.contains(&ext.as_str()) {
            st.w.extras.entry(rel_dir.to_owned()).or_default().images.push(rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("abshare-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn rels(w: &Walk) -> Vec<&str> {
        w.audio.iter().map(|f| f.rel.as_str()).collect()
    }

    fn why<'a>(w: &'a Walk, rel: &str) -> Option<&'a str> {
        w.skipped.iter().find(|(p, _)| p == rel).map(|(_, why)| *why)
    }

    /// A library beside a directory it must never see into.
    fn library_and_outside(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let base = tmp(name);
        let (lib, outside) = (base.join("lib"), base.join("outside"));
        std::fs::create_dir_all(lib.join("Book")).expect("mkdir");
        std::fs::write(lib.join("Book/01.mp3"), b"x").expect("w");
        std::fs::create_dir_all(outside.join("inner")).expect("mkdir");
        std::fs::write(outside.join("secret.mp3"), b"secret").expect("w");
        std::fs::write(outside.join("inner/secret.mp3"), b"secret").expect("w");
        (base, lib, outside)
    }

    #[test]
    fn a_symlinked_directory_inside_the_library_is_not_followed() {
        let (base, lib, outside) = library_and_outside("linkdir");
        symlink(&outside, lib.join("Elsewhere")).expect("link");
        // A link to a directory that is inside the library isn't followed either: the book
        // would be catalogued twice.
        symlink(lib.join("Book"), lib.join("Again")).expect("link");
        symlink("Book", lib.join("Relative")).expect("link");
        let w = walk(&lib).expect("walk");
        assert_eq!(rels(&w), ["Book/01.mp3"]);
        for link in ["Elsewhere", "Again", "Relative"] {
            assert_eq!(why(&w, link), Some("symbolic link (not followed)"), "{link}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Review L17: a directory that was a real directory when it was seen, and a symbolic
    /// link by the time it is listed. Listing it by path would read the link's target.
    #[test]
    fn a_directory_swapped_for_a_symlink_before_it_is_listed_is_refused() {
        let (base, lib, outside) = library_and_outside("swap");
        std::fs::create_dir_all(lib.join("Deep/inner")).expect("mkdir");
        let (root_fd, dev) = open_root(&lib).expect("root");
        let listing = |rel: &str| {
            let mut st = State { w: Walk::default(), stack: Vec::new(), entries: 0, dev, bounds: BOUNDS };
            list(&lib, &root_fd, rel, 1, &mut st).expect("list");
            st
        };
        // Before the swap, both directories list normally.
        assert_eq!(rels(&listing("Book").w), ["Book/01.mp3"]);
        assert!(listing("Deep/inner").w.skipped.is_empty());

        // The directory itself becomes a link out of the library...
        std::fs::rename(lib.join("Book"), lib.join("Book.real")).expect("rename");
        symlink(&outside, lib.join("Book")).expect("link");
        // ...and so does a parent of a directory waiting to be listed.
        std::fs::remove_dir_all(lib.join("Deep")).expect("rm");
        symlink(&outside, lib.join("Deep")).expect("link");
        for rel in ["Book", "Deep/inner"] {
            let st = listing(rel);
            assert!(st.w.audio.is_empty(), "{rel}: {:?}", st.w.audio);
            assert!(st.w.extras.is_empty() && st.stack.is_empty(), "{rel}");
            assert_eq!(why(&st.w, rel), Some("no longer a directory inside the library (not followed)"), "{rel}");
        }
        // A link that stays inside the library is refused just the same.
        std::fs::remove_file(lib.join("Book")).expect("rm");
        symlink("Book.real", lib.join("Book")).expect("link");
        assert!(listing("Book").w.audio.is_empty());
        // And `..` can't lead out.
        assert!(open_dir(root_fd.as_fd(), "../outside", dev).is_err());
        assert!(open_dir(root_fd.as_fd(), "Book.real/../../outside", dev).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fifos_sockets_and_other_special_files_are_skipped_without_being_opened() {
        let d = tmp("special");
        rustix::fs::mknodat(rustix::fs::CWD, d.join("pipe.mp3"), FileType::Fifo, Mode::from_raw_mode(0o600), 0)
            .expect("mkfifo");
        let _listener = std::os::unix::net::UnixListener::bind(d.join("sock.m4b")).expect("bind");
        std::fs::write(d.join("real.mp3"), b"x").expect("w");
        // Opening a FIFO with no writer would block for ever; the walk never opens anything.
        let w = walk(&d).expect("walk");
        assert_eq!(rels(&w), ["real.mp3"]);
        assert_eq!(why(&w, "pipe.mp3"), Some("not a regular file"));
        assert_eq!(why(&w, "sock.m4b"), Some("not a regular file"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_that_are_not_utf8_or_hold_control_characters_are_skipped() {
        let d = tmp("badnames");
        std::fs::create_dir_all(d.join("Book")).expect("mkdir");
        std::fs::write(d.join("Book").join(std::ffi::OsStr::from_bytes(b"bad\xff.mp3")), b"x").expect("w");
        std::fs::create_dir_all(d.join(std::ffi::OsStr::from_bytes(b"dir\xfe"))).expect("mkdir");
        std::fs::write(d.join(std::ffi::OsStr::from_bytes(b"dir\xfe")).join("inside.mp3"), b"x").expect("w");
        for name in ["line\nbreak.mp3", "bell\u{7}.mp3", "escape\u{1b}[31m.mp3", "del\u{7f}.m4b", "c1\u{85}.mp3"] {
            std::fs::write(d.join(name), b"x").expect("w");
        }
        std::fs::create_dir_all(d.join("tab\tdir")).expect("mkdir");
        std::fs::write(d.join("tab\tdir/inside.mp3"), b"x").expect("w");
        std::fs::write(d.join("fine.mp3"), b"x").expect("w");
        let w = walk(&d).expect("walk");
        assert_eq!(rels(&w), ["fine.mp3"]);
        assert_eq!(why(&w, "Book/<non-UTF-8 name>"), Some("name is not valid UTF-8"));
        assert_eq!(why(&w, "/<non-UTF-8 name>"), Some("name is not valid UTF-8"));
        for name in
            ["line\nbreak.mp3", "bell\u{7}.mp3", "escape\u{1b}[31m.mp3", "del\u{7f}.m4b", "c1\u{85}.mp3", "tab\tdir"]
        {
            assert_eq!(why(&w, name), Some("control characters in name"), "{name:?}");
        }
        assert_eq!(w.skipped.len(), 8, "{:?}", w.skipped);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_number_of_audio_files_is_bounded() {
        let d = tmp("maxfiles");
        std::fs::create_dir_all(d.join("sub")).expect("mkdir");
        for i in 0..3 {
            std::fs::write(d.join(format!("{i}.mp3")), b"x").expect("w");
        }
        let bounds = Bounds { files: 3, ..BOUNDS };
        assert_eq!(walk_bounded(&d, bounds).expect("at the limit").audio.len(), 3);
        std::fs::write(d.join("sub/one-more.mp3"), b"x").expect("w");
        let e = walk_bounded(&d, bounds).expect_err("over the limit");
        assert!(e.starts_with("more than 3 audio files under "), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Review L18: only audio files used to count, so anything else grew without bound.
    #[test]
    fn the_number_of_entries_of_any_kind_is_bounded() {
        let d = tmp("maxentries");
        std::fs::create_dir_all(d.join("sub")).expect("mkdir");
        std::fs::write(d.join("a.mp3"), b"x").expect("w");
        std::fs::write(d.join("cover.jpg"), b"x").expect("w");
        std::fs::write(d.join("notes.txt"), b"x").expect("w");
        std::fs::write(d.join(".hidden"), b"x").expect("w");
        symlink("a.mp3", d.join("link.mp3")).expect("link");
        // Six entries: the directory, the audio file, two sidecars, a hidden file, a link.
        let bounds = Bounds { entries: 6, ..BOUNDS };
        assert_eq!(walk_bounded(&d, bounds).expect("at the limit").audio.len(), 1);
        std::fs::write(d.join("sub/readme.pdf"), b"x").expect("w");
        let e = walk_bounded(&d, bounds).expect_err("over the limit");
        assert!(e.starts_with("more than 6 entries under "), "{e}");
        assert!(walk(&d).is_ok(), "well inside the real limit");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Review L18: the rows kept about skipped entries are bounded; the rest are counted.
    #[test]
    fn skipped_entries_are_listed_up_to_a_limit_and_counted_beyond_it() {
        let d = tmp("maxskipped");
        for i in 0..5 {
            symlink("/etc/passwd", d.join(format!("link{i}.mp3"))).expect("link");
        }
        std::fs::write(d.join("real.mp3"), b"x").expect("w");
        let w = walk_bounded(&d, Bounds { skipped: 2, ..BOUNDS }).expect("walk");
        assert_eq!((w.skipped.len(), w.skipped_more), (2, 3));
        assert_eq!(rels(&w), ["real.mp3"]);
        let w = walk(&d).expect("walk");
        assert_eq!((w.skipped.len(), w.skipped_more), (5, 0));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sizes_and_times_are_recorded_and_a_missing_root_is_an_error() {
        let d = tmp("meta");
        std::fs::write(d.join("a.mp3"), b"12345").expect("w");
        std::fs::write(d.join("big.txt"), vec![b'x'; MAX_SIDECAR_BYTES as usize + 1]).expect("w");
        std::fs::write(d.join("small.txt"), b"x").expect("w");
        let w = walk(&d).expect("walk");
        let m = std::fs::metadata(d.join("a.mp3")).expect("stat");
        use std::os::unix::fs::MetadataExt;
        assert_eq!(w.audio[0].size, 5);
        assert_eq!(w.audio[0].mtime_ns, m.mtime() * 1_000_000_000 + m.mtime_nsec());
        assert_eq!(w.extras[""].texts, ["small.txt"]);
        assert!(walk(&d.join("missing")).is_err());
        assert!(walk(&d.join("a.mp3")).is_err(), "a file is not a library root");
        let _ = std::fs::remove_dir_all(&d);
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
