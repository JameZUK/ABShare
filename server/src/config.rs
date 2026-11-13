//! `abshare.toml` (SPEC §5.7). Strict: unknown keys are an error, and the server refuses to
//! start on unsafe permissions.

use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
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

/// `listen` and `trusted_proxies`, parsed: what `serve` binds and whom it believes.
#[derive(Debug)]
pub struct Net {
    pub listen: SocketAddr,
    pub trusted: Vec<IpAddr>,
}

/// Shown by `serve` and `check-config` when [`Config::proxy_warning`] applies.
pub const PROXY_WARNING: &str = "listen is a loopback address and trusted_proxies is empty. That is right for local use only. Behind a reverse proxy on this host, ABShare would accept plain HTTP (it can't tell whether the proxy used TLS) and every client would share the proxy's address for lockouts and rate limits: set trusted_proxies to the proxy's address (such as [\"127.0.0.1\"]).";

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

    /// Parses `listen` (an IP address and port) and `trusted_proxies` (IP addresses). Part
    /// of [`Config::validate`], so `check-config` refuses what `serve` would.
    pub fn net(&self) -> Result<Net, String> {
        let listen = self
            .listen
            .parse::<SocketAddr>()
            .map_err(|_| format!("listen: {:?} is not an IP address and port, such as 127.0.0.1:8743", self.listen))?;
        let trusted = self
            .trusted_proxies
            .iter()
            .map(|p| p.parse::<IpAddr>().map_err(|_| format!("trusted_proxies: {p:?} is not an IP address")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Net { listen, trusted })
    }

    /// Whether to show [`PROXY_WARNING`]: a loopback `listen` with no trusted proxy. Local use
    /// is legitimate, so this is a warning and not an error.
    pub fn proxy_warning(&self) -> bool {
        self.trusted_proxies.is_empty() && self.net().is_ok_and(|n| n.listen.ip().is_loopback())
    }

    /// The `Host` value the server answers for (as `http::AppState` derives it): the host of
    /// `public_origin`, with its port if it names one.
    pub fn host_header(&self) -> Result<String, String> {
        let url = webauthn_rs::prelude::Url::parse(&self.public_origin).map_err(|e| format!("public_origin: {e}"))?;
        let host = url.host_str().ok_or("public_origin needs a host")?;
        Ok(match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_owned(),
        })
    }

    /// `public_origin` is an `https://` URL. Plain HTTP is accepted for this machine alone:
    /// exactly `localhost`, `127.0.0.1` or `[::1]`, with an optional port. The URL is parsed,
    /// so `http://localhost.example.com` and `http://localhost@example.com` are refused.
    fn check_origin(&self) -> Result<(), String> {
        let refuse = || "public_origin must be an https:// URL (or http://localhost for local use)".to_owned();
        let url = webauthn_rs::prelude::Url::parse(&self.public_origin).map_err(|_| refuse())?;
        let host = url.host_str().ok_or_else(refuse)?;
        let plain = url.username().is_empty() && url.password().is_none();
        let local = plain && matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        match url.scheme() {
            "https" => Ok(()),
            "http" if local => Ok(()),
            _ => Err(refuse()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.check_origin()?;
        self.net()?;
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

/// Every `[ffmpeg] libs` entry must exist: the jail refuses a missing one, so every probe
/// and encode would fail. The image's ffmpeg is static (since 0.9.0) and needs `libs = []`;
/// an older config may still list `/etc/ld.so.cache`, which the image no longer has.
pub fn check_ffmpeg_libs(libs: &[PathBuf]) -> Result<(), String> {
    for p in libs {
        if std::fs::symlink_metadata(p).is_err() {
            return Err(format!(
                "[ffmpeg] libs: {} doesn't exist; the jail would refuse every ffmpeg run \
                 (a static ffmpeg, as in the image, needs libs = [])",
                p.display()
            ));
        }
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

    fn with(from: &str, to: &str) -> Config {
        assert!(GOOD.contains(from), "{from}");
        toml::from_str(&GOOD.replace(from, to)).expect("parse")
    }

    #[test]
    fn plain_http_is_for_this_machine_only() {
        for ok in [
            "https://books.example.com",
            "https://books.example.com:8443",
            "http://localhost",
            "http://localhost:8743",
            "http://localhost:8743/",
            "http://127.0.0.1:8743",
            "http://[::1]:8743",
        ] {
            with("https://books.example.com", ok).validate().unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            "http://localhost.example.com",
            "http://localhost.example.com:8743",
            "http://localhostevil:8743",
            "http://localhost@books.example.com",
            "http://localhost:8743@books.example.com",
            "http://user@localhost:8743",
            "http://localhost.",
            "http://127.0.0.1.example.com",
            "http://127.0.0.2:8743",
            "http://10.0.0.10:8743",
            "http://[::2]:8743",
            "http://books.example.com/http://localhost",
            "ftp://localhost",
            "localhost:8743",
            "https://",
            "",
        ] {
            assert!(with("https://books.example.com", bad).validate().is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn the_host_header_follows_the_origin() {
        assert_eq!(with("x", "x").host_header().as_deref(), Ok("books.example.com"));
        let c = with("https://books.example.com", "http://localhost:8743/");
        assert_eq!(c.host_header().as_deref(), Ok("localhost:8743"));
    }

    #[test]
    fn listen_and_trusted_proxies_are_validated() {
        let listen = |v: &str| toml::from_str::<Config>(&format!("listen = {v:?}\n{GOOD}")).expect("parse");
        for ok in ["127.0.0.1:8743", "0.0.0.0:8743", "[::1]:8743", "[::]:8743", "10.0.0.10:80"] {
            listen(ok).validate().unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in ["", "8743", ":8743", "127.0.0.1", "127.0.0.1:", "127.0.0.1:99999", "localhost:8743", "::1:8743x"] {
            let err = listen(bad).validate().expect_err(bad);
            assert!(err.starts_with("listen:"), "{bad}: {err}");
        }
        let proxies = |v: &str| toml::from_str::<Config>(&format!("trusted_proxies = {v}\n{GOOD}")).expect("parse");
        let net = proxies(r#"["10.0.0.10", "::1"]"#).net().expect("valid");
        assert_eq!(net.trusted.len(), 2);
        for bad in [r#"["proxy"]"#, r#"["10.0.0.0/24"]"#, r#"["10.0.0.10:80"]"#, r#"["10.0.0.10", ""]"#] {
            let err = proxies(bad).validate().expect_err(bad);
            assert!(err.starts_with("trusted_proxies:"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_loopback_listener_without_a_trusted_proxy_is_warned_about() {
        let c = |head: &str| toml::from_str::<Config>(&format!("{head}\n{GOOD}")).expect("parse");
        assert!(c("").proxy_warning(), "the default listen is loopback");
        assert!(c(r#"listen = "[::1]:8743""#).proxy_warning());
        assert!(!c(r#"trusted_proxies = ["127.0.0.1"]"#).proxy_warning());
        assert!(!c(r#"listen = "0.0.0.0:8743""#).proxy_warning());
        assert!(!c(r#"listen = "10.0.0.10:8743""#).proxy_warning());
        assert!(PROXY_WARNING.contains("trusted_proxies"));
    }

    #[test]
    fn a_missing_ffmpeg_library_is_refused() {
        check_ffmpeg_libs(&[]).expect("none is fine");
        check_ffmpeg_libs(&[std::env::temp_dir()]).expect("an existing one is fine");
        let gone = std::env::temp_dir().join(format!("abshare-no-such-lib-{}", std::process::id()));
        let err = check_ffmpeg_libs(&[std::env::temp_dir(), gone]).expect_err("a missing one must be refused");
        assert!(err.contains("abshare-no-such-lib") && err.contains("libs = []"), "{err}");
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

    // 2026-11-12 review: this refusal had no test.
    #[test]
    fn a_config_file_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("abshare-config-{}.toml", std::process::id()));
        std::fs::write(&path, GOOD).expect("write");
        for mode in [0o620, 0o602, 0o666] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
            let err = Config::load(&path).expect_err("a writable config was accepted");
            assert!(err.contains("writable"), "{mode:o}: {err}");
        }
        // Not refused for its mode when only the owner can write (it fails later, or not at
        // all, for other reasons).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod");
        assert!(Config::load(&path).map_or_else(|e| !e.contains("writable"), |_| true));
        let _ = std::fs::remove_file(&path);
    }
}
