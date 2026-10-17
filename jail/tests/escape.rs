//! Escape tests: run the real `abshare-jail` around a probe and prove each confinement.
//!
//! These need Landlock (ABI 6+) in the running kernel. In CI, `ABSHARE_REQUIRE_LANDLOCK=1`
//! turns a missing Landlock into a failure instead of a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const JAIL: &str = env!("CARGO_BIN_EXE_abshare-jail");
const PROBE: &str = env!("CARGO_BIN_EXE_abshare-jail-probe");
const EACCES: &str = "denied 13";
const EPERM: &str = "denied 1";

/// A fresh scratch area: `input/` (with a book), `job/`, and `secret/` beside them.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "jail-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        for d in ["input", "job", "secret", "elsewhere"] {
            std::fs::create_dir_all(root.join(d)).expect("mkdir");
        }
        std::fs::write(root.join("input/book.m4b"), b"audio").expect("write");
        std::fs::write(root.join("secret/key"), b"secret").expect("write");
        Self { root }
    }

    fn p(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().into_owned()
    }

    /// Runs the probe inside the jail with the standard policy.
    fn run(&self, probe: &[&str]) -> Output {
        self.run_with(&[], probe)
    }

    fn run_with(&self, extra: &[&str], probe: &[&str]) -> Output {
        let mut cmd = Command::new(JAIL);
        cmd.args(["--read", &self.p("input"), "--write", &self.p("job")]);
        for lib in ["/usr/lib", "/usr/lib64", "/lib", "/lib64", "/etc/ld.so.cache"] {
            if Path::new(lib).exists() {
                cmd.args(["--lib", lib]);
            }
        }
        cmd.args(extra).arg("--").arg(PROBE).args(probe);
        cmd.output().expect("run jail")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn landlock_available() -> bool {
    let ok = std::fs::read_to_string("/sys/kernel/security/lsm").is_ok_and(|s| s.split(',').any(|m| m == "landlock"));
    if !ok && std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_some() {
        panic!("ABSHARE_REQUIRE_LANDLOCK is set but Landlock is not active");
    }
    ok
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

macro_rules! need_landlock {
    () => {
        if !landlock_available() {
            eprintln!("skipped: Landlock not active");
            return;
        }
    };
}

#[test]
fn reading_the_named_input_is_allowed() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["read", &s.p("input/book.m4b")])), "ok");
}

#[test]
fn reading_system_files_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["read", "/etc/passwd"])), EACCES);
    assert_eq!(out(&s.run(&["read", "/proc/1/environ"])), EACCES);
}

#[test]
fn reading_a_sibling_secret_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["read", &s.p("secret/key")])), EACCES);
}

#[test]
fn writing_in_the_job_directory_is_allowed() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["write", &s.p("job/out.tmp")])), "ok");
}

#[test]
fn writing_anywhere_else_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["write", &s.p("elsewhere/x")])), EACCES);
    assert_eq!(out(&s.run(&["write", &s.p("input/evil")])), EACCES);
    assert_eq!(out(&s.run(&["write", "/tmp/abshare-jail-escape"])), EACCES);
    assert!(!Path::new(&s.p("input/evil")).exists());
}

#[test]
fn network_sockets_are_refused_and_unix_sockets_allowed() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["inet"])), EPERM);
    assert_eq!(out(&s.run(&["inet6"])), EPERM);
    assert_eq!(out(&s.run(&["connect"])), EPERM);
    assert_eq!(out(&s.run(&["unix"])), "ok");
}

#[test]
fn starting_another_program_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["exec", "/bin/sh"])), EACCES);
    assert_eq!(out(&s.run(&["exec", "/usr/bin/env"])), EACCES);
}

#[test]
fn signalling_a_process_outside_the_jail_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    let me = std::process::id().to_string();
    assert_eq!(out(&s.run(&["kill", &me])), EPERM);
}

#[test]
fn threads_still_work() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["thread"])), "ok");
}

#[test]
fn the_environment_is_cleared() {
    need_landlock!();
    let s = Scratch::new();
    let o = Command::new(JAIL)
        .env("ABSHARE_SECRET_TOKEN", "x")
        .args(["--read", &s.p("input")])
        .args(["--lib", "/usr/lib", "--lib", "/lib64"])
        .args(["--", PROBE, "env"])
        .output()
        .expect("run");
    assert_eq!(out(&o), "0");
}

#[test]
fn the_file_size_limit_stops_large_writes() {
    need_landlock!();
    let s = Scratch::new();
    let o = s.run_with(&["--fsize-mib", "1"], &["big", &s.p("job/big")]);
    assert!(!o.status.success());
    assert_ne!(out(&o), "ok");
    let len = std::fs::metadata(s.p("job/big")).map(|m| m.len()).unwrap_or(0);
    assert!(len <= 1024 * 1024, "wrote {len} bytes");
}

#[test]
fn an_inherited_descriptor_makes_the_jail_refuse() {
    need_landlock!();
    let s = Scratch::new();
    // fd 7 stays open across exec: the jail must refuse rather than let it bypass Landlock.
    let script =
        format!("exec 7<'{}'; exec '{JAIL}' --read '{}' -- '{PROBE}' read /dev/fd/7", s.p("secret/key"), s.p("input"));
    let o = Command::new("/bin/sh").args(["-c", &script]).output().expect("run");
    assert_eq!(o.status.code(), Some(125));
    assert_eq!(out(&o), "");
    assert!(String::from_utf8_lossy(&o.stderr).contains("inherited file descriptors"));
}

#[test]
fn bad_arguments_never_run_the_program() {
    for args in [
        vec!["--read", "/nonexistent-abshare-path", "--", PROBE, "env"],
        vec!["--", "abshare-jail-probe", "env"],
        vec!["--read", "/tmp"],
        vec!["--surprise", "--", PROBE, "env"],
    ] {
        let mut cmd = Command::new(JAIL);
        cmd.args(&args);
        let o = cmd.output().expect("run");
        assert_eq!(o.status.code(), Some(125), "{args:?}");
        assert_eq!(out(&o), "", "{args:?}");
    }
}

#[test]
fn a_program_inside_the_writable_directory_is_refused() {
    let s = Scratch::new();
    let copy = s.p("job/probe");
    std::fs::copy(PROBE, &copy).expect("copy");
    let o = Command::new(JAIL).args(["--write", &s.p("job"), "--", &copy, "env"]).output().expect("run");
    assert_eq!(o.status.code(), Some(125));
    assert_eq!(out(&o), "");
}
