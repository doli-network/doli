//! Block-store integrity scan: the scan range and the missing-height report
//! shared by the 100-block `[INTEGRITY]` scan and `[INTEGRITY_CHECK]`, plus
//! the targeted archive repair with single-flight and back-off.
//!
//! Height 0 is the virtual genesis: `height_index[0]` exists but no block is
//! stored for it, so no scan visits it.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crypto::Hash;
use storage::archiver::{backfill_heights_from_archive, TargetedBackfillReport};
use storage::BlockStore;

/// Max missing heights kept in [`IntegrityReport::missing`]; the count stays exact.
pub(crate) const INTEGRITY_MISSING_LIST_CAP: usize = 1_000;
/// Missing heights named in one log line.
pub(crate) const INTEGRITY_LOG_HEIGHTS: usize = 10;
/// Max heights one targeted repair pass visits.
pub(crate) const INTEGRITY_REPAIR_MAX_HEIGHTS: usize = 1_000;
/// Scans skipped after a failed repair while the missing set is unchanged.
pub(crate) const INTEGRITY_REPAIR_BACKOFF_SCANS: u64 = 10;
/// Heights probed for the first stored block that names the genesis.
const GENESIS_PROBE_HEIGHTS: u64 = 10_000;

/// Result of one scan over `floor..=tip`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrityReport {
    pub floor: u64,
    pub tip: u64,
    pub missing_count: u64,
    /// Ascending, capped at `INTEGRITY_MISSING_LIST_CAP`.
    pub missing: Vec<u64>,
    /// BLAKE3 fold over the hashes of the served heights, ascending.
    pub commitment: Hash,
}

impl IntegrityReport {
    /// `"{count} missing blocks in {floor}..={tip}: [h, ...]"`, first
    /// `INTEGRITY_LOG_HEIGHTS` heights, then ` (+N more)`.
    pub(crate) fn missing_summary(&self) -> String {
        let shown: Vec<String> = self
            .missing
            .iter()
            .take(INTEGRITY_LOG_HEIGHTS)
            .map(u64::to_string)
            .collect();
        let mut line = format!(
            "{} missing blocks in {}..={}: [{}]",
            self.missing_count,
            self.floor,
            self.tip,
            shown.join(", ")
        );
        let more = self
            .missing_count
            .saturating_sub(INTEGRITY_LOG_HEIGHTS as u64);
        if more > 0 {
            let _ = write!(line, " (+{more} more)");
        }
        line
    }
}

fn block_stored(store: &BlockStore, height: u64) -> bool {
    matches!(store.get_block_by_height(height), Ok(Some(_)))
}

/// Lowest height the integrity scans check (always >= 1).
///
/// 1 when an archive is configured (it can refill any height) or block 1 is
/// stored (O(1) probe). Otherwise the lowest height >= 1 with a stored block:
/// a snap anchor S is index-only, so the floor is S+1; a pruned store gives
/// the cutoff. That path walks up from the numeric lowest index key.
pub(crate) fn scan_floor(store: &BlockStore, archive_configured: bool) -> u64 {
    if archive_configured || block_stored(store, 1) {
        return 1;
    }
    match store.height_range() {
        Ok(Some((lo, hi))) => (lo..=hi)
            .find(|&h| block_stored(store, h))
            .unwrap_or_else(|| hi.saturating_add(1)),
        _ => 1,
    }
}

/// Scan `scan_floor(..)..=scan_tip` with the RPC predicate
/// (`get_block_by_height` is `Ok(Some)`). Pure block-store reads.
pub(crate) fn integrity_scan(
    store: &BlockStore,
    scan_tip: u64,
    archive_configured: bool,
) -> IntegrityReport {
    let floor = scan_floor(store, archive_configured);
    let mut report = IntegrityReport {
        floor,
        tip: scan_tip,
        missing_count: 0,
        missing: Vec::new(),
        commitment: Hash::default(),
    };
    for h in floor..=scan_tip {
        match store.get_block_by_height(h) {
            Ok(Some(blk)) => {
                let mut hasher = crypto::Hasher::new();
                hasher.update(report.commitment.as_bytes());
                hasher.update(blk.hash().as_bytes());
                report.commitment = hasher.finalize();
            }
            _ => {
                report.missing_count += 1;
                if report.missing.len() < INTEGRITY_MISSING_LIST_CAP {
                    report.missing.push(h);
                }
            }
        }
    }
    report
}

/// Repair back-off state carried across 100-block scans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RepairBackoff {
    /// Number of the latest executed scan (first scan = 1).
    pub scans: u64,
    /// `(missing_digest, scan_no)` of the last repair that imported 0 or failed.
    pub failed: Option<(Hash, u64)>,
}

/// Digest of the missing set S: `missing_count` + listed heights; floor and tip excluded.
pub(crate) fn missing_digest(report: &IntegrityReport) -> Hash {
    let mut hasher = crypto::Hasher::new();
    hasher.update(&report.missing_count.to_le_bytes());
    for h in &report.missing {
        hasher.update(&h.to_le_bytes());
    }
    hasher.finalize()
}

/// Pure: true unless `failed == Some((digest, at))` and `scan_no <= at + INTEGRITY_REPAIR_BACKOFF_SCANS`.
pub(crate) fn should_attempt_repair(digest: &Hash, backoff: &RepairBackoff, scan_no: u64) -> bool {
    match backoff.failed {
        Some((d, at)) if d == *digest => {
            scan_no > at.saturating_add(INTEGRITY_REPAIR_BACKOFF_SCANS)
        }
        _ => true,
    }
}

/// Single-flight token for the integrity scan task; releases the flag on drop.
pub(crate) struct InFlightGuard(Arc<AtomicBool>);

impl InFlightGuard {
    /// `None` when a scan already holds the flag.
    pub(crate) fn try_acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Result of one [`scan_and_repair`] cycle.
#[derive(Debug)]
pub(crate) struct CycleOutcome {
    /// Final report: the re-scan after an import, else the first scan.
    pub report: IntegrityReport,
    /// `Some` iff a repair pass started.
    pub repair: Option<Result<TargetedBackfillReport, String>>,
}

/// Genesis named by the first stored block in `1..=GENESIS_PROBE_HEIGHTS`.
pub(crate) fn expected_genesis(store: &BlockStore) -> Option<Hash> {
    (1..=GENESIS_PROBE_HEIGHTS).find_map(|h| match store.get_block_by_height(h) {
        Ok(Some(blk)) => Some(blk.header.genesis_hash),
        _ => None,
    })
}

/// Targeted archive repair of the first `INTEGRITY_REPAIR_MAX_HEIGHTS` missing heights.
pub(crate) fn targeted_repair(
    store: &BlockStore,
    archive_dir: &Path,
    genesis: &Hash,
    report: &IntegrityReport,
) -> Result<TargetedBackfillReport, String> {
    let n = report.missing.len().min(INTEGRITY_REPAIR_MAX_HEIGHTS);
    backfill_heights_from_archive(archive_dir, store, genesis, &report.missing[..n])
}

/// One periodic scan + optional targeted repair. Blocking (RocksDB + file reads): call from spawn_blocking.
pub(crate) fn scan_and_repair(
    store: &BlockStore,
    scan_tip: u64,
    archive_dir: Option<&Path>,
    backoff: &mut RepairBackoff,
) -> CycleOutcome {
    backoff.scans += 1;
    let scan_no = backoff.scans;
    let report = integrity_scan(store, scan_tip, archive_dir.is_some());
    let idle = |report| CycleOutcome {
        report,
        repair: None,
    };
    if report.missing_count == 0 {
        backoff.failed = None;
        return idle(report);
    }
    let Some(dir) = archive_dir.filter(|d| d.exists()) else {
        tracing::warn!(
            "[INTEGRITY] {} — no archive configured, cannot auto-repair",
            report.missing_summary()
        );
        return idle(report);
    };
    let digest = missing_digest(&report);
    if !should_attempt_repair(&digest, backoff, scan_no) {
        let retry_at = backoff.failed.map_or(scan_no, |(_, at)| {
            at.saturating_add(INTEGRITY_REPAIR_BACKOFF_SCANS + 1)
        });
        tracing::info!(
            "[INTEGRITY] {} — repair backed off: same missing set as the last failed repair, retry in {} scans",
            report.missing_summary(),
            retry_at.saturating_sub(scan_no)
        );
        return idle(report);
    }
    let Some(genesis) = expected_genesis(store) else {
        tracing::warn!(
            "[INTEGRITY] {} — no stored block names the genesis, cannot auto-repair",
            report.missing_summary()
        );
        return idle(report);
    };
    tracing::info!(
        "[INTEGRITY] {} — auto-repairing from archive {:?}",
        report.missing_summary(),
        dir
    );
    let repair = targeted_repair(store, dir, &genesis, &report);
    match &repair {
        Ok(r) if r.imported > 0 => {
            tracing::info!(
                "[INTEGRITY] Auto-repair imported {} blocks from archive",
                r.imported
            );
            backoff.failed = None;
            let after = integrity_scan(store, scan_tip, true);
            if after.missing_count > 0 {
                tracing::warn!(
                    "[INTEGRITY] After auto-repair: still {} (archive may be incomplete)",
                    after.missing_summary()
                );
            }
            return CycleOutcome {
                report: after,
                repair: Some(repair),
            };
        }
        Ok(r) => tracing::warn!(
            "[INTEGRITY] Auto-repair imported 0 blocks ({} files missing, {} rejected) — next attempt in {} scans unless the missing set changes",
            r.missing_files,
            r.rejected,
            INTEGRITY_REPAIR_BACKOFF_SCANS + 1
        ),
        Err(e) => tracing::error!("[INTEGRITY] Auto-repair failed: {}", e),
    }
    backoff.failed = Some((digest, scan_no));
    CycleOutcome {
        report,
        repair: Some(repair),
    }
}
