use std::path::Path;

use anyhow::Result;

use crate::common::address_prefix;
use crate::parsers::parse_condition;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_issue_token(
    wallet_path: &Path,
    rpc_endpoint: &str,
    ticker: &str,
    supply: u64,
    condition: Option<String>,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    if ticker.is_empty() || ticker.len() > 16 {
        anyhow::bail!("Ticker must be 1-16 characters");
    }

    if supply == 0 {
        anyhow::bail!("Supply must be > 0");
    }

    let issuer_pubkey_hash = wallet.primary_pubkey_hash();
    let issuer_hash = Hash::from_hex(&issuer_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid issuer pubkey hash"))?;

    // asset_id is derived from genesis tx hash + output index after mining.
    // Use a placeholder for construction; the canonical asset_id is computed on-chain.
    let placeholder_asset_id = {
        use crypto::hash::hash_with_domain;
        let mut data = Vec::new();
        data.extend_from_slice(issuer_hash.as_bytes());
        data.extend_from_slice(ticker.as_bytes());
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_le_bytes();
        data.extend_from_slice(&nonce);
        hash_with_domain(b"DOLI_ASSET_PRE", &data)
    };

    // Default condition: signature of the issuer
    let cond = if let Some(cond_str) = &condition {
        parse_condition(cond_str)?
    } else {
        doli_core::Condition::signature(issuer_hash)
    };

    // Token UTXO amount = supply in token base units (not DOLI).
    // FungibleAsset.amount represents token quantity, not native DOLI.
    let token_output = Output::fungible_asset(
        supply,
        issuer_hash,
        placeholder_asset_id,
        supply,
        ticker,
        &cond,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create token output: {}", e))?;

    // Calculate fee: base + per-byte for token output extra_data
    let fee_units = {
        let extra_bytes: u64 = token_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let utxos: Vec<_> = rpc
        .get_utxos(&issuer_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available for fee");
    }

    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    let required = fee_units;
    for utxo in &utxos {
        if total_input >= required {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }

    if total_input < required {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {}",
            format_balance(total_input),
            format_balance(required)
        );
    }

    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected_utxos {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    let mut outputs = vec![token_output];
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, issuer_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let issuer_display = crypto::address::encode(&issuer_hash, address_prefix())
        .unwrap_or_else(|_| issuer_hash.to_hex());

    println!("Issuing Fungible Token:");
    println!("  Ticker:      {}", ticker);
    println!("  Supply:      {}", supply);
    println!("  Issuer:      {}", issuer_display);
    println!("  Fee:         {}", format_balance(fee_units));
    println!("  TX Hash:     {}", tx_hash.to_hex());
    println!("  Size:        {} bytes", tx_bytes.len());
    println!();
    println!("Broadcasting transaction...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(_) => {
            println!("Token issued successfully!");
            println!("TX Hash: {}", tx_hash.to_hex());
            println!();
            println!(
                "Asset ID (canonical): BLAKE3(\"DOLI_ASSET\" || {} || 0)",
                tx_hash.to_hex()
            );
            println!("Query with: doli token-info {}:0", tx_hash.to_hex());
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            anyhow::bail!("Token issuance failed: {}", e);
        }
    }

    Ok(())
}

pub(crate) async fn cmd_token_info(rpc_endpoint: &str, utxo_ref: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }

    let tx_info = rpc.get_transaction_json(parts[0]).await?;
    let output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(parts[1].parse::<usize>().unwrap_or(0)))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], parts[1]))?;

    let output_type = output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if output_type != "fungibleAsset" {
        anyhow::bail!("Output is not a fungible asset (type: {})", output_type);
    }

    let asset = output
        .get("asset")
        .ok_or_else(|| anyhow::anyhow!("Missing asset metadata"))?;

    let owner = output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("Fungible Token Info:");
    println!("{:-<60}", "");
    println!(
        "  Ticker:       {}",
        asset.get("ticker").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  Asset ID:     {}",
        asset.get("assetId").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  Total Supply: {}",
        asset
            .get("totalSupply")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );

    if let Some(owner_hash) = crypto::Hash::from_hex(owner) {
        let addr = crypto::address::encode(&owner_hash, address_prefix())
            .unwrap_or_else(|_| owner.to_string());
        println!("  Owner:        {}", addr);
    } else {
        println!("  Owner:        {}", owner);
    }

    if let Some(cond) = output.get("condition") {
        println!("  Condition:    {}", cond);
    }

    Ok(())
}
