//! State-root serve seam (State-Root Lazy Tier-0, M1).
//!
//! `serve_state_root` is the single memoize-on-compute entry point behind the
//! live `SyncRequest::GetStateRoot` handler (`validation_checks.rs`). It serves
//! both the diagnostic RPC and snap-sync quorum votes.
//!
//! M1 is behavior-ADDITIVE: it adds cache-on-compute WRITE-BACK and makes the
//! memo read best_hash-keyed so a stale tuple from a prior height is never
//! served as current. The returned root VALUE is byte-identical to the legacy
//! compute at every height. The eager per-block compute
//! (`apply_block/state_update.rs`) is retained until M2.
//!
//! Spec: `specs/state-root-commitment-architecture.md` — Migration Path steps 1-2.

use network::protocols::SyncResponse;

use super::Node;

impl Node {
    /// Serve the current state root, memoizing on cold/stale-memo compute.
    ///
    /// - memo HIT (cache `Some` and `cached.best_hash == current best_hash`):
    ///   return the cached tuple verbatim in O(1); the memo is not mutated.
    /// - memo COLD (cache `None`) or STALE (`cached.best_hash != best_hash`):
    ///   recompute the legacy root under read locks, DROP those read guards,
    ///   then take the `cached_state_root` write guard and store
    ///   `Some((root, best_hash, best_height))` (leaf-lock ordering — mirrors
    ///   `apply_block/state_update.rs` drop-then-write). A compute error is
    ///   returned as `SyncResponse::Error` and is NOT memoized.
    ///
    /// Mutual exclusion with the apply-write is provided by the single
    /// event-loop actor; no new locks are introduced.
    pub async fn serve_state_root(&self) -> SyncResponse {
        // Fast path: O(1) memo hit keyed on the current tip. Copy the memo tuple
        // and drop its read guard before reading chain_state (no nested guards).
        let memo = *self.cached_state_root.read().await;
        if let Some((root, hash, height)) = memo {
            let current_hash = self.chain_state.read().await.best_hash;
            if hash == current_hash {
                return SyncResponse::StateRoot {
                    block_hash: hash,
                    block_height: height,
                    state_root: root,
                };
            }
        }

        // Miss (cold or stale): recompute under read locks, then write back.
        let (best_hash, best_height, root_result) = {
            let chain_state = self.chain_state.read().await;
            let utxo_set = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            let best_hash = chain_state.best_hash;
            let best_height = chain_state.best_height;
            let root_result = storage::compute_state_root(&chain_state, &utxo_set, &ps);
            (best_hash, best_height, root_result)
            // chain_state / utxo_set / ps read guards drop here, BEFORE the
            // cached_state_root write guard is taken (leaf-lock ordering).
        };

        match root_result {
            Ok(root) => {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, best_hash, best_height));
                SyncResponse::StateRoot {
                    block_hash: best_hash,
                    block_height: best_height,
                    state_root: root,
                }
            }
            // Do NOT memoize an error.
            Err(e) => SyncResponse::Error(format!("State root error: {}", e)),
        }
    }
}
