//! INC-I-262 M1 — the exit summary is one pure function: a zero-Bond exit shows
//! the locked amount, the lock height and what stays spendable; a Bond-input
//! exit prints today's lines byte for byte.
//!
// covers: unbonding_tx exit
//
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::exit_summary_lines(&NetworkParams, best_height: u64, &WithdrawalSpec, &Transaction, total_penalty: u64) -> Vec<String>
//   O1 return, pure. Bond inputs (any gate) or gate closed: today's lines in order —
//      "You receive: {net} DOLI"; "Penalty burned: {p} DOLI" [+ "WARNING: High penalty — {pct}% of bond value will be burned." when pct >= 50]
//      or "No penalty — all bonds fully vested."; gate open: "Unbonding: the payout is locked until height {h} (unbonding period {n} blocks)."
//      Zero Bond inputs, gate open: a line with the locked amount (tx output) and the lock height, a line with the amount that
//      stays spendable (offered coins minus spent coins); never "You receive: 0.00000000 DOLI".
// PATHS: P-ZERO zero Bond, gate open | P-BOND Bond inputs, gate open | P-CLOSED Bond inputs, gate closed
// MATRIX
//   O1 x P-ZERO x {set A, set B} -> inc_i_262_m1_req_def_002_zero_bond_summary_shows_locked_and_spendable
//   O1 x {P-BOND, P-CLOSED} x {no penalty, 25%, 75%} -> inc_i_262_m1_req_def_002_golden_bond_exit_summary_text
// INPUT PARTITIONS: fee 1; UNIT one DOLI; best 1_000; devnet period 60 (lock 1070); bond unit 1 DOLI

use crypto::{Hash, KeyPair, PublicKey};
use doli_cli::unbonding_tx::{
    build_withdrawal_tx, exit_summary_lines, unbonding_lock_until, FeeUtxo, WithdrawalSpec,
};
use doli_core::validation::bond_owner;
use doli_core::{Network, NetworkParams};

const BEST: u64 = 1_000;
const CLOSED: u64 = 2_000;
const FEE: u64 = 1;
const UNIT: u64 = 100_000_000;

fn pk() -> PublicKey {
    *KeyPair::from_seed([0x62; 32]).public_key()
}

fn params(c1: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_257_unbonding_activation_height = c1;
    p
}

fn coin(tag: u8, amount: u64) -> FeeUtxo {
    FeeUtxo {
        tx_hash: Hash::from_bytes([tag; 32]),
        output_index: 0,
        amount,
    }
}

fn spec(bonds: Vec<(Hash, u32)>, net: u64, fee_utxos: Vec<FeeUtxo>) -> WithdrawalSpec {
    WithdrawalSpec {
        producer: pk(),
        bond_count: 2,
        bond_inputs: bonds,
        total_net: net,
        destination: bond_owner(&pk()),
        fee_utxos,
        fee: FEE,
    }
}

fn doli(units: u64) -> String {
    format!("{}.{:08} DOLI", units / UNIT, units % UNIT)
}

fn summary(c1: u64, s: &WithdrawalSpec, penalty: u64) -> Vec<String> {
    let p = params(c1);
    let tx = build_withdrawal_tx(&p, BEST, s).expect("summary build");
    exit_summary_lines(&p, BEST, s, &tx, penalty)
}

// REQ-DEF-002 — Decision: a failure means a producer exiting with no bonds is told it receives 0 while coins are locked, or is not told how much is locked, until when, and what stays spendable.
#[test]
fn inc_i_262_m1_req_def_002_zero_bond_summary_shows_locked_and_spendable() {
    let lock = unbonding_lock_until(&params(0), BEST);
    let cases = [
        (
            "set A",
            vec![
                coin(0xA3, 100 * UNIT),
                coin(0xA2, 5 * UNIT),
                coin(0xA1, FEE + 1),
            ],
            1,
            105 * UNIT,
        ),
        (
            "set B",
            vec![coin(0xB4, 40 * UNIT), coin(0xB3, 3 * UNIT)],
            3 * UNIT - FEE,
            40 * UNIT,
        ),
    ];
    for (what, offered, locked, spendable) in cases {
        let lines = summary(0, &spec(Vec::new(), 0, offered), 0);
        let has = |words: &[&str]| {
            lines.iter().any(|l| {
                let low = l.to_lowercase();
                words.iter().all(|w| low.contains(&w.to_lowercase()))
            })
        };
        let locked_txt = doli(locked);
        let spend_txt = doli(spendable);
        let height_txt = format!("height {lock}");
        let mut bad = Vec::new();
        if lines
            .iter()
            .any(|l| l.contains("You receive: 0.00000000 DOLI"))
        {
            bad.push("prints You receive: 0 while funds are locked".to_string());
        }
        if !has(&["lock", &locked_txt]) {
            bad.push(format!("no line with the locked amount {locked_txt}"));
        }
        if !has(&["lock", &height_txt]) {
            bad.push(format!("no line with the lock {height_txt}"));
        }
        if !has(&["spendable", &spend_txt]) {
            bad.push(format!(
                "no line with the amount that stays spendable {spend_txt}"
            ));
        }
        assert!(bad.is_empty(), "{what}: {bad:?}\nlines: {lines:#?}");
    }
}

fn bonds() -> Vec<(Hash, u32)> {
    vec![
        (Hash::from_bytes([0xD0; 32]), 0),
        (Hash::from_bytes([0xD1; 32]), 1),
    ]
}

const LOCK_LINE: &str =
    "Unbonding: the payout is locked until height 1070 (unbonding period 60 blocks).";

// REQ-DEF-002 — Decision: a failure means the zero-Bond summary fix changed what a Bond-input exit prints, which REQ-DEF-002 keeps byte-equal.
#[test]
fn inc_i_262_m1_req_def_002_golden_bond_exit_summary_text() {
    let fees = || vec![coin(0xF1, 5_000)];
    let cases: [(&str, u64, u64, u64, Vec<&str>); 4] = [
        (
            "gate open, no penalty",
            0,
            2 * UNIT,
            0,
            vec![
                "You receive: 2.00000000 DOLI",
                "No penalty — all bonds fully vested.",
                LOCK_LINE,
            ],
        ),
        (
            "gate open, 25% penalty",
            0,
            150_000_000,
            50_000_000,
            vec![
                "You receive: 1.50000000 DOLI",
                "Penalty burned: 0.50000000 DOLI",
                LOCK_LINE,
            ],
        ),
        (
            "gate open, 75% penalty",
            0,
            50_000_000,
            150_000_000,
            vec![
                "You receive: 0.50000000 DOLI",
                "Penalty burned: 1.50000000 DOLI",
                "WARNING: High penalty — 75% of bond value will be burned.",
                LOCK_LINE,
            ],
        ),
        (
            "gate closed, no penalty",
            CLOSED,
            2 * UNIT,
            0,
            vec![
                "You receive: 2.00000000 DOLI",
                "No penalty — all bonds fully vested.",
            ],
        ),
    ];
    assert_eq!(
        unbonding_lock_until(&params(0), BEST),
        1070,
        "FIXTURE: devnet period 60 + margin 10"
    );
    for (what, c1, net, penalty, want) in cases {
        let got = summary(c1, &spec(bonds(), net, fees()), penalty);
        assert_eq!(got, want, "{what}: Bond-input exit summary drifted");
    }
}
