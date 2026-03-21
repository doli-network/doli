//! Transaction Tauri commands: send, receive, history.
//!
//! Handles balance queries, sending DOLI, and transaction history.
//! Transaction signing happens entirely in Rust -- private keys never
//! cross the IPC boundary (GUI-NF-004).

use tauri::State;

use crate::commands::{BalanceResponse, HistoryEntryResponse, SendResponse};
use crate::state::AppState;

/// Get balance for the primary wallet address.
#[tauri::command]
pub async fn get_balance(
    address: Option<String>,
    state: State<'_, AppState>,
) -> Result<BalanceResponse, String> {
    let pubkey_hash = match address {
        Some(addr) => addr,
        None => {
            let wallet_guard = state.wallet.read().await;
            let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
            w.primary_pubkey_hash().map_err(|e| e.to_string())?
        }
    };

    let balance = {
        let rpc = state.rpc_client.read().await;
        rpc.get_balance(&pubkey_hash)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(BalanceResponse {
        confirmed: balance.confirmed,
        unconfirmed: balance.unconfirmed,
        immature: balance.immature,
        total: balance.total,
        formatted_total: wallet::format_balance(balance.total),
        formatted_confirmed: wallet::format_balance(balance.confirmed),
    })
}

/// Send DOLI to a recipient address.
///
/// The entire signing flow happens in Rust:
/// 1. Fetch UTXOs via RPC
/// 2. Build transaction (select inputs, create outputs with change)
/// 3. Sign with private key (in memory, zeroized after)
/// 4. Submit signed transaction via RPC
/// 5. Return tx hash (public data only)
#[tauri::command]
pub async fn send_doli(
    to: String,
    amount: String,
    fee: Option<String>,
    state: State<'_, AppState>,
) -> Result<SendResponse, String> {
    // Parse amount
    let amount_units = wallet::coins_to_units(&amount)?;
    let fee_units = match fee {
        Some(f) => wallet::coins_to_units(&f)?,
        None => 1, // Flat fee: 1 satoshi
    };

    // Get wallet info (public key hash for UTXO query)
    let (pubkey_hash, sender_hash) = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let pkh = w.primary_pubkey_hash().map_err(|e| e.to_string())?;
        let sender_bytes: [u8; 32] = hex::decode(&pkh)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "Invalid pubkey hash length".to_string())?;
        (pkh, sender_bytes)
    };

    // Decode recipient address
    let recipient_hash = decode_address(&to)?;

    // Fetch UTXOs
    let utxos = {
        let rpc = state.rpc_client.read().await;
        rpc.get_utxos(&pubkey_hash, true)
            .await
            .map_err(|e| e.to_string())?
    };

    // Build transaction
    let mut builder = wallet::TxBuilder::build_transfer(
        &utxos,
        recipient_hash,
        amount_units,
        fee_units,
        sender_hash,
    )
    .map_err(|e| e.to_string())?;

    // Sign transaction with private key (stays in Rust, zeroized on drop)
    let tx_hex = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let keypair = w.primary_keypair().map_err(|e| e.to_string())?;
        builder
            .sign_and_build(&keypair)
            .map_err(|e| e.to_string())?
    };

    // Submit via RPC
    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(SendResponse {
        tx_hash,
        amount: amount_units,
        fee: fee_units,
        formatted_amount: wallet::format_balance(amount_units),
    })
}

/// Get transaction history for the primary wallet address.
#[tauri::command]
pub async fn get_history(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<HistoryEntryResponse>, String> {
    let pubkey_hash = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_pubkey_hash().map_err(|e| e.to_string())?
    };

    let history = {
        let rpc = state.rpc_client.read().await;
        rpc.get_history(&pubkey_hash, limit.unwrap_or(50))
            .await
            .map_err(|e| e.to_string())?
    };

    let entries: Vec<HistoryEntryResponse> = history
        .into_iter()
        .map(|h| {
            let net = if h.amount_received > h.amount_sent {
                format!(
                    "+{}",
                    wallet::format_balance(h.amount_received - h.amount_sent)
                )
            } else if h.amount_sent > h.amount_received {
                format!(
                    "-{}",
                    wallet::format_balance(h.amount_sent - h.amount_received)
                )
            } else {
                wallet::format_balance(0)
            };

            HistoryEntryResponse {
                hash: h.hash,
                tx_type: h.tx_type,
                height: h.height,
                timestamp: h.timestamp,
                amount_received: h.amount_received,
                amount_sent: h.amount_sent,
                fee: h.fee,
                confirmations: h.confirmations,
                formatted_received: wallet::format_balance(h.amount_received),
                formatted_sent: wallet::format_balance(h.amount_sent),
                net_amount: net,
            }
        })
        .collect();

    Ok(entries)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Decode a bech32m address or hex pubkey hash into 32-byte array.
/// Uses `crypto::address::resolve()` which handles both bech32m and hex formats.
fn decode_address(address: &str) -> Result<[u8; 32], String> {
    let hash =
        crypto::address::resolve(address, None).map_err(|e| format!("Invalid address: {}", e))?;
    Ok(hash.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_address_hex() {
        let hex_addr = "aa".repeat(32);
        let result = decode_address(&hex_addr);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0xAAu8; 32]);
    }

    #[test]
    fn test_decode_address_invalid_hex() {
        let result = decode_address("not_hex");
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_address_wrong_length() {
        let short = "aa".repeat(16);
        let result = decode_address(&short);
        assert!(result.is_err());
    }
}
