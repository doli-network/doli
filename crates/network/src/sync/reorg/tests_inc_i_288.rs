//! INC-I-288 round 4 unit cells for the ReorgHandler side of the gate.
//!
// OUTPUT CONTRACT:
//   fn check_reorg_weighted(..) -> Option<ReorgResult>   O1 Some/None, O2 weight_delta
//   fn plan_reorg_with_weights(..) -> Option<ReorgResult> O1 Some, O2 weight_delta
//   fn record_fork_block_with_height(..) (LRU)            O3 knows_block of an older block
// PATHS: gate dormant (u64::MAX) | gate active (0)
// INPUT PARTITIONS:
//   P1 our tip tracked, our weight scalar stale (0), one-block candidate on F
//   P2 our side untracked (store-only), candidate tracked
//   P3 one hash re-recorded past max_tracked
// MATRIX: P1 x both O1,O2 · P2 x both O1,O2 · P3 x both O3

use std::collections::HashMap;

use super::tests_m5_common::{fin, fork_block};
use super::*;

const DORMANT: u64 = u64::MAX;

fn h(label: &[u8]) -> Hash {
    crypto::hash::hash(label)
}

/// F at height 1 on genesis, then A1..A3 (weight 1 each). Returns (F, A3).
fn our_branch(rh: &mut ReorgHandler) -> (Hash, Hash) {
    let f = h(b"F");
    rh.record_block_with_height(f, Hash::ZERO, 1, 1);
    let mut prev = f;
    for i in 1..=3u8 {
        let a = h(&[b'A', i]);
        rh.record_block_with_height(a, prev, 1, 1 + i as u64);
        prev = a;
    }
    (f, prev)
}

fn gossip_cell(gate: u64) -> Option<ReorgResult> {
    let mut rh = ReorgHandler::new().with_inc_i_288_gate(gate);
    let (f, a3) = our_branch(&mut rh);
    rh.set_current_weight(0);
    let x = fork_block(f, 99);
    rh.check_reorg_weighted(&x, a3, 1, |_| None, fin(None, None, 4))
}

#[test]
fn inc_i_288_gossip_door_weighs_from_the_ancestor_at_the_gate() {
    let old = gossip_cell(DORMANT).expect("dormant: stale scalar 0 lets X win");
    assert_eq!(old.weight_delta, 2, "dormant: parent acc 1 + 1 - stale 0");
    assert!(
        gossip_cell(0).is_none(),
        "active: X (1) is lighter than A1..A3 (3) above F"
    );
}

fn plan_cell(gate: u64) -> i64 {
    let mut rh = ReorgHandler::new().with_inc_i_288_gate(gate);
    let f = h(b"F");
    let a: Vec<Hash> = (1..=3u8).map(|i| h(&[b'A', i])).collect();
    let s = h(b"S");
    rh.record_fork_block_with_height(s, f, 1, 2);
    let parents: HashMap<Hash, Hash> = [(a[2], a[1]), (a[1], a[0]), (a[0], f), (f, Hash::ZERO)]
        .into_iter()
        .collect();
    rh.plan_reorg_with_weights(
        a[2],
        s,
        |x| parents.get(x).copied(),
        |_| None,
        |x| a.contains(x).then_some(1),
        fin(None, None, 4),
    )
    .expect("common ancestor F")
    .weight_delta
}

#[test]
fn inc_i_288_plan_weighs_an_untracked_tip_from_the_store_at_the_gate() {
    assert_eq!(plan_cell(DORMANT), 1, "dormant: S acc 1 - untracked tip 0");
    assert_eq!(plan_cell(0), -2, "active: S 1 - A1..A3 3 above F");
}

fn flood_cell(gate: u64) -> bool {
    let mut rh = ReorgHandler::new().with_inc_i_288_gate(gate);
    rh.max_tracked = 4;
    let a = h(b"A");
    rh.record_block_with_height(a, Hash::ZERO, 1, 1);
    for _ in 0..10 {
        rh.record_fork_block_with_height(h(b"S"), Hash::ZERO, 1, 1);
    }
    rh.knows_block(&a)
}

#[test]
fn inc_i_288_re_records_keep_one_lru_slot_at_the_gate() {
    assert!(!flood_cell(DORMANT), "dormant: duplicate pushes evict A");
    assert!(flood_cell(0), "active: S keeps one slot, A stays tracked");
}
