//! Handler for `doli template escrow-loan`.

use anyhow::Result;

use crate::parsers::resolve_to_hash;
use crate::rpc_client::coins_to_units;

use super::serialize::condition_to_cli_string;
use super::vault::SendOpts;

/// Handle the escrow-loan template subcommand.
///
/// Escrow-loan = bilateral OTC loan using guard-based conditions.
/// - Repayment path: spending TX pays >= repay_amount to lender at output[0]
/// - Lender reclaim path: lender signs after deadline_height
///
/// See `specs/defi-subsystem-architecture.md` D3 for the full specification.
pub(crate) async fn handle_escrow_loan(
    lender: &str,
    repay_amount_str: &str,
    deadline: u64,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let lender_hash = resolve_to_hash(lender)?;
    let repay_amount =
        coins_to_units(repay_amount_str).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if repay_amount == 0 {
        anyhow::bail!("repay-amount must be greater than zero");
    }

    let cond = doli_core::conditions::templates::escrow_loan(lender_hash, repay_amount, deadline);
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        super::vault::send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}
