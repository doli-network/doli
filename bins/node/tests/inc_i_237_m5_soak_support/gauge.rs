//! INC-I-237 M7.1 (REQ-GRV-026) — `doli_gossip_duplicate_cache_entries` is
//! written from B's own swarm on the /metrics path (not a dead gauge).
//!
//! covers: bins/node/src/metrics.rs bins/node/src/metrics_gossip.rs
//!   bins/node/src/node/startup.rs crates/network/src/service/swarm_loop.rs
//!   crates/network/src/service/command_api.rs crates/network/src/service/retention.rs
//!   vendor/libp2p-gossipsub/src/behaviour.rs
//!
//! OUTPUT CONTRACT: watch_retention + scrape_for_exposition (node) over
//!   NetworkService::retention (network) over Behaviour::retention_stats (vendor)
//!   O5: REGISTRY sample `doli_gossip_duplicate_cache_entries`; the watched
//!   Retention snapshot
//! PATHS: P1 H -> B attestation traffic, then quiescence, then a scrape
//! INPUT PARTITIONS: gauge vs B's swarm count (equal once both are stable);
//!   gauge vs ids B could hold (lower: attestation events B handled; upper:
//!   attestations H sent — capacity or another swarm's count exceeds it)
//! MATRIX: O5 x P1, both partitions asserted.

use std::time::{Duration, Instant};

use crypto::KeyPair;
use doli_node::metrics::{register_metrics, REGISTRY};
use doli_node::metrics_gossip::{scrape_for_exposition, watch_retention};
use network::NetworkCommand;

use crate::{c2, c3, pump, register_member, AtB, FAMILY_SLOT, SERIAL};

const SERIES: &str = "doli_gossip_duplicate_cache_entries";

fn rendered() -> Option<u64> {
    use prometheus::Encoder;
    let mut buf = Vec::new();
    prometheus::TextEncoder::new()
        .encode(&REGISTRY.gather(), &mut buf)
        .expect("encode registry");
    String::from_utf8(buf)
        .expect("utf8 exposition")
        .lines()
        .find_map(|l| {
            l.strip_prefix(SERIES)?
                .strip_prefix(' ')?
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map(|v| v as u64)
}

// REQ-GRV-026 — Decision: a failure means the duplicate-cache gauge is registered but not written from B's own swarm on the /metrics path, so mainnet stays blind to the C1 growth RUN 570 could not see (INC-I-187 / INC-I-232 dead-gauge shape).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn req_grv_026_duplicate_cache_gauge_reads_node_swarm() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    register_metrics();
    let (mut b, _member, _t) = c3::member_node().await;
    b.params.slot_duration = 600;
    b.params.genesis_time = c3::now_secs() - u64::from(FAMILY_SLOT) * 600 - 1;
    let honest = KeyPair::generate();
    register_member(&b, &honest).await;
    let b_port = c2::reserve_port();
    let net = c2::start_net(b_port, vec![]).await;
    let retention = net.retention();
    watch_retention(retention.clone());
    b.network = Some(net);
    let h = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let h_cmd = h.command_sender();
    c3::observe(h);

    let mut at_b = AtB::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut sent = 0u32;
    while at_b.attestations < 20 {
        assert!(
            Instant::now() < deadline,
            "setup: H -> B carried {} attestations",
            at_b.attestations
        );
        let a = c3::attestation(&honest, 0x51, 50 + sent).to_bytes();
        h_cmd
            .send(NetworkCommand::BroadcastAttestation(a))
            .await
            .unwrap();
        sent += 1;
        pump(
            &mut b,
            Instant::now() + Duration::from_millis(200),
            &mut at_b,
        )
        .await;
    }
    pump(&mut b, Instant::now() + Duration::from_secs(3), &mut at_b).await;

    let deadline = Instant::now() + Duration::from_secs(15);
    let (gauge, swarm) = loop {
        scrape_for_exposition();
        let g = rendered();
        let s = retention.snapshot().gossipsub.duplicate_cache.entries as u64;
        if (s > 0 && g == Some(s)) || Instant::now() >= deadline {
            break (g, s);
        }
        pump(
            &mut b,
            Instant::now() + Duration::from_millis(250),
            &mut at_b,
        )
        .await;
    };
    let gauge = gauge.unwrap_or_else(|| panic!("{SERIES} absent from the rendered registry"));
    assert_eq!(
        gauge, swarm,
        "{SERIES} {gauge} != B swarm duplicate-cache entries {swarm}"
    );
    assert!(
        gauge >= at_b.attestations && gauge <= u64::from(sent),
        "{SERIES} {gauge} outside [{} handled by B, {sent} sent by H]",
        at_b.attestations
    );
}
