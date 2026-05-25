//! Tests for `SlashingEvidence::PriceAttestationEquivocation` —
//! Phase 2.1 Oracle M7.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.4.
//!
//! M7 scope:
//!   - Adds the `PriceAttestationEquivocation` variant to
//!     `SlashingEvidence` (data.rs).
//!   - Validates equivocation evidence in `validate_slash_data`
//!     and `validate_slash_data_skip_vdf` (the two paths
//!     SlashProducer validation flows through — full and post-VDF-
//!     parallel-prepass).
//!   - The 100% bond burn comes for free via the existing
//!     `calculate_slash` invoked at epoch boundary in the
//!     producer-state machine — the evidence variant does not
//!     change the penalty.
//!
//! Validation rules (spec §1.4):
//!   1. Both attestations share `signer_pubkey`.
//!   2. `slash_data.producer_pubkey` matches the signer.
//!   3. Same `epoch_number` in both.
//!   4. Same `pair_id` in both.
//!   5. Different `price_cents` (otherwise it's a duplicate, not
//!      equivocation — M4 rule 5 already rejects duplicates).
//!   6. Both signatures verify.
//!   7. Oracle activation height has been crossed (defense-in-depth;
//!      pre-activation no PriceAttestation can be valid anyway, but
//!      explicit gating avoids any fabricated-evidence pathology).

use crate::consensus::{ConsensusParams, GENESIS_TIME};
use crate::network::Network;
use crate::transaction::{PriceAttestationData, SlashData, SlashingEvidence, Transaction, TxType};
use crate::validation::{validate_transaction, ValidationContext, ValidationError};
use crypto::{Hash, KeyPair, PublicKey, Signature};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ctx_post_activation(reporter: &KeyPair) -> ValidationContext {
    let mut ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        720,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_oracle_activation_height(0);
    ctx.active_producers.push(*reporter.public_key());
    ctx
}

fn sign_attestation(
    kp: &KeyPair,
    pair_id: Hash,
    price_cents: u64,
    epoch_number: u64,
) -> PriceAttestationData {
    let mut a = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents,
        pair_id,
        epoch_number,
        signature: Signature::default(),
    };
    a.signature = crypto::signature::sign_hash(&a.signing_message(), kp.private_key());
    a
}

fn slash_tx_with_evidence(target: PublicKey, evidence: SlashingEvidence) -> Transaction {
    let slash_data = SlashData {
        producer_pubkey: target,
        evidence,
        reporter_signature: Signature::default(),
    };
    let bytes = bincode::serialize(&slash_data).expect("serialize SlashData");
    Transaction {
        version: 1,
        tx_type: TxType::SlashProducer,
        inputs: Vec::new(),
        outputs: Vec::new(),
        extra_data: bytes,
    }
}

fn assert_rejected_with(result: Result<(), ValidationError>, needle: &str) {
    match result {
        Err(e) => {
            let s = format!("{e}");
            assert!(
                s.contains(needle),
                "expected error containing {needle:?}, got: {s}"
            );
        }
        Ok(()) => panic!("expected rejection containing {needle:?}, got Ok"),
    }
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — happy path for
//                  SlashingEvidence::PriceAttestationEquivocation
//   O1: return — Ok(()) when all 6 validation rules pass
// PATHS:
//   P1: same signer, same epoch, same pair, different prices,
//       both signatures valid, post-activation -> Ok
// INPUT PARTITIONS:
//   part-A (P1): equivocator kp; pair_id=[0x42;32]; epoch=2;
//                price_1=100, price_2=200. Both signed by equivocator.
// MATRIX: 1 output × 1 path × 1 partition = 1 cell
//   P1×part-A: O1✓
#[test]
fn test_m7_happy_path_accepts_valid_equivocation() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 200, 2);
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );

    let result = validate_transaction(&tx, &ctx);

    assert!(
        result.is_ok(),
        "valid equivocation evidence must accept, got: {result:?}"
    ); // O1
}

// ---------------------------------------------------------------------------
// Rule 1 — same signer in both attestations
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: rule 1
//   O1: return — Err when attestations have different signers
// PATHS / PARTITIONS:
//   part-A: a1 signed by kp_A, a2 signed by kp_B — DIFFERENT signers
//   -> reject
// MATRIX: P1×part-A: O1✓
#[test]
fn test_m7_rejects_different_signers() {
    let a_kp = KeyPair::generate();
    let b_kp = KeyPair::generate();
    let ctx = ctx_post_activation(&a_kp);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&a_kp, pair_id, 100, 2);
    let a2 = sign_attestation(&b_kp, pair_id, 200, 2);
    let tx = slash_tx_with_evidence(
        *a_kp.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "different signers"); // O1
}

// ---------------------------------------------------------------------------
// Rule 2 — producer_pubkey matches signer
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: rule 2
//   O1: return — Err when slash_data.producer_pubkey != signer
// PATHS / PARTITIONS:
//   part-A: producer_pubkey set to OTHER kp; both attestations
//   signed by equivocator. -> reject
#[test]
fn test_m7_rejects_target_mismatch() {
    let equivocator = KeyPair::generate();
    let bystander = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 200, 2);
    let tx = slash_tx_with_evidence(
        *bystander.public_key(), // wrong target
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "does not match slash target"); // O1
}

// ---------------------------------------------------------------------------
// Rule 3 — same epoch
// ---------------------------------------------------------------------------
#[test]
fn test_m7_rejects_cross_epoch_evidence() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 200, 3); // different epoch
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "different epochs");
}

// ---------------------------------------------------------------------------
// Rule 4 — same pair_id
// ---------------------------------------------------------------------------
#[test]
fn test_m7_rejects_different_pair_ids() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_a = Hash::from_bytes([0x42; 32]);
    let pair_b = Hash::from_bytes([0x99; 32]);
    let a1 = sign_attestation(&equivocator, pair_a, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_b, 200, 2);
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "different pairs");
}

// ---------------------------------------------------------------------------
// Rule 5 — different prices (NOT identical attestations)
// ---------------------------------------------------------------------------
#[test]
fn test_m7_rejects_identical_attestations() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 100, 2); // same price
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "identical price_cents");
}

// ---------------------------------------------------------------------------
// Rule 6 — both signatures must verify
// ---------------------------------------------------------------------------
#[test]
fn test_m7_rejects_invalid_first_signature() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let mut a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 200, 2);
    // Tamper a1 AFTER signing -> sig won't verify.
    a1.price_cents = 999;
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "first attestation signature invalid");
}

#[test]
fn test_m7_rejects_invalid_second_signature() {
    let equivocator = KeyPair::generate();
    let ctx = ctx_post_activation(&equivocator);
    let pair_id = Hash::from_bytes([0x42; 32]);
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let mut a2 = sign_attestation(&equivocator, pair_id, 200, 2);
    a2.price_cents = 888;
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "second attestation signature invalid");
}

// ---------------------------------------------------------------------------
// Rule 7 — oracle activation gate (defense-in-depth)
// ---------------------------------------------------------------------------
#[test]
fn test_m7_rejects_pre_activation_evidence() {
    let equivocator = KeyPair::generate();
    let mut ctx = ctx_post_activation(&equivocator);
    ctx.oracle_activation_height = 1_000;
    ctx.current_height = 999;
    let pair_id = Hash::from_bytes([0x42; 32]);
    // Sign attestations claiming the post-activation epoch matching
    // the slash height — but the slash itself fires pre-activation.
    let a1 = sign_attestation(&equivocator, pair_id, 100, 2);
    let a2 = sign_attestation(&equivocator, pair_id, 200, 2);
    let tx = slash_tx_with_evidence(
        *equivocator.public_key(),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: a1,
            attestation_2: a2,
        },
    );
    let result = validate_transaction(&tx, &ctx);
    assert_rejected_with(result, "oracle not activated");
}
