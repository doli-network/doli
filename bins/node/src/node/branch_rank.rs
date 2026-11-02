//! INC-I-289 N-3: the one rule that ranks cached branches for the hold, the walk
//! seed and the drain order. At/above the INC-I-288 gate a chain block..tip weighs
//! the bond of its distinct verified supporters (the doors' unit); below it, 1 per block.

use super::branch_support::SupportView;
use super::fork_walk::{cached_branches, BranchInfo};
use super::*;

/// How [`ranked_branches`] weighs a cached chain.
pub(super) enum BranchRank {
    /// 1 per block (below the gate).
    Depth,
    /// Bond of the chain's distinct supporters (at/above the gate).
    Bond(Box<RankViews>, Arc<BlockStore>),
}

/// One support view per epoch table, and the view each cached root's branch is
/// weighed with (the table of its own attach point); an unmapped root weighs 0.
pub(super) struct RankViews {
    views: Vec<SupportView>,
    by_root: HashMap<Hash, usize>,
}

impl RankViews {
    /// Adds a block the cache snapshot does not hold, in every view.
    pub(super) fn include(&mut self, block: &Block) {
        self.views.iter_mut().for_each(|v| v.include(block));
    }

    fn support(&self, store: &BlockStore, root: &Hash, h: &Hash) -> Option<network::BlockSupport> {
        self.views.get(*self.by_root.get(root)?)?.support(store, h)
    }
}

/// Weight and tip of the best cached chain under every cached block, by `rank`.
/// Ties go to the lower slot, then the lower hash, child by child.
pub(super) fn ranked_branches(
    cache: &HashMap<Hash, Block>,
    rank: &BranchRank,
) -> HashMap<Hash, BranchInfo> {
    match rank {
        BranchRank::Depth => cached_branches(cache, |_| 1),
        BranchRank::Bond(views, store) => bond_branches(cache, |r, h| views.support(store, r, h)),
    }
}

/// A set union is not additive, so each path block..tip under a block is weighed
/// whole. `support(root, block)`: `root` is the out-of-cache parent of the path.
fn bond_branches(
    cache: &HashMap<Hash, Block>,
    support: impl Fn(&Hash, &Hash) -> Option<network::BlockSupport>,
) -> HashMap<Hash, BranchInfo> {
    // A child apply's slot rule rejects (slot not above its cached parent's) does not
    // end its parent's chain: the parent stays a tip; the child's own path weighs 0.
    let advances = |b: &Block| {
        let parent_slot = cache.get(&b.header.prev_hash).map(|p| p.header.slot);
        let s = network::BlockSupport {
            slot: Some(b.header.slot),
            parent_slot,
            ..Default::default()
        };
        s.advances_slot()
    };
    let parents: HashSet<Hash> = (cache.values().filter(|b| advances(b)))
        .map(|b| b.header.prev_hash)
        .collect();
    let key = |h: &Hash| (cache[h].header.slot, *h);
    let mut best: HashMap<Hash, (u64, Hash, Vec<Hash>)> = HashMap::with_capacity(cache.len());
    for tip in cache.keys().filter(|h| !parents.contains(*h)) {
        let mut path = vec![*tip];
        while let Some(prev) = (cache.get(&path[path.len() - 1]))
            .map(|b| b.header.prev_hash)
            .filter(|p| cache.contains_key(p) && path.len() <= cache.len())
        {
            path.push(prev);
        }
        path.reverse();
        let root = cache[&path[0]].header.prev_hash;
        for i in 0..path.len() {
            let w = network::candidate_union(&path[i..], |h| support(&root, h));
            let rest = &path[i + 1..];
            let wins = best.get(&path[i]).is_none_or(|(bw, _, brest)| {
                let tie = || brest.iter().map(key).cmp(rest.iter().map(key));
                w.cmp(bw).then_with(tie) == std::cmp::Ordering::Greater
            });
            if wins {
                best.insert(path[i], (w, *tip, rest.to_vec()));
            }
        }
    }
    best.into_iter()
        .map(|(h, (w, tip, _))| (h, (w, tip)))
        .collect()
}

impl Node {
    /// The rank the doors' gate selects: [`BranchRank::Bond`] at/above the INC-I-288
    /// gate (the doors' predicate), else [`BranchRank::Depth`].
    pub(super) async fn branch_rank(&self) -> BranchRank {
        let above = {
            let sync = self.sync_manager.read().await;
            let tip_h = sync.fork_choice_finality().local_tip_height;
            sync.reorg_handler().weighs_from_ancestor(tip_h)
        };
        if above {
            BranchRank::Bond(
                Box::new(self.rank_views().await),
                Arc::clone(&self.block_store),
            )
        } else {
            BranchRank::Depth
        }
    }

    /// One snapshot, re-weighed per table of [`Self::rank_tables`].
    async fn rank_views(&self) -> RankViews {
        let (by_root, tables) = self.rank_tables().await;
        let mut tables = tables.into_iter();
        let mut views = Vec::new();
        if let Some(first) = tables.next() {
            let first = self.support_view(first).await;
            let rest: Vec<SupportView> = tables.map(|t| first.with_table(t)).collect();
            views.push(first);
            views.extend(rest);
        }
        RankViews { views, by_root }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use network::BlockSupport;

    fn blk(prev: Hash, slot: u32, by: u8) -> Block {
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: prev,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: slot as u64 * 10,
            slot,
            producer: crypto::PublicKey::from_bytes([by; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        Block::new(header, vec![])
    }

    // OUTPUT CONTRACT: bond_branches -> (weight, tip) per cached block.
    // INPUT PARTITIONS: two chains on one root: 2 blocks by 2 producers vs 4 blocks by
    //   1 producer | a tie in bond (lower child slot wins) | a leaf (itself, own bond).
    #[test]
    fn bond_branches_weigh_distinct_producers_not_blocks() {
        let root = blk(crypto::hash::hash(b"stored"), 1, 9);
        let s1 = blk(root.hash(), 2, 1);
        let s2 = blk(s1.hash(), 3, 2);
        let mut l = vec![blk(root.hash(), 4, 3)];
        for slot in 5..8 {
            l.push(blk(l[l.len() - 1].hash(), slot, 3));
        }
        let mut cache: HashMap<Hash, Block> = HashMap::new();
        for b in [&root, &s1, &s2].into_iter().chain(&l) {
            cache.insert(b.hash(), b.clone());
        }
        let support = |_: &Hash, h: &Hash| {
            let b = cache.get(h)?;
            Some(BlockSupport::single(*b.header.producer.as_bytes(), 1))
        };
        let best = bond_branches(&cache, support);
        assert_eq!(best[&root.hash()], (3, s2.hash()));
        assert_eq!(best[&s1.hash()], (2, s2.hash()));
        assert_eq!(best[&l[0].hash()], (1, l[3].hash()));
        assert_eq!(best[&s2.hash()], (1, s2.hash()));
        assert_eq!(
            cached_branches(&cache, |_| 1)[&root.hash()],
            (5, l[3].hash())
        );

        let t = blk(root.hash(), 3, 2);
        let t2 = blk(t.hash(), 4, 1);
        cache.insert(t.hash(), t.clone());
        cache.insert(t2.hash(), t2.clone());
        let tied = bond_branches(&cache, |_: &Hash, h: &Hash| {
            Some(BlockSupport::single(
                *cache.get(h)?.header.producer.as_bytes(),
                1,
            ))
        });
        assert_eq!(tied[&t.hash()], (2, t2.hash()));
        assert_eq!(
            tied[&root.hash()],
            (3, s2.hash()),
            "S1 (slot 2) beats T (slot 3)"
        );
    }
}
