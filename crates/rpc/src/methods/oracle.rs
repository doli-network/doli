//! Phase 2.1 Oracle RPC handlers (M9-M11).
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.9.
//!
//! These methods are read-only consumers of the surface built by M1-M8:
//!   * M5 wrote OraclePrice as OutputType=15 with a deterministic per-pair
//!     UTXO address.
//!   * M6 aggregates PriceAttestation txs (TxType=16) at the epoch
//!     boundary and consume-recreates the OraclePrice UTXO.
//!   * M8 computes the structural-share metric and triggers HALT below
//!     55%.
//!
//! All three methods are PURELY ADDITIVE — no consensus-visible behavior,
//! no state mutation. The RPC SURFACE itself needs no activation gate; the
//! DATA the methods surface is naturally gated by
//! `NetworkParams.oracle_activation_height` (= u64::MAX in Phase 2.1).
//! Pre-activation: `getOraclePrice` returns null, `getOracleAttestations`
//! returns an empty list, `getOracleStatus` returns `active=false`.
//!
//! OUTPUT CONTRACT:
//!   getOraclePrice(pair_id)         -> { pair_id, price_cents,
//!                                        last_update_height,
//!                                        contributor_count, is_stale,
//!                                        trust_model }
//!                                      | null  (UTXO absent)
//!                                      | RpcError::invalid_params (bad pair_id)
//!   getOracleAttestations(epoch, pair_id)
//!                                   -> { epoch, pair_id, attestations: [
//!                                        { attester_pubkey, attester_pubkey_hash,
//!                                          price_cents, bond_weight } ] }
//!   getOracleStatus()               -> { active, trust_model,
//!                                        structural_share, sunset_threshold,
//!                                        sunset_triggered, last_update_height,
//!                                        attester_count, activation_height,
//!                                        centralization_disclosure }
//!
//! INPUT PARTITIONS:
//!   getOraclePrice:
//!     pair_id    = { valid_64hex, malformed_hex, missing }
//!     utxo_state = { has_oracle_price(pair_id), absent }
//!     freshness  = { age <= blocks_per_reward_epoch, age > blocks_per_reward_epoch }
//!   getOracleAttestations:
//!     epoch       = { past_with_attestations, past_empty, current, future }
//!     pair_id     = { has_attestations_for_epoch, none_for_epoch, unknown }
//!     bond_source = { closing-epoch snapshot available, not available }
//!   getOracleStatus:
//!     activation   = { pre (u64::MAX), post }
//!     structural_share = { >=5500 bps (active), <5500 bps (sunset_triggered) }
//!     utxo_state   = { has any OraclePrice, none }

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

/// `trust_model` field value, locked verbatim per spec §1.9 + §6.
///
/// Surfaced by both `getOraclePrice` and `getOracleStatus`. Tests assert
/// byte-equality against this constant; any divergence between the spec
/// and the constant is a bug.
pub(super) const ORACLE_TRUST_MODEL: &str = "structural-anchored";

impl RpcContext {
    /// Read the per-pair OraclePrice UTXO (OutputType=15) and surface its
    /// extra_data fields. Spec §1.9 (M9).
    ///
    /// Request: `{ "pair_id": <64-char hex string> }`.
    ///
    /// Response (UTXO exists):
    /// ```json
    /// {
    ///   "pair_id": "<echo, 64-char hex>",
    ///   "price_cents": <u64>,
    ///   "last_update_height": <u64>,
    ///   "contributor_count": <u16>,
    ///   "is_stale": <bool>,
    ///   "trust_model": "structural-anchored"
    /// }
    /// ```
    ///
    /// Response (UTXO absent — pre-aggregation OR pre-activation): `null`.
    ///
    /// Staleness window (Phase 2.1): `blocks_per_reward_epoch` (= 360 on
    /// mainnet/testnet, 60 on devnet). Phase 2.3 tightens via a separate
    /// activation height; the value is sourced from
    /// `RpcContext::blocks_per_reward_epoch`, not a hardcoded global.
    pub(super) async fn get_oracle_price(&self, params: Value) -> Result<Value, RpcError> {
        let pair_id_hex = params
            .get("pair_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'pair_id' parameter"))?;

        let pair_id = crypto::Hash::from_hex(pair_id_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid pair_id hex (expect 64-char hex)"))?;

        let (utxo_tx_hash, utxo_index) = doli_core::oracle::oracle_price_outpoint(&pair_id);
        let outpoint = storage::Outpoint::new(utxo_tx_hash, utxo_index);

        let utxo_set = self.utxo_set.read().await;
        let entry = match utxo_set.get(&outpoint) {
            Some(e) => e,
            None => return Ok(Value::Null),
        };
        drop(utxo_set);

        let (price_cents, last_update_height, contributor_count, parsed_pair_id) = entry
            .output
            .parse_oracle_price()
            .ok_or_else(|| RpcError::internal_error("OraclePrice UTXO has malformed extra_data"))?;

        let current_height = self.chain_state.read().await.best_height;
        let max_staleness = self.blocks_per_reward_epoch;
        let is_stale = current_height.saturating_sub(last_update_height) > max_staleness;

        Ok(serde_json::json!({
            "pair_id": parsed_pair_id.to_hex(),
            "price_cents": price_cents,
            "last_update_height": last_update_height,
            "contributor_count": contributor_count,
            "is_stale": is_stale,
            "trust_model": ORACLE_TRUST_MODEL,
        }))
    }

    /// List all PriceAttestation txs (TxType=16) for a given epoch and
    /// pair_id. Used for transparency / audit. Spec §1.9 (M10).
    ///
    /// Request: `{ "epoch": <u64>, "pair_id": <64-char hex> }`.
    ///
    /// Response:
    /// ```json
    /// {
    ///   "epoch": <echo>,
    ///   "pair_id": <echo, 64-char hex>,
    ///   "attestations": [
    ///     {
    ///       "attester_pubkey":      "<64-char hex of Ed25519 pubkey>",
    ///       "attester_pubkey_hash": "<64-char hex of hash_with_domain(ADDRESS_DOMAIN, pubkey)>",
    ///       "price_cents":          <u64>,
    ///       "bond_weight":          <u64 | null>
    ///     },
    ///     ...
    ///   ]
    /// }
    /// ```
    ///
    /// `bond_weight` policy (locked design decision, 2026-05-25):
    /// `state_db.get_epoch_bond_snapshot()` returns `(snap, epoch_of_snap)`
    /// for at most ONE epoch — the most-recently-closed one. If the
    /// queried `epoch` matches the persisted snapshot's epoch, the
    /// per-attester `bond_weight` comes from that snapshot. Otherwise
    /// `bond_weight` is `null` (DOLI does not preserve historical
    /// bond_snapshots; documented in docs/rpc_reference.md).
    ///
    /// Sort order: attestations are sorted ascending by
    /// `attester_pubkey_hash` bytes, so the response is byte-identical
    /// across repeated calls with the same chain state.
    ///
    /// Empty-list contract: unknown epoch (no blocks in range), future
    /// epoch (> current chain height), and pruned-archive epochs all
    /// return `{ ..., "attestations": [] }` — never an error, never a
    /// panic.
    pub(super) async fn get_oracle_attestations(&self, params: Value) -> Result<Value, RpcError> {
        let epoch = params
            .get("epoch")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| RpcError::invalid_params("missing 'epoch' parameter (u64)"))?;
        let pair_id_hex = params
            .get("pair_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'pair_id' parameter"))?;
        let pair_id = crypto::Hash::from_hex(pair_id_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid pair_id hex (expect 64-char hex)"))?;

        let blocks_per_epoch = self.blocks_per_reward_epoch;
        let start = epoch.saturating_mul(blocks_per_epoch);
        let end_exclusive = start.saturating_add(blocks_per_epoch);

        // Walk blocks for the queried epoch. For each PriceAttestation tx
        // matching this pair_id and epoch, keep the latest occurrence per
        // signer (defense-in-depth — M4 rule 5 already rejects duplicates
        // within an epoch at validation time).
        let mut latest_by_signer: std::collections::HashMap<
            crypto::Hash,
            (crypto::PublicKey, u64),
        > = std::collections::HashMap::new();

        for height in start..end_exclusive {
            let block_opt = self.block_store.get_block_by_height(height).ok().flatten();
            let Some(block) = block_opt else {
                continue;
            };
            for tx in &block.transactions {
                if !tx.is_price_attestation() {
                    continue;
                }
                let Some(data) = tx.price_attestation_data() else {
                    continue;
                };
                if data.pair_id != pair_id || data.epoch_number != epoch {
                    continue;
                }
                let signer_hash = crypto::hash::hash_with_domain(
                    crypto::ADDRESS_DOMAIN,
                    data.signer_pubkey.as_bytes(),
                );
                latest_by_signer.insert(signer_hash, (data.signer_pubkey, data.price_cents));
            }
        }

        // Bond-weight source: only if the persisted bond_snapshot's epoch
        // matches the queried epoch. Otherwise bond_weight is null per
        // the locked design (historical bond_weights not preserved).
        let bond_snapshot_for_epoch: Option<std::collections::HashMap<crypto::Hash, u64>> = self
            .state_db
            .as_ref()
            .and_then(|db| db.get_epoch_bond_snapshot())
            .and_then(|(snap, snap_epoch)| {
                if snap_epoch == epoch {
                    Some(snap)
                } else {
                    None
                }
            });

        let mut entries: Vec<(crypto::Hash, crypto::PublicKey, u64)> = latest_by_signer
            .into_iter()
            .map(|(h, (pk, p))| (h, pk, p))
            .collect();
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        let attestations: Vec<Value> = entries
            .into_iter()
            .map(|(signer_hash, pubkey, price_cents)| {
                let bond_weight: Value = bond_snapshot_for_epoch
                    .as_ref()
                    .and_then(|snap| snap.get(&signer_hash).copied())
                    .map_or(Value::Null, Value::from);
                serde_json::json!({
                    "attester_pubkey":      hex::encode(pubkey.as_bytes()),
                    "attester_pubkey_hash": signer_hash.to_hex(),
                    "price_cents":          price_cents,
                    "bond_weight":          bond_weight,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "epoch":        epoch,
            "pair_id":      pair_id.to_hex(),
            "attestations": attestations,
        }))
    }
}

#[cfg(test)]
#[path = "tests_oracle.rs"]
mod tests;
