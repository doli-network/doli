//! UTXO-scalability M4 [F3] — the peak-ratio CLAIM (REQ-SCALE-014).
//!
//! OUTPUT CONTRACT: the client-side download+install path measured by
//!   `m4_measure::measure_download_install`.
//!   Outputs observable from the stable API:
//!     O1 the allocator high-water delta over the measured window
//!     O2 `StateDb::staged_utxo_len()` at session end
//!     O3 `VerifiedSnapshot.utxo_set` — empty (staged) or a whole image
//!   Paths: P1 n = 50_000, sink installed   P2 n = 100_000, sink installed
//!   MATRIX: P1xO1 + P2xO1 -> the ratio | P2xO2 -> every entry staged
//!           P2xO3 -> no whole image on the snapshot
//! INPUT PARTITIONS: two set sizes differing by exactly 2x from one seeded generator, so a
//!   ratio near 2.0 can only mean the peak still tracks the set size.
//!
//! Its own binary, separate from `m4_install_peak_probe.rs`: the probe must PASS on pre-M4
//! code to yield the `before` reading, and this file must FAIL. Both include ONE copy of the
//! allocator and the measurement routine (`tests/m4_measure/mod.rs`).

#[path = "m4_measure/mod.rs"]
mod m;

const N_SMALL: usize = 50_000;
const N_LARGE: usize = 100_000;
/// Flat within harness noise. The pre-M4 path reassembles the whole image, so its ratio
/// tracks n and lands near 2.0; anything at or below this can only be O(chunk).
const MAX_FLAT_RATIO: f64 = 1.25;

/// REQ-SCALE-014 — Decision: a failure here means the receiving node still holds a term
/// proportional to the whole UTXO set in RAM while installing it, so the RAM an operator
/// needs to join the network keeps growing with the chain — the exact ceiling M4 exists to
/// remove, one layer below the wire bound M3 removed.
/// INV-SYNC-014 — Decision: `staged_len` below `n` at session end means some chunks never
/// reached the staging family, so the promotion would install a truncated set that nothing
/// downstream rejects.
#[tokio::test(flavor = "multi_thread")]
async fn m4_install_peak_is_flat_in_set_size() {
    let small = m::measure_download_install(N_SMALL, m::PROBE_SEED).await;
    let large = m::measure_download_install(N_LARGE, m::PROBE_SEED).await;

    assert_eq!(
        large.installed_count, N_LARGE as u64,
        "the install landed {} of {} entries — a flat peak over a truncated set proves \
         nothing",
        large.installed_count, N_LARGE
    );

    assert_eq!(
        large.staged_len_at_session_end, N_LARGE as u64,
        "staging held {} of {} rows when the session closed — the chunks are not being \
         streamed into the staging family",
        large.staged_len_at_session_end, N_LARGE
    );
    assert_eq!(
        large.transfer_path, "staged_chunks",
        "the completed session still carries a whole UTXO image on VerifiedSnapshot — the \
         O(set) term moved, it did not go away"
    );

    let ratio = large.peak_bytes as f64 / small.peak_bytes as f64;
    assert!(
        ratio <= MAX_FLAT_RATIO,
        "peak(n={}) / peak(n={}) = {:.2} > {:.2} — the install peak still tracks the set \
         size ({} B vs {} B)",
        N_LARGE,
        N_SMALL,
        ratio,
        MAX_FLAT_RATIO,
        large.peak_bytes,
        small.peak_bytes
    );
}
