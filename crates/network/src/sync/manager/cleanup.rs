//! Cleanup — stale request/peer removal, stuck sync recovery, stall detection

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{info, warn};

use super::{SyncManager, SyncState};

impl SyncManager {
    /// Clean up stale requests and peers
    pub fn cleanup(&mut self) {
        let now = Instant::now();

        // Log current state for debugging
        let pending_count = self
            .peers
            .values()
            .filter(|s| s.pending_request.is_some())
            .count();
        info!(
            "[SYNC_DEBUG] Cleanup: state={:?}, peers={}, pending_peer_requests={}, pending_requests={}",
            self.state,
            self.peers.len(),
            pending_count,
            self.pending_requests.len()
        );

        // Clean up timed out body download requests
        // This moves timed-out hashes back to the failed queue for retry
        self.body_downloader.cleanup_timeouts();

        // Drain cancelled fork recovery peers into fork_sync_blacklist
        self.drain_cancelled_recovery_peer();

        // Remove timed out requests
        let timed_out: Vec<super::SyncRequestId> = self
            .pending_requests
            .iter()
            .filter(|(_, req)| now.duration_since(req.sent_at) > self.config.request_timeout)
            .map(|(id, _)| *id)
            .collect();

        for id in timed_out {
            if let Some(req) = self.pending_requests.remove(&id) {
                warn!(
                    "[SYNC_DEBUG] Request timeout: id={}, peer={}, elapsed={:?}, request={:?}",
                    id.0,
                    req.peer,
                    now.duration_since(req.sent_at),
                    req.request
                );
                if let Some(status) = self.peers.get_mut(&req.peer) {
                    status.pending_request = None;
                }
            }
        }

        // Remove stale peers
        let stale: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(_, status)| {
                now.duration_since(status.last_status_response) > self.config.stale_timeout
            })
            .map(|(peer, _)| *peer)
            .collect();

        for peer in stale {
            warn!("Removing stale peer {}", peer);
            self.remove_peer(&peer);
        }

        // Checkpoint download timeout: if downloading checkpoint state for >30s, try another peer
        if let SyncState::CheckpointDownloading { peer, started_at } = &self.state {
            if started_at.elapsed() > Duration::from_secs(30) {
                warn!(
                    "[CHECKPOINT] Download from peer {} timed out after 30s — resetting to Idle",
                    peer
                );
                self.state = SyncState::Idle;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }

        // Stall recovery: if "Synchronized" but significantly behind in slots,
        // we're in a deadlock (height matches but slot lags). Reset to Idle
        // to allow re-evaluation and potential resync.
        if matches!(self.state, SyncState::Synchronized) && !self.peers.is_empty() {
            let best_slot = self.best_peer_slot();
            let slot_lag = best_slot.saturating_sub(self.local_slot);
            let stall_threshold = self.max_slots_behind.saturating_mul(5);
            if slot_lag > stall_threshold {
                warn!(
                    "Stall detected: Synchronized but {} slots behind peers (threshold {}). Resetting to Idle for recovery.",
                    slot_lag, stall_threshold
                );
                self.state = SyncState::Idle;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }

        // Escape stuck sync states: if we're syncing and no sync progress for 30s,
        // either soft-retry the missing bodies or hard-reset.
        //
        // We use `last_sync_activity` which tracks ALL phases of header-first sync:
        // headers received, bodies received, and blocks applied. Previously this used
        // `last_block_applied` which only tracked the final phase, causing false "stuck"
        // detection during the header download phase (no blocks applied yet, but headers
        // were streaming in). This nuked 20-30K downloaded headers every 30s.
        // Bodies are much larger than headers and N4 may be serving 4 peers
        // simultaneously — give body download more time before declaring stuck.
        let stuck_threshold = if matches!(self.state, SyncState::DownloadingBodies { .. }) {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(30)
        };
        if self.state.is_syncing() && self.last_sync_activity.elapsed() > stuck_threshold {
            let was_processing = matches!(self.state, SyncState::Processing { .. });
            let is_downloading_bodies = matches!(self.state, SyncState::DownloadingBodies { .. });
            let have_pending_headers = !self.pending_headers.is_empty();
            let have_downloaded_bodies = !self.pending_blocks.is_empty();

            // Soft recovery for the body-downloader stall:
            // Bodies are arriving but not contiguous from our tip (parallel
            // peer responses arrived out of order). Keep pending_headers and
            // pending_blocks, rebuild the needed-bodies list to contain only
            // what's actually missing, and let the body downloader retry.
            if is_downloading_bodies
                && have_pending_headers
                && have_downloaded_bodies
                && !was_processing
                && self.body_stall_retries < 3
            {
                self.body_stall_retries += 1;

                // Rebuild headers_needing_bodies: walk pending_headers from the
                // local tip, skip bodies we already have, enqueue the rest.
                let mut rebuilt = VecDeque::new();
                let mut current = self.local_hash;
                let mut gap_found = false;

                for header in &self.pending_headers {
                    if header.prev_hash != current {
                        gap_found = true;
                    }

                    let h = header.hash();

                    if !gap_found {
                        if self.pending_blocks.contains_key(&h) {
                            current = h;
                            continue;
                        } else {
                            gap_found = true;
                        }
                    }

                    if !self.pending_blocks.contains_key(&h) {
                        rebuilt.push_back(h);
                    }
                    current = h;
                }

                self.headers_needing_bodies = rebuilt;

                // Unwedge request bookkeeping so the missing bodies can be
                // re-requested immediately (hashes may be stuck in in_flight).
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.pending_requests.clear();
                self.body_downloader.clear();

                // Reset timer and stay in DownloadingBodies.
                self.last_block_applied = Instant::now();
                self.last_sync_activity = Instant::now();
                let total = self.pending_headers.len();
                let pending = self.headers_needing_bodies.len();
                self.state = SyncState::DownloadingBodies { pending, total };

                warn!(
                    "Body stall retry {}/3: retained {} headers + {} downloaded bodies; \
                     re-requesting {} missing bodies from h={}",
                    self.body_stall_retries,
                    total,
                    self.pending_blocks.len(),
                    pending,
                    self.local_height + 1
                );
            } else if is_downloading_bodies && have_pending_headers {
                // Body download exhausted all soft retries. Reset to Idle so
                // start_sync() can re-evaluate: if the gap is still >1000 blocks
                // with 3+ peers, snap sync will activate instead of repeating
                // the slow header-first path that already failed.
                warn!(
                    "Body download stuck after {}/3 retries — resetting to Idle \
                     (gap={}, local_h={}, snap sync will re-evaluate)",
                    self.body_stall_retries,
                    self.network_tip_height.saturating_sub(self.local_height),
                    self.local_height
                );

                // Full reset — let start_sync() decide header-first vs snap sync
                self.pending_headers.clear();
                self.pending_blocks.clear();
                self.headers_needing_bodies.clear();
                self.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.body_downloader.clear();
                self.body_stall_retries = 0;
                self.header_downloader.clear();

                self.state = SyncState::Idle;
                self.last_sync_activity = Instant::now();
                if self.should_sync() {
                    self.start_sync();
                }
            } else {
                // Hard reset — only for header download failures or processing stuck.
                warn!(
                    "Sync stuck in {:?} for >30s with no progress \
                     (last_activity={:.0?} ago, local_h={}, network_tip={}) — resetting to Idle",
                    self.state,
                    self.last_sync_activity.elapsed(),
                    self.local_height,
                    self.network_tip_height
                );
                self.state = SyncState::Idle;
                self.pending_headers.clear();
                self.pending_blocks.clear();
                self.headers_needing_bodies.clear();
                self.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.body_downloader.clear();
                self.body_stall_retries = 0;
                // Reset header downloader so next sync starts from local_hash,
                // not from a stale expected_prev_hash from a previous (failed) cycle.
                self.header_downloader.clear();
                // If stuck in Processing, count toward deep fork detection.
                // The node downloaded a chain it can't apply — this is fork evidence.
                if was_processing {
                    self.consecutive_empty_headers += 1;
                    info!(
                        "[SYNC] STUCK_PROCESSING empties={} — counted as fork signal",
                        self.consecutive_empty_headers
                    );
                }
            }
        }

        // Expire stale header blacklist entries (30s cooldown — reduced from 60s
        // so peers become available between stuck-sync timeout cycles)
        self.header_blacklisted_peers
            .retain(|_, added| added.elapsed() < Duration::from_secs(30));

        // Expire fork sync blacklist entries (5-minute TTL).
        // These peers were temporarily excluded from fork recovery peer selection
        // because they were syncing or missing blocks. After 5 minutes they may
        // have caught up and become useful again.
        self.fork_sync_blacklist
            .retain(|_, added| added.elapsed() < Duration::from_secs(300));

        // NT8 fix: When ALL peers are blacklisted and we've been stuck for >120s
        // total, clear the blacklist entirely to retry with a fresh slate.
        // Without this, the node cycles Idle↔DownloadingHeaders forever because
        // blacklisted peers never expire before the next stuck-sync timeout.
        if !self.header_blacklisted_peers.is_empty()
            && self.best_peer().is_none()
            && self.should_sync()
            && matches!(self.state, SyncState::Idle)
        {
            let stuck_duration = self.last_block_applied.elapsed();
            if stuck_duration > Duration::from_secs(120) {
                if self.consecutive_empty_headers >= 20 {
                    // Genuinely on a dead fork: many consecutive empties + stuck for 2+ min.
                    // Redirect to fork_sync/rollback to find common ancestor.
                    let gap = self.network_tip_height.saturating_sub(self.local_height);
                    warn!(
                        "All peers blacklisted for >120s with {} consecutive empty headers (gap={}) \
                         — clearing blacklist and triggering fork_sync",
                        self.consecutive_empty_headers, gap
                    );
                    self.header_blacklisted_peers.clear();
                    self.consecutive_empty_headers = 3; // Trigger fork_sync
                } else {
                    // Temporary gossip hiccup — clear blacklist and retry normally.
                    warn!(
                        "All peers blacklisted for >120s (consecutive_empty_headers={}) — \
                         clearing blacklist for fresh retry",
                        self.consecutive_empty_headers
                    );
                    self.header_blacklisted_peers.clear();
                }
            }
        }

        // Periodic sync retry: if Idle and behind peers, restart sync.
        // This catches cases where sync was attempted, failed (e.g., empty headers
        // from gossip race), reset to Idle, and never retried.
        // IMPORTANT: Skip retry when fork_sync is active — its binary search runs
        // across multiple ticks and resetting to Idle interrupts it, causing infinite loops.
        if matches!(self.state, SyncState::Idle)
            && self.should_sync()
            && !self.is_fork_sync_active()
        {
            let gap = self.network_tip_height.saturating_sub(self.local_height);
            if gap <= 5 && gap > 0 {
                // Small gap: increment a retry counter. If we've been stuck in
                // Idle-but-behind for multiple ticks (5+), escalate to a full
                // start_sync which transitions to header download and forces
                // the pipeline to request blocks actively.
                self.idle_behind_retries = self.idle_behind_retries.saturating_add(1);
                if self.idle_behind_retries >= 5 {
                    info!(
                        "Sync retry: small gap ({} blocks) but stuck for {} ticks — forcing full sync restart.",
                        gap, self.idle_behind_retries
                    );
                    self.idle_behind_retries = 0;
                    self.start_sync();
                } else {
                    info!(
                        "Sync retry: Idle but behind peers (local_h={}, network_tip={}, gap={}, retry={}/5). Waiting for gossip.",
                        self.local_height, self.network_tip_height, gap, self.idle_behind_retries
                    );
                }
            } else {
                self.idle_behind_retries = 0;
                info!(
                    "Sync retry: Idle but behind peers (local_h={}, network_tip={}). Restarting sync.",
                    self.local_height, self.network_tip_height
                );
                self.start_sync();
            }
        } else if !matches!(self.state, SyncState::Idle) || !self.should_sync() {
            // Reset retry counter when no longer stuck
            self.idle_behind_retries = 0;
        }

        // Post-recovery grace timeout: if 120s have passed since snap sync
        // and grace hasn't cleared (10 blocks not applied), force-clear it.
        // This prevents grace from permanently blocking fork_sync when the node
        // landed on a fork after snap sync (can't apply blocks → grace never clears).
        if self.post_recovery_grace && self.post_recovery_grace_started.elapsed().as_secs() > 120 {
            warn!(
                "Post-recovery grace timeout: 120s elapsed with only {} blocks applied. \
                 Force-clearing to allow fork recovery.",
                self.blocks_applied_since_recovery
            );
            self.post_recovery_grace = false;
            self.blocks_applied_since_recovery = 0;
        }

        // Stuck-sync detection: if height hasn't advanced for >120s and we're
        // behind peers, the node is stuck. Escalate to fork_sync.
        if !self.is_fork_sync_active() && self.should_sync() {
            let gap = self.network_tip_height.saturating_sub(self.local_height);
            let stuck_secs = self.last_block_applied.elapsed().as_secs();
            if gap > 0
                && stuck_secs > 120
                && self.consecutive_empty_headers < 3
                && self.local_height > 0
            {
                warn!(
                    "Stuck-on-fork detected: no block applied for {}s, behind by {} blocks \
                     (local_h={}, network_tip={}). Escalating to fork_sync.",
                    stuck_secs, gap, self.local_height, self.network_tip_height
                );
                self.consecutive_empty_headers = 3;
            }
        }
    }
}
