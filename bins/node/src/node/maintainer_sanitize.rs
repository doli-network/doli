//! Node-local maintainer trust-root resets (INC-I-248): the retired-key
//! sanitizer and the snapshot install.

use doli_core::maintainer::holds_retired_key_at;
use doli_core::MaintainerSet;
use tracing::warn;

use super::Node;

impl Node {
    /// Reset a set that holds a key retired below the chain tip to EMPTY at the
    /// tip. Returns `true` when it reset; otherwise nothing is written.
    pub async fn sanitize_retired_maintainer_set(&self) -> bool {
        let tip = self.chain_state.read().await.best_height;
        self.sanitize_retired_maintainer_set_at(tip).await
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
