//! INC-I-289 round 2, Must K-2: a block finalized by the finality rule
//! (>= 67 % bond at CONFIRMATION_DEPTH 2) is never reverted — not by a gossip
//! reorg, not across a restart, not by a snap install. Devnet rig: every
//! producer weighs 1, so a branch weight is its block count.
//!
//! Layout: base h=1..=P by keys[0]; ours h=P+1..=T on P; F = P+1 is finalized
//! with T = F+2. The sibling forks at P (below F) with 5 blocks vs our 3.

// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) / Node::new(config, ..) (reopen) / Node::apply_snap_snapshot(snapshot)
//   O1: tip (chain_state.best_hash + best_height) O2: sync_manager.last_finalized_height() O3: block_store canonical hash at F (O4 apply_snap_snapshot return value: unconstrained on refusal — Err or Ok-without-install are both a refusal; asserted Ok on the control)
//   PATHS: P1 gossip of a heavier sibling forking below F (reorg doors) | P2 drop + Node::new on the same data dir (init restore) | P3 snap install anchored on a conflicting chain | P4 snap install anchored on our own chain above the tip | P5 = P2 below the INC-I-288 AH | P6 = P1 with nothing finalized (anti-vacuity)
//   INPUT PARTITIONS: IP-FIN F finalized through add_attestation_weight with tip >= F+2 | IP-SIB sibling at P, 5 blocks vs our 3 | IP-SNAP-X anchor h=P+5 > F on the sibling chain | IP-SNAP-OWN anchor h=T+2 on our chain | IP-BELOW AH = 1e9 | IP-NOFIN no attestation
//   MATRIX: P1 x O1 O2 -> k2a | P6 x O1 O2 -> k2a_control | P2 x O2 O1 -> k2b | P3 x O1 O2 O3 -> k2c | P4 x O1 O2 O3 O4 -> k2c_control | P5 x O2 -> k2_below
//   covers: fork_recovery snapshot_install init production_gate doors finality

use crate::inc_i_156_m1_harness as h;
use crate::inc_i_289_common::{block, in_below_ah_child, leader};
use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use network::{PeerId, VerifiedSnapshot};
use tempfile::TempDir;

/// Fork point: the last shared base block.
const P: u64 = 4;
/// The finalized height (first block of our branch).
const F: u64 = P + 1;
/// Our tip: F + CONFIRMATION_DEPTH.
const T: u64 = F + 2;
const OUR_SLOTS: [u32; 3] = [5, 6, 7];
const SIB_SLOTS: [u32; 5] = [8, 9, 10, 11, 12];
const EXT_SLOTS: [u32; 2] = [8, 9];

struct Rig {
    node: Node,
    keys: Vec<KeyPair>,
    /// `chain[h - 1]` is our canonical block at height h, h = 1..=T.
    chain: Vec<Block>,
    tmp: TempDir,
}

fn base(keys: &[KeyPair], p: &ConsensusParams) -> Vec<Block> {
    let mut prev = Hash::ZERO;
    (1..=P)
        .map(|hgt| {
            let b = block(hgt, hgt as u32, prev, &keys[0], p);
            prev = b.hash();
            b
        })
        .collect()
}

fn branch(
    keys: &[KeyPair],
    p: &ConsensusParams,
    prev: Hash,
    height: u64,
    slots: &[u32],
) -> Vec<Block> {
    let mut prev = prev;
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let b = block(height + i as u64, *s, prev, leader(keys, *s), p);
            prev = b.hash();
            b
        })
        .collect()
}

/// A devnet test node on the production state_db UTXO backend with `blocks` applied.
async fn node_with(keys: &[KeyPair], tmp: &TempDir, blocks: &[Block]) -> Node {
    h::init_env();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    h::install_production_utxo_backend(&node).await;
    for b in blocks {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block at slot {} must apply: {e}", b.header.slot));
    }
    node
}

/// Our chain base + ours (h=1..=T), tip asserted.
async fn rig() -> Rig {
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut p = ConsensusParams::devnet();
    p.genesis_time = 0;
    let mut chain = base(&keys, &p);
    let ours = branch(&keys, &p, chain[P as usize - 1].hash(), F, &OUR_SLOTS);
    chain.extend(ours);
    let node = node_with(&keys, &tmp, &chain).await;
    let r = Rig {
        node,
        keys,
        chain,
        tmp,
    };
    assert_eq!(
        tip(&r.node).await,
        (r.chain[T as usize - 1].hash(), T),
        "fixture: tip = T"
    );
    r
}

fn hash_at(r: &Rig, height: u64) -> Hash {
    r.chain[height as usize - 1].hash()
}

/// The sibling branch on P: 5 blocks (weight 5) vs our 3 above P.
fn sibling(r: &Rig) -> Vec<Block> {
    branch(&r.keys, &r.node.params, hash_at(r, P), F, &SIB_SLOTS)
}

async fn tip(node: &Node) -> (Hash, u64) {
    let cs = node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

async fn finalized(node: &Node) -> Option<u64> {
    node.sync_manager.read().await.last_finalized_height()
}

/// The real finalize path (m42 `finalize_at`): 67 % weight on F, tip = F+2.
async fn finalize_f(r: &Rig) {
    assert_eq!(
        finalized(&r.node).await,
        None,
        "fixture: nothing finalized before the attestation"
    );
    (r.node.sync_manager.write().await).add_attestation_weight(
        &hash_at(r, F),
        *r.keys[1].public_key(),
        1_000_000,
    );
    assert_eq!(
        finalized(&r.node).await,
        Some(F),
        "fixture: F must really be finalized, or every finality assertion is vacuous"
    );
}

async fn gossip_all(node: &mut Node, blocks: &[Block]) {
    for b in blocks {
        let _ = node.handle_new_block(b.clone(), PeerId::random()).await;
    }
}

/// Drop `r.node` and reopen the same data dir through the restoring constructor.
async fn reopen(r: Rig) -> Rig {
    let Rig {
        node,
        keys,
        chain,
        tmp,
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
        tmp,
    }
}

/// A snapshot of `node`'s own 3-state at its tip, as m1_snap_install_identity builds it.
async fn snapshot_of(node: &Node) -> VerifiedSnapshot {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let base = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("StateSnapshot::create");
    let root = storage::compute_state_root_from_bytes(
        &base.chain_state_bytes,
        &base.utxo_set_bytes,
        &base.producer_set_bytes,
    )
    .expect("state root over the snapshot bytes");
    VerifiedSnapshot {
        block_hash: base.block_hash,
        block_height: base.block_height,
        chain_state: base.chain_state_bytes,
        utxo_set: base.utxo_set_bytes,
        utxo_staged: None,
        producer_set: base.producer_set_bytes,
        state_root: root,
        block_header_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: Some(node.epoch_state.serialize()),
    }
}

// K2A INC-I-289 — Decision: a failure means a heavier gossip branch forking below a finalized block can still rewind it, so the doors' finality guard no longer holds above the gate.
#[tokio::test]
async fn k2a_inc_i_289_reorg_past_finalized_block_is_refused() {
    let mut r = rig().await;
    finalize_f(&r).await;
    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;

    assert_eq!(
        tip(&r.node).await,
        (hash_at(&r, T), T),
        "K-2 doors: a branch forking below finalized F={F} must not become the tip"
    );
    assert_eq!(
        finalized(&r.node).await,
        Some(F),
        "K-2 doors: finality kept"
    );
}

// K2A-CTL INC-I-289 — Decision: a failure means the sibling is not heavier by today's rule, so k2a passes without ever reaching the finality guard.
#[tokio::test]
async fn k2a_control_inc_i_289_same_sibling_reorgs_without_finality() {
    let mut r = rig().await;
    assert_eq!(
        finalized(&r.node).await,
        None,
        "fixture: no finality in the control"
    );
    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;

    let top = sib.last().unwrap();
    assert_eq!(
        tip(&r.node).await,
        (top.hash(), P + sib.len() as u64),
        "control: with nothing finalized the heavier sibling must win"
    );
}

// K2B INC-I-289 — Decision: a failure means a restart forgets the finalized height, so a heavier branch can rewind a finalized block after any node restart (the INC-I-289 lock clears on restart).
#[tokio::test]
async fn k2b_inc_i_289_restart_keeps_finalized_height_and_refuses_reorg_across_it() {
    let r = rig().await;
    finalize_f(&r).await;
    let mut r = reopen(r).await;

    assert_eq!(
        tip(&r.node).await,
        (hash_at(&r, T), T),
        "reopen restores the tip"
    );
    assert_eq!(
        finalized(&r.node).await,
        Some(F),
        "K-2 restart: the finalized height must survive Node::new on the same data dir"
    );

    let sib = sibling(&r);
    gossip_all(&mut r.node, &sib).await;
    assert_eq!(
        tip(&r.node).await,
        (hash_at(&r, T), T),
        "K-2 restart: a branch forking below finalized F={F} must not become the tip after a restart"
    );
}

// K2C INC-I-289 — Decision: a failure means a snap install can replace a finalized chain with a conflicting one, so the snap path bypasses the finality rule.
#[tokio::test]
async fn k2c_inc_i_289_snapshot_on_a_conflicting_chain_is_refused() {
    let mut r = rig().await;
    finalize_f(&r).await;

    let other_tmp = TempDir::new().unwrap();
    let mut other_blocks = r.chain[..P as usize].to_vec();
    other_blocks.extend(sibling(&r));
    let other = node_with(&r.keys, &other_tmp, &other_blocks).await;
    let snap = snapshot_of(&other).await;
    drop(other);
    assert!(snap.block_height >= F, "fixture: anchor at or above F");
    assert_ne!(
        snap.block_hash,
        hash_at(&r, T),
        "fixture: anchor is on another chain"
    );

    let _ = r.node.apply_snap_snapshot(snap).await;

    assert_eq!(
        tip(&r.node).await,
        (hash_at(&r, T), T),
        "K-2 snap: a snapshot whose chain conflicts with finalized F={F} must not install"
    );
    assert_eq!(finalized(&r.node).await, Some(F), "K-2 snap: finality kept");
    assert_eq!(
        r.node.block_store.get_hash_by_height(F).unwrap(),
        Some(hash_at(&r, F)),
        "K-2 snap: finalized block still canonical at F"
    );
}

// K2C-CTL INC-I-289 — Decision: a failure means the snap guard refuses snapshots on our own finalized chain, so a lagging node can no longer snap-sync forward.
#[tokio::test]
async fn k2c_control_inc_i_289_snapshot_on_our_finalized_chain_is_installed() {
    let mut r = rig().await;
    finalize_f(&r).await;

    let longer_tmp = TempDir::new().unwrap();
    let ext = branch(&r.keys, &r.node.params, hash_at(&r, T), T + 1, &EXT_SLOTS);
    let mut longer_blocks = r.chain.clone();
    longer_blocks.extend(ext.iter().cloned());
    let longer = node_with(&r.keys, &longer_tmp, &longer_blocks).await;
    let mut snap = snapshot_of(&longer).await;
    drop(longer);
    let anchor = (snap.block_hash, snap.block_height);
    assert_eq!(
        anchor,
        (ext.last().unwrap().hash(), T + 2),
        "fixture: anchor above T on our chain"
    );
    // The snap guard admits only an anchor it can link to F: our node holds
    // T+1, and the snapshot carries the signed anchor header (parent = T+1).
    gossip_all(&mut r.node, &ext[..1]).await;
    assert_eq!(
        tip(&r.node).await,
        (ext[0].hash(), T + 1),
        "fixture: our node is one block behind the anchor"
    );
    snap.block_header_bytes = Some(bincode::serialize(&ext[1].header).unwrap());

    r.node
        .apply_snap_snapshot(snap)
        .await
        .expect("a snapshot on our own finalized chain must install");

    assert_eq!(
        tip(&r.node).await,
        anchor,
        "control: tip moves to the anchor"
    );
    assert_eq!(finalized(&r.node).await, Some(F), "control: finality kept");
    assert_eq!(
        r.node.block_store.get_hash_by_height(F).unwrap(),
        Some(hash_at(&r, F)),
        "control: finalized block still canonical at F"
    );
}

// K2-BELOW INC-I-289 — Decision: a failure means finality persistence leaked below the INC-I-288 activation height, so the pre-gate restart behaviour changed without a gate.
#[tokio::test]
async fn k2_below_ah_inc_i_289_restart_forgets_finality_old_rule() {
    if !in_below_ah_child(
        "inc_i_289_k2_finality::k2_below_ah_inc_i_289_restart_forgets_finality_old_rule",
    ) {
        return;
    }
    let r = rig().await;
    finalize_f(&r).await;
    let r = reopen(r).await;

    assert_eq!(
        tip(&r.node).await,
        (hash_at(&r, T), T),
        "reopen restores the tip"
    );
    assert_eq!(
        finalized(&r.node).await,
        None,
        "below the AH the old rule holds: a restart starts with no finalized height"
    );
}
