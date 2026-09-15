//! Client side of the M3 [F3] chunked snap-sync session.
//!
//! One session replaces one `GetStateSnapshot` round trip: a manifest that is
//! O(1) in the set size, then a walk of sorted-key ranges reassembled into the
//! canonical image the legacy frame used to carry whole.

use libp2p::PeerId;
use tracing::{info, warn};

use crypto::Hash;

use crate::protocols::sync::StateSessionRefusal;

use crate::sync::{StagedUtxoMarker, UtxoChunkSink};

use super::{SyncManager, SyncPhase, SyncPipelineData, SyncState, VerifiedSnapshot};

/// A state transfer in progress against one peer.
pub(crate) struct StateSessionClient {
    pub session_id: u64,
    pub peer: PeerId,
    pub block_hash: Hash,
    pub block_height: u64,
    pub state_root: Hash,
    pub utxo_hash: Hash,
    pub chunk_max_bytes: u32,
    /// Cursor of the last VERIFIED range. A failed chunk resumes here, never at zero.
    pub cursor: Option<Vec<u8>>,
    /// Entries the manifest announced.
    pub utxo_count: u64,
    /// `hash(utxo_count LE || body0 || body1 || ...)`, folded as bodies arrive.
    /// One digest implementation for both paths, so they cannot drift.
    pub digest: crypto::Hasher,
    /// `utxo_count` LE header followed by every body accepted so far. Stays at
    /// the bare header on the staged path — nothing accumulates.
    pub image: Vec<u8>,
    pub chain_state: Vec<u8>,
    pub producer_set: Vec<u8>,
    pub block_header_bytes: Option<Vec<u8>>,
    pub epoch_bond_snapshot_bytes: Option<Vec<u8>>,
    pub epoch_accumulators_bytes: Option<Vec<u8>>,
    pub epoch_state_bytes: Option<Vec<u8>>,
}

impl SyncManager {
    /// The peer and cursor the next `GetStateChunk` must carry, if a session is live
    /// against `peer`.
    pub(crate) fn state_session_cursor(&self, peer: PeerId) -> Option<(u64, Option<Vec<u8>>, u32)> {
        let s = self.state_session.as_ref()?;
        if s.peer != peer {
            return None;
        }
        Some((s.session_id, s.cursor.clone(), s.chunk_max_bytes))
    }

    /// Admit a manifest: INC-I-143 F4 Gate 1 (exact root equality) and Gate 2
    /// (quorum-corroborated anchor height) both run BEFORE one chunk is requested.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_state_manifest(
        &mut self,
        peer: PeerId,
        session_id: u64,
        block_hash: Hash,
        block_height: u64,
        state_root: Hash,
        utxo_hash: Hash,
        utxo_count: u64,
        chunk_max_bytes: u32,
        chain_state: Vec<u8>,
        producer_set: Vec<u8>,
        block_header_bytes: Option<Vec<u8>>,
        epoch_bond_snapshot_bytes: Option<Vec<u8>>,
        epoch_accumulators_bytes: Option<Vec<u8>>,
        epoch_state_bytes: Option<Vec<u8>>,
    ) {
        let (target_height, quorum_root) = match &self.pipeline_data {
            SyncPipelineData::SnapDownloading {
                target_height,
                quorum_root,
                ..
            } => (*target_height, *quorum_root),
            _ => {
                warn!(
                    "[SNAP_SYNC] Unexpected state manifest from {} — not in SnapDownloading, ignoring",
                    peer
                );
                return;
            }
        };

        let min_acceptable = target_height.saturating_sub(100);
        if block_height < min_acceptable {
            warn!(
                "[SNAP_SYNC] Rejecting stale manifest from {} at height={} (target={}, min={})",
                peer, block_height, target_height, min_acceptable
            );
            self.handle_snap_download_error(peer);
            return;
        }

        if state_root != quorum_root {
            self.snap.integrity_refusals += 1;
            warn!(
                "[SNAP_SYNC] F4 REFUSE (manifest root): {:.16} != quorum {:.16} from {} (h={}) — refusals={}",
                state_root, quorum_root, peer, block_height, self.snap.integrity_refusals
            );
            self.clear_state_session();
            self.handle_snap_download_error(peer);
            return;
        }

        let quorum = self.snap_quorum();
        let corroborators = self
            .peers
            .values()
            .filter(|s| s.best_hash == block_hash && s.best_height == block_height)
            .count();
        if corroborators < quorum {
            self.snap.integrity_refusals += 1;
            warn!(
                "[SNAP_SYNC] F4 REFUSE (manifest height): anchor ({:.16}, h={}) corroborated by {}/{} — refusals={}",
                block_hash, block_height, corroborators, quorum, self.snap.integrity_refusals
            );
            self.clear_state_session();
            self.handle_snap_download_error(peer);
            return;
        }

        info!(
            "[SNAP_SYNC] Session {} admitted from {}: anchor ({:.16}, h={}), {} entries, chunk<={}B",
            session_id, peer, block_hash, block_height, utxo_count, chunk_max_bytes
        );
        // A session always begins from an EMPTY staging family: rows from an
        // earlier peer would be concatenated onto this stream and match neither
        // manifest.
        self.clear_state_session();
        self.clear_utxo_staging();

        let mut digest = crypto::Hasher::new();
        digest.update(&utxo_count.to_le_bytes());
        self.state_session = Some(StateSessionClient {
            session_id,
            peer,
            block_hash,
            block_height,
            state_root,
            utxo_hash,
            chunk_max_bytes,
            cursor: None,
            utxo_count,
            digest,
            image: utxo_count.to_le_bytes().to_vec(),
            chain_state,
            producer_set,
            block_header_bytes,
            epoch_bond_snapshot_bytes,
            epoch_accumulators_bytes,
            epoch_state_bytes,
        });
    }

    /// Accept one verified range and advance the cursor. `next_key == None` closes the
    /// session: the reassembled image is checked against the manifest digest and either
    /// becomes `SnapReady` or is refused and counted.
    pub fn handle_state_chunk(
        &mut self,
        peer: PeerId,
        session_id: u64,
        body: Vec<u8>,
        next_key: Option<Vec<u8>>,
    ) {
        match self.state_session.as_ref() {
            Some(s) if s.session_id == session_id && s.peer == peer => {}
            _ => {
                warn!(
                    "[SNAP_SYNC] Chunk for unknown session {} from {} — ignoring",
                    session_id, peer
                );
                return;
            }
        }

        // The body lands in the staging family BEFORE it is folded into the
        // digest, so a write that cannot be made durable abandons the session
        // rather than being counted as accepted (INC-I-204).
        if let Some(sink) = self.utxo_chunk_sink.clone() {
            if let Err(e) = sink.stage(&body) {
                warn!(
                    "[SNAP_SYNC] Staging a verified chunk of session {} failed: {} — abandoning \
                     the transfer",
                    session_id, e
                );
                self.clear_state_session();
                self.handle_snap_download_error(peer);
                return;
            }
        }

        let staged = self.utxo_chunk_sink.is_some();
        {
            let s = self
                .state_session
                .as_mut()
                .expect("session presence checked above");
            s.digest.update(&body);
            if !staged {
                s.image.extend_from_slice(&body);
            }
            s.cursor = next_key.clone();
        }

        if next_key.is_some() {
            return;
        }

        let session = self
            .state_session
            .take()
            .expect("session presence checked above");
        let reassembled = session.digest.finalize();
        if reassembled != session.utxo_hash {
            self.snap.integrity_refusals += 1;
            warn!(
                "[SNAP_SYNC] REFUSE (digest): reassembled {:.16} != manifest {:.16} from {} ({} entries) — refusals={}",
                reassembled,
                session.utxo_hash,
                peer,
                session.utxo_count,
                self.snap.integrity_refusals
            );
            // Quarantine: bytes that never passed the digest must not survive
            // into the next peer's stream.
            self.clear_utxo_staging();
            self.handle_snap_download_error(peer);
            return;
        }

        let (utxo_set, utxo_staged) = if staged {
            (
                Vec::new(),
                Some(StagedUtxoMarker {
                    utxo_hash: session.utxo_hash,
                    utxo_count: session.utxo_count,
                }),
            )
        } else {
            (session.image, None)
        };
        info!(
            "[SNAP_SYNC] Session {} complete: digest matches manifest ({} entries, {} staged rows)",
            session_id,
            session.utxo_count,
            self.staged_utxo_rows()
        );
        self.set_syncing(
            SyncPhase::SnapDownloading,
            SyncPipelineData::SnapReady {
                snapshot: VerifiedSnapshot {
                    block_hash: session.block_hash,
                    block_height: session.block_height,
                    chain_state: session.chain_state,
                    utxo_set,
                    utxo_staged,
                    producer_set: session.producer_set,
                    state_root: session.state_root,
                    block_header_bytes: session.block_header_bytes,
                    epoch_bond_snapshot_bytes: session.epoch_bond_snapshot_bytes,
                    epoch_accumulators_bytes: session.epoch_accumulators_bytes,
                    epoch_state_bytes: session.epoch_state_bytes,
                },
            },
            "snap_state_session_complete",
        );
    }

    /// Install the durable landing zone for verified chunk bodies.
    ///
    /// Without one the session keeps the M3 materialised-image path, which is
    /// what a backend with no staging family needs.
    pub fn set_utxo_chunk_sink(&mut self, sink: std::sync::Arc<dyn UtxoChunkSink>) {
        self.utxo_chunk_sink = Some(sink);
    }

    /// Rows currently staged, or 0 with no sink installed.
    pub(crate) fn staged_utxo_rows(&self) -> u64 {
        self.utxo_chunk_sink
            .as_ref()
            .map_or(0, |sink| sink.staged_len())
    }

    /// Discard whatever the sink holds. Every abandon path runs through here.
    pub(crate) fn clear_utxo_staging(&self) {
        if let Some(sink) = self.utxo_chunk_sink.as_ref() {
            if let Err(e) = sink.clear() {
                warn!(
                    "[SNAP_SYNC] Failed to clear the UTXO staging family: {} — the next session \
                     admission retries the clear",
                    e
                );
            }
        }
    }

    /// A chunk failed or timed out. The cursor does NOT rewind and no snap attempt is
    /// consumed — the next dispatch re-requests the same range (F-07).
    pub fn handle_state_chunk_error(&mut self, peer: PeerId, session_id: u64) {
        match self.state_session.as_ref() {
            Some(s) if s.session_id == session_id && s.peer == peer => {
                warn!(
                    "[SNAP_SYNC] Chunk failed in session {} from {} — resuming at the last verified cursor",
                    session_id, peer
                );
            }
            _ => {}
        }
    }

    /// A typed session refusal. All three reasons are retryable server conditions, so
    /// none blacklists the peer and none consumes a snap attempt (F-08).
    pub fn handle_state_session_unavailable(
        &mut self,
        peer: PeerId,
        session_id: u64,
        reason: StateSessionRefusal,
    ) {
        warn!(
            "[SNAP_SYNC] Session {} unavailable at {}: {:?} — retrying with a fresh manifest",
            session_id, peer, reason
        );
        if let Some(s) = self.state_session.as_ref() {
            if s.session_id == session_id {
                self.clear_state_session();
            }
        }
        if matches!(self.state, SyncState::Idle) {
            self.set_state(SyncState::Idle, "state_session_unavailable");
        }
    }

    /// Drop any live session. Called wherever the snap pipeline is abandoned so a stale
    /// cursor can never be spliced onto a new manifest.
    pub(crate) fn clear_state_session(&mut self) {
        if self.state_session.take().is_some() {
            self.clear_utxo_staging();
        }
    }
}
