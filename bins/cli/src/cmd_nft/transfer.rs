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

    // Get the NFT UTXO details via RPC to extract token_id and content
    let tx_info = rpc.get_transaction_json(&prev_tx_hash.to_hex()).await?;
    let nft_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], output_index))?;

    let nft_meta = nft_output
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

    // Parse recipient
    let recipient_hash = crypto::address::resolve(to, None)
        .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

    // The NFT output carries forward the same token_id and content, new owner
    // Use a simple signature condition for the new owner
    let new_cond = doli_core::Condition::signature(recipient_hash);
    let nft_amount = nft_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // Extract royalty info to preserve it on transfer
    let royalty = nft_meta.get("royalty").and_then(|r| {
        let creator = r.get("creator")?.as_str()?;
        let bps = r.get("bps")?.as_u64()?;
        let creator_hash = Hash::from_hex(creator)?;
        Some((creator_hash, bps as u16))
    });

    let new_nft_output = if let Some((creator_hash, royalty_bps)) = royalty {
        Output::nft_with_royalty(
            nft_amount,
            recipient_hash,
            token_id,
            &content_bytes,
            &new_cond,
            creator_hash,
            royalty_bps,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    } else {
        Output::nft(
            nft_amount,
            recipient_hash,
            token_id,
            &content_bytes,
            &new_cond,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    };

    // Calculate fee: base + per-byte for NFT output extra_data
    let sender_pubkey_hash = wallet.primary_pubkey_hash();
    let fee_units = {
        let extra_bytes: u64 = new_nft_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
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

    // Build inputs: NFT input first, then fee-paying UTXOs
    let nft_input = Input::new(prev_tx_hash, output_index);
    let mut inputs = vec![nft_input];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Build outputs: NFT + change
    let mut outputs = vec![new_nft_output];
    let change = total_fee_input - fee_units;
    if change > 0 {
        let sender_hash = Hash::from_hex(&sender_pubkey_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid sender pubkey hash"))?;
        outputs.push(Output::normal(change, sender_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign: BIP-143 per-input signing hash
    let keypair = wallet.primary_keypair()?;
    let signing_hash_0 = tx.signing_message_for_input(0);

    // Auto-provide signature witness for NFT covenant (input 0)
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
    // Covenant witnesses: one per input (NFT has witness, fee inputs have empty)
    let mut witnesses: Vec<Vec<u8>> = vec![witness_bytes];
    for _ in &selected_utxos {
        witnesses.push(Vec::new());
    }
    tx.set_covenant_witnesses(&witnesses);

    // Sign all inputs with per-input signing hash
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let recipient_display = crypto::address::encode(&recipient_hash, address_prefix())
        .unwrap_or_else(|_| recipient_hash.to_hex());

    println!("Transferring NFT:");
    println!("  Token ID: {}", token_id.to_hex());
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
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT transferred successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("NFT transfer failed: {}", e));
        }
    }

    Ok(())
}
