// OUTPUT CONTRACT: Node reorg doors at h >= AH (devnet AH = 0)
//   execute_reorg / handle_completed_fork_recovery / handle_new_block (wedge) /
//   try_consume_force_reorg / check_producer_eligibility
// OUTPUTS:
//   O1 return: execute_reorg Err; force outcome != Executed; eligibility Err
//   O2 chain_state.best_hash / best_height
//   O3 block_store canonical index at h=6
//   O4 state_db persisted chain-state tip
//   O6 fork_block_cache entry (insert AND overwrite)
//   O7 reorg_handler fork weight; O8 wedge_retained_tips; O9 stuck-fork signal
//   O10 rejected-branch memo (is_fork_branch_rejected)
// PATHS: P7 execute_reorg, P8 fork walk, P10 wedge escape, P11 forceReorgTo, eligibility
// INPUT PARTITIONS: forged (names the leader, pi by another key) | unsigned (empty pi) |
//   forged tail (head valid) | signed control | unregistered signer
// MATRIX: O1-O4 x {P7,P8,P10,P11} x {forged, unsigned}; O6-O10 on the doors that write them;
//   signed control per door; below-AH partition in inc_i_236_m2_reorg_doors_b.rs.
// covers: bins/node/src/node/validation_checks/mod.rs
// covers: bins/node/src/node/block_handling.rs
// covers: bins/node/src/node/fork_recovery.rs
// covers: bins/node/src/node/fork_walk.rs
// covers: bins/node/src/node/wedge_escape.rs
// covers: bins/node/src/node/force_reorg.rs
// covers: bins/node/src/node/header_auth.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use crypto::KeyPair;
use doli_node::node::ForceReorgOutcome;
use inc_i_236_m2_reorg_support::*;
use Sig::{Forged, Unsigned, Valid};

const REFUSED: [(&str, [Sig; 3]); 3] = [
    ("forged", [Forged, Forged, Forged]),
    ("unsigned", [Unsigned, Unsigned, Unsigned]),
    ("forged-tail", [Valid, Valid, Forged]),
];

async fn assert_tip_is(fx: &Fx, tip: &doli_core::Block, height: u64, label: &str) {
    let cs = fx.node.chain_state.read().await;
    assert_eq!(cs.best_hash, tip.hash(), "{label}: tip");
    assert_eq!(cs.best_height, height, "{label}: height");
}

// REQ-AUTH-020 — Decision: a failure means a forged fork can make execute_reorg roll back the canonical tip.
// REQ-AUTH-021 — Decision: a failure means a refused fork still rewrote in-memory or persisted chain state or the canonical index.
#[tokio::test]
async fn req_auth_020_p7_execute_reorg_forged_fork_no_rollback() {
    for (label, sigs) in REFUSED {
        let mut fx = fixture().await;
        let before = snap(&fx.node).await;
        let fork = branch(&fx, sigs);
        let result = run_execute_reorg(&mut fx, &fork).await;
        assert_eq!(
            snap(&fx.node).await,
            before,
            "{label}: execute_reorg must verify every fork block BEFORE the rollback"
        );
        assert!(
            result.is_err(),
            "{label}: execute_reorg must return Err on a bad fork signature"
        );
    }

    let mut fx = fixture().await;
    let fork = branch(&fx, [Valid; 3]);
    run_execute_reorg(&mut fx, &fork)
        .await
        .expect("signed control: execute_reorg");
    assert_tip_is(&fx, &fork[2], 8, "signed control").await;
}

// REQ-AUTH-020 — Decision: a failure means a forged walked branch writes fork weight or a cache body, or reorgs.
// REQ-AUTH-021 — Decision: a failure means a pi-variant walk blacklists the honest branch with the same hashes.
#[tokio::test]
async fn req_auth_020_p8_recovered_fork_forged_no_weight_no_cache() {
    for (label, sigs) in REFUSED {
        let mut fx = fixture().await;
        let before = snap(&fx.node).await;
        let fork = branch(&fx, sigs);
        let _ = run_walk(&mut fx, &fork).await;
        assert_eq!(snap(&fx.node).await, before, "{label}: no reorg");
        for b in fork
            .iter()
            .filter(|b| b.header.vdf_proof.pi != sign_of(&fx, b))
        {
            assert!(
                !weight_recorded(&fx.node, &b.hash()).await,
                "{label}: no fork weight for slot {}",
                b.header.slot
            );
            assert!(
                cached(&fx.node, &b.hash()).await.is_none(),
                "{label}: no cache insert for slot {}",
                b.header.slot
            );
        }
        assert!(
            !fx.node
                .sync_manager
                .read()
                .await
                .is_fork_branch_rejected(fork[2].hash(), fx.block_a.hash()),
            "{label}: a pi-variant walk must not mark the branch hash rejected"
        );
    }

    let mut fx = fixture().await;
    let honest = branch(&fx, [Valid; 3]);
    seed_cache(&fx.node, &honest[..1]).await;
    let forged = branch(&fx, [Forged; 3]);
    assert_eq!(
        honest[0].hash(),
        forged[0].hash(),
        "fixture: pi is outside the hash"
    );
    let _ = run_walk(&mut fx, &forged).await;
    let kept = cached(&fx.node, &honest[0].hash())
        .await
        .expect("overwrite: entry");
    assert_eq!(
        kept.header.vdf_proof.pi, honest[0].header.vdf_proof.pi,
        "overwrite: a forged walk must not replace a signed cached body"
    );

    let mut fx = fixture().await;
    let fork = branch(&fx, [Valid; 3]);
    run_walk(&mut fx, &fork)
        .await
        .expect("signed control: walk");
    assert_tip_is(&fx, &fork[2], 8, "signed control").await;
}

fn sign_of(fx: &Fx, b: &doli_core::Block) -> Vec<u8> {
    let mut h = b.header.clone();
    block_sig::sign_header(&mut h, leader(&fx.producers, b.header.slot));
    h.vdf_proof.pi
}

// REQ-AUTH-020 — Decision: a failure means a forged gossip sibling can drive the wedge escape into a reorg.
// REQ-AUTH-021 — Decision: a failure means a forged sibling is weighted, cached, retained or raises stuck-fork.
#[tokio::test]
async fn req_auth_020_p10_wedge_escape_forged_no_reorg() {
    let peer = network::PeerId::random();
    for (label, sigs) in [REFUSED[0], REFUSED[1]] {
        let mut fx = fixture().await;
        let before = snap(&fx.node).await;
        let fork = branch(&fx, sigs);
        fx.node
            .handle_new_block(fork[0].clone(), peer)
            .await
            .unwrap();
        let b = fork[0].hash();
        assert!(
            cached(&fx.node, &b).await.is_none(),
            "{label}: no cache write"
        );
        assert!(
            !weight_recorded(&fx.node, &b).await,
            "{label}: no fork weight"
        );
        assert!(
            fx.node.wedge_retained_tips.is_empty(),
            "{label}: not retained as validated"
        );
        assert!(!stuck_fork_raised(&fx.node).await, "{label}: no stuck-fork");
        for blk in &fork[1..] {
            fx.node.handle_new_block(blk.clone(), peer).await.unwrap();
        }
        assert_eq!(snap(&fx.node).await, before, "{label}: no reorg");
    }

    let mut fx = fixture().await;
    let fork = branch(&fx, [Valid; 3]);
    for blk in &fork {
        fx.node.handle_new_block(blk.clone(), peer).await.unwrap();
    }
    assert_tip_is(&fx, &fork[2], 8, "signed control").await;
}

// REQ-AUTH-020 — Decision: a failure means an armed forceReorgTo lands on forged bodies.
#[tokio::test]
async fn req_auth_020_p11_force_reorg_forged_no_reorg() {
    for (label, sigs) in REFUSED {
        let mut fx = fixture().await;
        let before = snap(&fx.node).await;
        let fork = branch(&fx, sigs);
        seed_cache(&fx.node, &fork).await;
        fx.node
            .sync_manager
            .write()
            .await
            .arm_force_reorg(fork[2].hash());
        let outcome = fx.node.try_consume_force_reorg().await;
        assert_eq!(snap(&fx.node).await, before, "{label}: no reorg");
        assert_ne!(
            outcome,
            Some(ForceReorgOutcome::Executed),
            "{label}: outcome"
        );
    }

    let mut fx = fixture().await;
    let fork = branch(&fx, [Valid; 3]);
    seed_cache(&fx.node, &fork).await;
    fx.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(fork[2].hash());
    let outcome = fx.node.try_consume_force_reorg().await;
    assert_eq!(outcome, Some(ForceReorgOutcome::Executed), "signed control");
    assert_tip_is(&fx, &fork[2], 8, "signed control").await;
}

// REQ-AUTH-020 — Decision: a failure means a self-signed outsider passes eligibility and gets weight via unwrap_or(1).
#[tokio::test]
async fn req_auth_020_unregistered_signed_producer_rejected_by_eligibility() {
    let fx = fixture().await;
    let slot = BRANCH_SLOTS[0];
    let outsider = KeyPair::generate();
    let rogue = block(6, slot, fx.base_tip, &outsider, Valid, &fx.params);
    assert!(
        fx.node.check_producer_eligibility(&rogue).await.is_err(),
        "a validly self-signed unregistered producer must be ineligible"
    );
    let scheduled = block(
        6,
        slot,
        fx.base_tip,
        leader(&fx.producers, slot),
        Valid,
        &fx.params,
    );
    fx.node
        .check_producer_eligibility(&scheduled)
        .await
        .expect("sentinel: the scheduled leader's signed block is eligible");
}
