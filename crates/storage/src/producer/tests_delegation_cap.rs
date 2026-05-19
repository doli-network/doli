//! INC-I-078: Per-producer `received_delegation_cap` storage-layer tests.
//!
//! Covers the **defensive cap check** that lives in
//! [`ProducerSet::delegate_bonds_capped`]. The primary block-apply check lives
//! at `bins/node/src/node/apply_block/tx_processing.rs` and is covered by node
//! integration tests; this file is the storage-layer regression lock.
//!
//! OUTPUT CONTRACT: fn delegate_bonds_capped(d, e, c, cap) -> Result<(), String>
//! O1: Result — Ok if delegation accepted, Err(reason) on cap exceedance.
//! O2: delegatee.received_delegations — unchanged on Err; entry appended on Ok.
//! O3: delegator.delegated_bonds — unchanged on Err; set to c on Ok.
//!
//! INPUT PARTITIONS (per `delegate_bonds_capped`):
//!   I1 = cap == 0                                                 → check bypassed
//!   I2 = cap > 0, current_total + bond_count <= cap               → accept
//!   I3 = cap > 0, current_total + bond_count == cap (boundary)    → accept
//!   I4 = cap > 0, current_total + bond_count == cap + 1 (overflow) → reject
//!   I5 = cap > 0, current_total already >= cap (grandfathered)    → any new add rejected
//!   I6 = cap > 0, multi-delegator race: two adds, each fits, sum > cap → second rejected
//!
//! MATRIX:
//!   O1×I1: Ok(()), O1×I2: Ok(()), O1×I3: Ok(()),
//!   O1×I4: Err("delegation cap exceeded"), O1×I5: Err, O1×I6: second call Err.
//!
//! These tests exist BEFORE the production caller wiring (which lives in
//! `bins/node`) — they verify the storage primitive in isolation so the
//! defensive layer is locked even if the primary check is bypassed.

#[allow(deprecated)]
use super::*;
use crypto::{Hash, KeyPair};

/// Helper: register a producer with `n_bonds` bonds at a unique outpoint index.
fn register_producer(ps: &mut ProducerSet, kp: &KeyPair, n_bonds: u32, idx: u32) {
    let info = ProducerInfo::new_with_bonds(
        *kp.public_key(),
        0,
        n_bonds as u64 * BOND_UNIT,
        (Hash::ZERO, idx),
        0,
        n_bonds,
    );
    ps.register(info, 0).unwrap();
}

// ---------------------------------------------------------------------------
// I1: cap == 0 means "no cap" — pre-activation behavior, unchanged from
// today. The 3-arg wrapper `delegate_bonds` also routes here.
// ---------------------------------------------------------------------------

#[test]
fn test_cap_zero_disables_check_via_wrapper() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 5, 0);
    register_producer(&mut ps, &kp_d, 5, 1);

    // 3-arg API == cap=0 == no cap. Even a delegation that would be huge
    // relative to any sane cap goes through.
    let r = ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 5);
    assert!(r.is_ok(), "cap=0 (wrapper) must not reject: {r:?}");

    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(a.received_delegations.len(), 1);
    assert_eq!(a.received_delegations[0].1, 5);
}

#[test]
fn test_cap_zero_disables_check_explicit() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 5, 0);
    register_producer(&mut ps, &kp_d, 5, 1);

    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), 5, 0);
    assert!(r.is_ok(), "cap=0 (explicit) must not reject: {r:?}");
}

// ---------------------------------------------------------------------------
// I2 + I3: cap > 0, below or at the boundary → accept.
// ---------------------------------------------------------------------------

#[test]
fn test_cap_below_boundary_accepts() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 5, 0);
    register_producer(&mut ps, &kp_d, 5, 1);

    // cap=10, requesting 3 → current(0)+3=3 < cap → accept
    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), 3, 10);
    assert!(r.is_ok(), "below cap must accept: {r:?}");

    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(a.received_delegations[0].1, 3);
}

#[test]
fn test_cap_at_exact_boundary_accepts() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 10, 0);
    register_producer(&mut ps, &kp_d, 5, 1);

    // cap=5, requesting 5 → current(0)+5=5 == cap → accept (inclusive bound)
    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), 5, 5);
    assert!(
        r.is_ok(),
        "delegation hitting cap exactly must accept: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// I4: cap > 0, would exceed by one → reject. Single-shot violation.
// ---------------------------------------------------------------------------

#[test]
fn test_cap_exceeded_by_one_rejects() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 10, 0);
    register_producer(&mut ps, &kp_d, 10, 1);

    // cap=5, requesting 6 → 6 > 5 → reject
    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), 6, 5);
    assert!(
        r.is_err(),
        "delegation over cap must reject (got Ok unexpectedly)"
    );
    let msg = r.unwrap_err();
    assert!(
        msg.contains("delegation cap exceeded"),
        "error must mention cap; got: {msg}"
    );

    // O2: delegatee.received_delegations unchanged
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "rejected delegation must NOT mutate delegatee state"
    );
    // O3: delegator unchanged
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(
        d.delegated_bonds, 0,
        "rejected delegation must NOT mutate delegator state"
    );
    assert!(d.delegated_to.is_none());
}

// ---------------------------------------------------------------------------
// I5: Grandfathered producer (already over cap at activation) → can keep
// existing delegations but cannot receive new ones.
// ---------------------------------------------------------------------------

#[test]
fn test_grandfathered_over_cap_rejects_new_delegations() {
    let kp_a = KeyPair::generate(); // grandfathered "whale" delegatee
    let kp_d1 = KeyPair::generate(); // first delegator (pre-cap)
    let kp_d2 = KeyPair::generate(); // second delegator (post-cap activation)
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 10, 0);
    register_producer(&mut ps, &kp_d1, 8, 1);
    register_producer(&mut ps, &kp_d2, 8, 2);

    // Pre-activation: cap=0, large delegation lands and sets current_total=8
    ps.delegate_bonds(kp_d1.public_key(), kp_a.public_key(), 8)
        .unwrap();

    // Post-activation: cap=5 < current_total=8 → ANY new delegation rejected
    let r = ps.delegate_bonds_capped(kp_d2.public_key(), kp_a.public_key(), 1, 5);
    assert!(
        r.is_err(),
        "grandfathered over-cap producer must reject new delegations"
    );

    // O2: delegatee.received_delegations unchanged (only the pre-cap entry)
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(
        a.received_delegations.len(),
        1,
        "no new entry must be appended"
    );
    assert_eq!(a.received_delegations[0].1, 8);

    // But: existing pre-cap delegators are NOT forced to shed. Pre-cap entry
    // still present, current_total still 8 > cap. This is the Option-A
    // grandfather behavior from spec §2.5.
}

// ---------------------------------------------------------------------------
// I6: Race — two valid-individually delegations to the same producer that
// together exceed the cap. The defensive layer catches the second one
// (deterministically by order of application within the epoch).
// ---------------------------------------------------------------------------

#[test]
fn test_cap_race_two_delegators_second_rejected() {
    let kp_a = KeyPair::generate();
    let kp_d1 = KeyPair::generate();
    let kp_d2 = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 10, 0);
    register_producer(&mut ps, &kp_d1, 5, 1);
    register_producer(&mut ps, &kp_d2, 5, 2);

    // cap=6. Each delegator wants 4 bonds. Individually 0+4=4<=6 (accept) and
    // 0+4=4<=6 (accept). Sequenced: first lands (total=4), second tries to
    // land 4 more → 4+4=8 > 6 → second rejected.
    let r1 = ps.delegate_bonds_capped(kp_d1.public_key(), kp_a.public_key(), 4, 6);
    assert!(r1.is_ok(), "first delegation under cap must accept: {r1:?}");

    let r2 = ps.delegate_bonds_capped(kp_d2.public_key(), kp_a.public_key(), 4, 6);
    assert!(
        r2.is_err(),
        "second delegation that pushes sum over cap must reject"
    );

    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(a.received_delegations.len(), 1);
    assert_eq!(a.received_delegations[0].1, 4);

    // Second delegator's state must be untouched.
    let d2 = ps.get_by_pubkey(kp_d2.public_key()).unwrap();
    assert_eq!(d2.delegated_bonds, 0);
    assert!(d2.delegated_to.is_none());
}

// ---------------------------------------------------------------------------
// Boundary safety: huge bond_count with finite cap must reject via
// saturating arithmetic (no wrap).
// ---------------------------------------------------------------------------

#[test]
fn test_cap_saturating_arithmetic_huge_values() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, u32::MAX, 0);
    register_producer(&mut ps, &kp_d, u32::MAX, 1);

    // cap=1000, requesting u32::MAX → would overflow naive u64 add only at
    // 2^32 — our saturating add must produce a value > cap and reject.
    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), u32::MAX, 1000);
    assert!(
        r.is_err(),
        "huge bond_count vs finite cap must reject deterministically"
    );
}

// ---------------------------------------------------------------------------
// Idempotency / no-side-effects guarantee for rejected calls.
// ---------------------------------------------------------------------------

#[test]
fn test_cap_rejection_leaves_active_cache_intact() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();
    let mut ps = ProducerSet::new();
    register_producer(&mut ps, &kp_a, 5, 0);
    register_producer(&mut ps, &kp_d, 5, 1);

    // Prime active cache by reading it once
    let cached_before = ps.active_producers_at_height(0).len();

    // Try a rejected delegation
    let r = ps.delegate_bonds_capped(kp_d.public_key(), kp_a.public_key(), 10, 5);
    assert!(r.is_err());

    // active set must be unchanged after rejection
    let cached_after = ps.active_producers_at_height(0).len();
    assert_eq!(
        cached_before, cached_after,
        "rejected delegation must not invalidate or change active producer membership"
    );
}
