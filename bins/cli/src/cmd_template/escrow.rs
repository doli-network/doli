//! Handler for `doli template escrow`.

use anyhow::Result;

use crate::parsers::resolve_to_hash;

use super::serialize::condition_to_cli_string;
use super::vault::SendOpts;

/// Handle the escrow template subcommand.
///
/// Escrow = m-of-n multisig release with timeout refund.
/// - Release: threshold-of-parties multisig
/// - Refund: refund signer after timeout_height
pub(crate) async fn handle_escrow(
    parties_str: &str,
    threshold: u8,
    timeout: u64,
    refund: &str,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let party_addrs: Vec<&str> = parties_str.split(',').map(|s| s.trim()).collect();
    if party_addrs.len() < 2 {
        anyhow::bail!("escrow requires at least 2 parties");
    }
    let parties: Result<Vec<crypto::Hash>> =
        party_addrs.iter().map(|a| resolve_to_hash(a)).collect();
    let parties = parties?;
    let refund_hash = resolve_to_hash(refund)?;

    let cond = doli_core::conditions::templates::escrow(parties, threshold, timeout, refund_hash);
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        super::vault::send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}
