//! `abshare-jail`: runs one program (ffmpeg or ffprobe) so that a compromise of it gains
//! nothing (SPEC D19).
//!
//! Before replacing itself with the program it:
//! 1. refuses to run if it inherited any file descriptor above 2;
//! 2. sets resource limits (address space, CPU, file size, open files, no core dumps);
//! 3. applies Landlock: read-only access to the named inputs, read+write in the named job
//!    directories, execute only the named program (and libraries), no TCP bind or connect,
//!    no signals or abstract Unix sockets outside the sandbox;
//! 4. applies seccomp filters that refuse non-Unix sockets, namespaces, tracing, mounts,
//!    io_uring, BPF, memfd execution, module loading and similar;
//! 5. clears the environment and replaces itself with the program.
//!
//! Any failure exits with status 125 before the program runs. It never falls back to
//! running unconfined.

mod args;
mod filter;
mod landlock_policy;

use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

/// Exit status when the jail refuses or fails to set up; the program never ran.
const REFUSED: u8 = 125;

fn main() -> ExitCode {
    match run() {
        Ok(never) => match never {},
        Err(e) => {
            eprintln!("abshare-jail: {e}");
            ExitCode::from(REFUSED)
        }
    }
}

enum Never {}

fn run() -> Result<Never, String> {
    let config = args::parse(std::env::args_os()).map_err(|e| format!("{e}\n{}", args::USAGE))?;
    check_inherited_fds()?;
    set_limits(&config)?;
    let filters = filter::compile()?;
    landlock_policy::apply(&config)?;
    filter::apply(&filters)?;
    let mut cmd = Command::new(&config.program);
    cmd.args(&config.args).env_clear();
    // CommandExt::exec is execve(2) with an argument vector: no shell is involved.
    let err = CommandExt::exec(&mut cmd);
    Err(format!("cannot start {}: {err}", config.program.display()))
}

/// An inherited descriptor would bypass Landlock (it was opened before the sandbox), so
/// the caller must pass only stdin, stdout and stderr.
fn check_inherited_fds() -> Result<(), String> {
    let dir = std::fs::read_dir("/proc/self/fd").map_err(|e| format!("cannot list /proc/self/fd: {e}"))?;
    let mut extra = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|e| format!("cannot list /proc/self/fd: {e}"))?;
        let Some(fd) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if fd <= 2 {
            continue;
        }
        // The directory handle used for this listing shows up in it; skip only that.
        let target = std::fs::read_link(entry.path()).unwrap_or_default();
        if target.starts_with("/proc") && target.to_string_lossy().ends_with("/fd") {
            continue;
        }
        extra.push(fd);
    }
    if extra.is_empty() { Ok(()) } else { Err(format!("refusing to run with inherited file descriptors {extra:?}")) }
}

fn set_limits(c: &args::Config) -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    const MIB: u64 = 1024 * 1024;
    let limits = [
        (Resource::As, c.mem_mib * MIB, "address space"),
        (Resource::Cpu, c.cpu_secs, "CPU time"),
        (Resource::Fsize, c.fsize_mib * MIB, "file size"),
        (Resource::Nofile, c.nofile, "open files"),
        (Resource::Core, 0, "core size"),
    ];
    for (resource, value, name) in limits {
        setrlimit(resource, Rlimit { current: Some(value), maximum: Some(value) })
            .map_err(|e| format!("cannot limit {name}: {e}"))?;
    }
    Ok(())
}
