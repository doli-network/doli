//! INC-I-289 M-2 (AUDIT-P2-639-006): above the INC-I-288 gate a branch whose
//! common ancestor with our chain is at or above a finalized block that our
//! canonical index holds descends from it, at any depth below the ancestor.
//! Devnet rig (k2 layout shifted by SHIFT): base h=1..=P, each by its slot
//! leader; ours
//! P+1..=P+3 with one leader; the sibling forks at P with 5 blocks, 3 leaders.
//!
// OUTPUT CONTRACT: Node::handle_new_block after Node::new (reopen)
//   O1 tip (best hash, height) O2 finalized height O3 plan-door finality rejects
// PATHS: P1 checkpoint FH=5, ancestor P = 1111 (> 1000 above FH), in process and
//   after a restart (plan door verdict) | P2
//   checkpoint at P+1 (ancestor below it) restored | P3 = P1 below the INC-I-288 AH,
//   no restart (pre-existing form; characterized at the M-1 commit, unchanged)
// INPUT PARTITIONS: sibling 5 blocks x 3 leaders vs ours 3 blocks x 1 leader
// MATRIX: P1 x O1 O2 -> m2a_control (in process) · P1 x O2 O3 -> m2a_restart · P2 x O1 O2 -> m2b · P3 x O1 -> m2c
//! covers: crates/network/src/sync/reorg/fork_choice.rs bins/node/src/node/fork_recovery.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use inc_i_289_common::{block, in_below_ah_child, leader};
use network::PeerId;
use tempfile::TempDir;

/// k2 slots shifted by a multiple of 3, so the leader pattern is kept.
const SHIFT: u64 = 1107;
const P: u64 = 4 + SHIFT;
const FH_LOW: u64 = 5;
const OUR_SLOTS: [u64; 3] = [5 + SHIFT, 8 + SHIFT, 11 + SHIFT];
const SIB_SLOTS: [u64; 5] = [8 + SHIFT, 9 + SHIFT, 10 + SHIFT, 11 + SHIFT, 12 + SHIFT];

struct Rig {
    node: Node,
    keys: Vec<KeyPair>,
    /// `chain[h - 1]` is our block at height h, h = 1..=P+3.
    chain: Vec<Block>,
    _tmp: TempDir,
}

fn branch(
    keys: &[KeyPair],
    p: &ConsensusParams,
    prev: Hash,
    height: u64,
    slots: &[u64],
) -> Vec<Block> {
    let mut prev = prev;
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let b = block(
                height + i as u64,
                *s as u32,
                prev,
                leader(keys, *s as u32),
                p,
            );
            prev = b.hash();
            b
        })
        .collect()
}

async fn apply(node: &mut Node, blocks: &[Block]) {
    for b in blocks {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block at slot {} must apply: {e}", b.header.slot));
    }
}

async fn tip(node: &Node) -> (Hash, u64) {
    let cs = node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

async fn finalized(node: &Node) -> Option<u64> {
    node.sync_manager.read().await.last_finalized_height()
}

/// Our chain to P+3 with `fh` finalized when the tip is fh+2.
async fn rig(fh: u64) -> Rig {
    h::init_env();
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut p = ConsensusParams::devnet();
    p.genesis_time = 0;
    let mut prev = Hash::ZERO;
    let mut chain: Vec<Block> = (1..=P)
        .map(|hgt| {
            let b = block(hgt, hgt as u32, prev, leader(&keys, hgt as u32), &p);
            prev = b.hash();
            b
        })
        .collect();
    chain.extend(branch(
        &keys,
        &p,
        chain[P as usize - 1].hash(),
        P + 1,
        &OUR_SLOTS,
    ));
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    h::install_production_utxo_backend(&node).await;
    apply(&mut node, &chain[..fh as usize + 2]).await;
    {
        let mut sm = node.sync_manager.write().await;
        sm.add_attestation_weight(
            &chain[fh as usize - 1].hash(),
            *keys[1].public_key(),
            1_000_000,
        );
        assert_eq!(
            sm.last_finalized_height(),
            Some(fh),
            "fixture: fh finalized"
        );
    }
    apply(&mut node, &chain[fh as usize + 2..]).await;
    assert_eq!(tip(&node).await.1, P + 3, "fixture: tip = P+3");
    Rig {
        node,
        keys,
        chain,
        _tmp: tmp,
    }
}

async fn reopen(r: Rig) -> Rig {
    let Rig {
        node,
        keys,
        chain,
        _tmp,
    } = r;
    let config = node.config.clone();
    drop(node);
    let node = Node::new(config, Some(keys[0].clone()), None, None, None, None)
        .await
        .expect("Node::new must reopen the data dir");
    Rig {
        node,
        keys,
        chain,
        _tmp,
    }
}

fn sibling(r: &Rig) -> Vec<Block> {
    branch(
        &r.keys,
        &r.node.params,
        r.chain[P as usize - 1].hash(),
        P + 1,
        &SIB_SLOTS,
    )
}

async fn gossip_all(node: &mut Node, blocks: &[Block]) {
    for b in blocks {
        let _ = node.handle_new_block(b.clone(), PeerId::random()).await;
    }
}

// M2A-CTL INC-I-289 — Decision: a failure means the in-process rig no longer reorgs to the heavier descending branch, so m2a_restart's admitted verdict says nothing about the node following it.
#[tokio::test]
async fn m2a_control_inc_i_289_in_process_branch_wins() {
    let mut r = rig(FH_LOW).await;
    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;
    // The node reorgs at the first heavier sibling block; later fixture blocks
    // carry a dummy VDF that full validation refuses, so the top is not asserted.
    let (tip_hash, tip_h) = tip(&r.node).await;
    assert!(
        tip_h > P && sib.iter().any(|b| b.hash() == tip_hash),
        "M-2: a heavier branch whose ancestor h={P} is above finalized h={FH_LOW} on our chain must win (tip h={tip_h})"
    );
    assert_eq!(finalized(&r.node).await, Some(FH_LOW), "M-2: finality kept");
}

// M2A-RESTART INC-I-289 — Decision: a failure means the checkpoint restored by a restart still locks the plan door against a branch that descends from it, so a restart no longer exits the lock (AUDIT-P2-639-006).
#[tokio::test]
async fn m2a_restart_inc_i_289_restored_deep_checkpoint_plan_door_admits() {
    let r = rig(FH_LOW).await;
    // Execution after a reopen is not asserted: the test chain's height-1 index
    // entry does not survive Node::new, so FORK_GUARD_BACKFILL refuses the apply.
    let r = reopen(r).await;
    assert_eq!(
        finalized(&r.node).await,
        Some(FH_LOW),
        "fixture: checkpoint restored"
    );
    let sib = sibling(&r);
    let mut r = r;
    gossip_all(&mut r.node, &sib).await;
    let obs = r
        .node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .observations();
    assert!(
        obs.plan_reorg_finality_entries > 0,
        "fixture: the plan door must be reached, or the verdict below is vacuous"
    );
    assert_eq!(
        obs.plan_reorg_finality_rejects, 0,
        "M-2: after a restart the plan door must admit a branch whose ancestor h={P} is above the restored finalized h={FH_LOW}"
    );
}

// M2B INC-I-289 — Decision: a failure means the canonical-index shortcut admits a branch that forks below the finalized block, so finality can be reverted.
#[tokio::test]
async fn m2b_inc_i_289_branch_forking_below_checkpoint_is_still_refused() {
    let r = rig(P + 1).await;
    let mut r = reopen(r).await;
    assert_eq!(
        finalized(&r.node).await,
        Some(P + 1),
        "fixture: checkpoint restored"
    );
    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;
    assert_eq!(
        tip(&r.node).await,
        (r.chain[P as usize + 2].hash(), P + 3),
        "M-2: a branch forking at h={P} below finalized h={} must not become the tip",
        P + 1
    );
}

// M2C INC-I-289 — Decision: a failure means the M-2 shortcut leaked below the INC-I-288 activation height, so the pre-gate plan door changed without a gate.
#[tokio::test]
async fn m2c_inc_i_289_below_gate_plan_door_unchanged() {
    if !in_below_ah_child("m2c_inc_i_289_below_gate_plan_door_unchanged") {
        return;
    }
    let mut r = rig(FH_LOW).await;
    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;
    // Characterization: the outcome measured at the M-1 commit (before M-2).
    assert_eq!(
        tip(&r.node).await,
        (sib[3].hash(), P + 4),
        "below the AH the in-process outcome must equal the pre-M-2 binary's"
    );
}
