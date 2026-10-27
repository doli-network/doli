//! Hashes removed by fork-remedy rollbacks (INC-I-281).
//!
//! A rollback removes index rows only; the bodies stay in the store. This is the
//! one owner of "which hashes did recent fork-remedy rollbacks remove", read by
//! the node's non-fork-choice intakes and by the post-rollback progress gates.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crypto::Hash;

use super::SyncManager;

/// Remembered hashes; consecutive rollbacks accumulate up to this many.
pub const ROLLBACK_MEMORY_CAP: usize = 64;
/// Lifetime of an entry, aligned with the 300 s post-rollback suppressor window.
pub const ROLLBACK_MEMORY_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
struct Removed {
    hash: Hash,
    at: Instant,
    hold_logged: bool,
}

/// Bounded memory of removed hashes plus the post-rollback progress bit.
#[derive(Debug, Default)]
pub struct RollbackMemory {
    removed: VecDeque<Removed>,
    /// A block that is not a remembered removed hash was applied since the
    /// most recent rollback (of any authority).
    fresh_apply_since_rollback: bool,
}

impl RollbackMemory {
    /// A fork-remedy rollback removed `hash`. Re-recording refreshes it.
    pub fn record(&mut self, hash: Hash, now: Instant) {
        self.removed.retain(|r| r.hash != hash);
        self.removed.push_back(Removed {
            hash,
            at: now,
            hold_logged: false,
        });
        while self.removed.len() > ROLLBACK_MEMORY_CAP {
            self.removed.pop_front();
        }
    }

    /// Any rollback completed: progress since it starts over.
    pub fn note_rollback(&mut self) {
        self.fresh_apply_since_rollback = false;
    }

    /// A block was applied (any path).
    pub fn note_applied(&mut self, hash: &Hash, now: Instant) {
        if self.removed_age(hash, now).is_none() {
            self.fresh_apply_since_rollback = true;
        }
    }

    /// Time since `hash` was removed, while it is remembered.
    pub fn removed_age(&self, hash: &Hash, now: Instant) -> Option<Duration> {
        self.removed
            .iter()
            .find(|r| r.hash == *hash)
            .map(|r| now.saturating_duration_since(r.at))
            .filter(|age| *age < ROLLBACK_MEMORY_TTL)
    }

    /// True the first time a hold of `hash` is reported (log once).
    pub fn first_hold(&mut self, hash: &Hash) -> bool {
        match self.removed.iter_mut().find(|r| r.hash == *hash) {
            Some(r) if !r.hold_logged => {
                r.hold_logged = true;
                true
            }
            _ => false,
        }
    }

    /// True if at least one of `applied` is not a remembered removed hash.
    pub fn any_not_removed(&self, applied: &[Hash], now: Instant) -> bool {
        applied.iter().any(|h| self.removed_age(h, now).is_none())
    }

    /// Rollback height for coordinator Gate 1: while only removed hashes were
    /// applied since the rollback, report no progress past the local height.
    pub fn gate1_rollback_height(&self, rb_h: Option<u64>, local_height: u64) -> Option<u64> {
        if self.fresh_apply_since_rollback {
            rb_h
        } else {
            rb_h.map(|h| h.max(local_height))
        }
    }
}

impl SyncManager {
    /// A fork-remedy rollback removed `hash` from the chain tip.
    pub fn note_fork_remedy_removed(&mut self, hash: Hash) {
        self.rollback_memory.record(hash, Instant::now());
    }

    /// Time since a fork-remedy rollback removed `hash`, while remembered.
    pub fn removed_by_rollback_age(&self, hash: &Hash) -> Option<Duration> {
        self.rollback_memory.removed_age(hash, Instant::now())
    }

    /// True the first time an intake holds `hash`.
    pub fn note_removed_block_held(&mut self, hash: &Hash) -> bool {
        self.rollback_memory.first_hold(hash)
    }

    /// Orphan accounting, except for a child of a remembered removed hash: the
    /// node left that branch on purpose, so it is not evidence against the tip.
    pub fn note_orphan_gossip_child(&mut self, parent: &Hash, need_height: u64, slot: u32) {
        if self.removed_by_rollback_age(parent).is_none() {
            self.note_orphan_gossip_block(need_height, slot);
        }
    }

    /// INC-I-138 D5 progress mark, narrowed: a re-apply of removed hashes only
    /// is not peer-confirmed progress.
    pub fn note_peer_applies_since_rollback(&mut self, applied: &[Hash]) {
        if self
            .rollback_memory
            .any_not_removed(applied, Instant::now())
        {
            self.note_peer_block_applied_since_rollback();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u8) -> Hash {
        crypto::hash::hash(&[n])
    }

    #[test]
    fn age_is_reported_until_the_ttl() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        m.record(h(1), t0);
        assert_eq!(m.removed_age(&h(1), t0), Some(Duration::ZERO));
        let late = t0 + ROLLBACK_MEMORY_TTL - Duration::from_secs(1);
        assert!(m.removed_age(&h(1), late).is_some());
        assert_eq!(m.removed_age(&h(1), t0 + ROLLBACK_MEMORY_TTL), None);
        assert_eq!(m.removed_age(&h(2), t0), None);
    }

    #[test]
    fn consecutive_rollbacks_accumulate_and_rerecord_refreshes() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        m.record(h(2), t0);
        m.record(h(1), t0 + Duration::from_secs(5));
        assert!(m.removed_age(&h(2), t0 + Duration::from_secs(6)).is_some());
        assert!(m.removed_age(&h(1), t0 + Duration::from_secs(6)).is_some());
        let t1 = t0 + Duration::from_secs(200);
        m.record(h(2), t1);
        assert_eq!(m.removed_age(&h(2), t1), Some(Duration::ZERO));
        assert_eq!(m.removed.len(), 2);
    }

    #[test]
    fn capacity_evicts_the_oldest() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        for i in 0..=ROLLBACK_MEMORY_CAP as u8 {
            m.record(h(i), t0);
        }
        assert_eq!(m.removed.len(), ROLLBACK_MEMORY_CAP);
        assert_eq!(m.removed_age(&h(0), t0), None);
        assert!(m.removed_age(&h(ROLLBACK_MEMORY_CAP as u8), t0).is_some());
    }

    #[test]
    fn hold_is_logged_once_per_record() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        assert!(!m.first_hold(&h(1)));
        m.record(h(1), t0);
        assert!(m.first_hold(&h(1)));
        assert!(!m.first_hold(&h(1)));
        m.record(h(1), t0);
        assert!(m.first_hold(&h(1)));
    }

    #[test]
    fn progress_ignores_removed_hashes() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        m.record(h(1), t0);
        assert!(!m.any_not_removed(&[h(1)], t0));
        assert!(!m.any_not_removed(&[], t0));
        assert!(m.any_not_removed(&[h(1), h(3)], t0));
        assert!(m.any_not_removed(&[h(1)], t0 + ROLLBACK_MEMORY_TTL));
    }

    #[test]
    fn gate1_height_only_narrows() {
        let t0 = Instant::now();
        let mut m = RollbackMemory::default();
        m.record(h(1), t0);
        m.note_rollback();
        assert_eq!(m.gate1_rollback_height(None, 7), None);
        m.note_applied(&h(1), t0);
        assert_eq!(m.gate1_rollback_height(Some(5), 6), Some(6));
        assert_eq!(m.gate1_rollback_height(Some(5), 4), Some(5));
        m.note_applied(&h(3), t0);
        assert_eq!(m.gate1_rollback_height(Some(5), 7), Some(5));
        m.note_rollback();
        assert_eq!(m.gate1_rollback_height(Some(5), 6), Some(6));
    }
}
