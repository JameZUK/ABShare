//! Parsing ffprobe's JSON output (untrusted bytes) into a bounded record, and turning a
//! record into a [`ProbedFile`]. Pure: the server runs ffprobe (jailed) and hands the bytes
//! here, so this parser can be fuzzed without ffmpeg.

use crate::model::{Codec, Container, ProbedFile, RawChapter, Tags};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What is stored per file in the catalogue (so unchanged files are never probed again).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeRecord {
    pub container: String,
    pub codec: String,
    pub profile: String,
    pub duration_ms: u64,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub tags: Vec<(String, String)>,
    pub chapters: Vec<(u64, u64, String)>,
    pub has_cover: bool,
    pub decodes: bool,
    /// Why the file is unplayable, for admins.
    pub problem: Option<String>,
}

// The subset of ffprobe's JSON we read. Unknown fields are ignored; wrong types fail.
#[derive(Deserialize)]
struct FfOut {
    #[serde(default)]
    format: Option<FfFormat>,
    #[serde(default)]
    streams: Vec<FfStream>,
    #[serde(default)]
    chapters: Vec<FfChapter>,
}

#[derive(Deserialize)]
struct FfFormat {
    #[serde(default)]
    format_name: String,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct FfStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    channels: Option<u32>,
    #[serde(default)]
    bit_rate: Option<String>,
    #[serde(default)]
    disposition: BTreeMap<String, i64>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct FfChapter {
    #[serde(default)]
    start_time: String,
    #[serde(default)]
    end_time: String,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

/// Seconds as a decimal string → milliseconds, refusing negative, non-finite and absurd values.
fn secs_to_ms(s: &str) -> Option<u64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && (0.0..=3_600_000.0).contains(&v)).then(|| (v * 1000.0).round() as u64)
}

fn bounded_u32(s: Option<&str>, max: u32) -> u32 {
    s.and_then(|s| s.trim().parse::<u64>().ok()).filter(|&v| v <= u64::from(max)).map_or(0, |v| v as u32)
}

/// Parses ffprobe's JSON. Pure, so it can be fuzzed without ffprobe.
pub fn parse_ffprobe(json: &[u8]) -> Result<ProbeRecord, String> {
    let out: FfOut = serde_json::from_slice(json).map_err(|e| format!("ffprobe output: {e}"))?;
    let format = out.format.ok_or("ffprobe output has no format")?;
    let audio = out.streams.iter().find(|s| s.codec_type == "audio").ok_or("no audio stream")?;
    let has_cover = out
        .streams
        .iter()
        .any(|s| s.codec_type == "video" && s.disposition.get("attached_pic").copied().unwrap_or(0) == 1);
    let duration_ms = format.duration.as_deref().and_then(secs_to_ms).unwrap_or(0);
    // Format tags first, then the audio stream's (some containers carry them there). Bounded
    // and de-duplicated in linear time: a file with huge tag maps can't stall the scanner
    // (review C3).
    let ok = |(k, v): &(&String, &String)| k.len() <= 64 && v.len() <= 64 * 1024;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tags: Vec<(String, String)> = Vec::new();
    for (k, v) in format.tags.iter().chain(audio.tags.iter()).filter(|kv| ok(kv)) {
        if tags.len() >= Tags::MAX {
            break;
        }
        if seen.insert(k.to_ascii_lowercase()) {
            tags.push((k.clone(), v.clone()));
        }
    }
    let mut chapters: Vec<(u64, u64, String)> = out
        .chapters
        .iter()
        .take(10_000)
        .filter_map(|c| {
            let s = secs_to_ms(&c.start_time)?;
            let e = secs_to_ms(&c.end_time).unwrap_or(s);
            let t = c.tags.get("title").cloned().unwrap_or_default();
            Some((s, e, t.chars().take(512).collect()))
        })
        .collect();
    chapters.sort_by_key(|c| c.0);
    let bitrate = bounded_u32(audio.bit_rate.as_deref().or(format.bit_rate.as_deref()), 50_000_000);
    Ok(ProbeRecord {
        container: format.format_name.chars().take(64).collect(),
        codec: audio.codec_name.chars().take(32).collect(),
        profile: audio.profile.clone().unwrap_or_default().chars().take(32).collect(),
        duration_ms,
        bitrate,
        sample_rate: bounded_u32(audio.sample_rate.as_deref(), 768_000),
        channels: audio.channels.filter(|&c| c <= 64).map_or(0, |c| c as u8),
        tags,
        chapters,
        has_cover,
        decodes: false,
        problem: None,
    })
}

/// Turns a stored record into the type `core` works with.
/// The `Content-Type` to serve a file with, from the container ffprobe found (its
/// comma-separated `format_name`), never from the file name.
pub fn audio_content_type(container: &str) -> &'static str {
    let has = |name: &str| container.split(',').any(|c| c.trim() == name);
    if has("mov") || has("mp4") || has("m4a") {
        "audio/mp4"
    } else if has("mp3") {
        "audio/mpeg"
    } else if has("ogg") {
        "audio/ogg"
    } else if has("flac") {
        "audio/flac"
    } else if has("wav") {
        "audio/wav"
    } else {
        "application/octet-stream"
    }
}

pub fn to_probed(rel: &str, size: u64, r: &ProbeRecord) -> ProbedFile {
    let container = match r.container.split(',').next().unwrap_or("") {
        "mov" => Container::Mp4,
        "mp3" => Container::Mp3,
        "ogg" => Container::Ogg,
        "flac" => Container::Flac,
        "wav" => Container::Wav,
        _ => Container::Other,
    };
    let profile = r.profile.to_ascii_lowercase();
    let codec = match r.codec.as_str() {
        "aac" if profile == "xhe-aac" => Codec::XheAac,
        "aac" if profile.starts_with("he-aac") => Codec::AacHe,
        "aac" => Codec::AacLc,
        "mp3" => Codec::Mp3,
        "opus" => Codec::Opus,
        "vorbis" => Codec::Vorbis,
        "flac" => Codec::Flac,
        c if c.starts_with("pcm_") => Codec::Pcm,
        c => Codec::Other(c.to_owned()),
    };
    ProbedFile {
        rel_path: rel.to_owned(),
        size,
        container,
        codec,
        duration_ms: r.duration_ms,
        bitrate: r.bitrate,
        sample_rate: r.sample_rate,
        channels: r.channels,
        tags: Tags::new(r.tags.iter().cloned()),
        chapters: r
            .chapters
            .iter()
            .map(|(s, e, t)| RawChapter { start_ms: *s, end_ms: *e, title: t.clone() })
            .collect(),
        has_cover: r.has_cover,
        decodes: r.decodes,
    }
}

#[cfg(test)]
mod content_type_tests {
    use super::audio_content_type as ct;

    #[test]
    fn the_content_type_comes_from_the_probed_container() {
        assert_eq!(ct("mov,mp4,m4a,3gp,3g2,mj2"), "audio/mp4");
        assert_eq!(ct("mp3"), "audio/mpeg");
        assert_eq!(ct("ogg"), "audio/ogg");
        assert_eq!(ct("flac"), "audio/flac");
        assert_eq!(ct("wav"), "audio/wav");
        // Whole names only, and anything else is served as opaque bytes.
        assert_eq!(ct("mp3x"), "application/octet-stream");
        assert_eq!(ct("html"), "application/octet-stream");
        assert_eq!(ct(""), "application/octet-stream");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_tag_maps_are_merged_in_linear_time() {
        let n = 80_000;
        let fmt: String = (0..n).map(|i| format!("\"f{i}\":\"v\"")).collect::<Vec<_>>().join(",");
        let st: String = (0..n).map(|i| format!("\"s{i}\":\"v\"")).collect::<Vec<_>>().join(",");
        let json = format!(
            r#"{{"format":{{"format_name":"mp3","duration":"10","tags":{{{fmt}}}}},"streams":[{{"codec_type":"audio","codec_name":"mp3","tags":{{{st}}}}}]}}"#
        );
        let t = std::time::Instant::now();
        let r = parse_ffprobe(json.as_bytes()).expect("parse");
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "took {:?}", t.elapsed());
        assert_eq!(r.tags.len(), Tags::MAX);
    }

    const SAMPLE: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"aac","profile":"LC","codec_type":"audio","sample_rate":"22050","channels":2,"bit_rate":"62794","disposition":{"default":1,"attached_pic":0},"tags":{"language":"eng"}},
        {"index":1,"codec_name":"mjpeg","codec_type":"video","disposition":{"attached_pic":1}}
      ],
      "chapters": [
        {"id":1,"start_time":"226.416984","end_time":"1331.827982","tags":{"title":"Chapter One"}},
        {"id":0,"start_time":"0.000000","end_time":"226.416984","tags":{"title":"Opening Credits"}}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"45288.880000","bit_rate":"63701",
        "tags":{"title":"Some Book","artist":"An Author","composer":"A Reader","audible_asin":"B0TESTASIN"}}
    }"#;

    #[test]
    fn a_typical_ffprobe_answer_parses() {
        let r = parse_ffprobe(SAMPLE.as_bytes()).expect("parse");
        assert_eq!((r.codec.as_str(), r.profile.as_str(), r.sample_rate, r.channels), ("aac", "LC", 22_050, 2));
        assert_eq!(r.duration_ms, 45_288_880);
        assert!(r.has_cover);
        assert_eq!(r.chapters[0], (0, 226_417, "Opening Credits".into()));
        let p = to_probed("Some Book [B0TESTASIN]/x.m4b", 1, &r);
        assert_eq!((p.container, p.codec), (Container::Mp4, Codec::AacLc));
        assert_eq!(p.tags.get(&["composer"]), Some("A Reader"));
    }

    #[test]
    fn absurd_or_hostile_values_are_bounded() {
        let j = r#"{"streams":[{"codec_type":"audio","codec_name":"aac","sample_rate":"99999999999","channels":4000}],
                   "chapters":[{"start_time":"-5","end_time":"NaN"},{"start_time":"inf"},{"start_time":"1e300"}],
                   "format":{"format_name":"mov","duration":"-1"}}"#;
        let r = parse_ffprobe(j.as_bytes()).expect("parse");
        assert_eq!((r.sample_rate, r.channels, r.duration_ms), (0, 0, 0));
        assert!(r.chapters.is_empty());
    }

    #[test]
    fn missing_pieces_and_wrong_types_are_errors() {
        assert!(parse_ffprobe(b"not json").is_err());
        assert!(parse_ffprobe(br#"{"streams":[]}"#).is_err());
        assert!(parse_ffprobe(br#"{"format":{"format_name":"mov"},"streams":[{"codec_type":"video"}]}"#).is_err());
        assert!(parse_ffprobe(br#"{"format":{"format_name":5},"streams":[]}"#).is_err());
    }

    #[test]
    fn xhe_aac_is_recognised() {
        let j = r#"{"streams":[{"codec_type":"audio","codec_name":"aac","profile":"xHE-AAC"}],"format":{"format_name":"mov","duration":"10"}}"#;
        let r = parse_ffprobe(j.as_bytes()).expect("parse");
        assert_eq!(to_probed("a.m4b", 1, &r).codec, Codec::XheAac);
    }

    #[test]
    fn parse_never_panics_on_arbitrary_bytes() {
        let mut x: u64 = 5;
        let bits: [&[u8]; 12] =
            [b"{", b"}", b"[", b"]", b"\"", b":", b",", b"format", b"streams", b"1", b"-", b"\\u0000"];
        for _ in 0..20_000 {
            let mut v = Vec::new();
            for _ in 0..(x % 40) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.extend_from_slice(bits[(x % 12) as usize]);
            }
            let _ = parse_ffprobe(&v);
        }
    }
}
