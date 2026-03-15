use std::fs;
use std::time::Duration;

use anyhow::Result;
use crypto::{hash_with_domain, PublicKey, ADDRESS_DOMAIN};

use super::config::{devnet_dir, load_config};
use super::process::{is_process_running, load_pid};

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

/// Load producer pubkey_hash from wallet file (for balance queries)
/// Returns the 32-byte hash used as UTXO key, not the 20-byte display address
#[allow(dead_code)]
fn load_producer_pubkey_hash(root: &std::path::Path, node_index: u32) -> Option<String> {
    let wallet_path = root
        .join("keys")
        .join(format!("producer_{}.json", node_index));
    if !wallet_path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&wallet_path).ok()?;
    let wallet: super::Wallet = serde_json::from_str(&contents).ok()?;
    let pubkey_hex = wallet.addresses.first()?.public_key.clone();

    // Parse public key and compute the pubkey_hash used for UTXOs
    let pubkey_bytes = hex::decode(&pubkey_hex).ok()?;
    let pubkey = PublicKey::try_from_slice(&pubkey_bytes).ok()?;
    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_ref());
    Some(pubkey_hash.to_hex())
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
                                        .map_or("-".to_string(), format_doli)
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
        let pid_running = pid.is_some_and(is_process_running);

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
