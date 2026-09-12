//! INC-I-217 M10 — `doli producer rotate-bls`: the RPC/print half (REQ-ROT-013).
//!
//! Every decision lives in `doli_cli::rotate_tx`; this file only fetches, shows
//! and submits. The wallet is opened READ-ONLY: a rotation changes the chain,
//! never `wallet.json`.

use std::io::{IsTerminal, Write};
use std::str::FromStr;

use anyhow::{anyhow, Result};
use crypto::BlsSecretKey;
use doli_cli::rotate_tx::{
    build_rotate_bls_tx, check_rotation_preconditions, rotate_bls_fee, rotation_answer_accepted,
    rotation_consent, rotation_report_lines, select_fee_utxo, ConsentOutcome, OnChainProducer,
    RotateTxError, SpendableUtxo, ROTATE_BLS_NOTICE,
};
use doli_core::genesis::genesis_hash;
use doli_core::Network;

use crate::rpc_client::{PendingUpdateInfo, ProducerInfo, RpcClient};
use crate::tx_retention;
use crate::wallet::Wallet;

pub(super) async fn handle_rotate_bls(wallet: &Wallet, rpc: &RpcClient, yes: bool) -> Result<()> {
    println!("Producer BLS Key Rotation");
    println!("{:-<60}", "");
    println!();

    let address = &wallet.addresses()[0];
    let bls_secret_hex = address.bls_private_key.clone();
    let wallet_bls_pubkey = match bls_secret_hex.as_deref() {
        Some(secret) => Some(bls_pubkey_of(secret)?),
        None => None,
    };

    // The genesis hash comes from the same function the validator uses, keyed by
    // the network the NODE reports — never from chainInfo.genesisHash.
    let chain_info = rpc.get_chain_info().await?;
    let network = Network::from_str(&chain_info.network).map_err(|_| {
        anyhow!(
            "the node reports an unknown network {:?}; refusing to sign a rotation whose genesis \
             hash cannot be derived",
            chain_info.network
        )
    })?;
    let genesis = *genesis_hash(network).as_bytes();

    // PLURAL getProducers: the singular getProducer cannot see a producer that is
    // still in pending_updates (register.rs:27-40, INC-I-147 / INC-I-148 RC-2).
    let producers = rpc.get_producers(false).await?;
    let info = producers
        .iter()
        .find(|p| p.public_key.eq_ignore_ascii_case(&address.public_key));
    show_queued_rotation(info);

    let target = check_rotation_preconditions(&on_chain_producer(info), wallet_bls_pubkey)?;
    let bls_secret_hex = bls_secret_hex.ok_or(RotateTxError::NoWalletBlsKey)?;

    let fee = rotate_bls_fee();
    let utxos = spendable_utxos(rpc, &wallet.primary_pubkey_hash()).await?;
    let fee_utxo = select_fee_utxo(&utxos, fee)?;

    let producer_secret_hex = wallet.primary_keypair()?.private_key().to_hex();
    let plan = build_rotate_bls_tx(
        &genesis,
        &producer_secret_hex,
        &bls_secret_hex,
        &fee_utxo,
        fee,
    )?;

    for line in rotation_report_lines(&plan, target.old_bls_pubkey, None) {
        println!("{line}");
    }
    println!();
    println!("{ROTATE_BLS_NOTICE}");
    println!();

    match rotation_consent(yes, std::io::stdin().is_terminal())? {
        ConsentOutcome::Accepted => {
            println!("Proceeding — irreversible key change accepted via --yes.");
        }
        ConsentOutcome::MustPrompt => ask_operator()?,
    }
    println!();

    println!("Submitting the rotation transaction...");
    let tx_hex = hex::encode(plan.tx.serialize());
    // The node's own text is surfaced verbatim: below the activation height it
    // answers [ERRTX-ROT002], and paraphrasing it would hide the real reason.
    let hash = rpc
        .send_transaction(&tx_hex)
        .await
        .map_err(|e| anyhow!("Error submitting the BLS key rotation: {}", e))?;

    // INV-CLI-002: a bare OK from sendTransaction is not evidence of retention.
    println!("Verifying the node retained the transaction...");
    let retention = tx_retention::require_retained(rpc, &hash, "BLS rotation").await?;

    println!("Rotation submitted.");
    println!("TX Hash: {hash}");
    println!("Retention: {}", retention.describe());
    println!();
    let effective_height = rpc.get_epoch_info().await.ok().map(|e| e.epoch_end_height);
    for line in rotation_report_lines(&plan, target.old_bls_pubkey, effective_height) {
        println!("{line}");
    }
    println!("Use 'doli producer status' to confirm the new key once the boundary passes.");

    Ok(())
}

/// The BLS public key the wallet secret actually derives — never the stored
/// `bls_public_key` field, which no one re-checks against the secret.
fn bls_pubkey_of(secret_hex: &str) -> Result<[u8; 48]> {
    let secret = BlsSecretKey::from_hex(secret_hex).map_err(|_| RotateTxError::InvalidBlsSecret)?;
    Ok(*secret.public_key().as_bytes())
}

fn parse_key_48(hex_str: &str) -> Option<[u8; 48]> {
    hex::decode(hex_str).ok()?.try_into().ok()
}

fn queued_rotation(info: &ProducerInfo) -> Option<&PendingUpdateInfo> {
    info.pending_updates
        .iter()
        .find(|u| u.update_type == "rotate_bls_key")
}

fn on_chain_producer(info: Option<&ProducerInfo>) -> OnChainProducer {
    let Some(info) = info else {
        return OnChainProducer {
            registered: false,
            bls_pubkey: None,
            pending_new_bls_pubkey: None,
        };
    };
    OnChainProducer {
        // Mirrors ProducerSet::resolve_rotation_inputs: Active in the map, or a
        // queued Register. Anything else is not a rotation the node would apply.
        registered: matches!(info.status.to_lowercase().as_str(), "active" | "pending"),
        bls_pubkey: parse_key_48(&info.bls_pubkey),
        pending_new_bls_pubkey: queued_rotation(info)
            .and_then(|u| u.new_bls_pubkey.as_deref())
            .and_then(parse_key_48),
    }
}

/// Context for the refusal that follows: a queued rotation always blocks a
/// second one, so this only ever prints ahead of that error.
fn show_queued_rotation(info: Option<&ProducerInfo>) {
    let Some(update) = info.and_then(queued_rotation) else {
        return;
    };
    println!("A BLS key rotation is already queued for this producer:");
    println!(
        "  Queued key:   {}",
        update.new_bls_pubkey.as_deref().unwrap_or("(not reported)")
    );
    match update.effective_at_height {
        Some(height) => println!("  Takes effect: height {height} (next epoch boundary)"),
        None => println!("  Takes effect: at the next epoch boundary"),
    }
    println!();
}

async fn spendable_utxos(rpc: &RpcClient, pubkey_hash: &str) -> Result<Vec<SpendableUtxo>> {
    Ok(rpc
        .get_utxos(pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .filter_map(|u| {
            Some(SpendableUtxo {
                tx_hash: hex::decode(&u.tx_hash).ok()?.try_into().ok()?,
                output_index: u.output_index,
                amount: u.amount,
            })
        })
        .collect())
}

/// Interactive confirmation. Every refusal path exits non-zero, so a caller can
/// never read the exit code as a rotation that happened (INV-CLI-003).
fn ask_operator() -> Result<()> {
    print!("Rotate the BLS key now? This cannot be undone. [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 {
        anyhow::bail!(
            "Refusing to rotate the BLS key: stdin closed before an answer. \
             Re-run with --yes to confirm you accept an irreversible key change."
        );
    }
    if !rotation_answer_accepted(&answer) {
        anyhow::bail!("Aborted — no rotation was submitted.");
    }
    Ok(())
}
