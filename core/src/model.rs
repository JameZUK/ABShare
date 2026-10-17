//! What the scanner knows about one audio file, independent of how it was probed.
//!
//! The server fills these in from jailed ffprobe output and a trial decode; `core` only
//! ever sees these types, so its rules can be tested without ffmpeg.

/// Tags as key/value pairs. Keys are compared case-insensitively; a key may repeat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags(Vec<(String, String)>);

impl Tags {
    /// At most this many tags are kept from one file.
    pub const MAX: usize = 256;

    pub fn new(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self(pairs.into_iter().take(Self::MAX).map(|(k, v)| (k.to_ascii_lowercase(), v)).collect())
    }

    /// The first non-blank value for any of `keys`, in the order the keys are given.
    pub fn get(&self, keys: &[&str]) -> Option<&str> {
        keys.iter()
            .find_map(|k| self.0.iter().find(|(key, v)| key == k && !v.trim().is_empty()).map(|(_, v)| v.as_str()))
    }
}

/// The container, as identified by probing (never by file extension).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Mp4,
    Mp3,
    Ogg,
    Flac,
    Wav,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    AacLc,
    AacHe,
    /// xHE-AAC / USAC: probes, but cannot be decoded (SPEC D10).
    XheAac,
    Mp3,
    Opus,
    Vorbis,
    Flac,
    Pcm,
    Other(String),
}

impl Codec {
    /// Whether every supported browser plays this codec natively in its container.
    pub fn browser_safe(&self) -> bool {
        matches!(self, Self::AacLc | Self::AacHe | Self::Mp3 | Self::Opus)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawChapter {
    pub start_ms: u64,
    pub end_ms: u64,
    pub title: String,
}

/// Everything the scanner learned about one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbedFile {
    /// Path relative to the library root, with `/` separators. Never shown to clients.
    pub rel_path: String,
    pub size: u64,
    pub container: Container,
    pub codec: Codec,
    pub duration_ms: u64,
    /// Average bit rate in bits per second.
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub tags: Tags,
    pub chapters: Vec<RawChapter>,
    pub has_cover: bool,
    /// A short trial decode finished with no decoder errors.
    pub decodes: bool,
}

impl ProbedFile {
    /// The file's name (last path component).
    pub fn file_name(&self) -> &str {
        self.rel_path.rsplit('/').next().unwrap_or(&self.rel_path)
    }

    /// The directory part of the path ("" for files at the library root).
    pub fn dir(&self) -> &str {
        self.rel_path.rsplit_once('/').map_or("", |(d, _)| d)
    }

    /// Whether the file can be played at all (directly or through a playable copy).
    pub fn playable(&self) -> bool {
        self.decodes && self.duration_ms > 0 && self.codec != Codec::XheAac
    }
}

/// Text from recognised sidecar files in a book's folder, already read by the server
/// (each capped at 64 KiB).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sidecars {
    pub desc: Option<String>,
    pub reader: Option<String>,
    /// Any other `.txt` in the folder (for "Read by …" lines).
    pub info: Vec<String>,
}
