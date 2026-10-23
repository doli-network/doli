//! INC-I-255 M6 — the mempool never holds Slash(p) beside a tx spending a
//! Bond UTXO owned by p: admitting the slash evicts it, it is refused while the
//! slash is resident, and revalidation evicts a pair that met below the gate.
//!
// covers: slash_state, utxo, block, assembly, tx_processing, slash_burn, slash_admission, pool, slash_withdrawal, withdrawal_economics, slash_bond_spend, mod.rs, lib.rs, validation_checks, protocol, security_model
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return; O2 residency of Slash(p), T(p), T(q) (receiver)
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O1 residency of Slash(p), T(p), T(q) (receiver)
// PATHS: P-ON devnet slash gate (0), admissions judged at height 1 (lock_until 1 expired)
//        | P-EDGE child process, slash gate env-set to 3, admission heights chosen per tx
// MATRIX
//   add_transaction O1,O2 x P-ON x T(p), T(q) resident -> Slash(p) admitted; T(p) evicted; T(q) kept (probe_5)
//   add_transaction O1,O2 x P-ON x Slash(p) resident -> T(p) Err, not resident; T(q) Ok; T(p) Ok once the slash leaves (probe_6)
//   revalidate O1 x P-EDGE x {Slash(p) judged at the gate, T(p) + T(q) judged below}:
//     below the gate -> all kept; at the gate -> T(p) evicted, Slash(p) and T(q) kept (probe_7)
// INPUT PARTITIONS:
//   insertion order: spend first | slash first
//   spender: the accused p | another producer q
//   judged height: below the gate | at/above the gate

use super::inc_i_255_m1_common::{in_child, run_in_child, tip};
use super::inc_i_255_m4_common::{admit_at, resident, revalidate_at};
use super::inc_i_255_m6_common::*;

const AT: u64 = 1;
const GATE: u64 = 3;

// REQ-SLASH-003 — Decision: a failure means a resident T(p) survives the admission of Slash(p), so both reach the builder and the block half-applies or p escapes the burn.
#[tokio::test]
async fn inc_i_255_m6_probe_5_admit_slash_evicts_spend() {
    if !in_child() {
        let test = "inc_i_255_m6_mempool::inc_i_255_m6_probe_5_admit_slash_evicts_spend";
        return run_in_child(test, 0);
    }
    let r = m6(FEE_TIE).await;
    let n = &r.w.node;
    for tx in [&r.t, &r.tq] {
        let res = admit_at(n, tx, AT).await;
        assert!(res.is_ok(), "PRECONDITION: Transfer admitted; got {res:?}");
    }
    let s = admit_at(n, &r.slash, AT).await;
    let (slash_res, t_res, tq_res) = (
        resident(n, &r.slash).await,
        resident(n, &r.t).await,
        resident(n, &r.tq).await,
    );
    assert!(
        s.is_ok() && slash_res,
        "Slash(p) must be admitted; got {s:?}"
    );
    assert!(
        !t_res,
        "admitting Slash(p) must evict the resident T(p) spending p's Bond; T(p) still resident"
    );
    assert!(tq_res, "NON-VACUITY: T(q) spending q's Bond stays resident");
}

// REQ-SLASH-003 — Decision: a failure means T(p) joins a resident Slash(p), so the builder can pack both.
#[tokio::test]
async fn inc_i_255_m6_probe_6_refuse_spend_while_slash_resident() {
    if !in_child() {
        let test = "inc_i_255_m6_mempool::inc_i_255_m6_probe_6_refuse_spend_while_slash_resident";
        return run_in_child(test, 0);
    }
    let r = m6(FEE_TIE).await;
    let n = &r.w.node;
    let s = admit_at(n, &r.slash, AT).await;
    assert!(s.is_ok(), "PRECONDITION: Slash(p) admitted; got {s:?}");
    let t = admit_at(n, &r.t, AT).await;
    let t_res = resident(n, &r.t).await;
    let other = admit_at(n, &r.tq, AT).await;
    assert!(
        t.is_err() && !t_res,
        "T(p) spending p's Bond must be refused while Slash(p) is resident; got {t:?}, resident {t_res}"
    );
    assert!(
        other.is_ok() && resident(n, &r.tq).await,
        "NON-VACUITY: T(q) spending q's Bond is admitted; got {other:?}"
    );
    n.mempool.write().await.remove_transaction(&r.slash.hash());
    let again = admit_at(n, &r.t, AT).await;
    assert!(
        again.is_ok(),
        "NON-VACUITY: with the slash gone T(p) is admitted; got {again:?}"
    );
}

// REQ-SLASH-003 — Decision: a failure means a pair that met below the gate stays resident after it, and the builder keeps offering both.
#[tokio::test]
async fn inc_i_255_m6_probe_7_revalidate_evicts_spend() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m6_mempool::inc_i_255_m6_probe_7_revalidate_evicts_spend",
            GATE,
        );
        return;
    }
    let r = m6(FEE_TIE).await;
    let n = &r.w.node;
    assert_eq!(tip(n).await.0, 0, "FIXTURE: genesis tip");
    // Judged one block above the gate: the activation block has no tenure.
    let s = admit_at(n, &r.slash, GATE).await;
    assert!(
        s.is_ok(),
        "PRECONDITION: Slash(p) judged above the gate admitted; got {s:?}"
    );
    for tx in [&r.t, &r.tq] {
        let res = admit_at(n, tx, AT).await;
        assert!(
            res.is_ok(),
            "PRECONDITION: Transfer judged below the gate admitted; got {res:?}"
        );
    }
    revalidate_at(n, AT).await;
    let below = (
        resident(n, &r.slash).await,
        resident(n, &r.t).await,
        resident(n, &r.tq).await,
    );
    revalidate_at(n, GATE).await;
    let at = (
        resident(n, &r.slash).await,
        resident(n, &r.t).await,
        resident(n, &r.tq).await,
    );
    assert_eq!(
        below,
        (true, true, true),
        "below the gate revalidate keeps (Slash(p), T(p), T(q))"
    );
    assert!(
        !at.1,
        "at the gate revalidate must evict T(p) beside Slash(p); (Slash(p), T(p), T(q)) resident = {at:?}"
    );
    assert!(
        at.0 && at.2,
        "NON-VACUITY: Slash(p) and T(q) stay resident; got {at:?}"
    );
}
