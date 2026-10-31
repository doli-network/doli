use super::branch_rank::{ranked_branches, BranchRank};
use super::*;

// INC-I-235: start, gate and bound the by-hash parent walk that lets a node on a
// losing sibling reach the competing branch's fork point and leave by reorg.

/// (weight of the cached chain rooted at a block, tip of that chain)
pub(super) type BranchInfo = (u64, Hash);

/// Weight and tip of the heaviest cached descendant chain under every cached
/// block, each block adding `weight(h)` (1 per block: depth). Heavier wins; ties
/// go to the lower slot, then the lower hash.
pub(super) fn cached_branches(
    cache: &HashMap<Hash, Block>,
    weight: impl Fn(&Hash) -> u64,
) -> HashMap<Hash, BranchInfo> {
    let mut children: HashMap<Hash, Vec<Hash>> = HashMap::new();
    for (h, b) in cache {
        children.entry(b.header.prev_hash).or_default().push(*h);
    }
    let rank =
        |h: &Hash, info: &BranchInfo| (info.0, std::cmp::Reverse((cache[h].header.slot, *h)));
    let mut memo: HashMap<Hash, BranchInfo> = HashMap::with_capacity(cache.len());
    for start in cache.keys() {
        let mut stack = vec![(*start, false)];
        while let Some((h, expanded)) = stack.pop() {
            if memo.contains_key(&h) {
                continue;
            }
            let kids = children.get(&h).map(Vec::as_slice).unwrap_or(&[]);
            if !expanded {
                stack.push((h, true));
                stack.extend(
                    kids.iter()
                        .filter(|k| !memo.contains_key(*k))
                        .map(|k| (*k, false)),
                );
                continue;
            }
            let best = kids
                .iter()
                .filter_map(|k| memo.get(k).map(|info| (k, *info)))
                .max_by_key(|(k, info)| rank(k, info));
            let w = weight(&h);
            memo.insert(
                h,
                best.map_or((w, h), |(_, (d, tip))| (d.saturating_add(w), tip)),
            );
        }
    }
    memo
}

/// Walk seeds in rank order: cached orphans whose parent is NOT canonical, by the
/// [`ranked_branches`] weight they root (lowest slot, then hash, on ties).
/// Returns (seed, branch tip) hashes.
pub(super) fn walk_seeds(
    cache: &HashMap<Hash, Block>,
    is_stored: impl Fn(&Hash) -> bool,
    rank: &BranchRank,
) -> Vec<(Hash, Hash)> {
    let branches = ranked_branches(cache, rank);
    let mut seeds: Vec<_> = cache
        .iter()
        .filter(|(_, b)| !is_stored(&b.header.prev_hash))
        .map(|(h, b)| {
            let (w, tip) = branches[h];
            ((std::cmp::Reverse(w), b.header.slot, *h), tip)
        })
        .collect();
    seeds.sort();
    seeds.into_iter().map(|((_, _, h), tip)| (h, tip)).collect()
}

/// The first of [`walk_seeds`], with its body.
pub(super) fn select_walk_seed(
    cache: &HashMap<Hash, Block>,
    is_stored: impl Fn(&Hash) -> bool,
    rank: &BranchRank,
) -> Option<(Block, Hash)> {
    let (seed, tip) = walk_seeds(cache, is_stored, rank).into_iter().next()?;
    Some((cache[&seed].clone(), tip))
}

/// Tip of the cached chain that descends from `from` (itself when it has none).
pub(super) fn cached_branch_tip(
    cache: &HashMap<Hash, Block>,
    from: Hash,
    rank: &BranchRank,
) -> Hash {
    ranked_branches(cache, rank)
        .get(&from)
        .map_or(from, |(_, tip)| *tip)
}

impl Node {
    /// True when `h` is on our canonical chain (a rolled-back body is not).
    pub(super) fn is_canonical(&self, h: &Hash) -> bool {
        matches!(self.block_store.get_height_by_hash(h), Ok(Some(_)))
    }

    /// Start a fork-recovery parent walk from the cached competing branch.
    /// Called from the ReorgCandidate gossip path and the Wedged terminal.
    /// Returns true when a walk was started.
    pub async fn try_trigger_fork_recovery(&mut self) -> bool {
        self.try_trigger_fork_recovery_from(None).await
    }

    /// INC-I-293 103: `sender` (the orphan's) is asked when connected, else a peer
    /// whose reported tip is off our chain; no sender keeps the tallest peer.
    pub(super) async fn try_trigger_fork_recovery_from(&mut self, sender: Option<PeerId>) -> bool {
        if !self.sync_manager.read().await.can_start_fork_recovery() {
            return false;
        }
        let local_tip = self.chain_state.read().await.best_hash;
        let rank = self.branch_rank().await;
        let seeds = {
            let cache = self.fork_block_cache.read().await;
            walk_seeds(&cache, |h| self.is_canonical(h), &rank)
        };
        // INC-I-288: at/above the gate a rejected branch passes the walk to the next seed.
        let tries = match rank {
            BranchRank::Bond(..) => seeds.len(),
            BranchRank::Depth => 1,
        };
        let picked = {
            let sm = self.sync_manager.read().await;
            seeds.into_iter().take(tries).find(|(_, tip)| {
                let rejected = sm.is_fork_branch_rejected(*tip, local_tip);
                if rejected {
                    debug!(
                        "[FORK] RECOVERY_SKIP branch tip {:.16} already lost fork choice on {:.16}",
                        tip, local_tip
                    );
                }
                !rejected
            })
        };
        let Some((seed_hash, branch_tip)) = picked else {
            return false;
        };
        let Some(seed) = self.fork_block_cache.read().await.get(&seed_hash).cloned() else {
            return false;
        };
        let mut sm = self.sync_manager.write().await;
        let peer = match sender {
            None => sm.best_peer_for_recovery(),
            Some(s) if sm.has_peer(&s) => Some(s),
            Some(s) => self.branch_peer(&sm, &s, None),
        };
        let Some(peer) = peer else {
            return false;
        };
        let seed_hash = seed.hash();
        let seed_slot = seed.header.slot;
        let started = sm.start_fork_recovery(seed, peer);
        if started {
            info!(
                "[FORK] RECOVERY_START seed {:.16} slot {} (branch tip {:.16}) from {}",
                seed_hash, seed_slot, branch_tip, peer
            );
        }
        started
    }

    /// Handle a completed fork recovery: eligibility-gate every walked block
    /// against the epoch-frozen schedule, then fork choice. A walk that does not
    /// move the tip is remembered so the same branch is not re-walked (LB-9).
    pub async fn handle_completed_fork_recovery(
        &mut self,
        recovery: network::sync::CompletedRecovery,
    ) -> Result<()> {
        let Some(fork_tip) = recovery.blocks.last().map(|b| b.hash()) else {
            return Ok(());
        };
        // INC-I-293 R1: at/above the gate a walk whose tip is already canonical (another
        // path adopted its branch first) is no fork. Weighed against our own chain it
        // ties on support, and the lower-hash rule would roll back every block above it.
        if self.is_canonical(&fork_tip) && self.weighs_by_support().await {
            info!(
                "[FORK] RECOVERY_DONE fork tip {:.16}: already canonical — no reorg",
                fork_tip
            );
            return Ok(());
        }
        let tip_before = self.chain_state.read().await.best_hash;
        let mut result = Ok(());
        let mut eligible = true;
        let fork_base = self
            .block_store
            .get_height_by_hash(&recovery.connection_point)
            .ok()
            .flatten();
        let junk = self.drop_junk_bodies(&recovery.blocks, fork_base).await;
        self.walk_bodies_failed(&recovery, &junk, fork_base).await;
        let junk_body = !junk.is_empty();
        let body_refusals = self.refused_reorg_blocks.body_refusals();
        let replayed = self.walk_replays_refused(&recovery, fork_base).await;
        let mut anchor = None;
        for (offset, block) in recovery.blocks.iter().enumerate().filter(|_| !replayed) {
            let height = fork_base.map(|h| h.saturating_add(offset as u64 + 1));
            if let Err(e) = self
                .check_producer_eligibility_at(block, height, fork_base, anchor)
                .await
            {
                warn!(
                    "[FORK] Walked block {:.16} at slot {} is ineligible ({}) — branch not reorged onto",
                    block.hash(),
                    block.header.slot,
                    e
                );
                eligible = false;
                break;
            }
            anchor = self
                .refused_reorg_blocks
                .anchor_of(block, height)
                .or(anchor);
        }
        if eligible && !junk_body && !replayed {
            result = self.evaluate_recovered_fork(recovery).await;
        }
        let body_refused = self.refused_reorg_blocks.body_refusals() != body_refusals;
        let tip_after = self.chain_state.read().await.best_hash;
        if tip_after == tip_before {
            let reason = match (&result, eligible) {
                (_, false) => "ineligible walked block".to_string(),
                (Err(e), _) => format!("reorg failed: {}", e),
                _ if junk_body => "a walked body does not match its header".to_string(),
                _ if body_refused => "a fork body was refused".to_string(),
                _ => "not heavier or refused by finality".to_string(),
            };
            warn!(
                "[FORK] RECOVERY_DONE fork tip {:.16}: no reorg ({})",
                fork_tip, reason
            );
            let body_level = junk_body || body_refused;
            if !body_level && !super::header_auth::is_fork_signature_rejection(&result) {
                let rank = self.branch_rank().await;
                let branch_tip = {
                    let cache = self.fork_block_cache.read().await;
                    cached_branch_tip(&cache, fork_tip, &rank)
                };
                self.sync_manager
                    .write()
                    .await
                    .mark_fork_branch_rejected(branch_tip, tip_before);
            }
        } else {
            info!(
                "[FORK] RECOVERY_DONE fork tip {:.16}: reorged, tip {:.16} -> {:.16}",
                fork_tip, tip_before, tip_after
            );
        }
        result
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
    fn seed_is_root_of_longest_cached_chain_with_missing_parent() {
        let stored = crypto::hash::hash(b"stored");
        let a = blk(stored, 100);
        let a1 = blk(a.hash(), 103);
        let a2 = blk(a1.hash(), 112);
        let stale = blk(crypto::hash::hash(b"stale"), 2);
        let mut cache = HashMap::new();
        for b in [&a, &a1, &a2, &stale] {
            cache.insert(b.hash(), b.clone());
        }
        let (seed, tip) = select_walk_seed(&cache, |h| *h == stored, &BranchRank::Depth).unwrap();
        assert_eq!(seed.hash(), a1.hash());
        assert_eq!(tip, a2.hash());
        assert_eq!(
            cached_branch_tip(&cache, a1.hash(), &BranchRank::Depth),
            a2.hash()
        );
        assert_eq!(
            cached_branch_tip(&cache, stale.hash(), &BranchRank::Depth),
            stale.hash()
        );
    }

    #[test]
    fn no_seed_when_every_cached_parent_is_stored() {
        let stored = crypto::hash::hash(b"stored");
        let a = blk(stored, 100);
        let cache = HashMap::from([(a.hash(), a)]);
        assert!(select_walk_seed(&cache, |h| *h == stored, &BranchRank::Depth).is_none());
    }
}
