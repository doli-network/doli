//! INC-I-236 M4: no header-keyed side effect before the header is authenticated.
//! Devnet node, signature AH = 0, every block fed through `Node::handle_new_block`.
//!
//! OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, block, peer) -> Result<()>
//!   O1 equivocation_detector pending proofs (EquivocationProof naming the victim)
//!   O2 mempool SlashProducer tx naming the victim (handle_equivocation; 0 for forged, 1 for P1d)
//!   O3 sync_manager stuck_fork_signal (HeightOccupied{is_better:true} arm)
//!   O4 chain_state.best_hash (must not move for a forged sibling)
//!   O5 return value — not asserted: O1-O3 fire before any validation verdict
//! PATHS: P1 check_block (before classification); P2 HeightOccupied{is_better:true} arm.
//! INPUT PARTITIONS:
//!   P1a forged pi empty, real block first      P1b forged pi empty, forged first
//!   P1c forged wrong-key signature             P1d two validly signed headers (lock)
//!   P2a forged pi empty sibling                P2b forged wrong-key sibling
//!   P2c validly signed sibling (lock)
//! Each P1 cell asserts O1+O2; each P2 forged cell asserts O3+O4; P2c asserts O3.

use crypto::KeyPair;

use super::inc_i_236_m4_common::{
    canonical_two, feed, header_block, proofs_against, reporter_node, slash_txs_against,
    stuck_fork_raised, wall_slot,
};

async fn assert_no_slash_side_effects(node: &doli_node::node::Node, victim: &KeyPair) {
    let pk = victim.public_key();
    assert_eq!(
        proofs_against(node, pk).await,
        0,
        "a header the victim never signed must not produce an EquivocationProof"
    );
    assert_eq!(
        slash_txs_against(node, pk).await,
        0,
        "a header the victim never signed must not put a SlashProducer tx in the mempool"
    );
}

// REQ-AUTH-007 — Decision: a failure means one unsigned gossip header can frame an honest producer for slashing.
#[tokio::test]
async fn req_auth_007_forged_unsigned_after_real_block_yields_no_proof_no_slash() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *victim.public_key();
    let slot = wall_slot(&node);
    let real = header_block(&mut node, 1, slot, genesis, &pk, Some(&victim), 0).await;
    let forged = header_block(&mut node, 1, slot, genesis, &pk, None, 1).await;
    assert_ne!(real.hash(), forged.hash(), "fixture: headers must conflict");
    assert!(
        forged.header.vdf_proof.pi.is_empty(),
        "fixture: forged is unsigned"
    );

    feed(&mut node, real).await;
    feed(&mut node, forged).await;

    assert_no_slash_side_effects(&node, &victim).await;
}

// REQ-AUTH-007 — Decision: a failure means a forged header recorded first turns the victim's own real block into slash evidence.
#[tokio::test]
async fn req_auth_007_forged_unsigned_before_real_block_does_not_poison_detector() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *victim.public_key();
    let slot = wall_slot(&node);
    let forged = header_block(&mut node, 1, slot, genesis, &pk, None, 1).await;
    let real = header_block(&mut node, 1, slot, genesis, &pk, Some(&victim), 0).await;

    feed(&mut node, forged).await;
    feed(&mut node, real).await;

    assert_no_slash_side_effects(&node, &victim).await;
}

// REQ-AUTH-007 — Decision: a failure means a signature by a key other than header.producer is treated as authentic.
#[tokio::test]
async fn req_auth_007_forged_wrong_key_signature_yields_no_proof_no_slash() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let attacker = KeyPair::generate();
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *victim.public_key();
    let slot = wall_slot(&node);
    let real = header_block(&mut node, 1, slot, genesis, &pk, Some(&victim), 0).await;
    let forged = header_block(&mut node, 1, slot, genesis, &pk, Some(&attacker), 1).await;
    assert!(
        !forged.header.vdf_proof.pi.is_empty(),
        "fixture: forged carries a signature"
    );

    feed(&mut node, real).await;
    feed(&mut node, forged).await;

    assert_no_slash_side_effects(&node, &victim).await;
}

// REQ-AUTH-007 — Decision: a failure means the fix disabled detection of real double production (regression lock).
#[tokio::test]
async fn req_auth_007_two_validly_signed_conflicting_headers_still_detected() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *victim.public_key();
    let slot = wall_slot(&node);
    let a = header_block(&mut node, 1, slot, genesis, &pk, Some(&victim), 0).await;
    let b = header_block(&mut node, 1, slot, genesis, &pk, Some(&victim), 1).await;
    assert_ne!(a.hash(), b.hash(), "fixture: headers must conflict");

    feed(&mut node, a).await;
    feed(&mut node, b).await;

    assert_eq!(
        proofs_against(&node, &pk).await,
        1,
        "equivocation must be detected"
    );
    assert_eq!(
        slash_txs_against(&node, &pk).await,
        1,
        "at the devnet slash gate signed double production yields exactly one SlashProducer tx"
    );
}

// REQ-AUTH-012 — Decision: a failure means one unsigned lower-slot header can push any node into fork recovery.
#[tokio::test]
async fn req_auth_012_forged_unsigned_lower_slot_sibling_does_not_signal_stuck_fork() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let (b1, b2) = canonical_two(&mut node, &victim, 10).await;
    stuck_fork_raised(&node).await;
    let forged = header_block(&mut node, 2, 5, b1.hash(), victim.public_key(), None, 1).await;

    feed(&mut node, forged).await;

    assert!(
        !stuck_fork_raised(&node).await,
        "an unsigned lower-slot sibling must not signal stuck fork"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        b2.hash(),
        "tip must not move"
    );
}

// REQ-AUTH-012 — Decision: a failure means a wrong-key signature passes as the producer's for fork-recovery signalling.
#[tokio::test]
async fn req_auth_012_forged_wrong_key_lower_slot_sibling_does_not_signal_stuck_fork() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let attacker = KeyPair::generate();
    let (b1, b2) = canonical_two(&mut node, &victim, 10).await;
    stuck_fork_raised(&node).await;
    let forged = header_block(
        &mut node,
        2,
        5,
        b1.hash(),
        victim.public_key(),
        Some(&attacker),
        1,
    )
    .await;

    feed(&mut node, forged).await;

    assert!(
        !stuck_fork_raised(&node).await,
        "a wrong-key lower-slot sibling must not signal stuck fork"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        b2.hash(),
        "tip must not move"
    );
}

// REQ-AUTH-012 — Decision: a failure means the fix blocked INC-I-040 recovery from a real better sibling (regression lock).
#[tokio::test]
async fn req_auth_012_signed_lower_slot_sibling_still_signals_stuck_fork() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let (b1, _b2) = canonical_two(&mut node, &victim, 10).await;
    stuck_fork_raised(&node).await;
    let sibling = header_block(
        &mut node,
        2,
        5,
        b1.hash(),
        victim.public_key(),
        Some(&victim),
        1,
    )
    .await;

    feed(&mut node, sibling).await;

    assert!(
        stuck_fork_raised(&node).await,
        "a validly signed lower-slot sibling must still signal stuck fork"
    );
}
