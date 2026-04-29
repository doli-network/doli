use std::path::Path;

use anyhow::Result;
use crypto::{signature, Hash};
use doli_core::{Input, Output, Transaction};

// AUDIT-CFG-001: Write offer files with owner-only permissions on Unix.
#[cfg(unix)]
fn write_offer_file(path: &str, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(content.as_bytes())
}
#[cfg(not(unix))]
fn write_offer_file(path: &str, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

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
    let tx_hash = Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let tx_info = rpc.get_transaction_json(&tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let seller_pubkey_hash = wallet.primary_pubkey_hash();

    // AUDIT-AUTH-002: Verify wallet owns this UTXO before creating offer.
    let utxo_owner = utxo_output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !utxo_owner.is_empty() && utxo_owner != seller_pubkey_hash {
        anyhow::bail!(
            "This UTXO belongs to {}, not your wallet ({})",
            utxo_owner,
            seller_pubkey_hash
        );
    }

    // Build offer JSON based on content type
    let (offer, label) = if output_type == "encryptedContent" {
        // EncryptedContent unsigned sell offer
        let ec_meta = utxo_output.get("encryptedContent");
        let mime_type = ec_meta
            .and_then(|ec| ec.get("mimeType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let royalty_json = ec_meta.and_then(|ec| ec.get("royalty")).cloned();

        let offer = serde_json::json!({
            "version": 1,
            "type": "ec_sell_offer",
            "content_utxo": {
                "tx_hash": tx_hash.to_hex(),
                "output_index": output_index,
            },
            "content_type": "encryptedContent",
            "mime_type": mime_type,
            "amount": amount,
            "price": price_units,
            "seller_pubkey_hash": seller_pubkey_hash,
            "royalty": royalty_json,
        });
        (offer, "Encrypted Content")
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

        let offer = serde_json::json!({
            "version": 1,
            "type": "nft_sell_offer",
            "nft_utxo": {
                "tx_hash": tx_hash.to_hex(),
                "output_index": output_index,
            },
            "token_id": token_id_hex,
            "content_hash": content_hash_hex,
            "nft_amount": amount,
            "price": price_units,
            "seller_pubkey_hash": seller_pubkey_hash,
        });
        (offer, "NFT")
    } else {
        anyhow::bail!(
            "Output is not an NFT or EncryptedContent (type: {})",
            output_type
        );
    };

    let offer_json = serde_json::to_string_pretty(&offer)?;
    write_offer_file(output_file, &offer_json)?;

    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller hash"))?;
    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("{} Sell Offer Created:", label);
    println!("  UTXO:     {}:{}", &tx_hash.to_hex()[..16], output_index);
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

/// Buy an NFT/EC from a sell offer file. Requires seller wallet access
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

    // Both NFT and EC unsigned offers route to cmd_nft_buy which handles both types
    let utxo_field = match offer_type {
        "nft_sell_offer" => "nft_utxo",
        "ec_sell_offer" => "content_utxo",
        _ => anyhow::bail!(
            "Not a valid unsigned sell offer file (type: {})",
            offer_type
        ),
    };

    let utxo_obj = offer
        .get(utxo_field)
        .ok_or_else(|| anyhow::anyhow!("Missing {} in offer", utxo_field))?;
    let tx_hash_hex = utxo_obj
        .get("tx_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing tx_hash in offer"))?;
    let out_index_u64 = utxo_obj
        .get("output_index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Missing output_index in offer"))?;
    let out_index = u32::try_from(out_index_u64)
        .map_err(|_| anyhow::anyhow!("output_index {} exceeds u32", out_index_u64))?;
    let price_units = offer
        .get("price")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Missing price in offer"))?;

    let _tx_hash =
        Hash::from_hex(tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash in offer"))?;

    let utxo_ref = format!("{}:{}", tx_hash_hex, out_index);
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

/// Seller signs their NFT/EC input with AnyoneCanPay and creates a signed offer file.
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

    // Parse UTXO reference
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let utxo_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;
    let utxo_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    // Get UTXO details from chain
    let tx_info = rpc.get_transaction_json(&utxo_tx_hash.to_hex()).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(utxo_output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output"))?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let seller_pubkey_hash = wallet.primary_pubkey_hash();
    let seller_hash = Hash::from_hex(&seller_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid seller pubkey hash"))?;
    let seller_keypair = wallet.primary_keypair()?;

    // AUDIT-AUTH-002: Verify wallet owns this UTXO.
    let utxo_owner = utxo_output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !utxo_owner.is_empty() && utxo_owner != seller_pubkey_hash {
        anyhow::bail!(
            "This UTXO belongs to {}, not your wallet ({})",
            utxo_owner,
            seller_pubkey_hash
        );
    }

    // === Build content output + royalty based on type ===
    let (buyer_hash, content_output, effective_royalty, label) = if output_type
        == "encryptedContent"
    {
        // EncryptedContent: resolve buyer pubkey for ECIES re-wrap
        let buyer_pubkey = super::resolve_buyer_pubkey(&rpc, buyer_address).await?;
        let buyer_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, buyer_pubkey.as_bytes());
        let (output, royalty) =
            super::build_ec_output_for_buyer(utxo_output, amount, &seller_keypair, &buyer_pubkey)?;
        (buyer_hash, output, royalty, "Encrypted Content")
    } else if output_type == "nft" {
        // NFT: decode buyer address (bech32)
        let (buyer_hash, _) = crypto::address::decode(buyer_address)
            .map_err(|_| anyhow::anyhow!("Invalid buyer address: {}", buyer_address))?;

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

        // Check for royalties
        let royalty_info = nft_meta.get("royalty").and_then(|r| {
            let creator = r.get("creator")?.as_str()?;
            let bps = r.get("bps")?.as_u64()?;
            if bps > doli_core::MAX_ROYALTY_BPS as u64 {
                return None;
            }
            Some((creator.to_string(), bps as u16))
        });
        let extra_data_hex = utxo_output
            .get("extraData")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let raw_extra = hex::decode(extra_data_hex).unwrap_or_default();
        let royalty_from_raw = if !raw_extra.is_empty() {
            let temp_out = Output {
                output_type: doli_core::OutputType::NFT,
                amount,
                pubkey_hash: seller_hash,
                lock_until: 0,
                extra_data: raw_extra,
            };
            temp_out.nft_royalty()
        } else {
            None
        };

        let effective_royalty = royalty_from_raw.or_else(|| {
            royalty_info
                .as_ref()
                .and_then(|(hex, bps)| Hash::from_hex(hex).map(|h| (h, *bps)))
        });

        let nft_to_buyer = if let Some((creator_hash, royalty_bps)) = effective_royalty {
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

        (buyer_hash, nft_to_buyer, effective_royalty, "NFT")
    } else {
        anyhow::bail!(
            "Output is not an NFT or EncryptedContent (type: {})",
            output_type
        );
    };

    // === Build outputs: content + payment + royalty ===
    let mut outputs = vec![content_output];
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

    outputs.push(Output::normal(seller_payment, seller_hash));
    if let Some((creator_hash, royalty_amount, _)) = royalty_output_info {
        outputs.push(Output::normal(royalty_amount, creator_hash));
    }

    // === Sign with AnyoneCanPay ===
    let committed_count = outputs.len() as u32;
    let content_input =
        Input::new_anyone_can_pay_partial(utxo_tx_hash, utxo_output_index, committed_count);

    let mut tx = Transaction::new_transfer(vec![content_input], outputs);

    let signing_hash = tx.signing_message_for_input(0);

    // Covenant witness (only for NFT — EC doesn't use covenants)
    let nft_witness = if output_type == "nft" {
        let mut w = doli_core::Witness::default();
        w.signatures.push(doli_core::ConditionWitnessSignature {
            pubkey: *seller_keypair.public_key(),
            signature: signature::sign_hash(&signing_hash, seller_keypair.private_key()),
        });
        w.encode()
    } else {
        Vec::new()
    };

    tx.inputs[0].signature = signature::sign_hash(&signing_hash, seller_keypair.private_key());
    tx.inputs[0].public_key = Some(*seller_keypair.public_key());

    // Build signed offer JSON
    let offer = serde_json::json!({
        "version": 3,
        "type": "nft_sell_offer_signed",
        "content_type": output_type,
        "nft_utxo": {
            "tx_hash": utxo_tx_hash.to_hex(),
            "output_index": utxo_output_index,
        },
        "nft_amount": amount,
        "price": price_units,
        "seller_pubkey_hash": seller_pubkey_hash,
        "buyer_address": buyer_address,
        "buyer_pubkey_hash": buyer_hash.to_hex(),
        "partial_tx": hex::encode(tx.serialize()),
        "seller_witness": hex::encode(&nft_witness),
        "outputs_count": tx.outputs.len(),
    });

    let offer_json = serde_json::to_string_pretty(&offer)?;
    write_offer_file(output_file, &offer_json)?;

    let seller_display = crypto::address::encode(&seller_hash, address_prefix())
        .unwrap_or_else(|_| seller_hash.to_hex());

    println!("Signed {} Sell Offer (PSBT):", label);
    println!(
        "  UTXO:     {}:{}",
        &utxo_tx_hash.to_hex()[..16],
        utxo_output_index
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
    if output_type == "encryptedContent" {
        println!("  Key re-wrapped for buyer (ECIES).");
    }
    println!("  Saved to: {}", output_file);
    println!();
    println!("The buyer completes the purchase with:");
    println!("  doli nft --from {}", output_file);
    println!();
    println!("No --seller-wallet needed. The seller's signature is embedded in the offer.");

    Ok(())
}
