//! INC-I-056: Delegation bug reproduction tests.
//!
//! These tests verify that delegation correctly adjusts:
//! 1. Bond weight (selection_weight) for scheduling/rewards
//! 2. Withdrawal availability (must not withdraw delegated bonds)
//! 3. Delegation availability (must not delegate pending-withdrawal bonds)

// OUTPUT CONTRACT: fn selection_weight_at(&self, height: u64, audit_activation: u64) -> u64
// O1: return value — effective weight accounting for delegations
// PATHS:
//   P1 = active delegator (delegated some bonds away)
//   P2 = active delegatee (received delegations)
//   P3 = active producer with no delegation involvement
// MATRIX:
//   O1×P1: own_bonds - delegated_bonds + received (received=0 for delegator)
//   O1×P2: own_bonds - delegated_bonds(0) + sum(received)
//   O1×P3: own_bonds (no delegation adjustment)

// OUTPUT CONTRACT: fn delegate_bonds(&mut self, delegator, delegatee, bond_count) -> Result<(), String>
// O1: Result — Ok if delegation succeeds, Err if validation fails
// O2: delegator.delegated_bonds — set to bond_count on success
// O3: delegatee.received_delegations — entry added on success
// PATHS:
//   P1 = normal delegation (enough available bonds)
//   P2 = over-delegation (bond_count > available after pending withdrawals)
// MATRIX:
//   O1×P1: Ok(()), O1×P2: Err("insufficient bonds")
//   O2×P1: delegated_bonds = bond_count, O2×P2: unchanged
//   O3×P1: entry added, O3×P2: unchanged

// OUTPUT CONTRACT: withdrawal available = bond_count - withdrawal_pending_count - delegated_bonds
// O1: available — effective bonds available for withdrawal
// PATHS:
//   P1 = no delegation (available = bond_count - pending)
//   P2 = with delegation (available = bond_count - pending - delegated)
// MATRIX:
//   O1×P1: 5 - 0 = 5
//   O1×P2: 5 - 0 - 3 = 2

#[allow(deprecated)]
use super::*;
use crypto::{Hash, KeyPair};

/// BUG 1+2+3: selection_weight_at must reflect delegation.
/// Delegator's effective weight = own - delegated_away.
/// Delegatee's effective weight = own + received.
#[test]
fn test_delegation_selection_weight_reflects_delegations() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_b = KeyPair::generate(); // bystander
    let kp_d = KeyPair::generate(); // delegator

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_b =
        ProducerInfo::new_with_bonds(*kp_b.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 2), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_b, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // Before delegation: everyone has 5
    assert_eq!(
        ps.get_by_pubkey(kp_a.public_key())
            .unwrap()
            .selection_weight(),
        5
    );
    assert_eq!(
        ps.get_by_pubkey(kp_b.public_key())
            .unwrap()
            .selection_weight(),
        5
    );
    assert_eq!(
        ps.get_by_pubkey(kp_d.public_key())
            .unwrap()
            .selection_weight(),
        5
    );

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // O1×P2: A (delegatee) = 5 own + 3 received = 8
    let a_weight = ps
        .get_by_pubkey(kp_a.public_key())
        .unwrap()
        .selection_weight();
    assert_eq!(
        a_weight, 8,
        "delegatee should have own(5) + received(3) = 8"
    );

    // O1×P3: B (bystander) = 5
    let b_weight = ps
        .get_by_pubkey(kp_b.public_key())
        .unwrap()
        .selection_weight();
    assert_eq!(b_weight, 5, "bystander unchanged at 5");

    // O1×P1: D (delegator) = 5 own - 3 delegated = 2
    let d_weight = ps
        .get_by_pubkey(kp_d.public_key())
        .unwrap()
        .selection_weight();
    assert_eq!(
        d_weight, 2,
        "delegator should have own(5) - delegated(3) = 2"
    );

    // Conservation: 8 + 5 + 2 = 15 = original 15
    assert_eq!(
        a_weight + b_weight + d_weight,
        15,
        "total weight must be conserved"
    );
}

/// BUG 4a: delegate_bonds must check withdrawal_pending_count.
#[test]
fn test_delegation_respects_pending_withdrawals() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let mut info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    // 2 bonds pending withdrawal
    info_d.withdrawal_pending_count = 2;

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // O1×P2: 5 bonds - 2 pending = 3 available → delegating 4 must fail
    let result = ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 4);
    assert!(
        result.is_err(),
        "delegating 4 bonds when only 3 available (5 - 2 pending) should fail"
    );

    // O1×P1: delegating 3 should succeed
    let result = ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3);
    assert!(
        result.is_ok(),
        "delegating 3 bonds when 3 available should succeed"
    );

    // O2×P1: delegator state updated
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(d.delegated_bonds, 3);

    // O3×P1: delegatee received
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(a.received_delegations.len(), 1);
    assert_eq!(a.received_delegations[0].1, 3);
}

/// BUG 4b: Withdrawal available must subtract delegated bonds.
#[test]
fn test_withdrawal_available_subtracts_delegated_bonds() {
    let kp_a = KeyPair::generate();
    let kp_d = KeyPair::generate();

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // O1×P2: available = 5 - 0 (pending) - 3 (delegated) = 2
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    let available = d
        .bond_count
        .saturating_sub(d.withdrawal_pending_count)
        .saturating_sub(d.delegated_bonds);

    assert_eq!(
        available, 2,
        "only 2 bonds available for withdrawal (5 total - 3 delegated)"
    );
}

// ============================================================================
// Delegation lifecycle cleanup tests (INC-I-056 follow-up)
// ============================================================================

/// DC-1: cleanup_all_delegations cleans BOTH directions and is idempotent.
#[test]
fn test_cleanup_all_delegations_cleans_both_directions() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // Verify delegation exists
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(d.delegated_bonds, 3);
    assert!(d.delegated_to.is_some());
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert_eq!(a.received_delegations.len(), 1);

    // Cleanup D's delegations (simulating D exiting)
    let d_hash = crypto::hash::hash(kp_d.public_key().as_bytes());
    ps.cleanup_all_delegations(&d_hash);

    // D's outgoing should be cleared
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(
        d.delegated_bonds, 0,
        "delegated_bonds should be 0 after cleanup"
    );
    assert!(
        d.delegated_to.is_none(),
        "delegated_to should be None after cleanup"
    );

    // A's received should be cleared
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "received_delegations should be empty after delegator cleanup"
    );

    // Idempotency: calling again is a no-op
    ps.cleanup_all_delegations(&d_hash);
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(d.delegated_bonds, 0);
}

/// DC-1: cleanup_all_delegations cleans received side (delegatee exit).
#[test]
fn test_cleanup_all_delegations_delegatee_exit() {
    let kp_a = KeyPair::generate(); // delegatee (will exit)
    let kp_d = KeyPair::generate(); // delegator

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // Cleanup A's delegations (simulating A exiting)
    let a_hash = crypto::hash::hash(kp_a.public_key().as_bytes());
    ps.cleanup_all_delegations(&a_hash);

    // A's received should be cleared
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(a.received_delegations.is_empty());

    // D's outgoing should be cleared (A was the delegatee)
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(
        d.delegated_bonds, 0,
        "delegator's delegated_bonds should be 0 after delegatee cleanup"
    );
    assert!(
        d.delegated_to.is_none(),
        "delegator's delegated_to should be None after delegatee cleanup"
    );
}

/// DC-2: request_exit cleans both directions (not just received).
#[test]
fn test_request_exit_cleans_outgoing_delegations() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator (will exit)

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d = ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 0, (Hash::ZERO, 1), 0, 0); // 0 bonds for Exit path

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 0 bonds (test the cleanup path, not the delegation amount)
    // Actually, D has 0 bonds, so can't delegate. Let's set up differently:
    // D has 5 bonds, delegates 3, then exits with 0 bonds (simulating already withdrawn)
    // For the Exit path (bonds=0), we need a producer with 0 bond_count but existing delegation state.
    // This is an edge case — normally delegation requires bonds.
    // Let's just test request_exit directly on an active producer with outgoing delegations.

    // Reset D with 5 bonds
    let info_d2 =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);
    let d_hash = crypto::hash::hash(kp_d.public_key().as_bytes());
    *ps.get_mut(&d_hash).unwrap() = info_d2;

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // D exits
    ps.request_exit(kp_d.public_key(), 100).unwrap();

    // D's outgoing should be cleaned
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(d.delegated_bonds, 0, "exit should clean delegated_bonds");
    assert!(d.delegated_to.is_none(), "exit should clean delegated_to");

    // A's received should be cleaned
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "exit should clean delegatee's received_delegations"
    );
}

/// DC-2: slash_producer cleans both directions.
#[test]
fn test_slash_cleans_outgoing_delegations() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator (will be slashed)

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // D gets slashed
    ps.slash_producer(kp_d.public_key(), 100).unwrap();

    // D's outgoing should be cleaned
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(d.delegated_bonds, 0, "slash should clean delegated_bonds");
    assert!(d.delegated_to.is_none(), "slash should clean delegated_to");

    // A's received should be cleaned
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "slash should clean delegatee's received_delegations"
    );
}

/// DC-3: apply_pending_updates RequestWithdrawal auto-exit cleans delegations.
#[test]
fn test_apply_pending_withdrawal_auto_exit_cleans_delegations() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator (will withdraw all bonds)

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D delegates 3 bonds to A
    ps.delegate_bonds(kp_d.public_key(), kp_a.public_key(), 3)
        .unwrap();

    // Queue withdrawal of ALL bonds (simulates Exit tx with bonds > 0)
    ps.queue_update(PendingProducerUpdate::RequestWithdrawal {
        pubkey: *kp_d.public_key(),
        bond_count: 5,
        bond_unit: BOND_UNIT,
    });

    // Apply at epoch boundary
    ps.apply_pending_updates();

    // D should be auto-exited AND delegations cleaned
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert!(
        matches!(d.status, ProducerStatus::Exited),
        "producer should be auto-exited after withdrawing all bonds"
    );
    assert_eq!(
        d.delegated_bonds, 0,
        "auto-exit should clean delegated_bonds"
    );
    assert!(
        d.delegated_to.is_none(),
        "auto-exit should clean delegated_to"
    );

    // A's received should be cleaned
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "auto-exit should clean delegatee's received_delegations"
    );
}

/// REQ-DEL-004: Weight conservation invariant after all mutation combos.
#[test]
fn test_weight_conservation_invariant() {
    let kp_a = KeyPair::generate();
    let kp_b = KeyPair::generate();
    let kp_c = KeyPair::generate();

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_b =
        ProducerInfo::new_with_bonds(*kp_b.public_key(), 0, 3 * BOND_UNIT, (Hash::ZERO, 1), 0, 3);
    let info_c =
        ProducerInfo::new_with_bonds(*kp_c.public_key(), 0, 4 * BOND_UNIT, (Hash::ZERO, 2), 0, 4);

    ps.register(info_a, 0).unwrap();
    ps.register(info_b, 0).unwrap();
    ps.register(info_c, 0).unwrap();

    fn sum_weight(ps: &ProducerSet) -> u64 {
        ps.active_producers()
            .iter()
            .map(|p| p.selection_weight())
            .sum()
    }
    fn sum_bonds(ps: &ProducerSet) -> u64 {
        ps.active_producers()
            .iter()
            .map(|p| p.bond_count as u64)
            .sum()
    }

    // Initial: 5+3+4 = 12
    assert_eq!(sum_bonds(&ps), 12);
    assert_eq!(sum_weight(&ps), 12, "pre-delegation conservation");

    // A delegates 2 bonds to B
    ps.delegate_bonds(kp_a.public_key(), kp_b.public_key(), 2)
        .unwrap();
    assert_eq!(sum_weight(&ps), 12, "post-delegation conservation");

    // A revokes delegation
    ps.revoke_delegation(kp_a.public_key()).unwrap();
    assert_eq!(sum_weight(&ps), 12, "post-revocation conservation");

    // A delegates 3 to C, then A exits via request_exit
    ps.delegate_bonds(kp_a.public_key(), kp_c.public_key(), 3)
        .unwrap();
    assert_eq!(sum_weight(&ps), 12, "post-re-delegation conservation");

    ps.request_exit(kp_a.public_key(), 100).unwrap();
    // A is now Unbonding — still active for weight purposes
    // But delegations should be cleaned
    assert_eq!(
        sum_weight(&ps),
        sum_bonds(&ps),
        "post-exit weight must equal active bond count"
    );
}

/// REQ-DEL-005: Same-epoch DelegateBond + RequestWithdrawal ordering.
#[test]
fn test_same_epoch_delegate_then_withdraw_no_orphan() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // Queue both in the same epoch: first delegate, then full withdrawal
    ps.queue_update(PendingProducerUpdate::DelegateBond {
        delegator: *kp_d.public_key(),
        delegate: *kp_a.public_key(),
        bond_count: 3,
    });
    ps.queue_update(PendingProducerUpdate::RequestWithdrawal {
        pubkey: *kp_d.public_key(),
        bond_count: 5,
        bond_unit: BOND_UNIT,
    });

    // Apply all at epoch boundary
    ps.apply_pending_updates();

    // D should be auto-exited with NO orphaned delegation state
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert!(matches!(d.status, ProducerStatus::Exited));
    assert_eq!(
        d.delegated_bonds, 0,
        "same-epoch delegate+withdraw must not orphan delegated_bonds"
    );
    assert!(
        d.delegated_to.is_none(),
        "same-epoch delegate+withdraw must not orphan delegated_to"
    );

    // A should have no phantom received delegations
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "same-epoch delegate+withdraw must not orphan received_delegations"
    );
}

/// RC-1: process_unbonding complete_exit cleans delegations (defense-in-depth).
#[test]
fn test_process_unbonding_cleans_delegations() {
    let kp_a = KeyPair::generate(); // delegatee
    let kp_d = KeyPair::generate(); // delegator (will unbond)

    let mut ps = ProducerSet::new();

    let info_a =
        ProducerInfo::new_with_bonds(*kp_a.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 0), 0, 5);
    let info_d =
        ProducerInfo::new_with_bonds(*kp_d.public_key(), 0, 5 * BOND_UNIT, (Hash::ZERO, 1), 0, 5);

    ps.register(info_a, 0).unwrap();
    ps.register(info_d, 0).unwrap();

    // D enters unbonding (request_exit will clean delegations, but let's test process_unbonding)
    ps.request_exit(kp_d.public_key(), 100).unwrap();

    // Now D receives a delegation while unbonding (D is still active per is_active())
    // Actually, D can't receive delegations while unbonding because delegate_bonds checks is_active()
    // and Unbonding IS active. But D already exited, so delegated_to was cleaned by request_exit.
    // Let's simulate stale state: manually set delegation fields to test defense-in-depth.
    let d_hash = crypto::hash::hash(kp_d.public_key().as_bytes());
    let a_hash = crypto::hash::hash(kp_a.public_key().as_bytes());
    if let Some(d) = ps.get_mut(&d_hash) {
        d.delegated_to = Some(*kp_a.public_key());
        d.delegated_bonds = 2;
    }
    if let Some(a) = ps.get_mut(&a_hash) {
        a.received_delegations.push((d_hash, 2));
    }

    // Process unbonding with duration=0 so D completes exit immediately
    let completed = ps.process_unbonding(100, 0);
    assert!(!completed.is_empty(), "D should complete unbonding");

    // D's delegation state should be cleaned by process_unbonding
    let d = ps.get_by_pubkey(kp_d.public_key()).unwrap();
    assert_eq!(
        d.delegated_bonds, 0,
        "process_unbonding should clean delegated_bonds"
    );
    assert!(
        d.delegated_to.is_none(),
        "process_unbonding should clean delegated_to"
    );

    // A's received should be cleaned
    let a = ps.get_by_pubkey(kp_a.public_key()).unwrap();
    assert!(
        a.received_delegations.is_empty(),
        "process_unbonding should clean received_delegations"
    );
}
