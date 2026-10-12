//! INC-I-237 M3 — one identity per message on the wire: the node does not
//! re-publish content it already published or already received via gossip.
//! Observable: the vendor refusal `Not publishing a message that has already
//! been published` (the WARN the M3 outcome metric counts) in this process.
//! Each test is its own process under nextest, so the capture is attributable.
//!
//! OUTPUT CONTRACT: Node::run_event_loop (gossip-timer arm); Node::handle_network_event
//!   (NetworkEvent::SyncRequest DirectAttestation -> on_sync_request -> handle_sync_request)
//!   O1-O5: none asserted
//!   O6: publish attempts B makes, observed as vendor duplicate refusals; what C receives
//! PATHS: P1 gossip tick with an unchanged GSet -> no second publish of the same
//!   digest; P2 DirectAttestation for an attestation B already received via
//!   gossip -> no re-publish
//! INPUT PARTITIONS: GSet {unchanged across >= 3 ticks}; attestation {already seen via gossip}
//! MATRIX: O6 x P1, O6 x P2. Non-vacuity: the capture sees B's own INFO lines,
//!   C received the first digest, B handled the direct request.

use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use network::protocols::SyncRequest;
use network::{NetworkCommand, NetworkConfig, NetworkService};

use super::inc_i_237_m2_node_common as c2;
use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;

/// A service on the node's network id and genesis, so the node keeps the peer.
async fn start_net_as(
    network_id: u32,
    genesis_hash: Hash,
    port: u16,
    boot: Vec<String>,
) -> NetworkService {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let config = NetworkConfig {
        listen_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        bootstrap_nodes: boot,
        max_peers: 8,
        bootstrap_slots: 8,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        memory_watchdog_threshold_bytes: 0,
        network_id,
        genesis_hash,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

// REQ-GRV-005 / INV-NETWORK-003 — Decision: a failure means the gossip timer re-publishes a byte-identical producer digest every tick (refused by the vendor and logged as a WARN on every node with log rotation off), i.e. a second publish of one wire identity.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unchanged_producer_digest_is_not_republished_every_gossip_tick() {
    let log = c::capture();
    let (mut b, _member, _t) = c::member_node().await;
    let (nid, gh) = (
        b.config.network.id(),
        b.chain_state.read().await.genesis_hash,
    );
    let b_port = c2::reserve_port();
    b.network = Some(start_net_as(nid, gh, b_port, vec![]).await);
    let c_net = start_net_as(nid, gh, c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let c_log = c::observe(c_net);
    let peers = b.network.as_ref().unwrap().peers_arc();
    let deadline = Instant::now() + Duration::from_secs(20);
    while peers.read().await.is_empty() {
        assert!(Instant::now() < deadline, "setup: C never connected to B");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ann = c::announcement(&b, &KeyPair::generate(), 1).await;
    b.producer_gset
        .write()
        .await
        .merge_one(ann)
        .expect("GSet seeded");
    let _ = tokio::time::timeout(Duration::from_secs(13), b.run_event_loop()).await;
    assert!(
        !c::captured(&log, "Entering main event loop").is_empty(),
        "non-vacuity: the tracing capture is live"
    );
    let digests = c::count_where(&c_log, |e| matches!(e, Ev::Digest(_)));
    let tail: Vec<String> = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| {
            !l.contains("PROD_DIAG") && !l.contains("Stale chain") && !l.contains("INFECTED")
        })
        .take(40)
        .cloned()
        .collect();
    assert!(
        digests >= 1,
        "non-vacuity: B's first digest must reach C; B log tail: {tail:?}"
    );
    let dups = c::captured(&log, c::DUP_PUBLISH);
    assert!(
        dups.is_empty(),
        "B re-published an unchanged digest: {dups:?}"
    );
}

// REQ-GRV-005 / INV-NETWORK-003 — Decision: a failure means the DIRECT_ATTEST_RECV path re-publishes an attestation B already received via gossip, so one attestation is put on the wire twice by B (vendor-refused, WARN per attestation per node).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_attestation_already_seen_via_gossip_is_not_republished() {
    let log = c::capture();
    let mut l = c::line().await;
    let m = l.member.clone();
    l.warm(c::is_attestation, |i| {
        NetworkCommand::BroadcastAttestation(c::attestation(&m, 0x10, 10 + i).to_bytes())
    })
    .await;
    let x = c::attestation(&l.member, 0x60, 600).to_bytes();
    l.send(NetworkCommand::BroadcastAttestation(x.clone()))
        .await;
    c::pump(&mut l.b, Duration::from_millis(2_500)).await;
    assert_eq!(
        c::count(&l.c_log, &Ev::Attestation(x.clone())),
        1,
        "setup: B received and relayed the attestation via gossip"
    );
    let dups_before = c::captured(&log, c::DUP_PUBLISH).len();
    let b_peer = *l.b.network.as_ref().unwrap().local_peer_id();
    let request = SyncRequest::DirectAttestation { data: x.clone() };
    l.send(NetworkCommand::RequestSync {
        peer_id: b_peer,
        request,
    })
    .await;
    c::pump(&mut l.b, Duration::from_millis(3_000)).await;
    assert!(
        !c::captured(&log, "DIRECT_ATTEST_RECV").is_empty(),
        "non-vacuity: B handled the direct attestation"
    );
    let dups = c::captured(&log, c::DUP_PUBLISH);
    assert_eq!(
        dups.len() - dups_before,
        0,
        "B re-published a gossip-received attestation: {dups:?}"
    );
}
