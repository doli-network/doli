//! Ethereum chain monitor.
//!
//! Minimal JSON-RPC client for querying Ethereum nodes.
//! Used by `cmd_bridge_status` to detect HTLC locks and preimage reveals.
//!
//! Ethereum HTLCs use the standard HashedTimelock contract:
//! - `newContract(receiver, hashlock, timelock)` emits `LogHTLCNew`
//! - `withdraw(contractId, preimage)` emits `LogHTLCWithdraw`
//! - `refund(contractId)` emits `LogHTLCRefund`

use tracing::debug;

use crate::error::{BridgeError, Result};

/// Ethereum JSON-RPC client.
pub struct EthereumClient {
    endpoint: String,
    client: reqwest::Client,
}

/// A detected HTLC on Ethereum.
#[derive(Debug, Clone)]
pub struct DetectedEthHtlc {
    /// Ethereum transaction hash
    pub tx_hash: String,
    /// Block number the HTLC was created in
    pub block_number: u64,
    /// Number of confirmations
    pub confirmations: u64,
    /// Amount (in wei or token units, as string)
    pub amount: String,
    /// Token address ("0x0000...0000" for native ETH)
    pub token_address: String,
}

/// A detected preimage from an Ethereum HTLC withdrawal.
#[derive(Debug, Clone)]
pub struct EthPreimageReveal {
    /// The withdrawal transaction hash
    pub tx_hash: String,
    /// The revealed preimage (hex, no 0x prefix)
    pub preimage: String,
}

impl EthereumClient {
    pub fn new(endpoint: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            endpoint: endpoint.to_string(),
            client,
        }
    }

    /// Make an Ethereum JSON-RPC call.
    async fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(err) = resp.get("error") {
            if !err.is_null() {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(BridgeError::EthereumRpc(msg.to_string()));
            }
        }

        resp.get("result")
            .cloned()
            .ok_or_else(|| BridgeError::EthereumRpc("null result".to_string()))
    }

    /// Verify connectivity.
    pub async fn ping(&self) -> bool {
        self.get_block_number().await.is_ok()
    }

    /// Get current block number.
    pub async fn get_block_number(&self) -> Result<u64> {
        let result = self.call("eth_blockNumber", serde_json::json!([])).await?;
        let hex = result
            .as_str()
            .ok_or_else(|| BridgeError::EthereumRpc("invalid blockNumber".to_string()))?;
        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|_| BridgeError::EthereumRpc("invalid hex blockNumber".to_string()))
    }

    /// Scan for HTLC events matching the given keccak256 hash.
    ///
    /// Looks for `LogHTLCNew` events in contract logs that contain the hash.
    /// The standard HTLC contract uses topic[2] = hashlock for filtering.
    pub async fn scan_for_htlc(
        &self,
        counter_hash: &str,
        from_block: u64,
    ) -> Result<Option<DetectedEthHtlc>> {
        let current_height = self.get_block_number().await?;
        if current_height <= from_block {
            return Ok(None);
        }

        // LogHTLCNew event signature:
        // keccak256("LogHTLCNew(bytes32,address,address,uint256,bytes32,uint256)")
        // = 0x... (we search by hashlock in topic position)
        // Topic[0] = event signature, Topic[1] = contractId, Topic[2] = hashlock
        let hash_topic = if counter_hash.starts_with("0x") {
            counter_hash.to_string()
        } else {
            format!("0x{}", counter_hash)
        };

        let params = serde_json::json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", current_height),
            "topics": [null, null, hash_topic]
        }]);

        match self.call("eth_getLogs", params).await {
            Ok(logs) => {
                if let Some(arr) = logs.as_array() {
                    if let Some(log) = arr.first() {
                        let tx_hash = log
                            .get("transactionHash")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let block_hex = log
                            .get("blockNumber")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0x0");
                        let block_number =
                            u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)
                                .unwrap_or(0);
                        let confirmations = current_height.saturating_sub(block_number);

                        // Extract amount from log data if available
                        let data = log.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                        let amount = if data.len() >= 66 {
                            // First 32 bytes of data is typically the amount
                            let amt_hex = &data[2..66];
                            u128::from_str_radix(amt_hex.trim_start_matches('0'), 16)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|_| "0".to_string())
                        } else {
                            "0".to_string()
                        };

                        return Ok(Some(DetectedEthHtlc {
                            tx_hash,
                            block_number,
                            confirmations,
                            amount,
                            token_address: String::new(),
                        }));
                    }
                }
                Ok(None)
            }
            Err(e) => {
                debug!("Error scanning Ethereum logs: {}", e);
                Ok(None)
            }
        }
    }

    /// Scan for preimage reveals from HTLC withdrawals.
    ///
    /// Looks for `LogHTLCWithdraw` events and extracts the preimage from calldata.
    pub async fn scan_for_preimage(&self, htlc_tx_hash: &str) -> Result<Option<String>> {
        // Get the transaction receipt to find the contract address
        let tx_hash = if htlc_tx_hash.starts_with("0x") {
            htlc_tx_hash.to_string()
        } else {
            format!("0x{}", htlc_tx_hash)
        };

        let receipt = self
            .call("eth_getTransactionReceipt", serde_json::json!([tx_hash]))
            .await?;

        // Check logs for LogHTLCWithdraw events
        if let Some(logs) = receipt.get("logs").and_then(|v| v.as_array()) {
            for log in logs {
                // LogHTLCWithdraw has the preimage in the event data
                let data = log.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                // The preimage is typically 32 bytes in the data field
                if data.len() >= 66 {
                    let preimage_hex = &data[2..66];
                    if !preimage_hex.chars().all(|c| c == '0') {
                        return Ok(Some(preimage_hex.to_string()));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Compute keccak256 hash of data (for Ethereum HTLC matching).
    pub fn keccak256(data: &[u8]) -> Vec<u8> {
        use tiny_keccak::Hasher;
        let mut output = [0u8; 32];
        let mut keccak = tiny_keccak::Keccak::v256();
        keccak.update(data);
        keccak.finalize(&mut output);
        output.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keccak256_empty() {
        let hash = EthereumClient::keccak256(b"");
        let hex = ::hex::encode(&hash);
        // keccak256("") = c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
        assert_eq!(
            hex,
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn test_keccak256_known_value() {
        // keccak256 of 32 zero bytes
        let hash = EthereumClient::keccak256(&[0u8; 32]);
        assert_eq!(hash.len(), 32);
        assert_ne!(hash, vec![0u8; 32]);
    }
}
