use std::path::Path;

use anyhow::Result;
use crypto::Hash;
use doli_core::{Input, Output, Transaction, TxType};

use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

/// Redeem a fractionalized NFT back to a whole NFT.
///
/// Finds the fractionalized NFT by token_id, collects all fraction token UTXOs,
/// verifies the full supply is held, and builds a RedeemNft transaction.
pub(crate) async fn cmd_nft_redeem(
    wallet_path: &Path,
    rpc_endpoint: &str,
    token_id_hex: &str,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let token_id =
        Hash::from_hex(token_id_hex).ok_or_else(|| anyhow::anyhow!("Invalid token_id hex"))?;
    let frac_asset_id = Output::fraction_asset_id(&token_id);
    let frac_asset_id_hex = frac_asset_id.to_hex();

    let sender_pubkey_hash = wallet.primary_pubkey_hash();

    // Find the fractionalized NFT UTXO
    let response = rpc.get_utxos_json(&sender_pubkey_hash, true).await?;
    let utxos = response
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid UTXO response"))?;

    let nft_utxo = utxos
        .iter()
        .find(|u| {
            u.get("outputType").and_then(|v| v.as_str()) == Some("nft")
                && u.get("nft")
                    .and_then(|n| n.get("tokenId"))
                    .and_then(|v| v.as_str())
                    == Some(token_id_hex)
                && u.get("nft")
                    .and_then(|n| n.get("fractionalized"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No fractionalized NFT with token_id {} found in wallet",
                &token_id_hex[..std::cmp::min(16, token_id_hex.len())]
            )
        })?;

    let nft_tx_hash_hex = nft_utxo
        .get("txHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing txHash in NFT UTXO"))?;
    let nft_output_index = nft_utxo
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let nft_amount = nft_utxo.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
    let prev_tx_hash =
        Hash::from_hex(nft_tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid NFT tx hash"))?;

    // Get NFT content/metadata from RPC
    let tx_info = rpc.get_transaction_json(&prev_tx_hash.to_hex()).await?;
    let nft_output_json = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(nft_output_index as usize))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot find output {}:{}",
                nft_tx_hash_hex,
                nft_output_index
            )
        })?;

    let nft_meta = nft_output_json
        .get("nft")
        .ok_or_else(|| anyhow::anyhow!("Output is not an NFT"))?;
    let content_hash_hex = nft_meta
        .get("contentHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let content_bytes = hex::decode(content_hash_hex).unwrap_or_default();

    // Get fractionalization metadata (total_shares)
    let total_shares = nft_meta
        .get("fractionShares")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot determine total_shares for fractionalized NFT"))?;

    // Find all FungibleAsset UTXOs with matching fraction_asset_id
    let all_utxos = rpc.get_utxos(&sender_pubkey_hash, true).await?;
    let fraction_utxos: Vec<_> = all_utxos
        .iter()
        .filter(|u| {
            u.output_type == "fungibleAsset"
                && u.spendable
                && u.asset_id() == Some(&frac_asset_id_hex)
        })
        .collect();

    let fraction_total: u64 = fraction_utxos.iter().map(|u| u.amount).sum();
    if fraction_total != total_shares {
        anyhow::bail!(
            "Insufficient fraction tokens. Have: {}, Need: {} (all shares required for redemption)",
            fraction_total,
            total_shares
        );
    }

    let owner_hash = Hash::from_hex(&sender_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid sender pubkey hash"))?;

    // Build unlocked NFT output (Output 0) — strip fractionalization metadata
    let new_cond = doli_core::Condition::signature(owner_hash);
    let new_nft_output = Output::nft(nft_amount, owner_hash, token_id, &content_bytes, &new_cond)
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?;

    // Calculate fee
    let extra_bytes: u64 = new_nft_output.extra_data.len() as u64;
    let fee_units = doli_core::consensus::BASE_FEE
        + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR;

    // Select fee-paying UTXOs (normal only)
    let fee_utxos: Vec<_> = all_utxos
        .iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected_fee_utxos = Vec::new();
    let mut total_fee_input = 0u64;
    for utxo in &fee_utxos {
        if total_fee_input >= fee_units {
            break;
        }
        selected_fee_utxos.push((*utxo).clone());
        total_fee_input += utxo.amount;
    }
    if total_fee_input < fee_units {
        anyhow::bail!(
            "Insufficient balance for fee. Available: {}, Required: {}",
            format_balance(total_fee_input),
            format_balance(fee_units)
        );
    }

    // Build inputs: NFT first, then fraction tokens, then fee UTXOs
    let nft_input = Input::new(prev_tx_hash, nft_output_index);
    let mut inputs = vec![nft_input];

    for utxo in &fraction_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    for utxo in &selected_fee_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Build outputs: unlocked NFT + change (no fraction tokens in outputs!)
    let mut outputs = vec![new_nft_output];
    let change = total_fee_input - fee_units;
    if change > 0 {
        outputs.push(Output::normal(change, owner_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Covenant witnesses: NFT input needs witness, fraction + fee inputs get empty
    let keypair = wallet.primary_keypair()?;
    let signing_hash_0 = tx.signing_message_for_input(0);
    let mut w = doli_core::Witness::default();
    w.signatures.push(doli_core::ConditionWitnessSignature {
        pubkey: *keypair.public_key(),
        signature: crypto::signature::sign_hash(&signing_hash_0, keypair.private_key()),
    });
    let nft_witness = w.encode();

    let mut witnesses: Vec<Vec<u8>> = vec![nft_witness];
    // Fraction token inputs may also have conditions — provide witnesses
    for i in 0..fraction_utxos.len() {
        let input_idx = i + 1;
        let signing_hash = tx.signing_message_for_input(input_idx);
        let mut fw = doli_core::Witness::default();
        fw.signatures.push(doli_core::ConditionWitnessSignature {
            pubkey: *keypair.public_key(),
            signature: crypto::signature::sign_hash(&signing_hash, keypair.private_key()),
        });
        witnesses.push(fw.encode());
    }
    // Fee inputs get empty witnesses
    for _ in &selected_fee_utxos {
        witnesses.push(Vec::new());
    }
    tx.set_covenant_witnesses(&witnesses);

    // Sign all inputs
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Redeeming fractionalized NFT:");
    println!("  Token ID:      {}", token_id.to_hex());
    println!("  Total Shares:  {}", total_shares);
    println!("  Fraction UTXOs: {}", fraction_utxos.len());
    println!(
        "  From:          {}:{}",
        &prev_tx_hash.to_hex()[..16],
        nft_output_index
    );
    println!("  Fee:           {}", format_balance(fee_units));
    println!("  TX Hash:       {}", tx_hash.to_hex());
    println!("  Size:          {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT redeemed successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Redemption failed: {}", e));
        }
    }

    Ok(())
}
