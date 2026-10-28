//! Walked fork bodies: cached at their heights; judged by commitments and by
//! refusal memory as body failures of the walk peers.

use super::*;

impl Node {
    /// Caches walked `recovery.blocks` at their heights above the connection
    /// point. A walked copy replaces a different cached copy, unless its supplier
    /// failed that hash: then an empty slot is a body refusal and a refetch.
    pub(in crate::node) async fn cache_walked(
        &mut self,
        recovery: &network::sync::CompletedRecovery,
    ) {
        let base = self
            .block_store
            .get_height_by_hash(&recovery.connection_point)
            .ok()
            .flatten();
        let ah = self.inc_i_178_attestation_bls_activation_height;
        let mut cache = self.fork_block_cache.write().await;
        let mut missing = Vec::new();
        for (i, block) in recovery.blocks.iter().enumerate() {
            let hash = block.hash();
            let height = base.map(|h| h.saturating_add(i as u64 + 1));
            let supplier = recovery.suppliers.get(i).copied().flatten();
            if supplier.is_some_and(|p| self.body_retry.has_failed(&hash, &p)) {
                if !cache.contains_key(&hash) {
                    missing.push((hash, height));
                }
                continue;
            }
            if cache.get(&hash).is_some_and(|c| c != block) {
                cache.remove(&hash);
            }
            let cached = crate::node::fork_cache::admit(&mut cache, ah, height, block.clone());
            if let Some(p) = supplier.filter(|_| cached) {
                let key = content_key(block);
                self.body_retry
                    .note_supplier(hash, key, p, block.header.slot);
            }
        }
        drop(cache);
        for (hash, height) in missing {
            self.refused_reorg_blocks.note_body_refusal();
            self.refetch_body(hash, height, true).await;
        }
    }

    /// Walked bodies that fail their header commitments are junk of the walk peers.
    pub(in crate::node) async fn walk_bodies_failed(
        &mut self,
        recovery: &network::sync::CompletedRecovery,
        junk: &[Hash],
        base: Option<u64>,
    ) {
        let faults = faults_of(
            &recovery.blocks,
            junk,
            base,
            &recovery.suppliers,
            Fault::Junk,
        );
        for fault in faults {
            self.body_failed(fault).await;
        }
    }

    /// Walked copies whose exact content a reorg refused: body failures of their
    /// walk peers and one counted body refusal. True when there was one.
    pub(in crate::node) async fn walk_replays_refused(
        &mut self,
        recovery: &network::sync::CompletedRecovery,
        base: Option<u64>,
    ) -> bool {
        let hits: Vec<Hash> = recovery
            .blocks
            .iter()
            .filter(|b| self.refused_reorg_blocks.content_refused(b))
            .map(Block::hash)
            .collect();
        let Some(first) = hits.first() else {
            return false;
        };
        warn!(
            "[FORK] Walked block {:.16} carries a body a reorg refused before — dropped, its header and branch stay open (INC-I-281)",
            first
        );
        self.refused_reorg_blocks.note_body_refusal();
        let faults = faults_of(
            &recovery.blocks,
            &hits,
            base,
            &recovery.suppliers,
            Fault::Failed,
        );
        for fault in faults {
            self.body_failed(fault).await;
        }
        true
    }
}
