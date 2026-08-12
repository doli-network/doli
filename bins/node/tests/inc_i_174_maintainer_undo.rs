//! INC-I-174 M1 — a reorged-out maintainer rotation is never undone.
//!
//! THE DEFECT (root cause already confirmed; this file only reproduces its SYMPTOMS).
//! `UndoData` (`crates/storage/src/state_db/types.rs`) has five fields and no maintainer
//! field. `rollback_one_block` (`bins/node/src/node/rollback.rs`) and `execute_reorg`
//! (`bins/node/src/node/block_handling.rs`) contain zero maintainer handling. The apply
//! path (`bins/node/src/node/apply_block/governance.rs`) mutates the node-local maintainer
//! set and persists it with `ms.save()` — an atomic file rename OUTSIDE the RocksDB
//! WriteBatch. So when a reorg drops the block that carried an `AddMaintainer` /
//! `RemoveMaintainer`, this node keeps the rotation forever, in memory AND on disk, while
//! the canonical chain does not have it. That set is the auto-update install trust root.
//!
//! Above `maintainer_derivation_activation_height` there is no self-heal: the one-shot
//! seed in `bins/node/src/node/periodic.rs` never re-fires once `members` is non-empty.
//!
//! covers: types.rs undo.rs batch.rs mod.rs governance.rs rollback.rs block_handling.rs
//! covers: maintainer.rs maintainer_wellformed.rs set.rs digest.rs periodic.rs
//! covers: maintainer_rewind/ state_update.rs helpers.rs data.rs derivation.rs
//!
//! Every test in this file COMPILES against today's API and fails on an ASSERTION.
//!
//! SIBLING FILES (split only for the 800-line test-file budget):
//!   * `inc_i_174_maintainer_reorg.rs` — the `execute_reorg` half (REQ-174-003/007).
//!     `execute_reorg` runs its OWN rollback loop; INC-I-040 is the precedent for a fix
//!     that lands in `rollback.rs` alone and leaves the reorg path broken.
//!   * `inc_i_174_maintainer_undo_capture.rs` — the capture/security half, a deliberate
//!     COMPILE-red for API that does not exist yet, kept separate so THESE reds still run.
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTION UNDER TEST
//!   R1: `Node::rollback_one_block(&mut self) -> Result<bool>`
//!   (observed after `Node::apply_block` has applied a block carrying a governance tx,
//!    i.e. through the real `process_transaction_governance` site)
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS (Rust rules: `&mut self` receiver + stores)
//!   O1  receiver mutation  `self.maintainer_state.set.members`      — WHO may sign a release
//!   O2  receiver mutation  `self.maintainer_state.set.threshold`    — HOW MANY must sign
//!   O3  receiver mutation  `self.maintainer_state.set.last_updated` — the chain height the
//!                          root claims; `getMaintainerSet.last_change_block` (REQ-174-007)
//!   O4  receiver mutation  `self.maintainer_state.last_derived_height` — the SEED ARM.
//!                          Zeroing it re-arms the one-shot bootstrap, which re-derives
//!                          the root from LIVE producer state and RE-ARMS REMOVED KEYS
//!                          (INC-I-172 R1). A restore that "restores" this to 0 is worse
//!                          than no restore at all.
//!   O5  persistent store   `maintainer_state.bin` in the data dir — read back
//!                          independently with `MaintainerState::load` (rule AQ-5).
//!                          A restore in memory only is undone by the next restart.
//!   O6  derived value      `maintainer_set_digest(set, genesis_hash)` — the fleet-wide
//!                          divergence instrument published on the `MAINTAINER_SET_DIGEST`
//!                          grep anchor by the apply path.
//!   O7  receiver mutation  `self.chain_state.best_height` / `best_hash` — the rewind
//!                          itself must still happen; a "fix" that refuses the rollback
//!                          would satisfy O1-O6 vacuously.
//!   O8  return value       `Result<bool>` — a refused rewind is not a restored one.
//!   (Not asserted here: the `info!` log line. `bins/node` has no tracing-capture
//!    dev-dependency and adding one would edit a non-test manifest. REQ-174-008 is
//!    therefore asserted through O6 — the VALUE the anchor must carry — and the grep
//!    anchor itself is left to QA. Stated, not silently skipped.)
//!
//! CODE PATHS
//!   P1  undo-based branch, the rolled-back block carried an `AddMaintainer`
//!   P2  undo-based branch, the rolled-back block carried a `RemoveMaintainer`
//!   P3  undo-based branch, the rolled-back block carried NO governance tx
//!   (`execute_reorg`'s independent loop — P4/P5/P6 — is in `inc_i_174_maintainer_reorg.rs`)
//!
//! INPUT PARTITIONS: (input classes that change the relationship between the asserted
//! quantities, not merely the branch taken)
//!   IP-A  add at the rewind TIP    (P1) — members 4 -> 5, threshold 3 -> 3.
//!         The threshold is UNCHANGED across 4->5, so on this partition a threshold
//!         assertion cannot fail. That is precisely why IP-R exists.
//!   IP-R  remove at the rewind TIP (P2) — members 5 -> 4, and the REMOVED KEY must be
//!         back. This is the partition where "the set has the right LENGTH" and "the set
//!         has the right MEMBERS" come apart: a restore that rebuilds a 5-member set from
//!         live producer state satisfies the count and fails the identity.
//!   IP-N  no governance tx         (P3) — every field byte-identical, including the
//!         on-disk bytes. The anti-vacuity control: a fix that rewrites the root
//!         unconditionally passes IP-A and IP-R and FAILS here.
//!
//! MATRIX  (8 outputs x 3 partitions)
//!   IP-A : O1 O2 O3 O4 O5 O6 O7 O8
//!   IP-R : O1 O2 O3 O4 O5 O6 O7 O8
//!   IP-N : O1 O2 O3 O4 O5 O6 O7 O8   (all "unchanged" — the control)
//!
//! ANTI-VACUITY
//!   IP-N is byte-identical harness to IP-A except the block carries no governance tx.
//!   IP-N is GREEN today. A green IP-A can therefore never come from a harness that
//!   rewrites the root unconditionally.

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{
    maintainer_set_digest, MaintainerChangeData, MaintainerSignature, MIN_MAINTAINERS,
};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, MaintainerSet, Transaction};
use doli_node::node::Node;
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

/// A devnet node with `n` genesis producers and an ATTACHED BUT UNSEEDED trust root.
///
/// This is the state every fresh node — and every node whose `maintainer_state.bin` was
/// deleted — starts from: `MaintainerState::default()`, i.e. `members == []` and
/// `last_derived_height == 0`. It is the input class that makes IP-E reachable with no
/// attacker and no hand-written undo record.
async fn unseeded_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
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

/// The mirror of the private `Node::maintainer_seed_is_done(state, one_shot = true)`
/// (`bins/node/src/node/periodic.rs`). Duplicated on purpose: it is private, and this is
/// the external contract a restore must not break.
fn seed_is_done(state: &MaintainerState) -> bool {
    !state.set.members.is_empty() || state.last_derived_height != 0
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
// The headline symptom: reorg-drops-rotation.
// ===========================================================================

/// REQ-174-002, IP-A x P1. O1 O2 O3 O4 O5 O6 O7 O8. **RED — assertion.**
///
/// Acceptance (§12 REQ-174-002 bullet 1): given a node that applied an `AddMaintainer` at
/// height H, when it rolls back to H-1 and the rotation is not re-applied, then the set has
/// exactly the members it had at H-1, the same threshold, and `last_updated` back at its
/// H-1 value.
#[tokio::test]
async fn req_174_002_rollback_undoes_an_add_maintainer_rotation() {
    // 4 members so the ADD lands inside MAX_MAINTAINERS (5).
    let (mut node, producers, tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let before_derived = derived_height(&node).await;
    let before_digest = digest(&node, &before);
    assert_eq!(before.members.len(), 4, "harness: 4 seated members");
    assert_eq!(before.threshold, 3, "harness: calculate_threshold(4) == 3");
    assert_eq!(before.last_updated, 0, "harness: seeded at height 0");

    // h=1..=3 — ordinary coinbase-only blocks.
    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    // h=4 — the ROTATION. Signed by 3 distinct seated members (threshold 3).
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let rot = build_block(4, 4, prev, &producers[0], &params, vec![tx]);
    apply(&mut node, &rot).await;

    let after = root(&node).await;
    assert_eq!(
        after.members.len(),
        5,
        "harness: the rotation must actually have applied, or the rollback under test \
         has nothing to undo and every assertion below is vacuous"
    );
    assert_eq!(after.last_updated, 4, "harness: rotation stamped at h=4");

    // ---- the rewind ----
    let rolled = node.rollback_one_block().await.expect("rollback errored");
    assert!(rolled, "O8: the rollback must have happened");
    assert_eq!(
        node.chain_state.read().await.best_height,
        3,
        "O7: the chain itself must be back at h=3 — a refused rewind would satisfy the \
         root assertions vacuously"
    );

    let restored = root(&node).await;
    assert_root_eq(&restored, &before, "IP-A");
    assert!(
        !restored.members.contains(newcomer.public_key()),
        "IP-A: O1 — the key added by the reorged-out block must NOT still be able to \
         authorize a binary install on this host"
    );
    // O6 — the digest an operator greps must match a node that never saw the block.
    assert_eq!(
        digest(&node, &restored),
        before_digest,
        "IP-A: O6 — MAINTAINER_SET_DIGEST must be identical to a node that never applied \
         the rolled-back block, or the fleet-wide grep reports a divergence that no \
         canonical block explains"
    );
    // O4 — the seed arm must NOT be reset to 0 (INC-I-172 R1 re-arm hazard).
    assert_eq!(
        derived_height(&node).await,
        before_derived,
        "IP-A: O4 — last_derived_height must be restored to its pre-block value"
    );
    // O5 — persistence parity, read back independently.
    let disk = on_disk(&tmp);
    assert_root_eq(&disk.set, &before, "IP-A/disk");
}

/// REQ-174-002, IP-R x P2. O1 O2 O3 O4 O5 O6 O7 O8. **RED — assertion.**
///
/// Acceptance (§12 REQ-174-002 bullet 2): same for a `RemoveMaintainer` at H — the removed
/// member is BACK. This is the partition where member COUNT and member IDENTITY diverge:
/// a fix that restores "some 5-member set" passes a length check and fails this.
#[tokio::test]
async fn req_174_002_rollback_undoes_a_remove_maintainer_rotation() {
    // 5 members: `can_remove()` needs len > MIN_MAINTAINERS (3).
    let (mut node, producers, tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let before = root(&node).await;
    // Harness sanity, asserted on the SEEDED set rather than on the literal 5:
    // `can_remove()` requires `len > MIN_MAINTAINERS`, so if the seed ever produced a
    // shorter set the removal below would be refused and this test would go green
    // without ever exercising a rollback.
    assert!(
        before.members.len() > MIN_MAINTAINERS,
        "harness sanity: the removal must be legal — {} seated members must exceed \
         MIN_MAINTAINERS {}",
        before.members.len(),
        MIN_MAINTAINERS
    );
    let before_digest = digest(&node, &before);
    let victim = before.members[4];

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    // Signers must EXCLUDE the target (`verify_multisig_excluding_at`).
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim)
        .take(3)
        .collect();
    let tx = maintainer_tx(false, &victim, &signers);
    let rot = build_block(4, 4, prev, &producers[0], &params, vec![tx]);
    apply(&mut node, &rot).await;

    let after = root(&node).await;
    assert_eq!(
        after.members.len(),
        4,
        "harness: the removal must actually have applied"
    );
    assert!(!after.members.contains(&victim), "harness: victim removed");

    // ---- the rewind ----
    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 3, "O7");

    let restored = root(&node).await;
    assert!(
        restored.members.contains(&victim),
        "IP-R: O1 — the member removed by the reorged-out block must be BACK. Left \
         removed, this host refuses a release the canonical chain's quorum signed, and \
         the divergence is silent: nothing re-derives the root above the one-shot seed."
    );
    assert_root_eq(&restored, &before, "IP-R");
    assert_eq!(digest(&node, &restored), before_digest, "IP-R: O6");
    let disk = on_disk(&tmp);
    assert_root_eq(&disk.set, &before, "IP-R/disk");
}

/// REQ-174-002 bullet 3, IP-N x P3. **GREEN today — the anti-vacuity control.**
///
/// Byte-identical harness to `req_174_002_rollback_undoes_an_add_maintainer_rotation`
/// except the rolled-back block carries NO governance tx. It passes before the fix, so a
/// green IP-A afterwards cannot come from a restore that fires unconditionally.
#[tokio::test]
async fn req_174_002_rollback_across_a_block_with_no_rotation_is_a_no_op() {
    let (mut node, producers, tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let mut prev = Hash::ZERO;
    for h in 1..=4u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }

    let before = root(&node).await;
    let before_derived = derived_height(&node).await;
    let before_digest = digest(&node, &before);
    let before_disk = std::fs::read(tmp.path().join("maintainer_state.bin")).ok();

    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(node.chain_state.read().await.best_height, 3, "O7");

    let after = root(&node).await;
    assert_root_eq(&after, &before, "IP-N");
    assert_eq!(derived_height(&node).await, before_derived, "IP-N: O4");
    assert_eq!(digest(&node, &after), before_digest, "IP-N: O6");
    assert_eq!(
        std::fs::read(tmp.path().join("maintainer_state.bin")).ok(),
        before_disk,
        "IP-N: O5 — rolling back a block that changed nothing must not rewrite the trust \
         root file. A restore that fires on every block turns a 100-block reorg into 100 \
         durable writes of the install authority."
    );
}

/// REQ-174-002 AC-4, IP-A x P1, O4. **GREEN today — a lock, not a red.**
///
/// After the restore, `maintainer_seed_is_done(&state, one_shot = true)` must still be
/// TRUE. A restore that zeroes `last_derived_height` (or empties `members`) re-arms the
/// one-shot bootstrap in `periodic.rs`, which `state_update.rs` calls on EVERY applied
/// block — so the very next block re-derives the root from LIVE producer state and
/// RE-ARMS ANY KEY GOVERNANCE REMOVED. That is the INC-I-172 R1 hazard, and it is
/// strictly worse than the bug this milestone fixes.
#[tokio::test]
async fn req_174_002_ac4_a_restore_must_not_re_arm_the_one_shot_seed() {
    let (mut node, producers, _tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let rot = build_block(4, 4, prev, &producers[0], &params, vec![tx]);
    apply(&mut node, &rot).await;

    assert!(node.rollback_one_block().await.expect("rollback errored"));

    {
        let state = node.maintainer_state.as_ref().unwrap().read().await;
        assert!(
            seed_is_done(&state),
            "AC-4: O4 — after the restore the one-shot seed must still read as DONE. \
             `!members.is_empty() || last_derived_height != 0` is the predicate in \
             periodic.rs; a restore that fails it re-derives the trust root from live \
             producer state on the next block and re-arms removed keys (INC-I-172 R1)."
        );
    }

    // Behavioural half of the same claim: driving the seed explicitly must be a no-op.
    let pinned = root(&node).await;
    node.maybe_bootstrap_maintainer_set(4).await;
    assert_root_eq(&root(&node).await, &pinned, "AC-4/re-seed");
}

/// REQ-174-002 AC-4 + REQ-174-SEC-001 AC-3, IP-E x P1. O1 O4 O5 O9 O10.
/// **RED before the QA-fix-1 guard — the empty set WAS installed.**
///
/// IP-E — the undo snapshot IS `{members: [], last_derived_height: 0}`.
///
/// Reachable with no attacker and no hand-written record. `capture_maintainer_undo` keys
/// on the transaction TYPE before verification (deliberate: the rollback path cannot
/// re-derive a verification verdict against a set that has already moved), so a node whose
/// root is still `MaintainerState::default()` records `{[], 0}` for the block that carries
/// a rotation. The one-shot seed then fires at the END of that same block
/// (`apply_block/state_update.rs` → `maybe_bootstrap_maintainer_set`), leaving a real root
/// above a snapshot of nothing.
///
/// `validate_persisted_set` CANNOT catch this: it carves the empty set out on purpose
/// (`crates/storage/src/maintainer_wellformed.rs`), because an unseeded node must stay
/// loadable and an emptied on-chain root must fail closed rather than become unbootable.
/// The carve-out is correct for the LOAD path and wrong for the REWIND path, so the guard
/// this test pins must live in `commit_maintainer_rewind`, not in the shared gate — if it
/// moved into the shared gate, `MaintainerState::load` would refuse every fresh node.
///
/// Installing `{[], 0}` satisfies `members.is_empty() && last_derived_height == 0`, which
/// `maintainer_seed_is_done` reads as "never seeded". The seed is driven on EVERY applied
/// block, so the next block re-derives the trust root from LIVE producer state and re-arms
/// any key governance removed — the INC-I-172 R1 hazard, strictly worse than the bug this
/// milestone fixes. AC-4 is absolute, so the refusal must also be LOUD (O9/O10).
#[tokio::test]
async fn req_174_sec_001_an_empty_undo_snapshot_is_refused_and_announced() {
    let (mut node, producers, tmp) = unseeded_node(4).await;
    let params = node.params.clone();

    assert!(
        root(&node).await.members.is_empty() && derived_height(&node).await == 0,
        "harness: IP-E requires the root to still be `MaintainerState::default()` when the \
         rotation block is applied — that is what makes the captured snapshot empty"
    );
    let before_unrestored = node.maintainer_rewind_unrestored_count;
    let before_restored = node.maintainer_rewind_count;

    // h=1 carries a rotation. Its signatures cannot verify against an empty set, so the
    // rotation itself is rejected — but the snapshot is captured on the TYPE, before that.
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let rot = build_block(1, 1, Hash::ZERO, &producers[0], &params, vec![tx]);
    apply(&mut node, &rot).await;

    // The one-shot seed fired inside h=1, so the live root is now real.
    let live = root(&node).await;
    assert_eq!(
        live.members.len(),
        4,
        "harness: the one-shot seed must have seated the genesis producers inside h=1, or \
         there is no real root for the empty restore to destroy and the test is vacuous"
    );
    let live_derived = derived_height(&node).await;
    assert_ne!(
        live_derived, 0,
        "harness: the seed armed last_derived_height"
    );

    // ---- the rewind ----
    assert!(node.rollback_one_block().await.expect("rollback errored"));
    assert_eq!(
        node.chain_state.read().await.best_height,
        0,
        "O7: the chain itself must still rewind — a refused REWIND would satisfy the root \
         assertions vacuously. Only the trust-root RESTORE is refused."
    );

    let after = root(&node).await;
    assert_root_eq(&after, &live, "IP-E");
    assert!(
        !after.members.is_empty(),
        "IP-E: O1 — the rewind must NEVER install an empty member list. Empty + \
         last_derived_height 0 is exactly what `maintainer_seed_is_done` reads as \
         'never seeded', so the next applied block re-derives the root from LIVE producer \
         state and re-arms any key governance removed (INC-I-172 R1)."
    );
    assert_eq!(
        derived_height(&node).await,
        live_derived,
        "IP-E: O4 — the seed arm must survive a refused restore"
    );
    {
        let state = node.maintainer_state.as_ref().unwrap().read().await;
        assert!(
            seed_is_done(&state),
            "IP-E: O4 — after a refused restore the one-shot seed must still read as DONE"
        );
    }
    // O5 — the refusal must not have degraded the file either.
    assert!(
        !on_disk(&tmp).set.members.is_empty(),
        "IP-E: O5 — a refused restore must leave the persisted root intact. Degrading the \
         FILE to an empty set survives the restart that would otherwise heal memory."
    );

    // O9/O10 — fail closed AND loud. REQ-174-SEC-001 AC-3: a refusal counted as a success
    // (`unrestored == 0`) gives the operator no signal from the rewind at all.
    assert_eq!(
        node.maintainer_rewind_unrestored_count,
        before_unrestored + 1,
        "IP-E: O10 — a refused restore must increment \
         `maintainer_rewind_unrestored_count` and emit MAINTAINER_REWIND_UNRESTORED. \
         REQ-174-005 AC-3 forbids a route by which the root survives a rewind un-restored \
         without announcing it."
    );
    assert_eq!(
        node.maintainer_rewind_count, before_restored,
        "IP-E: O9 — a refusal is NOT a restore and must not be counted as one"
    );
}

// ===========================================================================
// REQ-174-006 (Must) — the restored value survives a restart.
// ===========================================================================

/// REQ-174-006, IP-A x P1, O5. **RED — assertion.**
///
/// Acceptance: after a rollback past a rotation, `MaintainerState::load` from the data dir
/// returns the POST-rollback set — members, threshold, `last_updated` AND
/// `last_derived_height` — and the file still carries the `DMST` magic and
/// `MAINTAINER_STATE_VERSION` (no format regression, no version bump).
///
/// The load-bearing assertion is "disk == the H-1 set", not "disk == memory": before the
/// fix, disk and memory AGREE (both hold the un-rewound rotation), so a memory/disk parity
/// check alone is green for the wrong reason.
#[tokio::test]
async fn req_174_006_restart_after_rollback_reloads_the_rewound_set_from_disk() {
    let (mut node, producers, tmp) = seeded_node(4).await;
    let params = node.params.clone();

    let before = root(&node).await;
    let before_derived = derived_height(&node).await;

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let tx = maintainer_tx(true, newcomer.public_key(), &signers);
    let rot = build_block(4, 4, prev, &producers[0], &params, vec![tx]);
    apply(&mut node, &rot).await;
    assert_eq!(
        root(&node).await.members.len(),
        5,
        "harness: rotation applied"
    );

    assert!(node.rollback_one_block().await.expect("rollback errored"));

    // "Restart": re-read the file exactly as `Node::new` does.
    let reloaded = on_disk(&tmp);
    assert_root_eq(&reloaded.set, &before, "REQ-174-006/disk");
    assert!(
        !reloaded.set.members.contains(newcomer.public_key()),
        "REQ-174-006: O5 — an in-memory-only restore is undone by the next restart. The \
         updater reads this FILE to decide which keys may authorize a root install."
    );
    assert_eq!(
        reloaded.last_derived_height, before_derived,
        "REQ-174-006: O5 — last_derived_height must round-trip too, or the reloaded value \
         re-arms the one-shot seed"
    );

    // Memory/disk parity (weaker, but it is the property REQ-174-006 names).
    assert_root_eq(&reloaded.set, &root(&node).await, "REQ-174-006/parity");

    // No format regression: the header must be untouched by this work.
    let bytes = std::fs::read(tmp.path().join("maintainer_state.bin"))
        .expect("the trust-root file must exist after a restore that persists");
    assert_eq!(
        &bytes[..4],
        b"DMST",
        "REQ-174-006: the `DMST` magic must survive — a restore that writes a bare \
         bincode body recreates the pre-INC-I-172 ambiguity"
    );
    assert_eq!(
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        storage::MAINTAINER_STATE_VERSION,
        "REQ-174-006: MAINTAINER_STATE_VERSION must NOT move. The persisted BODY shape is \
         unchanged by this milestone; bumping it would fail-close every node on the \
         format-version branch of `MaintainerState::load`."
    );
}
