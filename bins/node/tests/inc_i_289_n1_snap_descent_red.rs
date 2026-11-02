//! INC-I-289 N-1 (AUDIT-P2-639-001): above the INC-I-288 gate a node with a
//! finalized block at FH and a tip short of the snap anchor installs a forward
//! snapshot on its own chain once the snap peer serves the header chain that
//! links FH to the anchor. Driven only through public sync calls that predate
//! the descent proof, so the same file compiles before and after it.
//! Devnet rig: one producer key signs every block, slot = height.
//!
// OUTPUT CONTRACT: SyncManager snap round (request_genesis_resync, add_peer,
//   snap_fallback_to_normal, next_snap_requests, next_request, handle_response)
//   then Node::apply_snap_snapshot
//   O1 tip (chain_state best hash, height) O2 last_finalized_height
//   (apply_snap_snapshot return value: unconstrained, a refusal is Ok-without-install)
// PATHS: P1 anchor on our chain, the snap peer serves its chain f+1..=anchor |
//   P2 anchor on a foreign chain (forks below FH), the snap peer serves its own chain
// INPUT PARTITIONS: FH = 998, local tip 1000, anchor 1600 (no local headers 1001..=1600)
// MATRIX: P1 x O1 O2 -> n1a · P2 x O1 O2 -> n1a_control
//! covers: bins/node/src/node/snapshot_install.rs crates/network/src/sync/manager/snap_descent.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use inc_i_289_common::block;
use network::protocols::{SyncRequest, SyncResponse};
use network::{PeerId, RecoveryReason, SyncPhase, SyncState, VerifiedSnapshot};
use tempfile::TempDir;

const FH: u64 = 998;
const TIP: u64 = 1000;
const ANCHOR: u64 = 1600;
/// The foreign chain shares heights 1..=FORK with ours, then diverges.
const FORK: u64 = 990;

/// `base` extended to `len` blocks (`chain[h - 1]` is height h). Slot = height
/// on a fresh chain; slot = height + 1 above a shared base, so the tail differs.
fn chain_of(key: &KeyPair, p: &ConsensusParams, base: &[Block], len: u64) -> Vec<Block> {
    let mut out = base.to_vec();
    let mut prev = out.last().map_or(Hash::ZERO, |b| b.hash());
    let shift = u32::from(!base.is_empty());
    for hgt in base.len() as u64 + 1..=len {
        let b = block(hgt, hgt as u32 + shift, prev, key, p);
        prev = b.hash();
        out.push(b);
    }
    out
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

/// The snapshot a peer at the tip of `blocks` serves, carrying its anchor header.
async fn snapshot_of(keys: &[KeyPair], blocks: &[Block]) -> VerifiedSnapshot {
    let tmp = TempDir::new().unwrap();
    let node = node_with(keys, &tmp, blocks).await;
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
    let anchor = blocks.last().unwrap();
    assert_eq!(
        (base.block_hash, base.block_height),
        (anchor.hash(), blocks.len() as u64),
        "fixture: the snapshot is anchored at the peer's tip"
    );
    VerifiedSnapshot {
        block_hash: base.block_hash,
        block_height: base.block_height,
        chain_state: base.chain_state_bytes,
        utxo_set: base.utxo_set_bytes,
        utxo_staged: None,
        producer_set: base.producer_set_bytes,
        state_root: root,
        block_header_bytes: Some(bincode::serialize(&anchor.header).unwrap()),
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: Some(node.epoch_state.serialize()),
    }
}

async fn tip(node: &Node) -> (Hash, u64) {
    let cs = node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

async fn finalized(node: &Node) -> Option<u64> {
    node.sync_manager.read().await.last_finalized_height()
}

/// Our node at TIP on `ours` with FH finalized through the real attestation path.
async fn our_node(keys: &[KeyPair], tmp: &TempDir, ours: &[Block]) -> Node {
    let node = node_with(keys, tmp, &ours[..TIP as usize]).await;
    let fh_hash = ours[FH as usize - 1].hash();
    let mut sm = node.sync_manager.write().await;
    sm.add_attestation_weight(&fh_hash, *keys[1].public_key(), 1_000_000);
    assert_eq!(
        sm.last_finalized_height(),
        Some(FH),
        "fixture: FH must really be finalized, or the guard is never reached"
    );
    drop(sm);
    node
}

/// GetHeaders as the snap peer answers it from its own chain: the headers after
/// `start`; a start hash it does not hold is answered from height FH + 1.
fn serve(chain: &[Block], start: Hash, max: u32) -> Vec<BlockHeader> {
    let from = chain
        .iter()
        .position(|b| b.hash() == start)
        .map_or(FH as usize, |i| i + 1);
    chain[from..]
        .iter()
        .take(max as usize)
        .map(|b| b.header.clone())
        .collect()
}

/// One snap round toward the snapshot's anchor: three peers advertise it and vote
/// its root; the download peer answers every GetHeaders from `peer_chain`. Stops
/// at the first other request (the manifest). Returns the batches served.
async fn snap_round(node: &Node, snap: &VerifiedSnapshot, peer_chain: &[Block]) -> usize {
    let mut sm = node.sync_manager.write().await;
    assert!(
        sm.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation),
        "fixture: a forward large-gap snap request is honoured"
    );
    let slot = peer_chain.last().unwrap().header.slot;
    let peers: Vec<PeerId> = (0..3).map(|_| PeerId::random()).collect();
    for p in &peers {
        sm.add_peer(*p, snap.block_height, snap.block_hash, slot);
    }
    // The first add_peer decided with one peer; re-decide with all three.
    sm.snap_fallback_to_normal();
    assert_eq!(
        sm.next_snap_requests().len(),
        3,
        "fixture: roots asked of all three peers"
    );
    for p in &peers {
        let vote = SyncResponse::StateRoot {
            block_hash: snap.block_hash,
            block_height: snap.block_height,
            state_root: snap.state_root,
        };
        sm.handle_response(*p, vote);
    }
    assert!(
        matches!(
            sm.state(),
            SyncState::Syncing {
                phase: SyncPhase::SnapDownloading,
                ..
            }
        ),
        "fixture: the root quorum moves the round to the download, got {:?}",
        sm.state()
    );
    let mut batches = 0;
    for _ in 0..64 {
        let Some((
            peer,
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            },
        )) = sm.next_request()
        else {
            break;
        };
        batches += 1;
        let served = serve(peer_chain, start_hash, max_count);
        sm.handle_response(peer, SyncResponse::Headers(served));
    }
    batches
}

// N1A INC-I-289 — Decision: a failure means a node that holds a finalized block refuses every forward snapshot on its own chain when it holds no headers for the gap, so snap sync stays closed to every node that ever finalized (AUDIT-P2-639-001).
#[tokio::test]
async fn n1a_inc_i_289_forward_snapshot_on_our_chain_installs_after_served_descent() {
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut p = ConsensusParams::devnet();
    p.genesis_time = 0;
    let chain = chain_of(&keys[0], &p, &[], ANCHOR);
    let snap = snapshot_of(&keys, &chain).await;
    let tmp = TempDir::new().unwrap();
    let mut node = our_node(&keys, &tmp, &chain).await;

    let batches = snap_round(&node, &snap, &chain).await;
    let _ = node.apply_snap_snapshot(snap.clone()).await;

    assert_eq!(
        tip(&node).await,
        (snap.block_hash, ANCHOR),
        "N-1: a forward snapshot on our chain must install once its header chain to \
         finalized h={FH} is served ({batches} GetHeaders batches served)"
    );
}

// N1A-CTL INC-I-289 — Decision: a failure means a snapshot on a chain that forks below the finalized block installs, so the descent proof lets a peer replace a finalized block.
#[tokio::test]
async fn n1a_control_inc_i_289_foreign_snapshot_stays_refused() {
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut p = ConsensusParams::devnet();
    p.genesis_time = 0;
    let ours = chain_of(&keys[0], &p, &[], TIP);
    let foreign = chain_of(&keys[0], &p, &ours[..FORK as usize], ANCHOR);
    assert_ne!(
        foreign[FH as usize - 1].hash(),
        ours[FH as usize - 1].hash(),
        "fixture: the foreign chain does not hold our finalized block"
    );
    let snap = snapshot_of(&keys, &foreign).await;
    let tmp = TempDir::new().unwrap();
    let mut node = our_node(&keys, &tmp, &ours).await;

    let batches = snap_round(&node, &snap, &foreign).await;
    let _ = node.apply_snap_snapshot(snap).await;

    assert_eq!(
        tip(&node).await,
        (ours[TIP as usize - 1].hash(), TIP),
        "N-1 control: a snapshot on a chain that forks below finalized h={FH} must not \
         install ({batches} GetHeaders batches served)"
    );
    assert_eq!(
        finalized(&node).await,
        Some(FH),
        "N-1 control: finality kept"
    );
}
