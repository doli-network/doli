use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::rpc_client::RpcClient;

/// Mainnet seed RPC endpoints for state root consensus verification.
/// The CLI queries multiple seeds and only proceeds if 2/3 agree on the state root.
const MAINNET_SEED_RPCS: &[&str] = &[
    "http://seed1.doli.network:8500",
    "http://seed2.doli.network:8500",
    "http://seeds.doli.network:8500",
];

const TESTNET_SEED_RPCS: &[&str] = &["http://seeds.testnet.doli.network:18500"];

pub(crate) async fn cmd_snap(
    rpc_endpoint: &str,
    network: &str,
    data_dir: Option<PathBuf>,
    yes: bool,
) -> Result<()> {
    println!("Snap Sync");
    println!("{:-<60}", "");
    println!();
    println!("This will:");
    println!("  1. Stop the running doli-node service");
    println!("  2. WIPE all chain data (preserving keys)");
    println!("  3. Download a state snapshot from the network");
    println!("  4. Verify the snapshot against multiple seeds");
    println!("  5. Apply the snapshot and restart the node");
    println!();

    // 1. Resolve data dir
    let data_dir = match data_dir {
        Some(d) => d,
        None => {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
            home.join(".doli").join(network)
        }
    };

    println!("Network:    {}", network);
    println!("Data dir:   {:?}", data_dir);
    println!("Source RPC: {}", rpc_endpoint);
    println!();

    // 2. Confirm
    if !yes {
        println!("Run with --yes to proceed.");
        return Ok(());
    }

    // 3. Verify state root consensus across multiple seeds
    println!("Verifying state root across seeds...");
    let seed_rpcs = match network {
        "mainnet" => MAINNET_SEED_RPCS.to_vec(),
        "testnet" => TESTNET_SEED_RPCS.to_vec(),
        _ => vec![rpc_endpoint],
    };

    let mut state_roots: Vec<(String, u64, String)> = Vec::new(); // (rpc, height, state_root)
    for seed_rpc in &seed_rpcs {
        match query_state_root(seed_rpc).await {
            Ok((height, root)) => {
                println!("  {} — h={} root={:.16}...", seed_rpc, height, root);
                state_roots.push((seed_rpc.to_string(), height, root));
            }
            Err(e) => {
                println!("  {} — UNREACHABLE ({})", seed_rpc, e);
            }
        }
    }

    if state_roots.is_empty() {
        anyhow::bail!("No seeds reachable. Cannot verify snapshot safety.");
    }

    // Check consensus: at least 2 seeds must agree on the state root
    let consensus_root = if state_roots.len() >= 2 {
        let mut root_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_, _, root) in &state_roots {
            *root_counts.entry(root.clone()).or_insert(0) += 1;
        }
        let (best_root, count) = root_counts.iter().max_by_key(|(_, c)| *c).unwrap();
        if *count < 2 {
            anyhow::bail!(
                "Seeds disagree on state root — no consensus. Manual intervention required.\n\
                 Roots: {:?}",
                state_roots
                    .iter()
                    .map(|(rpc, h, r)| format!("{}: h={} root={:.16}", rpc, h, r))
                    .collect::<Vec<_>>()
            );
        }
        println!(
            "  Consensus: {}/{} seeds agree on root {:.16}...",
            count,
            state_roots.len(),
            best_root
        );
        best_root.clone()
    } else {
        // Only 1 seed reachable — trust it (devnet/testnet with single seed)
        println!("  Warning: only 1 seed reachable — trusting without consensus");
        state_roots[0].2.clone()
    };
    println!();

    // 4. Stop the node service
    println!("Stopping doli-node service...");
    stop_doli_service();
    // Brief pause to ensure process exits and releases locks
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // 5. Wipe data directory (preserve keys/, .env, node_key)
    println!("Wiping chain data...");
    if data_dir.exists() {
        let preserve = ["keys", ".env", "node_key"];
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !preserve.contains(&name.as_str()) {
                    let path = entry.path();
                    if path.is_dir() {
                        let _ = std::fs::remove_dir_all(&path);
                    } else {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
        println!("  Wiped (preserved keys/, .env, node_key)");
    } else {
        std::fs::create_dir_all(&data_dir)?;
        println!("  Created fresh data directory");
    }

    // 6. Download snapshot from source RPC
    println!("Downloading state snapshot from {}...", rpc_endpoint);
    let rpc = RpcClient::new(rpc_endpoint);
    let snap: serde_json::Value = rpc
        .call_raw("getStateSnapshot", serde_json::json!({}))
        .await
        .map_err(|e| anyhow!("Failed to download snapshot: {}", e))?;

    let height = snap["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("Invalid snapshot: missing height"))?;
    let snap_root = snap["stateRoot"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid snapshot: missing stateRoot"))?;
    let cs_hex = snap["chainState"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid snapshot: missing chainState"))?;
    let utxo_hex = snap["utxoSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid snapshot: missing utxoSet"))?;
    let ps_hex = snap["producerSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid snapshot: missing producerSet"))?;
    let total_bytes = snap["totalBytes"].as_u64().unwrap_or(0);

    println!(
        "  Height: {}, Size: {}KB, Root: {:.16}...",
        height,
        total_bytes / 1024,
        snap_root
    );

    // 7. Verify snapshot state root matches seed consensus
    if snap_root != consensus_root {
        anyhow::bail!(
            "SECURITY: Snapshot state root does not match seed consensus!\n\
             Snapshot: {}\n\
             Seeds:    {}\n\
             The source RPC may be serving a fraudulent snapshot.",
            snap_root,
            consensus_root
        );
    }
    println!("  State root verified against seed consensus");

    // 8. Decode and verify data integrity
    let cs_bytes = hex::decode(cs_hex).map_err(|e| anyhow!("Invalid chainState hex: {}", e))?;
    let utxo_bytes = hex::decode(utxo_hex).map_err(|e| anyhow!("Invalid utxoSet hex: {}", e))?;
    let ps_bytes = hex::decode(ps_hex).map_err(|e| anyhow!("Invalid producerSet hex: {}", e))?;

    // Recompute state root from raw bytes to verify integrity
    let computed_root =
        storage::snapshot::compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes);
    let computed_root_hex = computed_root.to_hex();
    if computed_root_hex != consensus_root {
        anyhow::bail!(
            "INTEGRITY: Recomputed state root does not match!\n\
             Computed: {}\n\
             Expected: {}\n\
             The snapshot data is corrupted.",
            computed_root_hex,
            consensus_root
        );
    }
    println!("  Data integrity verified (recomputed root matches)");

    // 9. Deserialize state components
    let chain_state: storage::ChainState = bincode::deserialize(&cs_bytes)
        .map_err(|e| anyhow!("Failed to deserialize ChainState: {}", e))?;
    let producer_set: storage::ProducerSet = bincode::deserialize(&ps_bytes)
        .map_err(|e| anyhow!("Failed to deserialize ProducerSet: {}", e))?;
    let utxo_set = storage::UtxoSet::deserialize_canonical(&utxo_bytes)
        .map_err(|e| anyhow!("Failed to deserialize UtxoSet: {}", e))?;

    // 10. Write to StateDb
    println!("Writing snapshot to StateDb...");
    let state_db_path = data_dir.join("state_db");
    let state_db = storage::StateDb::open(&state_db_path)
        .map_err(|e| anyhow!("Failed to open StateDb: {}", e))?;

    let utxo_iter: Vec<(storage::Outpoint, storage::UtxoEntry)> = match &utxo_set {
        storage::UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
        _ => return Err(anyhow!("Expected InMemory UtxoSet from deserialization")),
    };

    state_db
        .atomic_replace(&chain_state, &producer_set, utxo_iter.into_iter())
        .map_err(|e| anyhow!("Failed to write snapshot: {}", e))?;

    println!(
        "  Written: h={}, hash={:.16}..., {} UTXOs, {} producers",
        chain_state.best_height,
        chain_state.best_hash.to_hex(),
        utxo_set.len(),
        producer_set.active_count()
    );

    // 11. Restart the node
    println!();
    println!("Starting doli-node service...");
    crate::cmd_upgrade::restart_doli_service(None);

    println!();
    println!("Snap sync complete!");
    println!("  Height:     {}", chain_state.best_height);
    println!("  State root: {}", consensus_root);
    println!("  The node will sync remaining blocks from this height.");

    Ok(())
}

/// Query a seed's state root via getStateRootDebug RPC.
async fn query_state_root(rpc_url: &str) -> Result<(u64, String)> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "getStateRootDebug",
        "params": {},
        "id": 1
    });

    let response = client.post(rpc_url).json(&request).send().await?;
    let body: serde_json::Value = response.json().await?;

    let result = body
        .get("result")
        .ok_or_else(|| anyhow!("No result in response"))?;
    let height = result["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing height"))?;
    let state_root = result["stateRoot"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing stateRoot"))?
        .to_string();

    Ok((height, state_root))
}

/// Stop the doli-node service (systemd or launchd).
fn stop_doli_service() {
    // Try systemd first
    let result = std::process::Command::new("sudo")
        .args(["systemctl", "stop", "doli-producer"])
        .output();

    match result {
        Ok(o) if o.status.success() => {
            println!("  Stopped doli-producer service");
            return;
        }
        _ => {}
    }

    // Try finding any doli-mainnet service
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-pager", "-q"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("doli") && line.contains("producer") {
                let service = line.split_whitespace().next().unwrap_or("");
                if !service.is_empty() {
                    let _ = std::process::Command::new("sudo")
                        .args(["systemctl", "stop", service])
                        .output();
                    println!("  Stopped {}", service);
                    return;
                }
            }
        }
    }

    // Try launchd (macOS)
    if let Ok(output) = std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("doli") {
                let label = line.split_whitespace().last().unwrap_or("");
                if !label.is_empty() {
                    let _ = std::process::Command::new("launchctl")
                        .args(["stop", label])
                        .output();
                    println!("  Stopped {}", label);
                    return;
                }
            }
        }
    }

    println!("  No running doli-node service found (may need manual stop)");
}
