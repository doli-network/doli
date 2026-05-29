use std::path::Path;

use anyhow::Result;

use crate::commands::ChannelCommands;
use crate::common::address_prefix;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

// Write a cooperative-close offer with owner-only permissions on Unix.
#[cfg(unix)]
fn write_offer_file(path: &str, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(content.as_bytes())
}
#[cfg(not(unix))]
fn write_offer_file(path: &str, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

/// A payment channel requires two distinct funding parties (it is a 2-of-2
/// covenant). Opening a channel where the counterparty equals the local wallet
/// produces a degenerate self-funded channel that locks capacity for no purpose.
/// Reject it before any UTXO is selected.
fn ensure_distinct_channel_parties(local: &crypto::Hash, remote: &crypto::Hash) -> Result<()> {
    if local == remote {
        anyhow::bail!(
            "Cannot open a payment channel to yourself: the counterparty address \
             matches your own wallet address. A channel requires two distinct parties."
        );
    }
    Ok(())
}

pub(crate) async fn cmd_channel(
    wallet_path: &Path,
    rpc_endpoint: &str,
    network: &str,
    command: ChannelCommands,
) -> Result<()> {
    use channels::close::{
        build_cooperative_close_offer, finalize_cooperative_close_offer, CooperativeCloseOffer,
    };
    use channels::commitment::derive_channel_seed;
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

            // P1-007: a channel needs two distinct parties — reject self-channels.
            ensure_distinct_channel_parties(&local_hash, &remote_hash)?;

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
                tx.inputs[i].public_key = Some(*keypair.public_key());
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
            output,
            force,
        } => {
            if force {
                // Unilateral force-close requires trustless-channel machinery
                // (pre-signed commitment exchange, revocation, penalty/watchtower)
                // that this build intentionally does not ship. See the INC-I-093
                // roadmap note: trustless channels are a separate, economically
                // reviewed product decision, not a CLI gap.
                anyhow::bail!(
                    "Unilateral force-close is not supported in this build.\n\
                     DOLI settles on-chain in ~10s at a flat fee, so cooperative close \
                     is the supported path:\n\
                     \n\
                     \u{2022} You:          doli channel close {ch} -o close.json\n\
                     \u{2022} Counterparty: doli channel close-finish close.json\n\
                     \n\
                     Trustless force-close (pre-signed commitments, penalty/watchtower) \
                     is a roadmap item gated on a concrete use case + economic review.",
                    ch = channel
                );
            }

            let wallet = Wallet::load(wallet_path)?;

            let store = ChannelStore::open(&store_path)?;

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

            // Cooperative close, step 1: build the close tx and sign our half of
            // the 2-of-2 funding covenant, then emit a portable offer file.
            let keypair = wallet.primary_keypair()?;
            let offer = build_cooperative_close_offer(
                &ch.channel_id,
                funding_hash,
                ch.funding_outpoint.output_index,
                local_hash,
                remote_hash,
                &ch.balance,
                ch.capacity,
                fee_units,
                &keypair,
            )
            .map_err(|e| anyhow::anyhow!("Failed to build cooperative-close offer: {}", e))?;

            let out_path =
                output.unwrap_or_else(|| format!("close-{}.json", ch.channel_id.short()));
            let offer_json = serde_json::to_string_pretty(&offer)?;
            write_offer_file(&out_path, &offer_json)?;

            // Mark the channel as cooperatively closing (final close happens once
            // the counterparty co-signs and broadcasts).
            let mut store = store;
            if let Some(ch_mut) = store.find_mut(&ch.channel_id) {
                let _ = ch_mut.transition(ChannelState::CooperativeClosing);
                store.save()?;
            }

            println!(
                "Cooperative close offer created for channel {}:",
                ch.channel_id
            );
            println!(
                "  Local payout:  {}",
                format_balance(ch.balance.local.saturating_sub(fee_units))
            );
            println!("  Remote payout: {}", format_balance(ch.balance.remote));
            println!("  Fee:           {}", format_balance(fee_units));
            println!("  Saved to:      {}", out_path);
            println!();
            println!("Send this file to your counterparty. They complete the close with:");
            println!("  doli channel close-finish {}", out_path);
        }

        ChannelCommands::CloseFinish { file } => {
            let wallet = Wallet::load(wallet_path)?;
            let rpc = RpcClient::new(rpc_endpoint);

            if !rpc.ping().await? {
                anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
            }

            let offer_json = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("Cannot read offer file {}: {}", file, e))?;
            let offer: CooperativeCloseOffer = serde_json::from_str(&offer_json)
                .map_err(|e| anyhow::anyhow!("Invalid cooperative-close offer file: {}", e))?;

            // Co-sign: verifies the initiator's signature over the exact tx and
            // completes the 2-of-2 covenant witness.
            let keypair = wallet.primary_keypair()?;
            let tx = finalize_cooperative_close_offer(&offer, &keypair)
                .map_err(|e| anyhow::anyhow!("Failed to finalize cooperative close: {}", e))?;

            let tx_hex = hex::encode(tx.serialize());
            let tx_hash = tx.hash();

            println!(
                "Finalizing cooperative close for channel {}:",
                offer.channel_id
            );
            println!("  Close TX: {}", tx_hash.to_hex());
            println!();
            println!("Broadcasting...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(result_hash) => {
                    println!("Channel closed cooperatively. TX: {}", result_hash);
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("MPTX007") {
                        anyhow::bail!(
                            "Broadcast rejected [MPTX007] — the 2-of-2 funding covenant was not \
                             satisfied.\nThis usually means the wrong wallet finalized the offer: \
                             only the channel's two funding parties can co-sign a cooperative \
                             close. Verify you are using the counterparty wallet for this channel.\n\
                             (node error: {})",
                            msg
                        );
                    }
                    anyhow::bail!("Failed to broadcast: {}", e);
                }
            }

            // Mark closed locally if we have a record for this channel.
            let mut store = ChannelStore::open(&store_path)?;
            if let Some(ch) = store
                .all_channels()
                .iter()
                .find(|c| c.channel_id.to_hex() == offer.channel_id)
                .map(|c| c.channel_id.clone())
            {
                if let Some(ch_mut) = store.find_mut(&ch) {
                    let _ = ch_mut.transition(ChannelState::Closed);
                    ch_mut.close_tx_hash = Some(tx_hash.to_hex());
                    store.save()?;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::Hash;

    // OUTPUT CONTRACT: fn ensure_distinct_channel_parties(local, remote) -> Result<()>
    // O1: Err  when local == remote (self-channel)
    // O2: Ok   when local != remote
    // PATHS: P1 identical, P2 distinct
    #[test]
    fn p1_007_rejects_self_channel() {
        let me = Hash::from_bytes([7u8; 32]);
        let other = Hash::from_bytes([9u8; 32]);
        // P1: counterparty == self -> rejected
        assert!(ensure_distinct_channel_parties(&me, &me).is_err());
        // P2: distinct parties -> accepted
        assert!(ensure_distinct_channel_parties(&me, &other).is_ok());
    }
}
