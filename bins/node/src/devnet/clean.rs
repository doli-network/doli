use std::fs;

use anyhow::Result;
use tracing::info;

use super::config::devnet_dir;
use super::process::scan_and_kill_all_pids;

/// Clean devnet data
pub fn clean(keep_keys: bool) -> Result<()> {
    let root = devnet_dir();

    if !root.exists() {
        println!("Devnet directory does not exist. Nothing to clean.");
        return Ok(());
    }

    // First stop any running nodes (scan ALL pid files, not just 0..node_count)
    info!("Stopping any running nodes...");
    let (stopped, _) = scan_and_kill_all_pids(&root, true);
    if stopped > 0 {
        info!("Stopped {} nodes.", stopped);
    }

    if keep_keys {
        // Keep keys directory, delete everything else
        info!("Cleaning devnet data (keeping keys)...");

        // Remove data, logs, pids directories
        if root.join("data").exists() {
            fs::remove_dir_all(root.join("data"))?;
        }
        if root.join("logs").exists() {
            fs::remove_dir_all(root.join("logs"))?;
        }
        if root.join("pids").exists() {
            fs::remove_dir_all(root.join("pids"))?;
        }

        // Remove config files but keep keys
        let _ = fs::remove_file(root.join("devnet.toml"));
        let _ = fs::remove_file(root.join("chainspec.json"));

        println!(
            "Cleaned devnet data (keys preserved at {:?})",
            root.join("keys")
        );
    } else {
        // Delete everything
        info!("Removing devnet directory...");
        fs::remove_dir_all(&root)?;
        println!("Removed devnet directory: {:?}", root);
    }

    Ok(())
}
