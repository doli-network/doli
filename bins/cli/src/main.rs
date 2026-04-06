//! DOLI CLI - Wallet command-line interface
//!
//! This binary provides wallet functionality:
//! - Key generation and management
//! - Transaction creation and signing
//! - Balance queries
//! - Network interaction

use anyhow::Result;
use clap::Parser;

mod cmd_bridge;
mod cmd_chain;
mod cmd_channel;
mod cmd_governance;
mod cmd_guardian;
mod cmd_init;
mod cmd_loan;
mod cmd_nft;
mod cmd_pool;
mod cmd_producer;
mod cmd_service;
mod cmd_snap;
mod cmd_token;
mod cmd_upgrade;
mod cmd_wallet;
mod commands;
mod common;
mod parsers;
mod paths;
mod rpc_client;
mod wallet;

use commands::{Cli, Commands};
use common::{default_rpc_for_network, expand_tilde, prefix_for_network, ADDRESS_PREFIX};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve wallet path: -w flag > DOLI_WALLET_FILE env > paths::resolve_wallet_path()
    let wallet = match &cli.wallet {
        Some(w) => expand_tilde(w),
        None => paths::resolve_wallet_path(&cli.network, None, None),
    };

    // Set address prefix and network name from --network flag
    let _ = ADDRESS_PREFIX.set(prefix_for_network(&cli.network).to_string());
    let _ = common::NETWORK.set(cli.network.clone());

    // Resolve RPC endpoint: explicit flag > network default
    let rpc_endpoint = cli
        .rpc
        .unwrap_or_else(|| default_rpc_for_network(&cli.network).to_string());

    match cli.command {
        Commands::Init {
            force,
            non_producer,
        } => {
            cmd_init::cmd_init(&cli.network, &wallet, force, non_producer)?;
        }
        Commands::New { name } => {
            cmd_wallet::cmd_new(&wallet, name)?;
        }
        Commands::Restore { name } => {
            cmd_wallet::cmd_restore(&wallet, name)?;
        }
        Commands::Address { label } => {
            cmd_wallet::cmd_address(&wallet, label)?;
        }
        Commands::Addresses => {
            cmd_wallet::cmd_addresses(&wallet)?;
        }
        Commands::Balance { address, all } => {
            cmd_wallet::cmd_balance(&wallet, &rpc_endpoint, address, all).await?;
        }
        Commands::Send {
            to,
            amount,
            fee,
            condition,
            yes,
        } => {
            cmd_wallet::cmd_send(&wallet, &rpc_endpoint, &to, &amount, fee, condition, yes).await?;
        }
        Commands::Spend {
            utxo,
            to,
            amount,
            witness,
            fee,
        } => {
            cmd_wallet::cmd_spend(&wallet, &rpc_endpoint, &utxo, &to, &amount, &witness, fee)
                .await?;
        }
        Commands::History { limit } => {
            cmd_wallet::cmd_history(&wallet, &rpc_endpoint, limit).await?;
        }
        Commands::Export { output } => {
            cmd_wallet::cmd_export(&wallet, &output)?;
        }
        Commands::Import { input } => {
            cmd_wallet::cmd_import(&wallet, &input)?;
        }
        Commands::Info => {
            cmd_wallet::cmd_info(&wallet)?;
        }
        Commands::AddBls => {
            cmd_wallet::cmd_add_bls(&wallet)?;
        }
        Commands::Sign { message, address } => {
            cmd_wallet::cmd_sign(&wallet, &message, address)?;
        }
        Commands::Verify {
            message,
            signature,
            pubkey,
        } => {
            cmd_wallet::cmd_verify(&message, &signature, &pubkey)?;
        }
        Commands::Producer { command } => {
            cmd_producer::cmd_producer(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Rewards { command } => {
            cmd_chain::cmd_rewards(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Chain => {
            cmd_chain::cmd_chain(&rpc_endpoint).await?;
        }
        Commands::ChainVerify => {
            cmd_chain::cmd_chain_verify(&rpc_endpoint).await?;
        }
        Commands::Update { command } => {
            cmd_governance::cmd_update(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Maintainer { command } => {
            cmd_governance::cmd_maintainer(&rpc_endpoint, command).await?;
        }
        Commands::Upgrade {
            version,
            yes,
            doli_node_path,
            service,
        } => {
            cmd_upgrade::cmd_upgrade(version, yes, doli_node_path, service).await?;
        }
        Commands::Release { command } => {
            cmd_governance::cmd_release(&wallet, command).await?;
        }
        Commands::Protocol { command } => {
            cmd_governance::cmd_protocol(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Pool { command } => {
            cmd_pool::cmd_pool(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Loan { command } => {
            cmd_loan::cmd_loan(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Channel { command } => {
            cmd_channel::cmd_channel(&wallet, &rpc_endpoint, &cli.network, command).await?;
        }
        Commands::Nft {
            list,
            info,
            mint,
            transfer,
            sell,
            sell_sign,
            buy,
            from,
            to,
            price,
            output,
            seller_wallet,
            condition,
            amount,
            witness,
            royalty,
            data,
            data_file,
            export,
            batch_mint,
            yes,
            fractionalize,
            shares,
            ticker,
            redeem,
        } => {
            if list {
                cmd_nft::cmd_nft_list(&wallet, &rpc_endpoint).await?;
            } else if let Some(utxo) = info {
                cmd_nft::cmd_nft_info(&rpc_endpoint, &utxo).await?;
            } else if let Some(content) = mint {
                // --data-file reads raw bytes from file and converts to hex
                let effective_data = if let Some(ref path) = data_file {
                    let bytes = std::fs::read(path).map_err(|e| {
                        anyhow::anyhow!("Failed to read --data-file {:?}: {}", path, e)
                    })?;
                    Some(hex::encode(&bytes))
                } else {
                    data
                };
                cmd_nft::cmd_mint(
                    &wallet,
                    &rpc_endpoint,
                    &content,
                    condition,
                    &amount,
                    royalty,
                    effective_data,
                )
                .await?;
            } else if let Some(utxo) = export {
                let out =
                    output.ok_or_else(|| anyhow::anyhow!("-o/--out is required for --export"))?;
                cmd_nft::cmd_nft_export(&rpc_endpoint, &utxo, &out).await?;
            } else if let Some(manifest) = batch_mint {
                cmd_nft::cmd_nft_batch_mint(&wallet, &rpc_endpoint, &manifest, yes).await?;
            } else if let Some(utxo) = transfer {
                let to = to.ok_or_else(|| anyhow::anyhow!("--to is required for --transfer"))?;
                cmd_nft::cmd_nft_transfer(&wallet, &rpc_endpoint, &utxo, &to, &witness).await?;
            } else if let Some(utxo) = sell {
                let price =
                    price.ok_or_else(|| anyhow::anyhow!("--price is required for --sell"))?;
                let out =
                    output.ok_or_else(|| anyhow::anyhow!("-o/--out is required for --sell"))?;
                cmd_nft::cmd_nft_sell(&wallet, &rpc_endpoint, &utxo, &price, &out).await?;
            } else if let Some(utxo) = sell_sign {
                let price =
                    price.ok_or_else(|| anyhow::anyhow!("--price is required for --sell-sign"))?;
                let to = to.ok_or_else(|| {
                    anyhow::anyhow!("--to (buyer address) is required for --sell-sign")
                })?;
                let out = output
                    .ok_or_else(|| anyhow::anyhow!("-o/--out is required for --sell-sign"))?;
                cmd_nft::cmd_nft_sell_sign(&wallet, &rpc_endpoint, &utxo, &price, &to, &out)
                    .await?;
            } else if let Some(utxo) = buy {
                let price =
                    price.ok_or_else(|| anyhow::anyhow!("--price is required for --buy"))?;
                let sw = seller_wallet
                    .ok_or_else(|| anyhow::anyhow!("--seller-wallet is required for --buy"))?;
                let seller_path = expand_tilde(&sw);
                cmd_nft::cmd_nft_buy(&wallet, &seller_path, &rpc_endpoint, &utxo, &price).await?;
            } else if let Some(file) = from {
                cmd_nft::cmd_nft_buy_from_offer(
                    &wallet,
                    seller_wallet.as_deref(),
                    &rpc_endpoint,
                    &file,
                )
                .await?;
            } else if let Some(token_id) = fractionalize {
                let shares = shares
                    .ok_or_else(|| anyhow::anyhow!("--shares is required for --fractionalize"))?;
                cmd_nft::cmd_nft_fractionalize(
                    &wallet,
                    &rpc_endpoint,
                    &token_id,
                    shares,
                    ticker.as_deref(),
                )
                .await?;
            } else if let Some(token_id) = redeem {
                cmd_nft::cmd_nft_redeem(&wallet, &rpc_endpoint, &token_id).await?;
            } else {
                anyhow::bail!(
                    "Specify an action: --list, --info, --mint, --transfer, --sell, --sell-sign, --buy, --from, --export, --batch-mint, --fractionalize, or --redeem\nRun 'doli nft --help' for details."
                );
            }
        }
        Commands::IssueToken {
            ticker,
            supply,
            condition,
        } => {
            cmd_token::cmd_issue_token(&wallet, &rpc_endpoint, &ticker, supply, condition).await?;
        }
        Commands::TokenInfo { utxo } => {
            cmd_token::cmd_token_info(&rpc_endpoint, &utxo).await?;
        }
        Commands::BridgeSwap {
            amount,
            chain,
            to,
            counter_rpc,
            confirmations,
        } => {
            cmd_bridge::cmd_bridge_swap(
                &wallet,
                &rpc_endpoint,
                &amount,
                &to,
                &chain,
                counter_rpc.as_deref(),
                confirmations,
            )
            .await?;
        }
        Commands::BridgeStatus {
            swap_id,
            btc_rpc,
            eth_rpc,
            auto,
        } => {
            cmd_bridge::cmd_bridge_status(
                &wallet,
                &rpc_endpoint,
                &swap_id,
                btc_rpc.as_deref(),
                eth_rpc.as_deref(),
                auto,
            )
            .await?;
        }
        Commands::BridgeBuy {
            swap_id,
            preimage,
            btc_rpc,
            eth_rpc,
            to,
            yes,
        } => {
            cmd_bridge::cmd_bridge_buy(
                &wallet,
                &rpc_endpoint,
                &swap_id,
                preimage.as_deref(),
                btc_rpc.as_deref(),
                eth_rpc.as_deref(),
                to.as_deref(),
                yes,
            )
            .await?;
        }
        Commands::BridgeList { chain, blocks } => {
            cmd_bridge::cmd_bridge_list(&rpc_endpoint, chain.as_deref(), blocks).await?;
        }
        Commands::BridgeWatch {
            btc_rpc,
            eth_rpc,
            interval,
        } => {
            cmd_bridge::cmd_bridge_watch(
                &wallet,
                &rpc_endpoint,
                btc_rpc.as_deref(),
                eth_rpc.as_deref(),
                interval,
            )
            .await?;
        }
        Commands::BridgeLock {
            amount,
            hash,
            preimage,
            lock,
            expiry,
            chain,
            to,
            counter_hash,
            multisig_threshold,
            multisig_keys,
            yes,
        } => {
            let resolved_hash = match (hash, preimage) {
                (Some(h), _) => h,
                (_, Some(p)) => {
                    let preimage_bytes =
                        hex::decode(&p).map_err(|_| anyhow::anyhow!("Invalid preimage hex"))?;
                    if preimage_bytes.len() != 32 {
                        anyhow::bail!("Preimage must be exactly 32 bytes (64 hex chars)");
                    }
                    let hash = crypto::hash::hash_with_domain(
                        doli_core::conditions::HASHLOCK_DOMAIN,
                        &preimage_bytes,
                    );
                    println!("Computed hashlock: {}", hash.to_hex());
                    hash.to_hex()
                }
                (None, None) => {
                    anyhow::bail!("Provide either --hash or --preimage");
                }
            };
            cmd_bridge::cmd_bridge_lock(
                &wallet,
                &rpc_endpoint,
                &amount,
                &resolved_hash,
                lock,
                expiry,
                &chain,
                &to,
                &counter_hash,
                multisig_threshold,
                multisig_keys.as_deref(),
                yes,
            )
            .await?;
        }
        Commands::BridgeClaim {
            utxo,
            preimage,
            to,
            yes,
        } => {
            cmd_bridge::cmd_bridge_claim(
                &wallet,
                &rpc_endpoint,
                &utxo,
                &preimage,
                to.as_deref(),
                yes,
            )
            .await?;
        }
        Commands::BridgeRefund { utxo, yes } => {
            cmd_bridge::cmd_bridge_refund(&wallet, &rpc_endpoint, &utxo, yes).await?;
        }
        Commands::Guardian { command } => {
            cmd_guardian::cmd_guardian(&rpc_endpoint, command).await?;
        }
        Commands::Service(command) => {
            cmd_service::cmd_service(&cli.network, command)?;
        }
        Commands::Wipe {
            network,
            data_dir,
            yes,
        } => {
            cmd_chain::cmd_wipe(&network, data_dir, yes)?;
        }
        Commands::Snap {
            data_dir,
            seed,
            no_restart,
            trust,
        } => {
            cmd_snap::cmd_snap(&cli.network, data_dir, seed, no_restart, trust).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_new_wallet() {
        let cli = Cli::try_parse_from(["doli", "new"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert!(matches!(cli.command, Commands::New { .. }));
    }

    #[test]
    fn test_parse_balance() {
        let cli = Cli::try_parse_from(["doli", "balance"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_parse_send_with_args() {
        let cli = Cli::try_parse_from(["doli", "send", "doli1abc123", "100"]);
        assert!(cli.is_ok());
        if let Commands::Send { to, amount, .. } = &cli.unwrap().command {
            assert_eq!(to, "doli1abc123");
            assert_eq!(amount, "100");
        } else {
            panic!("Expected Send command");
        }
    }

    #[test]
    fn test_parse_custom_wallet_and_rpc() {
        let cli = Cli::try_parse_from([
            "doli",
            "-w",
            "/tmp/test_wallet.json",
            "-r",
            "http://localhost:9999",
            "balance",
        ]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.wallet, Some("/tmp/test_wallet.json".to_string()));
        assert_eq!(cli.rpc, Some("http://localhost:9999".to_string()));
    }

    #[test]
    fn test_parse_producer_subcommands() {
        let cli = Cli::try_parse_from(["doli", "producer", "list"]);
        assert!(cli.is_ok());

        let cli = Cli::try_parse_from(["doli", "producer", "register", "--bonds", "5"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_parse_missing_subcommand_fails() {
        let cli = Cli::try_parse_from(["doli"]);
        assert!(cli.is_err());
    }
}
