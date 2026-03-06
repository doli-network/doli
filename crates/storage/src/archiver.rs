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
/// Each block is stored as `{height:010}.block` with a manifest tracking
/// the latest archived height. On startup, catches up from the last archived
/// height by reading missing blocks from the local BlockStore.
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

    /// Catch up: archive all blocks from `last_archived+1` to `tip` using the BlockStore.
    pub fn catch_up(
        dir: &Path,
        block_store: &super::BlockStore,
        tip: u64,
    ) -> Result<u64, std::io::Error> {
        std::fs::create_dir_all(dir)?;

        let last = read_manifest_height_from(dir).unwrap_or(0);
        if last >= tip {
            return Ok(0);
        }

        let mut archived = 0u64;
        for h in (last + 1)..=tip {
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

            let hash = block.hash();
            let block_path = dir.join(format!("{:010}.block", h));

            // Atomic write: tmp + rename
            let tmp_path = dir.join(format!("{:010}.block.tmp", h));
            std::fs::write(&tmp_path, &data)?;
            std::fs::rename(&tmp_path, &block_path)?;

            write_manifest(dir, h, &hash)?;
            archived += 1;
        }

        if archived > 0 {
            info!(
                "[ARCHIVER] Catch-up complete: archived {} blocks ({} to {})",
                archived,
                last + 1,
                last + archived
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

        // Atomic write: tmp + rename
        let tmp_path = self.dir.join(format!("{:010}.block.tmp", block.height));
        std::fs::write(&tmp_path, &block.data)?;
        std::fs::rename(&tmp_path, &block_path)?;

        write_manifest(&self.dir, block.height, &block.hash)?;

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

fn read_manifest_height_from(dir: &Path) -> Option<u64> {
    let manifest_path = dir.join("manifest.json");
    let data = std::fs::read_to_string(&manifest_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("latest_height")?.as_u64()
}

fn write_manifest(dir: &Path, height: u64, hash: &crypto::Hash) -> Result<(), std::io::Error> {
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
pub fn restore_from_archive(
    archive_dir: &Path,
    block_store: &super::BlockStore,
) -> Result<u64, String> {
    let manifest_height = read_manifest_height_from(archive_dir)
        .ok_or_else(|| "No manifest.json found in archive directory".to_string())?;

    info!(
        "[ARCHIVER] Restoring from archive: {} blocks available",
        manifest_height
    );

    let mut imported = 0u64;
    for h in 1..=manifest_height {
        let block_path = archive_dir.join(format!("{:010}.block", h));
        let data = std::fs::read(&block_path)
            .map_err(|e| format!("Failed to read block file at height {}: {}", h, e))?;

        let block: doli_core::Block = bincode::deserialize(&data)
            .map_err(|e| format!("Failed to deserialize block at height {}: {}", h, e))?;

        let expected_hash_str = block.hash().to_string();
        block_store
            .put_block_canonical(&block, h)
            .map_err(|e| format!("Failed to store block {}: {}", h, e))?;

        imported += 1;
        if h.is_multiple_of(1000) {
            info!(
                "[ARCHIVER] Restored {}/{} blocks (hash={}...)",
                h,
                manifest_height,
                &expected_hash_str[..16]
            );
        }
    }

    info!("[ARCHIVER] Restore complete: {} blocks imported", imported);

    Ok(imported)
}
