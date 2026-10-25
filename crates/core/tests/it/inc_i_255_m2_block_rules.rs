//! INC-I-255 M2 — block-level slash rules above the slashing gate: a per-block
//! cap and one slash per accused, judged before the mode split so every mode
//! reaches one verdict, and inert below the gate.
//!
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode) -> Result<(), ValidationError>
//   O1 return (InvalidSlash message) per mode; pure — no mutable params, no receiver, no store writes
// PATHS: P-AH (height >= slash gate and >= signature gate) | P-BELOW (height < slash gate)
//        | modes Full, Light, Replay | P-VDF evidence headers with an invalid VDF
// MATRIX
//   O1 x P-AH x cap + 1 distinct accused x 3 modes -> InvalidSlash, one message (inc_i_255_m2_block_over_cap_*)
//   O1 x P-AH x cap distinct accused x 3 modes -> Ok (inc_i_255_m2_block_at_cap_*)
//   O1 x P-AH x two slashes, one accused x 3 modes -> InvalidSlash, one message; two accused -> Ok (inc_i_255_m2_block_duplicate_*)
//   O1 x P-AH x P-VDF x cap + 1 x Full -> the cap verdict, not an evidence-VDF verdict (inc_i_255_m2_block_cap_judged_before_evidence_vdf)
//   O1 x P-BELOW x {1, cap + 1, duplicate} x Light/Replay -> Ok (inc_i_255_m2_block_rules_inert_below_gate)
// INPUT PARTITIONS:
//   slash count: 1 | 2 | cap | cap + 1; accused: distinct | repeated
//   evidence VDF: valid | garbage; gate: at | below

use crypto::{KeyPair, PublicKey};
use doli_core::block::{Block, BlockHeader};
use doli_core::transaction::{SlashData, Transaction};
use doli_core::validation::{
    validate_block_with_mode, SlashTargetState, ValidationContext, ValidationError, ValidationMode,
};

use super::inc_i_255_slash_evidence::{ctx_at, header, sign, HEIGHT};

const SLASH_CAP: usize = 8;
const BLOCK_SLOT: u32 = 5_000;
const EVIDENCE_SLOT: u32 = 4_990;
/// Slash gate 20 blocks below `HEIGHT`: tenure 20 exceeds the evidence age 10.
const OPEN_GATE: u64 = HEIGHT - 20;
const MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];

fn accused(i: usize) -> KeyPair {
    KeyPair::from_seed([0x60 + i as u8; 32])
}

fn signed(prev: u8, kp: &KeyPair) -> BlockHeader {
    let mut h = header(prev, EVIDENCE_SLOT, *kp.public_key());
    sign(&mut h, kp);
    h
}

fn slash(kp: &KeyPair, salts: (u8, u8)) -> Transaction {
    let (a, b) = (signed(salts.0, kp), signed(salts.1, kp));
    Transaction::new_slash_producer(SlashData::double_production(*kp.public_key(), a, b))
}

fn garbage_vdf(kp: &KeyPair, salts: (u8, u8)) -> Transaction {
    let mut hs = [
        header(salts.0, EVIDENCE_SLOT, *kp.public_key()),
        header(salts.1, EVIDENCE_SLOT, *kp.public_key()),
    ];
    for h in hs.iter_mut() {
        h.vdf_output.value = vec![0xEE; 32];
        sign(h, kp);
    }
    let [a, b] = hs;
    Transaction::new_slash_producer(SlashData::double_production(*kp.public_key(), a, b))
}

fn distinct(n: usize) -> Vec<Transaction> {
    (0..n).map(|i| slash(&accused(i), (0xA1, 0xB2))).collect()
}

fn block_producer() -> KeyPair {
    KeyPair::from_seed([0x0B; 32])
}

fn block_of(txs: Vec<Transaction>) -> Block {
    let producer = block_producer();
    let p = doli_core::consensus::ConsensusParams::devnet();
    let mut h = header(0x01, BLOCK_SLOT, *producer.public_key());
    h.timestamp = p.slot_to_timestamp(BLOCK_SLOT);
    let mut block = Block::new(h, txs);
    block.header.merkle_root = block.compute_merkle_root();
    let h = &block.header;
    let input = vdf::block_input(&h.prev_hash, &h.merkle_root, BLOCK_SLOT, &h.producer);
    let iterations = doli_core::network::Network::Devnet.heartbeat_vdf_iterations();
    block.header.vdf_output.value =
        doli_core::tpop::heartbeat::hash_chain_vdf(&input, iterations).to_vec();
    sign(&mut block.header, &producer);
    block
}

/// Parent at `BLOCK_SLOT - 1`, devnet unbonding period: evidence age 10.
fn ctx(slash_gate: u64, accused_keys: impl IntoIterator<Item = PublicKey>) -> ValidationContext {
    let active = SlashTargetState {
        active: true,
        oldest_bond_slot: Some(0),
        ..Default::default()
    };
    let mut c = ctx_at(HEIGHT, slash_gate, HEIGHT)
        .with_prev_slot(BLOCK_SLOT - 1)
        .with_producers_weighted(vec![(*block_producer().public_key(), 1)])
        .with_slash_targets(accused_keys.into_iter().map(|pk| (pk, active)));
    c.unbonding_period = doli_core::network::Network::Devnet
        .params()
        .unbonding_period;
    c
}

fn keys(n: usize) -> Vec<PublicKey> {
    (0..n).map(|i| *accused(i).public_key()).collect()
}

fn slash_msg(r: &Result<(), ValidationError>) -> Option<String> {
    match r {
        Err(ValidationError::InvalidSlash(m)) => Some(m.clone()),
        _ => None,
    }
}

fn refused_in_every_mode(block: &Block, ctx: &ValidationContext, case: &str) -> String {
    let msgs: Vec<_> = MODES
        .iter()
        .map(|m| {
            let r = validate_block_with_mode(block, ctx, *m);
            (m, slash_msg(&r), format!("{r:?}"))
        })
        .collect();
    for (mode, msg, raw) in &msgs {
        assert!(
            msg.is_some(),
            "{case}: {mode:?} must refuse with InvalidSlash; got {raw}"
        );
    }
    for (mode, msg, _) in &msgs {
        assert_eq!(
            msg, &msgs[0].1,
            "{case}: {mode:?} message differs from Full's"
        );
    }
    msgs[0].1.clone().unwrap_or_default()
}

// REQ-SLASH-004 — Decision: a failure means a block with more slashes than the cap is valid in some mode, so per-block evidence cost is unbounded.
#[test]
fn inc_i_255_m2_block_over_cap_rejected_in_every_mode() {
    let block = block_of(distinct(SLASH_CAP + 1));
    refused_in_every_mode(&block, &ctx(HEIGHT, keys(SLASH_CAP + 1)), "cap + 1");
}

// REQ-SLASH-004 — Decision: a failure means the cap or the duplicate rule over-rejects a legitimate block holding exactly the cap.
#[test]
fn inc_i_255_m2_block_at_cap_not_refused() {
    let block = block_of(distinct(SLASH_CAP));
    let c = ctx(OPEN_GATE, keys(SLASH_CAP));
    for mode in MODES {
        let r = validate_block_with_mode(&block, &c, mode);
        assert!(
            r.is_ok(),
            "{mode:?}: exactly the cap must validate; got {r:?}"
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means two slashes naming one accused in one block are valid in some mode (a double queue on apply).
#[test]
fn inc_i_255_m2_block_duplicate_accused_rejected_in_every_mode() {
    let kp = accused(0);
    let block = block_of(vec![slash(&kp, (0xA1, 0xB2)), slash(&kp, (0xC3, 0xD4))]);
    refused_in_every_mode(&block, &ctx(OPEN_GATE, keys(1)), "duplicate accused");

    let pair = block_of(distinct(2));
    let r = validate_block_with_mode(&pair, &ctx(OPEN_GATE, keys(2)), ValidationMode::Light);
    assert!(
        r.is_ok(),
        "NON-VACUITY: two distinct accused validate; got {r:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means an over-cap block still pays evidence-VDF verification before being refused (the cap does not bound cost).
#[test]
fn inc_i_255_m2_block_cap_judged_before_evidence_vdf() {
    let n = SLASH_CAP + 1;
    let valid = block_of(distinct(n));
    let cap_msg = slash_msg(&validate_block_with_mode(
        &valid,
        &ctx(HEIGHT, keys(n)),
        ValidationMode::Light,
    ));
    assert!(
        cap_msg.is_some(),
        "the over-cap block with valid evidence must be refused by the cap"
    );
    let garbage = block_of(
        (0..n)
            .map(|i| garbage_vdf(&accused(i), (0xA1, 0xB2)))
            .collect(),
    );
    let full = validate_block_with_mode(&garbage, &ctx(HEIGHT, keys(n)), ValidationMode::Full);
    assert_eq!(
        slash_msg(&full),
        cap_msg,
        "Full must refuse by the cap before any evidence VDF; got {full:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means the cap or duplicate rule fires below the slashing gate, changing pre-activation consensus.
#[test]
fn inc_i_255_m2_block_rules_inert_below_gate() {
    let below = HEIGHT + 1;
    let kp = accused(0);
    let single = block_of(vec![slash(&kp, (0xA1, 0xB2))]);
    let over = block_of(distinct(SLASH_CAP + 1));
    let dup = block_of(vec![slash(&kp, (0xA1, 0xB2)), slash(&kp, (0xC3, 0xD4))]);
    for mode in [ValidationMode::Light, ValidationMode::Replay] {
        let base = validate_block_with_mode(&single, &ctx(below, keys(1)), mode);
        assert!(
            base.is_ok(),
            "PRECONDITION {mode:?}: one slash validates below the gate; got {base:?}"
        );
        for (case, block, n) in [("cap + 1", &over, SLASH_CAP + 1), ("duplicate", &dup, 1)] {
            let r = validate_block_with_mode(block, &ctx(below, keys(n)), mode);
            assert!(
                r.is_ok(),
                "{mode:?} {case}: below the gate no block slash rule applies; got {r:?}"
            );
        }
    }
}

// REQ-SLASH-004 — Decision: a failure means the consensus cap constant is missing or differs from the value every test and peer assumes.
#[test]
fn inc_i_255_m2_cap_constant_named_in_consensus() {
    let src = include_str!("../../src/consensus/constants.rs");
    assert!(
        src.contains("pub const MAX_SLASH_TXS_PER_BLOCK: usize = 8;"),
        "consensus constants must define MAX_SLASH_TXS_PER_BLOCK = {SLASH_CAP}"
    );
}
