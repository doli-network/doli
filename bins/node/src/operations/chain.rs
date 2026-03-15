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

    // Check undo data availability
    let oldest_undo = current_height.saturating_sub(2000);
    if new_tip < oldest_undo {
        return Err(anyhow!(
            "Cannot truncate {} blocks — undo data only available for last 2000 blocks (height {} to {}). \
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
    // (contains the snapshot BEFORE that block was applied = state AT new_tip)
    if let Some(undo) = state_db.get_undo(new_tip + 1) {
        if let Ok(restored_ps) = bincode::deserialize::<ProducerSet>(&undo.producer_snapshot) {
            state_db.write_producer_set(&restored_ps)?;
            println!("Producer set restored from undo snapshot.");
        } else {
            println!("WARNING: Could not deserialize producer snapshot. Run 'recover' after startup if producers are wrong.");
        }
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

pub(crate) fn recover_chain_state(
    network: Network,
    data_dir: &PathBuf,
    skip_confirm: bool,
) -> Result<()> {
    use doli_core::consensus::{ConsensusParams, UNBONDING_PERIOD};
    use doli_core::transaction::TxType;
    use storage::{BlockStore, ChainState, ProducerInfo, ProducerSet, UtxoSet};

    println!("=== DOLI Chain State Recovery ===");
    println!();
    println!("Data directory: {:?}", data_dir);
    println!("Network: {}", network.name());
    println!();

    // Check if data directory exists
    if !data_dir.exists() {
        return Err(anyhow!("Data directory does not exist: {:?}", data_dir));
    }

    // Check if blocks directory exists
    let blocks_path = data_dir.join("blocks");
    if !blocks_path.exists() {
        return Err(anyhow!(
            "Blocks directory not found: {:?}. Nothing to recover.",
            blocks_path
        ));
    }

    // Open BlockStore
    println!("Opening BlockStore...");
    let block_store = BlockStore::open(&blocks_path)?;

    // Scan to find chain tip (highest block)
    println!("Scanning for blocks...");
    let mut tip_height = 0u64;
    let mut tip_hash = crypto::hash::hash(b"DOLI Genesis");
    let mut tip_slot = 0u32;
    let mut block_count = 0u64;

    // Scan heights starting from 1
    for height in 1..=u64::MAX {
        match block_store.get_block_by_height(height) {
            Ok(Some(block)) => {
                tip_height = height;
                tip_hash = block.hash();
                tip_slot = block.header.slot;
                block_count += 1;

                if block_count.is_multiple_of(1000) {
                    print!("\r  Scanned {} blocks (height {})...", block_count, height);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }
            Ok(None) => {
                // No more blocks at this height
                break;
            }
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
    println!("Found {} blocks:", block_count);
    println!("  Chain tip height: {}", tip_height);
    println!("  Chain tip hash:   {}", tip_hash);
    println!("  Chain tip slot:   {}", tip_slot);
    println!();

    // Check existing state files
    let state_path = data_dir.join("chain_state.bin");
    let utxo_path = data_dir.join("utxo");
    let producers_path = data_dir.join("producers.bin");

    let state_exists = state_path.exists();
    let utxo_exists = utxo_path.exists();
    let producers_exists = producers_path.exists();

    if state_exists || utxo_exists || producers_exists {
        println!("Existing state files:");
        if state_exists {
            println!("  - chain_state.bin (will be overwritten)");
        }
        if utxo_exists {
            println!("  - utxo/ (will be overwritten)");
        }
        if producers_exists {
            println!("  - producers.bin (will be overwritten)");
        }
        println!();
    }

    // Confirm with user
    if !skip_confirm {
        print!(
            "Proceed with recovery? This will rebuild state from {} blocks. [y/N] ",
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

    // Step 1: Rebuild canonical chain index from headers (fixes corrupt height_index)
    println!();
    println!("Rebuilding canonical chain index from headers...");
    let (reindex_tip, reindex_height) = block_store.rebuild_canonical_index()?;
    println!(
        "  Canonical chain: {} blocks, tip={}",
        reindex_height + 1,
        &reindex_tip.to_string()[..16]
    );

    // Step 2: Replay blocks using the now-correct height_index
    println!();
    println!("Rebuilding state from blocks...");

    // Initialize fresh state
    let genesis_hash = crypto::hash::hash(b"DOLI Genesis");
    let mut chain_state = ChainState::new(genesis_hash);
    let mut utxo_set = UtxoSet::new();
    let mut producer_set = ProducerSet::new();
    let params = ConsensusParams::for_network(network);

    // Replay all blocks
    for height in 1..=tip_height {
        let block = block_store
            .get_block_by_height(height)?
            .ok_or_else(|| anyhow!("Block at height {} disappeared during recovery", height))?;

        // Apply transactions to UTXO set
        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let is_coinbase = tx_index == 0 && tx.is_coinbase();
            let _ = utxo_set.spend_transaction(tx); // Ignore errors for coinbase
            utxo_set.add_transaction(tx, height, is_coinbase, block.header.slot)?;

            // Process registration transactions
            if tx.tx_type == TxType::Registration {
                if let Some(reg_data) = tx.registration_data() {
                    if let Some((bond_index, bond_output)) =
                        tx.outputs.iter().enumerate().find(|(_, o)| {
                            o.output_type == doli_core::transaction::OutputType::Bond
                        })
                    {
                        let tx_hash = tx.hash();
                        let era = params.height_to_era(height);

                        let producer_info = ProducerInfo::new_with_bonds(
                            reg_data.public_key,
                            height,
                            bond_output.amount,
                            (tx_hash, bond_index as u32),
                            era,
                            reg_data.bond_count,
                        );

                        if let Err(e) = producer_set.register(producer_info, height) {
                            warn!("Failed to register producer at height {}: {}", height, e);
                        }
                    }
                }
            }

            // Process exit transactions
            if tx.tx_type == TxType::Exit {
                if let Some(exit_data) = tx.exit_data() {
                    if let Err(e) = producer_set.request_exit(&exit_data.public_key, height) {
                        warn!("Failed to process exit at height {}: {}", height, e);
                    }
                }
            }

            // Process slash transactions
            if tx.tx_type == TxType::SlashProducer {
                if let Some(slash_data) = tx.slash_data() {
                    if let Err(e) = producer_set.slash_producer(&slash_data.producer_pubkey, height)
                    {
                        warn!("Failed to slash producer at height {}: {}", height, e);
                    }
                }
            }
        }

        // Process completed unbonding periods
        let _ = producer_set.process_unbonding(height, UNBONDING_PERIOD);

        // Update chain state
        chain_state.update(block.hash(), height, block.header.slot);

        // Set genesis timestamp from first block if needed
        if chain_state.genesis_timestamp == 0 && height == 1 {
            let block_timestamp = block.header.timestamp;
            chain_state.genesis_timestamp =
                block_timestamp - (block_timestamp % params.slot_duration);
        }

        // Progress indicator
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

    // Save recovered state
    println!();
    println!("Saving recovered state...");

    chain_state.save(&state_path)?;
    println!(
        "  Saved chain_state.bin (height={}, slot={})",
        chain_state.best_height, chain_state.best_slot
    );

    utxo_set.save(&utxo_path)?;
    println!("  Saved utxo/ ({} UTXOs)", utxo_set.len());

    // Merge genesis producers from hardcoded genesis constants.
    // Genesis producers are registered out-of-band (not via on-chain transactions),
    // so block replay alone cannot reconstruct them. We load them from the
    // hardcoded MAINNET_GENESIS_PRODUCERS and add any missing from the recovered set.
    let genesis_merged = match network {
        Network::Mainnet => {
            let genesis_producers = doli_core::genesis::mainnet_genesis_producers();
            let mut added = 0usize;
            for (pk, bonds) in &genesis_producers {
                if producer_set.get_by_pubkey(pk).is_none() {
                    let _ =
                        producer_set.register_genesis_producer(*pk, *bonds, network.bond_unit());
                    added += 1;
                }
            }
            added
        }
        _ => 0,
    };

    producer_set.save(&producers_path)?;
    println!(
        "  Saved producers.bin ({} producers, {} genesis merged)",
        producer_set.active_count(),
        genesis_merged
    );

    println!();
    println!("=== Recovery Complete ===");
    println!();
    println!("Chain state recovered:");
    println!("  Height: {}", chain_state.best_height);
    println!("  Hash:   {}", chain_state.best_hash);
    println!("  Slot:   {}", chain_state.best_slot);
    println!();
    println!("You can now start the node normally.");

    Ok(())
}
