//! A peer fork body is checked against every header commitment (merkle root and
//! presence root) before the node caches or uses it. A body-level refusal drops
//! that body only: its header and branch stay open.

use super::reorg_refusals::{content_key, Refusal};
use super::*;

/// Indices of `blocks`, at heights `base + 1 + i`, whose body does not match its header.
fn junk_indices(blocks: &[Block], ah: u64, base: Option<u64>) -> Vec<usize> {
    let height = |i: usize| base.map(|h| h.saturating_add(i as u64 + 1));
    (0..blocks.len())
        .filter(|&i| !blocks[i].body_matches_header(ah, height(i)))
        .collect()
}

impl Node {
    /// Hashes of `blocks` (at `base + 1 + i`) whose body does not match its header;
    /// a cached copy that fails at that height is dropped too.
    pub(super) async fn drop_junk_bodies(&self, blocks: &[Block], base: Option<u64>) -> Vec<Hash> {
        let ah = self.inc_i_178_attestation_bls_activation_height;
        let junk = junk_indices(blocks, ah, base);
        if junk.is_empty() {
            return Vec::new();
        }
        let mut cache = self.fork_block_cache.write().await;
        let mut hashes = Vec::with_capacity(junk.len());
        for i in junk {
            let hash = blocks[i].hash();
            let height = base.map(|h| h.saturating_add(i as u64 + 1));
            if cache
                .get(&hash)
                .is_some_and(|c| !c.body_matches_header(ah, height))
            {
                cache.remove(&hash);
            }
            hashes.push(hash);
        }
        hashes
    }

    /// Drops the cached copy of `hash` when its content key is `key`.
    pub(super) async fn evict_cached_body(&self, hash: &Hash, key: Hash) {
        let mut cache = self.fork_block_cache.write().await;
        if cache.get(hash).is_some_and(|c| content_key(c) == key) {
            cache.remove(hash);
        }
    }

    /// For an apply failure: drops that cached body and counts a body refusal.
    pub(super) async fn refused_body(&mut self, refusal: Refusal, hash: &Hash) -> bool {
        let Refusal::ApplyFailed(key) = refusal else {
            return false;
        };
        self.evict_cached_body(hash, key).await;
        self.refused_reorg_blocks.note_body_refusal();
        true
    }

    /// Light apply of a walked direct successor; a failure drops its cached copy
    /// and counts a body refusal.
    pub(super) async fn apply_fork_successor(&mut self, block: &Block) -> Result<()> {
        let result = self.apply_block(block.clone(), ValidationMode::Light).await;
        if result.is_err() {
            self.evict_cached_body(&block.hash(), content_key(block))
                .await;
            self.refused_reorg_blocks.note_body_refusal();
        }
        result
    }

    /// Caches walked `blocks` through the fork-cache gate at their heights above
    /// `connection_point`.
    pub(super) async fn cache_walked(&self, blocks: &[Block], connection_point: &Hash) {
        let base = self
            .block_store
            .get_height_by_hash(connection_point)
            .ok()
            .flatten();
        let ah = self.inc_i_178_attestation_bls_activation_height;
        let mut cache = self.fork_block_cache.write().await;
        for (i, block) in blocks.iter().enumerate() {
            let height = base.map(|h| h.saturating_add(i as u64 + 1));
            super::fork_cache::admit(&mut cache, ah, height, block.clone());
        }
    }

    /// The body `handle_new_block` evaluates: `None` for a known block; the
    /// stored body for a stored, rolled-back block (INC-I-147 D4); else `block`.
    pub(super) async fn intake_body(&self, block: Block) -> Result<Option<Block>> {
        let hash = block.hash();
        let Some(stored) = self.block_store.get_block(&hash)? else {
            return Ok(Some(block));
        };
        if self.block_store.get_height_by_hash(&hash)?.is_some() {
            return Ok(None);
        }
        let ah = self.config.network.params().inc_i_147_activation_height;
        if self.chain_state.read().await.best_height < ah {
            return Ok(None);
        }
        info!(
            "[REAPPLY] block {} has a body but no canonical entry (rolled back) — re-evaluating instead of short-circuiting (INC-I-147 D4)",
            hash
        );
        Ok(Some(stored))
    }
}
