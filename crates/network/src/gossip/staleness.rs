//! Gossip staleness primitives (INC-I-142, INV-NETWORK-003): the shared
//! raw-bytes identity [`SeenCache`], the [`StalenessCtx`] the relay table reads,
//! and the per-topic age bounds. The verdicts live in
//! [`relay_decision`](super::relay_decision); no path here ever `Reject`s.

use std::collections::{HashMap, VecDeque};

use crypto::Hasher;
use doli_core::Attestation;

use super::validation::wall_clock_slot_from;

/// SECONDARY age bound (in slots) for attestations. An attestation older than
/// `wall_slot - ATTEST_STALE_SLOTS` is `Ignore`d. 12 slots = 120s at
/// SLOT_DURATION=10s — deliberately GENEROUS. Since the design revision, identity
/// dedup ([`SeenCache`]) is the storm-closer; this bound's only job is to drop
/// truly-ancient attestations and cap the cache, so it stays wide enough that a
/// late-but-useful attestation backing a very recent block is never dropped.
pub const ATTEST_STALE_SLOTS: u32 = 12;

/// The attestation age bound; an unset clock (`genesis_time == 0`) fails open.
pub(super) fn attestation_is_current(
    a: &Attestation,
    genesis_time: u64,
    slot_duration: u64,
    now: u64,
) -> bool {
    let wall = wall_clock_slot_from(genesis_time, slot_duration, now);
    genesis_time == 0 || a.slot >= wall.saturating_sub(ATTEST_STALE_SLOTS)
}

/// SECONDARY age bound (seconds) for governance votes (M4). A vote whose
/// `timestamp` is older than `now_unix - VOTE_MAX_AGE_SECS` is `Ignore`d.
/// = 7 days: the auto-update GOVERNANCE VOTING WINDOW is `VETO_PERIOD`
/// (`crates/updater/src/constants.rs:12`) — network-aware (60s devnet, 300s
/// mainnet/testnet, documented target 7 days). A single compile-time bound
/// must safely EXCEED the window on every network so a vote still inside its
/// veto window is never dropped as stale. Age is only SECONDARY (identity
/// dedup closes the storm), so generous is correct.
pub const VOTE_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Shared [`SeenCache`] TTL (seconds). **Load-bearing:** MUST be ≥120s to cover
/// the full libp2p re-delivery escape window (`duplicate_cache` 60s →
/// `TIME_CACHE_DURATION` 120s). A shorter TTL reopens the storm. 180s gives margin
/// without approaching any topic's legitimate rebroadcast interval (the tx
/// rebroadcast-interval constraint is an M5 concern; M0–M3 topics are never
/// legitimately rebroadcast with an identical identity within 180s).
pub const SEEN_CACHE_TTL_SECS: u64 = 180;

/// Shared [`SeenCache`] capacity (drop-oldest bound).
///
/// RESOURCE COST: each entry ≈ 32-byte key (HashMap) + 8-byte timestamp + 32-byte
/// key copy (VecDeque order) + hashing/allocator overhead ≈ ~120 bytes. 16 384
/// entries ≈ **~2 MB** worst-case steady-state RAM — within the "a few MB" ceiling
/// (INV-NETWORK-002 forbids unbounded gossip heap growth). Real fleet volume
/// (≈40 producers × {attestation + heartbeat} per 10s slot × 18 TTL-slots ≈ 1 500
/// live entries) is far below the cap, so drop-oldest eviction effectively never
/// fires; the bound exists only to make unbounded growth structurally impossible.
pub const SEEN_CACHE_CAPACITY: usize = 16_384;

/// Transaction-topic identity-dedup TTL (seconds), M5 — the shared
/// [`SEEN_CACHE_TTL_SECS`] (180s). Transactions carry no embedded age, so
/// identity dedup is the WHOLE gate (no secondary filter).
/// **F-1 invariant `120s ≤ TX_SEEN_TTL < mempool_rebroadcast_interval`:** DOLI
/// has NO periodic mempool rebroadcast loop. Since INC-I-237 M2 a tx message
/// is Deferred: the mesh forwards it only after the node's Accept (every tx
/// newly admitted); no node re-publishes a received tx, and the origin
/// publishes each tx once as one legacy single-tx message.
/// The rebroadcast interval is effectively infinite, so `120s ≤ 180s < ∞`
/// holds with no per-topic override; after 180s a re-submitted txid re-propagates.
pub const TX_SEEN_TTL_SECS: u64 = SEEN_CACHE_TTL_SECS;

/// The five subscribed gossip topics (metric label + [`seen_key`] domain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GossipTopic {
    /// `/doli/blocks/1`
    Blocks,
    /// `/doli/producers/1`
    Producers,
    /// `/doli/attestations/1`
    Attestations,
    /// `/doli/votes/1`
    Votes,
    /// `/doli/txs/1`
    Transactions,
}

/// Read-only staleness context. Every field is already reachable at the gossip
/// handler (`handle_behaviour_event`) with zero new node→network plumbing.
pub struct StalenessCtx<'a> {
    /// Current Unix time in seconds (`now_unix_secs()`); 0 → clock unavailable.
    pub now_unix: u64,
    /// Genesis timestamp (Unix seconds); 0 → staleness disabled (fail-open).
    pub genesis_time: u64,
    /// Slot duration in seconds.
    pub slot_duration: u64,
    /// Local tip slot (`best_slot.load(Relaxed)`). Reserved for future height-
    /// window heuristics; staleness predicates use wall-clock slot, NOT this, so
    /// a lagging node never treats fresh gossip as stale (§7.1).
    pub best_slot: u32,
    /// Shared identity-dedup cache — PRIMARY storm-closer for ALL stateful topics.
    pub seen: &'a mut SeenCache,
}

/// Domain-separated shared-cache key = `BLAKE3(topic_discriminant || parts...)`.
///
/// Every identity-dedup topic passes the FULL RAW gossip message bytes as the sole
/// `part` (`seen_key(topic, &[data])`) — the gossipsub message-id equivalent with a
/// 180s app-side TTL. Keying on raw bytes (never extracted semantic sub-fields) is
/// what makes the cache SUPPRESSION-RESISTANT: a forged message that copies a
/// victim's semantic fields but flips any byte (e.g. a garbage signature — never
/// checked at this gate, and EXCLUDED from `tx.hash()`) produces a DIFFERENT key, so
/// it cannot pre-seed and suppress the genuine message (INC-I-142 SEC-LOGIC-001/002).
/// It also fixes the BLS/non-BLS collision (SEC-CONSENSUS-003) and the delimiter-free
/// concat ambiguity (SEC-LOGIC-003) for free: no field extraction, no delimiter.
/// Prefixing with the topic discriminant keeps identities from different topics from
/// colliding in the single shared [`SeenCache`].
pub(super) fn seen_key(topic: GossipTopic, parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(&[topic as u8]);
    for p in parts {
        hasher.update(p);
    }
    *hasher.finalize().as_bytes()
}

/// Bounded TTL + capacity identity dedup cache — the shared PRIMARY storm-closer
/// for attestations, votes and transactions.
///
/// **Capacity-capped (drop-oldest)** so it can never reintroduce the unbounded-
/// heap-growth shape the gossip-hardening invariant (INV-NETWORK-002) prevents.
/// See [`SEEN_CACHE_TTL_SECS`] / [`SEEN_CACHE_CAPACITY`] for the load-bearing TTL
/// and the RESOURCE COST bound.
pub struct SeenCache {
    ttl_secs: u64,
    capacity: usize,
    entries: HashMap<[u8; 32], u64>,
    order: VecDeque<[u8; 32]>,
}

impl SeenCache {
    /// Create an empty cache with the given TTL (seconds) and max capacity.
    pub fn new(ttl_secs: u64, capacity: usize) -> Self {
        Self {
            ttl_secs,
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// True iff `key` was recorded within TTL. Pure read — does NOT insert, so the
    /// classifier can honour "record identity on FIRST `Accept` only".
    pub fn contains(&self, key: &[u8; 32], now_unix: u64) -> bool {
        matches!(
            self.entries.get(key),
            Some(&seen_at) if now_unix.saturating_sub(seen_at) < self.ttl_secs
        )
    }

    /// Record `key` as seen at `now_unix`, enforcing capacity (drop-oldest).
    /// Call only on the Accept path (first sight of a genuinely-new message).
    pub fn record(&mut self, key: [u8; 32], now_unix: u64) {
        while self.order.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            } else {
                break;
            }
        }
        if self.entries.insert(key, now_unix).is_none() {
            self.order.push_back(key);
        }
    }

    /// Convenience combinator: returns whether `key` was already seen within TTL,
    /// recording it as a first sight otherwise. (Used by generic-dedup callers /
    /// tests; the classifiers use [`contains`](Self::contains) +
    /// [`record`](Self::record) so recording is gated on Accept.)
    pub fn check_and_insert(&mut self, key: [u8; 32], now_unix: u64) -> bool {
        let seen = self.contains(&key, now_unix);
        if !seen {
            self.record(key, now_unix);
        }
        seen
    }
}

#[cfg(test)]
#[path = "staleness_tests.rs"]
mod tests;
