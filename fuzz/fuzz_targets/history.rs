//! Listening-history sessions: whatever reports arrive, a session never credits more time
//! than it spans, and no report adds more than the join window.
#![no_main]
use abshare_core::history::{JOIN_GAP_MS, Step, Tail, step};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|evs: Vec<(i64, u64)>| {
    let mut session: Option<(i64, Tail, u64)> = None;
    for (t, pos) in evs.into_iter().take(500) {
        match step(session.map(|s| s.1), t, pos) {
            Step::Start { at, position_ms } => {
                assert_eq!((at, position_ms), (t, pos));
                session = Some((at, Tail { ended_at: at, position_ms }, 0));
            }
            Step::Extend { ended_at, position_ms, heard_ms } => {
                let (start, tail, heard) = session.expect("extend needs a session");
                assert!(ended_at > tail.ended_at && ended_at - tail.ended_at <= JOIN_GAP_MS);
                assert!(heard_ms <= (ended_at - tail.ended_at) as u64);
                let heard = heard + heard_ms;
                assert!(i128::from(heard) <= i128::from(ended_at) - i128::from(start));
                session = Some((start, Tail { ended_at, position_ms }, heard));
            }
        }
    }
});
