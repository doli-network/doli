//! A block for this slot stops a build only if its parent is on this node's branch.
//! Every behaviour test drives the real `try_produce_block`.
//!
//! covers: mod gates production_asks
//
// OUTPUT CONTRACT: fn Node::try_produce_block(&mut self) -> Result<()>
//   O2: self.last_produced_slot — Some(block slot) iff minted, else unchanged (None)
//   O2: self.chain_state best_height / best_hash — tip+1 and the block hash iff minted
//   O3: return — Ok(()) on every path; asserted inside `drive_seeded` on every poll
//   O4: block_store[tip+1] — Some(block with parent = old tip, by this key) iff minted
//   O5: signed_slots_db — the built slot is marked iff the guard passed (partition F)
//   O7: log — one `SLASHING PROTECTION` error per refused attempt (partition F)
//   O1, O6: none here; `production_asks_chosen_branch` covers the send.
// PATHS: P1 proceed (mint) | P2 stopped by the slot block | P3 refused by the signed-slot guard
// INPUT PARTITIONS (slot index holds a sibling by another key, for this slot and the next):
//   A: sibling parent unknown to the store       -> P1 (v6.46.0: P2)
//   B: sibling parent known, not canonical       -> P1 (v6.46.0: P2)
//   C: sibling parent == my tip                  -> P2 on both
//   D: sibling parent canonical at tip-2         -> P2 on both
//   E: source text of the two stops              -> one predicate, no old read
//   F: B, and the slot is marked as signed       -> P3 (v6.46.0: P2, the guard is never reached)
//   G: F, then new slots are not marked          -> P1 (v6.46.0: P2)
// MATRIX: {last_produced_slot, tip, block_store} x {A, B, G} by `assert_minted`,
//   x {C, D, F} by `assert_not_minted`; O3 in `drive_seeded` for all. O5 x {F, G} and
//   O7 x {F} in `signed_slot_refuses_second_build_behind_other_branch_sibling`.
//   C, D first show can_produce == Authorized, and pair with A, which changes only the parent.
// RIG: `production_asks_chosen_branch`. A peer gap of 5 at height 12 above the gate does
//   not stop a build on either version, so only the slot stop is under test.
// REQ-PASK-SEC-003 — Decision: a grep hit in the names and messages of this file would reveal a public-detail leak; checked by grep, no runtime test.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_node::producer::SignedSlotsDb;
use tempfile::TempDir;

use super::inc_i_289_common::block;
use super::production_asks_chosen_branch::{
    assert_gate_authorizes, assert_minted, assert_not_minted, assert_peer_gap, current_slot,
    in_child_of, rig_at, taller_peers, Rig, ABOVE_GATE, MINT_BUDGET, NO_MINT_BUDGET, POLL,
    TIP_ABOVE_TEN,
};

const MODULE: &str = "production_asks_slot_block";
/// Two marked slots, then one mint window.
const AFTER_MARKS_BUDGET: Duration = Duration::from_secs(35);
const PEER_GAP: u64 = 5;

/// The rig with a peer gap the peer rule lets through, and the gate clear.
async fn rig_with_clear_gates(partition: &str) -> Rig {
    let r = rig_at(TIP_ABOVE_TEN).await;
    taller_peers(&r, TIP_ABOVE_TEN + PEER_GAP).await;
    assert_peer_gap(&r, PEER_GAP, partition).await;
    assert_gate_authorizes(&r, partition).await;
    r
}

/// Writes a sibling by `other` on `parent` for this slot and the next.
fn seed_sibling(r: &Rig, parent: Hash, other: &KeyPair) {
    let slot = current_slot(&r.node);
    let next = r.tip + 1;
    for s in [slot, slot + 1] {
        let sibling = block(next, s, parent, other, &r.node.params);
        r.node
            .block_store
            .put_block(&sibling, next)
            .expect("fixture: seed the sibling");
    }
}

/// Marks this slot and the next as signed, as a build in those slots would.
fn mark_slots(r: &Rig) {
    let db = r.node.signed_slots_db.as_ref().expect("fixture: guard db");
    let slot = current_slot(&r.node) as u64;
    for s in [slot, slot + 1] {
        if !db.was_signed(s) {
            db.check_and_mark(s).expect("fixture: mark the slot");
        }
    }
}

/// Polls the production entry point with the sibling reseeded before every poll.
async fn drive_seeded(r: &mut Rig, parent: Hash, budget: Duration, mark: bool) -> bool {
    let other = KeyPair::generate();
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        seed_sibling(r, parent, &other);
        if mark {
            mark_slots(r);
        }
        r.node
            .try_produce_block()
            .await
            .expect("O3: try_produce_block returns Ok on every path");
        if r.node.chain_state.read().await.best_height > r.tip {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

async fn drive_with_sibling(r: &mut Rig, parent: Hash, budget: Duration) -> bool {
    drive_seeded(r, parent, budget, false).await
}

/// Stores a block at an old slot, off this node's chain, and returns its hash.
fn other_branch_parent(r: &Rig, partition: &str) -> Hash {
    let other = KeyPair::generate();
    let off = block(
        r.tip - 2,
        100,
        crypto::hash::hash(b"off"),
        &other,
        &r.node.params,
    );
    let store = &r.node.block_store;
    store
        .put_block(&off, r.tip - 2)
        .expect("fixture: put_block");
    let hash = off.hash();
    assert!(
        store.get_header(&hash).expect("store read").is_some(),
        "{partition}: precondition: the store knows this parent"
    );
    assert_eq!(
        store.get_height_by_hash(&hash).expect("store read"),
        None,
        "{partition}: precondition: this parent is not canonical"
    );
    hash
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Collects error logs of this process; call it only in a re-exec'd child.
fn capture_error_logs() -> Capture {
    let cap = Capture::default();
    let writer = cap.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::ERROR)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("fixture: log subscriber");
    cap
}

// REQ-PASK-005 — Decision: a failure means a block whose parent this node has never seen can still stop its build.
#[tokio::test]
async fn slot_taken_with_unknown_parent_does_not_stop_build() {
    if !in_child_of(
        MODULE,
        "slot_taken_with_unknown_parent_does_not_stop_build",
        ABOVE_GATE,
    ) {
        return;
    }
    let mut r = rig_with_clear_gates("A").await;
    let parent = crypto::hash::hash(b"parent the store has never seen");
    let store = &r.node.block_store;
    assert!(
        store.get_header(&parent).expect("store read").is_none(),
        "A: precondition: the parent is unknown"
    );

    let minted = drive_with_sibling(&mut r, parent, MINT_BUDGET).await;

    assert!(
        minted,
        "A: no mint: a sibling on an unknown parent stopped the build"
    );
    assert_minted(&r, "A").await;
}

// REQ-PASK-005 — Decision: a failure means a block from another known branch can still stop this node's build on its own branch.
#[tokio::test]
async fn slot_taken_by_other_branch_does_not_stop_build() {
    if !in_child_of(
        MODULE,
        "slot_taken_by_other_branch_does_not_stop_build",
        ABOVE_GATE,
    ) {
        return;
    }
    let mut r = rig_with_clear_gates("B").await;
    let parent = other_branch_parent(&r, "B");

    let minted = drive_with_sibling(&mut r, parent, MINT_BUDGET).await;

    println!("PROBE other_branch_sibling_mints={}", u8::from(minted));
    assert!(
        minted,
        "B: no mint: a sibling off this branch stopped the build"
    );
    assert_minted(&r, "B").await;
}

// REQ-PASK-005 — Decision: a failure means this node makes a second block on its own tip in a slot that already holds one.
#[tokio::test]
async fn slot_taken_on_my_tip_stops_build() {
    if !in_child_of(MODULE, "slot_taken_on_my_tip_stops_build", ABOVE_GATE) {
        return;
    }
    let mut r = rig_with_clear_gates("C").await;
    let parent = r.tip_hash;

    let minted = drive_with_sibling(&mut r, parent, NO_MINT_BUDGET).await;

    assert!(
        !minted,
        "C: a sibling on this node's tip must stop the build"
    );
    assert_not_minted(&r, "C").await;
}

// REQ-PASK-005 — Decision: a failure means the check reads only the tip, so a sibling on an earlier block of this branch no longer stops a build.
#[tokio::test]
async fn slot_taken_on_canonical_ancestor_stops_build() {
    if !in_child_of(
        MODULE,
        "slot_taken_on_canonical_ancestor_stops_build",
        ABOVE_GATE,
    ) {
        return;
    }
    let mut r = rig_with_clear_gates("D").await;
    let height = r.tip - 2;
    let store = &r.node.block_store;
    let parent = store
        .get_hash_by_height(height)
        .expect("store read")
        .expect("fixture: canonical ancestor");
    assert_eq!(
        store.get_height_by_hash(&parent).expect("store read"),
        Some(height),
        "D: precondition: the parent is canonical"
    );

    let minted = drive_with_sibling(&mut r, parent, NO_MINT_BUDGET).await;

    assert!(
        !minted,
        "D: a sibling on a canonical ancestor must stop the build"
    );
    assert_not_minted(&r, "D").await;
}

// REQ-PASK-005 — Decision: a failure means the two stops can disagree, so a build could pass the first and abort after the VDF, or the old read still runs.
#[test]
fn both_slot_stops_call_one_predicate() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/node/production");
    let call = "slot_block_on_my_branch(";
    let src = include_str!("../../src/node/production/mod.rs");
    assert_eq!(
        src.matches(call).count(),
        2,
        "E: production/mod.rs must call the predicate exactly twice"
    );
    let (before, after) = src
        .split_once("compute_block_vdf(")
        .expect("fixture: VDF call in production/mod.rs");
    assert_eq!(
        before.matches(call).count(),
        1,
        "E: one call before the VDF"
    );
    assert_eq!(after.matches(call).count(), 1, "E: one call after the VDF");

    let mut read = 0;
    for entry in std::fs::read_dir(dir).expect("fixture: production dir") {
        let path = entry.expect("fixture: dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("fixture: read source");
        read += 1;
        for old in ["has_block_for_slot", "seen_blocks_for_slot"] {
            assert!(
                !text.contains(old),
                "E: {} still names {old}",
                path.display()
            );
        }
    }
    assert!(read >= 2, "E: the scan must read the production sources");

    let gates = include_str!("../../src/node/production/gates.rs");
    assert!(
        gates.contains("pub fn slot_block_on_my_branch"),
        "E: gates.rs must define the predicate"
    );
}

// REQ-PASK-005 — Decision: a failure means the signed-slot guard is not what refuses a second build, or a marked slot blocks later slots, or the store sibling still stops first.
#[tokio::test]
async fn signed_slot_refuses_second_build_behind_other_branch_sibling() {
    if !in_child_of(
        MODULE,
        "signed_slot_refuses_second_build_behind_other_branch_sibling",
        ABOVE_GATE,
    ) {
        return;
    }
    let logs = capture_error_logs();
    let db_dir = TempDir::new().unwrap();
    let mut r = rig_with_clear_gates("F").await;
    r.node.signed_slots_db = Some(SignedSlotsDb::open(db_dir.path()).expect("fixture: guard db"));
    let parent = other_branch_parent(&r, "F");

    let minted = drive_seeded(&mut r, parent, NO_MINT_BUDGET, true).await;

    assert!(!minted, "F: a build in a marked slot must be refused");
    assert_not_minted(&r, "F").await;
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("SLASHING PROTECTION"),
        "F: O7: the signed-slot guard never refused, so another stop ran first; got: {text}"
    );

    let db = r.node.signed_slots_db.as_ref().expect("fixture: guard db");
    let slot = current_slot(&r.node) as u64;
    let marked_through = if db.was_signed(slot + 1) {
        slot + 1
    } else {
        slot
    };

    let minted = drive_with_sibling(&mut r, parent, AFTER_MARKS_BUDGET).await;

    assert!(
        minted,
        "G: no mint in an unmarked slot: the sibling, not the mark, was the stop"
    );
    assert_minted(&r, "G").await;
    let built = r.node.last_produced_slot.expect("G: O2 last_produced_slot");
    assert!(
        built > marked_through,
        "G: the build must fall in an unmarked slot"
    );
    let db = r.node.signed_slots_db.as_ref().expect("fixture: guard db");
    assert!(db.was_signed(built), "G: O5: the built slot is marked");
}
