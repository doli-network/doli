//! Stored-neighbor consistency check for `backfillFromPeer` (INC-I-308 P1-RPC-005).

use std::collections::HashSet;

use doli_core::Block;
use storage::BlockStore;

/// Why `block` must not be stored at `h`, or `None` when it fits the stored chain.
/// Heights in `repair` may be overwritten and their stored successor is stale.
pub(super) fn refusal(
    store: &BlockStore,
    block: &Block,
    h: u64,
    repair: &HashSet<u64>,
) -> Option<String> {
    let hash = block.hash();
    if !repair.contains(&h) {
        if let Ok(Some(existing)) = store.get_hash_by_height(h) {
            if existing != hash {
                return Some(format!("height index already holds {existing}"));
            }
        }
    }
    if !repair.contains(&(h + 1)) {
        if let Ok(Some(next)) = store.get_block_by_height(h + 1) {
            if next.header.prev_hash != hash {
                return Some(format!("stored h+1 prev_hash {}", next.header.prev_hash));
            }
        }
    }
    if h > 1 {
        if let Ok(Some(prev)) = store.get_hash_by_height(h - 1) {
            if block.header.prev_hash != prev {
                return Some(format!("prev_hash differs from stored h-1 {prev}"));
            }
        }
    }
    None
}
