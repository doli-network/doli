//! Peer management — add/remove/update peers, network tip tracking, gossip bookkeeping

use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;

use super::{PeerSyncStatus, SyncManager, SyncPipelineData, SyncState};
use crate::protocols::SyncRequest;

impl SyncManager {
    // =========================================================================
    // PEER MANAGEMENT
    // =========================================================================

    /// Register a new peer
    pub fn add_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        info!("Adding peer {} with height {}, slot {}", peer, height, slot);

        // When transitioning from 0 peers to non-zero, signal the node layer
        // to request fresh status from all peers immediately (not wait 30s).
        if self.peers.is_empty() {
            self.needs_mass_status_refresh = true;
        }
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

    /// Check if a peer is known to the sync manager
    pub fn has_peer(&self, peer: &PeerId) -> bool {
        self.peers.contains_key(peer)
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

    /// Post-apply catch-up: if we're behind a peer by one or more blocks,
    /// return a single-block pull request for the next height.
    ///
    /// This is the invariant: "after each apply, if anyone has more, ask for it."
    /// No timers, no thresholds, no new state — the method is `&self` and purely
    /// derived from the current peer map, local height, and pipeline state.
    ///
    /// Returns None when:
    /// - We are actively syncing (full sync pipeline handles it)
    /// - We are caught up (local_height >= best_peer_height)
    /// - We already have a pending GetBlockByHeight for local_height+1
    /// - No peer has a usable higher tip
    ///
    /// The returned request is NOT registered in pending_requests — the response
    /// flows through the normal block handling path, which treats untracked
    /// SyncResponse::Block as legitimate (see response.rs:34 comment).
    pub fn catch_up_request(&self) -> Option<(PeerId, SyncRequest)> {
        // Don't interfere with an active full-sync pipeline.
        if self.state.is_syncing() {
            return None;
        }

        let target_height = self.local_height + 1;

        // If we already sent a catch-up for this height, don't duplicate.
        if self
            .pipeline
            .pending_requests
            .values()
            .any(|r| matches!(r.request, SyncRequest::GetBlockByHeight { height } if height == target_height))
        {
            return None;
        }

        // Pick any peer whose advertised tip is higher than our local and who
        // has no outstanding request from us (to spread load and avoid queueing
        // behind a header-sync request).
        let peer = self
            .peers
            .iter()
            .filter(|(_, s)| s.best_height > self.local_height && s.pending_request.is_none())
            .max_by_key(|(_, s)| s.best_height)
            .map(|(pid, _)| *pid)?;

        Some((
            peer,
            SyncRequest::GetBlockByHeight {
                height: target_height,
            },
        ))
    }

    /// Get the peer with the highest height and their best_hash.
    /// Used by stale tip recovery to request a specific missing block.
    pub fn best_peer_with_hash(&self) -> Option<(PeerId, u64, crypto::Hash)> {
        self.peers
            .iter()
            .max_by_key(|(_, s)| s.best_height)
            .map(|(pid, s)| (*pid, s.best_height, s.best_hash))
    }

    /// Check if we're on a minority fork. Returns true if >66% of peers are
    /// at or above our height — meaning they've moved past us on a different
    /// chain. Peers at our exact height with a different hash are direct
    /// evidence. Peers above us who aren't sending us blocks that chain on
    /// our tip are indirect evidence (they passed through our height on a
    /// different fork).
    pub fn is_minority_fork(&self, local_height: u64) -> bool {
        let total = self.peers.len();
        if total < 3 {
            return false;
        }

        let peers_ahead = self
            .peers
            .values()
            .filter(|s| s.best_height > local_height)
            .count();

        peers_ahead > (total * 2) / 3
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
        let mut counted = 0;
        // Track unique chain tips only among non-agreeing peers to detect real forks.
        // Peers that match our canonical chain at their height are on the same chain
        // regardless of which specific height they've reported.
        let mut divergent_hashes = std::collections::HashSet::new();

        for status in self.peers.values() {
            // Skip peers stuck at height 0 when chain is past genesis — these are
            // stale connections whose status was never updated (the status protocol
            // only reports height at initial handshake). They're not real forks.
            if status.best_height == 0 && self.local_height > 10 {
                continue;
            }
            counted += 1;
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

        // Use counted (excluding stale h=0 peers) as the peer count for health.
        (counted, agreeing, unique_chain_tips)
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

    /// Notify sync manager that a gossip block was orphaned (parent ≠ local tip).
    /// Orphan gossip blocks are direct evidence the node is behind the network.
    /// After 3+ consecutive orphans, force batch sync to close the gap.
    pub fn note_orphan_gossip_block(&mut self, block_height: u64, block_slot: u32) {
        self.consecutive_orphan_gossip_blocks += 1;
        if block_height > self.network.network_tip_height {
            self.network.network_tip_height = block_height;
        }
        if block_slot > self.network.network_tip_slot {
            self.network.network_tip_slot = block_slot;
        }

        // Recovery Coordinator phase 2 (2026-04-15, synmgrefactor): shadow report.
        // Detectors feed evidence into the coordinator alongside their legacy
        // action. Coordinator runs in observation-only mode until phase 3 flip.
        let gap_for_report = self
            .network
            .network_tip_height
            .saturating_sub(self.local_height);
        self.recovery
            .report(super::recovery::RecoveryEvidence::OrphanGossip {
                slot: block_slot,
                gap: gap_for_report,
            });

        if self.consecutive_orphan_gossip_blocks >= 3 {
            let gap = self
                .network
                .network_tip_height
                .saturating_sub(self.local_height);
            let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized);
            if state_ok && gap >= 1 {
                // Fix #2b (2026-04-15): anti-cascade orphan path. If we applied
                // a block recently (within 60s) AND the gap is small (<50),
                // accumulating orphans means we're stuck on a minority fork —
                // peers are extending a different chain. start_sync() here
                // requests headers that won't link to our tip, eventually
                // escalating to snap sync (the 2026-04-15 deploy incident:
                // N1/N2/N3/seed1 ended on minority fork at h=33505, snap
                // sync triggered 2 minutes later, requiring manual rsync).
                //
                // Trigger signal_stuck_fork instead → resolve_shallow_fork
                // rolls back the divergent block(s) and retries from the
                // common ancestor. Recovery in seconds, no snap sync, no
                // block_store gaps.
                let recently_synced =
                    self.network.last_block_applied.elapsed() < Duration::from_secs(60);
                if recently_synced && gap < 50 {
                    // Fix #2b-bis (2026-04-15, synmgrefactor): discriminate
                    // "on minority fork" vs "behind canonical".
                    //
                    // If we've applied at least one block PAST the height we
                    // rolled back to, the prior rollback succeeded — we
                    // reconnected to canonical. Further orphan accumulation
                    // means peers have advanced beyond us (we're BEHIND), and
                    // rolling back again would make us MORE behind, not less.
                    // This was the folsi cascade on 2026-04-15 14:52-14:56:
                    // 25 consecutive rollbacks while peers kept advancing,
                    // gap growing from 2 to 50+.
                    //
                    // When proven post-rollback, fall through to the normal
                    // start_sync() path below → header-first sync catches up.
                    if let Some(rb_h) = self.fork.last_rollback_local_height {
                        if self.local_height > rb_h {
                            warn!(
                                "[SYNC] {} orphan gossip blocks (local_h={}, tip_h={}, gap={}) — \
                                 applied since last rollback (rb_h={}) → BEHIND not forked. \
                                 Suppressing stuck_fork signal, running normal sync.",
                                self.consecutive_orphan_gossip_blocks,
                                self.local_height,
                                self.network.network_tip_height,
                                gap,
                                rb_h
                            );
                            self.consecutive_orphan_gossip_blocks = 0;
                            self.start_sync();
                            return;
                        }
                    }

                    // Guard: if last block was applied <30s ago, we're on a
                    // live chain tip — orphans are from future blocks we haven't
                    // received yet, not evidence of a fork. Rolling back a fresh
                    // canonical block destroys good state and creates a real fork
                    // (2026-04-16 N4 h=2566 incident: rollback of slot 2572 →
                    // applied worse slot 2576 → stuck permanently).
                    let secs_since_apply = self.network.last_block_applied.elapsed().as_secs();
                    if secs_since_apply < 60 {
                        warn!(
                            "[SYNC] {} orphan gossip blocks (local_h={}, tip_h={}, gap={}) — \
                             last apply {}s ago (< 60s), suppressing rollback. \
                             Running normal sync instead.",
                            self.consecutive_orphan_gossip_blocks,
                            self.local_height,
                            self.network.network_tip_height,
                            gap,
                            secs_since_apply
                        );
                        self.consecutive_orphan_gossip_blocks = 0;
                        self.start_sync();
                        return;
                    }

                    warn!(
                        "[SYNC] {} orphan gossip blocks (local_h={}, tip_h={}, gap={}) — \
                         stale tip ({}s since last apply). \
                         Signaling stuck fork for shallow rollback (anti-cascade-orphan).",
                        self.consecutive_orphan_gossip_blocks,
                        self.local_height,
                        self.network.network_tip_height,
                        gap,
                        secs_since_apply
                    );
                    self.consecutive_orphan_gossip_blocks = 0;
                    self.signal_stuck_fork();
                    return;
                }

                warn!(
                    "[SYNC] {} consecutive orphan gossip blocks — forcing sync (local_h={}, tip_h={}, gap={})",
                    self.consecutive_orphan_gossip_blocks, self.local_height, self.network.network_tip_height, gap
                );
                self.consecutive_orphan_gossip_blocks = 0;
                self.start_sync();
            }
        }
    }

    /// Reset orphan gossip counter (called when a block is successfully applied).
    pub fn reset_orphan_counter(&mut self) {
        self.consecutive_orphan_gossip_blocks = 0;
    }
}
