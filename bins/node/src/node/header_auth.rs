//! INC-I-236 gossip-header authentication gate (REQ-AUTH-007, REQ-AUTH-012).

use doli_core::BlockHeader;

/// `true` below the AH; at or above it, only if the producer's block signature verifies.
pub(super) fn gossip_header_authenticated(
    header: &BlockHeader,
    prospective_height: u64,
    activation_height: u64,
) -> bool {
    prospective_height < activation_height
        || doli_core::validation::verify_block_signature(header).is_ok()
}
