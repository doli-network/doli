//! INC-I-237 M2 — shared harness: real `NetworkService`s on 127.0.0.1 and a
//! raw gossipsub peer that plays an OLD-version node (publishes any bytes,
//! records every message it receives with its gossipsub message id).
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.
#![allow(dead_code)] // each consumer uses a subset

use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance, MessageId};
use libp2p::swarm::SwarmEvent;
use libp2p::{identity::Keypair, Multiaddr, PeerId, Swarm};
use network::gossip::{new_gossipsub, MeshConfig, TRANSACTIONS_TOPIC};
use network::transport::build_transport;
use network::{NetworkConfig, NetworkService};
use tokio::sync::mpsc;

pub fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve port")
        .port()
}

pub fn addr(port: u16) -> String {
    format!("/ip4/127.0.0.1/tcp/{port}")
}

pub async fn start_node(port: u16, bootstrap_nodes: Vec<String>) -> NetworkService {
    start_node_cfg(port, bootstrap_nodes, false).await
}

/// `announce` = the dormant `tx_announce_enabled` config path.
pub async fn start_node_cfg(
    port: u16,
    bootstrap_nodes: Vec<String>,
    announce: bool,
) -> NetworkService {
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
        tx_announce_enabled: announce,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

/// A test tx whose identity is `tag` (outputs[0].amount).
pub fn tx(tag: u64) -> Transaction {
    Transaction::new_coinbase(tag, Hash::default(), tag, tag as u32)
}

pub fn tag(t: &Transaction) -> u64 {
    t.outputs.first().map(|o| o.amount).unwrap_or(0)
}

/// The gossipsub message id every DOLI node computes (BLAKE3, first 20 bytes).
pub fn doli_message_id(data: &[u8]) -> Vec<u8> {
    crypto::hash::hash(data).as_bytes()[..20].to_vec()
}

/// One message a raw peer received: (message id bytes, raw data).
pub type RawLog = Arc<Mutex<Vec<(Vec<u8>, Vec<u8>)>>>;

/// A gossipsub-only peer on the tx topic: no DOLI behaviours, no verdict
/// logic (it reports Ignore for everything it receives, so it never relays).
pub struct RawPeer {
    pub peer_id: PeerId,
    publish: mpsc::UnboundedSender<Vec<u8>>,
    pub received: RawLog,
}

impl RawPeer {
    pub fn publish(&self, data: Vec<u8>) {
        self.publish.send(data).expect("raw peer task alive");
    }

    pub fn received_ids(&self) -> Vec<Vec<u8>> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }
}

pub async fn raw_peer(dial: &str) -> RawPeer {
    let kp = Keypair::generate_ed25519();
    let peer_id = PeerId::from(kp.public());
    let mesh = MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };
    let behaviour = new_gossipsub(&kp, &mesh).expect("production gossipsub");
    let mut swarm: Swarm<gossipsub::Behaviour> = Swarm::new(
        build_transport(&kp, None).expect("transport"),
        behaviour,
        peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(300)),
    );
    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
    swarm.behaviour_mut().subscribe(&topic).unwrap();
    swarm.dial(dial.parse::<Multiaddr>().unwrap()).unwrap();
    let (publish, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let received: RawLog = Arc::default();
    let log = received.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                ev = swarm.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        propagation_source, message_id, message,
                    }) = ev {
                        log.lock().unwrap().push((message_id.0.clone(), message.data));
                        let _ = swarm.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, MessageAcceptance::Ignore,
                        );
                    }
                }
                Some(data) = rx.recv() => {
                    let _ = swarm.behaviour_mut().publish(topic.clone(), data);
                }
            }
        }
    });
    RawPeer {
        peer_id,
        publish,
        received,
    }
}

/// Raw id of a message, for logs.
pub fn id_hex(id: &MessageId) -> String {
    id.0.iter().map(|b| format!("{b:02x}")).collect()
}

/// Poll `probe` every 50 ms until it is true or `within` elapses.
pub async fn wait_until(within: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
