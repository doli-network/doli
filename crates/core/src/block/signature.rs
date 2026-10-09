//! Producer signature over a block header (INC-I-236), carried in `vdf_proof.pi`.

use crypto::Hash;

use super::BlockHeader;

/// Domain tag prefixed to the header hash before signing.
// The 0xFF byte makes the preimage invalid UTF-8, so `doli sign` (blake3 of a
// UTF-8 message) can never produce a block signature.
pub const BLOCK_SIGNATURE_DOMAIN: &[u8] = b"DOLI_BLOCK_SIG_V1\xFF";

impl BlockHeader {
    /// Message the producer signs: `blake3(BLOCK_SIGNATURE_DOMAIN || hash())`.
    pub fn signing_message(&self) -> Hash {
        crypto::hash::hash_concat(&[BLOCK_SIGNATURE_DOMAIN, self.hash().as_bytes()])
    }
}
