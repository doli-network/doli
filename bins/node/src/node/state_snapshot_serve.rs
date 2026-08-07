//! State-snapshot serve seam (INC-I-156 / AUDIT-P1-001).
//!
//! `serve_state_snapshot` is the single entry point behind the live
//! `SyncRequest::GetStateSnapshot` handler (`validation_checks.rs`). It mirrors
//! the `serve_state_root` seam in `state_root_serve.rs` — same shape, same
//! reason: returning a `SyncResponse` instead of writing to a libp2p
//! `ResponseChannel` makes the refusal branch directly observable.
//!
//! The snapshot body is a verbatim move of the former inline match arm; the
//! only added behaviour is the `[STATE_CORRUPT]` refusal at the top.

use crypto::Hash;
use network::protocols::SyncResponse;
use tracing::{error, info};

use super::Node;

impl Node {
    /// AUDIT-P1-001 (INC-I-156): the shared refusal predicate.
    ///
    /// Returns an operator-facing reason while `CF_META[rebuild_in_progress]`
    /// is set, i.e. while a destructive rebuild-from-genesis has committed its
    /// `clear()` of `cf_utxo` without reaching the trailing `atomic_replace`.
    /// In that state the durable UTXO set is a truncated subset of the chain
    /// `chain_state` names, and nothing else in the node detects it:
    /// `BlockHeader` carries no `state_root`, so a wrong set is never caught at
    /// block acceptance.
    ///
    /// Read straight from `CF_META` rather than cached on the struct so it is
    /// correct no matter how the `Node` was constructed and cannot go stale
    /// across the rebuild that sets it. The cost is one point lookup into a
    /// tiny, permanently block-cached column family.
    pub fn rebuild_halt_reason(&self) -> Option<String> {
        let (target_height, started_at) = self.state_db.get_rebuild_in_progress()?;
        // AUDIT-P3-103: the reader fails CLOSED, so it can report an armed halt
        // whose payload it could not decode. Say so rather than printing the
        // sentinel as if the marker had claimed it.
        let (target, started) = if target_height == storage::StateDb::REBUILD_TARGET_UNKNOWN {
            (
                "UNKNOWN (marker unreadable)".to_string(),
                "UNKNOWN".to_string(),
            )
        } else {
            (target_height.to_string(), started_at.to_string())
        };
        Some(format!(
            "[STATE_CORRUPT] An interrupted rebuild-from-genesis (target height {}, started at \
             unix {}) emptied the durable UTXO set and never finished replaying it. This node's \
             ledger is TRUNCATED — block production, state-snapshot and state-root service are \
             refused. Remedy: resync this node (wipe the data directory and snap-sync from a \
             healthy peer, or restore a checkpoint taken before the rebuild).",
            target, started
        ))
    }

    /// Serve a state snapshot for snap sync.
    ///
    /// Refuses with `SyncResponse::Error` while `rebuild_halt_reason()` is set:
    /// a node whose durable ledger was truncated by an interrupted rebuild must
    /// not hand that ledger to a bootstrapping peer.
    pub async fn serve_state_snapshot(&self, block_hash: Hash) -> SyncResponse {
        if let Some(reason) = self.rebuild_halt_reason() {
            error!("[SNAP_SYNC] Refusing GetStateSnapshot — {}", reason);
            return SyncResponse::Error(reason);
        }

        let chain_state = self.chain_state.read().await;
        // Serve snapshot at current tip regardless of requested hash.
        // The requesting node verifies the state root against quorum votes.
        // Previously this rejected requests where best_hash != block_hash,
        // causing a race condition: the peer advances between vote and
        // download, making snap sync fail 100% of the time on active chains.
        if chain_state.best_hash != block_hash {
            info!(
                "[SNAP_SYNC] Requested hash {} differs from tip {} — serving current tip (client verifies root)",
                block_hash, chain_state.best_hash
            );
        }
        let utxo_set = self.utxo_set.read().await;
        let ps = self.producer_set.read().await;
        match storage::StateSnapshot::create(&chain_state, &utxo_set, &ps) {
            Ok(snap) => {
                info!(
                    "[SNAP_SYNC] Serving snapshot at height={}, size={}KB, root={}",
                    snap.block_height,
                    snap.total_bytes() / 1024,
                    snap.state_root
                );
                // Option C: include anchor header so receiving node can persist it
                let block_header_bytes =
                    if snap.block_height >= doli_core::consensus::SNAP_HEADER_ACTIVATION_HEIGHT {
                        if let Ok(Some(header)) = self.block_store.get_header(&snap.block_hash) {
                            bincode::serialize(&header).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                let epoch_bond_snapshot_bytes = self
                    .state_db
                    .get_epoch_bond_snapshot()
                    .and_then(|(snap_data, epoch)| bincode::serialize(&(snap_data, epoch)).ok());
                let epoch_accumulators_bytes = self
                    .state_db
                    .get_attestation_accumulators()
                    .and_then(|data| bincode::serialize(&data).ok());
                // M7: complete EpochState for direct transfer (no reconstruction)
                let epoch_state_bytes = self.state_db.get_epoch_state();
                info!(
                    "[SNAP_SYNC] Sending snapshot response: h={} hash={:.16} cs={}B utxo={}B ps={}B epoch_state={}",
                    snap.block_height,
                    snap.block_hash,
                    snap.chain_state_bytes.len(),
                    snap.utxo_set_bytes.len(),
                    snap.producer_set_bytes.len(),
                    if epoch_state_bytes.is_some() { "included" } else { "MISSING" }
                );
                SyncResponse::StateSnapshot {
                    block_hash: snap.block_hash,
                    block_height: snap.block_height,
                    chain_state: snap.chain_state_bytes,
                    utxo_set: snap.utxo_set_bytes,
                    producer_set: snap.producer_set_bytes,
                    state_root: snap.state_root,
                    block_header_bytes,
                    epoch_bond_snapshot_bytes,
                    epoch_accumulators_bytes,
                    epoch_state_bytes,
                }
            }
            Err(e) => SyncResponse::Error(format!("Snapshot error: {}", e)),
        }
    }
}
