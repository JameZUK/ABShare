//! `abshare.toml` (SPEC §5.7). Strict: unknown keys are an error, and the server refuses to
//! start on unsafe permissions.

use serde::Deserialize;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    pub data_dir: PathBuf,
    pub public_origin: String,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    /// The built PWA (`web/app/build`). Without it only the API is served.
    #[serde(default)]
    pub web_dir: Option<PathBuf>,
    pub ffmpeg: Ffmpeg,
    #[serde(default)]
    pub cache: Cache,
    #[serde(rename = "library")]
    pub libraries: Vec<Library>,
}

fn default_listen() -> String {
    "127.0.0.1:8743".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub jail: PathBuf,
    /// Directories or files the jailed tools may load as code (for dynamically linked builds).
    #[serde(default)]
    pub libs: Vec<PathBuf>,
    #[serde(default = "one")]
    pub jobs: usize,
}

fn one() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    #[serde(default = "ten")]
    pub max_gib: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Self { max_gib: ten() }
    }
}

fn ten() -> u64 {
    10
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub name: String,
    pub path: PathBuf,
}

/// Most libraries one server serves.
const MAX_LIBRARIES: usize = 32;

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.permissions().mode() & 0o022 != 0 {
            return Err(format!("{} is group- or world-writable; refusing to start", path.display()));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let c: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        c.validate()?;
        let mut c = c;
        // The configured roots are trusted, but may be reached through links (systemd's
        // StateDirectory is one). Resolve them once, so that paths built beneath them contain
        // no links: the jail refuses any link in an input's path (review M1).
        if let Ok(p) = c.data_dir.canonicalize() {
            c.data_dir = p;
        }
        for l in &mut c.libraries {
            if let Ok(p) = l.path.canonicalize() {
                l.path = p;
            }
        }
        Ok(c)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.public_origin.starts_with("https://") && !self.public_origin.starts_with("http://localhost") {
            return Err("public_origin must be an https:// URL".into());
        }
        if self.libraries.is_empty() || self.libraries.len() > MAX_LIBRARIES {
            return Err(format!("between 1 and {MAX_LIBRARIES} [[library]] entries are needed"));
        }
        let mut names = std::collections::HashSet::new();
        for l in &self.libraries {
            let n = l.name.trim();
            if n.is_empty() || n.len() > 64 || n.chars().any(char::is_control) {
                return Err(format!("library name {:?} must be 1-64 printable characters", l.name));
            }
            if !names.insert(n.to_lowercase()) {
                return Err(format!("library name {:?} is used twice", l.name));
            }
            if !l.path.is_absolute() {
                return Err(format!("library {:?}: path must be absolute", l.name));
            }
        }
        for p in [&self.ffmpeg.ffmpeg, &self.ffmpeg.ffprobe, &self.ffmpeg.jail] {
            if !p.is_absolute() {
                return Err(format!("{} must be an absolute path", p.display()));
            }
        }
        if !(1..=16).contains(&self.ffmpeg.jobs) {
            return Err("ffmpeg.jobs must be between 1 and 16".into());
        }
        Ok(())
    }
}

/// Creates the data directory (0700) if needed and refuses one that others can write to or
/// that belongs to someone else.
pub fn prepare_data_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} is not a directory (symlinks are not accepted)", dir.display()));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("{} must be mode 0700 (it is {:o})", dir.display(), meta.permissions().mode() & 0o777));
    }
    if meta.uid() != rustix::process::geteuid().as_raw() {
        return Err(format!("{} must belong to the user running ABShare", dir.display()));
    }
    Ok(())
}

/// A library root must exist, be a directory, and be read-only to us (SPEC D3).
pub fn check_library_root(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("library {}: {e}", path.display()))?;
    if !meta.is_dir() {
        return Err(format!("library {} is not a directory", path.display()));
    }
    if rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok() {
        return Err(format!("library {} is writable by this user; mount it read-only (SPEC D3)", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
data_dir = "/var/lib/abshare"
public_origin = "https://books.example.com"
[ffmpeg]
ffmpeg = "/usr/bin/ffmpeg"
ffprobe = "/usr/bin/ffprobe"
jail = "/usr/local/bin/abshare-jail"
[[library]]
name = "Audiobooks"
path = "/srv/audiobooks"
"#;

    #[test]
    fn a_minimal_config_parses_with_defaults() {
        let c: Config = toml::from_str(GOOD).expect("parse");
        c.validate().expect("valid");
        assert_eq!(c.listen, "127.0.0.1:8743");
        assert_eq!((c.ffmpeg.jobs, c.cache.max_gib), (1, 10));
    }

    #[test]
    fn unknown_keys_are_refused() {
        let bad = format!("{GOOD}\nallow_unsandboxed_ffmpeg = true\n");
        assert!(toml::from_str::<Config>(&bad).is_err());
        let bad = GOOD.replace("[ffmpeg]", "[ffmpeg]\nshell = \"/bin/sh\"");
        assert!(toml::from_str::<Config>(&bad).is_err());
    }

    #[test]
    fn unsafe_values_are_refused() {
        for (from, to) in [
            ("https://books.example.com", "http://books.example.com"),
            ("/srv/audiobooks", "srv/audiobooks"),
            ("/usr/local/bin/abshare-jail", "abshare-jail"),
            ("name = \"Audiobooks\"", "name = \"\""),
        ] {
            let c: Config = toml::from_str(&GOOD.replace(from, to)).expect("parse");
            assert!(c.validate().is_err(), "{from} -> {to}");
        }
        let twice = format!("{GOOD}[[library]]\nname = \"audiobooks\"\npath = \"/srv/other\"\n");
        let c: Config = toml::from_str(&twice).expect("parse");
        assert!(c.validate().is_err());
    }

    #[test]
    fn a_writable_library_root_is_refused() {
        let dir = std::env::temp_dir().join(format!("abshare-lib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let err = check_library_root(&dir).expect_err("writable root must be refused");
        assert!(err.contains("read-only"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_data_dir_others_can_read_is_refused() {
        let dir = std::env::temp_dir().join(format!("abshare-data-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert!(prepare_data_dir(&dir).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert!(prepare_data_dir(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
