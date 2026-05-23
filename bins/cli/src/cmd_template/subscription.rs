//! Handler for `doli template subscription`.

use anyhow::Result;

use crate::parsers::resolve_to_hash;
use crate::rpc_client::coins_to_units;

use super::serialize::condition_to_cli_string;
use super::vault::SendOpts;

/// Handle the subscription template subcommand.
///
/// Subscription = time-gated bounded payment for recurring allowances.
/// Spending tx must pay at least `amount` to `recipient` at `output_index`,
/// within the time window [start, end].
pub(crate) async fn handle_subscription(
    recipient: &str,
    amount_str: &str,
    output_index: u8,
    start: u64,
    end: u64,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let recipient_hash = resolve_to_hash(recipient)?;
    let required_amount =
        coins_to_units(amount_str).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if required_amount == 0 {
        anyhow::bail!("amount must be greater than zero");
    }

    let cond = doli_core::conditions::templates::subscription(
        recipient_hash,
        required_amount,
        output_index,
        start,
        end,
    );
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        super::vault::send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}
