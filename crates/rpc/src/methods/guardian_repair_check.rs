//! Admission check for blocks fetched by `repairArchiveFromPeer` (INC-I-308 P1-RPC-007).

use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use storage::BlockStore;

/// Decode `data` and accept it only if it is the local canonical block at `h`,
/// or, when the local store has no block at `h`, if it is signed, its body matches, and it links to a stored neighbor.
pub(super) fn admissible(
    store: &BlockStore,
    data: &[u8],
    h: u64,
    sig_ah: u64,
    bls_ah: u64,
) -> Result<Block, String> {
    let block = doli_core::transaction::legacy::deserialize_block_compat(data)
        .ok_or_else(|| "bytes do not decode as a block".to_string())?;
    match store.get_hash_by_height(h) {
        Ok(Some(local)) if local == block.hash() => Ok(block),
        Ok(Some(local)) => Err(format!("hash differs from local canonical {local}")),
        _ => {
            verify_block_signature_at(&block.header, h, sig_ah).map_err(|e| e.to_string())?;
            if !block.body_matches_header(bls_ah, Some(h)) {
                return Err("body does not match its header".to_string());
            }
            linked(store, &block, h)?;
            Ok(block)
        }
    }
}

/// Same neighbor rule as `backfill_neighbor::refusal`, plus: at least one neighbor must be stored.
fn linked(store: &BlockStore, block: &Block, h: u64) -> Result<(), String> {
    let next = store.get_block_by_height(h + 1).ok().flatten();
    if let Some(n) = &next {
        if n.header.prev_hash != block.hash() {
            return Err(format!("stored h+1 prev_hash {}", n.header.prev_hash));
        }
    }
    let prev = if h > 1 {
        store.get_hash_by_height(h - 1).ok().flatten()
    } else {
        None
    };
    if let Some(p) = prev {
        if block.header.prev_hash != p {
            return Err(format!("prev_hash differs from stored h-1 {p}"));
        }
    }
    if next.is_none() && prev.is_none() {
        return Err("no stored neighbor to anchor".to_string());
    }
    Ok(())
}
