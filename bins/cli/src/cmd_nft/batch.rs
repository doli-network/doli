use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

/// Entry in the batch mint manifest
#[derive(serde::Deserialize)]
struct BatchEntry {
    /// Text content or description
    content: String,
    /// Optional hex-encoded binary data (image, etc.)
    data: Option<String>,
    /// Optional royalty percentage (e.g. 5.0 = 5%)
    royalty: Option<f64>,
}

pub(crate) async fn cmd_nft_batch_mint(
    wallet_path: &Path,
    rpc_endpoint: &str,
    manifest_path: &str,
    yes: bool,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Read manifest
    let manifest_data = std::fs::read_to_string(manifest_path)
        .map_err(|e| anyhow::anyhow!("Cannot read manifest: {}", e))?;
    let entries: Vec<BatchEntry> = serde_json::from_str(&manifest_data)
        .map_err(|e| anyhow::anyhow!("Invalid manifest JSON: {}", e))?;

    if entries.is_empty() {
        anyhow::bail!("Manifest is empty");
    }

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build NFT outputs
    let cond = doli_core::Condition::signature(from_hash);
    let mut nft_outputs = Vec::new();
    let mut total_extra_bytes = 0u64;

    for (i, entry) in entries.iter().enumerate() {
        // Determine content bytes
        let content_bytes = if let Some(hex_data) = &entry.data {
            hex::decode(hex_data).map_err(|_| anyhow::anyhow!("Invalid hex in entry {}", i))?
        } else {
            entry.content.as_bytes().to_vec()
        };

        // Compute unique token_id
        let nonce = format!("{}:{}", i, entry.content);
        let token_id = Output::compute_nft_token_id(&from_hash, nonce.as_bytes());

        // Build NFT output
        let nft_output = if let Some(royalty_pct) = entry.royalty {
            let royalty_bps = (royalty_pct * 100.0) as u16;
            Output::nft_with_royalty(
                1,
                from_hash,
                token_id,
                &content_bytes,
                &cond,
                from_hash,
                royalty_bps,
            )
            .map_err(|e| anyhow::anyhow!("NFT {} build failed: {}", i, e))?
        } else {
            Output::nft(1, from_hash, token_id, &content_bytes, &cond)
                .map_err(|e| anyhow::anyhow!("NFT {} build failed: {}", i, e))?
        };

        total_extra_bytes += nft_output.extra_data.len() as u64;
        nft_outputs.push(nft_output);
    }

    // Calculate fee + dust for each NFT (1 sat per NFT)
    let nft_dust = entries.len() as u64;
    let fee_units =
        doli_core::consensus::BASE_FEE + total_extra_bytes * doli_core::consensus::FEE_PER_BYTE;
    let total_needed = fee_units + nft_dust;

    // Select DOLI UTXOs for fee + dust
    let utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= total_needed {
            break;
        }
        selected.push(utxo.clone());
        total_input += utxo.amount;
    }
    if total_input < total_needed {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {} (fee {} + dust {})",
            format_balance(total_input),
            format_balance(total_needed),
            format_balance(fee_units),
            format_balance(nft_dust)
        );
    }

    // Build inputs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build outputs: all NFTs + DOLI change
    let mut outputs = nft_outputs;
    let change = total_input - total_needed;
    if change > 0 {
        outputs.push(Output::normal(change, from_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Batch Mint NFTs:");
    println!("  NFTs:     {}", entries.len());
    println!(
        "  Fee:      {} ({} bytes x 1 sat/byte + base)",
        format_balance(fee_units),
        total_extra_bytes
    );
    println!("  TX Hash:  {}", tx_hash.to_hex());
    println!("  Size:     {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Batch mint successful!");
            println!("TX Hash: {}", result_hash);
            println!("{} NFTs minted in a single transaction.", entries.len());
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Batch mint failed: {}", e));
        }
    }

    Ok(())
}
