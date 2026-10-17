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
    cmd.arg("--").arg(program).args(args);
    cmd.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", ff.jail.display()))?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let cap = limits.stdout_cap;
    let work = async move {
        let mut so = Vec::new();
        let mut se = Vec::new();
        let mut out_lim = (&mut out).take(cap as u64 + 1);
        let mut err_lim = (&mut err).take(64 * 1024);
        let (r1, r2) = tokio::join!(out_lim.read_to_end(&mut so), err_lim.read_to_end(&mut se));
        r1.map_err(|e| e.to_string())?;
        r2.map_err(|e| e.to_string())?;
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((status, so, se))
    };
    let (status, stdout, stderr) =
        tokio::time::timeout(limits.wall, work).await.map_err(|_| format!("{} timed out", program.display()))??;
    if stdout.len() > cap {
        return Err(format!("{} produced more than {cap} bytes", program.display()));
    }
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
