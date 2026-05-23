//! Handler for `doli template agent-allowance`.

use anyhow::Result;

use crate::parsers::resolve_to_hash;
use crate::rpc_client::coins_to_units;

use super::serialize::condition_to_cli_string;
use super::vault::SendOpts;

/// Handle the agent-allowance template subcommand.
///
/// Agent allowance = bounded delegation to an agent.
/// The agent can spend the UTXO but ONLY if the spending transaction
/// pays at least `amount` to `recipient` at `output_index`.
pub(crate) async fn handle_agent_allowance(
    agent: &str,
    recipient: &str,
    amount_str: &str,
    output_index: u8,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let agent_hash = resolve_to_hash(agent)?;
    let recipient_hash = resolve_to_hash(recipient)?;
    let required_amount =
        coins_to_units(amount_str).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
    if required_amount == 0 {
        anyhow::bail!("amount must be greater than zero");
    }

    let cond = doli_core::conditions::templates::agent_allowance(
        agent_hash,
        recipient_hash,
        required_amount,
        output_index,
    );
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        super::vault::send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}
