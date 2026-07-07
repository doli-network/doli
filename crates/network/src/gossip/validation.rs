//! Gossipsub message validation for block staleness filtering (INC-I-114).
//!
//! When `validate_messages` is enabled on the gossipsub config, every received
//! message is held un-forwarded until the application reports a verdict via
//! `report_message_validation_result`. This module provides the classification
//! logic that determines whether a gossiped block should be accepted (forwarded
//! to mesh peers), ignored (dropped silently without penalty), or rejected
//! (dropped with P4 peer-score penalty).
//!
//! # Staleness Rule
//!
//! A gossiped block is **stale** if its slot is older than
//! `current_wall_clock_slot - STALE_BLOCK_SLOT_THRESHOLD`. Stale blocks are
//! classified as `Ignore` (not `Reject`) because stale-but-honest gossip from
//! catching-up peers should not incur peer-score penalties — doing so risks
//! eviction cascades (INC-I-016).
//!
//! The staleness check uses **wall clock time** (not local best slot) to avoid
//! penalizing fresh blocks when the local node is behind. Lagging nodes receive
//! history via sync request/response, never gossip.

use doli_core::{decode_producer_set, Block};
use libp2p::gossipsub::MessageAcceptance;
use tracing::warn;

/// Maximum age (in slots) for a gossiped block to be considered "fresh" and
/// forwarded to mesh peers. Blocks older than `current_wall_clock_slot -
/// STALE_BLOCK_SLOT_THRESHOLD` are classified as `Ignore` (not forwarded, no
/// penalty).
///
/// 6 slots = 60 seconds at SLOT_DURATION=10s. This is generous enough to
/// tolerate minor clock skew and propagation delay, but tight enough to prevent
/// the INC-I-114 amplification storm where blocks 3-9 hours old were
/// re-forwarded through the mesh after dedup cache expiry.
pub const STALE_BLOCK_SLOT_THRESHOLD: u32 = 6;

/// Classify a deserialized block for gossipsub validation.
///
/// This is a pure function: no side effects, no I/O, no network access.
/// The caller deserializes the block once and passes it here for staleness
/// classification. This avoids double-deserialization on the hot path
/// (P1-001: classify + handler used to each deserialize independently).
///
/// # Classification Rules
///
/// - **Stale block** (slot < current_slot - STALE_BLOCK_SLOT_THRESHOLD):
///   `Ignore` — suppress forwarding without penalty (INC-I-016 safety).
/// - **Fresh or slightly-future block**: `Accept` — forward normally.
///
/// # Arguments
///
/// * `block` - A deserialized `Block` reference.
/// * `current_slot` - Current wall-clock slot derived from genesis timestamp.
///
/// # Returns
///
/// `MessageAcceptance` verdict for `report_message_validation_result`.
pub fn classify_block(block: &Block, current_slot: u32) -> MessageAcceptance {
    let block_slot = block.header.slot;

    // A block is stale if its slot is more than STALE_BLOCK_SLOT_THRESHOLD
    // slots behind the current wall-clock slot. This uses saturating_sub to
    // handle the case where current_slot < STALE_BLOCK_SLOT_THRESHOLD (early
    // chain, slot 0-5).
    let cutoff = current_slot.saturating_sub(STALE_BLOCK_SLOT_THRESHOLD);
    if block_slot < cutoff {
        return MessageAcceptance::Ignore;
    }

    MessageAcceptance::Accept
}

/// Classify raw gossipsub block message bytes for validation.
///
/// Thin wrapper around [`classify_block`] that handles deserialization.
/// Returns `Reject` for undeserializable bytes (garbage/truncated data),
/// which triggers a P4 peer-score penalty on the sender.
///
/// When `genesis_time == 0`, staleness filtering is disabled (fail-open) and
/// the function returns `Accept` for any deserializable block. This prevents
/// silent gossip death if a caller forgets to set genesis_time on NetworkConfig
/// (P1-002). A rate-limited warning is emitted.
///
/// # Arguments
///
/// * `data` - Raw serialized block bytes from the gossipsub message.
/// * `genesis_time` - Genesis timestamp (Unix seconds). 0 = staleness disabled.
/// * `slot_duration` - Slot duration in seconds.
/// * `now_unix` - Current Unix timestamp in seconds (caller-provided for testability, P2-003).
///
/// # Returns
///
/// A tuple of `(MessageAcceptance, Option<Block>)`. The `Option<Block>` is
/// `Some` when deserialization succeeded, allowing the caller to reuse the
/// block without re-deserializing (P1-001).
pub fn classify_block_gossip(
    data: &[u8],
    genesis_time: u64,
    slot_duration: u64,
    now_unix: u64,
) -> (MessageAcceptance, Option<Block>) {
    let block = match Block::deserialize(data) {
        Some(b) => b,
        None => return (MessageAcceptance::Reject, None),
    };

    // P1-002: If genesis_time is 0 (unset), skip staleness check entirely.
    // Gossip liveness > filter strictness on misconfiguration. Log once-ish.
    if genesis_time == 0 {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            warn!(
                "[GOSSIP_VALIDATE] genesis_time=0 — staleness filtering disabled. \
                 Set NetworkConfig.genesis_time for stale block filtering."
            );
        }
        return (MessageAcceptance::Accept, Some(block));
    }

    let current_slot = wall_clock_slot_from(genesis_time, slot_duration, now_unix);
    let acceptance = classify_block(&block, current_slot);
    (acceptance, Some(block))
}

/// Maximum age (in seconds) for a producer announcement to be forwarded on the
/// producer-announcement gossip topic (`PRODUCERS_TOPIC`).
///
/// Aligned with the discovery layer's `MAX_ANNOUNCEMENT_AGE_SECS` (3600): the
/// GSet merge already rejects any announcement older than one hour
/// (`ProducerSetError::StaleAnnouncement`), so a gossip message whose *newest*
/// announcement exceeds this age can change no node's producer set. Re-forwarding
/// such a message accomplishes nothing but log/bandwidth amplification — the
/// INC-I-137 / INC-I-120 Layer-3 stale-snapshot storm.
pub const PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS: u64 = 3600;

/// Classify a producer-announcement gossip message for forwarding (INC-I-137).
///
/// Producer announcements form a grow-only CRDT (GSet). To preserve convergence,
/// a genuinely-new announcement MUST still be forwarded exactly once. This gate
/// therefore suppresses re-forwarding **only** for messages that no node could
/// absorb: a decoded, non-empty `ProducerSet` in which **every** announcement is
/// older than [`PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS`] returns `Ignore` (dropped,
/// no peer-score penalty — stale-but-honest gossip must not trigger eviction
/// cascades, per INC-I-016).
///
/// Everything else fails open to `Accept`:
/// - any announcement within the TTL — so mixed-freshness snapshots and
///   genuinely-new producers still forward (CRDT convergence preserved);
/// - timestamp-less formats (bloom-digest delta-sync, legacy `Vec<PublicKey>`)
///   and bytes that do not decode as a `ProducerSet`;
/// - empty sets;
/// - `now_unix == 0` (clock unavailable — fail open, mirroring
///   [`classify_block_gossip`]'s `genesis_time == 0` behavior).
///
/// Pure function: no I/O, no side effects.
pub fn classify_producer_gossip(data: &[u8], now_unix: u64) -> MessageAcceptance {
    // Fail open when the wall clock is unavailable.
    if now_unix == 0 {
        return MessageAcceptance::Accept;
    }

    // Only the signed `ProducerSet` format carries per-announcement timestamps.
    // Digests, legacy bincode, and garbage do not decode here → Accept.
    let announcements = match decode_producer_set(data) {
        Ok(anns) if !anns.is_empty() => anns,
        _ => return MessageAcceptance::Accept,
    };

    // Ignore only when EVERY announcement is provably stale (strictly older than
    // the merge-acceptance window — the boundary age is treated as still-fresh,
    // matching classify_block's inclusive threshold). A single within-TTL
    // announcement means the message can still change a peer's GSet, so it must
    // be forwarded to preserve convergence.
    let all_stale = announcements.iter().all(|ann| {
        now_unix
            > ann
                .timestamp
                .saturating_add(PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS)
    });

    if all_stale {
        MessageAcceptance::Ignore
    } else {
        MessageAcceptance::Accept
    }
}

/// Compute the current wall-clock slot from an explicit Unix timestamp and
/// genesis parameters.
///
/// This mirrors `ConsensusParams::timestamp_to_slot()` but operates on raw
/// values to avoid a dependency on the full ConsensusParams struct in the
/// network crate. Accepts `now_unix` as a parameter for deterministic testing
/// (P2-003).
///
/// Returns 0 for timestamps before genesis or when slot_duration is 0.
pub fn wall_clock_slot_from(genesis_time: u64, slot_duration: u64, now_unix: u64) -> u32 {
    if slot_duration == 0 || now_unix < genesis_time {
        return 0;
    }
    let slot_u64 = (now_unix - genesis_time) / slot_duration;
    if slot_u64 > u64::from(u32::MAX) {
        u32::MAX
    } else {
        slot_u64 as u32
    }
}

/// Compute the current wall-clock slot from system time and genesis parameters.
///
/// Convenience wrapper around [`wall_clock_slot_from`] that reads
/// `SystemTime::now()`. Production code should use this; tests should use
/// `wall_clock_slot_from` with a pinned timestamp.
pub fn wall_clock_slot(genesis_time: u64, slot_duration: u64) -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    wall_clock_slot_from(genesis_time, slot_duration, now)
}

/// Return the current Unix timestamp in seconds (for passing to
/// [`classify_block_gossip`]).
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::{Block, BlockHeader};

    /// Build a minimal valid block with a given slot for testing.
    fn make_block(slot: u32) -> Block {
        let header = BlockHeader {
            version: 2,
            prev_hash: crypto::Hash::ZERO,
            merkle_root: crypto::Hash::ZERO,
            presence_root: crypto::Hash::ZERO,
            genesis_hash: crypto::Hash::ZERO,
            timestamp: 0,
            slot,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof { pi: vec![] },
            missed_producers: vec![],
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
        };
        Block::new(header, vec![])
    }

    fn make_block_bytes(slot: u32) -> Vec<u8> {
        make_block(slot).serialize()
    }

    /// Helper: check if the verdict is Accept (MessageAcceptance has no PartialEq)
    fn is_accept(v: MessageAcceptance) -> bool {
        matches!(v, MessageAcceptance::Accept)
    }

    fn is_ignore(v: MessageAcceptance) -> bool {
        matches!(v, MessageAcceptance::Ignore)
    }

    fn is_reject(v: MessageAcceptance) -> bool {
        matches!(v, MessageAcceptance::Reject)
    }

    // ── classify_block tests (pure slot logic) ────────────────────────

    #[test]
    fn fresh_block_accepted() {
        let block = make_block(100);
        assert!(
            is_accept(classify_block(&block, 100)),
            "A block at the current slot must be accepted"
        );
    }

    #[test]
    fn slightly_old_block_accepted() {
        let block = make_block(95);
        assert!(
            is_accept(classify_block(&block, 100)),
            "A block within the threshold must be accepted"
        );
    }

    #[test]
    fn boundary_block_at_exactly_threshold_accepted() {
        let block = make_block(100 - STALE_BLOCK_SLOT_THRESHOLD);
        assert!(
            is_accept(classify_block(&block, 100)),
            "A block exactly at the threshold boundary must be accepted (inclusive)"
        );
    }

    #[test]
    fn stale_block_one_past_threshold_ignored() {
        let block = make_block(100 - STALE_BLOCK_SLOT_THRESHOLD - 1);
        assert!(
            is_ignore(classify_block(&block, 100)),
            "A block one slot past the threshold must be ignored"
        );
    }

    #[test]
    fn very_stale_block_ignored() {
        let block = make_block(920);
        assert!(
            is_ignore(classify_block(&block, 2000)),
            "A block 1080 slots old must be ignored"
        );
    }

    #[test]
    fn future_block_accepted() {
        let block = make_block(105);
        assert!(
            is_accept(classify_block(&block, 100)),
            "A slightly future block must be accepted"
        );
    }

    #[test]
    fn early_chain_no_underflow() {
        let block = make_block(0);
        assert!(
            is_accept(classify_block(&block, 3)),
            "Early chain: slot 0 must be accepted when current_slot < threshold"
        );
    }

    // ── classify_block_gossip tests (bytes → deserialization + staleness) ──

    #[test]
    fn gossip_garbage_bytes_rejected() {
        let data = b"this is not a valid block";
        let (acc, block) = classify_block_gossip(data, 1000, 10, 2000);
        assert!(is_reject(acc), "Garbage bytes must be rejected");
        assert!(block.is_none(), "No block returned for garbage");
    }

    #[test]
    fn gossip_empty_bytes_rejected() {
        let data = b"";
        let (acc, block) = classify_block_gossip(data, 1000, 10, 2000);
        assert!(is_reject(acc), "Empty bytes must be rejected");
        assert!(block.is_none());
    }

    #[test]
    fn gossip_fresh_block_accepted_with_block() {
        // genesis_time=1000, slot_duration=10, now=2000 → slot=100
        let data = make_block_bytes(100);
        let (acc, block) = classify_block_gossip(&data, 1000, 10, 2000);
        assert!(is_accept(acc), "Fresh block must be accepted");
        assert!(block.is_some(), "Deserialized block must be returned");
        assert_eq!(block.unwrap().header.slot, 100);
    }

    #[test]
    fn gossip_stale_block_ignored_with_block() {
        // genesis_time=1000, slot_duration=10, now=2000 → slot=100
        // Block at slot 90: cutoff = 100-6 = 94, 90 < 94 → Ignore
        let data = make_block_bytes(90);
        let (acc, block) = classify_block_gossip(&data, 1000, 10, 2000);
        assert!(is_ignore(acc), "Stale block must be ignored");
        assert!(
            block.is_some(),
            "Block should still be returned for logging"
        );
    }

    // ── P1-002: genesis_time=0 fail-open ──────────────────────────────

    #[test]
    fn gossip_genesis_time_zero_accepts_any_valid_block() {
        // genesis_time=0 → staleness disabled → Accept any deserializable block
        let data = make_block_bytes(5);
        let (acc, block) = classify_block_gossip(&data, 0, 10, 999_999_999);
        assert!(
            is_accept(acc),
            "genesis_time=0 must Accept (fail-open, staleness disabled)"
        );
        assert!(block.is_some());
    }

    #[test]
    fn gossip_genesis_time_zero_still_rejects_garbage() {
        // genesis_time=0, but garbage bytes → still Reject
        let data = b"not-a-block";
        let (acc, _) = classify_block_gossip(data, 0, 10, 999_999_999);
        assert!(
            is_reject(acc),
            "genesis_time=0 must still Reject undeserializable garbage"
        );
    }

    // ── P2-002: non-Block payload tests ───────────────────────────────

    #[test]
    fn gossip_header_bytes_on_block_topic_rejected() {
        // A BlockHeader serialized to bytes is NOT a valid Block — Block expects
        // header + Vec<Transaction>. classify_block_gossip must Reject it.
        // This is the P0-001 regression test: header bytes should NEVER be routed
        // through classify_block_gossip (HEADERS_TOPIC is not a block topic), but
        // this test pins that if they somehow are, the result is Reject (not Accept).
        let header = BlockHeader {
            version: 2,
            prev_hash: crypto::Hash::ZERO,
            merkle_root: crypto::Hash::ZERO,
            presence_root: crypto::Hash::ZERO,
            genesis_hash: crypto::Hash::ZERO,
            timestamp: 0,
            slot: 100,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof { pi: vec![] },
            missed_producers: vec![],
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
        };
        let header_bytes = header.serialize();
        let (acc, block) = classify_block_gossip(&header_bytes, 1000, 10, 2000);
        assert!(
            is_reject(acc),
            "Header-only bytes must be Rejected when passed to block classification \
             (Block::deserialize fails on header-only data)"
        );
        assert!(block.is_none());
    }

    #[test]
    fn gossip_truncated_block_bytes_rejected() {
        // A valid block's bytes truncated to half should fail deserialization
        let full = make_block_bytes(100);
        let truncated = &full[..full.len() / 2];
        let (acc, block) = classify_block_gossip(truncated, 1000, 10, 2000);
        assert!(is_reject(acc), "Truncated block bytes must be rejected");
        assert!(block.is_none());
    }

    // ── P2-003: wall_clock_slot_from with explicit now_unix ───────────

    #[test]
    fn wall_clock_slot_from_normal_case() {
        // genesis=1000, slot_duration=10, now=1100 → slot=10
        assert_eq!(wall_clock_slot_from(1000, 10, 1100), 10);
    }

    #[test]
    fn wall_clock_slot_from_exactly_6_slots_boundary() {
        // genesis=1000, slot_duration=10, now=1060 → slot=6
        assert_eq!(wall_clock_slot_from(1000, 10, 1060), 6);
        // genesis=1000, slot_duration=10, now=1059 → slot=5
        assert_eq!(wall_clock_slot_from(1000, 10, 1059), 5);
    }

    #[test]
    fn wall_clock_slot_from_pre_genesis() {
        // now < genesis → 0
        assert_eq!(wall_clock_slot_from(2000, 10, 1000), 0);
    }

    #[test]
    fn wall_clock_slot_from_zero_duration() {
        assert_eq!(wall_clock_slot_from(1000, 0, 2000), 0);
    }

    #[test]
    fn wall_clock_slot_before_genesis_legacy() {
        let slot = wall_clock_slot(u64::MAX, 10);
        assert_eq!(slot, 0, "Before genesis, wall_clock_slot must return 0");
    }

    #[test]
    fn wall_clock_slot_zero_duration_legacy() {
        let slot = wall_clock_slot(0, 0);
        assert_eq!(slot, 0, "Zero slot_duration must return 0");
    }
}
