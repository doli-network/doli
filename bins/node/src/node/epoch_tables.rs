//! INC-I-289 L-2: the bounded store of epoch tables the reorg doors weigh with.

use std::collections::VecDeque;
use std::sync::Mutex;

use storage::EpochTable;

use super::*;

/// Ancestors whose refusal was already logged (one line per ancestor hash).
static LOGGED: Mutex<VecDeque<Hash>> = Mutex::new(VecDeque::new());
const LOGGED_CAP: usize = 64;

/// Kept: every epoch within `MAX_REORG_DEPTH` of the tip plus one epoch, at least this many.
pub(super) const MIN_EPOCH_TABLES: usize = 4;

impl Node {
    /// Capture the table after a boundary block (or an epoch-0 block) commits.
    pub(super) async fn capture_epoch_table(&self, height: u64, hash: Hash) {
        let bpe = self.config.network.blocks_per_reward_epoch();
        let at_boundary = doli_core::EpochSnapshot::is_epoch_boundary_with(height, bpe);
        if !(at_boundary || height < bpe) {
            return self.seed_tip_epoch_table().await;
        }
        let mut table = self.live_epoch_table(height - height % bpe).await;
        table.boundary_hash = at_boundary.then_some(hash);
        self.state_db.put_epoch_table(&table);
        let window = network::MAX_REORG_DEPTH as u64 + bpe;
        let keep_from = (height + 1).saturating_sub(window);
        self.state_db
            .prune_epoch_tables(keep_from, MIN_EPOCH_TABLES);
    }

    /// A tip epoch without a stored table (first start of this build, a boundary put
    /// lost to a crash) gets the live one, keyed as a capture keys it.
    pub(super) async fn seed_tip_epoch_table(&self) {
        let tip = self.chain_state.read().await.best_height;
        let start = tip - tip % self.config.network.blocks_per_reward_epoch();
        if self.state_db.get_epoch_table(start).is_some() {
            return;
        }
        let mut table = self.live_epoch_table(start).await;
        table.boundary_hash = match start {
            0 => None,
            s => self.block_store.get_hash_by_height(s).ok().flatten(),
        };
        self.state_db.put_epoch_table(&table);
        info!("EPOCH_TABLE: seeded the live table of tip epoch {start} (h={tip})");
    }

    /// Seed the store from an installed snapshot: only the anchor's epoch.
    pub(super) async fn seed_epoch_table_after_snap(&self, anchor: u64, hash: Hash) {
        let bpe = self.config.network.blocks_per_reward_epoch();
        let start = anchor - anchor % bpe;
        let mut table = self.live_epoch_table(start).await;
        table.boundary_hash = (start == anchor && anchor > 0).then_some(hash);
        self.state_db.clear_epoch_tables();
        self.state_db.put_epoch_table(&table);
    }

    /// A rolled-back boundary (or epoch-0) block takes its table with it.
    pub(super) fn drop_epoch_table_on_rollback(&self, height: u64) {
        let bpe = self.config.network.blocks_per_reward_epoch();
        if doli_core::EpochSnapshot::is_epoch_boundary_with(height, bpe) || height < bpe {
            self.state_db.delete_epoch_table(height - height % bpe);
        }
    }

    /// The table of the epoch holding the common ancestor: the stored one, else the
    /// live one when that epoch is the tip's. `None` = refuse the reorg (never the
    /// tip table for an older epoch).
    pub(super) async fn ancestor_table(&self, ancestor: (u64, Hash)) -> Option<EpochTable> {
        let tip = self.chain_state.read().await.best_height;
        let bpe = self.config.network.blocks_per_reward_epoch();
        let start = ancestor.0 - ancestor.0 % bpe;
        // A table from an abandoned boundary block is not this chain's.
        let stored_at = self.block_store.get_hash_by_height(start).ok().flatten();
        let canonical = |t: &EpochTable| match (t.boundary_hash, stored_at) {
            (Some(b), Some(c)) => b == c,
            _ => true,
        };
        if let Some(t) = self.state_db.get_epoch_table(start).filter(canonical) {
            return Some(t);
        }
        if start == tip - tip % bpe {
            return Some(self.live_epoch_table(start).await);
        }
        crate::metrics::FORK_CHOICE_TABLE_MISSING.inc();
        let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
        if !logged.contains(&ancestor.1) {
            if logged.len() >= LOGGED_CAP {
                logged.pop_front();
            }
            logged.push_back(ancestor.1);
            warn!(
                "FORK_TABLE: no epoch table for common ancestor {} (h={}, epoch start {}); reorg refused",
                ancestor.1, ancestor.0, start
            );
        }
        None
    }

    /// The ranking's table: the epoch of the lowest point where a cached branch
    /// meets the local chain (the tip's when none does). Missing: an empty table,
    /// so every branch weighs 0 and the tie rule orders them.
    pub(super) async fn rank_table(&self) -> EpochTable {
        let roots: Vec<Hash> = {
            let cache = self.fork_block_cache.read().await;
            (cache.values().map(|b| b.header.prev_hash))
                .filter(|p| !cache.contains_key(p))
                .collect()
        };
        let mut anchor: Option<(u64, Hash)> = None;
        for root in roots {
            if let Some(a) = self.attach_point(root).await {
                if anchor.is_none_or(|(h, _)| a.0 < h) {
                    anchor = Some(a);
                }
            }
        }
        let anchor = match anchor {
            Some(a) => a,
            None => {
                let cs = self.chain_state.read().await;
                (cs.best_height, cs.best_hash)
            }
        };
        self.ancestor_table(anchor).await.unwrap_or_default()
    }

    async fn live_epoch_table(&self, epoch_start: u64) -> EpochTable {
        let audit = (self.config.network.params()).security_audit_activation_height;
        let set = self.producer_set.read().await;
        EpochTable::new(
            epoch_start,
            None,
            self.epoch_state.producer_list.clone(),
            &set,
            |p| p.selection_weight_at(epoch_start, audit),
        )
    }

    /// The common ancestor of `from`'s branch and the local chain: the first block
    /// walking back from `from` that the canonical index holds.
    pub(super) async fn attach_point(&self, from: Hash) -> Option<(u64, Hash)> {
        let cache = self.fork_block_cache.read().await;
        let store = &self.block_store;
        let mut at = from;
        for _ in 0..=network::MAX_REORG_DEPTH + 1 {
            if at == Hash::ZERO {
                return Some((0, at));
            }
            if let Some(h) = store.get_height_by_hash(&at).ok().flatten() {
                if store.get_hash_by_height(h).ok().flatten() == Some(at) {
                    return Some((h, at));
                }
            }
            at = match cache.get(&at) {
                Some(b) => b.header.prev_hash,
                None => store.get_header(&at).ok().flatten()?.prev_hash,
            };
        }
        None
    }
}
