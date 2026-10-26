//! INC-I-256 M1 / R-2 — a slash admitted under the slashing gate is never the
//! eviction or expiry victim; below the gate it stays evictable; one resident
//! slash per accused.
//!
// covers: pool slash_admission entry lib slash_state
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Ok tx_hash | Err variant); O2 receiver residency (contains, len) incl. the eviction victim
// OUTPUT CONTRACT: fn Mempool::expire_old(&mut self)
//   O2 receiver residency of each entry
// OUTPUT CONTRACT: fn Mempool::add_system_transaction(&mut self, Transaction, u64, Option<&ZeroFlowSnapshot>) -> Result<Hash, MempoolError>
//   O1 return; O2 receiver residency
// PATHS: P-EVICT full pool, eviction loop | P-FULL no evictable entry -> Full | P-EXPIRE entry past max_age
//        | P-BELOW slash judged below the gate | P-DUP second slash for a resident accused
// INPUT PARTITIONS:
//   gate: devnet default (slash admitted fee-exempt) | env-raised (slash not exempt)
//   residents: {gated slash, normal tx} | {gated slash only} | {system-lane slash below gate, normal tx}
//   second slash: other header pair, same accused | the same tx again
// MATRIX:
//   O1,O2 x P-EVICT x {gated slash, normal} -> inc_i_256_m1_full_pool_evicts_normal_tx_not_gated_slash
//   O1,O2 x P-FULL x {gated slash only} -> inc_i_256_m1_full_pool_of_gated_slashes_refuses_with_full
//   O2 x P-EXPIRE x {gated slash, normal} -> inc_i_256_m1_expire_old_keeps_gated_slash
//   O1,O2 x P-BELOW x {add_transaction, system lane + evict, system lane + expire} -> inc_i_256_m1_below_gate_slash_stays_evictable_and_expirable
//   O1,O2 x P-DUP x {other pair, same tx} -> inc_i_256_m1_second_slash_for_resident_accused_refused_in_full_pool

use std::time::Duration;

use crypto::KeyPair;
use doli_core::network::Network;
use doli_core::validation::ValidationError;
use mempool::{MempoolError, MempoolPolicy};

use crate::inc_i_256_m1_fixture::{
    funded_transfer, policy, rig, slash, spawned_child, HIGH_FEE, LOW_FEE, TIP,
};

const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_256_M1_EVICTION_CHILD";
const RAISED_GATE: u64 = 1_000_000;
const PAST_ONE_SECOND: Duration = Duration::from_millis(1_100);

fn default_age() -> u64 {
    MempoolPolicy::testnet().max_age
}

// R-2 — Decision: a failure means a full pool evicts the fee-0 gated slash first, so a valid slash is lost before any block carries it.
#[test]
fn inc_i_256_m1_full_pool_evicts_normal_tx_not_gated_slash() {
    let x = KeyPair::generate();
    let mut r = rig(policy(2, default_age()), &[&x]);
    let s = slash(&x, (0xA1, 0xB2));
    let cheap = funded_transfer(&mut r.utxo, 0x11, LOW_FEE);
    let rich = funded_transfer(&mut r.utxo, 0x12, HIGH_FEE);
    r.pool
        .add_transaction(s.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the gated slash is admitted");
    assert_eq!(
        r.pool.get(&s.hash()).map(|e| e.fee),
        Some(0),
        "PRECONDITION: the slash is resident at fee 0"
    );
    r.pool
        .add_transaction(cheap.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the cheap transfer is admitted");
    assert_eq!(r.pool.len(), 2, "PRECONDITION: the pool is full");

    let res = r.pool.add_transaction(rich.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(&res, Ok(a) if a.tx_hash == rich.hash()),
        "the higher-fee transfer enters the full pool; got {res:?}"
    );
    assert!(
        r.pool.contains(&s.hash()),
        "the gated slash must survive eviction from a full pool"
    );
    assert!(
        !r.pool.contains(&cheap.hash()),
        "the lowest-fee normal tx is the eviction victim"
    );
    assert!(r.pool.contains(&rich.hash()));
    assert_eq!(r.pool.len(), 2, "exactly one entry was evicted");
}

// R-2 — Decision: a failure means a pool whose only resident is a gated slash still evicts it to admit a paying tx.
#[test]
fn inc_i_256_m1_full_pool_of_gated_slashes_refuses_with_full() {
    let x = KeyPair::generate();
    let mut r = rig(policy(1, default_age()), &[&x]);
    let s = slash(&x, (0xA1, 0xB2));
    let t = funded_transfer(&mut r.utxo, 0x13, HIGH_FEE);
    r.pool
        .add_transaction(s.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the gated slash is admitted");

    let res = r.pool.add_transaction(t.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(res, Err(MempoolError::Full)),
        "with no evictable entry the pool must refuse with Full; got {res:?}"
    );
    assert!(r.pool.contains(&s.hash()), "the gated slash stays resident");
    assert!(
        !r.pool.contains(&t.hash()),
        "the refused tx is not resident"
    );
    assert_eq!(r.pool.len(), 1);
}

// R-2 — Decision: a failure means a gated slash ages out of the pool before it is mined, and nothing re-creates it.
#[test]
fn inc_i_256_m1_expire_old_keeps_gated_slash() {
    let x = KeyPair::generate();
    let mut r = rig(policy(10, 0), &[&x]);
    let s = slash(&x, (0xA1, 0xB2));
    let t = funded_transfer(&mut r.utxo, 0x14, LOW_FEE);
    r.pool
        .add_transaction(s.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the gated slash is admitted");
    r.pool
        .add_transaction(t.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the transfer is admitted");
    std::thread::sleep(PAST_ONE_SECOND);

    r.pool.expire_old();
    assert!(
        !r.pool.contains(&t.hash()),
        "NON-VACUITY: the normal tx past max_age expires"
    );
    assert!(
        r.pool.contains(&s.hash()),
        "the gated slash must not expire"
    );
    assert_eq!(r.pool.len(), 1);
}

// R-2 — Decision: a failure means the exemption is keyed on the tx type instead of the gated admission verdict, so a slash judged below the gate becomes unevictable.
#[test]
fn inc_i_256_m1_below_gate_slash_stays_evictable_and_expirable() {
    if spawned_child(
        "inc_i_256_m1_slash_eviction::inc_i_256_m1_below_gate_slash_stays_evictable_and_expirable",
        CHILD_ENV,
        &[(GATE_ENV, RAISED_GATE)],
    ) {
        return;
    }
    assert_eq!(
        Network::Devnet
            .params()
            .effective_slashing_activation_height(),
        RAISED_GATE,
        "FIXTURE: env-raised slashing gate"
    );
    let x = KeyPair::generate();
    let mut r = rig(policy(2, default_age()), &[&x]);
    let s = slash(&x, (0xA1, 0xB2));

    let normal_lane = r.pool.add_transaction(s.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(normal_lane, Err(MempoolError::FeeTooLow(0, _))),
        "below the gate a slash owes the fee floor and has no input to pay it; got {normal_lane:?}"
    );
    assert!(!r.pool.contains(&s.hash()));

    let id = r
        .pool
        .add_system_transaction(s.clone(), TIP, None)
        .expect("PRECONDITION: the system lane holds a below-gate slash at fee 0");
    assert_eq!(id, s.hash());
    let cheap = funded_transfer(&mut r.utxo, 0x15, LOW_FEE);
    let rich = funded_transfer(&mut r.utxo, 0x16, HIGH_FEE);
    r.pool
        .add_transaction(cheap.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the cheap transfer is admitted");
    r.pool
        .add_transaction(rich.clone(), &r.utxo, TIP, 0)
        .expect("the higher-fee transfer enters the full pool");
    assert!(
        !r.pool.contains(&s.hash()),
        "below the gate the fee-0 slash is the eviction victim"
    );
    assert!(r.pool.contains(&cheap.hash()) && r.pool.contains(&rich.hash()));

    let mut aged = rig(policy(10, 0), &[&x]);
    aged.pool
        .add_system_transaction(s.clone(), TIP, None)
        .expect("PRECONDITION: system-lane slash resident");
    std::thread::sleep(PAST_ONE_SECOND);
    aged.pool.expire_old();
    assert!(
        !aged.pool.contains(&s.hash()),
        "below the gate the slash expires like any other tx"
    );
}

// R-2 — Decision: a failure means one accused can hold two resident gated slashes (unevictable, so a flood), or a refused duplicate evicts a paying tx.
#[test]
fn inc_i_256_m1_second_slash_for_resident_accused_refused_in_full_pool() {
    let x = KeyPair::generate();
    let mut r = rig(policy(2, default_age()), &[&x]);
    let first = slash(&x, (0xA1, 0xB2));
    let second = slash(&x, (0xC3, 0xD4));
    assert_ne!(first.hash(), second.hash(), "FIXTURE: two header pairs");
    let cheap = funded_transfer(&mut r.utxo, 0x17, LOW_FEE);
    r.pool
        .add_transaction(first.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the first slash is admitted");
    r.pool
        .add_transaction(cheap.clone(), &r.utxo, TIP, 0)
        .expect("PRECONDITION: the transfer is admitted");

    let refused = r.pool.add_transaction(second.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(&refused, Err(MempoolError::Validation(ValidationError::InvalidSlash(m)))
            if m.contains("already has a resident slash")),
        "a second slash for a resident accused must be refused; got {refused:?}"
    );
    assert!(!r.pool.contains(&second.hash()));
    assert!(
        r.pool.contains(&first.hash()) && r.pool.contains(&cheap.hash()),
        "the refusal must evict nothing"
    );
    assert_eq!(r.pool.len(), 2);

    let again = r.pool.add_transaction(first.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(again, Err(MempoolError::AlreadyExists)),
        "the same slash again is one resident tx; got {again:?}"
    );
    assert_eq!(r.pool.len(), 2);
}
