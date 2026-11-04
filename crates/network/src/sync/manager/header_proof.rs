//! Predicates that decide when a fetched header proves a peer's chain leaves our tip.

use crypto::Hash;

use crate::protocols::SyncRequest;
use crate::sync::headers::HeaderChainBreak;

/// The answered request was `GetHeadersByHeight` from our tip height, so its reply starts at tip+1.
pub(crate) fn reply_anchored_at_tip(request: Option<&SyncRequest>, local_height: u64) -> bool {
    matches!(request, Some(SyncRequest::GetHeadersByHeight { start_height, .. }) if *start_height == local_height)
}

/// A header reply break proves divergence from our tip.
pub(crate) fn chain_break_proves_tip_divergence(
    cb: &HeaderChainBreak,
    local_hash: Hash,
    anchored_at_tip: bool,
) -> bool {
    cb.valid_so_far == 0 && cb.expected == local_hash && cb.first_header_valid && anchored_at_tip
}

/// A gossip sibling of our tip (same height, same parent) proves divergence from our tip.
pub fn sibling_proves_tip_divergence(
    fork_height: u64,
    best_height: u64,
    sibling_prev: Hash,
    tip_prev: Hash,
) -> bool {
    fork_height == best_height && sibling_prev == tip_prev
}
