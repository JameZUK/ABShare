//! Progress merging: whatever arrives, the stored position stays inside the book and the
//! stored time never goes backwards.
#![no_main]
use abshare_core::progress::{Action, Event, Outcome, Progress, merge};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|i: (u32, Vec<(u8, u64, i64, bool)>)| {
    let (dur, evs) = i;
    let dur = u64::from(dur);
    let now = 1_800_000_000_000i64;
    let mut state: Option<Progress> = None;
    for (kind, pos, t, seen) in evs.into_iter().take(200) {
        let action = match kind % 4 {
            0 => Action::Position(pos),
            1 => Action::MarkFinished,
            2 => Action::MarkUnfinished,
            _ => Action::Restart,
        };
        let before = state.map(|s| s.updated_at);
        if let Outcome::Accepted(p) = merge(state, Event { action, event_time: t }, dur, now, seen) {
            assert!(p.position_ms <= dur);
            assert!(before.is_none_or(|b| p.updated_at > b));
            assert!(p.updated_at <= now + abshare_core::progress::MAX_FUTURE_MS);
            state = Some(p);
        }
    }
});
