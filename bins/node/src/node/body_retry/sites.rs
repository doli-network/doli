//! Peer bodies applied on the tip, and sync bodies the network refused.

use super::*;

impl Node {
    /// Applies a peer or cached body on the tip; a failure is a body failure of
    /// its supplier (the intake peer, else the recorded one).
    pub(in crate::node) async fn apply_peer_body(
        &mut self,
        block: Block,
        mode: ValidationMode,
    ) -> Result<()> {
        let held = block.clone();
        let result = self.apply_block(block, mode).await;
        if result.is_err() {
            let height = Some(self.chain_state.read().await.best_height.saturating_add(1));
            let ah = self.inc_i_178_attestation_bls_activation_height;
            let kind = if held.body_matches_header(ah, height) {
                Fault::Failed
            } else {
                Fault::Junk
            };
            let supplier = self.intake_supplier(&held.hash());
            self.body_failed(BodyFault::of(&held, height, supplier, kind))
                .await;
        }
        result
    }

    /// Records the suppliers of sync bodies the network refused on content.
    pub(in crate::node) async fn penalize_refused_sync_bodies(&mut self) {
        let refused = self
            .sync_manager
            .write()
            .await
            .take_refused_body_suppliers();
        for (peer, slot) in refused {
            self.bls_ingress_scorer.record_invalid_block(&peer, slot);
        }
    }
}
