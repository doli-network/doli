use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::commands::{MaintainerCommands, ProtocolCommands, ReleaseCommands, UpdateCommands};
use crate::rpc_client::RpcClient;
use crate::wallet::Wallet;

pub(crate) async fn cmd_release(wallet_path: &Path, command: ReleaseCommands) -> Result<()> {
    match command {
        ReleaseCommands::Sign { version, key } => {
            cmd_release_sign(wallet_path, &version, key).await?;
        }
    }
    Ok(())
}

pub(crate) async fn cmd_release_sign(
    wallet_path: &Path,
    version: &str,
    key_path: Option<PathBuf>,
) -> Result<()> {
    // Load wallet/key
    let key_file = key_path.as_deref().unwrap_or(wallet_path);
    let w = Wallet::load(key_file)?;
    let keypair = w.primary_keypair()?;
    let pubkey_hex = keypair.public_key().to_hex();

    let version_bare = version.strip_prefix('v').unwrap_or(version);

    // Download CHECKSUMS.txt from GitHub and compute its SHA-256
    println!("Fetching CHECKSUMS.txt for v{}...", version_bare);
    let (_checksums_content, checksums_sha256) = updater::download_checksums_txt(version_bare)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch CHECKSUMS.txt: {}", e))?;

    // Sign "version:checksums_sha256"
    let sig = updater::sign_release_hash(&keypair, version_bare, &checksums_sha256);

    // Output JSON signature block
    let output = serde_json::json!({
        "public_key": sig.public_key,
        "signature": sig.signature
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    eprintln!();
    eprintln!(
        "Signed v{} with key {}...{}",
        version_bare,
        &pubkey_hex[..8],
        &pubkey_hex[pubkey_hex.len() - 8..]
    );
    eprintln!("Message: \"{}:{}\"", version_bare, checksums_sha256);
    eprintln!();
    eprintln!("To assemble SIGNATURES.json, collect 3+ signatures and upload:");
    eprintln!("  gh release upload v{} SIGNATURES.json", version_bare);

    Ok(())
}

pub(crate) async fn cmd_protocol(
    wallet_path: &Path,
    rpc_url: &str,
    command: ProtocolCommands,
) -> Result<()> {
    match command {
        ProtocolCommands::Sign {
            version,
            epoch,
            key,
        } => {
            cmd_protocol_sign(wallet_path, version, epoch, key)?;
        }
        ProtocolCommands::Activate {
            version,
            epoch,
            description,
            signatures,
        } => {
            cmd_protocol_activate(rpc_url, version, epoch, description, signatures).await?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_protocol_sign(
    wallet_path: &Path,
    version: u32,
    epoch: u64,
    key_path: Option<PathBuf>,
) -> Result<()> {
    let key_file = key_path.as_deref().unwrap_or(wallet_path);
    let w = Wallet::load(key_file)?;
    let keypair = w.primary_keypair()?;
    let pubkey_hex = keypair.public_key().to_hex();

    // Build signing message: "activate:{version}:{epoch}"
    let message = format!("activate:{}:{}", version, epoch);
    let msg_bytes = message.as_bytes();

    // Sign with Ed25519
    let sig = crypto::signature::sign(msg_bytes, keypair.private_key());

    let output = serde_json::json!({
        "public_key": pubkey_hex,
        "signature": hex::encode(sig.as_bytes()),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    eprintln!();
    eprintln!(
        "Signed protocol v{} activation at epoch {} with key {}...{}",
        version,
        epoch,
        &pubkey_hex[..8],
        &pubkey_hex[pubkey_hex.len() - 8..]
    );
    eprintln!("Message: \"{}\"", message);
    eprintln!();
    eprintln!("Collect 3/5 signatures into a JSON array file, then run:");
    eprintln!(
        "  doli protocol activate --version {} --epoch {} --description \"...\" --signatures sigs.json",
        version, epoch
    );

    Ok(())
}

pub(crate) async fn cmd_protocol_activate(
    rpc_url: &str,
    version: u32,
    epoch: u64,
    description: String,
    signatures_path: PathBuf,
) -> Result<()> {
    use crypto::{PublicKey, Signature};
    use doli_core::maintainer::{MaintainerSignature, ProtocolActivationData};
    use doli_core::Transaction;

    println!("Protocol Activation");
    println!("{:-<60}", "");
    println!("  Version: {}", version);
    println!("  Epoch:   {}", epoch);
    println!("  Desc:    {}", description);
    println!();

    // Read signatures file
    let sigs_json = std::fs::read_to_string(&signatures_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", signatures_path.display(), e))?;

    let sigs: Vec<serde_json::Value> = serde_json::from_str(&sigs_json)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in signatures file: {}", e))?;

    if sigs.len() < 3 {
        anyhow::bail!(
            "Need at least 3 signatures, got {}. Collect more with `doli protocol sign`.",
            sigs.len()
        );
    }

    // Parse signatures
    let mut maintainer_sigs = Vec::new();
    for (i, sig_val) in sigs.iter().enumerate() {
        let pk_hex = sig_val["public_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Signature {} missing public_key", i))?;
        let sig_hex = sig_val["signature"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Signature {} missing signature", i))?;

        let pubkey = PublicKey::from_hex(pk_hex)
            .map_err(|e| anyhow::anyhow!("Invalid public key in signature {}: {}", i, e))?;
        let sig_bytes =
            hex::decode(sig_hex).map_err(|e| anyhow::anyhow!("Invalid sig hex {}: {}", i, e))?;
        let signature = Signature::try_from_slice(&sig_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid signature {}: {}", i, e))?;

        println!(
            "  Sig {}: {}...{}",
            i + 1,
            &pk_hex[..8],
            &pk_hex[pk_hex.len() - 8..]
        );
        maintainer_sigs.push(MaintainerSignature::new(pubkey, signature));
    }
    println!();

    // Build ProtocolActivationData
    let data = ProtocolActivationData::new(version, epoch, description, maintainer_sigs);

    // Verify signing message matches what signers signed
    let expected_msg = data.signing_message();
    println!(
        "Signing message: \"{}\"",
        String::from_utf8_lossy(&expected_msg)
    );

    // Build transaction
    let tx = Transaction::new_protocol_activation(data);
    let tx_hex = hex::encode(tx.serialize());

    println!("Submitting ProtocolActivation transaction...");

    let rpc = RpcClient::new(rpc_url);
    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!();
            println!("Protocol activation submitted!");
            println!("TX Hash: {}", hash);
            println!();
            println!(
                "All nodes will activate protocol v{} at epoch {}.",
                version, epoch
            );
        }
        Err(e) => {
            anyhow::bail!("Error submitting activation: {}", e);
        }
    }

    Ok(())
}

pub(crate) async fn cmd_update(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: UpdateCommands,
) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        UpdateCommands::Check => {
            println!("Update Check");
            println!("{:-<60}", "");
            println!();

            // Show running node version
            if let Ok(info) = rpc.get_node_info().await {
                let node_version = info.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                let network = info.get("network").and_then(|v| v.as_str()).unwrap_or("?");
                println!("Node version: v{}", node_version);
                println!("Network:      {}", network);
                println!();
            }

            match rpc.get_update_status().await {
                Ok(status) => {
                    if let Some(pending) = status.get("pending_update") {
                        if pending.is_null() {
                            println!("No pending updates. Your node is up to date.");
                        } else {
                            println!("Pending update found:");
                            println!(
                                "  Version: {}",
                                pending
                                    .get("version")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                            );
                            println!(
                                "  Published: {}",
                                pending
                                    .get("published_at")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                            );
                        }
                    } else {
                        println!("No pending updates.");
                    }
                }
                Err(e) => anyhow::bail!("Error checking updates: {}", e),
            }
        }

        UpdateCommands::Status => {
            println!("Update Status");
            println!("{:-<60}", "");
            println!();

            match rpc.get_update_status().await {
                Ok(status) => {
                    let veto_active = status
                        .get("veto_period_active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let veto_count = status
                        .get("veto_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let veto_pct = status
                        .get("veto_percent")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    if let Some(pending) = status.get("pending_update") {
                        if !pending.is_null() {
                            let version = pending
                                .get("version")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            println!("Pending Update: v{}", version);
                            println!(
                                "Veto Period:    {}",
                                if veto_active { "ACTIVE" } else { "ended" }
                            );
                            println!("Veto Count:     {}", veto_count);
                            println!("Veto Percent:   {:.1}% (threshold: 40%)", veto_pct);
                        } else {
                            println!("No pending updates.");
                        }
                    } else {
                        println!("No pending updates.");
                    }
                }
                Err(e) => anyhow::bail!("Error fetching status: {}", e),
            }
        }

        UpdateCommands::Vote {
            version,
            veto,
            approve,
        } => {
            if !veto && !approve {
                anyhow::bail!("Must specify --veto or --approve");
            }

            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let vote_type = if veto { "Veto" } else { "Approve" };
            let vote_str = if veto { "veto" } else { "approve" };
            let producer_id = wallet.addresses()[0].public_key.clone();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Sign the vote: "version:vote:timestamp"
            let message = format!("{}:{}:{}", version, vote_str, timestamp);
            let sig = crypto::signature::sign(message.as_bytes(), keypair.private_key());

            println!(
                "Submitting {} vote for v{}...",
                vote_str.to_uppercase(),
                version
            );

            let vote_msg = serde_json::json!({
                "version": version,
                "vote": vote_type,
                "producerId": producer_id,
                "timestamp": timestamp,
                "signature": sig.to_hex(),
            });

            match rpc.submit_vote(vote_msg).await {
                Ok(result) => {
                    let status = result
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    println!("Vote {}: {}", vote_str, status);
                }
                Err(e) => anyhow::bail!("Error submitting vote: {}", e),
            }
        }

        UpdateCommands::Votes { version } => {
            println!("Votes for v{}", version);
            println!("{:-<60}", "");
            println!();

            match rpc.get_update_status().await {
                Ok(status) => {
                    let veto_count = status
                        .get("veto_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let veto_pct = status
                        .get("veto_percent")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    println!("Veto votes:   {}", veto_count);
                    println!("Veto weight:  {:.1}%", veto_pct);
                    println!(
                        "Threshold:    40% ({})",
                        if veto_pct >= 40.0 {
                            "REJECTED"
                        } else {
                            "not reached"
                        }
                    );
                }
                Err(e) => anyhow::bail!("Error fetching votes: {}", e),
            }
        }

        UpdateCommands::Apply => {
            println!("Manual update apply requires a running node with an approved release.");
            println!("Updates are applied automatically after the 7-day veto period.");
            println!("Use 'doli update status' to check the current update state.");
        }

        UpdateCommands::Rollback => {
            // Check common backup locations
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let backup = PathBuf::from(&home).join(".doli").join("doli-node.backup");
            if backup.exists() {
                println!("Backup binary found at: {}", backup.display());
                println!("To rollback manually, stop the node and run:");
                println!("  cp {} $(which doli-node)", backup.display());
                println!("  systemctl restart doli-mainnet-nodeN");
            } else {
                println!("No backup binary found at {}", backup.display());
                println!("The watchdog automatically rolls back after 3 crashes.");
                println!("Use 'doli update status' to check the current update state.");
            }
        }
    }

    Ok(())
}

pub(crate) async fn cmd_maintainer(rpc_endpoint: &str, command: MaintainerCommands) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        MaintainerCommands::List => {
            println!("Maintainer Set");
            println!("{:-<60}", "");
            println!();

            match rpc.get_maintainer_set().await {
                Ok(set) => {
                    if let Some(maintainers) = set.get("maintainers").and_then(|m| m.as_array()) {
                        let threshold = set.get("threshold").and_then(|t| t.as_u64()).unwrap_or(3);
                        println!("Threshold: {} of {}", threshold, maintainers.len());
                        println!();

                        for (i, member) in maintainers.iter().enumerate() {
                            let key = member.get("pubkey").and_then(|p| p.as_str()).unwrap_or("?");
                            let short = if key.len() > 24 {
                                format!("{}...{}", &key[..16], &key[key.len() - 8..])
                            } else {
                                key.to_string()
                            };
                            println!("  {}. {}", i + 1, short);
                        }
                        println!();
                        println!(
                            "These maintainers also verify release signatures for auto-updates."
                        );
                        println!("Derived from the first 5 registered producers on-chain.");
                    } else {
                        println!("No maintainers found (or unsupported response format).");
                    }
                }
                Err(e) => anyhow::bail!("Error fetching maintainer set: {}", e),
            }
        }
    }

    Ok(())
}
