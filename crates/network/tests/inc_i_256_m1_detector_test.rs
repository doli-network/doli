//! INC-I-256 M1 / R-3 — the equivocation detector bounds its sliding window by
//! the local wall-clock slot (a far-future block cannot wipe it) and drops the
//! headers stored under the pre-gate admission once, when the gate is crossed.
//!
// covers: equivocation equivocation_tests header_auth block_handling network_events
//
// OUTPUT CONTRACT: fn EquivocationDetector::check_block_with(&mut self, &Block, u32 /*wall_slot*/, impl FnOnce(&BlockHeader) -> bool) -> Option<EquivocationProof>
//   O2 receiver: tracked entries (tracked_count), pending proofs (take_pending_proofs)
//   O3 return: None | Some(proof) naming producer, slot, (stored header, new header)
// OUTPUT CONTRACT: fn EquivocationDetector::note_gate(&mut self, u64 /*prospective height*/, u64 /*gate*/)
//   O2 receiver: tracked entries dropped at the first crossing only; latch kept across clear()
// PATHS: P-FUTURE header slot > wall + MAX_FUTURE_SLOTS (skipped) | P-STORE first header | P-PAIR conflicting header
//        | P-SWEEP wall advances past the window | P-CROSS first note_gate at/above the gate | P-LATCHED later calls
// INPUT PARTITIONS:
//   far-future slot: u32::MAX (one jump) | W + 361k stepping (wall fixed)
//   arrival order: oldest first | newer first, older later
//   slot range: wall = slot = u32::MAX | wall = u32::MAX - 1 | wall = slot = 0 | wall + MAX_FUTURE_SLOTS (+1)
//   gate: below | at (first crossing) | above (second call) | after clear() | frozen at u64::MAX
// MATRIX:
//   O2,O3 x P-FUTURE(u32::MAX) x {P-PAIR, P-SWEEP} -> inc_i_256_m1_u32_max_slot_block_does_not_wipe_detector
//   O2,O3 x P-FUTURE(stepping) x P-PAIR -> inc_i_256_m1_stepping_future_slots_wipe_nothing
//   O2,O3 x P-SWEEP x {newer first} -> inc_i_256_m1_out_of_order_entry_does_not_stop_sweep
//   O2,O3 x slot range x {P-STORE, P-PAIR, P-FUTURE} -> inc_i_256_m1_slot_arithmetic_saturates_at_u32_max
//   O2,O3 x {below, P-CROSS} x decoy -> inc_i_256_m1_gate_crossing_drops_decoy_then_pairs_real_headers
//   O2 x {P-LATCHED, after clear(), frozen} -> inc_i_256_m1_gate_latch_survives_clear_and_fires_once

use std::collections::HashSet;

use crypto::{Hash, PublicKey};
use doli_core::consensus::MAX_FUTURE_SLOTS;
use doli_core::{Block, BlockHeader};
use network::{EquivocationDetector, EquivocationProof};
use vdf::{VdfOutput, VdfProof};

const W: u32 = 10_000;
const WINDOW: u32 = 360;
const GATE: u64 = 50;

fn pk(n: u8) -> PublicKey {
    PublicKey::from_bytes([n; 32])
}

fn block(slot: u32, producer: u8, salt: &[u8]) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(salt),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: pk(producer),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

fn admit_all(_: &BlockHeader) -> bool {
    true
}

fn check(d: &mut EquivocationDetector, b: &Block, wall: u32) -> Option<EquivocationProof> {
    d.check_block_with(b, wall, admit_all)
}

fn pair(p: &EquivocationProof) -> (Hash, Hash) {
    (p.block_header_1.hash(), p.block_header_2.hash())
}

// R-3 — Decision: a failure means one sync block at slot u32::MAX wipes every stored header and freezes the window, so later double production is never proven.
#[test]
fn inc_i_256_m1_u32_max_slot_block_does_not_wipe_detector() {
    let mut d = EquivocationDetector::new();
    let honest: Vec<Block> = (1..=3).map(|p| block(W, p, b"honest")).collect();
    for b in &honest {
        assert!(check(&mut d, b, W).is_none(), "PRECONDITION: first header");
    }
    assert_eq!(d.tracked_count(), 3, "PRECONDITION: three stored headers");

    let bomb = block(u32::MAX, 9, b"bomb");
    assert!(
        check(&mut d, &bomb, W).is_none(),
        "the far-future block proves nothing"
    );
    let after_bomb = d.tracked_count();

    let mut wiped = 0;
    let mut proofs = Vec::new();
    for (i, h) in honest.iter().enumerate() {
        let conflict = block(W, i as u8 + 1, b"conflict");
        match check(&mut d, &conflict, W) {
            Some(p) => proofs.push(((h.hash(), conflict.hash()), p)),
            None => wiped += 1,
        }
    }
    println!("B2M1_METRIC DETECTOR_ENTRIES_WIPED={wiped}");
    assert_eq!(wiped, 0, "the u32::MAX block must not wipe stored headers");
    assert_eq!(
        after_bomb, 3,
        "the far-future block is neither tracked nor evicts anything"
    );
    for (expected, p) in &proofs {
        assert_eq!(
            (p.slot, pair(p)),
            (W, *expected),
            "proof names the stored and the new header"
        );
    }
    let pending = d.take_pending_proofs();
    let producers: HashSet<PublicKey> = pending.iter().map(|p| p.producer).collect();
    assert_eq!(pending.len(), 3, "exactly one proof per conflicting pair");
    assert_eq!(producers, (1..=3).map(pk).collect::<HashSet<_>>());

    let later = block(W + WINDOW + 1, 8, b"later");
    assert!(check(&mut d, &later, W + WINDOW + 1).is_none());
    assert_eq!(
        d.tracked_count(),
        1,
        "the window still sweeps once the wall clock moves past it"
    );
}

// R-3 — Decision: a failure means a fix that only special-cases u32::MAX still lets a stepping sequence of future slots wipe the window.
#[test]
fn inc_i_256_m1_stepping_future_slots_wipe_nothing() {
    let mut d = EquivocationDetector::new();
    let honest = block(W, 1, b"honest");
    assert!(check(&mut d, &honest, W).is_none());
    for k in 1..=5u32 {
        let step = block(W + (WINDOW + 1) * k, 2, &k.to_le_bytes());
        assert!(check(&mut d, &step, W).is_none());
        assert_eq!(
            d.tracked_count(),
            1,
            "step {k}: nothing wiped, nothing tracked"
        );
    }
    let conflict = block(W, 1, b"conflict");
    let p = check(&mut d, &conflict, W).expect("the honest header survived the stepping sequence");
    assert_eq!(pair(&p), (honest.hash(), conflict.hash()));
    assert_eq!(d.take_pending_proofs().len(), 1);
}

// R-3 — Decision: a failure means the sweep stops at a newer entry that arrived first, so an older entry outlives the window and the bound is not a full sweep.
#[test]
fn inc_i_256_m1_out_of_order_entry_does_not_stop_sweep() {
    let mut d = EquivocationDetector::new();
    let newer = block(1_000, 1, b"newer");
    let older = block(700, 2, b"older");
    assert!(check(&mut d, &newer, 1_000).is_none());
    assert!(check(&mut d, &older, 1_000).is_none());
    assert_eq!(
        d.tracked_count(),
        2,
        "PRECONDITION: both inside the window at wall 1000"
    );

    let tick = block(1_100, 3, b"tick");
    assert!(check(&mut d, &tick, 1_100).is_none());
    assert_eq!(
        d.tracked_count(),
        2,
        "slot 700 < 1100 - 360 must be swept even though a newer entry arrived first"
    );
    let older_conflict = block(700, 2, b"older-conflict");
    assert!(
        check(&mut d, &older_conflict, 1_100).is_none(),
        "the swept header no longer pairs"
    );
    let newer_conflict = block(1_000, 1, b"newer-conflict");
    let p = check(&mut d, &newer_conflict, 1_100).expect("the in-window entry survives the sweep");
    assert_eq!(pair(&p), (newer.hash(), newer_conflict.hash()));
}

// R-3 — Decision: a failure means slot or wall-slot arithmetic overflows (a panic on a peer block) or the future bound drops a legitimate near-future block.
#[test]
fn inc_i_256_m1_slot_arithmetic_saturates_at_u32_max() {
    let ahead = u32::try_from(MAX_FUTURE_SLOTS).expect("FIXTURE: small bound");

    let mut top = EquivocationDetector::new();
    assert!(check(&mut top, &block(u32::MAX, 1, b"a"), u32::MAX).is_none());
    assert_eq!(top.tracked_count(), 1, "wall = slot = u32::MAX: tracked");
    let p = check(&mut top, &block(u32::MAX, 1, b"a2"), u32::MAX).expect("pairs at u32::MAX");
    assert_eq!(p.slot, u32::MAX);

    let mut near_top = EquivocationDetector::new();
    assert!(check(&mut near_top, &block(u32::MAX, 2, b"b"), u32::MAX - ahead).is_none());
    assert_eq!(
        near_top.tracked_count(),
        1,
        "slot = wall + MAX_FUTURE_SLOTS at the top: tracked"
    );

    let mut bottom = EquivocationDetector::new();
    assert!(check(&mut bottom, &block(0, 3, b"z"), 0).is_none());
    assert_eq!(bottom.tracked_count(), 1, "wall = slot = 0: tracked");
    let p = check(&mut bottom, &block(0, 3, b"z2"), 0).expect("pairs at slot 0");
    assert_eq!(p.slot, 0);

    let mut edge = EquivocationDetector::new();
    assert!(check(&mut edge, &block(ahead, 4, b"edge"), 0).is_none());
    assert_eq!(
        edge.tracked_count(),
        1,
        "slot = wall + MAX_FUTURE_SLOTS: tracked"
    );
    assert!(check(&mut edge, &block(ahead + 1, 5, b"past"), 0).is_none());
    assert!(check(&mut edge, &block(u32::MAX, 6, b"far"), 0).is_none());
    assert_eq!(edge.tracked_count(), 1, "past the future bound: skipped");
}

// R-3 — Decision: a failure means a decoy stored under the pre-gate admit-all rule blocks the valid pair after the gate, so a real double production is never slashed.
#[test]
fn inc_i_256_m1_gate_crossing_drops_decoy_then_pairs_real_headers() {
    const S: u32 = 200;
    let mut d = EquivocationDetector::new();
    d.note_gate(GATE - 1, GATE);
    let decoy = block(S, 1, b"decoy");
    assert!(d.check_block_with(&decoy, S, admit_all).is_none());
    assert_eq!(
        d.tracked_count(),
        1,
        "PRECONDITION: below the gate the decoy is stored"
    );

    d.note_gate(GATE, GATE);
    assert_eq!(
        d.tracked_count(),
        0,
        "the first crossing drops pre-gate headers"
    );

    let decoy_hash = decoy.hash();
    let real = move |h: &BlockHeader| h.hash() != decoy_hash;
    let (a, b) = (block(S, 1, b"a"), block(S, 1, b"b"));
    assert!(
        d.check_block_with(&decoy, S, real).is_none(),
        "the decoy fails the real admission"
    );
    assert!(d.check_block_with(&a, S, real).is_none());
    let p = d
        .check_block_with(&b, S, real)
        .expect("A and B pair once the decoy is gone");
    assert_eq!(
        (p.producer, p.slot, pair(&p)),
        (pk(1), S, (a.hash(), b.hash()))
    );
    let pending = d.take_pending_proofs();
    assert_eq!(pending.len(), 1, "exactly one proof");
    assert_eq!(
        pair(&pending[0]),
        (a.hash(), b.hash()),
        "the proof names A and B, never the decoy"
    );
}

// R-3 — Decision: a failure means the gate latch clears the detector again on a later block or after clear(), wiping valid post-gate evidence.
#[test]
fn inc_i_256_m1_gate_latch_survives_clear_and_fires_once() {
    let mut d = EquivocationDetector::new();
    d.note_gate(GATE, GATE);
    assert!(check(&mut d, &block(300, 1, b"e"), 300).is_none());
    d.note_gate(GATE + 1, GATE);
    assert_eq!(
        d.tracked_count(),
        1,
        "a second crossing call must not clear again"
    );

    d.clear();
    assert_eq!(d.tracked_count(), 0);
    let f = block(301, 2, b"f");
    assert!(check(&mut d, &f, 301).is_none());
    d.note_gate(GATE + 2, GATE);
    assert_eq!(d.tracked_count(), 1, "the latch survives clear()");
    let f2 = block(301, 2, b"f2");
    let p = check(&mut d, &f2, 301).expect("the header stored after clear() still pairs");
    assert_eq!(pair(&p), (f.hash(), f2.hash()));

    let mut frozen = EquivocationDetector::new();
    assert!(check(&mut frozen, &block(400, 3, b"g"), 400).is_none());
    frozen.note_gate(u64::MAX - 1, u64::MAX);
    assert_eq!(frozen.tracked_count(), 1, "a frozen gate is never crossed");
}
