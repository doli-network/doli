//! INC-I-289 M-1 (AUDIT-P2-639-001): above the INC-I-288 gate a node with a
//! finalized checkpoint installs a forward snapshot when a served header chain
//! links its finalized block to the anchor; a broken, foreign or over-long chain
//! is refused. Devnet rig: one producer key signs every block, slot = height.
//!
// OUTPUT CONTRACT: SyncManager::accept_snap_descent_chain + Node::apply_snap_snapshot
//   O1 tip (chain_state best hash, height) O2 accept verdict O3 refusal log line
// PATHS: P1 served chain f+1..=anchor links | P2 a broken parent link | P3 a linked
//   chain ending at another hash | P4 span > SNAP_DESCENT_PROOF_MAX_HEADERS | P5 = P1
//   below the INC-I-288 AH (no proof machinery, old rule installs)
// INPUT PARTITIONS: fh = 998, local tip 1000, anchor 1600 on the same chain
// MATRIX: P1 x O1 O2 -> m1a · P2 P3 x O1 O2 -> m1b · P4 x O2 O3 -> m1c · P5 x O1 O2 -> m1d
//! covers: bins/node/src/node/snapshot_install.rs crates/network/src/sync/manager/snap_descent.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use inc_i_289_common::{block, in_below_ah_child};
use network::{VerifiedSnapshot, SNAP_DESCENT_PROOF_MAX_HEADERS};
use tempfile::TempDir;

const FH: u64 = 998;
const TIP: u64 = 1000;
const ANCHOR: u64 = 1600;

struct Rig {
    node: Node,
    /// `chain[h - 1]` is the block at height h, h = 1..=ANCHOR.
    chain: Vec<Block>,
    snap: VerifiedSnapshot,
    _tmp: (TempDir, TempDir),
}

fn chain_of(key: &KeyPair, p: &ConsensusParams, len: u64) -> Vec<Block> {
    let mut prev = Hash::ZERO;
    (1..=len)
        .map(|hgt| {
            let b = block(hgt, hgt as u32, prev, key, p);
            prev = b.hash();
            b
        })
        .collect()
}

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

async fn tip(node: &Node) -> (Hash, u64) {
    let cs = node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

/// Our node at TIP with FH finalized; the snapshot of a node at ANCHOR on the
/// same chain, carrying only its own header.
async fn rig() -> Rig {
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut p = ConsensusParams::devnet();
    p.genesis_time = 0;
    let chain = chain_of(&keys[0], &p, ANCHOR);
    let (ours_tmp, other_tmp) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let node = node_with(&keys, &ours_tmp, &chain[..TIP as usize]).await;
    let other = node_with(&keys, &other_tmp, &chain).await;
    let mut snap = snapshot_of(&other).await;
    drop(other);
    snap.block_header_bytes = Some(bincode::serialize(&chain[ANCHOR as usize - 1].header).unwrap());
    assert_eq!(
        (snap.block_hash, snap.block_height),
        (chain[ANCHOR as usize - 1].hash(), ANCHOR),
        "fixture: anchor at {ANCHOR} on our chain"
    );
    let fh_hash = chain[FH as usize - 1].hash();
    {
        let mut sm = node.sync_manager.write().await;
        sm.add_attestation_weight(&fh_hash, *keys[1].public_key(), 1_000_000);
        assert_eq!(
            sm.last_finalized_height(),
            Some(FH),
            "fixture: FH must really be finalized, or the guard is never reached"
        );
    }
    Rig {
        node,
        chain,
        snap,
        _tmp: (ours_tmp, other_tmp),
    }
}

/// The headers f+1..=anchor that a snapshot peer serves for GetHeaders(f).
fn served(r: &Rig) -> Vec<BlockHeader> {
    r.chain[FH as usize..ANCHOR as usize]
        .iter()
        .map(|b| b.header.clone())
        .collect()
}

async fn accept(r: &Rig, anchor_height: u64, anchor: Hash, headers: &[BlockHeader]) -> bool {
    let mut sm = r.node.sync_manager.write().await;
    sm.accept_snap_descent_chain(anchor_height, anchor, headers)
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

// M1A INC-I-289 — Decision: a failure means a lagging node with a checkpoint still refuses every forward snapshot on its own chain, so snap sync stays closed to every node that ever finalized (AUDIT-P2-639-001).
#[tokio::test]
async fn m1a_inc_i_289_forward_snapshot_with_served_descent_chain_is_installed() {
    let mut r = rig().await;
    let anchor = (r.snap.block_hash, r.snap.block_height);
    assert!(
        accept(&r, ANCHOR, anchor.0, &served(&r)).await,
        "M-1: a hash-linked chain f+1..=anchor must be accepted as the descent proof"
    );
    let snap = r.snap.clone();
    let _ = r.node.apply_snap_snapshot(snap).await;
    assert_eq!(
        tip(&r.node).await,
        anchor,
        "M-1: a forward snapshot proven to descend from finalized h={FH} must install"
    );
}

// M1B INC-I-289 — Decision: a failure means a peer can fake descent with a chain whose link breaks or that ends elsewhere, so the snap guard no longer protects the finalized block.
#[tokio::test]
async fn m1b_inc_i_289_broken_or_foreign_descent_chain_is_refused() {
    let mut r = rig().await;
    let anchor = r.snap.block_hash;
    let mut broken = served(&r);
    broken[5].prev_hash = Hash::ZERO;
    assert!(
        !accept(&r, ANCHOR, anchor, &broken).await,
        "M-1: a chain with a broken parent link must be refused"
    );
    let mut foreign = served(&r);
    let p = r.node.params.clone();
    let last = foreign.len() - 1;
    let other_key = KeyPair::generate();
    let prev = foreign[last].prev_hash;
    foreign[last] = block(ANCHOR, ANCHOR as u32 + 7, prev, &other_key, &p).header;
    assert!(
        !accept(&r, ANCHOR, anchor, &foreign).await,
        "M-1: a linked chain that ends at another hash must be refused"
    );
    let snap = r.snap.clone();
    let _ = r.node.apply_snap_snapshot(snap).await;
    assert_eq!(
        tip(&r.node).await,
        (r.chain[TIP as usize - 1].hash(), TIP),
        "M-1: with no valid proof the snapshot must not install"
    );
}

// M1C INC-I-289 — Decision: a failure means the proof has no bound, so one snapshot can make a node walk an unbounded peer-served chain.
#[tokio::test]
async fn m1c_inc_i_289_descent_chain_over_the_bound_is_refused_with_log_line() {
    let r = rig().await;
    let far = FH + SNAP_DESCENT_PROOF_MAX_HEADERS + 1;
    let cap = Capture::default();
    let writer = cap.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let headers = served(&r);
    let mut sm = r.node.sync_manager.write().await;
    let accepted = tracing::subscriber::with_default(subscriber, || {
        sm.accept_snap_descent_chain(far, r.snap.block_hash, &headers)
    });
    drop(sm);
    assert!(!accepted, "M-1: a span above the bound must be refused");
    let log = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("Descent proof refused") && log.contains("falling back to block sync"),
        "M-1: the bound refusal must log one line; got:\n{log}"
    );
}

// M1D INC-I-289 — Decision: a failure means the descent proof leaked below the INC-I-288 activation height, so the pre-gate snap install changed without a gate.
#[tokio::test]
async fn m1d_inc_i_289_below_gate_snapshot_installs_without_proof() {
    if !in_below_ah_child("m1d_inc_i_289_below_gate_snapshot_installs_without_proof") {
        return;
    }
    let mut r = rig().await;
    let anchor = (r.snap.block_hash, r.snap.block_height);
    assert!(
        !accept(&r, ANCHOR, anchor.0, &served(&r)).await,
        "below the AH no descent proof is taken"
    );
    let snap = r.snap.clone();
    r.node
        .apply_snap_snapshot(snap)
        .await
        .expect("below the AH the old rule installs");
    assert_eq!(
        tip(&r.node).await,
        anchor,
        "below the AH: the old rule installs"
    );
}
