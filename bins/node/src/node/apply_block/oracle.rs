//! Phase 2.1 Oracle M6 — epoch-boundary aggregator orchestrator.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.3.
//!
//! The pure-function median lives in `doli_core::oracle`. This file
//! orchestrates the integration:
//!   1. Gate: skip entirely when
//!      `height < oracle_activation_height` (default `u64::MAX` on
//!      all networks until a future binary flips the height).
//!   2. Closing-epoch block scan: walk every block in the just-closed
//!      epoch from `block_store`, collect every PriceAttestation tx
//!      grouped by `pair_id`.
//!   3. Per-pair aggregation: dedup latest contribution per attester
//!      (defense-in-depth — M4 rule 5 already rejects duplicates at
//!      validation), then compute bond-weighted median using the
//!      closing epoch's bond_snapshot.
//!   4. UTXO mutation: spend the previous OraclePrice UTXO (if any)
//!      and insert the new one, both keyed at the deterministic
//!      outpoint `(oracle_price_address(pair_id), 0)`.
//!
//! D.3 sunset gradient (DeFi L1 Foundations M3): replaces the single-
//! cliff sunset boolean with a 3-zone state machine (HEALTHY / WARNING
//! / HALT). The state machine transitions are tracked via
//! `OracleSunsetState` persisted in `state_db` (local bookkeeping,
//! NOT consensus state root). On restart, the state is loaded from
//! the DB; if absent (first start or pre-D.3 binary), defaults to
//! HEALTHY (all fields None).
//!
//! This function MUST run BEFORE the epoch_state rotation in
//! `post_commit_actions` (i.e., before `self.epoch_state = new_state`
//! at the call site) — we depend on `self.epoch_state.bond_snapshot`
//! still holding the CLOSING epoch's bond weights. The call site in
//! `post_commit.rs` invokes this helper at the correct point.

use super::*;
use crypto::ADDRESS_DOMAIN;
use doli_core::consensus::STRUCTURAL_PUBKEY_HASHES_HEX;
use doli_core::oracle::{
    bond_weighted_median, compute_structural_share_bps, dedupe_latest_per_attester,
    oracle_price_outpoint, AttestationContribution,
};
use doli_core::transaction::Output;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use storage::utxo::Outpoint;
use storage::utxo::UtxoEntry;
use tracing::{info, warn};

impl Node {
    /// Run the M6 epoch-boundary aggregator for every pair that
    /// received attestations in the closing epoch. No-op when the
    /// oracle activation height has not been crossed.
    ///
    /// Called from `post_commit_actions` at the epoch boundary,
    /// BEFORE `self.epoch_state` is rotated to the new epoch — the
    /// aggregator reads the closing epoch's `bond_snapshot` from
    /// the still-current `self.epoch_state`.
    pub(super) async fn aggregate_oracle_prices_at_epoch_boundary(&mut self, height: u64) {
        let params = self.config.network.params();
        // Strict-< gate, mirroring M4 rule 1. At
        // `current_height == oracle_activation_height` the aggregator
        // runs.
        if height < params.oracle_activation_height {
            return;
        }

        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        // post_commit fires this at the FIRST block of the new epoch
        // (`is_epoch_boundary_with` returns true when
        // `height % blocks_per_epoch == 0 && height > 0`). The closing
        // epoch is `current_epoch - 1`; its block range is
        // `[(current_epoch - 1) * blocks_per_epoch, current_epoch *
        // blocks_per_epoch)` = `[height - blocks_per_epoch, height)`.
        let closing_epoch_start = height.saturating_sub(blocks_per_epoch);
        let closing_epoch_end = height; // exclusive
        let current_epoch = height / blocks_per_epoch;

        // Phase 2.1 Oracle M8 + D.3 sunset gradient — compute
        // structural share and advance the sunset state machine.
        let structural_hashes: Vec<crypto::Hash> = STRUCTURAL_PUBKEY_HASHES_HEX
            .iter()
            .filter_map(|hex| crypto::Hash::from_hex(hex))
            .collect();
        let registered_at_map = {
            let producers = self.producer_set.read().await;
            let active = producers.active_producers_at_height(height);
            active
                .iter()
                .map(|p| {
                    let pubkey_hash =
                        crypto::hash::hash_with_domain(ADDRESS_DOMAIN, p.public_key.as_bytes());
                    (pubkey_hash, p.registered_at)
                })
                .collect::<HashMap<crypto::Hash, u64>>()
        };
        let share_bps = compute_structural_share_bps(
            &self.epoch_state.bond_snapshot,
            &registered_at_map,
            height,
            blocks_per_epoch,
            &structural_hashes,
        );

        // D.3: Load persisted sunset state from DB (or default if
        // absent — first start or pre-D.3 binary). Advance the state
        // machine, persist the result.
        let mut sunset_state = self.state_db.get_oracle_sunset_state().unwrap_or_default();
        let health = sunset_state.transition(share_bps, current_epoch);
        self.state_db.put_oracle_sunset_state(&sunset_state);

        let sunset_triggered = health.is_sunset_triggered();
        // Persist the flag for the validation path. Use Release
        // ordering so any subsequent Acquire-load in validation
        // observes the latest value (validate_transaction reads it
        // via ctx.oracle_sunset_triggered, which is constructed from
        // this atomic at every ValidationContext build site).
        self.oracle_sunset_triggered
            .store(sunset_triggered, Ordering::Release);

        match health {
            doli_core::oracle::OracleHealthState::Healthy => {
                info!(
                    "[ORACLE] epoch boundary height={}: health=HEALTHY share_bps={:?}",
                    height, share_bps
                );
            }
            doli_core::oracle::OracleHealthState::Warning => {
                warn!(
                    "[ORACLE] epoch boundary height={}: health=WARNING share_bps={:?} \
                     warning_since_epoch={:?} — aggregation continues",
                    height, share_bps, sunset_state.warning_since_epoch
                );
            }
            doli_core::oracle::OracleHealthState::HaltRecoverable => {
                warn!(
                    "[ORACLE] sunset HALT (recoverable) at height={}: share_bps={:?} \
                     halt_since_epoch={:?} — skipping aggregation, may auto-recover \
                     within {} epochs",
                    height,
                    share_bps,
                    sunset_state.halt_since_epoch,
                    doli_core::oracle::ORACLE_RECOVERY_EPOCHS
                );
                return;
            }
            doli_core::oracle::OracleHealthState::HaltPermanent => {
                warn!(
                    "[ORACLE] sunset HALT (PERMANENT) at height={}: share_bps={:?} \
                     — binary upgrade required to resume oracle",
                    height, share_bps
                );
                return;
            }
        }

        // Step 2 — scan closing-epoch blocks for PriceAttestations,
        // group by pair_id.
        let mut by_pair: HashMap<crypto::Hash, Vec<AttestationContribution>> = HashMap::new();
        for h in closing_epoch_start..closing_epoch_end {
            let block = match self.block_store.get_block_by_height(h) {
                Ok(Some(b)) => b,
                Ok(None) => continue,
                Err(e) => {
                    info!(
                        "[ORACLE] aggregator: block_store miss at height {}: {} — skipping",
                        h, e
                    );
                    continue;
                }
            };
            for tx in &block.transactions {
                if !tx.is_price_attestation() {
                    continue;
                }
                let Some(data) = tx.price_attestation_data() else {
                    continue;
                };
                let signer_hash =
                    crypto::hash::hash_with_domain(ADDRESS_DOMAIN, data.signer_pubkey.as_bytes());
                by_pair
                    .entry(data.pair_id)
                    .or_default()
                    .push(AttestationContribution {
                        signer_hash,
                        price_cents: data.price_cents,
                    });
            }
        }

        if by_pair.is_empty() {
            return;
        }

        // Step 3-4 — per pair: dedup, compute median, mutate UtxoSet.
        let mut utxo = self.utxo_set.write().await;
        for (pair_id, contributions) in by_pair {
            let deduped = dedupe_latest_per_attester(&contributions);
            let Some((median_price, contributor_count)) =
                bond_weighted_median(&deduped, &self.epoch_state.bond_snapshot)
            else {
                continue;
            };

            let (synth_tx, synth_idx) = oracle_price_outpoint(&pair_id);
            let outpoint = Outpoint::new(synth_tx, synth_idx);

            // Step 7 — consume the previous OraclePrice UTXO if it
            // exists (first epoch has nothing to consume; remove is
            // idempotent-on-absent).
            let _ = utxo.remove(&outpoint);

            // Step 6 — create new OraclePrice UTXO at the same
            // deterministic outpoint with the just-computed median.
            let entry = UtxoEntry {
                output: Output::oracle_price(pair_id, median_price, height, contributor_count),
                height,
                is_coinbase: false,
                is_epoch_reward: false,
            };
            if let Err(e) = utxo.insert(outpoint, entry) {
                info!(
                    "[ORACLE] aggregator: insert OraclePrice UTXO failed pair={} err={}",
                    pair_id, e
                );
            } else {
                info!(
                    "[ORACLE] aggregator: pair={} median_cents={} contributors={} height={}",
                    pair_id, median_price, contributor_count, height
                );
            }
        }
    }
}
