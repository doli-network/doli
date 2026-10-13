//! INC-I-237 M1 (D6 / C1) — block-path gate order on a live A-B-C line.
//!
//! OUTPUT CONTRACT: swarm block arm at B (behaviour_events.rs, gossipsub Message
//!   on /doli/blocks/1)
//!   O1: none
//!   O3: report_message_validation_result verdict — observed as forwarding:
//!       a block published at A reaches C only through B's Accept
//!   O5: B's GossipShedMetrics::blocks_dropped (memory-watchdog shed)
//!   O6: NewBlock events at B's node (local delivery) and at C
//! PATHS: P1 candidate block, B's global bucket empty, shed off -> forwarded,
//!   local drop; P2 candidate block, global empty AND shed armed -> forwarded,
//!   shed (not delivered at B); P3 non-candidate block, A's per-peer bucket
//!   empty -> Ignore (not forwarded); P4 candidate block with A's per-peer bucket
//!   empty -> forwarded (never per-peer gated)
//! INPUT PARTITIONS: slot >= best (candidate) vs slot < best (non-candidate);
//!   global bucket {full, empty}; shed flag {off, on}; per-peer bucket {full,
//!   empty}. Seam: NetworkService::memory_shed_flag() -> Arc<AtomicBool> (the
//!   flag the swarm block arm reads; watchdog disabled so only the test writes it).
//! MATRIX: O3 x P1-P4, O6(B) x P1-P2, O5 x P2 — all asserted.

use std::collections::HashSet;
use std::net::{SocketAddr, TcpListener};
use std::ops::Range;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, PublicKey};
use doli_core::{Block, BlockHeader};
use network::{NetworkCommand, NetworkConfig, NetworkEvent, NetworkService};
use tokio::sync::mpsc;

type Ids = Arc<Mutex<HashSet<u64>>>;

const WARM_A: u64 = 41_000_000;
const WARM_C: u64 = 42_000_000;
const EXHAUST: u64 = 43_000_000;
const BACKGROUND: u64 = 44_000_000;
const PROBE_GLOBAL: u64 = 45_000_000;
const PROBE_SHED: u64 = 46_000_000;
const NON_CANDIDATE: u64 = 47_000_000;
const CANDIDATE: u64 = 48_000_000;
const PROBES: u64 = 5;
const BURST: u64 = 30;
const PER_PEER_BLOCKS_PER_MIN: usize = 10;

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
        memory_watchdog_threshold_bytes: 0,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

fn block(slot: u32, id: u64) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: id,
        slot,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

fn spawn_recorder(mut node: NetworkService) -> Ids {
    let ids: Ids = Arc::default();
    let out = ids.clone();
    tokio::spawn(async move {
        while let Some(ev) = node.next_event().await {
            if let NetworkEvent::NewBlock(b, _, _) = ev {
                out.lock().unwrap().insert(b.header.timestamp);
            }
        }
    });
    ids
}

fn drain(node: &mut NetworkService, ids: &mut HashSet<u64>) {
    while let Some(ev) = node.try_next_event() {
        if let NetworkEvent::NewBlock(b, _, _) = ev {
            ids.insert(b.header.timestamp);
        }
    }
}

fn in_range(ids: &HashSet<u64>, r: Range<u64>) -> usize {
    ids.iter().filter(|i| r.contains(i)).count()
}

fn shared_in_range(ids: &Ids, r: Range<u64>) -> usize {
    in_range(&ids.lock().unwrap(), r)
}

async fn publish(cmd: &mpsc::Sender<NetworkCommand>, slot: u32, id: u64) {
    cmd.send(NetworkCommand::BroadcastBlock(block(slot, id)))
        .await
        .expect("command channel");
}

async fn wait_for(ids: &Ids, r: Range<u64>, want: usize, within: Duration) -> usize {
    let deadline = Instant::now() + within;
    loop {
        let n = shared_in_range(ids, r.clone());
        if n >= want || Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn warm_a_to_c(a_cmd: &mpsc::Sender<NetworkCommand>, slot: u32, c_ids: &Ids) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while shared_in_range(c_ids, WARM_A..WARM_A + 1_000) == 0 {
        assert!(
            Instant::now() < deadline,
            "A -> B -> C block mesh never formed"
        );
        publish(a_cmd, slot, WARM_A + i).await;
        i += 1;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

async fn warm_c_to_b(
    c_cmd: &mpsc::Sender<NetworkCommand>,
    b: &mut NetworkService,
    b_ids: &mut HashSet<u64>,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while in_range(b_ids, WARM_C..WARM_C + 1_000) == 0 {
        assert!(Instant::now() < deadline, "C -> B block mesh never formed");
        publish(c_cmd, 1, WARM_C + i).await;
        i += 1;
        tokio::time::sleep(Duration::from_millis(400)).await;
        drain(b, b_ids);
    }
}

// F-1 / REQ-GRV-009 / C1 — Decision: a failure means a shared budget (global block bucket) or fleet-wide memory pressure (watchdog shed) suppresses the forward of a fresh candidate block, so one loaded node stalls block propagation for every node behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_1_candidate_block_forwarded_with_global_bucket_empty_and_shed_armed() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let b_port = reserve_port();
    let mut b = start_node(b_port, Vec::new()).await;
    let boot = vec![format!("/ip4/127.0.0.1/tcp/{b_port}")];
    let a = start_node(reserve_port(), boot.clone()).await;
    let c = start_node(reserve_port(), boot).await;
    let (a_cmd, c_cmd) = (a.command_sender(), c.command_sender());
    let shed = b.memory_shed_flag();
    let shed_metrics = b.gossip_shed_metrics();
    let _a_ids = spawn_recorder(a);
    let c_ids = spawn_recorder(c);
    let mut b_ids = HashSet::new();

    warm_a_to_c(&a_cmd, 1, &c_ids).await;
    warm_c_to_b(&c_cmd, &mut b, &mut b_ids).await;

    for i in 0..150 {
        publish(&c_cmd, 1, EXHAUST + i).await;
    }
    let bg_cmd = c_cmd.clone();
    let background = tokio::spawn(async move {
        for i in 0.. {
            publish(&bg_cmd, 1, BACKGROUND + i).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    drain(&mut b, &mut b_ids);

    for k in 0..PROBES {
        publish(&a_cmd, 1, PROBE_GLOBAL + k).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let range = PROBE_GLOBAL..PROBE_GLOBAL + PROBES;
    let at_c = wait_for(
        &c_ids,
        range.clone(),
        PROBES as usize,
        Duration::from_secs(10),
    )
    .await;
    drain(&mut b, &mut b_ids);
    let at_b = in_range(&b_ids, range);
    assert!(
        at_b <= 3,
        "non-vacuity: B's global bucket must be empty during the probes (B delivered {at_b}/{PROBES})"
    );
    assert_eq!(
        at_c, PROBES as usize,
        "F-1: a candidate block must be reported Accept (forwarded to C) even when B's global bucket is empty"
    );

    let dropped0 = shed_metrics.blocks_dropped();
    shed.store(true, Ordering::SeqCst);
    for k in 0..PROBES {
        publish(&a_cmd, 1, PROBE_SHED + k).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let range = PROBE_SHED..PROBE_SHED + PROBES;
    let at_c = wait_for(
        &c_ids,
        range.clone(),
        PROBES as usize,
        Duration::from_secs(10),
    )
    .await;
    drain(&mut b, &mut b_ids);
    background.abort();
    assert_eq!(
        in_range(&b_ids, range),
        0,
        "shed armed: B's node must not receive the probes (local drop)"
    );
    assert!(
        shed_metrics.blocks_dropped() >= dropped0 + PROBES,
        "non-vacuity: the shed must have run on the probes"
    );
    assert_eq!(
        at_c, PROBES as usize,
        "F-1: with the global bucket empty AND the watchdog shed armed, a candidate block must still be forwarded"
    );
}

// REQ-GRV-SEC-003 / C1 — Decision: a failure means a non-candidate block over the sender's per-peer budget is still forwarded (Accept before the rate check), so one abusive peer's blocks are relayed network-wide, or a candidate block is per-peer gated and the canonical block stalls.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_sec_003_per_peer_exhausted_non_candidate_blocks_not_forwarded() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    const BEST: u32 = 1_000_000;
    let b_port = reserve_port();
    let mut b = start_node(b_port, Vec::new()).await;
    b.update_best_slot(BEST);
    let boot = vec![format!("/ip4/127.0.0.1/tcp/{b_port}")];
    let a = start_node(reserve_port(), boot.clone()).await;
    let c = start_node(reserve_port(), boot).await;
    let a_cmd = a.command_sender();
    let _a_ids = spawn_recorder(a);
    let c_ids = spawn_recorder(c);
    let mut b_ids = HashSet::new();

    warm_a_to_c(&a_cmd, BEST + 1, &c_ids).await;

    for i in 0..BURST {
        publish(&a_cmd, 100 + i as u32, NON_CANDIDATE + i).await;
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    drain(&mut b, &mut b_ids);
    let non_candidate_at_c = shared_in_range(&c_ids, NON_CANDIDATE..NON_CANDIDATE + BURST);

    for i in 0..BURST {
        publish(&a_cmd, BEST + 10, CANDIDATE + i).await;
    }
    let candidate_at_c = wait_for(
        &c_ids,
        CANDIDATE..CANDIDATE + BURST,
        BURST as usize,
        Duration::from_secs(15),
    )
    .await;
    drain(&mut b, &mut b_ids);

    assert_eq!(
        candidate_at_c, BURST as usize,
        "control + C1: a {BURST}-block candidate burst from A (per-peer bucket empty) must all reach C"
    );
    assert!(
        non_candidate_at_c <= PER_PEER_BLOCKS_PER_MIN + 2,
        "SEC-003: non-candidate blocks over A's per-peer budget must be reported Ignore (not forwarded); C received {non_candidate_at_c}/{BURST}"
    );
}
