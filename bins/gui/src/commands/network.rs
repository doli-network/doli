//! Network and chain info Tauri commands.
//!
//! Handles RPC endpoint management, connection testing, chain info queries,
//! and network switching.

use tauri::State;

use crate::commands::{ChainInfoResponse, ConnectionStatus, ConnectionTestResult};
use crate::state::AppState;

/// Get chain information from the connected node.
#[tauri::command]
pub async fn get_chain_info(state: State<'_, AppState>) -> Result<ChainInfoResponse, String> {
    let info = {
        let rpc = state.rpc_client.read().await;
        rpc.get_chain_info().await.map_err(|e| e.to_string())?
    };

    Ok(ChainInfoResponse {
        network: info.network,
        best_hash: info.best_hash,
        best_height: info.best_height,
        best_slot: info.best_slot,
        genesis_hash: info.genesis_hash,
    })
}

/// Set the RPC endpoint URL. Returns true if the connection test succeeds.
#[tauri::command]
pub async fn set_rpc_endpoint(url: String, state: State<'_, AppState>) -> Result<bool, String> {
    // Test connection to new endpoint first
    let test_client = wallet::RpcClient::new(&url);
    let reachable = test_client.test_connection().await.is_ok();

    if reachable {
        *state.rpc_client.write().await = wallet::RpcClient::new(&url);
        let mut config = state.config.write().await;
        config.custom_rpc_url = Some(url);
        let _ = config.save();
    }

    Ok(reachable)
}

/// Switch to a different network (mainnet, testnet, devnet).
#[tauri::command]
pub async fn set_network(network: String, state: State<'_, AppState>) -> Result<(), String> {
    if !["mainnet", "testnet", "devnet"].contains(&network.as_str()) {
        return Err(format!(
            "Invalid network: '{}'. Must be mainnet, testnet, or devnet",
            network
        ));
    }

    let new_url = {
        let mut config = state.config.write().await;
        config.network = network;
        config.custom_rpc_url = None;
        let _ = config.save();
        config.effective_rpc_url()
    };

    *state.rpc_client.write().await = wallet::RpcClient::new(&new_url);

    Ok(())
}

/// Test connectivity to a specific endpoint.
#[tauri::command]
pub async fn test_connection(url: String) -> Result<ConnectionTestResult, String> {
    let client = wallet::RpcClient::new(&url);
    match client.get_chain_info().await {
        Ok(info) => Ok(ConnectionTestResult {
            success: true,
            network: Some(info.network),
            height: Some(info.best_height),
            error: None,
        }),
        Err(e) => Ok(ConnectionTestResult {
            success: false,
            network: None,
            height: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Get the current connection status.
#[tauri::command]
pub async fn get_connection_status(state: State<'_, AppState>) -> Result<ConnectionStatus, String> {
    let (endpoint, network) = {
        let config = state.config.read().await;
        (config.effective_rpc_url(), config.network.clone())
    };

    let chain_info_result = {
        let rpc = state.rpc_client.read().await;
        rpc.get_chain_info().await
    };

    match chain_info_result {
        Ok(info) => Ok(ConnectionStatus {
            connected: true,
            endpoint,
            network,
            chain_height: Some(info.best_height),
            status: "connected".to_string(),
        }),
        Err(_) => Ok(ConnectionStatus {
            connected: false,
            endpoint,
            network,
            chain_height: None,
            status: "disconnected".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_valid_networks() {
        let valid = ["mainnet", "testnet", "devnet"];
        for n in &valid {
            assert!(valid.contains(n));
        }
        assert!(!valid.contains(&"invalid"));
    }
}
