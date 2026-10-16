//! Node-local maintainer trust-root resets (INC-I-248): the retired-key
//! sanitizer and the snapshot install.

use doli_core::maintainer::holds_retired_key_at;
use doli_core::MaintainerSet;
use tracing::warn;

use super::Node;

impl Node {
    /// Start-time check: reset a set that holds a retired key, or that was not
    /// derived from replayed history, to EMPTY at the tip. Returns `true` when it
    /// reset; otherwise nothing is written.
    /// Third check: a set that came from a post-gate producer seed.
    pub async fn sanitize_retired_maintainer_set(&self) -> bool {
        let tip = self.chain_state.read().await.best_height;
        self.sanitize_retired_maintainer_set_at(tip).await
            || self.reset_unreplayed_maintainer_set_at(tip).await
            || self.reset_post_gate_seed_set_at(tip).await
    }

    /// A non-empty set seeded at D = last_updated >= G > 0 below the tip, with block
    /// D stored and free of maintainer changes. A missing block or read error keeps it.
    async fn reset_post_gate_seed_set_at(&self, tip: u64) -> bool {
        let Some(ms) = &self.maintainer_state else {
            return false;
        };
        let gate = self
            .config
            .network
            .params()
            .maintainer_derivation_activation_height;
        let mut state = ms.write().await;
        let d = state.last_derived_height;
        if state.set.members.is_empty()
            || gate == 0
            || d < gate
            || state.set.last_updated != d
            || tip <= d
            || !matches!(
                self.block_store.get_block_by_height(d),
                Ok(Some(block)) if !block.transactions.iter().any(|tx| tx.is_maintainer_change())
            )
        {
            return false;
        }
        let members = state.set.member_count();
        state.set = MaintainerSet::new();
        state.last_derived_height = tip;
        let saved = state.save(&self.config.data_dir);
        warn!(
            "[MAINTAINER] trust root came from a post-gate producer seed, not from replayed history: {} members, D={}, reset to EMPTY at tip {} (saved: {:?}); waiting for a peer quorum (INC-I-248)",
            members, d, tip, saved
        );
        true
    }

    /// A non-empty set stamped at D >= 1 below the tip, with block D stored and
    /// block D+1 absent, skipped the history after D. A read error keeps the set.
    async fn reset_unreplayed_maintainer_set_at(&self, tip: u64) -> bool {
        let Some(ms) = &self.maintainer_state else {
            return false;
        };
        let mut state = ms.write().await;
        let d = state.last_derived_height;
        if state.set.members.is_empty() || tip <= d {
            return false;
        }
        let stored = |h: u64| {
            self.block_store
                .get_hash_by_height(h)
                .map(|hash| hash.is_some())
                .ok()
        };
        if d == 0 || stored(d) != Some(true) || stored(d.saturating_add(1)) != Some(false) {
            return false;
        }
        let members = state.set.member_count();
        state.set = MaintainerSet::new();
        state.last_derived_height = tip;
        let saved = state.save(&self.config.data_dir);
        warn!(
            "[MAINTAINER] trust root not derived from replayed history (block D present, block D+1 missing): {} members, D={}, reset to EMPTY at tip {} (saved: {:?}); waiting for a peer quorum (INC-I-248)",
            members, d, tip, saved
        );
        true
    }

    pub(super) async fn sanitize_retired_maintainer_set_at(&self, height: u64) -> bool {
        let Some(ms) = &self.maintainer_state else {
            return false;
        };
        let retired_after = self.params.retired_genesis_maintainer_keys_height;
        let mut state = ms.write().await;
        if !holds_retired_key_at(&state.set, height, retired_after) {
            return false;
        }
        let members = state.set.member_count();
        state.set = MaintainerSet::new();
        state.last_derived_height = height;
        let saved = state.save(&self.config.data_dir);
        warn!(
            "[MAINTAINER] trust root of {} members held a key retired after height {}: reset to EMPTY at height {} (saved: {:?}); waiting for a peer quorum (INC-I-248)",
            members, retired_after, height, saved
        );
        true
    }

    /// Snapshot install: the node skipped the history below `height`, so its
    /// root becomes EMPTY at `height`.
    pub(super) async fn reset_maintainer_root_to_snapshot(&self, height: u64) {
        let Some(ms) = &self.maintainer_state else {
            return;
        };
        let mut state = ms.write().await;
        state.set = MaintainerSet::new();
        state.last_derived_height = height;
        if let Err(e) = state.save(&self.config.data_dir) {
            warn!(
                "[MAINTAINER] failed to persist the snapshot trust root: {}",
                e
            );
        }
    }
}
