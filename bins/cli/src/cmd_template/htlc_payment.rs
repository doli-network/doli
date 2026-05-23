//! Handler for `doli template htlc-payment`.

use anyhow::Result;

use crate::parsers::resolve_to_hash;

use super::serialize::condition_to_cli_string;
use super::vault::SendOpts;

/// Handle the htlc-payment template subcommand.
///
/// HTLC = hash-locked payment with signed refund.
/// - Claim: reveal preimage after lock_height
/// - Refund: refund signer after expiry_height
pub(crate) async fn handle_htlc_payment(
    hash_hex: &str,
    lock: u64,
    expiry: u64,
    refund: &str,
    send_opts: Option<SendOpts<'_>>,
) -> Result<()> {
    let payment_hash = crypto::Hash::from_hex(hash_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid payment hash hex: {}", hash_hex))?;
    let refund_hash = resolve_to_hash(refund)?;

    let cond =
        doli_core::conditions::templates::htlc_payment(payment_hash, lock, expiry, refund_hash);
    let cli_string = condition_to_cli_string(&cond);

    if let Some(opts) = send_opts {
        super::vault::send_with_condition(opts, &cli_string).await
    } else {
        println!("{}", cli_string);
        Ok(())
    }
}
