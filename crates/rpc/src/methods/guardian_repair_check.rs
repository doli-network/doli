//! Admission check for blocks fetched by `repairArchiveFromPeer` (INC-I-308 P1-RPC-007).

use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use storage::BlockStore;

/// Decode `data` and accept it only if it is the local canonical block at `h`,
/// or, when the local store has no block at `h`, if it is signed and its body matches.
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
            Ok(block)
        }
    }
}
