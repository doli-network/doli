//! Periodic re-offer of the slashes in the `slash_reported` ledger, under the
//! same id, until the accused is Slashed or slash-queued or the slash is
//! refused for good.

use std::collections::HashSet;

use crypto::PublicKey;
use doli_core::transaction::Transaction;
use mempool::MempoolError;
use tracing::{debug, info};

use super::Node;

/// Ledger entries visited per periodic tick.
const REDELIVER_PER_TICK: usize = 8;

/// A refusal after which the ledger keeps the slash for a later re-offer.
pub(super) fn keeps_offer(e: &MempoolError) -> bool {
    matches!(
        e,
        MempoolError::Busy | MempoolError::Full | MempoolError::AlreadyExists
    )
}

impl Node {
    /// Admit `tx` on the normal lane at the tip; `Ok(false)` when a slash
    /// naming `accused` is already resident.
    pub(super) async fn admit_own_slash(
        &self,
        accused: &PublicKey,
        tx: &Transaction,
    ) -> Result<bool, MempoolError> {
        let (height, slot) = {
            let st = self.chain_state.read().await;
            (st.best_height, st.best_slot)
        };
        {
            let mut pool = self.mempool.write().await;
            let resident = pool.iter().any(|(_, e)| {
                e.tx.slash_data()
                    .is_some_and(|d| d.producer_pubkey == *accused)
            });
            if resident {
                return Ok(false);
            }
            let utxo = self.utxo_set.read().await;
            pool.add_transaction(tx.clone(), &utxo, height, slot)?;
        }
        if let Some(ref ws) = *self.ws_sender.read().await {
            let _ = ws.send(rpc::WsEvent::NewTx {
                hash: tx.hash().to_hex(),
                tx_type: format!("{:?}", tx.tx_type).to_lowercase(),
                size: tx.size(),
                fee: 0,
            });
        }
        Ok(true)
    }

    /// Re-offer up to `REDELIVER_PER_TICK` ledger slashes at/above the slash
    /// gate judged at best+1; each re-admitted slash is broadcast once.
    pub(super) async fn redeliver_slashes(&self) {
        let mined_at = self.chain_state.read().await.best_height.saturating_add(1);
        let gate = self
            .config
            .network
            .params()
            .effective_slashing_activation_height();
        if mined_at < gate {
            return;
        }
        let batch = self
            .slash_reported
            .lock()
            .unwrap()
            .next_batch(REDELIVER_PER_TICK);
        if batch.is_empty() {
            return;
        }
        let settled: HashSet<PublicKey> = {
            let producers = self.producer_set.read().await;
            batch
                .iter()
                .map(|(pk, _)| *pk)
                .filter(|pk| producers.is_slashed_or_slash_queued(pk))
                .collect()
        };
        for (accused, tx) in batch {
            if settled.contains(&accused) {
                self.slash_reported.lock().unwrap().forget(&accused);
                continue;
            }
            match self.admit_own_slash(&accused, &tx).await {
                Ok(false) => {}
                Ok(true) => {
                    let permit = self.network.as_ref().and_then(|n| {
                        super::rpc_broadcast::reserve_tx_broadcast(
                            &n.command_sender(),
                            &self.origin_tx_bucket,
                        )
                    });
                    let hash = tx.hash();
                    match permit {
                        Some(send) => send(tx),
                        None => debug!("slash {} re-admitted; no broadcast slot", hash),
                    }
                    info!("slash {} for {} re-offered", hash, accused);
                }
                Err(e) if keeps_offer(&e) => {
                    debug!("slash for {} kept for re-offer: {}", accused, e);
                }
                Err(e) => {
                    debug!("slash for {} dropped from the ledger: {}", accused, e);
                    self.slash_reported.lock().unwrap().forget(&accused);
                }
            }
        }
    }
}
