//! `doli import-bls` — INC-I-217 / REQ-ROT-014.
//!
//! Purely client-side recovery for a producer operator who still holds the BLS
//! secret the chain has registered, but whose `wallet.json` no longer matches it.
//! Nothing here reaches a block, a transaction or the producer set.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::rpc_client::RpcClient;
use crate::wallet::Wallet;

pub(crate) async fn cmd_import_bls(
    wallet_path: &Path,
    secret_hex: &str,
    force: bool,
    rpc: Option<String>,
    address: Option<String>,
) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    // Validates and derives in memory only. Every failure below returns before
    // save(), so a rejected secret leaves wallet.json byte-identical.
    let bls_pub = wallet.import_bls_key(secret_hex, force)?;
    let producer_pk = producer_public_key(&wallet, address.as_deref())?;

    if let Some(endpoint) = rpc.as_deref() {
        confirm_against_chain(endpoint, &producer_pk, &bls_pub).await?;
    }

    wallet.save(wallet_path)?;

    println!("BLS producer key imported.");
    println!("  BLS Public Key: {}", bls_pub);
    println!();
    println!("Backup:");
    println!("  Your 24-word phrase does NOT restore an imported key. This wallet");
    println!("  file is now the only copy of it — back up the file itself.");
    println!();
    if rpc.is_none() {
        println!("Confirm the chain agrees with this key:");
        println!("  compare the BLS Public Key above with getProducer -> blsPubkey");
        println!("  for producer {}.", producer_pk);
        println!();
    }
    println!("Restart the node to load the imported BLS key.");

    Ok(())
}

/// The Ed25519 public key whose producer registration is compared against the
/// chain. `--address` picks a non-primary wallet address; the default is primary.
fn producer_public_key(wallet: &Wallet, address: Option<&str>) -> Result<String> {
    let entry = match address {
        Some(a) => wallet
            .addresses()
            .iter()
            .find(|w| w.address == a)
            .ok_or_else(|| anyhow!("Address not found in this wallet: {}", a))?,
        None => wallet
            .addresses()
            .first()
            .ok_or_else(|| anyhow!("Wallet has no addresses"))?,
    };
    Ok(entry.public_key.clone())
}

/// Compare the derived public key with the one the chain has registered.
///
/// Runs BEFORE the wallet is written: a mismatch means the operator pasted the
/// wrong key, and the wallet they still have is worth more than the one they
/// were about to install.
async fn confirm_against_chain(endpoint: &str, producer_pk: &str, bls_pub: &str) -> Result<()> {
    let params = serde_json::json!({ "public_key": producer_pk });
    let info = RpcClient::new(endpoint)
        .call_raw("getProducer", params)
        .await?;

    match info.get("blsPubkey").and_then(serde_json::Value::as_str) {
        Some(on_chain) if on_chain.eq_ignore_ascii_case(bls_pub) => {
            println!("Chain check: MATCH — getProducer -> blsPubkey equals the imported key.");
            println!();
            Ok(())
        }
        Some(on_chain) => Err(anyhow!(
            "Chain check FAILED — nothing was written.\n  \
             The chain has blsPubkey {} for this producer,\n  \
             but the supplied secret derives {}.\n  \
             Importing it would not restore attestation. Check the secret you pasted.",
            on_chain,
            bls_pub
        )),
        None => Err(anyhow!(
            "Chain check FAILED — nothing was written.\n  \
             The chain records no blsPubkey for producer {}.\n  \
             Re-run without --rpc if you mean to import the key anyway.",
            producer_pk
        )),
    }
}

#[cfg(test)]
#[path = "cmd_wallet_bls_tests.rs"]
mod tests;
