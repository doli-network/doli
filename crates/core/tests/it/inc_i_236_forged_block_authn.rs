//! INC-I-236 M1 — block producer authentication: the forged-block contract.
//!
//! A block names its author in `header.producer` but, before the fix, carries no
//! signature; the VDF input is all public data. These tests are the attacker:
//! the forger never holds the victim's private key. From the activation height
//! (`inc_i_236_block_signature_activation_height`) the 64-byte Ed25519 signature
//! in `header.vdf_proof.pi` over `header.signing_message()` is mandatory, in
//! Full, Light and Replay alike (rule runs before `match mode`).
//!
// covers: block.rs network_params validation/block.rs validation/types.rs
// covers: crates/core/src/block.rs crates/core/src/network_params/mod.rs crates/core/src/network_params/defaults.rs crates/core/src/network_params/env_loader.rs crates/core/src/validation/block.rs crates/core/src/validation/types.rs crates/core/src/validation/error.rs
//!
//! TDD RED: does not compile until `signing_message`, the ctx/params field and
//! `ValidationError::InvalidBlockSignature { reason }` exist.
//!
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode)
//   O1 return Result<(), ValidationError> — the only output
//   O2 mutable params / receiver / store writes: NONE (pure)
//   PATHS: P-SIG (h >= AH: signature rule, then mode arm) | P-NOSIG (h < AH: pi never read)
//          x mode arm {Full, Light, Replay}
// INPUT PARTITIONS
//   IP-F  forged: producer = victim, pi empty
//   IP-V  validly signed by producer key over signing_message
//   IP-G  pi garbage (any length) below AH
//   IP-W  valid signature, producer NOT scheduled (eligibility control)
// MATRIX (this file; malformed / wrong-message / strict / golden cases in inc_i_236_block_sig_contract.rs)
//   O1 x P-SIG   x IP-F x 3 modes -> req_auth_005_forged_block_rejected_at_ah_{full,light,replay}
//   O1 x P-SIG   x IP-V x 3 modes -> req_auth_003_signed_block_accepted_at_ah_all_modes
//   O1 x P-NOSIG x IP-F x 3 modes -> req_auth_003_forged_block_unchanged_below_ah_all_modes
//   O1 x P-NOSIG (ctx default)    -> req_auth_003_ctx_default_never_requires_signature
//   O1 x P-NOSIG x IP-G x 3 modes -> req_auth_003_garbage_pi_ignored_below_ah
//   O1 x P-SIG   x IP-W x Full    -> req_auth_003_control_signed_but_unscheduled_still_rejected

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{Block, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::validation::validate_block_with_mode;
use doli_core::validation::{ValidationContext, ValidationError, ValidationMode};

pub(crate) const HEIGHT: u64 = 1_000;
pub(crate) const MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];

/// Block in `victim`'s slot built from public data only; ctx at `HEIGHT`, AH untouched.
pub(crate) fn forge_block_as(victim: PublicKey) -> (Block, ValidationContext) {
    let params = ConsensusParams::testnet();
    let network = Network::Testnet;
    let slot: u32 = 5_000;
    let timestamp = params.slot_to_timestamp(slot);
    let prev_slot = slot - 1;
    let prev_timestamp = params.slot_to_timestamp(prev_slot);
    let prev_hash = Hash::from_bytes([0xAB; 32]);

    let ctx = ValidationContext::new(params.clone(), network, timestamp, HEIGHT)
        .with_prev_block(prev_slot, prev_timestamp, prev_hash)
        .with_epoch_producer_list(vec![victim]);

    let txs: Vec<doli_core::transaction::Transaction> = Vec::new();
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let vdf_input = vdf::block_input(&prev_hash, &merkle_root, slot, &victim);
    let vdf_value = hash_chain_vdf(&vdf_input, network.heartbeat_vdf_iterations()).to_vec();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: ctx.params.genesis_hash,
        timestamp,
        slot,
        producer: victim,
        vdf_output: vdf::VdfOutput { value: vdf_value },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    (Block::new(header, txs), ctx)
}

/// Ctx with the signature gate armed exactly at the block height.
pub(crate) fn armed(mut ctx: ValidationContext) -> ValidationContext {
    ctx.inc_i_236_block_signature_activation_height = HEIGHT;
    ctx
}

/// Producer-key signature over `signing_message()`, placed in `pi`.
pub(crate) fn sign_into_pi(block: &mut Block, kp: &KeyPair) {
    let msg = block.header.signing_message();
    block.header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

/// A validly signed block for a fresh keypair scheduled in its own slot.
pub(crate) fn signed_block() -> (Block, ValidationContext, KeyPair) {
    let kp = KeyPair::from_seed([0x5A; 32]);
    let (mut block, ctx) = forge_block_as(*kp.public_key());
    sign_into_pi(&mut block, &kp);
    (block, armed(ctx), kp)
}

pub(crate) fn assert_sig_rejected(block: &Block, ctx: &ValidationContext, what: &str) {
    for mode in MODES {
        let verdict = validate_block_with_mode(block, ctx, mode);
        assert!(
            matches!(verdict, Err(ValidationError::InvalidBlockSignature { .. })),
            "{what}: {mode:?} must return InvalidBlockSignature, got {verdict:?}"
        );
    }
}

pub(crate) fn assert_accepted(block: &Block, ctx: &ValidationContext, what: &str) {
    for mode in MODES {
        let verdict = validate_block_with_mode(block, ctx, mode);
        assert!(
            verdict.is_ok(),
            "{what}: {mode:?} must accept, got {verdict:?}"
        );
    }
}

fn victim() -> PublicKey {
    PublicKey::from_bytes([0x11; 32])
}

fn forged_verdict(mode: ValidationMode) -> Result<(), ValidationError> {
    let (block, ctx) = forge_block_as(victim());
    validate_block_with_mode(&block, &armed(ctx), mode)
}

// REQ-AUTH-005 — Decision: a failure means a keyless party can still author a block in a victim's slot on a live node.
#[test]
fn req_auth_005_forged_block_rejected_at_ah_full() {
    let v = forged_verdict(ValidationMode::Full);
    assert!(
        matches!(v, Err(ValidationError::InvalidBlockSignature { .. })),
        "Full: forged unsigned block at h >= AH must be rejected, got {v:?}"
    );
}

// REQ-AUTH-005 — Decision: a failure means a light-syncing node imports forged blocks a full node rejects (split verdict).
#[test]
fn req_auth_005_forged_block_rejected_at_ah_light() {
    let v = forged_verdict(ValidationMode::Light);
    assert!(
        matches!(v, Err(ValidationError::InvalidBlockSignature { .. })),
        "Light: forged unsigned block at h >= AH must be rejected, got {v:?}"
    );
}

// REQ-AUTH-005 — Decision: a failure means replay/rebuild accepts a forged block live validation refused, so state diverges on restart.
#[test]
fn req_auth_005_forged_block_rejected_at_ah_replay() {
    let v = forged_verdict(ValidationMode::Replay);
    assert!(
        matches!(v, Err(ValidationError::InvalidBlockSignature { .. })),
        "Replay: forged unsigned block at h >= AH must be rejected, got {v:?}"
    );
}

// REQ-AUTH-003 — Decision: a failure means honest signed blocks are rejected after the AH and the chain halts.
#[test]
fn req_auth_003_signed_block_accepted_at_ah_all_modes() {
    let (block, ctx, kp) = signed_block();
    assert_eq!(block.header.producer, *kp.public_key());
    assert_eq!(block.header.vdf_proof.pi.len(), 64);
    assert_accepted(&block, &ctx, "validly signed block at h == AH");

    let mut later = ctx.clone();
    later.current_height = HEIGHT + 1_000_000;
    assert_accepted(&block, &later, "validly signed block far above AH");
}

// REQ-AUTH-003 — Decision: a failure means the rule leaked below the AH and rejects already-final history (consensus break).
#[test]
fn req_auth_003_forged_block_unchanged_below_ah_all_modes() {
    let (block, mut ctx) = forge_block_as(victim());
    ctx.inc_i_236_block_signature_activation_height = HEIGHT + 1;
    assert_accepted(&block, &ctx, "forged block at h == AH-1");
}

// REQ-AUTH-001 — Decision: a failure means a ctx site that forgets to wire the AH enforces the rule (fail-open default would be u64::MAX).
#[test]
fn req_auth_001_ctx_default_never_requires_signature() {
    let (block, ctx) = forge_block_as(victim());
    assert_eq!(ctx.inc_i_236_block_signature_activation_height, u64::MAX);
    assert_accepted(&block, &ctx, "forged block with default ctx");
}

// REQ-AUTH-003 — Decision: a failure means pre-AH blocks with arbitrary pi bytes (history) would be read and possibly rejected.
#[test]
fn req_auth_003_garbage_pi_ignored_below_ah() {
    for len in [1usize, 63, 64, 65, 10 * 1024] {
        let (mut block, mut ctx) = forge_block_as(victim());
        block.header.vdf_proof.pi = vec![0xEE; len];
        ctx.inc_i_236_block_signature_activation_height = HEIGHT + 1;
        assert_accepted(&block, &ctx, &format!("garbage pi len {len} below AH"));
    }
}

// REQ-AUTH-003 — Decision: a failure means a valid signature by an unscheduled key bypasses slot eligibility.
#[test]
fn req_auth_003_control_signed_but_unscheduled_still_rejected() {
    let scheduled = victim();
    let (mut block, ctx) = forge_block_as(scheduled);
    let impostor = KeyPair::from_seed([0x22; 32]);
    block.header.producer = *impostor.public_key();
    let vdf_input = vdf::block_input(
        &block.header.prev_hash,
        &block.header.merkle_root,
        block.header.slot,
        impostor.public_key(),
    );
    block.header.vdf_output = vdf::VdfOutput {
        value: hash_chain_vdf(&vdf_input, Network::Testnet.heartbeat_vdf_iterations()).to_vec(),
    };
    sign_into_pi(&mut block, &impostor);

    let verdict = validate_block_with_mode(&block, &armed(ctx), ValidationMode::Full);
    assert!(verdict.is_err(), "unscheduled producer must be rejected");
    assert!(
        !matches!(verdict, Err(ValidationError::InvalidBlockSignature { .. })),
        "signature is valid; rejection must come from eligibility, got {verdict:?}"
    );
}
