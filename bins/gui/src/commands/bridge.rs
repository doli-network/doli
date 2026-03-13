//! Bridge (HTLC) Tauri commands.
//!
//! Handles bridge lock, claim, and refund operations.

use tauri::State;

use crate::commands::{BridgeLockParams, TxResponse};
use crate::state::AppState;

/// Lock funds in a bridge HTLC.
#[tauri::command]
pub async fn bridge_lock(
    params: BridgeLockParams,
    state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    let amount_units = wallet::coins_to_units(&params.amount)?;

    let sender_hash = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let pkh = w.primary_pubkey_hash().map_err(|e| e.to_string())?;
        let bytes: [u8; 32] = hex::decode(&pkh)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "Invalid pubkey hash length".to_string())?;
        bytes
    };

    let recipient_hash: [u8; 32] = hex::decode(&params.recipient)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "Invalid recipient hash".to_string())?;

    let hash_lock = hex::decode(&params.hash_lock).map_err(|e| e.to_string())?;

    let mut extra = Vec::new();
    extra.extend_from_slice(&hash_lock);
    extra.extend_from_slice(&params.timeout_height.to_le_bytes());

    let mut builder = wallet::TxBuilder::new(wallet::TxType::BridgeLock);
    builder.add_input(sender_hash, 0);
    builder.add_output(
        amount_units,
        recipient_hash,
        0,
        params.timeout_height,
        Vec::new(),
    );
    builder.set_extra_data(extra);

    let tx_hex = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let keypair = w.primary_keypair().map_err(|e| e.to_string())?;
        builder
            .sign_and_build(&keypair)
            .map_err(|e| e.to_string())?
    };

    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(TxResponse {
        tx_hash,
        tx_type: "BridgeLock".to_string(),
        message: format!(
            "Locked {} DOLI in bridge HTLC",
            wallet::units_to_coins(amount_units)
        ),
    })
}

/// Claim a bridge HTLC with preimage.
#[tauri::command]
pub async fn bridge_claim(
    _utxo_ref: String,
    _preimage: String,
    _state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    Err("Bridge claim not yet fully implemented".to_string())
}

/// Refund an expired bridge HTLC.
#[tauri::command]
pub async fn bridge_refund(
    _utxo_ref: String,
    _state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    Err("Bridge refund not yet fully implemented".to_string())
}
