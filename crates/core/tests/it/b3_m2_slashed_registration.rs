//! B3 M2 — a Registration or AddBond whose key is Slashed or slash-queued is
//! refused by the core validation entries from the slashing gate on.
//!
// covers: registration transaction slash_state slash_registration slash_evidence types error mod error-codes
//
// OUTPUT CONTRACT: fn validate_not_slashed_registrant(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   (new, crates/core/src/validation/slash_registration.rs, re-exported from doli_core::validation)
//   O3 return: Err whose Display carries "[ERRTX071]" when slash_rule_active(ctx) and the Registration
//      key / AddBond producer key is `slashed` or `slash_queued` in ctx.slash_targets; Ok otherwise; pure
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O3 return: the predicate's Err, else the old verdict (Ok for these fixtures)
// OUTPUT CONTRACT: fn validate_transaction_skip_registration_vdf(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O3 return: the same verdict as validate_transaction (block Full mode with heavy VDF txs)
// No O1/O2/O4: both entries take shared references and write no store.
// PATHS: P-REFUSE slashed or queued at/above gate | P-BELOW below gate, or gate never armed
//        | P-CLEAN active or absent key at/above gate
// INPUT PARTITIONS:
//   target state: slashed | slash_queued | active | absent from slash_targets
//   height: GATE - 1 | GATE | GATE + 100; gate u64::MAX
//   tx: Registration | AddBond (inputs signed by a funder, producer named in the payload)
//   entry: validate_transaction | validate_transaction_skip_registration_vdf
// MATRIX:
//   O3 x P-REFUSE x {slashed, queued} x {GATE, GATE+100} x Registration x 2 entries -> b3_m2_req_b3_003_slashed_or_queued_registration_refused_at_gate
//   O3 x P-REFUSE x {slashed, queued} x {GATE, GATE+100} x AddBond -> b3_m2_req_b3_003_slashed_or_queued_add_bond_refused_at_gate
//   O3 x P-BELOW x {slashed, queued} x {GATE-1, MAX} x both types x 2 entries -> b3_m2_req_b3_003_below_gate_verdict_unchanged
//   O3 x P-CLEAN x {active, absent} x {GATE, GATE+100} x both types x 2 entries -> b3_m2_req_b3_003_clean_key_verdict_unchanged
//   O3 x P-CLEAN x another key slashed -> b3_m2_req_b3_003_refusal_reads_the_tx_key_only

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, RegistrationData, Transaction, TxType};
use doli_core::validation::{
    bond_owner, validate_transaction, validate_transaction_skip_registration_vdf, SlashTargetState,
    ValidationContext, ValidationError,
};
use doli_core::Network;

const CODE: &str = "ERRTX071";
const GATE: u64 = 200;
const LOCK: u64 = 10_000;
const NEVER: u64 = u64::MAX;

type Entry = fn(&Transaction, &ValidationContext) -> Result<(), ValidationError>;

const ENTRIES: [(&str, Entry); 2] = [
    ("validate_transaction", validate_transaction),
    (
        "validate_transaction_skip_registration_vdf",
        validate_transaction_skip_registration_vdf,
    ),
];

fn producer() -> KeyPair {
    KeyPair::from_seed([0xB3; 32])
}

fn funder() -> KeyPair {
    KeyPair::from_seed([0xB4; 32])
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn fixture_preconditions() {
    let p = ConsensusParams::devnet();
    for h in [GATE - 1, GATE, GATE + 100] {
        assert!(
            !Network::Devnet.is_in_genesis(h) && p.bond_amount(h) == unit(),
            "FIXTURE: h={h} is past genesis and in era 0"
        );
        assert!(LOCK >= h + p.blocks_per_era, "FIXTURE: bond lock floor");
    }
}

fn input(kp: &KeyPair) -> Vec<Input> {
    let mut i = Input::new(Hash::from_bytes([0x42; 32]), 0);
    i.public_key = Some(*kp.public_key());
    vec![i]
}

fn bond() -> Output {
    Output::bond(unit(), bond_owner(producer().public_key()), LOCK, 0)
}

fn registration() -> Transaction {
    let bls = BlsKeyPair::from_seed(&[0xB5; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let data = RegistrationData {
        public_key: *producer().public_key(),
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
        inputs: input(&producer()),
        outputs: vec![bond()],
        extra_data: bincode::serialize(&data).expect("FIXTURE: registration data"),
    }
}

fn add_bond() -> Transaction {
    let mut tx =
        Transaction::new_add_bond(input(&funder()), *producer().public_key(), 1, unit(), LOCK);
    tx.outputs = vec![bond()];
    tx
}

fn slashed() -> SlashTargetState {
    SlashTargetState {
        slashed: true,
        registered_at: 1,
        ..Default::default()
    }
}

fn queued() -> SlashTargetState {
    SlashTargetState {
        active: true,
        slash_queued: true,
        registered_at: 1,
        ..Default::default()
    }
}

fn active() -> SlashTargetState {
    SlashTargetState {
        active: true,
        registered_at: 1,
        ..Default::default()
    }
}

fn ctx(height: u64, gate: u64, targets: Vec<(PublicKey, SlashTargetState)>) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_inc_i_236_block_signature_activation_height(0)
        .with_inc_i_255_slashing_activation_height(gate)
        .with_slash_targets(targets)
}

fn of(state: SlashTargetState) -> Vec<(PublicKey, SlashTargetState)> {
    vec![(*producer().public_key(), state)]
}

fn coded(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(e) if format!("{e} {e:?}").contains(CODE))
}

fn cases() -> [(&'static str, Transaction); 2] {
    [("Registration", registration()), ("AddBond", add_bond())]
}

/// Every cell of `states` x `heights` under every entry gives `want_coded`
/// (refused with CODE) or Ok.
fn assert_cells(
    tx: &Transaction,
    kind: &str,
    states: &[(&str, Vec<(PublicKey, SlashTargetState)>)],
    heights: &[(u64, u64)],
    want_coded: bool,
) {
    for (state_name, targets) in states {
        for &(h, gate) in heights {
            for (entry_name, entry) in ENTRIES {
                let r = entry(tx, &ctx(h, gate, targets.clone()));
                if want_coded {
                    assert!(
                        coded(&r),
                        "{kind} with a {state_name} key at H={h} gate={gate} via {entry_name} must be refused with {CODE}: {r:?}"
                    );
                } else {
                    assert!(
                        r.is_ok(),
                        "{kind} with a {state_name} key at H={h} gate={gate} via {entry_name} must keep the old verdict Ok: {r:?}"
                    );
                }
            }
        }
    }
}

// REQ-B3-003 — Decision: a failure means a Registration for a Slashed or slash-queued key passes core validation at/above the gate, so mempool, builder, apply and replay accept it and storage orphans its Bond UTXOs at the boundary.
#[test]
fn b3_m2_req_b3_003_slashed_or_queued_registration_refused_at_gate() {
    fixture_preconditions();
    let states = [("Slashed", of(slashed())), ("slash-queued", of(queued()))];
    let heights = [(GATE, GATE), (GATE + 100, GATE)];
    assert_cells(&registration(), "Registration", &states, &heights, true);
}

// REQ-B3-003 — Decision: a failure means an AddBond naming a Slashed or slash-queued producer passes core validation at/above the gate, so its new Bond UTXOs are mined and orphaned.
#[test]
fn b3_m2_req_b3_003_slashed_or_queued_add_bond_refused_at_gate() {
    fixture_preconditions();
    let states = [("Slashed", of(slashed())), ("slash-queued", of(queued()))];
    let heights = [(GATE, GATE), (GATE + 100, GATE)];
    let r = validate_transaction(&add_bond(), &ctx(GATE, GATE, of(slashed())));
    let reg = validate_transaction(&registration(), &ctx(GATE, GATE, of(slashed())));
    if let (Err(a), Err(b)) = (&r, &reg) {
        assert_eq!(
            a.error_code(),
            b.error_code(),
            "Registration and AddBond must share ONE error variant"
        );
    }
    assert_cells(&add_bond(), "AddBond", &states, &heights, true);
}

// REQ-B3-003 — Decision: a failure means the new refusal leaks below the slashing gate (or with the gate never armed), changing consensus on blocks already mined without an activation height.
#[test]
fn b3_m2_req_b3_003_below_gate_verdict_unchanged() {
    fixture_preconditions();
    let states = [("Slashed", of(slashed())), ("slash-queued", of(queued()))];
    let heights = [(GATE - 1, GATE), (GATE + 100, NEVER)];
    for (kind, tx) in cases() {
        assert_cells(&tx, kind, &states, &heights, false);
    }
}

// REQ-B3-003 — Decision: a failure means the rule refuses an active or never-slashed key, so no producer can register or add a bond after the gate.
#[test]
fn b3_m2_req_b3_003_clean_key_verdict_unchanged() {
    fixture_preconditions();
    let states = [("active", of(active())), ("absent", Vec::new())];
    let heights = [(GATE, GATE), (GATE + 100, GATE)];
    for (kind, tx) in cases() {
        assert_cells(&tx, kind, &states, &heights, false);
    }
}

// REQ-B3-003 — Decision: a failure means the refusal keys on some other slashed producer in the context (or on the AddBond funder), not on the key the tx registers or bonds.
#[test]
fn b3_m2_req_b3_003_refusal_reads_the_tx_key_only() {
    fixture_preconditions();
    let other = *KeyPair::from_seed([0xB6; 32]).public_key();
    let funder_slashed = vec![(*funder().public_key(), slashed())];
    let states = [
        ("clean (another key Slashed)", vec![(other, slashed())]),
        ("clean (AddBond funder Slashed)", funder_slashed),
    ];
    let heights = [(GATE, GATE)];
    for (kind, tx) in cases() {
        assert_cells(&tx, kind, &states, &heights, false);
    }
    assert_cells(
        &registration(),
        "Registration",
        &[("Slashed", of(slashed()))],
        &heights,
        true,
    );
}
