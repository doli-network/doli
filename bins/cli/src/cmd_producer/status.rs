use std::path::Path;

use anyhow::Result;

use super::common::format_slot_duration;
use crate::common::address_prefix;
use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(super) async fn handle_status(
    wallet_path: &Path,
    rpc: &RpcClient,
    pubkey: Option<String>,
) -> Result<()> {
    let pk = match pubkey {
        Some(pk) => pk,
        None => {
            let wallet = Wallet::load(wallet_path)?;
            wallet.addresses()[0].public_key.clone()
        }
    };

    println!("Producer Status");
    println!("{:-<60}", "");
    println!();

    match rpc.get_producer(&pk).await {
        Ok(info) => {
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
            println!("Address:       {}", addr_display);
            println!("Status:        {}", info.status);
            println!("Registered at: block {}", info.registration_height);
            println!("Bond Count:    {}", info.bond_count);
            println!("Bond Amount:   {}", format_balance(info.bond_amount));
            println!("Current Era:   {}", info.era);

            // Show per-bond vesting info from getBondDetails
            if let Ok(details) = rpc.get_bond_details(&pk).await {
                let vq = details.vesting_quarter_slots;

                // Find oldest bond per tier for time-to-next display
                let mut oldest_q3: Option<u64> = None; // oldest age in Q3
                let mut oldest_q2: Option<u64> = None;
                let mut oldest_q1: Option<u64> = None;
                for bond in &details.bonds {
                    if bond.vested {
                        continue;
                    }
                    let quarters = bond.age_slots / vq;
                    match quarters {
                        0 => {
                            oldest_q1 =
                                Some(oldest_q1.map_or(bond.age_slots, |v| v.max(bond.age_slots)));
                        }
                        1 => {
                            oldest_q2 =
                                Some(oldest_q2.map_or(bond.age_slots, |v| v.max(bond.age_slots)));
                        }
                        2 => {
                            oldest_q3 =
                                Some(oldest_q3.map_or(bond.age_slots, |v| v.max(bond.age_slots)));
                        }
                        _ => {}
                    }
                }

                println!();
                println!(
                    "Bonds: {} ({}):",
                    details.bond_count,
                    format_balance(details.total_staked)
                );
                let s = &details.summary;
                if s.vested > 0 {
                    let label = if s.vested == 1 { "bond " } else { "bonds" };
                    println!("  Vested (0% penalty):   {} {}", s.vested, label);
                }
                if s.q3 > 0 {
                    let label = if s.q3 == 1 { "bond " } else { "bonds" };
                    let eta = oldest_q3
                        .map(|age| {
                            let remaining = (3 * vq).saturating_sub(age);
                            format!(" — {} to 0%", format_slot_duration(remaining))
                        })
                        .unwrap_or_default();
                    println!("  Q3 (25% penalty):      {} {}{}", s.q3, label, eta);
                }
                if s.q2 > 0 {
                    let label = if s.q2 == 1 { "bond " } else { "bonds" };
                    let eta = oldest_q2
                        .map(|age| {
                            let remaining = (2 * vq).saturating_sub(age);
                            format!(" — {} to 25%", format_slot_duration(remaining))
                        })
                        .unwrap_or_default();
                    println!("  Q2 (50% penalty):      {} {}{}", s.q2, label, eta);
                }
                if s.q1 > 0 {
                    let label = if s.q1 == 1 { "bond " } else { "bonds" };
                    let eta = oldest_q1
                        .map(|age| {
                            let remaining = vq.saturating_sub(age);
                            format!(" — {} to 50%", format_slot_duration(remaining))
                        })
                        .unwrap_or_default();
                    println!("  Q1 (75% penalty):      {} {}{}", s.q1, label, eta);
                }
                if details.withdrawal_pending_count > 0 {
                    println!();
                    println!(
                        "  Withdrawal pending: {} bonds (applied at next epoch boundary)",
                        details.withdrawal_pending_count
                    );
                }
            }

            // Show pending epoch-deferred updates
            if !info.pending_updates.is_empty() {
                println!();
                println!("Pending updates (applied at next epoch boundary):");
                for pu in &info.pending_updates {
                    match pu.update_type.as_str() {
                        "add_bond" => {
                            println!("  + Add {} bond(s)", pu.bond_count.unwrap_or(0))
                        }
                        "withdrawal" => {
                            println!("  - Withdraw {} bond(s)", pu.bond_count.unwrap_or(0))
                        }
                        "exit" => println!("  - Exit producer set"),
                        "register" => println!("  + Registration pending"),
                        other => println!("  ? {}", other),
                    }
                }
                if let Ok(epoch) = rpc.get_epoch_info().await {
                    let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                    println!(
                        "  ETA: ~{} minutes (Epoch {}, block {})",
                        eta_minutes,
                        epoch.current_epoch + 1,
                        epoch.epoch_end_height
                    );
                }
            }
        }
        Err(e) => {
            if e.to_string().contains("not found") {
                let pk_display = hex::decode(&pk)
                    .ok()
                    .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
                    .unwrap_or_else(|| {
                        format!("{}...{}", &pk[..16], &pk[pk.len().saturating_sub(8)..])
                    });
                println!("Producer Status");
                println!("{:-<60}", "");
                println!();
                println!("  Address: {}", pk_display);
                println!("  Status:  Not registered");
                println!();
                println!("If you recently registered, your registration may be pending");
                println!("activation at the next epoch boundary. Check again in a few");
                println!("minutes, or use 'doli producer register' to register.");
            } else {
                anyhow::bail!("Error: {}", e);
            }
        }
    }

    Ok(())
}

pub(super) async fn handle_bonds(
    wallet_path: &Path,
    rpc: &RpcClient,
    pubkey: Option<String>,
) -> Result<()> {
    let pk = match pubkey {
        Some(pk) => pk,
        None => {
            let wallet = Wallet::load(wallet_path)?;
            wallet.addresses()[0].public_key.clone()
        }
    };

    let details = rpc.get_bond_details(&pk).await?;
    let vq = details.vesting_quarter_slots;

    println!(
        "Bond Details ({} bonds, {} staked)",
        details.bond_count,
        format_balance(details.total_staked)
    );
    println!("{:-<60}", "");

    if details.bonds.is_empty() {
        println!("No bonds found.");
    } else {
        println!(
            " {:<4} {:<16} {:<12} {:<10} {:<8} Time to Next",
            "#", "Created (slot)", "Age", "Quarter", "Penalty"
        );
        println!(" {:-<70}", "");

        for (i, bond) in details.bonds.iter().enumerate() {
            let age_str = format_slot_duration(bond.age_slots);

            let quarter_label = if bond.vested {
                "Q4+".to_string()
            } else {
                let quarters = bond.age_slots / vq;
                format!("Q{}", quarters + 1)
            };

            let penalty_str = format!("{}%", bond.penalty_pct);

            let time_to_next = if bond.vested {
                "Fully vested".to_string()
            } else {
                // Slots until next quarter boundary
                let quarters_done = bond.age_slots / vq;
                let next_quarter_age = (quarters_done + 1) * vq;
                let slots_remaining = next_quarter_age.saturating_sub(bond.age_slots);
                let next_penalty = match quarters_done + 1 {
                    1 => "50%",
                    2 => "25%",
                    _ => "0%",
                };
                format!(
                    "{} to {}",
                    format_slot_duration(slots_remaining),
                    next_penalty
                )
            };

            println!(
                " {:<4} {:<16} {:<12} {:<10} {:<8} {}",
                i + 1,
                format!("slot {}", bond.creation_slot),
                age_str,
                quarter_label,
                penalty_str,
                time_to_next
            );
        }
    }

    if details.withdrawal_pending_count > 0 {
        println!();
        println!(
            "Withdrawal pending: {} bonds (applied at next epoch boundary)",
            details.withdrawal_pending_count
        );
    }

    Ok(())
}

pub(super) async fn handle_list(rpc: &RpcClient, active: bool, format: &str) -> Result<()> {
    match rpc.get_producers(active).await {
        Ok(producers) => {
            if producers.is_empty() {
                if format == "json" {
                    println!("[]");
                } else {
                    println!("No producers found.");
                }
                return Ok(());
            }

            match format {
                "json" => {
                    let json_producers: Vec<serde_json::Value> = producers
                        .iter()
                        .map(|p| {
                            let addr = hex::decode(&p.public_key)
                                .ok()
                                .and_then(|bytes| {
                                    crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                                })
                                .unwrap_or_else(|| p.public_key.clone());
                            serde_json::json!({
                                "status": p.status,
                                "address": addr,
                                "publicKey": p.public_key,
                                "bondCount": p.bond_count,
                                "bondAmount": p.bond_amount,
                                "era": p.era,
                                "registrationHeight": p.registration_height,
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json_producers)
                            .unwrap_or_else(|_| "[]".to_string())
                    );
                }
                "csv" => {
                    println!(
                        "status,address,public_key,bond_count,bond_amount,era,registration_height"
                    );
                    for p in &producers {
                        let addr = hex::decode(&p.public_key)
                            .ok()
                            .and_then(|bytes| {
                                crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                            })
                            .unwrap_or_else(|| p.public_key.clone());
                        println!(
                            "{},{},{},{},{},{},{}",
                            p.status,
                            addr,
                            p.public_key,
                            p.bond_count,
                            p.bond_amount,
                            p.era,
                            p.registration_height,
                        );
                    }
                }
                _ => {
                    // Default: table format
                    println!("Network Producers");
                    println!("{:-<80}", "");
                    println!();
                    println!(
                        "{:<10} {:<20} {:<10} {:<10}",
                        "Status", "Address", "Bonds", "Era"
                    );
                    println!("{:-<52}", "");

                    for p in &producers {
                        let addr_display = if let Ok(pubkey_bytes) = hex::decode(&p.public_key) {
                            crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
                                .map(|a| format!("{}...", &a[..16]))
                                .unwrap_or_else(|_| {
                                    format!(
                                        "{}...{}",
                                        &p.public_key[..8],
                                        &p.public_key[p.public_key.len() - 4..]
                                    )
                                })
                        } else {
                            format!(
                                "{}...{}",
                                &p.public_key[..8],
                                &p.public_key[p.public_key.len() - 4..]
                            )
                        };
                        println!(
                            "{:<10} {:<20} {:<10} {:<10}",
                            p.status, addr_display, p.bond_count, p.era
                        );
                    }

                    println!();
                    println!("Total: {} producers", producers.len());
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Error fetching producers: {}", e);
        }
    }

    Ok(())
}
