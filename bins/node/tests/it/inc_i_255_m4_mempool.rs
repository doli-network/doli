//! INC-I-255 M4 — the mempool never holds a slash for p beside p's bond
//! withdrawal: admitting the slash evicts the withdrawal, the withdrawal is
//! refused while the slash is resident, and revalidation evicts a pair that
//! met before the gate.
//!
// covers: slash_state, block, assembly, slash_admission, pool, slash_withdrawal, slash_evidence, entry, mod, lib, protocol, security_model
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return; O2 residency of Slash(p), RW(p), RW(q) (receiver)
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O1 residency of Slash(p), RW(p), RW(q) (receiver)
// PATHS: P-ON devnet slash gate (0) | P-EDGE child process, slash gate env-set to 2, admission heights chosen per tx
// MATRIX
//   add_transaction O1,O2 x P-ON x RW(p), RW(q) resident -> Slash(p) admitted; RW(p) evicted; RW(q) kept (probe_5)
//   add_transaction O1,O2 x P-ON x Slash(p) resident -> RW(p) Err, not resident; RW(q) Ok; RW(p) Ok once the slash leaves (probe_6)
//   revalidate O1 x P-EDGE x {Slash(p) judged at the gate, RW(p) + RW(q) judged below}:
//     below the gate -> all kept; at the gate -> RW(p) evicted, Slash(p) and RW(q) kept (probe_7)
// INPUT PARTITIONS:
//   insertion order: withdrawal first | slash first
//   withdrawing producer: the accused p | another producer q
//   judged height: below the gate | at/above the gate

use super::inc_i_255_m1_common::{in_child, run_in_child, tip};
use super::inc_i_255_m4_common::*;

const GATE: u64 = 2;

// REQ-SLASH-003 — Decision: a failure means a resident RW(p) survives the admission of Slash(p), so both reach the builder and the block half-applies or p escapes the burn.
#[tokio::test]
async fn inc_i_255_m4_probe_5_admit_slash_evicts_wd() {
    let r = m4(FEE_TIE).await;
    let n = &r.w.node;
    let h = tip(n).await.0;
    for tx in [&r.wd, &r.other_wd] {
        let res = admit_at(n, tx, h).await;
        assert!(
            res.is_ok(),
            "PRECONDITION: withdrawal admitted; got {res:?}"
        );
    }
    let s = admit_at(n, &r.slash, h).await;
    let (slash_res, wd_res, other_res) = (
        resident(n, &r.slash).await,
        resident(n, &r.wd).await,
        resident(n, &r.other_wd).await,
    );
    assert!(
        s.is_ok() && slash_res,
        "Slash(p) must be admitted; got {s:?}"
    );
    assert!(
        !wd_res,
        "admitting Slash(p) must evict the resident RW(p); RW(p) still resident"
    );
    assert!(
        other_res,
        "NON-VACUITY: RW(q) by another producer stays resident"
    );
}

// REQ-SLASH-003 — Decision: a failure means RW(p) joins a resident Slash(p), so the builder can pack both.
#[tokio::test]
async fn inc_i_255_m4_probe_6_refuse_wd_while_slash_resident() {
    let r = m4(FEE_TIE).await;
    let n = &r.w.node;
    let h = tip(n).await.0;
    let s = admit_at(n, &r.slash, h).await;
    assert!(s.is_ok(), "PRECONDITION: Slash(p) admitted; got {s:?}");
    let wd = admit_at(n, &r.wd, h).await;
    let wd_res = resident(n, &r.wd).await;
    let other = admit_at(n, &r.other_wd, h).await;
    assert!(
        wd.is_err() && !wd_res,
        "RW(p) must be refused while Slash(p) is resident; got {wd:?}, resident {wd_res}"
    );
    assert!(
        other.is_ok() && resident(n, &r.other_wd).await,
        "NON-VACUITY: RW(q) by another producer is admitted; got {other:?}"
    );
    n.mempool.write().await.remove_transaction(&r.slash.hash());
    let again = admit_at(n, &r.wd, h).await;
    assert!(
        again.is_ok(),
        "NON-VACUITY: with the slash gone RW(p) is admitted; got {again:?}"
    );
}

// REQ-SLASH-003 — Decision: a failure means a pair that met before the gate stays resident after it, and the builder keeps offering both.
#[tokio::test]
async fn inc_i_255_m4_probe_7_revalidate_evicts_wd() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m4_mempool::inc_i_255_m4_probe_7_revalidate_evicts_wd",
            GATE,
        );
        return;
    }
    let r = m4(FEE_TIE).await;
    let n = &r.w.node;
    assert_eq!(tip(n).await.0, 0, "FIXTURE: genesis tip");
    // Judged one block above the gate: the activation block has no tenure.
    let s = admit_at(n, &r.slash, GATE).await;
    assert!(
        s.is_ok(),
        "PRECONDITION: Slash(p) judged above the gate admitted; got {s:?}"
    );
    for tx in [&r.wd, &r.other_wd] {
        let res = admit_at(n, tx, 0).await;
        assert!(
            res.is_ok(),
            "PRECONDITION: withdrawal judged below the gate admitted; got {res:?}"
        );
    }
    revalidate_at(n, 0).await;
    let below = (
        resident(n, &r.slash).await,
        resident(n, &r.wd).await,
        resident(n, &r.other_wd).await,
    );
    revalidate_at(n, GATE).await;
    let at = (
        resident(n, &r.slash).await,
        resident(n, &r.wd).await,
        resident(n, &r.other_wd).await,
    );
    assert_eq!(
        below,
        (true, true, true),
        "below the gate revalidate keeps (Slash(p), RW(p), RW(q))"
    );
    assert!(
        !at.1,
        "at the gate revalidate must evict RW(p) beside Slash(p); (Slash(p), RW(p), RW(q)) resident = {at:?}"
    );
    assert!(
        at.0 && at.2,
        "NON-VACUITY: Slash(p) and RW(q) stay resident; got {at:?}"
    );
}
