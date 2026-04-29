use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

/// Atomic NFT/EC purchase: single transaction with seller's content + buyer's payment.
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
    let content_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash: {}", parts[0]))?;
    let content_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index: {}", parts[1]))?;

    // Get UTXO details
    let tx_info = rpc.get_transaction_json(&content_tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(content_output_index as usize))
        .ok_or_else(|| {
            anyhow::anyhow!("Cannot find output {}:{}", parts[0], content_output_index)
        })?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Resolve buyer and seller identities
    let buyer_pubkey_hash = buyer_wallet.primary_pubkey_hash();
    let buyer_hash = Hash::from_hex(&buyer_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid buyer pubkey hash"))?;
    let seller_pubkey_hash = seller_wallet.primary_pubkey_hash();
    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller pubkey hash"))?;
    let seller_keypair = seller_wallet.primary_keypair()?;

    // === Build content output + royalty based on type ===
    let (content_to_buyer, effective_royalty, label, is_ec) = if output_type == "encryptedContent" {
        // EncryptedContent: use buyer's pubkey for ECIES re-wrap
        let buyer_keypair = buyer_wallet.primary_keypair()?;
        let buyer_pubkey = buyer_keypair.public_key();
        let (output, royalty) =
            super::build_ec_output_for_buyer(utxo_output, amount, &seller_keypair, buyer_pubkey)?;
        (output, royalty, "Encrypted Content", true)
    } else if output_type == "nft" {
        let nft_meta = utxo_output
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

        let buyer_cond = doli_core::Condition::signature(buyer_hash);
        let royalty = nft_meta.get("royalty").and_then(|r| {
            let creator = r.get("creator")?.as_str()?;
            let bps = r.get("bps")?.as_u64()?;
            if bps > doli_core::MAX_ROYALTY_BPS as u64 {
                return None;
            }
            let creator_hash = Hash::from_hex(creator)?;
            Some((creator_hash, bps as u16))
        });

        let nft_to_buyer = if let Some((creator_hash, royalty_bps)) = royalty {
            Output::nft_with_royalty(
                amount,
                buyer_hash,
                token_id,
                &content_bytes,
                &buyer_cond,
                creator_hash,
                royalty_bps,
            )
            .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
        } else {
            Output::nft(amount, buyer_hash, token_id, &content_bytes, &buyer_cond)
                .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
        };

        (nft_to_buyer, royalty, "NFT", false)
    } else {
        anyhow::bail!(
            "Output is not an NFT or EncryptedContent (type: {})",
            output_type
        );
    };

    // === Build outputs ===
    // Payment -> seller (minus royalty if applicable)
    let mut seller_payment = price_units;
    let mut royalty_output_info = None;
    if let Some((creator_hash, royalty_bps)) = effective_royalty {
        if royalty_bps > 0 && creator_hash != seller_hash {
            let royalty_amount = (price_units as u128 * royalty_bps as u128 / 10000) as u64;
            if royalty_amount > 0 {
                seller_payment = price_units.saturating_sub(royalty_amount);
                royalty_output_info = Some((creator_hash, royalty_amount, royalty_bps));
            }
        }
    }
    let payment_to_seller = Output::normal(seller_payment, seller_hash);

    let mut outputs = vec![content_to_buyer, payment_to_seller];
    if let Some((creator_hash, royalty_amount, _)) = royalty_output_info {
        outputs.push(Output::normal(royalty_amount, creator_hash));
    }

    // AUDIT-LOGIC-001: Estimate fee from known outputs, then recalculate after full tx construction.
    let estimated_fee = {
        let extra_bytes: u64 = outputs.iter().map(|o| o.extra_data.len() as u64).sum();
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    // AUDIT-LOGIC-002: Use checked addition to prevent overflow.
    let required = price_units
        .checked_add(estimated_fee)
        .ok_or_else(|| anyhow::anyhow!("Price + fee overflows u64"))?;

    // Get buyer's spendable UTXOs for payment + fee
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
            format_balance(estimated_fee)
        );
    }

    // Change to buyer
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, buyer_hash));
    }

    // === Build transaction ===
    // Input 0: content (seller signs)
    // Inputs 1..N: buyer's payment UTXOs (buyer signs)
    let mut inputs = vec![Input::new(content_tx_hash, content_output_index)];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // === Sign ===
    let buyer_keypair = buyer_wallet.primary_keypair()?;

    // Covenant witness for NFT input (EC doesn't use covenants)
    if !is_ec {
        let signing_hash_0 = tx.signing_message_for_input(0);
        let mut w = doli_core::Witness::default();
        w.signatures.push(doli_core::ConditionWitnessSignature {
            pubkey: *seller_keypair.public_key(),
            signature: signature::sign_hash(&signing_hash_0, seller_keypair.private_key()),
        });
        let nft_witness = w.encode();

        let mut witnesses: Vec<Vec<u8>> = vec![nft_witness];
        for _ in &selected_utxos {
            witnesses.push(Vec::new());
        }
        tx.set_covenant_witnesses(&witnesses);
    }

    // Sign inputs: input 0 with seller key, inputs 1..N with buyer key
    let signing_hash_0 = tx.signing_message_for_input(0);
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
    // AUDIT-LOGIC-001: Use canonical fee from fully-constructed transaction.
    let fee_units = tx.minimum_fee();

    let buyer_display = crypto::address::encode(&buyer_hash, address_prefix())
        .unwrap_or_else(|_| buyer_hash.to_hex());
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("Atomic {} Purchase:", label);
    println!(
        "  {}:      {}:{} -> {}",
        label,
        &content_tx_hash.to_hex()[..16],
        content_output_index,
        buyer_display
    );
    println!(
        "  Payment:  {} -> {}",
        format_balance(price_units),
        seller_display
    );
    println!("  Fee:      {}", format_balance(fee_units));
    if let Some((_, royalty_amount, royalty_bps)) = royalty_output_info {
        println!(
            "  Royalty:  {} ({}%)",
            format_balance(royalty_amount),
            royalty_bps as f64 / 100.0
        );
    }
    if change > 0 {
        println!("  Change:   {} -> buyer", format_balance(change));
    }
    if is_ec {
        println!("  Key re-wrapped for buyer (ECIES).");
    }
    println!("  TX Hash:  {}", tx_hash.to_hex());
    println!("  Size:     {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("{} purchased successfully!", label);
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("{} purchase failed: {}", label, e));
        }
    }

    Ok(())
}

/// Buy from an offer file. Handles unsigned (v1), signed (PSBT), and EC offer types.
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
        "nft_sell_offer" | "ec_sell_offer" => {
            // Unsigned offer — requires --seller-wallet
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
            // Signed PSBT offer — no seller wallet needed (works for both NFT and EC)
            cmd_nft_buy_from_signed_offer(buyer_wallet_path, rpc_endpoint, &offer).await
        }
        _ => {
            anyhow::bail!("Unknown offer type: {}", offer_type);
        }
    }
}

/// Complete a PSBT-signed purchase. Buyer adds payment inputs and broadcasts.
/// Works for both NFT and EncryptedContent — the partial TX already has correct outputs.
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
    let content_type = offer
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("nft");

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

    // AUDIT-AUTH-001: Verify partial_tx outputs match offer claims before signing.
    // Without this, a malicious seller could craft outputs that redirect funds.
    let seller_pubkey_hash_hex = offer
        .get("seller_pubkey_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing seller_pubkey_hash in signed offer"))?;
    let seller_hash = Hash::from_hex(seller_pubkey_hash_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller_pubkey_hash in offer"))?;
    let outputs_count = offer
        .get("outputs_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    if partial_tx.outputs.is_empty() {
        anyhow::bail!("Signed offer partial_tx has no outputs");
    }
    if outputs_count > 0 && partial_tx.outputs.len() != outputs_count {
        anyhow::bail!(
            "Output count mismatch: offer says {} but partial_tx has {}",
            outputs_count,
            partial_tx.outputs.len()
        );
    }

    // Verify output 0 (content) goes to the buyer
    if partial_tx.outputs[0].pubkey_hash != buyer_hash {
        anyhow::bail!(
            "Content output goes to {} but buyer is {} — possible fraud",
            partial_tx.outputs[0].pubkey_hash.to_hex(),
            buyer_hash.to_hex()
        );
    }

    // Verify output 1 (payment) goes to the seller and matches the price
    if partial_tx.outputs.len() < 2 {
        anyhow::bail!("Signed offer missing payment output to seller");
    }
    // Sum all outputs to seller (payment may be split if royalty reduces it)
    let seller_total: u64 = partial_tx
        .outputs
        .iter()
        .skip(1)
        .filter(|o| o.pubkey_hash == seller_hash)
        .map(|o| o.amount)
        .sum();
    // Sum royalty outputs (outputs to neither buyer nor seller)
    let royalty_total: u64 = partial_tx
        .outputs
        .iter()
        .skip(1)
        .filter(|o| o.pubkey_hash != seller_hash && o.pubkey_hash != buyer_hash)
        .map(|o| o.amount)
        .sum();
    if seller_total + royalty_total != price_units {
        anyhow::bail!(
            "Payment mismatch: seller gets {} + royalty {}, but price is {}",
            seller_total,
            royalty_total,
            price_units
        );
    }

    // Estimate fee from partial tx outputs (recalculated canonically after full construction)
    let estimated_fee = {
        let extra_bytes: u64 = partial_tx
            .outputs
            .iter()
            .map(|o| o.extra_data.len() as u64)
            .sum();
        doli_core::consensus::BASE_FEE
            + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR
    };
    let required = price_units
        .checked_add(estimated_fee)
        .ok_or_else(|| anyhow::anyhow!("Price + fee overflows u64"))?;

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
            format_balance(estimated_fee)
        );
    }

    // === Reconstruct full transaction ===
    let mut inputs = vec![partial_tx.inputs[0].clone()];
    for utxo in &selected_utxos {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(tx_hash, utxo.output_index));
    }

    let mut outputs = partial_tx.outputs.clone();
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, buyer_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Preserve seller's sighash type on input 0
    tx.inputs[0].sighash_type = partial_tx.inputs[0].sighash_type;

    // Sign buyer's inputs
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
    let fee_units = tx.minimum_fee();

    let buyer_display = crypto::address::encode(&buyer_hash, address_prefix())
        .unwrap_or_else(|_| buyer_hash.to_hex());
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    let label = if content_type == "encryptedContent" {
        "Encrypted Content"
    } else {
        "NFT"
    };

    println!("PSBT {} Purchase:", label);
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
            println!("{} purchased successfully!", label);
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("{} purchase failed: {}", label, e));
        }
    }

    Ok(())
}
