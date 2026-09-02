//! Per-peer branch verdicts — "was this peer last seen on OUR chain?"
//!
//! INC-I-204 M2 / REQ-FORK-013. A peer eligible for `best_peer` is AHEAD of us and
//! therefore outside `recent_canonical_hashes` (`[local_height-199, local_height]`),
//! so it cannot be classified at selection time. The verdict is recorded while the
//! peer's reported height IS inside the ring, then read later under a freshness bound.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use libp2p::PeerId;

use crypto::Hash;

use super::SyncManager;

/// A verdict is not trusted longer than the status that produced it:
/// `SyncConfig::stale_timeout` (types.rs) drops the peer at the same age (cleanup.rs).
pub(in crate::sync::manager) const BRANCH_VERDICT_TTL: Duration = Duration::from_secs(300);

/// Where a peer's reported tip sits relative to our canonical chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchVerdict {
    /// Peer hash equals our canonical hash at the peer's reported height.
    Agreeing,
    /// Peer reported a different hash at a height we hold canonically.
    Divergent,
}

/// Classify a peer tip against our canonical ring.
///
/// `None` means UNCLASSIFIABLE (no ring entry at that height) — never a verdict.
pub(in crate::sync::manager) fn classify_branch(
    recent_canonical_hashes: &VecDeque<(u64, Hash)>,
    peer_height: u64,
    peer_hash: Hash,
) -> Option<BranchVerdict> {
    recent_canonical_hashes
        .iter()
        .find(|(h, _)| *h == peer_height)
        .map(|(_, our_hash)| {
            if *our_hash == peer_hash {
                BranchVerdict::Agreeing
            } else {
                BranchVerdict::Divergent
            }
        })
}

impl SyncManager {
    /// Record a verdict for `peer` from a status observation.
    ///
    /// An unclassifiable observation leaves any earlier verdict in place to age out;
    /// it is never upgraded into a verdict of its own.
    pub(in crate::sync::manager) fn record_branch_verdict(
        &mut self,
        peer: PeerId,
        height: u64,
        hash: Hash,
    ) {
        if let Some(verdict) = classify_branch(&self.recent_canonical_hashes, height, hash) {
            self.peer_branch_verdicts
                .insert(peer, (verdict, Instant::now()));
        }
    }

    /// True when `peer` carries a fresh `Agreeing` verdict.
    ///
    /// The verdict AND the peer status backing it must both be within
    /// `BRANCH_VERDICT_TTL`, so a gossip refresh cannot keep an old verdict alive.
    pub(in crate::sync::manager) fn is_on_our_branch(&self, peer: &PeerId) -> bool {
        let Some((BranchVerdict::Agreeing, recorded_at)) = self.peer_branch_verdicts.get(peer)
        else {
            return false;
        };
        recorded_at.elapsed() <= BRANCH_VERDICT_TTL
            && self
                .peers
                .get(peer)
                .is_some_and(|s| s.last_status_response.elapsed() <= BRANCH_VERDICT_TTL)
    }
}
