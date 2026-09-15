//! Deterministic UTXO fixtures shared by the M2 canonical-fold tests.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a builder, not a test). It asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! The generator is seeded, so the same `(n, seed)` yields byte-identical sets on
//! every host: the M2 byte-equality lock and the M2 allocation probe compare
//! numbers across runs and across machines.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};
use storage::{Outpoint, StateDb, UtxoEntry, UtxoSet};

pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const VARIED_TYPES: [OutputType; 8] = [
    OutputType::Normal,
    OutputType::Bond,
    OutputType::Multisig,
    OutputType::Hashlock,
    OutputType::HTLC,
    OutputType::Vesting,
    OutputType::NFT,
    OutputType::FungibleAsset,
];

/// Build `n` unique UTXO entries whose keys, values, and variable-length
/// `extra_data` all vary. Uniqueness comes from `i` in the last 8 key bytes;
/// the first 24 bytes are pseudo-random, so lexicographic order is unrelated to
/// insertion order.
pub fn randomized_entries(n: usize, seed: u64) -> Vec<(Outpoint, UtxoEntry)> {
    let mut rng = SplitMix64::new(seed);
    let mut out = Vec::with_capacity(n);

    for i in 0..n {
        let a = rng.next_u64();
        let b = rng.next_u64();
        let c = rng.next_u64();
        let d = rng.next_u64();

        let mut tx_hash = [0u8; 32];
        tx_hash[0..8].copy_from_slice(&a.to_le_bytes());
        tx_hash[8..16].copy_from_slice(&b.to_le_bytes());
        tx_hash[16..24].copy_from_slice(&c.to_le_bytes());
        tx_hash[24..32].copy_from_slice(&(i as u64).to_le_bytes());

        let mut pubkey_hash = [0u8; 32];
        pubkey_hash[0..8].copy_from_slice(&d.to_le_bytes());
        pubkey_hash[8..16].copy_from_slice(&(a ^ d).to_le_bytes());
        pubkey_hash[16..24].copy_from_slice(&(b ^ c).to_le_bytes());
        pubkey_hash[24..32].copy_from_slice(&(c ^ d).to_le_bytes());

        let extra_len = (d % 24) as usize;
        let mut extra_data = Vec::with_capacity(extra_len);
        for k in 0..extra_len {
            extra_data.push(((a >> (k % 8)) as u8) ^ (k as u8));
        }

        let entry = UtxoEntry {
            output: Output {
                output_type: VARIED_TYPES[(b % 8) as usize],
                amount: a % 1_000_000_000_000,
                pubkey_hash: Hash::from_bytes(pubkey_hash),
                lock_until: if b.is_multiple_of(3) {
                    0
                } else {
                    c % 5_000_000
                },
                extra_data,
            },
            height: c % 900_000,
            is_coinbase: a.is_multiple_of(7),
            is_epoch_reward: b.is_multiple_of(11),
        };

        out.push((
            Outpoint::new(Hash::from_bytes(tx_hash), (d % 8) as u32),
            entry,
        ));
    }

    out
}

pub fn build_in_memory(entries: &[(Outpoint, UtxoEntry)]) -> UtxoSet {
    let mut set = UtxoSet::new();
    for (outpoint, entry) in entries {
        set.insert(*outpoint, entry.clone())
            .expect("fixture: in-memory insert must succeed");
    }
    set
}

pub fn build_rocksdb(dir: &Path, entries: &[(Outpoint, UtxoEntry)]) -> UtxoSet {
    UtxoSet::from_state_db(open_state_db_with(dir, entries))
}

pub fn open_state_db_with(dir: &Path, entries: &[(Outpoint, UtxoEntry)]) -> Arc<StateDb> {
    let sdb = Arc::new(StateDb::open(dir).expect("fixture: StateDb::open must succeed"));
    for (outpoint, entry) in entries {
        sdb.insert_utxo(outpoint, entry)
            .expect("fixture: state_db insert must succeed");
    }
    sdb
}
