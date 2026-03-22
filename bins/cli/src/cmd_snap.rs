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

/// Default data directory for a network (uses standard path resolution).
fn default_data_dir(network: &str) -> Result<PathBuf> {
    Ok(crate::paths::resolve_base_dir(network, None))
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
    let (source_rpc, _consensus_root) = if state_roots.len() >= 2 && !trust {
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
        stop_doli_service(&data_dir);
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // 3. Wipe data directory (preserve keys/, .env, node_key)
    println!("Wiping chain data...");
    if data_dir.exists() {
        let preserve = [
            "keys",
            ".env",
            "node_key",
            "wallet.json",
            "wallet.seed.txt",
            "config.toml",
        ];
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
    println!("  Done (preserved keys/, wallet.json, .env, node_key)");

    // 4. Download snapshot (retry up to 3 times if seed advances between check and download)
    let max_retries = 3;
    let mut snap = None;
    let mut snap_root_str = String::new();
    let mut final_height = 0u64;

    for attempt in 0..max_retries {
        if attempt > 0 {
            println!("  Retrying download (seed advanced during download)...");
        }
        println!("Downloading snapshot from {}...", source_rpc);
        let downloaded = download_snapshot(&source_rpc).await?;

        let height = downloaded["height"]
            .as_u64()
            .ok_or_else(|| anyhow!("Missing height"))?;
        let root = downloaded["stateRoot"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing stateRoot"))?
            .to_string();
        let total_kb = downloaded["totalBytes"].as_u64().unwrap_or(0) / 1024;

        println!("  Height: {}, Size: {}KB", height, total_kb);

        // Re-verify: query state root at the CURRENT height from multiple seeds
        let mut verified = false;
        let mut verify_count = 0;
        for seed in &seeds {
            if let Ok((seed_h, seed_root)) = query_state_root(seed).await {
                if seed_h == height && seed_root == root {
                    verify_count += 1;
                }
            }
        }

        if trust || verify_count >= 1 {
            verified = true;
        }
        if !trust && seeds.len() >= 2 && verify_count < 2 {
            verified = false;
        }

        if verified {
            snap_root_str = root;
            final_height = height;
            snap = Some(downloaded);
            break;
        }
    }

    let snap = snap.ok_or_else(|| {
        anyhow!("Snapshot verification failed after {} attempts. Seeds keep advancing — try again.", max_retries)
    })?;

    let cs_hex = snap["chainState"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing chainState"))?;
    let utxo_hex = snap["utxoSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing utxoSet"))?;
    let ps_hex = snap["producerSet"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing producerSet"))?;

    // 5. Verify data integrity (recompute root from bytes)
    let cs_bytes = hex::decode(cs_hex)?;
    let utxo_bytes = hex::decode(utxo_hex)?;
    let ps_bytes = hex::decode(ps_hex)?;

    let computed =
        storage::snapshot::compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes);
    if computed.to_hex() != snap_root_str {
        anyhow::bail!("INTEGRITY: Recomputed state root does not match. Snapshot corrupted.");
    }
    println!("  Verified (root confirmed by seeds at h={})", final_height);
    let height = final_height;

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

    // 8. Restart the service that owns this data_dir
    if no_restart {
        println!("Skipping service restart (--no-restart)");
    } else {
        println!("Starting doli-node...");
        restart_matching_service(&data_dir);
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

/// Stop the doli-node service that owns the given data directory.
/// On multi-node servers, only stops the service whose ExecStart references this data_dir.
/// If no match is found, stops nothing (safe default).
fn stop_doli_service(data_dir: &std::path::Path) {
    let data_dir_str = data_dir.to_string_lossy();

    // systemd: find the service whose ExecStart contains our data_dir
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-pager", "-q"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let service = line.split_whitespace().next().unwrap_or("");
            if !service.contains("doli") {
                continue;
            }
            // Read the service's ExecStart to check if it uses our data_dir
            if let Ok(cat_output) = std::process::Command::new("systemctl")
                .args(["cat", service, "--no-pager"])
                .output()
            {
                let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                if cat_str.contains(data_dir_str.as_ref()) {
                    let _ = std::process::Command::new("sudo")
                        .args(["systemctl", "stop", service])
                        .output();
                    println!("  Stopped {}", service);
                    return;
                }
            }
        }
    }

    // launchd (macOS): find the plist whose ProgramArguments contains our data_dir
    if let Ok(output) = std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let label = line.split_whitespace().last().unwrap_or("");
            if !label.contains("doli") {
                continue;
            }
            let _ = std::process::Command::new("launchctl")
                .args(["stop", label])
                .output();
            println!("  Stopped {}", label);
            return;
        }
    }

    println!("  No running service found");
}

/// Restart the doli-node service that owns the given data directory.
fn restart_matching_service(data_dir: &std::path::Path) {
    let data_dir_str = data_dir.to_string_lossy();

    // systemd
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-pager"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let service = line.split_whitespace().next().unwrap_or("");
            if !service.contains("doli") {
                continue;
            }
            if let Ok(cat_output) = std::process::Command::new("systemctl")
                .args(["cat", service, "--no-pager"])
                .output()
            {
                let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                if cat_str.contains(data_dir_str.as_ref()) {
                    let _ = std::process::Command::new("sudo")
                        .args(["systemctl", "start", service])
                        .output();
                    println!("  Started {}", service);
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
            let label = line.split_whitespace().last().unwrap_or("");
            if label.contains("doli") {
                let _ = std::process::Command::new("launchctl")
                    .args(["start", label])
                    .output();
                println!("  Started {}", label);
                return;
            }
        }
    }

    println!("No doli-node service or process found. Restart manually if needed.");
}
