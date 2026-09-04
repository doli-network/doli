//! Node-side attestation ingress (INC-I-178 D4).

use super::*;

pub mod commit;
pub(crate) mod ingress;

impl Node {
    /// Build the attestation this node broadcasts (INC-I-178 D3, the ONE egress).
    ///
    /// Dual-signs when a BLS key is configured. A BLS signing error still ships the
    /// Ed25519 half: losing attendance costs the producer its epoch reward, while a
    /// missing BLS half only leaves it out of one aggregate (REQ-BLS-010).
    pub(crate) fn sign_attestation(
        &self,
        block_hash: Hash,
        slot: u32,
        height: u64,
        weight: u64,
        private_key: &crypto::PrivateKey,
        public_key: PublicKey,
    ) -> Attestation {
        match &self.bls_key {
            Some(bls) => Attestation::new_with_bls(
                block_hash,
                slot,
                height,
                weight,
                private_key,
                public_key,
                bls,
            )
            .unwrap_or_else(|e| {
                warn!("[ATTEST_EGRESS] BLS signing failed ({e}) — shipping Ed25519 only");
                Attestation::new(block_hash, slot, height, weight, private_key, public_key)
            }),
            None => Attestation::new(block_hash, slot, height, weight, private_key, public_key),
        }
    }
}
