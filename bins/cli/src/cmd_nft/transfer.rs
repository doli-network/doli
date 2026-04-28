use std::path::Path;

use anyhow::Result;
use crypto::Hash;
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::parsers::parse_witness;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_nft_transfer(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    to: &str,
    witness_str: &str,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse UTXO reference
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let prev_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash: {}", parts[0]))?;
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index: {}", parts[1]))?;

    // Get the UTXO details via RPC
    let tx_info = rpc.get_transaction_json(&prev_tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], output_index))?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Branch: EncryptedContent transfer (re-wrap key) vs legacy NFT transfer
    let (new_output, recipient_hash, is_encrypted) = if output_type == "encryptedContent" {
        // EncryptedContent transfer requires recipient's PUBLIC KEY (not just hash)
        // because ECIES re-wrap needs the actual key. Accept --to as:
        //   - 64 hex chars → raw pubkey
        //   - bech32 address → error (need pubkey for re-wrap)
        let recipient_pubkey = if to.len() == 64 && to.chars().all(|c| c.is_ascii_hexdigit()) {
            let pubkey_bytes: [u8; 32] = hex::decode(to)
                .map_err(|_| anyhow::anyhow!("Invalid pubkey hex"))?
                .try_into()
                .map_err(|_| anyhow::anyhow!("Pubkey must be 32 bytes"))?;
            crypto::PublicKey::from_bytes(pubkey_bytes)
        } else {
            anyhow::bail!(
                "EncryptedContent transfer requires recipient's public key (64 hex chars).\n\
                 The recipient can find theirs with: doli info\n\
                 Usage: doli nft --transfer <utxo> --to <recipient_pubkey_hex>"
            );
        };

        let recipient_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, recipient_pubkey.as_bytes());

        // Parse extra_data
        let extra_data_hex = utxo_output
            .get("encryptedContent")
            .and_then(|ec| ec.get("extraData"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing encryptedContent.extraData in RPC response"))?;
        let extra_data =
            hex::decode(extra_data_hex).map_err(|_| anyhow::anyhow!("Invalid hex in extraData"))?;

        if extra_data.len() < 128 {
            anyhow::bail!("Malformed EncryptedContent extra_data");
        }

        let ct_len = u32::from_le_bytes(extra_data[0..4].try_into()?) as usize;
        let offset = 4 + ct_len;
        if extra_data.len() < offset + 80 + 12 + 32 {
            anyhow::bail!("Truncated EncryptedContent extra_data");
        }

        let ciphertext = &extra_data[4..4 + ct_len];
        let mut wrapped_key = [0u8; 80];
        wrapped_key.copy_from_slice(&extra_data[offset..offset + 80]);
        let nonce: [u8; 12] = extra_data[offset + 80..offset + 92].try_into()?;
        let mut content_hash = [0u8; 32];
        content_hash.copy_from_slice(&extra_data[offset + 92..offset + 124]);

        // Unwrap the content key with sender's private key
        let keypair = wallet.primary_keypair()?;
        let content_key =
            crypto::encrypted_content::unwrap_key(&wrapped_key, keypair.private_key())
                .map_err(|_| anyhow::anyhow!("Failed to unwrap key — you are not the owner"))?;

        // Re-wrap with recipient's public key
        let new_wrapped_key = crypto::encrypted_content::wrap_key(&content_key, &recipient_pubkey)
            .map_err(|e| anyhow::anyhow!("Re-wrap failed: {}", e))?;

        // Build new EncryptedContent output with same content, new wrapped_key
        let output = Output::encrypted_content(
            amount,
            recipient_hash,
            ciphertext,
            &new_wrapped_key,
            &nonce,
            &content_hash,
        );

        (output, recipient_hash, true)
    } else if output_type == "nft" {
        // Legacy NFT transfer path
        let recipient_hash = crypto::address::resolve(to, None)
            .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

        let nft_meta = utxo_output
            .get("nft")
            .ok_or_else(|| anyhow::anyhow!("Output is not an NFT"))?;
        let token_id_hex = nft_meta
            .get("tokenId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing tokenId in NFT metadata"))?;
        let content_hash_hex = nft_meta
            .get("contentHash")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let token_id =
            Hash::from_hex(token_id_hex).ok_or_else(|| anyhow::anyhow!("Invalid token_id"))?;
        let content_bytes = hex::decode(content_hash_hex).unwrap_or_default();

        let new_cond = doli_core::Condition::signature(recipient_hash);

        let royalty = nft_meta.get("royalty").and_then(|r| {
            let creator = r.get("creator")?.as_str()?;
            let bps = r.get("bps")?.as_u64()?;
            let creator_hash = Hash::from_hex(creator)?;
            Some((creator_hash, bps as u16))
        });

        let output = if let Some((creator_hash, royalty_bps)) = royalty {
            Output::nft_with_royalty(
                amount,
                recipient_hash,
                token_id,
                &content_bytes,
                &new_cond,
                creator_hash,
                royalty_bps,
            )
            .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
        } else {
            Output::nft(amount, recipient_hash, token_id, &content_bytes, &new_cond)
                .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
        };

        (output, recipient_hash, false)
    } else {
        anyhow::bail!(
            "Output is not an NFT or EncryptedContent (type: {})",
            output_type
        );
    };

    // Calculate fee
    let sender_pubkey_hash = wallet.primary_pubkey_hash();
    let fee_units = {
        let extra_bytes: u64 = new_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let utxos: Vec<_> = rpc
        .get_utxos(&sender_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available for fee");
    }

    let mut selected_utxos = Vec::new();
    let mut total_fee_input = 0u64;
    for utxo in &utxos {
        if total_fee_input >= fee_units {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_fee_input += utxo.amount;
    }
    if total_fee_input < fee_units {
        anyhow::bail!(
            "Insufficient balance for fee. Available: {}, Required: {}",
            format_balance(total_fee_input),
            format_balance(fee_units)
        );
    }

    // Build inputs: content input first, then fee-paying UTXOs
    let content_input = Input::new(prev_tx_hash, output_index);
    let mut inputs = vec![content_input];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Build outputs: content + change
    let mut outputs = vec![new_output];
    let change = total_fee_input - fee_units;
    if change > 0 {
        let sender_hash = Hash::from_hex(&sender_pubkey_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid sender pubkey hash"))?;
        outputs.push(Output::normal(change, sender_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign: BIP-143 per-input signing hash
    let keypair = wallet.primary_keypair()?;

    // For legacy NFTs with covenants: provide witness for input 0
    if !is_encrypted {
        let signing_hash_0 = tx.signing_message_for_input(0);
        let witness_bytes = if witness_str == "none()" {
            let mut w = doli_core::Witness::default();
            w.signatures.push(doli_core::ConditionWitnessSignature {
                pubkey: *keypair.public_key(),
                signature: crypto::signature::sign_hash(&signing_hash_0, keypair.private_key()),
            });
            w.encode()
        } else {
            parse_witness(witness_str, &signing_hash_0)?
        };
        let mut witnesses: Vec<Vec<u8>> = vec![witness_bytes];
        for _ in &selected_utxos {
            witnesses.push(Vec::new());
        }
        tx.set_covenant_witnesses(&witnesses);
    }

    // Sign all inputs
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let recipient_display = crypto::address::encode(&recipient_hash, address_prefix())
        .unwrap_or_else(|_| recipient_hash.to_hex());

    if is_encrypted {
        println!("Transferring encrypted content:");
        println!(
            "  From:     {}:{}",
            &prev_tx_hash.to_hex()[..16],
            output_index
        );
        println!("  To:       {}", recipient_display);
        println!("  Fee:      {}", format_balance(fee_units));
        println!("  TX Hash:  {}", tx_hash.to_hex());
        println!("  Size:     {} bytes", tx_bytes.len());
        println!();
        println!("Key re-wrapped for recipient. Only they can decrypt after transfer.");
    } else {
        println!("Transferring NFT:");
        println!(
            "  From:     {}:{}",
            &prev_tx_hash.to_hex()[..16],
            output_index
        );
        println!("  To:       {}", recipient_display);
        println!("  Fee:      {}", format_balance(fee_units));
        println!("  TX Hash:  {}", tx_hash.to_hex());
        println!("  Size:     {} bytes", tx_bytes.len());
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Transfer successful!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Transfer failed: {}", e));
        }
    }

    Ok(())
}
