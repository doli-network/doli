//! INC-I-217 M10 — `doli producer rotate-bls`: the PURE half.
//!
//! TDD RED, EXPECTED: `doli_cli::rotate_tx` does not exist at HEAD, so this
//! module does not compile. That compile failure IS the red.
//!
//! Authority: `specs/bls-key-rotation-architecture.md` Module 7 (CLI) — which
//! SUPERSEDES the requirements doc: 1-in / 1-out fee-paying tx, the SPENT
//! outpoint inside the Ed25519 preimage, no expiry.
//!
//! WHY A PURE MODULE: `doli-cli` is a bin crate with a small lib facade
//! (INC-I-180 M3). Everything asserted below is reachable offline with fixed
//! keys because it lives in `bins/cli/src/rotate_tx.rs`, exported from
//! `bins/cli/src/lib.rs`. The RPC/wallet/print half
//! (`bins/cli/src/cmd_producer/rotate.rs`) is NOT tested here.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1 `rotation_auth_message(&[u8;32], &[u8;48], &SpendableUtxo) -> [u8;32]`
//!      O1 return: the 32 digest bytes — the ONLY output.
//!      O2 mutable params / O3 receiver / O4 store / O5 statics: NONE (all `&`).
//!      PATHS: one, unconditional (infallible).
//!   F2 `check_rotation_preconditions(&OnChainProducer, Option<[u8;48]>)
//!       -> Result<RotationTarget, RotateTxError>`
//!      O6 return: `Ok(target)` | `Err(variant)`; no other output.
//!      PATHS: P-OK | P-NO-WALLET-KEY | P-UNREGISTERED | P-PENDING | P-SAME.
//!   F3 `rotate_bls_fee() -> u64`
//!      O7 return: the fee the 240-byte payload costs. PATHS: one.
//!   F4 `select_fee_utxo(&[SpendableUtxo], u64) -> Result<SpendableUtxo, _>`
//!      O8 return: the chosen UTXO | `Err(NoUtxoCoversFee)`.
//!      PATHS: P-CHOSEN | P-NONE-COVERS (empty set, or every amount <= fee).
//!   F5 `build_rotate_bls_tx(&[u8;32], &str, &str, &SpendableUtxo, u64)
//!       -> Result<RotateTxPlan, RotateTxError>`
//!      O9  `plan.tx` — inputs, outputs, tx_type, extra_data, signatures.
//!      O10 `plan.payload` — the decoded 240 bytes.
//!      O11 `plan.auth_digest` — what the producer key actually signed.
//!      O12 `plan.fee` / `plan.change_value`.
//!      O13 error Display text — MUST NOT echo either secret.
//!      PATHS: P-BUILT | P-BAD-ED25519-SECRET | P-BAD-BLS-SECRET.
//!   F6 `rotation_report_lines(&RotateTxPlan, Option<[u8;48]>, Option<u64>)
//!       -> Vec<String>`
//!      O14 the returned lines: old key, new key, fee, effective height.
//!      O15 secret leakage across ALL lines — MUST BE ABSENT on every path.
//!      PATHS: P-WITH-HEIGHT | P-WITHOUT-HEIGHT.
//!   F7 `rotation_consent(bool, bool) -> Result<ConsentOutcome, RotateTxError>`
//!      O16 return: `Accepted` | `MustPrompt` | `Err(ConsentRequired)`.
//!      PATHS: P-YES | P-TTY-NO-YES | P-NO-TTY-NO-YES.
//!   F8 `rotation_answer_accepted(&str) -> bool`
//!      O17 return: the y/yes verdict. PATHS: accept | refuse.
//!   F9 `ROTATE_BLS_NOTICE: &str`
//!      O18 the disclosure text an operator reads before consenting.
//!
//!   INPUT PARTITIONS:
//!     I1  the M4 golden triple (genesis 0x11*32, key 0x22*48, outpoint 0x33*32/7)
//!     I2  a real Ed25519 producer key + a real BLS keypair, mainnet genesis
//!     I3  registered, different key, no pending      (the positive fixture)
//!     I4..I7 exactly ONE mutation of I3 each: no wallet key / unregistered /
//!            pending rotation / wallet key == on-chain key
//!     I8  UTXO sets: empty; all amounts <= fee; amount == fee exactly;
//!         several that cover, in two permutations
//!     I9  a 31-byte Ed25519 secret hex; I10 a 32-byte BLS hex that is not a
//!         valid scalar (0xFF*32 >= group order)
//!     I11 consent (yes, tty) in all four combinations
//!     I12 answers: "y" "Y" "yes" "YES" " yes " "" "n" "no" "yolo"
//!
//!   MATRIX: O1 x I1; O6 x I3..I7; O7 once; O8 x I8; O9..O12 x I2;
//!     O13 x I9,I10; O14/O15 x I2 (both height paths); O16 x I11; O17 x I12;
//!     O18 once. `plan.tx` is additionally run through the node's own
//!     `rotate_stateless` at and below the gate, so the CLI and consensus
//!     cannot drift apart silently.
//!
//! NO MOCKED CRYPTO: real Ed25519, a real BLS keypair, a real rotation PoP.
//!
//! REQUIRED DERIVES (the assertions below will not compile without them):
//!   `RotateTxError`: Debug + Display; `RotateTxPlan`: Debug;
//!   `SpendableUtxo`: Clone; `ConsentOutcome`: Debug.

use crypto::{BlsKeyPair, BlsSecretKey, KeyPair, PrivateKey, PublicKey, Signature};
use doli_cli::rotate_tx::{
    build_rotate_bls_tx, check_rotation_preconditions, rotate_bls_fee, rotation_answer_accepted,
    rotation_auth_message, rotation_consent, rotation_report_lines, select_fee_utxo,
    ConsentOutcome, OnChainProducer, RotateTxError, RotateTxPlan, SpendableUtxo, ROTATE_BLS_NOTICE,
};
use doli_core::consensus::{ConsensusParams, BASE_FEE, FEE_DIVISOR, FEE_PER_BYTE};
use doli_core::genesis::genesis_hash;
use doli_core::transaction::{OutputType, RotateBlsData, TxType, ROTATE_BLS_DATA_LEN};
use doli_core::validation::{rotate_stateless, ValidationContext, ValidationError};
use doli_core::Network;

// ---------------------------------------------------------------------------
// The M4 golden vector — copied verbatim from
// crates/core/tests/it/inc_i_217_m4_rotate_payload.rs (a test binary cannot be
// imported across crates). Hand-written from the spec, never captured from a run.
// ---------------------------------------------------------------------------

const GOLDEN_GENESIS: [u8; 32] = [0x11; 32];
const GOLDEN_NEW_KEY: [u8; 48] = [0x22; 48];
const GOLDEN_PREV_TX: [u8; 32] = [0x33; 32];
const GOLDEN_OUTPUT_INDEX: u32 = 7;
const GOLDEN_DIGEST_HEX: &str = "18ab518033c6720442af298538f831e3d58536f6d87df9aac3dc1872a477196b";

const NETWORK: Network = Network::Mainnet;
const GATE: u64 = 500;
const ABOVE_GATE: u64 = 1_000;

/// Fixed Ed25519 producer secret. Any 32 bytes are a valid seed.
const PRODUCER_SECRET_HEX: &str =
    "6161616161616161616161616161616161616161616161616161616161616161";

/// 31 bytes: valid hex, wrong length.
const SHORT_ED25519_SECRET_HEX: &str =
    "ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab00ab";

/// 32 bytes of 0xFF: valid hex, valid length, NOT a valid BLS scalar (>= r).
const BAD_BLS_SECRET_HEX: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

const UTXO_AMOUNT: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

fn producer_kp() -> KeyPair {
    KeyPair::from_private_key(
        PrivateKey::from_hex(PRODUCER_SECRET_HEX).expect("fixed producer secret is valid hex"),
    )
}

fn producer_pk_bytes() -> [u8; 32] {
    *producer_kp().public_key().as_bytes()
}

fn new_bls_kp() -> BlsKeyPair {
    BlsKeyPair::from_seed(&[0x63; 32]).expect("32-byte BLS seed is valid")
}

fn new_bls_secret_hex() -> String {
    hex::encode(new_bls_kp().secret_key().as_bytes())
}

fn new_bls_pubkey() -> [u8; 48] {
    *new_bls_kp().public_key().as_bytes()
}

/// A distinct on-chain key so "wallet key != on-chain key" is the normal case.
fn old_bls_pubkey() -> [u8; 48] {
    *BlsKeyPair::from_seed(&[0x64; 32])
        .expect("32-byte BLS seed is valid")
        .public_key()
        .as_bytes()
}

fn mainnet_genesis() -> [u8; 32] {
    *genesis_hash(NETWORK).as_bytes()
}

fn fee_utxo() -> SpendableUtxo {
    SpendableUtxo {
        tx_hash: [0x33; 32],
        output_index: 7,
        amount: UTXO_AMOUNT,
    }
}

fn utxo(seed: u8, index: u32, amount: u64) -> SpendableUtxo {
    SpendableUtxo {
        tx_hash: [seed; 32],
        output_index: index,
        amount,
    }
}

/// The positive precondition fixture: registered, a different on-chain key, no
/// pending rotation. Every negative below is exactly ONE mutation of this.
fn registered_with_other_key() -> OnChainProducer {
    OnChainProducer {
        registered: true,
        bls_pubkey: Some(old_bls_pubkey()),
        pending_new_bls_pubkey: None,
    }
}

fn built_plan() -> RotateTxPlan {
    build_rotate_bls_tx(
        &mainnet_genesis(),
        PRODUCER_SECRET_HEX,
        &new_bls_secret_hex(),
        &fee_utxo(),
        rotate_bls_fee(),
    )
    .expect("the fixed fixture builds a rotation")
}

fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), NETWORK, 0, height)
        .with_bls_key_rotation_activation_height(GATE)
}

// ---------------------------------------------------------------------------
// F1 — golden parity. The anti-drift tripwire.
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure here says the CLI computes the producer
// authorisation over different bytes than the node verifies, so every rotation
// the operator signs would be rejected as [ERRTX-ROT003] — or worse, would be
// valid over an outpoint the operator never chose.
#[test]
fn auth_message_reproduces_the_m4_golden_digest() {
    let outpoint = SpendableUtxo {
        tx_hash: GOLDEN_PREV_TX,
        output_index: GOLDEN_OUTPUT_INDEX,
        amount: UTXO_AMOUNT,
    };

    let digest = rotation_auth_message(&GOLDEN_GENESIS, &GOLDEN_NEW_KEY, &outpoint);

    assert_eq!(
        hex::encode(digest),
        GOLDEN_DIGEST_HEX,
        "the CLI digest must equal the M4 golden vector byte for byte"
    );
}

// REQ-ROT-013 — Decision: a failure here means the digest is not bound to the
// SPENT outpoint, which is the entire replay defence (REQ-ROT-SEC-003): the
// same signature would then authorise a rotation the operator never funded.
#[test]
fn auth_message_is_bound_to_the_outpoint_it_spends() {
    let base = rotation_auth_message(&GOLDEN_GENESIS, &GOLDEN_NEW_KEY, &fee_utxo());
    let other_index = rotation_auth_message(
        &GOLDEN_GENESIS,
        &GOLDEN_NEW_KEY,
        &utxo(0x33, GOLDEN_OUTPUT_INDEX + 1, UTXO_AMOUNT),
    );
    let other_hash = rotation_auth_message(
        &GOLDEN_GENESIS,
        &GOLDEN_NEW_KEY,
        &utxo(0x34, GOLDEN_OUTPUT_INDEX, UTXO_AMOUNT),
    );
    let other_genesis = rotation_auth_message(&[0x12; 32], &GOLDEN_NEW_KEY, &fee_utxo());
    let other_key = rotation_auth_message(&GOLDEN_GENESIS, &[0x23; 48], &fee_utxo());

    assert_ne!(base, other_index, "the output index must enter the digest");
    assert_ne!(base, other_hash, "the prev tx hash must enter the digest");
    assert_ne!(
        base, other_genesis,
        "the genesis hash must enter the digest"
    );
    assert_ne!(base, other_key, "the new BLS key must enter the digest");
    // The amount is NOT part of the preimage: same outpoint, same digest.
    assert_eq!(
        base,
        rotation_auth_message(
            &GOLDEN_GENESIS,
            &GOLDEN_NEW_KEY,
            &utxo(0x33, 7, UTXO_AMOUNT * 3)
        ),
        "the UTXO amount must not change the digest"
    );
}

// ---------------------------------------------------------------------------
// F5 — the built transaction
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure here is the CLI re-implementing the 240-byte
// layout instead of calling the core codec, which is the exact drift that would
// let a signed rotation decode to a key the operator never chose.
#[test]
fn payload_is_exactly_240_bytes_and_round_trips_through_the_core_codec() {
    let plan = built_plan();

    assert_eq!(
        plan.tx.extra_data.len(),
        ROTATE_BLS_DATA_LEN,
        "extra_data must be exactly the canonical 240 bytes"
    );

    let decoded = RotateBlsData::decode(&plan.tx.extra_data)
        .expect("the CLI payload must decode with the node's own codec");
    assert_eq!(
        decoded, plan.payload,
        "the reported payload must be the bytes actually carried"
    );
    assert_eq!(decoded.producer, producer_pk_bytes());
    assert_eq!(decoded.new_bls_pubkey, new_bls_pubkey());
    assert_eq!(
        decoded.encode().to_vec(),
        plan.tx.extra_data,
        "encode(decode(bytes)) must be the identity"
    );
}

// REQ-ROT-013 — Decision: a failure here means the producer signature does not
// verify over the digest the CLI claims it signed — the operator would be
// broadcasting an authorisation nobody can check against the key they hold.
#[test]
fn producer_signature_verifies_against_the_digest_the_cli_reports() {
    let plan = built_plan();
    let expected = rotation_auth_message(&mainnet_genesis(), &new_bls_pubkey(), &fee_utxo());

    assert_eq!(
        plan.auth_digest, expected,
        "the reported digest must be the one derived from the spent outpoint"
    );

    let signature = Signature::from_bytes(plan.payload.signature);
    let producer = PublicKey::from_bytes(plan.payload.producer);
    crypto::signature::verify(&plan.auth_digest, &signature, &producer)
        .expect("the authorisation must verify under the producer key it names");
}

// REQ-ROT-013 — Decision: a failure here is a shape the node refuses outright
// ([ERRTX-ROT001] / [ERRTX-ROT009] / [ERRTX-ROT008]) — a CLI that builds it has
// shipped a subcommand that can never produce an acceptable transaction.
#[test]
fn transaction_is_one_input_one_normal_output_and_reveals_the_producer_key() {
    let plan = built_plan();

    assert_eq!(plan.tx.tx_type, TxType::RotateBlsKey);
    assert_eq!(plan.tx.inputs.len(), 1, "exactly one input");
    assert_eq!(plan.tx.outputs.len(), 1, "exactly one output");
    assert_eq!(plan.tx.outputs[0].output_type, OutputType::Normal);

    let input = &plan.tx.inputs[0];
    assert_eq!(*input.prev_tx_hash.as_bytes(), fee_utxo().tx_hash);
    assert_eq!(input.output_index, fee_utxo().output_index);
    assert_eq!(
        input.public_key.map(|key| *key.as_bytes()),
        Some(producer_pk_bytes()),
        "the input must REVEAL the producer key the payload names"
    );

    assert_eq!(plan.fee, rotate_bls_fee());
    assert_eq!(
        plan.change_value,
        UTXO_AMOUNT - rotate_bls_fee(),
        "the single output carries the input minus the fee"
    );
    assert_eq!(plan.tx.outputs[0].amount, plan.change_value);
}

// REQ-ROT-013 — Decision: a failure here means the input is signed over the
// wrong message, so the tx is rejected on the generic input-signature rule and
// the operator sees a failure that names nothing about rotation.
#[test]
fn input_signature_verifies_under_signing_message_for_input_zero() {
    let plan = built_plan();
    let message = plan.tx.signing_message_for_input(0);
    let producer = producer_kp();

    crypto::signature::verify_hash(
        &message,
        &plan.tx.inputs[0].signature,
        producer.public_key(),
    )
    .expect("input 0 must be signed over its own signing message");
}

// REQ-ROT-013 — Decision: a failure here means the CLI and the node disagree
// about what a valid rotation is; the below-gate half proves the harness is not
// simply accepting everything handed to it.
#[test]
fn built_transaction_passes_rotate_stateless_above_the_gate_and_is_gated_below_it() {
    let plan = built_plan();

    let payload = rotate_stateless(&plan.tx, &ctx_at(ABOVE_GATE))
        .expect("the CLI-built transaction must satisfy every stateless rule");
    assert_eq!(payload.new_bls_pubkey, new_bls_pubkey());

    let err = rotate_stateless(&plan.tx, &ctx_at(GATE - 1))
        .expect_err("the same transaction must be refused below the activation height");
    assert!(
        matches!(err, ValidationError::RotateBlsNotActivated { .. }),
        "below the gate the refusal must be the gate itself, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// F2 — preconditions
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure of the positive case means the refusals
// below are vacuous; nothing would ever be allowed to rotate.
#[test]
fn accepts_a_registered_producer_whose_wallet_key_differs_and_has_no_pending_rotation() {
    let target = check_rotation_preconditions(&registered_with_other_key(), Some(new_bls_pubkey()))
        .expect("a different key on a registered producer is exactly the case to allow");

    assert_eq!(target.new_bls_pubkey, new_bls_pubkey());
    assert_eq!(target.old_bls_pubkey, Some(old_bls_pubkey()));
}

// REQ-ROT-013 — Decision: a failure here means the CLI spends a fee to install
// the key that is already installed — a paid no-op the operator cannot undo.
#[test]
fn refuses_when_the_wallet_key_already_equals_the_onchain_key() {
    let mut onchain = registered_with_other_key();
    onchain.bls_pubkey = Some(new_bls_pubkey());

    let err = check_rotation_preconditions(&onchain, Some(new_bls_pubkey()))
        .expect_err("rotating to the key already on chain must be refused");
    assert!(
        matches!(err, RotateTxError::SameKey),
        "expected SameKey, got: {err:?}"
    );
}

// REQ-ROT-013 — Decision: a failure here lets an operator queue a second
// rotation over a pending one; the node's one-pending rule then rejects it after
// the fee is spent, or worse the operator loses track of which key is live.
#[test]
fn refuses_when_a_rotation_is_already_pending() {
    let mut onchain = registered_with_other_key();
    onchain.pending_new_bls_pubkey = Some(new_bls_pubkey());

    let err = check_rotation_preconditions(&onchain, Some(new_bls_pubkey()))
        .expect_err("a pending rotation must block a second one");
    assert!(
        matches!(err, RotateTxError::RotationAlreadyPending),
        "expected RotationAlreadyPending, got: {err:?}"
    );
}

// REQ-ROT-013 — Decision: a failure here means the CLI would sign a proof of
// possession with no key, or install a key the operator does not hold the
// secret for — an instant, unrecoverable loss of attestation.
#[test]
fn refuses_when_the_wallet_has_no_bls_key() {
    let err = check_rotation_preconditions(&registered_with_other_key(), None)
        .expect_err("no wallet BLS key means there is nothing to rotate to");
    assert!(
        matches!(err, RotateTxError::NoWalletBlsKey),
        "expected NoWalletBlsKey, got: {err:?}"
    );
}

// REQ-ROT-013 — Decision: a failure here spends a fee on a transaction the node
// rejects at the stateful stage, and tells a confused operator nothing about
// the real problem (they never registered).
#[test]
fn refuses_when_the_producer_is_not_registered() {
    let mut onchain = registered_with_other_key();
    onchain.registered = false;

    let err = check_rotation_preconditions(&onchain, Some(new_bls_pubkey()))
        .expect_err("an unregistered key has no rotation to make");
    assert!(
        matches!(err, RotateTxError::ProducerNotRegistered),
        "expected ProducerNotRegistered, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// F3 / F4 — fee and UTXO selection
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure here means the 240-byte payload is not paid
// for, so the node rejects the transaction on the fee rule after the operator
// has already consented.
#[test]
fn fee_pays_for_the_240_byte_payload() {
    let expected = BASE_FEE + (ROTATE_BLS_DATA_LEN as u64) * FEE_PER_BYTE / FEE_DIVISOR;

    assert_eq!(rotate_bls_fee(), expected);
    assert!(
        rotate_bls_fee() > BASE_FEE,
        "the per-byte term must actually cost something, else the formula is dead"
    );
}

// REQ-ROT-013 — Decision: a failure here makes the transaction the CLI builds
// depend on the order the node happened to return UTXOs — two runs of the same
// command would sign different bytes, which no operator can audit.
#[test]
fn selects_the_smallest_utxo_that_strictly_covers_the_fee_whatever_the_input_order() {
    let fee = rotate_bls_fee();
    let small = utxo(0x01, 0, fee); // exactly the fee: leaves zero change
    let chosen = utxo(0x02, 1, fee + 1); // the smallest that strictly covers
    let large = utxo(0x03, 2, fee + 1_000);

    let forward = select_fee_utxo(&[small.clone(), chosen.clone(), large.clone()], fee)
        .expect("one UTXO strictly covers the fee");
    let reversed = select_fee_utxo(&[large, chosen.clone(), small], fee)
        .expect("input order must not change the choice");

    assert_eq!(forward.tx_hash, chosen.tx_hash);
    assert_eq!(forward.output_index, chosen.output_index);
    assert_eq!(forward.amount, chosen.amount);
    assert_eq!(reversed.tx_hash, forward.tx_hash);
    assert_eq!(reversed.output_index, forward.output_index);
}

// REQ-ROT-013 — Decision: a failure here builds a transaction with a zero or
// negative change output, which the node refuses — the operator is told
// "invalid transaction" instead of "you need more balance".
#[test]
fn refuses_when_no_single_utxo_strictly_covers_the_fee() {
    let fee = rotate_bls_fee();

    let empty = select_fee_utxo(&[], fee).expect_err("no UTXO at all must refuse");
    assert!(
        matches!(empty, RotateTxError::NoUtxoCoversFee { .. }),
        "expected NoUtxoCoversFee, got: {empty:?}"
    );

    let exact = select_fee_utxo(&[utxo(0x01, 0, fee), utxo(0x02, 1, fee - 1)], fee)
        .expect_err("a UTXO equal to the fee leaves no change and must refuse");
    assert!(
        matches!(exact, RotateTxError::NoUtxoCoversFee { .. }),
        "expected NoUtxoCoversFee, got: {exact:?}"
    );
}

// ---------------------------------------------------------------------------
// F7 / F8 / F9 — disclosure and consent
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure here means an operator consents without
// being told the rotation is irreversible or that the boundary block's own
// attestation can be lost — the two facts they cannot recover from.
#[test]
fn notice_states_the_irreversibility_and_the_boundary_effect() {
    let notice = ROTATE_BLS_NOTICE.to_lowercase();

    assert!(
        notice.contains("cannot be undone"),
        "the notice must say the rotation cannot be undone"
    );
    assert!(
        notice.contains("another rotation"),
        "the notice must say the only way back is another rotation"
    );
    assert!(
        notice.contains("attestation"),
        "the notice must warn about attestations"
    );
    assert!(
        notice.contains("boundary"),
        "the notice must say the change takes effect at an epoch boundary"
    );
    // The CLI cannot learn the activation height: no RPC exposes
    // bls_key_rotation_activation_height. Static text claiming one is a lie
    // that goes stale, exactly as the bond-lock notice refuses to name 418000.
    assert!(
        !notice
            .split(|c: char| !c.is_ascii_digit())
            .any(|run| run.len() >= 4),
        "the notice must not hard-code a block height it cannot know"
    );
}

// REQ-ROT-013 — Decision: a failure here is silent consent: a script that never
// passed --yes believes it rotated a key when it did not, or rotates one the
// operator never approved.
#[test]
fn consent_is_never_assumed_without_a_terminal_and_without_yes() {
    assert!(
        matches!(rotation_consent(true, false), Ok(ConsentOutcome::Accepted)),
        "--yes must proceed with no terminal attached"
    );
    assert!(
        matches!(rotation_consent(true, true), Ok(ConsentOutcome::Accepted)),
        "--yes must proceed with a terminal too"
    );
    assert!(
        matches!(
            rotation_consent(false, true),
            Ok(ConsentOutcome::MustPrompt)
        ),
        "an interactive run without --yes must be asked, not refused"
    );

    let err = rotation_consent(false, false)
        .expect_err("no terminal and no --yes must be an error, never silent consent");
    assert!(
        matches!(err, RotateTxError::ConsentRequired),
        "expected ConsentRequired, got: {err:?}"
    );
    assert!(
        err.to_string().contains("--yes"),
        "the refusal must tell the operator exactly how to proceed, got: {err}"
    );
}

// REQ-ROT-013 — Decision: a failure here treats a stray keystroke or an empty
// line as approval of an irreversible key change.
#[test]
fn only_an_affirmative_answer_is_consent() {
    for accepted in ["y", "Y", "yes", "YES", "Yes", " yes ", "y\n"] {
        assert!(
            rotation_answer_accepted(accepted),
            "{accepted:?} must be read as consent"
        );
    }
    for refused in ["", " ", "n", "N", "no", "NO", "yolo", "yep", "1", "\n"] {
        assert!(
            !rotation_answer_accepted(refused),
            "{refused:?} must NOT be read as consent"
        );
    }
}

// ---------------------------------------------------------------------------
// F6 — the report, and secret hygiene
// ---------------------------------------------------------------------------

// REQ-ROT-013 — Decision: a failure here means the operator cannot see which
// key is being replaced, which one replaces it, or when it takes effect — so
// they cannot detect that the CLI is rotating to the wrong key.
#[test]
fn report_shows_the_old_key_the_new_key_the_fee_and_the_effective_height() {
    let plan = built_plan();
    let lines = rotation_report_lines(&plan, Some(old_bls_pubkey()), Some(1234));
    let joined = lines.join("\n");

    assert!(
        joined.contains(&hex::encode(new_bls_pubkey())),
        "the new BLS public key must be shown in full: {joined}"
    );
    assert!(
        joined.contains(&hex::encode(old_bls_pubkey())),
        "the key being replaced must be shown in full: {joined}"
    );
    assert!(
        joined.contains(&rotate_bls_fee().to_string()),
        "the fee must be shown: {joined}"
    );
    assert!(
        joined.contains("1234"),
        "the effective height must be shown when it is known: {joined}"
    );

    // Unknown height: the report must not invent one.
    let without = rotation_report_lines(&plan, Some(old_bls_pubkey()), None).join("\n");
    assert!(
        !without.contains("1234"),
        "an unknown effective height must not be reported as a number: {without}"
    );
    assert!(
        without.contains(&hex::encode(new_bls_pubkey())),
        "the new key is shown whether or not the height is known"
    );
}

// REQ-ROT-013 — Decision: a failure here writes a BLS or spending secret to the
// operator's terminal and scrollback, where CI logs and screenshots keep it
// forever. `doli export` is the only deliberate secret path and it is not this.
#[test]
fn no_report_line_and_no_notice_contains_either_secret() {
    let plan = built_plan();
    let bls_secret = new_bls_secret_hex();

    let mut surfaces = rotation_report_lines(&plan, Some(old_bls_pubkey()), Some(1234));
    surfaces.extend(rotation_report_lines(&plan, None, None));
    surfaces.push(ROTATE_BLS_NOTICE.to_string());

    for line in &surfaces {
        let lower = line.to_lowercase();
        assert!(
            !lower.contains(&bls_secret.to_lowercase()),
            "a BLS secret reached operator-facing output: {line}"
        );
        assert!(
            !lower.contains(PRODUCER_SECRET_HEX),
            "an Ed25519 spending secret reached operator-facing output: {line}"
        );
    }

    // Non-vacuity: the secrets ARE in scope of this fixture, so an empty or
    // constant surface cannot pass by saying nothing.
    assert!(
        BlsSecretKey::from_hex(&bls_secret).is_ok(),
        "the fixture secret must be a real key, else this test asserts nothing"
    );
    assert!(
        surfaces
            .iter()
            .any(|line| line.contains(&hex::encode(new_bls_pubkey()))),
        "the surfaces under test must actually carry key material"
    );
}

// REQ-ROT-013 — Decision: a failure here echoes the secret the operator just
// mistyped into an error message, which is the single most likely way a BLS
// secret ends up in a bug report.
#[test]
fn a_rejected_secret_is_never_echoed_in_the_error() {
    let bad_bls = build_rotate_bls_tx(
        &mainnet_genesis(),
        PRODUCER_SECRET_HEX,
        BAD_BLS_SECRET_HEX,
        &fee_utxo(),
        rotate_bls_fee(),
    )
    .expect_err("0xFF*32 is not a valid BLS scalar");
    assert!(
        matches!(bad_bls, RotateTxError::InvalidBlsSecret),
        "expected InvalidBlsSecret, got: {bad_bls:?}"
    );
    let message = bad_bls.to_string().to_lowercase();
    assert!(
        !message.contains(BAD_BLS_SECRET_HEX),
        "the rejected BLS secret must not appear in the error: {message}"
    );

    let bad_ed = build_rotate_bls_tx(
        &mainnet_genesis(),
        SHORT_ED25519_SECRET_HEX,
        &new_bls_secret_hex(),
        &fee_utxo(),
        rotate_bls_fee(),
    )
    .expect_err("a 31-byte Ed25519 secret is not a key");
    assert!(
        matches!(bad_ed, RotateTxError::InvalidProducerSecret),
        "expected InvalidProducerSecret, got: {bad_ed:?}"
    );
    let message = bad_ed.to_string().to_lowercase();
    assert!(
        !message.contains(SHORT_ED25519_SECRET_HEX),
        "the rejected spending secret must not appear in the error: {message}"
    );
}
