use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// A block ready to be archived.
pub struct ArchiveBlock {
    pub height: u64,
    pub hash: crypto::Hash,
    pub data: Vec<u8>,
}

/// Archives blocks to a filesystem directory.
///
/// Each block is stored as `{height:010}.block` with a BLAKE3 checksum sidecar
/// (`{height:010}.blake3`) for integrity verification. A manifest tracks the
/// latest archived height and genesis_hash for chain identity.
pub struct BlockArchiver {
    rx: mpsc::Receiver<ArchiveBlock>,
    dir: PathBuf,
}

impl BlockArchiver {
    pub fn new(rx: mpsc::Receiver<ArchiveBlock>, dir: PathBuf) -> Self {
        Self { rx, dir }
    }

    /// Run the archiver loop. Call this in a tokio::spawn.
    pub async fn run(mut self) {
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            error!(
                "[ARCHIVER] Failed to create archive dir {:?}: {}",
                self.dir, e
            );
            return;
        }

        let last = self.read_manifest_height();
        info!(
            "[ARCHIVER] Started — dir={:?}, last_archived={}",
            self.dir,
            last.unwrap_or(0)
        );

        while let Some(block) = self.rx.recv().await {
            if let Err(e) = self.archive_block(&block) {
                error!("[ARCHIVER] Failed to archive block {}: {}", block.height, e);
            }
        }

        info!("[ARCHIVER] Channel closed, shutting down");
    }

    /// Catch up: archive all blocks from 1 to `tip` using the BlockStore.
    /// Skips blocks whose .block file already exists on disk to avoid redundant I/O.
    /// Fixes gaps left when the manifest advanced but intermediate files are missing.
    ///
    /// If `expected_genesis_hash` is provided, existing archive files are verified:
    /// stale files from a previous chain are overwritten with the correct block.
    pub fn catch_up(
        dir: &Path,
        block_store: &super::BlockStore,
        tip: u64,
        expected_genesis_hash: Option<&crypto::Hash>,
    ) -> Result<u64, std::io::Error> {
        std::fs::create_dir_all(dir)?;

        let mut archived = 0u64;
        let mut last_hash = None;
        let mut last_genesis_hash = None;
        let mut overwritten = 0u64;
        for h in 1..=tip {
            let block_path = dir.join(format!("{:010}.block", h));
            if block_path.exists() {
                // Verify genesis hash of existing file if we have an expected value
                if let Some(expected) = expected_genesis_hash {
                    if let Ok(data) = std::fs::read(&block_path) {
                        if let Some(block) =
                            doli_core::transaction::legacy::deserialize_block_compat(&data)
                        {
                            if block.header.genesis_hash != *expected {
                                warn!(
                                    "[ARCHIVER] Stale archive file at height {} \
                                     (genesis={}, expected={}) — overwriting",
                                    h,
                                    &block.header.genesis_hash.to_string()[..16],
                                    &expected.to_string()[..16]
                                );
                                overwritten += 1;
                                // Fall through to re-archive from block store
                            } else {
                                continue; // File is valid, skip
                            }
                        } else {
                            continue; // Can't deserialize, skip
                        }
                    } else {
                        continue; // Can't read, skip
                    }
                } else {
                    continue; // No genesis hash to verify, skip as before
                }
            }

            let block = match block_store.get_block_by_height(h) {
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

            last_hash = Some(block.hash());
            last_genesis_hash = Some(block.header.genesis_hash);
            write_block_file(dir, h, &data)?;
            write_checksum_file(dir, h, &data)?;
            archived += 1;

            if archived.is_multiple_of(100) {
                info!("[ARCHIVER] Catch-up progress: {}/{}", h, tip);
            }
        }

        // Update manifest to reflect actual tip
        if let (Some(hash), Some(genesis_hash)) = (last_hash, last_genesis_hash) {
            write_manifest(dir, tip, &hash, &genesis_hash)?;
        }

        if archived > 0 || overwritten > 0 {
            info!(
                "[ARCHIVER] Catch-up complete: {} new, {} overwritten (stale) in 1..={}",
                archived, overwritten, tip
            );
        }

        Ok(archived)
    }

    fn archive_block(&self, block: &ArchiveBlock) -> Result<(), std::io::Error> {
        let block_path = self.dir.join(format!("{:010}.block", block.height));

        // Skip if already archived
        if block_path.exists() {
            return Ok(());
        }

        write_block_file(&self.dir, block.height, &block.data)?;
        write_checksum_file(&self.dir, block.height, &block.data)?;

        // Derive genesis_hash from block data for manifest
        if let Some(b) = doli_core::transaction::legacy::deserialize_block_compat(&block.data) {
            write_manifest(&self.dir, block.height, &block.hash, &b.header.genesis_hash)?;
        } else {
            write_manifest_without_genesis(&self.dir, block.height, &block.hash)?;
        }

        if block.height.is_multiple_of(100) {
            info!(
                "[ARCHIVER] Archived block {} (hash={})",
                block.height,
                &block.hash.to_string()[..16]
            );
        }

        Ok(())
    }

    fn read_manifest_height(&self) -> Option<u64> {
        read_manifest_height_from(&self.dir)
    }
}

fn blake3_hex(data: &[u8]) -> String {
    let hash = crypto::hash::hash(data);
    hash.to_string()
}

fn write_block_file(dir: &Path, height: u64, data: &[u8]) -> Result<(), std::io::Error> {
    let block_path = dir.join(format!("{:010}.block", height));
    let tmp_path = dir.join(format!("{:010}.block.tmp", height));
    std::fs::write(&tmp_path, data)?;
    std::fs::rename(&tmp_path, &block_path)?;
    Ok(())
}

fn write_checksum_file(dir: &Path, height: u64, data: &[u8]) -> Result<(), std::io::Error> {
    let checksum = blake3_hex(data);
    let path = dir.join(format!("{:010}.blake3", height));
    let tmp = dir.join(format!("{:010}.blake3.tmp", height));
    std::fs::write(&tmp, &checksum)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Read the latest archived height from the manifest file.
pub fn manifest_height(dir: &Path) -> Option<u64> {
    read_manifest_height_from(dir)
}

fn read_manifest_height_from(dir: &Path) -> Option<u64> {
    let manifest_path = dir.join("manifest.json");
    let data = std::fs::read_to_string(&manifest_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("latest_height")?.as_u64()
}

/// Read the genesis_hash from the archive manifest file.
pub fn manifest_genesis_hash(dir: &Path) -> Option<String> {
    read_manifest_genesis_hash(dir)
}

fn read_manifest_genesis_hash(dir: &Path) -> Option<String> {
    let manifest_path = dir.join("manifest.json");
    let data = std::fs::read_to_string(&manifest_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("genesis_hash")?.as_str().map(|s| s.to_string())
}

fn write_manifest(
    dir: &Path,
    height: u64,
    hash: &crypto::Hash,
    genesis_hash: &crypto::Hash,
) -> Result<(), std::io::Error> {
    let manifest = serde_json::json!({
        "latest_height": height,
        "latest_hash": hash.to_string(),
        "genesis_hash": genesis_hash.to_string(),
    });

    let manifest_path = dir.join("manifest.json");
    let tmp_path = dir.join("manifest.json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&manifest).unwrap())?;
    std::fs::rename(&tmp_path, &manifest_path)?;
    Ok(())
}

fn write_manifest_without_genesis(
    dir: &Path,
    height: u64,
    hash: &crypto::Hash,
) -> Result<(), std::io::Error> {
    let manifest = serde_json::json!({
        "latest_height": height,
        "latest_hash": hash.to_string(),
    });

    let manifest_path = dir.join("manifest.json");
    let tmp_path = dir.join("manifest.json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&manifest).unwrap())?;
    std::fs::rename(&tmp_path, &manifest_path)?;
    Ok(())
}

/// Restore chain from an archive directory. Returns the number of blocks imported.
///
/// Verifies BLAKE3 checksums for each block and validates genesis_hash consistency.
/// After import, caller should run `recover --yes` to rebuild UTXO/producer state.
pub fn restore_from_archive(
    archive_dir: &Path,
    block_store: &super::BlockStore,
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
    block_store: &super::BlockStore,
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
    block_store: &super::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
) -> Result<u64, String> {
    import_archive_blocks(archive_dir, block_store, expected_genesis_hash, true, true)
}

fn import_archive_blocks(
    archive_dir: &Path,
    block_store: &super::BlockStore,
    expected_genesis_hash: Option<&crypto::Hash>,
    skip_existing: bool,
    force_replace: bool,
) -> Result<u64, String> {
    let manifest_height = read_manifest_height_from(archive_dir)
        .ok_or_else(|| "No manifest.json found in archive directory".to_string())?;

    // Validate genesis_hash from manifest if caller provided expected value
    if let Some(expected) = expected_genesis_hash {
        if let Some(archive_genesis) = read_manifest_genesis_hash(archive_dir) {
            let expected_str = expected.to_string();
            if archive_genesis != expected_str {
                return Err(format!(
                    "Genesis hash mismatch: archive={}, expected={}. Wrong chain!",
                    &archive_genesis[..16],
                    &expected_str[..16]
                ));
            }
        }
    }

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

        let block_path = archive_dir.join(format!("{:010}.block", h));
        let data = match std::fs::read(&block_path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing_files += 1;
                if missing_files <= 10 {
                    warn!("[ARCHIVER] Archive file missing at height {} — skipping", h);
                }
                continue;
            }
            Err(e) => {
                return Err(format!("Failed to read block file at height {}: {}", h, e));
            }
        };

        // Verify BLAKE3 checksum if sidecar exists
        let checksum_path = archive_dir.join(format!("{:010}.blake3", h));
        if checksum_path.exists() {
            let expected_checksum = std::fs::read_to_string(&checksum_path)
                .map_err(|e| format!("Failed to read checksum at height {}: {}", h, e))?;
            let actual_checksum = blake3_hex(&data);
            if actual_checksum.trim() != expected_checksum.trim() {
                return Err(format!(
                    "Checksum mismatch at height {}: expected={}, got={}",
                    h,
                    &expected_checksum.trim()[..16],
                    &actual_checksum[..16]
                ));
            }
        }

        let block: doli_core::Block =
            doli_core::transaction::legacy::deserialize_block_compat(&data).ok_or_else(|| {
                format!(
                    "Failed to deserialize block at height {} (tried current and legacy formats)",
                    h
                )
            })?;

        // Validate genesis_hash consistency within the archive
        if let Some(expected) = expected_genesis_hash {
            if block.header.genesis_hash != *expected {
                return Err(format!("Block {} has wrong genesis_hash (wrong chain)", h));
            }
        }

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
