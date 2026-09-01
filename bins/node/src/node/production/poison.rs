use super::*;

impl Node {
    /// Contain a self-produced block that `apply_block` rejected
    /// (INC-I-204 M4.2 / REQ-FORK-002).
    ///
    /// `apply_block` runs before broadcast, so the failed block was never
    /// gossiped. Only a failure AT the local tip may rewind; anything else keeps
    /// the published tip and discards the aborted apply's in-memory residue.
    pub async fn handle_failed_self_apply(
        &mut self,
        block: &Block,
        height: u64,
        err: anyhow::Error,
    ) -> Result<()> {
        warn!(
            "[BLOCK_POISON] apply_block failed on self-produced block at h={}: {}. \
             Purging its TXs and asking the rollback door for authority.",
            height, err
        );

        // Purge FIRST so every path below leaves a clean mempool — including the
        // one where the rewind errors out and the poison would otherwise re-fire
        // on the next slot.
        {
            let mut mempool = self.mempool.write().await;
            let before = mempool.len();
            for tx in &block.transactions {
                mempool.remove_transaction(&tx.hash());
            }
            let after = mempool.len();
            if before != after {
                warn!(
                    "[BLOCK_POISON] Purged {} TXs from mempool (block had {} TXs). \
                     Next tick will build a clean block.",
                    before - after,
                    block.transactions.len()
                );
            }
        }

        match self
            .rollback_one_block(RollbackAuthority::ProductionSelfApply {
                failed_height: height,
            })
            .await
        {
            Ok(RollbackOutcome::RolledBack) => {
                info!(
                    "[BLOCK_POISON] Rolled back h={} — the failed block WAS the local tip",
                    height
                );
                crate::metrics::POISON_CONTAINMENT
                    .with_label_values(&["rolled_back"])
                    .inc();
                Ok(())
            }
            Ok(outcome) => {
                self.resync_after_aborted_apply().await;
                let kept = self.sync_manager.read().await.local_tip().0;
                warn!(
                    "[BLOCK_POISON] Tip kept at h={} ({:?}) — no published state retracted",
                    kept, outcome
                );
                crate::metrics::POISON_CONTAINMENT
                    .with_label_values(&["tip_kept"])
                    .inc();
                Ok(())
            }
            Err(rb_err) => {
                error!(
                    "[BLOCK_POISON] Rollback failed: {}. Manual intervention needed.",
                    rb_err
                );
                self.resync_after_aborted_apply().await;
                crate::metrics::POISON_CONTAINMENT
                    .with_label_values(&["rollback_failed"])
                    .inc();
                Err(err)
            }
        }
    }

    /// Discard the in-memory residue an aborted `apply_block` may have left in
    /// the `ProducerSet` (tx loop) and `chain_state` before it failed.
    ///
    /// Nothing durable was written on the paths that reach here — the write batch
    /// was dropped uncommitted — so the committed StateDb IS the state at the kept
    /// tip. These are the same authoritative sources `Node::new()` reads at startup.
    async fn resync_after_aborted_apply(&mut self) {
        let kept_tip = self.sync_manager.read().await.local_tip().0;

        if let Some(durable) = self.state_db.get_chain_state() {
            let mut state = self.chain_state.write().await;
            *state = durable;
        }
        // Non-empty guard, copied from the startup loader this mirrors
        // (`init.rs:621`). A node that has not yet committed a block holds its
        // genesis producers IN MEMORY ONLY — no `write_producer_set` runs on the
        // `Node::new()` path — so an unguarded adopt would blank a populated set
        // during the genesis phase.
        let reloaded = self.state_db.load_producer_set();
        if reloaded.active_count() > 0 {
            let mut producers = self.producer_set.write().await;
            *producers = reloaded;
        }
        self.rebuild_producer_liveness(kept_tip);

        info!(
            "[BLOCK_POISON] Resynced in-memory chain state and producer set from StateDb at h={}",
            kept_tip
        );
    }
}
