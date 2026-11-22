//! Admission check for blocks fetched by `repairArchiveFromPeer` (INC-I-308 P1-RPC-007, R-1).

use crypto::Hash;
use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use storage::BlockStore;

/// Decode `data` and accept it only if its hash is the one our canonical store commits to at `h`,
/// it is signed, and its body matches its header.
pub(super) fn admissible(
    store: &BlockStore,
    data: &[u8],
    h: u64,
    sig_ah: u64,
    bls_ah: u64,
) -> Result<Block, String> {
    let block = doli_core::transaction::legacy::deserialize_block_compat(data)
        .ok_or_else(|| "bytes do not decode as a block".to_string())?;
    let want = anchor(store, h)?;
    if block.hash() != want {
        return Err(format!("hash differs from our chain's {want}"));
    }
    verify_block_signature_at(&block.header, h, sig_ah).map_err(|e| e.to_string())?;
    if !block.body_matches_header(bls_ah, Some(h)) {
        return Err("body does not match its header".to_string());
    }
    Ok(block)
}

/// Hash our canonical store commits to at `h`: the index hash, or `prev_hash` of the stored block at h+1.
fn anchor(store: &BlockStore, h: u64) -> Result<Hash, String> {
    let index = store.get_hash_by_height(h).ok().flatten();
    let next = store
        .get_block_by_height(h + 1)
        .ok()
        .flatten()
        .map(|b| b.header.prev_hash);
    match (index, next) {
        (Some(a), Some(b)) if a != b => Err(format!("local store contradicts itself at h={h}")),
        (Some(a), _) => Ok(a),
        (None, Some(b)) => Ok(b),
        (None, None) => Err(format!(
            "no trusted hash for h={h}; run backfillFromPeer first"
        )),
    }
}
