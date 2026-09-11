//! INC-I-217 M5 — `validation::rotate_bls::rotate_stateless`: the stateless consensus
//! rules of a `RotateBlsKey` transaction, and their wiring into `validate_transaction`.
//!
// covers: crates/core/src/validation/rotate_bls.rs, crates/core/src/validation/transaction.rs, crates/core/src/validation/error.rs
//!
//! Requirements: **REQ-ROT-SEC-007** (Must — payload shape, length, point validity,
//! producer binding, authorisation signature, rotation proof of possession) and
//! **REQ-ROT-SEC-003** (Must — the rotation is bound to the outpoint it spends, so a
//! captured rotation cannot be replayed onto another transaction).
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `doli_core::validation::rotate_bls` does not exist and neither does
//! `ValidationContext::with_bls_key_rotation_activation_height`. That compile failure is
//! the red, exactly as `inc_i_208_activation_height.rs` documents for itself.
//!
//! WHY AN INJECTED ACTIVATION HEIGHT IS USED HERE. `bls_key_rotation_activation_height`
//! ships at `u64::MAX` on all three networks (pinned in
//! `inc_i_217_m5_activation_height.rs`), so the ABOVE-gate side is UNREACHABLE from
//! shipped params until the pinning decision-session. This module therefore injects a low
//! height through the builder to reach the corpus at all. The BELOW-gate side IS reachable
//! from shipped params today and is exercised from them — here in
//! `req_rot_sec_007_the_gate_precedes_every_other_check` and, at block level, throughout
//! `inc_i_217_m5_block_gate.rs` (INV-GOV-001: both sides exercised, the reachable side
//! derived from the shipped values, never from a literal).
//!
//! WHY EVERY NEGATIVE IS PAIRED. `assert_rejected_with` re-runs the UNMUTATED fixture and
//! asserts it still returns `Ok` before it judges the mutant. A negative on its own cannot
//! distinguish "the rule fired" from "the fixture is broken", and a broken fixture would
//! make all 21 negatives pass while the validator does nothing.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: rotate_stateless(&Transaction, &ValidationContext)
//         -> Result<RotateBlsData, ValidationError>
//       O1 return Ok(payload): the DECODED payload — not a bool, so the test asserts the
//          returned value equals the payload that went in (a validator that returned a
//          default-constructed `RotateBlsData` would pass an `is_ok()` check)
//       O1' return Err(e): `e.to_string()` MUST carry the specific `[ERRTX-ROTnnn]` code;
//          a generic rejection is a distinct failure from the right rejection
//       O2 mutable params: NONE — both arguments are `&`
//       O3 receiver: NONE — free function
//       O4 persistent store writes: NONE — `rotate_stateless` is stateless by contract
//          (registration status, key uniqueness and one-pending are M6/M7)
//       O5 statics / O6 channels / O7 logs: NONE claimed
//       PATHS (the wire-contract check order, cheap/structural before the two
//       pairing-bearing checks, so a spam block cannot make validators pay pairings for
//       transactions that fail on shape):
//          P1 below activation height        -> ROT002
//          P2 inputs.len() != 1              -> ROT001
//          P3 outputs.len() != 1 or !Normal  -> ROT009
//          P4 extra_data len != 240 / decode -> ROT010
//          P5 new_bls_pubkey not a G1 point  -> ROT004
//          P6 producer != input public key   -> ROT008
//          P7 Ed25519 authorisation invalid  -> ROT003
//          P8 rotation PoP invalid           -> ROT007
//          P9 all pass                       -> Ok(payload)
//   F2: validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//       O1 return: `Ok(())` or the SAME `ValidationError` F1 produced — the dispatch arm
//          must delegate, not re-implement or swallow
//       PATHS (this suite): the `RotateBlsKey` arm on P9, P2, P7 and P8.
//
//   INPUT PARTITIONS (every partition is exactly ONE mutation from the positive fixture):
//     I1  0 inputs                     I2  2 inputs
//     I3  0 outputs                    I4  2 outputs           I5  output 0 is Bond
//     I6  extra_data 239 bytes         I7  241 bytes           I8  empty
//     I9  new_bls_pubkey 0xFF*48       I10 0x00*48             I11 compressed G1 identity
//     I12 payload.producer = a different Ed25519 key
//     I13 inputs[0].public_key = None
//     I14 signature = 64 zero bytes    I15 signature by a different Ed25519 key
//     I16 signature over a DIFFERENT outpoint (the replay bind)
//     I17 the whole valid tuple lifted onto a tx spending a different outpoint
//     I18 PoP under the REGISTRATION DST                I19 PoP bound to another producer
//     I20 PoP bound to another BLS key                  I21 bls_pop = 96 zero bytes
//     I22 shape AND PoP both violated (check-order probe)
//     I23 a below-gate context derived from the SHIPPED params
//     I24 the unmutated positive (asserted before EVERY negative)
//
//   MATRIX 2 functions x 9 paths x 24 partitions: every reachable cell has a named test.
//   NOT CLAIMED HERE: stateful verdicts (NotProducer / SameKey / AlreadyPending /
//   KeyInUse) — those are M6/M7 and `rotate_stateless` must not consult state at all.

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::genesis_hash;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{
    rotation_auth_digest, Input, Output, OutputType, RotateBlsData, Transaction, TxType,
};
use doli_core::validation::rotate_bls::rotate_stateless;
use doli_core::validation::{validate_transaction, ValidationContext};
use doli_core::Network;

const NETWORK: Network = Network::Mainnet;

/// Injected rotation activation height. See the header: the shipped value is `u64::MAX`
/// on every network, so the above-gate corpus is unreachable without an injection.
const ROTATION_AH: u64 = 500;

/// Comfortably above `ROTATION_AH` and below every other gate's default (`u64::MAX`).
const HEIGHT_ABOVE_GATE: u64 = 1_000;

// ---------------------------------------------------------------------------
// Fixture — ONE well-formed rotation plus mutators, so every negative below is
// exactly one mutation away from a transaction that validates.
// ---------------------------------------------------------------------------

fn ctx_above_gate() -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), NETWORK, 0, HEIGHT_ABOVE_GATE)
        .with_bls_key_rotation_activation_height(ROTATION_AH)
}

fn producer_kp() -> KeyPair {
    KeyPair::from_seed([0x21; 32])
}

fn other_kp() -> KeyPair {
    KeyPair::from_seed([0x22; 32])
}

fn new_bls() -> BlsKeyPair {
    BlsKeyPair::from_seed(&[0x23; 32]).expect("32-byte BLS seed is valid")
}

fn other_bls() -> BlsKeyPair {
    BlsKeyPair::from_seed(&[0x24; 32]).expect("32-byte BLS seed is valid")
}

/// The outpoint the positive fixture spends.
fn outpoint_a() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m5-outpoint-a"), 3)
}

/// A different outpoint, for the two replay-bind negatives.
fn outpoint_b() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m5-outpoint-b"), 11)
}

/// Ed25519 authorisation over the 32-byte `rotation_auth_digest` — raw digest bytes, no
/// second domain tag (the M10 CLI signs exactly this, so a `sign_message` wrapper here
/// would make every CLI-produced rotation unverifiable).
fn auth_signature(kp: &KeyPair, new_bls_pubkey: &[u8; 48], outpoint: (Hash, u32)) -> [u8; 64] {
    let digest = rotation_auth_digest(
        genesis_hash(NETWORK).as_bytes(),
        new_bls_pubkey,
        outpoint.0.as_bytes(),
        outpoint.1,
    );
    *crypto::signature::sign(&digest, kp.private_key()).as_bytes()
}

/// Proof of possession under the ROTATION DST, bound to `(new key, genesis, producer)`.
fn rotation_pop(bls: &BlsKeyPair, producer: &PublicKey) -> [u8; 96] {
    *crypto::sign_rotation_pop(
        bls.secret_key(),
        bls.public_key(),
        genesis_hash(NETWORK).as_bytes(),
        producer.as_bytes(),
    )
    .expect("rotation PoP signing cannot fail for a valid key")
    .as_bytes()
}

/// A payload every rule accepts, authorising `producer_kp` to install `new_bls`'s key
/// while spending `outpoint`.
fn well_formed_payload(outpoint: (Hash, u32)) -> RotateBlsData {
    let kp = producer_kp();
    let bls = new_bls();
    let new_bls_pubkey = *bls.public_key().as_bytes();
    RotateBlsData {
        producer: *kp.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: rotation_pop(&bls, kp.public_key()),
        signature: auth_signature(&kp, &new_bls_pubkey, outpoint),
    }
}

/// A 1-in / 1-out `RotateBlsKey` transaction carrying `payload` and spending `outpoint`.
/// The input reveals the producer key — "the fee payer IS the producer".
fn tx_carrying(payload: &RotateBlsData, outpoint: (Hash, u32)) -> Transaction {
    let mut input = Input::new(outpoint.0, outpoint.1);
    input.public_key = Some(*producer_kp().public_key());
    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(
            1_000_000,
            crypto::hash::hash(b"inc-i-217-m5-change"),
        )],
        extra_data: payload.encode().to_vec(),
    }
}

/// The positive fixture: `(transaction, the payload it carries)`.
fn well_formed() -> (Transaction, RotateBlsData) {
    let outpoint = outpoint_a();
    let payload = well_formed_payload(outpoint);
    (tx_carrying(&payload, outpoint), payload)
}

/// Mutate the payload of the positive fixture, keeping the outpoint and the transaction
/// shape identical, and re-encode.
fn mutate_payload(edit: impl FnOnce(&mut RotateBlsData)) -> Transaction {
    let outpoint = outpoint_a();
    let mut payload = well_formed_payload(outpoint);
    edit(&mut payload);
    tx_carrying(&payload, outpoint)
}

/// Mutate the transaction shape, leaving the payload untouched.
fn mutate_tx(edit: impl FnOnce(&mut Transaction)) -> Transaction {
    let (mut tx, _) = well_formed();
    edit(&mut tx);
    tx
}

/// Assert the POSITIVE still validates, then that `tx` is rejected with exactly `code`.
fn assert_rejected_with(tx: &Transaction, code: &str, why: &str) {
    let (good, payload) = well_formed();
    let control = rotate_stateless(&good, &ctx_above_gate());
    assert_eq!(
        control.as_ref().ok(),
        Some(&payload),
        "PAIRED POSITIVE BROKEN: the unmutated fixture must still validate, or the \
         rejection below proves nothing about the rule under test. Got: {control:?}"
    );

    let verdict = rotate_stateless(tx, &ctx_above_gate());
    let err = verdict.expect_err(&format!("expected {code}: {why}"));
    assert!(
        err.to_string().contains(code),
        "expected {code} ({why}), got: {err}"
    );
}

// ===========================================================================
// The positive — REQ-ROT-SEC-007
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the validator accepts a rotation that satisfies every
// rule, i.e. whether the feature can ever work at all. A failure here makes all 21
// negatives below meaningless, because "reject everything" would satisfy them.
#[test]
fn req_rot_sec_007_a_well_formed_rotation_validates_and_returns_its_payload() {
    let (tx, payload) = well_formed();

    let returned = rotate_stateless(&tx, &ctx_above_gate())
        .expect("a rotation satisfying every rule must validate above the gate");

    // O1: the DECODED payload, not a placeholder — a validator that returned a
    // default-constructed value would pass an `is_ok()` check and mis-install the key.
    assert_eq!(
        returned, payload,
        "rotate_stateless must return the payload it verified, byte for byte"
    );
    assert_eq!(returned.producer, *producer_kp().public_key().as_bytes());
    assert_eq!(returned.new_bls_pubkey, *new_bls().public_key().as_bytes());

    // O2/O3/O4: both arguments are `&`; re-reading the transaction proves the validator
    // mutated nothing observable and consulted no state.
    let (again, _) = well_formed();
    assert_eq!(tx, again);
}

// REQ-ROT-SEC-007 — Decision: whether the M4 fail-closed dispatch arm was actually replaced
// by a delegating call. `rotate_stateless` can be perfect and still unreachable if
// `validate_transaction` never calls it — that is the whole INV-VALIDATION-001 failure
// shape, and it is invisible to every direct-drive test above.
#[test]
fn req_rot_sec_007_validate_transaction_accepts_the_same_well_formed_rotation() {
    let (tx, _) = well_formed();

    assert!(
        validate_transaction(&tx, &ctx_above_gate()).is_ok(),
        "the RotateBlsKey dispatch arm must delegate to rotate_stateless and accept a \
         rotation that satisfies every rule above the gate"
    );

    // The re-export `doli_core::validation::rotate_stateless` is the symbol the M6
    // mempool admission path imports; losing it would break that wiring silently.
    assert!(doli_core::validation::rotate_stateless(&tx, &ctx_above_gate()).is_ok());
}

// ===========================================================================
// Check order and the activation gate — REQ-ROT-SEC-007
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the two pairing-bearing checks run before the cheap
// structural ones. If they do, a block full of rotations that fail on SHAPE still costs
// every validator a BLS pairing each — a free CPU-exhaustion vector at block-validation
// time. The transaction below violates both the input-count rule and the PoP rule; the
// code that comes back names which check ran first.
#[test]
fn req_rot_sec_007_shape_is_checked_before_the_proof_of_possession() {
    let mut tx = mutate_payload(|p| p.bls_pop = [0u8; 96]);
    tx.inputs.clear();

    let err = rotate_stateless(&tx, &ctx_above_gate())
        .expect_err("a transaction violating both rules must be rejected");
    assert!(
        err.to_string().contains("[ERRTX-ROT001]"),
        "the cheap shape check must run FIRST so a malformed spam transaction never \
         costs a pairing; expected [ERRTX-ROT001], got: {err}"
    );
}

// REQ-ROT-SEC-007 — Decision: whether the activation gate precedes every other check, and
// whether the BELOW-gate side is reachable from the SHIPPED parameters. INV-GOV-001: the
// height is read from `NetworkParams::defaults`, never written as a literal, so a future
// re-pin cannot silently move this row to the other side of the gate.
#[test]
fn req_rot_sec_007_the_gate_precedes_every_other_check() {
    let shipped = NetworkParams::defaults(NETWORK).bls_key_rotation_activation_height;
    let ctx = ValidationContext::new(ConsensusParams::mainnet(), NETWORK, 0, HEIGHT_ABOVE_GATE)
        .with_bls_key_rotation_activation_height(shipped);
    assert!(
        ctx.current_height < ctx.bls_key_rotation_activation_height,
        "precondition: this probe is only meaningful BELOW the shipped gate; if a future \
         re-pin put {HEIGHT_ABOVE_GATE} above it, this test needs a new height, not deletion"
    );

    // Even a rotation that is malformed in three ways must report the GATE, because
    // below the height the transaction type does not exist as far as consensus is
    // concerned — an un-upgraded node cannot decode it and must agree on the verdict.
    let mut tx = mutate_payload(|p| {
        p.bls_pop = [0u8; 96];
        p.signature = [0u8; 64];
    });
    tx.outputs.clear();

    let err = rotate_stateless(&tx, &ctx).expect_err("below the gate every rotation is invalid");
    assert!(
        err.to_string().contains("[ERRTX-ROT002]"),
        "expected [ERRTX-ROT002] below the shipped activation height, got: {err}"
    );

    // The well-formed rotation is rejected below the gate too — the gate is about the
    // height, not about the quality of the transaction.
    let (good, _) = well_formed();
    let good_err = rotate_stateless(&good, &ctx)
        .expect_err("a well-formed rotation is still invalid below the gate");
    assert!(good_err.to_string().contains("[ERRTX-ROT002]"));
}

// ===========================================================================
// I1-I2 — input count (ROT001)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether a rotation with no input can reach the producer
// binding check at all; with no input there is no authorising key, so "the fee payer IS
// the producer" would degrade into "anybody may rotate anybody's key".
#[test]
fn req_rot_sec_007_zero_inputs_is_rejected() {
    let tx = mutate_tx(|tx| tx.inputs.clear());
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT001]",
        "a rotation must carry exactly 1 input",
    );
}

// REQ-ROT-SEC-007 — Decision: whether a second input can smuggle a different authorising
// key past a validator that only inspects `inputs[0]`, which would let an attacker pair
// their own fee input with a victim's revealed key.
#[test]
fn req_rot_sec_007_two_inputs_is_rejected() {
    let tx = mutate_tx(|tx| {
        let extra = Input::new(outpoint_b().0, outpoint_b().1);
        tx.inputs.push(extra);
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT001]",
        "a rotation must carry exactly 1 input",
    );
}

// ===========================================================================
// I3-I5 — output shape (ROT009)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether a rotation may burn its whole input as fee. An
// output-less shape is a different transaction from the one the CLI builds and the one
// the fee/economics checks assume.
#[test]
fn req_rot_sec_007_zero_outputs_is_rejected() {
    let tx = mutate_tx(|tx| tx.outputs.clear());
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT009]",
        "a rotation must carry exactly 1 Normal output",
    );
}

// REQ-ROT-SEC-007 — Decision: whether a rotation can carry extra outputs, which would let
// it double as a value-moving transaction whose shape no economics check models.
#[test]
fn req_rot_sec_007_two_outputs_is_rejected() {
    let tx = mutate_tx(|tx| {
        let extra = Output::normal(1, crypto::hash::hash(b"inc-i-217-m5-extra"));
        tx.outputs.push(extra);
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT009]",
        "a rotation must carry exactly 1 Normal output",
    );
}

// REQ-ROT-SEC-007 — Decision: whether a rotation can MINT a Bond output. Bonds are the
// stake-weight primitive; a transaction type whose output type is unchecked is a free
// bond-creation path that never went through Register or AddBond.
#[test]
fn req_rot_sec_007_a_non_normal_output_is_rejected() {
    let tx = mutate_tx(|tx| tx.outputs[0].output_type = OutputType::Bond);
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT009]",
        "output 0 of a rotation must be OutputType::Normal",
    );
}

// ===========================================================================
// I6-I8 — payload length (ROT010)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the decoder is length-tolerant at the validation
// boundary. A short payload that still decodes would let a field be read from bytes the
// authorisation signature never covered.
#[test]
fn req_rot_sec_007_a_239_byte_payload_is_rejected() {
    let tx = mutate_tx(|tx| {
        tx.extra_data.truncate(239);
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT010]",
        "extra_data must be exactly 240 bytes",
    );
}

// REQ-ROT-SEC-007 — Decision: whether trailing bytes are ignored. A tolerated suffix is a
// malleability channel: two distinct transactions carrying the same authorised rotation
// would both be valid and would hash differently.
#[test]
fn req_rot_sec_007_a_241_byte_payload_is_rejected() {
    let tx = mutate_tx(|tx| tx.extra_data.push(0x00));
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT010]",
        "extra_data must be exactly 240 bytes",
    );
}

// REQ-ROT-SEC-007 — Decision: whether an empty payload takes a path that skips the rules
// entirely — the classic "no data, nothing to check, accept" shape.
#[test]
fn req_rot_sec_007_an_empty_payload_is_rejected() {
    let tx = mutate_tx(|tx| tx.extra_data.clear());
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT010]",
        "extra_data must be exactly 240 bytes",
    );
}

// ===========================================================================
// I9-I11 — the new BLS key must be a real G1 point (ROT004)
// ===========================================================================
//
// These three mutants re-sign the authorisation over the mutated key, so the Ed25519 rule
// is satisfied and only the point rule is broken by intent. A valid PoP for an invalid
// key cannot exist, so the PoP rule is necessarily broken too — ROT004 must win by the
// documented check order, which is exactly what the code assertion proves.

fn tx_with_new_key(new_bls_pubkey: [u8; 48]) -> Transaction {
    let outpoint = outpoint_a();
    let mut payload = well_formed_payload(outpoint);
    payload.new_bls_pubkey = new_bls_pubkey;
    payload.signature = auth_signature(&producer_kp(), &new_bls_pubkey, outpoint);
    tx_carrying(&payload, outpoint)
}

// REQ-ROT-SEC-007 — Decision: whether an arbitrary 48-byte string can be installed as an
// attestation key. A non-point key is unusable for aggregation, so the producer would
// silently drop out of every future presence bitfield — an un-recoverable self-slash the
// operator cannot undo without another rotation.
#[test]
fn req_rot_sec_007_an_all_ff_new_key_is_rejected() {
    let tx = tx_with_new_key([0xFF; 48]);
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT004]",
        "0xFF*48 is not a valid compressed G1 point",
    );
}

// REQ-ROT-SEC-007 — Decision: whether the all-zero key is accepted. `BlsPublicKeyWrapped`
// has an explicit `ZERO` constant and an `is_zero()` predicate, so zero is a value the
// codebase can produce by accident (an uninitialised field), not only by attack.
#[test]
fn req_rot_sec_007_an_all_zero_new_key_is_rejected() {
    let tx = tx_with_new_key([0x00; 48]);
    assert_rejected_with(&tx, "[ERRTX-ROT004]", "the all-zero key must be rejected");
}

// REQ-ROT-SEC-007 — Decision: whether the compressed G1 IDENTITY is accepted. The identity
// is the one "valid-looking" encoding for which a rogue-key attacker can produce a
// verifying PoP, so a point check that only tests on-curve-ness and forgets infinity
// admits a key that contributes nothing to any aggregate.
#[test]
fn req_rot_sec_007_the_g1_identity_encoding_is_rejected() {
    let mut identity = [0u8; 48];
    identity[0] = 0xc0;
    let tx = tx_with_new_key(identity);
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT004]",
        "the compressed G1 identity must be rejected",
    );
}

// ===========================================================================
// I12-I13 — the fee payer IS the producer (ROT008)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether anybody can rotate ANOTHER producer's key by paying
// the fee themselves. Without this binding the authorisation signature is the only
// defence, and a captured signature becomes a transferable capability.
#[test]
fn req_rot_sec_007_a_producer_field_that_differs_from_the_input_key_is_rejected() {
    let tx = mutate_payload(|p| p.producer = *other_kp().public_key().as_bytes());
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT008]",
        "payload.producer must equal inputs[0].public_key",
    );
}

// REQ-ROT-SEC-007 — Decision: whether a rotation that reveals NO authorising key is
// treated as "nothing to compare, therefore fine". `Input::public_key` is an `Option`
// whose `None` arm exists for pre-fork chain data, so the permissive reading is the
// natural mistake.
#[test]
fn req_rot_sec_007_an_input_without_a_public_key_is_rejected() {
    let tx = mutate_tx(|tx| tx.inputs[0].public_key = None);
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT008]",
        "a rotation must carry its authorising key on the input",
    );
}

// ===========================================================================
// I14-I17 — the Ed25519 authorisation and its outpoint bind (ROT003)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the authorisation signature is verified at all. A
// zero signature is what an unsigned, CLI-default-constructed payload carries, so this is
// the difference between "checked" and "field present".
#[test]
fn req_rot_sec_007_a_zero_authorisation_signature_is_rejected() {
    let tx = mutate_payload(|p| p.signature = [0u8; 64]);
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT003]",
        "the authorisation signature must verify",
    );
}

// REQ-ROT-SEC-007 — Decision: whether the signature is verified against the PRODUCER's key
// or against some other key in scope. A verifier that checks the signature against the
// wrong public key accepts any rotation its holder signs.
#[test]
fn req_rot_sec_007_an_authorisation_by_a_different_key_is_rejected() {
    let tx = mutate_payload(|p| {
        p.signature = auth_signature(&other_kp(), &p.new_bls_pubkey, outpoint_a());
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT003]",
        "only the producer's own key may authorise its rotation",
    );
}

// REQ-ROT-SEC-003 — Decision: whether the outpoint is really inside the signed digest. If
// it is not, one authorisation signature is valid for unlimited transactions, and an
// observer who sees a rotation in the mempool can re-submit it spending a different
// output — the rotation becomes replayable for as long as the producer holds any UTXO.
#[test]
fn req_rot_sec_003_an_authorisation_signed_over_another_outpoint_is_rejected() {
    let tx = mutate_payload(|p| {
        p.signature = auth_signature(&producer_kp(), &p.new_bls_pubkey, outpoint_b());
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT003]",
        "the digest binds the outpoint the transaction spends (REQ-ROT-SEC-003)",
    );
}

// REQ-ROT-SEC-003 — Decision: whether a COMPLETE, genuinely authorised rotation tuple can
// be lifted off its transaction and re-mounted on another one. This is the live replay:
// nothing is forged, every field is the producer's own, only the spent outpoint differs.
#[test]
fn req_rot_sec_003_a_valid_tuple_lifted_onto_another_outpoint_is_rejected() {
    let lifted = well_formed_payload(outpoint_a());
    let tx = tx_carrying(&lifted, outpoint_b());

    // Non-vacuity: the very same tuple on its OWN outpoint validates, so the rejection
    // below is the outpoint bind and not a defect in the tuple.
    let original = tx_carrying(&lifted, outpoint_a());
    assert!(
        rotate_stateless(&original, &ctx_above_gate()).is_ok(),
        "the lifted tuple must be genuinely valid on its own outpoint"
    );

    assert_rejected_with(
        &tx,
        "[ERRTX-ROT003]",
        "a captured rotation must not be replayable onto another outpoint",
    );
}

// ===========================================================================
// I18-I21 — the rotation proof of possession (ROT007)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the rotation PoP falls back to the REGISTRATION DST.
// A registration PoP is a public artefact of every producer's on-chain registration, so a
// fallback would let an attacker install a key whose PoP they merely copied from the
// chain instead of one they had to possess the secret for.
#[test]
fn req_rot_sec_007_a_registration_dst_pop_is_rejected() {
    let tx = mutate_payload(|p| {
        let bls = new_bls();
        p.bls_pop = *crypto::bls_sign_pop(bls.secret_key(), bls.public_key())
            .expect("registration PoP signing cannot fail")
            .as_bytes();
    });
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT007]",
        "a registration-DST PoP must not satisfy the rotation PoP",
    );
}

// REQ-ROT-SEC-007 — Decision: whether the PoP binds the PRODUCER. If it does not, a PoP
// produced for producer X is reusable to install the same BLS key under producer Y, which
// is the rogue-key setup for aggregate attestation forgery.
#[test]
fn req_rot_sec_007_a_pop_bound_to_another_producer_is_rejected() {
    let tx = mutate_payload(|p| p.bls_pop = rotation_pop(&new_bls(), other_kp().public_key()));
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT007]",
        "the rotation PoP binds the producer's Ed25519 key",
    );
}

// REQ-ROT-SEC-007 — Decision: whether the PoP binds the KEY BEING INSTALLED. A PoP for key
// K1 accepted while installing K2 is exactly the rogue-key attack: the attacker installs a
// key they do not possess and then subtracts their share from any aggregate.
#[test]
fn req_rot_sec_007_a_pop_bound_to_another_bls_key_is_rejected() {
    let tx = mutate_payload(|p| p.bls_pop = rotation_pop(&other_bls(), producer_kp().public_key()));
    assert_rejected_with(
        &tx,
        "[ERRTX-ROT007]",
        "the rotation PoP must cover the key being installed",
    );
}

// REQ-ROT-SEC-007 — Decision: whether the PoP is verified at all, or merely present. 96
// zero bytes is what an unsigned payload carries, and `BlsSignature::ZERO` is a
// constructible value in this codebase.
#[test]
fn req_rot_sec_007_a_zero_pop_is_rejected() {
    let tx = mutate_payload(|p| p.bls_pop = [0u8; 96]);
    assert_rejected_with(&tx, "[ERRTX-ROT007]", "the rotation PoP must verify");
}

// ===========================================================================
// F2 — the same verdicts through the public dispatch (REQ-ROT-SEC-007)
// ===========================================================================

// REQ-ROT-SEC-007 — Decision: whether the dispatch arm swallows, re-wraps or downgrades the
// error `rotate_stateless` produced. Operators and the M6 mempool read the ERRTX code; a
// generic `InvalidTransaction` on this path makes every rotation failure look alike.
#[test]
fn req_rot_sec_007_validate_transaction_propagates_the_specific_rotation_error() {
    // Two inputs, NOT zero: `validate_transaction` has a generic pre-match guard that
    // rejects an input-less transaction with `[ERRTX001] transaction must have inputs`
    // before the `match tx.tx_type` dispatch, so a zero-input rotation never reaches the
    // arm this test is about. Measured on the shipped validator by the M5 outcome probe.
    // The zero-input case still asserts ROT001 against `rotate_stateless` directly, above.
    let shape = mutate_tx(|tx| tx.inputs.push(tx.inputs[0].clone()));
    let replay = mutate_payload(|p| {
        p.signature = auth_signature(&producer_kp(), &p.new_bls_pubkey, outpoint_b());
    });
    let pop = mutate_payload(|p| p.bls_pop = [0u8; 96]);

    for (tx, code) in [
        (&shape, "[ERRTX-ROT001]"),
        (&replay, "[ERRTX-ROT003]"),
        (&pop, "[ERRTX-ROT007]"),
    ] {
        let err = validate_transaction(tx, &ctx_above_gate())
            .expect_err("validate_transaction must reject this rotation");
        assert!(
            err.to_string().contains(code),
            "the dispatch arm must propagate {code} unchanged, got: {err}"
        );
    }
}
