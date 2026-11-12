//! The built `abshare` binary from the outside: stopping on SIGTERM and SIGINT, what `serve`
//! and `check-config` refuse, the start-up warning, and the `health` command.
//!
//! Each test has its own data directory, an empty read-only library and a free loopback port.
//! Nothing here runs ffmpeg or needs the jail.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_abshare");

struct Site {
    dir: PathBuf,
    config: PathBuf,
    addr: SocketAddr,
}

impl Site {
    /// `head` is extra top-level configuration (it may override nothing set here).
    fn new(name: &str, head: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("abshare-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("library")).expect("mkdir");
        std::fs::create_dir_all(dir.join("data")).expect("mkdir");
        std::fs::set_permissions(dir.join("data"), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        std::fs::set_permissions(dir.join("library"), std::fs::Permissions::from_mode(0o555)).expect("chmod");
        // A port nothing else holds: bound, read, and released.
        let port = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
        let site = Self { config: dir.join("abshare.toml"), addr: SocketAddr::from(([127, 0, 0, 1], port)), dir };
        site.write(&format!("listen = \"127.0.0.1:{port}\"\n{head}"));
        site
    }

    fn write(&self, head: &str) {
        let text = format!(
            "{head}\ndata_dir = {data:?}\npublic_origin = \"http://localhost:{port}\"\n\
             [ffmpeg]\nffmpeg = \"/usr/bin/ffmpeg\"\nffprobe = \"/usr/bin/ffprobe\"\n\
             jail = \"/usr/local/bin/abshare-jail\"\n\
             [[library]]\nname = \"Audiobooks\"\npath = {lib:?}\n",
            data = self.dir.join("data").display().to_string(),
            lib = self.dir.join("library").display().to_string(),
            port = self.addr.port(),
        );
        std::fs::write(&self.config, text).expect("write config");
    }

    /// Root can write anywhere, so no library is read-only to it and `serve` rightly refuses
    /// to start: these tests can't run as root.
    fn library_is_read_only(&self) -> bool {
        let probe = self.dir.join("library/probe");
        let read_only = std::fs::write(&probe, b"").is_err();
        if !read_only {
            let _ = std::fs::remove_file(&probe);
            eprintln!("skipped: running as root, to whom every library is writable");
        }
        read_only
    }

    fn command(&self, cmd: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(cmd).arg("--config").arg(&self.config);
        c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    fn run(&self, cmd: &[&str]) -> Output {
        self.command(cmd).output().expect("run abshare")
    }

    /// Starts `serve` and waits until it answers.
    fn serve(&self) -> Child {
        let mut child = self.command(&["serve"]).spawn().expect("spawn abshare");
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.get("/healthz").is_none_or(|r| !r.starts_with("HTTP/1.1 200")) {
            if let Some(status) = child.try_wait().expect("wait") {
                panic!("serve exited early ({status}): {}", output(child).1);
            }
            assert!(Instant::now() < deadline, "serve did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        child
    }

    fn connect(&self) -> Option<TcpStream> {
        let s = TcpStream::connect_timeout(&self.addr, Duration::from_secs(2)).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        Some(s)
    }

    fn get(&self, path: &str) -> Option<String> {
        let mut s = self.connect()?;
        let port = self.addr.port();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n").ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).ok()?;
        Some(out)
    }

    /// A POST whose headers have arrived but whose body hasn't: a request in progress.
    fn half_sent_post(&self) -> TcpStream {
        let mut s = self.connect().expect("connect");
        let port = self.addr.port();
        write!(
            s,
            "POST /v1/auth/login/password HTTP/1.1\r\nHost: localhost:{port}\r\nOrigin: http://localhost:{port}\r\n\
             X-ABShare-Client: 1\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{"
        )
        .expect("write");
        // Long enough for the server to have read the headers and started on the request.
        std::thread::sleep(Duration::from_millis(300));
        s
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.dir.join("library"), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn signal(child: &Child, signal: rustix::process::Signal) {
    let pid = rustix::process::Pid::from_child(child);
    rustix::process::kill_process(pid, signal).expect("signal");
}

/// Waits for the process to exit by itself; kills it and fails if it doesn't in time.
fn exit_within(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("still running {limit:?} after the signal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn output(mut child: Child) -> (String, String) {
    let (mut out, mut err) = (String::new(), String::new());
    let _ = child.kill();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let _ = child.wait();
    (out, err)
}

#[test]
fn serve_stops_promptly_and_cleanly_on_sigterm_and_sigint() {
    for (name, sig) in [("term", rustix::process::Signal::TERM), ("int", rustix::process::Signal::INT)] {
        let site = Site::new(name, "");
        if !site.library_is_read_only() {
            return;
        }
        let mut child = site.serve();
        let asked = Instant::now();
        signal(&child, sig);
        let status = exit_within(&mut child, Duration::from_secs(3));
        // Killed by the signal's default action, the status would not be a success.
        assert!(status.success(), "{name}: a clean exit, not {status}");
        assert!(asked.elapsed() < Duration::from_secs(2), "{name}: took {:?}", asked.elapsed());
        let (out, _) = output(child);
        assert!(out.contains("stopped"), "{name}: {out}");
        assert!(site.connect().is_none(), "{name}: nothing listens any more");
    }
}

#[test]
fn a_stop_lets_a_request_in_progress_finish_and_takes_no_new_ones() {
    let site = Site::new("drain", "");
    if !site.library_is_read_only() {
        return;
    }
    let mut child = site.serve();
    let mut slow = site.half_sent_post();
    signal(&child, rustix::process::Signal::TERM);
    std::thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().expect("wait").is_none(), "the server waits for the request in progress");
    assert!(site.connect().is_none(), "new connections are refused while it drains");
    // The rest of the body arrives: the request is answered in full, then the server goes.
    slow.write_all(b"}").expect("write");
    let mut answer = String::new();
    slow.read_to_string(&mut answer).expect("read");
    let (head, body) = answer.split_once("\r\n\r\n").expect("a response");
    let length = head.lines().find_map(|l| l.strip_prefix("content-length: ")).expect("a length");
    assert!(head.starts_with("HTTP/1.1 ") && !body.is_empty(), "{answer}");
    assert_eq!(length.parse::<usize>().ok(), Some(body.len()), "the whole body arrived: {answer}");
    assert!(exit_within(&mut child, Duration::from_secs(3)).success());
    let (out, _) = output(child);
    assert!(out.contains("waiting up to 8 s for 1 requests in progress"), "{out}");
}

#[test]
fn a_stop_does_not_wait_for_ever() {
    let site = Site::new("bounded", "");
    if !site.library_is_read_only() {
        return;
    }
    let mut child = site.serve();
    // This body never arrives (the server itself would give up on it only after 15 s).
    let _stuck = site.half_sent_post();
    let asked = Instant::now();
    signal(&child, rustix::process::Signal::TERM);
    let status = exit_within(&mut child, Duration::from_secs(9) + Duration::from_millis(900));
    assert!(status.success(), "{status}");
    assert!(asked.elapsed() >= Duration::from_secs(7), "it waited the grace period: {:?}", asked.elapsed());
    let (out, _) = output(child);
    assert!(out.contains("stopped with 1 requests unfinished"), "{out}");
}

#[test]
fn a_second_signal_stops_at_once() {
    let site = Site::new("twice", "");
    if !site.library_is_read_only() {
        return;
    }
    let mut child = site.serve();
    let _stuck = site.half_sent_post();
    signal(&child, rustix::process::Signal::TERM);
    std::thread::sleep(Duration::from_millis(300));
    assert!(child.try_wait().expect("wait").is_none(), "draining");
    signal(&child, rustix::process::Signal::TERM);
    assert!(exit_within(&mut child, Duration::from_secs(3)).success());
}

#[test]
fn a_repeating_scan_stops_cleanly_on_sigterm() {
    let site = Site::new("scan", "");
    let mut child = site.command(&["scan", "--every", "60"]).spawn().expect("spawn");
    // The first scan (of nothing) ends with "took"; then it sleeps until the next one.
    let mut seen = String::new();
    let mut stdout = child.stdout.take().expect("stdout");
    let mut byte = [0u8; 1];
    while !seen.contains("took") {
        assert_eq!(stdout.read(&mut byte).expect("read"), 1, "scan ended early: {seen}");
        seen.push(byte[0] as char);
    }
    let asked = Instant::now();
    signal(&child, rustix::process::Signal::TERM);
    let status = exit_within(&mut child, Duration::from_secs(3));
    assert!(status.success(), "a clean exit, not {status}");
    assert!(asked.elapsed() < Duration::from_secs(2), "took {:?}", asked.elapsed());
}

#[test]
fn serve_refuses_a_library_it_can_write_to() {
    let site = Site::new("writable", "");
    std::fs::set_permissions(site.dir.join("library"), std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut child = site.command(&["serve"]).spawn().expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if Instant::now() > deadline {
            let _ = output(child);
            panic!("serve started with a writable library");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    let (_, err) = output(child);
    assert!(err.contains("read-only"), "{err}");
    assert!(site.connect().is_none(), "nothing was served");
}

#[test]
fn check_config_refuses_what_serve_would() {
    let site = Site::new("check", "");
    if !site.library_is_read_only() {
        return;
    }
    assert!(site.run(&["check-config"]).status.success(), "the plain configuration is fine");
    for (head, names) in [
        ("listen = \"localhost:8743\"", "listen:"),
        ("listen = \"8743\"", "listen:"),
        ("listen = \"127.0.0.1:8743\"\ntrusted_proxies = [\"proxy\"]", "trusted_proxies:"),
        ("listen = \"127.0.0.1:8743\"\ntrusted_proxies = [\"10.0.0.0/24\"]", "trusted_proxies:"),
    ] {
        site.write(head);
        for cmd in ["check-config", "serve"] {
            let out = site.run(&[cmd]);
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success() && err.contains(names), "{cmd} with {head}: {err}");
        }
    }
}

#[test]
fn a_loopback_listener_without_a_trusted_proxy_is_warned_about_at_start() {
    let warned = |err: &str| err.contains("warning") && err.contains("trusted_proxies") && err.contains("plain HTTP");
    let site = Site::new("warn", "");
    if !site.library_is_read_only() {
        return;
    }
    let out = site.run(&["check-config"]);
    assert!(out.status.success() && warned(&String::from_utf8_lossy(&out.stderr)), "check-config warns");
    let (_, err) = output(site.serve());
    assert!(warned(&err), "serve warns: {err}");

    let site = Site::new("nowarn", "trusted_proxies = [\"127.0.0.1\"]");
    let out = site.run(&["check-config"]);
    assert!(out.status.success() && !warned(&String::from_utf8_lossy(&out.stderr)), "no warning with a proxy set");
    // Through the "proxy" (this machine), a request needs its word that TLS was used.
    let child = site.command(&["serve"]).spawn().expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(20);
    while site.get("/healthz").is_none() {
        assert!(Instant::now() < deadline, "serve did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(site.get("/healthz").is_some_and(|r| r.starts_with("HTTP/1.1 403")), "plain HTTP is refused");
    assert!(site.run(&["health"]).status.success(), "health still works behind a same-host proxy");
    let (_, err) = output(child);
    assert!(!warned(&err), "{err}");
}

#[test]
fn health_reports_whether_the_server_answers() {
    let site = Site::new("health", "");
    if !site.library_is_read_only() {
        return;
    }
    let down = site.run(&["health"]);
    assert_eq!(down.status.code(), Some(1), "nothing is listening");
    assert!(String::from_utf8_lossy(&down.stderr).contains("health:"));
    let child = site.serve();
    assert_eq!(site.run(&["health"]).status.code(), Some(0), "the server is up");
    let _ = output(child);
    assert_eq!(site.run(&["health"]).status.code(), Some(1), "it has stopped");

    // Something else on the port, answering but not as ABShare would: not healthy.
    let other = std::net::TcpListener::bind(site.addr).expect("bind");
    let answer = std::thread::spawn(move || {
        let (mut s, _) = other.accept().expect("accept");
        let mut buf = [0u8; 512];
        let n = s.read(&mut buf).expect("read");
        let _ = s.write_all(b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\n\r\n");
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    assert_eq!(site.run(&["health"]).status.code(), Some(1), "a refusal is not health");
    let asked = answer.join().expect("join");
    let port = site.addr.port();
    assert!(asked.starts_with("GET /healthz HTTP/1.0\r\n") && asked.contains(&format!("Host: localhost:{port}\r\n")));
}
