// OUTPUT CONTRACT: spawn_update_service(config, data_dir, network, producer_count_fn, is_producer_fn, maintainer_keys_fn, recheck) -> UpdateService::run
//   O1: none
//   O2: none observable (service state is private)
//   O3: (vote_tx, pending) — not asserted
//   O6: external effect — one GET {custom_url}/latest.json per check_for_updates run
// PATHS: P1 first interval tick -> 1 fetch at once; P2 no recheck -> no 2nd fetch before the
//   600 s tick; P3 recheck.notify_one() -> 2nd fetch at once.
// INPUT PARTITIONS: check_interval_secs = 600 (next tick far away); recheck fired 0 | 1 times;
//   release source = loopback stub answering 404 (custom URL, no remote fallback).
// MATRIX: 3 paths x fetch count = 3 cells.
// covers: maintainer_quorum.rs mod.rs init.rs periodic.rs run.rs service.rs service_apply.rs service_checks.rs trust_root_wiring.rs

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use doli_core::Network;
use doli_node::updater::{spawn_update_service, UpdateConfig};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

async fn stub_release_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stub must bind loopback");
    let addr = listener.local_addr().expect("stub address");
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if String::from_utf8_lossy(&buf[..n]).contains("/latest.json") {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                let _ = sock.write_all(NOT_FOUND).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), hits)
}

async fn reaches(hits: &AtomicUsize, target: usize, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if hits.load(Ordering::SeqCst) >= target {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    hits.load(Ordering::SeqCst) >= target
}

/// REQ-248-003 (Must) — Decision: a failure means a node whose trust root was just repaired keeps refusing the pending release until the next multi-hour update tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inc_i_248_updater_recheck_on_notify() {
    let (url, hits) = stub_release_server().await;
    let data = TempDir::new().unwrap();
    let config = UpdateConfig {
        enabled: true,
        check_interval_secs: 600,
        custom_url: Some(url),
        ..UpdateConfig::default()
    };
    let recheck = Arc::new(Notify::new());
    let _service = spawn_update_service(
        config,
        data.path().to_path_buf(),
        Network::Testnet,
        || Some(0),
        |_| Some(false),
        || updater::TrustRoot::on_chain(Vec::new(), updater::REQUIRED_SIGNATURES),
        recheck.clone(),
    );

    assert!(
        reaches(&hits, 1, Duration::from_secs(5)).await,
        "the first interval tick must fetch at once"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "no second fetch before the 600 s tick without a recheck"
    );

    recheck.notify_one();
    assert!(
        reaches(&hits, 2, Duration::from_secs(5)).await,
        "a recheck must run check_for_updates at once"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
