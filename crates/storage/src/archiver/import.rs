use std::path::Path;

use tracing::{error, info, warn};

use super::manifest::{read_manifest_genesis_hash, read_manifest_height_from};
use super::write::blake3_hex;

/// Restore chain from an archive directory. Returns the number of blocks imported.
///
/// Verifies BLAKE3 checksums for each block and validates genesis_hash consistency.
/// After import, caller should run `recover --yes` to rebuild UTXO/producer state.
pub fn restore_from_archive(
    archive_dir: &Path,
    block_store: &crate::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
) -> Result<u64, String> {
    import_archive_blocks(
        archive_dir,
        block_store,
        expected_genesis_hash,
        false,
        false,
    )
}

/// Backfill missing blocks from an archive directory. Returns the number of blocks imported.
///
/// Like `restore_from_archive` but skips blocks that already exist in the BlockStore.
/// Designed for filling gaps left by snap sync without touching existing state.
pub fn backfill_from_archive(
    archive_dir: &Path,
    block_store: &crate::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
) -> Result<u64, String> {
    import_archive_blocks(archive_dir, block_store, expected_genesis_hash, true, false)
}

/// Force-backfill: like backfill, but also replaces existing blocks that differ
/// from the archive (detected via BLAKE3 checksum comparison).
///
/// INC-I-055: "fill gaps" vs "fix forks". Normal backfill skips any height where
/// a block exists. Force mode compares the existing block's serialized data against
/// the archive checksum and replaces mismatches. This fixes blocks from a fork that
/// were stored before a genesis reset.
pub fn force_backfill_from_archive(
    archive_dir: &Path,
    block_store: &crate::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
) -> Result<u64, String> {
    import_archive_blocks(archive_dir, block_store, expected_genesis_hash, true, true)
}

/// Outcome of [`backfill_heights_from_archive`]: one counter per per-height result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TargetedBackfillReport {
    pub imported: u64,
    pub already_present: u64,
    pub missing_files: u64,
    pub rejected: u64,
    pub heights_visited: u64,
    pub files_read: u64,
}

/// Backfill exactly `heights` from the archive; a bad height is counted and skipped.
/// `Err` only when the manifest is absent or names another genesis.
pub fn backfill_heights_from_archive(
    archive_dir: &Path,
    block_store: &crate::BlockStore,
    expected_genesis_hash: &crypto::Hash,
    heights: &[u64],
) -> Result<TargetedBackfillReport, String> {
    check_manifest(archive_dir, Some(expected_genesis_hash))?;
    let mut r = TargetedBackfillReport::default();
    for &h in heights {
        r.heights_visited += 1;
        if h == 0 {
            r.rejected += 1;
            error!("[ARCHIVER] Targeted backfill rejected h=0: virtual genesis");
            continue;
        }
        if matches!(block_store.get_block_by_height(h), Ok(Some(_))) {
            r.already_present += 1;
            continue;
        }
        let block = match read_archived_block(archive_dir, h, Some(expected_genesis_hash)) {
            Ok(b) => b,
            Err(ArchiveRead::Missing) => {
                r.missing_files += 1;
                if r.missing_files <= 10 {
                    warn!("[ARCHIVER] Archive file missing at height {} — skipping", h);
                }
                continue;
            }
            Err(e) => {
                if matches!(e, ArchiveRead::Invalid(_)) {
                    r.files_read += 1;
                }
                r.rejected += 1;
                error!(
                    "[ARCHIVER] Targeted backfill rejected h={}: {}",
                    h,
                    e.message()
                );
                continue;
            }
        };
        r.files_read += 1;
        let stored = check_links(block_store, h, &block).and_then(|()| {
            block_store
                .put_block_canonical(&block, h)
                .map_err(|e| format!("Failed to store block {}: {}", h, e))
        });
        match stored {
            Ok(()) => r.imported += 1,
            Err(e) => {
                r.rejected += 1;
                error!("[ARCHIVER] Targeted backfill rejected h={}: {}", h, e);
            }
        }
    }
    info!(
        "[ARCHIVER] Targeted backfill: {} heights, {} imported, {} present, {} files missing, {} rejected",
        r.heights_visited, r.imported, r.already_present, r.missing_files, r.rejected
    );
    Ok(r)
}

enum ArchiveRead {
    Missing,
    Unreadable(String),
    Invalid(String),
}

impl ArchiveRead {
    fn message(self) -> String {
        match self {
            Self::Missing => "archive file missing".to_string(),
            Self::Unreadable(e) | Self::Invalid(e) => e,
        }
    }
}

fn prefix16(s: &str) -> &str {
    s.get(..16).unwrap_or(s)
}

/// Returns the manifest height after the optional manifest genesis check.
fn check_manifest(archive_dir: &Path, expected: Option<&crypto::Hash>) -> Result<u64, String> {
    let manifest_height = read_manifest_height_from(archive_dir)
        .ok_or_else(|| "No manifest.json found in archive directory".to_string())?;
    if let (Some(expected), Some(archive_genesis)) =
        (expected, read_manifest_genesis_hash(archive_dir))
    {
        let expected_str = expected.to_string();
        if archive_genesis != expected_str {
            return Err(format!(
                "Genesis hash mismatch: archive={}, expected={}. Wrong chain!",
                prefix16(&archive_genesis),
                prefix16(&expected_str)
            ));
        }
    }
    Ok(manifest_height)
}

/// Read `{h:010}.block`, verify its `.blake3` sidecar, decode, check the genesis.
fn read_archived_block(
    archive_dir: &Path,
    h: u64,
    expected_genesis_hash: Option<&crypto::Hash>,
) -> Result<doli_core::Block, ArchiveRead> {
    let block_path = archive_dir.join(format!("{:010}.block", h));
    let data = match std::fs::read(&block_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ArchiveRead::Missing),
        Err(e) => {
            return Err(ArchiveRead::Unreadable(format!(
                "Failed to read block file at height {}: {}",
                h, e
            )))
        }
    };

    let checksum_path = archive_dir.join(format!("{:010}.blake3", h));
    if checksum_path.exists() {
        let expected_checksum = std::fs::read_to_string(&checksum_path).map_err(|e| {
            ArchiveRead::Invalid(format!("Failed to read checksum at height {}: {}", h, e))
        })?;
        let actual_checksum = blake3_hex(&data);
        if actual_checksum.trim() != expected_checksum.trim() {
            return Err(ArchiveRead::Invalid(format!(
                "Checksum mismatch at height {}: expected={}, got={}",
                h,
                prefix16(expected_checksum.trim()),
                prefix16(&actual_checksum)
            )));
        }
    }

    let block =
        doli_core::transaction::legacy::deserialize_block_compat(&data).ok_or_else(|| {
            ArchiveRead::Invalid(format!(
                "Failed to deserialize block at height {} (tried current and legacy formats)",
                h
            ))
        })?;

    if let Some(expected) = expected_genesis_hash {
        if block.header.genesis_hash != *expected {
            return Err(ArchiveRead::Invalid(format!(
                "Block {} has wrong genesis_hash (wrong chain)",
                h
            )));
        }
    }
    Ok(block)
}

/// SEC-002: the block must link to the stored parent and the stored child.
fn check_links(
    block_store: &crate::BlockStore,
    h: u64,
    block: &doli_core::Block,
) -> Result<(), String> {
    if let Ok(Some(parent)) = block_store.get_hash_by_height(h - 1) {
        if block.header.prev_hash != parent {
            return Err(format!("prev_hash does not link to stored h={}", h - 1));
        }
    }
    if let Ok(Some(child)) = block_store.get_block_by_height(h.saturating_add(1)) {
        if child.header.prev_hash != block.hash() {
            return Err(format!(
                "stored h={} does not link to this block",
                h.saturating_add(1)
            ));
        }
    }
    Ok(())
}

fn import_archive_blocks(
    archive_dir: &Path,
    block_store: &crate::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
    skip_existing: bool,
    force_replace: bool,
) -> Result<u64, String> {
    let manifest_height = check_manifest(archive_dir, expected_genesis_hash)?;

    let mode = if skip_existing {
        "Backfilling"
    } else {
        "Restoring"
    };
    info!(
        "[ARCHIVER] {} from archive: {} blocks available",
        mode, manifest_height
    );

    let mut imported = 0u64;
    let mut replaced = 0u64;
    let mut skipped = 0u64;
    let mut missing_files = 0u64;
    for h in 1..=manifest_height {
        // Skip or compare blocks that already exist in the BlockStore
        if skip_existing {
            if let Ok(Some(existing)) = block_store.get_block_by_height(h) {
                if force_replace {
                    // Force mode: compare existing block hash against archive checksum.
                    // If they match, skip. If they differ, fall through to replace.
                    let checksum_path = archive_dir.join(format!("{:010}.blake3", h));
                    if checksum_path.exists() {
                        if let Ok(existing_data) = bincode::serialize(&existing) {
                            let existing_checksum = blake3_hex(&existing_data);
                            let archive_checksum =
                                std::fs::read_to_string(&checksum_path).unwrap_or_default();
                            if existing_checksum.trim() == archive_checksum.trim() {
                                skipped += 1;
                                continue; // Block matches archive — skip
                            }
                            info!(
                                "[ARCHIVER] Force-replacing divergent block at h={} (local={:.16} archive={:.16})",
                                h, existing_checksum, archive_checksum.trim()
                            );
                            replaced += 1;
                            // Fall through to import from archive
                        } else {
                            skipped += 1;
                            continue; // Can't serialize to compare — skip
                        }
                    } else {
                        skipped += 1;
                        continue; // No checksum to compare — skip
                    }
                } else {
                    skipped += 1;
                    continue;
                }
            }
        }

        let block = match read_archived_block(archive_dir, h, expected_genesis_hash) {
            Ok(b) => b,
            Err(ArchiveRead::Missing) => {
                missing_files += 1;
                if missing_files <= 10 {
                    warn!("[ARCHIVER] Archive file missing at height {} — skipping", h);
                }
                continue;
            }
            Err(e) => return Err(e.message()),
        };

        let hash_str = block.hash().to_string();
        block_store
            .put_block_canonical(&block, h)
            .map_err(|e| format!("Failed to store block {}: {}", h, e))?;

        imported += 1;
        if imported.is_multiple_of(100) {
            info!(
                "[ARCHIVER] {} {}/{} blocks (hash={}...)",
                mode,
                h,
                manifest_height,
                &hash_str[..16]
            );
        }
    }

    if missing_files > 0 {
        warn!(
            "[ARCHIVER] {} archive files were missing and skipped",
            missing_files
        );
    }

    if skip_existing {
        if replaced > 0 {
            info!(
                "[ARCHIVER] Force-backfill complete: {} imported, {} replaced, {} present, {} missing",
                imported, replaced, skipped, missing_files
            );
        } else {
            info!(
                "[ARCHIVER] Backfill complete: {} blocks imported, {} already present, {} files missing",
                imported, skipped, missing_files
            );
        }
    } else {
        info!(
            "[ARCHIVER] Restore complete: {} blocks imported, {} files missing. \
             Run 'doli-node recover --yes' to rebuild state.",
            imported, missing_files
        );
    }

    Ok(imported)
}
