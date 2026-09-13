// OUTPUT CONTRACT: the observable output is P.peer_count() over real 127.0.0.1 sockets, reported
//   as one probe line `INC-I-221 OUTCOME peers_reached=<max>`. No receiver or persistent store writes.
// INPUT PARTITIONS: S down while P holds Q (P at 1 of min 2); S up from tick 6 (P below min, seed reachable).

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use doli_node::node::bootstrap_redial::due_bootstrap_redials;
use network::{NetworkConfig, NetworkService};

const MIN_PEERS: usize = 2;
const S_START_TICK: u64 = 6;
const POLL_TICKS_AFTER_S: u64 = 30;

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve an ephemeral 127.0.0.1 port")
        .port()
}

fn tcp_multiaddr(port: u16) -> String {
    format!("/ip4/127.0.0.1/tcp/{port}")
}

async fn start_node(port: u16, bootstrap_nodes: Vec<String>) -> NetworkService {
    let listen_addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("listen addr");
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
        .expect("start NetworkService on 127.0.0.1")
}

fn drain_events(svc: &mut NetworkService) {
    while svc.try_next_event().is_some() {}
}

// REQ-BSR-007 — Decision: a failure means a producer started before its seed stays at one peer over real sockets until someone restarts it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_started_before_seed_reaches_min_peers_without_restart() {
    let s_port = reserve_port();
    let q_port = reserve_port();
    let p_port = reserve_port();
    let bootstrap = vec![tcp_multiaddr(s_port), tcp_multiaddr(q_port)];

    let mut q = start_node(q_port, Vec::new()).await;
    let mut p = start_node(p_port, bootstrap.clone()).await;

    let connect_deadline = Instant::now() + Duration::from_secs(15);
    while p.peer_count().await < 1 {
        assert!(
            Instant::now() < connect_deadline,
            "P never connected to Q within 15 s"
        );
        drain_events(&mut p);
        drain_events(&mut q);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        p.peer_count().await,
        1,
        "precondition: P holds only Q while S is down"
    );

    let mut backoff: HashMap<String, (u32, Instant)> = HashMap::new();
    let mut s: Option<NetworkService> = None;
    let mut peers_reached = 1usize;
    for tick in 0..=(S_START_TICK + POLL_TICKS_AFTER_S) {
        if tick == S_START_TICK {
            s = Some(start_node(s_port, Vec::new()).await);
        }
        let peer_count = p.peer_count().await;
        peers_reached = peers_reached.max(peer_count);
        if peers_reached >= MIN_PEERS {
            break;
        }
        let due = due_bootstrap_redials(
            peer_count,
            MIN_PEERS,
            &bootstrap,
            &mut backoff,
            Instant::now(),
        );
        for addr in due {
            p.connect(&addr).await.expect("queue Connect command");
        }
        drain_events(&mut p);
        drain_events(&mut q);
        if let Some(svc) = s.as_mut() {
            drain_events(svc);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    println!("INC-I-221 OUTCOME peers_reached={peers_reached}");
    assert!(
        peers_reached >= MIN_PEERS,
        "P stayed at {peers_reached} peer(s): it never re-dialed its late seed"
    );
}
