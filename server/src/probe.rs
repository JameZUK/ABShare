//! Probing one file (SPEC §4.2 step 2): jailed ffprobe with a forced demuxer, its JSON parsed
//! into strictly typed structs with bounds, and a short trial decode whose stderr decides
//! whether the file is playable (ffmpeg can fail to decode and still exit 0).

use crate::config::Ffmpeg;
use crate::tools::{Limits, demuxer_for, run_jailed};
pub use abshare_core::ffprobe::{ProbeRecord, parse_ffprobe, to_probed};
use std::path::Path;
use std::time::Duration;

/// Probes `abs` (whose extension is `ext`) and trial-decodes it.
pub async fn probe(ff: &Ffmpeg, abs: &Path, ext: &str) -> ProbeRecord {
    match probe_inner(ff, abs, ext).await {
        Ok(r) => r,
        Err(problem) => ProbeRecord {
            container: String::new(),
            codec: String::new(),
            profile: String::new(),
            duration_ms: 0,
            bitrate: 0,
            sample_rate: 0,
            channels: 0,
            tags: vec![],
            chapters: vec![],
            has_cover: false,
            decodes: false,
            problem: Some(problem),
        },
    }
}

async fn probe_inner(ff: &Ffmpeg, abs: &Path, ext: &str) -> Result<ProbeRecord, String> {
    if matches!(ext, "aax" | "aa") {
        return Err("DRM-protected Audible file (.aax/.aa): not supported".into());
    }
    let demuxer = demuxer_for(ext).ok_or_else(|| format!("unsupported file type .{ext}"))?;
    let path = abs.to_string_lossy().into_owned();
    let args: Vec<String> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-f",
        demuxer,
        "-show_format",
        "-show_streams",
        "-show_chapters",
        "-of",
        "json",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .chain([path.clone()])
    .collect();
    let out = run_jailed(ff, &[abs], None, &ff.ffprobe, &args, &Limits::PROBE).await?;
    if !out.success {
        return Err(format!("ffprobe failed: {}", first_line(&out.stderr)));
    }
    let mut rec = parse_ffprobe(&out.stdout)?;
    if !container_matches(ext, &rec.container) {
        return Err(format!("contents ({}) don't match the .{ext} extension", rec.container));
    }
    if rec.profile.eq_ignore_ascii_case("xHE-AAC") {
        rec.problem = Some("xHE-AAC: FFmpeg can't decode it; re-download the book as AAC".into());
        return Ok(rec);
    }
    match trial_decode(ff, abs, demuxer, rec.duration_ms).await {
        Ok(()) => rec.decodes = true,
        Err(e) => rec.problem = Some(e),
    }
    Ok(rec)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(300).collect()
}

fn container_matches(ext: &str, format_name: &str) -> bool {
    let first = format_name.split(',').next().unwrap_or("");
    match ext {
        "m4b" | "m4a" | "mp4" => first == "mov",
        "mp3" => first == "mp3",
        "ogg" | "oga" | "opus" => first == "ogg",
        "flac" => first == "flac",
        "wav" => first == "wav",
        _ => false,
    }
}

/// Decodes 10 s from the start and 10 s from the middle. Any decoder error fails it.
async fn trial_decode(ff: &Ffmpeg, abs: &Path, demuxer: &str, duration_ms: u64) -> Result<(), String> {
    let path = abs.to_string_lossy().into_owned();
    let limits = Limits { mem_mib: 2048, cpu_secs: 60, fsize_mib: 1, wall: Duration::from_secs(90), stdout_cap: 1024 };
    let mid = duration_ms / 2000;
    for start in [0, mid] {
        // -ss and -t are input options here, so ffmpeg stops reading after 10 s of input.
        let args: Vec<String> = [
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-f",
            demuxer,
            "-ss",
            &start.to_string(),
            "-t",
            "10",
            "-i",
            &path,
            "-map",
            "0:a:0",
            "-f",
            "null",
            "-",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let out = run_jailed(ff, &[abs], None, &ff.ffmpeg, &args, &limits).await?;
        if !out.success || !out.stderr.trim().is_empty() {
            return Err(format!("trial decode at {start} s failed: {}", first_line(&out.stderr)));
        }
        if duration_ms < 30_000 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_must_match_extensions() {
        assert!(container_matches("m4b", "mov,mp4,m4a,3gp,3g2,mj2"));
        assert!(!container_matches("m4b", "hls"));
        assert!(!container_matches("mp3", "mov,mp4"));
        assert!(!container_matches("txt", "mp3"));
    }
}
