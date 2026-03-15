use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tracing::{info, warn};

use super::DevnetConfig;

/// Check if a process is running
#[cfg(unix)]
pub(super) fn is_process_running(pid: u32) -> bool {
    use std::process::Command;
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(super) fn is_process_running(_pid: u32) -> bool {
    // On non-Unix, assume running if PID file exists
    true
}

/// Send signal to process
#[cfg(unix)]
pub(super) fn kill_process(pid: u32, signal: &str) -> bool {
    use std::process::Command;
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(super) fn kill_process(pid: u32, _signal: &str) -> bool {
    // On Windows, use taskkill
    use std::process::Command;
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Save PID file for a node
pub(super) fn save_pid(root: &Path, node_index: u32, pid: u32) -> Result<()> {
    let pid_file = root.join("pids").join(format!("node{}.pid", node_index));
    fs::write(&pid_file, pid.to_string())?;
    Ok(())
}

/// Load PID from file
pub(super) fn load_pid(root: &Path, node_index: u32) -> Result<Option<u32>> {
    let pid_file = root.join("pids").join(format!("node{}.pid", node_index));
    if !pid_file.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&pid_file)?;
    Ok(contents.trim().parse().ok())
}

/// Scan pids directory and kill all node processes
/// Returns (stopped_count, node_indices)
pub(super) fn scan_and_kill_all_pids(root: &Path, force: bool) -> (usize, Vec<usize>) {
    let pids_dir = root.join("pids");
    let mut stopped = 0;
    let mut indices = Vec::new();

    if !pids_dir.exists() {
        return (0, indices);
    }

    // Scan for all node*.pid files
    if let Ok(entries) = fs::read_dir(&pids_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                // Match node*.pid pattern
                if filename.starts_with("node") && filename.ends_with(".pid") {
                    // Extract node index
                    if let Some(idx_str) = filename
                        .strip_prefix("node")
                        .and_then(|s| s.strip_suffix(".pid"))
                    {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            indices.push(idx);

                            // Read PID from file
                            if let Ok(content) = fs::read_to_string(&path) {
                                if let Ok(pid) = content.trim().parse::<u32>() {
                                    if is_process_running(pid) {
                                        info!("  Stopping node {} (PID {})...", idx, pid);

                                        // Send SIGTERM first
                                        #[cfg(unix)]
                                        {
                                            kill_process(pid, "-TERM");
                                        }
                                        #[cfg(not(unix))]
                                        {
                                            kill_process(pid, "");
                                        }

                                        // Brief wait
                                        std::thread::sleep(Duration::from_millis(500));

                                        // Force kill if still running
                                        if force && is_process_running(pid) {
                                            warn!(
                                                "  Node {} did not stop gracefully, forcing...",
                                                idx
                                            );
                                            #[cfg(unix)]
                                            {
                                                kill_process(pid, "-KILL");
                                            }
                                            #[cfg(not(unix))]
                                            {
                                                kill_process(pid, "");
                                            }
                                        }

                                        stopped += 1;
                                    }
                                }
                            }

                            // Remove PID file
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }

    (stopped, indices)
}

/// Start a single devnet node
pub(super) fn start_node(
    config: &DevnetConfig,
    root: &Path,
    node_index: u32,
    exe_path: &Path,
    bootstrap: Option<&str>,
) -> Result<Child> {
    let data_dir = root.join("data").join(format!("node{}", node_index));
    let log_file = root.join("logs").join(format!("node{}.log", node_index));
    let key_file = root
        .join("keys")
        .join(format!("producer_{}.json", node_index));
    let chainspec = root.join("chainspec.json");

    // Build command arguments
    let mut args = vec![
        "--network".to_string(),
        "devnet".to_string(),
        "--data-dir".to_string(),
        data_dir.to_string_lossy().to_string(),
        "run".to_string(),
        "--producer".to_string(),
        "--producer-key".to_string(),
        key_file.to_string_lossy().to_string(),
        "--chainspec".to_string(),
        chainspec.to_string_lossy().to_string(),
        "--p2p-port".to_string(),
        config.p2p_port(node_index).to_string(),
        "--rpc-port".to_string(),
        config.rpc_port(node_index).to_string(),
        "--metrics-port".to_string(),
        config.metrics_port(node_index).to_string(),
        "--no-auto-update".to_string(),
        "--force-start".to_string(),
        "--yes".to_string(), // Skip interactive confirmation for automation
    ];

    if let Some(addr) = bootstrap {
        args.push("--bootstrap".to_string());
        args.push(addr.to_string());
    }

    // Open log file
    let log = fs::File::create(&log_file)?;

    // Spawn the process with --yes flag (no stdin required)
    let child = Command::new(exe_path)
        .args(&args)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .stdin(Stdio::null()) // No stdin needed with --yes flag
        .spawn()
        .with_context(|| format!("Failed to start node {}", node_index))?;

    Ok(child)
}

/// Wait for RPC to become available
pub(super) async fn wait_for_rpc(addr: &str, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    let url = format!("http://{}", addr);

    while start.elapsed() < timeout {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getChainInfo",
            "params": {},
            "id": 1
        });

        match client.post(&url).json(&request).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {}
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!("Timeout waiting for RPC at {}", addr))
}
