use std::path::PathBuf;

use anyhow::{anyhow, Result};

/// Seed RPC endpoints per network for state root consensus verification.
fn seed_rpcs(network: &str) -> Vec<&'static str> {
    match network {
        "mainnet" => vec![
            "http://seed1.doli.network:8500",
            "http://seed2.doli.network:8500",
            "http://seeds.doli.network:8500",
        ],
        "testnet" => vec!["http://seeds.testnet.doli.network:18500"],
        _ => vec!["http://127.0.0.1:8500"],
    }
}

/// Default data directory for a network.
fn default_data_dir(network: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".doli").join(network))
}

pub(crate) async fn cmd_snap(
    network: &str,
    data_dir_override: Option<PathBuf>,
    custom_seeds: Vec<String>,
    no_restart: bool,
    trust: bool,
) -> Result<()> {
    let default_seeds = seed_rpcs(network);
    let seeds: Vec<&str> = if custom_seeds.is_empty() {
        default_seeds
    } else {
        custom_seeds.iter().map(|s| s.as_str()).collect()
    };
    let data_dir = match data_dir_override {
        Some(d) => d,
        None => default_data_dir(network)?,
    };

    println!("Snap Sync");
    println!("{:-<60}", "");
    println!();
    println!("Network:  {}", network);
    println!("Data dir: {}", data_dir.display());
    println!(
        "Seeds:    {}{}",
        seeds.len(),
        if !custom_seeds.is_empty() {
            " (custom)"
        } else {
            ""
        }
    );
    println!();

    // 1. Verify state root consensus across seeds
    println!("Verifying state root across seeds...");
    let mut state_roots: Vec<(&str, u64, String)> = Vec::new();
    for seed in &seeds {
        match query_state_root(seed).await {
            Ok((height, root)) => {
                println!("  {} — h={} root={:.16}...", seed, height, root);
                state_roots.push((seed, height, root));
            }
            Err(e) => {
                println!("  {} — UNREACHABLE ({})", seed, e);
            }
        }
    }

    if state_roots.is_empty() {
        anyhow::bail!("No seeds reachable. Cannot verify snapshot safety.");
    }

    // Consensus: 2/3 must agree, or trust single seed for testnet/devnet/--trust
    let (source_rpc, consensus_root) = if state_roots.len() >= 2 && !trust {
        let mut counts: std::collections::HashMap<&str, (usize, &str)> =
            std::collections::HashMap::new();
        for (rpc, _, root) in &state_roots {
            let entry = counts.entry(root.as_str()).or_insert((0, rpc));
            entry.0 += 1;
        }
        let (best_root, (count, best_rpc)) =
            counts.into_iter().max_by_key(|(_, (c, _))| *c).unwrap();
        if count < 2 {
            anyhow::bail!(
                "Seeds disagree on state root — no consensus. Try again later or check seed health."
            );
        }
        println!("  Consensus: {}/{} seeds agree", count, state_roots.len());
        (best_rpc.to_string(), best_root.to_string())
    } else {
        if trust {
            println!("  Trusting single seed (--trust)");
        } else {
            println!("  Warning: only 1 seed reachable — trusting without consensus");
        }
        (state_roots[0].0.to_string(), state_roots[0].2.clone())
    };
    println!();

    // 2. Stop the node service
    if no_restart {
        println!("Skipping service stop (--no-restart)");
    } else {
        println!("Stopping doli-node...");
        stop_doli_service();
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // 3. Wipe data directory (preserve keys/, .env, node_key)
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
    } else {
        std::fs::create_dir_all(&data_dir)?;
    }
    println!("  Done (preserved keys/, .env, node_key)");

    // 4. Download snapshot
    println!("Downloading snapshot from {}...", source_rpc);
    let snap = download_snapshot(&source_rpc).await?;

    let height = snap["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("Missing height"))?;
    let snap_root = snap["stateRoot"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing stateRoot"))?;
    let cs_hex = snap["chainState"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing chainState"))?;
    let utxo_hex = snap["utxoSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing utxoSet"))?;
    let ps_hex = snap["producerSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing producerSet"))?;
    let total_kb = snap["totalBytes"].as_u64().unwrap_or(0) / 1024;

    println!("  Height: {}, Size: {}KB", height, total_kb);

    // 5. Verify against seed consensus
    if snap_root != consensus_root {
        anyhow::bail!(
            "SECURITY: Snapshot root does not match seed consensus!\n\
             Snapshot: {}\n  Seeds: {}",
            snap_root,
            consensus_root
        );
    }

    // 6. Verify data integrity (recompute root from bytes)
    let cs_bytes = hex::decode(cs_hex)?;
    let utxo_bytes = hex::decode(utxo_hex)?;
    let ps_bytes = hex::decode(ps_hex)?;

    let computed =
        storage::snapshot::compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes);
    if computed.to_hex() != consensus_root {
        anyhow::bail!("INTEGRITY: Recomputed state root does not match. Snapshot corrupted.");
    }
    println!("  Verified (root matches {} seeds)", state_roots.len());

    // 7. Write to StateDb
    println!("Applying snapshot...");
    let chain_state: storage::ChainState = bincode::deserialize(&cs_bytes)?;
    let producer_set: storage::ProducerSet = bincode::deserialize(&ps_bytes)?;
    let utxo_set = storage::UtxoSet::deserialize_canonical(&utxo_bytes)
        .map_err(|e| anyhow!("UtxoSet deserialize: {}", e))?;

    let state_db = storage::StateDb::open(&data_dir.join("state_db"))
        .map_err(|e| anyhow!("StateDb open: {}", e))?;

    let utxo_iter: Vec<(storage::Outpoint, storage::UtxoEntry)> = match &utxo_set {
        storage::UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
        _ => return Err(anyhow!("Expected InMemory UtxoSet")),
    };

    state_db
        .atomic_replace(&chain_state, &producer_set, utxo_iter.into_iter())
        .map_err(|e| anyhow!("StateDb write: {}", e))?;

    println!(
        "  Applied: h={}, {} UTXOs, {} producers",
        height,
        utxo_set.len(),
        producer_set.active_count()
    );

    // 8. Restart
    if no_restart {
        println!("Skipping service restart (--no-restart)");
    } else {
        println!("Starting doli-node...");
        crate::cmd_upgrade::restart_doli_service(None);
    }

    println!();
    println!("Snap sync complete at height {}.", height);
    println!("The node will sync remaining blocks automatically.");

    Ok(())
}

/// Query state root from a seed via getStateRootDebug RPC.
async fn query_state_root(rpc_url: &str) -> Result<(u64, String)> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let body: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getStateRootDebug",
            "params": {},
            "id": 1
        }))
        .send()
        .await?
        .json()
        .await?;

    let r = body.get("result").ok_or_else(|| anyhow!("No result"))?;
    Ok((
        r["height"].as_u64().ok_or_else(|| anyhow!("No height"))?,
        r["stateRoot"]
            .as_str()
            .ok_or_else(|| anyhow!("No stateRoot"))?
            .to_string(),
    ))
}

/// Download full state snapshot via getStateSnapshot RPC.
async fn download_snapshot(rpc_url: &str) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120)) // large payload
        .build()?;

    let body: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getStateSnapshot",
            "params": {},
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| anyhow!("Connection failed: {}", e))?
        .json()
        .await?;

    if let Some(err) = body.get("error") {
        anyhow::bail!("RPC error: {}", err);
    }

    body.get("result")
        .cloned()
        .ok_or_else(|| anyhow!("No result"))
}

/// Stop the doli-node service (systemd or launchd).
fn stop_doli_service() {
    // systemd
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-pager", "-q"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("doli") {
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

    // launchd (macOS)
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

    println!("  No running service found");
}
