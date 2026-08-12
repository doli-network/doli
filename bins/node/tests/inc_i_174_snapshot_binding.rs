//! INC-I-174 M1 / AUDIT-P1-001 — a `cf_undo` maintainer record is authority for a BLOCK,
//! never for a HEIGHT.
//!
//! Fourth sibling of `inc_i_174_maintainer_undo.rs` (the `rollback_one_block` half),
//! `inc_i_174_maintainer_reorg.rs` (the `execute_reorg` half) and
//! `inc_i_174_maintainer_rewind_guards.rs` (the refusal branches). Split from the reorg
//! file only for the 800-line test-file budget — that file is at 762 lines and this
//! partition needs a harness the others do not have (`put_block_canonical`). Read the
//! first file's header for the full defect statement.
//!
//! covers: maintainer_rewind/ binding.rs mod.rs commit.rs types.rs undo.rs writes.rs
//!
//! WHY THIS FILE EXISTS
//! --------------------
//! The five-lens M1 security audit converged 5/5 on ONE structural property
//! (`AUDIT-P1-001`, SYS-001): the record was authorized by its HEIGHT and by the mere
//! presence of a rotation-typed tx at that height, never bound to the block it describes.
//!
//! `req_174_003_a_fossil_snapshot_from_an_abandoned_branch_is_never_the_trust_root` (the
//! reorg file) covers the half where the replacement block carries NO rotation — there the
//! read-side cross-check `if !block_mutates_maintainer_set(&block) { continue; }` catches
//! the fossil. It does NOT cover the half where the replacement block DOES carry one: the
//! cross-check then passes and the fossil installs through the SUCCESS exit.
//!
//! That second half is reachable with no attacker and no data-dir write, by a LEGITIMATE
//! operator recovery. `BlockStore::put_block_canonical` (`block_store/writes.rs:78-91`)
//! writes only `CF_HEIGHT_INDEX` + `CF_HASH_TO_HEIGHT`; it does not run `apply_block` and
//! never touches the 9-byte `cf_undo` family. Four production paths call it —
//! `backfillFromPeer` (an ONLINE RPC, `rpc/methods/backfill.rs:418`), `doli-node restore`
//! (`operations/restore.rs:355`), the archiver (`storage/archiver.rs:439`) and
//! `rebuild_canonical_index` (`block_store/writes.rs:254`). `plan_maintainer_rewind` reads
//! its block through `get_block_by_height` → the exact index those writers rewrote, so the
//! cross-check consults the WRONG block. INC-I-143 was a fleet-wide snap-sync/backfill
//! cascade, so this is a routine action on this fleet, not an exotic one.
//!
//! The record installed is this host's own former set — under INC-I-175, the five bootstrap
//! keys whose PRIVATE halves have been public on GitHub for ~149 days. It decides which
//! binary the auto-updater installs. And it installs through `info!` +
//! `maintainer_rewind_count += 1`: the operator is told SUCCESS.
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTION UNDER TEST
//!   `Node::plan_maintainer_rewind` -> `maintainer_rewind::binding::check_snapshot_binding`,
//!   observed through the real `Node::execute_reorg` caller (both are `pub(super)`, so an
//!   integration test reaches them only through a rewind). The exact `reason=` TOKEN is
//!   pinned separately by the unit tests on the pure predicate in
//!   `bins/node/src/node/maintainer_rewind/binding.rs`, which can name it directly;
//!   `bins/node` has no tracing-capture dev-dependency and adding one would edit a
//!   non-test manifest.
//!
//! OBSERVABLE OUTPUTS (Rust rules: `&mut self` receiver + persistent stores)
//!   O1 `self.maintainer_state.set.members`         — WHO may sign a release
//!   O2 `self.maintainer_state.set.threshold`       — HOW MANY must sign
//!   O3 `self.maintainer_state.set.last_updated`    — `getMaintainerSet.last_change_block`
//!   O5 `maintainer_state.bin` via `MaintainerState::load` (rule AQ-5)
//!   O6 `maintainer_set_digest(set, genesis_hash)`  — the fleet divergence instrument
//!   O7 `self.chain_state.best_height`              — the reorg must actually have
//!      happened, or every root assertion is satisfied vacuously by a refused reorg
//!   O8 `self.maintainer_rewind_count`              — the SUCCESS counter (REQ-174-010)
//!   O9 `self.maintainer_rewind_unrestored_count`   — the REFUSAL counter (REQ-174-005)
//!
//! CODE PATHS
//!   P12 `binding.rs` block-hash mismatch  -> `reason::SNAPSHOT_BLOCK_MISMATCH`
//!   P13 `binding.rs` all three bindings hold -> `Restore` (the control)
//!
//! INPUT PARTITIONS
//!   IP-BYPASS  the block at `h` was replaced OUT OF BAND by `put_block_canonical` with a
//!              DIFFERENT block that ALSO carries a rotation, while the record below it
//!              still describes the original. This is the ONLY partition that separates
//!              "the height carries a rotation, therefore this record applies" from "this
//!              record was captured for THIS block": the tx-type cross-check passes on
//!              both, and only the `block_hash` field disagrees.
//!   IP-SAME    the SAME block re-installed at `h` through the SAME out-of-band writer.
//!              Differs from IP-BYPASS in exactly ONE input — the identity of the block
//!              written — so a fix that simply refused every record after any
//!              `put_block_canonical` call, or refused every restore outright, fails here.
//!
//! MATRIX
//!   IP-BYPASS : O1 O2 O3 O5 O6 O7 O8 O9
//!   IP-SAME   : O1 O2 O3 O5 O7 O8 O9
//!
//! ANTI-VACUITY
//!   IP-SAME is IP-BYPASS's control and must stay GREEN both before and after the fix: it
//!   is the regression guard for the restore path the milestone exists to deliver. The
//!   `harness/precondition` assertions inside IP-BYPASS pin that the record really is
//!   still present and really does describe the abandoned block, so the refusal cannot be
//!   reached for an unrelated reason (a pruned record, an unreadable block).

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
// HARNESS — same shape as the three sibling files (integration test targets are
// separate crates, so a shared `mod` would put harness code outside `tests/`).
// ===========================================================================

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

fn digest(node: &Node, set: &MaintainerSet) -> String {
    hex::encode(maintainer_set_digest(
        set,
        node.params.genesis_hash.as_bytes(),
    ))
}

fn on_disk(dir: &TempDir) -> MaintainerState {
    MaintainerState::load(dir.path()).expect("O5: the persisted trust root must still decode")
}

fn assert_root_eq(actual: &MaintainerSet, expected: &MaintainerSet, ctx: &str) {
    assert_eq!(
        actual.members, expected.members,
        "{ctx}: O1 — member list. These keys are WHO may authorize a root binary install \
         on this host via the auto-updater."
    );
    assert_eq!(
        actual.threshold, expected.threshold,
        "{ctx}: O2 — threshold"
    );
    assert_eq!(
        actual.last_updated, expected.last_updated,
        "{ctx}: O3 — `getMaintainerSet.last_change_block`, the field that says WHICH BLOCK \
         this root came from"
    );
}

/// Drive the real `execute_reorg` over `rollback_depth` blocks, replacing them with one
/// coinbase-only block that builds on the common ancestor. Verbatim from the reorg file.
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

/// Build h=1..=2 plain, h=3 carrying a `RemoveMaintainer`, h=4..=5 plain. Returns
/// `(R0 before the rotation, the block applied at h=3)`.
async fn chain_with_rotation_at_3(
    node: &mut Node,
    producers: &[KeyPair],
) -> (MaintainerSet, Block) {
    let params = node.params.clone();
    let r0 = root(node).await;
    assert_eq!(
        r0.members.len(),
        5,
        "harness: R0 is the seeded 5-member root"
    );

    let mut prev = Hash::ZERO;
    for h in 1..=2u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(node, &b).await;
    }

    let victim = r0.members[4];
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim)
        .take(3)
        .collect();
    let rot = build_block(
        3,
        3,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(false, &victim, &signers)],
    );
    prev = rot.hash();
    apply(node, &rot).await;
    assert_eq!(
        root(node).await.members.len(),
        4,
        "harness: the rotation at h=3 applied (R0 -> R1)"
    );

    for h in 4..=5u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(node, &b).await;
    }
    assert_eq!(node.chain_state.read().await.best_height, 5, "harness");

    (r0, rot)
}

// ===========================================================================
// AUDIT-P1-001 — the record must be bound to its block, not to its height.
// ===========================================================================

/// IP-BYPASS x P12. O1 O2 O3 O5 O6 O7 O8 O9. **RED — assertion.**
///
/// A canonical-index bypass writer replaces the block at h=3 with a DIFFERENT block that
/// also carries a rotation, leaving the `cf_undo` record captured for the original in
/// place. A rewind across h=3 must REFUSE it: the record is authority for the block it was
/// captured from, and that block is no longer at that height.
///
/// On the pre-fix code the tx-type cross-check passes (the replacement carries an
/// `AddMaintainer`), the record is promoted to `Restore`, `validate_persisted_set` accepts
/// it (it is a well-formed former set), and the host's release-verification trust root is
/// set to a member list that exists on NO canonical chain — with `maintainer_rewind_count`
/// incremented and `MAINTAINER_REWIND_RESTORED` logged at `info!`.
#[tokio::test]
async fn audit_p1_001_a_record_captured_for_another_block_is_never_the_trust_root() {
    let (mut node, producers, tmp) = seeded_node(5).await;
    let params = node.params.clone();

    let (r0, rot) = chain_with_rotation_at_3(&mut node, &producers).await;
    let live = root(&node).await;
    let live_digest = digest(&node, &live);
    assert_eq!(live.members.len(), 4, "harness: the live root is R1");

    // The record at h=3 describes `rot` and holds R0 — the pre-rotation five.
    let record = node.state_db.get_maintainer_undo(3).expect(
        "harness/precondition: h=3 carries a rotation, so apply_block must have captured a \
         snapshot for it",
    );
    assert_eq!(
        record.block_hash,
        rot.hash(),
        "harness/precondition: the record must be stamped with the hash of the block it \
         was captured for, or this test cannot distinguish the two blocks at all"
    );
    assert_eq!(
        record.set.members.len(),
        5,
        "harness/precondition: the record holds R0, the set that will exist on NO chain \
         once the block below it is replaced"
    );

    // --- The bypass. A DIFFERENT block at the SAME height, also carrying a rotation. ---
    //
    // `put_block_canonical` is the exact primitive `backfillFromPeer`, `doli-node restore`,
    // the archiver and `rebuild_canonical_index` use. It rewrites CF_HEIGHT_INDEX +
    // CF_HASH_TO_HEIGHT and NOTHING else: no `apply_block`, no maintainer refresh. Calling
    // it directly is not a shortcut around production code — it IS the production code
    // those four paths run.
    let newcomer = KeyPair::generate();
    let add_signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let imposter = build_block(
        3,
        777,
        rot.header.prev_hash,
        &producers[1],
        &params,
        vec![maintainer_tx(true, newcomer.public_key(), &add_signers)],
    );
    assert_ne!(
        imposter.hash(),
        rot.hash(),
        "harness: the two blocks must actually differ"
    );
    node.block_store
        .put_block_canonical(&imposter, 3)
        .expect("block_store write");
    let seen = node
        .block_store
        .get_block_by_height(3)
        .expect("block_store read")
        .expect("harness: h=3 must resolve");
    assert_eq!(
        seen.hash(),
        imposter.hash(),
        "harness/precondition: the rewind path reads h=3 through CF_HEIGHT_INDEX, so the \
         bypass must actually have re-pointed it at the imposter"
    );
    assert!(
        node.state_db.get_maintainer_undo(3).is_some(),
        "harness/precondition: and `put_block_canonical` must NOT have refreshed or \
         removed the record — that omission is the whole defect"
    );

    // --- The rewind across h=3 --------------------------------------------------------
    reorg_away(&mut node, 3, &producers[2]).await;
    assert_eq!(
        node.chain_state.read().await.best_height,
        3,
        "O7: 5 - 3 rolled back + 1 applied = 3"
    );

    let after = root(&node).await;
    assert_root_eq(&after, &live, "IP-BYPASS");
    assert_ne!(
        after.members, r0.members,
        "IP-BYPASS: O1 — the record at h=3 was captured for a block that is no longer at \
         h=3, so it is NOT authority for this rewind. Installing it makes this host's \
         release-verification trust root a member list that exists on no canonical chain \
         — under INC-I-175, one still holding the five bootstrap keys whose private halves \
         are public on GitHub. Nothing but `block_hash` can tell the two blocks apart: \
         both carry a rotation, so the tx-type cross-check passes on both."
    );
    assert_eq!(
        after.members.len(),
        4,
        "IP-BYPASS: O1 — the LIVE root is kept. 5 members means the fossil installed."
    );
    assert_eq!(digest(&node, &after), live_digest, "IP-BYPASS: O6");
    assert_root_eq(&on_disk(&tmp).set, &live, "IP-BYPASS/disk");
    assert_eq!(
        node.maintainer_rewind_count, 0,
        "IP-BYPASS: O8 — and it must not be reported as a RESTORE. A wrong restore is \
         announced through the `info!` success exit, so the operator's only fleet-wide \
         signal points the wrong way — strictly worse than the pre-INC-I-174 behaviour, \
         which was at least stale-and-explicable."
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 1,
        "IP-BYPASS: O9 — REQ-174-005 AC-3, no silent route. The rewind crossed a height \
         that provably carried a rotation and could not restore it, so it must say so \
         through the counter and the MAINTAINER_REWIND_UNRESTORED anchor. The `reason=` \
         token is `snapshot_block_mismatch`, pinned by the binding unit tests."
    );
}

/// IP-SAME x P13. O1 O2 O3 O5 O7 O8 O9. **GREEN — control, must stay green.**
///
/// The SAME block re-installed at h=3 through the SAME out-of-band writer. Differs from
/// IP-BYPASS in exactly ONE input: the identity of the block written. The record still
/// describes it, so the restore must go through unchanged.
///
/// Without this control, a "fix" that refused every restore, or refused any record whose
/// height was ever touched by `put_block_canonical`, would satisfy IP-BYPASS while
/// deleting the milestone's entire deliverable.
#[tokio::test]
async fn audit_p1_001_control_a_record_that_still_describes_its_block_still_restores() {
    let (mut node, producers, tmp) = seeded_node(5).await;

    let (r0, rot) = chain_with_rotation_at_3(&mut node, &producers).await;

    // Re-install the SAME block through the same bypass writer — an idempotent
    // `backfillFromPeer` / archiver import of a height the node already holds.
    node.block_store
        .put_block_canonical(&rot, 3)
        .expect("block_store write");
    assert_eq!(
        node.block_store
            .get_block_by_height(3)
            .expect("block_store read")
            .expect("harness: h=3 must resolve")
            .hash(),
        rot.hash(),
        "harness: h=3 still resolves to the original block"
    );

    reorg_away(&mut node, 3, &producers[2]).await;
    assert_eq!(node.chain_state.read().await.best_height, 3, "O7");

    let after = root(&node).await;
    assert_root_eq(&after, &r0, "IP-SAME");
    assert_eq!(
        after.members.len(),
        5,
        "IP-SAME: O1 — the record still describes the block at h=3, so the pre-rotation \
         root must be restored. This is REQ-174-003, the milestone's deliverable: if the \
         AUDIT-P1-001 binding refuses here it has broken the fix it was added to protect."
    );
    assert_root_eq(&on_disk(&tmp).set, &r0, "IP-SAME/disk");
    assert_eq!(
        node.maintainer_rewind_count, 1,
        "IP-SAME: O8 — exactly one restore, reported as one"
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 0,
        "IP-SAME: O9 — and no false alarm on the security-graded anchor"
    );
}
