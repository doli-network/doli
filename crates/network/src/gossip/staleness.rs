//! Gossip staleness primitives (INC-I-142, INV-NETWORK-003): the
//! [`StalenessCtx`] the relay table reads and the per-topic age bounds.
//! Re-deliveries are dropped before the app by the vendor duplicate cache
//! ([`GOSSIP_DUPLICATE_CACHE_TIME`](super::GOSSIP_DUPLICATE_CACHE_TIME), keyed
//! by BLAKE3 of the raw data). The verdicts live in
//! [`relay_decision`](super::relay_decision); no staleness path ever `Reject`s.

use doli_core::Attestation;

use super::validation::wall_clock_slot_from;

/// Age bound (in slots) for attestations: older than
/// `wall_slot - ATTEST_STALE_SLOTS` is `Ignore`d. Wide on purpose, so a late
/// attestation backing a recent block is never dropped.
pub const ATTEST_STALE_SLOTS: u32 = 12;

/// Future bound (in slots) for attestations: newer than
/// `wall_slot + ATTEST_FUTURE_SLOTS` is `Ignore`d (M5-DEV-F1).
pub const ATTEST_FUTURE_SLOTS: u32 = 2;

/// The attestation age and future bounds; an unset clock (`genesis_time == 0`)
/// fails open.
pub(super) fn attestation_is_current(
    a: &Attestation,
    genesis_time: u64,
    slot_duration: u64,
    now: u64,
) -> bool {
    let wall = wall_clock_slot_from(genesis_time, slot_duration, now);
    genesis_time == 0
        || (a.slot >= wall.saturating_sub(ATTEST_STALE_SLOTS)
            && a.slot <= wall.saturating_add(ATTEST_FUTURE_SLOTS))
}

/// Age bound (seconds) for governance votes: a vote whose `timestamp` is older
/// than `now_unix - VOTE_MAX_AGE_SECS` is `Ignore`d. Must exceed the veto
/// window (`VETO_PERIOD`, `crates/updater/src/constants.rs`) on every network.
pub const VOTE_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// The five subscribed gossip topics (metric label).
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

/// Read-only per-message context the gossip handler builds for the relay table.
pub struct StalenessCtx {
    /// Current Unix time in seconds (`now_unix_secs()`); 0 → clock unavailable.
    pub now_unix: u64,
    /// Genesis timestamp (Unix seconds); 0 → staleness disabled (fail-open).
    pub genesis_time: u64,
    /// Slot duration in seconds.
    pub slot_duration: u64,
    /// Local tip slot (`best_slot.load(Relaxed)`). Staleness predicates use the
    /// wall-clock slot, NOT this, so a lagging node never treats fresh gossip
    /// as stale (§7.1).
    pub best_slot: u32,
    /// Local tip height; a gossip block is gated at prospective height `best_height + 1`.
    pub best_height: u64,
    /// INC-I-236 block signature AH; `u64::MAX` = dormant.
    pub block_signature_activation_height: u64,
}

#[cfg(test)]
#[path = "staleness_tests.rs"]
mod tests;
