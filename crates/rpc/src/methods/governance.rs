//! Governance handlers: submitVote, getUpdateStatus, getMaintainerSet, submitMaintainerChange

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

#[cfg(test)]
#[path = "tests_inc_i195_broadcast.rs"]
mod tests_inc_i195_broadcast;

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
    /// Two shapes, distinguished by the `source` field:
    ///
    /// * `"on-chain"` — the node's persisted `MaintainerState`. This IS the root
    ///   the node enforces for governance and the one the updater installs
    ///   against. `enforced: true`.
    /// * `"derived"` — no `MaintainerState` is attached, so the set is computed
    ///   from the producer set. **Advisory** (`enforced: false`): it is what the
    ///   seed WOULD produce, not what any node is enforcing.
    ///
    /// INC-I-172 M2 review F4. The `derived` branch used to be a FOURTH
    /// derivation: `all_producers()` (a `HashMap::values()` walk) plus a STABLE
    /// `sort_by_key(registered_at)` plus `take(5)` — the exact shape M2 froze as
    /// pre-activation-only, still live at every height here. Because every
    /// genesis producer ties at `registered_at == 0`, two honest nodes on the
    /// same chain printed different `maintainers` arrays, so the operator
    /// runbook's "compare `getMaintainerSet` against a known-good node" produced
    /// false mismatches. It now routes through
    /// `derive_canonical_maintainer_set`, the same total order the node uses at
    /// and above the gate, and reports whether that order is the one in force at
    /// the current height.
    pub(super) async fn get_maintainer_set(&self) -> Result<Value, RpcError> {
        use doli_core::maintainer::{
            maintainer_set_digest, INITIAL_MAINTAINER_COUNT, MAX_MAINTAINERS, MIN_MAINTAINERS,
        };

        // INC-I-173 M3a / F6 (AUDIT-P1-003). `genesis_hash` is published on EVERY
        // branch, including `none` — the chain identity is always knowable, and it
        // is what makes the digest a per-CHAIN answer.
        // `maintainer_set_digest` is published wherever a set exists, so an
        // operator can compare two nodes' trust roots with ONE scalar instead of
        // diffing member lists.
        let genesis_hash = self.params.genesis_hash;

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
                "source": "on-chain",
                "enforced": true,
                "maintainer_derivation_activation_height":
                    self.maintainer_derivation_activation_height,
                "maintainer_set_digest":
                    hex::encode(maintainer_set_digest(&state.set, genesis_hash.as_bytes())),
                "genesis_hash": genesis_hash.to_hex()
            }));
        }

        // Fallback: no MaintainerState is attached. ADVISORY only.
        let producer_set = match &self.producer_set {
            Some(ps) => ps,
            None => {
                // NO `maintainer_set_digest` here: there is no set to digest, and a
                // digest over an absent set is a value that invites a comparison it
                // cannot support. `genesis_hash` IS published — chain identity is
                // always knowable.
                return Ok(serde_json::json!({
                    "maintainers": [],
                    "threshold": 0,
                    "member_count": 0,
                    "source": "none",
                    "enforced": false,
                    "genesis_hash": genesis_hash.to_hex(),
                    "advisory_note": "No MaintainerState and no ProducerSet are attached to \
                                      this RPC context; no maintainer root can be reported."
                }));
            }
        };

        let height = self.chain_state.read().await.best_height;
        let producers = producer_set.read().await;

        // Canonical TOTAL order (registered_at, pubkey_bytes), identical to the
        // node's own derivation at and above the gate — so two honest nodes on
        // the same chain print the same array.
        let all = producers.all_producers();
        let candidates: Vec<(crypto::PublicKey, u64)> = all
            .iter()
            .map(|p| (p.public_key, p.registered_at))
            .collect();
        let derived = doli_core::maintainer::derive_canonical_maintainer_set(&candidates, height);

        let maintainers: Vec<_> = derived
            .members
            .iter()
            .map(|pk| {
                let info = all.iter().find(|p| p.public_key == *pk);
                serde_json::json!({
                    "pubkey": pk.to_hex(),
                    "registered_at_block": info.map(|p| p.registered_at),
                    "is_active_producer": info.map(|p| p.is_active())
                })
            })
            .collect();

        let activation_height = self.maintainer_derivation_activation_height;
        let advisory_note = if height >= activation_height {
            "ADVISORY. No MaintainerState is attached to this node, so nothing here is \
             enforced. The ordering shown is the canonical (registered_at, pubkey_bytes) \
             order the node uses at and above maintainer_derivation_activation_height."
        } else {
            "ADVISORY. No MaintainerState is attached to this node, so nothing here is \
             enforced. This chain is BELOW maintainer_derivation_activation_height, where \
             the node's own seed still uses the frozen HashMap-ordered stable sort, so the \
             enforced membership may differ from this canonical ordering. Compare nodes \
             only when source is \"on-chain\" on both."
        };

        Ok(serde_json::json!({
            "maintainers": maintainers,
            "threshold": derived.threshold,
            "member_count": derived.members.len(),
            "max_maintainers": MAX_MAINTAINERS,
            "min_maintainers": MIN_MAINTAINERS,
            "initial_maintainer_count": INITIAL_MAINTAINER_COUNT,
            "last_change_block": 0,
            "source": "derived",
            "enforced": false,
            "maintainer_derivation_activation_height": activation_height,
            "maintainer_set_digest":
                hex::encode(maintainer_set_digest(&derived, genesis_hash.as_bytes())),
            "genesis_hash": genesis_hash.to_hex(),
            "advisory_note": advisory_note
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

        // Submit to mempool (maintainer txs are state-only, no UTXO inputs).
        // The lock is scoped so it is released before the broadcast below,
        // mirroring `submit_transaction` (transaction.rs).
        {
            let mut mempool = self.mempool.write().await;
            mempool
                .add_system_transaction(tx.clone(), current_height)
                .map_err(|e| RpcError::internal_error(format!("mempool error: {}", e)))?;
        }

        // INC-I-195 — RELAY. Without this the transaction never leaves the node
        // that received the RPC, so any non-producer endpoint accepts it and
        // then silently never mines it. Mainnet seeds run `--relay-server`
        // without `--producer` and are the endpoint an operator reaches for, so
        // the INC-I-175 maintainer rotation would report success and never
        // apply (MEASURED on the local testnet 2026-08-29: accepted by the
        // seed, set unchanged ~36 blocks later; the same transactions applied
        // in 4-5 blocks each against a producer).
        //
        // Relaying is safe on the receiving side because a maintainer change is
        // 0-in/0-out: `handle_new_transaction`
        // (`bins/node/src/node/validation_checks.rs:1253-1259`) routes
        // `is_zero_flow()` transactions to the SAME `add_system_transaction`
        // lane this handler used. No new admission rule is introduced, and this
        // is transport only — it changes no consensus rule and no block
        // content, so it needs no activation height.
        (self.broadcast_tx)(tx);

        Ok(serde_json::json!({
            "status": "accepted",
            "tx_hash": tx_hash.to_hex(),
            "message": format!("Maintainer {} transaction submitted", params.action)
        }))
    }
}
