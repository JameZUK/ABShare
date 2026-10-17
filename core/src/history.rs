//! Listening-history session rules (SPEC §5.6). Pure; property-tested.
//!
//! History is built from accepted position reports. A report either extends the listener's
//! latest session for the same book on the same device, or starts a new one. Time heard is
//! credited only when the report is close behind the last one and the position moved forward
//! at a believable rate, so seeks, pauses and a client's made-up numbers add nothing: each
//! report can add at most [`JOIN_GAP_MS`], and never more than the time that passed.

/// A report this soon after the session's last one extends it; later ones start a new session.
pub const JOIN_GAP_MS: i64 = 2 * 60 * 1000;
/// Positions moving faster than this many times real time are a seek, not listening.
pub const MAX_SPEED: u64 = 3;
/// How long sessions are kept: 13 months (SPEC §5.6).
pub const KEEP_MS: i64 = 395 * 24 * 3600 * 1000;
/// At most this many sessions are kept per person; the oldest go first.
pub const MAX_SESSIONS: i64 = 50_000;

/// The tail of the latest session for this book and device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tail {
    /// Event time of the session's last report (milliseconds since the epoch).
    pub ended_at: i64,
    /// Position at that report.
    pub position_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Extend the session: it now ends at `ended_at`, at `position_ms`, with `heard_ms` more.
    Extend { ended_at: i64, position_ms: u64, heard_ms: u64 },
    /// Start a new session at `at` and `position_ms`, with nothing heard yet.
    Start { at: i64, position_ms: u64 },
}

/// What a position report at `event_time` and `position_ms` does to the history.
pub fn step(tail: Option<Tail>, event_time: i64, position_ms: u64) -> Step {
    let start = Step::Start { at: event_time, position_ms };
    let Some(t) = tail else { return start };
    let gap = event_time.saturating_sub(t.ended_at);
    if gap <= 0 || gap > JOIN_GAP_MS {
        return start;
    }
    let gap = gap as u64;
    // Backwards (a rewind) or implausibly fast (a seek): the session goes on, nothing is added.
    let moved = position_ms.saturating_sub(t.position_ms);
    let heard_ms = if moved <= gap.saturating_mul(MAX_SPEED) { moved.min(gap) } else { 0 };
    Step::Extend { ended_at: event_time, position_ms, heard_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_800_000_000_000;

    fn tail(ended_at: i64, position_ms: u64) -> Option<Tail> {
        Some(Tail { ended_at, position_ms })
    }

    #[test]
    fn the_first_report_starts_a_session_with_nothing_heard() {
        assert_eq!(step(None, T, 5_000), Step::Start { at: T, position_ms: 5_000 });
    }

    #[test]
    fn steady_listening_is_credited_at_real_time() {
        assert_eq!(
            step(tail(T, 60_000), T + 15_000, 75_000),
            Step::Extend { ended_at: T + 15_000, position_ms: 75_000, heard_ms: 15_000 }
        );
        // Faster playback is still only the time that passed.
        assert_eq!(
            step(tail(T, 60_000), T + 15_000, 90_000),
            Step::Extend { ended_at: T + 15_000, position_ms: 90_000, heard_ms: 15_000 }
        );
        // Slower playback, or a pause inside the window, is the position moved.
        assert_eq!(
            step(tail(T, 60_000), T + 15_000, 66_000),
            Step::Extend { ended_at: T + 15_000, position_ms: 66_000, heard_ms: 6_000 }
        );
    }

    #[test]
    fn seeks_and_rewinds_add_nothing_but_keep_the_session() {
        assert_eq!(
            step(tail(T, 60_000), T + 15_000, 60_000 + 45_001),
            Step::Extend { ended_at: T + 15_000, position_ms: 105_001, heard_ms: 0 }
        );
        assert_eq!(
            step(tail(T, 60_000), T + 15_000, 30_000),
            Step::Extend { ended_at: T + 15_000, position_ms: 30_000, heard_ms: 0 }
        );
        // Exactly three times real time is still listening.
        assert_eq!(
            step(tail(T, 0), T + 10_000, 30_000),
            Step::Extend { ended_at: T + 10_000, position_ms: 30_000, heard_ms: 10_000 }
        );
    }

    #[test]
    fn a_long_gap_or_time_going_backwards_starts_again() {
        assert_eq!(step(tail(T, 0), T + JOIN_GAP_MS + 1, 1), Step::Start { at: T + JOIN_GAP_MS + 1, position_ms: 1 });
        assert!(matches!(step(tail(T, 0), T + JOIN_GAP_MS, 1), Step::Extend { heard_ms: 1, .. }));
        assert_eq!(step(tail(T, 0), T, 1), Step::Start { at: T, position_ms: 1 });
        assert_eq!(step(tail(T, 0), T - 1, 1), Step::Start { at: T - 1, position_ms: 1 });
    }

    #[test]
    fn extreme_values_do_not_overflow() {
        assert!(matches!(step(tail(i64::MIN, 0), i64::MAX, u64::MAX), Step::Start { .. }));
        assert!(matches!(step(tail(i64::MAX, u64::MAX), i64::MIN, 0), Step::Start { .. }));
        assert!(matches!(step(tail(i64::MAX - 1, 0), i64::MAX, u64::MAX), Step::Extend { heard_ms: 0, .. }));
    }

    /// Property: whatever the reports, a session's time heard never exceeds the time it spans,
    /// and no single report adds more than the join window.
    #[test]
    fn time_heard_never_exceeds_time_spanned() {
        let mut x: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..2_000 {
            let mut t = T;
            let mut session: Option<(i64, Tail, u64)> = None; // (start, tail, heard)
            for _ in 0..50 {
                t += (next() % 200_000) as i64 - 20_000;
                let pos = next() % 1_000_000;
                match step(session.map(|s| s.1), t, pos) {
                    Step::Start { at, position_ms } => session = Some((at, Tail { ended_at: at, position_ms }, 0)),
                    Step::Extend { ended_at, position_ms, heard_ms } => {
                        let (start, tail, heard) = session.expect("extend needs a session");
                        assert!(heard_ms <= JOIN_GAP_MS as u64);
                        assert!(heard_ms <= (ended_at - tail.ended_at) as u64);
                        assert!(ended_at > tail.ended_at);
                        let heard = heard + heard_ms;
                        assert!(heard <= (ended_at - start) as u64);
                        session = Some((start, Tail { ended_at, position_ms }, heard));
                    }
                }
            }
        }
    }
}
