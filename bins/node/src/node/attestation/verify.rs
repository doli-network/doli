//! INC-I-178 D7 (M5) — post-AH verification of the carried attestation body.
//!
//! Below `inc_i_178_attestation_bls_activation_height` this module is inert: the
//! entry point returns before it touches a counter, so nothing here is a live
//! consensus dependency pre-AH.

use super::*;

use crypto::{BlsPublicKey, BlsSignature};
use doli_core::attestation::bls_attest_msg;
use doli_core::presence_commitment;
use tracing::debug;

use crate::metrics::{
    ATTESTATION_VERIFY_REJECTED, ATTESTATION_VERIFY_SKIPPED_LIGHT, ATTESTATION_VERIFY_TOTAL,
};

/// Header presence_root does not commit to the body it arrived with.
pub(crate) const REASON_ROOT_MISMATCH: &str = "root_mismatch";
/// Bits are set, but the carried aggregate is not a signature over the parent
/// by exactly those producers.
pub(crate) const REASON_AGGREGATE_INVALID: &str = "aggregate_invalid";
/// No bit is set, so the only legal aggregate is the empty one.
pub(crate) const REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD: &str =
    "aggregate_nonempty_for_empty_bitfield";
/// A set bit names a producer with no usable on-chain BLS key.
pub(crate) const REASON_MISSING_BLS_KEY: &str = "missing_bls_key";

/// Which of the two accepting arms the decision took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Accepted,
    /// The pairing was skipped because the local universe is known-divergent.
    SkippedLight,
}

/// The whole post-AH decision, with no locks, no `Node` and no I/O.
///
/// `Err` carries the stable reason label, which is both the Prometheus `reason`
/// value and the `reason` field of the returned `ValidationError`.
pub(crate) fn decide_attestation(
    presence_root: &Hash,
    bitfield: &[u8],
    aggregate: &[u8],
    parent_hash: &Hash,
    universe_divergent: bool,
    set_bit_keys: Result<Vec<BlsPublicKey>, PublicKey>,
) -> Result<Outcome, &'static str> {
    // Unconditional, empty bitfield included: the commitment needs no universe,
    // so it is the one check a divergent node can still make.
    if *presence_root != presence_commitment(bitfield, aggregate) {
        return Err(REASON_ROOT_MISMATCH);
    }
    if universe_divergent {
        return Ok(Outcome::SkippedLight);
    }
    // "No bit set" means BITS, not length: an honest-width all-zero bitfield
    // carries no claim either.
    if !bitfield.iter().any(|b| *b != 0) {
        return if aggregate.is_empty() {
            Ok(Outcome::Accepted)
        } else {
            Err(REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD)
        };
    }
    let keys = set_bit_keys.map_err(|_| REASON_MISSING_BLS_KEY)?;
    let Ok(sig) = BlsSignature::try_from_slice(aggregate) else {
        return Err(REASON_AGGREGATE_INVALID);
    };
    crypto::bls_verify_aggregate(&bls_attest_msg(parent_hash), &sig, &keys)
        .map(|()| Outcome::Accepted)
        .map_err(|_| REASON_AGGREGATE_INVALID)
}

impl Node {
    /// The ONE call site is `validate_block_for_apply`, after the VDF.
    pub(crate) async fn verify_block_attestation(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<(), validation::ValidationError> {
        let ah = self.inc_i_178_attestation_bls_activation_height;
        if height < ah {
            return Ok(());
        }
        ATTESTATION_VERIFY_TOTAL.inc();

        // Light = gap blocks after snap sync, Replay = own-store recovery; both
        // are known-divergent or known-trusted, Full is the live gossip path.
        let divergent = !matches!(mode, ValidationMode::Full) || self.snap_sync_height.is_some();

        let set_bit_keys = {
            let producers = self.producer_set.read().await;
            let active: Vec<PublicKey> = producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.public_key)
                .collect();
            let universe = commit::post_commit_universe_at(
                ah,
                height,
                &self.epoch_state.producer_list,
                &active,
            );
            keys::set_bit_bls_pubkeys(&universe, &block.attestation_bitfield, &producers)
        };

        match decide_attestation(
            &block.header.presence_root,
            &block.attestation_bitfield,
            &block.aggregate_bls_signature,
            &block.header.prev_hash,
            divergent,
            set_bit_keys,
        ) {
            Ok(outcome) => {
                if outcome == Outcome::SkippedLight {
                    ATTESTATION_VERIFY_SKIPPED_LIGHT.inc();
                }
                debug!(
                    "[ATTEST_VERIFY] ok h={} hash={:.16} outcome={:?} mode={:?}",
                    height,
                    block.hash(),
                    outcome,
                    mode
                );
                Ok(())
            }
            Err(reason) => {
                ATTESTATION_VERIFY_REJECTED
                    .with_label_values(&[reason])
                    .inc();
                warn!(
                    "[ATTEST_VERIFY] reject h={} hash={:.16} reason={} ah={}",
                    height,
                    block.hash(),
                    reason,
                    ah
                );
                Err(validation::ValidationError::AttestationVerifyFailed {
                    reason: reason.to_string(),
                    height,
                    activation_height: ah,
                })
            }
        }
    }
}
