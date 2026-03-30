use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::parsers::parse_condition;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_mint(
    wallet_path: &Path,
    rpc_endpoint: &str,
    content: &str,
    condition: Option<String>,
    amount: &str,
    royalty_pct: Option<f64>,
    data: Option<String>,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let minter_pubkey_hash = wallet.primary_pubkey_hash();
    let minter_hash = Hash::from_hex(&minter_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid minter pubkey hash"))?;

    // Parse amount (minimum 1 sat dust for pure NFT — protocol requires non-zero)
    let amount_units = std::cmp::max(
        1u64,
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?,
    );

    // Content bytes: --data overrides content with raw hex-decoded binary data.
    // Without --data, content is interpreted as hex (if 64 hex chars) or URI bytes.
    let content_bytes = if let Some(ref hex_data) = data {
        hex::decode(hex_data).map_err(|_| anyhow::anyhow!("Invalid hex in --data"))?
    } else if content.len() == 64 && content.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(content).unwrap_or_else(|_| content.as_bytes().to_vec())
    } else {
        content.as_bytes().to_vec()
    };

    // Generate a nonce from current timestamp for token_id uniqueness
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_le_bytes()
        .to_vec();
    let token_id = Output::compute_nft_token_id(&minter_hash, &nonce);

    // Default condition: signature of the minter (only minter can transfer)
    let cond = if let Some(cond_str) = &condition {
        parse_condition(cond_str)?
    } else {
        doli_core::Condition::signature(minter_hash)
    };

    // Build NFT output (with optional royalty)
    let royalty_bps = match royalty_pct {
        Some(pct) => {
            if !(0.0..=50.0).contains(&pct) {
                anyhow::bail!("Royalty must be between 0 and 50 percent");
            }
            (pct * 100.0) as u16 // percent to basis points
        }
        None => 0,
    };

    let nft_output = if royalty_bps > 0 {
        Output::nft_with_royalty(
            amount_units,
            minter_hash,
            token_id,
            &content_bytes,
            &cond,
            minter_hash, // creator = minter
            royalty_bps,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    } else {
        Output::nft(amount_units, minter_hash, token_id, &content_bytes, &cond)
            .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    };

    // Calculate fee: base + per-byte for NFT output extra_data
    let fee_units = {
        let extra_bytes: u64 = nft_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let utxos: Vec<_> = rpc
        .get_utxos(&minter_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available for fee");
    }

    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    let required = amount_units + fee_units;
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

    // Build inputs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected_utxos {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build outputs: NFT + change
    let mut outputs = vec![nft_output];
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, minter_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign each input (BIP-143: per-input signing hash)
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let minter_display = crypto::address::encode(&minter_hash, address_prefix())
        .unwrap_or_else(|_| minter_hash.to_hex());

    println!("Minting NFT:");
    println!("  Token ID:  {}", token_id.to_hex());
    println!("  Content:   {}", content);
    println!("  Minter:    {}", minter_display);
    if royalty_bps > 0 {
        println!("  Royalty:   {}% to creator", royalty_bps as f64 / 100.0);
    }
    if amount_units > 0 {
        println!("  Value:     {}", format_balance(amount_units));
    }
    println!("  Fee:       {}", format_balance(fee_units));
    println!("  TX Hash:   {}", tx_hash.to_hex());
    println!("  Size:      {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT minted successfully!");
            println!("TX Hash:  {}", result_hash);
            println!("Token ID: {}", token_id.to_hex());
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Mint failed: {}", e));
        }
    }

    Ok(())
}
