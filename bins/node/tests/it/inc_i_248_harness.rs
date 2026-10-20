//! INC-I-248 M1 — shared fixture: replay node A, snapshot at H, snap-joined node B.
//!
//! OUTPUT CONTRACT: N/A — fixture module. Its asserts are fixture preconditions only.
//! INPUT PARTITIONS: N/A — fixture module.

#![allow(dead_code)]

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{
    maintainer_set_digest, signing_message_at, MaintainerChangeData, MaintainerSignature,
    PeerMaintainerAnswer, ProtocolActivationData, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, MaintainerSet, Network, Transaction};
use doli_node::node::Node;
use network::protocols::SyncResponse;
use network::VerifiedSnapshot;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use updater::{sign_release_hash, verify_release_with_trust_root, Release, TrustRoot, UpdateError};
use vdf::{VdfOutput, VdfProof};

pub const GATE: u64 = 15_087;
pub const SEED_MIN_PRODUCERS: usize = 5;
pub const ROTATION_START: u64 = 20_000;
pub const SNAP_HEIGHT: u64 = 25_000;
pub const RELEASE_VERSION: &str = "9.9.9";
pub const RELEASE_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub const PRODUCER_SEEDS: [u8; 5] = [1, 2, 3, 4, 5];
pub const ROTATED_SEEDS: [u8; 5] = [101, 102, 103, 104, 105];
pub const OUTSIDER_SEEDS: [u8; 3] = [201, 202, 203];

pub fn kp(seed: u8) -> KeyPair {
    KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

pub fn pk(seed: u8) -> PublicKey {
    *kp(seed).public_key()
}

pub fn producer_keys() -> Vec<KeyPair> {
    PRODUCER_SEEDS.iter().map(|s| kp(*s)).collect()
}

pub fn rotated_keys() -> Vec<KeyPair> {
    ROTATED_SEEDS.iter().map(|s| kp(*s)).collect()
}

pub fn tip_hash() -> Hash {
    crypto::hash::hash(b"INC-I-248 replay node tip")
}

pub fn assert_testnet_params() {
    let p = Network::Testnet.params();
    assert_eq!(
        p.maintainer_derivation_activation_height, GATE,
        "fixture: testnet maintainer_derivation_activation_height moved"
    );
    assert_eq!(
        p.maintainer_seed_min_producers, SEED_MIN_PRODUCERS,
        "fixture: testnet maintainer_seed_min_producers moved"
    );
}

const _: () = assert!(ROTATION_START > GATE && SNAP_HEIGHT > ROTATION_START);

/// `new_for_test` node switched to Testnet, with an attached EMPTY maintainer state
/// (what a node with no `maintainer_state.bin` loads).
pub async fn testnet_node(producers: &[KeyPair]) -> (Node, TempDir) {
    assert_testnet_params();
    let temp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
        .await
        .expect("Node::new_for_test failed");
    node.config.network = Network::Testnet;
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    (node, temp)
}

pub async fn state_of(node: &Node) -> MaintainerState {
    node.maintainer_state
        .as_ref()
        .expect("fixture: maintainer_state must be attached")
        .read()
        .await
        .clone()
}

pub fn disk_state_of(node: &Node) -> MaintainerState {
    MaintainerState::load(&node.config.data_dir).expect("maintainer_state.bin must load")
}

pub fn node_genesis(node: &Node) -> [u8; 32] {
    *node.params.genesis_hash.as_bytes()
}

/// Digest bound to the node's own genesis hash (what `getMaintainerSet` publishes).
pub fn node_digest(node: &Node, set: &MaintainerSet) -> [u8; 32] {
    maintainer_set_digest(set, &node_genesis(node))
}

pub fn sorted(keys: &[PublicKey]) -> Vec<PublicKey> {
    let mut v = keys.to_vec();
    v.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    v
}

pub fn is_producer_seed(set: &MaintainerSet) -> bool {
    let producers: Vec<PublicKey> = PRODUCER_SEEDS.iter().map(|s| pk(*s)).collect();
    sorted(&set.members) == sorted(&producers)
}

// ---------------------------------------------------------------------------
// Governance plan
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add(u8),
    Remove(u8),
    Activate(u32, u64),
}

#[derive(Clone, Debug)]
pub struct Step {
    pub label: &'static str,
    pub height: u64,
    pub op: Op,
    pub signers: Vec<u8>,
    pub accept: bool,
}

fn step(label: &'static str, height: u64, op: Op, signers: &[u8], accept: bool) -> Step {
    Step {
        label,
        height,
        op,
        signers: signers.to_vec(),
        accept,
    }
}

/// Producers P1..P5 = seeds 1..5, rotated S1..S5 = 101..105, outsiders X1..X3 = 201..203.
/// Remove-then-add; every accepted step is threshold-valid for the set it runs on.
pub fn rotation_plan() -> Vec<Step> {
    let h = ROTATION_START;
    vec![
        step("remove P1", h, Op::Remove(1), &[2, 3, 4], true),
        step(
            "add S1 by outsiders",
            h + 10,
            Op::Add(101),
            &[201, 202, 203],
            false,
        ),
        step("re-add member P2", h + 20, Op::Add(2), &[3, 4, 5], false),
        step(
            "activate 2-of-4",
            h + 30,
            Op::Activate(9, 4_000),
            &[2, 3],
            false,
        ),
        step(
            "activate by outsiders",
            h + 40,
            Op::Activate(9, 4_000),
            &[201, 202, 203],
            false,
        ),
        step(
            "activate 3-of-4",
            h + 45,
            Op::Activate(9, 4_000),
            &[2, 3, 4],
            true,
        ),
        step("add S1", h + 50, Op::Add(101), &[2, 3, 4], true),
        step("remove P2", h + 100, Op::Remove(2), &[3, 4, 5], true),
        step("remove P3", h + 110, Op::Remove(3), &[4, 5, 101], true),
        step(
            "remove P4 below min",
            h + 120,
            Op::Remove(4),
            &[5, 101],
            false,
        ),
        step("add S2", h + 130, Op::Add(102), &[4, 5, 101], true),
        step("add S3", h + 140, Op::Add(103), &[4, 5, 101], true),
        step("remove P4", h + 200, Op::Remove(4), &[5, 101, 102], true),
        step("add S4", h + 210, Op::Add(104), &[101, 102, 103], true),
        step("remove P5", h + 300, Op::Remove(5), &[101, 102, 103], true),
        step("add S5", h + 310, Op::Add(105), &[101, 102, 103], true),
        step(
            "activate by rotated",
            h + 400,
            Op::Activate(10, 5_000),
            &[101, 102, 103],
            true,
        ),
        step(
            "activate by producers",
            h + 410,
            Op::Activate(10, 5_000),
            &[1, 2, 3],
            false,
        ),
        step(
            "re-add P1 by producers",
            h + 420,
            Op::Add(1),
            &[2, 3, 4],
            false,
        ),
        step(
            "add X1 to full set",
            h + 430,
            Op::Add(201),
            &[101, 102, 103],
            false,
        ),
    ]
}

pub fn last_rotation_height() -> u64 {
    rotation_plan()
        .iter()
        .filter(|s| s.accept && !matches!(s.op, Op::Activate(..)))
        .map(|s| s.height)
        .max()
        .unwrap()
}

/// The bytes the node verifies a maintainer change against at `height`.
pub fn change_message(
    genesis: &[u8; 32],
    is_add: bool,
    target: &PublicKey,
    height: u64,
) -> Vec<u8> {
    signing_message_at(
        genesis,
        is_add,
        target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        height,
        Network::Testnet
            .params()
            .inc_i_176_auth_binding_activation_height,
    )
}

pub fn step_message(genesis: &[u8; 32], s: &Step) -> Vec<u8> {
    match s.op {
        Op::Add(t) => change_message(genesis, true, &pk(t), s.height),
        Op::Remove(t) => change_message(genesis, false, &pk(t), s.height),
        Op::Activate(v, e) => {
            ProtocolActivationData::new(v, e, String::new(), vec![]).signing_message()
        }
    }
}

pub fn step_signatures(genesis: &[u8; 32], s: &Step) -> Vec<MaintainerSignature> {
    let message = step_message(genesis, s);
    s.signers
        .iter()
        .map(|seed| {
            let k = kp(*seed);
            MaintainerSignature::new(
                *k.public_key(),
                crypto::signature::sign(&message, k.private_key()),
            )
        })
        .collect()
}

pub fn step_tx(genesis: &[u8; 32], s: &Step) -> Transaction {
    let sigs = step_signatures(genesis, s);
    match s.op {
        Op::Add(t) | Op::Remove(t) => Transaction {
            version: 1,
            tx_type: if matches!(s.op, Op::Add(_)) {
                TxType::AddMaintainer
            } else {
                TxType::RemoveMaintainer
            },
            inputs: vec![],
            outputs: vec![],
            extra_data: MaintainerChangeData::new(pk(t), sigs).to_bytes(),
        },
        Op::Activate(v, e) => Transaction::new_protocol_activation(ProtocolActivationData::new(
            v,
            e,
            String::new(),
            sigs,
        )),
    }
}

/// One block's worth: the governance tx, then the per-block seed hook (apply_block order).
pub async fn apply_step(node: &Node, s: &Step) -> Option<(u32, u64)> {
    let tx = step_tx(&node_genesis(node), s);
    let producers = node.producer_set.read().await.clone();
    let verdict = node
        .process_transaction_governance(&tx, s.height, &producers)
        .await;
    node.maybe_bootstrap_maintainer_set(s.height).await;
    verdict
}

// ---------------------------------------------------------------------------
// Node A (replay) and its snapshot
// ---------------------------------------------------------------------------

pub struct ReplayNode {
    pub node: Node,
    pub producers: Vec<KeyPair>,
    pub _temp: TempDir,
}

/// Node A: seeded at the gate from its 5 producers, rotated to S1..S5, tip at SNAP_HEIGHT.
pub async fn build_replay_node_a() -> ReplayNode {
    let producers = producer_keys();
    let (node, temp) = testnet_node(&producers).await;
    node.maybe_bootstrap_maintainer_set(GATE).await;
    assert!(
        is_producer_seed(&state_of(&node).await.set),
        "fixture: node A must seed its 5 producers at the gate"
    );
    for s in rotation_plan() {
        apply_step(&node, &s).await;
    }
    let set = state_of(&node).await.set;
    let rotated: Vec<PublicKey> = ROTATED_SEEDS.iter().map(|s| pk(*s)).collect();
    assert_eq!(
        sorted(&set.members),
        sorted(&rotated),
        "fixture: node A must end the plan holding S1..S5"
    );
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = SNAP_HEIGHT;
        cs.best_hash = tip_hash();
    }
    ReplayNode {
        node,
        producers,
        _temp: temp,
    }
}

pub async fn snapshot_of(node: &Node) -> VerifiedSnapshot {
    let pending_root_ah = node
        .config
        .network
        .params()
        .inc_i_181_pending_root_activation_height;
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let base = storage::StateSnapshot::create_gated(&cs, &utxo, &ps, pending_root_ah)
        .expect("fixture: StateSnapshot::create_gated must succeed");
    VerifiedSnapshot {
        block_hash: base.block_hash,
        block_height: base.block_height,
        chain_state: base.chain_state_bytes,
        utxo_set: base.utxo_set_bytes,
        utxo_staged: None,
        producer_set: base.producer_set_bytes,
        state_root: base.state_root,
        block_header_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: Some(node.epoch_state.serialize()),
    }
}

/// Node B: fresh Testnet node, no maintainer file, A's snapshot installed.
pub async fn snap_joined_node_b(snapshot: VerifiedSnapshot) -> (Node, TempDir) {
    let (mut node, temp) = testnet_node(&producer_keys()).await;
    assert!(
        !node.config.data_dir.join("maintainer_state.bin").exists(),
        "fixture: node B must start without a maintainer file"
    );
    let height = snapshot.block_height;
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("fixture: apply_snap_snapshot must succeed");
    let cs = node.chain_state.read().await;
    assert_eq!(
        cs.best_height, height,
        "fixture: node B must be at the snap height"
    );
    assert!(
        cs.is_snap_synced(),
        "fixture: node B must be marked snap-synced"
    );
    drop(cs);
    (node, temp)
}

/// Everything a peer answer carries, read from a node's live state.
pub struct PeerView {
    pub block_hash: Hash,
    pub block_height: u64,
    pub set: MaintainerSet,
    pub digest: [u8; 32],
    pub set_bytes: Vec<u8>,
    pub last_derived_height: u64,
}

pub async fn peer_view(node: &Node) -> PeerView {
    let state = state_of(node).await;
    let cs = node.chain_state.read().await;
    PeerView {
        block_hash: cs.best_hash,
        block_height: cs.best_height,
        digest: node_digest(node, &state.set),
        set_bytes: bincode::serialize(&state.set).expect("MaintainerSet must serialize"),
        set: state.set,
        last_derived_height: state.last_derived_height,
    }
}

// ---------------------------------------------------------------------------
// Release verification against a node's resolved root
// ---------------------------------------------------------------------------

pub fn release_signed_by(signers: &[KeyPair]) -> Release {
    Release {
        version: RELEASE_VERSION.to_string(),
        binary_sha256: RELEASE_SHA256.to_string(),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        signatures: signers
            .iter()
            .map(|k| sign_release_hash(k, RELEASE_VERSION, RELEASE_SHA256))
            .collect(),
        target_networks: Vec::new(),
    }
}

pub async fn resolved_root(node: &Node) -> TrustRoot {
    doli_node::updater::resolve_trust_root(&state_of(node).await, node.config.network)
}

pub fn verdict_name(result: &Result<usize, UpdateError>) -> String {
    match result {
        Ok(n) => format!("Ok({n})"),
        Err(UpdateError::TrustRootUnavailable { .. }) => "TrustRootUnavailable".to_string(),
        Err(UpdateError::InsufficientSignatures { found, required }) => {
            format!("InsufficientSignatures({found}/{required})")
        }
        Err(e) => format!("Err({e})"),
    }
}

/// Verify a release signed by S1, S2, S3 against the node's resolved root.
pub async fn verify_rotated_release(node: &Node) -> Result<usize, UpdateError> {
    let signers: Vec<KeyPair> = rotated_keys().into_iter().take(3).collect();
    verify_release_with_trust_root(&release_signed_by(&signers), &resolved_root(node).await)
}

// ---------------------------------------------------------------------------
// Peer answers
// ---------------------------------------------------------------------------

pub fn answer_of(view: &PeerView, digest: [u8; 32]) -> PeerMaintainerAnswer {
    PeerMaintainerAnswer {
        set: view.set.clone(),
        last_derived_height: view.last_derived_height,
        digest,
        tip_height: view.block_height,
    }
}

/// Deliver `response` once from each of `peers` distinct fresh peer ids.
pub async fn deliver(node: &mut Node, response: &SyncResponse, peers: usize) {
    for _ in 0..peers {
        let event = network::NetworkEvent::SyncResponse {
            peer_id: network::PeerId::random(),
            response: response.clone(),
        };
        let _ = node.handle_network_event(event).await;
    }
}

/// Node on another chain: own genesis hash, A's maintainer state, tip at SNAP_HEIGHT.
pub async fn foreign_genesis_node(state: MaintainerState) -> (Node, TempDir) {
    let (mut node, temp) = testnet_node(&producer_keys()).await;
    node.config.network = Network::Devnet;
    let foreign = crypto::hash::hash(b"INC-I-248 foreign chain genesis");
    node.params.genesis_hash = foreign;
    {
        let mut cs = node.chain_state.write().await;
        cs.genesis_hash = foreign;
        cs.best_height = SNAP_HEIGHT;
        cs.best_hash = tip_hash();
    }
    node.set_maintainer_state(Arc::new(RwLock::new(state)));
    (node, temp)
}

// ---------------------------------------------------------------------------
// Real blocks on the node's tip
// ---------------------------------------------------------------------------

/// Coinbase-only child of the node's tip, built on the node's network params.
pub async fn tip_child(node: &Node, producer: &KeyPair) -> Block {
    let (prev_hash, height) = {
        let cs = node.chain_state.read().await;
        (cs.best_hash, cs.best_height + 1)
    };
    let params = ConsensusParams::for_network(node.config.network);
    let slot = height as u32;
    let coinbase = Transaction::new_coinbase(
        node.params.block_reward(height),
        doli_core::consensus::reward_pool_pubkey_hash(),
        height,
        0,
    );
    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: params.genesis_hash,
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
    Block::new(header, vec![coinbase])
}

/// Consensus params of the node's network, keeping the node's genesis hash (digest binding).
pub fn align_block_params(node: &mut Node) {
    let genesis = node.params.genesis_hash;
    node.params = ConsensusParams::for_network(node.config.network);
    node.params.genesis_hash = genesis;
}

/// Apply one coinbase-only block on the tip through `apply_block` (Light), trying each
/// producer until the scheduled one is accepted. Returns the applied height.
pub async fn apply_next_block(node: &mut Node, producers: &[KeyPair]) -> u64 {
    align_block_params(node);
    let before = node.chain_state.read().await.best_height;
    let mut errors = Vec::new();
    for producer in producers {
        let block = tip_child(node, producer).await;
        match node.apply_block(block, ValidationMode::Light).await {
            Ok(()) => {
                let after = node.chain_state.read().await.best_height;
                assert_eq!(
                    after,
                    before + 1,
                    "fixture: apply_block must advance the tip"
                );
                return after;
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    panic!(
        "fixture: no producer could apply a block at h={}: {errors:?}",
        before + 1
    );
}

/// Replay node seeded at the gate from its producers, tip at SNAP_HEIGHT, never armed.
pub async fn replay_node_at_snap_height() -> (Node, TempDir) {
    let (node, temp) = testnet_node(&producer_keys()).await;
    node.maybe_bootstrap_maintainer_set(GATE).await;
    assert!(
        is_producer_seed(&state_of(&node).await.set),
        "fixture: replay node must seed its 5 producers at the gate"
    );
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = SNAP_HEIGHT;
        cs.best_hash = tip_hash();
    }
    (node, temp)
}
