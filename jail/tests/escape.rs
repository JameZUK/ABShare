//! Escape tests: run the real `abshare-jail` around a probe and prove each confinement.
//!
//! These need Landlock (ABI 6+) in the running kernel, and the raw system-call tests need
//! python3 (the probe has no `unsafe`, so it can't make them itself). In CI,
//! `ABSHARE_REQUIRE_LANDLOCK=1` turns a missing Landlock or python3 into a failure instead
//! of a skip.

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

/// The python3 used for raw system calls the probe can't make without `unsafe`. `None` when
/// there is none here; with ABSHARE_REQUIRE_LANDLOCK set (as in CI) that is a failure, so
/// these tests can never pass by being skipped (review M17).
fn python() -> Option<&'static str> {
    let py = ["/usr/bin/python3", "/bin/python3"].into_iter().find(|p| Path::new(p).exists());
    if py.is_none() && std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_some() {
        panic!("ABSHARE_REQUIRE_LANDLOCK is set but there is no python3 for the raw system-call tests");
    }
    py
}

macro_rules! need_python {
    () => {
        match python() {
            Some(py) => py,
            None => {
                eprintln!("skipped: no python3");
                return;
            }
        }
    };
}

/// Runs a short Python script inside the jail.
fn python_output(s: &Scratch, py: &str, script: &str) -> Output {
    let mut cmd = Command::new(JAIL);
    cmd.args(["--read", &s.p("input"), "--write", &s.p("job")]);
    for lib in ["/usr/lib", "/usr/lib64", "/lib", "/lib64", "/etc/ld.so.cache"] {
        if Path::new(lib).exists() {
            cmd.args(["--lib", lib]);
        }
    }
    cmd.args(["--", py, "-I", "-S", "-c", script]).output().expect("run jail")
}

/// What a script printed inside the jail, with `killed` appended if a signal ended it.
fn python_in_jail(s: &Scratch, py: &str, script: &str) -> String {
    let o = python_output(s, py, script);
    format!("{}{}", String::from_utf8_lossy(&o.stdout).trim(), if o.status.code().is_none() { "killed" } else { "" })
}

/// x32 system-call numbers (bit 0x40000000) must not slip past the filter (review J1).
#[cfg(target_arch = "x86_64")]
#[test]
fn x32_system_calls_cannot_bypass_the_filter() {
    use std::os::unix::process::ExitStatusExt;
    need_landlock!();
    let py = need_python!();
    let s = Scratch::new();
    // unshare(CLONE_NEWUSER) and socket(AF_INET, SOCK_DGRAM) through their x32 numbers.
    for (name, nr, args) in [("unshare", 272, "0x10000000"), ("socket", 41, "2, 2, 0")] {
        let script = format!(
            "import ctypes\nl=ctypes.CDLL(None,use_errno=True)\nprint('start',flush=True)\nr=l.syscall(0x40000000|{nr},{args})\nprint('ok' if r>=0 else 'denied')"
        );
        // The guard kills the process (SIGSYS) at the call: the script started, and never
        // got as far as printing a result.
        let o = python_output(&s, py, &script);
        assert_eq!(out(&o), "start", "x32 {name}: {}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(o.status.signal(), Some(libc::SIGSYS), "x32 {name}: {:?}", o.status);
    }
}

/// One raw system call for [`assert_refused`]: a label, the number, and its arguments as
/// Python (`L` is `ctypes.c_long`, `S` is `ctypes.c_char_p`).
type Call = (&'static str, i64, String);

fn call(name: &'static str, nr: i64, args: &str) -> Call {
    (name, nr, args.to_owned())
}

fn syscall_script(calls: &[Call]) -> String {
    let mut script = String::from(
        "import ctypes,errno\nl=ctypes.CDLL(None,use_errno=True)\nl.syscall.restype=ctypes.c_long\n\
         L=ctypes.c_long\nS=ctypes.c_char_p\n\
         def t(n,nr,*a):\n ctypes.set_errno(0)\n r=l.syscall(L(nr),*a)\n \
         print(n,'ok' if r>=0 else errno.errorcode.get(ctypes.get_errno(),'?'),flush=True)\n",
    );
    for (name, nr, args) in calls {
        script.push_str(&format!("t('{name}',{nr},{args})\n"));
    }
    script
}

fn results(stdout: &[u8]) -> std::collections::BTreeMap<String, String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, r)| (n.to_owned(), r.to_owned())))
        .collect()
}

/// Makes each call inside the jail and asserts it fails there with exactly `errno`. The same
/// calls are then made outside the jail, where they must end differently: that is what
/// shows the filter (and not bad arguments or a lack of privilege) refused them, so the
/// test fails if the rule is removed. The arguments are chosen so that the calls change
/// nothing when they aren't filtered. For the calls named in `host_may_refuse` the host can
/// be configured to refuse everyone the same way; there the comparison is only reported.
fn assert_refused(py: &str, calls: &[Call], errno: &str, host_may_refuse: &[&str]) {
    let s = Scratch::new();
    let script = syscall_script(calls);
    let o = python_output(&s, py, &script);
    let jailed = results(&o.stdout);
    assert!(o.status.success(), "{:?}: {}", o.status, String::from_utf8_lossy(&o.stderr));
    let free = Command::new(py).args(["-I", "-S", "-c", &script]).output().expect("run python");
    let free = results(&free.stdout);
    for (name, _, _) in calls {
        assert_eq!(jailed.get(*name).map(String::as_str), Some(errno), "{name} inside the jail");
        let outside = free.get(*name).map(String::as_str).unwrap_or("<no result>");
        if host_may_refuse.contains(name) && outside == errno {
            eprintln!("note: this host also refuses {name} with {errno} outside the jail");
        } else {
            assert_ne!(outside, errno, "{name} fails the same way outside the jail, so this proves nothing");
        }
    }
}

/// Namespaces, tracing, in-memory executables, BPF, io_uring and mounts are refused by the
/// seccomp filter itself (review M17).
#[test]
fn dangerous_system_calls_are_refused_by_the_filter() {
    need_landlock!();
    let py = need_python!();
    let mut calls = vec![
        call("unshare", libc::SYS_unshare, "L(0)"),
        call("setns", libc::SYS_setns, "L(-1),L(0)"),
        // PTRACE_ATTACH to a process that can't exist.
        call("ptrace", libc::SYS_ptrace, "L(16),L(0x7fffffff),L(0),L(0)"),
        call("memfd_create", libc::SYS_memfd_create, "S(b'x'),L(0)"),
        call("execveat", libc::SYS_execveat, "L(-1),L(0),L(0),L(0),L(0)"),
        call("bpf", libc::SYS_bpf, "L(9999),L(0),L(0)"),
        call("io_uring_setup", libc::SYS_io_uring_setup, "L(0),L(0)"),
        call("io_uring_enter", libc::SYS_io_uring_enter, "L(-1),L(0),L(0),L(0),L(0),L(0)"),
        call("io_uring_register", libc::SYS_io_uring_register, "L(-1),L(0),L(0),L(0)"),
        call("mount", libc::SYS_mount, "L(0),L(0),L(0),L(0),L(0)"),
        // A new process: no CLONE_THREAD. (CLONE_SIGHAND without CLONE_VM is invalid, so
        // nothing is made where the filter is missing.)
        call("clone_process", libc::SYS_clone, &format!("L({}),L(0),L(0),L(0),L(0)", libc::CLONE_SIGHAND)),
    ];
    // A thread in a new namespace. (CLONE_THREAD without CLONE_SIGHAND is invalid, so again
    // nothing is made where the filter is missing.)
    for (name, flag) in [
        ("clone_newuser", libc::CLONE_NEWUSER),
        ("clone_newns", libc::CLONE_NEWNS),
        ("clone_newnet", libc::CLONE_NEWNET),
        ("clone_newpid", libc::CLONE_NEWPID),
        ("clone_newipc", libc::CLONE_NEWIPC),
        ("clone_newuts", libc::CLONE_NEWUTS),
        ("clone_newcgroup", libc::CLONE_NEWCGROUP),
    ] {
        calls.push(call(name, libc::SYS_clone, &format!("L({}),L(0),L(0),L(0),L(0)", libc::CLONE_THREAD | flag)));
    }
    assert_refused(py, &calls, "EPERM", &["bpf", "io_uring_setup"]);
}

/// `clone3` hides its flags from a filter, so it reports "not implemented" and the C library
/// falls back to `clone`.
#[test]
fn clone3_is_not_implemented_inside_the_jail() {
    need_landlock!();
    let py = need_python!();
    assert_refused(py, &[call("clone3", libc::SYS_clone3, "L(0),L(0)")], "ENOSYS", &[]);
}

/// System V IPC and POSIX message queues are refused (review L16).
#[test]
fn sysv_ipc_and_message_queues_are_refused() {
    need_landlock!();
    let py = need_python!();
    // Every call here is invalid or names something that doesn't exist, so none makes an
    // IPC object where the filter is missing.
    let calls = [
        call("shmget", libc::SYS_shmget, "L(0),L(0),L(0)"),
        call("shmat", libc::SYS_shmat, "L(-1),L(0),L(0)"),
        call("shmctl", libc::SYS_shmctl, "L(-1),L(2),L(0)"),
        call("shmdt", libc::SYS_shmdt, "L(0)"),
        call("msgget", libc::SYS_msgget, "L(0x7fabcdef),L(0)"),
        call("msgsnd", libc::SYS_msgsnd, "L(-1),L(0),L(0),L(0)"),
        call("msgrcv", libc::SYS_msgrcv, "L(-1),L(0),L(0),L(0),L(0)"),
        call("msgctl", libc::SYS_msgctl, "L(-1),L(2),L(0)"),
        call("semget", libc::SYS_semget, "L(0x7fabcdef),L(0),L(0)"),
        call("semop", libc::SYS_semop, "L(-1),L(0),L(0)"),
        call("semctl", libc::SYS_semctl, "L(-1),L(0),L(2),L(0)"),
        call("semtimedop", libc::SYS_semtimedop, "L(-1),L(0),L(0),L(0)"),
        call("mq_open", libc::SYS_mq_open, "L(0),L(0),L(0),L(0)"),
        call("mq_unlink", libc::SYS_mq_unlink, "L(0)"),
        call("mq_timedsend", libc::SYS_mq_timedsend, "L(-1),L(0),L(0),L(0),L(0)"),
        call("mq_timedreceive", libc::SYS_mq_timedreceive, "L(-1),L(0),L(0),L(0),L(0)"),
        call("mq_notify", libc::SYS_mq_notify, "L(-1),L(0)"),
        call("mq_getsetattr", libc::SYS_mq_getsetattr, "L(-1),L(0),L(0)"),
    ];
    assert_refused(py, &calls, "EPERM", &[]);
}

/// The same refusal seen natively, through the probe (no Python involved).
#[test]
fn an_in_memory_file_is_refused() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["memfd"])), EPERM);
}

/// The jailed program can never gain privileges (setuid programs, file capabilities), and
/// runs under a seccomp filter.
#[test]
fn no_new_privileges_and_a_seccomp_filter_are_in_force() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run(&["privs"])), "no_new_privs=true seccomp_filter=true");
}

/// Each resource limit is the one asked for, soft and hard, and core dumps are off.
#[test]
fn the_resource_limits_are_the_ones_asked_for() {
    need_landlock!();
    const MIB: u64 = 1024 * 1024;
    let s = Scratch::new();
    let o = s.run_with(&["--mem-mib", "300", "--cpu-secs", "7", "--fsize-mib", "3", "--nofile", "33"], &["limits"]);
    assert_eq!(
        out(&o),
        format!("as={0}/{0} cpu=7/7 fsize={1}/{1} nofile=33/33 core=0/0", 300 * MIB, 3 * MIB),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    // With no options the defaults apply: still bounded, still no core dumps.
    assert_eq!(
        out(&s.run(&["limits"])),
        format!("as={0}/{0} cpu=3600/3600 fsize={1}/{1} nofile=64/64 core=0/0", 1024 * MIB, 4096 * MIB)
    );
}

#[test]
fn the_address_space_limit_stops_large_allocations() {
    need_landlock!();
    let s = Scratch::new();
    assert_eq!(out(&s.run_with(&["--mem-mib", "256"], &["alloc", "16"])), "ok");
    assert_eq!(out(&s.run_with(&["--mem-mib", "256"], &["alloc", "1024"])), "denied 12");
}

#[test]
fn the_cpu_limit_stops_a_busy_program() {
    use std::os::unix::process::ExitStatusExt;
    need_landlock!();
    let s = Scratch::new();
    // The probe would spin for 20 s; one second of CPU is all it gets.
    let started = std::time::Instant::now();
    let o = s.run_with(&["--cpu-secs", "1"], &["spin", "20"]);
    let signal = o.status.signal();
    assert!(signal == Some(libc::SIGKILL) || signal == Some(libc::SIGXCPU), "{:?} {}", o.status, out(&o));
    assert_eq!(out(&o), "");
    assert!(started.elapsed() < std::time::Duration::from_secs(15), "{:?}", started.elapsed());
}

#[test]
fn the_open_file_limit_stops_descriptor_exhaustion() {
    need_landlock!();
    let s = Scratch::new();
    let book = s.p("input/book.m4b");
    assert_eq!(out(&s.run_with(&["--nofile", "16"], &["fds", &book, "4"])), "ok");
    assert_eq!(out(&s.run_with(&["--nofile", "16"], &["fds", &book, "64"])), "denied 24");
}

/// No sockets at all, so nothing can connect to a UNIX socket on the host (review J2).
#[test]
fn connecting_to_a_unix_socket_outside_the_jail_is_refused() {
    need_landlock!();
    let py = need_python!();
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
    assert_eq!(python_in_jail(&s, py, &script), "denied");
}

/// No new processes: a forked child could outlive the wall-clock kill (review J6).
#[test]
fn forking_is_refused_but_threads_work() {
    need_landlock!();
    let py = need_python!();
    let s = Scratch::new();
    let fork =
        "import os\ntry:\n pid=os.fork()\n print('ok') if pid else os._exit(0)\nexcept OSError:\n print('denied')";
    assert_eq!(python_in_jail(&s, py, fork), "denied");
    assert_eq!(
        python_in_jail(
            &s,
            py,
            "import threading\nt=threading.Thread(target=lambda:None);t.start();t.join();print('ok')"
        ),
        "ok"
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
