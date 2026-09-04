use super::*;
use crate::node::attestation::{commit, width};

impl Node {
    /// Check producer eligibility for a received gossip block.
    ///
    /// LIGHTWEIGHT CHECK: only verifies the producer is in the known set and
    /// the time window is valid for the block's slot. Does NOT validate against
    /// local chain state (which may be on a micro-fork).
    ///
    /// Full validation happens in apply_block() where the block is checked
    /// against the actual chain state it builds on.
    pub async fn check_producer_eligibility(&self, block: &Block) -> Result<()> {
        // Use the BLOCK's slot for eligibility, not our local chain state.
        // Our local tip may be on a different micro-fork, causing us to
        // reject valid blocks from the canonical chain.
        let height = block.header.slot as u64; // Approximate — exact height unknown for gossip blocks

        // Check: is the producer in the known set?
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        drop(producers);

        // If no active producers (pre-genesis), check GSet
        if !active.is_empty() && !active.contains(&block.header.producer) {
            // Producer not in active set — check if they're in GSet (bootstrap)
            let gset = self.producer_gset.read().await;
            let gset_producers = gset.active_producers(7200);
            drop(gset);
            if !gset_producers.contains(&block.header.producer) {
                anyhow::bail!(
                    "[ECON_PRODUCER] unknown producer {} — not in active set ({} active) or GSet",
                    &block.header.producer.to_hex()[..16],
                    active.len()
                );
            }
        }

        // Bond weights from epoch-locked snapshot (single source of truth).
        let weighted = self.bond_weights_for_scheduling(active).await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build bootstrap producer list from GSet (same source as production side).
        // Must be sorted by pubkey for deterministic fallback rank order.
        let mut bootstrap_producers = {
            let gset = self.producer_gset.read().await;
            gset.active_producers(7200) // 2h liveness window, same as production
        };
        if bootstrap_producers.is_empty() {
            let known = self.known_producers.read().await;
            bootstrap_producers = known.clone();
        }
        bootstrap_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Build liveness split for bootstrap validation (must match production side).
        let num_bp = bootstrap_producers.len();
        let liveness_window = std::cmp::max(
            consensus::LIVENESS_WINDOW_MIN,
            (num_bp as u64).saturating_mul(3),
        );
        let chain_height = height.saturating_sub(1);
        let cutoff = chain_height.saturating_sub(liveness_window);
        let (live_bp, stale_bp): (Vec<PublicKey>, Vec<PublicKey>) = {
            let (live, stale): (Vec<_>, Vec<_>) = bootstrap_producers.iter().partition(|pk| {
                match self.producer_liveness.get(pk) {
                    Some(&last_h) => last_h >= cutoff,
                    // No chain record: live if chain is young, stale otherwise
                    None => chain_height < liveness_window,
                }
            });
            (
                live.into_iter().copied().collect(),
                stale.into_iter().copied().collect(),
            )
        };
        // Deadlock safety: if all stale, treat all as live (filter disabled)
        let (live_bp, stale_bp) = if live_bp.is_empty() {
            (bootstrap_producers.clone(), Vec::new())
        } else {
            (live_bp, stale_bp)
        };

        // Scheduler fingerprint: compare across nodes to detect divergence.
        {
            let total_bonds: u64 = weighted.iter().map(|(_, b)| *b).sum();
            debug!(
                "[SCHED] VALIDATE slot={} producer={} producers={} total_bonds={} snap_epoch={}",
                block.header.slot,
                hex::encode(&block.header.producer.as_bytes()[..4]),
                weighted.len(),
                total_bonds,
                self.epoch_state.epoch,
            );
        }

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_producers_weighted(weighted)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp)
        // INC-I-173 M3 / AUDIT-P3-003. This path is HEADER-ONLY, so the fee gate
        // is unreachable from here and wiring it changes nothing today. It is
        // wired anyway so that all four ValidationContext builders carry the same
        // field set: a context that is silently weaker than its siblings is how
        // INV-VALIDATION-001 gets violated the next time this path grows.
        .with_inc_i_173_activation_height(self.config.network.params().inc_i_173_activation_height)
        .with_epoch_producer_list(if self.epoch_state.active_list.is_empty() {
            self.epoch_state.producer_list.clone()
        } else {
            self.epoch_state.active_list.clone()
        })
        .with_inc_i_026_scheduler_activation_height(
            self.config
                .network
                .params()
                .inc_i_026_scheduler_activation_height,
        )
        .with_encrypted_content_activation_height(
            self.config
                .network
                .params()
                .encrypted_content_activation_height,
        )
        .with_encrypted_content_v2_activation_height(
            self.config
                .network
                .params()
                .encrypted_content_v2_activation_height,
        )
        .with_security_audit_activation_height(
            self.config
                .network
                .params()
                .security_audit_activation_height,
        )
        .with_defi_activation_height(self.config.network.params().defi_activation_height)
        .with_amm_activation_height(self.config.network.params().amm_activation_height)
        .with_inc_i_092_activation_height(self.config.network.params().inc_i_092_activation_height)
        .with_inc_i_096_activation_height(self.config.network.params().inc_i_096_activation_height)
        .with_oracle_activation_height(self.config.network.params().oracle_activation_height)
        .with_oracle_sunset_triggered(
            self.oracle_sunset_triggered
                .load(std::sync::atomic::Ordering::Acquire),
        );

        // Apply chainspec if present
        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        validation::validate_producer_eligibility(&block.header, &ctx)?;
        Ok(())
    }

    /// Validate a block before applying it to the chain.
    ///
    /// Builds a full ValidationContext and calls `validate_block_with_mode`.
    /// In `Light` mode (gap blocks after snap sync), VDF is skipped.
    /// In `Full` mode (recent blocks near tip), VDF is verified.
    pub async fn validate_block_for_apply(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<(), validation::ValidationError> {
        let state = self.chain_state.read().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build weighted producer list using epoch-locked bond snapshot.
        // The snapshot is computed once at each epoch boundary and stays
        // constant for the entire epoch. This prevents mid-epoch add-bond
        // TXs from changing total_bonds and causing scheduler divergence.
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let pending_keys = producers.pending_registration_keys();
        drop(producers);

        // Bond weights from epoch-locked snapshot (single source of truth).
        let weighted = self.bond_weights_for_scheduling(active).await;

        // Build bootstrap producer list for validation.
        //
        // For Light mode (sync): the GSet reflects CURRENT network state
        // (includes producers that joined after genesis, e.g. N6/N8), but
        // historical blocks were produced with a DIFFERENT GSet composition.
        // bootstrap_fallback_order uses (slot + rank) % n — a different n
        // means completely different rank assignments → "invalid producer
        // for slot". Pass empty bootstrap_producers for ALL synced blocks:
        // - Genesis-phase blocks: accepted via empty-bootstrap-list fallback
        // - Transition block (361): same — producer_set not yet populated
        // - Post-genesis blocks: validated by deterministic bond-weighted
        //   scheduler (on-chain data), bypassing bootstrap path entirely
        // This is safe: header chain continuity is verified during header
        // download, and blocks were already validated by the network.
        let (bootstrap_producers, live_bp, stale_bp) =
            if matches!(mode, ValidationMode::Light | ValidationMode::Replay) {
                (Vec::new(), Vec::new(), Vec::new())
            } else {
                let mut bp = {
                    let gset = self.producer_gset.read().await;
                    gset.active_producers(7200)
                };
                if bp.is_empty() {
                    let known = self.known_producers.read().await;
                    bp = known.clone();
                }

                // ACTIVATION_DELAY filter: mirror the production code's filtering
                // (node.rs try_produce_block lines 4993-5014). Without this, the
                // validation path may compute a different producer count N than
                // production, causing slot % N mismatches → "invalid producer for slot".
                {
                    let producers = self.producer_set.read().await;
                    bp.retain(|pk| match producers.get_by_pubkey(pk) {
                        Some(info) => {
                            if !info.is_active() {
                                return false;
                            }
                            // Genesis producers: always eligible
                            if info.registered_at == 0 {
                                return true;
                            }
                            // Late joiners: must wait activation delay
                            height >= info.registered_at + storage::ACTIVATION_DELAY
                        }
                        None => {
                            // Not registered (gossip-discovered): include in bootstrap
                            true
                        }
                    });
                }

                bp.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                // Build liveness split
                let num_bp = bp.len();
                let liveness_window = std::cmp::max(
                    consensus::LIVENESS_WINDOW_MIN,
                    (num_bp as u64).saturating_mul(3),
                );
                let chain_height = height.saturating_sub(1);
                let cutoff = chain_height.saturating_sub(liveness_window);
                let (live, stale): (Vec<PublicKey>, Vec<PublicKey>) = {
                    let (l, s): (Vec<_>, Vec<_>) =
                        bp.iter()
                            .partition(|pk| match self.producer_liveness.get(pk) {
                                Some(&last_h) => last_h >= cutoff,
                                None => chain_height < liveness_window,
                            });
                    (
                        l.into_iter().copied().collect(),
                        s.into_iter().copied().collect(),
                    )
                };
                if live.is_empty() {
                    (bp.clone(), bp, Vec::new())
                } else {
                    (bp, live, stale)
                }
            };

        // Get previous block timestamp from block store for header validation
        let prev_timestamp = self
            .block_store
            .get_header(&state.best_hash)
            .ok()
            .flatten()
            .map(|h| h.timestamp)
            .unwrap_or(0);

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_prev_block(state.best_slot, prev_timestamp, state.best_hash)
        .with_producers_weighted(weighted)
        .with_pending_producer_keys(pending_keys)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp)
        .with_epoch_producer_list(
            // For scheduling validation: use active_production_list (tier-filtered)
            // so validation computes the same slot % N as production.
            // For missed_producers check below: use full epoch_producer_list.
            if self.epoch_state.active_list.is_empty() {
                self.epoch_state.producer_list.clone()
            } else {
                self.epoch_state.active_list.clone()
            },
        )
        .with_sig_verification_height(self.config.network.params().sig_verification_height)
        // INC-I-173 M3 / AUDIT-P3-003. CONSENSUS PATH, and this wiring is a
        // DIVERGENCE FIX, not a cosmetic one. Left unwired this context holds
        // u64::MAX while `apply_block/tx_processing.rs` is wired, so ABOVE the
        // gate the two paths DISAGREE: one rejects a block carrying a maintainer
        // transaction that the other accepts. It is consensus-visible above the
        // gate and needs no NEW height because it rides the same already-committed
        // `inc_i_173_activation_height`.
        //
        // CORRECTED 2026-08-11 (M3 review iteration 1, REV-173-M3-001). This
        // comment used to add "which no network has crossed". That is FALSE for
        // testnet: the gate is 133_000 and the live testnet tip measured
        // 134_159 (v6.24.1, agreed across RPC 8500/8501/8502). **On testnet this
        // wiring becomes active the moment the binary lands, not at a future
        // scheduled height**, so the testnet deploy is a SYNCHRONIZED
        // stop-all-then-start-all, never a rolling restart (INV-8 / INC-I-062):
        // a new-binary producer could immediately mine an `AddMaintainer` that
        // old-binary nodes reject. Mainnet (u64::MAX) and devnet (0) are
        // unaffected. History is NOT invalidated: above the gate the predicate is
        // strictly MORE permissive (the frozen three plus AddMaintainer /
        // RemoveMaintainer), so no block valid under the old rules becomes
        // invalid, and the running binary has no knowledge of this height at all.
        // M2 must re-pin the testnet height above the then-current tip and
        // re-verify the tip immediately before pinning.
        .with_inc_i_173_activation_height(self.config.network.params().inc_i_173_activation_height)
        .with_inc_i_026_scheduler_activation_height(
            self.config
                .network
                .params()
                .inc_i_026_scheduler_activation_height,
        )
        .with_fork_id(
            self.current_fork_id(),
            self.config.network.params().fork_id_activation_height,
        )
        .with_encrypted_content_activation_height(
            self.config
                .network
                .params()
                .encrypted_content_activation_height,
        )
        .with_encrypted_content_v2_activation_height(
            self.config
                .network
                .params()
                .encrypted_content_v2_activation_height,
        )
        .with_security_audit_activation_height(
            self.config
                .network
                .params()
                .security_audit_activation_height,
        )
        .with_defi_activation_height(self.config.network.params().defi_activation_height)
        .with_amm_activation_height(self.config.network.params().amm_activation_height)
        .with_inc_i_092_activation_height(self.config.network.params().inc_i_092_activation_height)
        .with_inc_i_096_activation_height(self.config.network.params().inc_i_096_activation_height)
        .with_oracle_activation_height(self.config.network.params().oracle_activation_height)
        .with_oracle_sunset_triggered(
            self.oracle_sunset_triggered
                .load(std::sync::atomic::Ordering::Acquire),
        );

        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        drop(state);

        // Validate missed_producers header field (P1-001: was unvalidated on receiving nodes)
        // NOTE: This check uses epoch_producer_list (full list), not active_production_list,
        // because missed producers can be attestors too.
        {
            const MAX_MISSED_PER_BLOCK: usize = 3;
            let missed = &block.header.missed_producers;

            // Length cap: production enforces MAX_MISSED_PER_BLOCK=3
            if missed.len() > MAX_MISSED_PER_BLOCK {
                return Err(validation::ValidationError::InvalidTransaction(format!(
                    "[ERRTX068] missed_producers has {} entries (max {})",
                    missed.len(),
                    MAX_MISSED_PER_BLOCK,
                )));
            }

            // Membership: all missed keys must be in the epoch producer list
            if !self.epoch_state.producer_list.is_empty() {
                for pk in missed {
                    if !self.epoch_state.producer_list.contains(pk) {
                        return Err(validation::ValidationError::InvalidTransaction(format!(
                            "[ERRTX069] missed_producers contains key {} not in epoch producer list (list_size={})",
                            hex::encode(&pk.as_bytes()[..4]),
                            self.epoch_state.producer_list.len(),
                        )));
                    }
                }
            }

            // NOTE: the previous total-cap check (ERRTX070) was removed in
            // v6.13.21. It compared local state with a canonical block header
            // field, making validation non-deterministic across nodes.
        }

        // P0-001: public_key enforcement is ACTIVE (v5.2.0+).
        // Input.public_key is part of the bincode wire format (#[serde(skip)] removed).
        // sig_verification_height=0 on all networks: enforce from genesis.

        // Attestation body (INC-I-178 D5/D6). Pre-AH the commitment is
        // BLAKE3(bitfield) and the stray-bit denominator is active_at(h); post-AH a
        // carried aggregate is bound too and the denominator is the universe width.
        if !block.attestation_bitfield.is_empty() {
            let ah = self.inc_i_178_attestation_bls_activation_height;
            let bf = &block.attestation_bitfield;
            if height < ah {
                let expected =
                    commit::block_presence_root_at(ah, height, bf, &block.aggregate_bls_signature);
                if block.header.presence_root != expected {
                    return Err(validation::ValidationError::InvalidTransaction(format!(
                        "presence_root mismatch: expected {}, got {}",
                        expected, block.header.presence_root,
                    )));
                }
            }
            let active: Vec<crypto::PublicKey> = {
                let producers = self.producer_set.read().await;
                let a = producers.active_producers_at_height(height);
                a.iter().map(|p| p.public_key).collect()
            };
            let base = &self.epoch_state.producer_list;
            let producer_count = commit::stray_bit_universe_width_at(ah, height, base, &active);
            if !width::bitfield_width_accepted_at(bf.len(), producer_count, height, &ah)
                || !doli_core::validate_attestation_bitfield_vec(bf, producer_count)
            {
                return Err(validation::ValidationError::InvalidTransaction(
                    "attestation_bitfield has bits set beyond producer_count".to_string(),
                ));
            }
        }

        validation::validate_block_with_mode(block, &ctx, mode).map_err(|e| {
            warn!(
                "[VALIDATION] Failed: h={} hash={:.16} producer={:.8} slot={} err={}",
                height,
                block.hash(),
                hex::encode(block.header.producer.as_bytes()),
                block.header.slot,
                e
            );
            e
        })?;

        // INC-I-178 D7 (M5): runs AFTER the VDF/eligibility/size checks (C8/F11).
        self.verify_block_attestation(block, height, mode).await
    }

    /// Validate block economics — prevents inflation and reward theft.
    ///
    /// Checks that cannot be done in the core validation crate because they
    /// require access to the UTXO set, producer registry, and block store.
    ///
    /// ## Coinbase validation (every block)
    /// - First TX must be coinbase (Transfer, no inputs, 1 output)
    /// - Amount must equal `block_reward(height)`
    /// - Recipient must be `reward_pool_pubkey_hash()`
    ///
    /// ## EpochReward validation (epoch boundary blocks)
    /// - EpochReward TX only allowed at epoch boundaries, post-genesis, epoch > 0
    /// - At most one EpochReward TX per block
    /// - Total distributed must not exceed pool balance (conservation)
    /// - Exact match of amounts and recipients (both Full and Light modes)
    pub async fn validate_block_economics(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<()> {
        // === Coinbase validation ===
        if block.transactions.is_empty() {
            anyhow::bail!(
                "[ECON_COINBASE_MISSING] block has no transactions (missing coinbase) at height={}",
                height
            );
        }

        let coinbase = &block.transactions[0];
        if !coinbase.is_coinbase() {
            anyhow::bail!(
                "[ECON_COINBASE_INVALID] first transaction is not a valid coinbase at height={}",
                height
            );
        }

        // AUDIT-VALID-001: Reject additional coinbase transactions at index > 0.
        // Without this check, a malicious producer could mint unlimited coins.
        for (i, tx) in block.transactions.iter().enumerate().skip(1) {
            if tx.is_coinbase() {
                anyhow::bail!(
                    "[ECON_EXTRA_COINBASE] coinbase transaction at index {} (only index 0 allowed) at height={}",
                    i, height
                );
            }
        }

        // AUDIT-P2-004 — Rule 5: at most ONE PriceAttestation per (attester,
        // epoch, pair_id) per block. The aggregator's defense-in-depth
        // `dedupe_latest_per_attester` picks the last entry per attester,
        // which would allow a producer to flood duplicate-or-revised
        // attestations within a single block, wasting block space and
        // creating a last-mover advantage on revised prices. Reject the
        // SECOND occurrence with [ERRTX-ORACLE002]. Gated by
        // oracle_activation_height — pre-activation no PriceAttestation can
        // reach the mempool so the check is moot today.
        if height >= self.config.network.params().oracle_activation_height {
            let mut seen: std::collections::HashSet<(crypto::Hash, u64, crypto::Hash)> =
                std::collections::HashSet::new();
            for (i, tx) in block.transactions.iter().enumerate() {
                if !tx.is_price_attestation() {
                    continue;
                }
                let Some(data) = tx.price_attestation_data() else {
                    continue;
                };
                let signer_hash = crypto::hash::hash_with_domain(
                    crypto::ADDRESS_DOMAIN,
                    data.signer_pubkey.as_bytes(),
                );
                let key = (signer_hash, data.epoch_number, data.pair_id);
                if !seen.insert(key) {
                    anyhow::bail!(
                        "[ERRTX-ORACLE002] duplicate PriceAttestation at block tx index {} \
                         (attester={} epoch={} pair_id={}) — Rule 5: at most one per \
                         (attester, epoch, pair_id) per block",
                        i,
                        signer_hash,
                        data.epoch_number,
                        data.pair_id
                    );
                }
            }
        }

        // Calculate extra fees from user transactions in this block.
        // Excluded from extra_fees calculation:
        // - Coinbase/EpochReward: protocol-generated, no user fees
        // - Genesis Registration (0 inputs, 0 outputs): protocol-generated VDF proof
        // User Registration (from mempool, has inputs/outputs) DOES pay per-byte fees.
        let extra_fees: u64 = block
            .transactions
            .iter()
            .filter(|tx| {
                !(tx.is_coinbase()
                    || tx.is_epoch_reward()
                    || tx.tx_type == TxType::Registration
                        && tx.inputs.is_empty()
                        && tx.outputs.is_empty())
            })
            .flat_map(|tx| tx.outputs.iter())
            .map(|o| {
                o.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE
                    / doli_core::consensus::FEE_DIVISOR
            })
            .sum();

        let base_reward = self.params.block_reward(height);
        let expected_with_fees = base_reward + extra_fees;
        let coinbase_amount = coinbase.outputs[0].amount;
        // Accept both formats during version transition:
        // - v4.9.0+: coinbase = block_reward + per-byte extra_fees
        // - v4.5.x:  coinbase = block_reward only (no per-byte fees)
        // External producers on older versions don't include extra_fees.
        // Their blocks are valid — they just generate less reward pool revenue.
        // See: N5 fork incident 2026-03-26 (coinbase mismatch on delta=0 reorg).
        if coinbase_amount != expected_with_fees && coinbase_amount != base_reward {
            anyhow::bail!(
                "[ECON_COINBASE_AMOUNT] coinbase amount {} != expected {} (base {} + extra_fees {}) at height={}",
                coinbase_amount,
                expected_with_fees,
                base_reward,
                extra_fees,
                height
            );
        }

        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        if coinbase.outputs[0].pubkey_hash != pool_hash {
            anyhow::bail!(
                "[ECON_COINBASE_RECIPIENT] coinbase recipient {} is not reward pool {} at height={}",
                coinbase.outputs[0].pubkey_hash,
                pool_hash,
                height
            );
        }

        // === INC-I-180: withdrawal-holdings gate (height-gated) ===
        //
        // Position is load-bearing: the EpochReward section below returns
        // `Ok(())` early in Full mode ([INC_I_081_MISSING_CHECK_SKIP]) whenever
        // the local store cannot prove a reward was owed, so a gate placed
        // after it is enforced in Light/Replay only (INC-I-034 class).
        // INC-I-080's cap stays BELOW that return: it is live from height 0 on
        // mainnet, so running it there would change canonical verdicts.
        //
        // Pre-mutation, all modes. Allowance mirrors apply at enqueue time:
        //   bond_count + pending AddBonds + AddBonds earlier in THIS block
        //   - withdrawal_pending_count - bonds charged by Exits and
        //     RequestWithdrawals earlier in THIS block
        // and the declared count is bound to the NAMED producer's Bond UTXOs,
        // so both ledger effects move the same producer by the same magnitude.
        // Pre-activation the block is skipped whole, so the historical
        // silent-skip path stays bit-identical (replay safety).
        let withdrawal_gate_ah = self
            .config
            .network
            .params()
            .withdrawal_holdings_gate_activation_height;
        if height >= withdrawal_gate_ah {
            // Resolve input types FIRST, under the utxo guard alone, and drop it
            // before taking the producer guard. `apply_block` takes utxo then
            // producers (mod.rs:197) while `rollback` takes producers then utxo
            // (rollback.rs:325); holding both here would join those two orders
            // into a lock cycle. Only one guard is ever held at a time.
            // Per withdrawal tx: (Bond inputs owned by the named producer, ALL
            // Bond-typed inputs). Both come from ONE pass over the same lookups.
            // `owned_live_bonds` adds, per DISTINCT named producer, how many Bond
            // UTXOs it owns in that same pre-block view — one owner-index scan
            // per producer, memoized, never one per transaction.
            let mut owned_live_bonds: std::collections::HashMap<crypto::Hash, u32> =
                std::collections::HashMap::new();
            let bond_inputs_by_tx: std::collections::HashMap<usize, (u32, u32)> = {
                let utxo = self.utxo_set.read().await;
                block
                    .transactions
                    .iter()
                    .enumerate()
                    .filter(|(_, tx)| tx.tx_type == TxType::RequestWithdrawal)
                    .map(|(i, tx)| {
                        // Count only the NAMED producer's own bonds. A tx signed
                        // by A, spending A's Bond UTXOs, may name B in
                        // `extra_data` (the Bond lock is bypassed for this tx
                        // type, validation/utxo.rs). Counting any Bond UTXO would
                        // debit B's weight against A's destroyed UTXOs and leave
                        // A's weight unbacked — the n11 shape (QA R2-B8).
                        // `hash_with_domain(ADDRESS_DOMAIN, ..)` is where
                        // Registration, AddBond and genesis all place Bond
                        // outputs; a malformed tx counts zero and is rejected by
                        // the declared-count comparison below.
                        let owner = tx.withdrawal_request_data().map(|wd| {
                            crypto::hash::hash_with_domain(
                                crypto::ADDRESS_DOMAIN,
                                wd.producer_pubkey.as_bytes(),
                            )
                        });
                        if let Some(addr) = owner {
                            owned_live_bonds.entry(addr).or_insert_with(|| {
                                u32::try_from(utxo.get_bond_entries(&addr).len())
                                    .unwrap_or(u32::MAX)
                            });
                        }
                        let (mut owned, mut all_bonds) = (0u32, 0u32);
                        for inp in &tx.inputs {
                            let Some(entry) = utxo
                                .get(&storage::Outpoint::new(inp.prev_tx_hash, inp.output_index))
                            else {
                                continue;
                            };
                            if entry.output.output_type != doli_core::transaction::OutputType::Bond
                            {
                                continue;
                            }
                            all_bonds = all_bonds.saturating_add(1);
                            if owner == Some(entry.output.pubkey_hash) {
                                owned = owned.saturating_add(1);
                            }
                        }
                        (i, (owned, all_bonds))
                    })
                    .collect()
            };
            let producers = self.producer_set.read().await;
            let mut in_block_addbond: std::collections::HashMap<crypto::Hash, u32> =
                std::collections::HashMap::new();
            let mut in_block_withdrawn: std::collections::HashMap<crypto::Hash, u32> =
                std::collections::HashMap::new();
            // R4 needs the hashes of the transactions at LOWER indices. Filled at
            // the HEAD of the loop with the PREVIOUS transaction: the arms below
            // use `continue`, so a tail insert would be skipped for a malformed
            // one. Paid for only when the block actually carries a withdrawal.
            let mut earlier_tx_hashes: std::collections::HashSet<crypto::Hash> =
                std::collections::HashSet::new();
            let block_has_withdrawal = !bond_inputs_by_tx.is_empty();
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                if block_has_withdrawal && tx_index > 0 {
                    earlier_tx_hashes.insert(block.transactions[tx_index - 1].hash());
                }
                match tx.tx_type {
                    TxType::AddBond => {
                        let Some(ab) = tx.add_bond_data() else {
                            continue;
                        };
                        let requested = tx
                            .outputs
                            .iter()
                            .filter(|o| o.output_type == doli_core::transaction::OutputType::Bond)
                            .count() as u32;
                        let pk_hash = crypto_hash(ab.producer_pubkey.as_bytes());
                        let prior = in_block_addbond.get(&pk_hash).copied().unwrap_or(0);
                        in_block_addbond.insert(pk_hash, prior.saturating_add(requested));
                    }
                    TxType::Exit => {
                        // An Exit carries zero inputs and zero outputs, so it
                        // shares a block with a withdrawal without any UTXO
                        // conflict — yet apply bumps `withdrawal_pending_count
                        // += bond_count` for it immediately (tx_processing.rs,
                        // Exit arm). Charging the allowance here is what keeps
                        // `[Exit(p), RequestWithdrawal(p, n)]` from being
                        // admitted and then half-applied.
                        //
                        // Apply re-reads an UNCHANGED `bond_count` per Exit and
                        // uses `+=`, so two Exits for one producer charge it
                        // TWICE. Reproduce that: parity with apply is the rule,
                        // not arithmetic tidiness. An Exit naming a producer
                        // the set has never seen charges nothing, exactly as
                        // apply's `get_by_pubkey` guard does.
                        let Some(ed) = tx.exit_data() else {
                            continue;
                        };
                        let pk_hash = crypto_hash(ed.public_key.as_bytes());
                        let held = producers
                            .get_by_pubkey(&ed.public_key)
                            .map(|i| i.bond_count)
                            .unwrap_or(0);
                        let prior = in_block_withdrawn.get(&pk_hash).copied().unwrap_or(0);
                        in_block_withdrawn.insert(pk_hash, prior.saturating_add(held));
                    }
                    TxType::RequestWithdrawal => {
                        let Some(wd) = tx.withdrawal_request_data() else {
                            continue;
                        };
                        let pk = &wd.producer_pubkey;
                        let pk_hash = crypto_hash(pk.as_bytes());
                        // An unknown producer has no holdings at all: the apply
                        // pass queues nothing for it, so admitting the block
                        // would spend Bond UTXOs with zero producer-set effect.
                        let Some(info) = producers.get_by_pubkey(pk) else {
                            // S3/F2: the reindex tool rebuilds the ProducerSet
                            // as it walks, so "registered here" is not knowable
                            // from a partially-rebuilt set (INC-I-064 shape).
                            if mode == ValidationMode::Replay {
                                warn!(
                                    "[REPLAY_SKIP] RequestWithdrawal at height={} names \
                                     unregistered producer={} ({} bonds)",
                                    height, pk_hash, wd.bond_count
                                );
                                continue;
                            }
                            anyhow::bail!(
                                "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER] RequestWithdrawal at height={} \
                                 for unregistered producer={} ({} bonds)",
                                height,
                                pk_hash,
                                wd.bond_count
                            );
                        };
                        let prior_add = in_block_addbond.get(&pk_hash).copied().unwrap_or(0);
                        let prior_wd = in_block_withdrawn.get(&pk_hash).copied().unwrap_or(0);
                        let allowance = info
                            .bond_count
                            .saturating_add(producers.pending_addbond_count(pk))
                            .saturating_add(prior_add)
                            .saturating_sub(info.withdrawal_pending_count)
                            .saturating_sub(prior_wd);
                        if wd.bond_count > allowance {
                            anyhow::bail!(
                                "[ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at height={} \
                                 producer={} requests {} bonds but allowance is {} \
                                 (held={}, pending_addbond={}, in_block_addbond={}, \
                                 withdrawal_pending={}, in_block_withdrawn={})",
                                height,
                                pk_hash,
                                wd.bond_count,
                                allowance,
                                info.bond_count,
                                producers.pending_addbond_count(pk),
                                prior_add,
                                info.withdrawal_pending_count,
                                prior_wd
                            );
                        }
                        // Bind the DECLARED count to the named producer's OWN
                        // Bond UTXOs destroyed. The allowance above bounds the
                        // declared number from ABOVE only, so under-declaring
                        // passes it trivially while `process_transaction_utxos`
                        // spends every input unconditionally: 434 Bond UTXOs
                        // gone, 1 bond removed, 433 unbacked weight units left
                        // — the mainnet n11 number from one transaction.
                        //
                        // R4: inputs resolve against the PRE-BLOCK UTXO view, so
                        // an outpoint created earlier in THIS block is invisible
                        // to both counters while apply spends it regardless.
                        // Refuse it. That is what makes the pre-block view
                        // exhaustive and the exclusivity count below complete.
                        if let Some(chained) = tx
                            .inputs
                            .iter()
                            .find(|inp| earlier_tx_hashes.contains(&inp.prev_tx_hash))
                        {
                            anyhow::bail!(
                                "[ECON_WITHDRAWAL_SAME_BLOCK_INPUT] RequestWithdrawal at \
                                 height={} producer={} spends outpoint {}:{} created by an \
                                 earlier transaction of this same block",
                                height,
                                pk_hash,
                                chained.prev_tx_hash,
                                chained.output_index
                            );
                        }
                        // S3/F2: every term of R3 and of both R2 shapes is read
                        // from the pre-block UTXO view, which Replay
                        // legitimately sees degraded. R1 and R4 above read the
                        // ProducerSet and the block itself, so they stay strict
                        // in all three modes. The allowance charge below still
                        // runs, or R1 would drift for later withdrawals.
                        if mode == ValidationMode::Replay {
                            warn!(
                                "[REPLAY_SKIP] RequestWithdrawal at height={} producer={} \
                                 — UTXO-bound rules not evaluated in Replay",
                                height, pk_hash
                            );
                            in_block_withdrawn
                                .insert(pk_hash, prior_wd.saturating_add(wd.bond_count));
                            continue;
                        }
                        let (bond_inputs, all_bond_inputs) =
                            bond_inputs_by_tx.get(&tx_index).copied().unwrap_or((0, 0));
                        // R3 EXCLUSIVITY (AUDIT-P1-001): every Bond-typed input
                        // must belong to the named producer, or an actor holding
                        // two producer keys declares B's true count and lets A's
                        // Bond UTXOs ride along — all inputs spent, only B's
                        // ledger moved. Runs BEFORE R2 so a foreign rider reports
                        // as a mismatch under either shape below.
                        if all_bond_inputs != bond_inputs {
                            anyhow::bail!(
                                "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] RequestWithdrawal at \
                                 height={} producer={} declares {} bonds but spends {} Bond \
                                 UTXO inputs OWNED BY IT of {} Bond inputs total (of {} inputs)",
                                height,
                                pk_hash,
                                wd.bond_count,
                                bond_inputs,
                                all_bond_inputs,
                                tx.inputs.len()
                            );
                        }
                        // R2 splits by shape. Declaring the WHOLE allowance is a
                        // FULL EXIT: the flush clamps bond_count to 0 and the
                        // auto-exit fires (producer/info.rs), so the ledger cannot
                        // outlive its bonds whatever the declared number was —
                        // the in-band repair for a ledger that already disagrees
                        // with its UTXOs. The obligation moves to the UTXO side:
                        // destroy EVERY Bond UTXO owned, else the survivors stay
                        // spendable with no ledger behind them. A PARTIAL keeps
                        // the strict declared == spent rule.
                        if wd.bond_count == allowance && wd.bond_count > 0 {
                            let addr = hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
                            let owned = owned_live_bonds.get(&addr).copied().unwrap_or(0);
                            if bond_inputs != owned {
                                anyhow::bail!(
                                    "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN] RequestWithdrawal at \
                                     height={} producer={} declares its full allowance of {} \
                                     bonds but spends {} of the {} Bond UTXOs it owns",
                                    height,
                                    pk_hash,
                                    wd.bond_count,
                                    bond_inputs,
                                    owned
                                );
                            }
                        } else if wd.bond_count != bond_inputs {
                            anyhow::bail!(
                                "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] RequestWithdrawal at \
                                 height={} producer={} declares {} bonds but spends {} Bond \
                                 UTXO inputs OWNED BY IT of {} Bond inputs total (of {} inputs)",
                                height,
                                pk_hash,
                                wd.bond_count,
                                bond_inputs,
                                all_bond_inputs,
                                tx.inputs.len()
                            );
                        }
                        in_block_withdrawn.insert(pk_hash, prior_wd.saturating_add(wd.bond_count));
                    }
                    _ => {}
                }
            }
        }

        // === EpochReward validation ===
        let epoch_reward_txs: Vec<&Transaction> = block
            .transactions
            .iter()
            .filter(|tx| tx.tx_type == TxType::EpochReward)
            .collect();

        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let is_epoch_boundary = height > 0
            && !self.config.network.is_in_genesis(height)
            && reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if !epoch_reward_txs.is_empty() {
            // EpochReward only allowed at epoch boundaries, post-genesis.
            //
            // This is a STRUCTURAL CONSENSUS rule — a block carrying an
            // EpochReward TX at a non-boundary height is invalid regardless of
            // sync path. Cheap constant-time arithmetic (`height %
            // blocks_per_epoch`); must fire in BOTH Full and Light modes.
            //
            // Historical note: this check was previously gated behind
            // `ValidationMode::Full` on the theory that Light-mode resync
            // might receive blocks whose "boundary-ness" depended on a
            // different `blocks_per_epoch`. That rationale was wrong:
            // `blocks_per_epoch` is a network-wide constant, not a per-fork
            // state. Skipping the check let the santiago 2026-04-16 05:11 UTC
            // cascade (INC-I-034) proceed — a non-boundary EpochReward was
            // rejected by Full-mode validation then silently "Applied" by a
            // Light-mode apply entry point, corrupting producer_liveness and
            // triggering [UTXO] FAIL downstream.
            if !is_epoch_boundary {
                anyhow::bail!(
                    "[ECON_EPOCH_NOT_BOUNDARY] EpochReward at non-boundary height={} (blocks_per_epoch={})",
                    height,
                    blocks_per_epoch
                );
            }

            // Defense-in-depth: even though `is_epoch_boundary=true` implies
            // `height >= blocks_per_epoch` (so `height / blocks_per_epoch >= 1`),
            // use checked arithmetic so a future refactor cannot silently
            // underflow. Pre-fix, this line executed for any height when
            // `mode=Light` skipped the boundary check — producing either a
            // debug panic or a release-mode wrap to u64::MAX.
            let completed_epoch =
                (height / blocks_per_epoch)
                    .checked_sub(1)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "[ECON_EPOCH_UNDERFLOW] completed_epoch underflow at height={} (blocks_per_epoch={}) — internal invariant violated",
                            height,
                            blocks_per_epoch
                        )
                    })?;

            // No EpochReward at epoch 0 (genesis bonds drained the pool)
            if completed_epoch == 0 {
                anyhow::bail!("[ECON_EPOCH_ZERO] EpochReward not allowed at epoch 0 (genesis pool used for bonds) at height={}", height);
            }

            // Exactly one EpochReward TX per block
            if epoch_reward_txs.len() != 1 {
                anyhow::bail!(
                    "[ECON_EPOCH_DUPLICATE] expected 1 EpochReward TX, got {} at height={}",
                    epoch_reward_txs.len(),
                    height
                );
            }
            let epoch_tx = epoch_reward_txs[0];

            // Structural consistency checks — always fire (Full + Light).
            // These are cheap constant-time checks of wire-format fields and
            // conservation; they do not depend on local fork state. Lifting
            // the Full-only gate closes the INC-I-034 apply-after-reject desync.
            //
            // Validate extra_data contains correct height + epoch
            if epoch_tx.extra_data.len() < 16 {
                anyhow::bail!(
                    "[ECON_EPOCH_EXTRA_DATA] EpochReward extra_data too short: {} bytes < 16 required at height={}",
                    epoch_tx.extra_data.len(),
                    height
                );
            }
            let embedded_height = u64::from_le_bytes(epoch_tx.extra_data[0..8].try_into().unwrap());
            let embedded_epoch = u64::from_le_bytes(epoch_tx.extra_data[8..16].try_into().unwrap());
            if embedded_height != height {
                anyhow::bail!(
                    "[ECON_EPOCH_HEIGHT] EpochReward embedded_height={} != block height={}",
                    embedded_height,
                    height
                );
            }
            if embedded_epoch != completed_epoch {
                anyhow::bail!(
                    "[ECON_EPOCH_NUMBER] EpochReward embedded_epoch={} != completed_epoch={} at height={}",
                    embedded_epoch,
                    completed_epoch,
                    height
                );
            }

            // Conservation: total distributed must not exceed pool balance.
            // Pre-activation: include current coinbase (side-effect consumes all).
            // Post-activation: only existing UTXOs (explicit inputs don't include
            // current coinbase — its hash isn't known at assembly time).
            // Conservation is consensus-critical; must fire in both modes.
            let total_distributed: u64 = epoch_tx.outputs.iter().map(|o| o.amount).sum();
            let pool_balance = {
                let utxo = self.utxo_set.read().await;
                let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                if height >= doli_core::consensus::EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT {
                    utxo_total // post-activation: only existing UTXOs
                } else {
                    utxo_total + coinbase_amount // pre-activation: + current coinbase
                }
            };

            if total_distributed > pool_balance && mode != ValidationMode::Replay {
                anyhow::bail!(
                    "[ECON_EPOCH_OVERFLOW] EpochReward total {} exceeds pool balance {} at height={} — inflation attack",
                    total_distributed,
                    pool_balance,
                    height
                );
            }

            // Distribution amount check depends on local-state rewards
            // calculation (`calculate_epoch_rewards`). In Light mode the local
            // state may be on a transient micro-fork, so keep Full-only.
            if matches!(mode, ValidationMode::Full) {
                // Exact match of amounts and recipients
                let expected = match self.calculate_epoch_rewards(completed_epoch).await {
                    Ok(outputs) => outputs,
                    Err(err) => {
                        warn!(
                            "[INC_I_081_VALIDATION_SKIP] Cannot validate EpochReward content at h={} for epoch={}: {}. \
                             Skipping strict comparison (Full mode degrades to Light for this block).",
                            height, completed_epoch, err
                        );
                        return Ok(());
                    }
                };
                let mut expected_sorted: Vec<(u64, crypto::Hash)> = expected;
                expected_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

                let mut actual_sorted: Vec<(u64, crypto::Hash)> = epoch_tx
                    .outputs
                    .iter()
                    .map(|o| (o.amount, o.pubkey_hash))
                    .collect();
                actual_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

                if expected_sorted != actual_sorted {
                    let total_distributed: u64 = actual_sorted.iter().map(|(a, _)| *a).sum();
                    let expected_total: u64 = expected_sorted.iter().map(|(a, _)| *a).sum();
                    warn!(
                        "[VALIDATION] EpochReward divergence: h={} hash={:.16} epoch={} expected_outputs={} actual_outputs={} expected_total={} actual_total={}",
                        height,
                        block.hash(),
                        completed_epoch,
                        expected_sorted.len(),
                        actual_sorted.len(),
                        expected_total,
                        total_distributed
                    );
                    anyhow::bail!(
                        "[ECON_EPOCH_DISTRIBUTION] EpochReward mismatch at height={}: \
                         expected {} outputs totaling {}, got {} outputs totaling {}",
                        height,
                        expected_sorted.len(),
                        expected_total,
                        actual_sorted.len(),
                        total_distributed
                    );
                }
            }

            // INC-I-064: Pool input verification runs in Full + Light modes.
            // The pool UTXO set is deterministic — comparing inputs against it
            // is safe even during sync/reorg. Previously gated behind Full-only,
            // which let syncing nodes accept EpochReward TXs with stale inputs.
            // Replay mode skips: historical blocks (e.g., E362) have mismatched
            // inputs that are part of the chain's consensus history.
            if mode != ValidationMode::Replay
                && height >= doli_core::consensus::EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT
            {
                if epoch_tx.inputs.is_empty() {
                    anyhow::bail!(
                        "[ECON_EPOCH_NO_INPUTS] EpochReward at height={} (post-activation) must have explicit pool inputs",
                        height
                    );
                }
                // Verify inputs match the sorted pool outpoints
                let utxo = self.utxo_set.read().await;
                let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                let mut expected_inputs: Vec<(crypto::Hash, u32)> = pool_utxos
                    .iter()
                    .map(|(op, _)| (op.tx_hash, op.index))
                    .collect();
                expected_inputs.sort();
                drop(utxo);

                let actual_inputs: Vec<(crypto::Hash, u32)> = epoch_tx
                    .inputs
                    .iter()
                    .map(|inp| (inp.prev_tx_hash, inp.output_index))
                    .collect();

                if actual_inputs != expected_inputs {
                    anyhow::bail!(
                        "{}",
                        format_epoch_inputs_mismatch(height, &expected_inputs, &actual_inputs)
                    );
                }
            } else if !epoch_tx.inputs.is_empty() {
                anyhow::bail!(
                    "[ECON_EPOCH_PRE_INPUTS] EpochReward at height={} (pre-activation) must not have inputs",
                    height
                );
            }
        } else if is_epoch_boundary && matches!(mode, ValidationMode::Full) {
            // Only enforce missing-EpochReward check in Full mode.
            // In Light mode (sync/reorg), the canonical chain may have blocks at epoch
            // boundaries produced by nodes with different epoch parameters (ConsensusParams
            // vs NetworkParams mismatch). Rejecting these blocks prevents recovery.
            //
            // Defense-in-depth: `is_epoch_boundary` implies height >= blocks_per_epoch,
            // but use checked_sub to harden against future changes to that invariant.
            let completed_epoch = (height / blocks_per_epoch).saturating_sub(1);
            if completed_epoch > 0 {
                let expected = match self.calculate_epoch_rewards(completed_epoch).await {
                    Ok(outputs) => outputs,
                    Err(err) => {
                        warn!(
                            "[INC_I_081_MISSING_CHECK_SKIP] Cannot enforce missing-EpochReward check at h={} for epoch={}: {}. \
                             Local store is incomplete — cannot prove an EpochReward was required.",
                            height, completed_epoch, err
                        );
                        return Ok(());
                    }
                };
                if !expected.is_empty() {
                    anyhow::bail!(
                        "[ECON_EPOCH_MISSING] epoch boundary at height={} missing EpochReward TX for epoch={} ({} qualified producers)",
                        height, completed_epoch, expected.len()
                    );
                }
            }
        }

        // === INC-I-080: per-producer AddBond cap (height-gated) ===
        //
        // Enforced HERE (pre-mutation, runs in Full/Light/Replay like the
        // EpochReward structural rule above) — NOT in the producer-effects
        // pass, which returns `()` and runs after the UTXO set was already
        // mutated. Rejecting at validation guarantees that an over-cap
        // AddBond never creates Bond UTXOs ("no orphan Bonds").
        //
        // Pre-activation (`height < AH`): skipped entirely — the historical
        // clip-at-epoch-flush behavior (`ProducerInfo::add_bonds`) is
        // preserved so replaying pre-activation blocks stays bit-identical
        // (no consensus change before the activation height).
        //
        // Post-activation: a block carrying an AddBond whose
        //   current bond_count + already-queued pending AddBonds
        //   + AddBonds earlier in THIS block + this request
        // would exceed MAX_BONDS_PER_PRODUCER is rejected. Determinism:
        // every node on the same params+height reaches the same verdict —
        // consensus-safe under a rolling deploy (gate flips atomically at
        // the activation height). Must run in ALL modes: a Light/Replay
        // apply of a post-AH over-cap block that Full-mode rejected would
        // diverge state (the INC-I-034 class of bug).
        let addbond_cap_ah = self
            .config
            .network
            .params()
            .addbond_cap_enforcement_activation_height;
        if height >= addbond_cap_ah {
            let producers = self.producer_set.read().await;
            // Running per-producer tally of AddBond bond counts seen earlier
            // in THIS block — they are not yet in `pending_updates` during
            // validation, but will all be flushed together at epoch boundary.
            let mut in_block: std::collections::HashMap<crypto::Hash, u32> =
                std::collections::HashMap::new();
            for tx in block.transactions.iter() {
                if tx.tx_type != TxType::AddBond {
                    continue;
                }
                let Some(ab) = tx.add_bond_data() else {
                    continue;
                };
                let pk = &ab.producer_pubkey;
                let pk_hash = crypto_hash(pk.as_bytes());
                let current = producers
                    .get_by_pubkey(pk)
                    .map(|i| i.bond_count)
                    .unwrap_or(0);
                let prior_in_block = in_block.get(&pk_hash).copied().unwrap_or(0);
                let pending = producers
                    .pending_addbond_count(pk)
                    .saturating_add(prior_in_block);
                // `requested` mirrors apply: the number of Bond outputs the
                // AddBond carries (what `ProducerInfo::add_bonds` receives).
                let requested = tx
                    .outputs
                    .iter()
                    .filter(|o| o.output_type == doli_core::transaction::OutputType::Bond)
                    .count() as u32;
                if let Err(e) = doli_core::validation::check_addbond_cap(
                    current,
                    pending,
                    requested,
                    height,
                    addbond_cap_ah,
                ) {
                    anyhow::bail!(
                        "[{}] AddBond cap exceeded at height={} producer={}: {}",
                        e.error_code(),
                        height,
                        pk_hash,
                        e
                    );
                }
                in_block.insert(pk_hash, prior_in_block.saturating_add(requested));
            }
        }

        Ok(())
    }

    /// Handle a new transaction from the network
    pub async fn handle_new_transaction(&self, tx: Transaction) -> Result<()> {
        let tx_hash = tx.hash();

        // Check if we already have this transaction
        {
            let mempool = self.mempool.read().await;
            if mempool.contains(&tx_hash) {
                debug!("Transaction {} already in mempool", tx_hash);
                return Ok(());
            }
        }

        // Add to mempool
        let current_height = self.chain_state.read().await.best_height;
        // INC-I-173 M3 / F4 (AUDIT-P3-002). SHAPE-based routing: the 0-fee system
        // lane is for transactions that are genuinely 0-in/0-out AND whose type is
        // authorized to exist in that shape. See the same note at the RPC
        // admission site in `crates/rpc/src/methods/transaction.rs`.
        let is_zero_flow = tx.is_zero_flow();
        let result = {
            let mut mempool = self.mempool.write().await;
            if is_zero_flow {
                // Zero-flow txs have no inputs/outputs/fees — use system tx path
                mempool
                    .add_system_transaction(tx.clone(), current_height)
                    .map(|_| ())
            } else {
                let utxo = self.utxo_set.read().await;
                mempool
                    .add_transaction(tx.clone(), &utxo, current_height)
                    .map(|_| ())
            }
        };

        match result {
            Ok(_) => {
                info!("Added transaction {} to mempool", tx_hash);
                // Broadcast to WebSocket subscribers
                if let Some(ref ws_tx) = *self.ws_sender.read().await {
                    let tx_type = format!("{:?}", tx.tx_type).to_lowercase();
                    let _ = ws_tx.send(rpc::WsEvent::NewTx {
                        hash: tx_hash.to_hex(),
                        tx_type,
                        size: tx.size(),
                        fee: 0,
                    });
                }
                // Broadcast to network
                if let Some(ref network) = self.network {
                    let _ = network.broadcast_transaction(tx).await;
                }
            }
            Err(e) => {
                debug!("Failed to add transaction {} to mempool: {}", tx_hash, e);
            }
        }

        Ok(())
    }

    /// Handle a sync request from a peer.
    /// Called by on_sync_request() in network_events.rs (the production path).
    /// Note: handle_sync_request_bg() in event_loop.rs is dead code.
    pub async fn handle_sync_request(
        &self,
        request: network::protocols::SyncRequest,
        channel: network::ResponseChannel<network::protocols::SyncResponse>,
    ) -> Result<()> {
        let response = match request {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                let mut headers = Vec::new();
                let state = self.chain_state.read().await;
                let genesis_hash = state.genesis_hash;
                let best_height = state.best_height;
                drop(state);

                // Determine starting height via O(1) hash→height index.
                // The hash_to_height index is populated by:
                // 1. rebuild_canonical_index (one-time migration on startup)
                // 2. Normal block insertion during sync/production
                // No linear fallback — avoids O(n) scans that caused timeouts.
                let start_height = if start_hash == genesis_hash {
                    0
                } else {
                    match self
                        .block_store
                        .get_height_by_hash(&start_hash)
                        .ok()
                        .flatten()
                    {
                        Some(h) => h,
                        None => {
                            // Unknown hash — respond empty so requester doesn't timeout
                            debug!(
                                "GetHeaders: unknown start_hash {} (responding with empty)",
                                start_hash
                            );
                            if let Some(ref network) = self.network {
                                let _ = network
                                    .send_sync_response(channel, SyncResponse::Headers(vec![]))
                                    .await;
                            }
                            return Ok(());
                        }
                    }
                };

                // Return headers from start_height+1 up to max_count
                // Use get_hash_by_height → get_header to avoid deserializing full blocks
                let end_height = (start_height + max_count as u64).min(best_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(hash)) = self.block_store.get_hash_by_height(height) {
                        if let Ok(Some(header)) = self.block_store.get_header(&hash) {
                            headers.push(header);
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                debug!(
                    "GetHeaders: returning {} headers (heights {}..={})",
                    headers.len(),
                    start_height + 1,
                    end_height
                );
                SyncResponse::Headers(headers)
            }

            SyncRequest::GetBodies { hashes } => {
                let mut bodies = Vec::new();
                for hash in hashes {
                    if let Ok(Some(block)) = self.block_store.get_block(&hash) {
                        bodies.push(block);
                    }
                }
                SyncResponse::Bodies(bodies)
            }

            SyncRequest::GetBlockByHeight { height } => {
                match self.block_store.get_block_by_height(height) {
                    Ok(Some(block)) => SyncResponse::Block(Some(block)),
                    _ => SyncResponse::Block(None),
                }
            }

            SyncRequest::GetBlockByHash { hash } => match self.block_store.get_block(&hash) {
                Ok(Some(block)) => SyncResponse::Block(Some(block)),
                _ => SyncResponse::Block(None),
            },

            // INC-I-012 F1: Height-based header request. Used after snap sync
            // when the node's local_hash is unrecognizable by peers. The server
            // uses its OWN canonical chain at start_height, bypassing the hash
            // lookup that causes the deadlock.
            SyncRequest::GetHeadersByHeight {
                start_height,
                max_count,
            } => {
                let mut headers = Vec::new();
                let state = self.chain_state.read().await;
                let best_height = state.best_height;
                drop(state);

                let max_count = max_count.min(2000); // Cap to prevent expensive iteration
                let end_height = start_height
                    .saturating_add(max_count as u64)
                    .min(best_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(hash)) = self.block_store.get_hash_by_height(height) {
                        if let Ok(Some(header)) = self.block_store.get_header(&hash) {
                            headers.push(header);
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                debug!(
                    "GetHeadersByHeight: returning {} headers (heights {}..={})",
                    headers.len(),
                    start_height + 1,
                    end_height
                );
                SyncResponse::Headers(headers)
            }

            SyncRequest::GetStateRoot { block_hash: _ } => {
                // M1 (State-Root Lazy Tier-0): memoize-on-compute seam. Serves
                // the memo in O(1) on a best_hash-keyed hit; on cold/stale memo,
                // recomputes the legacy root and WRITES IT BACK. Root value is
                // byte-identical to the prior inline compute at every height.
                self.serve_state_root().await
            }

            SyncRequest::GetStateSnapshot { block_hash } => {
                // INC-I-156 / AUDIT-P1-001: moved verbatim to the
                // `serve_state_snapshot` seam (`state_snapshot_serve.rs`), which
                // returns a `SyncResponse` and so can be exercised without a
                // libp2p ResponseChannel. It refuses while an interrupted
                // rebuild-from-genesis has left the durable UTXO set truncated.
                self.serve_state_snapshot(block_hash).await
            }

            SyncRequest::DirectAttestation { data } => {
                // Re-broadcast via gossip so it reaches minute tracker
                if let Some(ref network) = self.network {
                    let _ = network.broadcast_attestation(data).await;
                }
                SyncResponse::Block(None)
            }
        };

        if let Some(ref network) = self.network {
            let _ = network.send_sync_response(channel, response).await;
        }

        Ok(())
    }
}

// === INC-I-143 D5: EpochReward pool-input mismatch diagnostic ===

/// Max differing outpoints reported per side in an EpochReward pool-input
/// mismatch diagnostic. Bounds log volume so a large divergence (e.g. the
/// 360-input mismatch seen in INC-I-143) cannot flood the log.
const EPOCH_INPUTS_MISMATCH_SAMPLE: usize = 5;

/// Build a content-aware diagnostic for an EpochReward pool-input mismatch.
///
/// INC-I-143 (D5): the previous message printed only the two input COUNTS,
/// which were often EQUAL on a failure line (e.g. "expected 360 inputs, got
/// 360") because the divergence was in outpoint IDENTITY, not cardinality.
/// That actively misdirected diagnosis (seed2/seed3/n11 halt, 2026-07). This
/// reports WHAT differs — the symmetric difference of the two outpoint sets —
/// bounded to the first `EPOCH_INPUTS_MISMATCH_SAMPLE` entries per side so a
/// large mismatch cannot flood the log.
///
/// Pure and deterministic: identical inputs on every node produce an identical
/// string. It does NOT change the pass/fail decision — only the message text.
fn format_epoch_inputs_mismatch(
    height: u64,
    expected: &[(crypto::Hash, u32)],
    actual: &[(crypto::Hash, u32)],
) -> String {
    use std::collections::BTreeSet;
    let expected_set: BTreeSet<(crypto::Hash, u32)> = expected.iter().copied().collect();
    let actual_set: BTreeSet<(crypto::Hash, u32)> = actual.iter().copied().collect();

    let missing: Vec<(crypto::Hash, u32)> = expected_set.difference(&actual_set).copied().collect();
    let unexpected: Vec<(crypto::Hash, u32)> =
        actual_set.difference(&expected_set).copied().collect();

    let sample = |ops: &[(crypto::Hash, u32)]| -> String {
        ops.iter()
            .take(EPOCH_INPUTS_MISMATCH_SAMPLE)
            .map(|(h, i)| format!("{}:{}", &h.to_hex()[..16], i))
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "[ECON_EPOCH_INPUTS_MISMATCH] EpochReward pool inputs mismatch at height={}: \
         expected {} inputs, got {} ({} differing outpoints). \
         missing_from_actual ({}): [{}]; unexpected_in_actual ({}): [{}]",
        height,
        expected.len(),
        actual.len(),
        missing.len() + unexpected.len(),
        missing.len(),
        sample(&missing),
        unexpected.len(),
        sample(&unexpected),
    )
}

#[cfg(test)]
mod inc_i_143_d5_tests {
    //! INC-I-143 D5: the EpochReward pool-input mismatch diagnostic must report
    //! WHAT differs, not just the (often equal) input counts.
    //!
    //! OUTPUT CONTRACT:
    //!   Function under test: format_epoch_inputs_mismatch(height, expected, actual)
    //!   Output: the diagnostic String bailed at validation_checks.rs (~L779),
    //!           post-activation EpochReward pool-input verification failure branch.
    //! INPUT PARTITIONS:
    //!   P1. equal-count, content-divergent sets (the INC-I-143 case) — MUST name
    //!       the differing outpoints on BOTH sides; MUST NOT reduce to equal counts.
    //!   P2. large divergence (>SAMPLE differing per side) — MUST bound the listing
    //!       to SAMPLE entries per side (no log flood) while reporting true totals.
    //!   P3. pure/deterministic — identical inputs yield an identical string.

    use super::*;

    fn oc(byte: u8, index: u32) -> (crypto::Hash, u32) {
        (crypto::Hash::from_bytes([byte; 32]), index)
    }

    fn hex16(byte: u8) -> String {
        crypto::Hash::from_bytes([byte; 32]).to_hex()[..16].to_string()
    }

    // P1: equal-count, content-divergent sets. Reproduces INC-I-143 D5 —
    // pre-fix this printed only "expected 3 inputs, got 3" (equal counts).
    #[test]
    fn equal_count_divergent_sets_report_the_differing_outpoints() {
        // Both sides have 3 inputs (EQUAL count) but differ in identity:
        // expected = {oc1, oc2, oc3}; actual = {oc1, oc2, oc9}.
        let expected = vec![oc(1, 0), oc(2, 0), oc(3, 0)];
        let actual = vec![oc(1, 0), oc(2, 0), oc(9, 0)];

        let msg = format_epoch_inputs_mismatch(108_720, &expected, &actual);

        // Counts retained for continuity with prior tooling.
        assert!(msg.contains("expected 3 inputs, got 3"), "msg: {msg}");
        // The D5 fix: the differing outpoints MUST be named.
        // oc(3) is missing-from-actual; oc(9) is unexpected-in-actual.
        assert!(
            msg.contains(&hex16(3)),
            "must name missing outpoint; msg: {msg}"
        );
        assert!(
            msg.contains(&hex16(9)),
            "must name unexpected outpoint; msg: {msg}"
        );
        // Must NOT collapse to only-equal-counts with no identity info.
        assert!(
            msg.contains("differing"),
            "msg must quantify what differs: {msg}"
        );
    }

    // P2: large divergence is bounded — a 40-outpoint mismatch cannot flood.
    #[test]
    fn large_divergence_is_bounded_but_reports_true_totals() {
        // 20 distinct expected (bytes 0..20), 20 distinct actual (bytes 20..40),
        // zero overlap => 40 differing outpoints.
        let expected: Vec<_> = (0u8..20).map(|b| oc(b, 0)).collect();
        let actual: Vec<_> = (20u8..40).map(|b| oc(b, 0)).collect();

        let msg = format_epoch_inputs_mismatch(1, &expected, &actual);

        // True totals reported even though the listing is truncated.
        assert!(msg.contains("40 differing"), "msg: {msg}");
        assert!(msg.contains("missing_from_actual (20)"), "msg: {msg}");
        assert!(msg.contains("unexpected_in_actual (20)"), "msg: {msg}");
        // Bounded to SAMPLE (5) per side, ordered by Hash: bytes 0..5 shown,
        // byte 5 truncated out; bytes 20..25 shown, byte 25 truncated out.
        assert!(msg.contains(&hex16(0)), "first missing shown; msg: {msg}");
        assert!(
            !msg.contains(&hex16(5)),
            "6th missing must be bounded out; msg: {msg}"
        );
        assert!(
            msg.contains(&hex16(20)),
            "first unexpected shown; msg: {msg}"
        );
        assert!(
            !msg.contains(&hex16(25)),
            "6th unexpected must be bounded out; msg: {msg}"
        );
    }

    // P3: pure/deterministic — identical inputs => identical string.
    #[test]
    fn deterministic_for_identical_inputs() {
        let expected = vec![oc(1, 0), oc(2, 1)];
        let actual = vec![oc(1, 0), oc(7, 3)];
        let a = format_epoch_inputs_mismatch(5, &expected, &actual);
        let b = format_epoch_inputs_mismatch(5, &expected, &actual);
        assert_eq!(a, b);
    }
}
