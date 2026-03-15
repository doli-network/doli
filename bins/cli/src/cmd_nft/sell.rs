use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_nft_sell(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    price_str: &str,
    output_file: &str,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let price_units =
        coins_to_units(price_str).map_err(|e| anyhow::anyhow!("Invalid price: {}", e))?;
    if price_units == 0 {
        anyhow::bail!("Price must be greater than 0");
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let nft_tx_hash = Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let nft_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let tx_info = rpc.get_transaction_json(&nft_tx_hash.to_hex()).await?;
    let nft_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(nft_output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

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
    let nft_amount = nft_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let seller_pubkey_hash = wallet.primary_pubkey_hash();

    let offer = serde_json::json!({
        "version": 1,
        "type": "nft_sell_offer",
        "nft_utxo": {
            "tx_hash": nft_tx_hash.to_hex(),
            "output_index": nft_output_index,
        },
        "token_id": token_id_hex,
        "content_hash": content_hash_hex,
        "nft_amount": nft_amount,
        "price": price_units,
        "seller_pubkey_hash": seller_pubkey_hash,
    });

    let offer_json = serde_json::to_string_pretty(&offer)?;
    std::fs::write(output_file, &offer_json)?;

    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller hash"))?;
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("NFT Sell Offer Created:");
    println!("  Token ID: {}", token_id_hex);
    println!(
        "  UTXO:     {}:{}",
        &nft_tx_hash.to_hex()[..16],
        nft_output_index
    );
    println!("  Price:    {}", format_balance(price_units));
    println!("  Seller:   {}", seller_display);
    println!("  Saved to: {}", output_file);
    println!();
    println!("Share this file with the buyer. They complete with:");
    println!(
        "  doli nft --from {} --seller-wallet /path/to/seller-wallet.json",
        output_file
    );

    Ok(())
}

/// Buy an NFT from a sell offer file. Requires seller wallet access
/// because DOLI does not have SIGHASH flags for partial signing.
pub(crate) async fn cmd_nft_buy_from_file(
    buyer_wallet_path: &Path,
    seller_wallet_path: &Path,
    rpc_endpoint: &str,
    offer_file: &str,
) -> Result<()> {
    let offer_json = std::fs::read_to_string(offer_file)
        .map_err(|e| anyhow::anyhow!("Cannot read offer file: {}", e))?;
    let offer: serde_json::Value = serde_json::from_str(&offer_json)
        .map_err(|e| anyhow::anyhow!("Invalid offer file: {}", e))?;

    let offer_type = offer.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if offer_type != "nft_sell_offer" {
        anyhow::bail!("Not a valid sell offer file (type: {})", offer_type);
    }

    let nft_utxo = offer
        .get("nft_utxo")
        .ok_or_else(|| anyhow::anyhow!("Missing nft_utxo in offer"))?;
    let nft_tx_hash_hex = nft_utxo
        .get("tx_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing tx_hash in offer"))?;
    let nft_output_index = nft_utxo
        .get("output_index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Missing output_index in offer"))?
        as u32;
    let price_units = offer
        .get("price")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Missing price in offer"))?;

    let _nft_tx_hash = Hash::from_hex(nft_tx_hash_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid tx hash in offer"))?;

    let utxo_ref = format!("{}:{}", nft_tx_hash_hex, nft_output_index);
    let price_str = crate::rpc_client::units_to_coins(price_units);

    super::buy::cmd_nft_buy(
        buyer_wallet_path,
        seller_wallet_path,
        rpc_endpoint,
        &utxo_ref,
        &price_str,
    )
    .await
}

/// Seller signs their NFT input with AnyoneCanPay and creates a signed offer file.
/// The buyer can later complete the purchase WITHOUT the seller's wallet.
pub(crate) async fn cmd_nft_sell_sign(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    price_str: &str,
    buyer_address: &str,
    output_file: &str,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let price_units =
        coins_to_units(price_str).map_err(|e| anyhow::anyhow!("Invalid price: {}", e))?;
    if price_units == 0 {
        anyhow::bail!("Price must be greater than 0");
    }

    // Parse buyer address
    let (buyer_hash, _) = crypto::address::decode(buyer_address)
        .map_err(|_| anyhow::anyhow!("Invalid buyer address: {}", buyer_address))?;

    // Parse UTXO reference
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let nft_tx_hash = Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let nft_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    // Get NFT details from chain
    let tx_info = rpc.get_transaction_json(&nft_tx_hash.to_hex()).await?;
    let nft_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(nft_output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

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

    let seller_pubkey_hash = wallet.primary_pubkey_hash();
    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller pubkey hash"))?;
    let seller_keypair = wallet.primary_keypair()?;

    // === Build partial transaction ===
    // Input 0: NFT with AnyoneCanPay (seller signs only this input + committed outputs)
    // Outputs: NFT->buyer, payment->seller, optional royalty->creator
    // Buyer will add their payment inputs + change output later.

    // Output 0: NFT to buyer
    let buyer_cond = doli_core::Condition::signature(buyer_hash);

    // Check for royalties on the NFT being sold
    let royalty_info = nft_meta.get("royalty").and_then(|r| {
        let creator = r.get("creator")?.as_str()?;
        let bps = r.get("bps")?.as_u64()?;
        Some((creator.to_string(), bps as u16))
    });
    // Also check raw extra_data for royalty (the RPC may not expose it yet)
    let extra_data_hex = nft_output
        .get("extraData")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let raw_extra = hex::decode(extra_data_hex).unwrap_or_default();
    let royalty_from_raw = if !raw_extra.is_empty() {
        // Try to decode NFT royalty from raw UTXO output
        let temp_out = Output {
            output_type: doli_core::OutputType::NFT,
            amount: nft_amount,
            pubkey_hash: seller_hash,
            lock_until: 0,
            extra_data: raw_extra,
        };
        temp_out.nft_royalty()
    } else {
        None
    };

    let mut outputs = Vec::new();

    // NFT -> buyer (preserve royalty metadata if present)
    let effective_royalty = royalty_from_raw
        .as_ref()
        .map(|(h, bps)| (*h, *bps))
        .or_else(|| {
            royalty_info
                .as_ref()
                .and_then(|(hex, bps)| Hash::from_hex(hex).map(|h| (h, *bps)))
        });

    let nft_to_buyer = if let Some((creator_hash, royalty_bps)) = effective_royalty {
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
    outputs.push(nft_to_buyer);

    // Payment -> seller
    let mut seller_payment = price_units;
    let mut royalty_output_info = None;

    // Handle royalty if present
    if let Some((creator_hash, royalty_bps)) = royalty_from_raw {
        if royalty_bps > 0 && creator_hash != seller_hash {
            let royalty_amount = (price_units as u128 * royalty_bps as u128 / 10000) as u64;
            if royalty_amount > 0 {
                seller_payment = price_units.saturating_sub(royalty_amount);
                royalty_output_info = Some((creator_hash, royalty_amount, royalty_bps));
            }
        }
    } else if let Some((creator_hex, royalty_bps)) = &royalty_info {
        if let Some(creator_hash) = Hash::from_hex(creator_hex) {
            if *royalty_bps > 0 && creator_hash != seller_hash {
                let royalty_amount = (price_units as u128 * *royalty_bps as u128 / 10000) as u64;
                if royalty_amount > 0 {
                    seller_payment = price_units.saturating_sub(royalty_amount);
                    royalty_output_info = Some((creator_hash, royalty_amount, *royalty_bps));
                }
            }
        }
    }

    outputs.push(Output::normal(seller_payment, seller_hash));

    if let Some((creator_hash, royalty_amount, _)) = royalty_output_info {
        outputs.push(Output::normal(royalty_amount, creator_hash));
    }

    // Set committed_output_count so the buyer can append a change output
    // without invalidating the seller's signature.
    let committed_count = outputs.len() as u32;
    let nft_input =
        Input::new_anyone_can_pay_partial(nft_tx_hash, nft_output_index, committed_count);

    let mut tx = Transaction::new_transfer(vec![nft_input], outputs);

    // === Sign with AnyoneCanPay ===
    // signing_message_for_input(0) with AnyoneCanPay hashes only the first
    // committed_count outputs. The buyer can safely append change outputs.
    let signing_hash = tx.signing_message_for_input(0);

    // Covenant witness for NFT input
    let mut w = doli_core::Witness::default();
    w.signatures.push(doli_core::ConditionWitnessSignature {
        pubkey: *seller_keypair.public_key(),
        signature: signature::sign_hash(&signing_hash, seller_keypair.private_key()),
    });
    let nft_witness = w.encode();

    // Sign the regular input signature too
    tx.inputs[0].signature = signature::sign_hash(&signing_hash, seller_keypair.private_key());

    // Build signed offer JSON
    let offer = serde_json::json!({
        "version": 3,
        "type": "nft_sell_offer_signed",
        "nft_utxo": {
            "tx_hash": nft_tx_hash.to_hex(),
            "output_index": nft_output_index,
        },
        "token_id": token_id_hex,
        "content_hash": content_hash_hex,
        "nft_amount": nft_amount,
        "price": price_units,
        "seller_pubkey_hash": seller_pubkey_hash,
        "buyer_address": buyer_address,
        "buyer_pubkey_hash": buyer_hash.to_hex(),
        // Partial TX: seller's signed input + all outputs (buyer adds payment inputs)
        "partial_tx": hex::encode(tx.serialize()),
        "seller_witness": hex::encode(&nft_witness),
        "outputs_count": tx.outputs.len(),
    });

    let offer_json = serde_json::to_string_pretty(&offer)?;
    std::fs::write(output_file, &offer_json)?;

    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("Signed NFT Sell Offer (PSBT):");
    println!("  Token ID: {}", token_id_hex);
    println!(
        "  UTXO:     {}:{}",
        &nft_tx_hash.to_hex()[..16],
        nft_output_index
    );
    println!("  Price:    {}", format_balance(price_units));
    println!("  Seller:   {}", seller_display);
    println!("  Buyer:    {}", buyer_address);
    if let Some((_, royalty_amount, royalty_bps)) = royalty_output_info {
        println!(
            "  Royalty:  {} ({}%)",
            format_balance(royalty_amount),
            royalty_bps as f64 / 100.0
        );
    }
    println!("  Saved to: {}", output_file);
    println!();
    println!("The buyer completes the purchase with:");
    println!("  doli nft --from {}", output_file);
    println!();
    println!("No --seller-wallet needed. The seller's signature is embedded in the offer.");

    Ok(())
}
