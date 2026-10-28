// INC-I-281 M2.3 — P1 (R-3a residual of RC-B, Must): the list that judges a block after a
// boundary block depends on that boundary block. A walked branch whose boundary block C_B
// is not in the node's chain rotates to L_new(C_B), which the node cannot derive before
// applying C_B; the doors judge C_{B+1} with the node's L_new(D_B) and Light-mode
// execute_reorg never checks the schedule at all.
//
// OUTPUT CONTRACT: recovery of a node on [.., F, D_B, D_{B+1}] onto a branch on F or D_B.
// Functions: Node::handle_completed_fork_recovery (walk door, fork choice, execute_reorg),
//   Node::handle_new_block -> retain_sibling_and_try_escape (wedge door),
//   Node::execute_reorg (per-block schedule check at the block's parent, INC-I-010 layer-3
//   Light gating); shared gate check_producer_eligibility_at -> door_epoch_list.
// OUTPUTS:
//   O1 return value (Ok on the walk / wedge paths; a door refusal is not an Err)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at B, B+1, B+2; body presence of a refused block
//   O4 state_db persisted chain-state tip
//   O5 reorg-handler weight of walked blocks (recorded only past the walk door)
//   O6 wedge_retained_tips membership (the wedge door verdict)
//   O7 node epoch list after landing (= L_new(C_B), re-derived by applying C_B)
// PATHS: walk door | wedge door | execute_reorg apply loop x {Full gate, Light gate}
// INPUT PARTITIONS (block x derivable from local state x producer):
//   C_{B+1}  after C_B (not the node's): L_new(C_B)[s1] != L_new(D_B)[s1] -> admitted, applied
//   C'_{B+1} after C_B: L_new(D_B)[s1], not L_new(C_B)[s1]        -> door admits, reorg refuses
//   X_{B+1}  after C_B: unregistered key                           -> door refuses (membership)
//   Y_B      parent F (derivable): not L_close[slot]                -> door refuses
//   Z_{B+1}  parent D_B (the node's own boundary, derivable): not L_new(D_B)[s1]
//            -> doors refuse; direct execute_reorg refuses (Full) | applies (Light gate)
// MATRIX: walk x {C, C', X, Y, Z}; wedge x {C, X, Y, Z}; execute_reorg x Z x {Full, Light}.
//   O2-O4 in every cell; O5 on walk cells; O6 on wedge cells; O7 in reorg_lands.
//   C_{B+2} / C'_{B+2} / X_{B+2} / Z_{B+2}: slot s2 where both lists name the same leader,
//   so only the B+1 block discriminates; a 3-block branch outweighs the node's 2 blocks.
// covers: door_epoch validation_checks wedge_escape fork_walk force_reorg block_handling reorg_schedule
// covers: bins/node/src/node/validation_checks/door_epoch.rs
// covers: bins/node/src/node/validation_checks/mod.rs
// covers: bins/node/src/node/wedge_escape.rs
// covers: bins/node/src/node/fork_walk.rs
// covers: bins/node/src/node/force_reorg.rs
// covers: bins/node/src/node/block_handling.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, EpochState};
use doli_node::node::Node;
use inc_i_236_m2_reorg_support::{block, seed_cache, weight_recorded, Sig};
use network::sync::{CompletedRecovery, ReorgResult};
use network::PeerId;
use tempfile::TempDir;

/// Last block of epoch 1 on devnet (blocks_per_reward_epoch = 4).
const F: u64 = 7;
/// Boundary height: judged under L_close, rotates to the next list after commit.
const B: u64 = F + 1;
/// D_B = L_close[111 % 4]: the producer silent through F, so L_new(D_B) keeps it.
const D_B_SLOT: u32 = 111;
const D_B1_SLOT: u32 = 113;
/// C_B = L_close[112 % 4], a producer that already produced: L_new(C_B) drops the silent one.
const C_B_SLOT: u32 = 112;

struct Fx {
    node: Node,
    keys: Vec<KeyPair>,
    params: ConsensusParams,
    f_hash: Hash,
    l_close: Vec<PublicKey>,
    l_dead: Vec<PublicKey>,
    l_canon: Vec<PublicKey>,
    d: [Block; 2],
    c_b: Block,
    /// First slot after D_{B+1} where L_new(C_B) and L_new(D_B) name different leaders.
    s1: u32,
    /// First slot after `s1` where they name the same leader.
    s2: u32,
    _tmp: TempDir,
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    best_hash: Hash,
    best_height: u64,
    at_b: Option<Hash>,
    at_b1: Option<Hash>,
    at_b2: Option<Hash>,
    persisted: Option<Hash>,
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

fn key_of<'a>(keys: &'a [KeyPair], pk: &PublicKey) -> &'a KeyPair {
    keys.iter()
        .find(|k| k.public_key() == pk)
        .expect("fixture: producer key")
}

/// Base h=1..=F by three of four producers; the fourth (largest key) never produces.
async fn base_node(tmp: &TempDir, keys: &[KeyPair]) -> (Node, ConsensusParams, Hash) {
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let by = &keys[(h as usize - 1) % 3];
        let b = block(h, 100 + h as u32, prev, by, Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    (node, params, prev)
}

/// The node: base, then its own D_B (by the silent producer) and D_{B+1}. L_new(C_B) is
/// read from a twin node that applies the same base and C_B.
async fn fixture() -> Fx {
    let mut keys: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();
    keys.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let tmp = TempDir::new().unwrap();
    let (mut node, params, f_hash) = base_node(&tmp, &keys).await;
    let l_close = active(&node.epoch_state);
    let by_d_b = key_of(&keys, &at(&l_close, D_B_SLOT));
    let d_b = block(B, D_B_SLOT, f_hash, by_d_b, Sig::Valid, &params);
    node.apply_block(d_b.clone(), ValidationMode::Light)
        .await
        .expect("fixture: D_B");
    let l_dead = active(&node.epoch_state);
    let by_d_b1 = key_of(&keys, &at(&l_dead, D_B1_SLOT));
    let d_b1 = block(B + 1, D_B1_SLOT, d_b.hash(), by_d_b1, Sig::Valid, &params);
    node.apply_block(d_b1.clone(), ValidationMode::Light)
        .await
        .expect("fixture: D_B+1");
    let by_c_b = key_of(&keys, &at(&l_close, C_B_SLOT));
    let c_b = block(B, C_B_SLOT, f_hash, by_c_b, Sig::Valid, &params);
    let l_canon = {
        let twin_tmp = TempDir::new().unwrap();
        let (mut twin, _, _) = base_node(&twin_tmp, &keys).await;
        twin.apply_block(c_b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: twin C_B");
        active(&twin.epoch_state)
    };
    assert_ne!(l_canon, l_dead, "precondition: L_new(C_B) != L_new(D_B)");
    let differs = |s: &u32| at(&l_canon, *s) != at(&l_dead, *s);
    let s1 = (D_B1_SLOT + 1..D_B1_SLOT + 64)
        .find(differs)
        .expect("fixture: s1");
    let s2 = (s1 + 1..s1 + 64)
        .find(|s| !differs(s))
        .expect("fixture: s2");
    let fx = Fx {
        node,
        keys,
        params,
        f_hash,
        l_close,
        l_dead,
        l_canon,
        d: [d_b, d_b1],
        c_b,
        s1,
        s2,
        _tmp: tmp,
    };
    assert_preconditions(&fx).await;
    fx
}

/// The P1 shape: three different lists, the closing one recoverable from undo(D_B).
async fn assert_preconditions(fx: &Fx) {
    let all: Vec<PublicKey> = fx.keys.iter().map(|k| *k.public_key()).collect();
    assert_eq!(
        fx.l_close, all,
        "precondition: L_close = all four producers"
    );
    assert_eq!(
        fx.l_dead, all,
        "precondition: D_B's producer was silent, so L_new(D_B) keeps all four"
    );
    assert_eq!(
        fx.l_canon,
        all[..3].to_vec(),
        "precondition: L_new(C_B) drops the silent producer"
    );
    let undo = fx
        .node
        .state_db
        .get_undo(B)
        .expect("precondition: undo(D_B)");
    let snap = EpochState::deserialize(&undo.epoch_state_snapshot.expect("undo snapshot"))
        .expect("precondition: undo(D_B) epoch state decodes");
    assert_eq!(
        active(&snap),
        fx.l_close,
        "precondition: undo(D_B) = L_close"
    );
    assert_eq!(
        fx.state().await.best_hash,
        fx.d[1].hash(),
        "precondition: tip = D_B+1"
    );
}

impl Fx {
    fn key(&self, pk: &PublicKey) -> &KeyPair {
        key_of(&self.keys, pk)
    }

    fn mk(&self, prev: Hash, height: u64, slot: u32, by: &KeyPair) -> Block {
        block(height, slot, prev, by, Sig::Valid, &self.params)
    }

    /// [C_B, b1, b2]: b1 at s1 by `by1`, b2 at s2 by its leader under both lists.
    fn after_c_b(&self, by1: &KeyPair) -> Vec<Block> {
        let b1 = self.mk(self.c_b.hash(), B + 1, self.s1, by1);
        let by2 = self.key(&at(&self.l_canon, self.s2));
        let b2 = self.mk(b1.hash(), B + 2, self.s2, by2);
        vec![self.c_b.clone(), b1, b2]
    }

    /// Canonical: C_{B+1} = L_new(C_B)[s1].
    fn canonical(&self) -> Vec<Block> {
        self.after_c_b(self.key(&at(&self.l_canon, self.s1)))
    }

    /// C'_{B+1} = L_new(D_B)[s1]: scheduled only under the node's list.
    fn ineligible(&self) -> Vec<Block> {
        self.after_c_b(self.key(&at(&self.l_dead, self.s1)))
    }

    /// X_{B+1} by a key that is neither registered nor in the GSet.
    fn unknown(&self) -> Vec<Block> {
        self.after_c_b(&KeyPair::generate())
    }

    /// Y_B on F at C_B's slot by a registered producer that is not L_close[slot].
    fn closing_ineligible(&self) -> Vec<Block> {
        let y_by = self.key(&at(&self.l_close, C_B_SLOT + 1));
        let y_b = self.mk(self.f_hash, B, C_B_SLOT, y_by);
        let y1 = self.mk(
            y_b.hash(),
            B + 1,
            self.s1,
            self.key(&at(&self.l_canon, self.s1)),
        );
        let y2 = self.mk(
            y1.hash(),
            B + 2,
            self.s2,
            self.key(&at(&self.l_canon, self.s2)),
        );
        vec![y_b, y1, y2]
    }

    /// Z_{B+1} on the node's own D_B by L_new(C_B)[s1], which is not L_new(D_B)[s1].
    fn on_own_boundary(&self) -> Vec<Block> {
        let z1_by = self.key(&at(&self.l_canon, self.s1));
        let z1 = self.mk(self.d[0].hash(), B + 1, self.s1, z1_by);
        let z2 = self.mk(
            z1.hash(),
            B + 2,
            self.s2,
            self.key(&at(&self.l_dead, self.s2)),
        );
        vec![z1, z2]
    }

    async fn state(&self) -> State {
        let (best_hash, best_height) = {
            let cs = self.node.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        let at_h = |h: u64| self.node.block_store.get_hash_by_height(h).expect("store");
        State {
            best_hash,
            best_height,
            at_b: at_h(B),
            at_b1: at_h(B + 1),
            at_b2: at_h(B + 2),
            persisted: self.node.state_db.get_chain_state().map(|c| c.best_hash),
        }
    }

    fn applied(&self, b: &Block) -> bool {
        self.node
            .block_store
            .get_block(&b.hash())
            .expect("store")
            .is_some()
    }

    fn retained(&self, b: &Block) -> bool {
        self.node.wedge_retained_tips.contains(&b.hash())
    }
}

async fn run_walk(fx: &mut Fx, br: &[Block], connection_point: Hash) -> anyhow::Result<()> {
    let recovery = CompletedRecovery {
        blocks: br.to_vec(),
        connection_point,
    };
    fx.node.handle_completed_fork_recovery(recovery).await
}

async fn gossip(fx: &mut Fx, b: &Block) {
    fx.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

/// Rollback D_{B+1}, then apply `br` on D_B through execute_reorg (no door).
async fn run_direct_reorg(fx: &mut Fx, br: &[Block]) -> anyhow::Result<()> {
    seed_cache(&fx.node, &br[..br.len() - 1]).await;
    let plan = ReorgResult {
        rollback: vec![fx.d[1].hash()],
        common_ancestor: fx.d[0].hash(),
        new_blocks: br.iter().map(|b| b.hash()).collect(),
        weight_delta: 1,
    };
    fx.node
        .execute_reorg(plan, br.last().unwrap().clone())
        .await
}

/// Tip = br's last block at B+2, with `br` canonical from `first_h`, on every output.
async fn assert_landed(fx: &Fx, br: &[Block], first_h: u64, label: &str) {
    let st = fx.state().await;
    let tip = br.last().unwrap().hash();
    assert_eq!(st.best_hash, tip, "{label}: tip ({st:?})");
    assert_eq!(st.best_height, B + 2, "{label}: height");
    for (i, b) in br.iter().enumerate() {
        let h = first_h + i as u64;
        let got = fx.node.block_store.get_hash_by_height(h).expect("store");
        assert_eq!(got, Some(b.hash()), "{label}: canonical block at h={h}");
    }
    assert_eq!(st.persisted, Some(tip), "{label}: persisted tip");
}

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE postboundary={cell} wrong={}", wrong as u8);
}

// P1 (Must) — Decision: a failure means the fork walk still judges the block after the walked branch's own boundary with the node's L_new(D_B), so a minority whose dead branch holds the other boundary block can never walk back.
#[tokio::test]
async fn walk_door_accepts_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &canon, f).await;
    let admitted = weight_recorded(&fx.node, &canon[1].hash()).await;
    probe("walk_door_accepts_post_boundary_block", !admitted);
    assert!(
        admitted,
        "walk: C_B+1 = L_new(C_B)[s1] was refused, judged with the node's L_new(D_B) ({result:?})"
    );
    result.expect("walk: Ok");
}

// P1 (Must) — Decision: a failure means the wedge escape still drops the canonical block after the canonical boundary (lab reproduction), so the minority stays wedged.
#[tokio::test]
async fn wedge_door_accepts_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    gossip(&mut fx, &canon[0]).await;
    let c_b_retained = fx.retained(&canon[0]);
    gossip(&mut fx, &canon[1]).await;
    let kept = fx.retained(&canon[1]) || fx.state().await.at_b1 == Some(canon[1].hash());
    gossip(&mut fx, &canon[2]).await;
    probe("wedge_door_accepts_post_boundary_block", !kept);
    assert!(
        c_b_retained,
        "precondition: the wedge door admits C_B under L_close (M2.1)"
    );
    assert!(
        kept,
        "wedge: C_B+1 = L_new(C_B)[s1] dropped as ineligible under the node's L_new(D_B)"
    );
    assert_landed(&fx, &canon, B, "wedge").await;
}

// P1 (Must) — Decision: a failure means a node on a dead branch that holds the other boundary block does not end on the canonical branch after the walk, on any of tip, index, persisted tip or epoch list.
#[tokio::test]
async fn reorg_lands_on_post_boundary_block() {
    let mut fx = fixture().await;
    let canon = fx.canonical();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &canon, f).await;
    let st = fx.state().await;
    probe(
        "reorg_lands_on_post_boundary_block",
        st.at_b1 != Some(canon[1].hash()),
    );
    result.expect("walk: Ok");
    assert_landed(&fx, &canon, B, "reorg").await;
    assert_eq!(
        active(&fx.node.epoch_state),
        fx.l_canon,
        "reorg: the node re-derived L_new(C_B) by applying C_B"
    );
}

// P1 (Must) — Decision: a failure means execute_reorg still applies a block after the walked boundary whose producer is not scheduled under L_new(C_B), i.e. the door skip opened an unscheduled-branch hole.
#[tokio::test]
async fn reorg_refuses_ineligible_post_boundary_block() {
    let mut fx = fixture().await;
    let br = fx.ineligible();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &br, f).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[1]);
    probe("reorg_refuses_ineligible_post_boundary_block", applied);
    assert!(
        !applied,
        "C'_B+1 = L_new(D_B)[s1], not L_new(C_B)[s1], was applied by the reorg ({result:?}, {st:?})"
    );
    assert!(!fx.applied(&br[2]), "C'_B+2 applied on a refused parent");
    assert_eq!(
        st.best_hash,
        fx.c_b.hash(),
        "the reorg keeps the verified prefix C_B ({result:?})"
    );
    assert_eq!(st.best_height, B, "height of the kept prefix");
    assert_eq!(st.at_b, Some(fx.c_b.hash()), "canonical C_B at B");
    assert_eq!(
        (st.at_b1, st.at_b2),
        (None, None),
        "nothing canonical above C_B"
    );
    assert_eq!(st.persisted, Some(fx.c_b.hash()), "persisted tip = C_B");
}

// P1 (Must) — Decision: a failure means the fix skipped the schedule check for a block whose list IS derivable (parent F, or parent on the node's own boundary), the INC-I-010 fix-4 relaxation, or moved the refusal past a rollback.
#[tokio::test]
async fn door_refuses_derivable_ineligible_block() {
    type Make = fn(&Fx) -> Vec<Block>;
    let parts: [(&str, Make, bool); 2] = [
        ("closing", Fx::closing_ineligible, true),
        ("own_boundary", Fx::on_own_boundary, false),
    ];
    let mut admitted = Vec::new();
    for (label, make, on_f) in parts {
        let mut fx = fixture().await;
        let br = make(&fx);
        let cp = if on_f { fx.f_hash } else { fx.d[0].hash() };
        let before = fx.state().await;
        let result = run_walk(&mut fx, &br, cp).await;
        if fx.state().await != before || weight_recorded(&fx.node, &br[0].hash()).await {
            admitted.push(format!("walk/{label} ({result:?})"));
        }

        let mut fx = fixture().await;
        let br = make(&fx);
        let before = fx.state().await;
        gossip(&mut fx, &br[0]).await;
        let weighed = weight_recorded(&fx.node, &br[0].hash()).await;
        if fx.retained(&br[0]) || weighed || fx.state().await != before {
            admitted.push(format!("wedge/{label}"));
        }
    }
    probe(
        "door_refuses_derivable_ineligible_block",
        !admitted.is_empty(),
    );
    assert!(
        admitted.is_empty(),
        "doors that let a derivable, unscheduled block past the schedule check or moved the tip: {admitted:?}"
    );
}

// P1 (Must) — Decision: a failure means skipping the round-robin arm for a non-derivable block also dropped the membership arm, so any key can author a walked branch past the door.
#[tokio::test]
async fn nonderivable_block_unknown_producer_refused() {
    let mut admitted = Vec::new();
    let mut fx = fixture().await;
    let br = fx.unknown();
    let before = fx.state().await;
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &br, f).await;
    let weighed = weight_recorded(&fx.node, &br[1].hash()).await;
    if weighed || fx.applied(&br[1]) || fx.state().await != before {
        admitted.push(format!("walk ({result:?})"));
    }

    let mut fx = fixture().await;
    let br = fx.unknown();
    let before = fx.state().await;
    gossip(&mut fx, &br[0]).await;
    let c_b_retained = fx.retained(&br[0]);
    gossip(&mut fx, &br[1]).await;
    let weighed = weight_recorded(&fx.node, &br[1].hash()).await;
    if fx.retained(&br[1]) || weighed || fx.state().await != before {
        admitted.push("wedge".to_string());
    }
    probe(
        "nonderivable_block_unknown_producer_refused",
        !admitted.is_empty(),
    );
    assert!(
        c_b_retained,
        "precondition: the wedge door admits C_B, so X_B+1 reaches it"
    );
    assert!(
        admitted.is_empty(),
        "doors that let X_B+1 (unregistered, not in the GSet) past the membership check: {admitted:?}"
    );
}

// P1 (Must) — Decision: a failure means execute_reorg still applies an unscheduled block that reached it without a door (Light apply never checks the schedule).
#[tokio::test]
async fn reorg_refuses_unscheduled_block_without_door() {
    let mut fx = fixture().await;
    let br = fx.on_own_boundary();
    let result = run_direct_reorg(&mut fx, &br).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[0]);
    probe("reorg_refuses_unscheduled_block_without_door", applied);
    assert!(
        !applied,
        "execute_reorg applied Z_B+1, not L_new(D_B)[s1] at its parent D_B ({result:?}, {st:?})"
    );
    assert!(!fx.applied(&br[1]), "Z_B+2 applied on a refused parent");
    assert_eq!(
        st.best_hash,
        fx.d[0].hash(),
        "the reorg keeps its verified prefix (empty): tip = D_B"
    );
    assert_eq!(st.best_height, B, "height of the kept prefix");
    assert_eq!(st.at_b1, None, "nothing canonical above D_B");
    assert_eq!(st.persisted, Some(fx.d[0].hash()), "persisted tip = D_B");
}

// P1 (Should) — Decision: a failure means the reorg schedule check ignores the INC-I-010 layer-3 Light window, so a node after snap sync or in the floor-fallback window refuses valid blocks its divergent list misjudges.
#[tokio::test]
async fn reorg_light_gating_skips_schedule_check() {
    let mut refused = Vec::new();
    for gate in ["floor_fallback_window", "snap_sync_height"] {
        let mut fx = fixture().await;
        if gate == "floor_fallback_window" {
            fx.node.floor_fallback_window = true;
        } else {
            fx.node.snap_sync_height = Some(F);
        }
        let br = fx.on_own_boundary();
        let result = run_direct_reorg(&mut fx, &br).await;
        let st = fx.state().await;
        let tip = br[1].hash();
        let landed = st.best_hash == tip
            && st.best_height == B + 2
            && (st.at_b1, st.at_b2) == (Some(br[0].hash()), Some(tip))
            && st.persisted == Some(tip);
        if !landed {
            refused.push(format!("{gate} ({result:?}, {st:?})"));
        }
    }
    probe(
        "reorg_light_gating_skips_schedule_check",
        !refused.is_empty(),
    );
    assert!(
        refused.is_empty(),
        "the reorg schedule check ran inside the INC-I-010 layer-3 Light window: {refused:?}"
    );
}
