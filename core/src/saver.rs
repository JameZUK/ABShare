//! Data-saver streaming maths (SPEC D23).
//!
//! A data-saver stream is a *virtual* constant-bitrate MP3: MPEG-2 Layer III, 24 kHz, mono,
//! no bit reservoir, no ID3 or Xing header. At 24 kHz every frame holds 576 samples (24 ms)
//! and a whole number of bytes with no padding, so byte offsets map exactly to time and the
//! full length is known before any audio exists. The server makes the stream in segments of
//! a fixed number of frames, each encoded independently with a pre-roll that is then thrown
//! away, so segments join without a gap or click.
//!
//! Everything here is arithmetic on frame and sample counts. The server turns a
//! [`SegmentWindow`] into an ffmpeg invocation and checks the bytes it gets back with
//! [`check_segment`].

use core::fmt;
use core::ops::RangeInclusive;

/// Output sample rate of every data-saver stream.
pub const SAMPLE_RATE: u32 = 24_000;
/// Samples in one MPEG-2 Layer III frame.
pub const SAMPLES_PER_FRAME: u64 = 576;
/// Samples of delay libmp3lame adds before the first real sample (576 encoder + 529 decoder).
pub const ENCODER_DELAY: u64 = 1_105;
/// Segment starts must fall on this grid: a common multiple of the frame length (576) and
/// the 22.05 kHz / 44.1 kHz → 24 kHz resampling period (160 output samples), so a fast seek
/// in the source lands on the same resampler phase as a decode from the start.
pub const ALIGN_SAMPLES: u64 = 2_880;
/// Frames encoded before each segment (except the first) and discarded.
pub const PREROLL_FRAMES: u64 = 45;
/// Samples encoded after each segment so the encoder flushes cleanly past its end.
pub const POSTROLL_SAMPLES: u64 = 2 * ALIGN_SAMPLES;
/// How far the produced frame count of the last segment may differ from the computed one
/// before the segment is refused (about 1.2 s). Small differences come from the source's
/// exact decoded length; larger ones mean something is wrong.
pub const MAX_TAIL_ADJUST_FRAMES: u64 = 50;

const _: () = assert!((PREROLL_FRAMES * SAMPLES_PER_FRAME).is_multiple_of(ALIGN_SAMPLES));
const _: () = assert!(POSTROLL_SAMPLES.is_multiple_of(ALIGN_SAMPLES));

/// Bitrates offered for data saver. Each gives a whole number of bytes per frame at 24 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bitrate {
    /// 32 kb/s: 96-byte frames, 14.4 MB per hour.
    K32,
    /// 40 kb/s: 120-byte frames, 18 MB per hour.
    K40,
    /// 48 kb/s: 144-byte frames, 21.6 MB per hour.
    K48,
}

impl Bitrate {
    /// Bits per second.
    pub const fn bits_per_second(self) -> u64 {
        match self {
            Self::K32 => 32_000,
            Self::K40 => 40_000,
            Self::K48 => 48_000,
        }
    }

    /// Bytes in one frame: 72 × bitrate / sample rate for MPEG-2 Layer III, exact here.
    pub const fn frame_bytes(self) -> u64 {
        72 * self.bits_per_second() / SAMPLE_RATE as u64
    }

    /// The MPEG-2 Layer III bitrate index carried in every frame header.
    const fn header_index(self) -> u8 {
        match self {
            Self::K32 => 4,
            Self::K40 => 5,
            Self::K48 => 6,
        }
    }

    /// The first three header bytes every frame must carry: sync, MPEG-2, Layer III, no CRC,
    /// this bitrate, 24 kHz, no padding, private bit clear.
    pub const fn header_prefix(self) -> [u8; 3] {
        [0xFF, 0xF3, (self.header_index() << 4) | 0x04]
    }
}

const _: () = assert!(Bitrate::K32.frame_bytes() * SAMPLE_RATE as u64 == 72 * 32_000);
const _: () = assert!(Bitrate::K40.frame_bytes() * SAMPLE_RATE as u64 == 72 * 40_000);
const _: () = assert!(Bitrate::K48.frame_bytes() * SAMPLE_RATE as u64 == 72 * 48_000);

/// Why a profile, book or range was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaverError {
    /// Segment length must be a positive multiple of 5 frames (so segment starts land on
    /// [`ALIGN_SAMPLES`]), at most 100 000 frames.
    BadSegmentLength,
    /// A book must be between 1 sample and 1000 hours long.
    BadDuration,
    /// The requested byte range is empty, reversed or past the end.
    Unsatisfiable,
    /// A produced segment didn't have the expected shape.
    BadSegment(SegmentFault),
}

impl fmt::Display for SaverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSegmentLength => f.write_str("segment length is not a positive multiple of 5 frames"),
            Self::BadDuration => f.write_str("duration out of range"),
            Self::Unsatisfiable => f.write_str("range not satisfiable"),
            Self::BadSegment(fault) => write!(f, "bad segment: {fault}"),
        }
    }
}

impl std::error::Error for SaverError {}

/// What was wrong with an encoder's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentFault {
    /// The byte count isn't a whole number of frames.
    PartialFrame { len: u64 },
    /// A frame header differs from the profile's.
    BadHeader { frame: u64 },
    /// Too few frames to skip the pre-roll and keep the segment.
    TooShort { have: u64, need: u64 },
    /// The last segment's length is further from the computed one than allowed.
    TailMismatch { have: u64, want: u64 },
}

impl fmt::Display for SegmentFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PartialFrame { len } => write!(f, "{len} bytes is not a whole number of frames"),
            Self::BadHeader { frame } => write!(f, "unexpected header at frame {frame}"),
            Self::TooShort { have, need } => write!(f, "{have} frames, needed {need}"),
            Self::TailMismatch { have, want } => write!(f, "last segment has {have} frames, expected {want}"),
        }
    }
}

/// An encoding profile: bitrate plus segment length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Profile {
    bitrate: Bitrate,
    frames_per_segment: u64,
}

impl Profile {
    /// The default: 32 kb/s, 2500-frame (60 s) segments.
    pub const DEFAULT: Self = Self { bitrate: Bitrate::K32, frames_per_segment: 2_500 };

    /// A profile with the given segment length in frames.
    pub fn new(bitrate: Bitrate, frames_per_segment: u64) -> Result<Self, SaverError> {
        // At least the pre-roll: a shorter segment would make its window start before zero
        // (review C5).
        if frames_per_segment < PREROLL_FRAMES || !frames_per_segment.is_multiple_of(5) || frames_per_segment > 100_000
        {
            return Err(SaverError::BadSegmentLength);
        }
        Ok(Self { bitrate, frames_per_segment })
    }

    pub const fn bitrate(&self) -> Bitrate {
        self.bitrate
    }

    pub const fn frames_per_segment(&self) -> u64 {
        self.frames_per_segment
    }

    pub const fn segment_bytes(&self) -> u64 {
        self.frames_per_segment * self.bitrate.frame_bytes()
    }

    /// Lays out a book of `duration_samples` samples at 24 kHz.
    pub fn layout(&self, duration_samples: u64) -> Result<Layout, SaverError> {
        const MAX: u64 = 1_000 * 3_600 * SAMPLE_RATE as u64;
        if duration_samples == 0 || duration_samples > MAX {
            return Err(SaverError::BadDuration);
        }
        let total_frames = (duration_samples + ENCODER_DELAY).div_ceil(SAMPLES_PER_FRAME);
        Ok(Layout { profile: *self, duration_samples, total_frames })
    }
}

/// Converts a duration in milliseconds to 24 kHz samples, rounding to the nearest sample.
pub fn samples_from_ms(ms: u64) -> u64 {
    (ms * u64::from(SAMPLE_RATE) + 500) / 1_000
}

/// One book's virtual stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    profile: Profile,
    duration_samples: u64,
    total_frames: u64,
}

impl Layout {
    pub const fn profile(&self) -> Profile {
        self.profile
    }

    pub const fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// The `Content-Length` of the whole virtual file.
    pub const fn total_bytes(&self) -> u64 {
        self.total_frames * self.profile.bitrate.frame_bytes()
    }

    pub fn segment_count(&self) -> u64 {
        self.total_frames.div_ceil(self.profile.frames_per_segment)
    }

    /// The segments holding bytes `first..=last` of the virtual file.
    pub fn segments_for(&self, first: u64, last: u64) -> Result<RangeInclusive<u64>, SaverError> {
        if first > last || last >= self.total_bytes() {
            return Err(SaverError::Unsatisfiable);
        }
        let seg = self.profile.segment_bytes();
        Ok(first / seg..=last / seg)
    }

    /// Caps a requested range so one response never makes the server encode more than
    /// `max_segments` segments (SPEC D23). Returns the last byte to send: at most `last`, and
    /// at most the end of the `max_segments`-th segment counted from the one holding `first`.
    /// A media element that gets a shorter 206 than it asked for simply asks again.
    pub fn cap_range(&self, first: u64, last: u64, max_segments: u64) -> Result<u64, SaverError> {
        let segs = self.segments_for(first, last)?;
        let max_segments = max_segments.max(1);
        let last_allowed_seg = segs.start().saturating_add(max_segments - 1);
        if *segs.end() <= last_allowed_seg {
            return Ok(last);
        }
        Ok(self.segment_offset(last_allowed_seg + 1) - 1)
    }

    /// Byte offset of a segment's first byte in the virtual file.
    pub fn segment_offset(&self, index: u64) -> u64 {
        index * self.profile.segment_bytes()
    }

    /// Frames segment `index` contributes to the virtual file (the last may be short).
    pub fn segment_frames(&self, index: u64) -> u64 {
        let start = index * self.profile.frames_per_segment;
        self.total_frames.saturating_sub(start).min(self.profile.frames_per_segment)
    }

    /// What to feed the encoder for segment `index`, or `None` past the end.
    pub fn window(&self, index: u64) -> Option<SegmentWindow> {
        if index >= self.segment_count() {
            return None;
        }
        let fps = self.profile.frames_per_segment;
        let start = index * fps * SAMPLES_PER_FRAME;
        let (input_start, skip_frames, seek) = if index == 0 {
            // Decoded from the start: seeking to 0 shifts the output by the encoder delay.
            (0, 0, false)
        } else {
            let preroll = PREROLL_FRAMES * SAMPLES_PER_FRAME;
            (start.saturating_sub(preroll), PREROLL_FRAMES, true)
        };
        let last = index + 1 == self.segment_count();
        let input_end = if last { None } else { Some(start + fps * SAMPLES_PER_FRAME + POSTROLL_SAMPLES) };
        Some(SegmentWindow {
            index,
            input_start,
            input_end,
            seek,
            skip_frames,
            keep_frames: self.segment_frames(index),
            last,
        })
    }

    /// What to feed the encoder for `count` consecutive segments from `first` in one run
    /// (downloads: one ffmpeg start-up for many segments). The output is checked with
    /// [`Self::check_segment`] like a single segment, then cut into segments of
    /// [`Self::segment_frames`] each. `None` if `first` is past the end or `count` is 0.
    pub fn span_window(&self, first: u64, count: u64) -> Option<SegmentWindow> {
        if count == 0 {
            return None;
        }
        let head = self.window(first)?;
        let last_index = first.saturating_add(count - 1).min(self.segment_count() - 1);
        let tail = self.window(last_index)?;
        let keep_frames = (first..=last_index).map(|i| self.segment_frames(i)).sum();
        Some(SegmentWindow { input_end: tail.input_end, keep_frames, last: tail.last, ..head })
    }

    /// Checks an encoder's output for one segment and says which bytes to keep.
    ///
    /// Every frame header is checked. A middle segment must have at least
    /// `skip + keep` frames. The last segment may differ from the computed length by up to
    /// [`MAX_TAIL_ADJUST_FRAMES`]: extra frames are dropped, missing ones are filled with
    /// the profile's silent frame by the caller (`pad_frames`).
    pub fn check_segment(&self, window: &SegmentWindow, encoded: &[u8]) -> Result<Accepted, SaverError> {
        let fault = |f| Err(SaverError::BadSegment(f));
        let fb = self.profile.bitrate.frame_bytes();
        let len = encoded.len() as u64;
        if !len.is_multiple_of(fb) {
            return fault(SegmentFault::PartialFrame { len });
        }
        let frames = len / fb;
        let prefix = self.profile.bitrate.header_prefix();
        for (i, frame) in encoded.chunks_exact(fb as usize).enumerate() {
            if frame[..3] != prefix || frame[3] & 0xC3 != 0xC0 {
                return fault(SegmentFault::BadHeader { frame: i as u64 });
            }
        }
        let need = window.skip_frames + window.keep_frames;
        if window.last {
            let have = frames.saturating_sub(window.skip_frames);
            if have.abs_diff(window.keep_frames) > MAX_TAIL_ADJUST_FRAMES || frames < window.skip_frames {
                return fault(SegmentFault::TailMismatch { have, want: window.keep_frames });
            }
            let kept = have.min(window.keep_frames);
            return Ok(Accepted {
                start: window.skip_frames * fb,
                end: (window.skip_frames + kept) * fb,
                pad_frames: window.keep_frames - kept,
            });
        }
        if frames < need {
            return fault(SegmentFault::TooShort { have: frames, need });
        }
        Ok(Accepted { start: window.skip_frames * fb, end: need * fb, pad_frames: 0 })
    }
}

/// The encoder input for one segment, in 24 kHz samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentWindow {
    pub index: u64,
    /// First input sample to encode (after resampling to 24 kHz).
    pub input_start: u64,
    /// One past the last input sample to encode; `None` means to the end of the source.
    pub input_end: Option<u64>,
    /// Whether to fast-seek the source (every segment but the first).
    pub seek: bool,
    /// Leading frames of the encoder output to discard.
    pub skip_frames: u64,
    /// Frames of the encoder output to keep after the skipped ones.
    pub keep_frames: u64,
    /// Whether this is the book's last segment.
    pub last: bool,
}

impl SegmentWindow {
    /// The source time to fast-seek to before decoding: 3 s before the window, so the
    /// decoder settles before the exact cut. In seconds.
    pub fn seek_seconds(&self) -> f64 {
        let lead = 3 * u64::from(SAMPLE_RATE);
        self.input_start.saturating_sub(lead) as f64 / f64::from(SAMPLE_RATE)
    }
}

/// Which bytes of an encoder's output to keep, and how many silent frames to append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub start: u64,
    pub end: u64,
    pub pad_frames: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3_600 * SAMPLE_RATE as u64;

    fn frame(b: Bitrate) -> Vec<u8> {
        let mut f = vec![0u8; b.frame_bytes() as usize];
        f[..3].copy_from_slice(&b.header_prefix());
        f[3] = 0xC4;
        f
    }

    fn frames(b: Bitrate, n: u64) -> Vec<u8> {
        (0..n).flat_map(|_| frame(b)).collect()
    }

    /// Deterministic xorshift, as lyrion-anywhere's robustness loops.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn frame_sizes_are_whole_bytes() {
        assert_eq!(Bitrate::K32.frame_bytes(), 96);
        assert_eq!(Bitrate::K40.frame_bytes(), 120);
        assert_eq!(Bitrate::K48.frame_bytes(), 144);
    }

    #[test]
    fn header_matches_what_libmp3lame_writes() {
        // Measured from libmp3lame output in the Phase 0 prototype: ff f3 44 c4.
        assert_eq!(Bitrate::K32.header_prefix(), [0xFF, 0xF3, 0x44]);
    }

    #[test]
    fn a_three_minute_book_has_the_frame_count_the_encoder_produced() {
        // The prototype's continuous encode of 180 s produced exactly 7502 frames.
        let l = Profile::DEFAULT.layout(180 * u64::from(SAMPLE_RATE)).expect("layout");
        assert_eq!(l.total_frames(), 7_502);
        assert_eq!(l.total_bytes(), 720_192);
        assert_eq!(l.segment_count(), 4);
    }

    #[test]
    fn segment_lengths_that_break_the_alignment_grid_are_refused() {
        // Shorter than the pre-roll is refused too (review C5).
        for bad in [0, 1, 4, 5, 40, 2_501, 100_005] {
            assert_eq!(Profile::new(Bitrate::K32, bad), Err(SaverError::BadSegmentLength), "{bad}");
        }
        assert!(Profile::new(Bitrate::K48, 45).is_ok());
        assert!(Profile::new(Bitrate::K48, 50).is_ok());
    }

    #[test]
    fn absurd_durations_are_refused() {
        assert_eq!(Profile::DEFAULT.layout(0), Err(SaverError::BadDuration));
        assert_eq!(Profile::DEFAULT.layout(1_001 * HOUR), Err(SaverError::BadDuration));
        assert!(Profile::DEFAULT.layout(1).is_ok());
    }

    #[test]
    fn every_seeking_segment_starts_on_the_alignment_grid() {
        let l = Profile::DEFAULT.layout(85 * HOUR).expect("layout");
        for i in 1..l.segment_count() {
            let w = l.window(i).expect("window");
            assert!(w.seek);
            assert_eq!(w.input_start % ALIGN_SAMPLES, 0, "segment {i}");
            assert_eq!(w.input_start + PREROLL_FRAMES * SAMPLES_PER_FRAME, i * 2_500 * SAMPLES_PER_FRAME);
        }
    }

    #[test]
    fn the_first_segment_never_seeks() {
        let w = Profile::DEFAULT.layout(HOUR).expect("layout").window(0).expect("window");
        assert_eq!((w.input_start, w.seek, w.skip_frames), (0, false, 0));
    }

    #[test]
    fn segments_cover_the_whole_file_exactly_once() {
        let mut rng = Rng(0x5eed);
        for _ in 0..2_000 {
            let fps = 45 + 5 * rng.below(1_000);
            let bitrate = [Bitrate::K32, Bitrate::K40, Bitrate::K48][rng.below(3) as usize];
            let p = Profile::new(bitrate, fps).expect("profile");
            let l = p.layout(1 + rng.below(90 * HOUR)).expect("layout");
            let frames: u64 = (0..l.segment_count()).map(|i| l.segment_frames(i)).sum();
            assert_eq!(frames, l.total_frames());
            assert!(l.window(l.segment_count()).is_none());
            let last = l.window(l.segment_count() - 1).expect("last");
            assert!(last.last && last.input_end.is_none());
        }
    }

    #[test]
    fn any_byte_range_maps_to_the_segments_that_hold_it() {
        let mut rng = Rng(0xabcdef);
        for _ in 0..10_000 {
            let l = Profile::DEFAULT.layout(1 + rng.below(90 * HOUR)).expect("layout");
            let total = l.total_bytes();
            let a = rng.below(total);
            let b = a + rng.below(total - a);
            let segs = l.segments_for(a, b).expect("range");
            assert!(l.segment_offset(*segs.start()) <= a);
            assert!(l.segment_offset(*segs.end() + 1) > b);
            assert!(*segs.end() < l.segment_count());
        }
    }

    #[test]
    fn ranges_past_the_end_or_reversed_are_unsatisfiable() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let t = l.total_bytes();
        assert_eq!(l.segments_for(t, t), Err(SaverError::Unsatisfiable));
        assert_eq!(l.segments_for(10, 9), Err(SaverError::Unsatisfiable));
        assert_eq!(l.segments_for(0, u64::MAX), Err(SaverError::Unsatisfiable));
        assert!(l.segments_for(0, t - 1).is_ok());
    }

    #[test]
    fn a_span_covers_its_segments_exactly() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let n = l.segment_count();
        let w = l.span_window(3, 4).expect("span");
        let (first, last) = (l.window(3).expect("w"), l.window(6).expect("w"));
        assert_eq!((w.index, w.input_start, w.seek, w.skip_frames), (3, first.input_start, true, first.skip_frames));
        assert_eq!((w.input_end, w.keep_frames, w.last), (last.input_end, 4 * 2_500, false));
        // From the start: no seek and no pre-roll, like segment 0 alone.
        let z = l.span_window(0, 2).expect("span");
        assert_eq!((z.seek, z.skip_frames, z.input_start), (false, 0, 0));
        // Running past the end stops at the last segment, which may be short.
        let t = l.span_window(n - 2, 10).expect("span");
        assert!(t.last && t.input_end.is_none());
        assert_eq!(t.keep_frames, l.segment_frames(n - 2) + l.segment_frames(n - 1));
        assert_eq!(l.span_window(n, 1), None);
        assert_eq!(l.span_window(0, 0), None);
        assert_eq!(l.span_window(u64::MAX, u64::MAX), None);
        // One segment is exactly that segment's window.
        assert_eq!(l.span_window(5, 1), l.window(5));
        // The checked output cuts into whole segments.
        let out = frames(Bitrate::K32, 45 + 4 * 2_500 + 7);
        let a = l.check_segment(&w, &out).expect("accepted");
        assert_eq!(a.end - a.start, 4 * 2_500 * 96);
    }

    #[test]
    fn a_middle_segment_keeps_exactly_its_frames_after_the_preroll() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let w = l.window(3).expect("window");
        let out = frames(Bitrate::K32, 45 + 2_500 + 10);
        let a = l.check_segment(&w, &out).expect("accepted");
        assert_eq!(a, Accepted { start: 45 * 96, end: (45 + 2_500) * 96, pad_frames: 0 });
    }

    #[test]
    fn a_short_middle_segment_is_refused() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let w = l.window(3).expect("window");
        let out = frames(Bitrate::K32, 45 + 2_499);
        assert!(matches!(l.check_segment(&w, &out), Err(SaverError::BadSegment(SegmentFault::TooShort { .. }))));
    }

    #[test]
    fn a_partial_frame_or_foreign_header_is_refused() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let w = l.window(1).expect("window");
        let mut out = frames(Bitrate::K32, 45 + 2_500);
        out.push(0);
        assert!(matches!(l.check_segment(&w, &out), Err(SaverError::BadSegment(SegmentFault::PartialFrame { .. }))));
        out.pop();
        // A 40 kb/s frame header in a 32 kb/s stream: wrong bitrate index.
        out[96 * 7 + 2] = Bitrate::K40.header_prefix()[2];
        assert_eq!(l.check_segment(&w, &out), Err(SaverError::BadSegment(SegmentFault::BadHeader { frame: 7 })));
        // Stereo instead of mono.
        let mut out = frames(Bitrate::K32, 45 + 2_500);
        out[96 * 9 + 3] = 0x04;
        assert_eq!(l.check_segment(&w, &out), Err(SaverError::BadSegment(SegmentFault::BadHeader { frame: 9 })));
    }

    #[test]
    fn an_id3_or_xing_header_is_refused() {
        let l = Profile::DEFAULT.layout(HOUR).expect("layout");
        let w = l.window(0).expect("window");
        let mut out = b"ID3".to_vec();
        out.resize(96, 0);
        out.extend(frames(Bitrate::K32, 2_500));
        assert!(matches!(l.check_segment(&w, &out), Err(SaverError::BadSegment(_))));
    }

    #[test]
    fn the_last_segment_is_trimmed_or_padded_within_bounds() {
        let l = Profile::DEFAULT.layout(HOUR + 12_345).expect("layout");
        let w = l.window(l.segment_count() - 1).expect("last");
        let want = w.keep_frames;
        let long = l.check_segment(&w, &frames(Bitrate::K32, 45 + want + 3)).expect("long");
        assert_eq!((long.end - long.start) / 96, want);
        let short = l.check_segment(&w, &frames(Bitrate::K32, 45 + want - 4)).expect("short");
        assert_eq!(short.pad_frames, 4);
        assert_eq!((short.end - short.start) / 96 + short.pad_frames, want);
        let far = frames(Bitrate::K32, 45 + want - MAX_TAIL_ADJUST_FRAMES - 1);
        assert!(matches!(l.check_segment(&w, &far), Err(SaverError::BadSegment(SegmentFault::TailMismatch { .. }))));
    }

    #[test]
    fn check_segment_never_panics_on_arbitrary_bytes() {
        let mut rng = Rng(0xfeed);
        let l = Profile::DEFAULT.layout(2 * HOUR).expect("layout");
        for _ in 0..3_000 {
            let w = l.window(rng.below(l.segment_count())).expect("window");
            let len = rng.below(400_000) as usize;
            let mut buf = vec![0u8; len];
            for b in &mut buf {
                *b = rng.next() as u8;
            }
            let _ = l.check_segment(&w, &buf);
        }
    }

    #[test]
    fn an_open_ended_range_is_capped_to_whole_segments() {
        let l = Profile::DEFAULT.layout(85 * HOUR).expect("layout");
        let seg = Profile::DEFAULT.segment_bytes();
        let total = l.total_bytes();
        // From the start of segment 100 to the end of the file: ten segments only.
        assert_eq!(l.cap_range(100 * seg, total - 1, 10), Ok(110 * seg - 1));
        // From the middle of segment 100: still ends at the end of segment 109.
        assert_eq!(l.cap_range(100 * seg + 7, total - 1, 10), Ok(110 * seg - 1));
        // A short range is left alone.
        assert_eq!(l.cap_range(5, 95, 10), Ok(95));
        // Near the end, the real end wins.
        assert_eq!(l.cap_range(total - 10, total - 1, 10), Ok(total - 1));
        // Zero is treated as one segment.
        assert_eq!(l.cap_range(0, total - 1, 0), Ok(seg - 1));
        assert_eq!(l.cap_range(total, total, 10), Err(SaverError::Unsatisfiable));
    }

    #[test]
    fn capped_ranges_always_stay_inside_the_request() {
        let mut rng = Rng(0xcafe);
        for _ in 0..10_000 {
            let l = Profile::DEFAULT.layout(1 + rng.below(90 * HOUR)).expect("layout");
            let total = l.total_bytes();
            let a = rng.below(total);
            let b = a + rng.below(total - a);
            let cap = 1 + rng.below(20);
            let end = l.cap_range(a, b, cap).expect("cap");
            assert!(end >= a && end <= b);
            let segs = l.segments_for(a, end).expect("segs");
            assert!(segs.end() - segs.start() < cap);
        }
    }

    #[test]
    fn milliseconds_round_to_the_nearest_sample() {
        assert_eq!(samples_from_ms(1_000), 24_000);
        assert_eq!(samples_from_ms(0), 0);
        assert_eq!(samples_from_ms(85 * 3_600_000), 85 * HOUR);
    }
}
