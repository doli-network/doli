use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::common::address_prefix;
use crate::parsers::{condition_to_output_type, parse_condition, parse_witness};
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) fn cmd_new(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    if wallet_path.exists() {
        anyhow::bail!(
            "Wallet already exists at {:?}. Use a different path with -w.",
            wallet_path
        );
    }

    let name = name.unwrap_or_else(|| "default".to_string());

    let (wallet, phrase) = Wallet::new(&name);
    wallet.save(wallet_path)?;

    // Write seed phrase to a separate file
    let seed_path = wallet_path.with_extension("seed.txt");
    let mut seed_content = String::new();
    let words: Vec<&str> = phrase.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        seed_content.push_str(&format!("{}. {}\n", i + 1, word));
    }
    std::fs::write(&seed_path, &seed_content)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!();
    println!("  Your wallet has been created.");
    println!();
    println!("  Recovery phrase:");
    println!();
    for (i, chunk) in words.chunks(6).enumerate() {
        let numbered: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(j, w)| format!("{:>2}. {}", i * 6 + j + 1, w))
            .collect();
        println!("    {}", numbered.join("  "));
    }
    println!();
    println!("  Address: {}", bech32_addr);
    println!();
    println!("  Wallet saved to: {:?}", wallet_path);
    println!("  Seed phrase saved to: {:?}", seed_path);
    println!();
    println!("  WARNING: This is the ONLY time your recovery phrase is shown.");
    println!("  If you lose both the wallet file and these 24 words, your DOLI is gone forever.");
    println!("  Write them down on paper and store in a safe place.");
    println!("  Then delete the seed file:");
    println!("    rm {}", seed_path.display());
    println!();

    Ok(())
}

pub(crate) fn cmd_restore(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    if wallet_path.exists() {
        anyhow::bail!(
            "Wallet already exists at {:?}. Use a different path with -w.",
            wallet_path
        );
    }

    let name = name.unwrap_or_else(|| "default".to_string());

    // Read phrase from stdin to avoid shell history exposure
    println!("Enter your 24-word recovery phrase:");
    let mut phrase = String::new();
    std::io::stdin().read_line(&mut phrase)?;
    let phrase = phrase.trim();

    if phrase.is_empty() {
        anyhow::bail!("No seed phrase provided.");
    }

    let wallet = Wallet::from_seed_phrase(&name, phrase)?;
    wallet.save(wallet_path)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!();
    println!("  Wallet restored successfully.");
    println!();
    println!("  Address: {}", bech32_addr);
    println!("  Wallet saved to: {:?}", wallet_path);
    println!();

    Ok(())
}

pub(crate) fn cmd_address(wallet_path: &Path, label: Option<String>) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    let _hex_address = wallet.generate_address(label.as_deref())?;
    wallet.save(wallet_path)?;

    // Display bech32m for the newly generated address (last in list)
    let new_addr = wallet.addresses().last().expect("just added");
    let pubkey_bytes = hex::decode(&new_addr.public_key)?;
    let bech32 = crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("New address: {}", bech32);

    Ok(())
}

pub(crate) fn cmd_addresses(wallet_path: &Path) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    // Only show the primary address. Secondary addresses exist internally
    // for UTXO consolidation but should not be displayed to users.
    let addr = &wallet.addresses()[0];
    let pubkey_bytes = hex::decode(&addr.public_key)?;
    let bech32 = crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let label_str = match addr.label.as_deref() {
        Some(l) if !l.is_empty() => format!(" ({})", l),
        _ => String::new(),
    };
    println!("Address: {}{}", bech32, label_str);

    Ok(())
}

pub(crate) async fn cmd_balance(
    wallet_path: &Path,
    rpc_endpoint: &str,
    address: Option<String>,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection first
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}. Make sure a DOLI node is running and the RPC endpoint is correct.", rpc_endpoint);
    }

    let mut total_spendable = 0u64;
    let mut total_bonded = 0u64;
    let mut total_immature = 0u64;
    let mut total_unconfirmed = 0u64;
    let mut total_activating = 0u64;

    // Build pubkey_hash -> bonded_value map from producer set.
    // Bonds live in ProducerSet (consumed on registration), not as UTXOs.
    // Use bond_amount from ProducerSet — the actual amount locked on-chain.
    let (bond_map, pending_activation_map): (
        std::collections::HashMap<String, u64>,
        std::collections::HashMap<String, u64>,
    ) = match rpc.get_producers(false).await {
        Ok(producers) => {
            let bonds = producers
                .iter()
                .filter(|p| p.bond_amount > 0 && p.status == "active")
                .filter_map(|p| {
                    let pubkey_bytes = hex::decode(&p.public_key).ok()?;
                    let pubkey_hash =
                        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
                    Some((pubkey_hash.to_hex(), p.bond_amount))
                })
                .collect();
            let pending = producers
                .iter()
                .filter(|p| p.bond_amount > 0 && p.status == "pending")
                .filter_map(|p| {
                    let pubkey_bytes = hex::decode(&p.public_key).ok()?;
                    let pubkey_hash =
                        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
                    Some((pubkey_hash.to_hex(), p.bond_amount))
                })
                .collect();
            (bonds, pending)
        }
        Err(_) => (
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ),
    };

    println!("Balances:");
    println!("{:-<60}", "");

    // Collect addresses to query — all addresses internally for correct totals,
    // but only display under the primary address to avoid confusion.
    let all_pubkey_hashes: Vec<String> = if let Some(addr) = &address {
        let pubkey_hash =
            crypto::address::resolve(addr, None).map_err(|e| anyhow::anyhow!("{}", e))?;
        vec![pubkey_hash.to_hex()]
    } else {
        wallet
            .all_pubkey_hashes()
            .into_iter()
            .map(|(hash, _)| hash)
            .collect()
    };

    // Query balances from ALL addresses and aggregate
    for pubkey_hash_hex in &all_pubkey_hashes {
        if let Ok(balance) = rpc.get_balance(pubkey_hash_hex).await {
            let bonded = bond_map.get(pubkey_hash_hex.as_str()).copied().unwrap_or(0);
            let activating = pending_activation_map
                .get(pubkey_hash_hex.as_str())
                .copied()
                .unwrap_or(0);
            total_spendable += balance.confirmed;
            total_bonded += bonded;
            total_activating += activating;
            total_immature += balance.immature;
            total_unconfirmed += balance.unconfirmed;
        }
    }

    // Display consolidated balance under the primary address only
    let primary_display = wallet.primary_bech32_address(address_prefix());
    let label = wallet.addresses()[0].label.as_deref().unwrap_or("");
    let display_addr = if !label.is_empty() {
        format!("{} ({})", primary_display, label)
    } else {
        primary_display
    };

    println!("{}", display_addr);
    println!("  Spendable: {}", format_balance(total_spendable));
    if total_bonded > 0 {
        println!(
            "  Bonded:    {}  (producer bond)",
            format_balance(total_bonded)
        );
    }
    if total_activating > 0 {
        println!(
            "  Activating: {}  (pending epoch)",
            format_balance(total_activating)
        );
    }
    if total_immature > 0 {
        println!("  Immature:  {}", format_balance(total_immature));
    }
    if total_unconfirmed > 0 {
        println!("  Pending:   {}", format_balance(total_unconfirmed));
    }
    {
        let grand_total =
            total_spendable + total_bonded + total_activating + total_immature + total_unconfirmed;
        println!("  Total:     {}", format_balance(grand_total));
    }

    Ok(())
}

pub(crate) async fn cmd_send(
    wallet_path: &Path,
    rpc_endpoint: &str,
    to: &str,
    amount: &str,
    fee: Option<String>,
    condition: Option<String>,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // SAFETY: Reject raw hex — ambiguous (pubkey vs pubkey_hash). 32 DOLI burned 2026-03-22.
    let trimmed = to.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!(
            "Raw hex is not accepted for send — use a bech32 address (doli1...) instead.\n\
             Get the recipient address with: doli info"
        );
    }

    // Parse recipient address (must be bech32 doli1...)
    let recipient_hash = crypto::address::resolve(to, None)
        .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

    // Parse amount
    let amount_units =
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;

    if amount_units == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }
    if amount_units > 2_520_000_000_000_000 {
        anyhow::bail!("Amount exceeds maximum supply (25.2M DOLI)");
    }

    // Parse explicit fee if provided; otherwise auto-calculate after UTXO selection
    let explicit_fee: Option<u64> = if let Some(f) = &fee {
        Some(coins_to_units(f).map_err(|e| anyhow::anyhow!("Invalid fee: {}", e))?)
    } else {
        None
    };

    let recipient_display = crypto::address::encode(&recipient_hash, address_prefix())
        .unwrap_or_else(|_| recipient_hash.to_hex());

    // Get spendable normal UTXOs from ALL addresses in the wallet.
    // This ensures funds at secondary addresses are not stranded.
    // Each UTXO tracks which pubkey_hash owns it for correct signing.
    let all_pubkey_hashes = wallet.all_pubkey_hashes();
    let mut utxos: Vec<(crate::rpc_client::Utxo, String)> = Vec::new(); // (utxo, owner_pubkey_hash)
    for (pubkey_hash, _idx) in &all_pubkey_hashes {
        let addr_utxos: Vec<_> = rpc
            .get_utxos(pubkey_hash, true)
            .await?
            .into_iter()
            .filter(|u| u.output_type == "normal" && u.spendable)
            .map(|u| (u, pubkey_hash.clone()))
            .collect();
        utxos.extend(addr_utxos);
    }
    // Prefer confirmed UTXOs over pending (mempool) ones
    utxos.sort_by_key(|(u, _)| u.pending as u8);
    // Change always goes to the primary address
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available. Note: Coinbase outputs require {} confirmations before they can be spent.", doli_core::consensus::COINBASE_MATURITY);
    }

    // Select UTXOs with a preliminary fee estimate, then recalculate
    let preliminary_fee = explicit_fee.unwrap_or(1);
    let total_available: u64 = utxos.iter().map(|(u, _)| u.amount).sum();

    let mut selected_utxos: Vec<(crate::rpc_client::Utxo, String)> = Vec::new();
    let mut total_input = 0u64;
    for entry in &utxos {
        if total_input >= amount_units + preliminary_fee {
            break;
        }
        selected_utxos.push(entry.clone());
        total_input += entry.0.amount;
    }

    // Flat fee: 1 satoshi per transaction
    let fee_units = explicit_fee.unwrap_or(1);

    // Re-select if auto fee increased the requirement
    if explicit_fee.is_none() && total_input < amount_units + fee_units {
        selected_utxos.clear();
        total_input = 0;
        for entry in &utxos {
            if total_input >= amount_units + fee_units {
                break;
            }
            selected_utxos.push(entry.clone());
            total_input += entry.0.amount;
        }
    }

    let required = amount_units + fee_units;

    if total_available < required {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {}",
            format_balance(total_available),
            format_balance(required)
        );
    }

    if total_input < required {
        anyhow::bail!(
            "Insufficient balance. Selected: {}, Required: {}",
            format_balance(total_input),
            format_balance(required)
        );
    }

    println!("Preparing transaction:");
    println!("  To:     {}", recipient_display);
    println!("  Amount: {} DOLI", format_balance(amount_units));
    println!("  Fee:    {}", format_balance(fee_units));

    println!();
    println!("Using {} UTXO(s):", selected_utxos.len());
    for (utxo, _) in &selected_utxos {
        println!(
            "  {}:{} - {}",
            &utxo.tx_hash[..16],
            utxo.output_index,
            format_balance(utxo.amount)
        );
    }

    // Build transaction inputs (without signatures yet)
    let mut inputs: Vec<Input> = Vec::new();
    for (utxo, _) in &selected_utxos {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build transaction outputs
    let mut outputs: Vec<Output> = Vec::new();

    // Recipient output (with optional covenant condition)
    if let Some(cond_str) = &condition {
        let cond = parse_condition(cond_str)?;
        let output_type = condition_to_output_type(&cond);
        let output = Output::conditioned(output_type, amount_units, recipient_hash, &cond)
            .map_err(|e| anyhow::anyhow!("Invalid condition: {}", e))?;
        outputs.push(output);
        println!("  Condition: {}", cond_str);
    } else {
        outputs.push(Output::normal(amount_units, recipient_hash));
    }

    // Change output always goes to primary address
    let change = total_input - required;
    if change > 0 {
        let change_hash = Hash::from_hex(&from_pubkey_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
        outputs.push(Output::normal(change, change_hash));
        println!("  Change: {}", format_balance(change));
    }

    // Create unsigned transaction
    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign each input with the keypair that owns that UTXO.
    // Different inputs may come from different addresses in the wallet.
    for (i, (_utxo, owner_pubkey_hash)) in selected_utxos.iter().enumerate() {
        let keypair = wallet.keypair_for_pubkey_hash(owner_pubkey_hash)?;
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    // Serialize transaction
    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!();
    println!("Transaction hash: {}", tx_hash.to_hex());
    println!("Transaction size: {} bytes", tx_bytes.len());

    // Confirmation prompt
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

    // Submit to network
    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Transaction submitted successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error submitting transaction: {}", e);
            return Err(anyhow::anyhow!("Transaction submission failed: {}", e));
        }
    }

    Ok(())
}

pub(crate) async fn cmd_spend(
    wallet_path: &Path,
    rpc_endpoint: &str,
    utxo_ref: &str,
    to: &str,
    amount: &str,
    witness_str: &str,
    fee: Option<String>,
) -> Result<()> {
    use crypto::Hash;
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse UTXO reference: txhash:index
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index (e.g. abc123:0)");
    }
    let prev_tx_hash =
        Hash::from_hex(parts[0]).ok_or_else(|| anyhow::anyhow!("Invalid tx hash: {}", parts[0]))?;
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index: {}", parts[1]))?;

    // Parse recipient
    let recipient_hash = crypto::address::resolve(to, None)
        .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

    // Parse amount
    let amount_units =
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if amount_units == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }

    // Fee
    let fee_units = if let Some(f) = &fee {
        coins_to_units(f).map_err(|e| anyhow::anyhow!("Invalid fee: {}", e))?
    } else {
        1 // Flat fee: 1 satoshi
    };

    // Build transaction with single input
    let input = Input::new(prev_tx_hash, output_index);
    let outputs = vec![Output::normal(amount_units, recipient_hash)];
    let mut tx = Transaction::new_transfer(vec![input], outputs);

    // Parse witness and compute signing hash (BIP-143: per-input)
    let signing_hash = tx.signing_message_for_input(0);
    let witness_bytes = parse_witness(witness_str, &signing_hash)?;

    // Set covenant witness in tx.extra_data (SegWit-style)
    tx.set_covenant_witnesses(&[witness_bytes]);

    // Also sign with the wallet key (for inputs that need a signature in the witness)
    let keypair = wallet.primary_keypair()?;
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let recipient_display = crypto::address::encode(&recipient_hash, address_prefix())
        .unwrap_or_else(|_| recipient_hash.to_hex());

    println!("Spending covenant UTXO:");
    println!(
        "  UTXO:    {}:{}",
        &prev_tx_hash.to_hex()[..16],
        output_index
    );
    println!("  To:      {}", recipient_display);
    println!("  Amount:  {} DOLI", format_balance(amount_units));
    println!("  Fee:     {}", format_balance(fee_units));
    println!("  Witness: {}", witness_str);
    println!("  TX Hash: {}", tx_hash.to_hex());
    println!("  Size:    {} bytes", tx_bytes.len());

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Transaction submitted successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error submitting transaction: {}", e);
            return Err(anyhow::anyhow!("Transaction submission failed: {}", e));
        }
    }

    Ok(())
}

pub(crate) async fn cmd_history(
    wallet_path: &Path,
    rpc_endpoint: &str,
    limit: usize,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    println!("Transaction History (limit: {})", limit);
    println!("{:-<80}", "");

    // Use pubkey_hash instead of address for RPC call
    let primary_pubkey_hash = wallet.primary_pubkey_hash();

    match rpc.get_history(&primary_pubkey_hash, limit).await {
        Ok(entries) => {
            if entries.is_empty() {
                println!("No transactions found.");
            } else {
                for entry in entries {
                    let status = if entry.confirmations > 0 {
                        format!("{} confirmations", entry.confirmations)
                    } else {
                        "pending".to_string()
                    };

                    // Capitalize first letter of tx_type
                    let tx_type = entry
                        .tx_type
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default()
                        + &entry.tx_type.chars().skip(1).collect::<String>();

                    println!("Hash:     {}", entry.hash);
                    println!("Type:     {}", tx_type);
                    println!("Block:    {} ({})", entry.height, status);
                    if entry.amount_received > 0 {
                        println!("Received: +{}", format_balance(entry.amount_received));
                    }
                    if entry.amount_sent > 0 {
                        println!("Sent:     -{}", format_balance(entry.amount_sent));
                    }
                    if entry.fee > 0 {
                        println!("Fee:      {}", format_balance(entry.fee));
                    }
                    println!();
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Error fetching history: {}", e);
        }
    }

    Ok(())
}

pub(crate) fn cmd_export(wallet_path: &Path, output: &PathBuf) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    wallet.export(output)?;

    println!("Wallet exported to: {:?}", output);

    Ok(())
}

pub(crate) fn cmd_import(wallet_path: &PathBuf, input: &PathBuf) -> Result<()> {
    let wallet = Wallet::import(input)?;
    wallet.save(wallet_path)?;

    println!("Wallet imported from: {:?}", input);
    println!("Saved to: {:?}", wallet_path);

    Ok(())
}

pub(crate) fn cmd_info(wallet_path: &Path) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!("Wallet: {}", wallet.name());
    println!("Addresses: {}", wallet.addresses().len());
    println!();
    println!("Primary Address:");
    println!("  Address:    {}", bech32_addr);
    println!("  Public Key: {}", wallet.primary_public_key());
    if let Some(bls_pub) = wallet.primary_bls_public_key() {
        println!("  BLS Key:    {}", bls_pub);
    } else {
        println!("  BLS Key:    none (run 'doli add-bls' to generate)");
    }
    println!();
    println!("Use the address above for sending and receiving DOLI.");

    Ok(())
}

pub(crate) fn cmd_add_bls(wallet_path: &Path) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    if wallet.has_bls_key() {
        println!("BLS key already exists in this wallet.");
        println!(
            "  BLS Public Key: {}",
            wallet.primary_bls_public_key().unwrap_or("?")
        );
        return Ok(());
    }

    let bls_pub = wallet.add_bls_key()?;
    wallet.save(wallet_path)?;

    println!("BLS attestation key added.");
    println!("  BLS Public Key: {}", bls_pub);
    println!();
    println!("Restart the node to load the new BLS key.");

    Ok(())
}

pub(crate) fn cmd_sign(wallet_path: &Path, message: &str, address: Option<String>) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    let signature = wallet.sign_message(message, address.as_deref())?;

    println!("Message: {}", message);
    println!("Signature: {}", signature);

    Ok(())
}

pub(crate) fn cmd_verify(message: &str, signature: &str, pubkey: &str) -> Result<()> {
    let valid = crate::wallet::verify_message(message, signature, pubkey)?;

    println!("Message: {}", message);
    println!("Signature: {}", signature);
    println!("Public Key: {}", pubkey);
    println!("Valid: {}", valid);

    Ok(())
}
