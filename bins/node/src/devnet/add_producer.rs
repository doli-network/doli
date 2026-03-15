use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crypto::{hash_with_domain, KeyPair, ADDRESS_DOMAIN};
use doli_core::types::UNITS_PER_COIN;
use doli_core::Network;
use tracing::{info, warn};

use super::config::{devnet_dir, load_config};
use super::process::{is_process_running, load_pid, save_pid, start_node};
use super::{Wallet, WalletAddress};

/// Find the next producer index by scanning keys/ directory
pub(super) fn next_producer_index(root: &Path) -> u32 {
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
pub async fn add_producer(count: u32, bonds: u32, fund_amount: Option<u64>) -> Result<()> {
    if count == 0 || count > 10 {
        return Err(anyhow!("Count must be between 1 and 10, got {}", count));
    }
    if bonds == 0 {
        return Err(anyhow!("Bonds must be at least 1"));
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

    // Collect available funder wallets (all existing genesis producers)
    let mut funder_wallets: Vec<std::path::PathBuf> = Vec::new();
    for i in 0..config.node_count {
        let w = root.join("keys").join(format!("producer_{}.json", i));
        if w.exists() {
            funder_wallets.push(w);
        }
    }
    if funder_wallets.is_empty() {
        return Err(anyhow!("No funder wallets found in {:?}/keys/", root));
    }
    info!(
        "Found {} funder wallet(s) — will rotate across them",
        funder_wallets.len()
    );

    // Read actual bond_unit from chainspec (params may have stale OnceLock defaults)
    let chainspec_path = root.join("chainspec.json");
    let bond_unit_sats = if chainspec_path.exists() {
        let cs_data = fs::read_to_string(&chainspec_path)?;
        let cs: serde_json::Value = serde_json::from_str(&cs_data)?;
        cs["consensus"]["bond_amount"]
            .as_u64()
            .unwrap_or(Network::Devnet.params().bond_unit)
    } else {
        Network::Devnet.params().bond_unit
    };
    // CLI expects DOLI (not sats): use explicit amount or auto-calc (bonds * bond_unit + 1 buffer)
    let fund_amount_doli =
        fund_amount.unwrap_or_else(|| (bonds as u64 * bond_unit_sats / UNITS_PER_COIN) + 1);

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

        // Compute the new producer's pubkey_hash (64-char hex, required by send)
        let new_pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, keypair.public_key().as_ref());
        let new_addr = new_pubkey_hash.to_hex();

        // 2. Fund from rotating genesis wallet (avoids UTXO double-spend)
        let funder_idx = (idx as usize) % funder_wallets.len();
        let funder_wallet = &funder_wallets[funder_idx];
        let fund_str = format!("{}", fund_amount_doli);
        info!(
            "  Funding {} with {} DOLI from producer_{}...",
            &new_addr[..16],
            fund_str,
            funder_idx
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
                    let confirmed = json["result"]["confirmed"].as_u64().unwrap_or(0);
                    if confirmed >= bond_unit_sats {
                        info!("  Funded (confirmed): {} base units", confirmed);
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
        let bond_str = format!("{}", bonds);
        info!("  Registering producer with {} bond(s)...", bond_str);

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

        // Brief pause between producers (rotating wallets avoids most double-spend issues)
        if count > 1 {
            info!("  Waiting briefly before next producer...");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    println!();
    println!(
        "Added {} producer(s). Use 'doli-node devnet status' to verify.",
        count
    );

    Ok(())
}
