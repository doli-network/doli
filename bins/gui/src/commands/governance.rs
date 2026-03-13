//! Governance Tauri commands.
//!
//! Handles update checking, status, and voting on protocol updates.
//! Also includes sign/verify message commands (Could priority: GUI-FR-100, GUI-FR-101).

use tauri::State;

use crate::commands::{TxResponse, UpdateInfo, UpdateStatusResponse};
use crate::state::AppState;

/// Check for available protocol updates.
#[tauri::command]
pub async fn check_updates(_state: State<'_, AppState>) -> Result<Vec<UpdateInfo>, String> {
    // Protocol updates are governance-driven via on-chain transactions.
    // Placeholder until governance RPC methods are available.
    Ok(Vec::new())
}

/// Get current update/governance status.
#[tauri::command]
pub async fn update_status(_state: State<'_, AppState>) -> Result<UpdateStatusResponse, String> {
    Ok(UpdateStatusResponse {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: None,
        update_available: false,
    })
}

/// Vote on a protocol update (approve or reject).
#[tauri::command]
pub async fn vote_update(
    _version: String,
    _approve: bool,
    _state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    Err("Governance voting not yet implemented".to_string())
}

/// Sign a message with the wallet key.
#[tauri::command]
pub async fn sign_message(
    message: String,
    address: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let wallet_guard = state.wallet.read().await;
    let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;

    w.sign_message(&message, address.as_deref())
        .map_err(|e| e.to_string())
}

/// Verify a message signature.
#[tauri::command]
pub async fn verify_signature(
    message: String,
    signature: String,
    pubkey: String,
) -> Result<bool, String> {
    wallet::verify_message(&message, &signature, &pubkey).map_err(|e| e.to_string())
}
