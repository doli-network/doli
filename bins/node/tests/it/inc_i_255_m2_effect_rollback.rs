//! INC-I-255 M2 — undoing a block that holds a slash restores the producer set,
//! the burned bonds, supply and state root of a node that never applied it.
//!
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome>
//   O1 return (RolledBack)
//   O2 chain tip
//   O3 producer set: records, pending queue, exit history
//   O4 UTXO set and supply (burned Bond UTXOs re-inserted)
//   O5 state root
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (undo entry it records)
//   O1 undo entry: producer snapshot taken for a slash-only block
// PATHS: P-MID slash block mid-epoch (queued) | P-E0 slash block in epoch 0 (flushed in the same block)
//        | P-BND slash flushed at the following boundary, then the boundary and the slash block undone
//        | P-REORG undo, then a sibling block at the same height
// MATRIX
//   O1-O4 x P-MID   -> equal to a node that stopped before the block; O5 after both apply one more block (inc_i_255_m2_probe_rollback)
//   O1-O5 x P-E0    -> equal; status Active again                    (inc_i_255_m2_b9_epoch0_*)
//   O1-O5 x P-BND   -> after the boundary undo: equal to a node that stopped before the boundary;
//                      after the slash-block undo: equal to a node that stopped before it (inc_i_255_m2_b9_across_boundary_*)
//   O2-O5 x P-REORG -> equal to a node that only applied the sibling (inc_i_255_m2_b9_reorg_*)
// INPUT PARTITIONS:
//   slash height: epoch 0 | mid-epoch; depth undone: 1 | 4 (crosses a boundary flush)
//   follow-up: none | sibling block at the same height

use super::inc_i_255_m2_common::*;

async fn twins(height: u64) -> (Rig, Rig) {
    let keys = Keys::new();
    let (mut a, mut b) = (rig(&keys).await, rig(&keys).await);
    a.advance_to(height).await;
    b.advance_to(height).await;
    assert_same_root(
        &snap(&a.node).await,
        &snap(&b.node).await,
        "PRECONDITION: twins agree before the slash block",
    );
    (a, b)
}

async fn assert_restored(a: &mut Rig, b: &mut Rig, what: &str) {
    let pk = a.accused_pk();
    assert!(
        !queued_slash(&a.node, &pk).await,
        "B9 {what}: a queued Slash survived the rollback"
    );
    assert_eq!(
        status(&a.node, &pk).await,
        status(&b.node, &pk).await,
        "B9 {what}: accused status not restored"
    );
    let own = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    assert_eq!(
        live(&a.node, &own).await,
        own,
        "B9 {what}: burned Bond UTXOs not restored"
    );
    assert_same(&snap(&a.node).await, &snap(&b.node).await, what);
    assert_converged(a, b, what).await;
}

async fn rollback_mid_epoch() {
    let (mut a, mut b) = twins(EPOCH).await;
    let pk = a.accused_pk();
    a.step(vec![a.slash()]).await;
    assert!(
        queued_slash(&a.node, &pk).await,
        "PRECONDITION: mid-epoch the slash is queued"
    );
    a.rollback().await;
    assert_restored(&mut a, &mut b, "mid-epoch undo").await;
}

// REQ-SLASH-003 — Decision: a failure means undoing a slash block leaves its Slash queued or its bonds burned, so a reorged node forks at the next boundary (INC-I-185).
#[tokio::test]
async fn inc_i_255_m2_probe_rollback() {
    rollback_mid_epoch().await;
}

// REQ-SLASH-003 — Decision: a failure means an epoch-0 slash (flushed in its own block) survives the undo of that block.
#[tokio::test]
async fn inc_i_255_m2_b9_epoch0_rollback_restores_active_status_and_bonds() {
    let (mut a, mut b) = twins(0).await;
    let pk = a.accused_pk();
    a.step(vec![a.slash()]).await;
    assert!(
        status(&a.node, &pk)
            .await
            .is_some_and(|s| matches!(s, storage::ProducerStatus::Slashed { .. })),
        "PRECONDITION: in epoch 0 the slash is flushed by its own block"
    );
    a.rollback().await;
    assert_restored(&mut a, &mut b, "epoch-0 undo").await;
}

// REQ-SLASH-003 — Decision: a failure means a slash flushed at a boundary is not fully undone by the boundary snapshot plus the slash-block undo.
#[tokio::test]
async fn inc_i_255_m2_b9_across_boundary_rollback_matches_nodes_that_stopped_earlier() {
    let keys = Keys::new();
    let (mut a, mut mid, mut early) = (rig(&keys).await, rig(&keys).await, rig(&keys).await);
    for n in [&mut a, &mut mid, &mut early] {
        n.advance_to(EPOCH).await;
    }
    let pk = a.accused_pk();
    let slash = a.slash();
    a.step(vec![slash.clone()]).await;
    mid.step(vec![slash]).await;
    a.advance_to(2 * EPOCH - 1).await;
    mid.advance_to(2 * EPOCH - 1).await;
    a.advance_to(2 * EPOCH).await;
    assert!(
        status(&a.node, &pk)
            .await
            .is_some_and(|s| matches!(s, storage::ProducerStatus::Slashed { .. })),
        "PRECONDITION: the boundary flushed the slash"
    );

    a.rollback().await;
    assert_same(
        &snap(&a.node).await,
        &snap(&mid.node).await,
        "boundary undo vs node that stopped before the boundary",
    );
    for _ in 0..(EPOCH - 1) {
        a.rollback().await;
    }
    assert_eq!(
        a.best().await.0,
        EPOCH,
        "PRECONDITION: back at the boundary"
    );
    assert_restored(
        &mut a,
        &mut early,
        "slash-block undo after a boundary flush",
    )
    .await;
}

// REQ-SLASH-003 — Decision: a failure means a reorg across the slash block leaves state that differs from a node that never saw the orphan, a permanent split.
#[tokio::test]
async fn inc_i_255_m2_b9_reorg_to_sibling_matches_node_that_never_saw_the_orphan() {
    let (mut a, mut b) = twins(EPOCH).await;
    let pk = a.accused_pk();
    let sibling = b.next_block(Vec::new(), 1).await;
    a.step(vec![a.slash()]).await;
    a.rollback().await;
    a.apply(sibling.clone())
        .await
        .expect("sibling applies after the undo");
    b.apply(sibling).await.expect("sibling applies on the twin");
    assert!(
        !queued_slash(&a.node, &pk).await,
        "B9: the orphaned slash is still queued after the reorg"
    );
    assert_same_root(
        &snap(&a.node).await,
        &snap(&b.node).await,
        "reorg onto a sibling vs node that only saw the sibling",
    );
}
