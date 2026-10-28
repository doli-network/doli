use super::fork_walk::{cached_branches, select_walk_seed};
use super::*;

const CACHE_CAP: usize = 100;
const EVICT_BATCH: usize = 50;
const MAX_PINNED: usize = 50;
const MAX_DRAIN: u32 = 50;

/// Cached children of `parent` in drain order: longest cached descendant chain
/// first, then lower slot, then lower hash.
pub(super) fn drain_order(cache: &HashMap<Hash, Block>, parent: Hash) -> Vec<Hash> {
    let branches = cached_branches(cache);
    let mut kids: Vec<(std::cmp::Reverse<usize>, u32, Hash)> = cache
        .iter()
        .filter(|(_, b)| b.header.prev_hash == parent)
        .map(|(h, b)| (std::cmp::Reverse(branches[h].0), b.header.slot, *h))
        .collect();
    kids.sort();
    kids.into_iter().map(|(_, _, h)| h).collect()
}

/// The fork-cache gate. A body that does not match its header at `height` is not
/// cached. A different body cached under the hash is kept, unless `height` is
/// known and the cached body does not match its header there.
pub(super) fn admit(
    cache: &mut HashMap<Hash, Block>,
    ah: u64,
    height: Option<u64>,
    block: Block,
) -> bool {
    let hash = block.hash();
    if !block.body_matches_header(ah, height) {
        debug!(
            "[FORK_CACHE] Fork block {:.16} not cached: its body does not match its header (INC-I-281)",
            hash
        );
        return false;
    }
    let keep_cached = cache
        .get(&hash)
        .is_some_and(|c| *c != block && (height.is_none() || c.body_matches_header(ah, height)));
    if keep_cached {
        debug!(
            "[FORK_CACHE] Fork block {:.16} not cached: a different body is cached under its hash (INC-I-281)",
            hash
        );
        return false;
    }
    cache.insert(hash, block);
    true
}

/// The walk seed and its longest cached descendant chain, fork-nearest first,
/// capped at `MAX_PINNED`.
fn seed_chain(cache: &HashMap<Hash, Block>, is_canonical: impl Fn(&Hash) -> bool) -> HashSet<Hash> {
    let Some((seed, tip)) = select_walk_seed(cache, is_canonical) else {
        return HashSet::new();
    };
    let seed_hash = seed.hash();
    let mut path = Vec::new();
    let mut cur = tip;
    while let Some(b) = cache.get(&cur) {
        path.push(cur);
        if cur == seed_hash {
            break;
        }
        cur = b.header.prev_hash;
    }
    path.into_iter().rev().take(MAX_PINNED).collect()
}

impl Node {
    /// Insert a block into the fork cache through `admit`; false when it was not
    /// cached. Past `CACHE_CAP`, evict the `EVICT_BATCH` lowest-slot entries that
    /// are not on the walk-seed chain.
    pub(super) async fn cache_block_with_eviction(&self, hash: Hash, block: Block) -> bool {
        debug_assert_eq!(hash, block.hash());
        let height = self.door_position(&block).await.map(|(h, _)| h);
        let ah = self.inc_i_178_attestation_bls_activation_height;
        let mut cache = self.fork_block_cache.write().await;
        if !admit(&mut cache, ah, height, block) {
            return false;
        }
        if cache.len() <= CACHE_CAP {
            return true;
        }
        let pinned = seed_chain(&cache, |h| self.is_canonical(h));
        let mut by_slot: Vec<(u32, Hash)> = cache
            .iter()
            .filter(|(h, _)| !pinned.contains(*h))
            .map(|(h, b)| (b.header.slot, *h))
            .collect();
        by_slot.sort();
        for (_, h) in by_slot.into_iter().take(EVICT_BATCH) {
            cache.remove(&h);
        }
        true
    }

    /// Apply cached blocks that chain on the tip, in `drain_order`. A child
    /// that fails to apply is dropped and its next sibling is tried.
    pub(super) async fn drain_cached_children(&mut self, mode: ValidationMode) {
        let mut attempts = 0u32;
        'tip: while attempts < MAX_DRAIN {
            let tip_hash = self.chain_state.read().await.best_hash;
            let order = drain_order(&*self.fork_block_cache.read().await, tip_hash);
            for h in order {
                if attempts >= MAX_DRAIN {
                    break 'tip;
                }
                let Some(cached) = self.fork_block_cache.write().await.remove(&h) else {
                    continue;
                };
                if self.hold_removed_block(&cached).await {
                    let height = self.chain_state.read().await.best_height + 1;
                    let ah = self.inc_i_178_attestation_bls_activation_height;
                    admit(
                        &mut *self.fork_block_cache.write().await,
                        ah,
                        Some(height),
                        cached,
                    );
                    continue;
                }
                attempts += 1;
                info!(
                    "[ORPHAN_APPLY] Applying cached orphan {:.8} (slot {}) from fork cache [{}/{}]",
                    h, cached.header.slot, attempts, MAX_DRAIN
                );
                match self.apply_block(cached, mode).await {
                    Ok(()) => continue 'tip,
                    Err(e) => warn!("[ORPHAN_APPLY] Cached orphan {:.8} rejected: {}", h, e),
                }
            }
            break;
        }
    }

    /// INC-I-281: true if a non-fork-choice intake must not apply `block` (a
    /// child of the tip) now: a fork-remedy rollback removed it and a cached
    /// sibling would win fork choice against it. Decided by fork choice, not a
    /// timer. Fork choice does not call this.
    pub(super) async fn hold_removed_block(&self, block: &Block) -> bool {
        let hash = block.hash();
        if !self.sync_manager.read().await.removed_by_rollback(&hash) {
            return false;
        }
        let parent = block.header.prev_hash;
        let siblings: Vec<Hash> = {
            let cache = self.fork_block_cache.read().await;
            cache
                .iter()
                .filter(|(h, b)| **h != hash && b.header.prev_hash == parent)
                .map(|(h, _)| *h)
                .collect()
        };
        if siblings.is_empty() {
            return false;
        }
        // The rollback cleared the reorg handler: weigh the block as the wedge
        // escape weighs a sibling (parent weight + producer weight).
        let height = self.chain_state.read().await.best_height + 1;
        let weight = {
            let ps = self.producer_set.read().await;
            ps.get_by_pubkey(&block.header.producer)
                .map(|p| p.effective_weight(height))
                .unwrap_or(1)
        };
        let held = {
            let sync = self.sync_manager.read().await;
            let rh = sync.reorg_handler();
            let finality = sync.fork_choice_finality();
            let ours = rh.chain_weight(&parent).saturating_add(weight);
            siblings.iter().any(|s| {
                matches!(
                    rh.weigh_untracked(&hash, ours, s, finality),
                    network::WeightVerdict::Heavier | network::WeightVerdict::TieSwitch
                )
            })
        };
        if held
            && self
                .sync_manager
                .write()
                .await
                .note_removed_block_held(&hash)
        {
            info!(
                "[REAPPLY] Holding {:.8} (slot {}): removed by a fork-remedy rollback and a cached sibling wins fork choice; only fork choice may adopt a branch here (INC-I-281)",
                hash, block.header.slot
            );
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(prev: Hash, slot: u32) -> Block {
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: prev,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: slot as u64 * 10,
            slot,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        Block::new(header, vec![])
    }

    // OUTPUT CONTRACT: admit -> bool, cache entry under the hash.
    // INPUT PARTITIONS: junk body (known | unknown height) | first body | identical body |
    //   different passing body at unknown height | at a known height where the cached
    //   body passes | at a known height where the cached body fails.
    #[test]
    fn admit_refuses_junk_and_keeps_a_cached_body_that_matches_its_header() {
        let mut honest = blk(Hash::ZERO, 7);
        honest.header.merkle_root = doli_core::block::compute_merkle_root(&[]);
        let hash = honest.hash();
        let mut junk = honest.clone();
        junk.attestation_bitfield = vec![1];
        let mut other = honest.clone();
        other.aggregate_bls_signature = vec![9];
        let mut cache = HashMap::new();
        assert!(!admit(&mut cache, u64::MAX, Some(5), junk.clone()));
        assert!(!admit(&mut cache, u64::MAX, None, junk.clone()));
        assert!(cache.is_empty());
        assert!(admit(&mut cache, u64::MAX, None, honest.clone()));
        assert!(admit(&mut cache, u64::MAX, Some(5), honest.clone()));
        assert!(!admit(&mut cache, u64::MAX, None, other.clone()));
        assert!(!admit(&mut cache, u64::MAX, Some(5), other.clone()));
        assert_eq!(cache.get(&hash), Some(&honest));
        cache.insert(hash, junk);
        assert!(!admit(&mut cache, u64::MAX, None, honest.clone()));
        assert!(admit(&mut cache, u64::MAX, Some(5), honest.clone()));
        assert_eq!(cache.get(&hash), Some(&honest));
    }

    #[test]
    fn seed_chain_is_capped_and_starts_at_the_seed() {
        let stored = crypto::hash::hash(b"stored");
        let mut cache = HashMap::new();
        let mut prev = stored;
        let mut chain = Vec::new();
        for i in 0..(MAX_PINNED as u32 + 10) {
            let b = blk(prev, 100 + i);
            prev = b.hash();
            chain.push(prev);
            cache.insert(prev, b);
        }
        let pinned = seed_chain(&cache, |_| false);
        assert_eq!(pinned.len(), MAX_PINNED);
        assert!(chain[..MAX_PINNED].iter().all(|h| pinned.contains(h)));
        assert!(seed_chain(&HashMap::new(), |_| false).is_empty());
    }
}
