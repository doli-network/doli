//! M3 [F3] serve seam: `GetStateManifest` / `GetStateChunk`.
//!
//! Sibling of `state_snapshot_serve.rs` and refuses on the same node-wide
//! condition, but typed: a `SyncResponse::Error` would reach the client's
//! blacklist path, and every refusal here is a retryable server condition.

use crypto::Hash;
use network::protocols::sync::{
    StateSessionRefusal, SyncRequest, SyncResponse, MAX_CONCURRENT_STATE_SESSIONS,
    STATE_CHUNK_MAX_BYTES,
};
use tracing::{error, info, warn};

use super::Node;

/// PM-009: inbound sync serving limit, per production-check interval.
pub const MAX_SYNC_REQUESTS_PER_INTERVAL: u32 = 24;

impl Node {
    /// PM-009 admission for one inbound sync request.
    ///
    /// `Some(refusal)` is the response to send back; `None` admits and charges
    /// the request to the interval budget.
    ///
    /// `GetStateChunk` is exempt and costs nothing: chunk traffic is already
    /// bounded by `MAX_CONCURRENT_STATE_SESSIONS` and one outstanding chunk per
    /// session, and counting it here would let a session's own traffic saturate
    /// the cap and make the client blacklist the peer serving it correctly.
    pub fn admit_sync_request(&mut self, request: &SyncRequest) -> Option<SyncResponse> {
        if matches!(request, SyncRequest::GetStateChunk { .. }) {
            return None;
        }
        if self.sync_requests_this_interval >= MAX_SYNC_REQUESTS_PER_INTERVAL {
            return Some(SyncResponse::Error(
                "busy: sync serving limit reached".to_string(),
            ));
        }
        self.sync_requests_this_interval += 1;
        None
    }

    /// Open a chunked state-transfer session and describe its pinned view.
    pub async fn serve_state_manifest(&self, block_hash: Hash) -> SyncResponse {
        if let Some(reason) = self.rebuild_halt_reason() {
            error!("[SNAP_SYNC] Refusing GetStateManifest — {}", reason);
            return SyncResponse::StateSessionUnavailable {
                session_id: 0,
                reason: StateSessionRefusal::Halted(reason),
            };
        }

        self.state_sessions.evict_expired();
        if self.state_sessions.live_count() >= MAX_CONCURRENT_STATE_SESSIONS {
            warn!(
                "[SNAP_SYNC] Refusing GetStateManifest — {} sessions already pinned",
                MAX_CONCURRENT_STATE_SESSIONS
            );
            return SyncResponse::StateSessionUnavailable {
                session_id: 0,
                reason: StateSessionRefusal::Busy,
            };
        }

        // Serve at the current tip regardless of the requested hash, exactly as
        // the legacy frame does; the client verifies the root against quorum.
        let chain_state = self.chain_state.read().await.clone();
        if chain_state.best_hash != block_hash {
            info!(
                "[SNAP_SYNC] Manifest requested at {} but tip is {} — serving the tip",
                block_hash, chain_state.best_hash
            );
        }
        let producer_set = self.producer_set.read().await.clone();
        let handle = match self.utxo_set.read().await.pinnable_handle() {
            Ok(h) => h,
            Err(e) => return SyncResponse::Error(format!("Manifest error: {}", e)),
        };

        let opening = match self.state_sessions.open(handle).await {
            Ok(o) => o,
            Err(e) => return SyncResponse::Error(format!("Manifest error: {}", e)),
        };

        let state_root = storage::compose_state_root(
            &crypto::hash::hash(&chain_state.serialize_canonical()),
            &opening.utxo_hash,
            &crypto::hash::hash(&producer_set.serialize_canonical()),
        );

        let chain_state_bytes = match bincode::serialize(&chain_state) {
            Ok(b) => b,
            Err(e) => return SyncResponse::Error(format!("Manifest error: {}", e)),
        };
        let producer_set_bytes = match bincode::serialize(&producer_set) {
            Ok(b) => b,
            Err(e) => return SyncResponse::Error(format!("Manifest error: {}", e)),
        };

        let block_header_bytes =
            if chain_state.best_height >= doli_core::consensus::SNAP_HEADER_ACTIVATION_HEIGHT {
                match self.block_store.get_header(&chain_state.best_hash) {
                    Ok(Some(header)) => bincode::serialize(&header).ok(),
                    _ => None,
                }
            } else {
                None
            };

        info!(
            "[SNAP_SYNC] Session {} open at h={} ({} entries, root={:.16})",
            opening.session_id, chain_state.best_height, opening.utxo_count, state_root
        );

        SyncResponse::StateManifest {
            session_id: opening.session_id,
            block_hash: chain_state.best_hash,
            block_height: chain_state.best_height,
            state_root,
            utxo_hash: opening.utxo_hash,
            utxo_count: opening.utxo_count,
            chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
            chain_state: chain_state_bytes,
            producer_set: producer_set_bytes,
            block_header_bytes,
            epoch_bond_snapshot_bytes: self
                .state_db
                .get_epoch_bond_snapshot()
                .and_then(|(snap_data, epoch)| bincode::serialize(&(snap_data, epoch)).ok()),
            epoch_accumulators_bytes: self
                .state_db
                .get_attestation_accumulators()
                .and_then(|data| bincode::serialize(&data).ok()),
            epoch_state_bytes: self.state_db.get_epoch_state(),
        }
    }

    /// Serve one sorted-key range of a live session.
    ///
    /// `max_bytes` is peer-supplied and therefore advisory: it is clamped to
    /// `STATE_CHUNK_MAX_BYTES` here, or one request would re-materialise the
    /// whole set in this node's RAM.
    pub async fn serve_state_chunk(
        &self,
        session_id: u64,
        start_key: Option<Vec<u8>>,
        max_bytes: u32,
    ) -> SyncResponse {
        if let Some(reason) = self.rebuild_halt_reason() {
            error!("[SNAP_SYNC] Refusing GetStateChunk — {}", reason);
            return SyncResponse::StateSessionUnavailable {
                session_id,
                reason: StateSessionRefusal::Halted(reason),
            };
        }

        self.state_sessions.evict_expired();
        let budget = max_bytes.min(STATE_CHUNK_MAX_BYTES) as usize;

        let answer = self
            .state_sessions
            .request_range(session_id, start_key, budget)
            .await;

        match answer {
            Some(Ok((body, next_key))) => {
                if next_key.is_none() {
                    self.state_sessions.release(session_id);
                }
                SyncResponse::StateChunk {
                    session_id,
                    body,
                    next_key,
                }
            }
            Some(Err(e)) => {
                warn!("[SNAP_SYNC] Session {} range failed: {}", session_id, e);
                self.state_sessions.release(session_id);
                SyncResponse::Error(format!("State chunk error: {}", e))
            }
            None => SyncResponse::StateSessionUnavailable {
                session_id,
                reason: StateSessionRefusal::ManifestExpired,
            },
        }
    }
}
