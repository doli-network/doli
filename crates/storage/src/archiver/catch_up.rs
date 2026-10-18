use std::io;
use std::ops::RangeInclusive;
use std::path::Path;

use crypto::Hash;
use doli_core::transaction::legacy::deserialize_block_compat;
use tracing::{debug, error, info, warn};

use super::manifest::{self, Manifest};
use super::write::{block_path, pair_exists, write_pair};
use super::{ArchiveBlock, CatchUpReport};
use crate::BlockStore;

/// Resumable-save cadence of a long scan, in heights.
const SAVE_EVERY: u64 = 1000;

enum Check {
    Valid(Hash),
    Foreign,
    Unusable,
}

enum Step {
    Done(Hash),
    Stall(&'static str),
}

/// What the scan may keep from the previous manifest.
struct Base {
    trusted: Option<u64>,
    latest: u64,
    latest_hash: Option<String>,
}

struct Scan<'a> {
    dir: &'a Path,
    expected: &'a Hash,
    report: CatchUpReport,
    contiguous: bool,
    last: Option<Hash>,
    quiet: bool,
    stalled: u64,
    stale: u64,
}

fn check_file(dir: &Path, h: u64, expected: &Hash) -> Check {
    let Ok(data) = std::fs::read(block_path(dir, h)) else {
        return Check::Unusable;
    };
    match deserialize_block_compat(&data) {
        Some(b) if b.header.genesis_hash == *expected => Check::Valid(b.hash()),
        Some(_) => Check::Foreign,
        None => Check::Unusable,
    }
}

fn from_store(store: &BlockStore, h: u64, expected: &Hash) -> Option<(Vec<u8>, Hash)> {
    let b = store
        .get_block_by_height(h)
        .ok()
        .flatten()
        .filter(|b| b.header.genesis_hash == *expected)?;
    let data = bincode::serialize(&b).ok()?;
    Some((data, b.hash()))
}

fn from_block(block: &ArchiveBlock, expected: &Hash) -> Option<(Vec<u8>, Hash)> {
    deserialize_block_compat(&block.data)
        .filter(|b| b.header.genesis_hash == *expected)
        .map(|_| (block.data.clone(), block.hash))
}

/// Loads the manifest; H is trusted only when bound to `expected`, not above
/// `latest_height`, and file H passes the spot-check.
fn load_base(dir: &Path, expected: &Hash, report: &mut CatchUpReport) -> Base {
    let m = Manifest::load(dir).unwrap_or_default();
    let ours = m.genesis_hash.as_deref() == Some(expected.to_string().as_str());
    let latest = m.latest_height.filter(|_| ours);
    let claimed = m
        .verified_height
        .filter(|h| ours && latest.is_some_and(|l| *h <= l));
    let trusted = claimed.filter(|&h| h == 0 || spot_check(dir, h, expected, report));
    Base {
        trusted,
        latest: latest.unwrap_or(0),
        latest_hash: m.latest_hash.filter(|_| ours),
    }
}

fn spot_check(dir: &Path, h: u64, expected: &Hash, report: &mut CatchUpReport) -> bool {
    if !pair_exists(dir, h) {
        return false;
    }
    report.files_read += 1;
    matches!(check_file(dir, h, expected), Check::Valid(_))
}

impl<'a> Scan<'a> {
    fn new(dir: &'a Path, expected: &'a Hash, start: u64, report: CatchUpReport) -> Self {
        Self {
            dir,
            expected,
            report: CatchUpReport {
                verified_height: start,
                ..report
            },
            contiguous: true,
            last: None,
            quiet: false,
            stalled: 0,
            stale: 0,
        }
    }

    fn step(
        &mut self,
        h: u64,
        source: impl FnOnce() -> Option<(Vec<u8>, Hash)>,
    ) -> io::Result<Step> {
        if pair_exists(self.dir, h) {
            self.report.files_read += 1;
            match check_file(self.dir, h, self.expected) {
                Check::Valid(hash) => return Ok(Step::Done(hash)),
                Check::Unusable => {
                    return Ok(Step::Stall("file unreadable or undeserializable, kept"))
                }
                Check::Foreign => {
                    self.stale += 1;
                    if self.stale <= 10 {
                        warn!("[ARCHIVER] Stale archive file at height {} (other genesis) — overwriting", h);
                    }
                }
            }
        }
        let Some((data, hash)) = source() else {
            return Ok(Step::Stall(
                "no verified file and no block in the block store",
            ));
        };
        write_pair(self.dir, h, &data)?;
        self.report.files_written += 1;
        Ok(Step::Done(hash))
    }

    fn visit(
        &mut self,
        h: u64,
        source: impl FnOnce() -> Option<(Vec<u8>, Hash)>,
    ) -> io::Result<()> {
        match self.step(h, source)? {
            Step::Done(hash) => {
                self.last = Some(hash);
                if self.contiguous {
                    self.report.verified_height = h;
                }
            }
            Step::Stall(why) => {
                if self.contiguous && !self.quiet {
                    warn!("[ARCHIVER] Verified height stops below {}: {}", h, why);
                } else if self.contiguous {
                    debug!("[ARCHIVER] Verified height stops below {}: {}", h, why);
                }
                self.contiguous = false;
                self.stalled += 1;
            }
        }
        Ok(())
    }

    /// Visits `range` in order. On a write error the reached H is saved first.
    fn run(
        &mut self,
        range: RangeInclusive<u64>,
        source: impl Fn(u64) -> Option<(Vec<u8>, Hash)>,
        stop_at_stall: bool,
        save: &dyn Fn(&Scan) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut saved = self.report.verified_height;
        for h in range {
            if let Err(e) = self.visit(h, || source(h)) {
                let _ = save(self);
                return Err(e);
            }
            if stop_at_stall && !self.contiguous {
                break;
            }
            if h.is_multiple_of(SAVE_EVERY) && self.report.verified_height > saved {
                save(self)?;
                saved = self.report.verified_height;
            }
        }
        Ok(())
    }
}

fn save_scan(dir: &Path, expected: &Hash, base: &Base, latest: u64, scan: &Scan) -> io::Result<()> {
    let last = scan.last.map(|h| h.to_string());
    let hash = if latest == base.latest {
        base.latest_hash.clone().or(last)
    } else {
        last
    };
    manifest::save(
        dir,
        latest,
        &hash.unwrap_or_else(|| Hash::ZERO.to_string()),
        Some(&expected.to_string()),
        Some(scan.report.verified_height),
    )
}

fn changed(base: &Base, latest: u64, scan: &Scan) -> bool {
    base.trusted != Some(scan.report.verified_height) || base.latest != latest
}

pub(super) fn catch_up_bounded(
    dir: &Path,
    store: &BlockStore,
    tip: u64,
    expected: &Hash,
) -> io::Result<CatchUpReport> {
    std::fs::create_dir_all(dir)?;
    let mut report = CatchUpReport::default();
    let base = load_base(dir, expected, &mut report);
    let start = base.trusted.unwrap_or(0);
    let latest = base.latest.max(tip);
    if base.trusted.is_none() {
        info!(
            "[ARCHIVER] No trusted verified height: full check of 1..={}",
            tip
        );
    }
    let mut scan = Scan::new(dir, expected, start, report);
    let save = |s: &Scan| save_scan(dir, expected, &base, latest, s);
    scan.run(
        start.saturating_add(1)..=tip,
        |h| from_store(store, h, expected),
        false,
        &save,
    )?;
    if changed(&base, latest, &scan) {
        save(&scan)?;
    }
    if scan.report.files_written > 0 || scan.stalled > 0 || base.trusted.is_none() {
        info!(
            "[ARCHIVER] Catch-up complete: verified_height={} read={} written={} stalled={} (tip {})",
            scan.report.verified_height, scan.report.files_read, scan.report.files_written, scan.stalled, tip
        );
    }
    Ok(scan.report)
}

/// Live path for one finalized block: fills H+1..h-1 from the block store
/// (stopping at the first height it cannot verify or write), writes h, and
/// advances H contiguously.
pub fn archive_finalized(
    dir: &Path,
    block_store: &BlockStore,
    block: &ArchiveBlock,
    expected_genesis: &Hash,
) -> io::Result<CatchUpReport> {
    std::fs::create_dir_all(dir)?;
    let mut report = CatchUpReport::default();
    let base = load_base(dir, expected_genesis, &mut report);
    let start = base.trusted.unwrap_or(0);
    let mut scan = Scan::new(dir, expected_genesis, start, report);
    if block.height <= start {
        return Ok(scan.report);
    }
    scan.quiet = true;
    let latest = base.latest.max(block.height);
    let save = |s: &Scan| save_scan(dir, expected_genesis, &base, latest, s);
    scan.run(
        start + 1..=block.height - 1,
        |h| from_store(block_store, h, expected_genesis),
        true,
        &save,
    )?;
    scan.run(
        block.height..=block.height,
        |_| from_block(block, expected_genesis),
        false,
        &save,
    )?;
    if changed(&base, latest, &scan) {
        save(&scan)?;
    }
    Ok(scan.report)
}

/// `catch_up` without an expected genesis: presence-only, as before.
pub(super) fn catch_up_presence(dir: &Path, store: &BlockStore, tip: u64) -> io::Result<u64> {
    std::fs::create_dir_all(dir)?;
    let mut archived = 0u64;
    let mut last = None;
    for h in 1..=tip {
        if block_path(dir, h).exists() {
            continue;
        }
        let block = match store.get_block_by_height(h) {
            Ok(Some(b)) => b,
            Ok(None) => {
                warn!(
                    "[ARCHIVER] Block at height {} not found during catch-up, stopping",
                    h
                );
                break;
            }
            Err(e) => {
                warn!("[ARCHIVER] Error reading block {}: {}", h, e);
                break;
            }
        };
        let data = match bincode::serialize(&block) {
            Ok(d) => d,
            Err(e) => {
                error!("[ARCHIVER] Serialize error at height {}: {}", h, e);
                break;
            }
        };
        write_pair(dir, h, &data)?;
        last = Some((block.hash(), block.header.genesis_hash));
        archived += 1;
    }
    if let Some((hash, genesis)) = last {
        manifest::save_carrying(dir, tip, &hash, Some(&genesis))?;
        info!(
            "[ARCHIVER] Catch-up complete: {} new in 1..={}",
            archived, tip
        );
    }
    Ok(archived)
}
