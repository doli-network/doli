//! INC-I-237 M2 — node fixtures: a test node, admissible signed transfers, a
//! signature-malleated variant (same `tx.hash()`, different bytes), and real
//! `NetworkService`s on 127.0.0.1.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.

use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_node::node::Node;
use network::{NetworkConfig, NetworkEvent, NetworkService};
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;

pub const FUNDING: u64 = 10_000_000;

pub async fn make_node() -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    (node, kp, temp)
}

fn pkh(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// Seed one spendable UTXO owned by `kp` into both state stores.
pub async fn seed(node: &Node, kp: &KeyPair, prev: Hash) {
    let entry = UtxoEntry {
        output: Output::normal(FUNDING, pkh(kp)),
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let op = Outpoint::new(prev, 0);
    node.state_db
        .insert_utxo(&op, &entry)
        .expect("seed state_db");
    node.utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("seed utxo_set");
}

/// A valid, fee-paying transfer spending `prev:0` (seed it first to make it admissible).
pub fn transfer(kp: &KeyPair, prev: Hash) -> Transaction {
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: prev,
            output_index: 0,
            signature: crypto::Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        }],
        outputs: vec![Output::normal(0, pkh(kp))],
        extra_data: vec![],
    };
    tx.outputs[0].amount = FUNDING - tx.minimum_fee();
    let sh = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    tx
}

/// Same `tx.hash()` (signatures are excluded), different bytes, invalid signature.
pub fn malleate(tx: &Transaction) -> Transaction {
    let mut m = tx.clone();
    m.inputs[0].signature = crypto::Signature::from_bytes([0x5A; 64]);
    m
}

pub fn prev(tag: u8) -> Hash {
    Hash::from_bytes([tag; 32])
}

pub fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve port")
        .port()
}

pub fn addr(port: u16) -> String {
    format!("/ip4/127.0.0.1/tcp/{port}")
}

pub async fn start_net(port: u16, bootstrap_nodes: Vec<String>) -> NetworkService {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let listen_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let config = NetworkConfig {
        listen_addr,
        bootstrap_nodes,
        max_peers: 8,
        bootstrap_slots: 8,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        memory_watchdog_threshold_bytes: 0,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

/// Raw bytes of every tx C's node saw, one inner Vec per gossip event.
pub type TxLog = Arc<Mutex<Vec<Vec<Vec<u8>>>>>;

pub fn txs_of(ev: &NetworkEvent) -> Option<Vec<Transaction>> {
    match ev {
        NetworkEvent::NewTransaction(txs, _) => Some(txs.clone()),
        _ => None,
    }
}

/// Spawn a passive observer node that logs every tx gossip event.
pub fn observe(mut svc: NetworkService) -> TxLog {
    let log: TxLog = Arc::default();
    let out = log.clone();
    tokio::spawn(async move {
        while let Some(ev) = svc.next_event().await {
            if let Some(txs) = txs_of(&ev) {
                out.lock()
                    .unwrap()
                    .push(txs.iter().map(|t| t.serialize()).collect());
            }
        }
    });
    log
}

/// Events in `log` that carry a tx with `hash`.
pub fn events_with(log: &TxLog, hash: Hash) -> Vec<Vec<Vec<u8>>> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|ev| {
            ev.iter()
                .any(|b| Transaction::deserialize(b).map(|t| t.hash()) == Some(hash))
        })
        .cloned()
        .collect()
}

/// Feed `node` the tx gossip events of its own network for `dur` through the
/// production dispatch; other events are dropped (no peer management).
pub async fn pump(node: &mut Node, dur: Duration) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = node.network.as_mut().expect("node network attached");
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        {
            if txs_of(&ev).is_some() {
                let _ = node.handle_network_event(ev).await;
            }
        }
    }
}
