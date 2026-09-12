//! INC-I-217 M8 — the two apply paths, the rollback/reorg paths and the
//! activation-height override must agree about a `RotateBlsKey`.
//!
//! covers: bins/node/src/node/rewards.rs (`rebuild_producer_set_from_blocks`),
//! covers: bins/node/src/node/rollback.rs, bins/node/src/node/block_handling.rs
//! covers: bins/node/src/node/apply_block/{state_update,post_commit}.rs
//!
//! OUTPUT CONTRACT — `Node::rebuild_producer_set_from_blocks(&self, &mut ProducerSet, u64)`
//!   O1 the `&mut ProducerSet`: `serialize_canonical()` bytes
//!   O2 the `&mut ProducerSet`: `as_parts().2` — the ORDERED `pending_updates` queue
//!   O3 return `Result<()>`
//!   O4 no store write (asserted by the 156 suite; not re-asserted here)
//! OUTPUT CONTRACT — `Node::rollback_one_block` / `Node::execute_reorg`
//!   O1 `producer_set` — the target's `bls_pubkey`
//!   O2 `producer_set.serialize_canonical()`
//!   O3 `self.parent_sig_pool` — `total_signatures()`
//!   O4 `chain_state.best_height`
//! OUTPUT CONTRACT — `Node::apply_block` at an epoch boundary
//!   O1 `producer_set` — `bls_pubkey` and the queue
//!   O2 `state_db` — the persisted `ProducerSet` a restart reads
//!   O3 `self.parent_sig_pool`
//!
//! INPUT PARTITIONS (height of the observation relative to the rotation)
//!   P1 mid-epoch after the rotation, queue still in flight
//!   P2 at or after the epoch boundary that flushes the queue
//!   P3 rolled back to before the rotation block
//!   P4 reorged onto a sibling branch that never carried the rotation
//!
//! MATRIX
//!   P1: O1 -  O2 ✓        P2: O1 ✓  O2 ✓  O3 ✓
//!   P3: O1 ✓  O2 ✓  O3 ✓  P4: O1 ✓  O2 ✓
//!
//! The activation height reaches a test height ONLY through
//! `DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT`; the three shipped defaults stay
//! `u64::MAX` and `precondition_*` below re-asserts that.

use std::collections::HashSet;
use std::sync::Once;

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::genesis::genesis_hash;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{rotation_auth_digest, Input, Output, RotateBlsData};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, Transaction, TxType};
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, UtxoEntry, UtxoSet};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const NET: Network = Network::Devnet;
const EPOCH_LEN: u64 = 4;
const GENESIS_BLOCKS: u64 = 40;

/// Devnet override value. `> EPOCH_LEN` so no test observation lands in epoch 0,
/// where the flush runs every block and the pool clear does not (architecture D5).
const ROT_AH: u64 = 8;

/// Geometry of the APPLIED fixture (`apply_block`, real undo entries).
const A_ROT_H: u64 = 9;
const A_BOUNDARY: u64 = 12;

/// Geometry of the SEEDED fixture (`put_block_canonical`, no apply_block).
/// `S_REG_H > GENESIS_BLOCKS` — both paths SKIP a `Registration` at or below it.
const S_REG_H: u64 = 41;
const S_REG_FLUSH: u64 = 44;
const S_ROT_H: u64 = 45;
const S_MID: u64 = 47;
const S_BOUNDARY: u64 = 48;

const FUND: u64 = 100_000;
const CHANGE: u64 = 99_000;

static BOOT: Once = Once::new();

/// Every test in this file agrees on the same value, so the `Once` race is benign.
/// Must run before the first `Network::params()` touch: that call is `OnceLock`-cached.
fn boot() {
    BOOT.call_once(|| {
        std::env::set_var(
            "DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT",
            ROT_AH.to_string(),
        );
    });
}

fn assert_gate_armed() {
    let live = NET.params().bls_key_rotation_activation_height;
    assert_eq!(
        live, ROT_AH,
        "DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT is not honoured on devnet (resolved {live}). \
         env_loader.rs:550-551 pins the field to `defaults` on EVERY network; M8 must give it \
         the same non-mainnet `env_parse` arm every other activation height has. Until then no \
         rotation can be applied at any reachable height and this whole file is unreachable."
    );
}

// ==================== Shared builders ====================

fn new_bls(seed: u8) -> BlsKeyPair {
    BlsKeyPair::from_seed(&[seed; 32]).expect("a 32-byte BLS seed is valid")
}

/// A fully valid `RotateBlsKey` over `outpoint`: the payload signature covers
/// `rotation_auth_digest`, the PoP uses the rotation DST, and the input is signed
/// as an ordinary spend so `validate_transaction_with_utxos` also accepts it.
fn rotation_tx(producer: &KeyPair, bls: &BlsKeyPair, outpoint: Outpoint, to: Hash) -> Transaction {
    let new_bls_pubkey = *bls.public_key().as_bytes();
    let gh = genesis_hash(NET);
    let digest = rotation_auth_digest(
        gh.as_bytes(),
        &new_bls_pubkey,
        outpoint.tx_hash.as_bytes(),
        outpoint.index,
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
    let mut input = Input::new(outpoint.tx_hash, outpoint.index);
    input.public_key = Some(*producer.public_key());
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(CHANGE, to)],
        extra_data: payload.encode().to_vec(),
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, producer.private_key());
    tx
}

fn block_at(
    node: &Node,
    height: u64,
    slot: u32,
    prev: Hash,
    by: &KeyPair,
    txs: Vec<Transaction>,
) -> Block {
    let coinbase = Transaction::new_coinbase(
        node.params.block_reward(height),
        doli_core::consensus::reward_pool_pubkey_hash(),
        height,
        slot,
    );
    let mut all = vec![coinbase];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (u64::from(slot) * node.params.slot_duration),
        slot,
        producer: *by.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, all)
}

async fn make_node() -> (Node, KeyPair, TempDir) {
    boot();
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    assert_eq!(node.config.network.blocks_per_reward_epoch(), EPOCH_LEN);
    assert_eq!(node.config.network.genesis_blocks(), GENESIS_BLOCKS);
    (node, kp, temp)
}

fn address_of(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

async fn bls_of(node: &Node, pk: &PublicKey) -> Vec<u8> {
    node.producer_set
        .read()
        .await
        .get_by_pubkey(pk)
        .expect("the rotation target must be in the ProducerSet")
        .bls_pubkey
        .clone()
}

async fn canonical(node: &Node) -> Vec<u8> {
    node.producer_set.read().await.serialize_canonical()
}

/// bincode of the ORDERED queue. Order is the observable M7 measured for the
/// Exit/Slash/AddBond/Rotate arrival mix; a multiset comparison would not see it.
fn queue_bytes(ps: &ProducerSet) -> Vec<u8> {
    bincode::serialize(ps.as_parts().2).expect("PendingProducerUpdate must serialize")
}

// ==================== Fixture A — applied through apply_block ====================

/// Production UTXO backend, then a funded outpoint the rotation can spend.
async fn applied_fixture() -> (Node, KeyPair, BlsKeyPair, Outpoint, TempDir) {
    let (mut node, kp, temp) = make_node().await;
    assert_gate_armed();
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = UtxoSet::from_state_db(node.state_db.clone());
    }
    let outpoint = Outpoint::new(crypto::hash::hash(b"inc-i-217-m8-rotation-funding"), 0);
    node.utxo_set
        .write()
        .await
        .insert(
            outpoint,
            UtxoEntry {
                output: Output::normal(FUND, address_of(&kp)),
                height: 0,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: funding insert");
    let mut prev = Hash::ZERO;
    for h in 1..A_ROT_H {
        let block = block_at(&node, h, h as u32, prev, &kp, Vec::new());
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: apply_block failed at h={h}: {e}"));
    }
    (node, kp, new_bls(0x51), outpoint, temp)
}

/// Applies the rotation block at `A_ROT_H` and returns its hash.
async fn apply_rotation(node: &mut Node, kp: &KeyPair, bls: &BlsKeyPair, op: Outpoint) -> Hash {
    let prev = node.chain_state.read().await.best_hash;
    let tx = rotation_tx(kp, bls, op, address_of(kp));
    let block = block_at(node, A_ROT_H, A_ROT_H as u32, prev, kp, vec![tx]);
    let hash = block.hash();
    node.apply_block(block, ValidationMode::Light)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "the rotation block at h={A_ROT_H} was rejected: {e}. Above \
                 DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT={ROT_AH} a well-formed rotation must \
                 be a VALID block — if this is [ERRTX-ROT002] the env override is still missing."
            )
        });
    hash
}

async fn extend_plain(
    node: &mut Node,
    kp: &KeyPair,
    from: u64,
    to: u64,
    slot_base: u32,
) -> Vec<Block> {
    let mut out = Vec::new();
    let mut prev = node.chain_state.read().await.best_hash;
    for h in from..=to {
        let slot = slot_base + h as u32;
        let block = block_at(node, h, slot, prev, kp, Vec::new());
        prev = block.hash();
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={h} slot={slot}: {e}"));
        out.push(block);
    }
    out
}

// ==================== Fixture B — seeded store, replayed twice ====================

/// Writes `1..=S_BOUNDARY` straight into the block store. Nothing is applied, so
/// both replays below start from an identical EMPTY `ProducerSet` and the only
/// divergence either can show is the `RotateBlsKey` arm itself.
async fn seeded_fixture() -> (Node, KeyPair, BlsKeyPair, Vec<(u64, Block)>, TempDir) {
    let (node, kp, temp) = make_node().await;
    assert_gate_armed();
    let target = KeyPair::generate();
    let bls = new_bls(0x52);
    let unit = node.config.network.bond_unit();
    let op = Outpoint::new(crypto::hash::hash(b"inc-i-217-m8-seeded-outpoint"), 0);
    let mut blocks = Vec::new();
    for h in 1..=S_BOUNDARY {
        let txs = if h == S_REG_H {
            vec![Transaction::new_registration(
                Vec::new(),
                *target.public_key(),
                unit,
                u64::MAX,
                1,
            )]
        } else if h == S_ROT_H {
            vec![rotation_tx(&target, &bls, op, address_of(&target))]
        } else {
            Vec::new()
        };
        let block = block_at(&node, h, h as u32, Hash::ZERO, &kp, txs);
        node.block_store
            .put_block_canonical(&block, h)
            .expect("fixture: put_block_canonical");
        blocks.push((h, block));
    }
    node.block_store
        .ensure_blocks_present(1, S_BOUNDARY)
        .expect("fixture: the store must be DENSE over the replay range");
    (node, target, bls, blocks, temp)
}

/// The LIVE apply path, driven directly through the production per-tx function
/// `apply_block` uses, with `apply_block`'s own flush predicate.
fn live_replay(node: &Node, blocks: &[(u64, Block)], up_to: u64) -> ProducerSet {
    let mut ps = ProducerSet::new();
    let utxo = UtxoSet::new();
    for (height, block) in blocks.iter().filter(|(h, _)| *h <= up_to) {
        let mut dirty: HashSet<Hash> = HashSet::new();
        let mut regs: Vec<PublicKey> = Vec::new();
        for tx in &block.transactions {
            node.process_transaction_producer_effects(
                tx,
                *height,
                block.header.slot,
                &utxo,
                &mut ps,
                &mut dirty,
                &mut regs,
            );
        }
        if *height < EPOCH_LEN || height.is_multiple_of(EPOCH_LEN) {
            ps.apply_pending_updates_with_cap(0);
        }
        ps.process_unbonding(*height, doli_core::consensus::UNBONDING_PERIOD);
    }
    ps
}

fn rebuilt(node: &Node, up_to: u64) -> ProducerSet {
    let mut ps = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut ps, up_to)
        .expect("O3: a DENSE range must return Ok(())");
    ps
}

fn rotations_queued(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RotateBlsKey { .. }))
        .count()
}

// ==========================================================================
//  Preconditions
// ==========================================================================

/// REQ-ROT-003 — Decision: a shipped default that drifted off `u64::MAX` would make
/// every RED below pass for a reason that is not the code under test.
#[test]
fn precondition_the_three_shipped_defaults_are_still_frozen() {
    // INC-I-217: testnet was pinned to 176_200 on 2026-09-12; this harness runs on Devnet
    // (`NET`), whose default must stay u64::MAX so the gate is armed by ENV only.
    for (network, expected) in [
        (Network::Devnet, u64::MAX),
        (Network::Testnet, 176_200),
        (Network::Mainnet, u64::MAX),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).bls_key_rotation_activation_height,
            expected,
            "{network:?} default must be exactly {expected} — devnet/mainnet stay frozen, \
             M8 arms the devnet gate by ENV, never by pin"
        );
    }
}

// ==========================================================================
//  1 — REQ-ROT-007: the second apply implementation
// ==========================================================================

/// REQ-ROT-007 (Must) — Decision: whether a reorg or a rollback silently ERASES a
/// rotation from the chain's history, forking every node that replays instead of undoes.
///
/// Acceptance: `rebuild_producer_set_from_blocks` over blocks carrying a rotation
/// produces a `ProducerSet` byte-identical to the one the live per-tx path builds,
/// queue order included.
#[tokio::test]
async fn req_rot_007_rebuild_from_blocks_matches_live_apply_across_a_rotation() {
    let (node, target, bls, blocks, _t) = seeded_fixture().await;
    let tpk = *target.public_key();
    let new_key = bls.public_key().as_bytes().to_vec();

    let live_flush = live_replay(&node, &blocks, S_REG_FLUSH);
    let rebuilt_flush = rebuilt(&node, S_REG_FLUSH);
    assert_eq!(
        live_flush.serialize_canonical(),
        rebuilt_flush.serialize_canonical(),
        "fixture precondition: the two paths must already agree at h={S_REG_FLUSH}, BEFORE the \
         rotation. If they differ here the divergence measured below is not the rotation's."
    );
    assert!(
        rebuilt_flush.get_by_pubkey(&tpk).is_some(),
        "fixture precondition: the on-chain Registration at h={S_REG_H} must have flushed by \
         h={S_REG_FLUSH}, or the rotation is skipped as NotProducer on BOTH paths and this \
         test cannot fail"
    );

    // P1 — mid-epoch, queue still in flight. O2 only: the flush has not run.
    let live_mid = live_replay(&node, &blocks, S_MID);
    let rebuilt_mid = rebuilt(&node, S_MID);
    assert_eq!(
        rotations_queued(&live_mid, &tpk),
        1,
        "control: the LIVE path must hold exactly one queued RotateBlsKey at h={S_MID}"
    );
    assert_eq!(
        queue_bytes(&rebuilt_mid),
        queue_bytes(&live_mid),
        "REQ-ROT-007 / O2 [P1]: the rebuild's ORDERED pending_updates queue must equal the \
         live one at h={S_MID}. rewards.rs enumerates TxType by hand (7 arms, 1259-1494) and \
         has no RotateBlsKey arm, so the rebuild drops the rotation entirely. \
         live={} rebuilt={}",
        rotations_queued(&live_mid, &tpk),
        rotations_queued(&rebuilt_mid, &tpk)
    );

    // P2 — past the boundary that flushes it. O1 + O2.
    let live_end = live_replay(&node, &blocks, S_BOUNDARY);
    let rebuilt_end = rebuilt(&node, S_BOUNDARY);
    assert_eq!(
        live_end
            .get_by_pubkey(&tpk)
            .expect("live: target present")
            .bls_pubkey,
        new_key,
        "control: the live path must have INSTALLED the new key at h={S_BOUNDARY}"
    );
    assert_eq!(
        rebuilt_end
            .get_by_pubkey(&tpk)
            .expect("rebuilt: target present")
            .bls_pubkey,
        new_key,
        "REQ-ROT-007 / O1 [P2]: the rebuilt set must carry the SAME bls_pubkey as the live one"
    );
    assert_eq!(
        rebuilt_end.serialize_canonical(),
        live_end.serialize_canonical(),
        "REQ-ROT-007 / O1 [P2]: serialize_canonical() bincodes the whole ProducerInfo, so \
         bls_pubkey is inside the state root. A rebuild that drops the rotation hands every \
         reorging node a different state root than the one that applied it."
    );
    assert_eq!(
        queue_bytes(&rebuilt_end),
        queue_bytes(&live_end),
        "REQ-ROT-007 / O2 [P2]: both queues must be drained identically"
    );
    println!("ROT-REBUILD-EQ-LIVE");
}

// ==========================================================================
//  2 — REQ-ROT-006: reorg across the rotation block
// ==========================================================================

/// REQ-ROT-006 (Must) — Decision: whether a node that reorged away from a rotation
/// keeps attesting under a key the rest of the network has already abandoned.
///
/// Acceptance: after a reorg onto a sibling branch with no rotation, `bls_pubkey` is
/// the OLD key and the ProducerSet matches a node that never saw the rotation.
#[tokio::test]
async fn req_rot_006_reorg_across_the_rotation_restores_the_old_key() {
    let (mut a, kp, bls, op, _ta) = applied_fixture().await;
    let pk = *kp.public_key();
    let old_key = bls_of(&a, &pk).await;
    let new_key = bls.public_key().as_bytes().to_vec();
    assert_ne!(old_key, new_key, "fixture: the two keys must differ");

    let ancestor = a.chain_state.read().await.best_hash;
    let ancestor_h = a.chain_state.read().await.best_height;
    assert_eq!(ancestor_h, A_ROT_H - 1);

    // Branch R: the rotation, then plain blocks across the flush boundary.
    let rot_hash = apply_rotation(&mut a, &kp, &bls, op).await;
    let r_tail = extend_plain(&mut a, &kp, A_ROT_H + 1, A_BOUNDARY, 0).await;
    assert_eq!(
        bls_of(&a, &pk).await,
        new_key,
        "control: the flush at h={A_BOUNDARY} must have installed the new key on branch R"
    );

    // Branch S: same heights, higher slots, no rotation. Built on a second node so
    // the comparison target has applied EXACTLY these blocks and nothing else.
    let (mut c, _kc, _bc, _oc, _tc) = applied_fixture_from(&kp).await;
    let s_blocks = extend_plain(&mut c, &kp, A_ROT_H, A_BOUNDARY + 1, 100).await;
    assert_eq!(c.chain_state.read().await.best_height, A_BOUNDARY + 1);

    {
        let mut cache = a.fork_block_cache.write().await;
        for block in &s_blocks {
            cache.insert(block.hash(), block.clone());
        }
    }
    let mut rollback: Vec<Hash> = r_tail.iter().rev().map(Block::hash).collect();
    rollback.push(rot_hash);
    let reorg = network::sync::ReorgResult {
        rollback,
        common_ancestor: ancestor,
        new_blocks: s_blocks.iter().map(Block::hash).collect(),
        weight_delta: 1,
    };
    let trigger = s_blocks.last().expect("branch S is non-empty").clone();
    a.execute_reorg(reorg, trigger)
        .await
        .expect("execute_reorg must succeed onto a strictly longer sibling branch");

    assert_eq!(
        bls_of(&a, &pk).await,
        old_key,
        "REQ-ROT-006 / O1 [P4]: after reorging onto a branch that never carried the rotation, \
         bls_pubkey must be the PRE-rotation key. execute_reorg restores the ProducerSet from \
         the undo snapshot, or rebuilds it via rebuild_producer_set_from_blocks \
         (block_handling.rs:769) — the path that has no RotateBlsKey arm."
    );
    assert_eq!(
        canonical(&a).await,
        canonical(&c).await,
        "REQ-ROT-006 / O2 [P4]: the reorged node's ProducerSet must be byte-identical to a node \
         that applied only branch S. Any difference is a consensus fork."
    );
    println!("ROT-REORG-OLD-KEY");
}

/// Second node with the same producer key, so its genesis ProducerSet is identical.
async fn applied_fixture_from(kp: &KeyPair) -> (Node, KeyPair, BlsKeyPair, Outpoint, TempDir) {
    boot();
    let temp = TempDir::new().expect("tempdir");
    let mut node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = UtxoSet::from_state_db(node.state_db.clone());
    }
    let outpoint = Outpoint::new(crypto::hash::hash(b"inc-i-217-m8-rotation-funding"), 0);
    node.utxo_set
        .write()
        .await
        .insert(
            outpoint,
            UtxoEntry {
                output: Output::normal(FUND, address_of(kp)),
                height: 0,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: funding insert");
    let mut prev = Hash::ZERO;
    for h in 1..A_ROT_H {
        let block = block_at(&node, h, h as u32, prev, kp, Vec::new());
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: apply_block failed at h={h}: {e}"));
    }
    (node, kp.clone(), new_bls(0x51), outpoint, temp)
}

// ==========================================================================
//  3 — REQ-ROT-011 / D5: rollback across the rotation clears the pool
// ==========================================================================

/// REQ-ROT-011 (Must) — Decision: whether a rollback leaves halves verified under the
/// NEW key pooled against a producer the ProducerSet has just reverted to the OLD key,
/// which makes the next aggregate fail for the whole block.
///
/// Acceptance: after rolling back past the rotation the queue is empty, `bls_pubkey`
/// is the old key, and `parent_sig_pool` holds nothing.
#[tokio::test]
async fn req_rot_011_rollback_across_the_rotation_clears_the_parent_sig_pool() {
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let old_key = bls_of(&node, &pk).await;
    let before = canonical(&node).await;

    apply_rotation(&mut node, &kp, &bls, op).await;
    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        1,
        "control: the rotation must be queued after its block applies"
    );

    // Seed the pool so "empty" is an observation, not a vacuous pass.
    let parent = node.chain_state.read().await.best_hash;
    node.parent_sig_pool.insert(parent, pk, [0x33; 96]);
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        1,
        "control: the seeded pool entry must be present before the rollback"
    );

    let outcome = node
        .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback_one_block failed");
    assert_eq!(outcome, RollbackOutcome::RolledBack);

    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        0,
        "O1 [P3]: the queued rotation must be gone after the rollback"
    );
    assert_eq!(
        bls_of(&node, &pk).await,
        old_key,
        "O1 [P3]: bls_pubkey must be the pre-rotation key"
    );
    assert_eq!(
        canonical(&node).await,
        before,
        "O2 [P3]: the ProducerSet must be byte-identical to its pre-rotation value"
    );
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        0,
        "REQ-ROT-011 / O3 [P3] (architecture D5): a rollback that restores or rebuilds the \
         ProducerSet must also clear parent_sig_pool. The two restore sites are \
         rollback.rs:188 (`*producers = restored_producers`) and rollback.rs:192 (the \
         rebuild fallback); neither calls `self.parent_sig_pool.clear()` today."
    );
}

// ==========================================================================
//  4 — REQ-ROT-005/011: boundary crossing and restart durability
// ==========================================================================

/// REQ-ROT-005 (Must) — Decision: whether the rotation ever reaches the producer at all,
/// i.e. whether a mismatched producer can actually recover.
#[tokio::test]
async fn req_rot_005_queued_rotation_installs_the_new_key_at_the_epoch_boundary() {
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let old_key = bls_of(&node, &pk).await;
    let new_key = bls.public_key().as_bytes().to_vec();

    apply_rotation(&mut node, &kp, &bls, op).await;
    assert_eq!(
        bls_of(&node, &pk).await,
        old_key,
        "O1 [P1]: mid-epoch the key must NOT change — the update is epoch-deferred (D4)"
    );

    extend_plain(&mut node, &kp, A_ROT_H + 1, A_BOUNDARY, 0).await;
    assert_eq!(
        bls_of(&node, &pk).await,
        new_key,
        "REQ-ROT-005 / O1 [P2]: the boundary block at h={A_BOUNDARY} must install the new key"
    );
    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        0,
        "O2 [P2]: the queue must be drained by the flush"
    );
}

/// REQ-ROT-011 (Must) — Decision: whether a node restarted between the rotation tx and
/// the next boundary loses the rotation, leaving that producer mismatched forever.
///
/// HARNESS LIMIT: `Node::new_for_test` re-seeds `state_db` on every call, so a real
/// process restart is not constructible here. What IS constructible — and is exactly
/// what a restart reads — is the persisted `ProducerSet` plus the boundary flush.
#[tokio::test]
async fn req_rot_011_queued_rotation_survives_the_persisted_producer_set() {
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let new_key = bls.public_key().as_bytes().to_vec();

    apply_rotation(&mut node, &kp, &bls, op).await;

    let mut from_disk = node.state_db.load_producer_set();
    assert_eq!(
        rotations_queued(&from_disk, &pk),
        1,
        "O2 [P1]: the queued rotation must be DURABLE. apply_block persists the full \
         ProducerSet, and a restart rebuilds in-memory state from exactly these bytes; a \
         queue that does not round-trip loses the rotation on every node bounce."
    );
    from_disk.apply_pending_updates_with_cap(0);
    assert_eq!(
        from_disk
            .get_by_pubkey(&pk)
            .expect("target present after reload")
            .bls_pubkey,
        new_key,
        "O2 [P2]: the reloaded queue must still install the new key at the next boundary"
    );
}

// ==========================================================================
//  5 — INV-ATTEST-003 / D5: one predicate governs flush AND pool clear
// ==========================================================================

/// REQ-ROT-011 (Must) — Decision: whether a boundary block can install a NEW key while
/// halves verified under the OLD one stay pooled, poisoning the next aggregate.
///
/// Behavioural, not a source scan. Above epoch 0 the flush predicate
/// (`state_update.rs:180-182`) and the clear predicate (`post_commit.rs:414`) must agree:
/// at a non-boundary height NEITHER fires, at a boundary height BOTH do.
#[tokio::test]
async fn req_rot_011_boundary_flush_and_pool_clear_agree_above_epoch_zero() {
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let old_key = bls_of(&node, &pk).await;

    apply_rotation(&mut node, &kp, &bls, op).await;

    // Non-boundary heights: neither the flush nor the clear may fire.
    for h in (A_ROT_H + 1)..A_BOUNDARY {
        assert!(
            !doli_core::EpochSnapshot::is_epoch_boundary_with(h, EPOCH_LEN),
            "geometry: h={h} must not be an epoch boundary"
        );
        let parent = node.chain_state.read().await.best_hash;
        node.parent_sig_pool.insert(parent, pk, [0x44; 96]);
        extend_plain(&mut node, &kp, h, h, 0).await;
        assert_eq!(
            rotations_queued(&*node.producer_set.read().await, &pk),
            1,
            "[P1] h={h}: a non-boundary block must NOT flush the queue"
        );
        assert_eq!(
            bls_of(&node, &pk).await,
            old_key,
            "[P1] h={h}: a non-boundary block must NOT install the new key"
        );
        assert!(
            node.parent_sig_pool.get(&parent, &pk).is_some(),
            "[P1] h={h}: a non-boundary block must NOT clear parent_sig_pool — if it did, the \
             clear is governed by a WIDER predicate than the flush and the D5 pairing is false"
        );
    }

    let parent = node.chain_state.read().await.best_hash;
    node.parent_sig_pool.insert(parent, pk, [0x45; 96]);
    assert!(node.parent_sig_pool.total_signatures() >= 1);

    extend_plain(&mut node, &kp, A_BOUNDARY, A_BOUNDARY, 0).await;
    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        0,
        "[P2] h={A_BOUNDARY}: the boundary block MUST flush the queue"
    );
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        0,
        "INV-ATTEST-003 / O3 [P2]: the SAME boundary block that flushes a RotateBlsKey must \
         leave parent_sig_pool empty. A half pooled under the old key and aggregated after the \
         new key is installed fails verification for the WHOLE block (architecture D5)."
    );
}
