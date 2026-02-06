//! Local devnet management for development and testing.
//!
//! This module provides CLI commands to manage a local multi-node devnet:
//! - `devnet init --nodes N` - Initialize devnet with N producers
//! - `devnet start` - Start all nodes
//! - `devnet stop` - Stop all nodes
//! - `devnet status` - Show chain status
//! - `devnet clean [--keep-keys]` - Remove devnet data

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crypto::{hash_with_domain, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::chainspec::{ChainSpec, ConsensusSpec, GenesisProducer, GenesisSpec};
use doli_core::Network;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Read .env from the repo root directory at runtime.
fn load_devnet_env() -> Result<String> {
    // Read from .env in current working directory (repo root)
    let env_file = std::env::current_dir()?.join(".env");

    if !env_file.exists() {
        return Err(anyhow!(
            "Missing .env in current directory: {:?}\n\
             Run this command from the repository root.\n\
             You can copy .env.example to .env to get started.",
            std::env::current_dir().unwrap_or_default()
        ));
    }

    let contents =
        fs::read_to_string(&env_file).with_context(|| format!("Failed to read {:?}", env_file))?;

    info!("Loaded .env from {:?}", env_file);
    Ok(contents)
}

/// Devnet configuration stored in devnet.toml
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevnetConfig {
    /// Number of nodes in the devnet
    pub node_count: u32,
    /// Base P2P port (nodes use base + node_index)
    pub base_p2p_port: u16,
    /// Base RPC port (nodes use base + node_index)
    pub base_rpc_port: u16,
    /// Base metrics port (nodes use base + node_index)
    pub base_metrics_port: u16,
}

impl Default for DevnetConfig {
    fn default() -> Self {
        Self {
            node_count: 3,
            base_p2p_port: 50303,
            base_rpc_port: 28545,
            base_metrics_port: 9090,
        }
    }
}

impl DevnetConfig {
    /// Get P2P port for a specific node (0-indexed)
    pub fn p2p_port(&self, node_index: u32) -> u16 {
        self.base_p2p_port + node_index as u16
    }

    /// Get RPC port for a specific node (0-indexed)
    pub fn rpc_port(&self, node_index: u32) -> u16 {
        self.base_rpc_port + node_index as u16
    }

    /// Get metrics port for a specific node (0-indexed)
    pub fn metrics_port(&self, node_index: u32) -> u16 {
        self.base_metrics_port + node_index as u16
    }
}

/// Wallet file format (compatible with doli-cli)
#[derive(Clone, Debug, Serialize, Deserialize)]
struct WalletAddress {
    address: String,
    public_key: String,
    private_key: String,
    label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Wallet {
    name: String,
    version: u32,
    addresses: Vec<WalletAddress>,
}

/// Get the devnet root directory
fn devnet_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".doli")
        .join("devnet")
}

/// Load devnet configuration
fn load_config() -> Result<DevnetConfig> {
    let config_path = devnet_dir().join("devnet.toml");
    if !config_path.exists() {
        return Err(anyhow!(
            "Devnet not initialized. Run 'doli-node devnet init' first."
        ));
    }
    let contents = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {:?}", config_path))?;
    toml::from_str(&contents).with_context(|| "Failed to parse devnet.toml")
}

/// Save devnet configuration
fn save_config(config: &DevnetConfig) -> Result<()> {
    let config_path = devnet_dir().join("devnet.toml");
    let contents = toml::to_string_pretty(config)?;
    fs::write(&config_path, contents)?;
    Ok(())
}

/// Initialize a new devnet with N producer nodes
pub fn init(node_count: u32) -> Result<()> {
    // Validate node count
    if node_count == 0 || node_count > 20 {
        return Err(anyhow!(
            "Node count must be between 1 and 20, got {}",
            node_count
        ));
    }

    let root = devnet_dir();

    // Check if already initialized
    if root.join("devnet.toml").exists() {
        return Err(anyhow!(
            "Devnet already initialized at {:?}. Run 'doli-node devnet clean' first.",
            root
        ));
    }

    // Create root directory first so we can load .env
    fs::create_dir_all(&root)?;

    // Create default .env file from .env.devnet in repo root
    let env_path = root.join(".env");
    if !env_path.exists() {
        let env_contents = load_devnet_env()?;
        fs::write(&env_path, &env_contents)
            .with_context(|| format!("Failed to create {:?}", env_path))?;
        info!("  Created default .env configuration: {:?}", env_path);
    }

    // Load environment variables from devnet root .env file
    // This allows customizing network parameters before init
    doli_core::network_params::load_env_for_network("devnet", &root);

    info!(
        "Initializing devnet with {} nodes at {:?}",
        node_count, root
    );

    // Create directory structure
    fs::create_dir_all(&root)?;
    fs::create_dir_all(root.join("keys"))?;
    fs::create_dir_all(root.join("data"))?;
    fs::create_dir_all(root.join("logs"))?;
    fs::create_dir_all(root.join("pids"))?;

    // Generate wallets for each producer
    let mut genesis_producers = Vec::with_capacity(node_count as usize);

    for i in 0..node_count {
        let keypair = KeyPair::generate();
        let wallet = Wallet {
            name: format!("producer_{}", i),
            version: 1,
            addresses: vec![WalletAddress {
                address: keypair.address().to_hex(),
                public_key: keypair.public_key().to_hex(),
                private_key: keypair.private_key().to_hex(),
                label: Some("primary".to_string()),
            }],
        };

        // Save wallet file
        let wallet_path = root.join("keys").join(format!("producer_{}.json", i));
        let wallet_json = serde_json::to_string_pretty(&wallet)?;
        fs::write(&wallet_path, wallet_json)?;
        info!("  Created wallet: {:?}", wallet_path);

        // Add to genesis producers
        genesis_producers.push(GenesisProducer {
            name: format!("producer_{}", i),
            public_key: keypair.public_key().to_hex(),
            bond_count: 1,
        });

        // Create node data directory
        fs::create_dir_all(root.join("data").join(format!("node{}", i)))?;
    }

    // Build chainspec using NetworkParams (which reads from .env)
    // This allows customizing devnet parameters via ~/.doli/devnet/.env
    let params = Network::Devnet.params();

    // CRITICAL: Set a fixed genesis time for ALL nodes to ensure they compute
    // the same current_slot at the same wall-clock time. Without this, each node
    // would use its own startup time as genesis, breaking round-robin consensus.
    //
    // Set genesis time a few seconds in the future to give nodes time to start
    // and discover each other before production begins.
    let genesis_delay_secs = 15; // 15 seconds from now
    let genesis_time = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        // Round to slot boundary and add delay
        (now / params.slot_duration + genesis_delay_secs / params.slot_duration + 1)
            * params.slot_duration
    };

    let chainspec = ChainSpec {
        name: "DOLI Local Devnet".into(),
        id: "local-devnet".into(),
        network: Network::Devnet,
        genesis: GenesisSpec {
            timestamp: genesis_time, // Fixed time for all nodes
            message: "DOLI Local Devnet - Development and Testing".into(),
            initial_reward: params.initial_reward,
        },
        consensus: ConsensusSpec {
            slot_duration: params.slot_duration,
            slots_per_epoch: params.slots_per_reward_epoch,
            bond_amount: params.bond_unit,
        },
        genesis_producers,
    };

    // Save chainspec
    let chainspec_path = root.join("chainspec.json");
    chainspec.save(&chainspec_path)?;
    info!("  Created chainspec: {:?}", chainspec_path);

    // Save devnet config
    let config = DevnetConfig {
        node_count,
        ..Default::default()
    };
    save_config(&config)?;
    info!("  Created config: {:?}", root.join("devnet.toml"));

    println!();
    println!("Devnet initialized successfully!");
    println!("  Nodes: {}", node_count);
    println!("  Directory: {:?}", root);
    println!();
    println!("Next steps:");
    println!("  doli-node devnet start   # Start all nodes");
    println!("  doli-node devnet status  # Check status");

    Ok(())
}

/// Start all devnet nodes
pub async fn start() -> Result<()> {
    let config = load_config()?;
    let root = devnet_dir();

    // Check if already running
    let pids_dir = root.join("pids");
    if pids_dir.exists() {
        for entry in fs::read_dir(&pids_dir)? {
            let entry = entry?;
            if entry.path().extension().map_or(false, |e| e == "pid") {
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
            if entry.path().extension().map_or(false, |e| e == "log") {
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

/// Start a single devnet node
fn start_node(
    config: &DevnetConfig,
    root: &PathBuf,
    node_index: u32,
    exe_path: &PathBuf,
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
        "--no-dht".to_string(),
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

/// Save PID file for a node
fn save_pid(root: &PathBuf, node_index: u32, pid: u32) -> Result<()> {
    let pid_file = root.join("pids").join(format!("node{}.pid", node_index));
    fs::write(&pid_file, pid.to_string())?;
    Ok(())
}

/// Load PID from file
fn load_pid(root: &PathBuf, node_index: u32) -> Result<Option<u32>> {
    let pid_file = root.join("pids").join(format!("node{}.pid", node_index));
    if !pid_file.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&pid_file)?;
    Ok(contents.trim().parse().ok())
}

/// Check if a process is running
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    use std::process::Command;
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_process_running(_pid: u32) -> bool {
    // On non-Unix, assume running if PID file exists
    true
}

/// Send signal to process
#[cfg(unix)]
fn kill_process(pid: u32, signal: &str) -> bool {
    use std::process::Command;
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn kill_process(pid: u32, _signal: &str) -> bool {
    // On Windows, use taskkill
    use std::process::Command;
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Scan pids directory and kill all node processes
/// Returns (stopped_count, node_indices)
fn scan_and_kill_all_pids(root: &Path, force: bool) -> (usize, Vec<usize>) {
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

/// Wait for RPC to become available
async fn wait_for_rpc(addr: &str, timeout: Duration) -> Result<()> {
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

/// Load producer pubkey_hash from wallet file (for balance queries)
/// Returns the 32-byte hash used as UTXO key, not the 20-byte display address
fn load_producer_pubkey_hash(root: &PathBuf, node_index: u32) -> Option<String> {
    let wallet_path = root
        .join("keys")
        .join(format!("producer_{}.json", node_index));
    if !wallet_path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&wallet_path).ok()?;
    let wallet: Wallet = serde_json::from_str(&contents).ok()?;
    let pubkey_hex = wallet.addresses.first()?.public_key.clone();

    // Parse public key and compute the pubkey_hash used for UTXOs
    let pubkey_bytes = hex::decode(&pubkey_hex).ok()?;
    let pubkey = PublicKey::try_from_slice(&pubkey_bytes).ok()?;
    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_ref());
    Some(pubkey_hash.to_hex())
}

/// Format DOLI amount (raw units to display with 8 decimals)
fn format_doli(raw: u64) -> String {
    const DECIMALS: u64 = 100_000_000; // 8 decimal places
    let whole = raw / DECIMALS;
    let frac = raw % DECIMALS;
    if frac == 0 {
        format!("{}", whole)
    } else {
        // Format with decimals, trimming trailing zeros
        let frac_str = format!("{:08}", frac);
        let trimmed = frac_str.trim_end_matches('0');
        format!("{}.{}", whole, trimmed)
    }
}

/// Show devnet status
pub async fn status() -> Result<()> {
    let config = match load_config() {
        Ok(c) => c,
        Err(_) => {
            println!("Devnet not initialized. Run 'doli-node devnet init' first.");
            return Ok(());
        }
    };

    let root = devnet_dir();
    let client = reqwest::Client::new();

    // Find the first running node's RPC for network queries
    let mut network_rpc_url: Option<String> = None;
    for i in 0..config.node_count {
        if let Ok(Some(pid)) = load_pid(&root, i) {
            if is_process_running(pid) {
                network_rpc_url = Some(format!("http://127.0.0.1:{}", config.rpc_port(i)));
                break;
            }
        }
    }

    // Get chain info from first running node
    let (chain_height, chain_slot) = if let Some(ref url) = network_rpc_url {
        let chain_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getChainInfo",
            "params": {},
            "id": 1
        });

        match client
            .post(url)
            .json(&chain_request)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let result = &json["result"];
                    (
                        result["bestHeight"].as_u64().unwrap_or(0),
                        result["bestSlot"].as_u64().unwrap_or(0),
                    )
                } else {
                    (0, 0)
                }
            }
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    // Query network producers from any running node
    let network_producers = if let Some(ref url) = network_rpc_url {
        let producers_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getProducers",
            "params": { "activeOnly": true },
            "id": 1
        });

        match client
            .post(url)
            .json(&producers_request)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    json["result"].as_array().cloned().unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Print header
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!(
        "║  DOLI Devnet Status              Height: {:<8} Slot: {:<8}    ║",
        chain_height, chain_slot
    );
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Active Producers ({})                                               ║",
        network_producers.len()
    );
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  {:<4} {:<8} {:<20} {:<6} {:<6} {:>14}  ║",
        "#", "Status", "Public Key", "Bonds", "Era", "DOLI"
    );
    println!("╟──────────────────────────────────────────────────────────────────────╢");

    for (idx, producer) in network_producers.iter().enumerate() {
        let pubkey = producer["publicKey"]
            .as_str()
            .unwrap_or("-")
            .chars()
            .take(16)
            .collect::<String>()
            + "...";
        let status = producer["status"].as_str().unwrap_or("-");
        let bonds = producer["bondCount"]
            .as_u64()
            .map_or("-".to_string(), |v| v.to_string());
        let era = producer["era"]
            .as_u64()
            .map_or("-".to_string(), |v| v.to_string());

        // Get balance for this producer
        let balance_str = if let Some(ref url) = network_rpc_url {
            if let Some(full_pubkey) = producer["publicKey"].as_str() {
                // Compute pubkey_hash from public key for balance query
                if let Ok(pubkey_bytes) = hex::decode(full_pubkey) {
                    if let Ok(pubkey) = PublicKey::try_from_slice(&pubkey_bytes) {
                        let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_ref());
                        let balance_request = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "getBalance",
                            "params": { "address": pubkey_hash.to_hex() },
                            "id": 2
                        });

                        match client
                            .post(url)
                            .json(&balance_request)
                            .timeout(Duration::from_secs(2))
                            .send()
                            .await
                        {
                            Ok(resp) => {
                                if let Ok(json) = resp.json::<serde_json::Value>().await {
                                    json["result"]["total"]
                                        .as_u64()
                                        .map_or("-".to_string(), |v| format_doli(v))
                                } else {
                                    "-".to_string()
                                }
                            }
                            Err(_) => "-".to_string(),
                        }
                    } else {
                        "-".to_string()
                    }
                } else {
                    "-".to_string()
                }
            } else {
                "-".to_string()
            }
        } else {
            "-".to_string()
        };

        println!(
            "║  {:<4} {:<8} {:<20} {:<6} {:<6} {:>14}  ║",
            idx, status, pubkey, bonds, era, balance_str
        );
    }

    if network_producers.is_empty() {
        println!("║  (no producers found - is the network running?)                     ║");
    }

    // Determine how many node slots to check based on:
    // 1. Initial node_count from config
    // 2. PID files in pids directory (manually added nodes)
    // 3. Number of active producers (dynamically added producers need nodes)
    let pids_dir = root.join("pids");
    let mut all_node_indices: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();

    // Include configured nodes (0..node_count)
    for i in 0..config.node_count {
        all_node_indices.insert(i);
    }

    // Also include any additional nodes found in pids directory (manually added)
    if pids_dir.exists() {
        if let Ok(entries) = fs::read_dir(&pids_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                    if filename.starts_with("node") && filename.ends_with(".pid") {
                        if let Some(idx_str) = filename
                            .strip_prefix("node")
                            .and_then(|s| s.strip_suffix(".pid"))
                        {
                            if let Ok(idx) = idx_str.parse::<u32>() {
                                all_node_indices.insert(idx);
                            }
                        }
                    }
                }
            }
        }
    }

    // Include slots for all active producers (dynamically registered producers need nodes too)
    // This ensures we show node status for producers added after genesis
    let producer_count = network_producers.len() as u32;
    for i in 0..producer_count {
        all_node_indices.insert(i);
    }

    // Convert to sorted vec
    let all_node_indices: Vec<u32> = all_node_indices.into_iter().collect();

    // Collect node status (probe RPC for each node)
    struct NodeStatus {
        index: u32,
        pid: Option<u32>,
        running: bool,
        height: String,
    }

    let mut node_statuses = Vec::new();
    for &i in &all_node_indices {
        let pid = load_pid(&root, i).ok().flatten();
        let pid_running = pid.map_or(false, is_process_running);

        // Try to get chain info from RPC (also detects running nodes without PID files)
        let url = format!("http://127.0.0.1:{}", config.rpc_port(i));
        let chain_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getChainInfo",
            "params": {},
            "id": 1
        });

        let (rpc_running, height) = match client
            .post(&url)
            .json(&chain_request)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let result = &json["result"];
                    let h = result["bestHeight"]
                        .as_u64()
                        .map_or("-".to_string(), |v| v.to_string());
                    (true, h)
                } else {
                    (false, "-".to_string())
                }
            }
            Err(_) => (false, "-".to_string()),
        };

        // Node is running if either PID check or RPC check succeeds
        let running = pid_running || rpc_running;
        node_statuses.push(NodeStatus {
            index: i,
            pid,
            running,
            height,
        });
    }

    // Count running nodes
    let running_count = node_statuses.iter().filter(|n| n.running).count();
    let total_nodes = node_statuses.len();

    // Show managed nodes section
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Managed Nodes ({}/{} running)                                        ║",
        running_count, total_nodes
    );
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  {:<4} {:<8} {:<8} {:<12} {:<10} {:>14}  ║",
        "Node", "Status", "PID", "P2P", "RPC", "Height"
    );
    println!("╟──────────────────────────────────────────────────────────────────────╢");
    for ns in &node_statuses {
        let status = if ns.running { "Running" } else { "Stopped" };
        let pid_str = ns.pid.map_or("-".to_string(), |p| p.to_string());

        println!(
            "║  {:<4} {:<8} {:<8} {:<12} {:<10} {:>14}  ║",
            ns.index,
            status,
            pid_str,
            config.p2p_port(ns.index),
            config.rpc_port(ns.index),
            ns.height
        );
    }

    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    Ok(())
}

/// Find the next producer index by scanning keys/ directory
fn next_producer_index(root: &Path) -> u32 {
    let keys_dir = root.join("keys");
    let mut max_idx: Option<u32> = None;

    if let Ok(entries) = fs::read_dir(&keys_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(idx_str) = name
                    .strip_prefix("producer_")
                    .and_then(|s| s.strip_suffix(".json"))
                {
                    if let Ok(idx) = idx_str.parse::<u32>() {
                        max_idx = Some(max_idx.map_or(idx, |m: u32| m.max(idx)));
                    }
                }
            }
        }
    }

    max_idx.map_or(0, |m| m + 1)
}

/// Run the doli CLI binary with given args, capturing stdout
fn run_cli(args: &[&str]) -> Result<String> {
    // Use the doli binary from the same directory as our executable
    let our_exe = std::env::current_exe()?;
    let bin_dir = our_exe.parent().unwrap_or(Path::new("."));
    let cli_path = bin_dir.join("doli");

    if !cli_path.exists() {
        return Err(anyhow!(
            "doli CLI not found at {:?}. Build it with: cargo build -p doli-cli",
            cli_path
        ));
    }

    let output = Command::new(&cli_path)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run doli CLI: {:?} {:?}", cli_path, args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("doli CLI failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Add producer(s) to a running devnet
pub async fn add_producer(count: u32) -> Result<()> {
    if count == 0 || count > 10 {
        return Err(anyhow!("Count must be between 1 and 10, got {}", count));
    }

    let config = load_config()?;
    let root = devnet_dir();

    // Verify devnet is running (check that at least one node has a live RPC)
    let mut rpc_url: Option<String> = None;
    for i in 0..config.node_count {
        if let Ok(Some(pid)) = load_pid(&root, i) {
            if is_process_running(pid) {
                let url = format!("http://127.0.0.1:{}", config.rpc_port(i));
                // Quick RPC check
                let client = reqwest::Client::new();
                let req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "getChainInfo",
                    "params": {},
                    "id": 1
                });
                if let Ok(resp) = client
                    .post(&url)
                    .json(&req)
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        rpc_url = Some(url);
                        break;
                    }
                }
            }
        }
    }

    let rpc_url = rpc_url.ok_or_else(|| {
        anyhow!("No running devnet nodes found. Start the devnet first: doli-node devnet start")
    })?;

    info!("Using RPC: {}", rpc_url);

    // Load funding wallet (producer_0 — the first genesis producer)
    let funder_wallet = root.join("keys").join("producer_0.json");
    if !funder_wallet.exists() {
        return Err(anyhow!("Funder wallet not found: {:?}", funder_wallet));
    }

    // Get the bond amount from the chainspec or network params
    let params = Network::Devnet.params();
    let bond_amount = params.bond_unit;
    // Need enough for bond + a small buffer for tx fees
    let fund_amount = bond_amount * 2;

    let current_exe = std::env::current_exe()?;
    let bootstrap_addr = format!("/ip4/127.0.0.1/tcp/{}", config.p2p_port(0));

    // Load environment variables (child nodes inherit them)
    doli_core::network_params::load_env_for_network("devnet", &root);

    for _ in 0..count {
        let idx = next_producer_index(&root);
        info!("Adding producer {} ...", idx);

        // 1. Generate wallet
        let keypair = KeyPair::generate();
        let wallet = Wallet {
            name: format!("producer_{}", idx),
            version: 1,
            addresses: vec![WalletAddress {
                address: keypair.address().to_hex(),
                public_key: keypair.public_key().to_hex(),
                private_key: keypair.private_key().to_hex(),
                label: Some("primary".to_string()),
            }],
        };

        let wallet_path = root.join("keys").join(format!("producer_{}.json", idx));
        let wallet_json = serde_json::to_string_pretty(&wallet)?;
        fs::write(&wallet_path, &wallet_json)?;
        info!("  Created wallet: {:?}", wallet_path);

        // Compute the new producer's address (pubkey_hash)
        let new_addr = keypair.address().to_hex();

        // 2. Fund from producer_0
        let fund_str = format!("{}", fund_amount);
        info!(
            "  Funding {} with {} base units from producer_0...",
            &new_addr[..16],
            fund_str
        );

        let funder_str = funder_wallet.to_string_lossy().to_string();
        let fund_output = run_cli(&[
            "-r",
            &rpc_url,
            "-w",
            &funder_str,
            "send",
            &new_addr,
            &fund_str,
        ])?;
        info!("  Fund tx: {}", fund_output.trim());

        // 3. Wait for funding to confirm (poll balance)
        info!("  Waiting for funding confirmation...");
        let client = reqwest::Client::new();
        let new_pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, keypair.public_key().as_ref());
        let max_wait = Duration::from_secs(60);
        let start = std::time::Instant::now();
        let mut funded = false;

        while start.elapsed() < max_wait {
            let balance_req = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "getBalance",
                "params": { "address": new_pubkey_hash.to_hex() },
                "id": 1
            });

            if let Ok(resp) = client
                .post(&rpc_url)
                .json(&balance_req)
                .timeout(Duration::from_secs(3))
                .send()
                .await
            {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let total = json["result"]["total"].as_u64().unwrap_or(0);
                    if total >= bond_amount {
                        info!("  Funded: {} base units", total);
                        funded = true;
                        break;
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        if !funded {
            warn!(
                "  Timeout waiting for funding of producer {}. Skipping registration.",
                idx
            );
            continue;
        }

        // 4. Register as producer
        let new_wallet_str = wallet_path.to_string_lossy().to_string();
        let bond_str = format!("{}", bond_amount);
        info!("  Registering producer with bond={} ...", bond_str);

        let reg_output = run_cli(&[
            "-r",
            &rpc_url,
            "-w",
            &new_wallet_str,
            "producer",
            "register",
            "-b",
            &bond_str,
        ])?;
        info!("  Register tx: {}", reg_output.trim());

        // 5. Wait for registration to confirm
        info!("  Waiting for registration confirmation...");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // 6. Create data directory and start node
        let data_dir = root.join("data").join(format!("node{}", idx));
        fs::create_dir_all(&data_dir)?;

        let child = start_node(&config, &root, idx, &current_exe, Some(&bootstrap_addr))?;
        save_pid(&root, idx, child.id())?;
        info!(
            "  Started node {} (PID {}, P2P:{}, RPC:{})",
            idx,
            child.id(),
            config.p2p_port(idx),
            config.rpc_port(idx)
        );

        println!(
            "Producer {} added (PID {}, RPC: http://127.0.0.1:{})",
            idx,
            child.id(),
            config.rpc_port(idx)
        );
    }

    println!();
    println!(
        "Added {} producer(s). Use 'doli-node devnet status' to verify.",
        count
    );

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_devnet_config_ports() {
        let config = DevnetConfig::default();
        assert_eq!(config.p2p_port(0), 50303);
        assert_eq!(config.p2p_port(1), 50304);
        assert_eq!(config.rpc_port(0), 28545);
        assert_eq!(config.rpc_port(1), 28546);
        assert_eq!(config.metrics_port(0), 9090);
        assert_eq!(config.metrics_port(1), 9091);
    }

    #[test]
    fn test_next_producer_index() {
        let tmp = std::env::temp_dir().join("doli_test_next_idx");
        let keys_dir = tmp.join("keys");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&keys_dir).unwrap();

        // Empty directory → index 0
        assert_eq!(next_producer_index(&tmp), 0);

        // Add producer_0.json → next is 1
        fs::write(keys_dir.join("producer_0.json"), "{}").unwrap();
        assert_eq!(next_producer_index(&tmp), 1);

        // Add producer_2.json (gap) → next is 3
        fs::write(keys_dir.join("producer_2.json"), "{}").unwrap();
        assert_eq!(next_producer_index(&tmp), 3);

        let _ = fs::remove_dir_all(&tmp);
    }
}
