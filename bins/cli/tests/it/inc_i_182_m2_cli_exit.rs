//! INC-I-182 M2 — the CLI builds the producer's own full exit when the wallet
//! owns no live Bond UTXO: zero Bond inputs, fee and payout from Normal
//! UTXOs, one Unbonding payout to the producer, signed by the producer key;
//! and `exit` routes every exit through that builder.
//!
// covers: exit unbonding_tx producer_tx producer_ledger withdrawal common rpc_client lib
//
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::build_withdrawal_tx(&NetworkParams, best_height: u64, &WithdrawalSpec) -> Result<Transaction, String>
//   O1 return, pure. At/above the C1 gate with NO Bond inputs: one input, the smallest offered coin above
//      `fee` (INC-I-262); ONE output: Unbonding, owner bond_owner(producer),
//      amount total_net + that coin - fee, lock unbonding_lock_until(params, best); withdrawal data
//      (producer, bond_count, own address). Err containing "insufficient spendable balance" when the
//      offered total does not exceed `fee`. With Bond inputs, and below the gate: unchanged.
// OUTPUT CONTRACT: fn doli_cli::producer_tx::finalize_producer_tx(&mut Transaction, &KeyPair, &Hash)
//   O2 receiver: every input (fee inputs included) signed by the wallet key, then the producer-auth tail
// OUTPUT CONTRACT: source of bins/cli/src/cmd_producer/exit.rs (binary-only), fn handle_exit
//   O3 builds through build_withdrawal_tx + finalize_producer_tx; no Transaction::new_request_withdrawal;
//      never reads the UTXO-derived producer_info.bond_count; no Bond-UTXO shortfall bail;
//      the ledger-mismatch refusal stays for partial mismatches
// PATHS: P-ZERO no Bond inputs, C1 armed | P-FULL every Bond input, C1 armed | P-BELOW C1 not armed
//        | P-ERR offered fee UTXOs do not exceed the fee
// MATRIX
//   O1, O2 x P-ZERO x {one UTXO, first UTXO == fee (skipped)} -> ..._zero_bond_exit_built_and_valid_above_both_gates
//   O1 x P-ERR x {none, one == fee} -> ..._zero_bond_exit_insufficient_fee_utxos_refused (+ consensus refuses a 0 payout)
//   O1, O2 x P-FULL -> ..._full_exit_with_bonds_unbonding_form_valid
//   O1, O2 x P-BELOW x {bonds, zero bonds} -> ..._below_c1_gate_normal_form_still_valid
//   O3 -> ..._exit_command_routes_through_lib_builder
//   O1, O2 x P-ZERO -> inc_i_182_m2_cli_outcome_probe (PAUTH_SELF_EXIT_CLI_PATHS)
// INPUT PARTITIONS:
//   best height 1_000; C1 gate 0 (armed) | 2_000 (not armed); producer-auth gate 0 | off
//   fee UTXOs offered: none | one 5_000 | 1 then 7_000; fee 1; declared bonds 2

use crypto::{Hash, KeyPair, PublicKey};
use doli_cli::producer_tx::finalize_producer_tx;
use doli_cli::unbonding_tx::{build_withdrawal_tx, unbonding_lock_until, FeeUtxo, WithdrawalSpec};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{sign_producer_auth, Input, OutputType, Transaction, TxType};
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext, ValidationError};
use doli_core::{Network, NetworkParams};

const PAUTH: &str = "[ERRTX-PAUTH001]";
const BEST: u64 = 1_000;
const C1_LATER: u64 = 2_000;
const FEE: u64 = 1;
const BONDS: u32 = 2;
const METRIC: &str = "PAUTH_SELF_EXIT_CLI_PATHS=";

fn wallet() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

fn pk() -> PublicKey {
    *wallet().public_key()
}

fn own() -> Hash {
    bond_owner(&pk())
}

fn genesis() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

fn params(c1: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_257_unbonding_activation_height = c1;
    p
}

fn ctx(c1: u64, pauth: u64) -> ValidationContext {
    let p = params(c1);
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, BEST + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(p.inc_i_257_unbonding_activation_height, p.unbonding_period)
        .with_inc_i_182_producer_auth(pauth)
}

fn utxo(tag: u8, output_index: u32, amount: u64) -> FeeUtxo {
    FeeUtxo {
        tx_hash: Hash::from_bytes([tag; 32]),
        output_index,
        amount,
    }
}

fn bond_inputs() -> Vec<(Hash, u32)> {
    vec![
        (Hash::from_bytes([0xB1; 32]), 0),
        (Hash::from_bytes([0xB2; 32]), 1),
    ]
}

fn spec(bonds: Vec<(Hash, u32)>, total_net: u64, fee_utxos: Vec<FeeUtxo>) -> WithdrawalSpec {
    WithdrawalSpec {
        producer: pk(),
        bond_count: BONDS,
        bond_inputs: bonds,
        total_net,
        destination: own(),
        fee_utxos,
        fee: FEE,
    }
}

fn outpoints(tx: &Transaction) -> Vec<(Hash, u32)> {
    tx.inputs
        .iter()
        .map(|i| (i.prev_tx_hash, i.output_index))
        .collect()
}

fn finalized(mut tx: Transaction) -> Transaction {
    finalize_producer_tx(&mut tx, &wallet(), &genesis());
    tx
}

fn stripped(tx: &Transaction) -> Transaction {
    let mut t = tx.clone();
    t.extra_data.truncate(tx.extra_data.len() - 64);
    t
}

fn coded(r: &Result<(), ValidationError>, code: &str) -> bool {
    matches!(r, Err(ValidationError::InvalidTransaction(m)) if m.contains(code))
}

/// Valid above both gates and refused with the code once the auth tail is gone.
fn assert_authorized_and_valid(tx: &Transaction, what: &str) {
    let r = validate_transaction(tx, &ctx(0, 0));
    assert!(r.is_ok(), "{what}: refused above both gates: {r:?}");
    assert!(
        coded(&validate_transaction(&stripped(tx), &ctx(0, 0)), PAUTH),
        "{what}: without its auth tail it must be refused with {PAUTH}"
    );
}

/// The CLI's zero-Bond self-exit, built, finalized and validated above both gates.
fn cli_self_exit() -> Result<Transaction, String> {
    let offered = vec![utxo(0xF1, 1, 5_000)];
    let mut tx = build_withdrawal_tx(&params(0), BEST, &spec(Vec::new(), 0, offered))?;
    finalize_producer_tx(&mut tx, &wallet(), &genesis());
    validate_transaction(&tx, &ctx(0, 0)).map_err(|e| format!("{e:?}"))?;
    Ok(tx)
}

// REQ-PAUTH-004 — Decision: a failure means the CLI cannot build (or builds an invalid) exit for a producer whose wallet holds no live Bond UTXO, so that producer has no client path out of the set.
#[test]
fn inc_i_182_m2_req_pauth_004_zero_bond_exit_built_and_valid_above_both_gates() {
    let p = params(0);
    let offered = vec![utxo(0xF1, 1, 5_000), utxo(0xF2, 0, 7_000)];
    let tx = build_withdrawal_tx(&p, BEST, &spec(Vec::new(), 0, offered))
        .expect("zero-Bond exit must build above the C1 gate");
    assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
    assert_eq!(
        outpoints(&tx),
        vec![(Hash::from_bytes([0xF1; 32]), 1)],
        "only the first fee UTXO: it alone exceeds the fee"
    );
    assert_eq!(tx.outputs.len(), 1, "exactly one output");
    let out = &tx.outputs[0];
    assert_eq!(
        (out.output_type, out.pubkey_hash, out.amount, out.lock_until),
        (
            OutputType::Unbonding,
            own(),
            5_000 - FEE,
            unbonding_lock_until(&p, BEST)
        ),
        "one locked Unbonding payout to the producer of the fee input minus the fee"
    );
    assert!(out.extra_data.is_empty(), "payout carries no extra data");
    let data = tx.withdrawal_request_data().expect("withdrawal data");
    assert_eq!(
        (data.producer_pubkey, data.bond_count, data.destination),
        (pk(), BONDS, own()),
        "withdrawal data declares the full allowance to the producer's address"
    );

    let got = finalized(tx.clone());
    let mut want = tx;
    for i in 0..want.inputs.len() {
        want.inputs[i].public_key = Some(pk());
        let msg = want.signing_message_for_input(i);
        want.inputs[i].signature = crypto::signature::sign_hash(&msg, wallet().private_key());
    }
    sign_producer_auth(&mut want, &genesis(), &wallet());
    assert_eq!(
        got, want,
        "finalize = wallet key on every input + auth tail"
    );
    assert_authorized_and_valid(&got, "zero-Bond exit, one fee UTXO");

    let offered = vec![utxo(0xF1, 1, FEE), utxo(0xF2, 0, 7_000)];
    let tx = build_withdrawal_tx(&p, BEST, &spec(Vec::new(), 0, offered))
        .expect("first UTXO equal to the fee: the next coin above the fee is chosen");
    assert_eq!(
        outpoints(&tx),
        vec![(Hash::from_bytes([0xF2; 32]), 0)],
        "a UTXO equal to the fee is never spent; only the coin above the fee"
    );
    let paid: u64 = tx.outputs.iter().map(|o| o.amount).sum();
    assert_eq!(paid + FEE, 7_000, "outputs + fee == the one input");
    assert_authorized_and_valid(&finalized(tx), "zero-Bond exit, coin above the fee");
}

// REQ-PAUTH-004 — Decision: a failure means the CLI emits a zero-value payout (refused by every node) or signs without enough funds instead of telling the operator.
#[test]
fn inc_i_182_m2_req_pauth_004_zero_bond_exit_insufficient_fee_utxos_refused() {
    let mut zero = Transaction::new_request_withdrawal(
        vec![Input::new(Hash::from_bytes([0xF1; 32]), 1)],
        pk(),
        BONDS,
        own(),
        0,
    );
    zero.outputs[0].output_type = OutputType::Unbonding;
    zero.outputs[0].lock_until = unbonding_lock_until(&params(0), BEST);
    let r = validate_transaction(&finalized(zero), &ctx(0, 0));
    assert!(
        coded(&r, "[ERRTX003]"),
        "consensus refuses a zero-value payout with [ERRTX003]; got {r:?}"
    );

    for offered in [Vec::new(), vec![utxo(0xF1, 1, FEE)]] {
        let n = offered.len();
        let r = build_withdrawal_tx(&params(0), BEST, &spec(Vec::new(), 0, offered));
        assert!(
            matches!(&r, Err(m) if m.contains("insufficient spendable balance")),
            "{n} fee UTXO(s) not exceeding the fee: want Err(insufficient spendable balance); got {r:?}"
        );
    }
}

// REQ-PAUTH-004 — Decision: a failure means the normal full exit (all Bond inputs) the exit command will route through the lib builder is not the valid Unbonding form above both gates.
#[test]
fn inc_i_182_m2_req_pauth_004_full_exit_with_bonds_unbonding_form_valid() {
    let unit = Network::Devnet.bond_unit();
    let net = unit * u64::from(BONDS);
    let tx = build_withdrawal_tx(
        &params(0),
        BEST,
        &spec(bond_inputs(), net, vec![utxo(0xF1, 1, 5_000)]),
    )
    .expect("full exit builds above the C1 gate");
    assert_eq!(
        outpoints(&tx),
        bond_inputs(),
        "Bond inputs only, no wallet UTXO"
    );
    let out = &tx.outputs[0];
    assert_eq!(
        (
            tx.outputs.len(),
            out.output_type,
            out.pubkey_hash,
            out.amount
        ),
        (1, OutputType::Unbonding, own(), net - FEE),
        "one Unbonding payout of the withdrawn amount minus the fee"
    );
    assert_authorized_and_valid(&finalized(tx), "full exit with Bond inputs");
}

// REQ-PAUTH-004 — Decision: a failure means the CLI changes the exit shape on a network where C1 is not armed, or that shape fails the producer-auth rule.
#[test]
fn inc_i_182_m2_req_pauth_004_below_c1_gate_normal_form_still_valid() {
    let p = params(C1_LATER);
    let unit = Network::Devnet.bond_unit();
    let cases = [
        ("with Bond inputs", bond_inputs(), unit * u64::from(BONDS)),
        ("zero Bond inputs", Vec::new(), 0),
    ];
    for (what, bonds, net) in cases {
        let tx = build_withdrawal_tx(
            &p,
            BEST,
            &spec(bonds.clone(), net, vec![utxo(0xF1, 1, 5_000)]),
        )
        .unwrap_or_else(|e| panic!("{what}: below-gate build: {e}"));
        let mut want_inputs = bonds;
        want_inputs.push((Hash::from_bytes([0xF1; 32]), 1));
        assert_eq!(
            outpoints(&tx),
            want_inputs,
            "{what}: Bond inputs then the fee UTXO"
        );
        let out = &tx.outputs[0];
        assert_eq!(
            (
                tx.outputs.len(),
                out.output_type,
                out.lock_until,
                out.amount
            ),
            (1, OutputType::Normal, 0, net + 5_000 - FEE),
            "{what}: today's Normal payout, fee change rolled in"
        );
        let signed = finalized(tx);
        let r = validate_transaction(&signed, &ctx(C1_LATER, 0));
        assert!(r.is_ok(), "{what}: below the C1 gate, signed: {r:?}");
        assert!(
            coded(
                &validate_transaction(&stripped(&signed), &ctx(C1_LATER, 0)),
                PAUTH
            ),
            "{what}: below the C1 gate the auth tail is still required"
        );
    }
}

const EXIT_SRC: &str = include_str!("../../src/cmd_producer/exit.rs");

fn handle_exit_body(src: &str) -> &str {
    let start = src
        .find("async fn handle_exit")
        .expect("handle_exit in exit.rs");
    let end = src[start..]
        .find("async fn handle_slash")
        .map_or(src.len(), |e| start + e);
    &src[start..end]
}

fn exit_violations(body: &str) -> Vec<&'static str> {
    [
        (
            "builds through build_withdrawal_tx",
            body.contains("build_withdrawal_tx("),
        ),
        (
            "signs with finalize_producer_tx",
            body.contains("finalize_producer_tx("),
        ),
        (
            "no hand-built withdrawal",
            !body.contains("new_request_withdrawal"),
        ),
        (
            "never reads the UTXO-derived bondCount",
            !body.contains("producer_info.bond_count"),
        ),
        (
            "no Bond-UTXO shortfall bail",
            !body.contains("Insufficient Bond UTXOs"),
        ),
        (
            "partial ledger mismatch still refused",
            body.contains("Ledger mismatch"),
        ),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(what, _)| what)
    .collect()
}

// REQ-PAUTH-004 — Decision: a failure names the part of `doli producer exit` that still stops a producer with no live Bond UTXO, or still emits a payout consensus refuses above C1.
#[test]
fn inc_i_182_m2_req_pauth_004_exit_command_routes_through_lib_builder() {
    let old = "async fn handle_exit() { let bond_count = producer_info.bond_count; \
               bail!(\"Ledger mismatch\"); bail!(\"Insufficient Bond UTXOs\"); \
               Transaction::new_request_withdrawal(); finalize_producer_tx(&mut tx); }";
    assert_eq!(
        exit_violations(old).len(),
        4,
        "NON-VACUITY: the scan flags each blocker in the old shape"
    );
    let bad = exit_violations(handle_exit_body(EXIT_SRC));
    assert!(bad.is_empty(), "exit.rs handle_exit violates: {bad:?}");
}

// REQ-PAUTH-004 — Decision: the outcome metric; 0 means the CLI still cannot build a valid zero-Bond self-exit above both gates.
#[test]
fn inc_i_182_m2_cli_outcome_probe() {
    let r = cli_self_exit();
    let n = u8::from(r.is_ok());
    println!("{METRIC}{n}");
    assert!(n == 1, "CLI zero-Bond self-exit: {r:?}");
}
