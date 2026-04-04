//! Sync start & peer selection — should_sync, consensus_target_hash, best_peer, start_sync

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use libp2p::PeerId;
use tracing::{info, warn};

use crypto::Hash;

use crate::sync::manager::{SyncManager, SyncPhase, SyncPipelineData};

impl SyncManager {
    /// Check if we should start syncing
    pub(in crate::sync::manager) fn should_sync(&self) -> bool {
        if self.peers.len() < self.config.min_peers_for_sync {
            return false;
        }

        // Only trigger sync for SIGNIFICANT gaps (>= 3 blocks behind best peer).
        // A 1-2 block gap is normal gossip propagation delay — the gossip block
        // will arrive momentarily. Starting a full sync cycle for a 1-block gap
        // pulls the node out of Synchronized → Syncing → blocks production via
        // Check 1 → node misses its slot → falls further behind → cascade.
        //
        // Exception: at height 0 (initial sync), always sync — there's no gossip yet.
        let min_gap = if self.local_height == 0 { 1 } else { 3 };

        let best_peer_height = self.best_peer_height();
        let peer_gap = best_peer_height.saturating_sub(self.local_height);

        let network_gap = self
            .network
            .network_tip_height
            .saturating_sub(self.local_height);

        peer_gap >= min_gap || network_gap >= min_gap
    }

    /// Get the best_hash that the MAJORITY of eligible peers agree on.
    ///
    /// This prevents a partitioned minority from steering snap sync to a
    /// forked chain. Returns None if no (height, hash) pair has >= 2 peers.
    fn consensus_target_hash(&self) -> Option<Hash> {
        // INC-I-012 F10: Group by (height, hash) pairs, not just height.
        // During active forks, two honest peers at the same height may have
        // different blocks. The old code assumed all peers at the same height
        // have the same hash (using the first peer's hash as representative).
        // Now we correctly count agreement on both height AND hash.
        let mut pair_counts: HashMap<(u64, Hash), usize> = HashMap::new();
        for status in self.peers.values() {
            if status.best_height > self.local_height {
                *pair_counts
                    .entry((status.best_height, status.best_hash))
                    .or_default() += 1;
            }
        }
        if pair_counts.is_empty() {
            return None;
        }
        // Pick the (height, hash) pair with the most peers
        // Break ties by: most peers first, then highest height
        let ((_, hash), count) = pair_counts.into_iter().max_by_key(|((h, _), c)| (*c, *h))?;
        // Require at least 2 peers to agree (prevents single-peer poisoning)
        if count >= 2 {
            Some(hash)
        } else {
            None
        }
    }

    /// Get a peer to sync from, with load distribution.
    ///
    /// INC-I-014: The old implementation always picked the absolute best peer (highest
    /// height), causing ALL syncing nodes to hammer the same peer (usually seed).
    /// At 120+ nodes, this created a serial bottleneck where sync took exponentially
    /// longer because the seed's event loop was saturated with sync requests.
    ///
    /// Fix: collect all peers that are ahead of us, then randomly select one.
    /// This distributes sync load across all already-synced peers.
    pub(in crate::sync::manager) fn best_peer(&self) -> Option<PeerId> {
        // CRITICAL: Only consider peers with MORE BLOCKS (higher height).
        // Slot comparison alone is dangerous — see should_sync() comment.
        // Skip peers that recently returned empty headers (cooldown).
        let eligible: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(pid, status)| {
                status.best_height > self.local_height
                    && !self.fork.header_blacklisted_peers.contains_key(pid)
            })
            .map(|(peer, _)| *peer)
            .collect();

        if eligible.is_empty() {
            return None;
        }

        // Distribute load: pick a pseudo-random eligible peer seeded by local_height
        // AND sync_epoch. Without sync_epoch, all nodes at the same height compute
        // the identical index — 7 nodes at h=1 all hammer the same peer, saturating
        // its rate limit (INC-I-017). sync_epoch varies per node (incremented each
        // sync cycle), breaking the thundering herd.
        let idx = (self.local_height as usize)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(self.pipeline.sync_epoch as usize)
            % eligible.len();
        Some(eligible[idx])
    }

    /// Start the sync process.
    ///
    /// Idempotent: if we're already actively syncing (DownloadingHeaders, DownloadingBodies,
    /// Processing, or snap syncing), this is a no-op. This prevents the 43-calls/sec destructive
    /// reset that occurs on large networks when every update_peer() status response triggers
    /// start_sync(), nuking in-flight requests each time (INC-I-004 / INC-I-005 root cause).
    pub(in crate::sync::manager) fn start_sync(&mut self) {
        // Guard: don't nuke in-flight sync state if already syncing.
        // Callers like update_peer() fire on every StatusResponse from every peer.
        // Without this guard, a 44-node network calls start_sync() ~43 times/sec,
        // each time incrementing sync_epoch and clearing all pending requests.
        if self.state.is_syncing() {
            return;
        }

        if let Some(peer) = self.best_peer() {
            let best_height = match self.peers.get(&peer) {
                Some(status) => status.best_height,
                None => return,
            };
            let target_slot = match self.peers.get(&peer) {
                Some(status) => status.best_slot,
                None => return,
            };

            // New epoch: any in-flight responses from previous cycles will be
            // discarded because their epoch won't match.
            self.pipeline.sync_epoch += 1;

            // P4: Preserve valid downloaded data on sync restart.
            // Instead of nuking ALL state (which destroyed 20-30K headers every 30s
            // under stress), check if pending_headers has a valid prefix that chains
            // from our current local_hash. Keep valid data, discard only stale/forked.
            let preserved = self.preserve_valid_sync_data();
            if !preserved {
                // No valid data — clean slate
                self.pipeline.header_downloader.clear();
                self.pipeline.pending_headers.clear();
                self.pipeline.pending_blocks.clear();
                self.pipeline.headers_needing_bodies.clear();
            }
            // Always clear stale request tracking and body downloader in-flight state
            self.pipeline.pending_requests.clear();
            self.pipeline.body_downloader.clear();
            self.pipeline.body_stall_retries = 0;
            for s in self.peers.values_mut() {
                s.pending_request = None;
            }

            // Snap sync decision: use snap sync when far behind and enough peers
            let gap = best_height.saturating_sub(self.local_height);
            let enough_peers = self.peers.len() >= 3;
            let snap_allowed = self.snap.threshold < u64::MAX;
            let should_snap = enough_peers
                && self.snap.attempts < 3
                && snap_allowed
                && (self.local_height == 0 || gap > self.snap.threshold);

            // Fresh node optimization: don't start slow header-first sync.
            // Wait for 5 peers so snap sync can activate — it downloads state
            // in seconds instead of replaying 60K+ blocks over hours.
            // BUT: timeout after 60s to avoid deadlock when <5 peers are discoverable.
            if self.local_height == 0
                && !enough_peers
                && self.snap.attempts < 3
                && snap_allowed
                && gap > self.snap.threshold
            {
                let wait_start = self
                    .snap
                    .fresh_node_wait_start
                    .get_or_insert(Instant::now());
                let waited = wait_start.elapsed();
                if waited.as_secs() < 60 {
                    info!(
                        "[SNAP_SYNC] Fresh node: waiting for {} more peer(s) for snap sync ({}/3, gap={}, waited={}s)",
                        3 - self.peers.len(), self.peers.len(), gap, waited.as_secs()
                    );
                    return;
                }
                warn!(
                    "[SNAP_SYNC] Fresh node waited {}s for 5 peers but only have {} — falling back to header-first sync",
                    waited.as_secs(), self.peers.len()
                );
                self.snap.fresh_node_wait_start = None;
            }

            // Discv5 peer discovery grace: when discv5 is discovering peers via UDP,
            // wait for enough TCP connections before committing to header-first sync.
            // Without this, nodes N19/N22 start header-first before discv5 discovers
            // peers that could serve snap sync (5-30s for first random walk).
            if !enough_peers && self.snap.attempts < 3 && snap_allowed && gap > self.snap.threshold
            {
                if let Some(deadline) = self.snap.discv5_peer_grace_deadline {
                    if Instant::now() < deadline {
                        info!(
                            "[SNAP_SYNC] Discv5 grace: waiting for {} more peer(s) ({}/3, gap={}, grace_remaining={}s)",
                            3 - self.peers.len(),
                            self.peers.len(),
                            gap,
                            deadline.saturating_duration_since(Instant::now()).as_secs()
                        );
                        return;
                    }
                    // Grace expired — clear it and fall through to header-first
                    self.snap.discv5_peer_grace_deadline = None;
                    warn!(
                        "[SNAP_SYNC] Discv5 grace expired with only {} peers — falling back to header-first sync",
                        self.peers.len()
                    );
                }
            }

            if should_snap {
                // FIX: Use the hash that the MAJORITY of eligible peers agree on,
                // not a single peer's hash. This prevents a partitioned minority
                // (e.g. 3 forked nodes on one server) from steering snap sync
                // to the wrong chain. If no majority exists, fall back to
                // header-first sync — the network is too fragmented for snap sync.
                let target_hash = match self.consensus_target_hash() {
                    Some(hash) => hash,
                    None => {
                        warn!(
                            "[SNAP_SYNC] No majority best_hash among {} peers — network too fragmented for snap sync, falling back to header-first",
                            self.peers.len()
                        );
                        // Fall through to header-first sync below
                        let target_slot = match self.peers.get(&peer) {
                            Some(status) => status.best_slot,
                            None => return,
                        };
                        info!(
                            "Starting sync epoch {} with peer {} (target_slot={})",
                            self.pipeline.sync_epoch, peer, target_slot
                        );
                        self.pipeline.header_downloader.clear();
                        self.set_syncing(
                            SyncPhase::DownloadingHeaders,
                            SyncPipelineData::Headers {
                                target_slot,
                                peer,
                                headers_count: 0,
                            },
                            "snap_no_majority_fallback",
                        );
                        return;
                    }
                };

                info!(
                    "[SNAP_SYNC] Starting snap sync epoch {} — gap={}, target_height={}, target_hash={:.16} (majority-preferred)",
                    self.pipeline.sync_epoch, gap, best_height, target_hash
                );

                self.set_syncing(
                    SyncPhase::SnapCollecting,
                    SyncPipelineData::SnapCollecting {
                        target_hash,
                        target_height: best_height,
                        votes: Vec::new(),
                        asked: HashSet::new(),
                    },
                    "start_snap_sync",
                );
            } else {
                let preserved_count = self.pipeline.pending_headers.len();
                info!(
                    "Starting sync epoch {} with peer {} (target_slot={}, preserved_headers={})",
                    self.pipeline.sync_epoch, peer, target_slot, preserved_count
                );

                self.set_syncing(
                    SyncPhase::DownloadingHeaders,
                    SyncPipelineData::Headers {
                        target_slot,
                        peer,
                        headers_count: preserved_count,
                    },
                    "start_header_sync",
                );
            }

            // Reset the stuck-sync timer so this new attempt gets a full window
            // before cleanup() declares it stuck.
            self.network.last_sync_activity = Instant::now();
            self.network.last_block_applied = Instant::now();
        }
    }
}
