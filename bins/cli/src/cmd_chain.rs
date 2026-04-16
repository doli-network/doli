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

/// Orchestrate `doli chain-repair`: verify local integrity, start backfill
/// from a known-good peer, poll progress until complete, re-verify.
///
/// The pure helpers `validate_peer_url`, `format_gap_summary`,
/// `BackfillPhase::from_status`, and `format_progress` live below in this file
/// (unit-tested in `mod repair_chain_tests`).
pub(crate) async fn cmd_chain_repair(
    rpc_endpoint: &str,
    peer: &str,
    yes: bool,
    poll_interval_secs: u64,
    max_wait_secs: u64,
) -> Result<()> {
    // 0. Validate peer URL (pure helper)
    if let Err(msg) = validate_peer_url(peer, rpc_endpoint) {
        anyhow::bail!("{}", msg);
    }

    let rpc = RpcClient::new(rpc_endpoint);

    println!("Chain Repair");
    println!("{:-<60}", "");
    println!("Local:  {}", rpc_endpoint);
    println!("Peer:   {}", peer);
    println!();

    // 1. Check local integrity
    println!("Step 1: checking local chain integrity...");
    let integrity_before = rpc
        .verify_chain_integrity()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot verify local chain at {}: {}", rpc_endpoint, e))?;

    println!("{}", format_gap_summary(&integrity_before));
    if integrity_before.complete {
        println!();
        println!("Nothing to repair.");
        return Ok(());
    }

    // 2. Confirm
    if !yes {
        use std::io::Write;
        print!("Proceed with backfill from {}? [y/N] ", peer);
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if !trimmed.eq_ignore_ascii_case("y") && !trimmed.eq_ignore_ascii_case("yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // 3. Start backfill
    println!();
    println!("Step 2: starting backfill from {}...", peer);
    let start_resp = rpc
        .backfill_from_peer(peer)
        .await
        .map_err(|e| anyhow::anyhow!("backfillFromPeer failed: {}", e))?;

    let started = start_resp
        .get("started")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !started {
        let msg = start_resp
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("server reports no gaps (unexpected — verify just found gaps)");
        println!("Backfill did not start: {}", msg);
        return Ok(());
    }
    if let Some(total) = start_resp.get("total").and_then(|v| v.as_u64()) {
        println!("Backfill started: {} blocks to fetch", total);
    }
    if let Some(gaps) = start_resp.get("gaps").and_then(|v| v.as_str()) {
        println!("Gaps to fill: {}", gaps);
    }
    println!();

    // 4. Poll backfillStatus
    println!("Step 3: polling backfill progress...");
    let started_at = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_secs(poll_interval_secs.max(1));
    let max_wait = std::time::Duration::from_secs(max_wait_secs);
    loop {
        if started_at.elapsed() > max_wait {
            anyhow::bail!(
                "Backfill exceeded max wait of {} seconds. Check node logs; \
                 run 'doli chain-verify' to see progress so far.",
                max_wait_secs
            );
        }
        let status = rpc
            .backfill_status()
            .await
            .map_err(|e| anyhow::anyhow!("backfillStatus failed: {}", e))?;
        let phase = BackfillPhase::from_status(&status);
        println!("  {}", format_progress(&phase));
        match phase {
            BackfillPhase::Running { .. } => {
                tokio::time::sleep(poll_interval).await;
            }
            BackfillPhase::Failed(msg) => {
                anyhow::bail!("Backfill failed: {}", msg);
            }
            BackfillPhase::Complete { .. } => {
                break;
            }
        }
    }
    println!();

    // 5. Re-verify
    println!("Step 4: re-verifying local chain integrity...");
    let integrity_after = rpc
        .verify_chain_integrity()
        .await
        .map_err(|e| anyhow::anyhow!("Re-verify failed: {}", e))?;
    println!("{}", format_gap_summary(&integrity_after));
    if integrity_after.complete {
        println!();
        println!("Chain repair complete.");
    } else {
        println!();
        println!(
            "Some gaps remain. Try a different peer with \
             'doli chain-repair --peer <another-rpc-url>'."
        );
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

// ============================================================================
// INC-I-034 / M-Choice3 — `doli chain-repair` pure helpers
//
// Purpose: operator tool that closes block_store gaps on the LOCAL node by
// calling backfillFromPeer RPC against a known-good peer. Needed before the
// M-Choice1 HF activation so santiago/ivan/seed3 can heal their gaps before
// HALT_PRODUCTION fires.
//
// Spec: specs/scheduler-state-architecture.md → "What ADDS" →
//       "bins/cli/src/repair_chain.rs (new command, ~100 lines)"
// Milestone: docs/.workflow/milestone-progress.md → row `M-Choice3`
// ============================================================================

/// Reject malformed or self-pointing peer RPC URLs.
///
/// Accepts: well-formed `http(s)://HOST[:PORT]` where HOST:PORT differs from
/// `local_endpoint` modulo trailing slash.
///
/// Rejects:
///   - empty string
///   - libp2p peer ID (e.g. `12D3KooW...`) — MEMORY.md rule #1 trap:
///     `backfillFromPeer` takes an RPC URL, NEVER a peer ID
///   - self URL (exact match or trailing-slash variant of `local_endpoint`)
///   - URL missing `http://` or `https://` scheme
pub(crate) fn validate_peer_url(peer: &str, local_endpoint: &str) -> Result<(), String> {
    if peer.is_empty() {
        return Err("peer RPC URL is required (empty string not allowed)".to_string());
    }

    // Detect libp2p peer ID shape before checking scheme — gives a more helpful
    // error message (classic MEMORY.md rule #1 trap).
    let looks_like_peer_id = peer.starts_with("12D3KooW")
        || peer.starts_with("QmS")
        || peer.starts_with("QmT")
        || peer.starts_with("QmX")
        || peer.starts_with("QmY")
        || peer.starts_with("QmZ")
        || peer.starts_with("Qm1")
        || peer.starts_with("Qm2")
        || peer.starts_with("Qm3");
    if looks_like_peer_id {
        return Err(format!(
            "'{}' looks like a libp2p peer ID, not an RPC URL. \
             backfillFromPeer requires an RPC URL like http://HOST:PORT, \
             never a peer id.",
            peer
        ));
    }

    // Self-detection (modulo trailing slash). Compare before scheme check so a
    // missing-scheme self-reference is still flagged as self, not as missing
    // scheme. Note: the two asserted cases are exact-match and trailing-slash.
    let peer_stripped = peer.trim_end_matches('/');
    let local_stripped = local_endpoint.trim_end_matches('/');
    if peer_stripped == local_stripped {
        return Err(format!(
            "peer URL '{}' is the same as the local node endpoint — \
             cannot backfill from self",
            peer
        ));
    }

    // Scheme check
    if !peer.starts_with("http://") && !peer.starts_with("https://") {
        return Err(format!(
            "peer URL '{}' is missing a scheme — use http:// or https://",
            peer
        ));
    }

    Ok(())
}

/// Human summary of a `ChainIntegrity` report.
///
/// - `complete=true`              → "chain is complete (N blocks)"
/// - `missing_count>0`, ≤5 ranges → lists every range
/// - `missing_count>0`, >5 ranges → first 5 ranges + "(... N more ranges)"
pub(crate) fn format_gap_summary(integrity: &crate::rpc_client::ChainIntegrity) -> String {
    if integrity.complete {
        return format!(
            "chain is complete ({} blocks scanned up to tip {})",
            integrity.scanned, integrity.tip
        );
    }

    let total_ranges = integrity.missing.len();
    if total_ranges <= 5 {
        format!(
            "{} missing blocks across {} range(s): {}",
            integrity.missing_count,
            total_ranges,
            integrity.missing.join(", ")
        )
    } else {
        let shown = integrity.missing[..5].join(", ");
        let truncated = total_ranges - 5;
        format!(
            "{} missing blocks across {} range(s): {} (... {} more ranges)",
            integrity.missing_count, total_ranges, shown, truncated
        )
    }
}

/// Phase of an ongoing backfill, derived from a `BackfillStatusResponse`.
#[derive(Debug)]
pub(crate) enum BackfillPhase {
    /// Backfill is actively running.
    Running { imported: u64, total: u64, pct: u64 },
    /// Backfill finished successfully.
    Complete { imported: u64 },
    /// Backfill stopped with an error.
    Failed(String),
}

impl BackfillPhase {
    /// Interpret a `BackfillStatusResponse` from the RPC server into a local phase.
    ///
    ///   running=true              → Running { imported, total, pct }
    ///   running=false + err=Some  → Failed(err)
    ///   running=false + err=None  → Complete { imported }
    pub(crate) fn from_status(s: &crate::rpc_client::BackfillStatusResponse) -> Self {
        if s.running {
            BackfillPhase::Running {
                imported: s.imported,
                total: s.total,
                pct: s.pct,
            }
        } else if let Some(err) = &s.error {
            BackfillPhase::Failed(err.clone())
        } else {
            BackfillPhase::Complete {
                imported: s.imported,
            }
        }
    }
}

/// One-line progress string for a phase.
///
///   Running{i,t,p} → "running: i/t (p%)"
///   Complete{i}    → "complete: imported i block(s)"
///   Failed(m)      → "FAILED: m"
pub(crate) fn format_progress(phase: &BackfillPhase) -> String {
    match phase {
        BackfillPhase::Running {
            imported,
            total,
            pct,
        } => format!("running: {}/{} ({}%)", imported, total, pct),
        BackfillPhase::Complete { imported } => {
            format!("complete: imported {} block(s)", imported)
        }
        BackfillPhase::Failed(msg) => format!("FAILED: {}", msg),
    }
}

// ============================================================================
// Chain-repair orchestrator (defined below) + tests
// ============================================================================

#[cfg(test)]
mod repair_chain_tests {
    use super::*;
    use crate::rpc_client::ChainIntegrity;

    // NOTE: BackfillStatusResponse does not yet exist in rpc_client.rs.
    // The developer must add it (mirroring `crates/rpc/src/types/chain.rs::BackfillStatusResponse`)
    // when implementing the helpers. Tests import it here to pin the contract.
    use crate::rpc_client::BackfillStatusResponse;

    // -----------------------------------------------------------------------
    // Test fixtures
    // -----------------------------------------------------------------------

    const LOCAL: &str = "http://127.0.0.1:8500";

    fn integrity_complete() -> ChainIntegrity {
        ChainIntegrity {
            complete: true,
            tip: 10_000,
            scanned: 10_000,
            missing: vec![],
            missing_count: 0,
            chain_commitment: Some("deadbeef".to_string()),
        }
    }

    fn integrity_with_gaps(ranges: Vec<&str>, total: u64) -> ChainIntegrity {
        ChainIntegrity {
            complete: false,
            tip: 10_000,
            scanned: 10_000,
            missing: ranges.into_iter().map(String::from).collect(),
            missing_count: total,
            chain_commitment: None,
        }
    }

    fn status(
        running: bool,
        imported: u64,
        total: u64,
        pct: u64,
        error: Option<&str>,
    ) -> BackfillStatusResponse {
        BackfillStatusResponse {
            running,
            imported,
            total,
            pct,
            error: error.map(String::from),
        }
    }

    // =======================================================================
    // 1. validate_peer_url
    // Output Contract:
    //   - Mutable params: none
    //   - Receiver: none (free fn)
    //   - Return: Result<(), String> — Ok or Err with specific substrings
    //   - Persistent store: none
    // =======================================================================

    #[test]
    fn test_validate_peer_url_accepts_remote_host() {
        // Path: different host, well-formed URL, explicit scheme
        let r = validate_peer_url("http://192.168.1.10:8500", LOCAL);
        assert!(r.is_ok(), "remote RPC URL must be accepted, got: {:?}", r);
    }

    #[test]
    fn test_validate_peer_url_rejects_empty_string() {
        // Path: empty input
        let r = validate_peer_url("", LOCAL);
        let err = r.expect_err("empty peer must be rejected");
        assert!(
            err.to_lowercase().contains("required") || err.to_lowercase().contains("empty"),
            "error should explain peer is required, got: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_peer_url_rejects_libp2p_peer_id() {
        // Path: user pastes peer ID instead of RPC URL (classic MEMORY.md rule #1 trap).
        // backfillFromPeer takes an RPC URL (http://...), NEVER a libp2p peer ID.
        let r = validate_peer_url("12D3KooWAbCdEfGhIjKlMnOpQrStUvWxYzAbCdEfGhIjKlMn", LOCAL);
        let err = r.expect_err("peer ID must be rejected");
        let lower = err.to_lowercase();
        assert!(
            lower.contains("peer id") && lower.contains("rpc url"),
            "error must explain peer ID vs RPC URL confusion, got: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_peer_url_rejects_exact_self() {
        // Path: peer == local_endpoint exactly — self-backfill is nonsense
        let r = validate_peer_url(LOCAL, LOCAL);
        let err = r.expect_err("self-backfill must be rejected");
        assert!(
            err.to_lowercase().contains("self"),
            "error must mention 'self', got: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_peer_url_rejects_self_after_trailing_slash_strip() {
        // Path: peer == local_endpoint modulo trailing slash — still self
        let r = validate_peer_url("http://127.0.0.1:8500/", LOCAL);
        let err = r.expect_err("self-backfill with trailing slash must be rejected");
        assert!(
            err.to_lowercase().contains("self"),
            "error must mention 'self', got: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_peer_url_rejects_missing_scheme() {
        // Path: bare host:port — no http:// prefix
        let r = validate_peer_url("127.0.0.1:8500", LOCAL);
        let err = r.expect_err("missing scheme must be rejected");
        assert!(
            err.to_lowercase().contains("http"),
            "error must mention http scheme, got: {:?}",
            err
        );
    }

    // =======================================================================
    // 2. format_gap_summary
    // Output Contract:
    //   - Mutable params: none
    //   - Receiver: none (free fn)
    //   - Return: String with specific substrings
    //   - Persistent store: none
    // =======================================================================

    #[test]
    fn test_format_gap_summary_complete_chain() {
        // Path: missing_count=0 → summary says chain is complete
        let s = format_gap_summary(&integrity_complete());
        assert!(
            s.to_lowercase().contains("complete"),
            "summary for complete chain must contain 'complete', got: {:?}",
            s
        );
    }

    #[test]
    fn test_format_gap_summary_small_gap_list() {
        // Path: few ranges, total = 5
        let integrity = integrity_with_gaps(vec!["1-3", "7-8"], 5);
        let s = format_gap_summary(&integrity);
        assert!(
            s.contains("5"),
            "should mention total 5 missing, got: {:?}",
            s
        );
        assert!(
            s.to_lowercase().contains("missing"),
            "should contain word 'missing', got: {:?}",
            s
        );
        assert!(
            s.contains("1-3"),
            "should include first range 1-3, got: {:?}",
            s
        );
        assert!(s.contains("7-8"), "should include range 7-8, got: {:?}", s);
    }

    #[test]
    fn test_format_gap_summary_truncates_after_five_ranges() {
        // Path: more than 5 ranges → truncation with count suffix
        let integrity = integrity_with_gaps(
            vec!["1-2", "5", "9-10", "14", "20-22", "30", "40-41", "55"],
            15,
        );
        let s = format_gap_summary(&integrity);
        // First 5 must appear
        assert!(s.contains("1-2"), "first range must appear, got: {:?}", s);
        assert!(s.contains("5"), "second range must appear, got: {:?}", s);
        assert!(s.contains("9-10"), "third range must appear, got: {:?}", s);
        assert!(s.contains("14"), "fourth range must appear, got: {:?}", s);
        assert!(s.contains("20-22"), "fifth range must appear, got: {:?}", s);
        // After 5th range, truncation marker must indicate how many more
        let lower = s.to_lowercase();
        assert!(
            lower.contains("more") || lower.contains("..."),
            "truncation indicator must appear, got: {:?}",
            s
        );
        // 3 ranges were truncated (5..8 → 3 more)
        assert!(
            s.contains('3'),
            "truncation count must say 3 more ranges, got: {:?}",
            s
        );
    }

    // =======================================================================
    // 3. BackfillPhase::from_status
    // Output Contract:
    //   - Mutable params: none
    //   - Receiver: none (associated fn)
    //   - Return: BackfillPhase enum with correct variant + field values
    //   - Persistent store: none
    //
    // Enum shape expected:
    //   enum BackfillPhase {
    //       Running { imported: u64, total: u64, pct: u64 },
    //       Complete { imported: u64 },
    //       Failed(String),
    //   }
    // =======================================================================

    #[test]
    fn test_backfill_phase_from_status_running() {
        // Path: running=true, no error → Running variant with populated fields
        let s = status(true, 50, 100, 50, None);
        let phase = BackfillPhase::from_status(&s);
        match phase {
            BackfillPhase::Running {
                imported,
                total,
                pct,
            } => {
                assert_eq!(imported, 50);
                assert_eq!(total, 100);
                assert_eq!(pct, 50);
            }
            other => panic!("expected Running, got {:?}", other),
        }
    }

    #[test]
    fn test_backfill_phase_from_status_failed() {
        // Path: running=false, error=Some(...) → Failed(msg)
        let s = status(
            false,
            42,
            100,
            42,
            Some("HTTP error at height 42: connection refused"),
        );
        let phase = BackfillPhase::from_status(&s);
        match phase {
            BackfillPhase::Failed(msg) => {
                assert!(
                    msg.contains("HTTP error at height 42"),
                    "failure message must be preserved, got: {:?}",
                    msg
                );
            }
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn test_backfill_phase_from_status_complete() {
        // Path: running=false, error=None → Complete(imported)
        let s = status(false, 100, 100, 100, None);
        let phase = BackfillPhase::from_status(&s);
        match phase {
            BackfillPhase::Complete { imported } => {
                assert_eq!(imported, 100);
            }
            other => panic!("expected Complete, got {:?}", other),
        }
    }

    // =======================================================================
    // 4. format_progress
    // Output Contract:
    //   - Mutable params: none
    //   - Receiver: none (free fn)
    //   - Return: String with specific substrings per phase
    //   - Persistent store: none
    // =======================================================================

    #[test]
    fn test_format_progress_running() {
        // Path: Running variant
        let phase = BackfillPhase::Running {
            imported: 50,
            total: 100,
            pct: 50,
        };
        let s = format_progress(&phase);
        assert!(
            s.contains("50/100"),
            "must show imported/total, got: {:?}",
            s
        );
        assert!(s.contains("50%"), "must show percentage, got: {:?}", s);
    }

    #[test]
    fn test_format_progress_complete() {
        // Path: Complete variant
        let phase = BackfillPhase::Complete { imported: 100 };
        let s = format_progress(&phase);
        let lower = s.to_lowercase();
        assert!(
            lower.contains("imported 100") || lower.contains("imported: 100"),
            "must announce imported 100, got: {:?}",
            s
        );
    }

    #[test]
    fn test_format_progress_failed() {
        // Path: Failed variant
        let phase = BackfillPhase::Failed("connection refused".to_string());
        let s = format_progress(&phase);
        assert!(
            s.contains("FAILED"),
            "must contain 'FAILED' token, got: {:?}",
            s
        );
        assert!(
            s.contains("connection refused"),
            "must contain the failure message, got: {:?}",
            s
        );
    }
}
