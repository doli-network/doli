//! Bitcoin chain monitor.
//!
//! Connects to Bitcoin Core via JSON-RPC to:
//! - Detect HTLC outputs matching known swap hashes
//! - Detect preimage reveals from HTLC claims
//! - Query block height and confirmations
//!
//! Bitcoin HTLCs use P2WSH with the standard script:
//! ```text
//! OP_IF
//!   OP_SHA256 <hash> OP_EQUALVERIFY <receiver_pubkey> OP_CHECKSIG
//! OP_ELSE
//!   <timeout> OP_CHECKLOCKTIMEVERIFY OP_DROP <sender_pubkey> OP_CHECKSIG
//! OP_ENDIF
//! ```

use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::error::{BridgeError, Result};

/// Bitcoin Core RPC client.
pub struct BitcoinClient {
    endpoint: String,
    auth: String,
    client: reqwest::Client,
}

/// A detected Bitcoin HTLC that matches a known swap.
#[derive(Debug, Clone)]
pub struct DetectedBtcHtlc {
    /// Bitcoin transaction hash
    pub txid: String,
    /// Output index (vout)
    pub vout: u32,
    /// Amount in satoshis
    pub amount_sat: u64,
    /// Number of confirmations
    pub confirmations: u32,
    /// SHA256 hash from the script
    pub hash: String,
    /// Locktime from the script
    pub locktime: u64,
}

/// A detected preimage from a Bitcoin HTLC claim.
#[derive(Debug, Clone)]
pub struct BtcPreimageReveal {
    /// The HTLC being spent
    pub htlc_txid: String,
    pub htlc_vout: u32,
    /// The spending (claim) transaction
    pub claim_txid: String,
    /// The revealed preimage (hex)
    pub preimage: String,
}

impl BitcoinClient {
    pub fn new(endpoint: &str, auth: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            endpoint: endpoint.to_string(),
            auth: auth.to_string(),
            client,
        }
    }

    /// Make a Bitcoin Core JSON-RPC call.
    async fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let (user, pass) = self.auth.split_once(':').unwrap_or((&self.auth, ""));

        let resp = self
            .client
            .post(&self.endpoint)
            .basic_auth(user, Some(pass))
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
                return Err(BridgeError::BitcoinRpc(msg.to_string()));
            }
        }

        resp.get("result")
            .cloned()
            .ok_or_else(|| BridgeError::BitcoinRpc("null result".to_string()))
    }

    /// Get current block count.
    pub async fn get_block_count(&self) -> Result<u64> {
        let result = self.call("getblockcount", serde_json::json!([])).await?;
        result
            .as_u64()
            .ok_or_else(|| BridgeError::BitcoinRpc("invalid blockcount".to_string()))
    }

    /// Get block hash at height.
    pub async fn get_block_hash(&self, height: u64) -> Result<String> {
        let result = self
            .call("getblockhash", serde_json::json!([height]))
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| BridgeError::BitcoinRpc("invalid blockhash".to_string()))
    }

    /// Get a block with full transaction data.
    pub async fn get_block(&self, hash: &str) -> Result<serde_json::Value> {
        // verbosity=2 gives decoded transactions
        self.call("getblock", serde_json::json!([hash, 2])).await
    }

    /// Get a raw transaction (decoded).
    pub async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value> {
        self.call("getrawtransaction", serde_json::json!([txid, true]))
            .await
    }

    /// Compute SHA256 hash of a preimage (for Bitcoin HTLC matching).
    ///
    /// DOLI uses BLAKE3("DOLI_HASHLOCK" || preimage), but Bitcoin uses SHA256(preimage).
    /// Both sides use the same raw preimage bytes — the hash functions differ.
    pub fn sha256_hash(preimage: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(preimage);
        hex::encode(hasher.finalize())
    }

    /// Scan recent blocks for HTLC outputs matching known hashes.
    ///
    /// `watch_hashes` is a map of SHA256(preimage) → swap_id.
    /// Returns detected HTLCs where the script contains one of the watched hashes.
    pub async fn scan_for_htlcs(
        &self,
        from_height: u64,
        watch_hashes: &[(String, String)], // (sha256_hash, swap_id)
    ) -> Result<Vec<(DetectedBtcHtlc, String)>> {
        if watch_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let current_height = self.get_block_count().await?;
        if current_height <= from_height {
            return Ok(Vec::new());
        }

        let mut detected = Vec::new();
        let scan_from = from_height + 1;
        let scan_to = current_height.min(scan_from + 10); // Bitcoin blocks are slower, scan fewer

        for height in scan_from..=scan_to {
            match self.scan_block_for_htlcs(height, watch_hashes).await {
                Ok(htlcs) => detected.extend(htlcs),
                Err(e) => {
                    debug!("Error scanning Bitcoin block {}: {}", height, e);
                }
            }
        }

        if !detected.is_empty() {
            info!(
                "Found {} matching Bitcoin HTLC(s) in blocks {}-{}",
                detected.len(),
                scan_from,
                scan_to
            );
        }

        Ok(detected)
    }

    /// Scan a single Bitcoin block for matching HTLCs.
    async fn scan_block_for_htlcs(
        &self,
        height: u64,
        watch_hashes: &[(String, String)],
    ) -> Result<Vec<(DetectedBtcHtlc, String)>> {
        let hash = self.get_block_hash(height).await?;
        let block = self.get_block(&hash).await?;

        let txs = block
            .get("tx")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let confirmations = block
            .get("confirmations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let mut detected = Vec::new();

        for tx in &txs {
            let txid = tx.get("txid").and_then(|v| v.as_str()).unwrap_or_default();
            let vouts = tx
                .get("vout")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for vout in &vouts {
                let n = vout.get("n").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let value_btc = vout.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let amount_sat = (value_btc * 1e8) as u64;

                // Check the scriptPubKey for P2WSH or P2SH that might contain an HTLC
                let script_hex = vout
                    .get("scriptPubKey")
                    .and_then(|s| s.get("hex"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let asm = vout
                    .get("scriptPubKey")
                    .and_then(|s| s.get("asm"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Look for watched hashes in the script ASM or hex
                for (watched_hash, swap_id) in watch_hashes {
                    if asm.contains(watched_hash) || script_hex.contains(watched_hash) {
                        detected.push((
                            DetectedBtcHtlc {
                                txid: txid.to_string(),
                                vout: n,
                                amount_sat,
                                confirmations,
                                hash: watched_hash.clone(),
                                locktime: 0, // TODO: parse from witness script
                            },
                            swap_id.clone(),
                        ));
                    }
                }
            }
        }

        Ok(detected)
    }

    /// Scan recent blocks for preimage reveals from HTLC spends.
    ///
    /// When an HTLC is claimed, the witness data contains the preimage.
    /// We look for transactions spending known HTLC outputs.
    pub async fn scan_for_preimage_reveals(
        &self,
        from_height: u64,
        watch_outpoints: &[(String, u32, String)], // (txid, vout, swap_id)
    ) -> Result<Vec<(BtcPreimageReveal, String)>> {
        if watch_outpoints.is_empty() {
            return Ok(Vec::new());
        }

        let current_height = self.get_block_count().await?;
        if current_height <= from_height {
            return Ok(Vec::new());
        }

        let mut reveals = Vec::new();
        let scan_from = from_height + 1;
        let scan_to = current_height.min(scan_from + 10);

        for height in scan_from..=scan_to {
            match self.scan_block_for_reveals(height, watch_outpoints).await {
                Ok(r) => reveals.extend(r),
                Err(e) => {
                    debug!("Error scanning Bitcoin block {} for reveals: {}", height, e);
                }
            }
        }

        if !reveals.is_empty() {
            info!("Found {} Bitcoin preimage reveal(s)", reveals.len());
        }

        Ok(reveals)
    }

    /// Scan a single block for preimage reveals.
    async fn scan_block_for_reveals(
        &self,
        height: u64,
        watch_outpoints: &[(String, u32, String)],
    ) -> Result<Vec<(BtcPreimageReveal, String)>> {
        let hash = self.get_block_hash(height).await?;
        let block = self.get_block(&hash).await?;

        let txs = block
            .get("tx")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut reveals = Vec::new();

        for tx in &txs {
            let txid = tx.get("txid").and_then(|v| v.as_str()).unwrap_or_default();
            let vins = tx
                .get("vin")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for vin in &vins {
                let prev_txid = vin.get("txid").and_then(|v| v.as_str()).unwrap_or_default();
                let prev_vout = vin.get("vout").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                // Check if this input spends one of our watched outpoints
                for (watched_txid, watched_vout, swap_id) in watch_outpoints {
                    if prev_txid == watched_txid && prev_vout == *watched_vout {
                        // Found a spend! Extract preimage from witness data.
                        // In a P2WSH HTLC claim, the witness stack typically is:
                        // [signature, preimage, 1 (OP_TRUE for IF branch), witness_script]
                        let witness = vin
                            .get("txinwitness")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();

                        // The preimage is usually the second element (index 1)
                        // after the signature and before the branch selector
                        if witness.len() >= 3 {
                            let preimage_hex = witness[1].as_str().unwrap_or_default().to_string();

                            // Validate: should be 32 bytes (64 hex chars)
                            if preimage_hex.len() == 64 {
                                info!(
                                    "Preimage revealed on Bitcoin! swap={}, txid={}",
                                    swap_id, txid
                                );
                                reveals.push((
                                    BtcPreimageReveal {
                                        htlc_txid: prev_txid.to_string(),
                                        htlc_vout: prev_vout,
                                        claim_txid: txid.to_string(),
                                        preimage: preimage_hex,
                                    },
                                    swap_id.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(reveals)
    }

    /// Verify connectivity to Bitcoin Core.
    pub async fn ping(&self) -> bool {
        self.get_block_count().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hash() {
        let preimage = [0u8; 32];
        let hash = BitcoinClient::sha256_hash(&preimage);
        // SHA256 of 32 zero bytes
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, "0".repeat(64)); // shouldn't be all zeros
    }

    #[test]
    fn test_sha256_known_value() {
        // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let hash = BitcoinClient::sha256_hash(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
