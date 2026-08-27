use super::*;

impl Node {
    /// Process governance transactions (MaintainerAdd, MaintainerRemove, ProtocolActivation).
    ///
    /// Governance ops are applied immediately (not epoch-deferred), except ProtocolActivation
    /// which is verified here but applied when chain_state lock is acquired.
    ///
    /// Returns `Some((version, epoch))` if a ProtocolActivation was verified.
    ///
    /// INC-I-172 M2 (F3/F4): every authorization decision below is gated on
    /// `NetworkParams::maintainer_derivation_activation_height` against the
    /// chain-derived `height` (never a per-process counter, INV-SYNC-012).
    /// Below the gate the historical entry-counting counter and the producer-key
    /// fallback are reproduced byte-for-byte; at and above it, thresholds mean
    /// DISTINCT signers and an unusable on-chain root fails closed.
    ///
    /// INC-I-176 M2 (REQ-176-022): a SECOND, independent gate —
    /// `NetworkParams::inc_i_176_auth_binding_activation_height` (#22) — selects
    /// WHICH BYTES the two maintainer arms verify against, again on the
    /// chain-derived `height`. The `ProtocolActivation` arm is deliberately
    /// UNTOUCHED by #22: `activate:{v}:{e}` is a different signing family. Both
    /// gates are read from ONE `self.config.network.params()` binding at the top
    /// of this function, so "the two gates come from the same params" is a
    /// STRUCTURAL fact and not a convention (M2 review F11).
    ///
    /// This site stays NON-FATAL under both gates. It returns `Option`, never
    /// `Result`, and it is reached from `apply_block` — a failed authorization
    /// warns and skips, and the block that carried it is still applied.
    pub async fn process_transaction_governance(
        &self,
        tx: &Transaction,
        height: u64,
        producers: &ProducerSet,
    ) -> Option<(u32, u64)> {
        // ONE `params()` resolution for BOTH gates (M2 review F11). This function
        // runs for EVERY transaction in every applied block, not only governance
        // ones (`apply_block/mod.rs` calls it inside the tx loop), so a second
        // `Network::params()` read here would be a per-transaction cost on the
        // apply hot path. `params()` is a `OnceLock::get_or_init` returning
        // `&'static NetworkParams`; binding it once collapses two atomic acquire
        // loads back to the one this site already paid before M2.
        //
        // It is also the structural form of the property the two gates need:
        // #20 (WHICH COUNTER) and #22 (WHICH BYTES) are read from the SAME
        // `NetworkParams` value by construction, not by convention.
        let params = self.config.network.params();

        let activation_height = params.maintainer_derivation_activation_height;

        // INC-I-176 M2 (#22) — WHICH BYTES a maintainer authorization is verified
        // against.
        let auth_binding_activation_height = params.inc_i_176_auth_binding_activation_height;

        let genesis_hash = self.params.genesis_hash;

        // Process MaintainerAdd transactions — applied immediately (governance, not epoch-deferred)
        if tx.tx_type == TxType::AddMaintainer {
            if let Some(maintainer_state) = &self.maintainer_state {
                if let Some(data) =
                    doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                {
                    let mut ms = maintainer_state.write().await;
                    // INC-I-176 M2 / REQ-176-022 — Path-Coverage for gate #22.
                    //
                    //   height <  inc_i_176_auth_binding_activation_height
                    //     -> signing_message_legacy(true, target), i.e.
                    //        `format!("add:{}", target_hex)` BYTE-IDENTICAL to what
                    //        the live fleet accepts today. Frozen consensus
                    //        history; a node that changed its mind here would hold
                    //        a different maintainer trust root from every peer.
                    //     Selected by: testnet heights < 300_000; mainnet always
                    //        (#22 = u64::MAX, unpinned in M2); devnet heights
                    //        < 20 (#22 = 20, NOT 0 — the five fenced INC-I-174
                    //        suites run at heights 0-7 and stay on THIS arm).
                    //   height >= inc_i_176_auth_binding_activation_height
                    //     -> signing_message(genesis, true, target, sentinel), the
                    //        BLAKE3-256 domain-tagged, genesis-bound digest that
                    //        closes AUDIT-P0-011 and AUDIT-P1-016.
                    //     Selected by: testnet heights >= 300_000; devnet heights
                    //        >= 20 (devnet is the ONLY network on which this arm
                    //        is reachable today); mainnet only once #22 is pinned
                    //        at release.
                    //
                    // The free function is called DIRECTLY rather than through
                    // `MaintainerChangeData::signing_message`, which stays the
                    // legacy-only helper the in-repo CLI signer uses — below #22
                    // legacy is exactly what this verifier requires, so flipping
                    // the helper would make the signer emit bytes no live node
                    // accepts (M2 Decision 3).
                    //
                    // `height` is the chain-derived block height threaded from
                    // `apply_block`, never a per-process counter (INV-SYNC-012).
                    // `MAINTAINER_AUTH_VALID_BEFORE_UNSET` = u64::MAX = "never
                    // expires" = today's unbounded semantics; the payload gains a
                    // real `valid_before` field in M2.5.
                    let message = doli_core::maintainer::signing_message_at(
                        genesis_hash.as_bytes(),
                        true,
                        &data.target,
                        doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                        height,
                        auth_binding_activation_height,
                    );
                    if ms.set.verify_multisig_at(
                        &data.signatures,
                        &message,
                        height,
                        activation_height,
                    ) {
                        match ms.set.add_maintainer(data.target, height) {
                            Ok(()) => {
                                ms.last_derived_height = height;
                                if let Err(e) = ms.save(&self.config.data_dir) {
                                    warn!("Failed to persist maintainer state: {}", e);
                                }
                                info!(
                                    "[MAINTAINER] Added maintainer {} at height {}",
                                    data.target.to_hex(),
                                    height
                                );
                                Self::log_maintainer_set_digest(&ms.set, &genesis_hash, height);
                            }
                            Err(e) => warn!("[MAINTAINER] Add failed: {}", e),
                        }
                    } else {
                        warn!("[MAINTAINER] Rejected AddMaintainer: insufficient signatures");
                    }
                }
            }
        }

        // Process MaintainerRemove transactions — applied immediately
        if tx.tx_type == TxType::RemoveMaintainer {
            if let Some(maintainer_state) = &self.maintainer_state {
                if let Some(data) =
                    doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                {
                    let mut ms = maintainer_state.write().await;
                    // INC-I-176 M2 / REQ-176-022 — Path-Coverage for gate #22, the
                    // REMOVE arm. Same two branches, same heights, same selectors
                    // as the add arm above, with `is_add = false`:
                    //
                    //   height <  #22 -> signing_message_legacy(false, target)
                    //                    = `format!("remove:{}", target_hex)`
                    //   height >= #22 -> signing_message(genesis, false, target,
                    //                                    sentinel)
                    //
                    // The `false` is load-bearing and is the term a copy-paste of
                    // the add arm gets wrong: the action byte is INSIDE the signed
                    // preimage precisely so an `add` authorization can never be
                    // replayed as a `remove` (REQ-176-012). This arm also goes
                    // through a DIFFERENT verifier —
                    // `verify_multisig_excluding_at`, which drops the target's own
                    // signature — so a wiring change that fixed only the add arm
                    // would leave this one unbound.
                    let message = doli_core::maintainer::signing_message_at(
                        genesis_hash.as_bytes(),
                        false,
                        &data.target,
                        doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                        height,
                        auth_binding_activation_height,
                    );
                    if ms.set.verify_multisig_excluding_at(
                        &data.signatures,
                        &message,
                        &data.target,
                        height,
                        activation_height,
                    ) {
                        match ms.set.remove_maintainer(&data.target, height) {
                            Ok(()) => {
                                ms.last_derived_height = height;
                                if let Err(e) = ms.save(&self.config.data_dir) {
                                    warn!("Failed to persist maintainer state: {}", e);
                                }
                                info!(
                                    "[MAINTAINER] Removed maintainer {} at height {}",
                                    data.target.to_hex(),
                                    height
                                );
                                Self::log_maintainer_set_digest(&ms.set, &genesis_hash, height);
                            }
                            Err(e) => warn!("[MAINTAINER] Remove failed: {}", e),
                        }
                    } else {
                        warn!("[MAINTAINER] Rejected RemoveMaintainer: insufficient signatures");
                    }
                }
            }
        }

        // Process ProtocolActivation transactions — verified against on-chain maintainer set
        if tx.tx_type == TxType::ProtocolActivation {
            if let Some(data) = tx.protocol_activation_data() {
                let on_chain = match &self.maintainer_state {
                    Some(maintainer_state) => Some(maintainer_state.read().await.set.clone()),
                    None => None,
                };

                let mset = if height >= activation_height {
                    // F4 / REQ-172-002 — FAIL CLOSED. Activation authority comes
                    // from the on-chain maintainer root or from nowhere. The old
                    // producer-key fallback let any actor who could drive the root
                    // sub-threshold reclaim activation authority through the very
                    // key set INC-I-172 is retiring.
                    let set = on_chain.unwrap_or_default();
                    if !set.is_authorizable() {
                        warn!(
                            "[PROTOCOL] Rejected activation at height {}: the on-chain maintainer \
                             root is absent or sub-threshold ({} members, threshold {}). The \
                             producer-key fallback is closed at and above \
                             maintainer_derivation_activation_height {} (INC-I-172 F4).",
                            height,
                            set.member_count(),
                            set.threshold,
                            activation_height
                        );
                        return None;
                    }
                    set
                } else {
                    // PRE-ACTIVATION ONLY: use the on-chain set if bootstrapped,
                    // otherwise fall back to ad-hoc producer-key derivation.
                    // Frozen — this decides which activations took effect in
                    // consensus history.
                    match on_chain {
                        Some(set) if set.is_fully_bootstrapped() => set,
                        _ => Self::derive_ad_hoc_maintainer_set(producers, height),
                    }
                };

                let message = data.signing_message();
                if mset.verify_multisig_at(&data.signatures, &message, height, activation_height) {
                    info!(
                        "[PROTOCOL] Verified activation tx: v{} at epoch {}",
                        data.protocol_version, data.activation_epoch
                    );
                    return Some((data.protocol_version, data.activation_epoch));
                } else {
                    warn!("[PROTOCOL] Rejected activation: insufficient maintainer signatures");
                }
            }
        }

        None
    }

    /// INC-I-173 M3 / F6 (AUDIT-P1-003) — publish the chain-derived maintainer-set
    /// digest on a FIXED, greppable token after every applied rotation.
    ///
    /// `MAINTAINER_SET_DIGEST` is the grep anchor. It lets an operator correlate
    /// "do we hold the same release-verification trust root?" across the fleet from
    /// LOGS ALONE, without shipping member lists around and without an RPC round
    /// trip to every host. The same value is served by `getMaintainerSet`.
    fn log_maintainer_set_digest(
        set: &doli_core::MaintainerSet,
        genesis_hash: &crypto::Hash,
        height: u64,
    ) {
        let digest = doli_core::maintainer::maintainer_set_digest(set, genesis_hash.as_bytes());
        info!(
            "[MAINTAINER] MAINTAINER_SET_DIGEST={} members={} threshold={} last_updated={} height={}",
            hex::encode(digest),
            set.member_count(),
            set.threshold,
            set.last_updated,
            height
        );
    }

    /// Derive an ad-hoc MaintainerSet from producers.
    ///
    /// **PRE-ACTIVATION ONLY.** Reachable exclusively below
    /// `maintainer_derivation_activation_height`; at and above it,
    /// `ProtocolActivation` fails closed instead (INC-I-172 M2, F4). Kept —
    /// unchanged, including its HashMap-ordered non-determinism — because it
    /// decides which activations were accepted in consensus history.
    fn derive_ad_hoc_maintainer_set(
        producers: &ProducerSet,
        height: u64,
    ) -> doli_core::MaintainerSet {
        let mut sorted = producers.all_producers().to_vec();
        sorted.sort_by_key(|p| p.registered_at);
        let keys: Vec<crypto::PublicKey> = sorted
            .iter()
            .take(doli_core::maintainer::INITIAL_MAINTAINER_COUNT)
            .map(|p| p.public_key)
            .collect();
        doli_core::MaintainerSet::with_members(keys, height)
    }
}
