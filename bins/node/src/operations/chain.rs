use std::path::PathBuf;

use anyhow::{anyhow, Result};
use doli_core::Network;
use tracing::warn;

use crate::cli::expand_tilde_path;

pub(crate) fn reindex_canonical_chain(data_dir: &PathBuf) -> Result<()> {
    use storage::BlockStore;

    println!("=== DOLI Canonical Chain Reindex ===");
    println!();
    println!("Data directory: {:?}", data_dir);

    let blocks_path = data_dir.join("blocks");
    if !blocks_path.exists() {
        return Err(anyhow!(
            "Blocks directory not found: {:?}. Nothing to reindex.",
            blocks_path
        ));
    }

    let block_store = BlockStore::open(&blocks_path)?;
    let (tip_hash, tip_height) = block_store.rebuild_canonical_index()?;

    println!();
    println!("=== Reindex Complete ===");
    println!("  Tip hash:   {}", tip_hash);
    println!("  Tip height: {}", tip_height);
    println!();
    println!("Run 'doli-node recover --yes' next to rebuild UTXO/producer state.");

    Ok(())
}

/// Recover chain state from existing block data
///
/// This function scans the BlockStore to find all blocks and rebuilds:
/// - chain_state.bin (chain tip: height, hash, slot)
/// - UTXO set (all unspent outputs)
/// - producers.bin (registered producers)
pub(crate) fn truncate_chain(
    _network: Network,
    data_dir: &std::path::Path,
    blocks_to_remove: u64,
    skip_confirm: bool,
) -> Result<()> {
    use storage::{BlockStore, ProducerSet, StateDb};

    let data_dir = expand_tilde_path(data_dir);

    println!("=== DOLI Chain Truncation ===");
    println!();
    println!("Data directory: {:?}", data_dir);
    println!("Blocks to remove: {}", blocks_to_remove);
    println!();

    let blocks_path = data_dir.join("blocks");
    let block_store = BlockStore::open(&blocks_path)?;

    let state_db_path = data_dir.join("state_db");
    let state_db = StateDb::open(&state_db_path)?;

    let mut chain_state = match state_db.get_chain_state() {
        Some(cs) => cs,
        None => {
            println!("No chain state found. Nothing to truncate.");
            return Ok(());
        }
    };

    let current_height = chain_state.best_height;

    if blocks_to_remove == 0 {
        println!("Nothing to truncate (--blocks 0).");
        return Ok(());
    }

    let new_tip = current_height.saturating_sub(blocks_to_remove);
    if new_tip == 0 {
        return Err(anyhow!(
            "Cannot truncate to height 0. Use 'recover' instead."
        ));
    }

    // Check undo data availability. INC-I-071: window reduced from 2000 to 360
    // (one epoch). Deeper rollbacks must use `recover` (replay from blocks).
    let oldest_undo = current_height.saturating_sub(360);
    if new_tip < oldest_undo {
        return Err(anyhow!(
            "Cannot truncate {} blocks — undo data only available for last 360 blocks (height {} to {}). \
             Max truncation: {} blocks. For deeper rollback, use 'recover'.",
            blocks_to_remove,
            oldest_undo,
            current_height,
            current_height - oldest_undo
        ));
    }

    println!("Current tip:  height {}", current_height);
    println!(
        "New tip:      height {} (removing {} blocks)",
        new_tip, blocks_to_remove
    );
    println!();

    if !skip_confirm {
        println!(
            "This will roll back state from height {} to {} using undo data,",
            current_height, new_tip
        );
        println!("then delete blocks above the new tip.");
        println!("Press Ctrl+C to cancel, or wait 5 seconds to proceed...");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    // Step 1: Roll back state using undo data (newest first)
    println!("Rolling back state using undo data...");
    let mut rolled_back = 0u64;
    for height in (new_tip + 1..=current_height).rev() {
        let undo = state_db.get_undo(height).ok_or_else(|| {
            anyhow!(
                "Missing undo data at height {} — cannot continue rollback. \
                 Use 'recover' for full state rebuild.",
                height
            )
        })?;

        // Remove UTXOs created by this block
        for outpoint in &undo.created_utxos {
            state_db.remove_utxo(outpoint);
        }

        // Restore UTXOs spent by this block
        for (outpoint, entry) in &undo.spent_utxos {
            state_db.insert_utxo(outpoint, entry);
        }

        rolled_back += 1;
        if rolled_back.is_multiple_of(100) {
            println!("  rolled back {} blocks...", rolled_back);
        }
    }

    // Restore producer set from the undo data at new_tip + 1
    // (contains the snapshot BEFORE that block was applied = state AT new_tip).
    //
    // INC-I-071: empty producer_snapshot is the sentinel meaning "unchanged at
    // this height". Scan forward from new_tip+1 through the entries being
    // rolled back, locating the first non-empty entry — its BEFORE-state is
    // in the same producer-state era as new_tip's, so it is the correct
    // snapshot to restore. If every entry in the range is empty, producers
    // were unchanged across the whole rollback range and the on-disk
    // ProducerSet is already correct for new_tip.
    let mut producer_snapshot_bytes: Option<Vec<u8>> = None;
    for h in (new_tip + 1)..=current_height {
        if let Some(undo_h) = state_db.get_undo(h) {
            if !undo_h.producer_snapshot.is_empty() {
                producer_snapshot_bytes = Some(undo_h.producer_snapshot);
                break;
            }
        }
    }

    if let Some(bytes) = producer_snapshot_bytes {
        if let Ok(restored_ps) = bincode::deserialize::<ProducerSet>(&bytes) {
            state_db.write_producer_set(&restored_ps)?;
            println!("Producer set restored from undo snapshot.");
        } else {
            println!("WARNING: Could not deserialize producer snapshot. Run 'recover' after startup if producers are wrong.");
        }
    } else {
        println!(
            "Producer set unchanged across rollback range (all sentinel entries); \
             on-disk producer set is already correct for new tip."
        );
    }

    // Update chain state to new tip
    let new_tip_block = block_store.get_block_by_height(new_tip)?.ok_or_else(|| {
        anyhow!(
            "Block at new tip height {} not found in block store",
            new_tip
        )
    })?;

    chain_state.best_height = new_tip;
    chain_state.best_hash = new_tip_block.hash();
    chain_state.best_slot = new_tip_block.header.slot;
    state_db.put_chain_state(&chain_state)?;

    println!(
        "State rolled back: {} blocks (height {} → {})",
        rolled_back, current_height, new_tip
    );

    // Step 2: Delete blocks above new_tip from block store
    println!(
        "Deleting blocks above height {} from block store...",
        new_tip
    );
    let deleted = block_store.delete_blocks_above(new_tip)?;
    println!("Deleted {} blocks from block store.", deleted);

    // Step 3: Clean up undo data above new_tip
    state_db.prune_undo_above(new_tip);

    println!();
    println!("Truncation complete. Node is at height {}.", new_tip);
    println!("Start the node normally — it will sync forward from peers.");

    Ok(())
}

/// Recover chain state by replaying blocks through the canonical `apply_block()` path.
///
/// This uses `ValidationMode::Replay` to feed every block in the store through the
/// same state transition function used during normal operation. This guarantees:
/// - ALL transaction types are processed (Registration, AddBond, DelegateBond, etc.)
/// - Epoch state is correctly derived (bond_snapshot, producer_list, attestation)
/// - Undo data is produced (enables rollback after recovery)
/// - State is written to state_db atomically (not legacy files)
/// - Genesis hash comes from chainspec (not a hardcoded literal)
///
/// Side effects suppressed by headless Node (C8):
/// - network = None → no gossip possible
/// - mempool starts empty → no tx pollution
/// - No peer notifications, no VDF computation
pub(crate) fn recover_chain_state(
    network: Network,
    data_dir: &PathBuf,
    skip_confirm: bool,
) -> Result<()> {
    use storage::BlockStore;

    println!("=== DOLI Chain State Recovery (apply_block replay) ===");
    println!();
    println!("Data directory: {:?}", data_dir);
    println!("Network: {}", network.name());
    println!();

    if !data_dir.exists() {
        return Err(anyhow!("Data directory does not exist: {:?}", data_dir));
    }

    let blocks_path = data_dir.join("blocks");
    if !blocks_path.exists() {
        return Err(anyhow!(
            "Blocks directory not found: {:?}. Nothing to recover.",
            blocks_path
        ));
    }

    // Open BlockStore and scan for chain tip
    println!("Opening BlockStore...");
    let block_store = BlockStore::open(&blocks_path)?;

    println!("Scanning for blocks...");
    let mut tip_height = 0u64;
    let mut block_count = 0u64;

    for height in 1..=u64::MAX {
        match block_store.get_block_by_height(height) {
            Ok(Some(_)) => {
                tip_height = height;
                block_count += 1;
                if block_count.is_multiple_of(1000) {
                    print!("\r  Scanned {} blocks (height {})...", block_count, height);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }
            Ok(None) => break,
            Err(e) => {
                warn!("Error reading block at height {}: {}", height, e);
                break;
            }
        }
    }
    println!();

    if block_count == 0 {
        return Err(anyhow!(
            "No blocks found in BlockStore. Nothing to recover."
        ));
    }

    println!();
    println!("Found {} blocks (tip height: {})", block_count, tip_height);
    println!();

    if !skip_confirm {
        print!(
            "Proceed with recovery? This will wipe state_db and rebuild from {} blocks. [y/N] ",
            block_count
        );
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Recovery cancelled.");
            return Ok(());
        }
    }

    // Step 1: Rebuild canonical chain index
    println!();
    println!("Rebuilding canonical chain index from headers...");
    let (reindex_tip, reindex_height) = block_store.rebuild_canonical_index()?;
    println!(
        "  Canonical chain: {} blocks, tip={}",
        reindex_height + 1,
        &reindex_tip.to_string()[..16]
    );

    // Step 2: Wipe and recreate state_db (fresh start)
    let state_db_path = data_dir.join("state_db");
    if state_db_path.exists() {
        println!("Wiping existing state_db...");
        std::fs::remove_dir_all(&state_db_path)?;
    }

    // Step 3: Replay blocks via headless Node + apply_block(Replay)
    //
    // Construct a headless Node with the existing block_store and fresh state_db.
    // This reuses the exact same state transition code used during normal operation,
    // guaranteeing identical results for ALL transaction types, epoch boundaries,
    // and undo data generation. No shadow implementation.
    println!();
    println!("Replaying blocks through canonical apply_block()...");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        use crate::node::Node;
        use doli_core::validation::ValidationMode;

        // Construct headless Node for replay.
        // Uses new_for_test infrastructure (proven) with the correct network chainspec.
        let mut node = Node::new_for_replay(data_dir.clone(), network)
            .await
            .map_err(|e| anyhow!("Failed to construct replay node: {}", e))?;

        for height in 1..=tip_height {
            let block = node
                .block_store
                .get_block_by_height(height)?
                .ok_or_else(|| anyhow!("Block at height {} disappeared during recovery", height))?;

            node.apply_block(block, ValidationMode::Replay)
                .await
                .map_err(|e| anyhow!("apply_block failed at height {}: {}", height, e))?;

            if height % 500 == 0 || height == tip_height {
                let pct = (height as f64 / tip_height as f64) * 100.0;
                print!(
                    "\r  Replayed {}/{} blocks ({:.1}%)...",
                    height, tip_height, pct
                );
                std::io::Write::flush(&mut std::io::stdout()).ok();
            }
        }
        println!();

        let cs = node.chain_state.read().await;
        let utxo_count = node.utxo_set.read().await.len();
        let producer_count = node.producer_set.read().await.active_count();

        println!();
        println!("=== Recovery Complete ===");
        println!();
        println!("Chain state recovered (written to state_db):");
        println!("  Height:    {}", cs.best_height);
        println!("  Hash:      {}", cs.best_hash);
        println!("  Slot:      {}", cs.best_slot);
        println!("  UTXOs:     {}", utxo_count);
        println!("  Producers: {}", producer_count);
        println!();
        println!("You can now start the node normally.");

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
