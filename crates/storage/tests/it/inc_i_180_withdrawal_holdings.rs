//! INC-I-180 M1 — the storage primitives the withdrawal-holdings gate is built
//! on. Requirements: REQ-I180-002 (Must), REQ-I180-001 (Must, F4 half).
//!
//! covers: set_core.rs, info.rs, tx_processing.rs, validation_checks.rs
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE IS GREEN TODAY AND MUST STAY GREEN
//! ---------------------------------------------------------------------------
//! The INC-I-180 defect does NOT live in `doli-storage`. It lives in
//! `bins/node/src/node/apply_block/tx_processing.rs`, which SKIPS queueing a
//! `PendingProducerUpdate::RequestWithdrawal` when the requested bond count
//! exceeds `bond_count - withdrawal_pending_count`, AFTER the Bond UTXOs have
//! already been spent. The behavioural RED evidence is therefore in
//! `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`.
//!
//! What this file does is LOCK the two storage facts the fix makes
//! load-bearing (brief F2 and F4). Both are asserted nowhere else:
//!
//!   F2 — `ProducerSet::pending_addbond_count` is the exact term the new
//!        allowance adds. If its semantics drift (e.g. it starts counting
//!        UPDATES rather than OUTPOINTS) the allowance silently under-counts
//!        and the n11 shortfall returns with a different arithmetic.
//!   F4 — `ProducerSet::apply_pending_updates_with_cap` drains
//!        `pending_updates` in INSERTION (FIFO) order, not grouped by variant.
//!        The whole fix rests on an `AddBond` queued at h=244583 flushing
//!        BEFORE a `RequestWithdrawal` queued at h=244708 in the same epoch.
//!        Today nothing asserts that; it is an emergent property of a `for`
//!        loop over a `Vec`. A future refactor that groups by variant (a very
//!        natural optimisation) would silently re-open the bug: the withdrawal
//!        would drain a 433-bond producer to 0 and auto-exit it, and the
//!        subsequent `add_bonds` would return 0 because `is_active()` is false
//!        — the AddBond's Bond UTXO orphaned, exactly the INC-I-085 shape.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test (all pure in-memory, no persistent store, no side
//! channels, no blocking syscall — TERMINATION is not an output here):
//!
//! `ProducerSet::pending_addbond_count(&PublicKey) -> u32`
//!   O1: the returned u32. No receiver mutation (`&self`).
//!
//! `ProducerSet::apply_pending_updates_with_cap(&mut self, cap: u64) -> ()`
//!   Return value is `()`, so EVERY observable output is a receiver mutation:
//!   O2: `ProducerInfo::bond_count`            (post-flush)
//!   O3: `ProducerInfo::status`                (Active | Exited — auto-exit)
//!   O4: `ProducerInfo::selection_weight()`    (0 iff not active)
//!   O5: `ProducerInfo::withdrawal_pending_count` (drained back to 0)
//!   O6: `ProducerSet::pending_update_count()` (queue emptied by the flush)
//!   O7: `ProducerInfo::bond_entries.len()` / `additional_bonds.len()`
//!       (the FIFO vesting ledger — proves WHICH bonds were consumed)
//!
//! PATHS
//!   P1: no queued AddBond for the pubkey                → O1 == 0
//!   P2: >=1 queued AddBond                              → O1 == Σ outpoints.len()
//!   P3: flush order [AddBond, RequestWithdrawal] where the withdrawal count
//!       EXCEEDS the pre-flush bond_count but equals bond_count + pending
//!       → AddBond lands first, `apply_withdrawal` drains to 0, auto-exit
//!   P4: flush order [AddBond, RequestWithdrawal] where the withdrawal count
//!       is STRICTLY LESS than bond_count + pending
//!       → producer survives, still Active. This is the row that DISCRIMINATES
//!         FIFO from variant-grouped: under variant grouping the same inputs
//!         produce Exited.
//!   P5: `apply_withdrawal` legacy fallback — bond_entries empty, bond_count>0
//!   P6: withdrawal of 0 bonds (degenerate, must be a no-op)
//!
//! INPUT PARTITIONS (distinct arithmetic/logic classes)
//!   IP-A  no pending updates at all                         → P1
//!   IP-B  unknown pubkey with other producers' AddBonds queued → P1
//!   IP-C  one AddBond carrying 1 outpoint                   → P2
//!   IP-D  two AddBonds carrying 1 and 3 outpoints (sum=4)   → P2
//!   IP-E  n11 shape: bond_count 433, pending AddBond(1), withdraw 434 → P3
//!   IP-F  minimal shape: bond_count 1, pending AddBond(1), withdraw 1 → P4
//!   IP-G  bond_entries empty (legacy producer), withdraw all → P5
//!   IP-H  withdraw 0                                        → P6
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1×P1×IP-A → pending_addbond_count_is_zero_with_no_queue
//!   O1×P1×IP-B → pending_addbond_count_is_scoped_to_the_pubkey
//!   O1×P2×IP-C → pending_addbond_count_counts_outpoints_not_updates
//!   O1×P2×IP-D → pending_addbond_count_counts_outpoints_not_updates
//!   O2..O7×P3×IP-E → f4_fifo_flush_addbond_then_full_withdrawal_exits_at_zero
//!   O2..O4×P4×IP-F → f4_fifo_order_is_the_discriminating_row
//!   O2,O3,O5×P5×IP-G → legacy_producer_without_bond_entries_still_exits
//!   O2,O3×P6×IP-H → withdrawal_of_zero_bonds_is_a_no_op
//!   O2,O5×P7×IP-I → charging_withdrawal_pending_does_not_move_bond_count_before_the_flush
//!   O2,O3,O5×P7×IP-I → accumulated_exit_charges_drain_in_order_without_underflow
//!
//! QA-1 addendum — P7: charge `withdrawal_pending_count` WITHOUT flushing, the
//! shape apply's `Exit` arm produces. IP-I: two Exit-shaped charges against one
//! producer holding 10 bonds. The validation mirror added for ISSUE-001 charges
//! `info.bond_count` per `Exit` transaction; that is a faithful mirror only
//! because `bond_count` stays put until the epoch flush.

use crypto::{Hash, KeyPair, PublicKey};
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus};

/// n11's real bond count on mainnet at the moment of the incident
/// (`b03fe629…`, 434 unbacked selection-weight units after the AddBond).
const N11_BONDS: u32 = 433;

/// Devnet-independent bond unit. Only the COUNTS matter to every assertion
/// here; the unit is carried so `bond_amount` arithmetic stays honest.
const UNIT: u64 = 1_000_000;

fn queue_addbond(ps: &mut ProducerSet, pk: &PublicKey, outpoints: u32, tag: u8) {
    let h = Hash::from_bytes([tag; 32]);
    ps.queue_update(PendingProducerUpdate::AddBond {
        pubkey: *pk,
        outpoints: (0..outpoints).map(|i| (h, i)).collect(),
        bond_unit: UNIT,
        creation_slot: 0,
    });
}

fn queue_withdrawal(ps: &mut ProducerSet, pk: &PublicKey, bond_count: u32) {
    ps.queue_update(PendingProducerUpdate::RequestWithdrawal {
        pubkey: *pk,
        bond_count,
        bond_unit: UNIT,
    });
}

/// A producer already flushed at `bond_count` bonds — the ProducerSet ledger
/// half of the n11 precondition `U > P`.
fn flushed_producer(bond_count: u32) -> (ProducerSet, KeyPair) {
    let kp = KeyPair::generate();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*kp.public_key(), bond_count, UNIT)
        .expect("register_genesis_producer");
    (ps, kp)
}

// ───────────────────────── O1 — pending_addbond_count ─────────────────────
// REQ-I180-002: this is the exact term the new allowance adds. The AddBond cap
// (INC-I-080) already depends on it; the withdrawal gate makes it load-bearing
// in a SECOND place, so its contract is pinned here independently.

/// O1×P1×IP-A
#[test]
fn pending_addbond_count_is_zero_with_no_queue() {
    let (ps, kp) = flushed_producer(N11_BONDS);
    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        0,
        "O1: a fully flushed producer has no in-flight AddBond bonds"
    );
}

/// O1×P1×IP-B — scoping. If this term ever summed across producers, one
/// producer's queued AddBond would inflate another's withdrawal allowance.
#[test]
fn pending_addbond_count_is_scoped_to_the_pubkey() {
    let (mut ps, kp) = flushed_producer(N11_BONDS);
    let other = KeyPair::generate();
    ps.register_genesis_producer(*other.public_key(), 1, UNIT)
        .expect("register other");
    queue_addbond(&mut ps, other.public_key(), 5, 1);

    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        0,
        "O1: another producer's queued AddBond must NOT enter this producer's allowance"
    );
    assert_eq!(
        ps.pending_addbond_count(other.public_key()),
        5,
        "O1: the owning producer sees its own 5 in-flight bonds"
    );
}

/// O1×P2×IP-C and O1×P2×IP-D — the unit is OUTPOINTS (one Bond UTXO each),
/// not queued updates. `AddBondData.bond_count` is deliberately NOT consulted:
/// the epoch flush hands `outpoints` to `ProducerInfo::add_bonds`, so outpoints
/// is what actually lands.
#[test]
fn pending_addbond_count_counts_outpoints_not_updates() {
    let (mut ps, kp) = flushed_producer(N11_BONDS);

    queue_addbond(&mut ps, kp.public_key(), 1, 7);
    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        1,
        "O1×IP-C: one AddBond carrying 1 outpoint contributes 1"
    );

    queue_addbond(&mut ps, kp.public_key(), 3, 8);
    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        4,
        "O1×IP-D: two AddBonds carrying 1 and 3 outpoints contribute 4, \
         not 2 (the update count)"
    );
}

// ─────────────── O2..O7 — F4: flush order is INSERTION order ───────────────

/// O2..O7 × P3 × IP-E — the n11 replay at the STORAGE layer.
///
/// Precondition: ProducerSet ledger holds 433 bonds; one AddBond(1) is queued
/// and unflushed (the deferred-flush window). A withdrawal for the full UTXO
/// value — 434 — is queued after it.
///
/// The fix in `tx_processing.rs` only produces a correct end state if the flush
/// applies the AddBond FIRST. This test is the lock on that ordering.
#[test]
fn f4_fifo_flush_addbond_then_full_withdrawal_exits_at_zero() {
    let (mut ps, kp) = flushed_producer(N11_BONDS);
    let pk = *kp.public_key();

    queue_addbond(&mut ps, &pk, 1, 3);
    queue_withdrawal(&mut ps, &pk, N11_BONDS + 1);

    // Path witness: the withdrawal count EXCEEDS the pre-flush bond_count.
    // Without the AddBond landing first there are only 433 bonds to drain.
    {
        let info = ps.get_by_pubkey(&pk).expect("producer present");
        assert_eq!(info.bond_count, N11_BONDS, "pre-flush ledger is 433");
        assert!(
            N11_BONDS + 1 > info.bond_count,
            "fixture precondition: the withdrawal must exceed the pre-flush count, \
             otherwise this row does not discriminate flush order"
        );
    }
    assert_eq!(ps.pending_update_count(), 2, "two updates queued, in order");

    ps.apply_pending_updates_with_cap(0);

    let info = ps.get_by_pubkey(&pk).expect("producer still present");
    assert_eq!(info.bond_count, 0, "O2: all 434 bonds drained");
    assert_eq!(
        info.status,
        ProducerStatus::Exited,
        "O3: auto-exit at bond_count == 0 (info.rs apply_withdrawal)"
    );
    assert_eq!(
        info.selection_weight(),
        0,
        "O4: an exited producer carries no selection weight — this is the \
         quantity that stayed at 434 on mainnet n11"
    );
    assert_eq!(
        info.withdrawal_pending_count, 0,
        "O5: the pending counter is drained by the flush, not left dangling"
    );
    assert_eq!(ps.pending_update_count(), 0, "O6: queue emptied");
    assert!(
        info.bond_entries.is_empty(),
        "O7: the FIFO vesting ledger is fully consumed — no orphaned entry"
    );
    assert!(
        info.additional_bonds.is_empty(),
        "O7: the AddBond's outpoint was consumed by the withdrawal, not orphaned"
    );
}

/// O2..O4 × P4 × IP-F — the DISCRIMINATING row.
///
/// bond_count 1, queued AddBond(1), queued RequestWithdrawal(1).
///   FIFO order      : 1 → 2 → withdraw 1 → 1 bond left, still Active.
///   variant-grouped : withdraw 1 → 0 → Exited → `add_bonds` returns 0 because
///                     `is_active()` is false → 0 bonds, Exited, bond orphaned.
/// The two orders give DIFFERENT terminal states, so this test fails loudly if
/// the loop is ever reordered.
#[test]
fn f4_fifo_order_is_the_discriminating_row() {
    let (mut ps, kp) = flushed_producer(1);
    let pk = *kp.public_key();

    queue_addbond(&mut ps, &pk, 1, 4);
    queue_withdrawal(&mut ps, &pk, 1);

    ps.apply_pending_updates_with_cap(0);

    let info = ps.get_by_pubkey(&pk).expect("producer present");
    assert_eq!(
        info.bond_count, 1,
        "O2: AddBond first (1→2), then withdraw 1 → 1. A variant-grouped flush \
         would yield 0 here and orphan the AddBond's Bond UTXO"
    );
    assert_eq!(
        info.status,
        ProducerStatus::Active,
        "O3: the producer survives — no auto-exit"
    );
    assert_eq!(
        info.selection_weight(),
        1,
        "O4: weight matches the 1 real bond"
    );
}

/// O2,O3,O5 × P5 × IP-G — legacy producers carry no `bond_entries`.
/// `apply_withdrawal` has a second, easily-forgotten drain path for them; the
/// fix's "withdrawal always reaches 0 and exits" claim must hold there too.
#[test]
fn legacy_producer_without_bond_entries_still_exits() {
    let (mut ps, kp) = flushed_producer(N11_BONDS);
    let pk = *kp.public_key();
    {
        let info = ps.get_by_pubkey_mut(&pk).expect("producer present");
        info.bond_entries.clear(); // pre-StoredBondEntry shape
        info.withdrawal_pending_count = N11_BONDS;
    }

    queue_withdrawal(&mut ps, &pk, N11_BONDS);
    ps.apply_pending_updates_with_cap(0);

    let info = ps.get_by_pubkey(&pk).expect("producer present");
    assert_eq!(info.bond_count, 0, "O2: legacy fallback drained the ledger");
    assert_eq!(
        info.status,
        ProducerStatus::Exited,
        "O3: auto-exit fires on the legacy path too"
    );
    assert_eq!(
        info.withdrawal_pending_count, 0,
        "O5: legacy path also clears the pending counter"
    );
}

/// O2,O3 × P6 × IP-H — degenerate input. A 0-bond withdrawal must not exit an
/// active producer (`bond_count == 0` is the auto-exit trigger, and a
/// mis-ordered guard could reach it with an untouched ledger).
#[test]
fn withdrawal_of_zero_bonds_is_a_no_op() {
    let (mut ps, kp) = flushed_producer(N11_BONDS);
    let pk = *kp.public_key();

    queue_withdrawal(&mut ps, &pk, 0);
    ps.apply_pending_updates_with_cap(0);

    let info = ps.get_by_pubkey(&pk).expect("producer present");
    assert_eq!(info.bond_count, N11_BONDS, "O2: ledger untouched");
    assert_eq!(
        info.status,
        ProducerStatus::Active,
        "O3: a 0-bond withdrawal must never auto-exit"
    );
}

// ───────── O2,O5 × P7 — QA-1 / ISSUE-001: the double-charge substrate ──────
// The gate's `Exit` mirror charges `info.bond_count` per Exit transaction, so a
// block with two Exits for one producer charges it TWICE. That is only a
// faithful mirror of apply because `bond_count` does NOT move until the epoch
// flush — the apply arm re-reads an unchanged value. Pin that here: if a future
// refactor decremented `bond_count` at charge time, the mirror would over-count
// and start rejecting legal blocks.

/// O2,O5 × P7 × IP-I — charging `withdrawal_pending_count` leaves `bond_count`
/// untouched, so a second charge in the same block reads the same holding.
#[test]
fn charging_withdrawal_pending_does_not_move_bond_count_before_the_flush() {
    let (mut ps, kp) = flushed_producer(10);
    let pk = *kp.public_key();

    let first_read = ps.get_by_pubkey(&pk).expect("producer present").bond_count;
    ps.get_by_pubkey_mut(&pk)
        .expect("producer present")
        .withdrawal_pending_count += first_read;
    queue_withdrawal(&mut ps, &pk, first_read);

    let second_read = ps.get_by_pubkey(&pk).expect("producer present").bond_count;
    assert_eq!(
        second_read, first_read,
        "O2: bond_count is deferred to the epoch flush, so the SECOND Exit in a \
         block reads the same 10 the first one did. The validation mirror must \
         reproduce that, not correct it"
    );

    ps.get_by_pubkey_mut(&pk)
        .expect("producer present")
        .withdrawal_pending_count += second_read;
    queue_withdrawal(&mut ps, &pk, second_read);
    assert_eq!(
        ps.get_by_pubkey(&pk)
            .expect("producer present")
            .withdrawal_pending_count,
        20,
        "O5: two Exit-shaped charges accumulate to 2 x bond_count — the exact \
         quantity the allowance must subtract"
    );
}

/// O2,O3,O5 × P7 × IP-I — and the flush drains the accumulated charges in FIFO
/// order without underflowing.
#[test]
fn accumulated_exit_charges_drain_in_order_without_underflow() {
    let (mut ps, kp) = flushed_producer(10);
    let pk = *kp.public_key();

    for _ in 0..2 {
        let held = ps.get_by_pubkey(&pk).expect("producer present").bond_count;
        ps.get_by_pubkey_mut(&pk)
            .expect("producer present")
            .withdrawal_pending_count += held;
        queue_withdrawal(&mut ps, &pk, held);
    }
    ps.apply_pending_updates_with_cap(0);

    let info = ps.get_by_pubkey(&pk).expect("producer present");
    assert_eq!(
        info.bond_count, 0,
        "O2: the first withdrawal drains all 10; the second finds nothing left"
    );
    assert_eq!(
        info.status,
        ProducerStatus::Exited,
        "O3: auto-exit at bond_count == 0"
    );
    assert_eq!(
        info.withdrawal_pending_count, 10,
        "O5: the FIRST withdrawal drains 10 entries and credits 10 back off the \
         20 charged; the SECOND finds `bond_entries` empty and `bond_count == 0`, \
         so both of `apply_withdrawal`'s decrement paths are skipped and 10 stays \
         charged. Saturating arithmetic throughout — no underflow, no panic. The \
         residue is harmless to the gate: it only ever SHRINKS the allowance, and \
         the producer is already Exited"
    );
}
