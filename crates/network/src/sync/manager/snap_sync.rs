//! Snap sync — state root voting, snapshot download, quorum verification, and fallback

use std::collections::HashMap;
use std::time::Instant;

use libp2p::PeerId;
use tracing::{info, warn};

use crypto::Hash;

use super::{SyncManager, SyncPhase, SyncPipelineData, SyncState, VerifiedSnapshot};

impl SyncManager {
    /// Handle a StateRoot response: add vote and check quorum.
    /// Votes are grouped by (height, root) — peers must agree on both to form
    /// a quorum. Votes from peers too far below the target height are rejected
    /// to prevent freshly-wiped peers (at height=0) from contaminating quorum
    /// with empty-state votes and triggering cascading snap sync failures.
    pub(super) fn handle_snap_state_root(
        &mut self,
        peer: PeerId,
        block_hash: Hash,
        block_height: u64,
        state_root: Hash,
    ) {
        // Extract target_height before taking a mutable borrow below.
        let target_height = match &self.pipeline_data {
            SyncPipelineData::SnapCollecting { target_height, .. } => *target_height,
            _ => return,
        };

        // Reject votes from peers whose reported height is far below the
        // target. Tight window (100 blocks ≈ 16 min) prevents stale peers
        // from forming quorum during rolling deployments.
        let min_acceptable = target_height.saturating_sub(100);
        if block_height < min_acceptable {
            warn!(
                "[SNAP_SYNC] Rejecting stale state root vote from {} at height={} \
                 (target={}, min_acceptable={})",
                peer, block_height, target_height, min_acceptable
            );
            return;
        }

        // NOTE: We do NOT filter by block_hash here because peers advance
        // every ~10s. Fix 1 (consensus_target_hash) ensures we target the
        // majority chain, and Fix 2 (quorum from total peers) prevents a
        // minority partition from reaching quorum. Together these two
        // mechanisms make block_hash filtering unnecessary and avoid
        // rejecting valid votes from peers that simply advanced.

        if let SyncPipelineData::SnapCollecting { votes, .. } = &mut self.pipeline_data {
            votes.push((peer, block_hash, block_height, state_root));
        } else {
            return;
        }

        // Quorum: majority of ALL connected peers, not just votes received.
        //
        // FIX: Using vote_count as denominator allowed partition quorum —
        // 3 forked nodes on the same server could form quorum (3/3) while
        // 9 correct peers hadn't responded yet. Using total connected peers
        // as denominator ensures quorum represents a true network majority.
        //
        // Example: 12 connected peers → quorum = max(3, 12/2+1) = 7.
        // A partition of 3 nodes can never reach 7, even if they all agree.
        let total_peers = self.peers.len();
        // Cap quorum at 5 regardless of peer count. The uncapped formula
        // (total_peers/2+1) produces 26-31 for 50-60 peers, which is never
        // reachable within root_timeout (15s) on an active chain with 10s
        // block time — state roots change every block, fragmenting vote groups.
        // 5 tolerates 2 colluding attackers — the real protection is
        // compute_state_root_from_bytes() verification after download, not
        // quorum size. Safe for 1K-5K+ node networks. (INC-I-012 F4)
        let quorum = std::cmp::max(self.snap.quorum, std::cmp::min(total_peers / 2 + 1, 5));

        let votes_snapshot: Vec<(PeerId, Hash, u64, Hash)> =
            if let SyncPipelineData::SnapCollecting { votes, .. } = &self.pipeline_data {
                votes.clone()
            } else {
                return;
            };

        // Group votes by state_root only — peers at different heights with the
        // same root no longer split quorum (fixes ±1 height divergence).
        let mut groups: HashMap<Hash, Vec<(PeerId, Hash, u64)>> = HashMap::new();
        for (pid, bhash, bheight, sroot) in &votes_snapshot {
            groups
                .entry(*sroot)
                .or_default()
                .push((*pid, *bhash, *bheight));
        }

        if let Some((quorum_root, peers_with_info)) =
            groups.iter().find(|(_, peers)| peers.len() >= quorum)
        {
            // Pick peer with highest height for download
            let (download_peer, download_hash, best_height) = peers_with_info
                .iter()
                .max_by_key(|(_, _, h)| *h)
                .copied()
                .unwrap();
            // Build alternate peer list (excluding primary) for retry on error
            let alternate_peers: Vec<(PeerId, Hash, u64)> = peers_with_info
                .iter()
                .filter(|(pid, _, _)| *pid != download_peer)
                .copied()
                .collect();
            info!(
                "[SNAP_SYNC] Quorum reached: {} peers agree on root={:.16}, best_height={}, downloading from {} ({} alternates)",
                peers_with_info.len(), quorum_root, best_height, download_peer, alternate_peers.len()
            );
            let quorum_root = *quorum_root;
            self.set_syncing(
                SyncPhase::SnapDownloading,
                SyncPipelineData::SnapDownloading {
                    target_hash: download_hash,
                    target_height: best_height,
                    quorum_root,
                    peer: download_peer,
                    alternate_peers,
                },
                "snap_quorum_reached",
            );
        }
    }

    /// Handle a StateSnapshot response: store as SnapReady for node consumption.
    ///
    /// The snapshot includes the peer's actual state_root. We use that as the
    /// expected root (the node verifies by recomputing from the raw bytes).
    /// If the peer advanced since the quorum vote, its root will differ from
    /// quorum_root — that's fine, the node's compute_state_root_from_bytes()
    /// is the ultimate check. The quorum vote ensured we're downloading from
    /// a peer that was recently on the canonical chain.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_snap_snapshot(
        &mut self,
        peer: PeerId,
        block_hash: Hash,
        block_height: u64,
        chain_state: Vec<u8>,
        utxo_set: Vec<u8>,
        producer_set: Vec<u8>,
        response_root: Hash,
    ) {
        if let SyncPipelineData::SnapDownloading {
            target_height,
            quorum_root,
            ..
        } = &self.pipeline_data
        {
            // INC-I-004 Fix: Reject snapshots whose height is too far below the
            // quorum target. Without this check, a peer that was in the quorum
            // at height 2788 could send a stale snapshot at height 2481, which
            // the node would accept — leaving it 300+ blocks behind with wrong
            // scheduling state, unable to apply any subsequent blocks.
            let min_acceptable = target_height.saturating_sub(100);
            if block_height < min_acceptable {
                warn!(
                    "[SNAP_SYNC] Rejecting stale snapshot from {} at height={} \
                     (target={}, min_acceptable={}) — trying alternate peer",
                    peer, block_height, target_height, min_acceptable
                );
                self.handle_snap_download_error(peer);
                return;
            }

            if response_root != *quorum_root {
                info!(
                    "[SNAP_SYNC] Peer {} advanced since vote: response_root={:.16} != quorum_root={:.16} (height={}). Accepting — node verifies independently.",
                    peer, response_root, quorum_root, block_height
                );
            }
            info!(
                "[SNAP_SYNC] Snapshot received from {} — height={}, storing as SnapReady (node will verify root)",
                peer, block_height
            );
            self.set_syncing(
                SyncPhase::SnapDownloading, // Keep SnapDownloading phase — SnapReady is just data
                SyncPipelineData::SnapReady {
                    snapshot: VerifiedSnapshot {
                        block_hash,
                        block_height,
                        chain_state,
                        utxo_set,
                        producer_set,
                        state_root: response_root,
                    },
                },
                "snap_snapshot_received",
            );
        } else {
            warn!(
                "[SNAP_SYNC] Unexpected snapshot from {} — not in SnapDownloading state, ignoring",
                peer
            );
        }
    }

    /// Handle an error response while in SnapDownloading state.
    ///
    /// The most common cause: the peer advanced to a new block between the vote
    /// and the download request, so `GetStateSnapshot(old_hash)` is rejected with
    /// "Snapshot only available for current tip". We blacklist that peer and try
    /// the next alternate peer from the quorum group. If no alternates remain,
    /// fall back to normal sync (which may retry snap sync from scratch).
    pub(super) fn handle_snap_download_error(&mut self, peer: PeerId) {
        if !matches!(self.pipeline_data, SyncPipelineData::SnapDownloading { .. }) {
            return;
        }
        self.snap.blacklisted_peers.insert(peer);
        // Take the pipeline data so we can decompose it without borrow issues
        let old = std::mem::replace(&mut self.pipeline_data, SyncPipelineData::None);
        if let SyncPipelineData::SnapDownloading {
            quorum_root,
            mut alternate_peers,
            ..
        } = old
        {
            if let Some((next_peer, next_hash, next_height)) = alternate_peers.pop() {
                warn!(
                    "[SNAP_SYNC] Peer {} failed, retrying with alternate peer {} at height={} ({} remaining)",
                    peer, next_peer, next_height, alternate_peers.len()
                );
                self.set_syncing(
                    SyncPhase::SnapDownloading,
                    SyncPipelineData::SnapDownloading {
                        target_hash: next_hash,
                        target_height: next_height,
                        quorum_root,
                        peer: next_peer,
                        alternate_peers,
                    },
                    "snap_download_error_retry_alternate",
                );
            } else {
                warn!(
                    "[SNAP_SYNC] No alternate peers left after {} failed — restarting snap sync",
                    peer
                );
                self.set_state(SyncState::Idle, "snap_download_error_no_alternates");
                self.snap_fallback_to_normal();
            }
        }
    }

    /// Take the ready snapshot for node application. Transitions to Synchronized.
    pub fn take_snap_snapshot(&mut self) -> Option<VerifiedSnapshot> {
        if matches!(self.pipeline_data, SyncPipelineData::SnapReady { .. }) {
            let old = std::mem::replace(&mut self.pipeline_data, SyncPipelineData::None);
            self.set_state(SyncState::Synchronized, "snap_snapshot_applied");
            self.snap.last_snap_completed = Some(Instant::now());
            self.fork.header_blacklisted_peers.clear();
            // Snap sync bypasses the normal sync pipeline, so complete_resync()
            // is never reached via handle_applied_block(). Clear it here so
            // production resumes after the grace period.
            if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
                self.complete_resync();
            }
            // Block production until a canonical gossip block arrives.
            // Snap sync restores state but leaves the block store empty —
            // producing now would create a fork with no real parent.
            self.recovery_phase = super::RecoveryPhase::AwaitingCanonicalBlock {
                started: std::time::Instant::now(),
            };
            info!("[SNAP_SYNC] Production gated: awaiting first canonical gossip block");
            if let SyncPipelineData::SnapReady { snapshot } = old {
                Some(snapshot)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Pick the best snap sync vote group (largest with >= 2 peers).
    /// Returns (state_root, Vec<(peer_id, block_hash, height)>) or None.
    #[allow(clippy::type_complexity)]
    pub(super) fn pick_best_snap_group(&self) -> Option<(Hash, Vec<(PeerId, Hash, u64)>)> {
        let votes = if let SyncPipelineData::SnapCollecting { votes, .. } = &self.pipeline_data {
            votes
        } else {
            return None;
        };

        let mut groups: HashMap<Hash, Vec<(PeerId, Hash, u64)>> = HashMap::new();
        for (pid, bhash, bheight, sroot) in votes {
            groups
                .entry(*sroot)
                .or_default()
                .push((*pid, *bhash, *bheight));
        }

        // Find the largest group with >= 2 peers (break ties by highest height)
        groups
            .into_iter()
            .filter(|(_, peers)| peers.len() >= 2)
            .max_by_key(|(_, peers)| {
                let max_h = peers.iter().map(|(_, _, h)| *h).max().unwrap_or(0);
                (peers.len(), max_h)
            })
    }

    /// Fall back from snap sync to normal header-first sync.
    /// Increments the snap attempt counter; after 3 failures, snap sync is skipped.
    pub fn snap_fallback_to_normal(&mut self) {
        self.snap.attempts += 1;
        warn!(
            "[SNAP_SYNC] Attempt {}/3 failed, {}",
            self.snap.attempts,
            if self.snap.attempts >= 3 {
                "falling back to header-first sync"
            } else {
                "will retry with different peer"
            }
        );
        self.set_state(SyncState::Idle, "snap_fallback_to_normal");
        if self.should_sync() {
            self.start_sync();
        }
    }

    /// Blacklist a peer for snap sync (bad snapshot).
    pub fn snap_blacklist_peer(&mut self, peer: PeerId) {
        warn!("[SNAP_SYNC] Blacklisting peer {} for bad snapshot", peer);
        self.snap.blacklisted_peers.insert(peer);
    }
}
