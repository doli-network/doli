//! INC-I-174 M1 — the CAPTURE side, the SECURITY gate, and the LOUDNESS contract.
//!
//! covers: types.rs undo.rs batch.rs mod.rs governance.rs rollback.rs block_handling.rs
//! covers: maintainer.rs maintainer_wellformed.rs set.rs digest.rs periodic.rs
//! covers: maintainer_rewind/ state_update.rs helpers.rs lib.rs init.rs
//!
//! ###########################################################################
//! WAS A DELIBERATE COMPILE-RED. The API below now exists; these tests must pass.
//! ###########################################################################
//!
//! Every other INC-I-174 test file compiles today and fails on an ASSERTION. These tests
//! could not: the state they must observe did not exist yet. They are kept in a SEPARATE
//! target so `cargo test -p doli-node --test inc_i_174_maintainer_undo` and
//! `cargo test -p storage --test inc_i_174_undo_schema` still build and still produce
//! measured red output.
//!
//! API — assumed when these tests were written, and PROVIDED by the fix. One
//! substitution was made (item 2); the observable is identical and it is recorded in
//! `docs/.workflow/inc-i-174-M1-dev-report.md`:
//!
//! 1. `storage::state_db::MaintainerUndoSnapshot`
//!    ```ignore
//!    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
//!    pub struct MaintainerUndoSnapshot {
//!        pub set: doli_core::MaintainerSet,
//!        pub last_derived_height: u64,
//!    }
//!    ```
//!    A TYPED field, not `Vec<u8>`. Bytes would need a second, separately-maintained
//!    encoder — the exact shape that produced the `epoch_state_snapshot` "backward compat"
//!    comment that `inc_i_174_undo_schema.rs` measures to be false.
//!
//! 2. SUBSTITUTED. Assumed:
//!    `storage::state_db::UndoData::maintainer_snapshot: Option<MaintainerUndoSnapshot>`.
//!    Provided instead: a SEPARATE `cf_undo` record under a distinct 9-byte key prefix,
//!    reached by `StateDb::{get,put,delete}_maintainer_undo(height)` and written into the
//!    same `WriteBatch` by `BlockBatch::put_maintainer_undo`.
//!
//!    The observable is the same in both directions — record ABSENT is the "unchanged at
//!    this height" sentinel and carries no set bytes; record PRESENT is the pre-block
//!    state — so every assertion below is unchanged in strength. The substitution is
//!    forced by this milestone's own measurements in
//!    `crates/storage/tests/inc_i_174_undo_schema.rs`: a sixth field on `UndoData` makes
//!    EVERY pre-upgrade `cf_undo` entry undecodable, which `get_undo` reports as `None`,
//!    which silently drops `rollback_one_block` into the rebuild-from-genesis fallback
//!    and closes `execute_reorg`'s gate for the whole rewind range. Keyed separately, no
//!    existing entry is disturbed and there is no migration window to announce.
//!
//! 3. `storage::validate_persisted_set(source: &std::path::Path, set: &MaintainerSet)
//!     -> Result<(), storage::StorageError>`
//!    The function `crates/storage/src/maintainer_wellformed.rs` already contains, made
//!    `pub` and re-exported from `crates/storage/src/lib.rs`. NOT a new copy — the whole
//!    point of REQ-174-SEC-001 is that the restore gate and the `MaintainerState::load`
//!    gate CANNOT DRIFT, and two functions drift.
//!
//! 4. `Node::maintainer_rewind_count: u64` — public field, like `cumulative_rollback_depth`
//!    and `shallow_rollback_count`. Incremented once per maintainer-state rewind that
//!    RESTORED a value.
//!
//! 5. `Node::maintainer_rewind_unrestored_count: u64` — public field. Incremented once per
//!    rewind that crossed a height whose maintainer state could NOT be restored (no undo
//!    entry, an undecodable one, or a snapshot the gate refused). This is the machine-
//!    checkable half of "no silent route exists"; the `MAINTAINER_SET_DIGEST` grep anchor
//!    is the human half and is left to QA, because `bins/node` has no tracing-capture
//!    dev-dependency and adding one would edit a non-test manifest.
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTIONS UNDER TEST
//!   C1 `Node::apply_block` — the CAPTURE half (what lands in `cf_undo`)
//!   C2 `Node::rollback_one_block` — the RESTORE half, on refusal and fallback inputs
//!   C3 `storage::validate_persisted_set` — the authority gate the restore must reuse
//!
//! OBSERVABLE OUTPUTS
//!   O1 `get_maintainer_undo(h)`              — presence/absence and CONTENT
//!   O2 `maintainer_state.set`                — members / threshold / last_updated
//!   O3 `maintainer_state.last_derived_height`— the one-shot seed arm
//!   O4 `maintainer_state.bin`                — the persisted trust root
//!   O5 `Node::maintainer_rewind_count`       — restores actually performed
//!   O6 `Node::maintainer_rewind_unrestored_count` — the loudness counter
//!   O7 return of C3                          — `Ok(())` vs the specific `StorageError`
//!
//! PATHS
//!   PC1 block with no governance tx                    -> O1 absent
//!   PC2 block with ONE successful rotation             -> O1 present, BEFORE state
//!   PC3 block with TWO rotations                       -> O1 = state before the FIRST
//!   PC4 block with a governance tx that FAILS verify   -> restore is a no-op
//!   PC5 epoch-boundary block with no rotation          -> O1 absent (no epoch term)
//!   PR1 restore of a snapshot the gate REFUSES         -> fail closed, fail loud
//!   PR2 rewind with NO snapshot available              -> counted, never silent
//!
//! INPUT PARTITIONS: (for PR1 — the three refusals `validate_persisted_set` already makes)
//!   IP-DUP    duplicate member slots            (one key clears a k-of-n)
//!   IP-MAX    more members than MAX_MAINTAINERS (unreachable by any live path)
//!   IP-THR    threshold != calculate_threshold(len) (a quorum downgrade)
//!   IP-EMPTY  the AUDIT-P1-019 empty carve-out  (MUST still be accepted, or the two
//!             gates have drifted and an emptied root becomes a boot failure)
//!
//! MATRIX
//!   PC1: O1        PC2: O1 O2      PC3: O1        PC4: O1 O2 O3
//!   PC5: O1        PR1: O2 O3 O4 O5 O6 O7 x {IP-DUP, IP-MAX, IP-THR, IP-EMPTY}
//!   PR2: O2 O5 O6

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature, MAX_MAINTAINERS};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, MaintainerSet, Transaction};
use doli_node::node::Node;
use doli_node::node::RollbackOutcome;
use storage::state_db::MaintainerUndoSnapshot;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

// ===========================================================================
// HARNESS — duplicated from `inc_i_174_maintainer_undo.rs` on purpose: integration
// test targets are separate crates and a shared `mod` directory would put harness
// code outside a `tests/` path.
// ===========================================================================

async fn seeded_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    node.maybe_bootstrap_maintainer_set(0).await;
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
    Transaction {
        version: 1,
        tx_type: if is_add {
            TxType::AddMaintainer
        } else {
            TxType::RemoveMaintainer
        },
        inputs: vec![],
        outputs: vec![],
        extra_data: MaintainerChangeData::new(*target, signatures).to_bytes(),
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
        .unwrap()
        .read()
        .await
        .set
        .clone()
}

/// A set built field-by-field, bypassing `with_members` — the only way to produce the
/// shapes `validate_persisted_set` exists to refuse.
fn raw_set(members: Vec<PublicKey>, threshold: usize, last_updated: u64) -> MaintainerSet {
    MaintainerSet {
        members,
        threshold,
        last_updated,
    }
}

fn pubkey(seed: u8) -> PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

// ===========================================================================
// REQ-174-001 (Must) — capture.
// ===========================================================================

/// REQ-174-001 bullets 1-3 + 6, PC1/PC2/PC5 x O1. **COMPILE-RED.**
///
/// The capture predicate is PURELY "does this block carry an Add/RemoveMaintainer".
/// It must NOT reuse `needs_producer_snapshot`, which ORs in `at_epoch_boundary`
/// (`bins/node/src/node/apply_block/mod.rs`): producer mutations are epoch-DEFERRED,
/// maintainer changes are applied IMMEDIATELY in the per-tx loop (analysis §4 Q4), so an
/// epoch term would capture snapshots at heights that changed nothing and — worse — would
/// still miss nothing only by accident.
#[tokio::test]
async fn req_174_001_snapshot_is_captured_iff_the_block_carries_a_rotation() {
    let (mut node, producers, _t) = seeded_node(4).await;
    let params = node.params.clone();
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    assert_eq!(
        blocks_per_epoch, 4,
        "harness: devnet epoch length 4 — h=4 must be an epoch boundary so PC5 is real"
    );

    let before = root(&node).await;

    // h=1..=4 — coinbase only. h=4 is an EPOCH BOUNDARY (PC5).
    let mut prev = Hash::ZERO;
    for h in 1..=4u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
    for h in 1..=4u64 {
        assert!(
            node.state_db.get_undo(h).is_some(),
            "harness: the block's own UndoData entry must exist at h={h}, or the \
             maintainer assertion below is about an absent height rather than an \
             absent maintainer snapshot"
        );
        assert!(
            node.state_db.get_maintainer_undo(h).is_none(),
            "PC1/PC5: O1 — h={h} carries no governance tx, so the maintainer snapshot \
             must be the ABSENT sentinel with NO serialized set bytes. h=4 is also an \
             epoch boundary: if THIS one is Some, the capture predicate reused \
             `needs_producer_snapshot`'s `at_epoch_boundary` term (analysis §4 Q1/Q4.2)."
        );
    }

    // h=5 — the rotation (PC2).
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(3).collect();
    let rot = build_block(
        5,
        5,
        prev,
        &producers[0],
        &params,
        vec![maintainer_tx(true, newcomer.public_key(), &signers)],
    );
    apply(&mut node, &rot).await;
    assert_eq!(
        root(&node).await.members.len(),
        5,
        "harness: rotation applied"
    );

    let snap = node
        .state_db
        .get_maintainer_undo(5)
        .expect("PC2: O1 — a block carrying a rotation MUST capture the pre-block state");
    assert_eq!(
        snap,
        MaintainerUndoSnapshot::new(
            rot.hash(),
            doli_core::maintainer::maintainer_set_digest(
                &before,
                node.params.genesis_hash.as_bytes()
            ),
            before.clone(),
            0,
        ),
        "PC2: O1 — the captured value must be the state as it was BEFORE the block, \
         field for field. Capturing the AFTER state would make the restore a no-op that \
         looks like a fix. AUDIT-P1-001 adds three more fields to that comparison: the \
         header must name this format generation, `block_hash` must be the hash of the \
         block that carried the rotation (NOT merely its height), and `set_digest` must \
         be the digest of the captured set — a capture that stamps any of them wrong \
         makes every later restore refuse."
    );
}

/// REQ-174-001 bullet 5, PC3 x O1. **COMPILE-RED.**
///
/// Two rotations in ONE block. The snapshot is the state before the FIRST — a per-tx
/// capture would leave the intermediate state in `cf_undo` and restore a set that never
/// existed at any block boundary.
#[tokio::test]
async fn req_174_001_two_rotations_in_one_block_capture_the_state_before_the_first() {
    let (mut node, producers, _t) = seeded_node(5).await;
    let params = node.params.clone();
    let before = root(&node).await;

    // Two removals in one block: 5 -> 4 -> 3.
    let victim_a = before.members[4];
    let victim_b = before.members[3];
    let signers_a: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a)
        .take(3)
        .collect();
    let signers_b: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim_a && *kp.public_key() != victim_b)
        .take(3)
        .collect();

    let b1 = build_block(
        1,
        1,
        Hash::ZERO,
        &producers[0],
        &params,
        vec![
            maintainer_tx(false, &victim_a, &signers_a),
            maintainer_tx(false, &victim_b, &signers_b),
        ],
    );
    apply(&mut node, &b1).await;
    assert_eq!(root(&node).await.members.len(), 3, "harness: both applied");

    let snap = node
        .state_db
        .get_maintainer_undo(1)
        .expect("PC3: O1 — present");
    assert_eq!(
        snap.set, before,
        "PC3: O1 — the snapshot must be the state before the FIRST rotation (5 members). \
         4 members means the capture ran per transaction and recorded an intermediate \
         state that was never a block boundary."
    );
}

/// REQ-174-001 bullet 4, PC4 x O1 O2 O3. **COMPILE-RED.**
///
/// A governance tx whose signatures do NOT verify changes nothing, so rolling the block
/// back must also change nothing. Restoring an identical value is acceptable; restoring a
/// DIFFERENT one is not.
#[tokio::test]
async fn req_174_001_a_rotation_that_fails_verification_leaves_the_root_untouched() {
    let (mut node, producers, _t) = seeded_node(4).await;
    let params = node.params.clone();
    let before = root(&node).await;

    // ONE signature against a threshold of 3 — refused by `verify_multisig_at`.
    let newcomer = KeyPair::generate();
    let signers: Vec<&KeyPair> = producers.iter().take(1).collect();
    let b1 = build_block(
        1,
        1,
        Hash::ZERO,
        &producers[0],
        &params,
        vec![maintainer_tx(true, newcomer.public_key(), &signers)],
    );
    apply(&mut node, &b1).await;
    assert_eq!(
        root(&node).await,
        before,
        "harness: a sub-threshold rotation must not have applied"
    );

    if let Some(snap) = node.state_db.get_maintainer_undo(1) {
        assert_eq!(
            snap.set, before,
            "PC4: O1 — capturing on a tx that failed verification is allowed (the \
             predicate may key on the tx TYPE, before verification), but the captured \
             value must equal the live one so the restore is a no-op"
        );
    }

    assert_eq!(
        node.rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved {
            depth: 1
        })
        .await
        .expect("rollback"),
        RollbackOutcome::RolledBack
    );
    assert_eq!(
        root(&node).await,
        before,
        "PC4: O2 — rolling back a block whose rotation never applied must leave the trust \
         root byte-identical"
    );
    assert_eq!(
        node.maintainer_state
            .as_ref()
            .unwrap()
            .read()
            .await
            .last_derived_height,
        0,
        "PC4: O3 — and must not disturb the seed arm"
    );
}

// ===========================================================================
// REQ-174-SEC-001 (Must) — a restore is not authority until validated.
// `cf_undo` is a NEW on-disk route to the release-verification trust root, under
// the same host-write threat model as `maintainer_state.bin` (analysis §8, row 3:
// "NONE TODAY").
// ===========================================================================

/// REQ-174-SEC-001, C3 x {IP-DUP, IP-MAX, IP-THR, IP-EMPTY} x O7. **COMPILE-RED.**
///
/// The gate the restore must run is the SAME FUNCTION `MaintainerState::load` runs. This
/// test pins the four verdicts through the public re-export, so a second, private copy of
/// the policy inside the rewind path cannot drift from the load path.
///
/// The empty carve-out (AUDIT-P1-019 / INC-I-172 M1) MUST be honoured identically:
/// `bins/node/tests/inc_i_172_command_trust_root_test.rs` requires an emptied root to
/// LOAD, so that operator commands resolve it to an unusable `OnChain` root instead of
/// falling back to the compiled bootstrap keys.
#[test]
fn req_174_sec_001_the_restore_gate_is_the_load_gate_verbatim() {
    let src = std::path::Path::new("cf_undo:maintainer_snapshot");

    // IP-DUP — five slots, one key. `count_distinct_signers` iterates SLOTS, so one
    // signature clears a 3-of-5.
    let dup = raw_set(vec![pubkey(0xA1); MAX_MAINTAINERS], 3, 100);
    assert!(
        storage::validate_persisted_set(src, &dup).is_err(),
        "IP-DUP: O7 — a duplicated member vector must be refused on the RESTORE path too"
    );

    // IP-MAX — more members than any live derivation can produce.
    let over: Vec<PublicKey> = (0..(MAX_MAINTAINERS as u8 + 1)).map(pubkey).collect();
    let threshold = MaintainerSet::calculate_threshold(over.len());
    assert!(
        storage::validate_persisted_set(src, &raw_set(over, threshold, 12)).is_err(),
        "IP-MAX: O7 — refused"
    );

    // IP-THR — the genuine five with a downgraded threshold: a 1-of-5 that reports the
    // right member list.
    let genuine: Vec<PublicKey> = (0..5).map(|i| pubkey(0xB0 + i)).collect();
    assert!(
        storage::validate_persisted_set(src, &raw_set(genuine, 1, 700)).is_err(),
        "IP-THR: O7 — refused"
    );

    // IP-EMPTY — the carve-out. Refusing this would turn a survivable, already
    // fail-closed state into an unrecoverable host.
    let mut emptied = MaintainerSet::with_members(vec![pubkey(111), pubkey(112)], 8);
    emptied.members.clear();
    assert!(
        storage::validate_persisted_set(src, &emptied).is_ok(),
        "IP-EMPTY: O7 — the AUDIT-P1-019 empty carve-out must be honoured IDENTICALLY on \
         both gates, or the two policies have already drifted"
    );

    // The green control: a well-formed set is accepted.
    assert!(
        storage::validate_persisted_set(src, &MaintainerSet::with_members(genuine_five(), 42))
            .is_ok(),
        "control: the honest steady state must pass, or the gate bricks every rewind"
    );
}

fn genuine_five() -> Vec<PublicKey> {
    (0..5).map(|i| pubkey(0x10 + i)).collect()
}

/// REQ-174-SEC-001, PR1 x IP-DUP x O2 O3 O4 O5 O6. **COMPILE-RED.**
///
/// An attacker with data-dir write puts a malformed set into the `cf_undo` entry. The
/// rewind must fail CLOSED and LOUD: the live trust root is NOT replaced, and it NEVER
/// degrades to `MaintainerState::default()` — an empty set with threshold 0 re-arms the
/// compiled bootstrap keys, which is the INC-I-172 F5 failure the load path already
/// refuses.
#[tokio::test]
async fn req_174_sec_001_a_snapshot_the_gate_refuses_never_becomes_the_trust_root() {
    let (mut node, producers, tmp) = seeded_node(4).await;
    let params = node.params.clone();
    // The pre-rotation root. Not asserted directly — `post_rotation` below is the value
    // a fail-closed refusal must preserve — but bound so the harness reads in order.
    let _live = root(&node).await;

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
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
    apply(&mut node, &rot).await;
    let post_rotation = root(&node).await;

    // Poison the undo entry: five member slots holding ONE key.
    assert!(
        node.state_db.get_maintainer_undo(4).is_some(),
        "harness: the rotation at h=4 must have captured a snapshot, or the poisoning \
         below is writing a record the restore path would never have read"
    );
    // The poison is AUTHENTIC on every AUDIT-P1-001 binding — real header, the real hash
    // of the block at h=4, the real digest of the poisoned set — precisely so it reaches
    // `validate_persisted_set`, which is the gate this test exists to pin. A poison that
    // failed the cheaper binding checks first would leave that gate untested.
    let poison = raw_set(vec![pubkey(0xF1); MAX_MAINTAINERS], 3, 4);
    node.state_db
        .put_maintainer_undo(
            4,
            &MaintainerUndoSnapshot::new(
                rot.hash(),
                doli_core::maintainer::maintainer_set_digest(
                    &poison,
                    node.params.genesis_hash.as_bytes(),
                ),
                poison.clone(),
                4,
            ),
        )
        .expect("put_maintainer_undo");

    let unrestored_before = node.maintainer_rewind_unrestored_count;
    assert_eq!(
        node.rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved {
            depth: 1
        })
        .await
        .expect("rollback"),
        RollbackOutcome::RolledBack
    );

    let after = root(&node).await;
    assert_ne!(
        after,
        raw_set(vec![pubkey(0xF1); MAX_MAINTAINERS], 3, 4),
        "PR1: O2 — the refused set must NOT have become this host's authority. One \
         signature clears its 3-of-5."
    );
    assert!(
        !after.members.is_empty(),
        "PR1: O2 — and the refusal must NEVER degrade to `MaintainerState::default()`. \
         An empty root re-arms the compiled bootstrap keys (INC-I-172 F5) — strictly \
         worse than the divergence INC-I-174 describes."
    );
    assert_eq!(
        after, post_rotation,
        "PR1: O2 — fail CLOSED means the live value is kept untouched"
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count,
        unrestored_before + 1,
        "PR1: O6 — and fail LOUD: a refused restore must be counted, or the operator has \
         no signal that this host's trust root no longer tracks the canonical chain"
    );

    // O4 — a refused restore must not rewrite the persisted root either.
    let disk = MaintainerState::load(tmp.path()).expect("the persisted root must still load");
    assert_eq!(
        disk.set, post_rotation,
        "PR1: O4 — the file must be left exactly as found"
    );
}

// ===========================================================================
// REQ-174-005 / REQ-174-004 (Must) — no silent route out of a rewind.
// ===========================================================================

/// REQ-174-005, PR2 x O5 O6. **COMPILE-RED.**
///
/// A rewind that finds no usable snapshot — the rebuild-from-genesis fallback
/// (`get_undo` -> `None`), which is also EXACTLY what every pre-upgrade `cf_undo` entry
/// looks like during the REQ-174-004 migration window (measured in
/// `crates/storage/tests/inc_i_174_undo_schema.rs`). It must be COUNTED, never silent.
#[tokio::test]
async fn req_174_005_a_rewind_with_no_usable_snapshot_is_counted_not_silent() {
    let (mut node, producers, _t) = seeded_node(4).await;
    let params = node.params.clone();

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
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
    apply(&mut node, &rot).await;

    // Simulate a pre-upgrade entry: the rotation happened, but no maintainer snapshot
    // was ever recorded for that height.
    assert!(
        node.state_db.get_undo(4).is_some(),
        "harness: the block's own UndoData entry must survive — this test is about a \
         MISSING MAINTAINER snapshot, not about a height with no undo data at all"
    );
    node.state_db
        .delete_maintainer_undo(4)
        .expect("delete_maintainer_undo");
    assert!(
        node.state_db.get_maintainer_undo(4).is_none(),
        "harness: the snapshot must actually be gone"
    );

    let before = node.maintainer_rewind_unrestored_count;
    assert_eq!(
        node.rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved {
            depth: 1
        })
        .await
        .expect("rollback"),
        RollbackOutcome::RolledBack
    );

    assert_eq!(
        node.maintainer_rewind_unrestored_count,
        before + 1,
        "PR2: O6 — a rewind across a height whose maintainer state cannot be restored \
         must be COUNTED. `None` from `get_undo` is indistinguishable from \"this block \
         changed nothing\", so the absent-vs-unchanged distinction is the ONLY thing \
         standing between a bounded, announced degradation and a silent one. The same \
         counter is the machine-checkable half of REQ-174-005's grep anchor."
    );
}

/// REQ-174-010 / REQ-174-008, O5. **COMPILE-RED.**
///
/// The positive counterpart: a rewind that DID restore is counted too. Without this, a
/// rise in `maintainer_rewind_unrestored_count` cannot be read as a rate.
#[tokio::test]
async fn req_174_010_a_successful_maintainer_rewind_is_counted() {
    let (mut node, producers, _t) = seeded_node(4).await;
    let params = node.params.clone();

    let mut prev = Hash::ZERO;
    for h in 1..=3u64 {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        apply(&mut node, &b).await;
    }
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
    apply(&mut node, &rot).await;

    let before = node.maintainer_rewind_count;
    assert_eq!(
        node.rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved {
            depth: 1
        })
        .await
        .expect("rollback"),
        RollbackOutcome::RolledBack
    );
    assert_eq!(
        node.maintainer_rewind_count,
        before + 1,
        "O5 — one restore, one increment"
    );
    assert_eq!(
        node.maintainer_rewind_unrestored_count, 0,
        "O6 — and the loudness counter stays at zero on the healthy path, or it is noise"
    );
}
