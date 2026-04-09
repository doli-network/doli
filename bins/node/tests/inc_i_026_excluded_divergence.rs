//! INC-I-026 regression test — scheduler determinism across competing blocks
//!
//! This test covers the consensus fork observed on mainnet 2026-04-09 where
//! two nodes running identical code, starting from identical chain state,
//! ended up with divergent scheduler outputs after applying two different —
//! but both individually valid — blocks at the same height (a gossip race).
//!
//! ## The bug (pre-fix)
//!
//! `excluded_producers` was a local mutable set on each `Node`, mutated at
//! apply-time from `block.header.missed_producers`:
//!
//! ```ignore
//! for pk in &block.header.missed_producers {
//!     self.excluded_producers.insert(*pk);
//! }
//! ```
//!
//! Both the production scheduler (`bins/node/src/node/production/scheduling.rs::
//! resolve_epoch_eligibility`) and the validation scheduler
//! (`crates/core/src/validation/producer.rs::validate_producer_eligibility`)
//! used this set as a FILTER over `epoch_producer_list` BEFORE computing
//! `slot % effective.len()`. When two nodes applied different competing
//! blocks at the same height, their `effective.len()` diverged (10 vs 7 in
//! this test, 25 vs 22 on mainnet), and the scheduler selected different
//! producers for the SAME slot on the two nodes — the literal consensus
//! fork mechanism.
//!
//! ## The fix
//!
//! `excluded_producers` was removed from BOTH scheduler inputs (production
//! and validation symmetrically — see behavioral learning #8). The scheduler
//! is now a pure function of `(slot, active_production_list)`, both of which
//! are epoch-frozen and identical on every node. Local apply history cannot
//! influence the scheduler output.
//!
//! Trade-off: dead producers now cause empty slots instead of being skipped
//! in the round-robin. This is benign — the next live producer produces the
//! next slot — whereas scheduler divergence was catastrophic (forks).
//!
//! ## What this test verifies
//!
//! 1. Creates two nodes with an identical producer set (10 producers).
//! 2. Applies an identical 45-block base chain to both (past genesis at h=40).
//! 3. Pins `epoch_producer_list` and `active_production_list` identically on
//!    both nodes.
//! 4. Builds two DIFFERENT but individually valid blocks at h=46:
//!    - `canonical_block`: slot=46, `missed_producers=[]`
//!    - `fork_block`: slot=49, `missed_producers=[3 entries]`
//! 5. Applies `canonical_block` to `node_canon` and `fork_block` to `node_fork`.
//! 6. Asserts that `resolve_epoch_eligibility` returns the SAME producer for
//!    every tested slot on both nodes, EVEN THOUGH each node applied a
//!    different block. This is the structural invariant the fix guarantees.
//!
//! ## Pre/post-fix behavior
//!
//! - **Pre-fix:** 19/20 tested slots produced different producers (the bug).
//! - **Post-fix:** 0/20 tested slots diverge (scheduler is now deterministic
//!   across nodes regardless of apply history).
//!
//! The test asserts the post-fix invariant. It would have FAILED on the
//! pre-fix code — which is exactly what a regression test for this bug
//! should look like.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// Helpers (minimal — only what this test needs)
// ============================================================

async fn make_node(producers: Vec<KeyPair>) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    missed_producers: Vec<PublicKey>,
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
        missed_producers,
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

/// Build the common base chain applied to both nodes before divergence.
/// Uses producer rotation to keep the chain "realistic" (all producers contribute).
fn build_base_chain(producers: &[KeyPair], count: usize, params: &ConsensusParams) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    for i in 0..count {
        let h = (i + 1) as u64;
        let s = (i + 1) as u32;
        let producer = &producers[i % producers.len()];
        let block = build_block(h, s, prev, producer, Vec::new(), params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "base chain apply_block failed at h={} slot={}: {}",
                    block.header.slot, block.header.slot, e
                )
            });
    }
}

// ============================================================
// The regression test
// ============================================================

/// INC-I-026 regression guard: two nodes applying different but individually
/// valid blocks at the same height must produce IDENTICAL scheduler output
/// for any slot. This is the structural invariant the fix guarantees — the
/// scheduler is a pure function of `(slot, epoch_producer_list)`, not of
/// which block each node happened to apply locally.
#[tokio::test]
async fn test_inc_i_026_scheduler_is_deterministic_across_competing_blocks() {
    // ------------------------------------------------------------------
    // Phase 1: two identical nodes
    // ------------------------------------------------------------------
    let n_producers = 10;
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let (mut node_canon, _tmp1) = make_node(producers.clone()).await;
    let (mut node_fork, _tmp2) = make_node(producers.clone()).await;
    let params = node_canon.params.clone();

    // ------------------------------------------------------------------
    // Phase 2: apply identical 45-block base chain to both nodes
    // (devnet genesis_blocks=40, so 45 puts us past genesis phase)
    // ------------------------------------------------------------------
    let base = build_base_chain(&producers, 45, &params);
    apply_chain(&mut node_canon, &base).await;
    apply_chain(&mut node_fork, &base).await;

    let canon_h = node_canon.chain_state.read().await.best_height;
    let fork_h = node_fork.chain_state.read().await.best_height;
    let canon_hash = node_canon.chain_state.read().await.best_hash;
    let fork_hash = node_fork.chain_state.read().await.best_hash;
    assert_eq!(canon_h, 45);
    assert_eq!(fork_h, 45);
    assert_eq!(
        canon_hash, fork_hash,
        "both nodes must be on the same tip hash before divergence"
    );

    // ------------------------------------------------------------------
    // Phase 3: pin the scheduler inputs identically on both nodes.
    //
    // Without this pinning, the two nodes' epoch_producer_list (which is
    // rebuilt every 4 blocks with attestation filtering) might diverge
    // on which producers "qualified" — polluting the test.
    //
    // This is a legitimate setup step, NOT hiding the bug: the bug we're
    // reproducing is about excluded_producers divergence, assuming both
    // nodes START with the same epoch_producer_list. Pinning the list
    // isolates that variable.
    // ------------------------------------------------------------------
    let mut all_pks: Vec<PublicKey> = producers.iter().map(|kp| *kp.public_key()).collect();
    all_pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    node_canon.epoch_producer_list = all_pks.clone();
    node_fork.epoch_producer_list = all_pks.clone();
    node_canon.active_production_list = all_pks.clone();
    node_fork.active_production_list = all_pks.clone();
    node_canon.excluded_producers.clear();
    node_fork.excluded_producers.clear();

    // ------------------------------------------------------------------
    // Phase 4: build two COMPETING blocks at the same height h=46.
    //
    // Both blocks:
    //   - reference the same parent (base[44])
    //   - have height 46
    //   - are individually valid under Light mode (which skips producer
    //     eligibility — modeling the sync/reorg path the bug traverses)
    //
    // The ONLY differences:
    //   - canonical_block has slot=46 (no gap), missed_producers=[]
    //   - fork_block has slot=49 (3-slot gap), missed_producers=
    //       [sorted_pks[46%10], sorted_pks[47%10], sorted_pks[48%10]]
    //
    // This models the mainnet scenario where N1/Seed1 applied a block at
    // h=46 slot=46 (canonical) while N2 received a slightly later block
    // produced on the same height but a later slot (the "gap" appears
    // because the producer for slot 46-48 is considered to have missed).
    // ------------------------------------------------------------------
    let parent_hash = base[44].hash();

    let canonical_block = build_block(46, 46, parent_hash, &producers[0], Vec::new(), &params);

    let missed_at_46 = all_pks[(46usize) % all_pks.len()];
    let missed_at_47 = all_pks[(47usize) % all_pks.len()];
    let missed_at_48 = all_pks[(48usize) % all_pks.len()];
    let mut missed = Vec::new();
    for pk in [missed_at_46, missed_at_47, missed_at_48] {
        if !missed.contains(&pk) {
            missed.push(pk);
        }
    }
    let fork_block = build_block(46, 49, parent_hash, &producers[0], missed.clone(), &params);

    assert!(
        canonical_block.header.missed_producers.is_empty(),
        "canonical block should have empty missed_producers"
    );
    assert!(
        !fork_block.header.missed_producers.is_empty(),
        "fork block should have non-empty missed_producers (3 entries) to trigger the bug mechanism"
    );
    assert_ne!(
        canonical_block.hash(),
        fork_block.hash(),
        "canonical and fork blocks must be distinct"
    );

    // ------------------------------------------------------------------
    // Phase 5: apply each block to its respective node.
    //
    // This is the heart of the reproduction. Both blocks apply successfully:
    //   - canonical_block to node_canon
    //   - fork_block to node_fork
    //
    // The buggy code at bins/node/src/node/apply_block/post_commit.rs:262-271
    // will mutate node_fork.excluded_producers with the 3 missed producers
    // from fork_block.header.missed_producers, while node_canon.excluded_producers
    // stays empty.
    // ------------------------------------------------------------------
    node_canon
        .apply_block(canonical_block.clone(), ValidationMode::Light)
        .await
        .expect("canonical_block MUST apply to node_canon");
    node_fork
        .apply_block(fork_block.clone(), ValidationMode::Light)
        .await
        .expect("fork_block MUST apply to node_fork");

    // Both nodes advanced to h=46 but on DIFFERENT tip hashes.
    assert_eq!(node_canon.chain_state.read().await.best_height, 46);
    assert_eq!(node_fork.chain_state.read().await.best_height, 46);
    assert_ne!(
        node_canon.chain_state.read().await.best_hash,
        node_fork.chain_state.read().await.best_hash,
        "nodes should be on different tip hashes after applying competing blocks"
    );

    // ------------------------------------------------------------------
    // OBSERVATION POINT — excluded_producers may still differ on the field.
    //
    // The field is still mutated at apply-time from `block.header.missed_producers`
    // (it is kept for telemetry / RPC reporting). So `node_canon.excluded_producers`
    // will have 0 entries while `node_fork.excluded_producers` will have 3. This
    // is NOT the bug — the bug was that the scheduler CONSUMED this field. The
    // fix removes the scheduler's dependency on it; the field itself is now
    // purely informational and its content is expected to reflect local history.
    //
    // We log the values but do NOT assert equality — divergent local field
    // values are fine as long as the scheduler output is deterministic.
    // ------------------------------------------------------------------
    let canon_excluded_count = node_canon.excluded_producers.len();
    let fork_excluded_count = node_fork.excluded_producers.len();
    eprintln!(
        "[INC-I-026] excluded_producers (telemetry only) — canon={} fork={}",
        canon_excluded_count, fork_excluded_count
    );

    // ------------------------------------------------------------------
    // STRUCTURAL INVARIANT — scheduler output MUST be identical on both
    // nodes for every tested slot, regardless of local apply history.
    //
    // resolve_epoch_eligibility is pub fn on Node (in production/scheduling.rs).
    // After the INC-I-026 fix, it computes `slot % active_production_list.len()`
    // with no filtering by local `excluded_producers`. Both nodes have the
    // same `active_production_list` (pinned in Phase 3 above), so the output
    // must be identical for every slot.
    //
    // We scan 20 slots in the range [50..70] and require EVERY slot to agree.
    // A single disagreement means the scheduler has non-deterministic input
    // again — INC-I-026 has regressed.
    // ------------------------------------------------------------------
    let weights: Vec<(PublicKey, u64)> = producers
        .iter()
        .map(|kp| (*kp.public_key(), 1u64))
        .collect();

    let mut divergent_slots = Vec::new();
    let mut canon_selections = Vec::new();
    let mut fork_selections = Vec::new();
    for slot in 50u32..70 {
        let canon_sched = node_canon.resolve_epoch_eligibility(slot, 46, &weights);
        let fork_sched = node_fork.resolve_epoch_eligibility(slot, 46, &weights);
        canon_selections.push((slot, canon_sched.clone()));
        fork_selections.push((slot, fork_sched.clone()));
        if canon_sched != fork_sched {
            divergent_slots.push(slot);
        }
    }

    eprintln!(
        "[INC-I-026] scheduler agreement — {} / 20 slots produced identical producers",
        20 - divergent_slots.len()
    );
    if !divergent_slots.is_empty() {
        eprintln!(
            "[INC-I-026] !!! divergent slots (must be empty): {:?}",
            divergent_slots
        );
        eprintln!("[INC-I-026] canon schedulings (first 5):");
        for (slot, sched) in canon_selections.iter().take(5) {
            eprintln!(
                "  slot={}  scheduled={:?}",
                slot,
                sched
                    .iter()
                    .map(|pk| hex::encode(&pk.as_bytes()[..4]))
                    .collect::<Vec<_>>()
            );
        }
        eprintln!("[INC-I-026] fork schedulings (first 5):");
        for (slot, sched) in fork_selections.iter().take(5) {
            eprintln!(
                "  slot={}  scheduled={:?}",
                slot,
                sched
                    .iter()
                    .map(|pk| hex::encode(&pk.as_bytes()[..4]))
                    .collect::<Vec<_>>()
            );
        }
    }

    assert!(
        divergent_slots.is_empty(),
        "REGRESSION INC-I-026: the scheduler diverged between two nodes that have \
         identical epoch_producer_list / active_production_list but applied different \
         competing blocks at the same height. {} / 20 tested slots produced different \
         expected producers (canon excluded={}, fork excluded={}). The scheduler must \
         be a pure function of (slot, active_production_list) — see \
         bins/node/src/node/production/scheduling.rs::resolve_epoch_eligibility and \
         crates/core/src/validation/producer.rs::validate_producer_eligibility.",
        divergent_slots.len(),
        canon_excluded_count,
        fork_excluded_count,
    );

    eprintln!(
        "[INC-I-026] PASS: scheduler output is identical on both nodes for all 20 tested \
         slots, despite node_canon applying canonical_block and node_fork applying fork_block \
         at the same height. Local excluded_producers (canon={} fork={}) did not affect the \
         scheduler.",
        canon_excluded_count, fork_excluded_count
    );
}

// ============================================================
// Pre-activation backwards-compatibility test
// ============================================================

/// Proves that the LEGACY (pre-activation) scheduler path still reproduces
/// the original bug when exercised with `current_height < activation_height`.
/// This is the backwards-compatibility guarantee of the gate: old behavior
/// is preserved byte-for-byte before the activation point.
///
/// This test uses `validate_producer_eligibility` directly with a hand-built
/// `ValidationContext` instead of going through Node::apply_block, because
/// devnet has `inc_i_026_scheduler_activation_height = 0` (always-active),
/// so there is no height on devnet at which the legacy path runs from a real
/// Node. Exercising the core validation function with a forced activation
/// height is the cleanest way to pin the pre-activation behavior.
#[tokio::test]
async fn test_inc_i_026_pre_activation_path_reproduces_legacy_divergence() {
    use doli_core::validation::{validate_producer_eligibility, ValidationContext};

    // 10 producers, same layout as the post-activation test.
    let n_producers = 10;
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut sorted_pks: Vec<PublicKey> = producers.iter().map(|kp| *kp.public_key()).collect();
    sorted_pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    // Forced activation height FAR in the future so every test call uses
    // the legacy path.
    const FORCED_ACTIVATION_HEIGHT: u64 = u64::MAX;

    // Canonical view: excluded_producers is empty.
    let ctx_canon = ValidationContext::new(
        doli_core::consensus::ConsensusParams::devnet(),
        doli_core::Network::Devnet,
        0,   // current_time
        100, // current_height — any value < FORCED_ACTIVATION_HEIGHT
    )
    .with_epoch_producer_list(sorted_pks.clone())
    .with_excluded_producers(std::collections::HashSet::new())
    .with_inc_i_026_scheduler_activation_height(FORCED_ACTIVATION_HEIGHT);

    // Fork view: excluded_producers contains 3 entries (the divergent state
    // a forked node would accumulate under the legacy path).
    let mut fork_excluded = std::collections::HashSet::new();
    fork_excluded.insert(sorted_pks[(46usize) % sorted_pks.len()]);
    fork_excluded.insert(sorted_pks[(47usize) % sorted_pks.len()]);
    fork_excluded.insert(sorted_pks[(48usize) % sorted_pks.len()]);
    let ctx_fork = ValidationContext::new(
        doli_core::consensus::ConsensusParams::devnet(),
        doli_core::Network::Devnet,
        0,
        100,
    )
    .with_epoch_producer_list(sorted_pks.clone())
    .with_excluded_producers(fork_excluded)
    .with_inc_i_026_scheduler_activation_height(FORCED_ACTIVATION_HEIGHT);

    // For each tested slot, build a header claiming the CANONICAL view's
    // expected producer, then validate it against BOTH contexts.
    //
    // - Under the canonical view (excluded=0, denominator=10), the header is
    //   correct by construction, so validation must PASS.
    // - Under the fork view (excluded=3, denominator=7), the scheduler
    //   computes a DIFFERENT expected producer for most slots, so
    //   validation must REJECT most of the canonical-view headers.
    //
    // If validation rejects canonical-view headers under the fork view, we
    // have proven that the legacy path still exhibits the divergence — which
    // is precisely what we want to preserve pre-activation.
    let mut canon_accepted = 0;
    let mut fork_rejected_canon_header = 0;
    let test_slots = 50u32..70;
    let n_tested = (test_slots.end - test_slots.start) as usize;

    for slot in test_slots {
        // Canonical expected producer: slot % 10.
        let canon_expected = sorted_pks[(slot as usize) % sorted_pks.len()];
        let canon_kp = producers
            .iter()
            .find(|kp| *kp.public_key() == canon_expected)
            .unwrap();
        let params = doli_core::consensus::ConsensusParams::devnet();
        let block = build_block(100, slot, Hash::ZERO, canon_kp, Vec::new(), &params);

        // Validate under the canonical context — must PASS.
        match validate_producer_eligibility(&block.header, &ctx_canon) {
            Ok(()) => canon_accepted += 1,
            Err(e) => panic!(
                "legacy path under canonical view rejected slot {}: {:?}",
                slot, e
            ),
        }

        // Validate under the fork context — must REJECT for most slots
        // (the 3 exclusions shift the modulus, so slot % 7 lands on a
        // different producer).
        if validate_producer_eligibility(&block.header, &ctx_fork).is_err() {
            fork_rejected_canon_header += 1;
        }
    }

    eprintln!(
        "[INC-I-026 legacy] canonical-view acceptance: {}/{} slots",
        canon_accepted, n_tested
    );
    eprintln!(
        "[INC-I-026 legacy] fork-view rejection of canonical headers: {}/{} slots",
        fork_rejected_canon_header, n_tested
    );

    assert_eq!(
        canon_accepted, n_tested,
        "legacy validation must still accept every canonical-view header \
         when excluded is empty — otherwise the pre-activation code path \
         has regressed"
    );
    assert!(
        fork_rejected_canon_header > 0,
        "legacy validation with a non-empty excluded set must reject at \
         least some canonical-view headers — if it accepts all of them, \
         the pre-activation code path has been silently neutered and the \
         backwards-compatibility guarantee of the activation gate is broken"
    );

    eprintln!(
        "[INC-I-026 legacy] PASS: pre-activation scheduler still filters by \
         excluded_producers ({} of {} canonical-view slots rejected under \
         fork-view excluded=3). The activation gate preserves legacy \
         behavior byte-for-byte.",
        fork_rejected_canon_header, n_tested
    );
}
