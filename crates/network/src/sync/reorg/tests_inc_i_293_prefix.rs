//! INC-I-293 S-1 (run 649, operator ruling 1; SEC-LG-648-001 / REV-648-006): a
//! candidate whose weighed set is already ours (its blocks are on our chain) never
//! wins a tie, at every door and for every caller. Run 648: a completed walk whose
//! tip A1 was already canonical entered `check_reorg_supported`; `[A1..T]` and
//! `[A1]` tied (no block above A1 added a supporter), the lower-hash rule switched,
//! and the node rolled back 55 blocks. `0737243a` fenced only the walk caller
//! (`bins/node/src/node/fork_walk.rs:179`); the doors themselves have no prefix rule.
//!
// OUTPUT CONTRACT: fn ReorgHandler::check_reorg_supported(block, current_tip, w,
//     get_height, get_support, finality) -> Option<ReorgResult>
//   O1: Some/None (Some = a rollback list the caller can execute)
//   O2: ReorgResult.rollback (the blocks the caller would roll back)
//   O3: ReorgResult.weight_delta (callers switch on > 0, or on == 0 with the lower hash)
// OUTPUT CONTRACT: fn ReorgHandler::plan_reorg_with_support(current_tip, new_tip,
//     get_parent, get_height, get_support, finality) -> Option<ReorgResult>  O1, O2, O3
// PATHS: P1 candidate canonical (a prefix of our chain) | P2 candidate a sibling off
//   our chain; INC-I-288 gate active (0) | dormant (u64::MAX)
// INPUT PARTITIONS: X1 every block A1..T by producer P (bond 1); X2 A1 by P (bond 1),
//   B2..T by a zero-bond producer Z; X3 sibling X on F heavier (bond 2); X4 sibling
//   X on F tied (bond 1) with hash(X) < hash(T)
// MATRIX: d1 P1xX1 active check O1-O3 (red) · d1z P1xX2 active check O1 (red) ·
//   d1p P1xX2 active plan O1 (red) · d3 P1xX2 active walk decision O2 (red) ·
//   c1 P2xX3 active check+plan O1,O3 · c2 P2xX4 active check O1,O3 ·
//   pin P1xX1 dormant check+plan O1,O3

use std::collections::HashMap;

use super::tests_m5_common::{fin, fork_block};
use super::*;

const ACTIVE: u64 = 0;
const DORMANT: u64 = u64::MAX;
/// Real heights: F = 1, A1 = 2, B2..B8 = 3..9 (T = B8).
const TIP_H: u64 = 9;

fn h(label: &[u8]) -> Hash {
    crypto::hash::hash(label)
}

/// A block on `prev` whose hash is below `target` and not in `skip` (fixed slots).
fn block_below(prev: Hash, target: Hash, skip: &[Hash]) -> Block {
    (0u32..100_000)
        .map(|s| fork_block(prev, s))
        .find(|b| b.hash() < target && !skip.contains(&b.hash()))
        .expect("fixture: a block hashing below the tip within 100k slots")
}

/// Our chain F, A1, B2..B8 (tip T), every block recorded at its real height.
struct Rig {
    rh: ReorgHandler,
    f: Hash,
    a1: Block,
    t: Hash,
    /// Our side, tip first: [T, B7, .., B2, A1].
    ours: Vec<Hash>,
    parents: HashMap<Hash, Hash>,
    heights: HashMap<Hash, u64>,
    support: HashMap<Hash, BlockSupport>,
}

const P: SupporterId = [0xA1; 32];
const Z: SupporterId = [0x0B; 32];

fn rig(gate: u64, above_a1: (SupporterId, u64)) -> Rig {
    let mut rh = ReorgHandler::with_activation_heights(0, 0).with_inc_i_288_gate(gate);
    let f = h(b"S1-F");
    let t = h(b"S1-B8");
    let a1 = block_below(f, t, &[]);
    assert!(
        a1.hash() < t,
        "fixture: hash(A1) < hash(T), the lower-hash rule picks A1"
    );
    let mut parents = HashMap::from([(f, Hash::ZERO), (a1.hash(), f)]);
    let mut heights = HashMap::from([(f, 1), (a1.hash(), 2)]);
    let mut support = HashMap::new();
    let sup = |parent: Hash, (id, w): (SupporterId, u64)| BlockSupport {
        parent,
        ..BlockSupport::single(id, w)
    };
    support.insert(a1.hash(), sup(f, (P, 1)));
    rh.record_block_with_height(f, Hash::ZERO, 1, 1);
    rh.record_block_with_height(a1.hash(), f, 1, 2);
    let mut prev = a1.hash();
    let mut ours = vec![a1.hash()];
    for i in 2..=8u8 {
        let b = if i == 8 { t } else { h(&[b'S', b'1', b'B', i]) };
        rh.record_block_with_height(b, prev, 1, i as u64 + 1);
        parents.insert(b, prev);
        heights.insert(b, i as u64 + 1);
        support.insert(b, sup(prev, above_a1));
        ours.push(b);
        prev = b;
    }
    ours.reverse();
    assert_eq!(ours[0], t, "fixture: tip T = B8");
    Rig {
        rh,
        f,
        a1,
        t,
        ours,
        parents,
        heights,
        support,
    }
}

impl Rig {
    fn check(&self, cand: &Block) -> Option<ReorgResult> {
        let (hs, ss) = (&self.heights, &self.support);
        self.rh.check_reorg_supported(
            cand,
            self.t,
            1,
            |x| hs.get(x).copied(),
            |x| ss.get(x).cloned(),
            fin(None, None, TIP_H),
        )
    }

    fn plan(&self, new_tip: Hash) -> Option<ReorgResult> {
        let (ps, hs, ss) = (&self.parents, &self.heights, &self.support);
        self.rh.plan_reorg_with_support(
            self.t,
            new_tip,
            |x| ps.get(x).copied(),
            |x| hs.get(x).copied(),
            |x| ss.get(x).cloned(),
            fin(None, None, TIP_H),
        )
    }

    /// A sibling of A1 on F (not on our chain) backed by Q at `w`.
    fn sibling(&mut self, w: u64) -> Block {
        let x = block_below(self.f, self.t, &[self.a1.hash()]);
        let s = BlockSupport {
            parent: self.f,
            ..BlockSupport::single([0x51; 32], w)
        };
        // The walk records each walked block before the door (`fork_recovery.rs:51`).
        self.rh
            .record_fork_block_with_height(x.hash(), self.f, w, 2);
        self.support.insert(x.hash(), s);
        self.parents.insert(x.hash(), self.f);
        self.heights.insert(x.hash(), 2);
        x
    }
}

/// The walk caller's decision without the R1 fence: `fork_recovery.rs:73-105`
/// (check door, then `delta > 0 || (delta == 0 && tip < current)`) and on `None`
/// `fork_recovery.rs:127-153` (plan door, same rule). Returns the executed rollback.
fn walk_decision(r: &Rig, tip: &Block) -> Option<Vec<Hash>> {
    let switch = |p: &ReorgResult| p.weight_delta > 0 || (p.weight_delta == 0 && tip.hash() < r.t);
    match r.check(tip) {
        Some(p) => switch(&p).then_some(p.rollback),
        None => r.plan(tip.hash()).filter(switch).map(|p| p.rollback),
    }
}

// d1 — Decision: a failure means the gossip/walk door still lets a candidate on our
// own chain win a tie: [A1..T] vs [A1] tie and the lower hash rolls back T..A1.
#[test]
fn d1_check_door_keeps_our_tip_for_a_canonical_candidate_on_a_tie() {
    let r = rig(ACTIVE, (P, 1));
    let got = r.check(&r.a1);
    assert!(
        got.is_none(),
        "d1: the door weighed A1 (already canonical) against [A1..T] and returned a reorg: \
         rollback={} blocks (A1 inside: {}), weight_delta={:?}",
        got.as_ref().map_or(0, |p| p.rollback.len()),
        got.as_ref()
            .is_some_and(|p| p.rollback.contains(&r.a1.hash())),
        got.as_ref().map(|p| p.weight_delta),
    );
}

// d1z — Decision: a failure means the door rule depends on who produced the blocks
// above A1: zero-bond producers above A1 leave the same tie and the same rollback.
#[test]
fn d1z_check_door_keeps_our_tip_when_blocks_above_the_prefix_weigh_zero() {
    let r = rig(ACTIVE, (Z, 0));
    let got = r.check(&r.a1);
    assert!(
        got.is_none(),
        "d1z: canonical A1 won a tie at the check door: rollback={:?} delta={:?}",
        got.as_ref().map(|p| p.rollback.len()),
        got.as_ref().map(|p| p.weight_delta),
    );
}

// d1p — Decision: a failure means fixing only the check door moves the stale walk to
// the fallback plan door: new_tip A1 is canonical, new_chain is empty, both sides
// weigh 0, delta 0, and the caller's lower-hash rule rolls back B2..T.
#[test]
fn d1p_plan_door_refuses_a_canonical_new_tip() {
    let r = rig(ACTIVE, (Z, 0));
    let got = r.plan(r.a1.hash());
    assert!(
        got.is_none(),
        "d1p: plan door returned a plan to a block already on our chain: rollback={:?} \
         new_blocks={:?} delta={:?}",
        got.as_ref().map(|p| p.rollback.len()),
        got.as_ref().map(|p| p.new_blocks.len()),
        got.as_ref().map(|p| p.weight_delta),
    );
}

// d3 — Decision: a failure means removing the R1 fence (`fork_walk.rs:179`) re-opens
// the run-648 rollback: the walk's exact door sequence executes a rollback of our chain.
#[test]
fn d3_walk_decision_without_the_fence_rolls_back_nothing() {
    for (label, above) in [
        ("X1 one producer", (P, 1)),
        ("X2 zero-bond above A1", (Z, 0)),
    ] {
        let r = rig(ACTIVE, above);
        let rolled = walk_decision(&r, &r.a1);
        assert_eq!(
            rolled, None,
            "d3 {label}: a stale walk to canonical A1 rolled back our chain"
        );
    }
}

// c1 — Decision: a failure means the prefix rule also refuses a REAL heavier sibling
// off our chain (the fix over-reached): both doors must still plan a positive delta.
#[test]
fn c1_heavier_sibling_off_our_chain_still_reorgs() {
    let mut r = rig(ACTIVE, (P, 1));
    let x = r.sibling(2);
    let got = r
        .check(&x)
        .expect("c1: check door reorgs to the heavier sibling");
    assert_eq!(got.weight_delta, 1, "c1: Q (2) - P (1)");
    assert_eq!(got.rollback, r.ours, "c1: rollback = T..A1");
    let plan = r
        .plan(x.hash())
        .expect("c1: plan door plans the heavier sibling");
    assert_eq!(
        (plan.weight_delta, plan.new_blocks),
        (1, vec![x.hash()]),
        "c1: plan"
    );
}

// c2 — Decision: a failure means the fix removed the tie rule for real siblings: a
// tied sibling off our chain with the lower hash must still switch (INC-I-012).
#[test]
fn c2_tied_sibling_off_our_chain_with_lower_hash_still_switches() {
    let mut r = rig(ACTIVE, (P, 1));
    let x = r.sibling(1);
    assert!(x.hash() < r.t, "fixture: hash(X) < hash(T)");
    let got = r
        .check(&x)
        .expect("c2: a tied lower-hash sibling still switches");
    assert_eq!(
        (got.weight_delta, got.rollback.len()),
        (0, 8),
        "c2: tie, rollback T..A1"
    );
    assert_eq!(
        walk_decision(&r, &x),
        Some(r.ours.clone()),
        "c2: the walk caller switches"
    );
}

// pin — Decision: a failure means the below-gate doors changed for a prefix
// candidate. Below the INC-I-288 gate the accumulated depth rule makes the prefix
// strictly lighter (2 vs 9), so neither door yields a plan a caller executes.
#[test]
fn pin_below_gate_a_prefix_candidate_is_strictly_lighter() {
    let r = rig(DORMANT, (P, 1));
    assert!(
        r.check(&r.a1).is_none(),
        "pin: below gate the check door keeps T"
    );
    let plan = r.plan(r.a1.hash());
    assert!(
        plan.as_ref().is_none_or(|p| p.weight_delta < 0),
        "pin: below gate the plan door is strictly lighter: delta={:?}",
        plan.as_ref().map(|p| p.weight_delta)
    );
    assert_eq!(
        walk_decision(&r, &r.a1),
        None,
        "pin: no rollback below gate"
    );
}
