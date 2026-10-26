//! INC-I-257 M2 — Bond-output rule (D6) and Bond-input rule (REQ-014),
//! judged by the core validation entries that mempool, builder and apply call.
//!
// covers: validation transaction tx_types output_authority registration unbonding utxo
//
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 return: Ok | Err; pure (no mutable params, no receiver, no store)
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(&Transaction, &ValidationContext, &U) -> Result<(), ValidationError>
//   O1 return: Ok | Err (Bond-input refusal carries "ERRTX-UNBOND"); the provider is read-only
// PATHS: P-TRANSFER Bond output in a Transfer | P-ADDBOND Bond outputs of an AddBond
//        | P-REG Bond outputs of a Registration | P-SPEND Transfer spending a Bond past lock_until
// INPUT PARTITIONS:
//   height vs gate: gate - 1 (below) | gate | gate + 100; gate u64::MAX (never armed)
//   Bond owner: bond_owner(producer) | foreign hash
//   Bond amount: bond_unit | bond_unit - 1 | bond_unit + 1
//   Bond outputs: 1 | 2; Bond lock: finite (shipped CLI shape) | u64::MAX
//   spent input: Bond (lock passed) | Normal
// MATRIX:
//   O1 x P-TRANSFER x height -> inc_i_257_m2_req_unbond_013_transfer_bond_output_refused_at_gate
//   O1 x P-ADDBOND x {owner, amount, 2nd output} x height -> inc_i_257_m2_req_unbond_013_addbond_foreign_or_non_unit_refused
//   O1 x P-ADDBOND well-formed x {1, 2 outputs} x lock -> inc_i_257_m2_req_unbond_013_well_formed_addbond_accepted
//   O1 x P-REG x {owner, amount, 2nd output} x height -> inc_i_257_m2_req_unbond_013_registration_foreign_or_non_unit_refused
//   O1 x P-REG well-formed x {1, 2 outputs} x lock -> inc_i_257_m2_req_unbond_013_well_formed_registration_accepted
//   O1 x P-SPEND x height x spent input -> inc_i_257_m2_req_unbond_014_transfer_spending_bond_past_lock_flips_at_gate
//   O1 x P-ADDBOND undecodable producer data x {gate, gate + 100} -> inc_i_257_m2_req_unbond_013_bond_without_producer_data_refused

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, RegistrationData, Transaction, TxType};
use doli_core::validation::{
    bond_owner, validate_transaction, validate_transaction_with_utxos, UtxoInfo, UtxoProvider,
    ValidationContext, ValidationError,
};
use doli_core::Network;

const GATE: u64 = 200;
const PERIOD: u64 = 60;
const LOCK: u64 = 10_000;
const FEE: u64 = 500_000;
const NEVER: u64 = u64::MAX;

fn producer() -> KeyPair {
    KeyPair::from_seed([0x5D; 32])
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn own() -> Hash {
    bond_owner(producer().public_key())
}

fn foreign() -> Hash {
    Hash::from_bytes([0x99; 32])
}

fn ctx(height: u64, gate: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_inc_i_257_unbonding(gate, PERIOD)
}

fn fixture_preconditions() {
    let p = ConsensusParams::devnet();
    for h in [GATE - 1, GATE, GATE + 100] {
        assert!(
            !Network::Devnet.is_in_genesis(h) && p.bond_amount(h) == unit(),
            "FIXTURE: h={h} is past genesis and in era 0, so bond_unit at h is unambiguous"
        );
        assert!(
            LOCK >= h + p.blocks_per_era,
            "FIXTURE: registration lock floor"
        );
    }
}

fn input() -> Vec<Input> {
    let mut i = Input::new(Hash::from_bytes([0x42; 32]), 0);
    i.public_key = Some(*producer().public_key());
    vec![i]
}

fn bond(amount: u64, owner: Hash) -> Output {
    Output::bond(amount, owner, LOCK, 0)
}

fn add_bond(outputs: Vec<Output>) -> Transaction {
    let n = outputs.len() as u32;
    let mut tx = Transaction::new_add_bond(
        input(),
        *producer().public_key(),
        n,
        unit() * u64::from(n),
        LOCK,
    );
    tx.outputs = outputs;
    tx
}

fn registration(outputs: Vec<Output>) -> Transaction {
    let bls = BlsKeyPair::from_seed(&[0x5E; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let data = RegistrationData {
        public_key: *producer().public_key(),
        epoch: 0,
        vdf_output: Vec::new(),
        vdf_proof: Vec::new(),
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count: outputs.len() as u32,
        bls_pubkey: bls.public_key().as_bytes().to_vec(),
        bls_pop: pop.as_bytes().to_vec(),
    };
    Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: input(),
        outputs,
        extra_data: bincode::serialize(&data).expect("FIXTURE: registration data"),
    }
}

/// Below the gate (and with the gate never armed) the verdict is today's: Ok.
fn assert_unchanged_below(tx: &Transaction, case: &str) {
    for (h, gate) in [(GATE - 1, GATE), (GATE + 100, NEVER)] {
        let r = validate_transaction(tx, &ctx(h, gate));
        assert!(
            r.is_ok(),
            "{case}: H={h} gate={gate} must stay valid: {r:?}"
        );
    }
}

fn assert_refused_at_gate(tx: &Transaction, case: &str) {
    for h in [GATE, GATE + 100] {
        let r = validate_transaction(tx, &ctx(h, GATE));
        assert!(
            r.is_err(),
            "{case}: H={h} at/above the gate must be refused"
        );
    }
}

fn assert_accepted_everywhere(tx: &Transaction, case: &str) {
    assert_unchanged_below(tx, case);
    for h in [GATE, GATE + 100] {
        let r = validate_transaction(tx, &ctx(h, GATE));
        assert!(
            r.is_ok(),
            "{case}: H={h} at/above the gate must stay valid: {r:?}"
        );
    }
}

// REQ-UNBOND-013 — Decision: a failure means a Transfer can mint a Bond-typed output at/above the gate (selection weight or burn-escape value outside Registration/AddBond), or the rule leaks below the gate.
#[test]
fn inc_i_257_m2_req_unbond_013_transfer_bond_output_refused_at_gate() {
    fixture_preconditions();
    let twin = Transaction::new_transfer(input(), vec![Output::normal(unit(), own())]);
    assert_accepted_everywhere(&twin, "NON-VACUITY: the same Transfer paying Normal");
    for (case, owner) in [("own-owner Bond", own()), ("foreign-owner Bond", foreign())] {
        let tx = Transaction::new_transfer(input(), vec![bond(unit(), owner)]);
        assert_unchanged_below(&tx, case);
        assert_refused_at_gate(&tx, case);
    }
}

// REQ-UNBOND-013 — Decision: a failure means an AddBond can create a Bond the slash of its producer cannot find (foreign owner) or that breaks the one-output-one-unit weight (non-unit amount).
#[test]
fn inc_i_257_m2_req_unbond_013_addbond_foreign_or_non_unit_refused() {
    fixture_preconditions();
    let cases = [
        ("foreign owner", vec![bond(unit(), foreign())]),
        ("amount below unit", vec![bond(unit() - 1, own())]),
        ("amount above unit", vec![bond(unit() + 1, own())]),
        (
            "second output foreign",
            vec![bond(unit(), own()), bond(unit(), foreign())],
        ),
    ];
    for (case, outputs) in cases {
        let tx = add_bond(outputs);
        assert_unchanged_below(&tx, case);
        assert_refused_at_gate(&tx, case);
    }
}

// REQ-UNBOND-013 — Decision: a failure means the rule refuses the AddBond the shipped clients build (finite lock, one unit per output), so no producer can add a bond after the gate.
#[test]
fn inc_i_257_m2_req_unbond_013_well_formed_addbond_accepted() {
    fixture_preconditions();
    let max_lock = Output::bond(unit(), own(), u64::MAX, 0);
    let cases = [
        ("one unit, finite lock", vec![bond(unit(), own())]),
        ("two units", vec![bond(unit(), own()), bond(unit(), own())]),
        ("one unit, lock u64::MAX", vec![max_lock]),
    ];
    for (case, outputs) in cases {
        assert_accepted_everywhere(&add_bond(outputs), case);
    }
}

// REQ-UNBOND-013 — Decision: a failure means a Registration can lock its bond under a foreign owner or a non-unit amount, so its selection weight escapes the slash burn.
#[test]
fn inc_i_257_m2_req_unbond_013_registration_foreign_or_non_unit_refused() {
    fixture_preconditions();
    let cases = [
        ("foreign owner", vec![bond(unit(), foreign())]),
        ("amount above unit", vec![bond(unit() + 1, own())]),
        (
            "second output foreign",
            vec![bond(unit(), own()), bond(unit(), foreign())],
        ),
    ];
    for (case, outputs) in cases {
        let tx = registration(outputs);
        assert_unchanged_below(&tx, case);
        assert_refused_at_gate(&tx, case);
    }
}

// REQ-UNBOND-013 — Decision: a failure means the rule refuses the Registration the shipped clients build, so no producer can register after the gate.
#[test]
fn inc_i_257_m2_req_unbond_013_well_formed_registration_accepted() {
    fixture_preconditions();
    let max_lock = Output::bond(unit(), own(), u64::MAX, 0);
    let cases = [
        ("one unit, finite lock", vec![bond(unit(), own())]),
        ("two units", vec![bond(unit(), own()), bond(unit(), own())]),
        ("one unit, lock u64::MAX", vec![max_lock]),
    ];
    for (case, outputs) in cases {
        assert_accepted_everywhere(&registration(outputs), case);
    }
}

// REQ-UNBOND-013 — Decision: a failure means a Bond output whose producer cannot be read escapes the owner check at the placement rule.
#[test]
fn inc_i_257_m2_req_unbond_013_bond_without_producer_data_refused() {
    fixture_preconditions();
    let mut tx = add_bond(vec![bond(unit(), own())]);
    tx.extra_data = vec![0xFF];
    for h in [GATE, GATE + 100] {
        let r = validate_transaction(&tx, &ctx(h, GATE));
        assert!(
            matches!(&r, Err(e) if e.to_string().contains("ERRTX-UNBOND009")),
            "H={h}: an AddBond Bond with undecodable producer data: {r:?}"
        );
    }
}

struct One(Output);

fn spent_hash() -> Hash {
    Hash::from_bytes([0x42; 32])
}

impl UtxoProvider for One {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        (*tx_hash == spent_hash() && output_index == 0).then(|| UtxoInfo {
            output: self.0.clone(),
            pubkey: None,
            spent: false,
        })
    }
}

fn signed_transfer(amount_in: u64) -> Transaction {
    let kp = producer();
    let mut tx =
        Transaction::new_transfer(input(), vec![Output::normal(amount_in - FEE, foreign())]);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

fn with_utxos(spent: Output, h: u64, gate: u64) -> Result<(), ValidationError> {
    let tx = signed_transfer(spent.amount);
    validate_transaction_with_utxos(&tx, &ctx(h, gate), &One(spent))
}

// REQ-UNBOND-014 — Decision: a failure means a Transfer can drain a Bond whose finite lock has passed (a burn escape that bypasses the withdrawal and its lock), or the Bond-input rule leaks below the gate.
#[test]
fn inc_i_257_m2_req_unbond_014_transfer_spending_bond_past_lock_flips_at_gate() {
    fixture_preconditions();
    let passed = Output::bond(unit(), own(), 10, 0);
    let normal = Output::normal(unit(), own());
    for h in [GATE, GATE + 100] {
        let ok = with_utxos(normal.clone(), h, GATE);
        assert!(
            ok.is_ok(),
            "NON-VACUITY H={h}: the same Transfer of a Normal input: {ok:?}"
        );
        let r = with_utxos(passed.clone(), h, GATE);
        assert!(
            matches!(&r, Err(e) if e.to_string().contains("ERRTX-UNBOND")),
            "H={h}: a Transfer spending a Bond past its lock must be refused at/above the gate: {r:?}"
        );
    }
    for (h, gate) in [(GATE - 1, GATE), (GATE + 100, NEVER)] {
        let r = with_utxos(passed.clone(), h, gate);
        assert!(
            r.is_ok(),
            "H={h} gate={gate}: below the gate the spend stays valid: {r:?}"
        );
    }
}
