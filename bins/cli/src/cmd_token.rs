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
    use doli_core::Output;

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

    let cond = if let Some(cond_str) = &condition {
        parse_condition(cond_str)?
    } else {
        doli_core::Condition::signature(issuer_hash)
    };

    let sizing_output =
        Output::fungible_asset(supply, issuer_hash, Hash::ZERO, supply, ticker, &cond)
            .map_err(|e| anyhow::anyhow!("Failed to create token output: {}", e))?;
    let fee_units = doli_core::consensus::BASE_FEE
        + sizing_output.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE
            / doli_core::consensus::FEE_DIVISOR;

    let utxos: Vec<_> = rpc
        .get_utxos(&issuer_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available for fee");
    }

    let mut selected: Vec<(Hash, u32, u64)> = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= fee_units {
            break;
        }
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO hash"))?;
        selected.push((prev_tx_hash, utxo.output_index, utxo.amount));
        total_input += utxo.amount;
    }

    if total_input < fee_units {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {}",
            format_balance(total_input),
            format_balance(fee_units)
        );
    }

    let mut tx = build_issue_tx(&selected, issuer_hash, ticker, supply, &cond, fee_units)?;
    let asset_id = Output::compute_asset_id(&selected[0].0, selected[0].1);

    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
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
    println!("  Asset ID:    {}", asset_id.to_hex());
    println!("  Fee:         {}", format_balance(fee_units));
    println!("  TX Hash:     {}", tx_hash.to_hex());
    println!("  Size:        {} bytes", tx_bytes.len());
    println!();
    println!("Broadcasting transaction...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(_) => {
            println!("Token issued successfully!");
            println!("TX Hash: {}", tx_hash.to_hex());
            println!("Asset ID: {}", asset_id.to_hex());
            println!("Query with: doli token-info {}:0", tx_hash.to_hex());
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            anyhow::bail!("Token issuance failed: {}", e);
        }
    }

    Ok(())
}

pub(crate) fn build_issue_tx(
    selected: &[(crypto::Hash, u32, u64)],
    issuer_hash: crypto::Hash,
    ticker: &str,
    supply: u64,
    cond: &doli_core::Condition,
    fee: u64,
) -> Result<doli_core::Transaction> {
    use doli_core::{Input, Output, Transaction};

    let (anchor_hash, anchor_index, _) = selected
        .first()
        .ok_or_else(|| anyhow::anyhow!("No inputs selected"))?;
    let total_input = selected
        .iter()
        .try_fold(0u64, |acc, (_, _, amt)| acc.checked_add(*amt))
        .ok_or_else(|| anyhow::anyhow!("Input amount overflow"))?;
    if total_input < fee {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {}",
            format_balance(total_input),
            format_balance(fee)
        );
    }

    let asset_id = Output::compute_asset_id(anchor_hash, *anchor_index);
    let token_output = Output::fungible_asset(supply, issuer_hash, asset_id, supply, ticker, cond)
        .map_err(|e| anyhow::anyhow!("Failed to create token output: {}", e))?;

    let inputs: Vec<Input> = selected
        .iter()
        .map(|(h, idx, _)| Input::new(*h, *idx))
        .collect();
    let mut outputs = vec![token_output];
    let change = total_input - fee;
    if change > 0 {
        outputs.push(Output::normal(change, issuer_hash));
    }
    Ok(Transaction::new_transfer(inputs, outputs))
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

#[cfg(test)]
#[path = "cmd_token_tests.rs"]
mod tests;
