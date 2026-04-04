//! Channel condition builders.
//!
//! Constructs the three core conditions used in LN-Penalty channels:
//!
//! 1. **Funding**: 2-of-2 multisig
//! 2. **To-local** (commitment): `Or(And(Sig(counterparty), Hashlock(revocation)), And(Sig(self), Timelock(delay)))`
//! 3. **HTLC**: `Or(And(Sig(remote), Hashlock(payment)), And(Sig(local), TimelockExpiry(expiry)))`
//!
//! All conditions must fit within 4096 bytes of `extra_data`.

use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::transaction::{Output, OutputType, MAX_EXTRA_DATA_SIZE};
use doli_core::{Amount, BlockHeight, ConditionError};

/// Build a 2-of-2 multisig condition for the funding output.
///
/// Keys are sorted lexicographically for deterministic encoding.
pub fn funding_condition(alice_pubkey_hash: Hash, bob_pubkey_hash: Hash) -> Condition {
    let mut keys = vec![alice_pubkey_hash, bob_pubkey_hash];
    keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    Condition::multisig(2, keys)
}

/// Build a funding output: 2-of-2 multisig holding the channel capacity.
pub fn funding_output(
    capacity: Amount,
    alice_pubkey_hash: Hash,
    bob_pubkey_hash: Hash,
) -> Result<Output, ConditionError> {
    let cond = funding_condition(alice_pubkey_hash, bob_pubkey_hash);
    // pubkey_hash is set to alice (funder) for indexing; condition governs spending
    Output::conditioned(OutputType::Multisig, capacity, alice_pubkey_hash, &cond)
}

/// Build the "to_local" condition for a commitment transaction.
///
/// This is the core LN-Penalty construct:
/// - **Penalty path** (left/false): counterparty can sweep immediately if they know the
///   revocation preimage. `And(Sig(counterparty), Hashlock(revocation_hash))`
/// - **Delayed claim** (right/true): owner can claim after the dispute window.
///   `And(Sig(self), Timelock(dispute_height))`
pub fn to_local_condition(
    owner_pubkey_hash: Hash,
    counterparty_pubkey_hash: Hash,
    revocation_hash: Hash,
    dispute_height: BlockHeight,
) -> Condition {
    Condition::Or(
        // Penalty path: counterparty + revocation preimage
        Box::new(Condition::And(
            Box::new(Condition::Signature(counterparty_pubkey_hash)),
            Box::new(Condition::Hashlock(revocation_hash)),
        )),
        // Delayed claim: owner after dispute window
        Box::new(Condition::And(
            Box::new(Condition::Signature(owner_pubkey_hash)),
            Box::new(Condition::Timelock(dispute_height)),
        )),
    )
}

/// Build a "to_local" output for a commitment transaction.
pub fn to_local_output(
    amount: Amount,
    owner_pubkey_hash: Hash,
    counterparty_pubkey_hash: Hash,
    revocation_hash: Hash,
    dispute_height: BlockHeight,
) -> Result<Output, ConditionError> {
    let cond = to_local_condition(
        owner_pubkey_hash,
        counterparty_pubkey_hash,
        revocation_hash,
        dispute_height,
    );
    Output::conditioned(OutputType::Multisig, amount, owner_pubkey_hash, &cond)
}

/// Build a "to_remote" output — just a plain Normal output to the counterparty.
/// No conditions needed: the counterparty can spend immediately.
pub fn to_remote_output(amount: Amount, remote_pubkey_hash: Hash) -> Output {
    Output::normal(amount, remote_pubkey_hash)
}

/// Build an offered HTLC condition (we offered, remote claims with preimage).
///
/// - **Claim path** (left/false): remote reveals preimage. `And(Sig(remote), Hashlock(payment_hash))`
/// - **Timeout path** (right/true): local refunds after expiry. `And(Sig(local), TimelockExpiry(expiry))`
pub fn htlc_offered_condition(
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    payment_hash: Hash,
    expiry_height: BlockHeight,
) -> Condition {
    Condition::Or(
        // Claim: remote + preimage
        Box::new(Condition::And(
            Box::new(Condition::Signature(remote_pubkey_hash)),
            Box::new(Condition::Hashlock(payment_hash)),
        )),
        // Timeout: local after expiry
        Box::new(Condition::And(
            Box::new(Condition::Signature(local_pubkey_hash)),
            Box::new(Condition::TimelockExpiry(expiry_height)),
        )),
    )
}

/// Build an offered HTLC output.
pub fn htlc_offered_output(
    amount: Amount,
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    payment_hash: Hash,
    expiry_height: BlockHeight,
) -> Result<Output, ConditionError> {
    let cond = htlc_offered_condition(
        local_pubkey_hash,
        remote_pubkey_hash,
        payment_hash,
        expiry_height,
    );
    Output::conditioned(OutputType::HTLC, amount, local_pubkey_hash, &cond)
}

/// Build a received HTLC condition (remote offered, we claim with preimage).
///
/// - **Claim path** (left/false): local reveals preimage. `And(Sig(local), Hashlock(payment_hash))`
/// - **Timeout path** (right/true): remote refunds after expiry. `And(Sig(remote), TimelockExpiry(expiry))`
pub fn htlc_received_condition(
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    payment_hash: Hash,
    expiry_height: BlockHeight,
) -> Condition {
    Condition::Or(
        // Claim: local + preimage
        Box::new(Condition::And(
            Box::new(Condition::Signature(local_pubkey_hash)),
            Box::new(Condition::Hashlock(payment_hash)),
        )),
        // Timeout: remote after expiry
        Box::new(Condition::And(
            Box::new(Condition::Signature(remote_pubkey_hash)),
            Box::new(Condition::TimelockExpiry(expiry_height)),
        )),
    )
}

/// Build a received HTLC output.
pub fn htlc_received_output(
    amount: Amount,
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    payment_hash: Hash,
    expiry_height: BlockHeight,
) -> Result<Output, ConditionError> {
    let cond = htlc_received_condition(
        local_pubkey_hash,
        remote_pubkey_hash,
        payment_hash,
        expiry_height,
    );
    Output::conditioned(OutputType::HTLC, amount, local_pubkey_hash, &cond)
}

/// Verify that a condition encodes within the extra_data size limit.
pub fn verify_encoding_size(condition: &Condition) -> Result<usize, ConditionError> {
    let encoded = condition.encode()?;
    if encoded.len() > MAX_EXTRA_DATA_SIZE {
        return Err(ConditionError::EncodingTooLarge {
            size: encoded.len(),
        });
    }
    Ok(encoded.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash;
    use doli_core::conditions::{
        evaluate as evaluate_condition, EvalContext, Witness, WitnessSignature,
    };
    use doli_core::transaction::MAX_EXTRA_DATA_SIZE;

    fn test_hash(val: u8) -> Hash {
        hash(&[val])
    }

    // ── Size budget tests ─────────────────────────────────────────────

    #[test]
    fn funding_condition_fits_in_extra_data() {
        let cond = funding_condition(test_hash(1), test_hash(2));
        let size = verify_encoding_size(&cond).unwrap();
        assert!(size <= MAX_EXTRA_DATA_SIZE);
        // 2-of-2 multisig: version(1) + tag(1) + threshold(1) + count(1) + 2*32 = 68
        assert!(size <= 70, "funding should be ~68 bytes, got {}", size);
    }

    #[test]
    fn to_local_condition_fits_in_extra_data() {
        let cond = to_local_condition(test_hash(1), test_hash(2), test_hash(3), 1000);
        let size = verify_encoding_size(&cond).unwrap();
        assert!(size <= MAX_EXTRA_DATA_SIZE);
        // Or(And(Sig(32), Hashlock(32)), And(Sig(32), Timelock(8))) + tags
        assert!(size <= 120, "to_local should be ~112 bytes, got {}", size);
    }

    #[test]
    fn htlc_offered_condition_fits_in_extra_data() {
        let cond = htlc_offered_condition(test_hash(1), test_hash(2), test_hash(3), 5000);
        let size = verify_encoding_size(&cond).unwrap();
        assert!(size <= MAX_EXTRA_DATA_SIZE);
        assert!(
            size <= 120,
            "htlc_offered should be ~112 bytes, got {}",
            size
        );
    }

    #[test]
    fn htlc_received_condition_fits_in_extra_data() {
        let cond = htlc_received_condition(test_hash(1), test_hash(2), test_hash(3), 5000);
        let size = verify_encoding_size(&cond).unwrap();
        assert!(size <= MAX_EXTRA_DATA_SIZE);
        assert!(
            size <= 120,
            "htlc_received should be ~112 bytes, got {}",
            size
        );
    }

    // ── Determinism tests ─────────────────────────────────────────────

    #[test]
    fn funding_condition_keys_are_sorted() {
        let a = test_hash(1);
        let b = test_hash(2);
        assert_eq!(funding_condition(a, b), funding_condition(b, a));
    }

    #[test]
    fn conditions_are_deterministic() {
        let a = test_hash(1);
        let b = test_hash(2);
        let r = test_hash(3);
        // Same inputs → same condition
        assert_eq!(
            to_local_condition(a, b, r, 1000).encode().unwrap(),
            to_local_condition(a, b, r, 1000).encode().unwrap()
        );
        assert_eq!(
            htlc_offered_condition(a, b, r, 5000).encode().unwrap(),
            htlc_offered_condition(a, b, r, 5000).encode().unwrap()
        );
    }

    #[test]
    fn different_params_different_conditions() {
        let a = test_hash(1);
        let b = test_hash(2);
        let r1 = test_hash(3);
        let r2 = test_hash(4);
        // Different revocation hash → different condition
        assert_ne!(
            to_local_condition(a, b, r1, 1000).encode().unwrap(),
            to_local_condition(a, b, r2, 1000).encode().unwrap()
        );
        // Different dispute height → different condition
        assert_ne!(
            to_local_condition(a, b, r1, 1000).encode().unwrap(),
            to_local_condition(a, b, r1, 2000).encode().unwrap()
        );
    }

    // ── Encode/decode round-trip tests ────────────────────────────────

    #[test]
    fn funding_output_roundtrip() {
        let alice = test_hash(1);
        let bob = test_hash(2);
        let output = funding_output(5_000_000, alice, bob).unwrap();
        assert_eq!(output.output_type, OutputType::Multisig);
        assert_eq!(output.amount, 5_000_000);
        let decoded = output.condition().unwrap().unwrap();
        assert_eq!(decoded, funding_condition(alice, bob));
    }

    #[test]
    fn to_local_output_roundtrip() {
        let owner = test_hash(1);
        let cp = test_hash(2);
        let rev = test_hash(3);
        let output = to_local_output(1_000_000, owner, cp, rev, 1000).unwrap();
        assert_eq!(output.output_type, OutputType::Multisig);
        let decoded = output.condition().unwrap().unwrap();
        assert_eq!(decoded, to_local_condition(owner, cp, rev, 1000));
    }

    #[test]
    fn htlc_offered_output_roundtrip() {
        let local = test_hash(1);
        let remote = test_hash(2);
        let ph = test_hash(3);
        let output = htlc_offered_output(50_000, local, remote, ph, 5000).unwrap();
        assert_eq!(output.output_type, OutputType::HTLC);
        assert_eq!(output.amount, 50_000);
        let decoded = output.condition().unwrap().unwrap();
        assert_eq!(decoded, htlc_offered_condition(local, remote, ph, 5000));
    }

    #[test]
    fn htlc_received_output_roundtrip() {
        let local = test_hash(1);
        let remote = test_hash(2);
        let ph = test_hash(3);
        let output = htlc_received_output(30_000, local, remote, ph, 4000).unwrap();
        assert_eq!(output.output_type, OutputType::HTLC);
        let decoded = output.condition().unwrap().unwrap();
        assert_eq!(decoded, htlc_received_condition(local, remote, ph, 4000));
    }

    // ── Structure validation tests ────────────────────────────────────

    #[test]
    fn to_local_condition_structure() {
        let owner = test_hash(1);
        let cp = test_hash(2);
        let rev = test_hash(3);
        let cond = to_local_condition(owner, cp, rev, 1000);

        match cond {
            Condition::Or(penalty, delayed) => {
                match *penalty {
                    Condition::And(sig, hashlock) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == cp));
                        assert!(matches!(*hashlock, Condition::Hashlock(h) if h == rev));
                    }
                    _ => panic!("penalty path should be And(Sig, Hashlock)"),
                }
                match *delayed {
                    Condition::And(sig, timelock) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == owner));
                        assert!(matches!(*timelock, Condition::Timelock(1000)));
                    }
                    _ => panic!("delayed path should be And(Sig, Timelock)"),
                }
            }
            _ => panic!("to_local should be Or"),
        }
    }

    #[test]
    fn htlc_offered_condition_structure() {
        let local = test_hash(1);
        let remote = test_hash(2);
        let ph = test_hash(3);
        let cond = htlc_offered_condition(local, remote, ph, 5000);

        match cond {
            Condition::Or(claim, timeout) => {
                match *claim {
                    Condition::And(sig, hashlock) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == remote));
                        assert!(matches!(*hashlock, Condition::Hashlock(h) if h == ph));
                    }
                    _ => panic!("claim path should be And(Sig, Hashlock)"),
                }
                match *timeout {
                    Condition::And(sig, expiry) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == local));
                        assert!(matches!(*expiry, Condition::TimelockExpiry(5000)));
                    }
                    _ => panic!("timeout path should be And(Sig, TimelockExpiry)"),
                }
            }
            _ => panic!("htlc_offered should be Or"),
        }
    }

    #[test]
    fn htlc_received_condition_structure() {
        let local = test_hash(1);
        let remote = test_hash(2);
        let ph = test_hash(3);
        let cond = htlc_received_condition(local, remote, ph, 5000);

        match cond {
            Condition::Or(claim, timeout) => {
                // Claim: local + preimage
                match *claim {
                    Condition::And(sig, hashlock) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == local));
                        assert!(matches!(*hashlock, Condition::Hashlock(h) if h == ph));
                    }
                    _ => panic!("claim path should be And(Sig, Hashlock)"),
                }
                // Timeout: remote after expiry
                match *timeout {
                    Condition::And(sig, expiry) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == remote));
                        assert!(matches!(*expiry, Condition::TimelockExpiry(5000)));
                    }
                    _ => panic!("timeout path should be And(Sig, TimelockExpiry)"),
                }
            }
            _ => panic!("htlc_received should be Or"),
        }
    }

    // ── Condition evaluation tests (L1 compatibility) ─────────────────

    #[test]
    fn to_local_penalty_path_evaluates() {
        let counterparty_kp = crypto::KeyPair::generate();
        let cp_pubkey_hash = crypto::hash::hash_with_domain(
            crypto::ADDRESS_DOMAIN,
            counterparty_kp.public_key().as_bytes(),
        );

        let revocation_preimage = [42u8; 32];
        let rev_hash =
            crypto::hash::hash_with_domain(doli_core::HASHLOCK_DOMAIN, &revocation_preimage);

        let owner_hash = test_hash(99);
        let cond = to_local_condition(owner_hash, cp_pubkey_hash, rev_hash, 1000);

        let signing_hash = hash(b"test_signing_hash");
        let sig = crypto::signature::sign_hash(&signing_hash, counterparty_kp.private_key());

        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *counterparty_kp.public_key(),
                signature: sig,
            }],
            preimage: Some(revocation_preimage),
            or_branches: vec![false], // left = penalty
        };

        let ctx = EvalContext {
            current_height: 500, // before dispute window
            signing_hash: &signing_hash,
            transaction: None,
        };
        let mut branch_idx = 0;
        assert!(evaluate_condition(&cond, &witness, &ctx, &mut branch_idx));
    }

    #[test]
    fn to_local_delayed_path_evaluates() {
        let owner_kp = crypto::KeyPair::generate();
        let owner_pubkey_hash = crypto::hash::hash_with_domain(
            crypto::ADDRESS_DOMAIN,
            owner_kp.public_key().as_bytes(),
        );

        let cp_hash = test_hash(2);
        let rev_hash = test_hash(3);
        let dispute_height = 1000;
        let cond = to_local_condition(owner_pubkey_hash, cp_hash, rev_hash, dispute_height);

        let signing_hash = hash(b"test_signing_hash");
        let sig = crypto::signature::sign_hash(&signing_hash, owner_kp.private_key());

        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *owner_kp.public_key(),
                signature: sig,
            }],
            preimage: None,
            or_branches: vec![true], // right = delayed
        };

        // Should fail before dispute window
        let ctx_early = EvalContext {
            current_height: 999,
            signing_hash: &signing_hash,
            transaction: None,
        };
        let mut branch_idx = 0;
        assert!(!evaluate_condition(
            &cond,
            &witness,
            &ctx_early,
            &mut branch_idx
        ));

        // Should succeed at or after dispute window
        let ctx_after = EvalContext {
            current_height: 1000,
            signing_hash: &signing_hash,
            transaction: None,
        };
        branch_idx = 0;
        assert!(evaluate_condition(
            &cond,
            &witness,
            &ctx_after,
            &mut branch_idx
        ));
    }

    #[test]
    fn to_local_penalty_fails_with_wrong_preimage() {
        let cp_kp = crypto::KeyPair::generate();
        let cp_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, cp_kp.public_key().as_bytes());

        let real_preimage = [42u8; 32];
        let rev_hash = crypto::hash::hash_with_domain(doli_core::HASHLOCK_DOMAIN, &real_preimage);
        let cond = to_local_condition(test_hash(1), cp_hash, rev_hash, 1000);

        let signing_hash = hash(b"test");
        let sig = crypto::signature::sign_hash(&signing_hash, cp_kp.private_key());

        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *cp_kp.public_key(),
                signature: sig,
            }],
            preimage: Some([99u8; 32]), // WRONG preimage
            or_branches: vec![false],
        };

        let ctx = EvalContext {
            current_height: 500,
            signing_hash: &signing_hash,
            transaction: None,
        };
        let mut branch_idx = 0;
        assert!(!evaluate_condition(&cond, &witness, &ctx, &mut branch_idx));
    }

    // ── to_remote is just Normal ──────────────────────────────────────

    #[test]
    fn to_remote_output_is_normal() {
        let remote = test_hash(1);
        let output = to_remote_output(500_000, remote);
        assert_eq!(output.output_type, OutputType::Normal);
        assert_eq!(output.amount, 500_000);
        assert_eq!(output.pubkey_hash, remote);
        assert!(output.extra_data.is_empty());
    }

    // ── Funding output type is Multisig ───────────────────────────────

    #[test]
    fn funding_output_is_multisig() {
        let output = funding_output(1_000_000, test_hash(1), test_hash(2)).unwrap();
        assert_eq!(output.output_type, OutputType::Multisig);
    }

    // ── Edge case: same keys ──────────────────────────────────────────

    #[test]
    fn funding_condition_same_keys() {
        let k = test_hash(1);
        let cond = funding_condition(k, k);
        // Still valid, though degenerate
        verify_encoding_size(&cond).unwrap();
    }

    // ── Edge case: max height values ──────────────────────────────────

    #[test]
    fn conditions_with_max_height() {
        let cond = to_local_condition(test_hash(1), test_hash(2), test_hash(3), u64::MAX);
        verify_encoding_size(&cond).unwrap();

        let htlc = htlc_offered_condition(test_hash(1), test_hash(2), test_hash(3), u64::MAX);
        verify_encoding_size(&htlc).unwrap();
    }

    #[test]
    fn conditions_with_zero_height() {
        let cond = to_local_condition(test_hash(1), test_hash(2), test_hash(3), 0);
        verify_encoding_size(&cond).unwrap();

        let htlc = htlc_offered_condition(test_hash(1), test_hash(2), test_hash(3), 0);
        verify_encoding_size(&htlc).unwrap();
    }
}
