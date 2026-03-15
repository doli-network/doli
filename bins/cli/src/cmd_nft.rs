use std::path::Path;

use anyhow::Result;

use crate::common::{address_prefix, expand_tilde};
use crate::parsers::{parse_condition, parse_witness};
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_mint(
    wallet_path: &Path,
    rpc_endpoint: &str,
    content: &str,
    condition: Option<String>,
    amount: &str,
    royalty_pct: Option<f64>,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let minter_pubkey_hash = wallet.primary_pubkey_hash();
    let minter_hash = Hash::from_hex(&minter_pubkey_hash)
        .ok_or_else(|| anyhow::anyhow!("Invalid minter pubkey hash"))?;

    // Parse amount (minimum 1 sat dust for pure NFT — protocol requires non-zero)
    let amount_units = std::cmp::max(
        1u64,
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?,
    );

    // Content hash: if it looks like hex (64 chars), use as-is; otherwise treat as URI bytes
    let content_bytes = if content.len() == 64 && content.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(content).unwrap_or_else(|_| content.as_bytes().to_vec())
    } else {
        content.as_bytes().to_vec()
    };

    // Generate a nonce from current timestamp for token_id uniqueness
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_le_bytes()
        .to_vec();
    let token_id = Output::compute_nft_token_id(&minter_hash, &nonce);

    // Default condition: signature of the minter (only minter can transfer)
    let cond = if let Some(cond_str) = &condition {
        parse_condition(cond_str)?
    } else {
        doli_core::Condition::signature(minter_hash)
    };

    // Build NFT output (with optional royalty)
    let royalty_bps = match royalty_pct {
        Some(pct) => {
            if !(0.0..=50.0).contains(&pct) {
                anyhow::bail!("Royalty must be between 0 and 50 percent");
            }
            (pct * 100.0) as u16 // percent to basis points
        }
        None => 0,
    };

    let nft_output = if royalty_bps > 0 {
        Output::nft_with_royalty(
            amount_units,
            minter_hash,
            token_id,
            &content_bytes,
            &cond,
            minter_hash, // creator = minter
            royalty_bps,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    } else {
        Output::nft(amount_units, minter_hash, token_id, &content_bytes, &cond)
            .map_err(|e| anyhow::anyhow!("Failed to create NFT output: {}", e))?
    };

    // Get spendable normal UTXOs for fee (exclude bonds, conditioned, etc.)
    let fee_units = 1500u64;
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

    // Build outputs: NFT + change
    let mut outputs = vec![nft_output];
    let change = total_input - required;
    if change > 0 {
        outputs.push(Output::normal(change, minter_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign each input (BIP-143: per-input signing hash)
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let minter_display = crypto::address::encode(&minter_hash, address_prefix())
        .unwrap_or_else(|_| minter_hash.to_hex());

    println!("Minting NFT:");
    println!("  Token ID:  {}", token_id.to_hex());
    println!("  Content:   {}", content);
    println!("  Minter:    {}", minter_display);
    if royalty_bps > 0 {
        println!("  Royalty:   {}% to creator", royalty_bps as f64 / 100.0);
    }
    if amount_units > 0 {
        println!("  Value:     {}", format_balance(amount_units));
    }
    println!("  Fee:       {}", format_balance(fee_units));
    println!("  TX Hash:   {}", tx_hash.to_hex());
    println!("  Size:      {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("NFT minted successfully!");
            println!("TX Hash:  {}", result_hash);
            println!("Token ID: {}", token_id.to_hex());
        }
        Err(e) => {
            println!("Error: {}", e);
            return Err(anyhow::anyhow!("Mint failed: {}", e));
        }
    }

    Ok(())
}

pub(crate) async fn cmd_nft_transfer(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    to: &str,
    witness_str: &str,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::{Input, Output, Transaction};

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

    // Get spendable normal UTXOs for fee
    let sender_pubkey_hash = wallet.primary_pubkey_hash();
    let fee_units = 1500u64;
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

/// Atomic NFT purchase: single transaction with seller's NFT + buyer's payment.
/// Both wallets sign their respective inputs in one atomic tx.
pub(crate) async fn cmd_nft_buy(
    buyer_wallet_path: &Path,
    seller_wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    price_str: &str,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

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
    let fee_units = 1500u64;
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
    for i in 1..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, buyer_keypair.private_key());
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

pub(crate) async fn cmd_nft_info(rpc_endpoint: &str, utxo_ref: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }

    let tx_info = rpc.get_transaction_json(parts[0]).await?;
    let output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(parts[1].parse::<usize>().unwrap_or(0)))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], parts[1]))?;

    let output_type = output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if output_type != "nft" {
        anyhow::bail!("Output is not an NFT (type: {})", output_type);
    }

    let nft = output
        .get("nft")
        .ok_or_else(|| anyhow::anyhow!("Missing NFT metadata"))?;

    let owner = output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let amount = output.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);

    println!("NFT Info:");
    println!("{:-<60}", "");
    println!(
        "  Token ID:     {}",
        nft.get("tokenId").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  Content Hash: {}",
        nft.get("contentHash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    // Try to decode content hash as UTF-8 (might be a URI)
    if let Some(ch) = nft.get("contentHash").and_then(|v| v.as_str()) {
        if let Ok(bytes) = hex::decode(ch) {
            if let Ok(uri) = std::str::from_utf8(&bytes) {
                if uri.starts_with("http") || uri.starts_with("ipfs") {
                    println!("  Content URI:  {}", uri);
                }
            }
        }
    }

    if let Some(owner_hash) = crypto::Hash::from_hex(owner) {
        let addr = crypto::address::encode(&owner_hash, address_prefix())
            .unwrap_or_else(|_| owner.to_string());
        println!("  Owner:        {}", addr);
    } else {
        println!("  Owner:        {}", owner);
    }

    if amount > 0 {
        println!("  Value:        {}", format_balance(amount));
    }

    if let Some(cond) = output.get("condition") {
        println!("  Condition:    {}", cond);
    }

    Ok(())
}

pub(crate) async fn cmd_nft_list(wallet_path: &Path, rpc_endpoint: &str) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let pubkey_hash = wallet.primary_pubkey_hash();
    let response = rpc.get_utxos_json(&pubkey_hash, false).await?;
    let utxos = response
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid UTXO response"))?;

    let nfts: Vec<_> = utxos
        .iter()
        .filter(|u| u.get("outputType").and_then(|v| v.as_str()) == Some("nft"))
        .collect();

    if nfts.is_empty() {
        println!("No NFTs found in wallet.");
        return Ok(());
    }

    println!("NFTs ({}):", nfts.len());
    println!("{:-<80}", "");
    for nft in &nfts {
        let tx_hash = nft.get("txHash").and_then(|v| v.as_str()).unwrap_or("?");
        let idx = nft.get("outputIndex").and_then(|v| v.as_u64()).unwrap_or(0);
        let amount = nft.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);

        if let Some(meta) = nft.get("nft") {
            let token = meta.get("tokenId").and_then(|v| v.as_str()).unwrap_or("?");
            let content = meta
                .get("contentHash")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content_display = if let Ok(bytes) = hex::decode(content) {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    if s.starts_with("http") || s.starts_with("ipfs") || s.is_ascii() {
                        s.to_string()
                    } else {
                        format!("{}...", &content[..std::cmp::min(32, content.len())])
                    }
                } else {
                    format!("{}...", &content[..std::cmp::min(32, content.len())])
                }
            } else {
                content.to_string()
            };

            let tx_prefix = if tx_hash.len() >= 16 {
                &tx_hash[..16]
            } else {
                tx_hash
            };
            println!("  UTXO:    {}:{}", tx_prefix, idx);
            if token.len() >= 16 {
                println!("  Token:   {}...{}", &token[..8], &token[token.len() - 8..]);
            } else {
                println!("  Token:   {}", token);
            }
            if !content_display.is_empty() {
                println!("  Content: {}", content_display);
            }
            if amount > 1 {
                println!("  Value:   {}", format_balance(amount));
            }
            println!();
        }
    }

    Ok(())
}

pub(crate) async fn cmd_nft_sell(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    price_str: &str,
    output_file: &str,
) -> Result<()> {
    use crypto::Hash;

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
    use crypto::Hash;

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

    cmd_nft_buy(
        buyer_wallet_path,
        seller_wallet_path,
        rpc_endpoint,
        &utxo_ref,
        &price_str,
    )
    .await
}

// =============================================================================
// PSBT-STYLE NFT MARKETPLACE (sell-sign / buy from signed offer)
// =============================================================================

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
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

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
    // Outputs: NFT→buyer, payment→seller, optional royalty→creator
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

    // NFT → buyer (preserve royalty metadata if present)
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

    // Payment → seller
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
            // Unsigned v1 offer — requires --seller-wallet (legacy behavior)
            let sw = seller_wallet_path
                .ok_or_else(|| anyhow::anyhow!("Unsigned offer requires --seller-wallet"))?;
            let seller_path = expand_tilde(sw);
            cmd_nft_buy_from_file(buyer_wallet_path, &seller_path, rpc_endpoint, offer_file).await
        }
        "nft_sell_offer_signed" => {
            // Signed PSBT offer — no seller wallet needed
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
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

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

    let fee_units = 1500u64;
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
    // Buyer signs with SighashType::All — commits to everything
    let buyer_keypair = buyer_wallet.primary_keypair()?;
    for i in 1..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, buyer_keypair.private_key());
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
