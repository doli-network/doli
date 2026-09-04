//! The one shared per-signature attestation ingress (INC-I-178 D4).
//!
//! Both ingresses — gossip `on_new_attestation` and direct `record_direct_attestation`
//! — run this body, so the membership gate, attendance and the BLS verdict cannot
//! drift apart the way the two copies did in INC-I-191 / INC-I-192.

use super::*;

use crypto::{bls_verify, BlsPublicKey, BlsSignature, BLS_SIGNATURE_SIZE};
use doli_core::attestation::bls_attest_msg;

use crate::metrics::{ATTESTATION_BLS_VALID_BY_ATTESTER, ATTESTATION_BLS_VALID_TOTAL};

/// Outcome of checking the BLS half of one attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlsAttestVerdict {
    /// Verified against the attester's on-chain key — the only arm that pools.
    Valid([u8; BLS_SIGNATURE_SIZE]),
    /// No BLS bytes at all: a Release N-1 peer. Bridge arm, never scored.
    Empty,
    /// The attester published no usable on-chain key: unverifiable is not invalid.
    NoKey,
    /// Present, but not this attester's signature over the frozen preimage.
    Invalid,
}

impl Node {
    /// Derive an attester's authority from the LOCAL ProducerSet (Seam A, [F1]).
    /// `None` = not a producer-set member (reject; never trust the wire's
    /// self-declared weight). `Some(w)` = member with locally-derived selection
    /// weight, which may be 0 for a fully-delegated active producer (INV-ATTEST-001,
    /// so attendance admission uses `.is_some()`, not `w > 0`).
    pub(crate) fn derive_attester_weight(
        &self,
        producers: &ProducerSet,
        attester: &PublicKey,
        height: u64,
    ) -> Option<u64> {
        producers.get_by_pubkey(attester).map(|p| {
            p.selection_weight_at(
                height,
                self.config
                    .network
                    .params()
                    .security_audit_activation_height,
            )
        })
    }

    /// Admit one Ed25519-verified attestation: membership, then attendance, then the
    /// BLS verdict. Returns the locally derived weight, or `None` for a non-member.
    ///
    /// `height` is the height authority is derived at — the attested block's height at
    /// the gossip ingress, the local tip at the direct one.
    pub(crate) async fn ingest_attestation(
        &mut self,
        att: &Attestation,
        height: u64,
        source_peer: PeerId,
    ) -> Option<u64> {
        let derived = {
            let producers = self.producer_set.read().await;
            self.derive_attester_weight(&producers, &att.attester, height)
                .map(|w| {
                    let key = producers
                        .get_by_pubkey(&att.attester)
                        .map(|p| p.bls_pubkey.clone())
                        .unwrap_or_default();
                    (w, key)
                })
        };
        let Some((weight, onchain_bls_key)) = derived else {
            debug!(
                "[ATTEST_INGEST] dropping attestation from non-member {:.8}",
                att.attester
            );
            return None;
        };

        // Attendance is Ed25519-authenticated, so a member attends whatever the BLS
        // verdict is — and regardless of weight (INV-ATTEST-001).
        self.minute_tracker
            .record(att.attester, attestation_minute(att.slot));

        match self.bls_verdict(att, &onchain_bls_key) {
            BlsAttestVerdict::Valid(sig) => {
                if self
                    .parent_sig_pool
                    .insert(att.block_hash, att.attester, sig)
                {
                    let attester_hex = att.attester.to_hex();
                    let attester = &attester_hex[..8];
                    ATTESTATION_BLS_VALID_TOTAL.inc();
                    ATTESTATION_BLS_VALID_BY_ATTESTER
                        .with_label_values(&[attester])
                        .inc();
                    info!(
                        "[ATTEST_INGEST] valid bls attester={} parent={} sig_len={}",
                        attester,
                        &att.block_hash.to_hex()[..8],
                        sig.len()
                    );
                }
            }
            BlsAttestVerdict::Invalid => {
                // Release N records and logs only. A producer whose BLS key does not
                // match its on-chain key emits one bad half per block, relayed by every
                // honest peer; acting on the score would partition the fleet on ONE
                // misconfigured producer (INV-NETWORK-002 mesh-expulsion cascade).
                self.bls_ingress_scorer
                    .record_invalid_bls_attestation(&source_peer);
                warn!(
                    "[ATTEST_INGEST] unverifiable BLS half from {:.8} relayed by {} (score {})",
                    att.attester,
                    source_peer,
                    self.bls_ingress_scorer
                        .get_score(&source_peer)
                        .map(|s| s.value)
                        .unwrap_or(0)
                );
            }
            BlsAttestVerdict::Empty | BlsAttestVerdict::NoKey => {}
        }

        Some(weight)
    }

    /// Check the BLS half against the attester's on-chain key. Never mutates.
    fn bls_verdict(&self, att: &Attestation, onchain_bls_key: &[u8]) -> BlsAttestVerdict {
        if att.bls_signature.is_empty() {
            return BlsAttestVerdict::Empty;
        }
        let Ok(pubkey) = BlsPublicKey::try_from_slice(onchain_bls_key) else {
            return BlsAttestVerdict::NoKey;
        };

        // Flood bound. Ed25519 does not cover `bls_signature` and gossip dedups on raw
        // bytes (INV-NETWORK-004), so one honest attestation mints unlimited distinct
        // messages; BLS signing is deterministic, so any blob that differs from the
        // verified one already pooled is invalid without paying the ~493 us pairing.
        if let Some(pooled) = self.parent_sig_pool.get(&att.block_hash, &att.attester) {
            return if att.bls_signature.as_slice() == pooled.as_slice() {
                BlsAttestVerdict::Valid(*pooled)
            } else {
                BlsAttestVerdict::Invalid
            };
        }

        let Ok(sig_bytes) = <[u8; BLS_SIGNATURE_SIZE]>::try_from(att.bls_signature.as_slice())
        else {
            return BlsAttestVerdict::Invalid;
        };
        let Ok(sig) = BlsSignature::try_from_slice(&sig_bytes) else {
            return BlsAttestVerdict::Invalid;
        };
        if bls_verify(&bls_attest_msg(&att.block_hash), &sig, &pubkey).is_err() {
            return BlsAttestVerdict::Invalid;
        }
        BlsAttestVerdict::Valid(sig_bytes)
    }
}
