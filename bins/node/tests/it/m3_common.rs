//! UTXO-scalability M3 [F3] — shared fixture for the node-side session tests.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a builder, not a test). It declares no `#[test]`.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Mirrors `crates/storage/tests/common/mod.rs::randomized_entries`: the same seeded
//! generator, so a set built here and a set built there are byte-identical for the same
//! `(n, seed)` and the node-side and storage-side digests are comparable.

#![allow(dead_code)]

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};
use storage::{Outpoint, UtxoEntry};

pub const FIXTURE_SEED: u64 = 0x00D0_1100_2026_0915;

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

#[allow(clippy::manual_is_multiple_of)]
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

        out.push((
            Outpoint::new(Hash::from_bytes(tx_hash), (d % 8) as u32),
            UtxoEntry {
                output: Output {
                    output_type: VARIED_TYPES[(b % 8) as usize],
                    amount: a % 1_000_000_000_000,
                    pubkey_hash: Hash::from_bytes(pubkey_hash),
                    lock_until: if b % 3 == 0 { 0 } else { c % 5_000_000 },
                    extra_data,
                },
                height: c % 900_000,
                is_coinbase: a % 7 == 0,
                is_epoch_reward: b % 11 == 0,
            },
        ));
    }

    out
}

/// Number of entries whose canonical image exceeds `target_bytes`, MEASURED from
/// `canonical_len()` rather than guessed: sample a small set to learn the true average
/// bytes per entry, size up from that, then verify the real image against the target and
/// grow until it holds.
///
/// Returns `(entry_count, canonical_image_bytes)`.
pub fn entries_exceeding(target_bytes: usize, seed: u64) -> (usize, usize) {
    const SAMPLE: usize = 4_096;
    let sample_bytes = canonical_bytes_of(SAMPLE, seed);
    let per_entry = sample_bytes as f64 / SAMPLE as f64;
    assert!(
        per_entry > 1.0,
        "the fixture reported {:.3} bytes per entry — the measurement is broken",
        per_entry
    );

    let mut n = ((target_bytes as f64 / per_entry) * 1.05).ceil() as usize + 1;
    loop {
        let bytes = canonical_bytes_of(n, seed);
        if bytes > target_bytes {
            return (n, bytes);
        }
        n = (n as f64 * 1.10).ceil() as usize + 1;
        assert!(
            n <= 8_388_608,
            "the fixture generator never exceeded {} bytes — the measurement is broken",
            target_bytes
        );
    }
}

pub fn canonical_bytes_of(n: usize, seed: u64) -> usize {
    build_in_memory(n, seed)
        .canonical_len()
        .expect("canonical_len") as usize
}

pub fn build_in_memory(n: usize, seed: u64) -> storage::utxo::UtxoSet {
    let mut set = storage::utxo::UtxoSet::new();
    for (o, e) in randomized_entries(n, seed) {
        set.insert(o, e).expect("fixture insert");
    }
    set
}
