use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use doli_core::Network;
use tracing::{info, warn};

use storage::{BlockStore, StateDb};

/// Report from the archive-to-checkpoint bridge.
#[allow(dead_code)]
pub struct BridgeReport {
    /// Number of blocks imported from archive.
    pub blocks_imported: u64,
    /// Whether there are remaining gaps in the block store.
    pub has_gaps: bool,
}

/// Bridge checkpoint restore to archive backfill.
///
/// After a RocksDB checkpoint is restored (manual `cp` or guardian RPC), the
/// state_db reflects height C but the block store may have gaps for heights
/// below C. The archive directory (flat files with BLAKE3 checksums) contains
/// the sequential block log. This function composes two existing primitives:
///
/// 1. Delete stale `chain_commitment` from state_db (unconditionally stale
///    after checkpoint restore -- it references the pre-restore tip, not C).
/// 2. Backfill missing blocks from the archive directory into the block store.
///
/// Errors from archive backfill produce warnings, not hard failures. The
/// function is safe to call when no archive exists (early return with info log).
///
/// Currently used by the `bridgeFromArchive` guardian RPC (composed inline in
/// `crates/rpc/src/methods/guardian.rs`). This standalone version is infrastructure
/// for Option G (startup auto-detection in init.rs).
#[allow(dead_code)]
pub fn bridge_checkpoint_to_archive(
    block_store: &Arc<BlockStore>,
    state_db: &Arc<StateDb>,
    archive_dir: &Path,
) -> Result<BridgeReport> {
    // FM-1: No archive directory -- early return with info log
    if !archive_dir.exists() {
        info!(
            "[BRIDGE] Archive directory does not exist ({:?}), skipping backfill",
            archive_dir
        );
        return Ok(BridgeReport {
            blocks_imported: 0,
            has_gaps: false,
        });
    }

    // Step 1: DELETE stale chain_commitment BEFORE backfill (BC3, FM-4).
    // The commitment references the pre-restore tip H, not checkpoint height C.
    // It is unconditionally stale after any checkpoint restore.
    state_db.delete_chain_commitment();
    info!("[BRIDGE] Deleted stale chain_commitment after checkpoint restore");

    // Step 2: BACKFILL from archive using existing idempotent function (BC5).
    // Read genesis_hash from the block store for cross-chain validation.
    let genesis_hash = find_genesis_hash(block_store);

    let imported = match storage::archiver::backfill_from_archive(
        archive_dir,
        block_store,
        genesis_hash.as_ref(),
    ) {
        Ok(count) => count,
        Err(e) => {
            // Archive errors are warnings, not hard failures (AC3).
            warn!("[BRIDGE] Archive backfill encountered an error: {}", e);
            0
        }
    };

    // Step 3: REPORT summary
    let has_gaps = imported == 0 && genesis_hash.is_none();
    if imported > 0 {
        info!(
            "[BRIDGE] Backfill complete: {} blocks imported from archive",
            imported
        );
    } else {
        info!("[BRIDGE] Backfill complete: no missing blocks found in archive");
    }

    Ok(BridgeReport {
        blocks_imported: imported,
        has_gaps,
    })
}

/// Find the genesis_hash from an existing block in the store.
/// Scans the first 10,000 heights to find any block with a genesis_hash.
#[allow(dead_code)]
fn find_genesis_hash(block_store: &BlockStore) -> Option<crypto::Hash> {
    for h in 1..=10_000 {
        if let Ok(Some(block)) = block_store.get_block_by_height(h) {
            return Some(block.header.genesis_hash);
        }
    }
    None
}

pub(crate) fn backfill_from_archive(
    network: Network,
    data_dir: &Path,
    archive_dir: &Path,
    skip_confirm: bool,
) -> Result<()> {
    use storage::BlockStore;

    println!("=== DOLI Backfill from Archive ===");
    println!();
    println!("Archive:   {:?}", archive_dir);
    println!("Data dir:  {:?}", data_dir);
    println!("Network:   {}", network.name());
    println!();
    println!("This will import ONLY missing blocks without rebuilding state.");
    println!();

    if !archive_dir.exists() {
        return Err(anyhow!(
            "Archive directory does not exist: {:?}",
            archive_dir
        ));
    }

    if !skip_confirm {
        println!("Press Ctrl+C to cancel, or wait 5 seconds to proceed...");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    let blocks_path = data_dir.join("blocks");
    std::fs::create_dir_all(&blocks_path)?;
    let block_store = BlockStore::open(&blocks_path)?;

    // Read genesis_hash from an existing block in the store (not from embedded chainspec,
    // which may differ from the actual chain if a custom chainspec was used at launch).
    let genesis_hash = {
        let mut found = None;
        for h in 1..=10000 {
            if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                found = Some(block.header.genesis_hash);
                break;
            }
        }
        found.unwrap_or_else(|| doli_core::genesis::genesis_hash(network))
    };

    let imported =
        storage::archiver::backfill_from_archive(archive_dir, &block_store, Some(&genesis_hash))
            .map_err(|e| anyhow!("{}", e))?;

    println!();
    if imported > 0 {
        println!("Backfill complete: {} missing blocks imported.", imported);
    } else {
        println!("Backfill complete: no missing blocks found.");
    }

    Ok(())
}

pub(crate) async fn restore_from_rpc(
    network: Network,
    data_dir: &Path,
    rpc_url: &str,
    backfill: bool,
    skip_confirm: bool,
    skip_genesis_check: bool,
) -> Result<()> {
    use base64::Engine;
    use storage::BlockStore;

    let mode = if backfill { "Backfill" } else { "Restore" };
    println!("=== DOLI {} from RPC ===", mode);
    println!();
    println!("RPC URL:   {}", rpc_url);
    println!("Data dir:  {:?}", data_dir);
    println!("Network:   {}", network.name());
    println!();

    let client = reqwest::Client::new();

    // Step 1: Get chain info from archiver
    let chain_info: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getChainInfo",
            "params": {},
            "id": 1
        }))
        .send()
        .await?
        .json()
        .await?;

    let result = chain_info
        .get("result")
        .ok_or_else(|| anyhow!("No result in getChainInfo response"))?;
    let remote_height = result
        .get("bestHeight")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("Missing bestHeight"))?;

    // Step 2: Validate genesis hash by fetching block 1 from archiver
    // (getChainInfo.genesisHash uses embedded chainspec which may differ from
    // the actual chain genesis on relaunched networks)
    let blocks_path = data_dir.join("blocks");
    std::fs::create_dir_all(&blocks_path)?;
    let block_store = BlockStore::open(&blocks_path)?;

    let block1_resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getBlockRaw",
            "params": { "height": 1 },
            "id": 0
        }))
        .send()
        .await?
        .json()
        .await?;

    let block1_result = block1_resp
        .get("result")
        .ok_or_else(|| anyhow!("Archiver has no block 1 — cannot validate genesis"))?;
    let b64_block1 = block1_result
        .get("block")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing block data for block 1"))?;
    let block1_data = base64::engine::general_purpose::STANDARD
        .decode(b64_block1)
        .map_err(|e| anyhow!("Base64 decode error for block 1: {}", e))?;
    let block1: doli_core::Block = bincode::deserialize(&block1_data)
        .map_err(|e| anyhow!("Deserialize error for block 1: {}", e))?;
    let remote_genesis = block1.header.genesis_hash;

    let local_genesis = {
        let mut found = None;
        for h in 1..=10000 {
            if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                found = Some(block.header.genesis_hash);
                break;
            }
        }
        found.unwrap_or_else(|| doli_core::genesis::genesis_hash(network))
    };

    if remote_genesis != local_genesis {
        if skip_genesis_check {
            println!(
                "WARNING: Genesis hash mismatch: remote={}, local={}",
                &remote_genesis.to_string()[..16],
                &local_genesis.to_string()[..16]
            );
            println!("         --skip-genesis-check: proceeding anyway (trusted source)");
        } else {
            return Err(anyhow!(
                "Genesis hash mismatch: remote={}, local={}. Wrong chain!\n\
                 If backfilling from a trusted seed on the same post-reset chain, \
                 use --skip-genesis-check",
                &remote_genesis.to_string()[..16],
                &local_genesis.to_string()[..16]
            ));
        }
    }

    let genesis_str = remote_genesis.to_string();
    println!("Archiver height: {}", remote_height);
    if remote_genesis == local_genesis {
        println!("Genesis hash:    {} (match)", &genesis_str[..16]);
    } else {
        println!(
            "Genesis hash:    {} (mismatch — skipped)",
            &genesis_str[..16]
        );
    }
    println!();

    if !skip_confirm {
        println!(
            "This will {} blocks 1..{} via RPC.",
            mode.to_lowercase(),
            remote_height
        );
        println!("Press Ctrl+C to cancel, or wait 5 seconds to proceed...");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    // Step 3: Download and store blocks (BlockStore already opened in Step 2)
    let mut imported = 0u64;
    let mut skipped = 0u64;

    for h in 1..=remote_height {
        // Skip existing blocks in backfill mode
        if backfill {
            if let Ok(Some(_)) = block_store.get_block_by_height(h) {
                skipped += 1;
                continue;
            }
        }

        // Request raw block
        let resp: serde_json::Value = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "getBlockRaw",
                "params": { "height": h },
                "id": h
            }))
            .send()
            .await?
            .json()
            .await?;

        let block_result = resp
            .get("result")
            .ok_or_else(|| anyhow!("No result for block {} (block may not exist)", h))?;
        let b64_data = block_result
            .get("block")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing block data at height {}", h))?;
        let expected_checksum = block_result
            .get("blake3")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing blake3 checksum at height {}", h))?;

        // Decode base64
        let data = base64::engine::general_purpose::STANDARD
            .decode(b64_data)
            .map_err(|e| anyhow!("Base64 decode error at height {}: {}", h, e))?;

        // Verify BLAKE3 checksum
        let actual_checksum = crypto::hash::hash(&data).to_string();
        if actual_checksum != expected_checksum {
            return Err(anyhow!(
                "BLAKE3 mismatch at height {}: expected={}, got={}",
                h,
                &expected_checksum[..16],
                &actual_checksum[..16]
            ));
        }

        // Deserialize and store
        let block: doli_core::Block = bincode::deserialize(&data)
            .map_err(|e| anyhow!("Deserialize error at {}: {}", h, e))?;

        block_store
            .put_block_canonical(&block, h)
            .map_err(|e| anyhow!("Store error at {}: {}", h, e))?;

        imported += 1;
        if imported.is_multiple_of(100) {
            println!(
                "[{}] {}/{} blocks (skipped {})",
                mode, h, remote_height, skipped
            );
        }
    }

    println!();
    println!(
        "{} complete: {} blocks imported, {} skipped.",
        mode, imported, skipped
    );

    if !backfill && imported > 0 {
        println!();
        println!("Run 'doli-node recover --yes' to rebuild UTXO/producer state.");
    }

    Ok(())
}

pub(crate) fn restore_from_archive(
    network: Network,
    data_dir: &Path,
    archive_dir: &Path,
    skip_confirm: bool,
) -> Result<()> {
    use storage::BlockStore;

    println!("=== DOLI Restore from Archive ===");
    println!();
    println!("Archive:   {:?}", archive_dir);
    println!("Data dir:  {:?}", data_dir);
    println!("Network:   {}", network.name());
    println!();

    if !archive_dir.exists() {
        return Err(anyhow!(
            "Archive directory does not exist: {:?}",
            archive_dir
        ));
    }

    if !skip_confirm {
        println!("This will import blocks from the archive and rebuild chain state.");
        println!("Press Ctrl+C to cancel, or wait 5 seconds to proceed...");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    // Open or create BlockStore
    let blocks_path = data_dir.join("blocks");
    std::fs::create_dir_all(&blocks_path)?;
    let block_store = BlockStore::open(&blocks_path)?;

    // Get genesis hash for validation
    let genesis_hash = doli_core::genesis::genesis_hash(network);

    // Import blocks with checksum + genesis_hash verification
    let imported =
        storage::archiver::restore_from_archive(archive_dir, &block_store, Some(&genesis_hash))
            .map_err(|e| anyhow!("{}", e))?;

    println!();
    println!("Imported {} blocks. Rebuilding chain state...", imported);
    println!();

    drop(block_store);

    // Automatically run recover to rebuild UTXO/producers/chain_state
    super::recover_chain_state(network, &data_dir.to_path_buf(), true)?;

    println!();
    println!("Restore complete! You can now start the node.");

    Ok(())
}
