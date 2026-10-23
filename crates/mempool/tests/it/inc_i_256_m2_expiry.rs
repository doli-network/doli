// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — mempool admission and revalidation judge the slash
//! evidence window (AUDIT-P2-539) and the tenure floor (AUDIT-P2-547) at the
//! tip slot handed in by the caller (judged slot = tip slot + 1).
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32 /*tip slot*/) -> Result<AddTransactionResult, MempoolError>
//   O2 receiver: residency of the slash (contains)
//   O3 return: Ok | Err(Validation(InvalidSlash(EXPIRED))) | Err(Validation(InvalidSlash(PREDATES)))
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64 /*tip*/, u32 /*tip slot*/)
//   O2 receiver: residency of the slash (evicted through the stale-slash verdict); no return value
// PATHS: P-OK fresh pair | P-X expired at admission | P-Y pre-registration at admission
//        | P-KEEP revalidate inside the window | P-BUSY producer set locked (fail open) | P-EVICT expired at revalidate
// INPUT PARTITIONS (offence slot 7, devnet unbonding period P, dense slot == height):
//   tip slot: 20 | 7 + P - 2 (age P - 1) | 7 + P - 1 (age P) | 7 + 10P
//   accused registered_at: 0 (genesis) | 5 (before the offence) | 50 (after the offence)
// MATRIX:
//   O2,O3 x P-OK x {20, age P - 1}           -> inc_i_256_m2_fresh_pair_admitted
//   O2,O3 x P-X x {age P, 7 + 10P}           -> inc_i_256_m2_expired_pair_refused_at_admission
//   O2,O3 x {P-OK, P-Y} x registered_at      -> inc_i_256_m2_pre_registration_pair_refused_at_admission
//   O2 x {P-KEEP, P-BUSY, P-EVICT}           -> inc_i_256_m2_revalidate_evicts_slash_once_expired

use crypto::KeyPair;
use doli_core::network::Network;
use doli_core::validation::ValidationError;
use doli_core::Transaction;
use mempool::{MempoolError, MempoolPolicy};

use crate::inc_i_256_m1_fixture::{policy, rig, slash, Rig, SLOT};

const EXPIRED: &str = "slash evidence expired";
const PREDATES: &str = "predates the accused's current registration";

fn period() -> u32 {
    let p = Network::Devnet.params().unbonding_period;
    assert!(p > 2 && p < 1_000, "FIXTURE: devnet unbonding period {p}");
    p as u32
}

fn fresh(kp: &KeyPair) -> (Rig, Transaction) {
    let r = rig(policy(10, MempoolPolicy::testnet().max_age), &[kp]);
    (r, slash(kp, (0xA1, 0xB2)))
}

fn set_registered_at(r: &Rig, kp: &KeyPair, height: u64) {
    r.producers
        .try_write()
        .expect("FIXTURE: lock")
        .get_by_pubkey_mut(kp.public_key())
        .expect("FIXTURE: registered")
        .registered_at = height;
}

/// Offer `tx` at a dense tip (height == slot == `tip`); (result, resident).
fn offer(r: &mut Rig, tx: &Transaction, tip: u32) -> (Result<(), MempoolError>, bool) {
    let res = r
        .pool
        .add_transaction(tx.clone(), &r.utxo, u64::from(tip), tip)
        .map(|_| ());
    (res, r.pool.contains(&tx.hash()))
}

fn slash_msg(r: &Result<(), MempoolError>) -> String {
    match r {
        Err(MempoolError::Validation(ValidationError::InvalidSlash(m))) => m.clone(),
        _ => String::new(),
    }
}

// R-4 — Decision: a failure means the mempool refuses fresh evidence the block rules accept, so a real offence is never mined.
#[test]
fn inc_i_256_m2_fresh_pair_admitted() {
    for tip in [20, SLOT + period() - 2] {
        let x = KeyPair::generate();
        let (mut r, s) = fresh(&x);
        let (res, resident) = offer(&mut r, &s, tip);
        assert!(
            res.is_ok() && resident,
            "tip {tip}: fresh evidence is admitted and resident; got {res:?}"
        );
    }
}

// R-4 (AUDIT-P2-539) — Decision: a failure means the mempool admits evidence the block rules refuse, so the builder assembles a block every node rejects.
#[test]
fn inc_i_256_m2_expired_pair_refused_at_admission() {
    for tip in [SLOT + period() - 1, SLOT + 10 * period()] {
        let x = KeyPair::generate();
        let (mut r, s) = fresh(&x);
        let (res, resident) = offer(&mut r, &s, tip);
        let m = slash_msg(&res);
        assert!(
            m.contains(EXPIRED) && !resident,
            "tip slot {tip}, offence slot {SLOT}: expired evidence must be refused with the expiry error; got {res:?}, resident={resident}"
        );
    }
}

// R-4 (AUDIT-P2-546, AUDIT-P2-547) — Decision: a failure means evidence from before the accused's current registration is admitted, so an old offence is reused against a new registration.
#[test]
fn inc_i_256_m2_pre_registration_pair_refused_at_admission() {
    let tip = 60;
    let x = KeyPair::generate();
    let (mut r, s) = fresh(&x);
    set_registered_at(&r, &x, 5);
    let (res, resident) = offer(&mut r, &s, tip);
    assert!(
        res.is_ok() && resident,
        "NON-VACUITY: registered at 5, offence slot {SLOT}: age 54 < tenure 56; got {res:?}"
    );

    let y = KeyPair::generate();
    let (mut r, s) = fresh(&y);
    set_registered_at(&r, &y, 50);
    let (res, resident) = offer(&mut r, &s, tip);
    let m = slash_msg(&res);
    assert!(
        m.contains(PREDATES) && !m.contains(EXPIRED) && !resident,
        "registered at 50, offence slot {SLOT}: age 54 >= tenure 11 must be refused with the tenure error; got {res:?}, resident={resident}"
    );
}

// R-4 (AUDIT-P2-539) — Decision: a failure means a slash whose evidence expired stays resident and poisons every block the builder assembles from it.
#[test]
fn inc_i_256_m2_revalidate_evicts_slash_once_expired() {
    let x = KeyPair::generate();
    let (mut r, s) = fresh(&x);
    let (res, _) = offer(&mut r, &s, 20);
    res.expect("PRECONDITION: fresh evidence is admitted");

    let last_fresh = SLOT + period() - 2;
    r.pool
        .revalidate(&r.utxo, u64::from(last_fresh), last_fresh);
    assert!(
        r.pool.contains(&s.hash()),
        "tip slot {last_fresh}: age P - 1, the slash stays"
    );

    let expired = last_fresh + 1;
    let producers = r.producers.clone();
    let held = producers.try_write().expect("FIXTURE: hold the write lock");
    r.pool.revalidate(&r.utxo, u64::from(expired), expired);
    assert!(
        r.pool.contains(&s.hash()),
        "a busy producer set must not evict (fail open)"
    );
    drop(held);

    r.pool.revalidate(&r.utxo, u64::from(expired), expired);
    assert!(
        !r.pool.contains(&s.hash()),
        "tip slot {expired}: age P, revalidate must evict the expired slash"
    );
    assert_eq!(r.pool.len(), 0);
}
