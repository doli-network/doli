//! INC-I-249 M2 fixture: on-disk legacy archives built with std::fs, bincode
//! and blake3 only, so it compiles on 9eda87b4 too.
//!
//! OUTPUT CONTRACT: N/A — fixture file, asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, BlockHeader};
use vdf::{VdfOutput, VdfProof};

pub const HEAVY: u64 = 100_000;

fn producer() -> PublicKey {
    static P: OnceLock<PublicKey> = OnceLock::new();
    *P.get_or_init(|| *KeyPair::from_seed([7u8; 32]).public_key())
}

pub fn block_at(genesis: Hash, h: u64) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 1000 + h,
        slot: h as u32,
        producer: producer(),
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

pub fn block_bytes(genesis: Hash, h: u64) -> Vec<u8> {
    bincode::serialize(&block_at(genesis, h)).unwrap()
}

pub fn block_path(dir: &Path, h: u64) -> PathBuf {
    dir.join(format!("{:010}.block", h))
}

pub fn sum_path(dir: &Path, h: u64) -> PathBuf {
    dir.join(format!("{:010}.blake3", h))
}

fn write_pair(dir: &Path, h: u64, data: &[u8]) {
    std::fs::write(block_path(dir, h), data).unwrap();
    std::fs::write(sum_path(dir, h), crypto::hash::hash(data).to_string()).unwrap();
}

/// Pairs for 1..=n with header genesis `genesis` and a legacy manifest
/// (`latest_height`, `latest_hash`, `genesis_hash`; no `verified_height`).
pub fn build_legacy_archive(dir: &Path, n: u64, genesis: Hash) {
    std::fs::create_dir_all(dir).unwrap();
    const WORKERS: u64 = 8;
    std::thread::scope(|s| {
        for w in 0..WORKERS {
            s.spawn(move || {
                let mut h = 1 + w;
                while h <= n {
                    write_pair(dir, h, &block_bytes(genesis, h));
                    h += WORKERS;
                }
            });
        }
    });
    let latest_hash = if n > 0 {
        block_at(genesis, n).hash()
    } else {
        Hash::ZERO
    };
    let m = serde_json::json!({
        "latest_height": n,
        "latest_hash": latest_hash.to_string(),
        "genesis_hash": genesis.to_string(),
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
}

pub fn read_manifest(dir: &Path) -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn verified_height(dir: &Path) -> Option<u64> {
    read_manifest(dir)?.get("verified_height")?.as_u64()
}

/// Spawns a 5 ms ticker on the current runtime; the returned cell holds the
/// largest gap between two ticks, in ms.
pub fn spawn_heartbeat() -> Arc<AtomicU64> {
    let max = Arc::new(AtomicU64::new(0));
    let cell = max.clone();
    tokio::spawn(async move {
        let mut last = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let now = Instant::now();
            let gap = now.duration_since(last).as_millis() as u64;
            cell.fetch_max(gap, Ordering::Relaxed);
            last = now;
        }
    });
    max
}

pub fn max_gap_ms(cell: &AtomicU64) -> u64 {
    cell.load(Ordering::Relaxed)
}

/// Polls `done` every 20 ms (yielding to the runtime) until it holds or
/// `limit` passes; returns whether it held.
pub async fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    done()
}
