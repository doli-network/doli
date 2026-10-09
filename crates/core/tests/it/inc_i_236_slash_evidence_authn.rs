//! INC-I-236 M3 — slash evidence authentication and bounded decoding.
//!
// covers: crates/core/src/validation/tx_types.rs crates/core/src/validation/transaction.rs
//
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext)
//                  fn validate_transaction_skip_registration_vdf(&Transaction, &ValidationContext)
//   O1 return Result<(), ValidationError>; O2 none (no mutable params / receiver / store)
//   PATHS: P-AH (h >= AH: signature + genesis rules) | P-PRE (h < AH: today's verdict)
//          P-DEC (extra_data decode, every height)
// INPUT PARTITIONS
//   IE-SIGNED  both headers signed by accused over signing_message, correct genesis
//   IE-EMPTY   pi empty (keyless forgery, public VDFs)
//   IE-GARB    pi 64 / 63 / 65 garbage bytes
//   IE-OTHER   valid signature by a non-accused key
//   IE-SWAP    header 2 carries header 1's signature
//   IE-HALF    one header signed, the other unsigned
//   IE-GEN     signed, genesis_hash != ctx genesis (one or both headers)
//   IE-SAME    signed, identical headers (structural control)
//   ID-GARB    random / 0xFF extra_data, 1 B .. 4 MiB
//   ID-LEN     bincode Vec length prefix rewritten to huge values
//   ID-BIG     well-formed, signed evidence > 8 MiB
// MATRIX (every cell asserted on BOTH entry points)
//   O1 x P-AH  x IE-EMPTY/GARB/OTHER/SWAP/HALF/GEN -> InvalidSlash
//   O1 x P-AH  x IE-SIGNED -> Ok ; IE-SAME -> InvalidSlash
//   O1 x P-PRE x IE-EMPTY/GEN -> Ok (regression lock)
//   O1 x P-DEC x ID-GARB/LEN x {P-PRE, P-AH} -> InvalidSlash, no panic
//   O1 x P-AH  x ID-BIG -> InvalidSlash

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::block::BlockHeader;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{SlashData, SlashingEvidence, Transaction};
use doli_core::validation::{
    validate_transaction, validate_transaction_skip_registration_vdf, ValidationContext,
    ValidationError,
};

const HEIGHT: u64 = 1_000;
const SLOT: u32 = 5_000;
const NETWORK: Network = Network::Testnet;

fn base_ctx() -> ValidationContext {
    let params = ConsensusParams::testnet();
    let ts = params.slot_to_timestamp(SLOT);
    ValidationContext::new(params, NETWORK, ts, HEIGHT)
}

fn ctx_with_ah(ah: u64) -> ValidationContext {
    let mut ctx = base_ctx();
    ctx.inc_i_236_block_signature_activation_height = ah;
    ctx
}

fn armed() -> ValidationContext {
    ctx_with_ah(HEIGHT)
}

fn pre_ah_ctxs() -> [ValidationContext; 2] {
    [base_ctx(), ctx_with_ah(HEIGHT + 1)]
}

fn every_height_ctx() -> [ValidationContext; 3] {
    [base_ctx(), ctx_with_ah(HEIGHT + 1), armed()]
}

fn accused() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

/// Header with a valid public VDF for `producer`; `pi` empty.
fn header(ctx: &ValidationContext, prev_byte: u8, producer: PublicKey) -> BlockHeader {
    let prev_hash = Hash::from_bytes([prev_byte; 32]);
    let merkle_root = doli_core::block::compute_merkle_root(&[]);
    let input = vdf::block_input(&prev_hash, &merkle_root, SLOT, &producer);
    let value = hash_chain_vdf(&input, NETWORK.heartbeat_vdf_iterations()).to_vec();
    BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: ctx.params.genesis_hash,
        timestamp: ctx.params.slot_to_timestamp(SLOT),
        slot: SLOT,
        producer,
        vdf_output: vdf::VdfOutput { value },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

fn sign(h: &mut BlockHeader, kp: &KeyPair) {
    let msg = h.signing_message();
    h.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

fn signed_pair(ctx: &ValidationContext, kp: &KeyPair) -> (BlockHeader, BlockHeader) {
    let mut h1 = header(ctx, 0xAB, *kp.public_key());
    let mut h2 = header(ctx, 0xCD, *kp.public_key());
    sign(&mut h1, kp);
    sign(&mut h2, kp);
    (h1, h2)
}

fn slash_tx(accused: PublicKey, h1: BlockHeader, h2: BlockHeader) -> Transaction {
    Transaction::new_slash_producer(SlashData {
        producer_pubkey: accused,
        evidence: SlashingEvidence::DoubleProduction {
            block_header_1: h1,
            block_header_2: h2,
        },
        reporter_signature: Signature::from_bytes([0u8; 64]),
    })
}

fn raw_slash_tx(extra: Vec<u8>) -> Transaction {
    let ctx = armed();
    let pk = *accused().public_key();
    let mut tx = slash_tx(pk, header(&ctx, 1, pk), header(&ctx, 2, pk));
    tx.extra_data = extra;
    tx
}

type Verdict = (&'static str, Result<(), ValidationError>);

fn verdicts(tx: &Transaction, ctx: &ValidationContext) -> [Verdict; 2] {
    [
        ("validate_transaction", validate_transaction(tx, ctx)),
        (
            "validate_transaction_skip_registration_vdf",
            validate_transaction_skip_registration_vdf(tx, ctx),
        ),
    ]
}

fn assert_invalid_slash(tx: &Transaction, ctx: &ValidationContext, what: &str) {
    for (entry, v) in verdicts(tx, ctx) {
        assert!(
            matches!(v, Err(ValidationError::InvalidSlash(_))),
            "{what}: {entry} must return InvalidSlash, got {v:?}"
        );
    }
}

fn assert_ok(tx: &Transaction, ctx: &ValidationContext, what: &str) {
    for (entry, v) in verdicts(tx, ctx) {
        assert!(v.is_ok(), "{what}: {entry} must accept, got {v:?}");
    }
}

// REQ-AUTH-006 — Decision: a failure means anyone can slash any producer's bond with keyless, public-data evidence.
#[test]
fn req_auth_006_keyless_unsigned_evidence_rejected_at_ah() {
    let ctx = armed();
    let victim = PublicKey::from_bytes([0x11; 32]);
    let tx = slash_tx(
        victim,
        header(&ctx, 0xAB, victim),
        header(&ctx, 0xCD, victim),
    );
    assert_invalid_slash(&tx, &ctx, "unsigned keyless evidence at h == AH");
    let mut later = armed();
    later.current_height = HEIGHT + 10_000;
    assert_invalid_slash(&tx, &later, "unsigned keyless evidence far above AH");
}

// REQ-AUTH-006 — Decision: a failure means pi bytes of the right or wrong length bypass the signature check.
#[test]
fn req_auth_006_garbage_pi_rejected_at_ah() {
    let ctx = armed();
    let pk = *accused().public_key();
    for pi in [
        vec![0x42u8; 64],
        vec![0x00u8; 64],
        vec![0x42u8; 63],
        vec![0x42u8; 65],
    ] {
        let mut h1 = header(&ctx, 0xAB, pk);
        let mut h2 = header(&ctx, 0xCD, pk);
        h1.vdf_proof.pi = pi.clone();
        h2.vdf_proof.pi = pi.clone();
        let tx = slash_tx(pk, h1, h2);
        assert_invalid_slash(&tx, &ctx, &format!("garbage pi len {}", pi.len()));
    }
}

// REQ-AUTH-006 — Decision: a failure means a signature by any key, not the accused's, is accepted as proof.
#[test]
fn req_auth_006_signature_by_other_key_rejected_at_ah() {
    let ctx = armed();
    let pk = *accused().public_key();
    let attacker = KeyPair::from_seed([0x99; 32]);
    let mut h1 = header(&ctx, 0xAB, pk);
    let mut h2 = header(&ctx, 0xCD, pk);
    sign(&mut h1, &attacker);
    sign(&mut h2, &attacker);
    let tx = slash_tx(pk, h1, h2);
    assert_invalid_slash(&tx, &ctx, "both headers signed by attacker key");
}

// REQ-AUTH-006 — Decision: a failure means the signature is not bound to its own header, so one real block forges a second.
#[test]
fn req_auth_006_header2_reuses_header1_signature_rejected_at_ah() {
    let ctx = armed();
    let kp = accused();
    let (h1, mut h2) = signed_pair(&ctx, &kp);
    h2.vdf_proof.pi = h1.vdf_proof.pi.clone();
    let tx = slash_tx(*kp.public_key(), h1, h2);
    assert_invalid_slash(&tx, &ctx, "header 2 carries header 1 signature");
}

// REQ-AUTH-006 — Decision: a failure means only one header is checked, so one honest block plus one forgery slashes.
#[test]
fn req_auth_006_one_unsigned_header_rejected_at_ah() {
    let ctx = armed();
    let kp = accused();
    let (h1, mut h2) = signed_pair(&ctx, &kp);
    h2.vdf_proof.pi = Vec::new();
    let tx = slash_tx(*kp.public_key(), h1.clone(), h2.clone());
    assert_invalid_slash(&tx, &ctx, "header 2 unsigned");
    let tx = slash_tx(*kp.public_key(), h2, h1);
    assert_invalid_slash(&tx, &ctx, "header 1 unsigned");
}

// REQ-AUTH-006 — Decision: a failure means signed headers from another chain (testnet, devnet, a fork) slash on this one.
#[test]
fn req_auth_006_wrong_genesis_signed_evidence_rejected_at_ah() {
    let ctx = armed();
    let kp = accused();
    let pk = *kp.public_key();
    let foreign = Hash::from_bytes([0x77; 32]);
    assert_ne!(foreign, ctx.params.genesis_hash);

    let mut h1 = header(&ctx, 0xAB, pk);
    let mut h2 = header(&ctx, 0xCD, pk);
    h1.genesis_hash = foreign;
    h2.genesis_hash = foreign;
    sign(&mut h1, &kp);
    sign(&mut h2, &kp);
    let tx = slash_tx(pk, h1, h2);
    assert_invalid_slash(&tx, &ctx, "both headers foreign genesis");

    let (good1, _) = signed_pair(&ctx, &kp);
    let mut bad2 = header(&ctx, 0xCD, pk);
    bad2.genesis_hash = foreign;
    sign(&mut bad2, &kp);
    let tx = slash_tx(pk, good1, bad2);
    assert_invalid_slash(&tx, &ctx, "header 2 foreign genesis");
}

// REQ-AUTH-006 — Decision: a failure means real double production by a signing producer can no longer be slashed.
#[test]
fn req_auth_006_validly_signed_double_production_accepted_at_ah() {
    let ctx = armed();
    let kp = accused();
    let (h1, h2) = signed_pair(&ctx, &kp);
    let tx = slash_tx(*kp.public_key(), h1, h2);
    assert_ok(
        &tx,
        &ctx,
        "two signed, different headers, same slot, correct genesis",
    );
}

// REQ-AUTH-006 — Decision: a failure means the signature path bypasses the existing structural checks.
#[test]
fn req_auth_006_signed_identical_headers_still_rejected_at_ah() {
    let ctx = armed();
    let kp = accused();
    let (h1, _) = signed_pair(&ctx, &kp);
    let tx = slash_tx(*kp.public_key(), h1.clone(), h1);
    assert_invalid_slash(&tx, &ctx, "signed but identical headers");
}

// REQ-AUTH-006 — Decision: a failure means the fix changed the pre-AH verdict for unsigned evidence.
#[test]
fn req_auth_006_pre_ah_unsigned_evidence_verdict_unchanged() {
    let victim = PublicKey::from_bytes([0x11; 32]);
    for ctx in pre_ah_ctxs() {
        let tx = slash_tx(
            victim,
            header(&ctx, 0xAB, victim),
            header(&ctx, 0xCD, victim),
        );
        assert_ok(
            &tx,
            &ctx,
            "pre-AH unsigned keyless evidence (today's verdict)",
        );
    }
}

// REQ-AUTH-006 — Decision: a failure means the genesis rule leaked below the AH and changed pre-AH verdicts.
#[test]
fn req_auth_006_pre_ah_wrong_genesis_verdict_unchanged() {
    let victim = PublicKey::from_bytes([0x11; 32]);
    for ctx in pre_ah_ctxs() {
        let mut h1 = header(&ctx, 0xAB, victim);
        let mut h2 = header(&ctx, 0xCD, victim);
        h1.genesis_hash = Hash::from_bytes([0x77; 32]);
        h2.genesis_hash = Hash::from_bytes([0x77; 32]);
        let tx = slash_tx(victim, h1, h2);
        assert_ok(
            &tx,
            &ctx,
            "pre-AH foreign-genesis evidence (today's verdict)",
        );
    }
}

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u8
        })
        .collect()
}

// REQ-AUTH-SEC-003 — Decision: a failure means untrusted slash bytes can panic a node or pass as evidence.
#[test]
fn req_auth_sec_003_garbage_extra_data_rejected_without_panic() {
    let mut payloads = vec![vec![0x00u8], vec![0xFFu8; 7], vec![0xFFu8; 4096]];
    for (i, len) in [1usize, 31, 100, 4096, 65_536].into_iter().enumerate() {
        payloads.push(pseudo_random(len, 0xD011_0000 + i as u64));
    }
    for ctx in every_height_ctx() {
        for p in &payloads {
            let tx = raw_slash_tx(p.clone());
            assert_invalid_slash(&tx, &ctx, &format!("garbage extra_data len {}", p.len()));
        }
    }
}

// REQ-AUTH-SEC-003 — Decision: a failure means a multi-MB garbage payload panics or decodes as evidence.
#[test]
fn req_auth_sec_003_multi_mb_garbage_rejected_without_panic() {
    let big = pseudo_random(4 * 1024 * 1024, 0xBEEF);
    for ctx in every_height_ctx() {
        let tx = raw_slash_tx(big.clone());
        assert_invalid_slash(&tx, &ctx, "4 MiB garbage extra_data");
    }
}

/// Valid encoding with header 1's `vdf_output.value` length prefix replaced by `len`.
fn with_vec_len_prefix(len: u64) -> Vec<u8> {
    let ctx = armed();
    let kp = accused();
    let (h1, h2) = signed_pair(&ctx, &kp);
    let marker = h1.vdf_output.value.clone();
    let mut bytes = slash_tx(*kp.public_key(), h1, h2).extra_data;
    let mut needle = (marker.len() as u64).to_le_bytes().to_vec();
    needle.extend_from_slice(&marker);
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
        .expect("vdf_output length prefix located in encoding");
    bytes[at..at + 8].copy_from_slice(&len.to_le_bytes());
    bytes
}

// REQ-AUTH-SEC-003 — Decision: a failure means a forged length prefix triggers an unbounded allocation or panic.
#[test]
fn req_auth_sec_003_huge_vec_length_prefix_rejected_without_panic() {
    for len in [
        u64::MAX,
        u64::MAX / 2,
        1u64 << 40,
        1u64 << 32,
        64 * 1024 * 1024,
    ] {
        let payload = with_vec_len_prefix(len);
        for ctx in every_height_ctx() {
            let tx = raw_slash_tx(payload.clone());
            assert_invalid_slash(&tx, &ctx, &format!("vec length prefix {len}"));
        }
    }
}

// REQ-AUTH-SEC-003 — Decision: a failure means slash decoding is unbounded and signed multi-MB evidence is processed in full.
#[test]
fn req_auth_sec_003_oversized_wellformed_signed_evidence_rejected_at_ah() {
    let ctx = armed();
    let kp = accused();
    let pk = *kp.public_key();
    let filler: Vec<PublicKey> = vec![*KeyPair::from_seed([0x33; 32]).public_key(); 300_000];
    let mut h1 = header(&ctx, 0xAB, pk);
    let mut h2 = header(&ctx, 0xCD, pk);
    h1.missed_producers = filler.clone();
    h2.missed_producers = filler;
    sign(&mut h1, &kp);
    sign(&mut h2, &kp);
    let tx = slash_tx(pk, h1, h2);
    assert!(
        tx.extra_data.len() > 8 * 1024 * 1024,
        "payload must exceed 8 MiB"
    );
    assert_invalid_slash(&tx, &ctx, "well-formed signed evidence > 8 MiB");
}
