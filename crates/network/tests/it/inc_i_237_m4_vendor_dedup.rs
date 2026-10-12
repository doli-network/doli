//! INC-I-237 M4 (R6) — the vendor duplicate cache is the only gossip dedup:
//! it outlives the deleted 180 s app SeenCache and the attestation age window,
//! drops an in-window duplicate before the app, and its id (BLAKE3 of the full
//! raw bytes) gives every forged variant its own id (INV-NETWORK-004).
//! All tests here are regression pins (PASS before and after M4).
//!
//! covers: config staleness
//!
//! OUTPUT CONTRACT: production gossipsub (new_gossipsub + subscribe_to_topics)
//!   O3: publish() -> Result<MessageId, PublishError> (Duplicate = dup-cache hit)
//!   O6: Event::Message delivered to the hub app (id, data), counted per payload
//!   O5: consts GOSSIP_DUPLICATE_CACHE_TIME, ATTEST_STALE_SLOTS, slot_duration
//! PATHS: P1 forged variant first, then genuine; P2 identical bytes from two
//!   origins inside the window; P3 constant bounds
//! INPUT PARTITIONS: payload {attestation sig-forged, attestation BLS vs non-BLS,
//!   tx legacy single (same txid), tx 0xBA batch (same txid), vote sig-forged};
//!   origin {same publisher, two publishers (different source/seqno)};
//!   network {mainnet, testnet, devnet slot duration}
//! MATRIX: P1 x 5 payload partitions x {O3 both Ok + ids differ, O6 both
//!   delivered once}; P2 x {O3 both Ok + same id, O6 delivered once}; P3 x 3 networks.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{KeyPair, Signature};
use doli_core::{Network, NetworkParams};
use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance};
use libp2p::swarm::SwarmEvent;
use libp2p::{identity::Keypair, Multiaddr, PeerId, Swarm};
use network::gossip::staleness::ATTEST_STALE_SLOTS;
use network::gossip::{
    encode_tx_batch, new_gossipsub, subscribe_to_topics, MeshConfig, ATTESTATION_TOPIC,
    GOSSIP_DUPLICATE_CACHE_TIME, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use network::transport::build_transport;
use tokio::sync::{mpsc, oneshot};

use super::inc_i_237_m3_fake_node::{attestation, forge, now_secs};

type Log = Arc<Mutex<Vec<(Vec<u8>, Vec<u8>)>>>;
type PublishCmd = (String, Vec<u8>, oneshot::Sender<Result<Vec<u8>, String>>);

/// One production-config gossipsub swarm; it reports Ignore for everything, so it never forwards.
struct Peer {
    id: PeerId,
    addr: Option<Multiaddr>,
    cmd: mpsc::UnboundedSender<PublishCmd>,
    got: Log,
    subs: Arc<Mutex<HashSet<(PeerId, String)>>>,
}

impl Peer {
    async fn publish(&self, topic: &str, data: Vec<u8>) -> Result<Vec<u8>, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send((topic.to_string(), data, tx))
            .expect("peer task alive");
        rx.await.expect("publish reply")
    }

    fn deliveries(&self, data: &[u8]) -> Vec<Vec<u8>> {
        let got = self.got.lock().unwrap();
        got.iter()
            .filter(|(_, d)| d == data)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

async fn peer(dial: Option<&Multiaddr>) -> Peer {
    let kp = Keypair::generate_ed25519();
    let id = PeerId::from(kp.public());
    let mesh = MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };
    let mut behaviour = new_gossipsub(&kp, &mesh).expect("production gossipsub");
    subscribe_to_topics(&mut behaviour).expect("subscribe");
    let mut swarm: Swarm<gossipsub::Behaviour> = Swarm::new(
        build_transport(&kp, None).expect("transport"),
        behaviour,
        id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(60)),
    );
    let mut addr = None;
    match dial {
        Some(a) => swarm.dial(a.clone()).expect("dial"),
        None => {
            swarm
                .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
                .expect("listen");
            while addr.is_none() {
                if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
                    addr = Some(address);
                }
            }
        }
    }
    let (cmd, mut rx) = mpsc::unbounded_channel::<PublishCmd>();
    let got: Log = Arc::default();
    let subs: Arc<Mutex<HashSet<(PeerId, String)>>> = Arc::default();
    let (log, seen_subs) = (got.clone(), subs.clone());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                ev = swarm.select_next_some() => match ev {
                    SwarmEvent::Behaviour(gossipsub::Event::Message {
                        propagation_source, message_id, message,
                    }) => {
                        log.lock().unwrap().push((message_id.0.clone(), message.data));
                        let _ = swarm.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, MessageAcceptance::Ignore,
                        );
                    }
                    SwarmEvent::Behaviour(gossipsub::Event::Subscribed { peer_id, topic }) => {
                        seen_subs.lock().unwrap().insert((peer_id, topic.as_str().to_string()));
                    }
                    _ => {}
                },
                Some((topic, data, reply)) = rx.recv() => {
                    let r = swarm
                        .behaviour_mut()
                        .publish(IdentTopic::new(topic), data)
                        .map(|id| id.0)
                        .map_err(|e| format!("{e:?}"));
                    let _ = reply.send(r);
                }
            }
        }
    });
    Peer {
        id,
        addr,
        cmd,
        got,
        subs,
    }
}

async fn wait_until(within: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while !probe() {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

/// A spoke dialled into `hub`, returned once it has seen the hub join `topic`.
async fn spoke(hub: &Peer, topic: &str) -> Peer {
    let s = peer(hub.addr.as_ref()).await;
    let key = (hub.id, topic.to_string());
    let joined = wait_until(Duration::from_secs(15), || {
        s.subs.lock().unwrap().contains(&key)
    })
    .await;
    assert!(
        joined,
        "setup: spoke never saw the hub subscribe to {topic}"
    );
    s
}

/// P1: the spoke publishes `forged` first, then `genuine`; the dup cache must not suppress the genuine.
async fn assert_forged_first_leaves_genuine(topic: &str, forged: Vec<u8>, genuine: Vec<u8>) {
    assert_ne!(
        forged, genuine,
        "precondition: the variants differ in bytes"
    );
    let hub = peer(None).await;
    let s = spoke(&hub, topic).await;
    let id_f = s
        .publish(topic, forged.clone())
        .await
        .expect("forged published");
    let id_g = s
        .publish(topic, genuine.clone())
        .await
        .expect("genuine refused as a duplicate of the forged variant");
    assert_ne!(
        id_f, id_g,
        "forged and genuine share a gossipsub message id"
    );
    let both = wait_until(Duration::from_secs(10), || {
        !hub.deliveries(&forged).is_empty() && !hub.deliveries(&genuine).is_empty()
    })
    .await;
    assert!(both, "hub app did not receive both variants");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        hub.deliveries(&genuine),
        vec![id_g],
        "genuine delivered once, same id"
    );
    assert_eq!(
        hub.deliveries(&forged),
        vec![id_f],
        "forged delivered once, same id"
    );
}

// REQ-GRV-013 — Decision: a failure means the vendor duplicate cache expires before the deleted 180 s SeenCache TTL or inside the attestation age window, so R6 re-opens the 60-120 s re-delivery storm.
#[test]
fn req_grv_013_duplicate_cache_outlives_seen_cache_ttl_and_attestation_window() {
    let deleted_seen_cache_ttl = Duration::from_secs(180); // v6.43.3 SEEN_CACHE_TTL_SECS
    assert!(GOSSIP_DUPLICATE_CACHE_TIME >= deleted_seen_cache_ttl);
    let slots: Vec<u64> = [Network::Mainnet, Network::Testnet, Network::Devnet]
        .into_iter()
        .map(|n| NetworkParams::defaults(n).slot_duration)
        .collect();
    let max_slot = *slots.iter().max().expect("networks");
    assert!(max_slot > 0, "slot durations {slots:?}");
    let window = Duration::from_secs(u64::from(ATTEST_STALE_SLOTS) * max_slot);
    assert!(
        GOSSIP_DUPLICATE_CACHE_TIME >= window,
        "dup cache {GOSSIP_DUPLICATE_CACHE_TIME:?} < {ATTEST_STALE_SLOTS} slots x {max_slot} s"
    );
}

// REQ-GRV-013 — Decision: a failure means an identical message from a second origin inside the window reaches the app twice, so without the SeenCache every duplicate would be re-judged and re-forwarded.
#[tokio::test]
async fn req_grv_013_in_window_duplicate_from_second_origin_reaches_app_once() {
    let hub = peer(None).await;
    let a = spoke(&hub, TRANSACTIONS_TOPIC).await;
    let c = spoke(&hub, TRANSACTIONS_TOPIC).await;
    let m = b"inc-i-237-m4-r6-duplicate".to_vec();
    let id_a = a
        .publish(TRANSACTIONS_TOPIC, m.clone())
        .await
        .expect("first origin");
    let first = wait_until(Duration::from_secs(10), || !hub.deliveries(&m).is_empty()).await;
    assert!(first, "hub app never received the first copy");
    let id_c = c
        .publish(TRANSACTIONS_TOPIC, m.clone())
        .await
        .expect("second origin sent it");
    assert_eq!(id_a, id_c, "message id must not depend on source or seqno");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(hub.deliveries(&m), vec![id_a], "duplicate reached the app");
}

// INV-NETWORK-004 / REQ-GRV-013 — Decision: a failure means a sig-forged attestation shares the genuine's vendor id, so an attacker pre-seeds the dup cache and censors the genuine attestation.
#[tokio::test]
async fn inv_network_004_attestation_forged_variant_does_not_suppress_genuine() {
    let genuine = attestation(&KeyPair::generate(), 0x31, 7);
    let forged = forge(&genuine);
    assert_eq!(
        (forged.block_hash, forged.slot, forged.attester),
        (genuine.block_hash, genuine.slot, genuine.attester),
        "precondition: same signed identity"
    );
    assert_forged_first_leaves_genuine(ATTESTATION_TOPIC, forged.to_bytes(), genuine.to_bytes())
        .await;
}

fn tx_with_sig(sig: u8) -> doli_core::Transaction {
    let mut tx = doli_core::Transaction::new_transfer(
        vec![doli_core::Input::new(crypto::Hash::from_bytes([7; 32]), 0)],
        vec![doli_core::Output::normal(107, crypto::Hash::ZERO)],
    );
    tx.inputs[0].signature = Signature::from_bytes([sig; 64]);
    tx
}

// INV-NETWORK-004 / REQ-GRV-013 — Decision: a failure means a same-txid variant with another signature shares the genuine's vendor id, so a forged tx censors the genuine one (SEC-LOGIC-001).
#[tokio::test]
async fn inv_network_004_transaction_forged_variant_does_not_suppress_genuine() {
    let (forged, genuine) = (tx_with_sig(0xAA), tx_with_sig(0x00));
    assert_eq!(
        forged.hash(),
        genuine.hash(),
        "precondition: signatures excluded from txid"
    );
    assert_forged_first_leaves_genuine(TRANSACTIONS_TOPIC, forged.serialize(), genuine.serialize())
        .await;
    assert_forged_first_leaves_genuine(
        TRANSACTIONS_TOPIC,
        encode_tx_batch(&[forged]),
        encode_tx_batch(&[genuine]),
    )
    .await;
}

fn vote_with_sig(kp: &KeyPair, ts: u64, sig_hex: &str) -> Vec<u8> {
    format!(
        "{{\"version\":\"6.43.3\",\"vote\":\"approve\",\"producerId\":\"{}\",\"timestamp\":{ts},\"signature\":\"{sig_hex}\"}}",
        kp.public_key().to_hex()
    )
    .into_bytes()
}

// INV-NETWORK-004 / REQ-GRV-013 — Decision: a failure means a sig-forged governance vote shares the genuine's vendor id, so a forged vote suppresses a genuine veto (SEC-LOGIC-002).
#[tokio::test]
async fn inv_network_004_vote_forged_variant_does_not_suppress_genuine() {
    let kp = KeyPair::generate();
    let ts = now_secs();
    let msg = format!("6.43.3:approve:{ts}");
    let sig = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_hex();
    let genuine = vote_with_sig(&kp, ts, &sig);
    let forged = vote_with_sig(&kp, ts, &"00".repeat(64));
    assert_forged_first_leaves_genuine(VOTES_TOPIC, forged, genuine).await;
}

// INV-NETWORK-004 / REQ-GRV-013 — Decision: a failure means a BLS and a non-BLS attestation for the same (attester, block) share a vendor id, so one suppresses the other (SEC-CONSENSUS-003).
#[tokio::test]
async fn inv_network_004_attestation_bls_and_non_bls_do_not_collide() {
    let non_bls = attestation(&KeyPair::generate(), 0x32, 8);
    let mut bls = non_bls.clone();
    bls.bls_signature = vec![1u8; 48];
    assert!(
        non_bls.bls_signature.is_empty(),
        "precondition: first variant has no BLS"
    );
    assert_forged_first_leaves_genuine(ATTESTATION_TOPIC, non_bls.to_bytes(), bls.to_bytes()).await;
}
