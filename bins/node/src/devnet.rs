//! Local devnet management for development and testing.
//!
//! This module provides CLI commands to manage a local multi-node devnet:
//! - `devnet init --nodes N` - Initialize devnet with N producers
//! - `devnet start` - Start all nodes
//! - `devnet stop` - Stop all nodes
//! - `devnet status` - Show chain status
//! - `devnet clean [--keep-keys]` - Remove devnet data

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crypto::{hash_with_domain, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::chainspec::{ChainSpec, ConsensusSpec, GenesisProducer, GenesisSpec};
use doli_core::Network;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Read .env.example.devnet from the repo root directory at runtime.
fn load_devnet_env_example() -> Result<String> {
    // Read from .env.example.devnet in current working directory (repo root)
    let env_file = std::env::current_dir()?.join(".env.example.devnet");

    if !env_file.exists() {
        return Err(anyhow!(
            "Missing .env.example.devnet in current directory: {:?}\n\
             Run this command from the repository root.",
            std::env::current_dir().unwrap_or_default()
        ));
    }

    let contents = fs::read_to_string(&env_file)
        .with_context(|| format!("Failed to read {:?}", env_file))?;

    info!("Loaded .env.example.devnet from {:?}", env_file);
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

    // Create default .env file from .env.example.devnet in repo root
    let env_path = root.join(".env");
    if !env_path.exists() {
        let env_contents = load_devnet_env_example()?;
        fs::write(&env_path, &env_contents)
            .with_context(|| format!("Failed to create {:?}", env_path))?;
        info!("  Created default .env configuration: {:?}", env_path);
    }

    // Load environment variables from devnet root .env file
    // This allows customizing network parameters before init
    doli_core::env_loader::load_env_for_network("devnet", &root);

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
    doli_core::env_loader::load_env_for_network("devnet", &root);

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
    let config = match load_config() {
        Ok(c) => c,
        Err(_) => {
            println!("No devnet configuration found. Nothing to stop.");
            return Ok(());
        }
    };

    let root = devnet_dir();
    let mut stopped = 0;

    info!("Stopping {} devnet nodes...", config.node_count);

    for i in 0..config.node_count {
        if let Ok(Some(pid)) = load_pid(&root, i) {
            if is_process_running(pid) {
                info!("  Stopping node {} (PID {})...", i, pid);

                // Send SIGTERM first
                #[cfg(unix)]
                {
                    kill_process(pid, "-TERM");
                }
                #[cfg(not(unix))]
                {
                    kill_process(pid, "");
                }

                // Wait up to 5 seconds for graceful shutdown
                let mut terminated = false;
                for _ in 0..50 {
                    if !is_process_running(pid) {
                        terminated = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }

                // Force kill if still running
                if !terminated {
                    warn!("  Node {} did not stop gracefully, forcing...", i);
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

            // Remove PID file
            let pid_file = root.join("pids").join(format!("node{}.pid", i));
            let _ = fs::remove_file(pid_file);
        }
    }

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

    println!();
    println!("DOLI Devnet Status");
    println!("==================");
    println!();
    println!(
        "{:<6} {:<8} {:<12} {:<10} {:<10} {:<10} {:<14}",
        "Node", "Status", "PID", "Height", "Slot", "Peers", "DOLI"
    );
    println!("{}", "-".repeat(74));

    for i in 0..config.node_count {
        let pid = load_pid(&root, i).ok().flatten();
        let running = pid.map_or(false, is_process_running);
        let status = if running { "Running" } else { "Stopped" };
        let pid_str = pid.map_or("-".to_string(), |p| p.to_string());

        // Try to get chain info and balance from RPC
        let (height, slot, peers, balance) = if running {
            let url = format!("http://127.0.0.1:{}", config.rpc_port(i));

            // Get chain info
            let chain_request = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "getChainInfo",
                "params": {},
                "id": 1
            });

            let (h, s, p) = match client
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
                        let s = result["bestSlot"]
                            .as_u64()
                            .map_or("-".to_string(), |v| v.to_string());
                        let p = result["peerCount"]
                            .as_u64()
                            .map_or("-".to_string(), |v| v.to_string());
                        (h, s, p)
                    } else {
                        ("-".to_string(), "-".to_string(), "-".to_string())
                    }
                }
                Err(_) => ("-".to_string(), "-".to_string(), "-".to_string()),
            };

            // Get producer balance
            let balance_str = if let Some(pubkey_hash) = load_producer_pubkey_hash(&root, i) {
                let balance_request = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "getBalance",
                    "params": { "address": pubkey_hash },
                    "id": 2
                });

                match client
                    .post(&url)
                    .json(&balance_request)
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            let result = &json["result"];
                            // Use total balance (includes immature rewards)
                            result["total"]
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
            };

            (h, s, p, balance_str)
        } else {
            (
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
            )
        };

        println!(
            "{:<6} {:<8} {:<12} {:<10} {:<10} {:<10} {:<14}",
            i, status, pid_str, height, slot, peers, balance
        );
    }

    println!();
    println!("Ports:");
    for i in 0..config.node_count {
        println!(
            "  Node {}: P2P={}, RPC={}, Metrics={}",
            i,
            config.p2p_port(i),
            config.rpc_port(i),
            config.metrics_port(i)
        );
    }
    println!();

    Ok(())
}

/// Clean devnet data
pub fn clean(keep_keys: bool) -> Result<()> {
    let root = devnet_dir();

    if !root.exists() {
        println!("Devnet directory does not exist. Nothing to clean.");
        return Ok(());
    }

    // First stop any running nodes
    info!("Stopping any running nodes...");

    // Try to load config and stop nodes
    if let Ok(config) = load_config() {
        for i in 0..config.node_count {
            if let Ok(Some(pid)) = load_pid(&root, i) {
                if is_process_running(pid) {
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
                    if is_process_running(pid) {
                        #[cfg(unix)]
                        {
                            kill_process(pid, "-KILL");
                        }
                        #[cfg(not(unix))]
                        {
                            kill_process(pid, "");
                        }
                    }
                }
            }
        }
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
}
