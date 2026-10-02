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
    /// Insert a block into the fork cache. Past `CACHE_CAP`, evict the
    /// `EVICT_BATCH` lowest-slot entries that are not on the walk-seed chain.
    pub(super) async fn cache_block_with_eviction(&self, hash: Hash, block: Block) {
        let mut cache = self.fork_block_cache.write().await;
        cache.insert(hash, block);
        if cache.len() <= CACHE_CAP {
            return;
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
