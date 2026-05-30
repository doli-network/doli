//! Tests for `PriceAttestation` (TxType=16) and its `PriceAttestationData`
//! payload — Phase 2.1 Oracle M3.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.1
//!
//! M3 scope: type definition, discriminant, serialization round-trip,
//! signing-message commitment, signer-verify round-trip. NO validation
//! logic (that's M4: height-gate, attester-in-active-producers,
//! epoch-matches, duplicate-attestation-per-epoch, pair-has-pool, etc.).
//!
//! Field layout (spec §1.1):
//!   signer_pubkey:  [u8; 32]  Ed25519 public key of the attesting producer
//!   price_cents:    u64       Attested price in USD cents
//!   pair_id:        [u8; 32]  Asset pair identifier
//!   epoch_number:   u64       The epoch in which this attestation is valid
//!   signature:      [u8; 64]  Ed25519 signature over BLAKE3(
//!                              pair_id || price_cents || epoch_number)
//!
//! Total on-wire size: 32 + 8 + 32 + 8 + 64 = 144 bytes.
//!
//! IMPORTANT: spec §1.1 specifies the signature is over
//! `BLAKE3(pair_id || price_cents || epoch_number)` — NO domain prefix.
//! This deviates from the existing `DelegateBondData` / `RevokeDelegationData`
//! convention which uses `BLAKE3(domain || ...)`. We follow the spec
//! literally because (a) the spec was approved by 5/5 evaluators, and
//! (b) adding a domain prefix would change the on-wire signing-message
//! bytes and silently break any external attester implementation that
//! followed the spec text.
//!
//! These tests pin:
//!   1. `TxType::from_u32(16) == Some(TxType::PriceAttestation)`.
//!   2. `PriceAttestationData` round-trips through `to_bytes`/`from_bytes`
//!      and rejects every wrong length.
//!   3. `signing_message` commits to exactly `(pair_id, price_cents,
//!      epoch_number)` and ignores `signer_pubkey` and `signature`
//!      (same independence pattern as DelegateBondData's signing_message).
//!   4. `Transaction::new_price_attestation(data)` produces the correct
//!      shape (tx_type=PriceAttestation, empty inputs, empty outputs,
//!      extra_data=144 bytes) and `tx.price_attestation_data()` recovers
//!      the original payload.
//!   5. A real `sign_hash` over `signing_message()` verifies against
//!      `signer_pubkey` via `crypto::signature::verify`, and any tamper
//!      to a committed field invalidates the signature.

use crate::transaction::data::PriceAttestationData;
use crate::transaction::types::TxType;
use crate::transaction::Transaction;
use crypto::{Hash, KeyPair, PublicKey, Signature};

fn sample_data() -> PriceAttestationData {
    PriceAttestationData {
        signer_pubkey: PublicKey::from_bytes([0xaa; 32]),
        price_cents: 12_345,
        pair_id: Hash::from_bytes([0xbb; 32]),
        epoch_number: 7,
        signature: Signature::default(),
    }
}

// OUTPUT CONTRACT: fn TxType::from_u32(16) — PriceAttestation discriminant
//   O1: return — Some(TxType::PriceAttestation) for input 16
//   O2: return — None for adjacent free input 23 (TxType=23 is unassigned,
//                                                 must remain so for now)
// PATHS:
//   P1: input = 16 → matches PriceAttestation arm
//   P2: input = 23 → falls through to _ => None
// INPUT PARTITIONS:
//   part-A (P1): the M3 discriminant (16) — exactly one value
//   part-B (P2): a neighboring unassigned discriminant (23) that MUST
//               stay unassigned in M3 — guards against accidentally
//               wiring a wrong arm
// MATRIX: 2 outputs × 2 paths × 1 partition each = 4 cells (sparse — each
//         output is path-specific)
//   P1×part-A: O1✓
//   P2×part-B: O2✓
#[test]
fn test_txtype_16_round_trips_through_from_u32() {
    // P1×part-A
    assert_eq!(
        TxType::from_u32(16),
        Some(TxType::PriceAttestation),
        "TxType::from_u32(16) MUST resolve to PriceAttestation (spec §1.1)"
    );
    // Also assert the reverse: the enum variant has discriminant 16.
    assert_eq!(
        TxType::PriceAttestation as u32,
        16,
        "TxType::PriceAttestation MUST have discriminant exactly 16; \
         the spec §1.1 locks this value as the first free TxType slot."
    );

    // P2×part-B: 23 is still unassigned in this milestone. If a future
    // milestone uses 23, that milestone MUST update this assertion AND
    // the spec.
    assert_eq!(
        TxType::from_u32(23),
        None,
        "TxType discriminant 23 MUST be unassigned in M3 — a non-None \
         here means another milestone wired the wrong arm."
    );
}

// OUTPUT CONTRACT: PriceAttestationData::to_bytes / from_bytes round-trip
//   O1: to_bytes(d).len() — usize, exactly 144 (= PriceAttestationData::BYTES_LEN)
//   O2: from_bytes(to_bytes(d)) — Some(d') where d' == d (full field equality)
//   O3: from_bytes(slice with wrong length) — None
// PATHS:
//   P1: len == 144 → parses
//   P2: len != 144 → returns None
// INPUT PARTITIONS:
//   For P1:
//     part-A: a "typical" attestation (small price, low epoch, varied
//             pubkey/pair_id/signature) — exercises the standard layout
//     part-B: extreme values (price=u64::MAX, epoch=u64::MAX, all-bits
//             pubkey/pair_id/signature) — guards against integer overflow
//             or accidental sign-extension in the encoder
//   For P2:
//     part-C: 143 bytes (one short of the layout)
//     part-D: 145 bytes (one over the layout)
//     part-E: empty slice
//     part-F: 68 bytes (length of legacy DelegateBondData — guards
//             against accidental cross-type parsing)
// MATRIX: 3 outputs × 2 paths × {2,4} partitions = 14 cells
//   P1×part-A: O1✓ O2✓
//   P1×part-B: O1✓ O2✓
//   P2×part-C: O3✓
//   P2×part-D: O3✓
//   P2×part-E: O3✓
//   P2×part-F: O3✓
#[test]
fn test_price_attestation_data_round_trip() {
    // ---- P1 ----
    // part-A: typical
    let d_a = sample_data();
    let bytes_a = d_a.to_bytes();
    assert_eq!(
        bytes_a.len(),
        PriceAttestationData::BYTES_LEN,
        "encoded length must be exactly {} bytes (32+8+32+8+64)",
        PriceAttestationData::BYTES_LEN
    );
    assert_eq!(PriceAttestationData::BYTES_LEN, 144);
    let decoded_a =
        PriceAttestationData::from_bytes(&bytes_a).expect("typical attestation must round-trip");
    assert_eq!(decoded_a, d_a, "typical attestation round-trip mismatch");

    // part-B: extremes
    let d_b = PriceAttestationData {
        signer_pubkey: PublicKey::from_bytes([0xff; 32]),
        price_cents: u64::MAX,
        pair_id: Hash::from_bytes([0xff; 32]),
        epoch_number: u64::MAX,
        signature: Signature::try_from_slice(&[0xff; 64]).expect("64-byte slice is a valid sig"),
    };
    let bytes_b = d_b.to_bytes();
    assert_eq!(bytes_b.len(), PriceAttestationData::BYTES_LEN);
    let decoded_b =
        PriceAttestationData::from_bytes(&bytes_b).expect("extreme attestation must round-trip");
    assert_eq!(decoded_b, d_b, "extreme attestation round-trip mismatch");

    // ---- P2: wrong lengths ----
    // part-C: 143 bytes
    let short = vec![0u8; 143];
    assert!(
        PriceAttestationData::from_bytes(&short).is_none(),
        "143-byte input must be rejected (one short)"
    );
    // part-D: 145 bytes
    let long = vec![0u8; 145];
    assert!(
        PriceAttestationData::from_bytes(&long).is_none(),
        "145-byte input must be rejected (one over)"
    );
    // part-E: empty
    let empty: [u8; 0] = [];
    assert!(
        PriceAttestationData::from_bytes(&empty).is_none(),
        "empty input must be rejected"
    );
    // part-F: 68 bytes (legacy DelegateBondData length)
    let cross = vec![0u8; 68];
    assert!(
        PriceAttestationData::from_bytes(&cross).is_none(),
        "68-byte input (DelegateBondData legacy length) must be rejected — \
         a PriceAttestation parsed from a DelegateBond payload would be a \
         catastrophic cross-type confusion bug."
    );
}

// OUTPUT CONTRACT: fn PriceAttestationData::signing_message — field commitment
//   O1: return — Hash, deterministic for a given (pair_id, price_cents,
//                                                  epoch_number) triple
//   O2: return — DIFFERENT when any of (pair_id, price_cents,
//                                       epoch_number) differs
//   O3: return — SAME when only signer_pubkey or signature differs
//                (those are NOT in the commitment per spec §1.1)
// PATHS:
//   P1: hash computation (no branches)
// INPUT PARTITIONS:
//   part-A: change pair_id only → signing_message MUST change
//   part-B: change price_cents only → signing_message MUST change
//   part-C: change epoch_number only → signing_message MUST change
//   part-D: change signer_pubkey only → signing_message MUST NOT change
//           (signer_pubkey is the VERIFYING key; including it in the
//            commitment would be redundant — same independence pattern
//            as DelegateBondData::signing_message)
//   part-E: change signature only → signing_message MUST NOT change
//           (a signature can never sign over itself)
// MATRIX: 3 outputs × 1 path × 5 partitions = ... we assert one of O1/O2/O3
// per partition:
//   P1×part-A: O2✓
//   P1×part-B: O2✓
//   P1×part-C: O2✓
//   P1×part-D: O3✓
//   P1×part-E: O3✓
//   (O1 is trivially satisfied by the deterministic hash function.)
#[test]
fn test_signing_message_commits_to_pair_id_price_epoch_only() {
    let base = sample_data();
    let m_base = base.signing_message();

    // part-A: different pair_id
    let mut a = base.clone();
    a.pair_id = Hash::from_bytes([0x11; 32]);
    assert_ne!(
        a.signing_message(),
        m_base,
        "signing_message MUST commit to pair_id"
    );

    // part-B: different price_cents
    let mut b = base.clone();
    b.price_cents = base.price_cents.wrapping_add(1);
    assert_ne!(
        b.signing_message(),
        m_base,
        "signing_message MUST commit to price_cents"
    );

    // part-C: different epoch_number
    let mut c = base.clone();
    c.epoch_number = base.epoch_number.wrapping_add(1);
    assert_ne!(
        c.signing_message(),
        m_base,
        "signing_message MUST commit to epoch_number"
    );

    // part-D: different signer_pubkey — MUST NOT change the message
    let mut d = base.clone();
    d.signer_pubkey = PublicKey::from_bytes([0x99; 32]);
    assert_eq!(
        d.signing_message(),
        m_base,
        "signing_message MUST NOT commit to signer_pubkey \
         (it's the verifying key, not part of the signed payload)"
    );

    // part-E: different signature — MUST NOT change the message
    let mut e = base.clone();
    e.signature = Signature::try_from_slice(&[0x55; 64]).expect("64-byte slice is a valid sig");
    assert_eq!(
        e.signing_message(),
        m_base,
        "signing_message MUST NOT commit to signature \
         (a signature cannot sign over itself)"
    );
}

// OUTPUT CONTRACT: fn Transaction::new_price_attestation(data) + accessors
//   O1: result.tx_type — TxType::PriceAttestation
//   O2: result.inputs — empty Vec
//   O3: result.outputs — empty Vec
//   O4: result.extra_data.len() — 144 (= PriceAttestationData::BYTES_LEN)
//   O5: result.is_price_attestation() — true
//   O6: result.price_attestation_data() — Some(data) equal to input
//   O7: (negative) on a non-PriceAttestation tx, price_attestation_data() → None
//   O8: (negative) on a non-PriceAttestation tx, is_price_attestation() → false
// PATHS:
//   P1: Transaction::new_price_attestation construction
//   P2: round-trip through accessors on the constructed tx
//   P3: accessor behavior on a non-PriceAttestation tx (Coinbase used as
//       a structurally-simple non-PriceAttestation example)
// INPUT PARTITIONS:
//   For P1+P2: a single typical PriceAttestationData (already covered by
//              the round-trip test for layout variations)
//   For P3: a Coinbase tx — guards against the predicate returning true
//           for any non-PriceAttestation tx_type
// MATRIX: 8 outputs × 3 paths × 1 partition = matrix cells:
//   P1: O1✓ O2✓ O3✓ O4✓
//   P2: O5✓ O6✓
//   P3: O7✓ O8✓
#[test]
fn test_new_price_attestation_constructor_and_accessors() {
    let data = sample_data();

    // P1
    let tx = Transaction::new_price_attestation(data.clone());
    assert_eq!(tx.tx_type, TxType::PriceAttestation); // O1
    assert!(
        tx.inputs.is_empty(),
        "PriceAttestation tx must have no inputs"
    ); // O2
    assert!(
        tx.outputs.is_empty(),
        "PriceAttestation tx must have no outputs"
    ); // O3
    assert_eq!(
        tx.extra_data.len(),
        PriceAttestationData::BYTES_LEN,
        "extra_data must be exactly {} bytes",
        PriceAttestationData::BYTES_LEN
    ); // O4

    // P2
    assert!(tx.is_price_attestation()); // O5
    let recovered = tx
        .price_attestation_data()
        .expect("price_attestation_data() must succeed for a PriceAttestation tx");
    assert_eq!(
        recovered, data,
        "accessor must recover the original payload"
    ); // O6

    // P3: a different tx type
    let coinbase = Transaction::new_coinbase(100_000_000, Hash::default(), 1, 0);
    assert!(
        coinbase.price_attestation_data().is_none(),
        "price_attestation_data() must return None for a non-PriceAttestation tx"
    ); // O7
    assert!(
        !coinbase.is_price_attestation(),
        "is_price_attestation() must return false for a non-PriceAttestation tx"
    ); // O8
}

// OUTPUT CONTRACT: full sign-and-verify round trip with the real crypto layer
//   O1: For a `PriceAttestationData` whose `signature` was produced by
//       `crypto::signature::sign_hash(signing_message(), private_key)`,
//       `crypto::signature::verify(signing_message.as_bytes(), &signature,
//       &signer_pubkey)` returns Ok(()).
//   O2: For the same data with a tampered (pair_id|price_cents|epoch_number)
//       — i.e., a recomputed signing_message that doesn't match the one
//       that was signed — verification fails.
// PATHS:
//   P1: honest sign → verify (happy path)
//   P2: tamper → verify with the tampered signing_message (must fail)
// INPUT PARTITIONS:
//   part-A: tamper pair_id
//   part-B: tamper price_cents
//   part-C: tamper epoch_number
//   (signer_pubkey/signature tampering is covered by the
//    signing_message-independence test above — once that property holds,
//    tampering them does not affect the signing_message and so cannot
//    be detected here; that's M4's job to reject via separate rules.)
// MATRIX: 2 outputs × 2 paths × {1, 3} partitions = 7 cells
//   P1: O1✓
//   P2×part-A: O2✓
//   P2×part-B: O2✓
//   P2×part-C: O2✓
#[test]
fn test_sign_and_verify_round_trip() {
    let kp = KeyPair::generate();
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: Hash::from_bytes([0x42; 32]),
        epoch_number: 17,
        signature: Signature::default(),
    };
    let msg = data.signing_message();
    data.signature = crypto::signature::sign_hash(&msg, kp.private_key());

    // P1: honest verify
    let v = crypto::signature::verify(msg.as_bytes(), &data.signature, &data.signer_pubkey);
    assert!(v.is_ok(), "honest sign+verify must succeed; err: {v:?}"); // O1

    // P2 part-A: tamper pair_id
    let mut tampered = data.clone();
    tampered.pair_id = Hash::from_bytes([0x99; 32]);
    let bad_msg_a = tampered.signing_message();
    let v_a = crypto::signature::verify(bad_msg_a.as_bytes(), &data.signature, &data.signer_pubkey);
    assert!(
        v_a.is_err(),
        "verify with tampered pair_id must fail; got Ok unexpectedly"
    ); // O2

    // P2 part-B: tamper price_cents
    let mut tampered = data.clone();
    tampered.price_cents = data.price_cents.wrapping_add(1);
    let bad_msg_b = tampered.signing_message();
    let v_b = crypto::signature::verify(bad_msg_b.as_bytes(), &data.signature, &data.signer_pubkey);
    assert!(
        v_b.is_err(),
        "verify with tampered price_cents must fail; got Ok unexpectedly"
    ); // O2

    // P2 part-C: tamper epoch_number
    let mut tampered = data.clone();
    tampered.epoch_number = data.epoch_number.wrapping_add(1);
    let bad_msg_c = tampered.signing_message();
    let v_c = crypto::signature::verify(bad_msg_c.as_bytes(), &data.signature, &data.signer_pubkey);
    assert!(
        v_c.is_err(),
        "verify with tampered epoch_number must fail; got Ok unexpectedly"
    ); // O2
}

// OUTPUT CONTRACT: fn Transaction::is_state_only — PriceAttestation inclusion
//   O1: return — true for TxType::PriceAttestation (AUDIT-P1-003)
//   O2: return — true for TxType::DelegateBond (regression: existing types still state-only)
//   O3: return — false for TxType::Transfer (regression: UTXO-bearing types remain non-state-only)
// PATHS:
//   P1: PriceAttestation tx — new state-only inclusion (the audited bug)
//   P2: DelegateBond tx — pre-existing state-only type
//   P3: Transfer tx — non-state-only baseline
// INPUT PARTITIONS:
//   P1: a fresh PriceAttestation built via Transaction::new_price_attestation
//   P2: a fresh DelegateBond Transaction
//   P3: a fresh Transfer Transaction
// MATRIX:
//   P1×O1✓     P2×O2✓     P3×O3✓
#[test]
fn test_price_attestation_is_state_only() {
    // P1: PriceAttestation must be state-only so the mempool routes it via
    // add_system_transaction (skipping the UTXO-input-based fee check that
    // would otherwise reject a zero-input/zero-output tx with
    // MempoolError::FeeTooLow). See AUDIT-P1-003 in
    // docs/audits/security-audit-oracle-2026-05-29.md.
    let pa = Transaction::new_price_attestation(sample_data());
    assert!(
        pa.is_state_only(),
        "AUDIT-P1-003: PriceAttestation (TxType=16) must be state-only \
         so mempool admission bypasses the input-based fee check. \
         Was the new arm added to is_state_only() in transaction/core.rs?"
    ); // O1

    // P2: regression — pre-existing state-only types still classified as such
    let db = Transaction {
        version: 1,
        tx_type: TxType::DelegateBond,
        inputs: Vec::new(),
        outputs: Vec::new(),
        extra_data: Vec::new(),
    };
    assert!(db.is_state_only(), "DelegateBond must remain state-only"); // O2

    // P3: regression — UTXO-bearing types remain non-state-only
    let xfer = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: Vec::new(),
        outputs: Vec::new(),
        extra_data: Vec::new(),
    };
    assert!(
        !xfer.is_state_only(),
        "Transfer must NOT be state-only"
    ); // O3
}
