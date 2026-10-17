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

/// Whether the jail can run here: Landlock active with ABI 6 or later (Linux 6.12). Hosted CI
/// runners may have older kernels; there the tests skip unless ABSHARE_REQUIRE_LANDLOCK is set
/// (as it is on development machines and the deployment target).
fn landlock_available() -> bool {
    let active =
        std::fs::read_to_string("/sys/kernel/security/lsm").is_ok_and(|s| s.split(',').any(|m| m == "landlock"));
    let probe = Command::new(JAIL)
        .args([
            "--lib",
            "/usr/lib",
            "--lib",
            "/lib64",
            "--lib",
            "/lib",
            "--lib",
            "/etc/ld.so.cache",
            "--",
            PROBE,
            "env",
        ])
        .output();
    let jail_runs = active && probe.as_ref().is_ok_and(|o| o.status.success());
    if !jail_runs && std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_some() {
        let why = match &probe {
            Ok(o) => format!("{}: {}", o.status, String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => e.to_string(),
        };
        panic!("ABSHARE_REQUIRE_LANDLOCK is set but the jail cannot run (Landlock active: {active}): {why}");
    }
    jail_runs
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
fn every_socket_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["inet"])), EPERM);
    assert_eq!(out(&s.run(&["inet6"])), EPERM);
    assert_eq!(out(&s.run(&["connect"])), EPERM);
    assert_eq!(out(&s.run(&["unix"])), EPERM);
}

#[test]
fn starting_another_program_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    // Refused twice over: no new process (seccomp, EPERM), and no execution outside the
    // allowed paths (Landlock, EACCES) if one were made.
    for prog in ["/bin/sh", "/usr/bin/env"] {
        let o = out(&s.run(&["exec", prog]));
        assert!(o == EPERM || o == EACCES, "{prog}: {o}");
    }
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

/// Runs a short Python script inside the jail (for raw system calls the probe can't make
/// without `unsafe`). `None` when there's no python3 here.
fn python_in_jail(s: &Scratch, script: &str) -> Option<String> {
    let py = ["/usr/bin/python3", "/bin/python3"].into_iter().find(|p| Path::new(p).exists())?;
    let mut cmd = Command::new(JAIL);
    cmd.args(["--read", &s.p("input"), "--write", &s.p("job")]);
    for lib in ["/usr/lib", "/usr/lib64", "/lib", "/lib64", "/etc/ld.so.cache"] {
        if Path::new(lib).exists() {
            cmd.args(["--lib", lib]);
        }
    }
    let o = cmd.args(["--", py, "-I", "-S", "-c", script]).output().expect("run jail");
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout).trim(),
        if o.status.code().is_none() { "killed" } else { "" }
    ))
}

/// x32 system-call numbers (bit 0x40000000) must not slip past the filter (review J1).
#[cfg(target_arch = "x86_64")]
#[test]
fn x32_system_calls_cannot_bypass_the_filter() {
    need_landlock!();
    let s = Scratch::new();
    // unshare(CLONE_NEWUSER) and socket(AF_INET, SOCK_DGRAM) through their x32 numbers.
    for (name, nr, args) in [("unshare", 272, "0x10000000"), ("socket", 41, "2, 2, 0")] {
        let script = format!(
            "import ctypes\nl=ctypes.CDLL(None,use_errno=True)\nr=l.syscall(0x40000000|{nr},{args})\nprint('ok' if r>=0 else 'denied')"
        );
        let Some(out) = python_in_jail(&s, &script) else {
            eprintln!("skipped: no python3");
            return;
        };
        assert!(out != "ok", "x32 {name} succeeded inside the jail: {out}");
    }
}

/// No sockets at all, so nothing can connect to a UNIX socket on the host (review J2).
#[test]
fn connecting_to_a_unix_socket_outside_the_jail_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    // A socket path must fit in sun_path (108 bytes), which the target directory of a deep
    // checkout can exceed, so the listener lives in the system temporary directory: still
    // outside everything the jail may reach.
    struct Unlink(PathBuf);
    impl Drop for Unlink {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let name = s.root.file_name().expect("scratch name").to_string_lossy().into_owned();
    let path = std::env::temp_dir().join(format!("abshare-{name}.sock"));
    let _ = std::fs::remove_file(&path);
    let _listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
    let _unlink = Unlink(path.clone());
    let script = format!(
        "import socket\ntry:\n s=socket.socket(socket.AF_UNIX)\n s.connect({:?})\n print('ok')\nexcept OSError as e:\n print('denied')",
        path.to_string_lossy()
    );
    let Some(out) = python_in_jail(&s, &script) else {
        eprintln!("skipped: no python3");
        return;
    };
    assert_eq!(out, "denied");
}

/// No new processes: a forked child could outlive the wall-clock kill (review J6).
#[test]
fn forking_is_refused_but_threads_work() {
    need_landlock!();
    let s = Scratch::new();
    let Some(out) = python_in_jail(
        &s,
        "import os\ntry:\n pid=os.fork()\n print('ok') if pid else os._exit(0)\nexcept OSError:\n print('denied')",
    ) else {
        eprintln!("skipped: no python3");
        return;
    };
    assert_eq!(out, "denied");
    assert_eq!(
        python_in_jail(&s, "import threading\nt=threading.Thread(target=lambda:None);t.start();t.join();print('ok')")
            .as_deref(),
        Some("ok")
    );
}

/// An input path with a symbolic link anywhere in it is refused: a file swapped for a link
/// after the server checked it can't widen what ffmpeg may read (review M1).
#[test]
fn an_input_reached_through_a_symlink_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    std::os::unix::fs::symlink(s.root.join("secret/key"), s.root.join("input/link.m4b")).expect("link");
    std::os::unix::fs::symlink(s.root.join("secret"), s.root.join("linkdir")).expect("link");
    for input in [s.p("input/link.m4b"), s.p("linkdir/key")] {
        let o = Command::new(JAIL)
            .args(["--read", &input, "--write", &s.p("job"), "--lib", "/usr/lib", "--", PROBE, "read", &input])
            .output()
            .expect("run");
        assert!(
            !o.status.success() && !String::from_utf8_lossy(&o.stdout).contains("ok"),
            "{input}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    // A plain input still works.
    assert_eq!(out(&s.run(&["read", &s.p("input/book.m4b")])), "ok");
}
