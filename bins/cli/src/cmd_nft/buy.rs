use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

/// Atomic NFT purchase: single transaction with seller's NFT + buyer's payment.
/// Both wallets sign their respective inputs in one atomic tx.
pub(crate) async fn cmd_nft_buy(
    buyer_wallet_path: &Path,
    seller_wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    price_str: &str,
) -> Result<()> {
    let buyer_wallet = Wallet::load(buyer_wallet_path)?;
    let seller_wallet = Wallet::load(seller_wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse price
    let price_units =
        coins_to_units(price_str).map_err(|e| anyhow::anyhow!("Invalid price: {}", e))?;
    if price_units == 0 {
        anyhow::bail!("Price must be greater than 0");
    }

    // Parse UTXO reference
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let nft_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash: {}", parts[0]))?;
    let nft_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index: {}", parts[1]))?;

    // Get NFT UTXO details
    let tx_info = rpc.get_transaction_json(&nft_tx_hash.to_hex()).await?;
    let nft_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(nft_output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], nft_output_index))?;

    let nft_meta = nft_output
        .get("nft")
        .ok_or_else(|| anyhow::anyhow!("Output is not an NFT"))?;
    let token_id_hex = nft_meta
        .get("tokenId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing tokenId"))?;
    let content_hash_hex = nft_meta
        .get("contentHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let token_id =
        Hash::from_hex(token_id_hex).ok_or_else(|| anyhow::anyhow!("Invalid token_id"))?;
    let content_bytes = hex::decode(content_hash_hex).unwrap_or_default();
    let nft_amount = nft_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Resolve buyer and seller hashes
    let buyer_pubkey_hash = buyer_wallet.primary_pubkey_hash();
    let buyer_hash = Hash::from_hex(&buyer_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid buyer pubkey hash"))?;
    let seller_pubkey_hash = seller_wallet.primary_pubkey_hash();
    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller pubkey hash"))?;

    // Get buyer's spendable UTXOs for payment + fee
    // Fee will be recalculated after outputs are built, but estimate from known NFT content size
    let mut fee_units = 1u64; // provisional — recalculated below
    let required = price_units + fee_units;
    let buyer_utxos: Vec<_> = rpc
        .get_utxos(&buyer_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    for utxo in &buyer_utxos {
        if total_input >= required {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }
    if total_input < required {
        anyhow::bail!(
            "Buyer insufficient balance. Available: {}, Required: {} (price {} + fee {})",
            format_balance(total_input),
            format_balance(required),
            format_balance(price_units),
            format_balance(fee_units)
        );
    }

    // === Build atomic transaction ===
    // Input 0: NFT (seller signs)
    // Inputs 1..N: buyer's payment UTXOs (buyer signs)
    let mut inputs = vec![Input::new(nft_tx_hash, nft_output_index)];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Output 0: NFT to buyer (preserve royalty metadata)
    let buyer_cond = doli_core::Condition::signature(buyer_hash);
    let royalty = nft_meta.get("royalty").and_then(|r| {
        let creator = r.get("creator")?.as_str()?;
        let bps = r.get("bps")?.as_u64()?;
        let creator_hash = Hash::from_hex(creator)?;
        Some((creator_hash, bps as u16))
    });

    let nft_to_buyer = if let Some((creator_hash, royalty_bps)) = royalty {
        Output::nft_with_royalty(
            nft_amount,
            buyer_hash,
            token_id,
            &content_bytes,
            &buyer_cond,
            creator_hash,
            royalty_bps,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    } else {
        Output::nft(
            nft_amount,
            buyer_hash,
            token_id,
            &content_bytes,
            &buyer_cond,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    };

    // Output 1: Payment to seller (minus royalty if applicable)
    let mut seller_payment = price_units;
    if let Some((creator_hash, royalty_bps)) = royalty {
        if royalty_bps > 0 && creator_hash != seller_hash {
            let royalty_amount = (price_units as u128 * royalty_bps as u128 / 10000) as u64;
            if royalty_amount > 0 {
                seller_payment = price_units.saturating_sub(royalty_amount);
            }
        }
    }
    let payment_to_seller = Output::normal(seller_payment, seller_hash);

    // Output 2: Royalty to creator (if applicable)
    let mut outputs = vec![nft_to_buyer, payment_to_seller];
    if let Some((creator_hash, royalty_bps)) = royalty {
        if royalty_bps > 0 && creator_hash != seller_hash {
            let royalty_amount = (price_units as u128 * royalty_bps as u128 / 10000) as u64;
            if royalty_amount > 0 {
                outputs.push(Output::normal(royalty_amount, creator_hash));
            }
        }
    }

    // Recalculate fee from actual outputs extra_data
    fee_units = {
        let extra_bytes: u64 = outputs.iter().map(|o| o.extra_data.len() as u64).sum();
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let required = price_units + fee_units;
    if total_input < required {
        anyhow::bail!(
            "Buyer insufficient balance after fee recalc. Available: {}, Required: {} (price {} + fee {})",
            format_balance(total_input),
            format_balance(required),
            format_balance(price_units),
            format_balance(fee_units)
        );
    }

    // Output 3: Change to buyer (if any)
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, buyer_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // === Sign (BIP-143: per-input signing hash) ===
    let seller_keypair = seller_wallet.primary_keypair()?;
    let buyer_keypair = buyer_wallet.primary_keypair()?;

    // Covenant witness for NFT input (seller's signature satisfies the condition)
    let signing_hash_0 = tx.signing_message_for_input(0);
    let mut w = doli_core::Witness::default();
    w.signatures.push(doli_core::ConditionWitnessSignature {
        pubkey: *seller_keypair.public_key(),
        signature: signature::sign_hash(&signing_hash_0, seller_keypair.private_key()),
    });
    let nft_witness = w.encode();

    // Set covenant witnesses: NFT has witness, payment inputs have empty
    let mut witnesses: Vec<Vec<u8>> = vec![nft_witness];
    for _ in &selected_utxos {
        witnesses.push(Vec::new());
    }
    tx.set_covenant_witnesses(&witnesses);

    // Sign inputs: input 0 with seller key, inputs 1..N with buyer key
    tx.inputs[0].signature = signature::sign_hash(&signing_hash_0, seller_keypair.private_key());
    tx.inputs[0].public_key = Some(*seller_keypair.public_key());
    for i in 1..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, buyer_keypair.private_key());
        tx.inputs[i].public_key = Some(*buyer_keypair.public_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let buyer_display = crypto::address::encode(&buyer_hash, address_prefix())
        .unwrap_or_else(|_| buyer_hash.to_hex());
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("Atomic NFT Purchase:");
    println!("  Token ID: {}", token_id.to_hex());
    println!(
        "  NFT:      {}:{} -> {}",
        &nft_tx_hash.to_hex()[..16],
        nft_output_index,
        buyer_display
    );
    println!(
        "  Payment:  {} -> {}",
        format_balance(price_units),
        seller_display
    );
    println!("  Fee:      {}", format_balance(fee_units));
    if change > 0 {
        println!("  Change:   {} -> buyer", format_balance(change));
    }
    println!("  TX Hash:  {}", tx_hash.to_hex());
    println!("  Size:     {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT purchased successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("NFT purchase failed: {}", e));
        }
    }

    Ok(())
}

/// Buy from an offer file. Handles both unsigned (v1) and signed (v2/PSBT) offers.
pub(crate) async fn cmd_nft_buy_from_offer(
    buyer_wallet_path: &Path,
    seller_wallet_path: Option<&str>,
    rpc_endpoint: &str,
    offer_file: &str,
) -> Result<()> {
    let offer_json = std::fs::read_to_string(offer_file)
        .map_err(|e| anyhow::anyhow!("Cannot read offer file: {}", e))?;
    let offer: serde_json::Value = serde_json::from_str(&offer_json)
        .map_err(|e| anyhow::anyhow!("Invalid offer file: {}", e))?;

    let offer_type = offer.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match offer_type {
        "nft_sell_offer" => {
            // Unsigned v1 offer -- requires --seller-wallet (legacy behavior)
            let sw = seller_wallet_path
                .ok_or_else(|| anyhow::anyhow!("Unsigned offer requires --seller-wallet"))?;
            let seller_path = crate::common::expand_tilde(sw);
            super::sell::cmd_nft_buy_from_file(
                buyer_wallet_path,
                &seller_path,
                rpc_endpoint,
                offer_file,
            )
            .await
        }
        "nft_sell_offer_signed" => {
            // Signed PSBT offer -- no seller wallet needed
            cmd_nft_buy_from_signed_offer(buyer_wallet_path, rpc_endpoint, &offer).await
        }
        _ => {
            anyhow::bail!("Unknown offer type: {}", offer_type);
        }
    }
}

/// Complete a PSBT-signed NFT purchase. Buyer adds payment inputs and broadcasts.
pub(crate) async fn cmd_nft_buy_from_signed_offer(
    buyer_wallet_path: &Path,
    rpc_endpoint: &str,
    offer: &serde_json::Value,
) -> Result<()> {
    let buyer_wallet = Wallet::load(buyer_wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse offer fields
    let partial_tx_hex = offer
        .get("partial_tx")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing partial_tx in signed offer"))?;
    let seller_witness_hex = offer
        .get("seller_witness")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing seller_witness in signed offer"))?;
    let price_units = offer
        .get("price")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Missing price in offer"))?;
    let buyer_pubkey_hash_hex = offer
        .get("buyer_pubkey_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing buyer_pubkey_hash in offer"))?;
    let token_id_hex = offer
        .get("token_id")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    // Verify buyer matches
    let buyer_pubkey_hash = buyer_wallet.primary_pubkey_hash();
    if buyer_pubkey_hash != buyer_pubkey_hash_hex {
        anyhow::bail!(
            "This offer is for buyer {}, but your wallet is {}",
            buyer_pubkey_hash_hex,
            buyer_pubkey_hash
        );
    }

    let buyer_hash = Hash::from_hex(&buyer_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid buyer pubkey hash"))?;

    // Deserialize the partial transaction
    let partial_tx_bytes =
        hex::decode(partial_tx_hex).map_err(|_| anyhow::anyhow!("Invalid partial_tx hex"))?;
    let partial_tx = Transaction::deserialize(&partial_tx_bytes)
        .ok_or_else(|| anyhow::anyhow!("Cannot deserialize partial transaction"))?;

    let seller_witness = hex::decode(seller_witness_hex)
        .map_err(|_| anyhow::anyhow!("Invalid seller_witness hex"))?;

    // The partial TX has: input[0] = seller's NFT (signed with AnyoneCanPay), outputs = final
    // We need to add buyer's payment inputs.

    // Calculate fee from partial tx outputs (which include NFT extra_data)
    let fee_units = {
        let extra_bytes: u64 = partial_tx
            .outputs
            .iter()
            .map(|o| o.extra_data.len() as u64)
            .sum();
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let required = price_units + fee_units;

    // Get buyer's spendable UTXOs
    let buyer_utxos: Vec<_> = rpc
        .get_utxos(&buyer_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    for utxo in &buyer_utxos {
        if total_input >= required {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }
    if total_input < required {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {} (price {} + fee {})",
            format_balance(total_input),
            format_balance(required),
            format_balance(price_units),
            format_balance(fee_units)
        );
    }

    // === Reconstruct full transaction ===
    // Input 0: seller's NFT (keep signature + sighash from partial TX)
    let mut inputs = vec![partial_tx.inputs[0].clone()];

    // Inputs 1..N: buyer's payment UTXOs (SighashType::All)
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    // Outputs: seller's committed outputs + buyer's change.
    // The seller's AnyoneCanPay signature covers only the first N outputs
    // (committed_output_count). The buyer can safely append a change output.
    let mut outputs = partial_tx.outputs.clone();
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, buyer_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Preserve seller's sighash type and signature on input 0
    tx.inputs[0].sighash_type = partial_tx.inputs[0].sighash_type;

    // === Sign buyer's inputs ===
    // Buyer signs with SighashType::All -- commits to everything
    let buyer_keypair = buyer_wallet.primary_keypair()?;
    for i in 1..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, buyer_keypair.private_key());
        tx.inputs[i].public_key = Some(*buyer_keypair.public_key());
    }

    // Set covenant witnesses: seller's witness for input 0, empty for buyer inputs
    let mut witnesses: Vec<Vec<u8>> = vec![seller_witness];
    for _ in &selected_utxos {
        witnesses.push(Vec::new());
    }
    tx.set_covenant_witnesses(&witnesses);

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let buyer_display = crypto::address::encode(&buyer_hash, address_prefix())
        .unwrap_or_else(|_| buyer_hash.to_hex());
    let seller_pubkey_hash_hex = offer
        .get("seller_pubkey_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let seller_hash = Hash::from_hex(seller_pubkey_hash_hex).unwrap_or(Hash::ZERO);
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("PSBT NFT Purchase:");
    println!("  Token ID: {}", token_id_hex);
    println!("  Buyer:    {}", buyer_display);
    println!("  Seller:   {}", seller_display);
    println!("  Price:    {}", format_balance(price_units));
    println!("  Fee:      {}", format_balance(fee_units));
    if change > 0 {
        println!("  Change:   {}", format_balance(change));
    }
    println!("  TX Hash:  {}", tx_hash.to_hex());
    println!("  Size:     {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT purchased successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("NFT purchase failed: {}", e));
        }
    }

    Ok(())
}
