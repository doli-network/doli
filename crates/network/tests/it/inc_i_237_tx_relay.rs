//! INC-I-237 — tx gossip must not be relayed before the node has checked it.
//!
//! Before: `classify_transaction_gossip` returns Accept on first sight, so
//! gossipsub forwards every tx batch to the mesh BEFORE the node runs mempool
//! admission. A tx that no node can admit (the INC-I-237 SlashProducer) is
//! relayed by every hop, forever. The node already re-publishes each tx its
//! mempool accepts (`handle_new_transaction` -> `broadcast_transaction`), so
//! raw mesh forwarding is not needed for propagation.
//!
//! Topology (real `NetworkService`s, 127.0.0.1): F -- B -- C. F and C have no
//! direct link. B's "node" drains events, re-publishes NOTHING and never sends
//! a verdict (M2: no verdict = no forward). Then C must not receive any of F's txs.
//!
//! OUTPUT CONTRACT
//!   O1: count of F-originated NewTransaction events at C   (must be 0)
//!   O2: count of F-originated NewTransaction events at B   (non-vacuity, > 0)
//!   O3: B -> C tx path works (B's own publish reaches C)   (non-vacuity)
//! INPUT PARTITIONS: IP-1 B delivers txs to its node, node rejects them all.

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use network::{NetworkCommand, NetworkConfig, NetworkEvent, NetworkService};

const F_BASE: u64 = 1_000_000;
const B_BASE: u64 = 9_000_000;
const FLOOD_TXS: u64 = 500;

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve port")
        .port()
}

async fn start_node(port: u16, bootstrap_nodes: Vec<String>) -> NetworkService {
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
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

fn tx(tag: u64) -> Transaction {
    Transaction::new_coinbase(tag, Hash::default(), tag, tag as u32)
}

fn origin(t: &Transaction) -> u64 {
    t.outputs.first().map(|o| o.amount).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inc_i_237_tx_batch_not_relayed_before_node_accepts_it() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let b_port = reserve_port();
    let mut b = start_node(b_port, Vec::new()).await;
    let boot = vec![format!("/ip4/127.0.0.1/tcp/{b_port}")];
    let mut f = start_node(reserve_port(), boot.clone()).await;
    let mut c = start_node(reserve_port(), boot).await;
    let (b_cmd, f_cmd) = (b.command_sender(), f.command_sender());

    // B's node: counts F's txs, re-publishes nothing (rejects all).
    let b_seen_f = Arc::new(AtomicU64::new(0));
    let b_seen = b_seen_f.clone();
    tokio::spawn(async move {
        while let Some(ev) = b.next_event().await {
            if let NetworkEvent::NewTransaction(txs, _) = ev {
                let f = txs.iter().filter(|t| (F_BASE..B_BASE).contains(&origin(t)));
                b_seen.fetch_add(f.count() as u64, Ordering::Relaxed);
            }
        }
    });
    tokio::spawn(async move { while f.next_event().await.is_some() {} });

    // O3: warm the B -> C tx mesh until C receives B's own publish.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut n = 0;
    'warm: loop {
        assert!(Instant::now() < deadline, "C never received B's own tx");
        b_cmd
            .send(NetworkCommand::BroadcastTransaction(tx(B_BASE + n)))
            .await
            .unwrap();
        n += 1;
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if let Ok(Some(NetworkEvent::NewTransaction(..))) =
                tokio::time::timeout(Duration::from_millis(100), c.next_event()).await
            {
                break 'warm;
            }
        }
    }

    // F publishes until B has seen its txs (F -> B mesh warm), then floods.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while b_seen_f.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "B never received F's txs");
        f_cmd
            .send(NetworkCommand::BroadcastTransaction(tx(F_BASE + i)))
            .await
            .unwrap();
        i += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    for j in 0..FLOOD_TXS {
        f_cmd
            .send(NetworkCommand::BroadcastTransaction(tx(F_BASE
                + 100_000
                + j)))
            .await
            .unwrap();
    }

    let mut c_got_f = 0u64;
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        if let Ok(Some(NetworkEvent::NewTransaction(txs, _))) =
            tokio::time::timeout(Duration::from_millis(100), c.next_event()).await
        {
            c_got_f += txs
                .iter()
                .filter(|t| (F_BASE..B_BASE).contains(&origin(t)))
                .count() as u64;
        }
    }
    let b_got_f = b_seen_f.load(Ordering::Relaxed);
    eprintln!("b_got_f={b_got_f} c_got_f={c_got_f}");
    assert!(b_got_f > 0, "setup: B must receive F's txs");
    assert_eq!(
        c_got_f, 0,
        "INC-I-237: B relayed {c_got_f} of F's txs to C although B's node accepted none"
    );
}
