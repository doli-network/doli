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
            self.pipeline.pending_requests.len()
        );

        // Clean up timed out body download requests
        // This moves timed-out hashes back to the failed queue for retry
        self.pipeline.body_downloader.cleanup_timeouts();

        // Remove timed out requests
        let timed_out: Vec<super::SyncRequestId> = self
            .pipeline
            .pending_requests
            .iter()
            .filter(|(_, req)| now.duration_since(req.sent_at) > self.config.request_timeout)
            .map(|(id, _)| *id)
            .collect();

        for id in timed_out {
            if let Some(req) = self.pipeline.pending_requests.remove(&id) {
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

        // Snap sync timeouts
        match &self.state {
            SyncState::SnapCollectingRoots { started_at, .. } => {
                if started_at.elapsed() > self.snap.root_timeout {
                    // Quorum wasn't reached in time. Instead of falling back to
                    // header-first (which always fails for nodes at h=0 or on a
                    // fork), pick the largest vote group with >= 2 peers.
                    // Safety: the node independently verifies the snapshot via
                    // compute_state_root_from_bytes() after download — quorum
                    // only selects WHICH chain to download, not trust.
                    if let Some((best_root, best_peers)) = self.pick_best_snap_group() {
                        let (download_peer, download_hash, best_height) = best_peers
                            .iter()
                            .max_by_key(|(_, _, h)| *h)
                            .copied()
                            .unwrap();
                        let alternate_peers: Vec<(PeerId, crypto::Hash, u64)> = best_peers
                            .iter()
                            .filter(|(pid, _, _)| *pid != download_peer)
                            .copied()
                            .collect();
                        warn!(
                            "[SNAP_SYNC] No quorum after {:?} — using best group: {} peers agree on root={:.16}, downloading from {} (height={})",
                            self.snap.root_timeout, best_peers.len(), best_root, download_peer, best_height
                        );
                        self.set_state(
                            SyncState::SnapDownloading {
                                target_hash: download_hash,
                                target_height: best_height,
                                quorum_root: best_root,
                                peer: download_peer,
                                alternate_peers,
                                started_at: Instant::now(),
                            },
                            "snap_root_timeout_best_group",
                        );
                    } else {
                        warn!(
                            "[SNAP_SYNC] State root collection timed out after {:?} — no group with >= 2 peers, falling back",
                            self.snap.root_timeout
                        );
                        self.snap_fallback_to_normal();
                    }
                }
            }
            SyncState::SnapDownloading {
                started_at, peer, ..
            } => {
                if started_at.elapsed() > self.snap.download_timeout {
                    let peer = *peer;
                    warn!(
                        "[SNAP_SYNC] Snapshot download from {} timed out after {:?} — trying alternate peer",
                        peer, self.snap.download_timeout
                    );
                    self.handle_snap_download_error(peer);
                }
            }
            _ => {}
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
                self.set_state(SyncState::Idle, "stall_synchronized_behind");
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
        if self.state.is_syncing() && self.network.last_sync_activity.elapsed() > stuck_threshold {
            let was_processing = matches!(self.state, SyncState::Processing { .. });
            let is_downloading_bodies = matches!(self.state, SyncState::DownloadingBodies { .. });
            let have_pending_headers = !self.pipeline.pending_headers.is_empty();
            let have_downloaded_bodies = !self.pipeline.pending_blocks.is_empty();

            // Soft recovery for the body-downloader stall:
            // Bodies are arriving but not contiguous from our tip (parallel
            // peer responses arrived out of order). Keep pending_headers and
            // pending_blocks, rebuild the needed-bodies list to contain only
            // what's actually missing, and let the body downloader retry.
            if is_downloading_bodies
                && have_pending_headers
                && have_downloaded_bodies
                && !was_processing
                && self.pipeline.body_stall_retries < 3
            {
                self.pipeline.body_stall_retries += 1;

                // Rebuild headers_needing_bodies: walk pending_headers from the
                // local tip, skip bodies we already have, enqueue the rest.
                let mut rebuilt = VecDeque::new();
                let mut current = self.local_hash;
                let mut gap_found = false;

                for header in &self.pipeline.pending_headers {
                    if header.prev_hash != current {
                        gap_found = true;
                    }

                    let h = header.hash();

                    if !gap_found {
                        if self.pipeline.pending_blocks.contains_key(&h) {
                            current = h;
                            continue;
                        } else {
                            gap_found = true;
                        }
                    }

                    if !self.pipeline.pending_blocks.contains_key(&h) {
                        rebuilt.push_back(h);
                    }
                    current = h;
                }

                self.pipeline.headers_needing_bodies = rebuilt;

                // Unwedge request bookkeeping so the missing bodies can be
                // re-requested immediately (hashes may be stuck in in_flight).
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.pipeline.pending_requests.clear();
                self.pipeline.body_downloader.clear();

                // Reset timer and stay in DownloadingBodies.
                self.network.last_block_applied = Instant::now();
                self.network.last_sync_activity = Instant::now();
                let total = self.pipeline.pending_headers.len();
                let pending = self.pipeline.headers_needing_bodies.len();
                self.set_state(
                    SyncState::DownloadingBodies { pending, total },
                    "body_stall_soft_retry",
                );

                warn!(
                    "Body stall retry {}/3: retained {} headers + {} downloaded bodies; \
                     re-requesting {} missing bodies from h={}",
                    self.pipeline.body_stall_retries,
                    total,
                    self.pipeline.pending_blocks.len(),
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
                    self.pipeline.body_stall_retries,
                    self.network
                        .network_tip_height
                        .saturating_sub(self.local_height),
                    self.local_height
                );

                // Full reset — let start_sync() decide header-first vs snap sync
                self.pipeline.pending_headers.clear();
                self.pipeline.pending_blocks.clear();
                self.pipeline.headers_needing_bodies.clear();
                self.pipeline.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.pipeline.body_downloader.clear();
                self.pipeline.body_stall_retries = 0;
                self.pipeline.header_downloader.clear();

                self.set_state(SyncState::Idle, "body_download_exhausted");
                self.network.last_sync_activity = Instant::now();
                if self.should_sync() {
                    self.start_sync();
                }
            } else {
                // Hard reset — only for header download failures or processing stuck.
                warn!(
                    "Sync stuck in {:?} for >30s with no progress \
                     (last_activity={:.0?} ago, local_h={}, network_tip={}) — resetting to Idle",
                    self.state,
                    self.network.last_sync_activity.elapsed(),
                    self.local_height,
                    self.network.network_tip_height
                );
                self.set_state(SyncState::Idle, "cleanup_stuck_sync");
                self.pipeline.pending_headers.clear();
                self.pipeline.pending_blocks.clear();
                self.pipeline.headers_needing_bodies.clear();
                self.pipeline.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.pipeline.body_downloader.clear();
                self.pipeline.body_stall_retries = 0;
                // Reset header downloader so next sync starts from local_hash,
                // not from a stale expected_prev_hash from a previous (failed) cycle.
                self.pipeline.header_downloader.clear();
                // If stuck in Processing, count toward deep fork detection.
                // The node downloaded a chain it can't apply — this is fork evidence.
                if was_processing {
                    self.fork.consecutive_empty_headers += 1;
                    info!(
                        "Stuck Processing counted as fork signal (consecutive_empty_headers={})",
                        self.fork.consecutive_empty_headers
                    );
                }
            }
        }

        // Expire stale header blacklist entries (30s cooldown — reduced from 60s
        // so peers become available between stuck-sync timeout cycles)
        self.fork
            .header_blacklisted_peers
            .retain(|_, added| added.elapsed() < Duration::from_secs(30));

        // NT8 fix: When ALL peers are blacklisted and we've been stuck for >120s
        // total, clear the blacklist entirely to retry with a fresh slate.
        // Without this, the node cycles Idle↔DownloadingHeaders forever because
        // blacklisted peers never expire before the next stuck-sync timeout.
        if !self.fork.header_blacklisted_peers.is_empty()
            && self.best_peer().is_none()
            && self.should_sync()
            && matches!(self.state, SyncState::Idle)
        {
            let stuck_duration = self.network.last_block_applied.elapsed();
            if stuck_duration > Duration::from_secs(120) {
                if self.fork.consecutive_empty_headers >= 20 {
                    // Genuinely on a dead fork: many consecutive empties + stuck for 2+ min.
                    // Escalate to snap sync if enough peers AND gap is large enough.
                    // For small gaps (≤12), redirect to rollback/fork_sync to preserve
                    // block history and avoid snap sync cascade.
                    let enough_peers = self.peers.len() >= 3;
                    let gap = self
                        .network
                        .network_tip_height
                        .saturating_sub(self.local_height);
                    if enough_peers && gap > 12 {
                        warn!(
                            "All peers blacklisted for >120s with {} consecutive empty headers — \
                             escalating to snap sync (gap={})",
                            self.fork.consecutive_empty_headers, gap
                        );
                        self.fork.header_blacklisted_peers.clear();
                        self.fork.needs_genesis_resync = true;
                    } else if enough_peers && gap <= 12 {
                        warn!(
                            "All peers blacklisted for >120s (gap={}) — clearing blacklist \
                             and signaling fork recovery (small gap, preserving block history)",
                            gap
                        );
                        self.fork.header_blacklisted_peers.clear();
                        self.signal_stuck_fork(); // Signal fork recovery without forcing counter
                    } else {
                        warn!(
                            "All peers blacklisted for >120s with {} consecutive empty headers \
                             but only {} peers (need 3 for snap sync) — clearing blacklist to retry",
                            self.fork.consecutive_empty_headers, self.peers.len()
                        );
                        self.fork.header_blacklisted_peers.clear();
                    }
                } else {
                    // Temporary gossip hiccup — clear blacklist and retry normally.
                    warn!(
                        "All peers blacklisted for >120s (consecutive_empty_headers={}) — \
                         clearing blacklist for fresh retry",
                        self.fork.consecutive_empty_headers
                    );
                    self.fork.header_blacklisted_peers.clear();
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
            let gap = self
                .network
                .network_tip_height
                .saturating_sub(self.local_height);
            if gap <= 5 && gap > 0 {
                // Small gap: increment a retry counter. If we've been stuck in
                // Idle-but-behind for multiple ticks (5+), escalate to a full
                // start_sync which transitions to header download and forces
                // the pipeline to request blocks actively.
                self.network.idle_behind_retries =
                    self.network.idle_behind_retries.saturating_add(1);
                if self.network.idle_behind_retries >= 5 {
                    info!(
                        "Sync retry: small gap ({} blocks) but stuck for {} ticks — forcing full sync restart.",
                        gap, self.network.idle_behind_retries
                    );
                    self.network.idle_behind_retries = 0;
                    self.start_sync();
                } else {
                    info!(
                        "Sync retry: Idle but behind peers (local_h={}, network_tip={}, gap={}, retry={}/5). Waiting for gossip.",
                        self.local_height, self.network.network_tip_height, gap, self.network.idle_behind_retries
                    );
                }
            } else {
                self.network.idle_behind_retries = 0;
                info!(
                    "Sync retry: Idle but behind peers (local_h={}, network_tip={}). Restarting sync.",
                    self.local_height, self.network.network_tip_height
                );
                self.start_sync();
            }
        } else if !matches!(self.state, SyncState::Idle) || !self.should_sync() {
            // Reset retry counter when no longer stuck
            self.network.idle_behind_retries = 0;
        }

        // Post-recovery grace timeout: if 120s have passed since snap sync
        // and grace hasn't cleared (10 blocks not applied), force-clear it.
        // This prevents grace from permanently blocking fork_sync when the node
        // landed on a fork after snap sync (can't apply blocks → grace never clears).
        if let super::RecoveryPhase::PostRecoveryGrace {
            started,
            blocks_applied,
        } = self.recovery_phase
        {
            if started.elapsed().as_secs() > 120 {
                warn!(
                    "Post-recovery grace timeout: 120s elapsed with only {} blocks applied. \
                     Force-clearing to allow fork recovery.",
                    blocks_applied
                );
                self.recovery_phase = super::RecoveryPhase::Normal;
            }
        }

        // Stuck-sync detection: if height hasn't advanced for >120s and we're
        // behind peers, the node is stuck. The correct escalation depends on gap:
        //
        // - gap <= 1000: likely on a fork with small gap. Escalate to fork_sync
        //   via consecutive_empty_headers = 3 (triggers resolve_shallow_fork).
        // - gap > 1000: too far behind for fork_sync (binary search only covers
        //   1000 blocks). Force snap sync via needs_genesis_resync.
        //
        // 120s gives gossip/header-first plenty of time before escalating.
        // Note: post_recovery_grace is intentionally NOT checked here. If the
        // node did snap sync but landed on a fork, it will be stuck forever
        // because grace requires 10 applied blocks to clear — but no blocks
        // can be applied on a fork. The 120s timeout is sufficient protection.
        if !self.is_fork_sync_active() && self.should_sync() {
            let gap = self
                .network
                .network_tip_height
                .saturating_sub(self.local_height);
            let stuck_secs = self.network.last_block_applied.elapsed().as_secs();
            if gap > 0 && stuck_secs > 120 {
                if gap > 1000 {
                    // Large gap: fork_sync can't help (only searches 1000 blocks).
                    // Force snap sync to jump to tip.
                    if self.snap.attempts < 3 && self.peers.len() >= 3 {
                        warn!(
                            "Stuck-sync detected: no block applied for {}s, behind by {} blocks \
                             (local_h={}, network_tip={}). Gap too large for fork_sync — forcing snap sync.",
                            stuck_secs, gap, self.local_height, self.network.network_tip_height
                        );
                        self.fork.needs_genesis_resync = true;
                    }
                } else if self.fork.consecutive_empty_headers < 3 && self.local_height > 0 {
                    // Small gap with non-zero height: likely on a fork.
                    // Signal fork recovery via dedicated flag (not counter force-set).
                    warn!(
                        "Stuck-on-fork detected: no block applied for {}s, behind by {} blocks \
                         (local_h={}, network_tip={}). Signaling fork recovery.",
                        stuck_secs, gap, self.local_height, self.network.network_tip_height
                    );
                    self.signal_stuck_fork();
                }
            }
        }

        // Height offset detection: if blocks ARE being applied (last_block_applied
        // is recent) but the gap to peers stays constant, the node has a corrupted
        // height counter from a bad reorg. Normal "behind" means blocks aren't being
        // applied; height offset means blocks ARE applied but gap never closes because
        // each block increments both our height and the canonical height equally.
        //
        // Fix: force snap sync to reset chain_state to correct height.
        if self.should_sync() && self.local_height > 0 {
            let gap = self
                .network
                .network_tip_height
                .saturating_sub(self.local_height);
            let blocks_recent = self.network.last_block_applied.elapsed().as_secs() < 30;

            if gap >= 2 && blocks_recent {
                match self.fork.stable_gap_since {
                    Some((prev_gap, since)) => {
                        // Gap is stable (within ±1 of what we first saw)
                        if gap.abs_diff(prev_gap) <= 1 && since.elapsed().as_secs() > 120 {
                            warn!(
                                "Height offset detected: gap={} has been stable for {}s while \
                                 blocks are being applied. This indicates a corrupted height \
                                 counter from a bad reorg. Forcing snap sync to correct.",
                                gap,
                                since.elapsed().as_secs()
                            );
                            self.fork.needs_genesis_resync = true;
                            self.fork.stable_gap_since = None;
                        } else if gap.abs_diff(prev_gap) > 1 {
                            // Gap changed significantly — reset tracker
                            self.fork.stable_gap_since = Some((gap, Instant::now()));
                        }
                    }
                    None => {
                        self.fork.stable_gap_since = Some((gap, Instant::now()));
                    }
                }
            } else {
                // Gap closed or blocks not being applied — reset
                self.fork.stable_gap_since = None;
            }
        } else {
            self.fork.stable_gap_since = None;
        }
    }
}
