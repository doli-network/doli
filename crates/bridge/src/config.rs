//! Bridge watcher configuration.

use serde::{Deserialize, Serialize};

/// Configuration for the bridge watcher daemon.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatcherConfig {
    /// DOLI node RPC endpoint (e.g. "http://127.0.0.1:8501")
    pub doli_rpc: String,

    /// DOLI wallet path for auto-claiming
    pub doli_wallet: String,

    /// Bitcoin Core RPC endpoint (e.g. "http://127.0.0.1:8332")
    pub bitcoin_rpc: String,

    /// Bitcoin Core RPC auth (user:password)
    pub bitcoin_auth: String,

    /// Poll interval in seconds (how often to check both chains)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// Path to the swap database file (JSON)
    #[serde(default = "default_swap_db")]
    pub swap_db_path: String,

    /// Auto-claim: automatically claim when preimage is discovered
    #[serde(default = "default_true")]
    pub auto_claim: bool,

    /// Auto-refund: automatically refund expired HTLCs
    #[serde(default = "default_true")]
    pub auto_refund: bool,

    /// Minimum confirmations on Bitcoin before considering a lock valid
    #[serde(default = "default_btc_confirmations")]
    pub btc_min_confirmations: u32,
}

fn default_poll_interval() -> u64 {
    10
}
fn default_swap_db() -> String {
    "bridge_swaps.json".to_string()
}
fn default_true() -> bool {
    true
}
fn default_btc_confirmations() -> u32 {
    1
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            doli_rpc: "http://127.0.0.1:8501".to_string(),
            doli_wallet: "wallet.json".to_string(),
            bitcoin_rpc: "http://127.0.0.1:8332".to_string(),
            bitcoin_auth: "user:password".to_string(),
            poll_interval_secs: default_poll_interval(),
            swap_db_path: default_swap_db(),
            auto_claim: true,
            auto_refund: true,
            btc_min_confirmations: default_btc_confirmations(),
        }
    }
}
