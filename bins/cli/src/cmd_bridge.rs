use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::common::address_prefix;
use crate::parsers::parse_witness;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

// =============================================================================
// SWAP PERSISTENCE
// =============================================================================

fn swaps_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".doli")
        .join("swaps")
}

fn save_swap(
    swap_id: &str,
    preimage: &[u8],
    chain: &str,
    target_address: &str,
    counter_hash: &str,
    lock: u64,
    expiry: u64,
) -> Result<()> {
    let dir = swaps_dir();
    std::fs::create_dir_all(&dir)?;
    let safe_id = swap_id.replace(':', "_");
    let path = dir.join(format!("{}.json", safe_id));
    let data = serde_json::json!({
        "swapId": swap_id,
        "preimage": hex::encode(preimage),
        "chain": chain,
        "targetAddress": target_address,
        "counterHash": counter_hash,
        "lockHeight": lock,
        "expiryHeight": expiry,
        "createdAt": chrono::Utc::now().to_rfc3339()
    });
    std::fs::write(&path, serde_json::to_string_pretty(&data)?)?;
    Ok(())
}

fn load_swap_preimage(swap_id: &str) -> Option<String> {
    let safe_id = swap_id.replace(':', "_");
    let path = swaps_dir().join(format!("{}.json", safe_id));
    let data = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    json.get("preimage")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Resolve target chain name to chain ID.
fn resolve_chain_id(chain: &str) -> Result<u8> {
    use doli_core::transaction::{
        BRIDGE_CHAIN_BITCOIN, BRIDGE_CHAIN_BSC, BRIDGE_CHAIN_CARDANO, BRIDGE_CHAIN_ETHEREUM,
        BRIDGE_CHAIN_LITECOIN, BRIDGE_CHAIN_MONERO,
    };
    match chain.to_lowercase().as_str() {
        "bitcoin" | "btc" => Ok(BRIDGE_CHAIN_BITCOIN),
        "ethereum" | "eth" => Ok(BRIDGE_CHAIN_ETHEREUM),
        "monero" | "xmr" => Ok(BRIDGE_CHAIN_MONERO),
        "litecoin" | "ltc" => Ok(BRIDGE_CHAIN_LITECOIN),
        "cardano" | "ada" => Ok(BRIDGE_CHAIN_CARDANO),
        "bsc" | "bnb" => Ok(BRIDGE_CHAIN_BSC),
        _ => anyhow::bail!(
            "Unknown chain: {}. Supported: bitcoin, ethereum, monero, litecoin, cardano, bsc",
            chain
        ),
    }
}

/// Compute counter-chain hash from raw preimage bytes.
/// Bitcoin uses SHA256, Ethereum uses keccak256.
fn compute_counter_hash(preimage: &[u8], chain_id: u8) -> Vec<u8> {
    use doli_core::transaction::{BRIDGE_CHAIN_BITCOIN, BRIDGE_CHAIN_LITECOIN};
    use sha2::{Digest, Sha256};
    use tiny_keccak::{Hasher, Keccak};

    match chain_id {
        BRIDGE_CHAIN_BITCOIN | BRIDGE_CHAIN_LITECOIN => {
            let mut hasher = Sha256::new();
            hasher.update(preimage);
            hasher.finalize().to_vec()
        }
        // Ethereum and others: keccak256
        _ => {
            let mut output = [0u8; 32];
            let mut keccak = Keccak::v256();
            keccak.update(preimage);
            keccak.finalize(&mut output);
            output.to_vec()
        }
    }
}

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
    counter_hash_hex: &str,
    multisig_threshold: Option<u8>,
    multisig_keys: Option<&str>,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
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

    let chain_id = resolve_chain_id(chain)?;

    let expected_hash =
        Hash::from_hex(hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid hash: {}", hash_hex))?;

    let counter_hash = Hash::from_hex(counter_hash_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid counter hash: {}", counter_hash_hex))?;

    let amount_units =
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if amount_units == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build bridge HTLC output (v2 with counter_hash)
    // Optionally wrap the HTLC condition in And(Multisig(...), Htlc(...))
    let bridge_output =
        if let (Some(threshold), Some(keys_str)) = (multisig_threshold, multisig_keys) {
            let key_strs: Vec<&str> = keys_str.split(',').map(|s| s.trim()).collect();
            let keys: Vec<crypto::Hash> = key_strs
                .iter()
                .map(|k| crate::parsers::resolve_to_hash(k))
                .collect::<Result<Vec<_>>>()?;
            let htlc_cond = doli_core::Condition::htlc(expected_hash, lock, expiry);
            let ms_cond = doli_core::Condition::multisig(threshold, keys);
            let wrapped = doli_core::Condition::And(Box::new(ms_cond), Box::new(htlc_cond));
            let condition_bytes = wrapped
                .encode()
                .map_err(|e| anyhow::anyhow!("Failed to encode wrapped condition: {:?}", e))?;
            let metadata_len = 3 + target_address.len() + 32; // version + chain + addr_len + addr + counter_hash
            if condition_bytes.len() + metadata_len > 512 {
                anyhow::bail!("Multisig-wrapped HTLC condition too large for extra_data");
            }
            let mut extra_data = condition_bytes;
            extra_data.push(2u8); // BRIDGE_HTLC_CURRENT_VERSION
            extra_data.push(chain_id);
            extra_data.push(target_address.len() as u8);
            extra_data.extend_from_slice(target_address.as_bytes());
            extra_data.extend_from_slice(counter_hash.as_bytes());
            doli_core::Output {
                output_type: doli_core::OutputType::BridgeHTLC,
                amount: amount_units,
                pubkey_hash: from_hash,
                lock_until: 0,
                extra_data,
            }
        } else {
            Output::bridge_htlc(
                amount_units,
                from_hash,
                expected_hash,
                lock,
                expiry,
                chain_id,
                target_address.as_bytes(),
                counter_hash,
            )
            .map_err(|e| anyhow::anyhow!("Failed to create bridge HTLC: {}", e))?
        };

    // Calculate fee: base + per-byte for bridge output extra_data
    let fee_units = {
        let extra_bytes: u64 = bridge_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
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

    println!("Bridge HTLC Lock (v2):");
    println!("  Amount:       {} DOLI", format_balance(amount_units));
    println!("  Target:       {} -> {}", chain_name, target_address);
    println!("  DOLI Hash:    {}", hash_hex);
    println!("  Counter Hash: {}", counter_hash_hex);
    println!("  Lock:         {} (claim after)", lock);
    println!("  Expiry:       {} (refund after)", expiry);
    println!("  Fee:          {}", format_balance(fee_units));
    println!("  TX Hash:      {}", tx_hash.to_hex());
    println!("  Size:         {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Bridge HTLC locked successfully!");
            println!("TX Hash: {}", result_hash);
            println!();
            println!(
                "Counterparty should now lock on {} to address {}",
                chain_name, target_address
            );
            println!("using counter hash: {}", counter_hash_hex);
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
    yes: bool,
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
    let fee_units = 1u64;
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

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

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
    yes: bool,
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
    let fee_units = 1u64;
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

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

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

/// Orchestrate a complete cross-chain atomic swap.
///
/// Generates preimage, derives both DOLI and counter-chain hashes,
/// calculates timelocks, creates the BridgeHTLC on DOLI, and prints
/// instructions for the counterparty.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_bridge_swap(
    wallet_path: &Path,
    rpc_endpoint: &str,
    amount: &str,
    to_address: &str,
    chain: &str,
    counter_rpc: Option<&str>,
    confirmations: u32,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to DOLI node at {}", rpc_endpoint);
    }

    let chain_id = resolve_chain_id(chain)?;
    let chain_name = Output::bridge_chain_name(chain_id);

    let amount_units =
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if amount_units == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }

    // Step 1: Generate preimage (32 bytes, CSPRNG)
    let mut preimage = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut preimage);

    // Step 2: Derive hashes
    let expected_hash =
        crypto::hash::hash_with_domain(doli_core::conditions::HASHLOCK_DOMAIN, &preimage);
    let counter_hash_bytes = compute_counter_hash(&preimage, chain_id);
    let mut ch_buf = [0u8; 32];
    ch_buf.copy_from_slice(&counter_hash_bytes);
    let counter_hash = Hash::from_bytes(ch_buf);

    // Step 3: Calculate timelocks
    let chain_info = rpc.get_chain_info().await?;
    let doli_height = chain_info.best_height;

    let (lock_height, expiry_height) = match chain_id {
        doli_core::transaction::BRIDGE_CHAIN_BITCOIN
        | doli_core::transaction::BRIDGE_CHAIN_LITECOIN => {
            // Bitcoin/Litecoin: DOLI expiry must be shorter than BTC expiry
            (doli_height + 3, doli_height + 360)
        }
        doli_core::transaction::BRIDGE_CHAIN_ETHEREUM
        | doli_core::transaction::BRIDGE_CHAIN_BSC => {
            // Ethereum/BSC: DOLI expiry must be shorter than ETH/BSC expiry
            (doli_height + 3, doli_height + 360)
        }
        _ => (doli_height + 3, doli_height + 360),
    };

    // Step 4: Build and broadcast BridgeHTLC on DOLI
    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    let bridge_output = Output::bridge_htlc(
        amount_units,
        from_hash,
        expected_hash,
        lock_height,
        expiry_height,
        chain_id,
        to_address.as_bytes(),
        counter_hash,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create bridge HTLC: {}", e))?;

    // Calculate fee: base + per-byte for bridge output extra_data
    let fee_units = {
        let extra_bytes: u64 = bridge_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
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
        outputs.push(Output::normal(change, from_hash));
    }

    let mut tx = Transaction::new_transfer(inputs, outputs);
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);

    println!("Broadcasting BridgeHTLC...");
    let result_hash = rpc
        .send_transaction(&tx_hex)
        .await
        .map_err(|e| anyhow::anyhow!("Broadcast failed: {}", e))?;

    // Step 5: Print swap summary
    println!();
    println!("============================================");
    println!("SWAP INITIATED");
    println!("  swap_id:        {}:0", result_hash);
    println!("  DOLI locked:    {} DOLI", format_balance(amount_units));
    println!("  Target:         {} -> {}", chain_name, to_address);
    println!("  DOLI hash:      {}", expected_hash.to_hex());
    println!("  Counter hash:   {}", counter_hash.to_hex());
    println!("  Lock height:    {} (claim after)", lock_height);
    println!("  Expiry height:  {} (refund after)", expiry_height);
    if confirmations > 0 {
        println!(
            "  Confirmations:  {} required on {}",
            confirmations, chain_name
        );
    }
    println!("============================================");
    println!();
    println!("The counterparty must deposit on {} using:", chain_name);
    println!("  Hash:    {}", counter_hash.to_hex());
    println!("  Address: {}", to_address);
    println!();
    println!("============================================");
    println!("PREIMAGE (save offline — DO NOT share):");
    println!("  P = {}", hex::encode(preimage));
    println!("============================================");
    println!();

    // Save preimage to disk
    let swap_id = format!("{}:0", result_hash);
    if let Err(e) = save_swap(
        &swap_id,
        &preimage,
        chain_name,
        to_address,
        &counter_hash.to_hex(),
        lock_height,
        expiry_height,
    ) {
        println!("Warning: failed to save swap data: {}", e);
    } else {
        let safe_id = swap_id.replace(':', "_");
        println!("Preimage saved to ~/.doli/swaps/{}.json", safe_id);
    }
    println!();

    println!("Monitor with: doli bridge-status {}:0", result_hash);
    if counter_rpc.is_some() {
        println!(
            "  Add --{}-rpc to also monitor the {} side",
            chain.to_lowercase(),
            chain_name
        );
    }

    Ok(())
}

/// Query the status of a bridge swap on-demand (no watcher needed).
///
/// Reads the BridgeHTLC UTXO from DOLI, optionally queries the counter-chain,
/// and can auto-claim or auto-refund when conditions are met.
pub(crate) async fn cmd_bridge_status(
    wallet_path: &Path,
    rpc_endpoint: &str,
    swap_id: &str,
    btc_rpc: Option<&str>,
    eth_rpc: Option<&str>,
    auto_claim: bool,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::Output;

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to DOLI node at {}", rpc_endpoint);
    }

    // Parse swap_id (txhash:output_index)
    let parts: Vec<&str> = swap_id.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("swap_id format: txhash:output_index");
    }
    let tx_hash_hex = parts[0];
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    // Step 1: Read the BridgeHTLC UTXO from DOLI
    let tx_info = rpc.get_transaction_json(tx_hash_hex).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", tx_hash_hex, output_index))?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "bridgeHtlc" {
        anyhow::bail!("Output is type '{}', expected 'bridgeHtlc'", output_type);
    }

    let utxo_amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Extract bridge metadata from RPC response
    let bridge = utxo_output.get("bridge");
    let target_chain_name = bridge
        .and_then(|b| b.get("targetChain"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let target_chain_id = bridge
        .and_then(|b| b.get("targetChainId"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    let target_address = bridge
        .and_then(|b| b.get("targetAddress"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let counter_hash_hex = bridge
        .and_then(|b| b.get("counterHash"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Extract condition info (lock/expiry heights)
    let condition = utxo_output.get("condition");
    let lock_height = find_condition_value(condition, "timelock", "height");
    let expiry_height = find_condition_value(condition, "timelockExpiry", "height");

    // Step 2: Determine DOLI-side state
    let chain_info = rpc.get_chain_info().await?;
    let current_height = chain_info.best_height;

    // Check if UTXO still exists (not yet spent)
    let utxo_pubkey = utxo_output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    // Try both the HTLC creator and our wallet to find the UTXO
    let utxo_exists = {
        let mut found = false;
        for pubkey in &[utxo_pubkey, from_pubkey_hash.as_str()] {
            if pubkey.is_empty() {
                continue;
            }
            if let Ok(utxos) = rpc.get_utxos(pubkey, false).await {
                if utxos
                    .iter()
                    .any(|u| u.tx_hash == tx_hash_hex && u.output_index == output_index)
                {
                    found = true;
                    break;
                }
            }
        }
        found
    };

    let doli_state = if utxo_exists {
        if expiry_height > 0 && current_height >= expiry_height {
            "Expired (refund available)"
        } else {
            "Locked"
        }
    } else {
        "Spent (claimed or refunded)"
    };

    // Step 3: Print status
    println!("Bridge Swap Status: {}", swap_id);
    println!("  DOLI:");
    println!("    State:        {}", doli_state);
    println!("    Amount:       {} DOLI", format_balance(utxo_amount));
    if lock_height > 0 {
        println!("    Lock height:  {}", lock_height);
    }
    if expiry_height > 0 {
        let blocks_remaining = expiry_height.saturating_sub(current_height);
        println!(
            "    Expiry:       {} ({} blocks remaining)",
            expiry_height, blocks_remaining
        );
    }
    println!("    Current:      {}", current_height);
    println!("  Counter chain:");
    println!("    Chain:        {}", target_chain_name);
    println!("    Address:      {}", target_address);
    if !counter_hash_hex.is_empty() {
        println!("    Counter hash: {}", counter_hash_hex);
    }

    // Step 4: Query counter-chain if RPC provided
    if target_chain_id == doli_core::transaction::BRIDGE_CHAIN_BITCOIN
        || target_chain_id == doli_core::transaction::BRIDGE_CHAIN_LITECOIN
    {
        if let Some(btc_endpoint) = btc_rpc {
            println!();
            println!("  Bitcoin:");
            let btc = bridge::bitcoin::BitcoinClient::new(btc_endpoint, "");
            match btc.get_block_count().await {
                Ok(btc_height) => {
                    println!("    Connected:    yes (height: {})", btc_height);
                    if !counter_hash_hex.is_empty() {
                        let watch = vec![(counter_hash_hex.to_string(), swap_id.to_string())];
                        let start = btc_height.saturating_sub(144);
                        match btc.scan_for_htlcs(start, &watch).await {
                            Ok(found) if !found.is_empty() => {
                                let (htlc, _) = &found[0];
                                println!(
                                    "    HTLC found:   txid={} amount={}sat confs={}",
                                    &htlc.txid[..16],
                                    htlc.amount_sat,
                                    htlc.confirmations
                                );
                            }
                            Ok(_) => println!("    HTLC:         not found yet"),
                            Err(e) => println!("    Scan error:   {}", e),
                        }
                    }
                }
                Err(e) => println!("    Error:        {}", e),
            }
        }
    }

    if target_chain_id == doli_core::transaction::BRIDGE_CHAIN_ETHEREUM
        || target_chain_id == doli_core::transaction::BRIDGE_CHAIN_BSC
    {
        if let Some(eth_endpoint) = eth_rpc {
            println!();
            println!("  {}:", target_chain_name);
            let eth = bridge::ethereum::EthereumClient::new(eth_endpoint);
            match eth.get_block_number().await {
                Ok(eth_height) => {
                    println!("    Connected:    yes (height: {})", eth_height);
                    if !counter_hash_hex.is_empty() {
                        let start = eth_height.saturating_sub(1000);
                        match eth.scan_for_htlc(counter_hash_hex, start).await {
                            Ok(Some(htlc)) => {
                                println!(
                                    "    HTLC found:   tx={} confs={}",
                                    &htlc.tx_hash[..16],
                                    htlc.confirmations
                                );
                            }
                            Ok(None) => println!("    HTLC:         not found yet"),
                            Err(e) => println!("    Scan error:   {}", e),
                        }
                    }
                }
                Err(e) => println!("    Error:        {}", e),
            }
        }
    }

    // Step 5: Auto-claim hint (check saved preimages)
    if utxo_exists && doli_state == "Locked" {
        if let Some(saved_preimage) = load_swap_preimage(swap_id) {
            println!();
            println!("Saved preimage found: {}", saved_preimage);
            println!("Claim with:");
            println!(
                "  doli bridge-claim {} --preimage {}",
                swap_id, saved_preimage
            );
        } else {
            println!();
            println!("Tip: If you have the preimage, claim with:");
            println!("  doli bridge-claim {} --preimage <PREIMAGE>", swap_id);
        }
    }

    // Step 6: Auto-refund if expired and UTXO still unspent
    if utxo_exists && expiry_height > 0 && current_height >= expiry_height {
        println!();
        if auto_claim {
            println!("HTLC expired. Auto-refunding...");
            let prev_tx_hash =
                Hash::from_hex(tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;

            let fee_units = 1u64;
            let refund_amount = utxo_amount - fee_units;

            let refund_dest = if !utxo_pubkey.is_empty() {
                Hash::from_hex(utxo_pubkey)
                    .ok_or_else(|| anyhow::anyhow!("Invalid HTLC pubkey hash"))?
            } else {
                Hash::from_hex(&from_pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?
            };

            let input = doli_core::Input::new(prev_tx_hash, output_index);
            let mut tx = doli_core::Transaction::new_transfer(
                vec![input],
                vec![Output::normal(refund_amount, refund_dest)],
            );

            let signing_hash = tx.signing_message_for_input(0);
            let witness_bytes = parse_witness("branch(right)+none()", &signing_hash)?;
            tx.set_covenant_witnesses(&[witness_bytes]);

            let keypair = wallet.primary_keypair()?;
            tx.inputs[0].signature =
                crypto::signature::sign_hash(&signing_hash, keypair.private_key());

            let tx_hex = hex::encode(tx.serialize());
            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => println!("Refund TX: {}", hash),
                Err(e) => println!("Refund failed: {} (may need to refund manually)", e),
            }
        } else {
            println!("HTLC expired. Refund with:");
            println!("  doli bridge-refund {}", swap_id);
        }
    }

    Ok(())
}

/// Recursively find a value in a condition JSON tree.
fn find_condition_value(cond: Option<&serde_json::Value>, cond_type: &str, field: &str) -> u64 {
    let cond = match cond {
        Some(c) => c,
        None => return 0,
    };
    if let Some(t) = cond.get("type").and_then(|v| v.as_str()) {
        if t == cond_type {
            return cond.get(field).and_then(|v| v.as_u64()).unwrap_or(0);
        }
    }
    for key in &["left", "right", "a", "b"] {
        if let Some(child) = cond.get(key) {
            let v = find_condition_value(Some(child), cond_type, field);
            if v > 0 {
                return v;
            }
        }
    }
    0
}

// =============================================================================
// BRIDGE BUY (counterparty/buyer side)
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_bridge_buy(
    wallet_path: &Path,
    rpc_endpoint: &str,
    swap_id: &str,
    preimage_hex: Option<&str>,
    btc_rpc: Option<&str>,
    eth_rpc: Option<&str>,
    to_address: Option<&str>,
    yes: bool,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to DOLI node at {}", rpc_endpoint);
    }

    // Parse swap_id (txhash:output_index)
    let parts: Vec<&str> = swap_id.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("swap_id format: txhash:output_index");
    }
    let tx_hash_hex = parts[0];
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    // Step 1: Read the BridgeHTLC UTXO from DOLI
    let tx_info = rpc.get_transaction_json(tx_hash_hex).await?;
    let utxo_output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index as usize))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", tx_hash_hex, output_index))?;

    let output_type = utxo_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "bridgeHtlc" {
        anyhow::bail!("Output is type '{}', expected 'bridgeHtlc'", output_type);
    }

    let utxo_amount = utxo_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Extract bridge metadata
    let bridge = utxo_output.get("bridge");
    let target_chain_name = bridge
        .and_then(|b| b.get("targetChain"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let target_chain_id = bridge
        .and_then(|b| b.get("targetChainId"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    let target_address = bridge
        .and_then(|b| b.get("targetAddress"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let counter_hash_hex = bridge
        .and_then(|b| b.get("counterHash"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Extract condition info (lock/expiry heights)
    let condition = utxo_output.get("condition");
    let lock_height = find_condition_value(condition, "timelock", "height")
        .max(find_condition_value(condition, "timelock", "minHeight"));
    let expiry_height = find_condition_value(condition, "timelockExpiry", "height").max(
        find_condition_value(condition, "timelockExpiry", "maxHeight"),
    );

    // Step 2: Display swap details
    println!("Bridge Swap Details:");
    println!("  Swap ID:        {}", swap_id);
    println!("  Amount:         {} DOLI", format_balance(utxo_amount));
    println!("  Chain:          {}", target_chain_name);
    println!("  Target Address: {}", target_address);
    if !counter_hash_hex.is_empty() {
        println!("  Counter Hash:   {}", counter_hash_hex);
    }
    if lock_height > 0 {
        println!("  Lock Height:    {} (claim after)", lock_height);
    }
    if expiry_height > 0 {
        println!("  Expiry Height:  {} (refund after)", expiry_height);
    }

    // Step 3: Resolve preimage
    let resolved_preimage = if let Some(p) = preimage_hex {
        Some(p.to_string())
    } else if let Some(saved) = load_swap_preimage(swap_id) {
        println!();
        println!("Found saved preimage: {}", saved);
        Some(saved)
    } else if let Some(btc_endpoint) = btc_rpc {
        // Scan Bitcoin for preimage reveals
        if target_chain_id == doli_core::transaction::BRIDGE_CHAIN_BITCOIN
            || target_chain_id == doli_core::transaction::BRIDGE_CHAIN_LITECOIN
        {
            println!();
            println!("Scanning Bitcoin for preimage...");
            let btc = bridge::bitcoin::BitcoinClient::new(btc_endpoint, "");
            match btc.get_block_count().await {
                Ok(btc_height) => {
                    if !counter_hash_hex.is_empty() {
                        let watch = vec![(counter_hash_hex.to_string(), swap_id.to_string())];
                        let start = btc_height.saturating_sub(144);
                        match btc.scan_for_htlcs(start, &watch).await {
                            Ok(found) if !found.is_empty() => {
                                let (htlc, _) = &found[0];
                                println!(
                                    "  HTLC found: txid={} confs={}",
                                    &htlc.txid[..16],
                                    htlc.confirmations
                                );
                                // TODO: extract preimage from BTC spending tx if available
                                None
                            }
                            _ => {
                                println!("  No HTLC claim found on Bitcoin");
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
                Err(e) => {
                    println!("  Bitcoin error: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else if let Some(eth_endpoint) = eth_rpc {
        // Scan Ethereum for preimage reveals
        if target_chain_id == doli_core::transaction::BRIDGE_CHAIN_ETHEREUM
            || target_chain_id == doli_core::transaction::BRIDGE_CHAIN_BSC
        {
            println!();
            println!("Scanning {} for preimage...", target_chain_name);
            let eth = bridge::ethereum::EthereumClient::new(eth_endpoint);
            match eth.get_block_number().await {
                Ok(eth_height) => {
                    if !counter_hash_hex.is_empty() {
                        let start = eth_height.saturating_sub(1000);
                        match eth.scan_for_htlc(counter_hash_hex, start).await {
                            Ok(Some(htlc)) => {
                                println!(
                                    "  HTLC found: tx={} confs={}",
                                    &htlc.tx_hash[..16],
                                    htlc.confirmations
                                );
                                // TODO: extract preimage from ETH claim event if available
                                None
                            }
                            _ => {
                                println!("  No HTLC claim found on {}", target_chain_name);
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
                Err(e) => {
                    println!("  {} error: {}", target_chain_name, e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Step 5: If no preimage found/provided, print instructions and exit
    let final_preimage = match resolved_preimage {
        Some(p) => p,
        None => {
            println!();
            println!("No preimage available. To claim this swap, provide:");
            println!("  --preimage <64 hex chars>");
            println!("  --btc-rpc <endpoint>  (to scan Bitcoin for preimage)");
            println!("  --eth-rpc <endpoint>  (to scan Ethereum for preimage)");
            return Ok(());
        }
    };

    // Validate preimage
    let preimage_bytes =
        hex::decode(&final_preimage).map_err(|_| anyhow::anyhow!("Invalid preimage hex"))?;
    if preimage_bytes.len() != 32 {
        anyhow::bail!("Preimage must be exactly 32 bytes (64 hex chars)");
    }

    let fee_units = 1u64;
    if utxo_amount <= fee_units {
        anyhow::bail!(
            "HTLC amount {} is too small to cover fee {}",
            format_balance(utxo_amount),
            format_balance(fee_units)
        );
    }
    let claim_amount = utxo_amount - fee_units;

    // Step 6: Confirmation prompt
    println!();
    println!("Claim Summary:");
    println!("  Claim Amount: {}", format_balance(claim_amount));
    println!("  Fee:          {}", format_balance(fee_units));
    println!("  Preimage:     {}", final_preimage);

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Step 7: Build claim transaction with branch(left)+preimage witness
    let prev_tx_hash =
        Hash::from_hex(tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash"))?;

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
    let witness_str = format!("branch(left)+preimage({})", final_preimage);
    let witness_bytes = parse_witness(&witness_str, &signing_hash)?;
    tx.set_covenant_witnesses(&[witness_bytes]);

    let keypair = wallet.primary_keypair()?;
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);

    // Step 8: Broadcast
    println!();
    println!("Broadcasting claim transaction...");
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
            return Err(anyhow::anyhow!("Bridge buy (claim) failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// BRIDGE LIST (scan recent blocks for bridge HTLCs)
// =============================================================================

pub(crate) async fn cmd_bridge_list(
    rpc_endpoint: &str,
    chain_filter: Option<&str>,
    scan_blocks: u64,
) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to DOLI node at {}", rpc_endpoint);
    }

    let chain_info = rpc.get_chain_info().await?;
    let current_height = chain_info.best_height;
    let start_height = current_height.saturating_sub(scan_blocks).max(1);

    println!(
        "Scanning blocks {} to {} for bridge HTLCs...",
        start_height, current_height
    );
    println!();

    struct SwapEntry {
        swap_id: String,
        amount: u64,
        chain: String,
        target_address: String,
        counter_hash: String,
        lock: u64,
        expiry: u64,
    }
    let mut entries: Vec<SwapEntry> = Vec::new();

    for height in start_height..=current_height {
        // Get block by height
        let block = match rpc
            .call_raw("getBlockByHeight", serde_json::json!({ "height": height }))
            .await
        {
            Ok(b) => b,
            Err(_) => continue,
        };

        let tx_hashes = match block.get("transactions").and_then(|t| t.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>(),
            None => continue,
        };

        for tx_hash in &tx_hashes {
            let tx_info = match rpc.get_transaction_json(tx_hash).await {
                Ok(info) => info,
                Err(_) => continue,
            };

            let outputs = match tx_info.get("outputs").and_then(|o| o.as_array()) {
                Some(arr) => arr,
                None => continue,
            };

            for (idx, output) in outputs.iter().enumerate() {
                let output_type = output
                    .get("outputType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if output_type != "bridgeHtlc" {
                    continue;
                }

                let amount = output.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
                let bridge = output.get("bridge");
                let chain_name = bridge
                    .and_then(|b| b.get("targetChain"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let target_addr = bridge
                    .and_then(|b| b.get("targetAddress"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let counter_hash = bridge
                    .and_then(|b| b.get("counterHash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let condition = output.get("condition");
                let lock = find_condition_value(condition, "timelock", "height")
                    .max(find_condition_value(condition, "timelock", "minHeight"));
                let expiry = find_condition_value(condition, "timelockExpiry", "height").max(
                    find_condition_value(condition, "timelockExpiry", "maxHeight"),
                );

                // Apply chain filter
                if let Some(filter) = chain_filter {
                    if !chain_name.to_lowercase().contains(&filter.to_lowercase()) {
                        continue;
                    }
                }

                entries.push(SwapEntry {
                    swap_id: format!("{}:{}", tx_hash, idx),
                    amount,
                    chain: chain_name,
                    target_address: target_addr,
                    counter_hash,
                    lock,
                    expiry,
                });
            }
        }
    }

    if entries.is_empty() {
        println!("No bridge HTLCs found in the last {} blocks.", scan_blocks);
    } else {
        println!(
            "{:<20} {:<14} {:<10} {:<20} {:<18} {:<8} {:<8}",
            "Swap ID", "Amount", "Chain", "Target Address", "Counter Hash", "Lock", "Expiry"
        );
        println!("{:-<100}", "");

        for entry in &entries {
            let swap_display = if entry.swap_id.len() > 18 {
                format!("{}...", &entry.swap_id[..16])
            } else {
                entry.swap_id.clone()
            };
            let addr_display = if entry.target_address.len() > 18 {
                format!("{}...", &entry.target_address[..16])
            } else {
                entry.target_address.clone()
            };
            let hash_display = if entry.counter_hash.len() > 16 {
                format!("{}...", &entry.counter_hash[..14])
            } else {
                entry.counter_hash.clone()
            };

            println!(
                "{:<20} {:<14} {:<10} {:<20} {:<18} {:<8} {:<8}",
                swap_display,
                format_balance(entry.amount),
                entry.chain,
                addr_display,
                hash_display,
                entry.lock,
                entry.expiry,
            );
        }

        println!();
        println!("Total: {} bridge HTLC(s)", entries.len());
    }

    Ok(())
}
