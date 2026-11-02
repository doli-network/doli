//! INC-I-289 N-4 (AUDIT-P1-644-001): a node whose tip epoch has no stored epoch
//! table (first start of this build, or a boundary put lost to a crash) seeds it
//! from the live table, so a later fork anchored in that epoch is still weighed.
//!
// OUTPUT CONTRACT: Node::new (start seed), Node::apply_block (capture point),
//   Node::handle_new_block (door lookup). n4b runs in process: a Node::new reopen of a
//   new_for_test data dir does not take gossip reorgs, so n4a carries the restart.
//   O1 state_db.get_epoch_table(tip epoch) (presence, digest, list, bonds); O2 the tip.
// PATHS: P1 restart with the tip epoch's table missing; P2 the table present;
//   P3 a mid-epoch block applied while the table is missing.
// INPUT PARTITIONS: tip 22 (epoch 20) | tip F=5 (epoch 4), fork on F after the
//   boundary at 8 | same with the table kept (control).
// MATRIX: P1 x O1 -> n4a · P3 x O2 -> n4b · P2 x O2 -> n4b_control · P3 x O1 -> n4c
//! covers: bins/node/src/node/epoch_tables.rs init.rs apply_block/mod.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use inc_i_289_common::{block, leader, F};
use network::PeerId;
use tempfile::TempDir;

/// A node on the production backend with h=1..=top applied (keys[0], slot = h).
async fn chain_node(keys: &[KeyPair], tmp: &TempDir, top: u64) -> Node {
    h::init_env();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    h::install_production_utxo_backend(&node).await;
    let p = node.params.clone();
    let mut prev = Hash::ZERO;
    for height in 1..=top {
        let b = block(height, height as u32, prev, &keys[0], &p);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block {height} must apply: {e}"));
    }
    node
}

fn keys3() -> Vec<KeyPair> {
    (0..3).map(|_| KeyPair::generate()).collect()
}

async fn restart(node: Node, keys: &[KeyPair]) -> Node {
    let config = node.config.clone();
    drop(node);
    Node::new(config, Some(keys[0].clone()), None, None, None, None)
        .await
        .expect("Node::new must reopen the data dir")
}

/// A chain on `prev` from `height`, one block per slot, each by its slot leader.
fn chain_on(node: &Node, keys: &[KeyPair], prev: Hash, height: u64, slots: &[u32]) -> Vec<Block> {
    let mut prev = prev;
    (slots.iter().enumerate())
        .map(|(i, s)| {
            let b = block(height + i as u64, *s, prev, leader(keys, *s), &node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// The weighing view of a table's producer set (activity counters move mid-epoch).
fn keys_of(t: &storage::EpochTable) -> Vec<(crypto::PublicKey, Vec<u8>, String)> {
    let set = t.producer_set().expect("table producer set decodes");
    let mut v: Vec<_> = (set.all_producers().iter())
        .map(|p| {
            (
                p.public_key,
                p.bls_pubkey.clone(),
                format!("{:?}", p.status),
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    v
}

async fn tip(node: &Node) -> (u64, Hash) {
    let cs = node.chain_state.read().await;
    (cs.best_height, cs.best_hash)
}

// N4A — Decision: a failure means a node that starts in an epoch without a table never gets one.
#[tokio::test]
async fn n4a_restart_into_an_epoch_without_a_table_seeds_it() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let node = chain_node(&keys, &tmp, 22).await;
    let want = node
        .state_db
        .get_epoch_table(20)
        .expect("fixture: captured");
    node.state_db.delete_epoch_table(20);
    let node = restart(node, &keys).await;
    let got = node.state_db.get_epoch_table(20);
    assert!(
        got.is_some(),
        "N-4: the tip epoch's table is seeded at start"
    );
    let got = got.unwrap();
    assert_eq!(got.producer_list, want.producer_list, "N-4: decode list");
    assert_eq!(got.bonds, want.bonds, "N-4: bonds");
    assert_eq!(got.boundary_hash, want.boundary_hash, "N-4: boundary block");
    assert_eq!(keys_of(&got), keys_of(&want), "N-4: BLS keys and status");
}

// N4C — Decision: a failure means a lost boundary put is never back-filled while the node runs.
#[tokio::test]
async fn n4c_a_mid_epoch_block_back_fills_a_missing_table() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let mut node = chain_node(&keys, &tmp, 21).await;
    let want = node
        .state_db
        .get_epoch_table(20)
        .expect("fixture: captured");
    node.state_db.delete_epoch_table(20);
    let prev = tip(&node).await.1;
    let b = block(22, 22, prev, &keys[0], &node.params);
    node.apply_block(b, ValidationMode::Light)
        .await
        .expect("fixture: block 22");
    let got = node.state_db.get_epoch_table(20);
    assert!(got.is_some(), "N-4: the capture point back-fills the table");
    let got = got.unwrap();
    assert_eq!(got.bonds, want.bonds, "N-4: bonds");
    assert_eq!(keys_of(&got), keys_of(&want), "N-4: BLS keys and status");
}

/// Base h=1..=F (tip epoch's table dropped when `hole`, as a lost boundary put or a
/// start without the store leaves it), our three blocks by one producer past the boundary at 8, the heavier sibling on F.
async fn hole_rig(hole: bool) -> (Node, Vec<Block>, TempDir) {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let mut node = chain_node(&keys, &tmp, F).await;
    let bpe = node.config.network.blocks_per_reward_epoch();
    assert!(F / bpe < (F + 3) / bpe, "fixture: tip epoch is after F's");
    if hole {
        node.state_db.delete_epoch_table(F - F % bpe);
    }
    let f_hash = tip(&node).await.1;
    for b in chain_on(&node, &keys, f_hash, F + 1, &[6, 9, 12]) {
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: own block");
    }
    assert_eq!(tip(&node).await.0, F + 3, "fixture: own tip");
    let sib = chain_on(&node, &keys, f_hash, F + 1, &[7, 8]);
    (node, sib, tmp)
}

// N4B — Decision: a failure means a fork anchored in the hole epoch is refused for good.
#[tokio::test]
async fn n4b_fork_anchored_in_the_hole_epoch_is_weighed_after_the_boundary() {
    let (mut node, sib, _tmp) = hole_rig(true).await;
    for b in &sib {
        let _ = node.handle_new_block(b.clone(), PeerId::random()).await;
    }
    assert_eq!(
        tip(&node).await.1,
        sib[1].hash(),
        "N-4: the heavier sibling anchored in the hole epoch wins"
    );
}

// N4B-CTL — Decision: a failure means n4b is vacuous (the rig never reorgs).
#[tokio::test]
async fn n4b_control_with_the_table_kept_the_sibling_wins() {
    let (mut node, sib, _tmp) = hole_rig(false).await;
    for b in &sib {
        let _ = node.handle_new_block(b.clone(), PeerId::random()).await;
    }
    assert_eq!(
        tip(&node).await.1,
        sib[1].hash(),
        "control: the sibling wins"
    );
}
