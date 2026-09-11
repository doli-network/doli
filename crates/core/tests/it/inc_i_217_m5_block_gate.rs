//! INC-I-217 M5 — the D6 block-level gate: a block carrying a `RotateBlsKey` transaction
//! below `bls_key_rotation_activation_height` is INVALID, in every `ValidationMode`.
//!
// covers: crates/core/src/validation/block.rs, crates/core/src/validation/rotate_bls.rs, crates/core/src/network_params/defaults.rs
//!
//! Requirements: **REQ-ROT-004** (Must — the activation gate) and **REQ-ROT-SEC-008**
//! (Must — an invalid rotation makes the CARRYING BLOCK invalid; there is no skip arm at
//! block level). Architecture decision D6.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `ValidationContext::with_bls_key_rotation_activation_height` and the
//! `[ERRTX-ROT002]` / `[ERRTX-ROT010]` variants do not exist yet. That compile failure is
//! the red, exactly as `inc_i_208_activation_height.rs` documents for itself.
//!
//! WHY THE GATE MUST HOLD IN EVERY MODE. `Light` and `Replay` exist to skip the expensive,
//! time-sensitive header and VDF work during sync and disaster recovery. If the rotation
//! gate lived inside a mode-specific branch, a node syncing a historical range would reach
//! a DIFFERENT verdict on the same block than a node validating it live. Consensus is the
//! agreement itself: an un-upgraded node — which cannot even decode a `RotateBlsKey`
//! transaction — and an upgraded node have to reject the same block, which is why the
//! check belongs ahead of the mode match, not inside it. That is D6.
//!
//! HOW THE BELOW-GATE HEIGHT IS DERIVED (INV-GOV-001). The shipped value is `u64::MAX` on
//! all three networks, so every real height is below it. The tests assert that RELATIONSHIP
//! against `NetworkParams::defaults`, never a literal — when the pinning decision-session
//! moves a network to a real height, these rows stay on the side of the gate they were
//! written to test, and the precondition assertion says so instead of passing silently.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: validate_block_with_mode(&Block, &ValidationContext, ValidationMode)
//         -> Result<(), ValidationError>
//       O1 return: `Ok(())` or `Err(e)`; `e.to_string()` MUST carry `[ERRTX-ROT002]` on
//          the below-gate path and MUST NOT carry it above the gate
//       O2 mutable params: NONE — block and context are both `&`
//       O3 receiver / O4 persistent store / O5 statics / O6 channels: NONE
//       PATHS:
//          P1 height < gate AND the block carries a RotateBlsKey -> Err(ROT002),
//             for EVERY ValidationMode (the check precedes the mode match)
//          P2 height < gate AND no RotateBlsKey                  -> unchanged verdict
//          P3 height >= gate AND a well-formed rotation          -> not ROT002
//          P4 height >= gate AND a malformed rotation            -> Err, no skip arm
//   F2: NetworkParams::defaults(Network).bls_key_rotation_activation_height
//       O1 return: the shipped gate, used ONLY as the source of the below-gate
//          relationship — never copied into an assertion as a literal.
//
//   INPUT PARTITIONS:
//     I1 the three shipped networks x their shipped gate value (P1)
//     I2 the three ValidationMode variants, enumerated exhaustively by a match that the
//        compiler breaks if a fourth variant is ever added
//     I3 a block whose ONLY defect is the rotation it carries (paired with I4)
//     I4 the byte-identical block with the rotation removed — the non-vacuity control:
//        it must be ACCEPTED below the gate, or "reject" would be about the block and
//        not about the rotation
//     I5 a well-formed rotation above an injected gate (P3)
//     I6 a malformed rotation above an injected gate (P4)
//
//   MATRIX 4 paths x 3 modes x 3 networks: every reachable cell has a named test.
//   NOT CLAIMED HERE: the per-rule verdicts themselves — those are
//   `inc_i_217_m5_rotate_stateless.rs`. This file claims only WHICH BLOCKS the node
//   accepts, and that the answer does not depend on the mode.

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::genesis_hash;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{
    rotation_auth_digest, Input, Output, RotateBlsData, Transaction, TxType,
};
use doli_core::validation::{validate_block_with_mode, ValidationContext, ValidationMode};
use doli_core::{Block, BlockHeader, Network};

/// Every `ValidationMode` the node can ask for. Kept honest by
/// [`mode_name`], which the compiler breaks if a fourth variant appears.
const ALL_MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];

/// Injected gate for the ABOVE-gate cases. The shipped value is `u64::MAX` everywhere, so
/// that side of the gate is unreachable from shipped params until the pinning session.
const INJECTED_AH: u64 = 500;

const BLOCK_SLOT: u32 = 9;
const BLOCK_HEIGHT: u64 = 1_000;

/// Exhaustiveness guard: adding a `ValidationMode` variant fails to compile here, so
/// [`ALL_MODES`] can never silently stop covering every mode.
fn mode_name(mode: ValidationMode) -> &'static str {
    match mode {
        ValidationMode::Full => "Full",
        ValidationMode::Light => "Light",
        ValidationMode::Replay => "Replay",
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn producer_kp() -> KeyPair {
    KeyPair::from_seed([0x31; 32])
}

fn payer_kp() -> KeyPair {
    KeyPair::from_seed([0x32; 32])
}

fn new_bls() -> BlsKeyPair {
    BlsKeyPair::from_seed(&[0x33; 32]).expect("32-byte BLS seed is valid")
}

fn outpoint() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m5-block-outpoint"), 1)
}

/// A rotation every stateless rule accepts, authorised by `producer_kp`.
fn well_formed_rotation() -> Transaction {
    let kp = producer_kp();
    let bls = new_bls();
    let new_bls_pubkey = *bls.public_key().as_bytes();
    let (prev_tx_hash, output_index) = outpoint();

    let digest = rotation_auth_digest(
        genesis_hash(Network::Mainnet).as_bytes(),
        &new_bls_pubkey,
        prev_tx_hash.as_bytes(),
        output_index,
    );
    let payload = RotateBlsData {
        producer: *kp.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: *crypto::sign_rotation_pop(
            bls.secret_key(),
            bls.public_key(),
            genesis_hash(Network::Mainnet).as_bytes(),
            kp.public_key().as_bytes(),
        )
        .expect("rotation PoP signing cannot fail")
        .as_bytes(),
        signature: *crypto::signature::sign(&digest, kp.private_key()).as_bytes(),
    };

    rotation_tx_with(payload.encode().to_vec())
}

/// A rotation whose payload is one byte short — malformed by every decoder.
fn malformed_rotation() -> Transaction {
    let mut tx = well_formed_rotation();
    tx.extra_data.truncate(239);
    tx
}

fn rotation_tx_with(extra_data: Vec<u8>) -> Transaction {
    let (prev_tx_hash, output_index) = outpoint();
    let mut input = Input::new(prev_tx_hash, output_index);
    input.public_key = Some(*producer_kp().public_key());
    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(
            1_000_000,
            crypto::hash::hash(b"inc-i-217-m5-block-change"),
        )],
        extra_data,
    }
}

/// An ordinary transfer — the non-vacuity control. A block holding only this must keep
/// whatever verdict it had before the gate existed.
fn plain_transfer() -> Transaction {
    let mut input = Input::new(crypto::hash::hash(b"inc-i-217-m5-transfer-outpoint"), 0);
    input.public_key = Some(*payer_kp().public_key());
    Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input],
        outputs: vec![Output::normal(
            500_000,
            crypto::hash::hash(b"inc-i-217-m5-transfer-recipient"),
        )],
        extra_data: Vec::new(),
    }
}

fn block_timestamp(params: &ConsensusParams) -> u64 {
    params.genesis_time + params.slot_duration * u64::from(BLOCK_SLOT)
}

/// A block that satisfies every header, merkle, size and slot rule, so the ONLY thing
/// that can reject it is a transaction rule. Without this the "not ROT002" assertions
/// below would be satisfied by a fixture that fails on its header.
fn block_with(txs: Vec<Transaction>, params: &ConsensusParams) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: params.genesis_hash,
        timestamp: block_timestamp(params),
        slot: BLOCK_SLOT,
        producer: *producer_kp().public_key(),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let mut block = Block::new(header, txs);
    block.header.merkle_root = block.compute_merkle_root();
    block
}

/// A context whose wall clock matches the block, so `validate_header` passes in `Full`
/// mode and the transaction loop is actually reached.
fn ctx_for(network: Network, gate: u64) -> ValidationContext {
    let params = ConsensusParams::for_network(network);
    let now = block_timestamp(&params);
    ValidationContext::new(params, network, now, BLOCK_HEIGHT)
        .with_bls_key_rotation_activation_height(gate)
}

/// The shipped gate for `network`, asserted to be ABOVE `BLOCK_HEIGHT` so the probe is on
/// the side of the gate it claims to test (INV-GOV-001).
fn shipped_gate_above_block_height(network: Network) -> u64 {
    let gate = NetworkParams::defaults(network).bls_key_rotation_activation_height;
    assert!(
        BLOCK_HEIGHT < gate,
        "{network:?}: this probe only means anything BELOW the shipped rotation gate. The \
         gate is now {gate} and the probe height is {BLOCK_HEIGHT}; a re-pin moved this \
         row to the other side, so it needs a lower probe height — not deletion."
    );
    gate
}

// ===========================================================================
// P1 — below the SHIPPED gate, a rotation poisons the block, in every mode
// ===========================================================================

// REQ-ROT-004 — Decision: whether a `RotateBlsKey` transaction can enter a block before the
// feature is activated. If it can, an un-upgraded node (which cannot decode the type) and
// an upgraded node disagree on the block, which is a chain split — the INC-I-075 shape.
// The height is derived from the SHIPPED params on all three networks (INV-GOV-001).
#[test]
fn req_rot_004_a_rotation_below_the_shipped_gate_invalidates_the_block_on_every_network() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let gate = shipped_gate_above_block_height(network);
        let ctx = ctx_for(network, gate);
        let params = ConsensusParams::for_network(network);
        let block = block_with(vec![well_formed_rotation()], &params);

        let err = validate_block_with_mode(&block, &ctx, ValidationMode::Full)
            .expect_err("a block carrying a rotation below the activation height must be INVALID");
        assert!(
            err.to_string().contains("[ERRTX-ROT002]"),
            "{network:?}: expected [ERRTX-ROT002] below the shipped gate ({gate}), got: {err}"
        );
    }
}

// REQ-ROT-004 — Decision: whether the gate sits inside a mode-specific branch. `Light` and
// `Replay` skip the header and VDF work during sync and disaster recovery; if the gate
// went with them, a syncing node would ACCEPT a block a live node REJECTS and the two
// would end up on different chains (D6). The claim is stated as the ERROR CODE, not as
// "some error": a mode that rejects for an unrelated reason (a header rule, a size rule)
// would satisfy a bare `is_err()` while the gate itself is missing from that path.
#[test]
fn req_rot_004_every_mode_reports_the_gate_by_its_own_error_code() {
    let gate = shipped_gate_above_block_height(Network::Mainnet);
    let ctx = ctx_for(Network::Mainnet, gate);
    let params = ConsensusParams::mainnet();
    let block = block_with(vec![well_formed_rotation()], &params);

    for mode in ALL_MODES {
        let err = validate_block_with_mode(&block, &ctx, mode)
            .expect_err("a below-gate rotation must be rejected");
        assert!(
            err.to_string().contains("[ERRTX-ROT002]"),
            "{}: expected [ERRTX-ROT002] — the gate must precede the mode match so an \
             un-upgraded node and an upgraded node reach the SAME verdict (D6). Got: {err}",
            mode_name(mode)
        );
    }
}

// ===========================================================================
// P2 — non-vacuity: a block with no rotation is untouched by the gate
// ===========================================================================

// REQ-ROT-004 — Decision: whether the gate was written over-broadly, e.g. "below the
// activation height, reject". That would invalidate every historical block on the chain
// the moment the binary rolls out. It also proves the P1 rejections above are about the
// ROTATION and not about the fixture block.
#[test]
fn req_rot_004_a_block_without_a_rotation_is_unaffected_by_the_gate() {
    let params = ConsensusParams::mainnet();
    let block = block_with(vec![plain_transfer()], &params);

    let gate = shipped_gate_above_block_height(Network::Mainnet);
    let below = ctx_for(Network::Mainnet, gate);
    let above = ctx_for(Network::Mainnet, 0);

    for mode in [ValidationMode::Light, ValidationMode::Replay] {
        let below_verdict = validate_block_with_mode(&block, &below, mode);
        let above_verdict = validate_block_with_mode(&block, &above, mode);

        assert!(
            below_verdict.is_ok(),
            "{}: a block carrying no rotation must keep the verdict it had before the \
             gate existed, got: {below_verdict:?}",
            mode_name(mode)
        );
        assert_eq!(
            format!("{below_verdict:?}"),
            format!("{above_verdict:?}"),
            "{}: the rotation gate must not change the verdict on a block that carries \
             no rotation, on either side of the height",
            mode_name(mode)
        );
    }
}

// ===========================================================================
// P3 — above the gate, the D6 check stops blocking
// ===========================================================================

// REQ-ROT-004 — Decision: whether the gate ever OPENS. A gate that rejects on both sides is
// indistinguishable from the M4 fail-closed arm, and the feature would be permanently
// unreachable while every test above still passed. The assertion is deliberately narrow —
// this fixture has no UTXO or producer state, so a later check may still reject it; what
// must not appear is the GATE's own code.
#[test]
fn req_rot_004_above_the_gate_a_well_formed_rotation_is_not_blocked_by_d6() {
    let ctx = ctx_for(Network::Mainnet, INJECTED_AH);
    assert!(
        ctx.current_height >= ctx.bls_key_rotation_activation_height,
        "precondition: this probe must sit AT OR ABOVE the injected gate"
    );

    let params = ConsensusParams::mainnet();
    let block = block_with(vec![well_formed_rotation()], &params);

    for mode in ALL_MODES {
        if let Err(err) = validate_block_with_mode(&block, &ctx, mode) {
            assert!(
                !err.to_string().contains("[ERRTX-ROT002]"),
                "{}: above the activation height the D6 gate must not fire; the block may \
                 still fail a later check, but not this one. Got: {err}",
                mode_name(mode)
            );
        }
    }
}

// REQ-ROT-004 — Decision: whether the gate is an OFF-BY-ONE. `current_height == gate` is
// the first activated block; a `>` comparison would delay the feature by exactly one
// block and, worse, would make two implementations of the same rule disagree on that
// block — a one-block fork at every activation.
#[test]
fn req_rot_004_the_gate_opens_at_the_activation_height_itself() {
    let params = ConsensusParams::mainnet();
    let block = block_with(vec![well_formed_rotation()], &params);
    let now = block_timestamp(&params);

    let at_gate = ValidationContext::new(params.clone(), Network::Mainnet, now, INJECTED_AH)
        .with_bls_key_rotation_activation_height(INJECTED_AH);
    if let Err(err) = validate_block_with_mode(&block, &at_gate, ValidationMode::Light) {
        assert!(
            !err.to_string().contains("[ERRTX-ROT002]"),
            "the activation height itself is ACTIVATED (>=, not >); got: {err}"
        );
    }

    let one_below = ValidationContext::new(params, Network::Mainnet, now, INJECTED_AH - 1)
        .with_bls_key_rotation_activation_height(INJECTED_AH);
    let err = validate_block_with_mode(&block, &one_below, ValidationMode::Light)
        .expect_err("the block one below the activation height is still gated");
    assert!(
        err.to_string().contains("[ERRTX-ROT002]"),
        "one block below the gate must still be rejected, got: {err}"
    );
}

// ===========================================================================
// P4 — REQ-ROT-SEC-008: no skip arm at block level
// ===========================================================================

// REQ-ROT-SEC-008 — Decision: whether an invalid rotation is merely IGNORED instead of
// invalidating its block. The skip-at-apply pattern exists in this codebase for STATEFUL
// producer failures, so reaching for it here is the natural mistake — and it would mean
// the crypto is never enforced anywhere in consensus, letting any user install an
// arbitrary BLS key for any registered producer (architecture D6 / decision C8).
#[test]
fn req_rot_sec_008_a_malformed_rotation_above_the_gate_invalidates_the_block() {
    let ctx = ctx_for(Network::Mainnet, INJECTED_AH);
    let params = ConsensusParams::mainnet();
    let block = block_with(vec![malformed_rotation()], &params);

    for mode in ALL_MODES {
        let err = validate_block_with_mode(&block, &ctx, mode).expect_err(&format!(
            "{}: a block carrying a malformed rotation must be INVALID — there is no skip \
             arm at block level (REQ-ROT-SEC-008)",
            mode_name(mode)
        ));
        assert!(
            err.to_string().contains("[ERRTX-ROT010]"),
            "{}: expected the payload-length code [ERRTX-ROT010], got: {err}",
            mode_name(mode)
        );
    }
}

// REQ-ROT-SEC-008 — Decision: whether the block-level rejection is really caused by the
// rotation. The control below is the SAME block with the rotation removed: it must be
// accepted, or the test above would pass on a fixture that was invalid for its own
// reasons and would keep passing after the rule was deleted.
#[test]
fn req_rot_sec_008_the_same_block_without_the_rotation_is_accepted() {
    let ctx = ctx_for(Network::Mainnet, INJECTED_AH);
    let params = ConsensusParams::mainnet();

    let poisoned = block_with(vec![plain_transfer(), malformed_rotation()], &params);
    let clean = block_with(vec![plain_transfer()], &params);

    for mode in [ValidationMode::Light, ValidationMode::Replay] {
        assert!(
            validate_block_with_mode(&poisoned, &ctx, mode).is_err(),
            "{}: one malformed rotation poisons the whole block",
            mode_name(mode)
        );
        assert!(
            validate_block_with_mode(&clean, &ctx, mode).is_ok(),
            "{}: the identical block without the rotation must be ACCEPTED",
            mode_name(mode)
        );
    }
}
