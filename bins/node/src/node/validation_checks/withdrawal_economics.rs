use super::*;

impl Node {
    /// INC-I-180 withdrawal-holdings rules and the INC-I-171 vesting payout
    /// bound: two INDEPENDENT height gates over ONE pre-block UTXO scan. The
    /// pass is entered when EITHER gate is live and each rule keeps its own
    /// `height >=` check, so below both heights nothing here runs.
    pub(crate) async fn check_withdrawal_economics(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<()> {
        // Pre-mutation, all modes. Allowance mirrors apply at enqueue time:
        //   bond_count + pending AddBonds + AddBonds earlier in THIS block
        //   - withdrawal_pending_count - bonds charged by Exits and
        //     RequestWithdrawals earlier in THIS block
        // and the declared count is bound to the NAMED producer's Bond UTXOs,
        // so both ledger effects move the same producer by the same magnitude.
        let params = self.config.network.params();
        let withdrawal_gate_ah = params.withdrawal_holdings_gate_activation_height;
        let vesting_ah = params.inc_i_171_vesting_penalty_activation_height;
        let vesting_disable = params.inc_i_171_vesting_penalty_disable_height;
        let vesting_live = height >= vesting_ah && height < vesting_disable;
        // INC-I-171 M6 observe-only shadow. Entering the pass on this term ALONE is
        // verdict-neutral (INV-VEST-007): the `'holdings` block breaks on its own
        // gate below, and `vesting_payout_verdict` returns Ok while height <
        // vesting_ah, so no arm can bail.
        let shadow_active = mode == ValidationMode::Full && height < vesting_ah;
        if height < withdrawal_gate_ah && !vesting_live && !shadow_active {
            return Ok(());
        }
        // Resolve input types FIRST, under the utxo guard alone, and drop it
        // before taking the producer guard. `apply_block` takes utxo then
        // producers (mod.rs:197) while `rollback` takes producers then utxo
        // (rollback.rs:325); holding both here would join those two orders
        // into a lock cycle. Only one guard is ever held at a time.
        // The map carries the WHOLE `WithdrawalInputs` per withdrawal
        // tx: the vesting bound below cannot reopen this guard, so every
        // UTXO-derived term it needs is captured here, in ONE pass.
        // `owned_live_bonds` adds, per DISTINCT named producer, how many Bond
        // UTXOs it owns in that same pre-block view — one owner-index scan
        // per producer, memoized, never one per transaction.
        let mut owned_live_bonds: std::collections::HashMap<crypto::Hash, u32> =
            std::collections::HashMap::new();
        let resolved_by_tx: std::collections::HashMap<usize, storage::producer::WithdrawalInputs> = {
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
                            u32::try_from(utxo.get_bond_entries(&addr).len()).unwrap_or(u32::MAX)
                        });
                    }
                    // `owner == None` matched no Bond in the inline scan this
                    // call replaces, so `owned` is 0 there while `all_bonds`,
                    // which no owner argument can change, still counts them all.
                    let probe =
                        owner.unwrap_or_else(|| crypto::hash::hash(b"inc-i-171-m3-unnamed-owner"));
                    let mut resolved =
                        storage::producer::resolve_withdrawal_inputs(tx, &utxo, &probe);
                    if owner.is_none() {
                        resolved.owned_bonds = 0;
                    }
                    (i, resolved)
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
        let block_has_withdrawal = !resolved_by_tx.is_empty();
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
                    let resolved = resolved_by_tx.get(&tx_index);
                    // The INC-I-180 rules keep their own gate and report FIRST.
                    // Their Replay skips `break` rather than `continue`, so the
                    // independently gated vesting bound below is still reached.
                    'holdings: {
                        if height < withdrawal_gate_ah {
                            break 'holdings;
                        }
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
                                break 'holdings;
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
                            break 'holdings;
                        }
                        let (bond_inputs, all_bond_inputs) =
                            resolved.map_or((0, 0), |r| (r.owned_bonds, r.all_bonds));
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
                    if let Some(resolved) = resolved {
                        if let Err(e) = self.vesting_payout_verdict(
                            tx,
                            resolved,
                            block.header.slot,
                            height,
                            vesting_ah,
                            vesting_disable,
                        ) {
                            // Warn-only in Replay for the R2/R3 reason above: it
                            // reads the pre-block UTXO view, which a reindex
                            // legitimately sees degraded (INC-I-064).
                            if mode == ValidationMode::Replay {
                                warn!(
                                    "[REPLAY_SKIP] [{}] RequestWithdrawal at height={} \
                                     producer={}: {}",
                                    e.error_code(),
                                    height,
                                    pk_hash,
                                    e
                                );
                            } else {
                                anyhow::bail!(
                                    "[{}] RequestWithdrawal at height={} producer={}: {}",
                                    e.error_code(),
                                    height,
                                    pk_hash,
                                    e
                                );
                            }
                        }
                    }
                    if let Some(resolved) = resolved.filter(|_| shadow_active) {
                        self.vesting_shadow_observe(
                            tx,
                            resolved,
                            block.header.slot,
                            height,
                            pk_hash,
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// INC-I-171 M6: observe-only. Returns nothing — below the activation height the
    /// verdict is whatever it was before this ran (INV-VEST-007).
    fn vesting_shadow_observe(
        &self,
        tx: &Transaction,
        resolved: &storage::producer::WithdrawalInputs,
        block_slot: doli_core::types::Slot,
        height: u64,
        pk_hash: crypto::Hash,
    ) {
        crate::metrics::VESTING_SHADOW_EVALUATED.inc();
        // The live gate's own predicate, forced on: one predicate, three call sites
        // (INV-VEST-008).
        if let Err(e) = self.vesting_payout_verdict(tx, resolved, block_slot, height, 0, u64::MAX) {
            crate::metrics::VESTING_WOULD_REJECT
                .with_label_values(&[e.error_code()])
                .inc();
            info!(
                "[VESTING_SHADOW] [{}] RequestWithdrawal at height={} producer={} tx={}: {}",
                e.error_code(),
                height,
                pk_hash,
                tx.hash(),
                e
            );
        }
    }

    /// INC-I-171: the payout ceiling for ONE RequestWithdrawal, read from the
    /// pre-block UTXO view the caller already resolved.
    fn vesting_payout_verdict(
        &self,
        tx: &Transaction,
        resolved: &storage::producer::WithdrawalInputs,
        block_slot: doli_core::types::Slot,
        height: u64,
        activation: u64,
        disable: u64,
    ) -> Result<(), doli_core::validation::ValidationError> {
        if height < activation || height >= disable {
            return Ok(());
        }
        // INV-VEST-013 fail closed: an undecodable creation slot is an unknown
        // age, and an unknown age must never read as fully vested.
        if let Some(&(input_index, len)) = resolved.malformed_bond_inputs.first() {
            return Err(
                doli_core::validation::ValidationError::WithdrawalBondExtraDataMalformed {
                    input_index: u32::try_from(input_index).unwrap_or(u32::MAX),
                    len,
                },
            );
        }
        // `validate_withdrawal_request_data` (core validation/tx_types.rs:568-580)
        // admits EXACTLY one output, of type Normal, so this sum IS the payout.
        let payout = tx
            .outputs
            .iter()
            .fold(0u64, |acc, o| acc.saturating_add(o.amount));
        // Every slot is `Some`: the malformed guard above already returned.
        let spent: Vec<(u64, doli_core::types::Slot)> = resolved
            .spent_bonds
            .iter()
            .filter_map(|(amount, slot)| slot.map(|s| (*amount, s)))
            .collect();
        doli_core::validation::vesting::check_withdrawal_payout_bound(
            payout,
            &spent,
            resolved.non_bond_total,
            block_slot,
            self.config.network.params().vesting_quarter_slots,
            height,
            activation,
            disable,
        )
    }
}
