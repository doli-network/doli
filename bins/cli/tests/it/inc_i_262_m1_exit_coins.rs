//! INC-I-262 M1 — a zero-Bond exit at the unbonding gate spends one coin, the
//! smallest above the fee, and locks only that coin minus the fee; Bond-input
//! exits and the withdrawal command keep their bytes.
//!
// covers: unbonding_tx exit withdrawal producer_tx
//
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::build_withdrawal_tx(&NetworkParams, best_height: u64, &WithdrawalSpec) -> Result<Transaction, String>
//   O1 return Ok: inputs (outpoints, order), outputs (count, type, owner, amount, lock), withdrawal data
//   O2 return Err: message containing "insufficient spendable balance"; no tx
// OUTPUT CONTRACT: fn doli_cli::producer_tx::finalize_producer_tx(&mut Transaction, &KeyPair, &Hash)
//   O3 receiver: serialized bytes (golden hash) after signing
// PATHS: P-ZERO no Bond input, gate open | P-NONE no coin above the fee | P-BOND Bond inputs, gate open
//        | P-BELOW gate closed (Bond inputs, zero Bond) | P-WD withdrawal command spec
// MATRIX
//   O1 x P-ZERO x {set A RPC order, set A reversed} -> inc_i_262_m1_req_def_002_coin_set_a_locks_one_unit
//   O1 x P-ZERO x {set B RPC order 40u then 3u} -> inc_i_262_m1_req_def_002_coin_set_b_spends_smallest_above_fee
//   O2 x P-NONE x {none, one == fee, two == fee} -> inc_i_262_m1_req_def_002_no_coin_above_fee_refused
//   O3 x {P-BOND, P-BELOW, P-WD} -> inc_i_262_m1_req_def_002_golden_bond_exit_and_withdrawal_bytes
// INPUT PARTITIONS:
//   fee 1; UNIT = one DOLI; best 1_000; gate 0 (open) | 2_000 (closed)
//   set A = {fee+1, 5u, 100u} offered {100u, 5u, fee+1}; set B = {3u, 40u} offered {40u, 3u}

use crypto::{Hash, KeyPair, PublicKey};
use doli_cli::producer_tx::finalize_producer_tx;
use doli_cli::unbonding_tx::{build_withdrawal_tx, unbonding_lock_until, FeeUtxo, WithdrawalSpec};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{OutputType, Transaction};
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext};
use doli_core::{Network, NetworkParams};

const BEST: u64 = 1_000;
const CLOSED: u64 = 2_000;
const FEE: u64 = 1;
const UNIT: u64 = 100_000_000;
const BONDS: u32 = 2;

fn wallet() -> KeyPair {
    KeyPair::from_seed([0x62; 32])
}

fn pk() -> PublicKey {
    *wallet().public_key()
}

fn own() -> Hash {
    bond_owner(&pk())
}

fn params(c1: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_257_unbonding_activation_height = c1;
    p
}

fn ctx() -> ValidationContext {
    let p = params(0);
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, BEST + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(0, p.unbonding_period)
        .with_inc_i_182_producer_auth(0)
}

fn coin(tag: u8, amount: u64) -> FeeUtxo {
    FeeUtxo {
        tx_hash: Hash::from_bytes([tag; 32]),
        output_index: 0,
        amount,
    }
}

fn zero_bond(fee_utxos: Vec<FeeUtxo>) -> WithdrawalSpec {
    WithdrawalSpec {
        producer: pk(),
        bond_count: BONDS,
        bond_inputs: Vec::new(),
        total_net: 0,
        destination: own(),
        fee_utxos,
        fee: FEE,
    }
}

fn spent(tx: &Transaction) -> Vec<(Hash, u32)> {
    tx.inputs
        .iter()
        .map(|i| (i.prev_tx_hash, i.output_index))
        .collect()
}

fn signed(mut tx: Transaction) -> Transaction {
    finalize_producer_tx(&mut tx, &wallet(), &ConsensusParams::devnet().genesis_hash);
    tx
}

/// (spent outpoints, output count, output type, owner, locked amount, lock height)
type Shape = (Vec<(Hash, u32)>, usize, OutputType, Hash, u64, u64);

fn shape(tx: &Transaction) -> Shape {
    let o = &tx.outputs[0];
    (
        spent(tx),
        tx.outputs.len(),
        o.output_type,
        o.pubkey_hash,
        o.amount,
        o.lock_until,
    )
}

fn want(tag: u8, locked: u64) -> Shape {
    (
        vec![(Hash::from_bytes([tag; 32]), 0)],
        1,
        OutputType::Unbonding,
        own(),
        locked,
        unbonding_lock_until(&params(0), BEST),
    )
}

// REQ-DEF-002 — Decision: a failure means a zero-Bond exit locks a large coin for the unbonding period when a coin of fee+1 would do.
#[test]
fn inc_i_262_m1_req_def_002_coin_set_a_locks_one_unit() {
    let rpc_order = vec![
        coin(0xA3, 100 * UNIT),
        coin(0xA2, 5 * UNIT),
        coin(0xA1, FEE + 1),
    ];
    let mut reversed = rpc_order.clone();
    reversed.reverse();
    for (what, offered) in [("RPC order", rpc_order), ("reversed", reversed)] {
        let tx = build_withdrawal_tx(&params(0), BEST, &zero_bond(offered))
            .unwrap_or_else(|e| panic!("set A {what}: {e}"));
        let got = shape(&tx);
        println!("INC_I_262_SET_A_{what}: locked={} spent={:?}", got.4, got.0);
        assert_eq!(
            got,
            want(0xA1, 1),
            "set A {what}: only the fee+1 coin is spent and exactly 1 is locked"
        );
        let r = validate_transaction(&signed(tx), &ctx());
        assert!(r.is_ok(), "set A {what}: the one-coin exit is valid: {r:?}");
    }
}

// REQ-DEF-002 — Decision: a failure means with no exact fee+1 coin the exit locks a coin larger than the smallest one above the fee.
#[test]
fn inc_i_262_m1_req_def_002_coin_set_b_spends_smallest_above_fee() {
    let offered = vec![coin(0xB4, 40 * UNIT), coin(0xB3, 3 * UNIT)];
    let tx = build_withdrawal_tx(&params(0), BEST, &zero_bond(offered)).expect("set B builds");
    let got = shape(&tx);
    println!("INC_I_262_SET_B: locked={} spent={:?}", got.4, got.0);
    assert_eq!(
        got,
        want(0xB3, 3 * UNIT - FEE),
        "set B: the 3-unit coin is spent and 3 units minus the fee are locked"
    );
    let r = validate_transaction(&signed(tx), &ctx());
    assert!(r.is_ok(), "set B: valid: {r:?}");
}

// REQ-DEF-002 — Decision: a failure means the exit adds coins that each do not exceed the fee, or signs with no coin able to pay a positive payout, instead of reporting the shortfall.
#[test]
fn inc_i_262_m1_req_def_002_no_coin_above_fee_refused() {
    let cases = [
        ("no coin", Vec::new()),
        ("one coin equal to the fee", vec![coin(0xC1, FEE)]),
        (
            "two coins equal to the fee",
            vec![coin(0xC1, FEE), coin(0xC2, FEE)],
        ),
    ];
    for (what, offered) in cases {
        let r = build_withdrawal_tx(&params(0), BEST, &zero_bond(offered));
        assert!(
            matches!(&r, Err(m) if m.contains("insufficient spendable balance")),
            "{what}: want Err(insufficient spendable balance), nothing built; got {:?}",
            r.map(|t| shape(&t))
        );
    }
}

fn golden(tx: Transaction) -> String {
    crypto::hash::hash(&signed(tx).serialize()).to_hex()
}

fn bond_inputs(n: u8) -> Vec<(Hash, u32)> {
    (0..n)
        .map(|i| (Hash::from_bytes([0xD0 + i; 32]), u32::from(i)))
        .collect()
}

fn spec(
    bonds: Vec<(Hash, u32)>,
    count: u32,
    net: u64,
    dest: Hash,
    fee_utxos: Vec<FeeUtxo>,
) -> WithdrawalSpec {
    WithdrawalSpec {
        producer: pk(),
        bond_count: count,
        bond_inputs: bonds,
        total_net: net,
        destination: dest,
        fee_utxos,
        fee: FEE,
    }
}

const GOLDEN: [(&str, &str); 6] = [
    (
        "exit, Bond inputs, gate open",
        "2b32e5aac9f59a2afaa18af0c58a3eb350fed7c36faf627e2e0d83d1a4d0c388",
    ),
    (
        "exit, Bond inputs, gate closed",
        "ff05ba69e89f26acdfce0da8d089e9dd23062dd8d6b9186037488cc137f065c5",
    ),
    (
        "exit, zero Bond, gate closed",
        "0ad5310fc2ae76eb22c397140ea5e1515d44a6b0d0fb9cc48b3fdf57072caa50",
    ),
    (
        "withdraw, gate open",
        "659f10d706296afadad4274062f498df4488acda8a934f7d9d33802ef88c3407",
    ),
    (
        "withdraw, gate closed, foreign destination",
        "018524581e2b4bee3f4a8ad5ddafcfd49aa9d8aa17b55fbce3cf7cac2a357bf5",
    ),
    (
        "withdraw, gate closed, own destination",
        "433e7dca8e5485ac2c989fed6989d7eab980fc92cf70625587ed74abc71a1e33",
    ),
];

fn golden_cases() -> Vec<(&'static str, String)> {
    let unit = Network::Devnet.bond_unit();
    let foreign = Hash::from_bytes([0xEE; 32]);
    let fees = || vec![coin(0xF1, 5_000), coin(0xF2, 9_000)];
    let build = |c1: u64, s: WithdrawalSpec| {
        golden(build_withdrawal_tx(&params(c1), BEST, &s).expect("golden build"))
    };
    vec![
        (
            "exit, Bond inputs, gate open",
            build(0, spec(bond_inputs(2), BONDS, 2 * unit, own(), fees())),
        ),
        (
            "exit, Bond inputs, gate closed",
            build(CLOSED, spec(bond_inputs(2), BONDS, 2 * unit, own(), fees())),
        ),
        (
            "exit, zero Bond, gate closed",
            build(CLOSED, spec(Vec::new(), BONDS, 0, own(), fees())),
        ),
        (
            "withdraw, gate open",
            build(0, spec(bond_inputs(1), 1, unit / 2, own(), fees())),
        ),
        (
            "withdraw, gate closed, foreign destination",
            build(CLOSED, spec(bond_inputs(1), 1, unit / 2, foreign, fees())),
        ),
        (
            "withdraw, gate closed, own destination",
            build(CLOSED, spec(bond_inputs(1), 1, unit, own(), fees())),
        ),
    ]
}

// REQ-DEF-002 — Decision: a failure means the coin-choice fix changed the bytes of an exit with Bond inputs, a below-gate exit or a withdrawal, which REQ-DEF-002 keeps byte-equal.
#[test]
fn inc_i_262_m1_req_def_002_golden_bond_exit_and_withdrawal_bytes() {
    let got = golden_cases();
    for (name, h) in &got {
        println!("GOLDEN_OBSERVED {name}: {h}");
    }
    let drift: Vec<String> = got
        .iter()
        .zip(GOLDEN.iter())
        .filter(|((_, h), (_, w))| h != w)
        .map(|((n, h), (_, w))| format!("{n}: got {h}, golden {w}"))
        .collect();
    assert!(
        drift.is_empty(),
        "golden bytes drifted:\n{}",
        drift.join("\n")
    );
}
