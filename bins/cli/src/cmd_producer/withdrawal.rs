use anyhow::Result;
use crypto::{signature, Hash, PublicKey};
use doli_core::{Input, Transaction};

use super::common::{compute_fifo_breakdown, display_fifo_breakdown};
use crate::common::address_prefix;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(super) async fn handle_request_withdrawal(
    wallet: &Wallet,
    rpc: &RpcClient,
    count: u32,
    destination: Option<String>,
) -> Result<()> {
    let pubkey_hash = wallet.primary_pubkey_hash();

    println!("Request Withdrawal");
    println!("{:-<60}", "");
    println!();

    if count < 1 {
        anyhow::bail!("Must withdraw at least 1 bond");
    }

    // Destination defaults to wallet address (accept doli1... or hex)
    let dest_hash = match &destination {
        Some(d) => crypto::address::resolve(d, None)
            .map_err(|e| anyhow::anyhow!("Invalid destination address: {}", e))?,
        None => {
            Hash::from_hex(&pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?
        }
    };

    let dest_display = crypto::address::encode(&dest_hash, address_prefix())
        .unwrap_or_else(|_| dest_hash.to_hex());

    // Fetch per-bond details for FIFO breakdown
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

    // Calculate and display FIFO breakdown (oldest first)
    let breakdown = compute_fifo_breakdown(&details, count);

    println!("Withdrawing {} bonds (FIFO — oldest first):", count);
    println!("Destination: {}", dest_display);
    display_fifo_breakdown(&breakdown);
    println!();
    println!("You receive: {}", format_balance(breakdown.total_net));
    if breakdown.total_penalty > 0 {
        println!(
            "Penalty burned: {}",
            format_balance(breakdown.total_penalty)
        );
        let pct = (breakdown.total_penalty * 100) / (breakdown.total_net + breakdown.total_penalty);
        if pct >= 50 {
            println!(
                "WARNING: High penalty — {}% of bond value will be burned.",
                pct
            );
        }
    } else {
        println!("No penalty — all bonds fully vested.");
    }
    let total_net = breakdown.total_net;
    println!("Bonds remaining: {}", details.bond_count - count);
    println!();

    // Parse producer public key
    let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
    let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

    // Lock/unlock: find Bond UTXOs to consume as inputs
    let all_utxos = rpc.get_utxos(&pubkey_hash, false).await?;
    let bond_utxos: Vec<_> = all_utxos
        .iter()
        .filter(|u| u.output_type == "bond")
        .collect();

    let bond_unit = {
        let params = rpc.get_network_params().await?;
        params.bond_unit
    };
    let required_bond_value = bond_unit * count as u64;
    let available_bond_value: u64 = bond_utxos.iter().map(|u| u.amount).sum();

    if available_bond_value < required_bond_value {
        anyhow::bail!(
            "Insufficient Bond UTXOs. Need {} DOLI in bonds, have {}",
            format_balance(required_bond_value),
            format_balance(available_bond_value)
        );
    }

    // Select Bond UTXOs to cover bond_count x bond_unit (any order)
    let mut bond_inputs: Vec<Input> = Vec::new();
    let mut bond_input_total = 0u64;
    for utxo in &bond_utxos {
        if bond_input_total >= required_bond_value {
            break;
        }
        let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid Bond UTXO tx_hash"))?;
        bond_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
        bond_input_total += utxo.amount;
    }

    // Select a normal UTXO to cover the tx fee
    let normal_utxos: Vec<_> = all_utxos
        .iter()
        .filter(|u| u.output_type == "normal")
        .collect();
    let mut fee_input_total = 0u64;
    let mut fee_inputs: Vec<Input> = Vec::new();
    let fee_estimate = 1000u64.max((bond_inputs.len() as u64 + 2) * 500);
    for utxo in &normal_utxos {
        if fee_input_total >= fee_estimate {
            break;
        }
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        fee_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
        fee_input_total += utxo.amount;
    }

    // Combine bond + fee inputs
    let mut all_inputs = bond_inputs;
    all_inputs.extend(fee_inputs);

    // Auto-calculate fee: max(1000, (inputs + outputs) * 500) — ensures fee_rate >= 1 sat/byte
    let num_outputs = 2u64; // payout + change
    let fee = 1000u64.max((all_inputs.len() as u64 + num_outputs) * 500);
    if fee_input_total < fee {
        anyhow::bail!(
            "Insufficient spendable balance for tx fee. Need {}, have {}",
            format_balance(fee),
            format_balance(fee_input_total)
        );
    }

    // Create request-withdrawal transaction with Bond + fee inputs
    // Validation requires exactly 1 output, so fee change is rolled into payout
    let fee_change = fee_input_total - fee;
    let payout_amount = total_net + fee_change;
    let mut tx = Transaction::new_request_withdrawal(
        all_inputs,
        producer_pubkey,
        count,
        dest_hash,
        payout_amount,
    );

    // Sign all inputs (BIP-143: per-input signing hash)
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_hex = hex::encode(tx.serialize());
    println!("Submitting withdrawal request...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!("Withdrawal submitted. TX: {}", hash);
            println!("Funds available now.");
            // Show epoch boundary ETA for bond removal
            if let Ok(epoch) = rpc.get_epoch_info().await {
                let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                println!(
                    "Bonds removed at next epoch boundary (~{} minutes, Epoch {}).",
                    eta_minutes,
                    epoch.current_epoch + 1,
                );
            }
        }
        Err(e) => {
            anyhow::bail!("Error requesting withdrawal: {}", e);
        }
    }

    Ok(())
}
