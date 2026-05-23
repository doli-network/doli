//! Handler for `doli template vault`.

use anyhow::Result;
use std::path::Path;

use crate::parsers::resolve_to_hash;

use super::serialize::condition_to_cli_string;

/// Handle the vault template subcommand.
///
/// Vault = delayed-withdrawal with cosigner emergency override.
/// - Delayed claim: owner signs after unlock_height
/// - Immediate override: 2-of-2 multisig (owner + cosigner)
pub(crate) async fn handle_vault(
    owner: &str,
    cosigner: &str,
    unlock_height: u64,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let owner_hash = resolve_to_hash(owner)?;
    let cosigner_hash = resolve_to_hash(cosigner)?;

    let cond = doli_core::conditions::templates::vault(owner_hash, cosigner_hash, unlock_height);
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}

/// Options for the --send mode (shared across all template handlers).
pub(crate) struct SendOpts<'a> {
    pub wallet_path: &'a Path,
    pub rpc_endpoint: &'a str,
    pub to: &'a str,
    pub amount: &'a str,
    pub fee: Option<&'a str>,
}

/// Build and broadcast a transaction with the given condition.
/// Delegates to `cmd_send` which already handles mainnet guard warnings
/// internally (REQ-SDK-006).
pub(super) async fn send_with_condition(opts: SendOpts<'_>, cli_string: &str) -> Result<()> {
    crate::cmd_wallet::cmd_send(
        opts.wallet_path,
        opts.rpc_endpoint,
        opts.to,
        opts.amount,
        opts.fee.map(|s| s.to_string()),
        Some(cli_string.to_string()),
        false, // don't skip confirmation
    )
    .await
}
