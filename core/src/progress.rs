//! Listening-position merge rules (SPEC §5.6). Pure; property-tested.
//!
//! The server keeps one position per user per book. Clients report events stamped with
//! their own clock and an idempotency key. The newest event wins, within bounds that stop a
//! device with a wrong clock (or a malicious client) from rewriting history.

/// How far in the future (by the server's clock) an event may be stamped.
pub const MAX_FUTURE_MS: i64 = 5 * 60 * 1000;
/// How old an event may be (offline listening queued for up to this long).
pub const MAX_AGE_MS: i64 = 90 * 24 * 3600 * 1000;
/// Within this much of the end, a book counts as finished (less for short items: see
/// [`finish_margin`]).
pub const FINISH_MARGIN_MS: u64 = 30_000;

/// How close to the end counts as finished: 30 s, or 2% of a short item (a 3-minute episode
/// finishes in its last 3.6 s, not its last 30).
pub fn finish_margin(duration_ms: u64) -> u64 {
    FINISH_MARGIN_MS.min(duration_ms / 50)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub finished: bool,
    /// The client's event time of the event that set this (milliseconds since the epoch).
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Playback position report.
    Position(u64),
    /// The listener marked the book finished.
    MarkFinished,
    /// The listener marked it unfinished (position kept).
    MarkUnfinished,
    /// Start again from the beginning (clears finished).
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub action: Action,
    pub event_time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Stored as the new state.
    Accepted(Progress),
    /// Older than (or as old as) what is stored: nothing changes; here is the current state.
    Stale(Progress),
    /// The idempotency key was already used: nothing changes.
    Duplicate,
    TooFarInFuture,
    TooOld,
}

/// Applies an event. `seen_key` says whether its idempotency key was already recorded.
pub fn merge(stored: Option<Progress>, event: Event, duration_ms: u64, now: i64, seen_key: bool) -> Outcome {
    if seen_key {
        return Outcome::Duplicate;
    }
    if event.event_time > now.saturating_add(MAX_FUTURE_MS) {
        return Outcome::TooFarInFuture;
    }
    if event.event_time < now.saturating_sub(MAX_AGE_MS) {
        return Outcome::TooOld;
    }
    if let Some(s) = stored
        && event.event_time <= s.updated_at
    {
        return Outcome::Stale(s);
    }
    let prev = stored.unwrap_or(Progress { position_ms: 0, duration_ms, finished: false, updated_at: i64::MIN });
    let clamp = |p: u64| p.min(duration_ms);
    let next = match event.action {
        Action::Position(p) => {
            let position_ms = clamp(p);
            let near_end = duration_ms > 0 && position_ms + finish_margin(duration_ms) >= duration_ms;
            Progress { position_ms, duration_ms, finished: prev.finished || near_end, updated_at: event.event_time }
        }
        Action::MarkFinished => Progress { finished: true, duration_ms, updated_at: event.event_time, ..prev },
        Action::MarkUnfinished => Progress { finished: false, duration_ms, updated_at: event.event_time, ..prev },
        Action::Restart => Progress { position_ms: 0, duration_ms, finished: false, updated_at: event.event_time },
    };
    Outcome::Accepted(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const DUR: u64 = 10 * 3_600_000;

    fn pos(p: u64, t: i64) -> Event {
        Event { action: Action::Position(p), event_time: t }
    }

    fn accepted(o: Outcome) -> Progress {
        match o {
            Outcome::Accepted(p) => p,
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    #[test]
    fn a_newer_event_wins_and_an_older_one_is_stale() {
        let a = accepted(merge(None, pos(1000, NOW - 10_000), DUR, NOW, false));
        let b = accepted(merge(Some(a), pos(5000, NOW - 5_000), DUR, NOW, false));
        assert_eq!(b.position_ms, 5000);
        assert_eq!(merge(Some(b), pos(1, NOW - 7_000), DUR, NOW, false), Outcome::Stale(b));
        assert_eq!(merge(Some(b), pos(1, NOW - 5_000), DUR, NOW, false), Outcome::Stale(b));
    }

    #[test]
    fn duplicates_future_and_ancient_events_are_refused() {
        assert_eq!(merge(None, pos(1, NOW), DUR, NOW, true), Outcome::Duplicate);
        assert_eq!(merge(None, pos(1, NOW + MAX_FUTURE_MS + 1), DUR, NOW, false), Outcome::TooFarInFuture);
        assert!(matches!(merge(None, pos(1, NOW + MAX_FUTURE_MS), DUR, NOW, false), Outcome::Accepted(_)));
        assert_eq!(merge(None, pos(1, NOW - MAX_AGE_MS - 1), DUR, NOW, false), Outcome::TooOld);
        assert_eq!(merge(None, pos(1, i64::MIN), DUR, NOW, false), Outcome::TooOld);
        assert_eq!(merge(None, pos(1, i64::MAX), DUR, NOW, false), Outcome::TooFarInFuture);
    }

    #[test]
    fn positions_are_clamped_and_finishing_is_detected() {
        assert_eq!(accepted(merge(None, pos(u64::MAX, NOW), DUR, NOW, false)).position_ms, DUR);
        assert!(accepted(merge(None, pos(DUR - 29_000, NOW), DUR, NOW, false)).finished);
        assert!(!accepted(merge(None, pos(DUR - 31_000, NOW), DUR, NOW, false)).finished);
        // A short item isn't finished two thirds of the way through.
        assert!(!accepted(merge(None, pos(13_000, NOW), 20_000, NOW, false)).finished);
        assert!(accepted(merge(None, pos(19_700, NOW), 20_000, NOW, false)).finished);
        assert_eq!(finish_margin(DUR), FINISH_MARGIN_MS);
        assert_eq!(finish_margin(180_000), 3_600);
        assert_eq!(finish_margin(0), 0);
    }

    #[test]
    fn finished_sticks_until_unmarked_or_restarted() {
        let f = accepted(merge(None, Event { action: Action::MarkFinished, event_time: NOW - 3 }, DUR, NOW, false));
        let p = accepted(merge(Some(f), pos(10, NOW - 2), DUR, NOW, false));
        assert!(p.finished);
        let u =
            accepted(merge(Some(p), Event { action: Action::MarkUnfinished, event_time: NOW - 1 }, DUR, NOW, false));
        assert!(!u.finished && u.position_ms == 10);
        let r = accepted(merge(Some(f), Event { action: Action::Restart, event_time: NOW }, DUR, NOW, false));
        assert_eq!((r.position_ms, r.finished), (0, false));
    }

    /// Property: applying the same set of events in any order ends in the state of the
    /// newest event that is within bounds.
    #[test]
    fn order_of_arrival_does_not_matter() {
        let mut x: u64 = 0xdead_beef;
        for _ in 0..2_000 {
            let mut events: Vec<Event> = (0..8)
                .map(|i| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    pos(x % DUR, NOW - 1_000_000 + i * 1000 + (x % 500) as i64)
                })
                .collect();
            let newest = *events.iter().max_by_key(|e| e.event_time).expect("events");
            for round in 0..3 {
                if round > 0 {
                    events.rotate_left((x % 8) as usize);
                    events.swap(0, (x % 7) as usize + 1);
                }
                let mut state = None;
                for e in &events {
                    if let Outcome::Accepted(p) = merge(state, *e, DUR, NOW, false) {
                        state = Some(p);
                    }
                }
                let s = state.expect("state");
                assert_eq!(s.updated_at, newest.event_time);
                if let Action::Position(p) = newest.action {
                    assert_eq!(s.position_ms, p.min(DUR));
                }
            }
        }
    }
}
