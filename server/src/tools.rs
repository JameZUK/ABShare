//! Running ffmpeg and ffprobe, always through `abshare-jail` (SPEC D19, §5.4).

use crate::config::Ffmpeg;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

pub struct Limits {
    pub mem_mib: u64,
    pub cpu_secs: u64,
    pub fsize_mib: u64,
    pub wall: Duration,
    pub stdout_cap: usize,
}

impl Limits {
    pub const PROBE: Self =
        Self { mem_mib: 2048, cpu_secs: 60, fsize_mib: 1, wall: Duration::from_secs(90), stdout_cap: 4 * 1024 * 1024 };
}

pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    /// At most 64 KiB, for diagnostics shown only to admins.
    pub stderr: String,
}

/// How much of the program's standard error is kept; the rest is read and thrown away.
const STDERR_CAP: u64 = 64 * 1024;

/// Runs `program` jailed with read access to `read` and write access to `write`.
pub async fn run_jailed(
    ff: &Ffmpeg,
    read: &[&Path],
    write: Option<&Path>,
    program: &Path,
    args: &[String],
    limits: &Limits,
) -> Result<Output, String> {
    let mut cmd = tokio::process::Command::new(&ff.jail);
    for r in read {
        cmd.arg("--read").arg(r);
    }
    if let Some(w) = write {
        cmd.arg("--write").arg(w);
    }
    for l in &ff.libs {
        cmd.arg("--lib").arg(l);
    }
    cmd.args(["--mem-mib", &limits.mem_mib.to_string(), "--cpu-secs", &limits.cpu_secs.to_string()]);
    cmd.args(["--fsize-mib", &limits.fsize_mib.to_string()]);
    // Enough descriptors for every input (a book in 200 parts opens 200 files), bounded
    // (review M5).
    let nofile = (64 + 2 * read.len()).min(4_096);
    cmd.args(["--nofile", &nofile.to_string()]);
    cmd.arg("--").arg(program).args(args);
    cmd.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", ff.jail.display()))?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let cap = limits.stdout_cap;
    // Neither pipe is ever left unread while the program runs: a program blocked writing to
    // a full pipe can't exit, and would hold its lane until the wall-clock limit (review M3).
    // Standard error past its cap is read and discarded. Standard output past its cap is an
    // error, so reading stops there and the program is killed below.
    let read_out = async {
        let mut so = Vec::new();
        (&mut out).take(cap as u64 + 1).read_to_end(&mut so).await.map_err(|e| e.to_string())?;
        if so.len() > cap {
            return Err(format!("{} produced more than {cap} bytes", program.display()));
        }
        Ok(so)
    };
    let read_err = async {
        let mut se = Vec::new();
        (&mut err).take(STDERR_CAP).read_to_end(&mut se).await.map_err(|e| e.to_string())?;
        tokio::io::copy(&mut err, &mut tokio::io::sink()).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(se)
    };
    let work = async {
        let (so, se) = tokio::try_join!(read_out, read_err)?;
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((status, so, se))
    };
    let done = match tokio::time::timeout(limits.wall, work).await {
        Ok(done) => done,
        Err(_) => Err(format!("{} timed out", program.display())),
    };
    let (status, stdout, stderr) = match done {
        Ok(done) => done,
        Err(e) => {
            // Too much output, a failed read or the time limit: the program may still be
            // running. Kill it and wait for it, so it is gone before the caller goes on.
            let _ = child.kill().await;
            return Err(e);
        }
    };
    Ok(Output { success: status.success(), stdout, stderr: String::from_utf8_lossy(&stderr).into_owned() })
}

/// The forced input demuxer for a file extension (SPEC §5.4): HLS, concat and other
/// "meta" demuxers can never be selected by a file's contents.
pub fn demuxer_for(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "m4b" | "m4a" | "mp4" => "mov",
        "mp3" => "mp3",
        "ogg" | "oga" | "opus" => "ogg",
        "flac" => "flac",
        "wav" => "wav",
        _ => return None,
    })
}

/// For tests: the real jail (built by the workspace) around the system ffmpeg, or `None`
/// where it can't run (no Landlock ABI 6, or no ffmpeg). With ABSHARE_REQUIRE_LANDLOCK set,
/// not being able to run is a failure instead of a skip.
#[cfg(test)]
pub async fn test_ffmpeg() -> Option<Ffmpeg> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let ff = Ffmpeg {
        ffmpeg: "/usr/bin/ffmpeg".into(),
        ffprobe: "/usr/bin/ffprobe".into(),
        jail: target.join(profile).join("abshare-jail"),
        libs: ["/usr/lib", "/lib", "/lib64", "/etc/ld.so.cache"]
            .map(Into::into)
            .into_iter()
            .filter(|p: &std::path::PathBuf| p.exists())
            .collect(),
        jobs: 1,
    };
    let limits = Limits { mem_mib: 2048, cpu_secs: 10, fsize_mib: 1, wall: Duration::from_secs(20), stdout_cap: 4096 };
    let run = run_jailed(&ff, &[], None, &ff.ffmpeg, &["-hide_banner".into(), "-version".into()], &limits).await;
    let ok = ff.jail.exists() && run.as_ref().is_ok_and(|o| o.success);
    if !ok && std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_some() {
        let why = match &run {
            Ok(o) => o.stderr.trim().to_owned(),
            Err(e) => e.clone(),
        };
        panic!("ABSHARE_REQUIRE_LANDLOCK is set but the jailed ffmpeg cannot run ({}): {why}", ff.jail.display());
    }
    ok.then_some(ff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The jail's probe (built beside the jail by the workspace), as a program that is noisy
    /// or slow on demand. `None` where the jail can't run; with ABSHARE_REQUIRE_LANDLOCK set
    /// that is a failure instead of a skip.
    async fn probe() -> Option<(Ffmpeg, std::path::PathBuf)> {
        let ff = test_ffmpeg().await?;
        let probe = ff.jail.with_file_name("abshare-jail-probe");
        if !probe.exists() {
            assert!(
                std::env::var_os("ABSHARE_REQUIRE_LANDLOCK").is_none(),
                "ABSHARE_REQUIRE_LANDLOCK is set but {} is not built",
                probe.display()
            );
            return None;
        }
        Some((ff, probe))
    }

    fn limits(wall_secs: u64, stdout_cap: usize) -> Limits {
        Limits { mem_mib: 512, cpu_secs: 30, fsize_mib: 1, wall: Duration::from_secs(wall_secs), stdout_cap }
    }

    async fn run(ff: &Ffmpeg, probe: &Path, args: &[&str], limits: &Limits) -> (Result<Output, String>, Duration) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let started = Instant::now();
        let r = run_jailed(ff, &[], None, probe, &args, limits).await;
        (r, started.elapsed())
    }

    macro_rules! need_probe {
        () => {
            match probe().await {
                Some(p) => p,
                None => {
                    eprintln!("skipped: the jail cannot run here");
                    return;
                }
            }
        };
    }

    /// Review M3: a program that writes without end to standard output used to block in
    /// `write` and hold its lane until the wall-clock limit.
    #[tokio::test]
    async fn too_much_standard_output_is_an_error_at_once() {
        let (ff, probe) = need_probe!();
        let (r, took) = run(&ff, &probe, &["flood-out"], &limits(30, 4096)).await;
        assert_eq!(r.err().as_deref(), Some(format!("{} produced more than 4096 bytes", probe.display()).as_str()));
        assert!(took < Duration::from_secs(10), "took {took:?}: the program wasn't stopped");
    }

    #[tokio::test]
    async fn standard_output_is_accepted_up_to_the_cap_and_no_further() {
        let (ff, probe) = need_probe!();
        let (r, _) = run(&ff, &probe, &["out", "4096"], &limits(30, 4096)).await;
        let o = r.expect("exactly at the cap");
        assert!(o.success);
        assert_eq!(o.stdout.len(), 4096);
        let (r, _) = run(&ff, &probe, &["out", "4097"], &limits(30, 4096)).await;
        assert!(r.is_err_and(|e| e.contains("produced more than 4096 bytes")));
    }

    /// Review M3: a program that is very noisy on standard error used to block once the
    /// 64 KiB that are kept had been read.
    #[tokio::test]
    async fn a_program_noisy_on_standard_error_still_finishes() {
        let (ff, probe) = need_probe!();
        let (r, took) = run(&ff, &probe, &["noise-err", "8"], &limits(30, 4096)).await;
        let o = r.expect("the program finishes");
        assert!(o.success);
        assert_eq!(o.stdout, b"done\n");
        assert_eq!(o.stderr.len() as u64, STDERR_CAP, "only the first 64 KiB are kept");
        assert!(took < Duration::from_secs(10), "took {took:?}");
    }

    #[tokio::test]
    async fn the_wall_clock_limit_stops_a_slow_program() {
        let (ff, probe) = need_probe!();
        let (r, took) = run(&ff, &probe, &["sleep", "60"], &limits(1, 4096)).await;
        assert_eq!(r.err().as_deref(), Some(format!("{} timed out", probe.display()).as_str()));
        assert!(took >= Duration::from_secs(1) && took < Duration::from_secs(10), "took {took:?}");
        // A quick program under the same limit is fine.
        let (r, _) = run(&ff, &probe, &["sleep", "0"], &limits(5, 4096)).await;
        assert_eq!(r.expect("runs").stdout, b"ok\n");
    }
}
