//! Governance handlers: submitVote, getUpdateStatus, getMaintainerSet, submitMaintainerChange

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Submit a vote for a pending update (governance veto system)
    pub(super) async fn submit_vote(&self, params: Value) -> Result<Value, RpcError> {
        let params: SubmitVoteParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // 1. Decode and validate the producer's public key
        let pubkey = crypto::PublicKey::from_hex(&params.vote.producer_id)
            .map_err(|_| RpcError::invalid_params("Invalid producer_id public key"))?;

        // 2. Verify the producer is registered
        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;
        {
            let producers = producer_set.read().await;
            let pubkey_hash =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
            if producers.get(&pubkey_hash).is_none() {
                return Err(RpcError::invalid_params("Producer not registered"));
            }
        }

        // 3. Verify the Ed25519 signature over "version:vote:timestamp"
        let signing_message = format!(
            "{}:{}:{}",
            params.vote.version, params.vote.vote, params.vote.timestamp
        );
        let sig_bytes = hex::decode(&params.vote.signature)
            .map_err(|_| RpcError::invalid_params("Invalid signature hex"))?;
        let signature = crypto::Signature::try_from_slice(&sig_bytes)
            .map_err(|_| RpcError::invalid_params("Invalid signature format"))?;

        crypto::signature::verify(signing_message.as_bytes(), &signature, &pubkey)
            .map_err(|_| RpcError::invalid_params("Signature verification failed"))?;

        // 4. Serialize and broadcast the verified vote
        let vote_data = serde_json::to_vec(&params.vote)
            .map_err(|e| RpcError::internal_error(format!("Failed to serialize vote: {}", e)))?;

        (self.broadcast_vote)(vote_data);

        Ok(serde_json::json!({
            "status": "submitted",
            "message": "Vote submitted and broadcast to network"
        }))
    }

    /// Get the current update status (pending updates, votes, etc.)
    ///
    /// Calls the update status callback to read live state from UpdateService.
    pub(super) async fn get_update_status(&self) -> Result<Value, RpcError> {
        Ok((self.update_status)())
    }

    /// Get current maintainer set
    ///
    /// Returns the maintainer set derived from the blockchain.
    /// First 5 registered producers become maintainers automatically.
    pub(super) async fn get_maintainer_set(&self) -> Result<Value, RpcError> {
        use doli_core::maintainer::{INITIAL_MAINTAINER_COUNT, MAX_MAINTAINERS, MIN_MAINTAINERS};

        // Read from on-chain MaintainerState if available
        if let Some(ms) = &self.maintainer_state {
            let state = ms.read().await;
            let maintainers: Vec<_> = state
                .set
                .members
                .iter()
                .map(|pk| {
                    serde_json::json!({
                        "pubkey": pk.to_hex(),
                    })
                })
                .collect();

            return Ok(serde_json::json!({
                "maintainers": maintainers,
                "threshold": state.set.threshold,
                "member_count": state.set.members.len(),
                "max_maintainers": MAX_MAINTAINERS,
                "min_maintainers": MIN_MAINTAINERS,
                "initial_maintainer_count": INITIAL_MAINTAINER_COUNT,
                "last_change_block": state.set.last_updated,
                "source": "on-chain"
            }));
        }

        // Fallback: derive ad-hoc from producer set (pre-bootstrap or no MaintainerState)
        let producer_set = match &self.producer_set {
            Some(ps) => ps,
            None => {
                return Ok(serde_json::json!({
                    "maintainers": [],
                    "threshold": 0,
                    "member_count": 0,
                    "source": "none"
                }));
            }
        };

        let producers = producer_set.read().await;
        let mut sorted_producers = producers.all_producers();
        sorted_producers.sort_by_key(|p| p.registered_at);

        let maintainers: Vec<_> = sorted_producers
            .into_iter()
            .take(INITIAL_MAINTAINER_COUNT)
            .map(|p| {
                serde_json::json!({
                    "pubkey": p.public_key.to_hex(),
                    "registered_at_block": p.registered_at,
                    "is_active_producer": p.is_active()
                })
            })
            .collect();

        let member_count = maintainers.len();
        let threshold = doli_core::maintainer::MaintainerSet::calculate_threshold(member_count);

        Ok(serde_json::json!({
            "maintainers": maintainers,
            "threshold": threshold,
            "member_count": member_count,
            "max_maintainers": MAX_MAINTAINERS,
            "min_maintainers": MIN_MAINTAINERS,
            "initial_maintainer_count": INITIAL_MAINTAINER_COUNT,
            "last_change_block": 0,
            "source": "derived"
        }))
    }

    /// Submit a maintainer change (add or remove)
    ///
    /// Requires 3/5 signatures from current maintainers.
    pub(super) async fn submit_maintainer_change(&self, params: Value) -> Result<Value, RpcError> {
        #[derive(serde::Deserialize)]
        struct SubmitMaintainerChangeParams {
            action: String,        // "add" or "remove"
            target_pubkey: String, // Hex-encoded public key
            signatures: Vec<SignatureEntry>,
            reason: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct SignatureEntry {
            pubkey: String,
            signature: String,
        }

        let params: SubmitMaintainerChangeParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Validate action
        if params.action != "add" && params.action != "remove" {
            return Err(RpcError::invalid_params("action must be 'add' or 'remove'"));
        }

        // Parse target public key
        let target = crypto::PublicKey::from_hex(&params.target_pubkey)
            .map_err(|e| RpcError::invalid_params(format!("invalid target pubkey: {}", e)))?;

        // Parse and validate signatures
        let mut signatures = Vec::new();
        for entry in params.signatures {
            let pubkey = crypto::PublicKey::from_hex(&entry.pubkey)
                .map_err(|e| RpcError::invalid_params(format!("invalid signer pubkey: {}", e)))?;
            let signature = crypto::Signature::from_hex(&entry.signature)
                .map_err(|e| RpcError::invalid_params(format!("invalid signature: {}", e)))?;
            signatures.push(doli_core::maintainer::MaintainerSignature { pubkey, signature });
        }

        // Check signature count (need at least 3)
        if signatures.len() < doli_core::maintainer::MAINTAINER_THRESHOLD {
            return Err(RpcError::invalid_params(format!(
                "insufficient signatures: need {}, got {}",
                doli_core::maintainer::MAINTAINER_THRESHOLD,
                signatures.len()
            )));
        }

        // Create the transaction
        let tx = if params.action == "add" {
            doli_core::Transaction::new_add_maintainer(target, signatures)
        } else {
            doli_core::Transaction::new_remove_maintainer(target, signatures, params.reason)
        };

        let tx_hash = tx.hash();

        // Get current height for mempool validation
        let current_height = {
            let chain_state = self.chain_state.read().await;
            chain_state.best_height
        };

        // Submit to mempool (maintainer txs are state-only, no UTXO inputs)
        let mut mempool = self.mempool.write().await;
        mempool
            .add_system_transaction(tx, current_height)
            .map_err(|e| RpcError::internal_error(format!("mempool error: {}", e)))?;

        Ok(serde_json::json!({
            "status": "accepted",
            "tx_hash": tx_hash.to_hex(),
            "message": format!("Maintainer {} transaction submitted", params.action)
        }))
    }
}
