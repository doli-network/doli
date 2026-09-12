//! INC-I-217 M7 — what a rotation-carrying block does to a live node TODAY.
//!
//! covers: REQ-ROT-006, REQ-ROT-SEC-008
//!
//! ## FEASIBILITY — read before adding to this file
//!
//! `bls_key_rotation_activation_height` is `u64::MAX` on all three networks and is NOT
//! env-overridable (`network_params/env_loader.rs:551` copies it from defaults). M5 put the
//! D6 gate ahead of the mode match (`validation/block.rs:122`), and `apply_block` calls
//! `validate_block_for_apply` unconditionally (`apply_block/mod.rs:110`) — BEFORE the undo
//! snapshot decision at `mod.rs:173`. So no rotation-carrying block can reach the snapshot
//! decision, the queue-at-apply arm, or the boundary flush from this binary, in ANY
//! `ValidationMode`. Raising the gate is out of scope: activation heights are consensus
//! history.
//!
//! What that leaves reachable here, and what lives elsewhere:
//!   - REACHABLE: the gate verdict itself, and the proof that a rejected block leaves NO
//!     residue (no undo entry, no producer-set change, no height advance). Those are below.
//!   - `block_mutates_producer_set` — the REQ-ROT-006 predicate that decides whether the
//!     undo entry is a full snapshot or the empty sentinel — is `pub(super)` and is covered
//!     in `src/node/apply_block/helpers.rs::rotation_tests`, where the decision is made.
//!   - The verdict, the queueing and the boundary flush are in
//!     `crates/storage/src/producer/tests_rotation.rs`.
//!
//! ## WHAT M8 STILL OWES (none of it reachable until the gate can be crossed)
//!   1. Reorg across a FLUSHED rotation: roll back over the boundary block, assert the OLD
//!      `bls_pubkey` is restored and `compute_state_root` equals the root at `h-1`.
//!   2. Apply-vs-rebuild byte identity: `rebuild_producer_set_from_blocks` (apply #2, via
//!      `rewards.rs:1258`) must yield a `serialize_canonical()` identical to the
//!      `apply_block` path, flushing at the same boundary height.
//!   3. All four rebuild call sites exercised: `rollback.rs:144`, `rollback.rs:267`,
//!      `block_handling.rs:739`, `block_handling.rs:910`.
//!   4. D5 pool invariant: after the commit of any block that flushes a `RotateBlsKey`,
//!      `parent_sig_pool` holds no entry for that attester (`post_commit.rs:421`), and both
//!      reorg branches (`rollback.rs:184-186`, `:189`) clear it too.
//!
//! ## OUTPUT CONTRACT: fn apply_block(&mut self, Block, ValidationMode) -> Result<()>
//!   O1 return: `Err` carrying `[ERRTX-ROT002]` for a rotation below the gate; `Ok` for the
//!      control block.
//!   O2 receiver — `self.producer_set`: byte-identical after a rejected block.
//!   O3 persistent store — `state_db.get_undo(h)`: absent for a rejected block; present
//!      with the EMPTY sentinel for the mid-epoch control block.
//!   O4 receiver — `self.chain_state.best_height`: unchanged on reject, advanced on accept.
//!   O5 statics / O6 channels: none asserted here.
//!
//!   PATHS: P1 block carries a rotation, height < gate -> Err, no residue.
//!          P2 byte-identical block with the rotation removed -> Ok (non-vacuity control).
//!          P3 rollback across the accepted control block -> ProducerSet byte-identical.
//!   INPUT PARTITIONS: I1 a well-formed rotation (every stateless rule satisfied, so the
//!   ONLY reason to reject is the gate). I2 the same block without it.
//!   MATRIX: P1xO1, P1xO2, P1xO3, P1xO4, P2xO1, P2xO3, P3xO2 — each has a named test.

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::genesis_hash;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{rotation_auth_digest, Input, Output, RotateBlsData};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, Transaction, TxType};
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// The network `Node::new_for_test` runs on. The payload signs over THIS genesis hash, so
/// the block's only possible defect is the activation gate.
const TEST_NETWORK: Network = Network::Devnet;

/// Mid-epoch height, below any plausible `blocks_per_reward_epoch`, so the control block
/// lands on the empty-sentinel side.
const MID_EPOCH_HEIGHT: u64 = 3;

async fn make_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

fn rotation_outpoint() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m7-rotation-outpoint"), 0)
}

/// A rotation every stateless rule accepts. Crypto is real: a malformed payload would make
/// the rejection tests pass for a reason that has nothing to do with the gate.
fn well_formed_rotation(producer: &KeyPair) -> Transaction {
    let bls = BlsKeyPair::from_seed(&[0x77; 32]).expect("32-byte BLS seed is valid");
    let new_bls_pubkey = *bls.public_key().as_bytes();
    let (prev_tx_hash, output_index) = rotation_outpoint();
    let gh = genesis_hash(TEST_NETWORK);

    let digest = rotation_auth_digest(
        gh.as_bytes(),
        &new_bls_pubkey,
        prev_tx_hash.as_bytes(),
        output_index,
    );
    let payload = RotateBlsData {
        producer: *producer.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: *crypto::sign_rotation_pop(
            bls.secret_key(),
            bls.public_key(),
            gh.as_bytes(),
            producer.public_key().as_bytes(),
        )
        .expect("rotation PoP signing cannot fail")
        .as_bytes(),
        signature: *crypto::signature::sign(&digest, producer.private_key()).as_bytes(),
    };

    let mut input = Input::new(prev_tx_hash, output_index);
    input.public_key = Some(*producer.public_key());
    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(
            1_000_000,
            crypto::hash::hash(b"inc-i-217-m7-change"),
        )],
        extra_data: payload.encode().to_vec(),
    }
}

/// A coinbase-only block, optionally carrying `extra`. The two variants differ in exactly
/// one transaction, which is what makes the control non-vacuous.
fn build_block(
    height: u64,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    extra: Vec<Transaction>,
) -> Block {
    let slot = height as u32;
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);

    let mut txs = vec![coinbase];
    txs.extend(extra);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + (slot as u64 * params.slot_duration),
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, txs)
}

/// Apply coinbase-only blocks `1..up_to` and return the tip hash.
async fn fill_to(
    node: &mut Node,
    producer: &KeyPair,
    params: &ConsensusParams,
    up_to: u64,
) -> Hash {
    let mut prev = Hash::ZERO;
    for h in 1..up_to {
        let block = build_block(h, prev, producer, params, Vec::new());
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("filler apply_block failed at h={h}: {e}"));
    }
    prev
}

async fn producer_set_bytes(node: &Node) -> Vec<u8> {
    let ps = node.producer_set.read().await;
    ps.serialize_canonical()
}

// ===========================================================================
// Precondition — the relationship every test below depends on
// ===========================================================================

// REQ-ROT-SEC-008 — Decision: whether the gate still sits above every reachable height. The
// moment the pinning session lands a real value this fails loudly, instead of letting the
// rejection tests keep passing for a reason that no longer holds.
#[test]
fn precondition_the_rotation_gate_is_above_every_test_height() {
    for network in [Network::Devnet, Network::Testnet, Network::Mainnet] {
        assert!(
            NetworkParams::defaults(network).bls_key_rotation_activation_height > MID_EPOCH_HEIGHT,
            "{network:?} pinned bls_key_rotation_activation_height at or below the test \
             height — this file's rejection tests no longer test the gate. Move the M8 \
             work listed in the module header here."
        );
    }
}

// ===========================================================================
// P1 — a rotation-carrying block is rejected, with no residue
// ===========================================================================

// REQ-ROT-SEC-008 — Decision: whether a node accepts a rotation before the fleet has agreed
// on a height. An un-upgraded peer cannot even decode the type, so an accepting node forks
// alone the moment the first rotation is gossiped.
#[tokio::test]
async fn rotation_block_is_rejected_below_the_gate() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let prev = fill_to(&mut node, &producers[0], &params, MID_EPOCH_HEIGHT).await;

    let block = build_block(
        MID_EPOCH_HEIGHT,
        prev,
        &producers[0],
        &params,
        vec![well_formed_rotation(&producers[0])],
    );
    let err = node
        .apply_block(block, ValidationMode::Light)
        .await
        .expect_err("a rotation below the gate must make the carrying block invalid");

    assert!(
        err.to_string().contains("[ERRTX-ROT002]"),
        "expected the D6 gate error, got: {err}"
    );
}

// REQ-ROT-006 — Decision: whether a rejected rotation block leaves a half-applied undo entry
// or a mutated producer set. Residue from a rejected block is what poisons the next apply.
#[tokio::test]
async fn rejected_rotation_block_leaves_no_residue() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let prev = fill_to(&mut node, &producers[0], &params, MID_EPOCH_HEIGHT).await;

    let before_bytes = producer_set_bytes(&node).await;
    let before_height = node.chain_state.read().await.best_height;

    let block = build_block(
        MID_EPOCH_HEIGHT,
        prev,
        &producers[0],
        &params,
        vec![well_formed_rotation(&producers[0])],
    );
    assert!(node
        .apply_block(block, ValidationMode::Light)
        .await
        .is_err());

    assert!(
        node.state_db.get_undo(MID_EPOCH_HEIGHT).is_none(),
        "a rejected block wrote an undo entry at h={MID_EPOCH_HEIGHT}"
    );
    assert_eq!(
        producer_set_bytes(&node).await,
        before_bytes,
        "a rejected rotation block mutated the producer set"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        before_height,
        "a rejected block advanced the chain"
    );
}

// ===========================================================================
// P2 — the non-vacuity control
// ===========================================================================

// REQ-ROT-006 — Decision: whether the rejection above is about the ROTATION or about the
// block shape. Without this row a bad header would produce the same `is_err()` and the
// rejection tests would prove nothing.
#[tokio::test]
async fn the_same_block_without_the_rotation_is_accepted() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let prev = fill_to(&mut node, &producers[0], &params, MID_EPOCH_HEIGHT).await;

    let block = build_block(MID_EPOCH_HEIGHT, prev, &producers[0], &params, Vec::new());
    node.apply_block(block, ValidationMode::Light)
        .await
        .expect("the byte-identical block minus the rotation must be accepted");

    let undo = node
        .state_db
        .get_undo(MID_EPOCH_HEIGHT)
        .expect("an accepted block writes an undo entry");
    assert!(
        undo.producer_snapshot.is_empty(),
        "a mid-epoch block with no producer-mutating tx must use the empty sentinel — if \
         this is non-empty, `rotation_block_mutates_the_producer_set` in \
         apply_block/helpers.rs is passing vacuously"
    );
}

// ===========================================================================
// P3 — rollback byte identity
// ===========================================================================

// REQ-ROT-006 — Decision: whether rolling back the block at the rotation's height restores
// the producer set exactly. This is the property the rotation undo snapshot exists to give;
// pinning it on the control block means M8 inherits a working baseline, not a new question.
#[tokio::test]
async fn rollback_across_the_control_block_restores_the_producer_set() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let prev = fill_to(&mut node, &producers[0], &params, MID_EPOCH_HEIGHT).await;

    let before = producer_set_bytes(&node).await;
    let block = build_block(MID_EPOCH_HEIGHT, prev, &producers[0], &params, Vec::new());
    node.apply_block(block, ValidationMode::Light)
        .await
        .unwrap();

    let outcome = node
        .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback_one_block failed");
    assert_eq!(outcome, RollbackOutcome::RolledBack);

    assert_eq!(
        producer_set_bytes(&node).await,
        before,
        "rollback did not restore the producer set byte-for-byte"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        MID_EPOCH_HEIGHT - 1
    );
}
