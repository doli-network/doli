use std::str::FromStr;

use anyhow::Result;
use crypto::{Hash, PublicKey};
use doli_core::validation::bond_owner;
use doli_core::Network;

use super::common::{compute_fifo_breakdown, display_fifo_breakdown};
use crate::common::address_prefix;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;
use doli_cli::producer_ledger::{
    max_withdrawable, producer_set_allowance, select_bond_inputs_by_count,
    withdrawal_ledger_mismatch,
};
use doli_cli::producer_tx::{chain_genesis_hash, finalize_producer_tx};
use doli_cli::unbonding_tx::{
    build_withdrawal_tx, builder_params, unbonding_gate_open, unbonding_lock_until, FeeUtxo,
    WithdrawalSpec,
};

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

    // INC-I-180: the bond count comes from the ProducerSet. The UTXO-derived
    // count is 0 for a producer that owns no live Bond UTXO.
    let received_total: u64 = producer_info
        .received_delegations
        .iter()
        .map(|d| d.bond_count as u64)
        .sum();
    let allowance_p = producer_set_allowance(
        producer_info.selection_weight,
        received_total,
        producer_info.delegated_bonds as u64,
    );
    if allowance_p == 0 {
        anyhow::bail!("Producer has no bonds to withdraw");
    }

    // Fetch per-bond details for FIFO breakdown
    let details = rpc.get_bond_details(&pk).await?;
    let live = details.bond_count as u64;
    // No live Bond UTXO is the zero-Bond-input exit; a partial mismatch aborts.
    if live != 0 && withdrawal_ledger_mismatch(live, allowance_p) {
        anyhow::bail!(
            "Ledger mismatch: wallet shows {} Bond UTXO(s) but the ProducerSet allowance is {}. \
             Aborting exit — retry after the next epoch boundary.",
            details.bond_count,
            allowance_p
        );
    }
    let available = max_withdrawable(allowance_p, details.withdrawal_pending_count as u64);
    if available == 0 {
        anyhow::bail!("All bonds already have pending withdrawals");
    }
    let withdraw_count = available as u32; // withdraw ALL available bonds

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
    if live == 0 {
        println!("  no live Bond UTXO — no Bond input is spent; spendable coins pay the fee");
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

    let pubkey_hash = wallet.primary_pubkey_hash();
    let all_utxos = rpc.get_utxos(&pubkey_hash, false).await?;

    // AUDIT-P2-003: bind selection to COUNT, not value — emit exactly `withdraw_count` Bond inputs.
    let mut bond_inputs: Vec<(Hash, u32)> = Vec::new();
    if live != 0 {
        let bond_utxos: Vec<_> = all_utxos
            .iter()
            .filter(|u| u.output_type == "bond")
            .collect();
        let bond_amounts: Vec<u64> = bond_utxos.iter().map(|u| u.amount).collect();
        let selected_idx = select_bond_inputs_by_count(&bond_amounts, withdraw_count)
            .map_err(|e| anyhow::anyhow!(e))?;
        for &i in &selected_idx {
            let utxo = &bond_utxos[i];
            let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid Bond UTXO tx_hash"))?;
            bond_inputs.push((prev_tx_hash, utxo.output_index));
        }
    }

    let mut fee_utxos: Vec<FeeUtxo> = Vec::new();
    for utxo in all_utxos
        .iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
    {
        let tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        fee_utxos.push(FeeUtxo {
            tx_hash,
            output_index: utxo.output_index,
            amount: utxo.amount,
        });
    }

    let rpc_params = rpc.get_network_params().await?;
    let network = Network::from_str(&rpc_params.network).map_err(|_| {
        anyhow::anyhow!(
            "Unknown network reported by the node: {}",
            rpc_params.network
        )
    })?;
    let net_params = builder_params(network);
    let chain_info = rpc.get_chain_info().await?;
    let best_height = chain_info.best_height;
    let genesis = chain_genesis_hash(&chain_info.genesis_hash).map_err(|e| anyhow::anyhow!(e))?;

    let spec = WithdrawalSpec {
        producer: producer_pubkey,
        bond_count: withdraw_count,
        bond_inputs,
        total_net: breakdown.total_net,
        destination: bond_owner(&producer_pubkey),
        fee_utxos,
        fee: 1,
    };
    let mut tx = build_withdrawal_tx(net_params, best_height, &spec)
        .map_err(|e| anyhow::anyhow!("Cannot build the exit: {}", e))?;
    if unbonding_gate_open(net_params, best_height) {
        println!(
            "Unbonding: the payout is locked until height {} (unbonding period {} blocks).",
            unbonding_lock_until(net_params, best_height),
            net_params.unbonding_period
        );
    }

    // Inputs (BIP-143) then the producer-auth signature over the chain genesis.
    let keypair = wallet.primary_keypair()?;
    finalize_producer_tx(&mut tx, &keypair, &genesis);

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
