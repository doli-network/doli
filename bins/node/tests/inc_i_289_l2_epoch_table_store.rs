//! INC-I-289 L-2 (AUDIT-P2-639-004, operator ruling 3/4): every applied epoch
//! boundary stores its epoch table (decode list, bonds, BLS-bearing set) in a bounded,
//! persisted, local store; a node without the ancestor's table refuses the reorg.
//!
// OUTPUT CONTRACT: Node::apply_block (capture + prune), Node::new (reopen),
//   Node::apply_snap_snapshot (seed), Node::handle_new_block (gossip door lookup).
//   O1 state_db.epoch_table_starts() (count, ends); O2 each table's digest / list /
//   bonds; O3 the tip; O4 doli_fork_choice_table_missing_total; O5 refusal log lines.
// PATHS: P1 capture+prune at every boundary; P2 restart; P3 snap install; P4 door
//   with the ancestor table missing; P5 door with it present (control).
// INPUT PARTITIONS: tip 1200 far past the window (bpe 4) | tip 22 (tables 0..20) |
//   receiver at 10, donor at 22 | two sibling blocks on one ancestor F, table dropped
//   | same, table kept.
// MATRIX: P1 x O1 -> l2a · P1 x O1 O2 -> l2b · P2 x O1 O2 -> l2c · P3 x O1 O2 -> l2d
//   · P4 x O3 O4 O5 -> l2e · P5 x O3 -> l2e_control
//! covers: bins/node/src/node/epoch_tables.rs branch_support.rs apply_block/mod.rs
//!   fork_recovery.rs crates/storage/src/state_db/epoch_tables.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use std::sync::{Arc, Mutex};

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::metrics::FORK_CHOICE_TABLE_MISSING;
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use inc_i_289_common::{block, rig, tip_hash, F};
use network::{PeerId, VerifiedSnapshot};
use tempfile::TempDir;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const REFUSAL_LOG: &str = "FORK_TABLE";

/// A node on the production backend with h=1..=top applied (keys[0], slot = h).
async fn chain_node(keys: &[KeyPair], tmp: &TempDir, top: u64) -> (Node, Vec<Block>) {
    h::init_env();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    h::install_production_utxo_backend(&node).await;
    let p = node.params.clone();
    let (mut prev, mut chain) = (Hash::ZERO, Vec::new());
    for height in 1..=top {
        let b = block(height, height as u32, prev, &keys[0], &p);
        prev = b.hash();
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block {height} must apply: {e}"));
        chain.push(b);
    }
    (node, chain)
}

fn keys3() -> Vec<KeyPair> {
    (0..3).map(|_| KeyPair::generate()).collect()
}

/// The ruled count: ceil(MAX_REORG_DEPTH / bpe) + 1 boundaries, at least 4.
fn ruled_count(bpe: u64) -> usize {
    let depth = network::MAX_REORG_DEPTH as u64;
    (depth.div_ceil(bpe) + 1).max(4) as usize
}

fn tables(node: &Node) -> Vec<(u64, Hash)> {
    let db = &node.state_db;
    (db.epoch_table_starts().into_iter())
        .map(|s| (s, db.get_epoch_table(s).expect("listed table").digest()))
        .collect()
}

// L2A — Decision: a failure means the store grows without bound (or captures nothing).
#[tokio::test]
async fn l2a_store_holds_the_ruled_number_of_tables() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (node, _) = chain_node(&keys, &tmp, 1200).await;
    let bpe = node.config.network.blocks_per_reward_epoch();
    let want = ruled_count(bpe);
    assert_eq!(want, 251, "fixture: devnet bpe 4 -> 251 tables");
    assert_eq!(
        node.state_db.epoch_table_starts().len(),
        want,
        "L-2 bound: the store must hold exactly ceil(1000/bpe)+1 tables"
    );
}

// L2B — Decision: a failure means pruning drops the wrong end of the window.
#[tokio::test]
async fn l2b_pruning_drops_the_oldest_and_keeps_the_newest() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (node, chain) = chain_node(&keys, &tmp, 1200).await;
    let starts = node.state_db.epoch_table_starts();
    assert_eq!(starts.first(), Some(&200), "L-2 prune: oldest kept = 200");
    let db = &node.state_db;
    assert!(
        db.get_epoch_table(196).is_none() && db.get_epoch_table(0).is_none(),
        "L-2 prune: tables outside the window are gone"
    );
    let newest = db.get_epoch_table(1200).expect("L-2: newest table kept");
    assert_eq!(
        newest.boundary_hash,
        Some(chain[1199].hash()),
        "newest = block 1200"
    );
    assert_eq!(
        newest.producer_list, node.epoch_state.producer_list,
        "decode list"
    );
}

// L2C — Decision: a failure means a restart loses the tables (store not persisted).
#[tokio::test]
async fn l2c_restart_keeps_the_tables() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (node, _) = chain_node(&keys, &tmp, 22).await;
    let before = tables(&node);
    assert_eq!(before.len(), 6, "L-2: tables 0,4,..,20 before the restart");
    let config = node.config.clone();
    drop(node);
    let node = Node::new(config, Some(keys[0].clone()), None, None, None, None)
        .await
        .expect("Node::new must reopen the data dir");
    assert_eq!(
        tables(&node),
        before,
        "L-2: the same tables after a restart"
    );
}

/// A snapshot of `node`'s own 3-state at its tip (as k2_finality builds it).
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
    .expect("state root");
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

// L2D — Decision: a failure means a snap-synced node keeps stale tables or has none.
#[tokio::test]
async fn l2d_snap_install_seeds_only_the_anchor_epoch() {
    let keys = keys3();
    let (t1, t2) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (donor, _) = chain_node(&keys, &t1, 22).await;
    let (mut node, _) = chain_node(&keys, &t2, 10).await;
    node.apply_snap_snapshot(snapshot_of(&donor).await)
        .await
        .expect("snap install");
    let installed = node.chain_state.read().await.best_height;
    assert_eq!(installed, 22, "fixture: installed");
    let starts = node.state_db.epoch_table_starts();
    assert_eq!(starts, vec![20], "L-2 snap: anchor epoch only");
    let got = node.state_db.get_epoch_table(20).unwrap();
    let want = donor.state_db.get_epoch_table(20).unwrap();
    assert_eq!(
        got.producer_list, want.producer_list,
        "L-2 snap: decode list"
    );
    assert_eq!(got.bonds, want.bonds, "L-2 snap: bonds");
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);
struct Fields(String);
impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}
impl<T: tracing::Subscriber> Layer<T> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, T>) {
        let mut f = Fields(String::new());
        e.record(&mut f);
        self.0.lock().unwrap().push(f.0);
    }
}

/// Ours: one producer at F+1..=F+3 (tip in the epoch after F's); the sibling on F
/// by the other two producers (2 > 1 in bond above the gate).
async fn fork_rig(drop_ancestor_table: bool) -> (inc_i_289_common::Rig, Vec<Block>) {
    let r = rig(&[6, 9, 12]).await;
    let bpe = r.node.config.network.blocks_per_reward_epoch();
    assert!(
        F / bpe < (F + 3) / bpe,
        "fixture: tip epoch is after the ancestor's"
    );
    if drop_ancestor_table {
        r.node.state_db.delete_epoch_table(F - F % bpe);
    }
    let sib = inc_i_289_common::chain_on(&r, r.f_hash, F + 1, &[7, 8]);
    (r, sib)
}

// L2E — Decision: a failure means a node without the ancestor's table weighs with its tip table.
#[tokio::test]
async fn l2e_missing_ancestor_table_refuses_and_logs_once() {
    let (mut r, sib) = fork_rig(true).await;
    let before_tip = tip_hash(&r).await;
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let m0 = FORK_CHOICE_TABLE_MISSING.get();
    let _ = r
        .node
        .handle_new_block(sib[0].clone(), PeerId::random())
        .await;
    let m1 = FORK_CHOICE_TABLE_MISSING.get();
    assert_eq!(m1 - m0, 1, "L-2 refusal: metric +1");
    let _ = r
        .node
        .handle_new_block(sib[1].clone(), PeerId::random())
        .await;
    assert_eq!(
        tip_hash(&r).await,
        before_tip,
        "L-2 refusal: no reorg without the table"
    );
    let lines = logs.0.lock().unwrap();
    let n = lines.iter().filter(|l| l.contains(REFUSAL_LOG)).count();
    assert_eq!(
        n, 1,
        "L-2 refusal: one log line for two attempts on one ancestor"
    );
}

// L2E-CTL — Decision: a failure means the refusal test is vacuous (the sibling never wins).
#[tokio::test]
async fn l2e_control_with_the_table_the_heavier_sibling_wins() {
    let (mut r, sib) = fork_rig(false).await;
    for b in &sib {
        let _ = r.node.handle_new_block(b.clone(), PeerId::random()).await;
    }
    assert_eq!(
        tip_hash(&r).await,
        sib[1].hash(),
        "control: the sibling wins"
    );
}
