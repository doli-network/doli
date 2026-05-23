//! Dispatch for `doli template <subcommand>`.

use std::path::Path;

use anyhow::Result;

use crate::commands::TemplateCommands;

use super::agent_allowance::handle_agent_allowance;
use super::escrow::handle_escrow;
use super::htlc_payment::handle_htlc_payment;
use super::subscription::handle_subscription;
use super::vault::{handle_vault, SendOpts};

pub(crate) async fn cmd_template(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: TemplateCommands,
) -> Result<()> {
    match command {
        TemplateCommands::Vault {
            owner,
            cosigner,
            unlock_height,
            send,
            to,
            amount,
            fee,
        } => {
            let send_opts = build_send_opts(send, wallet_path, rpc_endpoint, &to, &amount, &fee)?;
            handle_vault(&owner, &cosigner, unlock_height, send_opts).await?;
        }

        TemplateCommands::Escrow {
            parties,
            threshold,
            timeout,
            refund,
            send,
            to,
            amount,
            fee,
        } => {
            let send_opts = build_send_opts(send, wallet_path, rpc_endpoint, &to, &amount, &fee)?;
            handle_escrow(&parties, threshold, timeout, &refund, send_opts).await?;
        }

        TemplateCommands::HtlcPayment {
            hash,
            lock,
            expiry,
            refund,
            send,
            to,
            amount,
            fee,
        } => {
            let send_opts = build_send_opts(send, wallet_path, rpc_endpoint, &to, &amount, &fee)?;
            handle_htlc_payment(&hash, lock, expiry, &refund, send_opts).await?;
        }

        TemplateCommands::Subscription {
            recipient,
            amount: sub_amount,
            output_index,
            start,
            end,
            send,
            to,
            send_amount,
            fee,
        } => {
            let send_opts =
                build_send_opts(send, wallet_path, rpc_endpoint, &to, &send_amount, &fee)?;
            handle_subscription(&recipient, &sub_amount, output_index, start, end, send_opts)
                .await?;
        }

        TemplateCommands::AgentAllowance {
            agent,
            recipient,
            amount: allow_amount,
            output_index,
            send,
            to,
            send_amount,
            fee,
        } => {
            let send_opts =
                build_send_opts(send, wallet_path, rpc_endpoint, &to, &send_amount, &fee)?;
            handle_agent_allowance(&agent, &recipient, &allow_amount, output_index, send_opts)
                .await?;
        }
    }

    Ok(())
}

/// Build SendOpts when --send is active; None for --dry-run (default).
fn build_send_opts<'a>(
    send: bool,
    wallet_path: &'a Path,
    rpc_endpoint: &'a str,
    to: &'a Option<String>,
    amount: &'a Option<String>,
    fee: &'a Option<String>,
) -> Result<Option<SendOpts<'a>>> {
    if !send {
        return Ok(None);
    }

    let to = to
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--to is required when using --send"))?;
    let amount = amount
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--amount is required when using --send"))?;

    Ok(Some(SendOpts {
        wallet_path,
        rpc_endpoint,
        to,
        amount,
        fee: fee.as_deref(),
    }))
}
