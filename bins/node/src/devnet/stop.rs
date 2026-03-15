use anyhow::Result;
use tracing::info;

use super::config::devnet_dir;
use super::process::scan_and_kill_all_pids;

/// Stop all devnet nodes
pub async fn stop() -> Result<()> {
    let root = devnet_dir();

    if !root.exists() {
        println!("No devnet directory found. Nothing to stop.");
        return Ok(());
    }

    info!("Stopping all devnet nodes...");

    // Scan pids directory for ALL node PID files (not just 0..node_count)
    let (stopped, _indices) = scan_and_kill_all_pids(&root, true);

    if stopped > 0 {
        println!("Stopped {} nodes.", stopped);
    } else {
        println!("No running nodes found.");
    }

    Ok(())
}
