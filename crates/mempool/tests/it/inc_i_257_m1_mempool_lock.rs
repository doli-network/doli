//! INC-I-257 M1 — mempool admission uses the shared spend-lock predicate,
//! judged at tip + 1, on confirmed AND unconfirmed parents, for every input of
//! every tx type (no blanket withdrawal bypass).
//!
// covers: pool calculate_inputs unbonding
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Ok | Err); O2 residency of the tx (receiver: contains)
//   no store writes; the UtxoSet argument is read-only
// PATHS: P-TIP confirmed Unbonding parent | P-CHILD unconfirmed Unbonding parent | P-FEE locked Unbonding
//        fee input of a RequestWithdrawal | P-BOND Transfer spending a Bond past its lock | P-SAT top of range
// INPUT PARTITIONS:
//   P-TIP: tip = lock_until - 2 (judged at lock_until - 1) -> refused | tip = lock_until - 1 -> admitted
//   P-CHILD: parent output Unbonding -> child refused | parent output Normal -> child admitted (CPFP intact)
//   P-FEE: extra input Unbonding locked past tip + 1 -> refused | unlocked at tip + 1 -> admitted
//   P-BOND: devnet gate 0 -> refused | devnet gate raised by env in a child process -> admitted
//   P-SAT: tip = u64::MAX - 2 -> refused | tip = u64::MAX - 1 and u64::MAX -> admitted, no panic
//   P-MAT: coinbase input, tip = maturity - 1 -> refused | tip = maturity -> admitted (judged at tip)
// MATRIX:
//   O1 x P-MAT -> inc_i_257_m1_req_unbond_005_mempool_coinbase_maturity_judged_at_tip
//   O1,O2 x P-TIP -> inc_i_257_m1_req_unbond_005_mempool_judges_lock_at_tip_plus_one
//   O1,O2 x P-CHILD -> inc_i_257_m1_req_unbond_005_mempool_refuses_child_of_unconfirmed_unbonding_parent
//   O1,O2 x P-FEE -> inc_i_257_m1_req_unbond_006_mempool_refuses_locked_unbonding_fee_input
//   O1,O2 x P-BOND -> inc_i_257_m1_req_unbond_014_mempool_bond_spent_by_transfer_*
//   O1,O2 x P-SAT -> inc_i_257_m1_req_unbond_sec_001_mempool_tip_plus_one_saturates

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

const UNBONDING_TYPE_BYTE: u8 = 16;
const PERIOD: u64 = 60;
const FEE: u64 = 1_000;
const COIN: u64 = 50_000;
const TIP: u64 = 5;
const GATE_ENV: &str = "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT";
const SLASH_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_257_M1_MEMPOOL_CHILD";

fn unbonding_type() -> OutputType {
    OutputType::from_u8(UNBONDING_TYPE_BYTE)
        .expect("PRECONDITION: type byte 16 (Unbonding) must decode")
}

fn kp() -> KeyPair {
    KeyPair::from_seed([0x57; 32])
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn op(tag: u8) -> Outpoint {
    Outpoint::new(Hash::from_bytes([tag; 32]), 0)
}

fn unbonding_out(amount: u64, lock_until: u64) -> Output {
    Output {
        output_type: unbonding_type(),
        amount,
        pubkey_hash: addr(kp().public_key()),
        lock_until,
        extra_data: Vec::new(),
    }
}

fn seed(utxo: &mut UtxoSet, at: Outpoint, output: Output) {
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(at, entry).expect("FIXTURE: seed UTXO");
}

fn signed(mut tx: Transaction) -> Transaction {
    let k = kp();
    for i in 0..tx.inputs.len() {
        tx.inputs[i].public_key = Some(*k.public_key());
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, k.private_key());
    }
    tx
}

fn transfer(from: Outpoint, amount_in: u64) -> Transaction {
    let inputs = vec![Input::new(from.tx_hash, from.index)];
    let out = Output::normal(amount_in - FEE, addr(kp().public_key()));
    signed(Transaction::new_transfer(inputs, vec![out]))
}

fn gated_withdrawal(inputs: &[Outpoint], net: u64) -> Transaction {
    let pk = *kp().public_key();
    let ins = inputs
        .iter()
        .map(|o| Input::new(o.tx_hash, o.index))
        .collect();
    let mut tx = Transaction::new_request_withdrawal(ins, pk, 1, addr(&pk), net);
    tx.outputs = vec![unbonding_out(net, TIP + 1 + PERIOD + 5)];
    signed(tx)
}

fn devnet_pool() -> Mempool {
    Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    )
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn offer(
    pool: &mut Mempool,
    tx: &Transaction,
    utxo: &UtxoSet,
    tip: u64,
) -> Result<(), MempoolError> {
    pool.add_transaction(tx.clone(), utxo, tip, 1).map(|_| ())
}

/// Parent: re-exec `test` alone with `env`, return true. Child: return false.
fn spawned_child(test: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(format!("inc_i_257_m1_mempool_lock::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env_remove(GATE_ENV)
        .env_remove(SLASH_ENV);
    for (k, v) in env {
        cmd.env(k, v.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

// REQ-UNBOND-005 — Decision: a failure means the mempool judges the lock at the tip instead of the next block's height, so it admits a spend one block early or refuses one that the next block may carry.
#[test]
fn inc_i_257_m1_req_unbond_005_mempool_judges_lock_at_tip_plus_one() {
    let lock = 100;
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, op(0x01), unbonding_out(COIN, lock));
    let t = transfer(op(0x01), COIN);

    let mut early = devnet_pool();
    let r = offer(&mut early, &t, &utxo, lock - 2);
    assert!(
        r.is_err(),
        "tip lock_until - 2 (block at lock_until - 1) must refuse: {r:?}"
    );
    assert!(
        !early.contains(&t.hash()),
        "a refused spend must not be resident"
    );

    let mut on_time = devnet_pool();
    let r = offer(&mut on_time, &t, &utxo, lock - 1);
    assert!(
        r.is_ok(),
        "tip lock_until - 1 (block at lock_until) must admit: {r:?}"
    );
    assert!(on_time.contains(&t.hash()), "an admitted spend is resident");
}

// REQ-UNBOND-005 — Decision: a failure means a child can spend a still-unmined locked payout through the in-mempool parent path, which skips every lock check today.
#[test]
fn inc_i_257_m1_req_unbond_005_mempool_refuses_child_of_unconfirmed_unbonding_parent() {
    let mut utxo = UtxoSet::new();
    let pk = *kp().public_key();
    seed(
        &mut utxo,
        op(0x02),
        Output::bond(unit(), addr(&pk), u64::MAX, 0),
    );
    seed(&mut utxo, op(0x03), Output::normal(COIN, addr(&pk)));
    let net = unit() - FEE;
    let parent = gated_withdrawal(&[op(0x02)], net);
    let normal_parent = transfer(op(0x03), COIN);

    let mut pool = devnet_pool();
    let r = offer(&mut pool, &parent, &utxo, TIP);
    assert!(
        r.is_ok(),
        "PRECONDITION: the gated withdrawal is admitted: {r:?}"
    );
    offer(&mut pool, &normal_parent, &utxo, TIP).expect("PRECONDITION: Normal parent admitted");

    let child = transfer(Outpoint::new(parent.hash(), 0), net);
    let r = offer(&mut pool, &child, &utxo, TIP);
    assert!(
        r.is_err(),
        "child of an unconfirmed Unbonding parent must be refused: {r:?}"
    );
    assert!(
        !pool.contains(&child.hash()),
        "refused child must not be resident"
    );
    assert!(pool.contains(&parent.hash()), "the parent stays resident");

    let cpfp = transfer(Outpoint::new(normal_parent.hash(), 0), COIN - FEE);
    let r = offer(&mut pool, &cpfp, &utxo, TIP);
    assert!(
        r.is_ok(),
        "NON-VACUITY: a child of an unconfirmed Normal parent is still admitted: {r:?}"
    );
}

// REQ-UNBOND-006 — Decision: a failure means the mempool's blanket RequestWithdrawal bypass releases a locked Unbonding output used as an extra (fee) input.
#[test]
fn inc_i_257_m1_req_unbond_006_mempool_refuses_locked_unbonding_fee_input() {
    let pk = *kp().public_key();
    for (lock, admitted) in [(200, false), (TIP + 1, true)] {
        let mut utxo = UtxoSet::new();
        seed(
            &mut utxo,
            op(0x04),
            Output::bond(unit(), addr(&pk), u64::MAX, 0),
        );
        seed(&mut utxo, op(0x05), unbonding_out(COIN, lock));
        let tx = gated_withdrawal(&[op(0x04), op(0x05)], unit() + COIN - FEE);
        let mut pool = devnet_pool();
        let r = offer(&mut pool, &tx, &utxo, TIP);
        assert_eq!(
            r.is_ok(),
            admitted,
            "fee input locked until {lock}, tip {TIP}: expected admitted={admitted}, got {r:?}"
        );
        assert_eq!(
            pool.contains(&tx.hash()),
            admitted,
            "residency for lock {lock}"
        );
    }
}

fn bond_transfer_case() -> (UtxoSet, Transaction) {
    let pk = *kp().public_key();
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, op(0x06), Output::bond(unit(), addr(&pk), 10, 0));
    let t = transfer(op(0x06), unit());
    (utxo, t)
}

// REQ-UNBOND-014 — Decision: a failure means the mempool admits a Transfer that moves a Bond whose finite lock has passed, a burn escape that apply would refuse.
#[test]
fn inc_i_257_m1_req_unbond_014_mempool_bond_spent_by_transfer_refused_at_gate() {
    let (utxo, t) = bond_transfer_case();
    let mut pool = devnet_pool();
    let r = offer(&mut pool, &t, &utxo, 50);
    assert!(
        r.is_err(),
        "devnet (gate 0): Transfer spending a Bond past its lock must be refused: {r:?}"
    );
    assert!(!pool.contains(&t.hash()));
}

// REQ-UNBOND-014 / REQ-UNBOND-017 — Decision: a failure means the Bond-input rule leaks below the gate, so pre-gate mempool behaviour changes.
#[test]
fn inc_i_257_m1_req_unbond_014_mempool_bond_spent_by_transfer_admitted_below_gate() {
    if spawned_child(
        "inc_i_257_m1_req_unbond_014_mempool_bond_spent_by_transfer_admitted_below_gate",
        &[(GATE_ENV, 1_000_000), (SLASH_ENV, 1_000_000)],
    ) {
        return;
    }
    let (utxo, t) = bond_transfer_case();
    let mut pool = devnet_pool();
    let r = offer(&mut pool, &t, &utxo, 50);
    assert!(
        r.is_ok(),
        "below the gate the Transfer of an expired Bond keeps today's verdict: {r:?}"
    );
    assert!(pool.contains(&t.hash()));
}

// REQ-UNBOND-005 — Decision: a failure means moving the lock judgement to tip + 1 also moved coinbase maturity, so a minted output is admitted one block early or a mature one is refused.
#[test]
fn inc_i_257_m1_req_unbond_005_mempool_coinbase_maturity_judged_at_tip() {
    let maturity = Network::Devnet.coinbase_maturity();
    assert!(maturity >= 1, "NON-VACUITY: devnet has a coinbase maturity");
    let mut utxo = UtxoSet::new();
    let minted = UtxoEntry {
        output: Output::normal(COIN, addr(kp().public_key())),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    utxo.insert(op(0x08), minted)
        .expect("FIXTURE: seed coinbase");
    let t = transfer(op(0x08), COIN);
    for (tip, admitted) in [(maturity - 1, false), (maturity, true)] {
        let mut pool = devnet_pool();
        let r = offer(&mut pool, &t, &utxo, tip);
        assert_eq!(
            r.is_ok(),
            admitted,
            "tip {tip}, maturity {maturity}: expected admitted={admitted}, got {r:?}"
        );
    }
}

// REQ-UNBOND-SEC-001 — Decision: a failure means the tip + 1 judgement overflows (a panic in admission) at the top of the height range.
#[test]
fn inc_i_257_m1_req_unbond_sec_001_mempool_tip_plus_one_saturates() {
    let m = u64::MAX;
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, op(0x07), unbonding_out(COIN, m));
    let t = transfer(op(0x07), COIN);
    for (tip, admitted) in [(m - 2, false), (m - 1, true), (m, true)] {
        let mut pool = devnet_pool();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            offer(&mut pool, &t, &utxo, tip)
        }))
        .unwrap_or_else(|_| panic!("admission panicked at tip {tip}"));
        assert_eq!(
            r.is_ok(),
            admitted,
            "tip {tip}: expected admitted={admitted}, got {r:?}"
        );
    }
}
