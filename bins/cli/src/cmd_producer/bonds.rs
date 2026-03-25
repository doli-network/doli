use anyhow::Result;
use crypto::{signature, Hash, PublicKey};
use doli_core::{Input, Output, Transaction};

use super::common::{compute_fifo_breakdown, display_fifo_breakdown};
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(super) async fn handle_add_bond(
    wallet: &Wallet,
    keypair: &crypto::KeyPair,
    pubkey_hash: &str,
    rpc: &RpcClient,
    count: u32,
) -> Result<()> {
    println!("Add Bond");
    println!("{:-<60}", "");
    println!();

    if !(1..=10000).contains(&count) {
        anyhow::bail!("Bond count must be between 1 and 10000");
    }

    // Get network parameters from node (bond_unit is network-specific)
    let network_params = rpc.get_network_params().await?;
    let bond_unit = network_params.bond_unit;
    let bond_display = bond_unit / 100_000_000; // Convert to DOLI per bond
    let required_amount = bond_unit * count as u64;

    // Verify producer is registered before proceeding
    let pk_hex = hex::encode(keypair.public_key().as_bytes());
    let producers = rpc.get_producers(false).await?;
    let is_registered = producers.iter().any(|p| p.public_key == pk_hex);
    if !is_registered {
        anyhow::bail!(
            "This key is not registered as a producer. Register first with 'doli producer register'.\n\
             Public key: {}",
            pk_hex
        );
    }

    println!(
        "Adding {} bond(s) = {} DOLI",
        count,
        count as u64 * bond_display
    );
    println!();

    // Get spendable normal UTXOs (exclude bonds, conditioned, NFTs, tokens, etc.)
    let utxos: Vec<_> = rpc
        .get_utxos(pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

    // Calculate fee: base + per-byte for Bond output extra_data (4 bytes each)
    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    let fee = {
        let extra_bytes = count as u64 * 4;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    for utxo in &utxos {
        if total_input >= required_amount + fee {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }

    if total_available < required_amount + fee {
        anyhow::bail!(
            "Insufficient balance. Required: {} DOLI + {} fee, Available: {}",
            count as u64 * bond_display,
            format_balance(fee),
            format_balance(total_available)
        );
    }

    // Build inputs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected_utxos {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Parse producer public key
    let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
    let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

    // Calculate lock_until for Bond UTXO (same as registration)
    let chain_info = rpc.get_chain_info().await?;
    let blocks_per_era: u64 = match chain_info.network.as_str() {
        "devnet" => 576,
        _ => 12_614_400,
    };
    let lock_until = chain_info.best_height + blocks_per_era + 1000;

    // Create add-bond transaction (lock/unlock: creates Bond UTXO)
    let mut tx =
        Transaction::new_add_bond(inputs, producer_pubkey, count, required_amount, lock_until);

    // Add change output
    let change = total_input - required_amount - fee;
    if change > 0 {
        let change_hash =
            Hash::from_hex(pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
        tx.outputs.push(Output::normal(change, change_hash));
    }

    // Sign (BIP-143: per-input signing hash)
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    // Submit
    let tx_hex = hex::encode(tx.serialize());
    println!("Submitting add-bond transaction...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!("Bonds added successfully!");
            println!("TX Hash: {}", hash);
            println!();
            // Show activation epoch ETA
            if let Ok(epoch) = rpc.get_epoch_info().await {
                let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                println!("Status: Pending activation (epoch-deferred).",);
                println!(
                    "Estimated activation: ~{} minutes (Epoch {}, block {}).",
                    eta_minutes,
                    epoch.current_epoch + 1,
                    epoch.epoch_end_height
                );
            }
        }
        Err(e) => {
            anyhow::bail!("Error adding bonds: {}", e);
        }
    }

    Ok(())
}

pub(super) async fn handle_simulate_withdrawal(
    wallet: &Wallet,
    rpc: &RpcClient,
    count: u32,
) -> Result<()> {
    println!("Simulated Withdrawal (dry run — no transaction submitted)");
    println!("{:-<60}", "");
    println!();

    if count < 1 {
        anyhow::bail!("Must simulate at least 1 bond");
    }

    let pk = wallet.addresses()[0].public_key.clone();
    let details = rpc.get_bond_details(&pk).await?;

    let available = details.bond_count - details.withdrawal_pending_count;
    if count > available {
        anyhow::bail!(
            "--count must be between 1 and {} (your available bonds)",
            available
        );
    }

    // Show bond inventory
    let s = &details.summary;
    println!("Your bonds ({} total):", details.bond_count);
    if s.vested > 0 {
        println!("  {} bonds — vested (0% penalty)", s.vested);
    }
    if s.q3 > 0 {
        println!("  {} bonds — Q3 (25% penalty)", s.q3);
    }
    if s.q2 > 0 {
        println!("  {} bonds — Q2 (50% penalty)", s.q2);
    }
    if s.q1 > 0 {
        println!("  {} bonds — Q1 (75% penalty)", s.q1);
    }
    println!();

    // Calculate and display FIFO breakdown
    let breakdown = compute_fifo_breakdown(&details, count);

    println!("Withdrawing {} bonds (FIFO — oldest first):", count);
    display_fifo_breakdown(&breakdown);
    println!();
    println!("You would receive: {}", format_balance(breakdown.total_net));
    if breakdown.total_penalty > 0 {
        println!(
            "Penalty burned: {}",
            format_balance(breakdown.total_penalty)
        );
    }
    println!("Bonds remaining: {}", details.bond_count - count);

    Ok(())
}
