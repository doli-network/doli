//! Guardian CLI commands — fork detection, emergency halt/resume, checkpoint creation.
//!
//! Wraps the existing Guardian RPC methods (`pauseProduction`, `resumeProduction`,
//! `createCheckpoint`, `getGuardianStatus`) and adds a multi-node fork monitor.

use std::io::{self, Write};

use anyhow::Result;

use crate::commands::GuardianCommands;
use crate::rpc_client::RpcClient;

pub(crate) async fn cmd_guardian(rpc_endpoint: &str, command: GuardianCommands) -> Result<()> {
    match command {
        GuardianCommands::Status => cmd_status(rpc_endpoint).await,
        GuardianCommands::Halt { yes } => cmd_halt(rpc_endpoint, yes).await,
        GuardianCommands::Resume => cmd_resume(rpc_endpoint).await,
        GuardianCommands::Checkpoint => cmd_checkpoint(rpc_endpoint).await,
        GuardianCommands::Monitor {
            endpoints,
            loop_secs,
        } => cmd_monitor(endpoints, loop_secs).await,
    }
}

/// `doli guardian status` — pretty-print getGuardianStatus
async fn cmd_status(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    println!("Guardian Status");
    println!("{:-<60}", "");

    let result = rpc
        .call_raw("getGuardianStatus", serde_json::json!({}))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Cannot connect to node at {}. Details: {}.\nMake sure a DOLI node is running.",
                rpc_endpoint,
                e
            )
        })?;

    let paused = result
        .get("production_paused")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reason = result
        .get("production_block_reason")
        .and_then(|v| v.as_str());
    let height = result
        .get("chain_height")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let slot = result
        .get("chain_slot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let best_hash = result
        .get("best_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let last_cp = result.get("last_checkpoint").and_then(|v| v.as_str());
    let last_healthy = result
        .get("last_healthy_checkpoint")
        .and_then(|v| v.as_str());

    let status_str = if paused { "PAUSED" } else { "ACTIVE" };
    println!("Production:    {}", status_str);
    if let Some(r) = reason {
        println!("  Reason:      {}", r);
    }
    println!();
    println!("Chain Height:  {}", height);
    println!("Chain Slot:    {}", slot);
    println!("Best Hash:     {}", best_hash);
    println!();
    println!("Last Checkpoint:         {}", last_cp.unwrap_or("none"));
    println!(
        "Last Healthy Checkpoint: {}",
        last_healthy.unwrap_or("none")
    );

    Ok(())
}

/// `doli guardian halt` — pause block production
async fn cmd_halt(rpc_endpoint: &str, yes: bool) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !yes {
        print!("Pause block production on {}? [y/N] ", rpc_endpoint);
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") && !input.trim().eq_ignore_ascii_case("yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let result = rpc
        .call_raw("pauseProduction", serde_json::json!({}))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to pause production at {}. Details: {}",
                rpc_endpoint,
                e
            )
        })?;

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let message = result.get("message").and_then(|v| v.as_str()).unwrap_or("");

    println!("Status: {}", status);
    if !message.is_empty() {
        println!("{}", message);
    }

    Ok(())
}

/// `doli guardian resume` — resume block production
async fn cmd_resume(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    let result = rpc
        .call_raw("resumeProduction", serde_json::json!({}))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to resume production at {}. Details: {}",
                rpc_endpoint,
                e
            )
        })?;

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let message = result.get("message").and_then(|v| v.as_str()).unwrap_or("");

    println!("Status: {}", status);
    if !message.is_empty() {
        println!("{}", message);
    }

    Ok(())
}

/// `doli guardian checkpoint` — create RocksDB snapshot
async fn cmd_checkpoint(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    println!("Creating checkpoint...");

    let result = rpc
        .call_raw("createCheckpoint", serde_json::json!([]))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to create checkpoint at {}. Details: {}",
                rpc_endpoint,
                e
            )
        })?;

    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let path = result.get("path").and_then(|v| v.as_str()).unwrap_or("?");
    let height = result.get("height").and_then(|v| v.as_u64()).unwrap_or(0);

    println!("Status:  {}", status);
    println!("Height:  {}", height);
    println!("Path:    {}", path);

    Ok(())
}

/// `doli guardian monitor` — poll multiple nodes, detect chain forks
///
/// Queries `getChainInfo` on each endpoint, groups nodes by best_hash,
/// and reports whether all nodes agree on the same chain tip.
async fn cmd_monitor(endpoints: Vec<String>, loop_secs: Option<u64>) -> Result<()> {
    if endpoints.is_empty() {
        anyhow::bail!(
            "No endpoints specified. Use --endpoint for each node.\n\
             Example: doli guardian monitor --endpoint http://127.0.0.1:8500 --endpoint http://127.0.0.1:8501"
        );
    }

    loop {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("\n[{}] Checking {} node(s)...", timestamp, endpoints.len());
        println!("{:-<60}", "");

        let mut groups: std::collections::BTreeMap<String, Vec<(String, u64)>> =
            std::collections::BTreeMap::new();
        let mut unreachable = Vec::new();

        for ep in &endpoints {
            let rpc = RpcClient::new(ep);
            match rpc.call_raw("getChainInfo", serde_json::json!({})).await {
                Ok(info) => {
                    let hash = info
                        .get("best_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let height = info
                        .get("best_height")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    groups.entry(hash).or_default().push((ep.clone(), height));
                }
                Err(_) => {
                    unreachable.push(ep.clone());
                }
            }
        }

        if groups.len() <= 1 && unreachable.is_empty() {
            // All nodes agree
            if let Some((hash, nodes)) = groups.iter().next() {
                let height = nodes.first().map(|(_, h)| *h).unwrap_or(0);
                println!(
                    "OK — {} node(s) agree at height {} ({}...)",
                    nodes.len(),
                    height,
                    &hash[..hash.len().min(16)]
                );
            }
        } else {
            // Fork or partial reachability
            if groups.len() > 1 {
                println!("FORK DETECTED — {} distinct chain tips!", groups.len());
            }
            for (i, (hash, nodes)) in groups.iter().enumerate() {
                let heights: Vec<String> = nodes.iter().map(|(_, h)| h.to_string()).collect();
                println!(
                    "  Group {} ({} node(s), height(s): {}): {}...",
                    i + 1,
                    nodes.len(),
                    heights.join(", "),
                    &hash[..hash.len().min(16)]
                );
                for (ep, h) in nodes {
                    println!("    {} (h={})", ep, h);
                }
            }
            if !unreachable.is_empty() {
                println!("  Unreachable ({}):", unreachable.len());
                for ep in &unreachable {
                    println!("    {}", ep);
                }
            }
        }

        match loop_secs {
            Some(secs) => {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            }
            None => break,
        }
    }

    Ok(())
}
