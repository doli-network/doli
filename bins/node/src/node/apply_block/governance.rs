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
    pub async fn process_transaction_governance(
        &self,
        tx: &Transaction,
        height: u64,
        producers: &ProducerSet,
    ) -> Option<(u32, u64)> {
        let activation_height = self
            .config
            .network
            .params()
            .maintainer_derivation_activation_height;

        let genesis_hash = self.params.genesis_hash;

        // Process MaintainerAdd transactions — applied immediately (governance, not epoch-deferred)
        if tx.tx_type == TxType::AddMaintainer {
            if let Some(maintainer_state) = &self.maintainer_state {
                if let Some(data) =
                    doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                {
                    let mut ms = maintainer_state.write().await;
                    let message = data.signing_message(true);
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
                    let message = data.signing_message(false);
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
