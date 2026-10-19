//! Block-store integrity scan: the scan range and the missing-height report
//! shared by the 100-block `[INTEGRITY]` scan and `[INTEGRITY_CHECK]`.
//!
//! Height 0 is the virtual genesis: `height_index[0]` exists but no block is
//! stored for it, so no scan visits it.

use std::fmt::Write as _;

use crypto::Hash;
use storage::BlockStore;

/// Max missing heights kept in [`IntegrityReport::missing`]; the count stays exact.
pub(crate) const INTEGRITY_MISSING_LIST_CAP: usize = 1_000;
/// Missing heights named in one log line.
pub(crate) const INTEGRITY_LOG_HEIGHTS: usize = 10;

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
