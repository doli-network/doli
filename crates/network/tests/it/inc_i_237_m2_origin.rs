//! INC-I-237 M2 (R4 / C5) — own-origin txs publish ONE tx per gossip message in
//! the EXISTING legacy single-tx encoding; the 100 ms / 50-tx batcher and the
//! dormant announce flush are gone (REQ-GRV-022, W08).
//!
//! OUTPUT CONTRACT: NetworkCommand::BroadcastTransaction(tx) on the swarm
//!   O1: none   O2: none   O3: none   O4: none   O5: none
//!   O6: gossipsub publishes on /doli/txs/1 — observed by a raw peer as
//!       (message id, data): count per burst, data bytes, id function
//! PATHS: P1 burst of N commands inside one old flush window, legacy config;
//!   P2 same burst with the dormant `tx_announce_enabled = true` config
//! INPUT PARTITIONS: N = 5 distinct txs (a batcher would emit 1 message);
//!   config announce {off, on} (the on path used to emit 0xAA hash batches)
//! MATRIX: O6 x {P1, P2} — count == N, data == tx.serialize(), first byte not
//!   0xAA/0xBA, decode_tx_gossip -> FullBatch([tx]), id == BLAKE3(data)[..20].

use std::collections::HashSet;
use std::time::Duration;

use network::gossip::{decode_tx_gossip, TxGossipMessage};
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;

const N: u64 = 5;

/// Publishes a burst of N own-origin txs from a fresh node; returns every raw
/// message the observer got for the burst tags.
async fn burst(announce: bool, base: u64) -> Vec<(Vec<u8>, Vec<u8>)> {
    let a_port = h::reserve_port();
    let mut a = h::start_node_cfg(a_port, Vec::new(), announce).await;
    let a_cmd = a.command_sender();
    tokio::spawn(async move { while a.next_event().await.is_some() {} });
    let r = h::raw_peer(&h::addr(a_port)).await;
    let mut warm = 0;
    let warmed = loop {
        a_cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(
                base + 900 + warm,
            )))
            .await
            .unwrap();
        warm += 1;
        let log = r.received.clone();
        if h::wait_until(Duration::from_millis(400), || {
            !log.lock().unwrap().is_empty()
        })
        .await
        {
            break true;
        }
        if warm > 60 {
            break false;
        }
    };
    assert!(warmed, "setup: the raw observer never received A's publish");
    tokio::time::sleep(Duration::from_millis(300)).await;
    r.received.lock().unwrap().clear();
    for k in 0..N {
        a_cmd
            .send(NetworkCommand::BroadcastTransaction(h::tx(base + k)))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let all = r.received.lock().unwrap().clone();
    all
}

fn assert_one_legacy_tx_per_message(got: &[(Vec<u8>, Vec<u8>)], base: u64, row: &str) {
    assert_eq!(
        got.len() as u64,
        N,
        "{row}: {N} own-origin broadcasts must be {N} gossip messages, got {} (first bytes {:?})",
        got.len(),
        got.iter()
            .map(|(_, d)| d.first().copied())
            .collect::<Vec<_>>()
    );
    let mut seen = HashSet::new();
    for (id, data) in got {
        assert!(
            !matches!(data.first(), Some(0xAA) | Some(0xBA)),
            "{row}: own-origin publish must be the legacy single-tx encoding"
        );
        let t = match decode_tx_gossip(data) {
            Some(TxGossipMessage::FullBatch(mut txs)) if txs.len() == 1 => txs.remove(0),
            _ => panic!("{row}: message does not decode as one legacy tx"),
        };
        assert!((base..base + N).contains(&h::tag(&t)), "{row}: foreign tx");
        assert_eq!(
            *data,
            h::tx(h::tag(&t)).serialize(),
            "{row}: bytes != tx.serialize()"
        );
        assert_eq!(
            *id,
            h::doli_message_id(data),
            "{row}: message-id function moved"
        );
        assert!(seen.insert(h::tag(&t)), "{row}: tx published twice");
    }
}

// REQ-GRV-022 — Decision: a failure means own-origin txs are still batched (one resident sibling would Ignore the whole batch at receivers under D5) or use a new wire format old nodes cannot decode.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_022_n_rpc_broadcasts_are_n_legacy_single_tx_messages() {
    let got = burst(false, 41_000_000).await;
    assert_one_legacy_tx_per_message(&got, 41_000_000, "announce off");
}

// REQ-GRV-022 — Decision: a failure means the dormant announce flush survived, so flipping tx_announce_enabled silently switches origin publishes to 0xAA hash batches that M2 receivers Ignore.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_022_announce_config_still_publishes_legacy_single_tx() {
    let got = burst(true, 42_000_000).await;
    assert_one_legacy_tx_per_message(&got, 42_000_000, "announce on");
}

// REQ-GRV-022 — Decision: a failure means a swarm-side tx batcher or announce flusher is still compiled into the command path (a relay-side composition point R4 forbids).
#[test]
fn req_grv_022_no_batcher_symbol_in_the_swarm_command_path() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service");
    for file in ["swarm_loop.rs", "command_handling.rs"] {
        let src = std::fs::read_to_string(dir.join(file)).expect("read source");
        for token in [
            "let mut tx_batch",
            "tx_flush",
            "tx_announce_batch",
            "encode_tx_batch(",
            "encode_tx_announce(",
        ] {
            assert!(
                !src.contains(token),
                "{file} still contains `{token}` (R4: batcher deleted)"
            );
        }
    }
}
