use std::fs;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tracing::{info, warn};

use super::config::{devnet_dir, load_config};
use super::process::{is_process_running, save_pid, start_node, wait_for_rpc};

/// Start all devnet nodes
pub async fn start() -> Result<()> {
    let config = load_config()?;
    let root = devnet_dir();

    // Check if already running
    let pids_dir = root.join("pids");
    if pids_dir.exists() {
        for entry in fs::read_dir(&pids_dir)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "pid") {
                let pid_str = fs::read_to_string(entry.path())?;
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    if is_process_running(pid) {
                        return Err(anyhow!(
                            "Devnet appears to be running (PID {} found). Run 'doli-node devnet stop' first.",
                            pid
                        ));
                    }
                }
            }
        }
    }

    info!("Starting {} devnet nodes...", config.node_count);

    // Load environment variables from devnet root .env file
    // This allows configuring network parameters via ~/.doli/devnet/.env
    // Child processes inherit these env vars, so nodes will pick them up
    doli_core::network_params::load_env_for_network("devnet", &root);

    // Clean data directories for fresh start (preserves keys and chainspec)
    // This prevents issues with stale data, schema mismatches, or corrupted state
    let data_dir = root.join("data");
    if data_dir.exists() {
        info!("Cleaning data directories for fresh start...");
        for i in 0..config.node_count {
            let node_data = data_dir.join(format!("node{}", i));
            if node_data.exists() {
                fs::remove_dir_all(&node_data)?;
            }
            fs::create_dir_all(&node_data)?;
        }
    }

    // Clean logs directory
    let logs_dir = root.join("logs");
    if logs_dir.exists() {
        for entry in fs::read_dir(&logs_dir)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "log") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    // Get current executable path
    let current_exe = std::env::current_exe()?;

    // Start node 0 first (bootstrap node - no --bootstrap flag)
    let node0_child = start_node(&config, &root, 0, &current_exe, None)?;
    save_pid(&root, 0, node0_child.id())?;
    info!("  Started node 0 (bootstrap) with PID {}", node0_child.id());

    // Give node 0 a moment to initialize before checking RPC
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify node 0 is still running
    if !is_process_running(node0_child.id()) {
        // Check log for errors
        let log_path = root.join("logs").join("node0.log");
        if log_path.exists() {
            let log_content = fs::read_to_string(&log_path).unwrap_or_default();
            let last_lines: Vec<&str> = log_content.lines().rev().take(10).collect();
            warn!("Node 0 exited unexpectedly. Last log lines:");
            for line in last_lines.into_iter().rev() {
                warn!("  {}", line);
            }
        }
        return Err(anyhow!(
            "Node 0 (PID {}) exited unexpectedly",
            node0_child.id()
        ));
    }

    // Wait for node 0 RPC to be ready
    let node0_rpc = format!("127.0.0.1:{}", config.rpc_port(0));
    info!("  Waiting for node 0 RPC at {}...", node0_rpc);
    wait_for_rpc(&node0_rpc, Duration::from_secs(30)).await?;
    info!("  Node 0 RPC ready");

    // Start remaining nodes with bootstrap pointing to node 0
    let bootstrap_addr = format!("/ip4/127.0.0.1/tcp/{}", config.p2p_port(0));
    for i in 1..config.node_count {
        let child = start_node(&config, &root, i, &current_exe, Some(&bootstrap_addr))?;
        save_pid(&root, i, child.id())?;
        info!("  Started node {} with PID {}", i, child.id());

        // Small delay between node starts
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    println!();
    println!("Devnet started successfully!");
    println!("  Nodes: {}", config.node_count);
    println!();
    println!("RPC endpoints:");
    for i in 0..config.node_count {
        println!("  Node {}: http://127.0.0.1:{}", i, config.rpc_port(i));
    }
    println!();
    println!("Commands:");
    println!("  doli-node devnet status  # Check status");
    println!("  doli-node devnet stop    # Stop all nodes");

    Ok(())
}
