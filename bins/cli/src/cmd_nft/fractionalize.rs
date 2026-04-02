use std::path::Path;

use anyhow::Result;
use crypto::Hash;
use doli_core::{Input, Output, Transaction, TxType};

use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

/// Fractionalize an NFT into fungible shares.
///
/// Finds the NFT UTXO by token_id, builds a FractionalizeNft transaction
/// that produces a fractionalized NFT output + a FungibleAsset output with
/// the requested number of shares.
pub(crate) async fn cmd_nft_fractionalize(
    wallet_path: &Path,
    rpc_endpoint: &str,
    token_id_hex: &str,
    shares: u64,
    ticker: Option<&str>,
) -> Result<()> {
    if shares == 0 {
        anyhow::bail!("--shares must be greater than 0");
    }

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let token_id =
        Hash::from_hex(token_id_hex).ok_or_else(|| anyhow::anyhow!("Invalid token_id hex"))?;
    let ticker = ticker.unwrap_or("FRAC");

    // Find the NFT UTXO owned by this wallet with matching token_id
    let sender_pubkey_hash = wallet.primary_pubkey_hash();
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
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No NFT with token_id {} found in wallet",
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

    // Get NFT content from RPC transaction data
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

    // Check if already fractionalized
    if nft_meta
        .get("fractionalized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        anyhow::bail!("NFT is already fractionalized");
    }

    let frac_asset_id = Output::fraction_asset_id(&token_id);
    let owner_hash = Hash::from_hex(&sender_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid sender pubkey hash"))?;

    // Build fractionalized NFT output (Output 0)
    let base_cond = doli_core::Condition::signature(owner_hash);
    let base_nft = Output::nft(nft_amount, owner_hash, token_id, &content_bytes, &base_cond)
        .map_err(|e| anyhow::anyhow!("Failed to create base NFT output: {}", e))?;
    let frac_extra =
        Output::build_fractionalized_extra_data(&base_nft.extra_data, &frac_asset_id, shares);
    let frac_nft_output = Output {
        extra_data: frac_extra,
        ..base_nft
    };

    // Build fraction token output (Output 1)
    let frac_cond = doli_core::Condition::signature(owner_hash);
    let frac_token_output = Output::fungible_asset(
        shares,
        owner_hash,
        frac_asset_id,
        shares,
        ticker,
        &frac_cond,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create fraction token output: {}", e))?;

    // Calculate fee
    let extra_bytes: u64 =
        frac_nft_output.extra_data.len() as u64 + frac_token_output.extra_data.len() as u64;
    let fee_units = doli_core::consensus::BASE_FEE
        + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR;

    // Select fee-paying UTXOs
    let fee_utxos: Vec<_> = rpc
        .get_utxos(&sender_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    if fee_utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available for fee");
    }

    let mut selected_utxos = Vec::new();
    let mut total_fee_input = 0u64;
    for utxo in &fee_utxos {
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

    // Build inputs: NFT first, then fee UTXOs
    let nft_input = Input::new(prev_tx_hash, nft_output_index);
    let mut inputs = vec![nft_input];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Build outputs: fractionalized NFT + fraction tokens + change
    let mut outputs = vec![frac_nft_output, frac_token_output];
    let change = total_fee_input - fee_units;
    if change > 0 {
        outputs.push(Output::normal(change, owner_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign: covenant witness for NFT input (input 0)
    let keypair = wallet.primary_keypair()?;
    let signing_hash_0 = tx.signing_message_for_input(0);
    let mut w = doli_core::Witness::default();
    w.signatures.push(doli_core::ConditionWitnessSignature {
        pubkey: *keypair.public_key(),
        signature: crypto::signature::sign_hash(&signing_hash_0, keypair.private_key()),
    });
    let witness_bytes = w.encode();

    let mut witnesses: Vec<Vec<u8>> = vec![witness_bytes];
    for _ in &selected_utxos {
        witnesses.push(Vec::new());
    }
    tx.set_covenant_witnesses(&witnesses);

    // Sign all inputs with per-input signing hash
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Fractionalizing NFT:");
    println!("  Token ID: {}", token_id.to_hex());
    println!("  Shares:   {}", shares);
    println!("  Ticker:   {}", ticker);
    println!(
        "  From:     {}:{}",
        &prev_tx_hash.to_hex()[..16],
        nft_output_index
    );
    println!("  Fee:      {}", format_balance(fee_units));
    println!("  TX Hash:  {}", tx_hash.to_hex());
    println!("  Size:     {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT fractionalized successfully!");
            println!("TX Hash: {}", result_hash);
            println!("Fraction asset ID: {}", frac_asset_id.to_hex());
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Fractionalization failed: {}", e));
        }
    }

    Ok(())
}
