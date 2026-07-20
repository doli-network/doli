//! Unified application-level gossip staleness gate (INC-I-142, INV-NETWORK-003).
//!
//! # Why this module exists
//!
//! libp2p re-delivers a still-circulating gossip message to the application once
//! its `duplicate_cache` (60s) expires but before its peer-score
//! `TIME_CACHE_DURATION` (120s) evicts it. On re-delivery the application either
//! `Accept`s (→ re-forward to the mesh) or `Ignore`s (→ dropped, no forward, no
//! peer penalty). Historically only BLOCKS/TIER1/PRODUCERS applied a semantic
//! staleness gate; the five remaining re-forward-risk topics returned
//! unconditional `Accept`, so re-forwarding was gated only by cache timing — the
//! INC-I-142 self-sustaining duplicate storm.
//!
//! # The structural fix (design revision 2026-07-20 — identity dedup PRIMARY)
//!
//! Every re-forward-risk topic is modelled as a variant of [`GossipTopic`], and
//! all classification routes through [`classify_gossip`], whose match is
//! **exhaustive with no wildcard arm** — "Accept by default" is structurally
//! unreachable.
//!
//! An **age-only** gate cannot close the storm: with the `duplicate_cache_time`
//! backstop (M9) intentionally skipped, libp2p re-delivers at age 60–120s. An age
//! window wide enough to accept genuinely-late messages (attestations, 120s) is
//! therefore *also* wide enough to re-accept a 60–120s re-delivery → the storm
//! stays open. The revision promotes **identity dedup** to the PRIMARY mechanism:
//! a re-delivery is always "already-seen" (→ `Ignore`) while a genuinely-new late
//! message is not (→ `Accept`). The semantic age filter is retained as a
//! SECONDARY bound whose only job is to drop truly-ancient messages and cap the
//! [`SeenCache`]; it is deliberately kept generous and no longer load-bearing for
//! storm closure.
//!
//! Each `classify_*_gossip` runs, in order:
//! 1. **Decode** → on failure `Accept` (fail-open, never `Reject`).
//! 2. **IDENTITY dedup (PRIMARY).** If the identity key is already in the shared
//!    [`SeenCache`] within TTL → `Ignore`. This closes the re-delivery storm
//!    independent of `duplicate_cache_time`.
//! 3. **SEMANTIC age filter (SECONDARY).** If the slot is far past the acceptance
//!    window → `Ignore` (bounds the cache; kept generous).
//! 4. Otherwise **record identity on this first Accept** and `Accept` (+forward
//!    exactly once).
//!
//! # Correctness rule (INV-NETWORK-003, carried into every classifier)
//!
//! A message that is genuinely new to this node MUST still `Accept`+forward
//! exactly once; only fully-stale / already-known messages may be `Ignore`d.
//! Every classifier **fails open to `Accept`** on decode failure or clock-
//! unavailable (`genesis_time == 0`). No classifier ever returns `Reject`:
//! `Reject` applies a P4 peer-score penalty and can graylist honest catching-up
//! peers (INC-I-016 eviction cascade). Headers decode via
//! [`BlockHeader::deserialize`], NEVER `Block::deserialize` (P0-001): header bytes
//! fail `Block` decode → would `Reject` → mesh-expulsion cascade (INV-NETWORK-002).

use std::collections::{HashMap, VecDeque};

use crypto::Hasher;
use doli_core::{Attestation, BlockHeader, Heartbeat};
use libp2p::gossipsub::MessageAcceptance;

use super::validation::{
    classify_block_gossip, classify_producer_gossip, wall_clock_slot_from,
    STALE_BLOCK_SLOT_THRESHOLD,
};
use super::{
    ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC, PRODUCERS_TOPIC,
    TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// SECONDARY age bound (in slots) for attestations. An attestation older than
/// `wall_slot - ATTEST_STALE_SLOTS` is `Ignore`d. 12 slots = 120s at
/// SLOT_DURATION=10s — deliberately GENEROUS. Since the design revision, identity
/// dedup ([`SeenCache`]) is the storm-closer; this bound's only job is to drop
/// truly-ancient attestations and cap the cache, so it stays wide enough that a
/// late-but-useful attestation backing a very recent block is never dropped.
pub const ATTEST_STALE_SLOTS: u32 = 12;

/// SECONDARY age bound (in slots) for heartbeats. A heartbeat older than
/// `wall_slot - HEARTBEAT_STALE_SLOTS` is `Ignore`d. Kept at 6 slots = 60s (equal
/// to the block staleness window) rather than the tighter 3/30s: with identity
/// dedup now closing the storm, a tight age bound buys nothing and only risks
/// dropping a current heartbeat under minor clock skew. Generous is correct here.
pub const HEARTBEAT_STALE_SLOTS: u32 = 6;

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
/// has NO mempool rebroadcast loop — `BroadcastTransaction` is emitted only on
/// RPC submission and batched/flushed every 100ms; downstream nodes propagate
/// via the libp2p mesh, never re-publishing from the mempool. The rebroadcast
/// interval is effectively infinite, so `120s ≤ 180s < ∞` holds with no
/// per-topic override; after 180s a re-submitted txid re-propagates.
pub const TX_SEEN_TTL_SECS: u64 = SEEN_CACHE_TTL_SECS;

/// Every gossip topic DOLI subscribes to that carries re-forward risk.
///
/// Exhaustive by construction — [`classify_gossip`] matches on this enum with no
/// wildcard, so a future topic cannot silently default to `Accept`.
///
/// Note: the dynamic per-region block topics produced by
/// [`region_topic`](super::region_topic) are intentionally NOT modelled here —
/// they have no static `*_TOPIC` const and are not statically subscribed.
///
/// The `u8` discriminant is used as the domain-separation prefix in
/// [`seen_key`], so identities from different topics can never collide in the
/// shared [`SeenCache`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GossipTopic {
    /// `/doli/blocks/1` — block bodies (existing staleness gate).
    Blocks,
    /// `/doli/t1/blocks/1` — Tier 1 dense-mesh block bodies (existing gate).
    Tier1Blocks,
    /// `/doli/producers/1` — producer announcements GSet (existing gate).
    Producers,
    /// `/doli/attestations/1` — finality-gadget attestations (identity + age).
    Attestations,
    /// `/doli/heartbeats/1` — presence heartbeats (identity + age).
    Heartbeats,
    /// `/doli/headers/1` — lightweight block headers (identity + age).
    Headers,
    /// `/doli/votes/1` — auto-update governance votes (timestamp gate, M4).
    Votes,
    /// `/doli/txs/1` — transactions (identity seen-cache gate, M5).
    Transactions,
}

impl GossipTopic {
    /// Total function: topic string → `Option<Self>`.
    ///
    /// `None` means the string is not a re-forward-risk subscribed topic (e.g. a
    /// dynamic region topic or an unknown string); the caller handles it per
    /// local policy without routing through a staleness classifier.
    pub fn from_topic_str(topic: &str) -> Option<Self> {
        match topic {
            BLOCKS_TOPIC => Some(Self::Blocks),
            TIER1_BLOCKS_TOPIC => Some(Self::Tier1Blocks),
            PRODUCERS_TOPIC => Some(Self::Producers),
            ATTESTATION_TOPIC => Some(Self::Attestations),
            HEARTBEATS_TOPIC => Some(Self::Heartbeats),
            HEADERS_TOPIC => Some(Self::Headers),
            VOTES_TOPIC => Some(Self::Votes),
            TRANSACTIONS_TOPIC => Some(Self::Transactions),
            _ => None,
        }
    }
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
fn seen_key(topic: GossipTopic, parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(&[topic as u8]);
    for p in parts {
        hasher.update(p);
    }
    *hasher.finalize().as_bytes()
}

/// THE single classification entry point. Exhaustive match — no wildcard
/// `Accept`. Block/Producer arms delegate to the existing, unchanged validation
/// functions; the five newer arms delegate to per-topic classifiers.
pub fn classify_gossip(
    topic: GossipTopic,
    data: &[u8],
    ctx: &mut StalenessCtx<'_>,
) -> MessageAcceptance {
    match topic {
        GossipTopic::Blocks | GossipTopic::Tier1Blocks => {
            classify_block_gossip(data, ctx.genesis_time, ctx.slot_duration, ctx.now_unix).0
        }
        GossipTopic::Producers => classify_producer_gossip(data, ctx.now_unix),
        GossipTopic::Attestations => classify_attestation_gossip(data, ctx),
        GossipTopic::Heartbeats => classify_heartbeat_gossip(data, ctx),
        GossipTopic::Headers => classify_header_gossip(data, ctx),
        GossipTopic::Votes => classify_vote_gossip(data, ctx),
        GossipTopic::Transactions => classify_transaction_gossip(data, ctx),
        // NO wildcard arm — a new variant forces a new classifier here.
    }
}

/// Classify a gossiped [`Attestation`] for forwarding (INC-I-142, M1).
///
/// Order: PRIMARY raw-bytes identity dedup ([`seen_key`] over the full message —
/// the storm-closer AND suppression-resistant, SEC-LOGIC-001) → fail-open on
/// clock/decode → SECONDARY generous age bound → record identity on first Accept.
/// Never `Reject`s.
pub fn classify_attestation_gossip(data: &[u8], ctx: &mut StalenessCtx<'_>) -> MessageAcceptance {
    // PRIMARY: raw-bytes identity dedup — closes the 60–120s re-delivery storm
    // independent of libp2p's duplicate_cache_time, and (unlike a semantic key) a
    // forged variant with garbage-but-different bytes gets a DIFFERENT key, so it
    // cannot suppress the genuine attestation (SEC-LOGIC-001 / SEC-CONSENSUS-001/003).
    let key = seen_key(GossipTopic::Attestations, &[data]);
    if ctx.seen.contains(&key, ctx.now_unix) {
        return MessageAcceptance::Ignore;
    }
    // fail-open: clock unavailable → Accept, but record so a byte-identical
    // re-delivery is still Ignored (record-on-first-Accept, uniform across paths).
    if ctx.genesis_time == 0 {
        ctx.seen.record(key, ctx.now_unix);
        return MessageAcceptance::Accept;
    }
    // SECONDARY: generous age bound needs the decoded slot; fail-open on decode.
    let attestation = match Attestation::from_bytes(data) {
        Some(a) => a,
        None => {
            ctx.seen.record(key, ctx.now_unix);
            return MessageAcceptance::Accept; // fail-open, never Reject
        }
    };
    let wall_slot = wall_clock_slot_from(ctx.genesis_time, ctx.slot_duration, ctx.now_unix);
    if attestation.slot < wall_slot.saturating_sub(ATTEST_STALE_SLOTS) {
        // truly-ancient: Ignore WITHOUT recording — the same bytes re-decode stale.
        return MessageAcceptance::Ignore;
    }
    // First Accept → record identity so any re-delivery is Ignored.
    ctx.seen.record(key, ctx.now_unix);
    MessageAcceptance::Accept
}

/// Classify a gossiped [`Heartbeat`] for forwarding (INC-I-142, M2).
///
/// Same shape as [`classify_attestation_gossip`]: raw-bytes identity dedup (a forged
/// `(producer, slot)` variant with a different signature byte cannot suppress the
/// genuine heartbeat, SEC-LOGIC-002) → fail-open → SECONDARY age bound → record.
pub fn classify_heartbeat_gossip(data: &[u8], ctx: &mut StalenessCtx<'_>) -> MessageAcceptance {
    let key = seen_key(GossipTopic::Heartbeats, &[data]);
    if ctx.seen.contains(&key, ctx.now_unix) {
        return MessageAcceptance::Ignore;
    }
    if ctx.genesis_time == 0 {
        ctx.seen.record(key, ctx.now_unix);
        return MessageAcceptance::Accept;
    }
    let heartbeat = match Heartbeat::deserialize(data) {
        Some(h) => h,
        None => {
            ctx.seen.record(key, ctx.now_unix);
            return MessageAcceptance::Accept; // fail-open, never Reject
        }
    };
    let wall_slot = wall_clock_slot_from(ctx.genesis_time, ctx.slot_duration, ctx.now_unix);
    if heartbeat.slot < wall_slot.saturating_sub(HEARTBEAT_STALE_SLOTS) {
        return MessageAcceptance::Ignore;
    }
    ctx.seen.record(key, ctx.now_unix);
    MessageAcceptance::Accept
}

/// Classify a gossiped [`BlockHeader`] for forwarding (INC-I-142, M3).
///
/// Raw-bytes identity dedup (unified onto [`seen_key`] over the message bytes for
/// consistency with the other topics — equivalent to the old `header.hash()` key for
/// storm closure since header bytes uniquely determine the hash, and headers were
/// already suppression-safe). **P0-001:** the SECONDARY age filter decodes via
/// [`BlockHeader::deserialize`], NEVER `Block::deserialize` — header bytes fail
/// `Block` decode, which would `Reject` → P4 penalty → mesh-expulsion cascade
/// (INV-NETWORK-002). Decode failure fails OPEN to `Accept`.
pub fn classify_header_gossip(data: &[u8], ctx: &mut StalenessCtx<'_>) -> MessageAcceptance {
    let key = seen_key(GossipTopic::Headers, &[data]);
    if ctx.seen.contains(&key, ctx.now_unix) {
        return MessageAcceptance::Ignore;
    }
    if ctx.genesis_time == 0 {
        ctx.seen.record(key, ctx.now_unix);
        return MessageAcceptance::Accept;
    }
    let header = match BlockHeader::deserialize(data) {
        Some(h) => h,
        None => {
            ctx.seen.record(key, ctx.now_unix);
            return MessageAcceptance::Accept; // fail-open, never Reject (P0-001)
        }
    };
    let wall_slot = wall_clock_slot_from(ctx.genesis_time, ctx.slot_duration, ctx.now_unix);
    if header.slot < wall_slot.saturating_sub(STALE_BLOCK_SLOT_THRESHOLD) {
        return MessageAcceptance::Ignore;
    }
    ctx.seen.record(key, ctx.now_unix);
    MessageAcceptance::Accept
}

/// Minimal deserialization mirror of the ONLY governance-vote field the SECONDARY
/// age filter needs: `timestamp`. Identity dedup keys on the RAW message bytes
/// ([`seen_key`]), not extracted semantic fields, so `version`/`vote`/`producer_id`
/// are no longer decoded here — removing them closes SEC-LOGIC-002: a forged vote
/// copying a victim's `(producer_id, version, vote)` but with a garbage signature
/// has different bytes → different key → cannot suppress the genuine vote. The
/// `network` layer treats votes as opaque bytes and must NOT depend on the
/// higher-level `updater` crate. Unknown fields are ignored; a missing/renamed
/// `timestamp` makes decode fail → fail-open `Accept`.
#[derive(serde::Deserialize)]
struct VoteAge {
    timestamp: u64,
}

/// Classify a gossiped governance vote for forwarding (INC-I-142, M4).
/// Order: PRIMARY raw-bytes identity dedup (suppression-resistant) → decode
/// `timestamp` for the SECONDARY age bound (`VOTE_MAX_AGE_SECS`) → record on first
/// Accept. Fails open on JSON decode failure or `timestamp == 0`. Never `Reject`s.
pub fn classify_vote_gossip(data: &[u8], ctx: &mut StalenessCtx<'_>) -> MessageAcceptance {
    let key = seen_key(GossipTopic::Votes, &[data]);
    if ctx.seen.contains(&key, ctx.now_unix) {
        return MessageAcceptance::Ignore;
    }
    let vote = match serde_json::from_slice::<VoteAge>(data) {
        Ok(v) => v,
        Err(_) => {
            ctx.seen.record(key, ctx.now_unix);
            return MessageAcceptance::Accept; // fail-open, never Reject
        }
    };
    // Cannot judge age without a timestamp → fail-open Accept, but still record the
    // identity so any byte-identical re-delivery is Ignored — keeps "record on first
    // Accept" uniform and closes the timestamp==0 re-forward residual (review F2).
    if vote.timestamp == 0 {
        ctx.seen.record(key, ctx.now_unix);
        return MessageAcceptance::Accept;
    }
    if ctx.now_unix > vote.timestamp.saturating_add(VOTE_MAX_AGE_SECS) {
        return MessageAcceptance::Ignore;
    }
    ctx.seen.record(key, ctx.now_unix);
    MessageAcceptance::Accept
}

/// Classify a gossiped transaction message for forwarding (INC-I-142, M5).
/// Transactions carry no embedded age, so BATCH-LEVEL raw-bytes identity dedup is
/// the WHOLE gate: the storm is byte-identical batch re-delivery. Keying on the raw
/// message bytes (NOT per-tx `tx.hash()`, which is SegWit-style and EXCLUDES the
/// signature, `transaction/core.rs:491`) is what makes it suppression-resistant — a
/// forged same-txid variant with a different signature byte has different message
/// bytes → different key → cannot pre-seed and censor the genuine tx (SEC-LOGIC-001).
/// No decode needed. A byte-identical batch re-seen within [`TX_SEEN_TTL_SECS`] is
/// `Ignore`d; anything else (incl. undecodable bytes) is a first sight → `Accept` +
/// record. Never `Reject`s.
pub fn classify_transaction_gossip(data: &[u8], ctx: &mut StalenessCtx<'_>) -> MessageAcceptance {
    let key = seen_key(GossipTopic::Transactions, &[data]);
    if ctx.seen.contains(&key, ctx.now_unix) {
        return MessageAcceptance::Ignore;
    }
    ctx.seen.record(key, ctx.now_unix);
    MessageAcceptance::Accept
}

/// Bounded TTL + capacity identity dedup cache — the shared PRIMARY storm-closer
/// for all stateful topics (attestations, heartbeats, headers; votes/txs later).
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
