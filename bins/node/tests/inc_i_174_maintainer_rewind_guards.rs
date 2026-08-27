//! INC-I-174 M1 — the REFUSAL branches of the maintainer rewind.
//!
//! Third sibling of `inc_i_174_maintainer_undo.rs` (the `rollback_one_block` half) and
//! `inc_i_174_maintainer_reorg.rs` (the `execute_reorg` half). Split from them only for the
//! 800-line test-file budget; read the first file's header for the full defect statement.
//!
//! covers: maintainer_rewind/ commit.rs mod.rs periodic.rs rollback.rs maintainer.rs
//!
//! WHY THIS FILE EXISTS
//! --------------------
//! Reviewer findings F2 and F9. The two sibling files cover what the rewind RESTORES; this
//! one covers every way it decides NOT to restore. Those branches are the security-graded
//! half — each one leaves this host's release-verification trust root un-rewound, so each
//! one must be COUNTED and ANNOUNCED rather than taken silently (REQ-174-005 AC-3).
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTION UNDER TEST
//!   `Node::commit_maintainer_rewind` and `Node::plan_maintainer_rewind`, both observed
//!   through the real `Node::rollback_one_block` caller (they are `pub(in crate::node)` /
//!   `pub(super)`, so an integration test reaches them only through a rewind).
//!
//! OBSERVABLE OUTPUTS
//!   O1 `self.maintainer_state.set.members`         — WHO may sign a release
//!   O3 `self.maintainer_state.set.last_updated`    — `getMaintainerSet.last_change_block`
//!   O5 `maintainer_state.bin` via `MaintainerState::load` (rule AQ-5)
//!   O7 `self.chain_state.best_height`              — the rewind must have happened, or
//!      every root assertion is satisfied vacuously by a refused rollback
//!   O8 `self.maintainer_rewind_count`              — the SUCCESS counter (REQ-174-010)
//!   O9 `self.maintainer_rewind_unrestored_count`   — the REFUSAL counter (REQ-174-005)
//!   (The `MAINTAINER_REWIND_UNRESTORED` log LINE is not asserted: `bins/node` has no
//!    tracing-capture dev-dependency and adding one would edit a non-test manifest. The
//!    counter is the machine-checkable half the requirement names.)
//!
//! CODE PATHS
//!   P8  `commit.rs` seed re-arm guard, BELOW `maintainer_derivation_activation_height`
//!   P9  `commit.rs` seed re-arm guard, AT OR ABOVE the gate (the control)
//!   P10 `mod.rs` `Ok(None)` from `get_block_by_height` -> `reason::BLOCK_UNREADABLE`
//!   P11 `commit.rs` `ms.save()` failure -> `reason::PERSIST_FAILED`
//!
//! INPUT PARTITIONS
//!   IP-PRE   a restorable snapshot of 1..4 members, rewound at a height BELOW the
//!            derivation gate. Below the gate `maintainer_seed_is_done` is
//!            `is_fully_bootstrapped()` (`len >= 5`), so a 4-member root is "not yet
//!            seeded" and the one-shot bootstrap RE-ARMS on the next applied block,
//!            re-deriving the trust root from LIVE producer state (INC-I-172 R1). The
//!            partition exists because every other node test runs on Devnet, whose gate is
//!            0, so only the post-activation branch was ever exercised — while mainnet's
//!            gate is 172_000 and mainnet is BELOW it.
//!   IP-POST  the SAME 4-member snapshot at or above the gate. The control: it must be
//!            installed. Without it, a "fix" that refuses every partial restore would pass
//!            IP-PRE for the wrong reason.
//!   IP-HOLE  a rewind range whose block is missing from the block store (INC-I-152 /
//!            AUDIT-P1-003 shape). CANNOT PROVE, not PROVABLY DIVERGED.
//!   IP-RO    the trust-root file cannot be written. The restore succeeded in memory and
//!            must be put BACK, or memory and disk diverge in a way no restart resolves.
//!
//! MATRIX
//!   IP-PRE  : O1 O3 O5 O7 O8 O9
//!   IP-POST : O1 O3 O7 O8 O9
//!   IP-HOLE : O1 O7 O8 O9
//!   IP-RO   : O1 O5 O8 O9
//!
//! ANTI-VACUITY
//!   IP-POST is IP-PRE's control and differs from it in ONE input (the network whose
//!   activation height is read). `no_hole_control` is IP-HOLE's: the identical rewind with
//!   the block left in place must take neither counter off zero.

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, MaintainerSet, Network, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

// ===========================================================================
// HARNESS — same shape as the two sibling files.
// ===========================================================================

/// A devnet node with `n` genesis producers and a SEEDED maintainer root.
async fn seeded_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    node.maybe_bootstrap_maintainer_set(0).await;
    assert_eq!(
        root(&node).await.members.len(),
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

async fn root(node: &Node) -> MaintainerSet {
    node.maintainer_state
        .as_ref()
        .expect("harness: a maintainer root must be attached")
        .read()
        .await
        .set
        .clone()
}

/// Build h=1 and h=2, each carrying one `RemoveMaintainer`, so the tip's undo snapshot is
/// a FOUR-member set. Returns the root as of each boundary: `(R0 5 members, R1 4, R2 3)`.
async fn two_removals(
    node: &mut Node,
    producers: &[KeyPair],
) -> (MaintainerSet, MaintainerSet, MaintainerSet) {
    let params = node.params.clone();
    let r0 = root(node).await;
    assert_eq!(
        r0.members.len(),
        5,
        "harness: R0 is the seeded 5-member root"
    );

    let victim_a = r0.members[4];
    let signers_a: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a)
        .take(3)
        .collect();
    let b1 = build_block(
        1,
        1,
        Hash::ZERO,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_a, &signers_a)],
    );
    apply(node, &b1).await;
    let r1 = root(node).await;
    assert_eq!(r1.members.len(), 4, "harness: first removal (R0 -> R1)");

    let victim_b = r0.members[3];
    let signers_b: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a && *kp.public_key() != victim_b)
        .take(3)
        .collect();
    let b2 = build_block(
        2,
        2,
        b1.hash(),
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim_b, &signers_b)],
    );
    apply(node, &b2).await;
    let r2 = root(node).await;
    assert_eq!(r2.members.len(), 3, "harness: second removal (R1 -> R2)");

    // The record the rollback below will find at h=2 is R1 — FOUR members.
    let snapshot = node
        .state_db
        .get_maintainer_undo(2)
        .expect("harness/precondition: h=2 carries a rotation, so it must have a snapshot");
    assert_eq!(
        snapshot.set.members.len(),
        4,
        "harness/precondition: the restorable snapshot must be the 1..4-member class — a \
         5-member snapshot is `is_fully_bootstrapped()` and would pass the pre-activation \
         guard too, making the partition non-discriminating"
    );

    (r0, r1, r2)
}

// ===========================================================================
// REQ-174-002 AC-4 / reviewer F2 — the seed re-arm guard must ask the SAME
// question `periodic.rs` asks, on the SAME side of the derivation gate.
// ===========================================================================

/// IP-PRE x P8. O1 O3 O5 O7 O8 O9. **RED — assertion.**
///
/// `commit_maintainer_rewind` refuses a snapshot whose installation would re-arm the
/// one-shot bootstrap seed. It used to spell that condition out inline as
/// `members.is_empty() && last_derived_height == 0` — which is the POST-activation form of
/// `Node::maintainer_seed_is_done` only. Below `maintainer_derivation_activation_height`
/// the predicate is `is_fully_bootstrapped()` (`len >= 5`), so a restored set of 1..4
/// members satisfies NEITHER: the inline guard lets it through, and
/// `maybe_bootstrap_maintainer_set` — driven on every applied block — then re-derives this
/// host's trust root from LIVE producer state, re-arming any key governance removed
/// (INC-I-172 R1). The rewind is counted as a SUCCESS while doing that.
///
/// Mainnet's gate is 172_000, so mainnet is on this side of it today. Every other node test
/// runs on Devnet, whose gate is 0, so only the post-activation branch was ever exercised.
///
/// The network is switched AFTER the chain is built and BEFORE the rewind: the chain must
/// be built under Devnet governance rules (the harness's `maintainer_tx` signs for the
/// post-activation distinct-signer check), and the only thing the switch changes on the
/// rewind path is which side of the gate `commit_maintainer_rewind` reads.
#[tokio::test]
async fn req_174_002_ac4_b_a_partial_restore_below_the_derivation_gate_is_refused() {
    let (mut node, producers, tmp) = seeded_node(5).await;
    let (_r0, _r1, r2) = two_removals(&mut node, &producers).await;

    // The params override: Testnet's gate is 127_200, far above this tip of 2.
    node.config.network = Network::Testnet;
    assert!(
        node.config
            .network
            .params()
            .maintainer_derivation_activation_height
            > node.chain_state.read().await.best_height,
        "harness: the whole partition is 'tip is BELOW the derivation gate'"
    );

    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 1, "O7");

    let after = root(&node).await;
    assert_eq!(
        after.members, r2.members,
        "IP-PRE: O1 — the live trust root must be KEPT. Installing the 4-member snapshot \
         below the gate leaves `maintainer_seed_is_done` false, so the one-shot bootstrap \
         re-arms and `apply_block` re-derives this host's install authority from LIVE \
         producer state on the very next block (INC-I-172 R1). Fail closed instead."
    );
    assert_eq!(after.last_updated, r2.last_updated, "IP-PRE: O3");
    assert_eq!(
        MaintainerState::load(tmp.path())
            .expect("O5: the persisted root must still decode")
            .set
            .members,
        r2.members,
        "IP-PRE: O5 — and the refusal must not have written the snapshot to disk either"
    );
    assert_eq!(
        node.maintainer_rewind_count, 0,
        "IP-PRE: O8 — a refusal must NEVER be counted as a restore. The counter is the \
         rate denominator an operator uses to read the refusal counter."
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 1,
        "IP-PRE: O9 — REQ-174-005 AC-3: no silent route. The rewind left the trust root \
         un-rewound, so it must say so through the counter and the \
         MAINTAINER_REWIND_UNRESTORED anchor."
    );
}

/// IP-POST x P9. O1 O3 O7 O8 O9. **GREEN — control.**
///
/// The SAME 4-member snapshot, differing in exactly ONE input: the node stays on Devnet,
/// whose gate is 0, so the tip is AT OR ABOVE it. Above the gate the seed is one-shot and a
/// non-empty root is "already seeded", so there is nothing to re-arm and the restore must
/// go through. Without this control a fix that refused every partial restore would satisfy
/// IP-PRE for a reason that has nothing to do with the gate.
#[tokio::test]
async fn req_174_002_ac4_b_control_the_same_partial_restore_above_the_gate_is_installed() {
    let (mut node, producers, _tmp) = seeded_node(5).await;
    let (_r0, r1, _r2) = two_removals(&mut node, &producers).await;

    assert_eq!(
        node.config
            .network
            .params()
            .maintainer_derivation_activation_height,
        0,
        "harness: Devnet's gate is 0, so every height is at or above it"
    );

    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 1, "O7");

    let after = root(&node).await;
    assert_eq!(
        after.members, r1.members,
        "IP-POST: O1 — above the gate the 4-member snapshot IS the correct pre-block root \
         and must be installed. If this fails, the F2 fix over-refused."
    );
    assert_eq!(after.last_updated, r1.last_updated, "IP-POST: O3");
    assert_eq!(node.maintainer_rewind_count, 1, "IP-POST: O8");
    assert_eq!(node.maintainer_rewind_unrestored_count, 0, "IP-POST: O9");
}

// ===========================================================================
// REQ-174-005 / reviewer F9 — the two refusal branches that had no test.
// ===========================================================================

/// IP-HOLE x P10. O1 O7 O8 O9. **GREEN — coverage, not a fix.**
///
/// A block missing from the store inside the rewind range. This is the branch an operator
/// is most likely to reach in the field (INC-I-152 / AUDIT-P1-003 holed store), and it is
/// the one the `reason=` token exists to separate from the rest: `block_unreadable` means
/// CANNOT PROVE, not PROVABLY DIVERGED. It must still be counted — a rewind that cannot
/// rule out a dropped rotation has left the trust root in an unverified state.
///
/// The `Err(..)` sibling branch at the same site is a declared gap (test-plan section 6):
/// forcing a RocksDB read error requires corrupting or closing the handle under the live
/// node, and both branches produce the same token, the same plan variant and the same
/// counter — they differ only in the prose.
#[tokio::test]
async fn req_174_005_an_unreadable_block_in_the_rewind_range_is_counted_not_silent() {
    let (mut node, producers, _tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let mut prev = Hash::ZERO;
    let mut tip_hash = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        tip_hash = b.hash();
        apply(&mut node, &b).await;
    }

    // Punch the hole at the tip, the height `rollback_one_block` plans over.
    node.block_store
        .remove_canonical_entry(3, tip_hash)
        .expect("block_store write");
    assert!(
        node.block_store
            .get_block_by_height(3)
            .expect("block_store read")
            .is_none(),
        "harness: the hole must actually exist, or this asserts nothing"
    );

    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 2, "O7");

    assert_eq!(
        root(&node).await.members,
        before.members,
        "IP-HOLE: O1 — the live root is KEPT. `block_unreadable` establishes no divergence, \
         so degrading the root would turn a cannot-prove into a real one."
    );
    assert_eq!(
        node.maintainer_rewind_count, 0,
        "IP-HOLE: O8 — nothing was restored, so nothing may be reported restored"
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 1,
        "IP-HOLE: O9 — REQ-174-005 AC-3. A rewind that cannot read a block in its own range \
         cannot prove that height carried no rotation, so it must not pass silently."
    );
}

/// IP-HOLE control. **GREEN.** The identical rewind with the block left in place must move
/// NEITHER counter: `plan_maintainer_rewind` returns `Unchanged` and `commit` is a no-op.
/// Without this the test above would pass even if every rollback incremented the counter.
#[tokio::test]
async fn req_174_005_control_a_readable_range_with_no_rotation_is_a_silent_no_op() {
    let (mut node, producers, _tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 2, "O7");

    assert_eq!(root(&node).await.members, before.members, "control: O1");
    assert_eq!(node.maintainer_rewind_count, 0, "control: O8");
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 0,
        "control: O9 — a clean range must not raise the security-graded anchor, or the \
         fleet-wide grep decays into alarm fatigue"
    );
}

/// IP-RO x P11. O1 O5 O8 O9. **GREEN — coverage, not a fix.**
///
/// The restore succeeds in memory and `ms.save()` fails. The in-memory value must be put
/// BACK so memory and disk still agree: leaving them divergent replaces one silent
/// inconsistency with a worse one that no restart can resolve (`Node::new` re-reads the
/// file, and the updater reads it to decide who may authorize a root install).
///
/// The failure is forced by pointing `config.data_dir` at a path that does not exist, which
/// makes `create_owner_only` on the temp file fail. Nothing else on the rollback path reads
/// `config.data_dir` — `block_store` and `state_db` are already-open handles.
#[tokio::test]
async fn req_174_005_a_failed_persist_is_rolled_back_counted_and_announced() {
    let (mut node, producers, tmp) = seeded_node(5).await;
    let (_r0, _r1, r2) = two_removals(&mut node, &producers).await;

    node.config.data_dir = tmp.path().join("no-such-dir").join("deeper");
    assert!(
        !node.config.data_dir.exists(),
        "harness: the persist target must be unwritable, or this asserts nothing"
    );

    assert!(node.rollback_one_block().await.expect("rollback errored"));

    assert_eq!(
        root(&node).await.members,
        r2.members,
        "IP-RO: O1 — the in-memory root must be put BACK to the pre-restore value. Memory \
         and disk disagreeing is the one outcome no restart resolves."
    );
    assert_eq!(
        MaintainerState::load(tmp.path())
            .expect("O5: the persisted root must still decode")
            .set
            .members,
        r2.members,
        "IP-RO: O5 — and the on-disk value is the one memory was put back to"
    );
    assert_eq!(
        node.maintainer_rewind_count, 0,
        "IP-RO: O8 — an unpersisted restore is not a restore"
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 1,
        "IP-RO: O9 — REQ-174-005 AC-3, and this is the loudest case: the rewind got as far \
         as knowing the right answer and could not keep it."
    );
}
