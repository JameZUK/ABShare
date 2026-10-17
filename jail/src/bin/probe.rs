//! Test helper for the jail's escape tests: tries one action and reports the result.
//! Prints `ok` (exit 0) or `denied <errno>` (exit 3). Not installed.

use std::process::ExitCode;

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
