// covers: run.rs periodic.rs init.rs mod.rs archive_thread.rs
//! INC-I-249 M2: archive IO stays off the node start path and the runtime.
//!
//! OUTPUT CONTRACT:
//!   fn Node::run_periodic_tasks(&mut self) on a current_thread runtime
//!     O1 largest scheduling gap of a concurrent 5 ms task (heartbeat)
//!   fn start_archive_thread(&mut Node, dir, tip) -> io::Result<ArchiveThread>
//!     O1 call latency; O2 caller-runtime heartbeat gap after the call
//!     O3 manifest.json verified_height (mid-check and after join)
//!     O4 node.archive_tx; O5 ArchiveThread.stale_dir
//!     O6 ArchiveThreadExit {catch_up_runs, catch_up, live_archived}
//!     O7 the thread ends once the channel closes
//! INPUT PARTITIONS:
//!   P1 100k legacy archive (no verified_height), genesis = params, tip = 100k
//!   P2 20-file archive whose manifest genesis is foreign, tip = 0
//!   P3 empty archive dir, tip = 0, one live block at height 1

use std::path::Path;
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::Block;
use doli_node::node::archive_thread::{start_archive_thread, ArchiveThread, ArchiveThreadExit};
use doli_node::node::Node;
use storage::archiver::ArchiveBlock;
use tempfile::TempDir;

use super::inc_i_249_archive_fixture as fx;

async fn node_in(tmp: &TempDir) -> Node {
    Node::new_for_test(tmp.path().join("node"), vec![KeyPair::generate()])
        .await
        .expect("Node::new_for_test")
}

async fn join_bounded(at: ArchiveThread, limit: Duration) -> ArchiveThreadExit {
    let finished = fx::wait_until(limit, || at.thread.is_finished()).await;
    assert!(
        finished,
        "archive thread still running {limit:?} after its channel closed"
    );
    at.thread.join().expect("archive thread panicked")
}

/// `.block` files in `dir` whose header genesis is not `genesis`.
fn blocks_not_of(dir: &Path, genesis: Hash) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "block"))
        .filter(|p| {
            let data = std::fs::read(p).unwrap();
            !bincode::deserialize::<Block>(&data).is_ok_and(|b| b.header.genesis_hash == genesis)
        })
        .count()
}

// REQ-ARCH-007 — Decision: a failure means run_periodic_tasks still runs an O(N) archive catch-up on the event-loop thread and freezes RPC/P2P on 1 vCPU
// REQ-ARCH-001 — Decision: reproduction (c) of the event-loop freeze, must fail on 9eda87b4 and on the M1 code
#[tokio::test(flavor = "current_thread")]
async fn repro_periodic_does_not_catch_up_on_runtime() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    let mut node = node_in(&tmp).await;
    let genesis = node.params.genesis_hash;
    fx::build_legacy_archive(&archive, fx::HEAVY, genesis);
    {
        let mut cs = node.chain_state.write().await;
        cs.genesis_hash = genesis;
        cs.best_height = fx::HEAVY;
    }
    node.archive_dir = Some(archive.clone());
    let manifest_before = std::fs::read(archive.join("manifest.json")).unwrap();

    let gap = fx::spawn_heartbeat();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let t = Instant::now();
    let res = node.run_periodic_tasks().await;
    let took = t.elapsed();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let max = fx::max_gap_ms(&gap);
    eprintln!(
        "run_periodic_tasks -> {:?} in {:?}; max heartbeat gap {} ms; manifest verified_height {:?}",
        res.as_ref().map(|_| ()),
        took,
        max,
        fx::verified_height(&archive)
    );
    assert_eq!(
        std::fs::read(archive.join("manifest.json")).unwrap(),
        manifest_before,
        "run_periodic_tasks rewrote manifest.json: an archive catch-up ran on the event loop"
    );
    assert!(
        max < 500,
        "max heartbeat gap {max} ms >= 500 ms: run_periodic_tasks held the only runtime thread (archive catch-up on the event loop)"
    );
}

// REQ-ARCH-006 — Decision: a failure means node start still blocks on the O(N) archive check, so RPC/P2P stay down for minutes after a restart
// REQ-ARCH-001 — Decision: reproduction (a) of the start-path wait, must fail before the seam returns early
#[tokio::test(flavor = "current_thread")]
async fn repro_start_does_not_wait_for_archive() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    let mut node = node_in(&tmp).await;
    fx::build_legacy_archive(&archive, fx::HEAVY, node.params.genesis_hash);

    let t = Instant::now();
    let at =
        start_archive_thread(&mut node, archive.clone(), fx::HEAVY).expect("start_archive_thread");
    let start_ms = t.elapsed().as_millis();
    assert!(
        start_ms < 1000,
        "start_archive_thread took {start_ms} ms >= 1000 ms: the caller waited on the archive"
    );
    assert!(at.stale_dir.is_none(), "a same-genesis archive was fenced");
    assert!(
        node.archive_tx.is_some(),
        "start_archive_thread did not set archive_tx"
    );

    let gap = fx::spawn_heartbeat();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let max = fx::max_gap_ms(&gap);
    assert!(
        max < 1000,
        "max heartbeat gap {max} ms >= 1000 ms on the caller runtime during the background check"
    );
    let mid = fx::verified_height(&archive);
    assert!(
        mid.is_none_or(|h| h < fx::HEAVY),
        "background check already done ~100 ms after start (verified_height {mid:?}): it ran before start returned"
    );

    node.archive_tx = None;
    let exit = join_bounded(at, Duration::from_secs(300)).await;
    assert_eq!(
        exit.catch_up_runs, 1,
        "one start must run exactly one catch-up"
    );
    assert_eq!(
        exit.catch_up.map(|r| r.verified_height),
        Some(fx::HEAVY),
        "catch-up report verified_height"
    );
    assert_eq!(exit.live_archived, 0, "no live block was sent");
    assert_eq!(
        fx::verified_height(&archive),
        Some(fx::HEAVY),
        "manifest verified_height after the background check"
    );
}

// REQ-ARCH-006 — Decision: a failure means a foreign-genesis archive stays importable after start or its stale copy is never deleted (INV-ARCHIVER-001)
#[tokio::test(flavor = "current_thread")]
async fn start_fences_foreign_archive_and_thread_deletes_it() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    let mut node = node_in(&tmp).await;
    let genesis = node.params.genesis_hash;
    let foreign = crypto::hash::hash(b"inc-i-249-m2-foreign-genesis");
    assert_ne!(foreign, genesis);
    fx::build_legacy_archive(&archive, 20, foreign);

    let at = start_archive_thread(&mut node, archive.clone(), 0).expect("start_archive_thread");
    let stale = at
        .stale_dir
        .clone()
        .expect("a foreign-genesis archive must be fenced");
    assert!(archive.is_dir(), "no archive dir after the fence");
    assert_eq!(
        blocks_not_of(&archive, genesis),
        0,
        "foreign .block files remain in the live archive dir"
    );
    let manifest_genesis = fx::read_manifest(&archive)
        .and_then(|m| m.get("genesis_hash")?.as_str().map(str::to_string));
    assert_ne!(
        manifest_genesis,
        Some(foreign.to_string()),
        "live archive manifest still names the foreign genesis"
    );
    let gone = fx::wait_until(Duration::from_secs(60), || !stale.exists()).await;
    assert!(gone, "stale dir {stale:?} still exists 60 s after start");

    node.archive_tx = None;
    join_bounded(at, Duration::from_secs(60)).await;
}

// REQ-ARCH-008 — Decision: a failure means a finalized block sent after start is never written by the owner thread, so the live archive silently stops
#[tokio::test(flavor = "current_thread")]
async fn block_sent_after_start_is_archived() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let mut node = node_in(&tmp).await;
    let genesis = node.params.genesis_hash;

    let at = start_archive_thread(&mut node, archive.clone(), 0).expect("start_archive_thread");
    assert!(at.stale_dir.is_none(), "an empty archive was fenced");
    let block = fx::block_at(genesis, 1);
    let data = bincode::serialize(&block).unwrap();
    node.archive_tx
        .as_ref()
        .expect("start_archive_thread did not set archive_tx")
        .try_send(ArchiveBlock {
            height: 1,
            hash: block.hash(),
            data: data.clone(),
        })
        .expect("archive channel refused the block");
    let path = fx::block_path(&archive, 1);
    let written = fx::wait_until(Duration::from_secs(30), || path.exists()).await;
    assert!(written, "{path:?} not written 30 s after the live send");

    node.archive_tx = None;
    let exit = join_bounded(at, Duration::from_secs(60)).await;
    assert_eq!(exit.live_archived, 1, "live blocks archived by the thread");
    assert_eq!(std::fs::read(&path).unwrap(), data, ".block bytes");
    assert_eq!(
        std::fs::read_to_string(fx::sum_path(&archive, 1))
            .unwrap()
            .trim(),
        crypto::hash::hash(&data).to_string(),
        ".blake3 checksum"
    );
    assert_eq!(
        fx::verified_height(&archive),
        Some(1),
        "manifest verified_height"
    );
}

// REQ-ARCH-006 — Decision: M2 outcome probe; a large START_RETURN_MS means start still waits on the 100k-file archive
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn inc_i_249_m2_outcome_probe() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    let mut node = node_in(&tmp).await;
    fx::build_legacy_archive(&archive, fx::HEAVY, node.params.genesis_hash);

    let t = Instant::now();
    let at =
        start_archive_thread(&mut node, archive.clone(), fx::HEAVY).expect("start_archive_thread");
    println!("START_RETURN_MS={}", t.elapsed().as_millis());

    node.archive_tx = None;
    join_bounded(at, Duration::from_secs(300)).await;
}
