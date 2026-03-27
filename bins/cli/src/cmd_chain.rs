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
            println!("Reward Pool:  {}", format_balance(info.reward_pool_balance));
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

    // 3. Safety: stop the service if running (Restart=always would revive it)
    let service_name = format!("doli-{}", network);
    let service_active = std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", &service_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if service_active {
        println!("Stopping {} service...", service_name);
        let _ = std::process::Command::new("systemctl")
            .args(["stop", &service_name])
            .status();
        // Fallback with sudo if unprivileged
        let still_active = std::process::Command::new("systemctl")
            .args(["is-active", "--quiet", &service_name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if still_active {
            let _ = std::process::Command::new("sudo")
                .args(["systemctl", "stop", &service_name])
                .status();
        }
        // Brief pause for process cleanup
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Also check for any doli-node process using this data dir
    let data_dir_str = data_dir.to_string_lossy().to_string();
    let is_running = std::process::Command::new("pgrep")
        .args(["-f", &format!("doli-node.*{}", data_dir_str)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_running {
        anyhow::bail!(
            "A doli-node process is still running with this data directory.\n\
             Stop it manually: sudo systemctl stop {} && sudo kill $(pgrep -f doli-node)",
            service_name
        );
    }

    // 4. Collect, display, confirm, delete, verify
    let result = wipe_data_dir(&data_dir, yes)?;
    match result {
        WipeResult::AlreadyClean => {
            println!("Nothing to wipe — data directory is already clean.");
        }
        WipeResult::DryRun => {
            println!("This will delete all chain data. The node will resync from peers.");
            println!("Run with --yes to proceed.");
        }
        WipeResult::Wiped { deleted, remaining } => {
            println!();
            println!("Wiped {} items.", deleted);
            if remaining.is_empty() {
                println!("Data directory is clean.");
            } else {
                println!("Warning: {} items remain after wipe:", remaining.len());
                for item in &remaining {
                    let suffix = if item.is_dir() { "/" } else { "" };
                    println!(
                        "  - {}{}",
                        item.strip_prefix(&data_dir).unwrap_or(item).display(),
                        suffix
                    );
                }
            }
            println!("Start the node to resync from peers.");
        }
    }

    Ok(())
}

/// Files/directories preserved during wipe (everything else is deleted).
/// CRITICAL: wallet.json contains private keys. Deleting it loses funds permanently.
/// This list MUST match the preserve list in cmd_snap.rs.
const WIPE_PRESERVE: &[&str] = &[
    "keys",
    ".env",
    "wallet.json",
    "wallet.seed.txt",
    "node_key",
    "config.toml",
];

/// Result of a wipe operation.
#[derive(Debug)]
enum WipeResult {
    AlreadyClean,
    DryRun,
    Wiped {
        deleted: usize,
        remaining: Vec<PathBuf>,
    },
}

/// Collect all entries in `dir` that are NOT in the preserve list.
fn collect_deletable(dir: &Path, preserve: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !preserve.contains(&name.as_str()) {
                out.push(entry.path());
            }
        }
    }
    out
}

/// Core wipe logic: scan, optionally delete, verify.
/// Extracted for testability.
fn wipe_data_dir(data_dir: &Path, execute: bool) -> Result<WipeResult> {
    let mut found_items = collect_deletable(data_dir, WIPE_PRESERVE);

    // Also scan subdirectories (node1/, node2/) for multi-node setups
    let top_level: Vec<PathBuf> = found_items.clone();
    for path in &top_level {
        if path.is_dir() {
            let data_subdir = path.join("data");
            if data_subdir.is_dir() {
                found_items.extend(collect_deletable(&data_subdir, WIPE_PRESERVE));
            }
        }
    }

    if found_items.is_empty() {
        return Ok(WipeResult::AlreadyClean);
    }

    // Display what will be deleted/preserved
    println!("Will DELETE:");
    for item in &found_items {
        let suffix = if item.is_dir() { "/" } else { "" };
        println!(
            "  - {}{}",
            item.strip_prefix(data_dir).unwrap_or(item).display(),
            suffix
        );
    }
    println!();
    println!("Will PRESERVE:");
    for name in WIPE_PRESERVE {
        let path = data_dir.join(name);
        if path.exists() {
            println!("  - {}/", name);
        }
    }
    println!();

    if !execute {
        return Ok(WipeResult::DryRun);
    }

    // Delete
    let mut deleted = 0;
    for item in &found_items {
        let result = if item.is_dir() {
            std::fs::remove_dir_all(item)
        } else {
            std::fs::remove_file(item)
        };
        match result {
            Ok(()) => deleted += 1,
            Err(e) => eprintln!("Warning: failed to remove {:?}: {}", item, e),
        }
    }

    // Re-scan to verify
    let remaining = collect_deletable(data_dir, WIPE_PRESERVE);

    Ok(WipeResult::Wiped { deleted, remaining })
}

#[cfg(test)]
mod wipe_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create the exact file layout from the bug report (#9)
    fn create_mainnet_layout(dir: &Path) {
        fs::create_dir_all(dir.join("blocks")).unwrap();
        fs::create_dir_all(dir.join("state_db")).unwrap();
        fs::create_dir_all(dir.join("signed_slots.db")).unwrap();
        fs::write(dir.join("producer_gset.bin"), b"data").unwrap();
        fs::write(dir.join("peers.cache"), b"data").unwrap();
        fs::write(dir.join("node_key"), b"data").unwrap();
        fs::write(dir.join("maintainer_state.bin"), b"data").unwrap();
        fs::write(dir.join("producer.lock"), b"12345").unwrap();
        // Preserved items
        fs::create_dir_all(dir.join("keys")).unwrap();
        fs::write(dir.join("keys").join("wallet.json"), b"secret").unwrap();
        fs::write(dir.join(".env"), b"NETWORK=mainnet").unwrap();
    }

    /// Test 1: collect_deletable finds ALL non-preserved items
    /// This is the exact bug — maintainer_state.bin and producer.lock
    /// must appear in the deletable list.
    #[test]
    fn test_collect_deletable_finds_all_chain_files() {
        let tmp = TempDir::new().unwrap();
        create_mainnet_layout(tmp.path());

        let deletable = collect_deletable(tmp.path(), WIPE_PRESERVE);
        let names: Vec<String> = deletable
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        // The two files from bug #9 MUST be in the list
        assert!(names.contains(&"maintainer_state.bin".to_string()));
        assert!(names.contains(&"producer.lock".to_string()));

        // Standard chain data must also be present
        assert!(names.contains(&"blocks".to_string()));
        assert!(names.contains(&"state_db".to_string()));
        assert!(names.contains(&"signed_slots.db".to_string()));
        assert!(names.contains(&"producer_gset.bin".to_string()));
        assert!(names.contains(&"peers.cache".to_string()));

        // Total: 8 items to delete
        assert_eq!(deletable.len(), 7);
    }

    /// Test 2: keys/ and .env are NEVER in the deletable list
    #[test]
    fn test_collect_deletable_preserves_keys_and_env() {
        let tmp = TempDir::new().unwrap();
        create_mainnet_layout(tmp.path());

        let deletable = collect_deletable(tmp.path(), WIPE_PRESERVE);
        let names: Vec<String> = deletable
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(!names.contains(&"keys".to_string()));
        assert!(!names.contains(&".env".to_string()));
    }

    /// Test 3: wipe actually deletes everything and reports clean
    #[test]
    fn test_wipe_deletes_all_and_reports_clean() {
        let tmp = TempDir::new().unwrap();
        create_mainnet_layout(tmp.path());

        let result = wipe_data_dir(tmp.path(), true).unwrap();

        match result {
            WipeResult::Wiped { deleted, remaining } => {
                assert_eq!(deleted, 7);
                assert!(remaining.is_empty(), "remaining: {:?}", remaining);
            }
            other => panic!("Expected Wiped, got {:?}", other),
        }

        // Verify preserved items still exist
        assert!(tmp.path().join("keys").exists());
        assert!(tmp.path().join("keys").join("wallet.json").exists());
        assert!(tmp.path().join(".env").exists());

        // Verify deleted items are gone
        assert!(!tmp.path().join("blocks").exists());
        assert!(!tmp.path().join("state_db").exists());
        assert!(!tmp.path().join("maintainer_state.bin").exists());
        assert!(!tmp.path().join("producer.lock").exists());
    }

    /// Test 4: dry run (execute=false) does NOT delete anything
    #[test]
    fn test_wipe_dry_run_preserves_everything() {
        let tmp = TempDir::new().unwrap();
        create_mainnet_layout(tmp.path());

        let result = wipe_data_dir(tmp.path(), false).unwrap();
        assert!(matches!(result, WipeResult::DryRun));

        // Everything must still exist
        assert!(tmp.path().join("blocks").exists());
        assert!(tmp.path().join("maintainer_state.bin").exists());
        assert!(tmp.path().join("producer.lock").exists());
        assert!(tmp.path().join("keys").exists());
    }

    /// Test 5: empty directory reports AlreadyClean
    #[test]
    fn test_wipe_empty_dir_is_already_clean() {
        let tmp = TempDir::new().unwrap();
        // Only preserved items
        fs::create_dir_all(tmp.path().join("keys")).unwrap();
        fs::write(tmp.path().join(".env"), b"x").unwrap();

        let result = wipe_data_dir(tmp.path(), true).unwrap();
        assert!(matches!(result, WipeResult::AlreadyClean));
    }

    /// Test 6: truly empty directory is AlreadyClean
    #[test]
    fn test_wipe_truly_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let result = wipe_data_dir(tmp.path(), true).unwrap();
        assert!(matches!(result, WipeResult::AlreadyClean));
    }

    /// Test 7: unknown/future files are also deleted (inverted logic)
    /// If the node adds new files in v5.0, wipe must still clean them.
    #[test]
    fn test_wipe_deletes_unknown_future_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("some_future_cache.db"), b"data").unwrap();
        fs::write(tmp.path().join("new_state_v5.bin"), b"data").unwrap();
        fs::create_dir_all(tmp.path().join("snapshots")).unwrap();

        let result = wipe_data_dir(tmp.path(), true).unwrap();

        match result {
            WipeResult::Wiped { deleted, remaining } => {
                assert_eq!(deleted, 3);
                assert!(remaining.is_empty());
            }
            other => panic!("Expected Wiped, got {:?}", other),
        }
    }

    /// Test 8: re-scan detects files that reappear after deletion
    /// Simulates the exact bug #9 scenario: a file is recreated between
    /// delete and verify. The remaining list must report it.
    #[test]
    fn test_rescan_detects_recreated_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("producer.lock"), b"pid").unwrap();

        // Wipe it
        let result = wipe_data_dir(tmp.path(), true).unwrap();
        match &result {
            WipeResult::Wiped { deleted, remaining } => {
                assert_eq!(*deleted, 1);
                assert!(remaining.is_empty());
            }
            other => panic!("Expected Wiped, got {:?}", other),
        }

        // Simulate node restart recreating the file
        fs::write(tmp.path().join("producer.lock"), b"new_pid").unwrap();

        // A second scan must detect the recreated file
        let leftover = collect_deletable(tmp.path(), WIPE_PRESERVE);
        assert_eq!(leftover.len(), 1);
        assert!(leftover[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("producer.lock"));
    }

    /// Test 9: wipe with only preserved items and chain data
    /// Exact reproduction of bug report filesystem layout
    #[test]
    fn test_bug_9_exact_reproduction() {
        let tmp = TempDir::new().unwrap();

        // Exact layout from the bug report
        fs::create_dir_all(tmp.path().join("blocks")).unwrap();
        fs::create_dir_all(tmp.path().join("state_db")).unwrap();
        fs::create_dir_all(tmp.path().join("signed_slots.db")).unwrap();
        fs::write(tmp.path().join("producer_gset.bin"), b"x").unwrap();
        fs::write(tmp.path().join("peers.cache"), b"x").unwrap();
        // These two were the bug — they survived the wipe
        fs::write(tmp.path().join("maintainer_state.bin"), [0u8; 232]).unwrap();
        fs::write(tmp.path().join("producer.lock"), b"12345\n").unwrap();

        let result = wipe_data_dir(tmp.path(), true).unwrap();

        match result {
            WipeResult::Wiped { deleted, remaining } => {
                // ALL 8 items must be deleted
                assert_eq!(deleted, 7, "should delete all 7 chain data items");
                // NOTHING should remain
                assert!(
                    remaining.is_empty(),
                    "directory must be clean, but found: {:?}",
                    remaining
                );
            }
            other => panic!("Expected Wiped, got {:?}", other),
        }

        // Double-check: the two bug files are gone
        assert!(
            !tmp.path().join("maintainer_state.bin").exists(),
            "maintainer_state.bin must not survive wipe"
        );
        assert!(
            !tmp.path().join("producer.lock").exists(),
            "producer.lock must not survive wipe"
        );
    }

    /// Test 10: multi-node layout (node1/data/) also wiped
    /// remove_dir_all on node1/ removes everything inside recursively,
    /// so the sub-files are counted but already gone when deletion runs.
    #[test]
    fn test_wipe_multinode_layout() {
        let tmp = TempDir::new().unwrap();
        let n1 = tmp.path().join("node1");
        let n1_data = n1.join("data");
        fs::create_dir_all(&n1_data).unwrap();
        fs::write(n1_data.join("blocks.db"), b"x").unwrap();
        fs::write(n1_data.join("state.bin"), b"x").unwrap();
        fs::create_dir_all(n1_data.join("keys")).unwrap();

        let result = wipe_data_dir(tmp.path(), true).unwrap();

        match result {
            WipeResult::Wiped { deleted, remaining } => {
                // node1/ is removed via remove_dir_all (takes everything with it)
                // sub-items fail individually (already gone) but node1/ itself succeeds
                assert!(deleted >= 1, "at least node1/ must be deleted");
                assert!(remaining.is_empty());
                assert!(!n1.exists(), "node1/ must be gone");
            }
            other => panic!("Expected Wiped, got {:?}", other),
        }
    }
}
