//! INC-I-080 — AddBond silent-clip → AH-gated validation rejection.
//!
//! Bug (confirmed, memory.db entries 719–723): an AddBond that would push a
//! producer past `MAX_BONDS_PER_PRODUCER` is silently CLIPPED at epoch flush
//! (`ProducerInfo::add_bonds` saturates `available_slots`; the excess Bond
//! UTXOs are orphaned, value lost, no error). No cap check exists at
//! validation or apply.
//!
//! Fix (height-gated, INC-I-075 three-question verdict = AH required):
//!   PRE-activation  : behavior UNCHANGED (clip path preserved — replay safety
//!                      for historical blocks).
//!   POST-activation : the AddBond is REJECTED at block-apply validation
//!                      (`check_addbond_cap` → Err), so the carrying block is
//!                      invalid fleet-wide and no Bond UTXOs are ever created
//!                      ("no orphan Bonds").
//!
//! NOTE — this DIVERGES from the INC-I-078 DelegateBond sibling, which uses a
//! SKIP pattern (tx stays in block, no state effect). Skip is wrong for
//! AddBond because the Bond output UTXOs would still be created and orphaned;
//! the confirmed diagnosis (entry 722) and T3 ("no orphan Bonds") require
//! true rejection. Consensus-safety is preserved because the rejection is
//! height-gated and deterministic across all nodes past the same AH.
//!
// OUTPUT CONTRACT: fn check_addbond_cap(current, pending, requested, height, activation_height) -> Result<(), ValidationError>
//   O1: Result<(), ValidationError>
//       = Ok(())  (accept — clip path or under cap)
//       | Err(ValidationError::AddBondCapExceeded { current, pending, requested, max })
//   PATHS:
//     P1: height <  activation_height                                  → Ok(())  ALWAYS (pre-AH gate dominates; clip path preserved)
//     P2: height >= activation_height ∧ current+pending+requested <= MAX → Ok(())
//     P3: height >= activation_height ∧ current+pending+requested >  MAX → Err(AddBondCapExceeded{..})
//   INPUT PARTITIONS (distinct math/logic classes per path):
//     IP-A  sum == MAX exactly (accept boundary)             : {2999,0,1} ; {2998,0,2}            → P2
//     IP-B  sum == MAX+1 via current+requested only (reject)  : {2999,0,2}                          → P3
//     IP-C  reject driven BY pending>0 (proves pending summed): {2999,1,1}                          → P3
//     IP-D  pre-AH with over-cap inputs (gate must dominate)  : {2999,0,2} height < AH              → P1
//     IP-E  saturating arithmetic, no u32 overflow panic      : {u32::MAX,1,1} post-AH              → P3
// OUTPUT CONTRACT: fn ProducerInfo::add_bonds(outpoints, amount_per_bond, creation_slot) -> u32  (EXISTING — documents PRE-AH clip; unchanged on both branches)
//   O2: u32 = bonds actually added.  Side effects: self.bond_count, self.additional_bonds.len()
//   PATHS:
//     P4: bond_count + outpoints.len() <= MAX → added == outpoints.len()  (all stored, 0 orphaned)
//     P5: bond_count + outpoints.len() >  MAX → added == MAX - bond_count  (clip; excess outpoints dropped)
//   INPUT PARTITIONS:
//     IP-F  {bond_count=2999, 2 outpoints} → added=1, bond_count=3000, orphaned=1   → P5
//     IP-G  {bond_count=2998, 2 outpoints} → added=2, bond_count=3000, orphaned=0   → P4
// OUTPUT CONTRACT: fn ProducerSet::pending_addbond_count(&pubkey) -> u32
//   O3: u32 = Σ outpoints.len() over queued PendingProducerUpdate::AddBond for pubkey
//   PATHS:
//     P6: ≥1 queued AddBond for pubkey → sum of their outpoints.len()
//     P7: no pending / unknown pubkey  → 0
//   INPUT PARTITIONS:
//     IP-H  one AddBond(1 outpoint) queued for pk → 1   → P6
//     IP-I  no pending updates for pk              → 0   → P7
//
// MATRIX (every O × PATH × INPUT-PARTITION cell has an assertion):
//   O1×P2×IP-A → t1_at_2999_addbond_1_post_ah_accepted ; t5_at_2998_addbond_2_post_ah_accepted
//   O1×P3×IP-B → t3_at_2999_addbond_2_post_ah_rejected
//   O1×P3×IP-C → t4_at_2999_pending_1_addbond_1_post_ah_rejected
//   O1×P1×IP-D → gate_pre_activation_overcap_accepted
//   O1×P3×IP-E → saturating_no_overflow_post_ah_rejected
//   O2×P5×IP-F → t2_pre_ah_clip_documents_current_behavior
//   O2×P4×IP-G → add_bonds_full_when_exactly_at_max
//   O3×P6×IP-H → pending_addbond_count_sums_queued
//   O3×P7×IP-I → pending_addbond_count_zero_when_none

use crypto::{Hash, KeyPair};
use doli_core::validation::{check_addbond_cap, ValidationError};
use doli_core::MAX_BONDS_PER_PRODUCER;
use storage::{PendingProducerUpdate, ProducerSet};

const AH: u64 = 231_830; // post == height >= AH ; pre == height < AH
const POST: u64 = AH; // height at activation (>= AH ⇒ enforced)
const PRE: u64 = AH - 1; // height one below activation (clip path)

// ── O1×P2×IP-A : sum == MAX exactly, post-AH ⇒ accepted ──────────────
#[test]
fn t1_at_2999_addbond_1_post_ah_accepted() {
    // current 2999 + pending 0 + requested 1 == 3000 == MAX → accept
    assert_eq!(MAX_BONDS_PER_PRODUCER, 3000);
    let r = check_addbond_cap(2999, 0, 1, POST, AH);
    assert!(r.is_ok(), "2999+0+1==MAX must be accepted, got {r:?}");
}

#[test]
fn t5_at_2998_addbond_2_post_ah_accepted() {
    // boundary: 2998 + 0 + 2 == 3000 == MAX → accept
    let r = check_addbond_cap(2998, 0, 2, POST, AH);
    assert!(r.is_ok(), "2998+0+2==MAX must be accepted, got {r:?}");
}

// ── O1×P3×IP-B : sum == MAX+1 (current+requested), post-AH ⇒ reject ──
#[test]
fn t3_at_2999_addbond_2_post_ah_rejected() {
    let r = check_addbond_cap(2999, 0, 2, POST, AH);
    assert_eq!(
        r,
        Err(ValidationError::AddBondCapExceeded {
            current: 2999,
            pending: 0,
            requested: 2,
            max: MAX_BONDS_PER_PRODUCER,
        }),
        "2999+0+2 (=3001 > MAX) must be rejected with structured error"
    );
}

// ── O1×P3×IP-C : rejection DRIVEN by pending>0 (proves pending summed) ─
#[test]
fn t4_at_2999_pending_1_addbond_1_post_ah_rejected() {
    // current+requested alone (2999+1=3000) is at the cap and would be
    // accepted. Only by summing the in-flight pending AddBond(+1) does the
    // total reach 3001 and trip the cap. This is the regression that the
    // silent-clip bug never caught.
    let r = check_addbond_cap(2999, 1, 1, POST, AH);
    assert_eq!(
        r,
        Err(ValidationError::AddBondCapExceeded {
            current: 2999,
            pending: 1,
            requested: 1,
            max: MAX_BONDS_PER_PRODUCER,
        }),
        "pending in-flight AddBond must be summed into the cap check"
    );
}

// ── O1×P1×IP-D : pre-AH with over-cap inputs ⇒ gate dominates, accept ─
#[test]
fn gate_pre_activation_overcap_accepted() {
    // Identical over-cap inputs as T3, but height < AH: the pre-activation
    // clip path is preserved (replay safety). MUST be Ok regardless of cap.
    let r = check_addbond_cap(2999, 0, 2, PRE, AH);
    assert!(
        r.is_ok(),
        "pre-activation (height < AH) must NOT reject — clip path preserved, got {r:?}"
    );
}

// ── O1×P3×IP-E : saturating arithmetic, no overflow panic, post-AH ───
#[test]
fn saturating_no_overflow_post_ah_rejected() {
    let r = check_addbond_cap(u32::MAX, 1, 1, POST, AH);
    assert!(
        matches!(r, Err(ValidationError::AddBondCapExceeded { .. })),
        "extreme inputs must saturate (no panic) and reject, got {r:?}"
    );
}

// ── O2×P5×IP-F : EXISTING add_bonds clip — documents the bug behavior ─
// Passes on BOTH branches: the pre-AH clip path is intentionally unchanged.
#[test]
fn t2_pre_ah_clip_documents_current_behavior() {
    let kp = KeyPair::generate();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*kp.public_key(), 2999, 1)
        .expect("register");
    let info = ps.get_by_pubkey_mut(kp.public_key()).expect("info");
    assert_eq!(info.bond_count, 2999);

    let h = Hash::from_bytes([7u8; 32]);
    let added = info.add_bonds(vec![(h, 0), (h, 1)], 1, 0);

    assert_eq!(added, 1, "only 1 slot available (2999→3000): clip to 1");
    assert_eq!(info.bond_count, 3000, "bond_count reaches the cap");
    let orphaned = 2 - added;
    assert_eq!(orphaned, 1, "1 Bond UTXO silently orphaned (the bug)");
}

// ── O2×P4×IP-G : add_bonds full when it lands exactly at MAX ─────────
#[test]
fn add_bonds_full_when_exactly_at_max() {
    let kp = KeyPair::generate();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*kp.public_key(), 2998, 1)
        .expect("register");
    let info = ps.get_by_pubkey_mut(kp.public_key()).expect("info");

    let h = Hash::from_bytes([9u8; 32]);
    let added = info.add_bonds(vec![(h, 0), (h, 1)], 1, 0);

    assert_eq!(added, 2, "2998+2 == MAX: all added, none orphaned");
    assert_eq!(info.bond_count, 3000);
}

// ── O3×P6×IP-H : pending_addbond_count sums queued AddBond outpoints ──
#[test]
fn pending_addbond_count_sums_queued() {
    let kp = KeyPair::generate();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*kp.public_key(), 1, 1)
        .expect("register");

    let h = Hash::from_bytes([3u8; 32]);
    ps.queue_update(PendingProducerUpdate::AddBond {
        pubkey: *kp.public_key(),
        outpoints: vec![(h, 0)],
        bond_unit: 1,
        creation_slot: 0,
    });

    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        1,
        "one queued AddBond with 1 outpoint → pending count 1"
    );
}

// ── O3×P7×IP-I : pending_addbond_count zero when no pending ──────────
#[test]
fn pending_addbond_count_zero_when_none() {
    let kp = KeyPair::generate();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*kp.public_key(), 1, 1)
        .expect("register");

    assert_eq!(
        ps.pending_addbond_count(kp.public_key()),
        0,
        "no pending updates → 0"
    );
    let other = KeyPair::generate();
    assert_eq!(
        ps.pending_addbond_count(other.public_key()),
        0,
        "unknown pubkey → 0"
    );
}
