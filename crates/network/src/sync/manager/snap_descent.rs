//! INC-I-289 AUDIT-P2-639-001: a peer-served descent proof for a snap anchor.
//!
//! Above the INC-I-288 gate a node with a finalized checkpoint `f` installs a
//! snapshot only when its anchor descends from `f`. A lagging node holds no
//! headers for the gap, so before the state download the snap peer serves the
//! header range `f+1..=anchor` through the existing `GetHeaders` request.

use crypto::Hash;
use doli_core::BlockHeader;
use libp2p::PeerId;
use tracing::{info, warn};

use super::{SyncManager, SyncPipelineData};
use crate::protocols::SyncRequest;

/// Most headers one descent proof may span: 20 `GetHeaders` batches of the
/// 500-header `max_headers_per_request`, about 27.8 h of 10 s slots. Snap starts
/// at a 500-block gap, so a node offline for a day still proves descent; a
/// longer gap falls back to block sync. Only the chain tail is kept (O(1) memory).
pub const SNAP_DESCENT_PROOF_MAX_HEADERS: u64 = 10_000;

/// Headers asked per proof request: the header-first batch size.
const PROOF_BATCH: u64 = 500;

/// A hash-linked header chain from the finalized block toward the anchor.
/// `tail` is the last linked header; the proof is complete at the anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapDescentProof {
    finalized: Hash,
    anchor: (u64, Hash),
    tail: (u64, Hash),
}

impl SnapDescentProof {
    /// `None` when the anchor is not above `fh` or the span exceeds the bound.
    fn new(fh: u64, f: Hash, anchor_height: u64, anchor: Hash) -> Option<Self> {
        let span = anchor_height.checked_sub(fh).filter(|s| *s > 0)?;
        if span > SNAP_DESCENT_PROOF_MAX_HEADERS {
            warn!(
                "[SNAP_SYNC] Descent proof refused: anchor h={} is {} headers above finalized h={} (max {}) — falling back to block sync",
                anchor_height, span, fh, SNAP_DESCENT_PROOF_MAX_HEADERS
            );
            return None;
        }
        Some(Self {
            finalized: f,
            anchor: (anchor_height, anchor),
            tail: (fh, f),
        })
    }

    fn is_complete(&self) -> bool {
        self.tail == self.anchor
    }

    /// Link `headers` onto the tail: each parent is the previous hash, and the
    /// header at the anchor height must hash to the anchor.
    fn extend(&mut self, headers: &[BlockHeader]) -> Result<(), &'static str> {
        if headers.is_empty() {
            return Err("empty header range");
        }
        for header in headers {
            if self.tail.0 >= self.anchor.0 {
                return Err("chain runs past the anchor height");
            }
            if header.prev_hash != self.tail.1 {
                return Err("broken parent link");
            }
            self.tail = (self.tail.0 + 1, header.hash());
        }
        if self.tail.0 == self.anchor.0 && self.tail.1 != self.anchor.1 {
            return Err("chain ends at a hash other than the anchor");
        }
        Ok(())
    }

    fn next_request(&self) -> SyncRequest {
        let count = (self.anchor.0 - self.tail.0).min(PROOF_BATCH) as u32;
        SyncRequest::get_headers(self.tail.1, count)
    }
}

impl SyncManager {
    /// The finalized checkpoint a snap anchor must descend from, above the gate.
    fn snap_descent_finality(&self) -> Option<(u64, Hash)> {
        if self.local_height < self.config.inc_i_288_fork_choice_activation_height {
            return None;
        }
        let fin = self.fork_choice_finality();
        fin.finalized_height.zip(fin.finalized_hash)
    }

    /// `true` once a served chain linked `f` to this anchor.
    pub fn snap_descent_proven(&self, anchor_height: u64, anchor: Hash, f: Hash) -> bool {
        self.snap_descent.as_ref().is_some_and(|p| {
            p.is_complete() && p.anchor == (anchor_height, anchor) && p.finalized == f
        })
    }

    /// Verify a whole served chain (`f+1..=anchor`) against the current
    /// finalized checkpoint and record it on success.
    pub fn accept_snap_descent_chain(
        &mut self,
        anchor_height: u64,
        anchor: Hash,
        headers: &[BlockHeader],
    ) -> bool {
        let Some((fh, f)) = self.snap_descent_finality() else {
            return false;
        };
        let Some(mut proof) = SnapDescentProof::new(fh, f, anchor_height, anchor) else {
            return false;
        };
        match proof.extend(headers) {
            Ok(()) if proof.is_complete() => {
                self.snap_descent = Some(proof);
                true
            }
            Ok(()) => {
                warn!("[SNAP_SYNC] Descent proof refused: chain stops before the anchor");
                false
            }
            Err(e) => {
                warn!("[SNAP_SYNC] Descent proof refused: {}", e);
                false
            }
        }
    }

    /// Dispatch hook for `SnapDownloading` before the manifest: the next proof
    /// batch, `Ok(None)` when no proof is needed or it is complete, `Err` when
    /// the span exceeds the bound (the caller leaves the peer before any download).
    pub(super) fn snap_descent_next_request(
        &mut self,
        target_height: u64,
        target_hash: Hash,
    ) -> Result<Option<SyncRequest>, ()> {
        let Some((fh, f)) = self.snap_descent_finality() else {
            return Ok(None);
        };
        if target_height <= fh {
            return Ok(None);
        }
        let current = self
            .snap_descent
            .as_ref()
            .is_some_and(|p| p.anchor == (target_height, target_hash) && p.finalized == f);
        if !current {
            self.snap_descent = SnapDescentProof::new(fh, f, target_height, target_hash);
        }
        match self.snap_descent.as_ref() {
            None => Err(()),
            Some(p) => Ok((!p.is_complete()).then(|| p.next_request())),
        }
    }

    /// An over-span primary is a per-peer fault: leave it for the next quorum
    /// peer when one has an anchor within the bound. `false` when none has.
    pub(super) fn snap_descent_leave_over_span(&mut self, peer: PeerId) -> bool {
        let Some((fh, _)) = self.snap_descent_finality() else {
            return false;
        };
        let in_bound = match &self.pipeline_data {
            SyncPipelineData::SnapDownloading {
                alternate_peers, ..
            } => alternate_peers
                .iter()
                .any(|(_, _, h)| h.saturating_sub(fh) <= SNAP_DESCENT_PROOF_MAX_HEADERS),
            _ => false,
        };
        if in_bound {
            self.handle_snap_download_error(peer);
        }
        in_bound
    }

    /// `true` when a `Headers` response from `peer` answers a pending proof batch.
    pub(super) fn snap_descent_owns(&self, peer: PeerId) -> bool {
        self.state_session.is_none()
            && self.snap_descent.as_ref().is_some_and(|p| !p.is_complete())
            && matches!(&self.pipeline_data,
                SyncPipelineData::SnapDownloading { peer: p, .. } if *p == peer)
    }

    /// Link one served batch; a broken or foreign chain refuses this peer.
    pub(super) fn handle_snap_descent_headers(&mut self, peer: PeerId, headers: Vec<BlockHeader>) {
        let Some(proof) = self.snap_descent.as_mut() else {
            return;
        };
        match proof.extend(&headers) {
            Ok(()) if proof.is_complete() => info!(
                "[SNAP_SYNC] Descent proof complete: anchor h={} links to finalized block via {}",
                proof.anchor.0, peer
            ),
            Ok(()) => {}
            Err(e) => {
                warn!(
                    "[SNAP_SYNC] Descent proof refused from {}: {} — trying alternate peer",
                    peer, e
                );
                self.snap_descent = None;
                self.handle_snap_download_error(peer);
            }
        }
    }
}
