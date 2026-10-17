//! Which cached files to evict when the cache is over its size limit: least recently used
//! first, never anything in use, down to a low-water mark so eviction doesn't run on every
//! write. Pure: the server measures and deletes.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<K> {
    pub key: K,
    pub bytes: u64,
    /// When it was last used (seconds); older goes first.
    pub last_used: i64,
    /// In use right now (a copy someone is listening to): never evicted.
    pub pinned: bool,
}

/// The keys to delete so the total fits in `limit` bytes, evicting down to 90% of it.
/// Returns nothing while the total is within the limit.
pub fn evict<K: Clone>(entries: &[Entry<K>], limit: u64) -> Vec<K> {
    let total: u64 = entries.iter().map(|e| e.bytes).sum();
    if total <= limit {
        return Vec::new();
    }
    let target = limit / 10 * 9;
    let mut order: Vec<&Entry<K>> = entries.iter().filter(|e| !e.pinned).collect();
    order.sort_by_key(|e| e.last_used);
    let mut left = total;
    let mut out = Vec::new();
    for e in order {
        if left <= target {
            break;
        }
        left = left.saturating_sub(e.bytes);
        out.push(e.key.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(key: &'static str, bytes: u64, last_used: i64, pinned: bool) -> Entry<&'static str> {
        Entry { key, bytes, last_used, pinned }
    }

    #[test]
    fn nothing_goes_while_under_the_limit() {
        assert!(evict(&[e("a", 50, 1, false), e("b", 50, 2, false)], 100).is_empty());
        assert!(evict::<&str>(&[], 0).is_empty());
    }

    #[test]
    fn least_recently_used_goes_first_down_to_ninety_percent() {
        let entries = [e("new", 40, 30, false), e("old", 40, 10, false), e("mid", 40, 20, false)];
        // 120 > 100: evict to <= 90: dropping "old" leaves 80.
        assert_eq!(evict(&entries, 100), ["old"]);
        // 120 > 50: evict to <= 45: "old", "mid" leave 40.
        assert_eq!(evict(&entries, 50), ["old", "mid"]);
    }

    #[test]
    fn pinned_entries_are_never_evicted_even_if_still_over() {
        let entries = [e("playing", 500, 1, true), e("a", 10, 2, false)];
        assert_eq!(evict(&entries, 100), ["a"]);
        assert!(evict(&[e("playing", 500, 1, true)], 100).is_empty());
    }

    #[test]
    fn a_zero_limit_clears_everything_unpinned() {
        let entries = [e("a", 1, 1, false), e("b", 1, 2, false), e("c", 1, 3, true)];
        assert_eq!(evict(&entries, 0), ["a", "b"]);
    }
}
