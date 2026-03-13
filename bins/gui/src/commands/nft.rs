//! NFT and Token Tauri commands.
//!
//! Handles NFT minting, transfer, info queries, and token issuance.

use tauri::State;

use crate::commands::{NftInfoResponse, TokenInfoResponse, TxResponse};
use crate::state::AppState;

/// Mint a new NFT.
#[tauri::command]
pub async fn mint_nft(
    content: String,
    value: Option<String>,
    state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    let value_units = match value {
        Some(v) => wallet::coins_to_units(&v)?,
        None => 0,
    };

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

    let mut builder = wallet::TxBuilder::new(wallet::TxType::NftMint);
    builder.add_input(sender_hash, 0);
    builder.add_output(value_units, sender_hash, 4, 0, content.as_bytes().to_vec());

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
        tx_type: "NftMint".to_string(),
        message: format!(
            "Minted NFT with content: {}",
            &content[..content.len().min(50)]
        ),
    })
}

/// Transfer an NFT to a new owner.
#[tauri::command]
pub async fn transfer_nft(
    _utxo_ref: String,
    _to: String,
    _state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    Err("NFT transfer not yet implemented".to_string())
}

/// Get NFT info for a specific UTXO.
#[tauri::command]
pub async fn nft_info(
    _utxo_ref: String,
    _state: State<'_, AppState>,
) -> Result<NftInfoResponse, String> {
    Err("NFT info lookup not yet implemented".to_string())
}

/// Issue a new token.
#[tauri::command]
pub async fn issue_token(
    ticker: String,
    supply: String,
    state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    let supply_units = wallet::coins_to_units(&supply)?;

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

    let mut extra = Vec::new();
    extra.extend_from_slice(ticker.as_bytes());
    extra.push(0);
    extra.extend_from_slice(&supply_units.to_le_bytes());

    let mut builder = wallet::TxBuilder::new(wallet::TxType::TokenIssuance);
    builder.add_input(sender_hash, 0);
    builder.add_output(supply_units, sender_hash, 0, 0, Vec::new());
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
        tx_type: "TokenIssuance".to_string(),
        message: format!("Issued token {} with supply {}", ticker, supply),
    })
}

/// Get token info for a specific UTXO.
#[tauri::command]
pub async fn token_info(
    _utxo_ref: String,
    _state: State<'_, AppState>,
) -> Result<TokenInfoResponse, String> {
    Err("Token info lookup not yet implemented".to_string())
}
