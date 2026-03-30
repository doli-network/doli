//! Peer management — add/remove/update peers, network tip tracking, gossip bookkeeping

use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;

use super::{PeerSyncStatus, SyncManager, SyncPipelineData, SyncState};

impl SyncManager {
    // =========================================================================
    // PEER MANAGEMENT
    // =========================================================================

    /// Register a new peer
    pub fn add_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        info!("Adding peer {} with height {}, slot {}", peer, height, slot);

        // Clear peer loss tracker since we have a peer again
        self.peers_lost_at = None;

        self.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: height,
                best_hash: hash,
                best_slot: slot,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        // NETWORK TIP FROM PEER STATUS: Update network tip based on peer claims
        // This is critical for production gating - even if we haven't received the
        // actual block via gossip yet, knowing that a peer claims a higher height
        // tells us we shouldn't produce until we're caught up.
        if height > self.network.network_tip_height {
            debug!(
                "Network tip height updated from peer status: {} -> {}",
                self.network.network_tip_height, height
            );
            self.network.network_tip_height = height;
        }
        if slot > self.network.network_tip_slot {
            debug!(
                "Network tip slot updated from peer status: {} -> {}",
                self.network.network_tip_slot, slot
            );
            self.network.network_tip_slot = slot;
        }

        // Check if we should start syncing
        // Note: Also check Synchronized state - after successful sync, state is Synchronized,
        // and we need to re-sync if peers advance beyond us
        // CHECKPOINT: Sync trigger check in add_peer
        let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized);
        if state_ok && self.should_sync() {
            self.start_sync();
        }
    }

    /// Update peer status
    pub fn update_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        if let Some(status) = self.peers.get_mut(&peer) {
            status.best_height = height;
            status.best_hash = hash;
            status.best_slot = slot;
            status.last_status_response = Instant::now();
        }

        // Also update network tip from peer status (same as add_peer)
        if height > self.network.network_tip_height {
            self.network.network_tip_height = height;
        }
        if slot > self.network.network_tip_slot {
            self.network.network_tip_slot = slot;
        }

        // Check if we should start syncing (same as add_peer)
        // This ensures we re-sync when peers advance beyond our height
        // Note: Also check Synchronized state - after successful sync, state is Synchronized,
        // and we need to re-sync if peers advance beyond us
        let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized);
        if state_ok && self.should_sync() {
            self.start_sync();
        }
    }

    /// Refresh all peers' timestamps when activity is detected on the network
    /// Call this when receiving blocks/transactions via gossip to prevent stale timeouts
    pub fn refresh_all_peers(&mut self) {
        let now = Instant::now();
        for status in self.peers.values_mut() {
            status.last_status_response = now;
            status.last_block_received = Some(now); // Gossip proves both liveness and data flow
        }
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);

        // FIX: Recompute network_tip_height from remaining peers + local height.
        // Without this, a peer that briefly reported an inflated height (e.g.,
        // during a fork) permanently inflates network_tip_height, creating a
        // phantom gap that triggers unnecessary sync/snap sync (Path E cascade).
        let peer_max_height = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        let peer_max_slot = self.peers.values().map(|p| p.best_slot).max().unwrap_or(0);
        self.network.network_tip_height = peer_max_height.max(self.local_height);
        self.network.network_tip_slot = peer_max_slot.max(self.local_slot);

        // Track when we lost all peers (for peer loss timeout)
        if self.peers.is_empty() && self.peers_lost_at.is_none() {
            info!(
                "All peers lost — starting peer loss timeout ({}s)",
                self.peer_loss_timeout_secs
            );
            self.peers_lost_at = Some(Instant::now());
        }

        // Release body downloader hashes back to failed queue so they
        // can be re-requested from another peer. Without this, hashes
        // stay in in_flight forever and the body downloader stalls.
        self.pipeline.body_downloader.cancel_peer(peer);

        // Cancel any pending requests from this peer
        self.pipeline
            .pending_requests
            .retain(|_, req| &req.peer != peer);

        // If we were syncing from this peer, try another
        if let SyncPipelineData::Headers {
            peer: sync_peer, ..
        } = &self.pipeline_data
        {
            if sync_peer == peer {
                self.set_state(SyncState::Idle, "header_peer_disconnected");
                self.pipeline_data = SyncPipelineData::None;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }

        // If snap downloading from this peer, try alternate or fall back
        if let SyncPipelineData::SnapDownloading {
            peer: snap_peer, ..
        } = &self.pipeline_data
        {
            if snap_peer == peer {
                warn!(
                    "[SNAP_SYNC] Download peer {} disconnected — trying alternate",
                    peer
                );
                self.handle_snap_download_error(*peer);
            }
        }
    }

    /// Get the number of connected peers with known status
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Get an iterator over all connected peer IDs
    pub fn peer_ids(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.peers.keys().copied()
    }

    /// Get the best (highest) height among all connected peers AND network gossip
    /// This considers both individual peer statuses and blocks received via gossip
    /// Returns 0 if no network data is available
    pub fn best_peer_height(&self) -> u64 {
        let peer_max = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        // Return the higher of peer data or network gossip tip
        peer_max.max(self.network.network_tip_height)
    }

    /// Get the LOWEST height among all connected peers
    /// Used for fork detection: if we're far ahead of ANY peer, something is wrong
    /// Returns None if no peers (can't determine lowest)
    pub fn lowest_peer_height(&self) -> Option<u64> {
        self.peers.values().map(|p| p.best_height).min()
    }

    /// Get the best (highest) slot among all connected peers AND network gossip
    /// This considers both individual peer statuses and blocks received via gossip
    /// Returns 0 if no network data is available
    pub fn best_peer_slot(&self) -> u32 {
        // Only use actual peer status data, not gossip-inflated network_tip_slot.
        // network_tip_slot can be permanently inflated by orphan/fork blocks
        // received via gossip before validation.
        self.peers.values().map(|p| p.best_slot).max().unwrap_or(0)
    }

    /// Get a health summary for checkpoint tagging.
    ///
    /// Returns (peer_count, peers_agreeing_with_local, total_unique_hashes).
    /// If peers_agreeing == peer_count and unique_hashes == 1, the network is healthy.
    ///
    /// Compares each peer's `best_hash` against our canonical hash **at the
    /// peer's reported height** (via `recent_canonical_hashes` ring buffer).
    /// This tolerates the normal status-protocol lag where peers report a hash
    /// 1-2 blocks behind our tip, while still detecting real forks (different
    /// hash at the same height).
    pub fn checkpoint_health(&self) -> (usize, usize, usize) {
        let peer_count = self.peers.len();
        if peer_count == 0 {
            return (0, 0, 0);
        }

        let mut agreeing = 0;
        // Track unique chain tips only among non-agreeing peers to detect real forks.
        // Peers that match our canonical chain at their height are on the same chain
        // regardless of which specific height they've reported.
        let mut divergent_hashes = std::collections::HashSet::new();

        for status in self.peers.values() {
            // Look up our canonical hash at the peer's reported height
            let our_hash_at_peer_height = self
                .recent_canonical_hashes
                .iter()
                .find(|(h, _)| *h == status.best_height)
                .map(|(_, hash)| *hash);

            match our_hash_at_peer_height {
                Some(our_hash) if our_hash == status.best_hash => {
                    // Peer is on our chain, just behind our tip
                    agreeing += 1;
                }
                Some(_) => {
                    // Same height, different hash = real fork
                    divergent_hashes.insert(status.best_hash);
                }
                None => {
                    // Peer's height not in our ring buffer (>200 blocks behind
                    // or ahead of us). Conservative: count as not agreeing.
                    divergent_hashes.insert(status.best_hash);
                }
            }
        }

        // unique_chain_tips: 1 (ours) + number of distinct divergent hashes.
        // If all peers agree, divergent_hashes is empty → 1 tip total.
        let unique_chain_tips = if divergent_hashes.is_empty() {
            1
        } else {
            1 + divergent_hashes.len()
        };

        (peer_count, agreeing, unique_chain_tips)
    }

    /// Update the network tip slot from a received block via gossip
    ///
    /// This should be called when receiving blocks from gossip, before applying them.
    /// Unlike `update_peer()`, this doesn't require knowing which peer sent the block.
    /// It tracks "what slot the network has reached" based on blocks we've seen.
    ///
    /// Note: Height is updated through the normal block application path (update_local_tip),
    /// since blocks don't directly contain their height - it's computed from chain position.
    pub fn update_network_tip_slot(&mut self, slot: u32) {
        if slot > self.network.network_tip_slot {
            debug!(
                "Network tip slot updated from gossip: {} -> {}",
                self.network.network_tip_slot, slot
            );
            self.network.network_tip_slot = slot;
        }
    }

    /// Update network tip height when we successfully apply a block
    /// This is called after block application, not from gossip
    pub fn update_network_tip_height(&mut self, height: u64) {
        if height > self.network.network_tip_height {
            self.network.network_tip_height = height;
        }
    }

    /// Get current network tip (from gossip and applied blocks)
    pub fn network_tip(&self) -> (u64, u32) {
        (
            self.network.network_tip_height,
            self.network.network_tip_slot,
        )
    }

    /// Note that we received a block via gossip network (P0 #3)
    pub fn note_block_received_via_gossip(&mut self) {
        self.network.last_block_seen = Instant::now();
        self.last_block_received_via_gossip = Some(Instant::now());
        // NOTE: We intentionally do NOT reset consecutive_empty_headers here.
        // Receiving gossip blocks proves the *network* is alive, but NOT that we're on
        // the canonical chain. If we're on a fork, we receive gossip blocks from the
        // canonical chain that we can't apply (orphans). Resetting the counter here
        // would prevent deep fork detection from ever triggering, leaving the node
        // permanently stuck on a dead fork.
    }

    /// Note that we received a block from a specific peer (P1 #5)
    pub fn note_block_received_from_peer(&mut self, peer_id: PeerId) {
        self.network.last_block_seen = Instant::now();
        if let Some(status) = self.peers.get_mut(&peer_id) {
            status.last_block_received = Some(Instant::now());
            // Implicitly, if they sent us a block, they are reachable
            status.last_status_response = Instant::now();
        }
    }

    /// Check if the chain is stale (no blocks received for `threshold` duration).
    /// Used by Node to detect stuck state and trigger re-sync.
    pub fn is_chain_stale(&self, threshold: Duration) -> bool {
        self.network.last_block_seen.elapsed() > threshold
    }
}
