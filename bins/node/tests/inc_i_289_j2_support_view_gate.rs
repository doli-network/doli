//! INC-I-289 J-2 (operator ruling 2, SEC-CF-639-004): the bond support view and the
//! fork-credit fill are read only at/above the INC-I-288 gate, so below it (mainnet
//! today) the node must not build or fill them.
//!
// OUTPUT CONTRACT: Node::handle_new_block (ReorgCandidate arm; HeightOccupied and
//   Orphan arms into the wedge door), Node::handle_completed_fork_recovery,
//   Node::on_new_attestation (fork-only hash).
//   O1 doli_support_views_built_total delta per site
//   O2 doli_fork_credits_total delta (fork-credit fills)
//   O3 reach evidence per site: fork recovery started | tip on the recovered tip |
//      tip on the wedge sibling (each is after the view site in its door)
// PATHS: competing block | recovered fork | wedge plan | fork-only attestation
// INPUT PARTITIONS: local tip below the gate (child process, gate 1e9) | at/above it
// MATRIX: below -> O1 = 0 at every site, O2 = 0, O3 reached; above -> O1 >= 1 at
//   every site, O2 >= 1, O3 reached
//! covers: block_handling.rs fork_recovery.rs wedge_escape.rs branch_support.rs gossip_verdicts.rs

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use doli_core::Attestation;
use doli_node::metrics::{FORK_CREDITS, SUPPORT_VIEWS_BUILT};
use inc_i_289_common::*;
use network::sync::CompletedRecovery;
use network::PeerId;

const S: u32 = 210;

fn views() -> u64 {
    SUPPORT_VIEWS_BUILT.get()
}

/// A competing block whose parent is stored above our tip: the ReorgCandidate arm.
async fn competing_block() -> (Rig, doli_core::Block) {
    let mut r = rig(&[S]).await;
    let d = r.ours[0].clone();
    let x = chain_on(&r, d.hash(), F + 2, &[S + 1]).remove(0);
    r.node.block_store.put_block_canonical(&x, F + 2).unwrap();
    let y = chain_on(&r, x.hash(), F + 3, &[S + 2]).remove(0);
    gossip(&mut r, &y).await;
    assert!(
        r.node.sync_manager.read().await.is_fork_recovery_active(),
        "reach: the ReorgCandidate arm ran past its door (fork recovery started)"
    );
    (r, y)
}

/// A heavier walked branch on F (3 v 2 blocks): the recovered-fork evaluation.
async fn recovered_fork() {
    let mut r = rig(&[S, S + 1]).await;
    let walked = chain_on(&r, r.f_hash, F + 1, &[S + 3, S + 4, S + 5]);
    let rec = CompletedRecovery {
        blocks: walked.clone(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };
    r.node.handle_completed_fork_recovery(rec).await.unwrap();
    assert_on(
        &r,
        &walked[2],
        F + 3,
        "reach: the recovered branch is adopted",
    )
    .await;
}

/// A tied sibling (retained), then its child (2 v 1): the wedge plan.
async fn wedge_plan() {
    let mut r = rig(&[S]).await;
    let sib = chain_on(&r, r.f_hash, F + 1, &[S + 2, S + 3]);
    gossip(&mut r, &sib[0]).await;
    gossip(&mut r, &sib[1]).await;
    assert_on(&r, &sib[1], F + 2, "reach: the wedge plan reorged").await;
}

/// [support views at each door site; fork credits from two member attestations].
async fn drive() -> [u64; 4] {
    let v0 = views();
    let (mut r, y) = competing_block().await;
    let v1 = views();
    recovered_fork().await;
    let v2 = views();
    wedge_plan().await;
    let v3 = views();
    let c0 = FORK_CREDITS.get();
    let own = *r.node.producer_key.as_ref().unwrap().public_key();
    for k in r.keys.clone().iter().filter(|k| *k.public_key() != own) {
        let a = Attestation::new(
            y.hash(),
            y.header.slot,
            F + 3,
            1,
            k.private_key(),
            *k.public_key(),
        );
        r.node
            .on_new_attestation(a.to_bytes(), PeerId::random())
            .await;
    }
    [v1 - v0, v2 - v1, v3 - v2, FORK_CREDITS.get() - c0]
}

// J-2a INC-I-289 — Decision: a failure means mainnet (gate u64::MAX) still clones the producer set and every cached block's facts per fork event for a view nothing reads.
#[tokio::test]
async fn j2a_below_gate_builds_no_support_view() {
    if !in_below_ah_child("j2a_below_gate_builds_no_support_view") {
        return;
    }
    let got = drive().await;
    assert_eq!(
        got,
        [0, 0, 0, 0],
        "J-2a: below the gate [competing, recovered, wedge] views and fork credits must be 0"
    );
}

// J-2b INC-I-289 — Decision: a failure means the gate also removed the view above it, so the bond doors weigh nothing.
#[tokio::test]
async fn j2b_above_gate_builds_support_view() {
    let r = rig(&[S]).await;
    let gate = (r.node.config.network.params()).inc_i_288_fork_choice_activation_height;
    assert!(gate <= F, "fixture: the node runs above the gate ({gate})");
    drop(r);
    let got = drive().await;
    assert!(
        got.iter().all(|n| *n >= 1),
        "J-2b: above the gate every site builds a view and the attestation is credited: {got:?}"
    );
}
