use anyhow::Result;
use crypto::{signature, Hash, PublicKey};
use doli_core::{Input, Transaction};

use super::common::{compute_fifo_breakdown, display_fifo_breakdown};
use crate::common::address_prefix;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(super) async fn handle_exit(wallet: &Wallet, rpc: &RpcClient, force: bool) -> Result<()> {
    println!("Producer Exit");
    println!("{:-<60}", "");
    println!();

    // Check current status
    let pk = wallet.addresses()[0].public_key.clone();
    let producer_info = rpc.get_producer(&pk).await?;

    if producer_info.status == "exited" {
        anyhow::bail!("Producer has already exited");
    }
    if producer_info.status == "unbonding" {
        anyhow::bail!(
            "Producer is in unbonding state. Use 'request-withdrawal' to withdraw bonds."
        );
    }

    let bond_count = producer_info.bond_count;
    if bond_count == 0 {
        anyhow::bail!("Producer has no bonds to withdraw");
    }

    // Fetch per-bond details for FIFO breakdown
    let details = rpc.get_bond_details(&pk).await?;
    let available = details.bond_count - details.withdrawal_pending_count;
    if available == 0 {
        anyhow::bail!("All bonds already have pending withdrawals");
    }
    let withdraw_count = available; // withdraw ALL available bonds

    // Show bond inventory
    let s = &details.summary;
    println!(
        "Exiting producer set — withdrawing ALL {} bonds:",
        withdraw_count
    );
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

    // Calculate FIFO breakdown
    let breakdown = compute_fifo_breakdown(&details, withdraw_count);
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
    println!();

    if !force {
        println!("Use --force to proceed with exit.");
        return Ok(());
    }

    // Parse producer public key
    let pubkey_bytes = hex::decode(&pk)?;
    let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

    // Destination: wallet's own address
    let pubkey_hash = wallet.primary_pubkey_hash();
    let dest_hash =
        Hash::from_hex(&pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?;

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
    let required_bond_value = bond_unit * withdraw_count as u64;
    let available_bond_value: u64 = bond_utxos.iter().map(|u| u.amount).sum();

    if available_bond_value < required_bond_value {
        anyhow::bail!(
            "Insufficient Bond UTXOs for exit. Need {} DOLI, have {}",
            format_balance(required_bond_value),
            format_balance(available_bond_value)
        );
    }

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
    let fee_estimate = 1u64;
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

    // Flat fee: 1 satoshi per transaction
    let fee = 1u64;
    if fee_input_total < fee {
        anyhow::bail!(
            "Insufficient spendable balance for tx fee. Need {}, have {}",
            format_balance(fee),
            format_balance(fee_input_total)
        );
    }

    // Create RequestWithdrawal for ALL bonds with Bond + fee inputs
    // Validation requires exactly 1 output, so fee change is rolled into payout
    let total_net = breakdown.total_net;
    let fee_change = fee_input_total - fee;
    let payout_amount = total_net + fee_change;
    let mut tx = Transaction::new_request_withdrawal(
        all_inputs,
        producer_pubkey,
        withdraw_count,
        dest_hash,
        payout_amount,
    );

    // Sign all inputs (BIP-143: per-input signing hash)
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].public_key = Some(*keypair.public_key());
    }

    let tx_hex = hex::encode(tx.serialize());

    println!("Submitting exit (withdrawal of all bonds)...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!("Exit submitted successfully!");
            println!("TX Hash: {}", hash);
            println!();
            println!("Bonds withdrawn. Producer will be removed at next epoch boundary.");
        }
        Err(e) => {
            anyhow::bail!("Error submitting exit: {}", e);
        }
    }

    Ok(())
}

pub(super) async fn handle_slash(rpc: &RpcClient, block1: String, block2: String) -> Result<()> {
    println!("Submit Slashing Evidence");
    println!("{:-<60}", "");
    println!();

    println!("Checking blocks for equivocation evidence...");
    println!("  Block 1: {}", block1);
    println!("  Block 2: {}", block2);

    // Get block info from RPC to verify it's a valid equivocation
    let block1_resp = rpc.get_block(&block1).await?;
    let block2_resp = rpc.get_block(&block2).await?;

    // Verify they're for the same slot (equivocation)
    if block1_resp.slot != block2_resp.slot {
        anyhow::bail!("Blocks are for different slots (block 1 slot: {}, block 2 slot: {}). Slashing requires two different blocks for the SAME slot.", block1_resp.slot, block2_resp.slot);
    }

    // Verify they're from the same producer
    if block1_resp.producer != block2_resp.producer {
        anyhow::bail!("Blocks are from different producers (block 1: {}, block 2: {}). Slashing requires blocks from the SAME producer.", block1_resp.producer, block2_resp.producer);
    }

    // Verify blocks are different
    if block1_resp.hash == block2_resp.hash {
        anyhow::bail!("Both hashes refer to the same block. Slashing requires two DIFFERENT blocks for the same slot.");
    }

    let producer_addr = hex::decode(&block1_resp.producer)
        .ok()
        .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
        .unwrap_or_else(|| block1_resp.producer.clone());

    println!();
    println!("Equivocation confirmed!");
    println!("  Producer: {}", producer_addr);
    println!("  Slot:     {}", block1_resp.slot);
    println!(
        "  Block 1:  {} (height {})",
        block1_resp.hash, block1_resp.height
    );
    println!(
        "  Block 2:  {} (height {})",
        block2_resp.hash, block2_resp.height
    );
    println!();

    println!("Note: Slashing evidence submission requires full block headers with VDF proofs.");
    println!();
    println!("Nodes automatically detect and submit slashing evidence when they");
    println!("receive conflicting blocks for the same slot. If you have raw block");
    println!("data with VDF proofs, use the node's internal submission mechanism.");
    println!();
    println!("The equivocating producer ({}) will be:", producer_addr);
    println!("  - Have their entire bond burned (100% penalty)");
    println!("  - Excluded from the producer set immediately");
    println!("  - Can re-register like any new producer (standard registration VDF)");

    Ok(())
}
