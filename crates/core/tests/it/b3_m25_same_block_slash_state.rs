//! B3 M2.5 — REQ-B3-003 / REV-B3-001: one block with SlashProducer(K) and a Registration or AddBond for K.
//!
// covers: slash_state, slash_registration
//
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode) -> Result<(), ValidationError>
//   O1 return: Err(SlashedRegistrant { producer: K hex }) ([ERRTX071]) in every mode when the slash rule is active and
//      the block holds SlashProducer(K) and a Registration / AddBond whose slash_registrant is K, in either order;
//      otherwise the old verdict (Ok for these fixtures)
//   O2 mutable params / receiver: none (shared references only)
//   O3 store writes: none
// PATHS: P-SAME gate active, registrant == accused | P-BELOW slash gate above the height | P-OTHER registrant != accused
// INPUT PARTITIONS:
//   tx beside the slash: AddBond(K) | Registration(K) (K exited, locked collateral) | AddBond(K') | Registration(K')
//   order: slash first | registrant first; mode: Full | Light | Replay
// MATRIX:
//   O1 x P-SAME x AddBond x slash first x 3 modes -> b3_m25_core_slash_then_add_bond_same_key_refused
//   O1 x P-SAME x AddBond x registrant first x 3 modes -> b3_m25_core_add_bond_then_slash_same_key_refused
//   O1 x P-SAME x Registration x slash first x 3 modes -> b3_m25_core_slash_then_registration_same_key_refused
//   O1 x P-SAME x Registration x registrant first x 3 modes -> b3_m25_core_registration_then_slash_same_key_refused
//   O1 x P-BELOW x both types x both orders x {Light, Replay} -> Ok (b3_m25_core_below_gate_verdict_unchanged)
//   O1 x P-OTHER x both types x both orders x 3 modes -> Ok (b3_m25_core_other_key_accepted)

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::block::{Block, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, RegistrationData, SlashData, Transaction, TxType};
use doli_core::validation::{
    bond_owner, validate_block_with_mode, SlashTargetState, ValidationContext, ValidationError,
    ValidationMode,
};
use doli_core::Network;

use super::inc_i_255_slash_evidence::{ctx_at, header, sign, HEIGHT};

const BLOCK_SLOT: u32 = 5_000;
const EVIDENCE_SLOT: u32 = 4_990;
const OPEN_GATE: u64 = HEIGHT - 20;
const LOCK: u64 = 10_000_000;
const MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    AddBond,
    Registration,
}

fn accused() -> KeyPair {
    KeyPair::from_seed([0x71; 32])
}

fn other() -> KeyPair {
    KeyPair::from_seed([0x72; 32])
}

fn funder() -> KeyPair {
    KeyPair::from_seed([0x73; 32])
}

fn block_producer() -> KeyPair {
    KeyPair::from_seed([0x0B; 32])
}

fn unit() -> u64 {
    ConsensusParams::devnet().bond_amount(HEIGHT)
}

fn fixture_preconditions() {
    let p = ConsensusParams::devnet();
    assert!(
        !Network::Devnet.is_in_genesis(HEIGHT) && unit() > 0,
        "FIXTURE: HEIGHT is past genesis with a non-zero bond"
    );
    assert!(
        LOCK >= HEIGHT + p.blocks_per_era,
        "FIXTURE: bond lock floor"
    );
}

fn signed(prev: u8, kp: &KeyPair) -> BlockHeader {
    let mut h = header(prev, EVIDENCE_SLOT, *kp.public_key());
    sign(&mut h, kp);
    h
}

fn slash(kp: &KeyPair) -> Transaction {
    let (a, b) = (signed(0xA1, kp), signed(0xB2, kp));
    Transaction::new_slash_producer(SlashData::double_production(*kp.public_key(), a, b))
}

fn input(kp: &KeyPair) -> Vec<Input> {
    let mut i = Input::new(Hash::from_bytes([0x42; 32]), 0);
    i.public_key = Some(*kp.public_key());
    vec![i]
}

fn bond(producer: &PublicKey) -> Output {
    Output::bond(unit(), bond_owner(producer), LOCK, 0)
}

fn add_bond(producer: &KeyPair) -> Transaction {
    let pk = *producer.public_key();
    let mut tx = Transaction::new_add_bond(input(&funder()), pk, 1, unit(), LOCK);
    tx.outputs = vec![bond(&pk)];
    tx
}

fn registration(producer: &KeyPair) -> Transaction {
    let bls = BlsKeyPair::from_seed(&[0xB5; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let data = RegistrationData {
        public_key: *producer.public_key(),
        epoch: 0,
        vdf_output: Vec::new(),
        vdf_proof: Vec::new(),
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count: 1,
        bls_pubkey: bls.public_key().as_bytes().to_vec(),
        bls_pop: pop.as_bytes().to_vec(),
    };
    Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: input(producer),
        outputs: vec![bond(producer.public_key())],
        extra_data: bincode::serialize(&data).expect("FIXTURE: registration data"),
    }
}

fn registrant(kind: Kind, kp: &KeyPair) -> Transaction {
    match kind {
        Kind::AddBond => add_bond(kp),
        Kind::Registration => registration(kp),
    }
}

fn block_of(txs: Vec<Transaction>) -> Block {
    let producer = block_producer();
    let p = ConsensusParams::devnet();
    let mut h = header(0x01, BLOCK_SLOT, *producer.public_key());
    h.timestamp = p.slot_to_timestamp(BLOCK_SLOT);
    let mut block = Block::new(h, txs);
    block.header.merkle_root = block.compute_merkle_root();
    let h = &block.header;
    let input = vdf::block_input(&h.prev_hash, &h.merkle_root, BLOCK_SLOT, &h.producer);
    let iterations = Network::Devnet.heartbeat_vdf_iterations();
    block.header.vdf_output.value =
        doli_core::tpop::heartbeat::hash_chain_vdf(&input, iterations).to_vec();
    sign(&mut block.header, &producer);
    block
}

fn ordered(kind: Kind, kp: &KeyPair, slash_first: bool) -> Block {
    let (s, r) = (slash(&accused()), registrant(kind, kp));
    block_of(if slash_first { vec![s, r] } else { vec![r, s] })
}

/// AddBond: K active. Registration: K exited with locked collateral (a Registration
/// for an active key is refused by the duplicate-registration rule).
fn target(kind: Kind) -> SlashTargetState {
    match kind {
        Kind::AddBond => SlashTargetState {
            active: true,
            ..Default::default()
        },
        Kind::Registration => SlashTargetState {
            locked_collateral: true,
            ..Default::default()
        },
    }
}

fn ctx(slash_gate: u64, kind: Kind) -> ValidationContext {
    let other_active = SlashTargetState {
        active: true,
        ..Default::default()
    };
    let mut c = ctx_at(HEIGHT, slash_gate, HEIGHT)
        .with_prev_slot(BLOCK_SLOT - 1)
        .with_producers_weighted(vec![(*block_producer().public_key(), 1)])
        .with_slash_targets([
            (*accused().public_key(), target(kind)),
            (*other().public_key(), other_active),
        ]);
    c.unbonding_period = Network::Devnet.params().unbonding_period;
    c
}

fn names_accused(r: &Result<(), ValidationError>) -> bool {
    let want = accused().public_key().to_hex();
    matches!(r, Err(ValidationError::SlashedRegistrant { producer }) if *producer == want)
        && matches!(r, Err(e) if format!("{e}").contains("ERRTX071"))
}

fn non_vacuity(kind: Kind) {
    let c = ctx(OPEN_GATE, kind);
    for (what, block) in [
        ("the slash alone", block_of(vec![slash(&accused())])),
        (
            "the registrant alone",
            block_of(vec![registrant(kind, &accused())]),
        ),
    ] {
        for mode in MODES {
            let r = validate_block_with_mode(&block, &c, mode);
            assert!(
                r.is_ok(),
                "NON-VACUITY {kind:?} {mode:?}: {what} must validate; got {r:?}"
            );
        }
    }
}

fn assert_same_block_refused(kind: Kind, slash_first: bool) {
    fixture_preconditions();
    non_vacuity(kind);
    let block = ordered(kind, &accused(), slash_first);
    let c = ctx(OPEN_GATE, kind);
    let order = if slash_first {
        "[Slash(K), X(K)]"
    } else {
        "[X(K), Slash(K)]"
    };
    for mode in MODES {
        let r = validate_block_with_mode(&block, &c, mode);
        assert!(
            names_accused(&r),
            "{mode:?}: block {order} with X = {kind:?} must be refused with SlashedRegistrant {{ producer: K }} [ERRTX071]; got {r:?}"
        );
    }
}

// REQ-B3-003 — Decision: a failure means a block with Slash(K) then AddBond(K) is valid, so the new Bond UTXOs escape the burn and are orphaned at the boundary.
#[test]
fn b3_m25_core_slash_then_add_bond_same_key_refused() {
    assert_same_block_refused(Kind::AddBond, true);
}

// REQ-B3-003 — Decision: a failure means the block rule depends on tx order, so AddBond(K) listed before Slash(K) still mints orphaned bonds.
#[test]
fn b3_m25_core_add_bond_then_slash_same_key_refused() {
    assert_same_block_refused(Kind::AddBond, false);
}

// REQ-B3-003 — Decision: a failure means an exited K with locked collateral can re-register in the block that slashes it, and the new bond is orphaned.
#[test]
fn b3_m25_core_slash_then_registration_same_key_refused() {
    assert_same_block_refused(Kind::Registration, true);
}

// REQ-B3-003 — Decision: a failure means the Registration arm depends on tx order, so Registration(K) before Slash(K) still passes.
#[test]
fn b3_m25_core_registration_then_slash_same_key_refused() {
    assert_same_block_refused(Kind::Registration, false);
}

// REQ-B3-003 — Decision: a failure means the same-block arm fires below the slashing gate, a consensus change on old blocks without an activation height.
#[test]
fn b3_m25_core_below_gate_verdict_unchanged() {
    fixture_preconditions();
    for kind in [Kind::AddBond, Kind::Registration] {
        let c = ctx(HEIGHT + 1, kind);
        for slash_first in [true, false] {
            let block = ordered(kind, &accused(), slash_first);
            for mode in [ValidationMode::Light, ValidationMode::Replay] {
                let r = validate_block_with_mode(&block, &c, mode);
                assert!(
                    r.is_ok(),
                    "below the gate {mode:?} {kind:?} slash_first={slash_first}: old verdict Ok; got {r:?}"
                );
            }
        }
    }
}

// REQ-B3-003 — Decision: a failure means the arm over-refuses an unrelated producer's Registration or AddBond mined beside a slash.
#[test]
fn b3_m25_core_other_key_accepted() {
    fixture_preconditions();
    for kind in [Kind::AddBond, Kind::Registration] {
        let c = ctx(OPEN_GATE, kind);
        for slash_first in [true, false] {
            let block = ordered(kind, &other(), slash_first);
            for mode in MODES {
                let r = validate_block_with_mode(&block, &c, mode);
                assert!(
                    r.is_ok(),
                    "{mode:?}: Slash(K) beside {kind:?}(K') slash_first={slash_first} must validate; got {r:?}"
                );
            }
        }
    }
}
