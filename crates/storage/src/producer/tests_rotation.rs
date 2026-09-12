//! INC-I-217 M7 — rotation verdict, queueing and epoch-boundary flush.
//!
//! covers: REQ-ROT-005, REQ-ROT-006, REQ-ROT-008, REQ-ROT-SEC-004,
//! REQ-ROT-SEC-005, REQ-ROT-SEC-006
//!
//! The node-level queue-at-apply path is unreachable while
//! `bls_key_rotation_activation_height` is `u64::MAX` (the D6 block gate rejects
//! any carrying block first), so this module carries the whole M7 burden.
//!
//! ## OUTPUT CONTRACT
//!
//! F1: `rotate_verdict(&RotateBlsData, &RotationInputs) -> Result<(), RotateSkip>`
//!   O1 return: `Ok(())` or the FIRST tripped `RotateSkip`
//!   O2 mutable params: NONE (both args are `&`)
//!   O3 receiver / O4 persistent store / O5 statics / O6 channels: NONE
//!
//! F2: `ProducerSet::resolve_rotation_inputs(&self, &RotateBlsData) -> RotationInputs`
//!   O1 return: the four stateful facts
//!   O2..O6: NONE — `&self`
//!
//! F3: `apply_rotation(&mut ProducerSet, &RotateBlsData, u64) -> Result<(), RotateSkip>`
//!   O1 return: `Ok(())` or the skip reason
//!   O2 receiver mutation: `pending_updates` gains exactly one `RotateBlsKey` on `Ok`,
//!      and is untouched on every `Err`
//!   O3 persistent store: NONE (queue lives in memory until `save`)
//!
//! F4: `ProducerSet::apply_pending_updates_with_cap(&mut self, u64)`
//!   O1 return: `()`
//!   O2 receiver mutation: `producers[pubkey].bls_pubkey` set to the queued target,
//!      UNCONDITIONALLY on status; `pending_updates` drained; `active_cache` cleared
//!   O3 `serialize_canonical()`: unchanged before the flush, changed after
//!
//! PATHS (of the verdict, in evaluation order — a wire contract):
//!   P1 NotProducer   P2 SameKey   P3 AlreadyPending   P4 KeyInUse   P5 Ok
//!
//! INPUT PARTITIONS:
//!   I1 sender absent from the map and absent from the queue
//!   I2 sender `Active` in the map
//!   I3 sender in the map with status `Unbonding` / `Exited` / `Slashed`
//!   I4 sender only as a queued `Register`
//!   I5 target key == sender's own current key
//!   I6 target key == another producer's current key
//!   I7 target key == another queued `RotateBlsKey` target
//!   I8 target key == a queued `Register`'s `info.bls_pubkey`
//!   I9 a `RotateBlsKey` already queued for the sender
//!   I10 several of I1/I5/I8/I9 tripped at once (the order pins)
//!
//! MATRIX: every (P, I) cell reachable from the rules has a named test below;
//! the flush half adds status x {Active, Unbonding, Exited, Slashed, absent} and
//! the four REQ-ROT-008 queue orderings.

#[allow(deprecated)]
use super::*;
use doli_core::transaction::RotateBlsData;

use crypto::{Hash, KeyPair};

// ===== Fixtures =====

fn kp(seed: u8) -> KeyPair {
    KeyPair::from_seed([seed; 32])
}

/// A rotation payload. Crypto fields are zero on purpose: since M5 every
/// crypto/shape failure is a `ValidationError` that kills the block, so nothing
/// in this module may read them.
fn rot(producer: &KeyPair, target: u8) -> RotateBlsData {
    RotateBlsData {
        producer: *producer.public_key().as_bytes(),
        new_bls_pubkey: [target; 48],
        bls_pop: [0u8; 96],
        signature: [0u8; 64],
    }
}

fn info_with_bls(k: &KeyPair, bls: Option<u8>, idx: u32) -> ProducerInfo {
    let mut info = ProducerInfo::new(
        *k.public_key(),
        0,
        BOND_UNIT,
        (Hash::ZERO, idx),
        0,
        BOND_UNIT,
    );
    info.bls_pubkey = bls.map(|b| vec![b; 48]).unwrap_or_default();
    info
}

/// Register `k` as `Active` carrying BLS key `bls`.
fn register(ps: &mut ProducerSet, k: &KeyPair, bls: Option<u8>, idx: u32) {
    ps.register(info_with_bls(k, bls, idx), 0).unwrap();
}

/// Queue a `Register` without touching the map — the SEC-006 "pending register"
/// partition (I4) and the D8 "queued register key" partition (I8).
fn queue_register(ps: &mut ProducerSet, k: &KeyPair, bls: Option<u8>, idx: u32) {
    ps.queue_update(PendingProducerUpdate::Register {
        info: Box::new(info_with_bls(k, bls, idx)),
        height: 10,
    });
}

fn set_status(ps: &mut ProducerSet, k: &KeyPair, status: ProducerStatus) {
    ps.get_by_pubkey_mut(k.public_key()).unwrap().status = status;
}

fn verdict(ps: &ProducerSet, data: &RotateBlsData) -> Result<(), RotateSkip> {
    rotate_verdict(data, &ps.resolve_rotation_inputs(data))
}

fn bls_of(ps: &ProducerSet, k: &KeyPair) -> Vec<u8> {
    ps.get_by_pubkey(k.public_key()).unwrap().bls_pubkey.clone()
}

// ===== F1/F2 — verdict x sender state (REQ-ROT-SEC-006) =====

// REQ-ROT-SEC-006 — Decision: whether a payload naming a key that was never registered can
// queue a mutation, which would let anyone plant a pending update for a pubkey they invented.
#[test]
fn absent_producer_is_not_a_producer() {
    let ps = ProducerSet::new();
    assert_eq!(
        verdict(&ps, &rot(&kp(1), 0xAA)),
        Err(RotateSkip::NotProducer)
    );
}

// REQ-ROT-SEC-006 — Decision: whether the ordinary, whole-point case is accepted at all;
// if this fails the feature is inert and every other row below is vacuous.
#[test]
fn active_producer_rotating_to_a_fresh_key_is_accepted() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    assert_eq!(verdict(&ps, &rot(&a, 0xAA)), Ok(()));
}

// REQ-ROT-SEC-006 — Decision: whether a producer on its way out can still install a key,
// which would let an exiting operator hand live attestation authority to a third party.
#[test]
fn unbonding_producer_is_rejected() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    set_status(&mut ps, &a, ProducerStatus::Unbonding { started_at: 5 });
    assert_eq!(
        verdict(&ps, &rot(&a, 0xAA)),
        Err(RotateSkip::NotProducer),
        "SEC-006 names Unbonding as non-eligible alongside Exited"
    );
}

// REQ-ROT-SEC-006 — Decision: same threat as Unbonding, one status further along.
#[test]
fn exited_producer_is_rejected() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    set_status(&mut ps, &a, ProducerStatus::Exited);
    assert_eq!(verdict(&ps, &rot(&a, 0xAA)), Err(RotateSkip::NotProducer));
}

// REQ-ROT-SEC-006 — Decision: whether a slashed operator can re-key and re-enter, which
// would make slashing recoverable by a 240-byte transaction.
#[test]
fn slashed_producer_is_rejected() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    set_status(&mut ps, &a, ProducerStatus::Slashed { slashed_at: 7 });
    assert_eq!(verdict(&ps, &rot(&a, 0xAA)), Err(RotateSkip::NotProducer));
}

// REQ-ROT-SEC-006 — Decision: whether the eligibility scan looks only at the map, which
// would reject a producer that registered and rotated inside the same epoch.
#[test]
fn producer_with_only_a_queued_register_is_accepted() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    queue_register(&mut ps, &a, Some(0x11), 0);

    assert!(ps.get_by_pubkey(a.public_key()).is_none());
    assert_eq!(verdict(&ps, &rot(&a, 0xAA)), Ok(()));
}

// REQ-ROT-SEC-006 — Decision: whether `current_bls` falls back to the queued Register, i.e.
// whether a same-key no-op is detectable before the producer exists in the map.
#[test]
fn queued_register_bls_key_is_what_same_key_compares_against() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    queue_register(&mut ps, &a, Some(0x11), 0);

    assert_eq!(
        ps.resolve_rotation_inputs(&rot(&a, 0x11)).current_bls,
        Some(vec![0x11u8; 48])
    );
    assert_eq!(verdict(&ps, &rot(&a, 0x11)), Err(RotateSkip::SameKey));
}

// ===== F1 — SameKey (REQ-ROT-SEC-004, third bullet) =====

// REQ-ROT-SEC-004 — Decision: whether a no-op rotation is distinguished from a collision;
// charging KeyInUse here would tell an operator its own key belongs to someone else.
#[test]
fn rotating_to_own_current_key_is_same_key_not_key_in_use() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    assert_eq!(verdict(&ps, &rot(&a, 0x11)), Err(RotateSkip::SameKey));
}

// ===== F1 — AlreadyPending (REQ-ROT-SEC-005) =====

// REQ-ROT-SEC-005 — Decision: whether a producer can stack rotations inside one deferral
// window, which makes the key installed at the boundary depend on queue order, not intent.
// Also the SEC-003 "mined at h, resubmitted at h+1" case.
#[test]
fn second_rotation_inside_the_deferral_window_is_already_pending() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    assert_eq!(apply_rotation(&mut ps, &rot(&a, 0xAA), 100), Ok(()));

    assert_eq!(
        verdict(&ps, &rot(&a, 0xBB)),
        Err(RotateSkip::AlreadyPending),
        "a rotation resubmitted at h+1 must be refused while the first is queued"
    );
}

// ===== F1 — KeyInUse (REQ-ROT-SEC-004, D8) =====

// REQ-ROT-SEC-004 — Decision: whether two producers can end an epoch sharing one BLS key,
// which makes an aggregate signature unattributable and the bitfield index ambiguous.
#[test]
fn target_equal_to_another_producers_current_key_is_key_in_use() {
    let (a, v) = (kp(1), kp(2));
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    register(&mut ps, &v, Some(0x22), 1);
    assert_eq!(verdict(&ps, &rot(&a, 0x22)), Err(RotateSkip::KeyInUse));
}

// REQ-ROT-SEC-004 — Decision: whether the scan covers the queue, i.e. whether two producers
// racing to the same key in one epoch both land at the boundary.
#[test]
fn target_equal_to_another_queued_rotation_target_is_key_in_use() {
    let (a, b) = (kp(1), kp(2));
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    register(&mut ps, &b, Some(0x22), 1);
    assert_eq!(apply_rotation(&mut ps, &rot(&a, 0xAA), 100), Ok(()));
    assert_eq!(verdict(&ps, &rot(&b, 0xAA)), Err(RotateSkip::KeyInUse));
}

// REQ-ROT-SEC-004 — Decision: whether an incoming registration's key is reserved, i.e.
// whether a rotation can claim the key of a producer that registers at the same boundary.
#[test]
fn target_equal_to_a_queued_registers_key_is_key_in_use() {
    let (a, n) = (kp(1), kp(2));
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    queue_register(&mut ps, &n, Some(0xCC), 1);
    assert_eq!(verdict(&ps, &rot(&a, 0xCC)), Err(RotateSkip::KeyInUse));
}

// REQ-ROT-SEC-004 — Decision: whether the O(n) scan leaked HashMap iteration order into the
// verdict, which forks nodes holding identical logical state.
#[test]
fn key_in_use_scan_is_order_independent() {
    let a = kp(1);
    let mut forward = ProducerSet::new();
    register(&mut forward, &a, Some(0x11), 0);
    for (i, seed) in (2u8..12).enumerate() {
        register(&mut forward, &kp(seed), Some(0x20 + seed), i as u32 + 1);
    }

    let mut reverse = ProducerSet::new();
    for (i, seed) in (2u8..12).rev().enumerate() {
        register(&mut reverse, &kp(seed), Some(0x20 + seed), 10 - i as u32);
    }
    register(&mut reverse, &a, Some(0x11), 0);

    for target in [0x11u8, 0x25, 0xFE] {
        assert_eq!(
            verdict(&forward, &rot(&a, target)),
            verdict(&reverse, &rot(&a, target)),
            "verdict for target 0x{target:02x} depended on insertion order"
        );
    }
}

// ===== F1 — evaluation ORDER (wire contract; M8's rebuild must reproduce it) =====

// REQ-ROT-008 — Decision: whether the first reason reported is NotProducer when several
// trip. The order is a wire contract — M8 rebuilds verdicts from the same inputs, and a
// different reason there means a different log line, a different metric and, once any
// reason ever becomes consensus-visible, a different chain.
#[test]
fn order_pin_not_producer_wins_over_every_other_reason() {
    let ghost = kp(9);
    let mut ps = ProducerSet::new();
    // Ghost is absent from the map AND absent from the queue (NotProducer),
    // the target is already held by V (KeyInUse), and a rotation for ghost is
    // already queued (AlreadyPending).
    register(&mut ps, &kp(2), Some(0x22), 0);
    ps.queue_update(PendingProducerUpdate::RotateBlsKey {
        pubkey: *ghost.public_key(),
        new_bls_pubkey: vec![0x33u8; 48],
        height: 90,
    });

    assert_eq!(
        verdict(&ps, &rot(&ghost, 0x22)),
        Err(RotateSkip::NotProducer),
        "order is NotProducer -> SameKey -> AlreadyPending -> KeyInUse"
    );
}

// REQ-ROT-008 — Decision: whether SameKey outranks AlreadyPending and KeyInUse.
#[test]
fn order_pin_same_key_wins_over_already_pending_and_key_in_use() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    // A rotation for A is already queued, and the target is A's own key, which
    // the O(n) scan also sees as "in use by a producer".
    ps.queue_update(PendingProducerUpdate::RotateBlsKey {
        pubkey: *a.public_key(),
        new_bls_pubkey: vec![0xAAu8; 48],
        height: 90,
    });

    assert_eq!(verdict(&ps, &rot(&a, 0x11)), Err(RotateSkip::SameKey));
}

// REQ-ROT-008 — Decision: whether AlreadyPending outranks KeyInUse.
#[test]
fn order_pin_already_pending_wins_over_key_in_use() {
    let (a, v) = (kp(1), kp(2));
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    register(&mut ps, &v, Some(0x22), 1);
    ps.queue_update(PendingProducerUpdate::RotateBlsKey {
        pubkey: *a.public_key(),
        new_bls_pubkey: vec![0xAAu8; 48],
        height: 90,
    });

    assert_eq!(
        verdict(&ps, &rot(&a, 0x22)),
        Err(RotateSkip::AlreadyPending)
    );
}

// REQ-ROT-008 — Decision: whether the verdict is a function of its arguments alone. If it
// is not, M8's rebuild can reach a different answer from the same bytes.
#[test]
fn rotate_verdict_is_pure() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    register(&mut ps, &kp(2), Some(0x22), 1);

    let before = ps.serialize_canonical();
    for data in [
        rot(&a, 0xAA),
        rot(&a, 0x11),
        rot(&a, 0x22),
        rot(&kp(9), 0x33),
    ] {
        let inputs = ps.resolve_rotation_inputs(&data);
        let first = rotate_verdict(&data, &inputs);
        assert_eq!(
            first,
            rotate_verdict(&data, &inputs),
            "verdict is not stable"
        );
        assert_eq!(first, rotate_verdict(&data, &inputs));
    }
    assert_eq!(
        ps.serialize_canonical(),
        before,
        "resolving inputs or taking a verdict mutated the producer set"
    );
}

// ===== F3 — apply_rotation (queueing; REQ-ROT-008 / D3 "skip means no state change") =====

// REQ-ROT-008 — Decision: whether an accepted rotation queues exactly one update carrying
// the exact pubkey, target and height the boundary will replay.
#[test]
fn accepted_rotation_queues_exactly_one_update() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    let before = ps.pending_update_count();

    assert_eq!(apply_rotation(&mut ps, &rot(&a, 0xAA), 777), Ok(()));
    assert_eq!(ps.pending_update_count(), before + 1);

    let queued = ps.pending_updates_for(a.public_key());
    assert_eq!(queued.len(), 1);
    match queued[0] {
        PendingProducerUpdate::RotateBlsKey {
            pubkey,
            new_bls_pubkey,
            height,
        } => {
            assert_eq!(pubkey, a.public_key());
            assert_eq!(new_bls_pubkey, &vec![0xAAu8; 48]);
            assert_eq!(*height, 777);
        }
        other => panic!("expected RotateBlsKey, got {other:?}"),
    }
}

// REQ-ROT-008 — Decision: whether ANY skip reason leaves residue. A skip that queues, logs
// a counter or clears a cache is a state change that only one of the two apply paths makes,
// which is the shape of a rebuild divergence.
#[test]
fn every_skip_reason_leaves_the_set_byte_identical() {
    let (a, v, ghost) = (kp(1), kp(2), kp(9));
    let mut base = ProducerSet::new();
    register(&mut base, &a, Some(0x11), 0);
    register(&mut base, &v, Some(0x22), 1);

    let cases: [(&str, RotateBlsData, RotateSkip); 4] = [
        ("NotProducer", rot(&ghost, 0xAA), RotateSkip::NotProducer),
        ("SameKey", rot(&a, 0x11), RotateSkip::SameKey),
        ("KeyInUse", rot(&a, 0x22), RotateSkip::KeyInUse),
        ("AlreadyPending", rot(&a, 0xBB), RotateSkip::AlreadyPending),
    ];

    for (name, data, expected) in cases {
        let mut ps = base.clone();
        if expected == RotateSkip::AlreadyPending {
            apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();
        }
        let count_before = ps.pending_update_count();
        let bytes_before = ps.serialize_canonical();

        assert_eq!(apply_rotation(&mut ps, &data, 101), Err(expected), "{name}");
        assert_eq!(ps.pending_update_count(), count_before, "{name} queued");
        assert_eq!(
            ps.serialize_canonical(),
            bytes_before,
            "{name} mutated state"
        );
    }
}

// REQ-ROT-SEC-005 — Decision: whether the intra-block duplicate is caught by sequential
// apply alone, which is the whole reason D3 could drop the rejecting block rule.
#[test]
fn two_rotations_for_one_producer_in_one_block_queue_only_the_first() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);

    assert_eq!(apply_rotation(&mut ps, &rot(&a, 0xAA), 100), Ok(()));
    assert_eq!(
        apply_rotation(&mut ps, &rot(&a, 0xBB), 100),
        Err(RotateSkip::AlreadyPending)
    );
    assert_eq!(ps.pending_updates_for(a.public_key()).len(), 1);
}

// REQ-ROT-SEC-004 — Decision: whether the D8 race is closed by sequential apply, i.e.
// whether two producers naming the same target in one block both reach the boundary.
#[test]
fn two_producers_racing_to_one_key_in_one_block_only_the_first_queues() {
    let (a, b) = (kp(1), kp(2));
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    register(&mut ps, &b, Some(0x22), 1);

    assert_eq!(apply_rotation(&mut ps, &rot(&a, 0xAA), 100), Ok(()));
    assert_eq!(
        apply_rotation(&mut ps, &rot(&b, 0xAA), 100),
        Err(RotateSkip::KeyInUse)
    );
    assert_eq!(ps.pending_update_count(), 1);
}

// ===== F4 — epoch-boundary flush (REQ-ROT-005, D4) =====

// REQ-ROT-005 — Decision: whether the queued rotation actually reaches `bls_pubkey`. This
// is the milestone: without it M7 is a queue that nothing drains.
#[test]
fn queued_rotation_changes_the_key_at_the_boundary() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();

    ps.apply_pending_updates();

    assert_eq!(bls_of(&ps, &a), vec![0xAAu8; 48]);
    println!("ROT-KEY-CHANGED-AT-BOUNDARY");
}

// REQ-ROT-005 — Decision: whether the rotation is rooted BEFORE the boundary. The queue is
// outside `serialize_canonical`, so a key that moves at queue time would change the state
// root at a height where un-upgraded nodes compute the old one.
#[test]
fn key_and_state_root_are_frozen_until_the_boundary() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    let pre_queue = ps.serialize_canonical();

    apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();
    assert_eq!(
        bls_of(&ps, &a),
        vec![0x11u8; 48],
        "key moved before the boundary"
    );
    assert_eq!(
        ps.serialize_canonical(),
        pre_queue,
        "queueing a rotation changed the producer contribution to the state root"
    );

    ps.apply_pending_updates();
    assert_ne!(
        ps.serialize_canonical(),
        pre_queue,
        "the boundary flush did not reach the state root"
    );
}

// REQ-ROT-005 / D4 (skeptic A6) — Decision: whether the flush arm grew a status branch.
// A branch here makes apply and M8's rebuild disagree whenever the status moved between
// queue and flush, which is exactly the `[Exit, Rotate]` case below.
#[test]
fn flush_is_unconditional_on_status() {
    let statuses = [
        ("Unbonding", ProducerStatus::Unbonding { started_at: 5 }),
        ("Exited", ProducerStatus::Exited),
        ("Slashed", ProducerStatus::Slashed { slashed_at: 7 }),
    ];

    for (name, status) in statuses {
        let a = kp(1);
        let mut ps = ProducerSet::new();
        register(&mut ps, &a, Some(0x11), 0);
        apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();

        // Status moves AFTER the rotation is queued, BEFORE the boundary.
        set_status(&mut ps, &a, status);
        ps.apply_pending_updates();

        assert_eq!(
            bls_of(&ps, &a),
            vec![0xAAu8; 48],
            "{name} producer did not receive the queued key — the arm is status-branched"
        );
    }
}

// REQ-ROT-008 — Decision: whether a queued rotation for a producer that is no longer in the
// map panics or corrupts. Nothing removes producers today, so this is the guard against a
// future remover turning a boundary into a fleet-wide panic.
#[test]
fn flush_for_an_absent_producer_is_a_silent_drain() {
    let ghost = kp(9);
    let mut ps = ProducerSet::new();
    register(&mut ps, &kp(1), Some(0x11), 0);
    ps.queue_update(PendingProducerUpdate::RotateBlsKey {
        pubkey: *ghost.public_key(),
        new_bls_pubkey: vec![0xAAu8; 48],
        height: 100,
    });
    let before = ps.serialize_canonical();

    ps.apply_pending_updates();

    assert_eq!(
        ps.serialize_canonical(),
        before,
        "an absent target changed state"
    );
    assert!(!ps.has_pending_updates());
}

// REQ-ROT-005 — Decision: whether the rotation arm drains its entry. A retained entry
// re-applies at every subsequent boundary and pins the key against later rotations.
#[test]
fn the_queue_is_fully_drained_after_the_flush() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();

    ps.apply_pending_updates();

    assert!(!ps.has_pending_updates());
    assert_eq!(ps.pending_update_count(), 0);
    assert!(ps.pending_updates_for(a.public_key()).is_empty());
}

// ===== REQ-ROT-008 — boundary ORDERING (the M8 determinism contract) =====

/// Build `[first, second]` in the given order, flush, return the canonical bytes
/// plus the observable producer facts.
fn flush_ordered(
    a: &KeyPair,
    order: [PendingProducerUpdate; 2],
) -> (Vec<u8>, ProducerStatus, Vec<u8>) {
    let mut ps = ProducerSet::new();
    register(&mut ps, a, Some(0x11), 0);
    for u in order {
        ps.queue_update(u);
    }
    ps.apply_pending_updates();
    let info = ps.get_by_pubkey(a.public_key()).expect("producer survives");
    (
        ps.serialize_canonical(),
        info.status,
        info.bls_pubkey.clone(),
    )
}

fn exit_of(a: &KeyPair) -> PendingProducerUpdate {
    PendingProducerUpdate::Exit {
        pubkey: *a.public_key(),
        height: 100,
    }
}

fn slash_of(a: &KeyPair) -> PendingProducerUpdate {
    PendingProducerUpdate::Slash {
        pubkey: *a.public_key(),
        height: 100,
    }
}

fn rotate_of(a: &KeyPair, target: u8) -> PendingProducerUpdate {
    PendingProducerUpdate::RotateBlsKey {
        pubkey: *a.public_key(),
        new_bls_pubkey: vec![target; 48],
        height: 100,
    }
}

// REQ-ROT-008 — Decision: what an Exit and a rotation for the SAME producer in one epoch
// produce, and whether the two arrival orders commute. If they do not commute, the queue
// order is consensus-visible and M8's rebuild must reproduce it exactly.
#[test]
fn exit_and_rotation_in_one_epoch_commute() {
    let a = kp(1);
    let (bytes_ex, status_ex, key_ex) = flush_ordered(&a, [exit_of(&a), rotate_of(&a, 0xAA)]);
    let (bytes_re, status_re, key_re) = flush_ordered(&a, [rotate_of(&a, 0xAA), exit_of(&a)]);

    assert_eq!(
        status_ex,
        ProducerStatus::Unbonding { started_at: 100 },
        "[Exit, Rotate]: the exit must still take effect"
    );
    assert_eq!(
        key_ex,
        vec![0xAAu8; 48],
        "[Exit, Rotate]: the unconditional arm must still install the key"
    );
    assert_eq!((status_re, &key_re), (status_ex, &key_ex));
    assert_eq!(
        bytes_ex, bytes_re,
        "Exit and RotateBlsKey do NOT commute at the boundary. If this is the real \
         behaviour, pin the exact difference here instead of asserting equality — the \
         arrival order has become consensus-visible and M8's rebuild must reproduce it."
    );
}

// REQ-ROT-008 — Decision: same question for Slash, whose arm also mutates only `status`.
#[test]
fn slash_and_rotation_in_one_epoch_commute() {
    let a = kp(1);
    let (bytes_sl, status_sl, key_sl) = flush_ordered(&a, [slash_of(&a), rotate_of(&a, 0xAA)]);
    let (bytes_re, status_re, key_re) = flush_ordered(&a, [rotate_of(&a, 0xAA), slash_of(&a)]);

    assert_eq!(status_sl, ProducerStatus::Slashed { slashed_at: 100 });
    assert_eq!(key_sl, vec![0xAAu8; 48]);
    assert_eq!((status_re, &key_re), (status_sl, &key_sl));
    assert_eq!(
        bytes_sl, bytes_re,
        "Slash and RotateBlsKey do NOT commute at the boundary — pin the exact difference"
    );
}

// REQ-ROT-SEC-006 — Decision: whether a producer that registers and rotates inside one
// epoch ends up registered with the NEW key. The Register arm inserts the info wholesale,
// so a rotation applied first would be overwritten.
#[test]
fn register_then_rotation_lands_registered_with_the_new_key() {
    let n = kp(3);
    let mut ps = ProducerSet::new();
    queue_register(&mut ps, &n, Some(0x33), 0);
    ps.queue_update(rotate_of(&n, 0xAA));

    ps.apply_pending_updates();

    let info = ps.get_by_pubkey(n.public_key()).expect("register applied");
    assert_eq!(info.status, ProducerStatus::Active);
    assert_eq!(info.bls_pubkey, vec![0xAAu8; 48]);
}

// REQ-ROT-008 — Decision: whether any arrival order of the four arms can panic at a
// boundary. A panic here halts every node in the fleet at the same height.
#[test]
fn no_queue_ordering_panics_at_the_boundary() {
    let a = kp(1);
    let orders: [[PendingProducerUpdate; 2]; 6] = [
        [exit_of(&a), rotate_of(&a, 0xAA)],
        [rotate_of(&a, 0xAA), exit_of(&a)],
        [slash_of(&a), rotate_of(&a, 0xAA)],
        [rotate_of(&a, 0xAA), slash_of(&a)],
        [rotate_of(&a, 0xAA), rotate_of(&a, 0xBB)],
        [rotate_of(&a, 0xAA), rotate_of(&a, 0xAA)],
    ];
    for order in orders {
        let mut ps = ProducerSet::new();
        register(&mut ps, &a, Some(0x11), 0);
        for u in order {
            ps.queue_update(u);
        }
        ps.apply_pending_updates();
        assert!(!ps.has_pending_updates());
    }
}

// ===== Persistence + the two exhaustive accessors =====

// REQ-ROT-005 — Decision: whether a queued rotation survives a restart. A variant that does
// not round-trip is dropped on reload, so the key never installs on the restarted node
// while it installs on every peer — a one-node fork at the next boundary.
#[test]
fn queued_rotation_survives_the_save_load_round_trip() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    apply_rotation(&mut ps, &rot(&a, 0xAA), 777).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("producers.bin");
    ps.save(&path).unwrap();
    let loaded = ProducerSet::load(&path).unwrap();

    assert_eq!(loaded.pending_update_count(), 1);
    match loaded.pending_updates_for(a.public_key())[0] {
        PendingProducerUpdate::RotateBlsKey {
            pubkey,
            new_bls_pubkey,
            height,
        } => {
            assert_eq!(pubkey, a.public_key());
            assert_eq!(new_bls_pubkey, &vec![0xAAu8; 48]);
            assert_eq!(*height, 777);
        }
        other => panic!("rotation did not round-trip: {other:?}"),
    }
}

// REQ-ROT-005 — Decision: whether the two exhaustive filters learned the new variant. A
// missing arm hides the rotation from the RPC and from `pending_updates_by_pubkey`, which
// the mempool's one-pending check reads.
#[test]
fn both_pending_accessors_see_the_rotation() {
    let a = kp(1);
    let mut ps = ProducerSet::new();
    register(&mut ps, &a, Some(0x11), 0);
    apply_rotation(&mut ps, &rot(&a, 0xAA), 100).unwrap();

    assert_eq!(ps.pending_updates_for(a.public_key()).len(), 1);

    let grouped = ps.pending_updates_by_pubkey();
    let mine = grouped
        .get(a.public_key())
        .expect("rotation must be grouped under the rotating producer");
    assert_eq!(mine.len(), 1);
    assert!(matches!(
        mine[0],
        PendingProducerUpdate::RotateBlsKey { .. }
    ));
}
