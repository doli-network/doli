use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_mint(
    wallet_path: &Path,
    rpc_endpoint: &str,
    content: &str,
    _condition: Option<String>,
    amount: &str,
    royalty_pct: Option<f64>,
    content_type: Option<String>,
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

    // Parse amount (minimum 1 sat dust — protocol requires non-zero)
    let amount_units = std::cmp::max(
        1u64,
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?,
    );

    // Content bytes: --data overrides content with raw hex-decoded binary data.
    // Without --data, content is interpreted as hex (if 64 hex chars) or URI/path bytes.
    let content_bytes = if let Some(ref hex_data) = data {
        hex::decode(hex_data).map_err(|_| anyhow::anyhow!("Invalid hex in --data"))?
    } else if std::path::Path::new(content).exists() {
        // If content looks like a file path, read the file
        std::fs::read(content)?
    } else if content.len() == 64 && content.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(content).unwrap_or_else(|_| content.as_bytes().to_vec())
    } else {
        content.as_bytes().to_vec()
    };

    if content_bytes.is_empty() {
        anyhow::bail!("Content is empty");
    }

    // Encrypt content with AES-256-GCM + ECIES key wrapping
    let keypair = wallet.primary_keypair()?;
    let content_key = crypto::encrypted_content::generate_content_key();
    let (ciphertext, nonce) =
        crypto::encrypted_content::encrypt_content(&content_key, &content_bytes)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;
    let wrapped_key = crypto::encrypted_content::wrap_key(&content_key, keypair.public_key())
        .map_err(|e| anyhow::anyhow!("Key wrapping failed: {}", e))?;
    let c_hash = crypto::encrypted_content::content_hash(&content_bytes);

    // Determine MIME type: explicit --content-type, auto-detect from file extension, or none
    let mime_type: Option<String> = content_type.or_else(|| {
        let path = std::path::Path::new(content);
        path.extension()
            .and_then(|ext| match ext.to_str()?.to_lowercase().as_str() {
                "jpg" | "jpeg" => Some("image/jpeg".to_string()),
                "png" => Some("image/png".to_string()),
                "gif" => Some("image/gif".to_string()),
                "webp" => Some("image/webp".to_string()),
                "svg" => Some("image/svg+xml".to_string()),
                "bmp" => Some("image/bmp".to_string()),
                "pdf" => Some("application/pdf".to_string()),
                "txt" => Some("text/plain".to_string()),
                "json" => Some("application/json".to_string()),
                "mp3" => Some("audio/mpeg".to_string()),
                "mp4" => Some("video/mp4".to_string()),
                "wav" => Some("audio/wav".to_string()),
                "zip" => Some("application/zip".to_string()),
                "html" => Some("text/html".to_string()),
                _ => None,
            })
    });

    // Convert royalty percentage to basis points (5.0% -> 500 bps)
    let royalty_bps: u16 = if let Some(pct) = royalty_pct {
        if !(0.0..=50.0).contains(&pct) {
            anyhow::bail!("Royalty must be between 0% and 50%, got {}%", pct);
        }
        (pct * 100.0) as u16
    } else {
        0
    };

    // Build EncryptedContent output (v1 if MIME or royalty specified, v0 otherwise)
    let use_v1 = mime_type.is_some() || royalty_bps > 0;
    let ec_output = if use_v1 {
        let mime_bytes = mime_type.as_deref().unwrap_or("").as_bytes();
        Output::encrypted_content_v1(
            amount_units,
            minter_hash,
            &ciphertext,
            &wrapped_key,
            &nonce,
            &c_hash,
            mime_bytes,
            minter_hash, // creator = minter
            royalty_bps,
        )
    } else {
        Output::encrypted_content(
            amount_units,
            minter_hash,
            &ciphertext,
            &wrapped_key,
            &nonce,
            &c_hash,
        )
    };

    // Calculate fee: base + per-byte for extra_data
    let fee_units = {
        let extra_bytes: u64 = ec_output.extra_data.len() as u64;
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

    // Build outputs: EncryptedContent + change
    let mut outputs = vec![ec_output];
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, minter_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign each input
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let minter_display = crypto::address::encode(&minter_hash, address_prefix())
        .unwrap_or_else(|_| minter_hash.to_hex());

    println!(
        "Minting encrypted content{}:",
        if use_v1 { " (v1)" } else { "" }
    );
    println!("  Content hash: {}", hex::encode(c_hash));
    println!("  Plaintext:    {} bytes", content_bytes.len());
    println!("  Ciphertext:   {} bytes", ciphertext.len());
    if let Some(ref mime) = mime_type {
        println!("  Content type: {}", mime);
    }
    println!("  Owner:        {}", minter_display);
    if royalty_bps > 0 {
        println!("  Royalty:      {}% to creator", royalty_bps as f64 / 100.0);
    }
    if amount_units > 1 {
        println!("  Value:        {}", format_balance(amount_units));
    }
    println!("  Fee:          {}", format_balance(fee_units));
    println!("  TX Hash:      {}", tx_hash.to_hex());
    println!("  Size:         {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Content minted successfully!");
            println!("TX Hash:      {}", result_hash);
            println!("Content Hash: {}", hex::encode(c_hash));
            println!();
            println!("Only you can decrypt this content with your private key.");
            println!(
                "Use `doli nft --export {}:0` to decrypt and save.",
                result_hash
            );
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Mint failed: {}", e));
        }
    }

    Ok(())
}
