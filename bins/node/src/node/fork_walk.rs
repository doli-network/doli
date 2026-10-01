use super::*;

// INC-I-235: start, gate and bound the by-hash parent walk that lets a node on a
// losing sibling reach the competing branch's fork point and leave by reorg.

/// (depth of the cached chain rooted at a block, tip of that chain)
type BranchInfo = (usize, Hash);

/// Depth and tip of the cached descendant chain under every cached block.
/// Deeper wins; ties go to the lower slot, then the lower hash.
fn cached_branches(cache: &HashMap<Hash, Block>) -> HashMap<Hash, BranchInfo> {
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
            memo.insert(h, best.map_or((1, h), |(_, (d, tip))| (d + 1, tip)));
        }
    }
    memo
}

/// Walk seed: the cached orphan whose parent is NOT stored and which roots the
/// longest cached chain (lowest slot on ties). Returns (seed, branch tip).
pub(super) fn select_walk_seed(
    cache: &HashMap<Hash, Block>,
    is_stored: impl Fn(&Hash) -> bool,
) -> Option<(Block, Hash)> {
    let branches = cached_branches(cache);
    cache
        .iter()
        .filter(|(_, b)| !is_stored(&b.header.prev_hash))
        .max_by_key(|(h, b)| {
            let (depth, _) = branches[*h];
            (depth, std::cmp::Reverse((b.header.slot, **h)))
        })
        .map(|(h, b)| (b.clone(), branches[h].1))
}

/// Tip of the cached chain that descends from `from` (itself when it has none).
fn cached_branch_tip(cache: &HashMap<Hash, Block>, from: Hash) -> Hash {
    cached_branches(cache)
        .get(&from)
        .map_or(from, |(_, tip)| *tip)
}

impl Node {
    /// Start a fork-recovery parent walk from the cached competing branch.
    /// Called from the production gate and the Wedged terminal.
    pub async fn try_trigger_fork_recovery(&mut self) {
        if !self.sync_manager.read().await.can_start_fork_recovery() {
            return;
        }
        let local_tip = self.chain_state.read().await.best_hash;
        let picked = {
            let cache = self.fork_block_cache.read().await;
            select_walk_seed(&cache, |h| self.block_store.has_block(h).unwrap_or(false))
        };
        let Some((seed, branch_tip)) = picked else {
            return;
        };
        let mut sm = self.sync_manager.write().await;
        if sm.is_fork_branch_rejected(branch_tip, local_tip) {
            debug!(
                "[FORK] RECOVERY_SKIP branch tip {:.16} already lost fork choice on {:.16}",
                branch_tip, local_tip
            );
            return;
        }
        let Some(peer) = sm.best_peer_for_recovery() else {
            return;
        };
        let seed_hash = seed.hash();
        if sm.start_fork_recovery(seed, peer) {
            info!(
                "[FORK] RECOVERY_START seed {:.16} (branch tip {:.16}) from {}",
                seed_hash, branch_tip, peer
            );
        }
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
        let tip_before = self.chain_state.read().await.best_hash;
        let mut result = Ok(());
        let mut eligible = true;
        for block in &recovery.blocks {
            if let Err(e) = self.check_producer_eligibility(block).await {
                warn!(
                    "[FORK] Walked block {:.16} at slot {} is ineligible ({}) — branch not reorged onto",
                    block.hash(),
                    block.header.slot,
                    e
                );
                eligible = false;
                break;
            }
        }
        if eligible {
            result = self.evaluate_recovered_fork(recovery).await;
        }
        if self.chain_state.read().await.best_hash == tip_before {
            let branch_tip = {
                let cache = self.fork_block_cache.read().await;
                cached_branch_tip(&cache, fork_tip)
            };
            self.sync_manager
                .write()
                .await
                .mark_fork_branch_rejected(branch_tip, tip_before);
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
        let (seed, tip) = select_walk_seed(&cache, |h| *h == stored).unwrap();
        assert_eq!(seed.hash(), a1.hash());
        assert_eq!(tip, a2.hash());
        assert_eq!(cached_branch_tip(&cache, a1.hash()), a2.hash());
        assert_eq!(cached_branch_tip(&cache, stale.hash()), stale.hash());
    }

    #[test]
    fn no_seed_when_every_cached_parent_is_stored() {
        let stored = crypto::hash::hash(b"stored");
        let a = blk(stored, 100);
        let cache = HashMap::from([(a.hash(), a)]);
        assert!(select_walk_seed(&cache, |h| *h == stored).is_none());
    }
}
