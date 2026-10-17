//! Encoder output for a data-saver segment: checked strictly, never panics, and an accepted
//! segment always has exactly the frames the layout expects.
#![no_main]
use abshare_core::saver::{Bitrate, Profile};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|i: (u8, u32, u16, &[u8])| {
    let (b, dur, seg, data) = i;
    let bitrate = [Bitrate::K32, Bitrate::K40, Bitrate::K48][b as usize % 3];
    let Ok(profile) = Profile::new(bitrate, 2_500) else { return };
    let Ok(layout) = profile.layout(u64::from(dur) * 1000 + 1) else { return };
    let Some(w) = layout.window(u64::from(seg) % layout.segment_count()) else { return };
    if let Ok(a) = layout.check_segment(&w, data) {
        let fb = bitrate.frame_bytes();
        assert!(a.end as usize <= data.len());
        assert_eq!((a.end - a.start) / fb + a.pad_frames, w.keep_frames);
    }
    let total = layout.total_bytes();
    let first = u64::from(dur) % total;
    if let Ok(end) = layout.cap_range(first, total - 1, 10) {
        assert!(end >= first && end < total);
    }
});
