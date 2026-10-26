//! Detected double production -> one canonical SlashProducer, kept in the
//! re-offer ledger, admitted on the normal lane and broadcast only when admitted.

use crypto::Hash;
use doli_core::transaction::{SlashData, Transaction};
use network::EquivocationProof;
use tracing::{debug, info};

use super::slash_redeliver::keeps_offer;
use super::Node;

impl Node {
    /// At/above the slash gate judged at best+1, admit and broadcast one
    /// canonical slash for `proof`. `None` below the gate, when the accused
    /// already has a ledger entry or a resident slash, or when admission
    /// refuses it. The ledger entry is written before admission and dropped
    /// only on a refusal `keeps_offer` does not cover.
    pub(super) async fn submit_equivocation_slash(
        &self,
        proof: &EquivocationProof,
    ) -> Option<Hash> {
        let mined_at = self.chain_state.read().await.best_height.saturating_add(1);
        let gate = self
            .config
            .network
            .params()
            .effective_slashing_activation_height();
        if mined_at < gate {
            return None;
        }
        let accused = proof.producer;
        if self.slash_reported.lock().unwrap().contains(&accused) {
            debug!("slash for {} already in the ledger", accused);
            return None;
        }
        let resident = self.mempool.read().await.iter().any(|(_, e)| {
            e.tx.slash_data()
                .is_some_and(|d| d.producer_pubkey == accused)
        });
        if resident {
            debug!("slash for {} already resident; not resubmitted", accused);
            return None;
        }
        let tx = Transaction::new_slash_producer(SlashData::double_production(
            accused,
            proof.block_header_1.clone(),
            proof.block_header_2.clone(),
        ));
        self.slash_reported
            .lock()
            .unwrap()
            .mark(accused, tx.clone());
        let permit = self.network.as_ref().and_then(|n| {
            super::rpc_broadcast::reserve_tx_broadcast(&n.command_sender(), &self.origin_tx_bucket)
        });
        let hash = tx.hash();
        match self.admit_own_slash(&accused, &tx).await {
            Ok(true) => {}
            Ok(false) => {
                debug!("slash for {} already resident; kept in the ledger", accused);
                return None;
            }
            Err(e) => {
                if !keeps_offer(&e) {
                    self.slash_reported.lock().unwrap().forget(&accused);
                }
                debug!("slash {} for {} not admitted: {}", hash, accused, e);
                return None;
            }
        }
        match permit {
            Some(send) => send(tx),
            None => debug!("slash {} admitted; no broadcast slot", hash),
        }
        info!("slash {} for {} admitted", hash, accused);
        Some(hash)
    }
}
