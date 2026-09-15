//! UTXO-scalability M4 [F1][F3] — the OUTCOME PROBE (docs/.workflow/m4-outcome-metric.txt).
//!
//! REQ-SCALE-014 — Decision: a rise in `M4_DL_INSTALL_PEAK_BYTES` at a fixed `n` is the
//! milestone regressing in the only dimension an operator feels — the RAM their box must
//! have free to join the network from a snapshot.
//! REQ-SCALE-002 — Decision: `M4_INSTALL_UTXO_COUNT` below `n` would mean the streamed
//! install silently truncates the set, which nothing downstream rejects (`BlockHeader`
//! carries no state root).
//! REQ-SCALE-006 / INV-SYNC-007 — Decision: `M4_INSTALL_UTXO_HASH` must be IDENTICAL in the
//! before and after runs; a changed digest would mean the RAM win was bought with a state
//! change, which is the one outcome M4 may not produce.
//! REQ-SCALE-013 — Decision: `M4_TRANSFER_PATH` names which implementation produced the
//! number, so a `before`/`after` pair that accidentally measured the same code path is
//! visible instead of silently comparing a value with itself.
//!
//! OUTPUT CONTRACT: this file is a MEASUREMENT harness. Its observable outputs are the
//!   `M4_*` lines on stdout; it asserts only the non-vacuity of what it measured.
//!     O1 M4_DL_INSTALL_PEAK_BYTES_50K    O2 M4_DL_INSTALL_PEAK_BYTES_100K
//!     O3 M4_PEAK_RATIO                   O4 M4_INSTALL_UTXO_HASH_{50K,100K}
//!     O5 M4_INSTALL_UTXO_COUNT_{50K,100K} O6 M4_STAGED_MAX_LEN
//!     O7 M4_TRANSFER_PATH
//!   Paths: P1 a full download+install at n = 50_000   — O1, O4, O5, O6, O7
//!          P2 a full download+install at n = 100_000  — O2, O4, O5, O6, O7
//!          P3 the two peaks compared                  — O3
//!   MATRIX: P1xO1+O4+O5 | P2xO2+O4+O5 | P1xO6+O7 + P2xO6+O7 | P3xO3
//! INPUT PARTITIONS: two set sizes differing by exactly 2x, same seeded generator, so the
//!   ratio isolates the size term and nothing else. Both runs drive the SAME entry points.
//!
//! It does NOT assert that the ratio is flat. That claim belongs to `m4_peak_ratio_red.rs`,
//! so this file keeps printing comparable numbers in BOTH the before and the after run.
//!
//! Its OWN test binary on purpose: the counting `#[global_allocator]` in
//! `tests/m4_measure/mod.rs` is per-BINARY. Declared in `bins/node/Cargo.toml`.

#[path = "m4_measure/mod.rs"]
mod m;

const N_SMALL: usize = 50_000;
const N_LARGE: usize = 100_000;

/// REQ-SCALE-014 — Decision: this is the milestone's externally observable number, and the
/// only one an operator can act on. Nothing here is a test count or an agent verdict.
#[tokio::test(flavor = "multi_thread")]
async fn m4_install_peak_probe() {
    let small = m::measure_download_install(N_SMALL, m::PROBE_SEED).await;
    let large = m::measure_download_install(N_LARGE, m::PROBE_SEED).await;

    let ratio = large.peak_bytes as f64 / small.peak_bytes as f64;
    let staged_max = small
        .staged_len_at_session_end
        .max(large.staged_len_at_session_end);

    // Leading newline: libtest leaves the `test <name> ...` prefix unterminated, which would
    // otherwise make the first witness line un-greppable at line start.
    println!();
    println!("M4_DL_INSTALL_PEAK_BYTES_50K={}", small.peak_bytes);
    println!("M4_DL_INSTALL_PEAK_BYTES_100K={}", large.peak_bytes);
    println!("M4_PEAK_RATIO={:.2}", ratio);
    println!("M4_INSTALL_UTXO_HASH_50K={}", small.installed_hash);
    println!("M4_INSTALL_UTXO_HASH_100K={}", large.installed_hash);
    println!("M4_INSTALL_UTXO_COUNT_50K={}", small.installed_count);
    println!("M4_INSTALL_UTXO_COUNT_100K={}", large.installed_count);
    println!("M4_STAGED_MAX_LEN={}", staged_max);
    println!("M4_TRANSFER_PATH={}", large.transfer_path);
    println!("M4_SET_BYTES_50K={}", small.set_bytes);
    println!("M4_SET_BYTES_100K={}", large.set_bytes);
    println!("M4_CHUNKS_100K={}", large.chunks);

    assert_eq!(
        small.installed_count, N_SMALL as u64,
        "the n={} install landed {} entries — the peak describes a different workload",
        N_SMALL, small.installed_count
    );
    assert_eq!(
        large.installed_count, N_LARGE as u64,
        "the n={} install landed {} entries — the peak describes a different workload",
        N_LARGE, large.installed_count
    );
    assert_eq!(
        small.installed_hash.len(),
        64,
        "the witness must be a 32-byte hex hash"
    );
    assert_ne!(
        small.installed_hash, large.installed_hash,
        "the two runs produced the same digest — they measured one set twice"
    );
    assert!(
        small.peak_bytes > 0 && large.peak_bytes > 0,
        "a peak of 0 means the probe measured nothing"
    );
    assert!(
        large.set_bytes > 1_000_000,
        "M4_SET_BYTES_100K={} is too small to dominate harness noise",
        large.set_bytes
    );
    assert!(
        large.chunks > 1,
        "the session walked the whole set in {} message(s) — the chunked path is not being \
         measured",
        large.chunks
    );
}
