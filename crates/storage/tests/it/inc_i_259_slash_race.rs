//! INC-I-259: a slash admitted at the boundary is never lost to a withdrawal.
//!
//! covers: set_lifecycle.rs, info.rs, set_core.rs, set_registration.rs
//!
//! OUTPUT CONTRACT: fn slash_producer(&mut self, pubkey, height) -> Result<u64, StorageError>
//!   O2: target status, delegation fields of target and delegator, exit_history
//!   O3: Ok(bond_amount) | Err(STOR009 unknown) | Err(STOR041 already Slashed)
//! OUTPUT CONTRACT: fn ProducerInfo::apply_withdrawal(&mut self, count, unit)
//!   O2: bond_count, bond_entries, status
//! OUTPUT CONTRACT: fn register(&mut self, info, height) -> Result<(), StorageError>
//!   O3: Err(STOR011) for a slashed key, whatever the queue order was
//! INPUT PARTITIONS:
//!   slash_producer Ok path: status Active | Unbonding | Exited after a whole-bond withdrawal
//!   queue order at one boundary: Withdrawal then Slash | Slash then Withdrawal
//!   boundary split: Withdrawal at boundary k, Slash at boundary k+1
//!   apply_withdrawal to bond_count 0: status Slashed (kept) | other (Exited)

use crypto::{Hash, KeyPair};
use storage::{PendingProducerUpdate, ProducerInfo, ProducerSet, ProducerStatus};

const UNIT: u64 = 1_000_000_000;
const SLASH_H: u64 = 50;

struct Fixture {
    set: ProducerSet,
    target: KeyPair,
    delegator: KeyPair,
}

fn fixture() -> Fixture {
    let target = KeyPair::generate();
    let delegator = KeyPair::generate();
    let mut set = ProducerSet::new();
    let x = ProducerInfo::new_with_bonds(*target.public_key(), 0, UNIT, (Hash::ZERO, 0), 0, 1);
    let d =
        ProducerInfo::new_with_bonds(*delegator.public_key(), 0, 2 * UNIT, (Hash::ZERO, 1), 0, 2);
    set.register(x, 0).unwrap();
    set.register(d, 0).unwrap();
    set.delegate_bonds(delegator.public_key(), target.public_key(), 1)
        .unwrap();
    Fixture {
        set,
        target,
        delegator,
    }
}

fn queue_whole_withdrawal(f: &mut Fixture) {
    f.set
        .queue_update(PendingProducerUpdate::RequestWithdrawal {
            pubkey: *f.target.public_key(),
            bond_count: 1,
            bond_unit: UNIT,
        });
}

fn queue_slash(f: &mut Fixture) {
    f.set.queue_update(PendingProducerUpdate::Slash {
        pubkey: *f.target.public_key(),
        height: SLASH_H,
    });
}

fn slashed_at(h: u64) -> ProducerStatus {
    ProducerStatus::Slashed { slashed_at: h }
}

fn assert_slashed_for_life(f: &mut Fixture, bonds_left: u32) {
    let x = f.set.get_by_pubkey(f.target.public_key()).unwrap();
    assert_eq!(x.status, slashed_at(SLASH_H));
    assert_eq!(x.bond_count, bonds_left);
    assert!(x.received_delegations.is_empty());
    let d = f.set.get_by_pubkey(f.delegator.public_key()).unwrap();
    assert_eq!(d.delegated_to, None);
    assert_eq!(d.delegated_bonds, 0);

    let h = SLASH_H + 10;
    let again = ProducerInfo::new(*f.target.public_key(), h, UNIT, (Hash::ZERO, 2), 0, UNIT);
    let err = f.set.register(again, h).unwrap_err();
    assert!(err.to_string().contains("STOR011"), "got: {err}");
    assert_eq!(
        f.set.get_by_pubkey(f.target.public_key()).unwrap().status,
        slashed_at(SLASH_H)
    );
}

#[test]
fn inc_i_259_withdrawal_then_slash_in_one_boundary_ends_slashed() {
    let mut f = fixture();
    queue_whole_withdrawal(&mut f);
    queue_slash(&mut f);
    f.set.apply_pending_updates();
    assert_slashed_for_life(&mut f, 0);
}

#[test]
fn inc_i_259_slash_at_boundary_after_withdrawal_boundary_ends_slashed() {
    let mut f = fixture();
    queue_whole_withdrawal(&mut f);
    f.set.apply_pending_updates();
    let x = f.set.get_by_pubkey(f.target.public_key()).unwrap();
    assert_eq!(x.status, ProducerStatus::Exited);

    queue_slash(&mut f);
    f.set.apply_pending_updates();
    assert_slashed_for_life(&mut f, 0);
}

#[test]
fn inc_i_259_slash_then_withdrawal_in_one_boundary_stays_slashed() {
    let mut f = fixture();
    queue_slash(&mut f);
    queue_whole_withdrawal(&mut f);
    f.set.apply_pending_updates();
    assert_slashed_for_life(&mut f, 0);
}

#[test]
fn inc_i_259_apply_withdrawal_never_downgrades_slashed() {
    let kp = KeyPair::generate();
    let mut info =
        ProducerInfo::new_with_bonds(*kp.public_key(), 0, 2 * UNIT, (Hash::ZERO, 0), 0, 2);
    info.slash(SLASH_H);
    info.apply_withdrawal(2, UNIT);
    assert_eq!(info.bond_count, 0);
    assert!(info.bond_entries.is_empty());
    assert_eq!(info.status, slashed_at(SLASH_H));
}

#[test]
fn inc_i_259_apply_withdrawal_to_zero_still_exits_an_active_producer() {
    let kp = KeyPair::generate();
    let mut info =
        ProducerInfo::new_with_bonds(*kp.public_key(), 0, 2 * UNIT, (Hash::ZERO, 0), 0, 2);
    info.apply_withdrawal(1, UNIT);
    assert_eq!(info.status, ProducerStatus::Active);
    info.apply_withdrawal(1, UNIT);
    assert_eq!(info.status, ProducerStatus::Exited);
}

#[test]
fn inc_i_259_already_slashed_is_refused_and_keeps_first_height() {
    let mut f = fixture();
    let pk = *f.target.public_key();
    f.set.slash_producer(&pk, SLASH_H).unwrap();
    let err = f.set.slash_producer(&pk, SLASH_H + 5).unwrap_err();
    assert!(err.to_string().contains("STOR041"), "got: {err}");
    assert_slashed_for_life(&mut f, 1);
}

#[test]
fn inc_i_259_unbonding_producer_is_slashed() {
    let mut f = fixture();
    let pk = *f.target.public_key();
    f.set.request_exit(&pk, 10).unwrap();
    assert!(matches!(
        f.set.get_by_pubkey(&pk).unwrap().status,
        ProducerStatus::Unbonding { .. }
    ));
    assert_eq!(f.set.slash_producer(&pk, SLASH_H).unwrap(), UNIT);
    assert_eq!(
        f.set.get_by_pubkey(&pk).unwrap().status,
        slashed_at(SLASH_H)
    );
}

#[test]
fn inc_i_259_unknown_producer_is_refused() {
    let mut set = ProducerSet::new();
    let kp = KeyPair::generate();
    let err = set.slash_producer(kp.public_key(), SLASH_H).unwrap_err();
    assert!(err.to_string().contains("STOR009"), "got: {err}");
}
