//! INC-I-195 — `submitMaintainerChange` must BROADCAST, not only enqueue.
//!
//! The handler ended at `mempool.add_system_transaction` and returned
//! `{"status":"accepted"}` without ever calling `broadcast_tx`, so the
//! transaction never left the node that received the RPC. The ordinary
//! transaction path (`transaction.rs`) routes the same 0-in/0-out shape to the
//! system lane and THEN broadcasts; the gossip ingest path
//! (`bins/node/src/node/validation_checks.rs:1253-1259`) admits it on the
//! receiving side. Only the broadcast call was missing.
//!
//! MEASURED consequence (local testnet, 2026-08-29): a removal submitted to the
//! seed (`--relay-server`, no `--producer`) answered `accepted` and the
//! maintainer set was unchanged ~36 blocks later. The identical rotation
//! against a producer applied in 4-5 blocks per transaction, 10 of 10. A
//! mainnet operator reaches for a seed RPC, and mainnet seeds are relays.
//!
//! OUTPUT CONTRACT: fn RpcContext::submit_maintainer_change(&self, params) -> Result<Value, RpcError>
//!   O1: Ok(accepted) AND the tx handed to `broadcast_tx` exactly once
//!   O2: Err(invalid_params) AND `broadcast_tx` NOT called
//! PATHS: valid add | valid remove | bad action | too few signatures
//! INPUT PARTITIONS:
//!   P1: action="add", 3 well-formed signatures
//!   P2: action="remove", 3 well-formed signatures, with reason
//!   P3: action="transfer" (not add/remove)
//!   P4: action="add", 2 signatures (below MAINTAINER_THRESHOLD)
//! MATRIX: P1→O1, P2→O1, P3→O2, P4→O2

use std::sync::{Arc, Mutex};

use super::*;
use doli_core::network::Network;
use doli_core::Transaction;
use mempool::{Mempool, MempoolPolicy};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

struct Harness {
    ctx: RpcContext,
    broadcast: Arc<Mutex<Vec<Transaction>>>,
    _tempdir: TempDir,
}

fn build_harness() -> Harness {
    let tempdir = TempDir::new().expect("tempdir");
    let chain_state = Arc::new(RwLock::new(ChainState::new(crypto::Hash::ZERO)));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let params = doli_core::consensus::ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        Network::Mainnet,
    )));

    let broadcast: Arc<Mutex<Vec<Transaction>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = broadcast.clone();
    let ctx = RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params,
        Network::Mainnet,
    )
    .with_broadcast(move |tx| sink.lock().expect("broadcast sink").push(tx));

    Harness {
        ctx,
        broadcast,
        _tempdir: tempdir,
    }
}

/// A deterministic, genuinely valid Ed25519 public key. The handler PARSES the
/// key material, so a repeated-byte string is rejected before the relay is even
/// reached; the keys must be real curve points. The quorum itself is verified on
/// the apply path, not here, so these need not authorize anything.
fn keypair(seed_byte: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_seed([seed_byte; 32])
}

fn pk(seed_byte: u8) -> String {
    keypair(seed_byte).public_key().to_hex()
}

fn sigs(n: usize) -> Vec<serde_json::Value> {
    (0..n)
        .map(|i| {
            let kp = keypair(i as u8 + 1);
            let sig = crypto::signature::sign(b"inc-i-195 relay fixture", kp.private_key());
            serde_json::json!({
                "pubkey": kp.public_key().to_hex(),
                "signature": sig.to_hex(),
            })
        })
        .collect()
}

/// P1 → O1. The acceptance test: an accepted `add` must reach the network.
#[tokio::test]
async fn inc_i195_add_is_broadcast_not_only_enqueued() {
    let h = build_harness();
    let params = serde_json::json!({
        "action": "add",
        "target_pubkey": pk(0xaa),
        "signatures": sigs(3),
    });
    let out = h
        .ctx
        .submit_maintainer_change(params)
        .await
        .expect("submit accepted");
    assert_eq!(out["status"], "accepted");

    let sent = h.broadcast.lock().expect("sink").clone();
    assert_eq!(
        sent.len(),
        1,
        "an accepted maintainer change MUST be broadcast exactly once, \
         otherwise it dies in the mempool of whichever node received the RPC"
    );
    assert_eq!(
        sent[0].hash().to_hex(),
        out["tx_hash"].as_str().expect("tx_hash"),
        "the broadcast tx must be the tx the caller was told was accepted"
    );
}

/// P2 → O1. The remove arm carries a `reason` and must relay identically.
#[tokio::test]
async fn inc_i195_remove_is_broadcast_not_only_enqueued() {
    let h = build_harness();
    let params = serde_json::json!({
        "action": "remove",
        "target_pubkey": pk(0xbb),
        "signatures": sigs(3),
        "reason": "INC-I-175 rotation",
    });
    let out = h
        .ctx
        .submit_maintainer_change(params)
        .await
        .expect("submit accepted");
    assert_eq!(out["status"], "accepted");
    assert_eq!(
        h.broadcast.lock().expect("sink").len(),
        1,
        "an accepted maintainer removal MUST be broadcast exactly once"
    );
}

/// P3 → O2 and P4 → O2. A rejected submission must not put anything on the
/// wire: the relay is a consequence of acceptance, never of receipt.
#[tokio::test]
async fn inc_i195_rejected_submissions_broadcast_nothing() {
    for params in [
        serde_json::json!({
            "action": "transfer",
            "target_pubkey": pk(0xcc),
            "signatures": sigs(3),
        }),
        serde_json::json!({
            "action": "add",
            "target_pubkey": pk(0xdd),
            "signatures": sigs(2),
        }),
    ] {
        let h = build_harness();
        assert!(
            h.ctx
                .submit_maintainer_change(params.clone())
                .await
                .is_err(),
            "{params} must be rejected"
        );
        assert!(
            h.broadcast.lock().expect("sink").is_empty(),
            "a rejected submission must broadcast nothing"
        );
    }
}
