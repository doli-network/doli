//! Tauri commands for node lifecycle control.
//!
//! These commands let the frontend start/stop the embedded doli-node process,
//! query its status, switch networks (with restart), and read log output.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

/// Node status returned to the frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatus {
    pub running: bool,
    pub network: String,
    pub rpc_url: String,
    pub log_path: String,
}

/// Start the embedded doli-node process.
#[tauri::command]
pub async fn start_node(state: State<'_, AppState>) -> Result<(), String> {
    let mut mgr = state.node_manager.write().await;
    mgr.start()
}

/// Stop the embedded doli-node process.
#[tauri::command]
pub async fn stop_node(state: State<'_, AppState>) -> Result<(), String> {
    let mut mgr = state.node_manager.write().await;
    mgr.stop()
}

/// Get the current status of the embedded node.
#[tauri::command]
pub async fn node_status(state: State<'_, AppState>) -> Result<NodeStatus, String> {
    let mut mgr = state.node_manager.write().await;
    Ok(NodeStatus {
        running: mgr.is_running(),
        network: mgr.network().to_string(),
        rpc_url: mgr.rpc_url(),
        log_path: mgr.log_path().to_string_lossy().to_string(),
    })
}

/// Restart the node, optionally switching to a different network.
///
/// If `network` is provided, the node will start on that network.
/// The RPC client is also updated to point at the new local endpoint.
#[tauri::command]
pub async fn restart_node(
    network: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let new_rpc_url = {
        let mut mgr = state.node_manager.write().await;
        let net = network
            .as_deref()
            .unwrap_or_else(|| mgr.network().to_string().leak());
        mgr.restart(net)?;
        mgr.rpc_url()
    };

    // Update the RPC client to use the (possibly new) local endpoint.
    *state.rpc_client.write().await = wallet::RpcClient::new(&new_rpc_url);

    Ok(())
}

/// Return the last N lines from the node's log file.
#[tauri::command]
pub async fn get_node_logs(
    lines: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let mgr = state.node_manager.read().await;
    let n = lines.unwrap_or(100);
    mgr.tail_log(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_status_serialization() {
        let status = NodeStatus {
            running: true,
            network: "testnet".to_string(),
            rpc_url: "http://127.0.0.1:18500".to_string(),
            log_path: "/home/user/.doli/node.log".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"running\":true"));
        assert!(json.contains("\"network\":\"testnet\""));
        assert!(json.contains("\"rpcUrl\":\"http://127.0.0.1:18500\""));
        assert!(json.contains("\"logPath\":\"/home/user/.doli/node.log\""));
    }

    #[test]
    fn test_node_status_deserialization() {
        let json = r#"{"running":false,"network":"mainnet","rpcUrl":"http://127.0.0.1:8500","logPath":"/tmp/node.log"}"#;
        let status: NodeStatus = serde_json::from_str(json).unwrap();
        assert!(!status.running);
        assert_eq!(status.network, "mainnet");
        assert_eq!(status.rpc_url, "http://127.0.0.1:8500");
    }

    #[test]
    fn test_node_status_camel_case_keys() {
        let status = NodeStatus {
            running: false,
            network: "devnet".to_string(),
            rpc_url: "http://127.0.0.1:28500".to_string(),
            log_path: "/data/node.log".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        // Verify camelCase (not snake_case).
        assert!(json.contains("rpcUrl"));
        assert!(json.contains("logPath"));
        assert!(!json.contains("rpc_url"));
        assert!(!json.contains("log_path"));
    }
}
