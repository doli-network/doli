//! INC-I-249 M1 outcome probe (ignored; run on demand).
//!
//! OUTPUT CONTRACT: fn probe_catch_up_block_files_read_100k
//!   O7 stdout: `FILES_READ=<n>` — `.block` files whose access time moved
//!   during ONE `BlockArchiver::catch_up` call. Asserts nothing.
//! INPUT PARTITIONS: one fixed fixture — 100_000 archived heights, every pair
//!   valid for the expected genesis, manifest `verified_height` = 99_000,
//!   empty block store, tip = 100_000.
//!
//! Reads are counted from the filesystem (atime older than mtime is advanced
//! by a read), so the same probe measures code with and without a read
//! counter.

use std::fs::{File, FileTimes};
use std::time::{Duration, SystemTime};

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::BlockArchiver;
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const TIP: u64 = 100_000;
const VERIFIED: u64 = 99_000;

fn block_with_genesis(genesis: Hash) -> Block {
    let kp = KeyPair::generate();
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 1000,
        slot: 1,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

#[test]
#[ignore = "outcome probe: builds a 100k-file archive"]
fn probe_catch_up_block_files_read_100k() {
    let genesis = crypto::hash::hash(b"inc-i-249-probe-genesis");
    let block = block_with_genesis(genesis);
    let data = bincode::serialize(&block).unwrap();
    let checksum = crypto::hash::hash(&data).to_string();

    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
    for h in 1..=TIP {
        let block_path = dir.join(format!("{:010}.block", h));
        std::fs::write(&block_path, &data).unwrap();
        std::fs::write(dir.join(format!("{:010}.blake3", h)), &checksum).unwrap();
        File::options()
            .write(true)
            .open(&block_path)
            .unwrap()
            .set_times(FileTimes::new().set_accessed(old))
            .unwrap();
    }
    let manifest = serde_json::json!({
        "latest_height": TIP,
        "latest_hash": block.hash().to_string(),
        "genesis_hash": genesis.to_string(),
        "verified_height": VERIFIED,
    });
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();

    let store_dir = TempDir::new().unwrap();
    let store = BlockStore::open(store_dir.path()).unwrap();
    BlockArchiver::catch_up(dir, &store, TIP, Some(&genesis)).unwrap();

    let threshold = old + Duration::from_secs(86_400);
    let read = (1..=TIP)
        .filter(|h| {
            let meta = std::fs::metadata(dir.join(format!("{:010}.block", h))).unwrap();
            meta.accessed().unwrap() > threshold
        })
        .count();
    println!("FILES_READ={}", read);
}
