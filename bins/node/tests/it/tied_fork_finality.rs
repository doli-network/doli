//! INC-I-190 tied-fork wedge reproduction + F2 depth-2 finality fix proof.
//!
//! DETERMINISTIC, in-process reproduction (no live network, no timing, no random
//! slot races). Uses the real Node / apply_block / handle_new_block / plan_reorg
//! machinery — the SAME harness pattern as fork_recovery.rs TEST 13 — so the exact
//! same-height weight-tied fork lands on demand every run.
//!
//! THE SCENARIO (INC-I-190 verdict):
//!   1. Chain genesis..h=5, applied to a node running THIS worktree's fixed logic.
//!   2. Same-height TIE at h=6: sibling A (canonical lower slot) and sibling B
//!      (adjacent higher slot), same parent (h=5), equal per-block weight.
//!   3. A is applied first and accumulates attestation weight PAST the 67%
//!      finality threshold.
//!   4. The B branch EXTENDS (B, D1=h7, D2=h8) until strictly longer/heavier.
//!
//! THE FIX (F2, INC-I-190 D1): finality requires a locally-applied descendant at
//! depth >= CONFIRMATION_DEPTH (=2). A pending block at h=6 can no longer
//! self-finalize at depth 0 the instant it crosses 67%. Because the tie's A tip is
//! NEVER extended (B extends instead), A never reaches depth 2, so finality never
//! locks on A, and plan_reorg is free to switch onto the strictly-longer B branch.
//! Pre-F2 (depth-0 lock) the node wedged on A forever.
//
// OUTPUT CONTRACT (per .claude/protocols/output-contract.md)
//   Functions under test: Node::handle_new_block / on_new_attestation / apply_block
//                         + FinalityTracker::check_finality (the F2 primitive)
//   OBSERVABLE OUTPUTS asserted:
//     O1: node.chain_state.best_hash                          (A-tip vs B-tip d2)
//     O2: node.chain_state.best_height                        (6 wedged vs 8 escaped)
//     O3: sync_manager.last_finalized_height()                (checkpoint present/absent)
//     O4: sync_manager.reorg_handler().last_finality_height() (reorg-gate arm state)
//     O5: FinalityTracker::check_finality(tip) -> Option<cp>  (checkpoint object)
//   INPUT PARTITIONS:
//     IP1: A tip at depth 0, >=67% weight            => F2: no checkpoint (O3=None,O4=None)
//     IP2: A tip at depth 1, >=67% weight            => F2: no checkpoint
//     IP3: A branch extended to depth 2, >=67%       => checkpoint at h=6 (the pre-F2 lock)
//     IP4: B branch strictly longer, finality UNSET  => REORG onto B (O2=8, O1=d2)
//     IP5: B branch strictly longer, finality SET@6  => WEDGE on A (O2=6, O1=A)
//   MATRIX: IP4 (F2 efficacy) is the primary reliability cell; IP5 is the contrast
//     that proves the assertion catches a depth-0 regression; IP1..IP3 machine-check
//     the depth boundary that separates the two.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::finality::{FinalityTracker, CONFIRMATION_DEPTH};
use doli_core::validation::ValidationMode;
use doli_core::{Attestation, Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS (standalone — mirror fork_recovery.rs, namespaced in this module)
// ============================================================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

fn build_chain(
    start_height: u64,
    start_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = prev_hash;
    for i in 0..count {
        let h = start_height + i as u64;
        let s = start_slot + i as u32;
        let block = build_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at slot={}: {}", block.header.slot, e));
    }
}

/// Build the tied-fork common prefix (h=1..5) + canonical sibling A (h=6, lower
/// slot) applied as the local tip, and return (base_tip_hash, block_a).
///
/// Mirrors fork_recovery.rs TEST 13's construction exactly so the same-height,
/// equal-per-block-weight tie is byte-for-byte reproducible.
async fn setup_tie_with_a_tip(node: &mut Node, producers: &[KeyPair]) -> (Hash, Block) {
    let params = node.params.clone();

    // Common prefix h=1..5.
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(node, &base).await;
    assert_eq!(node.chain_state.read().await.best_height, 5);
    let base_tip = base[4].hash(); // h=5

    // Canonical sibling A at h=6 with the LOWER (better) slot — becomes local tip.
    let block_a = build_block(6, 100, base_tip, &producers[0], &params);
    apply_chain(node, std::slice::from_ref(&block_a)).await;
    assert_eq!(node.chain_state.read().await.best_height, 6);
    assert_eq!(node.chain_state.read().await.best_hash, block_a.hash());

    (base_tip, block_a)
}

/// Build the competing, strictly-longer B branch: B(h=6, higher slot) and
/// descendants D1(h=7), D2(h=8), each produced by the deterministically-scheduled
/// slot leader so the wedge-escape eligibility gate (INC-I-143 M2, FIX A) ADMITS
/// the branch. Same producer-selection logic as fork_recovery.rs TEST 13.
fn build_b_branch(
    producers: &[KeyPair],
    base_tip: Hash,
    params: &ConsensusParams,
) -> (Block, Block, Block) {
    let mut scheduled: Vec<&KeyPair> = producers.iter().collect();
    scheduled.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let leader = |slot: u32| scheduled[(slot as usize) % scheduled.len()];

    let block_b = build_block(6, 200, base_tip, leader(200), params);
    let d1 = build_block(7, 201, block_b.hash(), leader(201), params);
    let d2 = build_block(8, 202, d1.hash(), leader(202), params);
    (block_b, d1, d2)
}

/// Drive the REAL finality ingress: every genuine ProducerSet member attests to
/// block `block`. on_new_attestation derives each attester's authority from the
/// LOCAL ProducerSet (Seam A / F1) and feeds add_attestation_weight, so the
/// finality numerator reaches 100% of total network weight (>= the 67% threshold).
/// Nothing is spoofed — these are the real genesis producers.
async fn attest_all_producers_for(
    node: &mut Node,
    producers: &[KeyPair],
    block: &Block,
    height: u64,
) {
    let peer = network::PeerId::random();
    for p in producers {
        let att = Attestation::new(
            block.hash(),
            block.header.slot,
            height,
            1, // self-declared weight is IGNORED (F1 derives from ProducerSet)
            p.private_key(),
            *p.public_key(),
        );
        assert!(
            att.verify().is_ok(),
            "attestation must be validly self-signed"
        );
        node.on_new_attestation(att.to_bytes(), peer).await;
    }
}

// ============================================================
// TEST 1 (PRIMARY, IP4): F2 efficacy — the tie RESOLVES, no wedge.
// ============================================================
#[tokio::test]
async fn test_inc_i190_f2_tied_fork_reorgs_instead_of_wedging() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let peer = network::PeerId::random();

    // Prefix h=1..5 + canonical sibling A (h=6) as the local tip.
    let (base_tip, block_a) = setup_tie_with_a_tip(&mut node, &producers).await;

    // Precondition: nothing finalized yet.
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        None,
        "precondition: no finality before attestations"
    );

    // Every real producer attests A -> numerator reaches 100% of total weight,
    // well PAST the 67% threshold. (Real F1 ingress; no spoofing.)
    attest_all_producers_for(&mut node, &producers, &block_a, 6).await;

    // O3/O4 (IP1) — F2 EFFICACY: A holds >=67% weight but sits at depth 0, so it
    // must NOT be finalized. No depth-0 self-finality -> the reorg gate stays
    // un-armed and the tie remains resolvable.
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        None,
        "F2: A at depth 0 must NOT finalize despite >=67% weight (INC-I-190 D1)"
    );
    assert_eq!(
        node.sync_manager
            .read()
            .await
            .reorg_handler()
            .last_finality_height(),
        None,
        "F2: reorg finality gate must be un-armed at depth 0"
    );

    // Now the B branch out-extends A: B(h6), D1(h7), D2(h8) via the gossip path.
    let (block_b, d1, d2) = build_b_branch(&producers, base_tip, &params);
    node.handle_new_block(block_b.clone(), peer).await.unwrap();
    node.handle_new_block(d1.clone(), peer).await.unwrap();
    node.handle_new_block(d2.clone(), peer).await.unwrap();

    // O1/O2 (IP4) — the node REORGS onto the strictly-longer sibling branch via
    // the existing plan_reorg/execute_reorg path. No permanent wedge; no
    // genesis-fallback / snap.
    assert_eq!(
        node.chain_state.read().await.best_height,
        8,
        "F2: node must escape the tie and reorg onto the longer B branch (h=8)"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        d2.hash(),
        "F2: tip must be the B-branch tip d2, not the A sibling"
    );

    // SAFETY INVARIANT (no permanent wedge): finality must never sit ABOVE the
    // canonical tip. After the escape the finalized height (if any) stays <= 8, so
    // it can never block the tip the node just adopted.
    //
    // OBSERVED AFTER-EFFECT (honest finding, not a wedge): last_finalized_height()
    // is Some(6) here, not None. The finality depth gate is HEIGHT-only
    // (applied_tip_height >= pending.height + CONFIRMATION_DEPTH), not ancestry-
    // aware: A's stale pending entry (100% weight) finalizes at h=6 the moment the
    // *B* branch apply reaches h=8 during execute_reorg. This is benign — it lands
    // AFTER the reorg has already switched the tip to d2@8, and 6 <= 8 so it does
    // not re-wedge — but a branch-agnostic finalization of a rolled-back block is
    // worth the developer's attention (reported alongside this test).
    let finalized = node.sync_manager.read().await.last_finalized_height();
    let best_height = node.chain_state.read().await.best_height;
    assert!(
        finalized.is_none_or(|h| h <= best_height),
        "F2: finality ({finalized:?}) must never exceed the canonical tip ({best_height}) — else the tip is wedged"
    );
    assert_eq!(
        finalized,
        Some(6),
        "observed after-effect: height-only depth gate finalizes the rolled-back A@6 once the B branch reaches h=8 (benign; below the tip)"
    );
}

// ============================================================
// TEST 2 (CONTRAST, IP5): a finalized A at h=6 WEDGES the same sequence.
// ============================================================
//
// Proves the primary assertion would CATCH a depth-0 regression: if A is finalized
// at h=6 (which pre-F2 happened the instant A crossed 67% at depth 0), the SAME B
// branch can no longer be adopted — plan_reorg refuses to cross the finalized
// height and the node stays wedged on the minority A branch.
//
// The finality LOCK here is armed through the REAL public finality API
// (track_block_for_finality with a descendant that reaches depth 2 — the exact
// production trigger described in the F2 commit). Under F2 that trigger requires a
// real A-branch descendant at h>=8; the tie never provides one, which is precisely
// why F2 keeps A un-finalized in TEST 1. Here we supply that descendant on purpose
// to reconstruct the pre-F2 locked state and show what it costs.
#[tokio::test]
async fn test_inc_i190_depth0_finality_lock_would_wedge_tied_fork() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let peer = network::PeerId::random();

    let (base_tip, block_a) = setup_tie_with_a_tip(&mut node, &producers).await;

    // Every producer attests A -> >=67% weight accumulated on A.
    attest_all_producers_for(&mut node, &producers, &block_a, 6).await;

    // IP1 sanity: still un-finalized at depth 0 (this is exactly F2).
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        None,
        "depth 0: A not finalized yet (F2)"
    );

    // ARM THE LOCK: simulate an A-branch descendant reaching depth 2. This is the
    // production ancestor-finalization trigger — a newly applied block at h=8 makes
    // A(h=6) eligible (8 >= 6 + CONFIRMATION_DEPTH). The descendant itself has no
    // attestations, so ONLY A crosses the threshold and finalizes.
    {
        let mut sync = node.sync_manager.write().await;
        let fake_descendant = crypto::hash::hash(&[0xD2, 0xAA]);
        sync.track_block_for_finality(fake_descendant, 8, 202, 3);
    }

    // IP3: A is now finalized at h=6, and the reorg finality gate is armed at 6.
    // (This also proves the >=67% weight in TEST 1 was real, not silently dropped.)
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        Some(6),
        "arming: A must finalize at h=6 once a depth-2 descendant exists"
    );
    assert_eq!(
        node.sync_manager
            .read()
            .await
            .reorg_handler()
            .last_finality_height(),
        Some(6),
        "arming: reorg finality gate armed at the finalized height 6"
    );

    // Feed the SAME strictly-longer B branch.
    let (block_b, d1, d2) = build_b_branch(&producers, base_tip, &params);
    node.handle_new_block(block_b.clone(), peer).await.unwrap();
    node.handle_new_block(d1.clone(), peer).await.unwrap();
    node.handle_new_block(d2.clone(), peer).await.unwrap();

    // O1/O2 (IP5) — WEDGE: plan_reorg's finality guard rejects crossing the
    // finalized height (the tie's common ancestor is h=5 < finalized h=6), so the
    // node stays on the minority A branch. This is the INC-I-190 wedge.
    assert_eq!(
        node.chain_state.read().await.best_height,
        6,
        "regression contrast: a finalized A at h=6 WEDGES — the longer B branch is refused"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        block_a.hash(),
        "regression contrast: tip stays the finalized A sibling, never advances to d2"
    );
    assert_ne!(
        node.chain_state.read().await.best_hash,
        d2.hash(),
        "regression contrast: node did NOT adopt the longer branch (wedge)"
    );
}

// ============================================================
// TEST 3 (CONTRAST, IP1..IP3): the depth boundary at the finality primitive.
// ============================================================
//
// Machine-checks the exact toggle that separates escape from wedge, on the real
// FinalityTracker public API with the scenario's numbers (block at h=6, 100%
// weight). At depth 0/1 NO checkpoint is produced (F2 — the reorg gate stays
// un-armed, TEST 1 escapes). At depth 2 the checkpoint appears at h=6 (the lock
// pre-F2 created immediately at depth 0, TEST 2 wedges). If CONFIRMATION_DEPTH were
// reverted to 0, check_finality(6) would return Some and TEST 1's escape assertion
// would flip to a wedge — which is how this suite catches the regression.
#[test]
fn test_inc_i190_finality_checkpoint_absent_at_depth0_present_at_depth2() {
    assert_eq!(
        CONFIRMATION_DEPTH, 2,
        "this contrast assumes the F2 depth-2 confirmation constant"
    );

    let mut tracker = FinalityTracker::new();
    let a_hash = crypto::hash::hash(&[6u8]); // stands in for sibling A at h=6
    tracker.track_block(a_hash, 6, 100, 3); // total network weight = 3

    // 3 distinct genuine attesters, weight 1 each => 100% (>= 67%).
    for seed in 0u8..3 {
        tracker.add_attestation_weight(a_hash, PublicKey::from_bytes([seed; 32]), 1);
    }

    // IP1: depth 0 (applied tip == A's height) => NO checkpoint (F2 escape-enabler).
    assert!(
        tracker.check_finality(6).is_none(),
        "depth 0: F2 must NOT create a checkpoint for A (no depth-0 self-finality)"
    );
    // IP2: depth 1 => still NO checkpoint.
    assert!(
        tracker.check_finality(7).is_none(),
        "depth 1: F2 must NOT create a checkpoint for A"
    );
    // IP3: depth 2 => checkpoint at h=6 (the lock the pre-F2 depth-0 rule created
    // the instant A crossed 67%).
    let cp = tracker
        .check_finality(8)
        .expect("depth 2: A must finalize (>=67% + depth-2 descendant)");
    assert_eq!(
        cp.height, 6,
        "the checkpoint that would wedge the tie is at h=6"
    );
    assert!(
        cp.is_finalized(),
        "checkpoint must be at/above the 67% threshold"
    );
}
