// INC-I-281 M2.1 — R-3a (Must), root cause RC-B: recovery doors judge a block at the
// epoch-boundary height against the node's post-rotation list instead of the list in
// force at the block's parent.
//
// OUTPUT CONTRACT: the four recovery doors
// Functions: Node::handle_completed_fork_recovery (walk), Node::handle_new_block ->
//   retain_sibling_and_try_escape (wedge), Node::try_apply_cached_chain (cached),
//   Node::try_consume_force_reorg (force); shared gate check_producer_eligibility.
// OUTPUTS:
//   O1 return value (walk/cached Result, force ForceReorgOutcome)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at the boundary height B
//   O4 state_db persisted chain-state tip
//   O5 reorg-handler fork weight of the block at B (set only past the eligibility gate)
//   O6 wedge_retained_tips membership of the block at B (wedge door)
//   O7 fork_block_cache entry of the block at B (cached door removes it past the gate)
// PATHS: walk | wedge | cached | force
// INPUT PARTITIONS: block at B = F + 1, parent F = last block of the closing epoch
//   close-only: producer = L_close[slot % |L_close|], not L_new[slot % |L_new|]
//   dead-only:  producer = L_new[slot % |L_new|], not L_close[slot % |L_close|]
//   neither:    a registered, scheduled producer at the wrong index of both lists
// MATRIX: O1-O5 x {walk, wedge, force} x {close-only, dead-only, neither}; O6 on wedge;
//   node tip = dead D2 (its own boundary block D1 at B rotated it to L_new).
//   cached door: its only reachable state is tip = F (the chain must connect to the
//   tip), here after a fork-remedy rollback of D2, D1; O1/O2/O7 x all partitions.
// covers: validation_checks/mod.rs door_epoch fork_walk wedge_escape fork_recovery force_reorg
// covers: bins/node/src/node/validation_checks/mod.rs
// covers: bins/node/src/node/validation_checks/door_epoch.rs
// covers: bins/node/src/node/fork_walk.rs
// covers: bins/node/src/node/wedge_escape.rs
// covers: bins/node/src/node/fork_recovery.rs
// covers: bins/node/src/node/force_reorg.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, EpochState};
use doli_node::node::{ForceReorgOutcome, Node, RollbackAuthority};
use inc_i_236_m2_reorg_support::{block, cached, seed_cache, weight_recorded, Sig};
use network::sync::CompletedRecovery;
use tempfile::TempDir;

/// Last block of epoch 1 on devnet (blocks_per_reward_epoch = 4).
const F: u64 = 7;
/// The epoch-boundary height: validated under L_close, rotates to L_new after commit.
const B: u64 = F + 1;
const D1_SLOT: u32 = 109;
const D2_SLOT: u32 = 110;
/// Slot of every competing block at B: L_close[112 % 4] != L_new[112 % 3].
const S: u32 = 112;

struct Fx {
    node: Node,
    keys: Vec<KeyPair>,
    params: ConsensusParams,
    f_hash: Hash,
    l_close: Vec<PublicKey>,
    l_new: Vec<PublicKey>,
    dead: Vec<Block>,
    _tmp: TempDir,
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    best_hash: Hash,
    best_height: u64,
    canonical_b: Option<Hash>,
    persisted_tip: Option<Hash>,
}

fn active(es: &EpochState) -> Vec<PublicKey> {
    if es.active_list.is_empty() {
        es.producer_list.clone()
    } else {
        es.active_list.clone()
    }
}

fn at(list: &[PublicKey], slot: u32) -> PublicKey {
    list[slot as usize % list.len()]
}

impl Fx {
    fn key(&self, pk: &PublicKey) -> &KeyPair {
        self.keys
            .iter()
            .find(|k| k.public_key() == pk)
            .expect("fixture: producer key")
    }

    /// A branch at B on F: first block by `first`, then `rest` slots by their L_new leader.
    fn branch(&self, first: PublicKey, rest: &[u32]) -> Vec<Block> {
        let mut out = vec![block(
            B,
            S,
            self.f_hash,
            self.key(&first),
            Sig::Valid,
            &self.params,
        )];
        for (i, slot) in rest.iter().enumerate() {
            let prev = out.last().unwrap().hash();
            let pk = at(&self.l_new, *slot);
            out.push(block(
                B + 1 + i as u64,
                *slot,
                prev,
                self.key(&pk),
                Sig::Valid,
                &self.params,
            ));
        }
        out
    }

    /// Canonical: C1 by L_close[S % 4] (eligible at its parent F), C2, C3 by L_new.
    fn canonical(&self) -> Vec<Block> {
        self.branch(at(&self.l_close, S), &[113, 115])
    }

    /// X1 by L_new[S % 3] only (eligible under the dead list), X2, X3 by L_new.
    fn dead_only(&self) -> Vec<Block> {
        self.branch(at(&self.l_new, S), &[113, 114])
    }

    /// Y1 by a scheduled producer at the wrong index of both lists, Y2, Y3 by L_new.
    fn neither(&self) -> Vec<Block> {
        let (c, d) = (at(&self.l_close, S), at(&self.l_new, S));
        let wrong = *self
            .l_new
            .iter()
            .find(|pk| **pk != c && **pk != d)
            .expect("fixture: a third scheduled producer");
        self.branch(wrong, &[115, 117])
    }

    async fn state(&self) -> State {
        let (best_hash, best_height) = {
            let cs = self.node.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        State {
            best_hash,
            best_height,
            canonical_b: self.node.block_store.get_hash_by_height(B).expect("store"),
            persisted_tip: self.node.state_db.get_chain_state().map(|c| c.best_hash),
        }
    }
}

/// Base h=1..=F by three of four producers (the fourth never produces, so the
/// attestation filter drops it at B), then the node's own dead D1 at B and D2.
async fn fixture() -> Fx {
    let tmp = TempDir::new().unwrap();
    let mut keys: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();
    keys.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(
            h,
            100 + h as u32,
            prev,
            &keys[(h as usize - 1) % 3],
            Sig::Valid,
            &params,
        );
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let l_close = active(&node.epoch_state);
    let d1_pk = at(&l_close, D1_SLOT);
    let d1_key = keys
        .iter()
        .find(|k| *k.public_key() == d1_pk)
        .unwrap()
        .clone();
    let d1 = block(B, D1_SLOT, prev, &d1_key, Sig::Valid, &params);
    node.apply_block(d1.clone(), ValidationMode::Light)
        .await
        .expect("fixture: dead D1");
    let l_new = active(&node.epoch_state);
    let d2_pk = at(&l_new, D2_SLOT);
    let d2_key = keys
        .iter()
        .find(|k| *k.public_key() == d2_pk)
        .unwrap()
        .clone();
    let d2 = block(B + 1, D2_SLOT, d1.hash(), &d2_key, Sig::Valid, &params);
    node.apply_block(d2.clone(), ValidationMode::Light)
        .await
        .expect("fixture: dead D2");
    let fx = Fx {
        node,
        keys,
        params,
        f_hash: prev,
        l_close,
        l_new,
        dead: vec![d1, d2],
        _tmp: tmp,
    };
    assert_preconditions(&fx).await;
    fx
}

/// The RC-B shape, checked on every fixture: the lists differ at S, and the
/// pre-rotation list is recoverable from the undo record of the node's own D1.
async fn assert_preconditions(fx: &Fx) {
    assert_eq!(
        fx.l_close.len(),
        4,
        "precondition: L_close = all four producers"
    );
    assert_eq!(
        fx.l_new.len(),
        3,
        "precondition: L_new drops the silent producer"
    );
    assert_ne!(
        at(&fx.l_close, S),
        at(&fx.l_new, S),
        "precondition: the two lists name different leaders at slot S"
    );
    assert!(
        fx.l_new.contains(&at(&fx.l_close, S)),
        "precondition: C1's producer stays scheduled, so L_new(C1) == L_new(D1)"
    );
    let undo = fx
        .node
        .state_db
        .get_undo(B)
        .expect("precondition: undo(D1)");
    let snap = EpochState::deserialize(&undo.epoch_state_snapshot.expect("undo snapshot"))
        .expect("precondition: undo(D1) epoch state decodes");
    assert_eq!(
        active(&snap),
        fx.l_close,
        "precondition: undo(D1) holds L_close"
    );
    let tip = fx.node.chain_state.read().await.best_hash;
    assert_eq!(tip, fx.dead[1].hash(), "precondition: tip = dead D2");
}

async fn run_walk(fx: &mut Fx, br: &[Block]) -> anyhow::Result<()> {
    let recovery = CompletedRecovery {
        blocks: br.to_vec(),
        connection_point: fx.f_hash,
        suppliers: Vec::new(),
    };
    fx.node.handle_completed_fork_recovery(recovery).await
}

/// Gossips the branch in order; returns whether the block at B was retained.
async fn run_wedge(fx: &mut Fx, br: &[Block]) -> bool {
    let peer = network::PeerId::random();
    fx.node
        .handle_new_block(br[0].clone(), peer)
        .await
        .expect("wedge: handle_new_block at B");
    let retained = fx.node.wedge_retained_tips.contains(&br[0].hash());
    for b in &br[1..] {
        fx.node
            .handle_new_block(b.clone(), peer)
            .await
            .expect("wedge: handle_new_block");
    }
    retained
}

async fn run_force(fx: &mut Fx, br: &[Block]) -> Option<ForceReorgOutcome> {
    seed_cache(&fx.node, br).await;
    let target = br.last().unwrap().hash();
    fx.node.sync_manager.write().await.arm_force_reorg(target);
    fx.node.try_consume_force_reorg().await
}

/// Fork-remedy rollback of D2 and D1: tip = F, the cached door's reachable state.
async fn roll_back_to_f(fx: &mut Fx) {
    for _ in 0..2 {
        fx.node
            .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
            .await
            .expect("rollback");
    }
    assert_eq!(
        fx.state().await.best_hash,
        fx.f_hash,
        "precondition: tip = F"
    );
    assert_eq!(
        active(&fx.node.epoch_state),
        fx.l_close,
        "precondition: rollback restored L_close"
    );
}

async fn run_cached(fx: &mut Fx, br: &[Block]) -> anyhow::Result<()> {
    seed_cache(&fx.node, &br[..br.len() - 1]).await;
    fx.node
        .try_apply_cached_chain(br.last().unwrap().clone())
        .await
}

async fn assert_on(fx: &Fx, br: &[Block], label: &str) {
    let st = fx.state().await;
    let tip = br.last().unwrap();
    assert_eq!(
        st.best_hash,
        tip.hash(),
        "{label}: tip is the canonical tip"
    );
    assert_eq!(st.best_height, B + br.len() as u64 - 1, "{label}: height");
    assert_eq!(
        st.canonical_b,
        Some(br[0].hash()),
        "{label}: canonical block at B"
    );
    assert_eq!(st.persisted_tip, Some(tip.hash()), "{label}: persisted tip");
}

fn eligibility_refusal(result: &anyhow::Result<()>) -> bool {
    result
        .as_ref()
        .err()
        .is_some_and(|e| e.to_string().contains("FORK_INVALID_PRODUCER"))
}

fn probe(door: &str, refused: bool) {
    println!("INC281_PROBE door={door} refused={}", refused as u8);
}

/// Runs a refused-partition branch through every door; returns the doors that let
/// its block at B past the schedule check (empty = all refused before any rollback).
async fn doors_accepting(make: fn(&Fx) -> Vec<Block>) -> Vec<String> {
    let mut accepted = Vec::new();

    let mut fx = fixture().await;
    let br = make(&fx);
    let before = fx.state().await;
    let result = run_walk(&mut fx, &br).await;
    if fx.state().await != before || weight_recorded(&fx.node, &br[0].hash()).await {
        accepted.push(format!("walk ({result:?})"));
    }

    let mut fx = fixture().await;
    let br = make(&fx);
    let before = fx.state().await;
    let retained = run_wedge(&mut fx, &br).await;
    if retained || fx.state().await != before || weight_recorded(&fx.node, &br[0].hash()).await {
        accepted.push(format!("wedge (retained={retained})"));
    }

    let mut fx = fixture().await;
    let br = make(&fx);
    let before = fx.state().await;
    let outcome = run_force(&mut fx, &br).await;
    if outcome != Some(ForceReorgOutcome::Ineligible) || fx.state().await != before {
        accepted.push(format!("force ({outcome:?})"));
    }

    let mut fx = fixture().await;
    roll_back_to_f(&mut fx).await;
    let br = make(&fx);
    let before = fx.state().await;
    let result = run_cached(&mut fx, &br).await;
    let gated = cached(&fx.node, &br[0].hash()).await.is_some();
    if !eligibility_refusal(&result) || !gated || fx.state().await != before {
        accepted.push(format!("cached ({result:?})"));
    }

    accepted
}

// R-3a (Must) — Decision: a failure means the fork walk still judges the canonical boundary block by the dead branch's post-rotation list, so a minority on a boundary-straddling fork can never walk back.
#[tokio::test]
async fn inc_i_281_walk_door_accepts_boundary_block_eligible_at_parent() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let result = run_walk(&mut fx, &canon).await;
    let landed = fx.state().await.best_hash == canon[2].hash();
    let refused = !landed && !weight_recorded(&fx.node, &canon[0].hash()).await;
    probe("walk", refused);
    assert!(
        !refused,
        "walk: C1 is L_close[S % 4] at its parent F but was refused on eligibility ({result:?})"
    );
    result.expect("walk: Ok");
    assert_on(&fx, &canon, "walk").await;
}

// R-3a (Must) — Decision: a failure means the wedge escape still drops the canonical boundary sibling on eligibility, the silent drop seen on every minority node.
#[tokio::test]
async fn inc_i_281_wedge_door_accepts_boundary_block_eligible_at_parent() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let retained = run_wedge(&mut fx, &canon).await;
    probe("wedge", !retained);
    assert!(
        retained,
        "wedge: sibling C1 is L_close[S % 4] at its parent F but was dropped on eligibility"
    );
    assert_on(&fx, &canon, "wedge").await;
}

// R-3a (Must) — Decision: a failure means the fix broke the one door that judges the boundary block correctly today (tip = F after a fork-remedy rollback).
#[tokio::test]
async fn inc_i_281_cached_chain_door_accepts_boundary_block_eligible_at_parent() {
    let mut fx = fixture().await;
    roll_back_to_f(&mut fx).await;
    let canon = fx.canonical();
    let result = run_cached(&mut fx, &canon).await;
    let refused =
        eligibility_refusal(&result) || cached(&fx.node, &canon[0].hash()).await.is_some();
    probe("cached", refused);
    assert!(
        !refused,
        "cached: C1 is L_close[S % 4] at the tip F but was refused ({result:?})"
    );
    result.expect("cached: Ok");
    assert_on(&fx, &canon, "cached").await;
}

// R-3a (Must) — Decision: a failure means an operator forceReorgTo onto the canonical branch is still refused as Ineligible on a boundary-straddling fork.
#[tokio::test]
async fn inc_i_281_force_reorg_door_accepts_boundary_block_eligible_at_parent() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let outcome = run_force(&mut fx, &canon).await;
    probe("force", outcome == Some(ForceReorgOutcome::Ineligible));
    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Executed),
        "force: C1 is L_close[S % 4] at its parent F"
    );
    assert_on(&fx, &canon, "force").await;
}

// R-3a (Must) — Decision: a failure means a door accepts a boundary block scheduled only by the dead branch's list, i.e. the fix relaxed the schedule instead of re-targeting it.
#[tokio::test]
async fn inc_i_281_doors_refuse_block_eligible_only_under_dead_list() {
    let accepted = doors_accepting(Fx::dead_only).await;
    assert!(
        accepted.is_empty(),
        "doors that let X1 = L_new[S % 3] (not L_close[S % 4]) past the schedule check: {accepted:?}"
    );
}

// R-3a (Must) — Decision: a failure means a door accepts a boundary block that no list schedules (schedule check lost before rollback).
#[tokio::test]
async fn inc_i_281_doors_refuse_block_eligible_under_neither_list() {
    let accepted = doors_accepting(Fx::neither).await;
    assert!(
        accepted.is_empty(),
        "doors that let Y1 (scheduled by neither list at S) past the schedule check: {accepted:?}"
    );
}
