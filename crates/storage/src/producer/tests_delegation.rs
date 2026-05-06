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
