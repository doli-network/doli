use std::path::Path;

use anyhow::Result;

use crate::commands::ChannelCommands;
use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_channel(
    wallet_path: &Path,
    rpc_endpoint: &str,
    network: &str,
    command: ChannelCommands,
) -> Result<()> {
    use channels::close::{build_cooperative_close, build_force_close};
    use channels::commitment::{derive_channel_seed, CommitmentPair};
    use channels::config::ChannelConfig;
    use channels::funding::build_funding_tx_with_change;
    use channels::store::ChannelStore;
    use channels::types::{ChannelBalance, ChannelId, ChannelState, FundingOutpoint};
    use channels::ChannelRecord;
    use crypto::{signature, Hash};

    let config = match network {
        "testnet" => ChannelConfig::testnet(rpc_endpoint),
        _ => ChannelConfig::mainnet(rpc_endpoint),
    };

    // Channel store lives next to wallet file
    let store_path = Path::new(wallet_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(&config.store_path);

    match command {
        ChannelCommands::Open {
            peer,
            capacity,
            fee,
        } => {
            let wallet = Wallet::load(wallet_path)?;
            let rpc = RpcClient::new(rpc_endpoint);

            if !rpc.ping().await? {
                anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
            }

            // Parse counterparty address
            let remote_hash = crypto::address::resolve(&peer, None)
                .map_err(|e| anyhow::anyhow!("Invalid peer address: {}", e))?;

            // Parse capacity
            let capacity_units = coins_to_units(&capacity)
                .map_err(|e| anyhow::anyhow!("Invalid capacity: {}", e))?;
            if capacity_units == 0 {
                anyhow::bail!("Capacity must be greater than zero");
            }

            if capacity_units < config.min_channel_capacity {
                anyhow::bail!(
                    "Minimum channel capacity is {}",
                    format_balance(config.min_channel_capacity)
                );
            }

            // Parse fee
            let fee_units = if let Some(f) = &fee {
                coins_to_units(f).map_err(|e| anyhow::anyhow!("Invalid fee: {}", e))?
            } else {
                1u64 // Flat fee: 1 satoshi
            };

            // Get spendable UTXOs
            let from_pubkey_hash = wallet.primary_pubkey_hash();
            let utxos: Vec<_> = rpc
                .get_utxos(&from_pubkey_hash, true)
                .await?
                .into_iter()
                .filter(|u| u.output_type == "normal" && u.spendable)
                .collect();

            if utxos.is_empty() {
                anyhow::bail!("No spendable UTXOs available");
            }

            // Select UTXOs
            let required = capacity_units + fee_units;
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
                    "Insufficient balance. Available: {}, Required: {} (capacity) + {} (fee)",
                    format_balance(total_input),
                    format_balance(capacity_units),
                    format_balance(fee_units)
                );
            }

            let local_hash = Hash::from_hex(&from_pubkey_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid local pubkey hash"))?;

            // Build inputs with amounts for funding
            let inputs_with_amounts: Vec<(Hash, u32, u64)> = selected
                .iter()
                .map(|u| {
                    let h = Hash::from_hex(&u.tx_hash)
                        .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"));
                    h.map(|hash| (hash, u.output_index, u.amount))
                })
                .collect::<Result<Vec<_>>>()?;

            println!("Opening payment channel:");
            println!("  Peer:     {}", peer);
            println!("  Capacity: {} DOLI", format_balance(capacity_units));
            println!("  Fee:      {}", format_balance(fee_units));
            println!("  Inputs:   {} UTXO(s)", selected.len());

            let change = total_input - required;
            if change > 0 {
                println!("  Change:   {}", format_balance(change));
            }

            // Build funding tx
            let mut tx = build_funding_tx_with_change(
                inputs_with_amounts,
                local_hash,
                remote_hash,
                capacity_units,
                fee_units,
                local_hash, // change goes back to us
            )
            .map_err(|e| anyhow::anyhow!("Failed to build funding tx: {}", e))?;

            // Sign each input
            let keypair = wallet.primary_keypair()?;
            for i in 0..tx.inputs.len() {
                let signing_hash = tx.signing_message_for_input(i);
                tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
            }

            let tx_bytes = tx.serialize();
            let tx_hex = hex::encode(&tx_bytes);
            let tx_hash = tx.hash();

            println!();
            println!("Funding TX hash: {}", tx_hash.to_hex());
            println!("Broadcasting...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(result_hash) => {
                    println!("Funding TX submitted: {}", result_hash);
                }
                Err(e) => {
                    anyhow::bail!("Failed to broadcast funding TX: {}", e);
                }
            }

            // Create channel record
            let channel_id = ChannelId::from_funding_outpoint(&tx_hash, 0);
            let channel_seed = derive_channel_seed(&keypair, &channel_id.0);

            let record = ChannelRecord {
                channel_id: channel_id.clone(),
                state: ChannelState::FundingBroadcast,
                local_pubkey_hash: *local_hash.as_bytes(),
                remote_pubkey_hash: *remote_hash.as_bytes(),
                funding_outpoint: FundingOutpoint {
                    tx_hash: *tx_hash.as_bytes(),
                    output_index: 0,
                },
                capacity: capacity_units,
                balance: ChannelBalance::new(capacity_units, 0),
                commitment_number: 0,
                channel_seed,
                revocation_store: Default::default(),
                dispute_window: config.dispute_window,
                htlcs: Vec::new(),
                funding_confirmations: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                close_tx_hash: None,
                penalty_tx_hash: None,
            };

            let mut store = ChannelStore::open(&store_path)?;
            store.add(record);
            store.save()?;

            println!();
            println!("Channel opened: {}", channel_id);
            println!(
                "Waiting for {} confirmation(s)...",
                config.funding_confirmations
            );
            println!("Store: {}", store_path.display());
        }

        ChannelCommands::Pay { channel, amount } => {
            let amount_units =
                coins_to_units(&amount).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
            if amount_units == 0 {
                anyhow::bail!("Amount must be greater than zero");
            }

            let mut store = ChannelStore::open(&store_path)?;

            // Find channel by prefix match
            let ch = store
                .all_channels()
                .iter()
                .find(|c| c.channel_id.to_hex().starts_with(&channel))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Channel not found: {}", channel))?;

            if !ch.state.is_active() {
                anyhow::bail!(
                    "Channel {} is not active (state: {})",
                    ch.channel_id,
                    ch.state
                );
            }

            if amount_units > ch.balance.local {
                anyhow::bail!(
                    "Insufficient channel balance. Local: {}, Requested: {}",
                    format_balance(ch.balance.local),
                    format_balance(amount_units)
                );
            }

            let new_balance = ch
                .balance
                .pay_local_to_remote(amount_units)
                .ok_or_else(|| anyhow::anyhow!("Payment exceeds local balance"))?;

            // Update channel in store
            let ch_mut = store
                .find_mut(&ch.channel_id)
                .ok_or_else(|| anyhow::anyhow!("Channel disappeared"))?;
            ch_mut.update_balance(new_balance.clone());
            ch_mut.advance_commitment();
            store.save()?;

            println!("Payment sent through channel {}:", ch.channel_id);
            println!("  Amount:  {} DOLI", format_balance(amount_units));
            println!("  Local:   {}", format_balance(new_balance.local));
            println!("  Remote:  {}", format_balance(new_balance.remote));
            println!("  Commit#: {}", ch.commitment_number + 1);
        }

        ChannelCommands::Close {
            channel,
            fee,
            force,
        } => {
            let wallet = Wallet::load(wallet_path)?;
            let rpc = RpcClient::new(rpc_endpoint);

            if !rpc.ping().await? {
                anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
            }

            let mut store = ChannelStore::open(&store_path)?;

            let ch = store
                .all_channels()
                .iter()
                .find(|c| c.channel_id.to_hex().starts_with(&channel))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Channel not found: {}", channel))?;

            if ch.state.is_terminal() {
                anyhow::bail!("Channel {} is already closed", ch.channel_id);
            }

            let fee_units = if let Some(f) = &fee {
                coins_to_units(f).map_err(|e| anyhow::anyhow!("Invalid fee: {}", e))?
            } else {
                1u64 // Flat fee: 1 satoshi
            };

            let local_hash = Hash::from_bytes(ch.local_pubkey_hash);
            let remote_hash = Hash::from_bytes(ch.remote_pubkey_hash);
            let funding_hash = ch.funding_outpoint.tx_hash_as_crypto();

            if force {
                // Force close: broadcast latest commitment tx
                let keypair = wallet.primary_keypair()?;
                let channel_seed = derive_channel_seed(&keypair, &ch.channel_id.0);

                let rpc_client = channels::rpc::RpcClient::new(rpc_endpoint);
                let current_height = rpc_client
                    .get_height()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get chain height: {}", e))?;
                let dispute_height = current_height + ch.dispute_window;

                let commitment =
                    CommitmentPair::new(ch.commitment_number, ch.balance.clone(), &channel_seed);

                let mut tx = build_force_close(
                    &commitment,
                    funding_hash,
                    ch.funding_outpoint.output_index,
                    local_hash,
                    remote_hash,
                    dispute_height,
                    ch.capacity,
                    fee_units,
                )
                .map_err(|e| anyhow::anyhow!("Failed to build force close: {}", e))?;

                // Sign
                for i in 0..tx.inputs.len() {
                    let signing_hash = tx.signing_message_for_input(i);
                    tx.inputs[i].signature =
                        signature::sign_hash(&signing_hash, keypair.private_key());
                }

                let tx_hex = hex::encode(tx.serialize());
                let tx_hash = tx.hash();

                println!("Force closing channel {}:", ch.channel_id);
                println!("  Dispute window: {} blocks", ch.dispute_window);

                match rpc.send_transaction(&tx_hex).await {
                    Ok(result_hash) => {
                        println!("  Commitment TX: {}", result_hash);
                    }
                    Err(e) => {
                        anyhow::bail!("Failed to broadcast: {}", e);
                    }
                }

                let ch_mut = store
                    .find_mut(&ch.channel_id)
                    .ok_or_else(|| anyhow::anyhow!("Channel disappeared"))?;
                ch_mut.transition(ChannelState::ForceClosing);
                ch_mut.close_tx_hash = Some(tx_hash.to_hex());
                store.save()?;
            } else {
                // Cooperative close
                let mut tx = build_cooperative_close(
                    funding_hash,
                    ch.funding_outpoint.output_index,
                    local_hash,
                    remote_hash,
                    &ch.balance,
                    ch.capacity,
                    fee_units,
                )
                .map_err(|e| anyhow::anyhow!("Failed to build cooperative close: {}", e))?;

                // Sign
                let keypair = wallet.primary_keypair()?;
                for i in 0..tx.inputs.len() {
                    let signing_hash = tx.signing_message_for_input(i);
                    tx.inputs[i].signature =
                        signature::sign_hash(&signing_hash, keypair.private_key());
                }

                let tx_hex = hex::encode(tx.serialize());
                let tx_hash = tx.hash();

                println!("Closing channel {} cooperatively:", ch.channel_id);
                println!(
                    "  Local payout:  {}",
                    format_balance(ch.balance.local.saturating_sub(fee_units))
                );
                println!("  Remote payout: {}", format_balance(ch.balance.remote));
                println!("  Fee:           {}", format_balance(fee_units));

                match rpc.send_transaction(&tx_hex).await {
                    Ok(result_hash) => {
                        println!("  Close TX: {}", result_hash);
                    }
                    Err(e) => {
                        anyhow::bail!("Failed to broadcast: {}", e);
                    }
                }

                let ch_mut = store
                    .find_mut(&ch.channel_id)
                    .ok_or_else(|| anyhow::anyhow!("Channel disappeared"))?;
                ch_mut.transition(ChannelState::Closed);
                ch_mut.close_tx_hash = Some(tx_hash.to_hex());
                store.save()?;
            }
        }

        ChannelCommands::List { all } => {
            let store = ChannelStore::open(&store_path)?;
            let channels: Vec<_> = if all {
                store.all_channels().to_vec()
            } else {
                store.active_channels().into_iter().cloned().collect()
            };

            if channels.is_empty() {
                println!("No channels found.");
                if !all {
                    println!("Use --all to include closed channels.");
                }
                return Ok(());
            }

            println!(
                "{:<18} {:<18} {:<14} {:<14} {:<10}",
                "CHANNEL", "STATE", "LOCAL", "REMOTE", "CAPACITY"
            );
            println!("{}", "-".repeat(74));

            for ch in &channels {
                println!(
                    "{:<18} {:<18} {:<14} {:<14} {:<10}",
                    ch.channel_id.short(),
                    ch.state.to_string(),
                    format_balance(ch.balance.local),
                    format_balance(ch.balance.remote),
                    format_balance(ch.capacity),
                );
            }

            println!();
            println!(
                "{} channel(s) ({})",
                channels.len(),
                if all { "all" } else { "active" }
            );
        }

        ChannelCommands::Info { channel } => {
            let store = ChannelStore::open(&store_path)?;

            let ch = store
                .all_channels()
                .iter()
                .find(|c| c.channel_id.to_hex().starts_with(&channel))
                .ok_or_else(|| anyhow::anyhow!("Channel not found: {}", channel))?;

            let local_addr =
                crypto::address::encode(&Hash::from_bytes(ch.local_pubkey_hash), address_prefix())
                    .unwrap_or_else(|_| hex::encode(ch.local_pubkey_hash));

            let remote_addr =
                crypto::address::encode(&Hash::from_bytes(ch.remote_pubkey_hash), address_prefix())
                    .unwrap_or_else(|_| hex::encode(ch.remote_pubkey_hash));

            println!("Channel: {}", ch.channel_id.to_hex());
            println!("State:   {}", ch.state);
            println!("Local:   {}", local_addr);
            println!("Remote:  {}", remote_addr);
            println!();
            println!("Capacity:    {}", format_balance(ch.capacity));
            println!("Local bal:   {}", format_balance(ch.balance.local));
            println!("Remote bal:  {}", format_balance(ch.balance.remote));
            println!("Commit #:    {}", ch.commitment_number);
            println!();
            println!(
                "Funding:     {}:{}",
                hex::encode(ch.funding_outpoint.tx_hash),
                ch.funding_outpoint.output_index
            );
            println!("Dispute:     {} blocks", ch.dispute_window);
            println!("HTLCs:       {}", ch.htlcs.len());
            println!(
                "Created:     {}",
                ch.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            );

            if let Some(ref close_hash) = ch.close_tx_hash {
                println!("Close TX:    {}", close_hash);
            }
            if let Some(ref penalty_hash) = ch.penalty_tx_hash {
                println!("Penalty TX:  {}", penalty_hash);
            }
        }
    }

    Ok(())
}
