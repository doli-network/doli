use std::path::Path;

use anyhow::Result;
use crypto::PublicKey;
use doli_core::transaction::{DelegateBondData, RevokeDelegationData, Transaction};

use crate::common::address_prefix;
use crate::rpc_client::RpcClient;
use crate::wallet::Wallet;

pub(super) async fn handle_delegate(
    wallet: &Wallet,
    rpc: &RpcClient,
    delegatee_hex: &str,
    bond_count: u32,
) -> Result<()> {
    println!("Delegate Bond Weight");
    println!("{:-<60}", "");
    println!();

    if !(1..=100).contains(&bond_count) {
        anyhow::bail!("Bond count must be between 1 and 100");
    }

    // Parse delegatee public key
    let delegatee_bytes = hex::decode(delegatee_hex)
        .map_err(|_| anyhow::anyhow!("Invalid delegatee public key (expected hex)"))?;
    let delegatee_pubkey = PublicKey::try_from_slice(&delegatee_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid delegatee public key: {}", e))?;

    // Get our own pubkey
    let pk_hex = wallet.addresses()[0].public_key.clone();
    let pubkey_bytes = hex::decode(&pk_hex)?;
    let delegator_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid wallet public key: {}", e))?;

    // Cannot self-delegate
    if delegator_pubkey == delegatee_pubkey {
        anyhow::bail!("Cannot delegate to yourself");
    }

    // Verify delegator is registered and active
    let my_info = rpc.get_producer(&pk_hex).await.map_err(|e| {
        if e.to_string().contains("not found") {
            anyhow::anyhow!("Your key is not registered as a producer")
        } else {
            e
        }
    })?;
    if my_info.status != "active" {
        anyhow::bail!(
            "Your producer status is '{}' — must be active to delegate",
            my_info.status
        );
    }
    if my_info.delegated_to.is_some() {
        anyhow::bail!(
            "You already have an active delegation. Revoke it first with 'doli producer revoke-delegation'."
        );
    }

    // Check available bonds (bond_count - withdrawal_pending - delegated_bonds)
    let available = my_info.bond_count.saturating_sub(my_info.delegated_bonds);
    if bond_count > available {
        anyhow::bail!(
            "Insufficient available bonds. Have {} total, {} already delegated, {} available.",
            my_info.bond_count,
            my_info.delegated_bonds,
            available
        );
    }

    // Verify delegatee exists and is active
    let delegatee_info = rpc.get_producer(delegatee_hex).await.map_err(|e| {
        if e.to_string().contains("not found") {
            anyhow::anyhow!("Delegatee is not a registered producer")
        } else {
            e
        }
    })?;
    if delegatee_info.status != "active" {
        anyhow::bail!(
            "Delegatee status is '{}' — must be active to receive delegation",
            delegatee_info.status
        );
    }

    // Display delegation details
    let delegatee_addr = hex::decode(delegatee_hex)
        .ok()
        .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
        .unwrap_or_else(|| {
            format!(
                "{}...{}",
                &delegatee_hex[..12],
                &delegatee_hex[delegatee_hex.len() - 8..]
            )
        });

    let network_params = rpc.get_network_params().await?;
    let bond_display = network_params.bond_unit / 100_000_000;

    println!("Delegating {} bond(s) to:", bond_count);
    println!("  Delegatee: {}", delegatee_addr);
    println!("  Value:     {} DOLI", bond_count as u64 * bond_display);
    println!("  Reward split: delegatee keeps 10%, you receive 90%");
    println!();

    // Build DelegateBond transaction (no inputs, no outputs)
    let data = DelegateBondData::new(delegator_pubkey, delegatee_pubkey, bond_count);
    let tx = Transaction::new_delegate_bond(data);

    let tx_hex = hex::encode(tx.serialize());
    println!("Submitting delegation transaction...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!("Delegation submitted successfully!");
            println!("TX Hash: {}", hash);
            println!();
            println!("Delegation will take effect at the next epoch boundary.");
            if let Ok(epoch) = rpc.get_epoch_info().await {
                let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                println!(
                    "Estimated activation: ~{} minutes (Epoch {}, block {}).",
                    eta_minutes,
                    epoch.current_epoch + 1,
                    epoch.epoch_end_height
                );
            }
        }
        Err(e) => {
            anyhow::bail!("Error submitting delegation: {}", e);
        }
    }

    Ok(())
}

pub(super) async fn handle_revoke_delegation(wallet: &Wallet, rpc: &RpcClient) -> Result<()> {
    println!("Revoke Delegation");
    println!("{:-<60}", "");
    println!();

    // Get our own pubkey
    let pk_hex = wallet.addresses()[0].public_key.clone();
    let pubkey_bytes = hex::decode(&pk_hex)?;
    let delegator_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid wallet public key: {}", e))?;

    // Verify we have an active delegation
    let my_info = rpc.get_producer(&pk_hex).await.map_err(|e| {
        if e.to_string().contains("not found") {
            anyhow::anyhow!("Your key is not registered as a producer")
        } else {
            e
        }
    })?;

    let delegate_hex = match &my_info.delegated_to {
        Some(hex) => hex.clone(),
        None => {
            anyhow::bail!("No active delegation to revoke");
        }
    };

    let delegate_bytes = hex::decode(&delegate_hex)
        .map_err(|_| anyhow::anyhow!("Invalid delegate public key in producer state"))?;
    let delegate_pubkey = PublicKey::try_from_slice(&delegate_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid delegate public key: {}", e))?;

    let delegate_addr = hex::decode(&delegate_hex)
        .ok()
        .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
        .unwrap_or_else(|| {
            format!(
                "{}...{}",
                &delegate_hex[..12],
                &delegate_hex[delegate_hex.len() - 8..]
            )
        });

    println!(
        "Revoking delegation of {} bond(s) from:",
        my_info.delegated_bonds
    );
    println!("  Delegatee: {}", delegate_addr);
    println!();
    println!("Note: Unbonding delay applies after revocation.");
    println!();

    // Build RevokeDelegation transaction (no inputs, no outputs)
    let data = RevokeDelegationData::new(delegator_pubkey, delegate_pubkey);
    let tx = Transaction::new_revoke_delegation(data);

    let tx_hex = hex::encode(tx.serialize());
    println!("Submitting revocation transaction...");

    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!("Revocation submitted successfully!");
            println!("TX Hash: {}", hash);
            println!();
            println!("Revocation will take effect at the next epoch boundary.");
            if let Ok(epoch) = rpc.get_epoch_info().await {
                let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                println!(
                    "Estimated activation: ~{} minutes (Epoch {}, block {}).",
                    eta_minutes,
                    epoch.current_epoch + 1,
                    epoch.epoch_end_height
                );
            }
        }
        Err(e) => {
            anyhow::bail!("Error submitting revocation: {}", e);
        }
    }

    Ok(())
}

pub(super) async fn handle_delegation_status(
    wallet_path: &Path,
    rpc: &RpcClient,
    address: Option<String>,
) -> Result<()> {
    let pk = match address {
        Some(pk) => pk,
        None => {
            let wallet = Wallet::load(wallet_path)?;
            wallet.addresses()[0].public_key.clone()
        }
    };

    println!("Delegation Status");
    println!("{:-<60}", "");
    println!();

    let info = rpc.get_producer(&pk).await.map_err(|e| {
        if e.to_string().contains("not found") {
            anyhow::anyhow!("Producer not found")
        } else {
            e
        }
    })?;

    let addr_display = hex::decode(&info.public_key)
        .ok()
        .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
        .unwrap_or_else(|| {
            format!(
                "{}...{}",
                &info.public_key[..16],
                &info.public_key[info.public_key.len() - 8..]
            )
        });

    println!("Producer:         {}", addr_display);
    println!("Status:           {}", info.status);
    println!("Bond Count:       {}", info.bond_count);
    println!("Selection Weight: {} (effective)", info.selection_weight);
    println!();

    // Outgoing delegation
    match &info.delegated_to {
        Some(delegate_hex) => {
            let delegate_addr = hex::decode(delegate_hex)
                .ok()
                .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
                .unwrap_or_else(|| {
                    format!(
                        "{}...{}",
                        &delegate_hex[..12],
                        &delegate_hex[delegate_hex.len() - 8..]
                    )
                });
            println!("Delegated To:     {}", delegate_addr);
            println!("Delegated Bonds:  {}", info.delegated_bonds);
            println!(
                "Available Bonds:  {}",
                info.bond_count.saturating_sub(info.delegated_bonds)
            );
        }
        None => {
            println!("Delegated To:     (none)");
        }
    }
    println!();

    // Incoming delegations
    if info.received_delegations.is_empty() {
        println!("Received Delegations: (none)");
    } else {
        let total_received: u32 = info.received_delegations.iter().map(|d| d.bond_count).sum();
        println!(
            "Received Delegations: {} total from {} delegator(s)",
            total_received,
            info.received_delegations.len()
        );
        println!("  {:<48} {:<10}", "Delegator Hash", "Bonds");
        println!("  {:-<60}", "");
        for d in &info.received_delegations {
            let hash_display = format!(
                "{}...{}",
                &d.delegator_hash[..12],
                &d.delegator_hash[d.delegator_hash.len() - 8..]
            );
            println!("  {:<48} {:<10}", hash_display, d.bond_count);
        }
    }

    // Pending delegation updates
    let delegation_pending: Vec<_> = info
        .pending_updates
        .iter()
        .filter(|pu| pu.update_type == "delegate_bond" || pu.update_type == "revoke_delegation")
        .collect();
    if !delegation_pending.is_empty() {
        println!();
        println!("Pending (applied at next epoch boundary):");
        for pu in delegation_pending {
            match pu.update_type.as_str() {
                "delegate_bond" => {
                    println!("  + Delegate {} bond(s)", pu.bond_count.unwrap_or(0));
                }
                "revoke_delegation" => {
                    println!("  - Revoke delegation");
                }
                _ => {}
            }
        }
    }

    Ok(())
}
