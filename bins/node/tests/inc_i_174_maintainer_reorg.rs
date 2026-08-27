//! INC-I-174 M1 — a reorged-out maintainer rotation is never undone: the REORG half.
//!
//! Sibling of `inc_i_174_maintainer_undo.rs`, which covers `rollback_one_block`. Split
//! only for the 800-line test-file budget; the defect, the harness and the OUTPUT
//! CONTRACT are the same. Read that file's header first — it carries the full defect
//! statement and the output enumeration.
//!
//! covers: types.rs undo.rs batch.rs mod.rs governance.rs rollback.rs block_handling.rs
//! covers: maintainer.rs maintainer_wellformed.rs set.rs digest.rs periodic.rs
//! covers: maintainer_rewind/ state_update.rs helpers.rs
//!
//! WHY THIS FILE EXISTS SEPARATELY FROM THE ROLLBACK TESTS
//! -------------------------------------------------------
//! `execute_reorg` runs its OWN rollback loop, independent of `rollback_one_block`.
//! INC-I-040 is the precedent, verbatim: "rollback_one_block does restore it ... but
//! missed execute_reorg which has its own independent rollback loop". Analysis §7.3 names
//! this the highest single regression risk of the fix. A fix that lands only in
//! `rollback.rs` makes the sibling file green and leaves this one red.
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTION UNDER TEST
//!   R2 `Node::execute_reorg(&mut self, ReorgResult, Block) -> Result<()>`
//!      (observed after `Node::apply_block` applied blocks carrying governance txs
//!       through the real `process_transaction_governance` site)
//!   Plus the drop-then-re-mine sequence, which reaches `governance.rs` a second time.
//!
//! OBSERVABLE OUTPUTS (Rust rules: `&mut self` receiver + persistent stores)
//!   O1 `self.maintainer_state.set.members`       — WHO may sign a release
//!   O2 `self.maintainer_state.set.threshold`     — HOW MANY must sign
//!   O3 `self.maintainer_state.set.last_updated`  — `getMaintainerSet.last_change_block`
//!   O4 `self.maintainer_state.last_derived_height` — the one-shot seed arm (INC-I-172 R1)
//!   O5 `maintainer_state.bin`, read back with `MaintainerState::load` (rule AQ-5)
//!   O6 `maintainer_set_digest(set, genesis_hash)` — the fleet divergence instrument
//!   O7 `self.chain_state.best_height` — the reorg must actually have happened, or every
//!      root assertion is satisfied vacuously by a refused reorg
//!   O8 `self.maintainer_rewind_count` — REQ-174-010. A rewind that installs a root no
//!      chain ever had ALSO publishes a success signal, so the value channel alone does
//!      not cover the defect: the operator must not be told "restored" for a divergence.
//!   O9 `self.maintainer_rewind_unrestored_count` — REQ-174-005. Asserted so a "fix" that
//!      merely converts the wrong restore into a loud refusal is distinguishable from one
//!      that correctly concludes the range needs no restore at all.
//!   (The `MAINTAINER_SET_DIGEST` log LINE is not asserted: `bins/node` has no
//!    tracing-capture dev-dependency and adding one would edit a non-test manifest.
//!    REQ-174-008 is covered here through O6, the VALUE the anchor must carry.)
//!
//! CODE PATHS
//!   P4 undo-based reorg, ONE rotation inside the rewind range
//!   P5 undo-based reorg, TWO rotations inside the rewind range
//!   P6 drop, then the winning branch re-mines the SAME rotation at a DIFFERENT height
//!   P7 TWO reorgs over an OVERLAPPING height window (reviewer F1)
//!
//! INPUT PARTITIONS:
//!   IP-M  rotation in the MIDDLE of a 3-block rewind — proves the restore is not keyed
//!         on the rewind TIP (which is all `rollback_one_block` ever sees)
//!   IP-2  two rotations at different heights in ONE range. The only partition that can
//!         tell oldest-wins from newest-wins: oldest-wins yields 5 members, newest-wins
//!         yields 4, no-restore yields 3. A member-count assertion alone cannot
//!         distinguish a correct fix from an intermediate-state fix on any other input.
//!   IP-Q  re-mine after an EXPLICIT drop. `add_maintainer` is membership-idempotent but
//!         `last_updated`-NON-idempotent (`Err(AlreadyMaintainer)`, caller only `warn!`s),
//!         so a naive reorg test passes here FOR THE WRONG REASON. The drop is therefore
//!         constructed explicitly and the assertion is on `last_updated`.
//!   IP-C  two nodes at the same canonical tip, one of which reorged past the rotation
//!   IP-F  a SECOND reorg whose range overlaps the first. The abandoned branch left a
//!         `cf_undo` maintainer record behind at a height the replacement branch re-used
//!         WITHOUT a rotation, and nothing refreshes or deletes that record — `put_undo`
//!         is unconditional, `put_maintainer_undo` is not. The only partition that can
//!         tell "the record still belongs to the block at this height" from "a snapshot
//!         exists here, therefore restore it": the fossil yields a 4-member set that
//!         exists on NO chain, the correct answer is the 5-member branch-B root.
//!
//! MATRIX
//!   IP-M : O1 O2 O3 O4 O5 O6 O7
//!   IP-2 : O1 O2 O3 O6 O7
//!   IP-Q : O1 O3 O7
//!   IP-C : O1 O2 O3 O6
//!   IP-F : O1 O2 O3 O6 O7 O8 O9
//!
//! ANTI-VACUITY
//!   The no-rotation control lives in the sibling file
//!   (`req_174_002_rollback_across_a_block_with_no_rotation_is_a_no_op`, GREEN today).
//!   IP-2's three distinguishable outcomes are this file's own internal control.

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{maintainer_set_digest, MaintainerChangeData, MaintainerSignature};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, MaintainerSet, Transaction};
use doli_node::node::Node;
use network::sync::ReorgResult;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

// ===========================================================================
// HARNESS — mirrors bins/node/tests/fork_recovery.rs so the two files stay
// comparable, plus a maintainer root attached with `set_maintainer_state`.
// ===========================================================================

/// A devnet node with `n` genesis producers and a SEEDED maintainer root.
///
/// `Node::new_for_test` is hardwired to `Network::Devnet`, whose
/// `maintainer_derivation_activation_height` is 0 — so every governance decision below
/// takes the POST-activation branch (distinct signers, fail-closed), which is the branch
/// mainnet will be on. The seed happens at height 0, so `set.last_updated == 0` before any
/// rotation and a rotation at height H is unambiguously distinguishable from it.
async fn seeded_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root(&node).await;
    assert_eq!(
        seeded.members.len(),
        n,
        "harness: all {n} genesis producers must be seated in the trust root"
    );
    (node, producers, temp)
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    extra_txs: Vec<Transaction>,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let mut txs = vec![coinbase];
    txs.extend(extra_txs);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
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

    Block::new(header, txs)
}

/// A governance transaction the real apply path will act on.
///
/// `signers` must be CURRENT members: above the gate `verify_multisig_at` counts DISTINCT
/// member slots, so the caller has to supply `threshold` different seated keys.
fn maintainer_tx(is_add: bool, target: &PublicKey, signers: &[&KeyPair]) -> Transaction {
    let action = if is_add { "add" } else { "remove" };
    let message = format!("{}:{}", action, target.to_hex()).into_bytes();
    let signatures: Vec<MaintainerSignature> = signers
        .iter()
        .map(|kp| {
            MaintainerSignature::new(
                *kp.public_key(),
                crypto::signature::sign(&message, kp.private_key()),
            )
        })
        .collect();
    let data = MaintainerChangeData::new(*target, signatures);
    Transaction {
        version: 1,
        tx_type: if is_add {
            TxType::AddMaintainer
        } else {
            TxType::RemoveMaintainer
        },
        inputs: vec![],
        outputs: vec![],
        extra_data: data.to_bytes(),
    }
}

async fn apply(node: &mut Node, block: &Block) {
    node.apply_block(block.clone(), ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("apply_block failed at slot {}: {e}", block.header.slot));
}

/// O1/O2/O3 — the in-memory trust root.
async fn root(node: &Node) -> MaintainerSet {
    node.maintainer_state
        .as_ref()
        .expect("harness: a maintainer root must be attached")
        .read()
        .await
        .set
        .clone()
}

/// O4 — the seed arm.
async fn derived_height(node: &Node) -> u64 {
    node.maintainer_state
        .as_ref()
        .unwrap()
        .read()
        .await
        .last_derived_height
}

/// O6 — the value the `MAINTAINER_SET_DIGEST` anchor publishes.
fn digest(node: &Node, set: &MaintainerSet) -> String {
    hex::encode(maintainer_set_digest(
        set,
        node.params.genesis_hash.as_bytes(),
    ))
}

/// O5 — read the persisted root back INDEPENDENTLY (assertion rule AQ-5).
fn on_disk(dir: &TempDir) -> MaintainerState {
    MaintainerState::load(dir.path()).expect(
        "O5: the persisted trust root must still decode. If a rewind leaves a file that \
         `MaintainerState::load` refuses, the node fails closed at the NEXT restart — the \
         rewind has converted a divergence into an outage.",
    )
}

/// Assert every value-channel output of a maintainer root at once, so no test can
/// fill a matrix cell by asserting only the member count.
fn assert_root_eq(actual: &MaintainerSet, expected: &MaintainerSet, ctx: &str) {
    assert_eq!(
        actual.members, expected.members,
        "{ctx}: O1 — member list. These keys are WHO may authorize a root binary install \
         on this host via the auto-updater. A stale extra member is a live signing key the \
         canonical chain has no record of; a stale missing member is a key the chain still \
         trusts and this host will refuse."
    );
    assert_eq!(
        actual.threshold, expected.threshold,
        "{ctx}: O2 — threshold. HOW MANY distinct members must sign."
    );
    assert_eq!(
        actual.last_updated, expected.last_updated,
        "{ctx}: O3 — last_updated is served as `getMaintainerSet.last_change_block`. It is \
         the only field that says WHICH BLOCK this root came from, so it is the divergence \
         instrument (REQ-174-007). Membership can be right while this is wrong — that is \
         exactly the AlreadyMaintainer case in analysis §1.4."
    );
}

// ===========================================================================
// REQ-174-002 (Must) — `rollback_one_block` restores the maintainer set.
// ===========================================================================
// REQ-174-003 (Must) — `execute_reorg` restores over the whole rewind range.
// This is the INC-I-040 regression shape: `execute_reorg` has its OWN rollback
// loop and has historically drifted from `rollback_one_block`.
// ===========================================================================

/// Drive the real `execute_reorg` over `rollback_depth` blocks, replacing them with one
/// coinbase-only block that builds on the common ancestor.
async fn reorg_away(node: &mut Node, rollback_depth: usize, producer: &KeyPair) {
    let params = node.params.clone();
    let current = node.chain_state.read().await.best_height;
    let target = current - rollback_depth as u64;

    let mut rollback = Vec::new();
    for h in ((target + 1)..=current).rev() {
        rollback.push(
            node.block_store
                .get_block_by_height(h)
                .expect("block_store read")
                .expect("harness: the rolled-back block must be stored")
                .hash(),
        );
    }
    let ancestor = node
        .block_store
        .get_block_by_height(target)
        .expect("block_store read")
        .expect("harness: the common ancestor must be stored");

    // Slot 900+ keeps the replacement distinct from every block built above.
    let triggering = build_block(
        target + 1,
        900 + target as u32,
        ancestor.hash(),
        producer,
        &params,
        vec![],
    );
    let result = ReorgResult {
        rollback,
        common_ancestor: ancestor.hash(),
        new_blocks: vec![triggering.hash()],
        weight_delta: 1,
    };
    node.execute_reorg(result, triggering)
        .await
        .expect("execute_reorg errored");
}

/// REQ-174-003 bullet 1, IP-M x P4. O1 O2 O3 O4 O6 O7. **RED — assertion.**
///
/// A 3-block rewind whose MIDDLE block carried the rotation, replaced by a branch that
/// does not contain it. `rollback_one_block` and `execute_reorg` are independent loops:
/// fixing only the former is the INC-I-040 regression, verbatim.
#[tokio::test]
async fn req_174_003_execute_reorg_undoes_a_rotation_in_the_middle_of_the_range() {
    let (mut node, producers, tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let before_digest = digest(&node, &before);
    let before_derived = derived_height(&node).await;

    let mut prev = Hash::ZERO;
    for h in 1..=2u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    // h=3 — the rotation, in the MIDDLE of the 3..=5 rewind range.
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let rot = build_block(3, 3, prev, &producers[0], &params, vec![tx]);
    prev = rot.hash();
    apply(&mut node, &rot).await;
    assert_eq!(
        root(&node).await.members.len(),
        5,
        "harness: rotation applied"
    );

    for h in 4..=5u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    assert_eq!(node.chain_state.read().await.best_height, 5);

    reorg_away(&mut node, 3, &producers[1]).await;

    // O7 — the reorg itself must have happened (3 rolled back, 1 applied).
    assert_eq!(
        node.chain_state.read().await.best_height,
        3,
        "O7: 5 - 3 rolled back + 1 applied = 3"
    );

    let restored = root(&node).await;
    assert_root_eq(&restored, &before, "IP-M");
    assert!(
        !restored.members.contains(newcomer.public_key()),
        "IP-M: O1 — `execute_reorg` runs its OWN rollback loop. INC-I-040 was exactly \
         this: `rollback_one_block` restored the state and `execute_reorg` did not."
    );
    assert_eq!(digest(&node, &restored), before_digest, "IP-M: O6");
    assert_eq!(derived_height(&node).await, before_derived, "IP-M: O4");
    assert_root_eq(&on_disk(&tmp).set, &before, "IP-M/disk");
}

/// REQ-174-003 bullet 2, IP-2 x P5. O1 O2 O3 O6 O7. **RED — assertion.**
///
/// TWO rotations at different heights inside ONE rewind range. The restored value must be
/// the one from before the OLDEST — the "keep overwriting while walking, oldest wins"
/// shape already used for `producer_snapshot` at `block_handling.rs`.
///
/// This partition is the ONLY one that can distinguish oldest-wins from newest-wins:
/// oldest-wins yields 5 members, newest-wins yields 4, no-restore yields 3.
#[tokio::test]
async fn req_174_003_execute_reorg_restores_from_before_the_oldest_rotation_in_range() {
    let (mut node, producers, _tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let before_digest = digest(&node, &before);
    assert_eq!(before.members.len(), 5, "harness");

    let mut prev = Hash::ZERO;
    for h in 1..=2u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    // h=3 — remove member[4]  => 4 members
    let victim_a = before.members[4];
    let signers_a: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a)
        .take(3)
        .collect();
    let b3 = build_block(
        3,
        3,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_a, &signers_a)],
    );
    prev = b3.hash();
    apply(&mut node, &b3).await;
    assert_eq!(root(&node).await.members.len(), 4, "harness: first removal");

    // h=4 — remove member[3]  => 3 members (MIN_MAINTAINERS; no more removals possible)
    let victim_b = before.members[3];
    let signers_b: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a && *kp.public_key() != victim_b)
        .take(3)
        .collect();
    let b4 = build_block(
        4,
        4,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_b, &signers_b)],
    );
    prev = b4.hash();
    apply(&mut node, &b4).await;
    let mid = root(&node).await;
    assert_eq!(mid.members.len(), 3, "harness: second removal");

    // h=5 — plain block, so the range 3..=5 is not entirely rotations.
    let b5 = build_block(5, 5, prev, &producers[0], &params, vec![]);
    apply(&mut node, &b5).await;

    reorg_away(&mut node, 3, &producers[1]).await;
    assert_eq!(node.chain_state.read().await.best_height, 3, "O7");

    let restored = root(&node).await;
    assert_root_eq(&restored, &before, "IP-2");
    assert_eq!(
        restored.members.len(),
        5,
        "IP-2: O1 — the restored set must be the one from BEFORE THE OLDEST rotation in \
         the range (5 members). 4 members means the newest snapshot won and an \
         intermediate state was installed; 3 means nothing was restored at all."
    );
    assert!(
        restored.members.contains(&victim_a) && restored.members.contains(&victim_b),
        "IP-2: O1 — both members removed inside the rewind range must be back"
    );
    assert_eq!(digest(&node, &restored), before_digest, "IP-2: O6");
}

/// REQ-174-003 / reviewer F1, IP-F x P7. O1 O2 O3 O6 O7 O8 O9. **RED — assertion.**
///
/// TWO reorgs over an OVERLAPPING window. The first one abandons a branch that carried a
/// rotation at h=7; the replacement branch re-uses h=7 with NO rotation. `put_undo` is
/// unconditional so the `UndoData` at h=7 is overwritten, but `put_maintainer_undo` is
/// written only when the NEW block carries a rotation — so the maintainer record from the
/// ABANDONED branch survives, and nothing on either rewind path deletes it
/// (`prune_undo_before` reaps the TAIL, 100 blocks behind).
///
/// A second rewind that trusts "a snapshot exists at h, therefore restore it" therefore
/// installs a member list that exists on NO chain — and, because that is the success exit,
/// increments `maintainer_rewind_count` and logs `MAINTAINER_REWIND_RESTORED` at `info!`.
/// The operator gets a SUCCESS signal for a divergence in the auto-updater's install
/// authority, which is strictly worse than the pre-INC-I-174 behaviour (stale but
/// explicable). Hence O8/O9 are asserted alongside the value channels.
///
/// The fix is read-side: the snapshot at `h` is authority only if the block NOW at `h`
/// carries a rotation. Everything else is a fossil.
#[tokio::test]
async fn req_174_003_a_fossil_snapshot_from_an_abandoned_branch_is_never_the_trust_root() {
    let (mut node, producers, _tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let r0 = root(&node).await;
    let r0_digest = digest(&node, &r0);
    assert_eq!(
        r0.members.len(),
        5,
        "harness: R0 is the seeded 5-member root"
    );

    // --- Branch A: h=1..=10, rotation X at h=6 and rotation Y at h=7 --------------
    let mut prev = Hash::ZERO;
    for h in 1..=5u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    let victim_a = r0.members[4];
    let signers_a: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a)
        .take(3)
        .collect();
    let b6 = build_block(
        6,
        6,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_a, &signers_a)],
    );
    prev = b6.hash();
    apply(&mut node, &b6).await;
    assert_eq!(
        root(&node).await.members.len(),
        4,
        "harness: rotation X applied at h=6 (R0 -> R1)"
    );

    let victim_b = r0.members[3];
    let signers_b: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a && *kp.public_key() != victim_b)
        .take(3)
        .collect();
    let b7 = build_block(
        7,
        7,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_b, &signers_b)],
    );
    prev = b7.hash();
    apply(&mut node, &b7).await;
    assert_eq!(
        root(&node).await.members.len(),
        3,
        "harness: rotation Y applied at h=7 (R1 -> R2)"
    );

    for h in 8..=10u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    assert_eq!(node.chain_state.read().await.best_height, 10, "harness");

    // --- Reorg 1: back to the common ancestor h=5. Correct today. -----------------
    reorg_away(&mut node, 5, &producers[1]).await;
    assert_eq!(node.chain_state.read().await.best_height, 6, "O7: reorg-1");
    assert_root_eq(&root(&node).await, &r0, "reorg-1");
    assert_eq!(node.maintainer_rewind_count, 1, "O8: reorg-1 restored once");

    // The defect's PRECONDITION, asserted so the test cannot pass vacuously: the record
    // written for branch A's h=7 is still in `cf_undo` after the rewind that abandoned it.
    let fossil = node.state_db.get_maintainer_undo(7).expect(
        "harness/precondition: branch A's h=7 maintainer record must still be in cf_undo. \
         If this fires, some later change started deleting rewound maintainer records and \
         every assertion below would pass for a reason unrelated to the fossil.",
    );
    assert_eq!(
        fossil.set.members.len(),
        4,
        "harness/precondition: the fossil is R1 — a set that will exist on NO chain once \
         branch B replaces h=7"
    );

    // --- Branch B: h=7..=10 replaced, carrying NO rotation ------------------------
    let mut prev = node
        .block_store
        .get_block_by_height(6)
        .expect("block_store read")
        .expect("harness: the reorg-1 replacement block at h=6 must be canonical")
        .hash();
    for h in 7..=10u64 {
        let b = build_block(h, 900 + h as u32 + 10, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    assert_eq!(node.chain_state.read().await.best_height, 10, "harness");
    assert_root_eq(&root(&node).await, &r0, "branch-B");

    // --- Reorg 2: back to the common ancestor h=6, range 7..=10 -------------------
    reorg_away(&mut node, 4, &producers[2]).await;
    assert_eq!(node.chain_state.read().await.best_height, 7, "O7: reorg-2");

    let after = root(&node).await;
    assert_root_eq(&after, &r0, "IP-F");
    assert_eq!(
        after.members.len(),
        5,
        "IP-F: O1 — branch B carries no rotation anywhere in 7..=10, so the correct \
         post-reorg root is branch B's own root R0 (5 members). 4 members means the \
         FOSSIL snapshot left at h=7 by the abandoned branch A was installed: a member \
         list that exists on no chain, and one that decides who may authorize a binary \
         install on this host through the auto-updater."
    );
    assert_eq!(digest(&node, &after), r0_digest, "IP-F: O6");
    assert_eq!(
        node.maintainer_rewind_count, 1,
        "IP-F: O8 — the second rewind must NOT report a restore. A wrong restore is not \
         only wrong, it is ANNOUNCED as a success (`MAINTAINER_REWIND_RESTORED`, `info!`), \
         so the operator's only fleet-wide signal points the wrong way."
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 0,
        "IP-F: O9 — and it must not be a loud REFUSAL either. Branch B provably carries \
         no rotation in 7..=10, so the range needs no restore at all; announcing an \
         unrestorable rewind here would be a false alarm on a security-graded anchor."
    );
}

// ===========================================================================
// REQ-174-007 (Should) — `last_change_block` convergence.
// This is the case §1.4 shows fails today for a reason a naive test misses.
// ===========================================================================

/// REQ-174-003 bullet 3 / REQ-174-007, IP-Q x P6. O1 O3 O6. **RED — assertion.**
///
/// The winning branch RE-MINES the same rotation at a DIFFERENT height. After the reorg the
/// set must be correct AND `last_updated` must equal the NEW canonical height.
///
/// Why this needs care: `MaintainerSet::add_maintainer` is membership-idempotent but
/// `last_updated`-NON-idempotent — it returns `Err(AlreadyMaintainer)` and stamps nothing,
/// and `governance.rs` only `warn!`s on that error. So on today's code the re-mined
/// rotation is a NO-OP and `last_updated` keeps the OLD height. A test that asserts only
/// membership passes for the wrong reason. The DROP is therefore constructed explicitly
/// (an intervening rollback), and the assertion is on `last_updated`.
#[tokio::test]
async fn req_174_007_a_re_mined_rotation_stamps_the_new_canonical_height() {
    let (mut node, producers, _tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    let fork_point = prev;

    // Losing branch: the rotation at h=4.
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let losing = build_block(4, 4, fork_point, &producers[0], &params, vec![tx.clone()]);
    apply(&mut node, &losing).await;
    assert_eq!(root(&node).await.last_updated, 4, "harness: stamped at h=4");

    // The DROP, made explicit.
    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 3, "O7");

    // Winning branch: a filler block at h=4, then the SAME rotation re-mined at h=5.
    let filler = build_block(4, 400, fork_point, &producers[1], &params, vec![]);
    let filler_hash = filler.hash();
    apply(&mut node, &filler).await;
    let winner = build_block(5, 401, filler_hash, &producers[1], &params, vec![tx]);
    apply(&mut node, &winner).await;

    let final_root = root(&node).await;
    assert!(
        final_root.members.contains(newcomer.public_key()),
        "IP-Q: O1 — the re-mined rotation must be in effect"
    );
    assert_eq!(
        final_root.members.len(),
        5,
        "IP-Q: O1 — exactly one copy of the newcomer; a double-apply would be worse than \
         a no-apply"
    );
    assert_eq!(
        final_root.last_updated, 5,
        "IP-Q: O3 — `last_updated` (served as `getMaintainerSet.last_change_block`) must \
         name the NEW canonical height 5, not the reorged-out height 4. On today's code \
         the rollback leaves the newcomer seated, so the re-mined tx hits \
         `Err(AlreadyMaintainer)`, `governance.rs` only warns, and this field silently \
         keeps pointing at a block that is no longer on any chain."
    );
}

/// REQ-174-007, cross-node convergence. O1 O2 O3 O6. **RED — assertion.**
///
/// Two nodes at the SAME tip: one saw the rotation and reorged past it, one never saw it.
/// Both must report the same `last_change_block` and the same digest. This is the property
/// an operator actually checks across the fleet.
#[tokio::test]
async fn req_174_007_two_nodes_at_the_same_tip_agree_on_last_change_block() {
    let producers: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();

    async fn boot(producers: &[KeyPair]) -> (Node, TempDir) {
        let temp = TempDir::new().unwrap();
        let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
            .await
            .unwrap();
        node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
        node.maybe_bootstrap_maintainer_set(0).await;
        (node, temp)
    }

    let (mut reorged, _t1) = boot(&producers).await;
    let (mut clean, _t2) = boot(&producers).await;
    let params = reorged.params.clone();

    // Common prefix h=1..=3 on both.
    let mut prev = Hash::ZERO;
    let mut common = Vec::new();
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        common.push(b);
    }
    for b in &common {
        apply(&mut reorged, b).await;
        apply(&mut clean, b).await;
    }

    // `reorged` additionally applies a rotation at h=4 and then rolls it back.
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let rot = build_block(
        4,
        4,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(true, newcomer.public_key(), &signers)],
    );
    apply(&mut reorged, &rot).await;
    assert!(reorged
        .rollback_one_block()
        .await
        .expect("rollback errored"));

    // Both now advance on the SAME canonical block at h=4.
    let canonical = build_block(4, 444, prev, &producers[1], &params, vec![]);
    apply(&mut reorged, &canonical).await;
    apply(&mut clean, &canonical).await;

    assert_eq!(
        reorged.chain_state.read().await.best_hash,
        clean.chain_state.read().await.best_hash,
        "harness: both nodes must be at the same tip, or the comparison is meaningless"
    );

    let a = root(&reorged).await;
    let b = root(&clean).await;
    assert_root_eq(&a, &b, "REQ-174-007");
    assert_eq!(
        digest(&reorged, &a),
        digest(&clean, &b),
        "REQ-174-007: O6 — two nodes at the same canonical tip must publish the same \
         MAINTAINER_SET_DIGEST. This is the ONLY fleet-wide instrument for the divergence \
         INC-I-174 creates, and a divergent install trust root is invisible in every other \
         signal: heights match, state roots match, `getMaintainerSet` looks well formed."
    );
}
