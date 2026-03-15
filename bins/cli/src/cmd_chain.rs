use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::commands::RewardsCommands;
use crate::rpc_client::{format_balance, RpcClient};

pub(crate) async fn cmd_chain(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    println!("Chain Information");
    println!("{:-<60}", "");

    match rpc.get_chain_info().await {
        Ok(info) => {
            println!("Network:      {}", info.network);
            println!("Best Height:  {}", info.best_height);
            println!("Best Slot:    {}", info.best_slot);
            println!("Best Hash:    {}", info.best_hash);
            println!("Genesis Hash: {}", info.genesis_hash);
        }
        Err(e) => {
            anyhow::bail!("Cannot connect to node at {}. Details: {}. Make sure a DOLI node is running and accessible.", rpc_endpoint, e);
        }
    }

    Ok(())
}

pub(crate) async fn cmd_chain_verify(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    println!("Chain Integrity Verification");
    println!("{:-<60}", "");
    println!("Scanning all blocks from genesis to tip...\n");

    match rpc.verify_chain_integrity().await {
        Ok(result) => {
            println!("Tip Height:       {}", result.tip);
            println!("Blocks Scanned:   {}", result.scanned);
            println!(
                "Complete:         {}",
                if result.complete { "YES" } else { "NO" }
            );
            println!("Missing Blocks:   {}", result.missing_count);
            if !result.missing.is_empty() {
                println!("Missing Ranges:   {}", result.missing.join(", "));
            }
            println!();
            if let Some(commitment) = result.chain_commitment {
                println!("Chain Commitment: {}", commitment);
                println!();
                println!("This 32-byte BLAKE3 fingerprint uniquely identifies the exact");
                println!(
                    "sequence of all blocks 1..{}. Two nodes with the same",
                    result.tip
                );
                println!("commitment have identical chains.");
            } else {
                println!("Chain Commitment: UNAVAILABLE (chain is incomplete)");
                println!();
                println!("Run 'backfillFromPeer' to fill gaps, then verify again.");
            }
        }
        Err(e) => {
            anyhow::bail!("Cannot verify chain at {}. Details: {}", rpc_endpoint, e);
        }
    }

    Ok(())
}

pub(crate) async fn cmd_rewards(
    _wallet_path: &Path,
    rpc_endpoint: &str,
    command: RewardsCommands,
) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        RewardsCommands::List => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
            println!("Use 'doli rewards info' for current epoch details.");
        }

        RewardsCommands::Claim {
            epoch: _,
            recipient: _,
        } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
        }

        RewardsCommands::ClaimAll { recipient: _ } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
        }

        RewardsCommands::History { limit: _ } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claim history — use 'doli history' to see received rewards.");
        }

        RewardsCommands::Info => {
            println!("Reward Epoch Information");
            println!("{:-<60}", "");
            println!();

            match rpc.get_epoch_info().await {
                Ok(info) => {
                    println!("Current Height:      {}", info.current_height);
                    println!("Current Epoch:       {}", info.current_epoch);
                    println!(
                        "Last Complete Epoch: {}",
                        info.last_complete_epoch
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "None".to_string())
                    );
                    println!();
                    println!("Blocks per Epoch:    {}", info.blocks_per_epoch);
                    println!("Blocks Remaining:    {}", info.blocks_remaining);
                    println!();
                    println!(
                        "Epoch {} Range:     {} - {} (exclusive)",
                        info.current_epoch, info.epoch_start_height, info.epoch_end_height
                    );
                    println!("Block Reward:        {}", format_balance(info.block_reward));
                    println!();
                    println!(
                        "Progress: [{}{}] {}%",
                        "=".repeat(
                            ((info.blocks_per_epoch - info.blocks_remaining) * 30
                                / info.blocks_per_epoch) as usize
                        ),
                        " ".repeat((info.blocks_remaining * 30 / info.blocks_per_epoch) as usize),
                        ((info.blocks_per_epoch - info.blocks_remaining) * 100
                            / info.blocks_per_epoch)
                    );
                }
                Err(e) => {
                    anyhow::bail!("Error fetching epoch info: {}", e);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn cmd_wipe(network: &str, data_dir: Option<PathBuf>, yes: bool) -> Result<()> {
    println!("Wipe Chain Data");
    println!("{:-<60}", "");
    println!();

    // 1. Resolve data dir
    let data_dir = match data_dir {
        Some(d) => d,
        None => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            home.join(".doli").join(network)
        }
    };

    // 2. Verify directory exists
    if !data_dir.exists() {
        anyhow::bail!("Data directory does not exist: {:?}", data_dir);
    }

    println!("Network:   {}", network);
    println!("Data dir:  {:?}", data_dir);
    println!();

    // 3. Safety check: is doli-node running with this data dir?
    let data_dir_str = data_dir.to_string_lossy().to_string();
    let is_running = std::process::Command::new("pgrep")
        .args(["-f", &format!("doli-node.*{}", data_dir_str)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_running {
        anyhow::bail!(
            "A doli-node process is running with this data directory.\n\
             Stop the node first: sudo systemctl stop <service>"
        );
    }

    // 4. Collect everything EXCEPT preserved items
    //    Inverted logic: delete all, preserve only what's listed.
    //    This ensures old/legacy files from any version are cleaned up.
    let preserve_names: &[&str] = &["keys", ".env"];

    fn collect_deletable(dir: &Path, preserve: &[&str], out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !preserve.contains(&name.as_str()) {
                    out.push(entry.path());
                }
            }
        }
    }

    let mut found_items: Vec<PathBuf> = Vec::new();

    // Scan data_dir itself
    collect_deletable(&data_dir, preserve_names, &mut found_items);

    // Scan subdirectories (node1/, node2/, etc.) — also clean inside them
    let top_level: Vec<PathBuf> = found_items.clone();
    for path in &top_level {
        if path.is_dir() {
            // Scan data/ subdirectory inside node dirs
            let data_subdir = path.join("data");
            if data_subdir.is_dir() {
                collect_deletable(&data_subdir, preserve_names, &mut found_items);
            }
        }
    }

    if found_items.is_empty() {
        println!("Nothing to wipe — data directory is already clean.");
        return Ok(());
    }

    println!("Will DELETE:");
    for item in &found_items {
        let suffix = if item.is_dir() { "/" } else { "" };
        println!(
            "  - {}{}",
            item.strip_prefix(&data_dir).unwrap_or(item).display(),
            suffix
        );
    }
    println!();
    println!("Will PRESERVE:");
    for name in preserve_names {
        let path = data_dir.join(name);
        if path.exists() {
            println!("  - {}/", name);
        }
    }
    println!();

    // 5. Confirm
    if !yes {
        println!("This will delete all chain data. The node will resync from peers.");
        println!("Run with --yes to proceed.");
        return Ok(());
    }

    // 6. Delete
    let mut deleted = 0;
    for item in &found_items {
        let result = if item.is_dir() {
            std::fs::remove_dir_all(item)
        } else {
            std::fs::remove_file(item)
        };
        match result {
            Ok(()) => {
                deleted += 1;
            }
            Err(e) => {
                eprintln!("Warning: failed to remove {:?}: {}", item, e);
            }
        }
    }

    // 7. Summary
    println!();
    println!("Wiped {} items. Data directory is clean.", deleted);
    println!("Start the node to resync from peers.");

    Ok(())
}
