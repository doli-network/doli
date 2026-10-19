//! INC-I-249 M1.5 outcome probe (ignored; run on demand).
//!
//! OUTPUT CONTRACT: fn probe_second_catch_up_files_read_with_legacy_file
//!   O1 stdout: `FILES_READ=<n>` — `.block` files whose access time moved
//!   during the SECOND `BlockArchiver::catch_up` call. Asserts nothing.
//! INPUT PARTITIONS: one fixed fixture — 10_000 archived heights for the
//!   expected genesis, no manifest, empty block store, tip = 10_000; the file
//!   at height 100 holds a valid header (expected genesis) and a body that
//!   does not deserialize, with a correct `.blake3` sidecar.
//!
//! The first call stands for the first start after an upgrade, the second for
//! the next start. Reads are counted from the filesystem (atime reset before
//! the second call), so the probe measures code with and without a counter.
//! covers: crates/storage/src/archiver/catch_up.rs

use std::fs::{File, FileTimes};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::BlockArchiver;
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const TIP: u64 = 10_000;
const LEGACY: u64 = 100;

fn header_with_genesis(genesis: Hash) -> BlockHeader {
    let kp = KeyPair::generate();
    BlockHeader {
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
    }
}

fn age_block_files(dir: &Path, old: SystemTime) {
    for h in 1..=TIP {
        File::options()
            .write(true)
            .open(dir.join(format!("{:010}.block", h)))
            .unwrap()
            .set_times(FileTimes::new().set_accessed(old))
            .unwrap();
    }
}

#[test]
#[ignore = "outcome probe: builds a 10k-file archive"]
fn probe_second_catch_up_files_read_with_legacy_file() {
    let genesis = crypto::hash::hash(b"inc-i-249-m15-probe-genesis");
    let header = header_with_genesis(genesis);
    let data = bincode::serialize(&Block::new(header.clone(), vec![])).unwrap();
    let mut legacy = bincode::serialize(&header).unwrap();
    legacy.extend_from_slice(&[0xFF; 64]);
    assert!(doli_core::transaction::legacy::deserialize_block_compat(&legacy).is_none());

    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    for h in 1..=TIP {
        let bytes = if h == LEGACY { &legacy } else { &data };
        std::fs::write(dir.join(format!("{:010}.block", h)), bytes).unwrap();
        let sum = crypto::hash::hash(bytes).to_string();
        std::fs::write(dir.join(format!("{:010}.blake3", h)), sum).unwrap();
    }

    let store_dir = TempDir::new().unwrap();
    let store = BlockStore::open(store_dir.path()).unwrap();
    BlockArchiver::catch_up(dir, &store, TIP, Some(&genesis)).unwrap();

    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
    age_block_files(dir, old);
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
