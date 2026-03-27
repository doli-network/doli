use super::*;

impl Node {
    /// Process governance transactions (MaintainerAdd, MaintainerRemove, ProtocolActivation).
    ///
    /// Governance ops are applied immediately (not epoch-deferred), except ProtocolActivation
    /// which is verified here but applied when chain_state lock is acquired.
    ///
    /// Returns `Some((version, epoch))` if a ProtocolActivation was verified.
    pub async fn process_transaction_governance(
        &self,
        tx: &Transaction,
        height: u64,
        producers: &ProducerSet,
    ) -> Option<(u32, u64)> {
        // Process MaintainerAdd transactions — applied immediately (governance, not epoch-deferred)
        if tx.tx_type == TxType::AddMaintainer {
            if let Some(maintainer_state) = &self.maintainer_state {
                if let Some(data) =
                    doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                {
                    let mut ms = maintainer_state.write().await;
                    let message = data.signing_message(true);
                    if ms.set.verify_multisig(&data.signatures, &message) {
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
                    if ms
                        .set
                        .verify_multisig_excluding(&data.signatures, &message, &data.target)
                    {
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
                // Use on-chain MaintainerSet if available, fall back to ad-hoc derivation
                let mset = if let Some(maintainer_state) = &self.maintainer_state {
                    let ms = maintainer_state.read().await;
                    if ms.set.is_fully_bootstrapped() {
                        ms.set.clone()
                    } else {
                        // Not yet bootstrapped — derive ad-hoc
                        Self::derive_ad_hoc_maintainer_set(producers, height)
                    }
                } else {
                    Self::derive_ad_hoc_maintainer_set(producers, height)
                };

                let message = data.signing_message();
                if mset.verify_multisig(&data.signatures, &message) {
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

    /// Derive an ad-hoc MaintainerSet from producers (used when on-chain set not yet bootstrapped).
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
