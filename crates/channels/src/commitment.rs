//! Commitment transaction construction and revocation key management.
//!
//! Each channel update produces a pair of commitment transactions (one per party).
//! Revoking the old state means revealing the revocation preimage, which lets
//! the counterparty sweep via the penalty path if the old commitment is broadcast.

use crypto::hash::hash_with_domain;
use crypto::{Hash, KeyPair};
use doli_core::conditions::{Witness, WitnessSignature, HASHLOCK_DOMAIN};
use doli_core::transaction::{Input, Transaction};
use doli_core::BlockHeight;
use serde::{Deserialize, Serialize};

use crate::conditions::{to_local_output, to_remote_output};
use crate::error::{ChannelError, Result};
use crate::types::{ChannelBalance, CommitmentNumber, InFlightHtlc, PaymentDirection};

/// Domain separator for revocation preimage generation.
const REVOCATION_DOMAIN: &[u8] = b"DOLI_CHANNEL_REVOKE";

/// A pair of commitment transactions — one for each side.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitmentPair {
    /// Commitment number (monotonically increasing).
    pub number: CommitmentNumber,
    /// Balance distribution at this commitment.
    pub balance: ChannelBalance,
    /// Revocation preimage for this commitment (known only to the owner).
    /// Revealed when revoking (transitioning to a newer commitment).
    pub revocation_preimage: [u8; 32],
    /// Hash of the revocation preimage (embedded in the to_local condition).
    pub revocation_hash: Hash,
    /// Remote party's revocation hash for their commitment at this number.
    pub remote_revocation_hash: Option<Hash>,
    /// In-flight HTLCs at this commitment.
    pub htlcs: Vec<InFlightHtlc>,
}

/// Generate a deterministic revocation preimage for a given commitment number.
///
/// Uses `H(REVOCATION_DOMAIN || channel_seed || commitment_number)` where
/// `channel_seed` is derived from the party's private key and channel ID.
pub fn generate_revocation_preimage(channel_seed: &[u8; 32], number: CommitmentNumber) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 8);
    data.extend_from_slice(channel_seed);
    data.extend_from_slice(&number.to_le_bytes());
    let hash = hash_with_domain(REVOCATION_DOMAIN, &data);
    *hash.as_bytes()
}

/// Compute the revocation hash from a preimage, using the same domain as
/// the L1 hashlock evaluation.
pub fn revocation_hash(preimage: &[u8; 32]) -> Hash {
    hash_with_domain(HASHLOCK_DOMAIN, preimage)
}

/// Derive a channel seed from a keypair and channel ID.
/// This seed is used to deterministically generate all revocation preimages.
pub fn derive_channel_seed(keypair: &KeyPair, channel_id: &[u8; 32]) -> [u8; 32] {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(keypair.public_key().as_bytes());
    data.extend_from_slice(channel_id);
    let hash = hash_with_domain(b"DOLI_CHANNEL_SEED", &data);
    *hash.as_bytes()
}

impl CommitmentPair {
    /// Create a new commitment pair for the given number and balance.
    pub fn new(number: CommitmentNumber, balance: ChannelBalance, channel_seed: &[u8; 32]) -> Self {
        let preimage = generate_revocation_preimage(channel_seed, number);
        let hash = revocation_hash(&preimage);
        Self {
            number,
            balance,
            revocation_preimage: preimage,
            revocation_hash: hash,
            remote_revocation_hash: None,
            htlcs: Vec::new(),
        }
    }

    /// Set the remote party's revocation hash (received during the update protocol).
    pub fn set_remote_revocation_hash(&mut self, hash: Hash) {
        self.remote_revocation_hash = Some(hash);
    }

    /// Build the local commitment transaction.
    ///
    /// Our output uses `to_local` (delayed + revocable).
    /// Their output uses `to_remote` (immediately spendable by them).
    ///
    /// `capacity` is the funding UTXO amount. `fee` is deducted from local balance
    /// (broadcaster pays). Returns `CapacityMismatch` if the invariant
    /// `balance.total() + htlc_total + fee != capacity` is violated.
    #[allow(clippy::too_many_arguments)]
    pub fn build_local_commitment(
        &self,
        funding_tx_hash: Hash,
        funding_output_index: u32,
        local_pubkey_hash: Hash,
        remote_pubkey_hash: Hash,
        dispute_height: BlockHeight,
        capacity: doli_core::Amount,
        fee: doli_core::Amount,
    ) -> Result<Transaction> {
        // Enforce supply invariant: balance + HTLCs must equal full capacity.
        // Fee is deducted from local's output only (broadcaster pays).
        let htlc_total: doli_core::Amount = self.htlcs.iter().map(|h| h.amount).sum();
        let actual = self.balance.total() + htlc_total;
        if actual != capacity {
            return Err(ChannelError::CapacityMismatch {
                expected: capacity,
                actual,
            });
        }

        if fee > 0 && self.balance.local < fee {
            return Err(ChannelError::InsufficientBalance {
                need: fee,
                have: self.balance.local,
            });
        }

        let input = Input::new(funding_tx_hash, funding_output_index);
        let mut outputs = Vec::new();

        // to_local: our balance minus fee, with revocation + timelock
        let local_after_fee = self.balance.local - fee;
        if local_after_fee > 0 {
            let local_out = to_local_output(
                local_after_fee,
                local_pubkey_hash,
                remote_pubkey_hash,
                self.revocation_hash,
                dispute_height,
            )?;
            outputs.push(local_out);
        }

        // to_remote: their balance, immediately spendable
        if self.balance.remote > 0 {
            outputs.push(to_remote_output(self.balance.remote, remote_pubkey_hash));
        }

        // HTLC outputs
        for htlc in &self.htlcs {
            let payment_hash = Hash::from_bytes(htlc.payment_hash);
            let htlc_out = match htlc.direction {
                PaymentDirection::Outgoing => crate::conditions::htlc_offered_output(
                    htlc.amount,
                    local_pubkey_hash,
                    remote_pubkey_hash,
                    payment_hash,
                    htlc.expiry_height,
                )?,
                PaymentDirection::Incoming => crate::conditions::htlc_received_output(
                    htlc.amount,
                    local_pubkey_hash,
                    remote_pubkey_hash,
                    payment_hash,
                    htlc.expiry_height,
                )?,
            };
            outputs.push(htlc_out);
        }

        if outputs.is_empty() {
            return Err(ChannelError::InsufficientBalance { need: 1, have: 0 });
        }

        Ok(Transaction::new_transfer(vec![input], outputs))
    }

    /// Verify that a received revocation preimage matches a known hash.
    pub fn verify_revocation(preimage: &[u8; 32], expected_hash: &Hash) -> bool {
        revocation_hash(preimage) == *expected_hash
    }
}

/// Store of revoked commitment preimages.
///
/// Grows linearly with the number of channel updates — this is the fundamental
/// tradeoff of LN-Penalty. Each entry is 40 bytes (8B number + 32B preimage).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RevocationStore {
    /// Map of commitment number → revocation preimage (received from counterparty).
    entries: Vec<(CommitmentNumber, [u8; 32])>,
}

impl RevocationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a revocation preimage received from the counterparty.
    pub fn add(&mut self, number: CommitmentNumber, preimage: [u8; 32]) {
        // Avoid duplicates
        if !self.entries.iter().any(|(n, _)| *n == number) {
            self.entries.push((number, preimage));
        }
    }

    /// Look up a revocation preimage by commitment number.
    pub fn get(&self, number: CommitmentNumber) -> Option<&[u8; 32]> {
        self.entries
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, p)| p)
    }

    /// Check if we have a revocation preimage that matches a given hash.
    /// Returns the preimage if found. Used when detecting a revoked commitment on-chain.
    pub fn find_by_hash(&self, hash: &Hash) -> Option<&[u8; 32]> {
        self.entries
            .iter()
            .find(|(_, preimage)| revocation_hash(preimage) == *hash)
            .map(|(_, p)| p)
    }

    /// Number of stored revocations.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Build a witness for claiming a to_local output via the delayed path (right/true branch).
pub fn build_delayed_claim_witness(signing_hash: &Hash, keypair: &KeyPair) -> Witness {
    let sig = crypto::signature::sign_hash(signing_hash, keypair.private_key());
    Witness {
        signatures: vec![WitnessSignature {
            pubkey: *keypair.public_key(),
            signature: sig,
        }],
        preimage: None,
        or_branches: vec![true], // right branch = delayed claim
    }
}

/// Build a witness for the penalty path (left/false branch): sig + revocation preimage.
pub fn build_penalty_witness(
    signing_hash: &Hash,
    keypair: &KeyPair,
    revocation_preimage: &[u8; 32],
) -> Witness {
    let sig = crypto::signature::sign_hash(signing_hash, keypair.private_key());
    Witness {
        signatures: vec![WitnessSignature {
            pubkey: *keypair.public_key(),
            signature: sig,
        }],
        preimage: Some(*revocation_preimage),
        or_branches: vec![false], // left branch = penalty
    }
}

/// Build a witness for claiming an HTLC with a preimage (left/false branch).
pub fn build_htlc_claim_witness(
    signing_hash: &Hash,
    keypair: &KeyPair,
    payment_preimage: &[u8; 32],
) -> Witness {
    let sig = crypto::signature::sign_hash(signing_hash, keypair.private_key());
    Witness {
        signatures: vec![WitnessSignature {
            pubkey: *keypair.public_key(),
            signature: sig,
        }],
        preimage: Some(*payment_preimage),
        or_branches: vec![false], // left branch = claim with preimage
    }
}

/// Build a witness for refunding an expired HTLC (right/true branch).
pub fn build_htlc_timeout_witness(signing_hash: &Hash, keypair: &KeyPair) -> Witness {
    let sig = crypto::signature::sign_hash(signing_hash, keypair.private_key());
    Witness {
        signatures: vec![WitnessSignature {
            pubkey: *keypair.public_key(),
            signature: sig,
        }],
        preimage: None,
        or_branches: vec![true], // right branch = timeout refund
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelBalance, HtlcState, InFlightHtlc, PaymentDirection};

    // ── Revocation preimage generation ────────────────────────────────

    #[test]
    fn revocation_preimage_is_deterministic() {
        let seed = [42u8; 32];
        assert_eq!(
            generate_revocation_preimage(&seed, 0),
            generate_revocation_preimage(&seed, 0)
        );
    }

    #[test]
    fn revocation_preimages_differ_per_commitment() {
        let seed = [42u8; 32];
        let p0 = generate_revocation_preimage(&seed, 0);
        let p1 = generate_revocation_preimage(&seed, 1);
        let p2 = generate_revocation_preimage(&seed, 2);
        assert_ne!(p0, p1);
        assert_ne!(p1, p2);
        assert_ne!(p0, p2);
    }

    #[test]
    fn revocation_preimages_differ_per_seed() {
        let p1 = generate_revocation_preimage(&[1u8; 32], 0);
        let p2 = generate_revocation_preimage(&[2u8; 32], 0);
        assert_ne!(p1, p2);
    }

    #[test]
    fn revocation_preimage_at_max_commitment() {
        let seed = [42u8; 32];
        // Should not panic at u64::MAX
        let p = generate_revocation_preimage(&seed, u64::MAX);
        assert_ne!(p, [0u8; 32]);
    }

    // ── Revocation hash ───────────────────────────────────────────────

    #[test]
    fn revocation_hash_matches_hashlock_domain() {
        let preimage = [99u8; 32];
        let hash = revocation_hash(&preimage);
        let expected = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
        assert_eq!(hash, expected);
    }

    #[test]
    fn revocation_hash_is_not_identity() {
        let preimage = [42u8; 32];
        let hash = revocation_hash(&preimage);
        assert_ne!(*hash.as_bytes(), preimage);
    }

    #[test]
    fn different_preimages_different_hashes() {
        let h1 = revocation_hash(&[1u8; 32]);
        let h2 = revocation_hash(&[2u8; 32]);
        assert_ne!(h1, h2);
    }

    // ── Channel seed derivation ───────────────────────────────────────

    #[test]
    fn channel_seed_is_deterministic() {
        let kp = KeyPair::generate();
        let channel_id = [5u8; 32];
        let s1 = derive_channel_seed(&kp, &channel_id);
        let s2 = derive_channel_seed(&kp, &channel_id);
        assert_eq!(s1, s2);
    }

    #[test]
    fn channel_seed_differs_per_channel() {
        let kp = KeyPair::generate();
        let s1 = derive_channel_seed(&kp, &[1u8; 32]);
        let s2 = derive_channel_seed(&kp, &[2u8; 32]);
        assert_ne!(s1, s2);
    }

    #[test]
    fn channel_seed_differs_per_keypair() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let channel_id = [5u8; 32];
        let s1 = derive_channel_seed(&kp1, &channel_id);
        let s2 = derive_channel_seed(&kp2, &channel_id);
        assert_ne!(s1, s2);
    }

    // ── RevocationStore ───────────────────────────────────────────────

    #[test]
    fn revocation_store_add_and_get() {
        let mut store = RevocationStore::new();
        assert!(store.is_empty());

        let preimage = [7u8; 32];
        store.add(5, preimage);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(5), Some(&preimage));
        assert!(store.get(4).is_none());
    }

    #[test]
    fn revocation_store_find_by_hash() {
        let mut store = RevocationStore::new();
        let preimage = [7u8; 32];
        let hash = revocation_hash(&preimage);
        store.add(5, preimage);

        assert_eq!(store.find_by_hash(&hash), Some(&preimage));
        assert!(store.find_by_hash(&Hash::from_bytes([0u8; 32])).is_none());
    }

    #[test]
    fn revocation_store_no_duplicates() {
        let mut store = RevocationStore::new();
        store.add(0, [1u8; 32]);
        store.add(0, [1u8; 32]); // duplicate
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn revocation_store_multiple_entries() {
        let mut store = RevocationStore::new();
        for i in 0..100u8 {
            store.add(i as u64, [i; 32]);
        }
        assert_eq!(store.len(), 100);
        for i in 0..100u8 {
            assert_eq!(store.get(i as u64), Some(&[i; 32]));
        }
    }

    #[test]
    fn revocation_store_find_by_hash_scans_all() {
        let mut store = RevocationStore::new();
        for i in 0..10u8 {
            store.add(i as u64, [i; 32]);
        }
        // Find the last one
        let hash = revocation_hash(&[9u8; 32]);
        assert_eq!(store.find_by_hash(&hash), Some(&[9u8; 32]));
    }

    // ── CommitmentPair ────────────────────────────────────────────────

    #[test]
    fn commitment_pair_verify_revocation() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(500, 500), &seed);
        assert!(CommitmentPair::verify_revocation(
            &pair.revocation_preimage,
            &pair.revocation_hash
        ));
        assert!(!CommitmentPair::verify_revocation(
            &[0u8; 32],
            &pair.revocation_hash
        ));
    }

    #[test]
    fn commitment_pair_set_remote_revocation() {
        let seed = [42u8; 32];
        let mut pair = CommitmentPair::new(0, ChannelBalance::new(500, 500), &seed);
        assert!(pair.remote_revocation_hash.is_none());

        let remote_hash = Hash::from_bytes([99u8; 32]);
        pair.set_remote_revocation_hash(remote_hash);
        assert_eq!(pair.remote_revocation_hash, Some(remote_hash));
    }

    #[test]
    fn commitment_builds_valid_local_tx() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(700, 300), &seed);
        let funding_hash = crypto::hash::hash(b"funding");
        let local_pk = crypto::hash::hash(b"local");
        let remote_pk = crypto::hash::hash(b"remote");

        let tx = pair
            .build_local_commitment(funding_hash, 0, local_pk, remote_pk, 1144, 1000, 0)
            .unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.inputs[0].prev_tx_hash, funding_hash);
        assert_eq!(tx.inputs[0].output_index, 0);
        assert_eq!(tx.outputs.len(), 2);
        assert_eq!(tx.outputs[0].amount, 700); // to_local
        assert_eq!(tx.outputs[1].amount, 300); // to_remote
    }

    #[test]
    fn commitment_skips_zero_local_balance() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(0, 1000), &seed);
        let tx = pair
            .build_local_commitment(
                crypto::hash::hash(b"f"),
                0,
                crypto::hash::hash(b"l"),
                crypto::hash::hash(b"r"),
                1144,
                1000,
                0,
            )
            .unwrap();
        assert_eq!(tx.outputs.len(), 1); // only to_remote
    }

    #[test]
    fn commitment_skips_zero_remote_balance() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(1000, 0), &seed);
        let tx = pair
            .build_local_commitment(
                crypto::hash::hash(b"f"),
                0,
                crypto::hash::hash(b"l"),
                crypto::hash::hash(b"r"),
                1144,
                1000,
                0,
            )
            .unwrap();
        assert_eq!(tx.outputs.len(), 1); // only to_local
    }

    #[test]
    fn commitment_both_zero_errors() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(0, 0), &seed);
        let result = pair.build_local_commitment(
            crypto::hash::hash(b"f"),
            0,
            crypto::hash::hash(b"l"),
            crypto::hash::hash(b"r"),
            1144,
            0,
            0,
        );
        assert!(result.is_err());
    }

    #[test]
    fn commitment_with_htlcs() {
        let seed = [42u8; 32];
        // Balance reduced by HTLC amounts to maintain invariant
        let mut pair = CommitmentPair::new(0, ChannelBalance::new(350, 500), &seed);
        pair.htlcs.push(InFlightHtlc {
            htlc_id: 0,
            payment_hash: [11u8; 32],
            amount: 100,
            expiry_height: 5000,
            direction: PaymentDirection::Outgoing,
            state: HtlcState::Pending,
            preimage: None,
        });
        pair.htlcs.push(InFlightHtlc {
            htlc_id: 1,
            payment_hash: [22u8; 32],
            amount: 50,
            expiry_height: 6000,
            direction: PaymentDirection::Incoming,
            state: HtlcState::Pending,
            preimage: None,
        });

        // capacity = 350 + 500 + 100 + 50 = 1000
        let tx = pair
            .build_local_commitment(
                crypto::hash::hash(b"f"),
                0,
                crypto::hash::hash(b"l"),
                crypto::hash::hash(b"r"),
                1144,
                1000,
                0,
            )
            .unwrap();

        // to_local + to_remote + 2 HTLCs = 4 outputs
        assert_eq!(tx.outputs.len(), 4);
    }

    #[test]
    fn commitment_rejects_capacity_mismatch() {
        let seed = [42u8; 32];
        let pair = CommitmentPair::new(0, ChannelBalance::new(700, 300), &seed);
        let result = pair.build_local_commitment(
            crypto::hash::hash(b"f"),
            0,
            crypto::hash::hash(b"l"),
            crypto::hash::hash(b"r"),
            1144,
            500, // wrong capacity
            0,
        );
        assert!(result.is_err());
    }

    #[test]
    fn successive_commitments_have_different_revocations() {
        let seed = [42u8; 32];
        let c0 = CommitmentPair::new(0, ChannelBalance::new(500, 500), &seed);
        let c1 = CommitmentPair::new(1, ChannelBalance::new(400, 600), &seed);
        let c2 = CommitmentPair::new(2, ChannelBalance::new(300, 700), &seed);

        assert_ne!(c0.revocation_preimage, c1.revocation_preimage);
        assert_ne!(c1.revocation_preimage, c2.revocation_preimage);
        assert_ne!(c0.revocation_hash, c1.revocation_hash);
    }

    // ── Witness builders ──────────────────────────────────────────────

    #[test]
    fn delayed_claim_witness_right_branch() {
        let kp = KeyPair::generate();
        let hash = crypto::hash::hash(b"signing");
        let w = build_delayed_claim_witness(&hash, &kp);
        assert_eq!(w.signatures.len(), 1);
        assert!(w.preimage.is_none());
        assert_eq!(w.or_branches, vec![true]);
    }

    #[test]
    fn penalty_witness_left_branch_with_preimage() {
        let kp = KeyPair::generate();
        let hash = crypto::hash::hash(b"signing");
        let preimage = [42u8; 32];
        let w = build_penalty_witness(&hash, &kp, &preimage);
        assert_eq!(w.signatures.len(), 1);
        assert_eq!(w.preimage, Some(preimage));
        assert_eq!(w.or_branches, vec![false]);
    }

    #[test]
    fn htlc_claim_witness_left_branch_with_preimage() {
        let kp = KeyPair::generate();
        let hash = crypto::hash::hash(b"signing");
        let preimage = [99u8; 32];
        let w = build_htlc_claim_witness(&hash, &kp, &preimage);
        assert_eq!(w.signatures.len(), 1);
        assert_eq!(w.preimage, Some(preimage));
        assert_eq!(w.or_branches, vec![false]);
    }

    #[test]
    fn htlc_timeout_witness_right_branch_no_preimage() {
        let kp = KeyPair::generate();
        let hash = crypto::hash::hash(b"signing");
        let w = build_htlc_timeout_witness(&hash, &kp);
        assert_eq!(w.signatures.len(), 1);
        assert!(w.preimage.is_none());
        assert_eq!(w.or_branches, vec![true]);
    }

    #[test]
    fn witness_encode_decode_roundtrip() {
        let kp = KeyPair::generate();
        let hash = crypto::hash::hash(b"signing");
        let preimage = [42u8; 32];

        // Test penalty witness
        let w = build_penalty_witness(&hash, &kp, &preimage);
        let encoded = w.encode();
        let decoded = doli_core::Witness::decode(&encoded).unwrap();
        assert_eq!(decoded.signatures.len(), 1);
        assert_eq!(decoded.preimage, Some(preimage));
        assert_eq!(decoded.or_branches, vec![false]);

        // Test delayed claim witness
        let w2 = build_delayed_claim_witness(&hash, &kp);
        let encoded2 = w2.encode();
        let decoded2 = doli_core::Witness::decode(&encoded2).unwrap();
        assert!(decoded2.preimage.is_none());
        assert_eq!(decoded2.or_branches, vec![true]);
    }
}
