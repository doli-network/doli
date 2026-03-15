use std::path::Path;

use anyhow::Result;

use crate::common::address_prefix;
use crate::parsers::parse_witness;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_bridge_lock(
    wallet_path: &Path,
    rpc_endpoint: &str,
    amount: &str,
    hash_hex: &str,
    lock: u64,
    expiry: u64,
    chain: &str,
    target_address: &str,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::transaction::{
        BRIDGE_CHAIN_BITCOIN, BRIDGE_CHAIN_CARDANO, BRIDGE_CHAIN_ETHEREUM, BRIDGE_CHAIN_LITECOIN,
        BRIDGE_CHAIN_MONERO,
    };
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    if lock >= expiry {
        anyhow::bail!(
            "Lock height ({}) must be less than expiry height ({}). \
             Lock = when claim becomes available, Expiry = when refund becomes available.",
            lock,
            expiry
        );
    }

    let chain_id = match chain.to_lowercase().as_str() {
        "bitcoin" | "btc" => BRIDGE_CHAIN_BITCOIN,
        "ethereum" | "eth" => BRIDGE_CHAIN_ETHEREUM,
        "monero" | "xmr" => BRIDGE_CHAIN_MONERO,
        "litecoin" | "ltc" => BRIDGE_CHAIN_LITECOIN,
        "cardano" | "ada" => BRIDGE_CHAIN_CARDANO,
        _ => anyhow::bail!(
            "Unknown chain: {}. Supported: bitcoin, ethereum, monero, litecoin, cardano",
            chain
        ),
    };

    let expected_hash =
        Hash::from_hex(hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid hash: {}", hash_hex))?;

    let amount_units =
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if amount_units == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build bridge HTLC output
    let bridge_output = Output::bridge_htlc(
        amount_units,
        from_hash,
        expected_hash,
        lock,
        expiry,
        chain_id,
        target_address.as_bytes(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create bridge HTLC: {}", e))?;

    // Get spendable normal UTXOs for funding (exclude bonds, conditioned, etc.)
    let fee_units = 1500u64;
    let required = amount_units + fee_units;
    let utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= required {
            break;
        }
        selected.push(utxo.clone());
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
    for utxo in &selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    let mut outputs = vec![bridge_output];
    let change = total_input - required;
    if change > 0 {
        outputs.push(doli_core::Output::normal(change, from_hash));
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

    let chain_name = Output::bridge_chain_name(chain_id);

    println!("Bridge HTLC Lock:");
    println!("  Amount:     {} DOLI", format_balance(amount_units));
    println!("  Target:     {} -> {}", chain_name, target_address);
    println!("  Hash:       {}", hash_hex);
    println!("  Lock:       {} (claim after)", lock);
    println!("  Expiry:     {} (refund after)", expiry);
    println!("  Fee:        {}", format_balance(fee_units));
    println!("  TX Hash:    {}", tx_hash.to_hex());
    println!("  Size:       {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Bridge HTLC locked successfully!");
            println!("TX Hash: {}", result_hash);
            println!();
            println!(
                "Counterparty should now lock {} on {} to address {}",
                format_balance(amount_units),
                chain_name,
                target_address
            );
            println!("using the same hash: {}", hash_hex);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Bridge lock failed: {}", e));
        }
    }

    Ok(())
}

pub(crate) async fn cmd_bridge_claim(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    preimage_hex: &str,
    to_address: Option<&str>,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let prev_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid index"))?;

    // Get the bridge HTLC UTXO details
    let tx_info = rpc.get_transaction_json(&prev_tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

    let utxo_amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fee_units = 1000u64;
    if utxo_amount <= fee_units {
        anyhow::bail!(
            "HTLC amount {} is too small to cover fee {}. Would result in zero claim.",
            format_balance(utxo_amount),
            format_balance(fee_units)
        );
    }
    let claim_amount = utxo_amount - fee_units;

    // Verify the UTXO is actually a BridgeHTLC
    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "bridgeHtlc" && output_type != "htlc" && output_type != "hashlock" {
        println!(
            "Warning: output type is '{}', expected 'bridgeHtlc'. Proceeding anyway.",
            output_type
        );
    }

    // Determine destination: --to address or own wallet
    let dest_hash = if let Some(addr) = to_address {
        let (h, _) = crypto::address::decode(addr)
            .map_err(|_| anyhow::anyhow!("Invalid --to address: {}", addr))?;
        h
    } else {
        let from_pubkey_hash = wallet.primary_pubkey_hash();
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?
    };

    let input = Input::new(prev_tx_hash, output_index);
    let mut tx =
        Transaction::new_transfer(vec![input], vec![Output::normal(claim_amount, dest_hash)]);

    let signing_hash = tx.signing_message_for_input(0);
    let witness_str = format!("branch(left)+preimage({})", preimage_hex);
    let witness_bytes = parse_witness(&witness_str, &signing_hash)?;
    tx.set_covenant_witnesses(&[witness_bytes]);

    let keypair = wallet.primary_keypair()?;
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Bridge HTLC Claim:");
    println!(
        "  UTXO:     {}:{}",
        &prev_tx_hash.to_hex()[..16],
        output_index
    );
    println!("  Preimage: {}", preimage_hex);
    println!("  Amount:   {}", format_balance(claim_amount));
    println!("  Fee:      {}", format_balance(fee_units));
    println!("  TX Hash:  {}", tx_hash.to_hex());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Bridge HTLC claimed successfully!");
            println!("TX Hash: {}", result_hash);
            println!();
            println!("IMPORTANT: Preimage is now public on-chain.");
            println!("Counterparty can use it to claim their locked funds.");
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Bridge claim failed: {}", e));
        }
    }

    Ok(())
}

pub(crate) async fn cmd_bridge_refund(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let prev_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid index"))?;

    let tx_info = rpc.get_transaction_json(&prev_tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

    let utxo_amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fee_units = 1000u64;
    if utxo_amount <= fee_units {
        anyhow::bail!(
            "HTLC amount {} is too small to cover fee {}. Would result in zero refund.",
            format_balance(utxo_amount),
            format_balance(fee_units)
        );
    }
    let refund_amount = utxo_amount - fee_units;

    // Verify the UTXO is actually a BridgeHTLC
    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "bridgeHtlc" && output_type != "htlc" && output_type != "hashlock" {
        println!(
            "Warning: output type is '{}', expected 'bridgeHtlc'. Proceeding anyway.",
            output_type
        );
    }

    // Refund goes to the original HTLC creator (the pubkey_hash in the output)
    let htlc_pubkey_hash = utxo_output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    let refund_dest = if !htlc_pubkey_hash.is_empty() {
        // Send back to the HTLC creator, not the caller
        let h = Hash::from_hex(htlc_pubkey_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid HTLC pubkey hash"))?;
        if htlc_pubkey_hash != from_pubkey_hash {
            let creator_addr = crypto::address::encode(&h, address_prefix())
                .unwrap_or_else(|_| htlc_pubkey_hash.to_string());
            println!(
                "Refunding to HTLC creator: {} (not your wallet)",
                creator_addr
            );
        }
        h
    } else {
        // Fallback to caller's wallet if pubkey_hash not available
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?
    };

    let input = Input::new(prev_tx_hash, output_index);
    let mut tx = Transaction::new_transfer(
        vec![input],
        vec![Output::normal(refund_amount, refund_dest)],
    );

    let signing_hash = tx.signing_message_for_input(0);
    let witness_str = "branch(right)+none()";
    let witness_bytes = parse_witness(witness_str, &signing_hash)?;
    tx.set_covenant_witnesses(&[witness_bytes]);

    let keypair = wallet.primary_keypair()?;
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Bridge HTLC Refund:");
    println!(
        "  UTXO:    {}:{}",
        &prev_tx_hash.to_hex()[..16],
        output_index
    );
    println!("  Amount:  {}", format_balance(refund_amount));
    println!("  Fee:     {}", format_balance(fee_units));
    println!("  TX Hash: {}", tx_hash.to_hex());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Bridge HTLC refunded successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Bridge refund failed: {}", e));
        }
    }

    Ok(())
}
