//! INC-I-255 M1 shared fixtures: devnet nodes, signed double-production
//! evidence, hand-built slash transactions and per-path verdict probes.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::block::{compute_merkle_root, Block, BlockHeader};
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{SlashData, SlashingEvidence, Transaction};
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::Network;
use doli_node::node::Node;
use mempool::MempoolError;
use network::EquivocationProof;
use storage::PendingProducerUpdate;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_178_m0_common::wait_for_fresh_second;
use super::inc_i_236_m4_common::{feed, header_block};
use crate::block_sig::sign_header;
use crate::inc_i_156_m1_harness::install_production_utxo_backend;

pub const SLOT: u32 = 7;
pub const MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];
pub const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_255_M1_CHILD";

/// Devnet node whose key is `reporter`; `accused` is a registered genesis producer.
pub struct World {
    pub node: Node,
    pub reporter: KeyPair,
    pub accused: KeyPair,
    _tmp: TempDir,
}

impl World {
    pub fn accused_pk(&self) -> PublicKey {
        *self.accused.public_key()
    }
}

pub async fn world_with(reporter: &KeyPair, accused: &KeyPair) -> World {
    let tmp = TempDir::new().unwrap();
    let node = Node::new_for_test(
        tmp.path().to_path_buf(),
        vec![reporter.clone(), accused.clone()],
    )
    .await
    .expect("Node::new_for_test failed");
    install_production_utxo_backend(&node).await;
    World {
        node,
        reporter: reporter.clone(),
        accused: accused.clone(),
        _tmp: tmp,
    }
}

pub async fn world() -> World {
    world_with(&KeyPair::generate(), &KeyPair::generate()).await
}

pub fn chain_genesis() -> Hash {
    doli_core::chainspec::ChainSpec::devnet().genesis_hash()
}

/// Header by `kp` at `slot` with a valid devnet VDF, signed after every field is set.
pub fn evidence_header(
    kp: &KeyPair,
    genesis: Hash,
    slot: u32,
    salt: u8,
    missed: usize,
) -> BlockHeader {
    let producer = *kp.public_key();
    let prev_hash = Hash::from_bytes([salt; 32]);
    let merkle_root = compute_merkle_root(&[]);
    let input = vdf::block_input(&prev_hash, &merkle_root, slot, &producer);
    let value = hash_chain_vdf(&input, Network::Devnet.heartbeat_vdf_iterations()).to_vec();
    let mut h = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: slot as u64,
        slot,
        producer,
        vdf_output: VdfOutput { value },
        vdf_proof: VdfProof::empty(),
        missed_producers: vec![producer; missed],
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    sign_header(&mut h, kp);
    h
}

/// Two conflicting signed headers by `kp` for `SLOT`.
pub fn pair(kp: &KeyPair, genesis: Hash) -> (BlockHeader, BlockHeader) {
    (
        evidence_header(kp, genesis, SLOT, 0xA1, 0),
        evidence_header(kp, genesis, SLOT, 0xB2, 0),
    )
}

pub fn by_hash(a: BlockHeader, b: BlockHeader) -> (BlockHeader, BlockHeader) {
    if a.hash().as_bytes() <= b.hash().as_bytes() {
        (a, b)
    } else {
        (b, a)
    }
}

pub fn slash_tx(
    accused: PublicKey,
    h1: BlockHeader,
    h2: BlockHeader,
    sig: Signature,
) -> Transaction {
    Transaction::new_slash_producer(SlashData {
        producer_pubkey: accused,
        evidence: SlashingEvidence::DoubleProduction {
            block_header_1: h1,
            block_header_2: h2,
        },
        reporter_signature: sig,
    })
}

/// Headers ordered by hash ascending, default reporter signature.
pub fn canonical(accused: PublicKey, a: BlockHeader, b: BlockHeader) -> Transaction {
    let (lo, hi) = by_hash(a, b);
    slash_tx(accused, lo, hi, Signature::default())
}

pub fn valid_slash(w: &World) -> Transaction {
    let (a, b) = pair(&w.accused, chain_genesis());
    canonical(w.accused_pk(), a, b)
}

/// The node's normal-lane admission call, with the node's arguments.
pub async fn admit(node: &Node, tx: &Transaction) -> Result<(), MempoolError> {
    let (h, s) = {
        let st = node.chain_state.read().await;
        (st.best_height, st.best_slot)
    };
    let mut pool = node.mempool.write().await;
    let utxo = node.utxo_set.read().await;
    pool.add_transaction(tx.clone(), &utxo, h, s).map(|_| ())
}

/// Normal lane, else the fee-free system lane; true when the tx is resident.
pub async fn inject(node: &Node, tx: &Transaction) -> bool {
    if admit(node, tx).await.is_ok() {
        return true;
    }
    let h = node.chain_state.read().await.best_height;
    node.mempool
        .write()
        .await
        .add_system_transaction(tx.clone(), h, None)
        .is_ok()
}

/// Feeds two signed sibling blocks by the accused through gossip intake;
/// returns (resident slash ids naming the accused, canonical id).
pub async fn handler_slashes(w: &mut World) -> (Vec<Hash>, Hash) {
    let genesis = w.node.chain_state.read().await.best_hash;
    let pk = w.accused_pk();
    let a = header_block(&mut w.node, 1, SLOT, genesis, &pk, Some(&w.accused), 0).await;
    let b = header_block(&mut w.node, 1, SLOT, genesis, &pk, Some(&w.accused), 1).await;
    assert_ne!(a.hash(), b.hash(), "FIXTURE: sibling headers must conflict");
    let expected = canonical(pk, a.header.clone(), b.header.clone()).hash();
    feed(&mut w.node, a).await;
    feed(&mut w.node, b).await;
    (resident_slashes(&w.node, &pk).await, expected)
}

pub fn proof_of(w: &World, a: BlockHeader, b: BlockHeader) -> EquivocationProof {
    EquivocationProof {
        producer: w.accused_pk(),
        block_header_1: a,
        block_header_2: b,
        slot: SLOT,
    }
}

pub async fn resident_slashes(node: &Node, accused: &PublicKey) -> Vec<Hash> {
    let pool = node.mempool.read().await;
    pool.iter()
        .filter(|(_, e)| {
            e.tx.slash_data()
                .is_some_and(|d| d.producer_pubkey == *accused)
        })
        .map(|(h, _)| *h)
        .collect()
}

/// Real builder block for the next height with `extra` appended; also the
/// hashes the builder itself selected.
pub async fn full_block_with(node: &mut Node, extra: &[Transaction]) -> (Block, Vec<Hash>) {
    let kp = node.producer_key.clone().expect("FIXTURE: producer key");
    let pk = *kp.public_key();
    let (prev_hash, prev_slot, height) = {
        let st = node.chain_state.read().await;
        (st.best_hash, st.best_slot, st.best_height + 1)
    };
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(prev_hash, prev_slot, height, slot, pk)
            .await
            .expect("FIXTURE: build_block_content returned Err");
        let Some((header, mut txs, bf)) = built else {
            continue;
        };
        let selected = txs.iter().map(|t| t.hash()).collect();
        txs.extend_from_slice(extra);
        let mut header = BlockHeader {
            merkle_root: compute_merkle_root(&txs),
            ..header
        };
        let (vdf_output, vdf_proof) = node
            .compute_block_vdf(&prev_hash, &header)
            .await
            .expect("FIXTURE: compute_block_vdf returned Err");
        header.vdf_output = vdf_output;
        header.vdf_proof = vdf_proof;
        sign_header(&mut header, &kp);
        let block = Block {
            header,
            transactions: txs,
            aggregate_bls_signature: Vec::new(),
            attestation_bitfield: bf,
        };
        return (block, selected);
    }
    panic!("FIXTURE: build_block_content returned None on every attempt");
}

pub async fn tip(node: &Node) -> (u64, Hash) {
    let st = node.chain_state.read().await;
    (st.best_height, st.best_hash)
}

pub async fn tip_root(node: &Node) -> (u64, Hash, Hash) {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let root = storage::compute_state_root(&cs, &utxo, &ps).expect("state root");
    (cs.best_height, cs.best_hash, root)
}

/// Slash queued for `pk`, or already flushed to the Slashed status.
pub async fn slash_recorded(node: &Node, pk: &PublicKey) -> bool {
    let ps = node.producer_set.read().await;
    ps.get_by_pubkey(pk).is_some_and(|p| p.is_slashed())
        || ps
            .pending_updates_for(pk)
            .iter()
            .any(|u| matches!(u, PendingProducerUpdate::Slash { .. }))
}

pub fn mempool_slash_msg(r: &Result<(), MempoolError>) -> Option<String> {
    match r {
        Err(MempoolError::Validation(ValidationError::InvalidSlash(m))) => Some(m.clone()),
        _ => None,
    }
}

pub fn block_slash_msg(r: &Result<(), ValidationError>) -> Option<String> {
    match r {
        Err(ValidationError::InvalidSlash(m)) => Some(m.clone()),
        _ => None,
    }
}

pub fn apply_slash_msg(r: &anyhow::Result<()>) -> Option<String> {
    r.as_ref()
        .err()?
        .chain()
        .find_map(|e| match e.downcast_ref() {
            Some(ValidationError::InvalidSlash(m)) => Some(m.clone()),
            _ => None,
        })
}

pub fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-exec `test` alone with the devnet slash gate set to `gate` and the
/// INC-I-257 unbonding lock closed (the Run B world: Normal payouts, Bond Transfers).
pub fn run_in_child(test: &str, gate: u64) {
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(GATE_ENV, gate.to_string())
        .env(
            "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
            u64::MAX.to_string(),
        )
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child {test} at slash gate {gate} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
