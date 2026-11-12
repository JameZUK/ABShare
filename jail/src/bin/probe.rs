//! Test helper for the jail's escape tests: tries one action and reports the result.
//! Prints `ok` (exit 0) or `denied <errno>` (exit 3). Not installed.
//!
//! The server's tests of `run_jailed` use it too, as a program that is noisy or slow on
//! demand (`out`, `flood-out`, `noise-err`, `sleep`).

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// Writes `total` bytes (`None`: until writing fails) in 64 KiB pieces.
fn emit(mut to: impl Write, total: Option<u64>) -> std::io::Result<()> {
    let chunk = [b'x'; 64 * 1024];
    let mut left = total;
    loop {
        let n = left.map_or(chunk.len(), |l| l.min(chunk.len() as u64) as usize);
        if n == 0 {
            return to.flush();
        }
        to.write_all(&chunk[..n])?;
        left = left.map(|l| l - n as u64);
    }
}

/// The resource limits the jail set, as `name=soft/hard` (`-` for unlimited).
fn limits() -> String {
    use rustix::process::{Resource, getrlimit};
    let show = |v: Option<u64>| v.map_or("-".to_owned(), |v| v.to_string());
    [
        ("as", Resource::As),
        ("cpu", Resource::Cpu),
        ("fsize", Resource::Fsize),
        ("nofile", Resource::Nofile),
        ("core", Resource::Core),
    ]
    .map(|(name, r)| {
        let l = getrlimit(r);
        format!("{name}={}/{}", show(l.current), show(l.maximum))
    })
    .join(" ")
}

fn report(r: std::io::Result<()>) -> ExitCode {
    match r {
        Ok(()) => {
            println!("ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("denied {}", e.raw_os_error().unwrap_or(-1));
            ExitCode::from(3)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    match arg(0).as_str() {
        "read" => report(std::fs::read(arg(1)).map(drop)),
        "write" => report(std::fs::write(arg(1), b"x")),
        "big" => report(std::fs::write(arg(1), vec![0u8; 8 * 1024 * 1024])),
        "inet" => report(std::net::TcpListener::bind("127.0.0.1:0").map(drop)),
        "inet6" => report(std::net::TcpListener::bind("[::1]:0").map(drop)),
        "connect" => report(std::net::TcpStream::connect("127.0.0.1:9").map(drop)),
        "unix" => report(std::os::unix::net::UnixDatagram::unbound().map(drop)),
        "exec" => report(std::process::Command::new(arg(1)).status().map(drop)),
        "kill" => {
            let pid = arg(1).parse().ok().and_then(rustix::process::Pid::from_raw);
            report(match pid {
                Some(p) => rustix::process::kill_process(p, rustix::process::Signal::CONT).map_err(Into::into),
                None => Err(std::io::Error::other("bad pid")),
            })
        }
        "thread" => report(std::thread::spawn(|| 1).join().map(drop).map_err(|_| std::io::Error::other("join"))),
        // Resource limits and process attributes (review M17).
        "limits" => {
            println!("{}", limits());
            ExitCode::SUCCESS
        }
        "alloc" => {
            let bytes = arg(1).parse::<usize>().unwrap_or(0).saturating_mul(1024 * 1024);
            let mut v: Vec<u8> = Vec::new();
            report(v.try_reserve_exact(bytes).map_err(|_| std::io::Error::from_raw_os_error(libc::ENOMEM)))
        }
        "spin" => {
            // Burns CPU for a bounded time, so a missing CPU limit can't hang the tests.
            let until = Instant::now() + Duration::from_secs(arg(1).parse().unwrap_or(0));
            let mut x = 0u64;
            while Instant::now() < until {
                x = std::hint::black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
            }
            report(Ok(()))
        }
        "fds" => {
            let mut open = Vec::new();
            let r = (0..arg(2).parse::<u32>().unwrap_or(0))
                .try_for_each(|_| std::fs::File::open(arg(1)).map(|f| open.push(f)));
            report(r)
        }
        "privs" => {
            let nnp = rustix::thread::no_new_privs().unwrap_or(false);
            let filtered =
                matches!(rustix::thread::secure_computing_mode(), Ok(rustix::thread::SecureComputingMode::Filter));
            println!("no_new_privs={nnp} seccomp_filter={filtered}");
            ExitCode::SUCCESS
        }
        "memfd" => {
            report(rustix::fs::memfd_create("probe", rustix::fs::MemfdFlags::CLOEXEC).map(drop).map_err(Into::into))
        }
        // A noisy or slow program, for the server's `run_jailed` tests (review M3).
        // Exactly that many bytes on standard output, and nothing else.
        "out" => match emit(std::io::stdout().lock(), Some(arg(1).parse().unwrap_or(0))) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => report(Err(e)),
        },
        "flood-out" => report(emit(std::io::stdout().lock(), None)),
        "noise-err" => {
            let bytes = arg(1).parse::<u64>().unwrap_or(0).saturating_mul(1024 * 1024);
            match emit(std::io::stderr().lock(), Some(bytes)) {
                Ok(()) => {
                    println!("done");
                    ExitCode::SUCCESS
                }
                Err(e) => report(Err(e)),
            }
        }
        "sleep" => {
            std::thread::sleep(Duration::from_secs(arg(1).parse().unwrap_or(0)));
            report(Ok(()))
        }
        "env" => {
            println!("{}", std::env::vars_os().count());
            ExitCode::SUCCESS
        }
        _ => {
            println!("unknown");
            ExitCode::from(4)
        }
    }
}
