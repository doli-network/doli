//! Tests for `PriceAttestation` (TxType=16) validation — Phase 2.1 Oracle M4.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.1
//!
//! M4 scope (4 of the 7 spec rules — the ctx-only + data-only subset):
//!   1. Height gate: reject if `current_height < oracle_activation_height`
//!      with `[ERRTX-ORACLE001]`.
//!   2. Attester must be in `ctx.active_producers`.
//!   3. `epoch_number` must equal the current reward epoch
//!      (`reward_epoch::from_height(current_height)`).
//!   6. Signature must verify against `signer_pubkey` over
//!      `signing_message()`.
//!
//! Rules 4 (`pair_id` -> AMM pool with liquidity >= MINIMUM_LIQUIDITY) and 5
//! (at-most-one per attester per (epoch, pair_id)) are NOT implemented in M4 —
//! they require UTXO access and block-scope tracking respectively, both of
//! which only land at M6 (`apply_block`). M4 is the structural + ctx-only
//! pass; M6 is the deep-context pass. Rule 7 (TX fee) reuses the existing fee
//! validation infrastructure — no new code in M4.
//!
//! Structural validation also covers:
//!   - inputs MUST be empty (PriceAttestation is purely informational)
//!   - outputs MUST be empty (no UTXO mutation at validation time; the
//!     OraclePrice UTXO is created by `apply_block` at epoch boundary in M6)
//!   - `extra_data` MUST decode as a 144-byte `PriceAttestationData`
//!
//! Predecessor commits:
//!   - M1 (d80f127f): `NetworkParams.oracle_activation_height` field
//!   - ME1 (214a2e39): `ERRTX_ORACLE_001/002/003` constants
//!   - M2 (13e1ccd3): `STRUCTURAL_PUBKEY_HASHES_HEX` (not exercised by M4)
//!   - M3 (19960adb): `TxType::PriceAttestation` + `PriceAttestationData`

use crate::consensus::{ConsensusParams, GENESIS_TIME};
use crate::network::Network;
use crate::transaction::{Input, Output, PriceAttestationData, Transaction};
use crate::validation::{validate_transaction, ValidationContext, ValidationError};
use crypto::{Hash, KeyPair, Signature};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a fresh ValidationContext with `oracle_activation_height = 0`
/// (always-on) and a single registered active producer keyed on `kp`.
///
/// `current_height` controls rule 1 (height gate) AND rule 3 (epoch) — the
/// epoch is derived as `reward_epoch::from_height(current_height)`.
fn ctx_with(kp: &KeyPair, current_height: u64) -> ValidationContext {
    let mut ctx = ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        GENESIS_TIME + 120,
        current_height,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_oracle_activation_height(0);
    ctx.active_producers.push(*kp.public_key());
    ctx
}

/// Build a fully-signed PriceAttestation transaction that should pass all
/// four M4 rules when submitted under `ctx_with(&kp, current_height)`.
fn signed_attestation(kp: &KeyPair, current_height: u64) -> Transaction {
    let epoch = crate::consensus::reward_epoch::from_height(current_height);
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: epoch,
        signature: Signature::default(),
    };
    let msg = data.signing_message();
    data.signature = crypto::signature::sign_hash(&msg, kp.private_key());
    Transaction::new_price_attestation(data)
}

/// Assert that `validate_transaction` rejected the tx with an error whose
/// `Display` representation contains `needle`. Centralizes the assertion so
/// every rule's negative-path test can pin the error string shape — drift
/// between `errors_oracle.rs` templates and the emission site fails loudly.
fn assert_rejected_with(result: Result<(), ValidationError>, needle: &str) {
    match result {
        Err(e) => {
            let s = format!("{e}");
            assert!(
                s.contains(needle),
                "expected error to contain {needle:?}, got: {s}"
            );
        }
        Ok(()) => panic!("expected rejection containing {needle:?}, got Ok"),
    }
}

// ---------------------------------------------------------------------------
// Happy path — gates the entire M4 implementation
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — happy path PriceAttestation
//   O1: return — Ok(()) when all 4 M4 rules pass simultaneously
// PATHS:
//   P1: rule-1 OK, rule-2 OK, rule-3 OK, rule-6 OK -> Ok(())
// INPUT PARTITIONS:
//   part-A (P1): current_height = 360 (epoch 1, post-activation 0,
//                 attester in active_producers, signature valid)
// MATRIX: 1 output × 1 path × 1 partition = 1 cell
//   P1×part-A: O1✓
#[test]
fn test_m4_happy_path_accepts_valid_attestation() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert!(
        result.is_ok(),
        "happy-path PriceAttestation must validate Ok, got: {result:?}"
    ); // O1
}

// ---------------------------------------------------------------------------
// Rule 1 — Height gate (oracle_activation_height)
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — rule 1 height gate
//   O1: return — Err containing "[ERRTX-ORACLE001]" when
//                current_height < oracle_activation_height
//   O2: return — Ok(()) when current_height == oracle_activation_height
//                (boundary: comparison is strict <)
//   O3: return — Ok(()) when current_height > oracle_activation_height
// PATHS:
//   P1: current_height < activation -> rule-1 reject
//   P2: current_height == activation -> rule-1 passes, downstream Ok
//   P3: current_height > activation -> rule-1 passes, downstream Ok
// INPUT PARTITIONS:
//   part-A (P1): activation=1000, current_height=999 (just under)
//   part-B (P1): activation=u64::MAX, current_height=any (always-off)
//   part-C (P2): activation=360, current_height=360 (exact boundary)
//   part-D (P3): activation=0, current_height=360 (always-on)
// MATRIX: 3 outputs × 3 paths × 4 partitions = 12 cells (sparse)
//   P1×part-A: O1✓     P1×part-B: O1✓
//   P2×part-C: O2✓
//   P3×part-D: O3✓ (covered by happy-path test above, re-asserted here for
//                   explicit rule-1 coverage)
#[test]
fn test_m4_rule1_height_gate_rejects_pre_activation() {
    let kp = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 999);
    ctx.oracle_activation_height = 1000;
    // Attestation epoch must match current_height=999 (epoch 2) so rule 3
    // doesn't fire first.
    let tx = signed_attestation(&kp, 999);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "[ERRTX-ORACLE001]"); // O1 (P1×part-A)
}

#[test]
fn test_m4_rule1_height_gate_rejects_when_activation_is_u64_max() {
    let kp = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 360);
    ctx.oracle_activation_height = u64::MAX;
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "[ERRTX-ORACLE001]"); // O1 (P1×part-B)
}

#[test]
fn test_m4_rule1_height_gate_boundary_strict_lt() {
    let kp = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 360);
    ctx.oracle_activation_height = 360;
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert!(
        result.is_ok(),
        "current_height == activation must pass (strict < gate), got: {result:?}"
    ); // O2 (P2×part-C)
}

// ---------------------------------------------------------------------------
// Rule 2 — Attester in active_producers
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — rule 2 active-producer check
//   O1: return — Err(...) when signer_pubkey NOT in ctx.active_producers
//   O2: return — Ok(()) when signer_pubkey IS in ctx.active_producers
// PATHS:
//   P1: signer not present in active_producers -> reject
//   P2: signer present in active_producers -> accept (downstream)
// INPUT PARTITIONS:
//   part-A (P1): active_producers = [other_kp.public_key()] (signer missing)
//   part-B (P1): active_producers = [] (empty set)
//   part-C (P2): active_producers = [signer.public_key()] (single member, signer)
// MATRIX: 2 outputs × 2 paths × 3 partitions = 6 cells (sparse)
//   P1×part-A: O1✓     P1×part-B: O1✓
//   P2×part-C: O2✓ (covered by happy-path test, re-asserted via rule 1 boundary)
#[test]
fn test_m4_rule2_rejects_when_signer_not_in_active_producers() {
    let kp = KeyPair::generate();
    let other = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 360);
    // Replace the single active producer with `other` — signer absent.
    ctx.active_producers = vec![*other.public_key()];
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "not in active_producers"); // O1 (P1×part-A)
}

#[test]
fn test_m4_rule2_rejects_when_active_producers_is_empty() {
    let kp = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 360);
    ctx.active_producers.clear();
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "not in active_producers"); // O1 (P1×part-B)
}

// ---------------------------------------------------------------------------
// Rule 3 — epoch_number == reward_epoch(current_height)
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — rule 3 epoch match
//   O1: return — Err(...) when epoch_number != current_epoch
//   O2: return — Ok(()) when epoch_number == current_epoch
// PATHS:
//   P1: stale attestation (epoch_number = current - 1) -> reject
//   P2: future attestation (epoch_number = current + 1) -> reject
//   P3: matched attestation -> accept
// INPUT PARTITIONS:
//   part-A (P1): current_height=720 (epoch 2), epoch_number=1 (stale by 1)
//   part-B (P2): current_height=720 (epoch 2), epoch_number=3 (future by 1)
//   part-C (P3): current_height=720, epoch_number=2 (matched; covered by
//                happy-path under different height, asserted here directly)
// MATRIX: 2 outputs × 3 paths × 3 partitions = sparse
//   P1×part-A: O1✓
//   P2×part-B: O1✓
//   P3×part-C: O2✓
#[test]
fn test_m4_rule3_rejects_stale_epoch() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 720); // epoch 2
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 1, // stale by 1
        signature: Signature::default(),
    };
    data.signature = crypto::signature::sign_hash(&data.signing_message(), kp.private_key());
    let tx = Transaction::new_price_attestation(data);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "epoch_number"); // O1 (P1×part-A)
}

#[test]
fn test_m4_rule3_rejects_future_epoch() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 720); // epoch 2
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 3, // future by 1
        signature: Signature::default(),
    };
    data.signature = crypto::signature::sign_hash(&data.signing_message(), kp.private_key());
    let tx = Transaction::new_price_attestation(data);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "epoch_number"); // O1 (P2×part-B)
}

#[test]
fn test_m4_rule3_accepts_matching_epoch() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 720); // epoch 2
    let tx = signed_attestation(&kp, 720);

    let result = validate_transaction(&tx, &ctx);

    assert!(
        result.is_ok(),
        "epoch_number matching current epoch must accept, got: {result:?}"
    ); // O2 (P3×part-C)
}

// ---------------------------------------------------------------------------
// Rule 6 — Signature verifies against signer_pubkey
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — rule 6 signature verification
//   O1: return — Err(...) when signature does not verify against
//                signer_pubkey over signing_message()
//   O2: return — Ok(()) when signature verifies (covered by happy path)
// PATHS:
//   P1: signature was computed by a different key (forgery) -> reject
//   P2: signed-over fields tampered after signing -> reject
//   P3: zero/default signature -> reject
// INPUT PARTITIONS:
//   part-A (P1): foreign keypair signs the same message; signer_pubkey is kp
//   part-B (P2): legit sig, then flip price_cents
//   part-C (P3): Signature::default() (all-zeros)
// MATRIX: sparse, 1 output × 3 paths × 3 partitions
//   P1×part-A: O1✓     P2×part-B: O1✓     P3×part-C: O1✓
#[test]
fn test_m4_rule6_rejects_foreign_signature() {
    let kp = KeyPair::generate();
    let attacker = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(), // attester claims to be kp
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 1,
        signature: Signature::default(),
    };
    // ...but the attacker signs it with their key.
    data.signature = crypto::signature::sign_hash(&data.signing_message(), attacker.private_key());
    let tx = Transaction::new_price_attestation(data);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "signature"); // O1 (P1×part-A)
}

#[test]
fn test_m4_rule6_rejects_tampered_price_cents() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 1,
        signature: Signature::default(),
    };
    data.signature = crypto::signature::sign_hash(&data.signing_message(), kp.private_key());
    // Tamper AFTER signing.
    data.price_cents = 999;
    let tx = Transaction::new_price_attestation(data);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "signature"); // O1 (P2×part-B)
}

#[test]
fn test_m4_rule6_rejects_zero_signature() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 1,
        signature: Signature::default(), // never signed
    };
    let tx = Transaction::new_price_attestation(data);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "signature"); // O1 (P3×part-C)
}

// ---------------------------------------------------------------------------
// Structural — inputs / outputs / extra_data layout
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — structural shape of
//                  PriceAttestation
//   O1: return — Err(...) when tx.inputs is non-empty
//   O2: return — Err(...) when tx.outputs is non-empty
//   O3: return — Err(...) when tx.extra_data is empty / wrong length
//   O4: return — Err(...) when tx.extra_data fails to deserialize
//                (correct 144-byte length but garbage PublicKey/Signature)
// PATHS:
//   P1: tx with one input -> reject
//   P2: tx with one output -> reject
//   P3: tx with extra_data of length 0 -> reject (cannot decode payload)
//   P4: tx with extra_data of length 143 -> reject (off-by-one)
//   P5: tx with extra_data of length 144 but unparseable as PriceAttestationData
//       -> reject (note: all 32-byte values are valid PublicKey/Hash, but the
//                  Signature is bounded; this case is dominated by rule 6
//                  rejecting an unparseable signature)
// INPUT PARTITIONS:
//   part-A (P1): one Input pointing at a dummy outpoint
//   part-B (P2): one Output of type Normal, 1 DOLI
//   part-C (P3): extra_data = vec![]
//   part-D (P4): extra_data = vec![0u8; 143]
// MATRIX: sparse
//   P1×part-A: O1✓     P2×part-B: O2✓
//   P3×part-C: O3✓     P4×part-D: O3✓
#[test]
fn test_m4_structural_rejects_nonempty_inputs() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut tx = signed_attestation(&kp, 360);
    tx.inputs.push(Input::new(Hash::ZERO, 0));

    let result = validate_transaction(&tx, &ctx);

    assert!(result.is_err(), "non-empty inputs must reject"); // O1
}

#[test]
fn test_m4_structural_rejects_nonempty_outputs() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut tx = signed_attestation(&kp, 360);
    tx.outputs.push(Output::normal(1, Hash::ZERO));

    let result = validate_transaction(&tx, &ctx);

    assert!(result.is_err(), "non-empty outputs must reject"); // O2
}

#[test]
fn test_m4_structural_rejects_empty_extra_data() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut tx = signed_attestation(&kp, 360);
    tx.extra_data.clear();

    let result = validate_transaction(&tx, &ctx);

    assert!(result.is_err(), "empty extra_data must reject"); // O3
}

#[test]
fn test_m4_structural_rejects_wrong_length_extra_data() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    let mut tx = signed_attestation(&kp, 360);
    tx.extra_data.truncate(143); // off-by-one

    let result = validate_transaction(&tx, &ctx);

    assert!(result.is_err(), "143-byte extra_data must reject"); // O3
}

// ---------------------------------------------------------------------------
// M8 — Sunset HALT rejection
// ---------------------------------------------------------------------------

// OUTPUT CONTRACT: fn validate_transaction — sunset rejection
//   O1: return — Err containing "[ERRTX-ORACLE003]" when
//                ctx.oracle_sunset_triggered = true, even if all
//                other M4 rules pass
// PATHS:
//   P1: ctx_with sunset_triggered=true, otherwise-happy attestation
// INPUT PARTITIONS:
//   part-A (P1): otherwise-valid happy-path attestation; flag set
// MATRIX:
//   P1×part-A: O1✓
#[test]
fn test_m8_sunset_triggered_rejects_attestation() {
    let kp = KeyPair::generate();
    let mut ctx = ctx_with(&kp, 360);
    ctx = ctx.with_oracle_sunset_triggered(true);
    let tx = signed_attestation(&kp, 360);

    let result = validate_transaction(&tx, &ctx);

    assert_rejected_with(result, "[ERRTX-ORACLE003]"); // O1
}

// OUTPUT CONTRACT: sunset off-by-default
//   O1: return — Ok(()) when oracle_sunset_triggered = false
//        (default); equivalent to happy-path which already covers
//        this, but pinning the default-false explicitly defends
//        against accidental default-true regressions in
//        ValidationContext::new().
#[test]
fn test_m8_default_sunset_state_is_off() {
    let kp = KeyPair::generate();
    let ctx = ctx_with(&kp, 360);
    // Confirm the default — IMPORTANT: this test would break if a
    // future change defaulted oracle_sunset_triggered to true.
    assert!(
        !ctx.oracle_sunset_triggered,
        "default ValidationContext must have oracle_sunset_triggered = false"
    );
    let tx = signed_attestation(&kp, 360);
    let result = validate_transaction(&tx, &ctx);
    assert!(result.is_ok()); // O1
}
