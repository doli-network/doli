use std::path::Path;

use anyhow::Result;

use crate::commands::ProducerCommands;
use crate::rpc_client::RpcClient;
use crate::wallet::Wallet;

use super::bonds::{handle_add_bond, handle_simulate_withdrawal};
use super::exit::{handle_exit, handle_slash};
use super::register::handle_register;
use super::status::{handle_bonds, handle_list, handle_status};
use super::withdrawal::handle_request_withdrawal;

pub(crate) async fn cmd_producer(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: ProducerCommands,
) -> Result<()> {
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
            handle_register(&wallet, &keypair, &pubkey_hash, &rpc, bonds).await?;
        }

        ProducerCommands::Status { pubkey } => {
            handle_status(wallet_path, &rpc, pubkey).await?;
        }

        ProducerCommands::Bonds { pubkey } => {
            handle_bonds(wallet_path, &rpc, pubkey).await?;
        }

        ProducerCommands::List { active } => {
            handle_list(&rpc, active).await?;
        }

        ProducerCommands::AddBond { count } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();
            handle_add_bond(&wallet, &keypair, &pubkey_hash, &rpc, count).await?;
        }

        ProducerCommands::RequestWithdrawal { count, destination } => {
            let wallet = Wallet::load(wallet_path)?;
            handle_request_withdrawal(&wallet, &rpc, count, destination).await?;
        }

        ProducerCommands::SimulateWithdrawal { count } => {
            let wallet = Wallet::load(wallet_path)?;
            handle_simulate_withdrawal(&wallet, &rpc, count).await?;
        }

        ProducerCommands::Exit { force } => {
            let wallet = Wallet::load(wallet_path)?;
            handle_exit(&wallet, &rpc, force).await?;
        }

        ProducerCommands::Slash { block1, block2 } => {
            handle_slash(&rpc, block1, block2).await?;
        }
    }

    Ok(())
}
