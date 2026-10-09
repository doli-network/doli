//! Producer signature over the final block header (INC-I-236), carried in `vdf_proof.pi`.

use super::*;

impl Node {
    /// Sign the final header with the producer key; must run after every header field is set.
    pub(super) fn sign_block_header(&self, header: &mut BlockHeader) {
        if let Some(kp) = &self.producer_key {
            debug_assert_eq!(header.producer, *kp.public_key());
            let msg = header.signing_message();
            header.vdf_proof.pi =
                crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
        }
    }
}
