use std::path::Path;

use anyhow::Result;

use crate::commands::ProducerCommands;
use crate::common::address_prefix;
use crate::rpc_client::{format_balance, BondDetailsInfo, RpcClient};
use crate::wallet::Wallet;

/// Format a slot duration as human-readable time (SLOT_DURATION = 10s)
pub(crate) fn format_slot_duration(slots: u64) -> String {
    let seconds = slots * 10;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("~{}h {}m", hours, minutes)
    } else {
        format!("~{}m", minutes)
    }
}

/// FIFO breakdown tier: (count, penalty_pct, gross_amount, net_amount)
struct FifoBreakdown {
    total_net: u64,
    total_penalty: u64,
    tiers: Vec<(u32, u8, u64, u64)>,
}

/// Compute FIFO breakdown for withdrawing `count` bonds (oldest first)
fn compute_fifo_breakdown(details: &BondDetailsInfo, count: u32) -> FifoBreakdown {
    let mut total_net: u64 = 0;
    let mut total_penalty: u64 = 0;
    let mut tiers: Vec<(u32, u8, u64, u64)> = Vec::new();

    let mut current_tier_pct: Option<u8> = None;
    let mut tier_count: u32 = 0;
    let mut tier_gross: u64 = 0;
    let mut tier_net: u64 = 0;

    for entry in details.bonds.iter().take(count as usize) {
        let pct = entry.penalty_pct;
        let penalty = (entry.amount * pct as u64) / 100;
        let net = entry.amount - penalty;
        total_net += net;
        total_penalty += penalty;

        if current_tier_pct == Some(pct) {
            tier_count += 1;
            tier_gross += entry.amount;
            tier_net += net;
        } else {
            if let Some(prev_pct) = current_tier_pct {
                tiers.push((tier_count, prev_pct, tier_gross, tier_net));
            }
            current_tier_pct = Some(pct);
            tier_count = 1;
            tier_gross = entry.amount;
            tier_net = net;
        }
    }
    if let Some(pct) = current_tier_pct {
        tiers.push((tier_count, pct, tier_gross, tier_net));
    }

    FifoBreakdown {
        total_net,
        total_penalty,
        tiers,
    }
}

/// Display FIFO breakdown table
fn display_fifo_breakdown(breakdown: &FifoBreakdown) {
    for (cnt, pct, gross, net) in &breakdown.tiers {
        let tier_label = match pct {
            0 => "vested (0% penalty)".to_string(),
            p => format!("Q{} ({}% penalty)", (4 - p / 25), p),
        };
        println!(
            "  {} x {}: {} -> {} ({} burned)",
            cnt,
            tier_label,
            format_balance(*gross),
            format_balance(*net),
            format_balance(gross - net)
        );
    }
    if breakdown.tiers.len() > 1 {
        let total_gross = breakdown.total_net + breakdown.total_penalty;
        println!("  {:-<50}", "");
        println!(
            "  Total: {} -> {} ({} burned)",
            format_balance(total_gross),
            format_balance(breakdown.total_net),
            format_balance(breakdown.total_penalty)
        );
    }
}

pub(crate) async fn cmd_producer(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: ProducerCommands,
) -> Result<()> {
    use crypto::{bls_sign_pop, signature, BlsSecretKey, Hash, PublicKey};
    use doli_core::{Input, Output, Transaction};

    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        ProducerCommands::Register { bonds } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Producer Registration");
            println!("{:-<60}", "");
            println!();

            // Validate bond count (WHITEPAPER Section 6.3: max 1000 bonds)
            if !(1..=10000).contains(&bonds) {
                anyhow::bail!("Bond count must be between 1 and 10000");
            }

            // Check if already registered or pending (WHITEPAPER: "public key is not already registered")
            let pk_hex = &wallet.addresses()[0].public_key;
            if let Ok(info) = rpc.get_producer(pk_hex).await {
                match info.status.to_lowercase().as_str() {
                    "active" => {
                        anyhow::bail!("This key is already registered as an active producer (pubkey: {}, bonds: {}). Use 'doli producer add-bond' to increase your bond count.", pk_hex, info.bond_count);
                    }
                    "pending" => {
                        anyhow::bail!("This key already has a pending registration (pubkey: {}). It will activate at the next epoch boundary.", pk_hex);
                    }
                    _ => {} // exited/slashed — allow re-registration
                }
            }

            // Get network parameters from node (bond_unit is network-specific)
            let chain_info = rpc.get_chain_info().await?;
            let network_params = rpc.get_network_params().await?;
            let bond_unit = network_params.bond_unit;
            let bond_display = bond_unit / 100_000_000; // Convert to DOLI per bond
            let required_amount = bond_unit * bonds as u64;

            println!(
                "Registering with {} bond(s) = {} DOLI",
                bonds,
                bonds as u64 * bond_display
            );
            println!();

            // Get spendable normal UTXOs (exclude bonds, conditioned, NFTs, tokens, etc.)
            let utxos: Vec<_> = rpc
                .get_utxos(&pubkey_hash, true)
                .await?
                .into_iter()
                .filter(|u| u.output_type == "normal" && u.spendable)
                .collect();
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            // Output count: bond_count bonds + 1 change
            let num_outputs = bonds as u64 + 1;

            // Initial UTXO selection with minimum fee estimate
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            let mut fee = 1000u64.max((utxos.len() as u64 + num_outputs) * 500);
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
            }

            // Auto-calculate fee: max(1000, (inputs + outputs) * 500) — ensures fee_rate >= 1 sat/byte
            fee = 1000u64.max((selected_utxos.len() as u64 + num_outputs) * 500);

            // Re-select if fee increased the requirement
            if total_input < required_amount + fee {
                selected_utxos.clear();
                total_input = 0;
                for utxo in &utxos {
                    selected_utxos.push(utxo.clone());
                    total_input += utxo.amount;
                    fee = 1000u64.max((selected_utxos.len() as u64 + num_outputs) * 500);
                    if total_input >= required_amount + fee {
                        break;
                    }
                }
            }

            if total_available < required_amount + fee {
                anyhow::bail!(
                    "Insufficient balance for bond. Required: {} DOLI + {} fee, Available: {}",
                    bonds as u64 * bond_display,
                    format_balance(fee),
                    format_balance(total_available)
                );
            }

            // Build registration transaction
            let mut inputs: Vec<Input> = Vec::new();
            for utxo in &selected_utxos {
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
                inputs.push(Input::new(prev_tx_hash, utxo.output_index));
            }

            // Parse public key for registration
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Calculate lock_until based on network
            // Lock must be at least current_height + blocks_per_era
            let blocks_per_era: u64 = match chain_info.network.as_str() {
                "devnet" => 576, // ~10 minutes at 1s slots
                _ => 12_614_400, // ~4 years at 10s slots (mainnet/testnet)
            };
            let lock_until = chain_info.best_height + blocks_per_era + 1000;

            // Compute hash-chain VDF proof for registration (~5 seconds)
            let current_epoch = (chain_info.best_slot / 360) as u32;
            let vdf_input = vdf::registration_input(&producer_pubkey, current_epoch);
            let vdf_iterations = vdf::T_REGISTER_BASE;
            println!(
                "Computing registration VDF ({} iterations)...",
                vdf_iterations
            );
            let vdf_output = doli_core::tpop::heartbeat::hash_chain_vdf(&vdf_input, vdf_iterations);
            println!("VDF proof computed.");
            println!();

            // Create registration transaction with bonds and hash-chain VDF output
            // Hash-chain VDF is self-verifying (recompute to verify), no separate proof needed
            let reg_data = doli_core::transaction::RegistrationData {
                public_key: producer_pubkey,
                epoch: current_epoch,
                vdf_output: vdf_output.to_vec(),
                vdf_proof: vec![], // Hash-chain VDF: no separate proof, output IS the proof
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: 0,
                bond_count: bonds,
                bls_pubkey: {
                    let bls_priv_hex =
                        wallet.addresses()[0]
                            .bls_private_key
                            .as_ref()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Wallet has no BLS key. Run: doli --wallet <wallet> add-bls"
                                )
                            })?;
                    let bls_sk = BlsSecretKey::from_hex(bls_priv_hex)
                        .map_err(|e| anyhow::anyhow!("Invalid BLS secret key: {}", e))?;
                    let bls_pk = bls_sk.public_key();
                    bls_pk.as_bytes().to_vec()
                },
                bls_pop: {
                    let bls_priv_hex = wallet.addresses()[0].bls_private_key.as_ref().unwrap(); // already validated above
                    let bls_sk = BlsSecretKey::from_hex(bls_priv_hex).unwrap();
                    let bls_pk = bls_sk.public_key();
                    let pop = bls_sign_pop(&bls_sk, &bls_pk)
                        .map_err(|e| anyhow::anyhow!("Failed to generate BLS PoP: {}", e))?;
                    pop.as_bytes().to_vec()
                },
            };
            let extra_data = bincode::serialize(&reg_data)
                .map_err(|e| anyhow::anyhow!("Failed to serialize registration data: {}", e))?;

            // Create one Bond UTXO per bond (each with bond_unit amount)
            let pubkey_hash_for_bond =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer_pubkey.as_bytes());
            let outputs: Vec<Output> = (0..bonds)
                .map(|_| {
                    doli_core::transaction::Output::bond(
                        bond_unit,
                        pubkey_hash_for_bond,
                        lock_until,
                        0, // creation_slot stamped by node at block application
                    )
                })
                .collect();

            let mut tx = Transaction {
                version: 1,
                tx_type: doli_core::transaction::TxType::Registration,
                inputs,
                outputs,
                extra_data,
            };

            // Add change output if needed
            let change = total_input - required_amount - fee;
            if change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
                tx.outputs.push(Output::normal(change, change_hash));
            }

            // Sign each input (BIP-143: per-input signing hash)
            for i in 0..tx.inputs.len() {
                let signing_hash = tx.signing_message_for_input(i);
                tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
            }

            // Submit transaction
            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting registration transaction...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Registration submitted successfully!");
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
                    println!("Use 'doli producer status' to check registration status.");
                }
                Err(e) => {
                    anyhow::bail!("Error submitting registration: {}", e);
                }
            }
        }

        ProducerCommands::Status { pubkey } => {
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
                        .and_then(|bytes| {
                            crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                        })
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
                                    oldest_q1 = Some(
                                        oldest_q1.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
                                }
                                1 => {
                                    oldest_q2 = Some(
                                        oldest_q2.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
                                }
                                2 => {
                                    oldest_q3 = Some(
                                        oldest_q3.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
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
                            .and_then(|bytes| {
                                crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                            })
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
        }

        ProducerCommands::Bonds { pubkey } => {
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
        }

        ProducerCommands::List { active } => {
            println!("Network Producers");
            println!("{:-<80}", "");
            println!();

            match rpc.get_producers(active).await {
                Ok(producers) => {
                    if producers.is_empty() {
                        println!("No producers found.");
                    } else {
                        println!(
                            "{:<10} {:<20} {:<10} {:<10}",
                            "Status", "Address", "Bonds", "Era"
                        );
                        println!("{:-<52}", "");

                        for p in &producers {
                            let addr_display = if let Ok(pubkey_bytes) = hex::decode(&p.public_key)
                            {
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
                Err(e) => {
                    anyhow::bail!("Error fetching producers: {}", e);
                }
            }
        }

        ProducerCommands::AddBond { count } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

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
                .get_utxos(&pubkey_hash, true)
                .await?
                .into_iter()
                .filter(|u| u.output_type == "normal" && u.spendable)
                .collect();
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            // Output count: bond_count bonds + 1 change
            let num_outputs = count as u64 + 1;

            // Initial UTXO selection with minimum fee estimate
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            let mut fee = 1000u64.max((utxos.len() as u64 + num_outputs) * 500);
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
            }

            // Auto-calculate fee: max(1000, (inputs + outputs) * 500) — ensures fee_rate >= 1 sat/byte
            fee = 1000u64.max((selected_utxos.len() as u64 + num_outputs) * 500);

            // Re-select if fee increased the requirement
            if total_input < required_amount + fee {
                selected_utxos.clear();
                total_input = 0;
                for utxo in &utxos {
                    selected_utxos.push(utxo.clone());
                    total_input += utxo.amount;
                    fee = 1000u64.max((selected_utxos.len() as u64 + num_outputs) * 500);
                    if total_input >= required_amount + fee {
                        break;
                    }
                }
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
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
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
            let mut tx = Transaction::new_add_bond(
                inputs,
                producer_pubkey,
                count,
                required_amount,
                lock_until,
            );

            // Add change output
            let change = total_input - required_amount - fee;
            if change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
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
        }

        ProducerCommands::RequestWithdrawal { count, destination } => {
            let wallet = Wallet::load(wallet_path)?;
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
                None => Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?,
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
                let pct = (breakdown.total_penalty * 100)
                    / (breakdown.total_net + breakdown.total_penalty);
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
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
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
        }

        ProducerCommands::SimulateWithdrawal { count } => {
            let wallet = Wallet::load(wallet_path)?;

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
        }

        ProducerCommands::Exit { force } => {
            let wallet = Wallet::load(wallet_path)?;

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
                let pct = (breakdown.total_penalty * 100)
                    / (breakdown.total_net + breakdown.total_penalty);
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
            let dest_hash = Hash::from_hex(&pubkey_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?;

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
            let fee_estimate = 1000u64.max((bond_inputs.len() as u64 + 2) * 500);
            for utxo in &normal_utxos {
                if fee_input_total >= fee_estimate {
                    break;
                }
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
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
        }

        ProducerCommands::Slash { block1, block2 } => {
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

            println!(
                "Note: Slashing evidence submission requires full block headers with VDF proofs."
            );
            println!();
            println!("Nodes automatically detect and submit slashing evidence when they");
            println!("receive conflicting blocks for the same slot. If you have raw block");
            println!("data with VDF proofs, use the node's internal submission mechanism.");
            println!();
            println!("The equivocating producer ({}) will be:", producer_addr);
            println!("  - Have their entire bond burned (100% penalty)");
            println!("  - Excluded from the producer set immediately");
            println!("  - Can re-register like any new producer (standard registration VDF)");
        }
    }

    Ok(())
}
