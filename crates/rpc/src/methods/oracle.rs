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
}

#[cfg(test)]
mod tests {
    //! OUTPUT CONTRACT and INPUT PARTITIONS are documented at the top of
    //! the parent module. Each test below pins one partition.

    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use crypto::Hash;
    use doli_core::network::Network;
    use doli_core::transaction::Output;
    use mempool::{Mempool, MempoolPolicy};
    use storage::{BlockStore, ChainState, Outpoint, UtxoEntry, UtxoSet};
    use tempfile::TempDir;

    /// Tempdir held by the test for the BlockStore path; dropped at test
    /// end. The `_tempdir` field keeps it alive — RpcContext does not own
    /// it because BlockStore is opened under it.
    struct TestCtx {
        ctx: RpcContext,
        utxo_set: Arc<RwLock<UtxoSet>>,
        chain_state: Arc<RwLock<ChainState>>,
        _tempdir: TempDir,
    }

    /// Build a minimal `RpcContext` for M9 testing. Mainnet defaults
    /// (blocks_per_reward_epoch = 360); UTXO set is empty; chain_state at
    /// height 0 unless mutated by the test.
    fn build_ctx() -> TestCtx {
        let tempdir = TempDir::new().expect("tempdir");
        let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
        let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
        let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
        let params = doli_core::consensus::ConsensusParams::default();
        let mempool = Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::default(),
            params.clone(),
            Network::Mainnet,
        )));

        let ctx = RpcContext::new_for_network(
            chain_state.clone(),
            block_store,
            utxo_set.clone(),
            mempool,
            params,
            Network::Mainnet,
        );
        TestCtx {
            ctx,
            utxo_set,
            chain_state,
            _tempdir: tempdir,
        }
    }

    /// Insert an OraclePrice UTXO at the deterministic per-pair address
    /// (mirrors M6's aggregator) so M9 can find it via
    /// `oracle_price_outpoint(pair_id)`.
    async fn insert_oracle_price(
        utxo_set: &Arc<RwLock<UtxoSet>>,
        pair_id: Hash,
        price_cents: u64,
        last_update_height: u64,
        contributor_count: u16,
        creation_height: u64,
    ) {
        let output =
            Output::oracle_price(pair_id, price_cents, last_update_height, contributor_count);
        let (tx_hash, index) = doli_core::oracle::oracle_price_outpoint(&pair_id);
        let outpoint = Outpoint::new(tx_hash, index);
        let entry = UtxoEntry {
            output,
            height: creation_height,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo_set
            .write()
            .await
            .insert(outpoint, entry)
            .expect("insert oracle price utxo");
    }

    /// Set the chain state's best height to a known value so staleness
    /// computation is deterministic.
    async fn set_best_height(chain_state: &Arc<RwLock<ChainState>>, height: u64) {
        chain_state.write().await.best_height = height;
    }

    fn pair_id_fixture() -> Hash {
        // BLAKE3("ORACLE_PAIR" || "DOLI/USD") — same shape as production
        // but value is irrelevant; tests use bit-identical pair_id throughout.
        crypto::hash::hash_with_domain(b"ORACLE_PAIR", b"DOLI/USD")
    }

    // ---------- partition: utxo_state = has_oracle_price + freshness fresh ----------
    #[tokio::test]
    async fn m9_happy_path_returns_parsed_extra_data() {
        let t = build_ctx();
        let pair_id = pair_id_fixture();

        insert_oracle_price(&t.utxo_set, pair_id, 12_345, 1_000, 8, 1_000).await;
        set_best_height(&t.chain_state, 1_100).await;

        let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
        let resp = t
            .ctx
            .get_oracle_price(params)
            .await
            .expect("M9 happy-path Ok");

        assert_eq!(resp["pair_id"].as_str().unwrap(), pair_id.to_hex());
        assert_eq!(resp["price_cents"].as_u64().unwrap(), 12_345);
        assert_eq!(resp["last_update_height"].as_u64().unwrap(), 1_000);
        assert_eq!(resp["contributor_count"].as_u64().unwrap(), 8);
        assert!(!resp["is_stale"].as_bool().unwrap());
        assert_eq!(resp["trust_model"].as_str().unwrap(), "structural-anchored");
    }

    // ---------- partition: freshness = age > blocks_per_reward_epoch ----------
    #[tokio::test]
    async fn m9_is_stale_true_when_age_exceeds_epoch_width() {
        let t = build_ctx();
        let pair_id = pair_id_fixture();

        // Mainnet blocks_per_reward_epoch = 360. age = 1000 - 100 = 900 > 360.
        insert_oracle_price(&t.utxo_set, pair_id, 100, 100, 1, 100).await;
        set_best_height(&t.chain_state, 1_000).await;

        let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
        let resp = t.ctx.get_oracle_price(params).await.unwrap();
        assert!(
            resp["is_stale"].as_bool().unwrap(),
            "age={} should be > epoch_width={}",
            900,
            360
        );
    }

    // ---------- partition: freshness = age <= blocks_per_reward_epoch ----------
    #[tokio::test]
    async fn m9_is_stale_false_when_age_within_window() {
        let t = build_ctx();
        let pair_id = pair_id_fixture();

        // age = 1000 - 900 = 100, well within 360-block window
        insert_oracle_price(&t.utxo_set, pair_id, 100, 900, 1, 900).await;
        set_best_height(&t.chain_state, 1_000).await;

        let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
        let resp = t.ctx.get_oracle_price(params).await.unwrap();
        assert!(!resp["is_stale"].as_bool().unwrap());
    }

    // ---------- partition: utxo_state = absent (pre-aggregation OR pre-activation) ----------
    #[tokio::test]
    async fn m9_returns_null_when_utxo_absent() {
        let t = build_ctx();
        let pair_id = pair_id_fixture();

        let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
        let resp = t.ctx.get_oracle_price(params).await.unwrap();
        assert!(
            resp.is_null(),
            "Expected null when OraclePrice UTXO is absent, got {:?}",
            resp
        );
    }

    // ---------- partition: trust_model byte-equality ----------
    #[tokio::test]
    async fn m9_trust_model_byte_equal_to_constant() {
        let t = build_ctx();
        let pair_id = pair_id_fixture();
        insert_oracle_price(&t.utxo_set, pair_id, 1, 1, 1, 1).await;
        set_best_height(&t.chain_state, 1).await;

        let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
        let resp = t.ctx.get_oracle_price(params).await.unwrap();
        assert_eq!(
            resp["trust_model"].as_str().unwrap().as_bytes(),
            b"structural-anchored",
            "trust_model must be the literal string 'structural-anchored'"
        );
        // Locks the production const against accidental edit.
        assert_eq!(ORACLE_TRUST_MODEL, "structural-anchored");
    }

    // ---------- partition: pair_id = malformed hex ----------
    #[tokio::test]
    async fn m9_malformed_pair_id_returns_invalid_params() {
        let t = build_ctx();

        let params = serde_json::json!({ "pair_id": "not-hex" });
        let err = t
            .ctx
            .get_oracle_price(params)
            .await
            .expect_err("expected invalid_params");
        assert_eq!(err.code, -32602, "invalid_params code expected");
    }

    // ---------- partition: pair_id = missing ----------
    #[tokio::test]
    async fn m9_missing_pair_id_returns_invalid_params() {
        let t = build_ctx();

        let params = serde_json::json!({});
        let err = t
            .ctx
            .get_oracle_price(params)
            .await
            .expect_err("expected invalid_params");
        assert_eq!(err.code, -32602);
    }
}
