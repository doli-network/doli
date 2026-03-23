//! Production gate — single source of truth for block production authorization
//!
//! Implements defense-in-depth with 11 layers of safety checks that ALL must pass
//! before block production is authorized.

use std::time::Instant;

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;

use super::{
    ProductionAuthorization, SyncManager, MIN_PEERS_TIER1, MIN_PEERS_TIER2, MIN_PEERS_TIER3,
};

impl SyncManager {
    // =========================================================================
    // PRODUCTION GATE - Single source of truth for block production authorization
    // =========================================================================

    /// Update fork detection and gossip state. Call BEFORE can_produce().
    ///
    /// Side effects extracted from can_produce() to make it a pure query.
    /// This fixes a class of race-like bugs where a "read" operation (can_produce)
    /// mutated state, causing the system to behave differently depending on how
    /// often production was checked.
    pub fn update_production_state(&mut self) {
        // Layer 9 side effect: detect minority fork and set persistent flag.
        // Previously embedded in can_produce() where it was called every slot.
        //
        // IMPORTANT: Only compare hashes at the SAME height. Peers 1-2 blocks
        // ahead are NOT forked — they simply received the next block before us.
        // The previous "1-2 ahead = disagree" heuristic caused false fork
        // detection during normal catching-up: a node finishing sync would see
        // most peers 1 block ahead (e.g., 3 agree vs 50 "disagree"), triggering
        // the persistent fork flag and blocking production. (INC-I-005)
        let mut agree = 1u32;
        let mut disagree = 0u32;
        for status in self.peers.values() {
            if status.best_height == self.local_height {
                if status.best_hash == self.local_hash {
                    agree += 1;
                } else if status.best_hash != Hash::ZERO {
                    disagree += 1;
                }
            }
            // Peers ahead by 1-2 blocks are ignored — being behind is not a fork.
        }
        if disagree > 0 && agree < disagree && !self.fork.fork_mismatch_detected {
            warn!(
                "FORK DETECTION: We are in minority at height {} ({} agree, {} disagree) — setting persistent fork flag",
                self.local_height, agree, disagree
            );
            self.fork.fork_mismatch_detected = true;
        }

        // Layer 10.5 side effect: reset gossip timer on network stall bypass.
        // Previously embedded in can_produce() at the circuit breaker.
        let best_peer_height = self.best_peer_height();
        if self.local_height > 1 && self.local_height >= best_peer_height {
            let last_gossip = self
                .last_block_received_via_gossip
                .unwrap_or(Instant::now());
            let silence_secs = last_gossip.elapsed().as_secs();

            if silence_secs > self.max_solo_production_secs {
                // INC-I-005: Only consider near-tip peers (within 5 blocks).
                // Syncing peers far below the tip should not prevent timer reset.
                let near_tip_peers = self
                    .peers
                    .values()
                    .filter(|p| self.local_height.saturating_sub(p.best_height) <= 5)
                    .count();
                let near_tip_at_our_height = self
                    .peers
                    .values()
                    .filter(|p| p.best_height == self.local_height)
                    .count();
                let near_tip_majority =
                    near_tip_peers > 0 && near_tip_at_our_height > near_tip_peers / 2;

                if near_tip_majority {
                    info!(
                        "CIRCUIT BREAKER BYPASS: {}/{} near-tip peers at height {} — \
                         network stall detected, resetting gossip timer (silence={}s, total_peers={})",
                        near_tip_at_our_height,
                        near_tip_peers,
                        self.local_height,
                        silence_secs,
                        self.peers.len()
                    );
                    self.last_block_received_via_gossip = Some(Instant::now());
                }
            }
        }
    }

    /// Check if block production is authorized - THE SINGLE SOURCE OF TRUTH
    ///
    /// This method implements defense-in-depth for production safety:
    /// 1. Explicit block check (invariant violations, manual blocks)
    /// 2. Resync-in-progress check
    /// 3. Active sync check (downloading headers/bodies/processing)
    /// 4. Post-resync grace period check
    /// 5. Peer synchronization check (within N slots/heights)
    ///
    /// ALL checks must pass for production to be authorized.
    ///
    /// NOTE: This method is now side-effect-free. Call update_production_state()
    /// before this to handle fork detection and gossip timer mutations.
    pub fn can_produce(&mut self, current_slot: u32) -> ProductionAuthorization {
        // === CHECKPOINT: Entry point with all key values ===
        let best_peer_h = self.best_peer_height();
        let best_peer_s = self.best_peer_slot();
        info!(
            "[CAN_PRODUCE] slot={} local_h={} local_s={} peer_h={} peer_s={} peers={} state={:?}",
            current_slot,
            self.local_height,
            self.local_slot,
            best_peer_h,
            best_peer_s,
            self.peers.len(),
            self.state
        );

        // Layer 1: Explicit production block
        if let Some(ref reason) = self.production_blocked {
            return ProductionAuthorization::BlockedExplicit {
                reason: reason.clone(),
            };
        }

        // Layer 2: Resync in progress
        if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
            return ProductionAuthorization::BlockedResync {
                grace_remaining_secs: self.resync_grace_period_secs,
            };
        }

        // Layer 2.5: Post-snap-sync canonical block gate
        // After snap sync, the block store is empty — producing immediately would create
        // a fork because there's no real parent block to build on. Wait until at least
        // one canonical gossip block has been received and applied, proving we're on the
        // canonical chain and giving the block store a real parent.
        if matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        ) {
            return ProductionAuthorization::BlockedAwaitingCanonicalBlock;
        }

        // Layer 3: Active sync in progress
        if self.state.is_syncing() {
            return ProductionAuthorization::BlockedSyncing;
        }

        // Layer 4: Bootstrap gate - CRITICAL for preventing isolated forks
        //
        // Defense in depth: We use is_in_bootstrap_phase() which DERIVES the bootstrap
        // state from actual conditions (height == 0, lost peers, etc.) rather than
        // relying on stored flags. This makes invalid states impossible.
        //
        // During bootstrap, ALL nodes start at height 0. If late-joining nodes only
        // connect to other late-joining nodes (also at height 0), they'll think
        // they're caught up and produce at height 1 - creating isolated forks.
        //
        // The fix: Wait until we have CREDIBLE EVIDENCE the network has advanced:
        // - Either a block arrived via gossip (network_tip_slot > 0)
        // - Or a peer reported height > 0
        // - Or we ARE at height 0 and can legitimately produce the first block
        //
        // This prevents the scenario where late nodes produce competing genesis chains.
        if self.is_in_bootstrap_phase() && self.first_peer_status_received.is_some() {
            // Bootstrap phase detected (derived from state):
            // - height == 0: We're at genesis, need to verify network state
            // - peers empty after connecting: Lost all peers, need to re-establish
            //
            // We need evidence the network is real before producing

            // Check 1: Have we received any peer status at all?
            if self.first_peer_status_received.is_none() {
                return ProductionAuthorization::BlockedBootstrap {
                    reason: "Waiting for peer status response".to_string(),
                };
            }

            // Check 2: If we lost all peers (height > 0 but peers empty), wait for reconnection.
            // After peer_loss_timeout_secs, allow production to resume solo — the peer
            // may be permanently down and halting the chain is worse than a temporary fork.
            if self.local_height > 0 && self.peers.is_empty() {
                let past_timeout = self
                    .peers_lost_at
                    .map(|t| t.elapsed().as_secs() >= self.peer_loss_timeout_secs)
                    .unwrap_or(false);
                if !past_timeout {
                    return ProductionAuthorization::BlockedBootstrap {
                        reason: "Lost all peers - waiting for reconnection".to_string(),
                    };
                }
                info!(
                    "Peer loss timeout reached ({}s) — resuming solo production at height {}",
                    self.peer_loss_timeout_secs, self.local_height
                );
            }

            // Check 3: Have we seen any chain activity? (block via gossip OR peer with height > 0)
            let has_chain_activity = self.network.network_tip_slot > 0
                || self.network.network_tip_height > 0
                || self.best_peer_height() > 0
                || self.best_peer_slot() > 0;

            if !has_chain_activity {
                // All peers are at height 0 too - this could be:
                // (A) True genesis - we're the first producer
                // (B) Partition of late nodes - dangerous!
                //
                // To distinguish: Wait for the bootstrap grace period.
                // If we're truly first, no harm in waiting a bit.
                // If we're partitioned, waiting gives us time to connect to the real network.
                if let Some(first_status) = self.first_peer_status_received {
                    let elapsed = first_status.elapsed().as_secs();
                    if elapsed < self.bootstrap_grace_period_secs {
                        // Still in bootstrap window - wait longer for chain evidence
                        return ProductionAuthorization::BlockedBootstrap {
                            reason: format!(
                                "All peers at height 0 - waiting for chain evidence ({}s/{}s)",
                                elapsed, self.bootstrap_grace_period_secs
                            ),
                        };
                    }

                    // INC-I-005: Grace period expired but require minimum peer quorum
                    // before producing at genesis. Two isolated nodes at h=0 shouldn't
                    // start a fork chain when a canonical chain might already exist.
                    // During mass deploys, new nodes connect to each other before
                    // discovering the canonical chain — without this check, they
                    // produce fork blocks within the first 15s.
                    let min_genesis_quorum = self.min_peers_for_production.max(5);
                    if self.peers.len() < min_genesis_quorum {
                        return ProductionAuthorization::BlockedBootstrap {
                            reason: format!(
                                "Genesis quorum not met: {}/{} peers at h=0 (need {} to confirm true genesis)",
                                self.peers.len(), min_genesis_quorum, min_genesis_quorum
                            ),
                        };
                    }
                    // Enough peers, all at h=0, grace expired — true genesis
                }
            }
        }

        // Layer 5: Post-resync grace period (absorbs former Layer 5.6 first-sync grace)
        // PGD-002: effective_grace is capped at max_grace_cap_secs to prevent
        // exponential backoff from disabling producers for 480s+ (the root cause
        // of the 2026-03-15 network halt).
        if let Some(completed) = self.last_resync_completed {
            let elapsed = completed.elapsed().as_secs();
            let uncapped_grace = if self.consecutive_resync_count > 1 {
                self.resync_grace_period_secs * (1 << (self.consecutive_resync_count - 1).min(4))
            } else {
                self.resync_grace_period_secs
            };
            let effective_grace = uncapped_grace.min(self.max_grace_cap_secs);

            if uncapped_grace > self.max_grace_cap_secs {
                debug!(
                    "PGD-002: Grace period capped from {}s to {}s (resync #{})",
                    uncapped_grace, effective_grace, self.consecutive_resync_count
                );
            }

            if elapsed < effective_grace {
                return ProductionAuthorization::BlockedResync {
                    grace_remaining_secs: effective_grace - elapsed,
                };
            }
        }

        // Layer 5.5: Minimum peer count check (echo chamber prevention)
        //
        // With too few peers, a node might form an isolated cluster with other forked nodes
        // where all peers agree on the wrong chain. This check ensures we have enough
        // diverse viewpoints before trusting peer data for production decisions.
        //
        // Example: Node 8 has only 1 peer (another forked node at same height)
        // Without this check: height_ahead = 0 → "not ahead" → AUTHORIZED (bad!)
        // With this check: peers=1 < min=2 → BLOCKED (prevents echo chamber)
        info!(
            "[CAN_PRODUCE] Layer5.5: peers={} min_required={}",
            self.peers.len(),
            self.min_peers_for_production
        );
        let past_peer_loss_timeout = self
            .peers_lost_at
            .map(|t| t.elapsed().as_secs() >= self.peer_loss_timeout_secs)
            .unwrap_or(false);
        // INC-001: The `local_height > 0` bypass allowed ALL nodes at genesis to produce
        // with just 1 peer, creating 5 competing blocks for slot 1. Only bypass the peer
        // check at height 0 if min_peers is 1 (devnet). For testnet/mainnet (min_peers=2),
        // enforce the peer requirement even at height 0.
        let genesis_bypass = self.local_height == 0 && self.min_peers_for_production <= 1;
        // INC-I-005: peer_loss_timeout bypass must NOT apply at height 0 (never synced).
        // The timeout is for nodes that WERE synced and lost peers — solo production
        // resumes the chain. At height 0, the node has no data and would produce a
        // solo fork from genesis (N60 produced 424 blocks at height 1 this way).
        let peer_loss_bypass = past_peer_loss_timeout && self.local_height > 0;
        if self.peers.len() < self.min_peers_for_production && !genesis_bypass && !peer_loss_bypass
        {
            // Skip if peer loss timeout expired AND we have data — solo production is preferable to chain halt
            warn!(
                "FORK PREVENTION: Only {} peers (need {}) - blocking production to prevent echo chamber",
                self.peers.len(), self.min_peers_for_production
            );
            return ProductionAuthorization::BlockedInsufficientPeers {
                peer_count: self.peers.len(),
                min_required: self.min_peers_for_production,
            };
        }

        // Layer 6: Peer synchronization check - too far BEHIND
        //
        // IMPORTANT: Only compare SLOTS, not heights. Heights are unreliable because
        // forked nodes accumulate inflated block counts (height > slot). A single
        // forked peer with height=200 would block honest nodes at height=158.
        // Slots are time-based and can't be inflated by forks.
        //
        // GUARD: Skip this check when local_height >= best_peer_height.
        // When peers are still syncing (height=0) they report valid best_slot from
        // their clock, creating a false "behind" signal. A node whose height is at
        // or ahead of every peer is definitionally NOT behind the network —
        // its local_slot is only stale because it stopped producing, and it can't
        // produce because this layer blocks it, creating a deadlock.
        let best_peer_slot = self.best_peer_slot();

        // Only check if we have peer data
        if !self.peers.is_empty() && best_peer_slot > 0 {
            let slot_diff = best_peer_slot.saturating_sub(self.local_slot);

            if slot_diff > self.max_slots_behind {
                let best_peer_height = self.best_peer_height();

                // Guard: If we're at or ahead of all peers by height, slot lag
                // is a stale artifact — we ARE the tip, just haven't produced
                // recently. Don't block; let production advance local_slot.
                if self.local_height >= best_peer_height {
                    info!(
                        "[CAN_PRODUCE] Layer6: slot_diff={} exceeds max={}, but local_height={} >= peer_height={} - allowing",
                        slot_diff, self.max_slots_behind, self.local_height, best_peer_height
                    );
                } else {
                    return ProductionAuthorization::BlockedBehindPeers {
                        local_height: self.local_height,
                        peer_height: best_peer_height,
                        height_diff: best_peer_height.saturating_sub(self.local_height),
                    };
                }
            }
        }

        // Layer 6.5: Height lag check — graduated production gate.
        //
        // Industry standard (cf. Eth2 beacon, Tendermint, Substrate): nodes MUST NOT
        // produce blocks while syncing or significantly behind peers. Producing from
        // stale state creates competing forks and can trigger infinite reorg loops
        // (the old 60s timeout escape caused exactly this — see postmortem).
        //
        // Two gates:
        //   1. Sync state gate: block production in any active sync state
        //   2. Height lag gate: block production when >3 blocks behind peers
        //
        // The only timeout escape is for tiny lags (2-3 blocks) where the node is
        // likely on the same chain lineage, just slightly behind gossip propagation.

        // Gate 1: Active sync state — never produce while downloading/processing/fork-resolving
        if self.state.is_syncing() || self.fork.fork_sync.is_some() {
            info!(
                "[CAN_PRODUCE] Layer6.5: BLOCKED — active sync state={}, cannot produce",
                self.sync_state_name()
            );
            return ProductionAuthorization::BlockedBehindPeers {
                local_height: self.local_height,
                peer_height: self.best_peer_height(),
                height_diff: self.best_peer_height().saturating_sub(self.local_height),
            };
        }

        // Gate 2: Height lag — block production when significantly behind
        let best_peer_height = self.best_peer_height();
        if !self.peers.is_empty() && best_peer_height > 0 {
            let height_lag = best_peer_height.saturating_sub(self.local_height);

            if height_lag > 5 {
                // Very large lag (>5): unconditionally block. No timeout escape.
                // The node must sync to tip before producing.
                info!(
                    "[CAN_PRODUCE] Layer6.5: BLOCKED — lag={} (local_h={}, peer_h={}). \
                     Must sync to tip before producing.",
                    height_lag, self.local_height, best_peer_height
                );
                self.behind_since = None;
                return ProductionAuthorization::BlockedBehindPeers {
                    local_height: self.local_height,
                    peer_height: best_peer_height,
                    height_diff: height_lag,
                };
            } else if height_lag > 3 {
                // INC-001: Graduated timeout for lag 4-5. During mass node join,
                // gossip propagation delays cause brief 4-5 block lags that are
                // NOT forks. Allow production after 60s to avoid starving slots.
                let behind_secs = self
                    .behind_since
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    .as_secs();
                if behind_secs <= 60 {
                    info!(
                        "[CAN_PRODUCE] Layer6.5: BLOCKED — lag={} (local_h={}, peer_h={}) behind_for={}s/60s",
                        height_lag, self.local_height, best_peer_height, behind_secs
                    );
                    return ProductionAuthorization::BlockedBehindPeers {
                        local_height: self.local_height,
                        peer_height: best_peer_height,
                        height_diff: height_lag,
                    };
                }
                info!(
                    "[CAN_PRODUCE] Layer6.5: lag={} but timeout elapsed ({}s>60s) — allowing",
                    height_lag, behind_secs
                );
            } else if height_lag >= 2 {
                // Small lag (2-3 blocks): allow immediately.
                //
                // INC-001: A 2-3 block lag is NORMAL on a 5-producer network with 10s
                // slots. It means gossip blocks haven't been applied yet, NOT a fork.
                // The node-level check in try_produce_block() already prevents production
                // when >3 blocks behind (early chain) or >5 blocks behind (normal).
                //
                // Previously this had a 30s timeout which was fatal: the node would
                // miss its slot, fall further behind, trigger sync, and sync would
                // cascade into fork_sync → ancestor at h=0 → full reset. The node
                // NEVER produced because the 30s timeout kept being interrupted by sync.
                debug!(
                    "[CAN_PRODUCE] Layer6.5: small lag={} (local_h={}, peer_h={}) — \
                     allowing (gossip will catch up)",
                    height_lag, self.local_height, best_peer_height
                );
            } else {
                // Gap closed — reset tracker
                self.behind_since = None;
            }
        } else {
            self.behind_since = None;
        }

        // Layer 7: REMOVED — Satoshi principle: always extend your best chain.
        //  Fork detection via AheadOfPeers caused chain deadlock (2026-02-25).
        //  When the tip node's peers are syncing behind, AheadOfPeers blocks
        //  production, which prevents peers from catching up, creating a
        //  permanent deadlock where nobody produces.
        //  Forks are resolved by: (1) longest chain reorg, (2) sync failures (Layer 8),
        //  (3) chain mismatch detection (Layer 9).
        info!(
            "[CAN_PRODUCE] Layer7: SKIPPED (removed) — peers={} best_peer={} local={} ahead={}",
            self.peers.len(),
            best_peer_height,
            self.local_height,
            self.local_height.saturating_sub(best_peer_height)
        );

        // Layer 9: Chain Hash Verification — INFORMATIONAL ONLY
        //
        // When our chain has diverged from peers:
        // - GetHeaders requests return empty (peer doesn't have our tip as ancestor)
        // - This increments consecutive_sync_failures
        // - After 3+ failures, we're likely on a fork
        //
        // This catches forks where height comparison is inconclusive.
        info!(
            "[CAN_PRODUCE] Layer8: sync_failures={} max_failures={}",
            self.fork.consecutive_sync_failures, self.fork.max_sync_failures_before_fork_detection
        );
        if self.fork.consecutive_sync_failures >= self.fork.max_sync_failures_before_fork_detection
        {
            warn!(
                "FORK DETECTION: {} consecutive sync failures - blocking production",
                self.fork.consecutive_sync_failures
            );
            return ProductionAuthorization::BlockedSyncFailures {
                failure_count: self.fork.consecutive_sync_failures,
            };
        }

        // Layer 8.5: Persistent fork mismatch flag.
        //
        // If a prior Layer 9 check detected we're in the minority, keep blocking
        // until a successful resync clears the flag. Without this, Layer 9 oscillates:
        // detects fork → blocks → peers advance beyond ±2 window → Layer 9 forgets
        // → node resumes producing on orphan chain → repeat.
        if self.fork.fork_mismatch_detected {
            warn!(
                "[CAN_PRODUCE] Layer8.5: BLOCKED — fork_mismatch_detected flag set, awaiting resync (local_h={})",
                self.local_height
            );
            return ProductionAuthorization::BlockedChainMismatch {
                peer_id: self
                    .peers
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or_else(PeerId::random),
                local_hash: self.local_hash,
                peer_hash: Hash::default(),
                local_height: self.local_height,
            };
        }

        // Layer 9: Chain Hash Verification (P0 #1)
        //
        // Count peers at same height that agree (same hash) vs disagree (different hash).
        // Only block production if we're in the clear minority — the majority keeps
        // producing so the heaviest chain rule resolves the fork naturally.
        //
        // IMPORTANT: Only compare at the SAME height. Peers 1-2 blocks ahead
        // are simply ahead, not forked. The previous heuristic caused false
        // fork detection during catching-up (INC-I-005: N36 saw 3 agree vs
        // 50 "disagree" because 50 peers had the next block).
        let mut agree = 1u32; // Count ourselves — we agree with our own chain
        let mut disagree = 0u32;
        let mut first_mismatch_peer = None;
        let mut first_mismatch_hash = self.local_hash;
        for (peer_id, status) in &self.peers {
            if status.best_height == self.local_height {
                // Same height: compare hashes directly
                if status.best_hash == self.local_hash {
                    agree += 1;
                } else if status.best_hash != Hash::ZERO {
                    disagree += 1;
                    if first_mismatch_peer.is_none() {
                        first_mismatch_peer = Some(*peer_id);
                        first_mismatch_hash = status.best_hash;
                    }
                }
            }
            // Peers ahead by 1-2 blocks are ignored — being behind is not a fork.
        }
        // Only block if we're in the minority — majority keeps producing.
        // NOTE: fork_mismatch_detected is now set by update_production_state(),
        // not here. can_produce() is side-effect-free.
        if disagree > 0 && agree < disagree {
            if let Some(peer_id) = first_mismatch_peer {
                warn!(
                    "FORK DETECTION: We are in minority at height {} ({} agree, {} disagree)",
                    self.local_height, agree, disagree
                );
                return ProductionAuthorization::BlockedChainMismatch {
                    peer_id,
                    local_hash: self.local_hash,
                    peer_hash: first_mismatch_hash,
                    local_height: self.local_height,
                };
            }
        }

        // Layer 10: Gossip Activity Watchdog (P0 #3)
        //
        // If we have peers but haven't received ANY blocks via gossip for a long time,
        // we are likely isolated (e.g., in a "ping-only" partition).
        // Exceptions:
        // - No peers connected (handled by MinPeers check)
        // - Initial bootstrap (handled by BootstrapGate)
        // - No peer is ahead of us: gossip silence is expected when WE are the tip.
        //   Without this exception, all nodes deadlock: nobody produces → no gossip
        //   → watchdog blocks everyone → permanent halt.
        if !self.peers.is_empty() && best_peer_height > self.local_height {
            let last_gossip = self
                .last_block_received_via_gossip
                .unwrap_or(Instant::now());
            let elapsed = last_gossip.elapsed();

            if elapsed.as_secs() > self.gossip_activity_timeout_secs {
                warn!(
                    "FORK DETECTION: No gossip activity for {}s (timeout {}) with {} peers (peer_h={} > local_h={}) - blocking production",
                    elapsed.as_secs(), self.gossip_activity_timeout_secs, self.peers.len(),
                    best_peer_height, self.local_height
                );
                return ProductionAuthorization::BlockedNoGossipActivity {
                    seconds_since_gossip: elapsed.as_secs(),
                    peer_count: self.peers.len(),
                };
            }
        }

        // Layer 10.5: Solo production circuit breaker
        //
        // Complement to Layer 10: If WE are the tip (local_height >= best_peer_height),
        // gossip silence is expected — Layer 10 allows it. But if gossip has been silent
        // for max_solo_production_secs (default 50s = 5 slots), we're likely building
        // an orphan chain in isolation. Pause to prevent long parallel forks.
        //
        // PGD-003/PGD-004: NETWORK STALL RECOVERY — When the circuit breaker fires
        // but ALL peers report the SAME height as us, this is "entire network stalled"
        // not "solo orphan chain." Every earlier layer (8, 8.5, 9) has already verified
        // hash agreement, so by the time we reach here with all peers at our height,
        // the chain is healthy — just stalled. Allow production to break the deadlock.
        // If the block propagates, gossip resumes naturally. If not (genuine isolation
        // despite peer agreement), the circuit breaker fires again in 50s — limiting
        // orphan growth to 1 block per 50s cycle.
        //
        // Exception: at genesis (height <= 1) — first blocks legitimately have no gossip.
        if self.local_height > 1 && self.local_height >= best_peer_height {
            let last_gossip = self
                .last_block_received_via_gossip
                .unwrap_or(Instant::now());
            let silence_secs = last_gossip.elapsed().as_secs();

            if silence_secs > self.max_solo_production_secs {
                // PGD-003 + INC-001 + INC-I-005: Check for network stall.
                // Peers still syncing (far below tip) should not count against
                // the majority check. Only consider peers "near tip" (within 5
                // blocks of our height) when deciding if the network is stalled.
                // Without this, deploying many new nodes that are syncing dilutes
                // the majority below 50% and permanently halts the chain.
                let near_tip_peers: Vec<_> = self
                    .peers
                    .values()
                    .filter(|p| self.local_height.saturating_sub(p.best_height) <= 5)
                    .collect();
                let peers_at_our_height = near_tip_peers
                    .iter()
                    .filter(|p| p.best_height == self.local_height)
                    .count();
                let near_tip_count = near_tip_peers.len();
                let majority_at_our_height =
                    near_tip_count > 0 && peers_at_our_height > near_tip_count / 2;

                if majority_at_our_height {
                    // Network stall: everyone stuck at the same height, nobody producing.
                    // Allow production. The gossip timer was already reset by
                    // update_production_state() (called before can_produce).
                    info!(
                        "CIRCUIT BREAKER BYPASS: {}/{} near-tip peers at height {} — \
                         network stall detected, allowing production (silence={}s, total_peers={})",
                        peers_at_our_height,
                        near_tip_count,
                        self.local_height,
                        silence_secs,
                        self.peers.len()
                    );
                    // Fall through to Authorized
                } else {
                    // Not all peers at our height — genuine isolation or mixed state.
                    // Block until gossip resumes or state is reset.
                    warn!(
                        "CIRCUIT BREAKER: Produced solo for {}s (limit {}s) with no gossip blocks received. \
                         Pausing production to avoid building orphan chain. local_h={} peer_h={}",
                        silence_secs, self.max_solo_production_secs,
                        self.local_height, best_peer_height
                    );
                    return ProductionAuthorization::BlockedNoGossipActivity {
                        seconds_since_gossip: silence_secs,
                        peer_count: self.peers.len(),
                    };
                }
            }
        }

        // Layer 11: Finality conflict check
        // If we have a finalized block, ensure our chain doesn't conflict with it.
        // This prevents producing blocks on a fork that has been superseded by finality.
        if let Some(finalized_height) = self.last_finalized_height() {
            if self.local_height < finalized_height {
                info!(
                    "[CAN_PRODUCE] Layer11: local_height={} < finalized_height={} - blocked",
                    self.local_height, finalized_height
                );
                return ProductionAuthorization::BlockedConflictsFinality {
                    local_finalized_height: finalized_height,
                };
            }
        }

        // All checks passed - production is authorized
        info!("[CAN_PRODUCE] AUTHORIZED - all checks passed");
        ProductionAuthorization::Authorized
    }

    /// Quick boolean check for production authorization
    pub fn is_production_safe(&mut self, current_slot: u32) -> bool {
        matches!(
            self.can_produce(current_slot),
            ProductionAuthorization::Authorized
        )
    }

    /// Explicitly block production (e.g., due to invariant violation)
    pub fn block_production(&mut self, reason: &str) {
        warn!("Production blocked: {}", reason);
        self.production_blocked = Some(reason.to_string());
    }

    /// Clear explicit production block
    pub fn unblock_production(&mut self) {
        if self.production_blocked.is_some() {
            info!("Production unblocked");
            self.production_blocked = None;
        }
    }

    /// Signal that a forced resync is starting
    ///
    /// This blocks production until the resync completes and grace period expires.
    pub fn start_resync(&mut self) {
        info!("Resync started - production blocked");
        self.recovery_phase = super::RecoveryPhase::ResyncInProgress;
        self.consecutive_resync_count += 1;
        self.blocks_since_resync_completed = 0; // PGD-001: reset stable block counter

        // Log exponential backoff info (PGD-002: capped at max_grace_cap_secs)
        if self.consecutive_resync_count > 1 {
            let uncapped =
                self.resync_grace_period_secs * (1 << (self.consecutive_resync_count - 1).min(4));
            let effective_grace = uncapped.min(self.max_grace_cap_secs);
            warn!(
                "Consecutive resync #{} - grace period {}s (capped from {}s)",
                self.consecutive_resync_count, effective_grace, uncapped
            );
        }
    }

    /// Signal that a forced resync has completed
    ///
    /// Starts the grace period timer before production can resume.
    pub fn complete_resync(&mut self) {
        info!("Resync completed - starting grace period");
        self.recovery_phase = super::RecoveryPhase::Normal;
        self.last_resync_completed = Some(Instant::now());
    }

    /// Clear the post-snap-sync production gate.
    /// Called when a canonical gossip block has been successfully applied,
    /// proving we're on the canonical chain.
    pub fn clear_awaiting_canonical_block(&mut self) {
        if matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        ) {
            info!("[SNAP_SYNC] Canonical gossip block received — production gate cleared");
            self.recovery_phase = super::RecoveryPhase::Normal;
        }
        if self.fork.fork_mismatch_detected {
            info!("[FORK_RECOVERY] Canonical gossip block applied — fork mismatch flag cleared");
            self.fork.fork_mismatch_detected = false;
        }
    }

    /// Check if we're waiting for a canonical block after snap sync.
    pub fn is_awaiting_canonical_block(&self) -> bool {
        matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        )
    }

    /// Reset consecutive resync counter (call after stable operation)
    pub fn reset_resync_counter(&mut self) {
        if self.consecutive_resync_count > 0 {
            debug!(
                "Resetting consecutive resync counter (was {})",
                self.consecutive_resync_count
            );
            self.consecutive_resync_count = 0;
        }
    }

    /// Check if a resync is currently in progress
    pub fn is_resync_in_progress(&self) -> bool {
        matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress)
    }

    /// Get the current consecutive resync count
    pub fn consecutive_resync_count(&self) -> u32 {
        self.consecutive_resync_count
    }

    /// Get blocks applied since last reset (indicates active sync progress)
    pub fn blocks_applied(&self) -> u64 {
        self.network.blocks_applied
    }

    /// Signal that we have connected to at least one peer
    ///
    /// Bootstrap gate is now driven by `first_peer_status_received` (set via
    /// `note_peer_status_received()`). This method is kept for callers that
    /// signal peer connection before status exchange completes.
    pub fn set_peer_connected(&mut self) {
        if self.first_peer_status_received.is_none() {
            debug!("First peer connection noted - awaiting peer status for bootstrap gate");
        }
    }

    /// Signal that we received a valid peer status response
    ///
    /// This is called when a PeerStatus arrives. It updates the timestamps
    /// used by the bootstrap gate to determine if we have peer info.
    pub fn note_peer_status_received(&mut self) {
        let now = Instant::now();
        // Track FIRST status for grace period calculation
        if self.first_peer_status_received.is_none() {
            self.first_peer_status_received = Some(now);
            debug!("First peer status received - starting bootstrap grace period");
        }
        self.last_peer_status_received = Some(now);
    }

    /// Check if bootstrap gate is satisfied (have peer status or grace period expired)
    pub fn is_bootstrap_ready(&self) -> bool {
        // first_peer_status_received tracks both connection and status in one field:
        // None = no peer has sent status yet (standalone mode, OK to produce)
        // Some(_) = at least one peer status received (bootstrap can proceed)
        //
        // When None and peers.is_empty(), we're standalone — safe to produce.
        // When None and peers exist, we're waiting for status — handled by bootstrap gate.
        self.first_peer_status_received.is_some() || self.peers.is_empty()
    }

    /// Check if we're in bootstrap phase - DERIVED FROM STATE, NOT STORED
    ///
    /// Defense in depth: This method computes bootstrap state from actual conditions
    /// rather than relying on a stored flag. This makes invalid states impossible:
    /// - If height == 0, we're definitionally in bootstrap (no matter how we got here)
    /// - If we lost all peers after connecting, we need to re-bootstrap
    ///
    /// This is the "make invalid states unrepresentable" principle.
    pub fn is_in_bootstrap_phase(&self) -> bool {
        // Primary: at genesis height = ALWAYS bootstrap mode
        // This is the key insight: height 0 means newbie, period.
        if self.local_height == 0 {
            return true;
        }

        // Secondary: had peer status but lost all peers
        // This could indicate network partition or need to resync
        if self.first_peer_status_received.is_some() && self.peers.is_empty() {
            return true;
        }

        false
    }

    /// Configure the production gate settings
    pub fn configure_production_gate(&mut self, grace_period_secs: u64, max_slots_behind: u32) {
        self.resync_grace_period_secs = grace_period_secs;
        self.max_slots_behind = max_slots_behind;
    }

    /// Set the bootstrap grace period (wait time at genesis for chain evidence)
    ///
    /// At genesis, when all peers are at height 0, the node waits this duration
    /// before allowing block production. This helps distinguish between:
    /// - True genesis (we're first producer, safe to start)
    /// - Network partition (we're isolated, dangerous to produce)
    pub fn set_bootstrap_grace_period_secs(&mut self, secs: u64) {
        self.bootstrap_grace_period_secs = secs;
    }

    /// Set the minimum peers required for production (P0 #5 echo chamber prevention)
    ///
    /// - For mainnet/testnet: 2 (default) - require multiple peers to prevent echo chambers
    /// - For devnet without DHT: 1 - allow single-peer production since discovery is limited
    pub fn set_min_peers_for_production(&mut self, min_peers: usize) {
        self.min_peers_for_production = min_peers;
        info!(
            "Set min_peers_for_production to {} for echo chamber prevention",
            min_peers
        );
    }

    /// Set the producer tier and adjust min_peers_for_production accordingly.
    ///
    /// If no blocks are received via gossip for this duration, production is blocked.
    /// This should be calibrated to the slot duration (e.g., 18 * slot_duration).
    pub fn set_gossip_activity_timeout_secs(&mut self, secs: u64) {
        self.gossip_activity_timeout_secs = secs;
        info!("Set gossip_activity_timeout_secs to {} seconds", secs);
    }

    /// Set the producer tier and adjust min_peers_for_production accordingly.
    ///
    /// Tier 1 validators need more peers (dense mesh), Tier 3 stakers need fewer.
    /// The `active_producer_count` caps min_peers so small networks aren't deadlocked
    /// (a node can have at most `active_producer_count - 1` peers).
    pub fn set_tier(&mut self, tier: u8, active_producer_count: usize) {
        self.tier = tier;

        // Tier-based min_peers only applies to large networks (500+ producers).
        // In small networks ALL producers are trivially "Tier 1", but the Tier 1
        // min_peers (3) is designed for dense validator meshes at scale.
        // Keep the default min_peers (2) until the network grows enough for
        // tiering to be meaningful.
        if active_producer_count < 500 {
            info!(
                "Set tier={} min_peers_for_production={} (skipped tier override: network_size={} < 500)",
                tier, self.min_peers_for_production, active_producer_count
            );
            return;
        }

        let tier_min = match tier {
            1 => MIN_PEERS_TIER1,
            2 => MIN_PEERS_TIER2,
            3 => MIN_PEERS_TIER3,
            _ => MIN_PEERS_TIER3, // Default: backward compatible
        };
        let max_possible = active_producer_count.saturating_sub(1).max(1);
        self.min_peers_for_production = tier_min.min(max_possible);
        info!(
            "Set tier={} min_peers_for_production={} (tier_req={}, network_size={})",
            tier, self.min_peers_for_production, tier_min, active_producer_count
        );
    }

    /// Get the current tier.
    pub fn tier(&self) -> u8 {
        self.tier
    }

    // =========================================================================
    // FINALITY TRACKING
    // =========================================================================

    /// Track a newly applied block for finality.
    pub fn track_block_for_finality(
        &mut self,
        hash: crypto::Hash,
        height: u64,
        slot: u32,
        total_weight: u64,
    ) {
        self.finality_tracker
            .track_block(hash, height, slot, total_weight);
    }

    /// Add attestation weight to a pending block.
    pub fn add_attestation_weight(&mut self, block_hash: &crypto::Hash, weight: u64) {
        self.finality_tracker
            .add_attestation_weight(*block_hash, weight);
        // Check if this triggers finality
        if let Some(checkpoint) = self.finality_tracker.check_finality() {
            info!(
                "FINALITY: Block {} finalized at height {} (attestation {}/{})",
                checkpoint.block_hash,
                checkpoint.height,
                checkpoint.attestation_weight,
                checkpoint.total_weight
            );
            self.reorg_handler
                .set_last_finality_height(checkpoint.height);
        }
    }

    /// Prune stale pending blocks from the finality tracker.
    pub fn prune_finality(&mut self, current_slot: u32) {
        self.finality_tracker.prune_old_pending(current_slot);
    }

    /// Get the last finalized height, if any.
    pub fn last_finalized_height(&self) -> Option<u64> {
        self.finality_tracker
            .last_finalized
            .as_ref()
            .map(|c| c.height)
    }

    // =========================================================================
    // DIAGNOSTICS
    // =========================================================================

    /// Check if sync failures indicate we're on a fork (no-op, kept for API compatibility)
    pub fn has_sync_failure_fork_indicator(&self) -> bool {
        false
    }

    /// Network tip height (best seen via gossip or peer status)
    pub fn network_tip_height(&self) -> u64 {
        self.network.network_tip_height
    }

    /// Network tip slot (best seen via gossip or peer status)
    pub fn network_tip_slot(&self) -> u32 {
        self.network.network_tip_slot
    }

    /// Get consecutive sync failure count (for health diagnostics)
    pub fn consecutive_sync_failure_count(&self) -> u32 {
        self.fork.consecutive_sync_failures
    }

    /// Get consecutive empty header response count (for shallow fork detection)
    pub fn consecutive_empty_headers(&self) -> u32 {
        self.fork.consecutive_empty_headers
    }

    /// Reset empty headers counter after a rollback changes the local tip.
    /// The next sync attempt will use the new tip hash.
    pub fn reset_empty_headers(&mut self) {
        self.fork.consecutive_empty_headers = 0;
    }

    /// Check if post-recovery grace period is active.
    /// During grace, fork_sync should not be activated — the node needs time
    /// to sync via header-first / gossip before fork detection is meaningful.
    pub fn post_recovery_grace_active(&self) -> bool {
        matches!(
            self.recovery_phase,
            super::RecoveryPhase::PostRecoveryGrace { .. }
        )
    }

    /// Check if a stuck-fork signal was raised by cleanup or apply-failure detection.
    /// Reads and clears the signal (transitions from StuckForkDetected → Normal).
    pub fn take_stuck_fork_signal(&mut self) -> bool {
        if matches!(self.recovery_phase, super::RecoveryPhase::StuckForkDetected) {
            self.recovery_phase = super::RecoveryPhase::Normal;
            true
        } else {
            false
        }
    }

    /// Signal a stuck fork. Only transitions to StuckForkDetected from Normal
    /// or PostRollback phases — other phases have higher priority.
    pub fn signal_stuck_fork(&mut self) {
        match self.recovery_phase {
            super::RecoveryPhase::Normal | super::RecoveryPhase::PostRollback => {
                self.recovery_phase = super::RecoveryPhase::StuckForkDetected;
            }
            _ => {
                // Don't override active resync, post-recovery grace, or snap sync
                debug!(
                    "Stuck fork signal ignored — recovery phase {:?} has priority",
                    self.recovery_phase
                );
            }
        }
    }

    /// Activate post-recovery grace period. Called after snap sync / forced recovery.
    pub fn set_post_recovery_grace(&mut self) {
        self.recovery_phase = super::RecoveryPhase::PostRecoveryGrace {
            started: Instant::now(),
            blocks_applied: 0,
        };
        self.fork.consecutive_empty_headers = 0;
        self.fork.consecutive_apply_failures = 0;
        info!("Post-recovery grace activated: fork_sync suppressed until 10 blocks applied or 120s timeout.");
    }

    /// Check if sync manager has signaled that a full genesis resync is needed.
    /// Returns false if snap sync is disabled (--no-snap-sync), regardless of
    /// how many internal paths set the flag. This is the SINGLE gate that
    /// prevents snap sync from firing when the operator has forbidden it.
    pub fn needs_genesis_resync(&self) -> bool {
        if self.snap.threshold == u64::MAX {
            // --no-snap-sync: allow the genesis resync signal through.
            // The recovery path (reset_state_only) preserves block data — it only
            // resets UTXO, ProducerSet, and ChainState to genesis, then header-first
            // sync rebuilds state from preserved blocks. No snap sync is needed.
            // Previously this hardcoded false, creating a permanent deadlock for
            // forked --no-snap-sync nodes with no recovery path.
            if self.fork.needs_genesis_resync {
                tracing::warn!(
                    "--no-snap-sync: genesis resync signal active (local_h={}, gap={}). \
                     Recovery will use header-first full resync (block data preserved).",
                    self.local_height,
                    self.best_peer_height().saturating_sub(self.local_height)
                );
            }
        }
        self.fork.needs_genesis_resync
    }

    /// Central gate for all genesis resync requests.
    ///
    /// Replaces 9 scattered `needs_genesis_resync = true` assignments with a single
    /// decision point that enforces:
    /// 1. Monotonic progress floor (won't reset below confirmed_height_floor)
    /// 2. No concurrent recovery (won't trigger if ResyncInProgress)
    /// 3. Rate limiting (max MAX_CONSECUTIVE_RESYNCS, with cooldown)
    /// 4. Snap sync availability (won't trigger if snap sync disabled)
    /// 5. Snap attempt limit (won't trigger after 3 failed snap attempts)
    ///
    /// Returns true if the request was honored, false if refused.
    pub fn request_genesis_resync(&mut self, reason: super::RecoveryReason) -> bool {
        // Gate 1: Monotonic progress floor
        if self.confirmed_height_floor > 0 {
            warn!(
                "[RECOVERY] Genesis resync REFUSED: confirmed_height_floor={} \
                 (reason: {:?}). Manual intervention required.",
                self.confirmed_height_floor, reason
            );
            return false;
        }

        // Gate 2: No concurrent recovery
        if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
            info!(
                "[RECOVERY] Genesis resync REFUSED: resync already in progress \
                 (reason: {:?})",
                reason
            );
            return false;
        }

        // Gate 3: Rate limiting
        if self.consecutive_resync_count >= super::MAX_CONSECUTIVE_RESYNCS {
            warn!(
                "[RECOVERY] Genesis resync REFUSED: {} consecutive resyncs (max {}) \
                 (reason: {:?}). Manual intervention required.",
                self.consecutive_resync_count,
                super::MAX_CONSECUTIVE_RESYNCS,
                reason
            );
            return false;
        }

        // Gate 4: Snap sync must be available
        if self.snap.threshold == u64::MAX {
            info!(
                "[RECOVERY] Genesis resync REFUSED: snap sync disabled \
                 (reason: {:?}). Header-first recovery only.",
                reason
            );
            return false;
        }

        // Gate 5: Snap attempt limit
        if self.snap.attempts >= 3 {
            info!(
                "[RECOVERY] Genesis resync REFUSED: snap attempts exhausted ({}/3) \
                 (reason: {:?})",
                self.snap.attempts, reason
            );
            return false;
        }

        // All gates passed -- honor the request
        info!(
            "[RECOVERY] Genesis resync ACCEPTED: {:?} \
             (floor={}, resync_count={}, snap_attempts={}, phase={:?})",
            reason,
            self.confirmed_height_floor,
            self.consecutive_resync_count,
            self.snap.attempts,
            self.recovery_phase
        );
        self.fork.needs_genesis_resync = true;
        true
    }

    /// Returns true if peers consistently reject our chain tip (deep fork).
    /// Requires ALL conditions:
    /// 1. Many consecutive empty header responses (peers don't recognize our chain)
    /// 2. We are significantly behind peers (not just a 1-block fork)
    /// 3. At least one peer is at a similar height (within 100 blocks) — proving
    ///    they SHOULD have our block range. Peers far ahead may be snap-synced
    ///    without old blocks; empty responses from them are history gaps, not forks.
    ///
    /// Short forks (1-2 blocks) are normal and resolve naturally via heaviest chain.
    /// Only trigger genesis resync for genuine deep forks where we're stuck.
    pub fn is_deep_fork_detected(&self) -> bool {
        if self.fork.consecutive_empty_headers < 10 {
            return false;
        }
        // Must be significantly behind peers to qualify as deep fork
        let best_peer_height = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        if best_peer_height <= self.local_height + 5 {
            return false;
        }
        // Small gaps (≤12 blocks) are NOT deep forks — resolve_shallow_fork()
        // and fork_sync can handle them via rollback without wiping state.
        // Snap sync for small gaps loses block history and creates a cascade:
        // snap → no block 1 → next fork → rollback impossible → re-snap.
        let gap = best_peer_height.saturating_sub(self.local_height);
        if gap <= 12 {
            return false;
        }
        // If snap sync can handle this gap, don't escalate to deep fork.
        // next_request() will attempt snap sync first.
        let gap = best_peer_height.saturating_sub(self.local_height);
        let enough_peers = self.peers.len() >= 3;
        if enough_peers && gap > self.snap.threshold {
            return false;
        }
        // Require at least one peer whose height is close to ours (within 100 blocks).
        // If ALL peers are far ahead, empty headers likely mean they snap-synced
        // and lack our block range — not that we're on a fork.
        let has_close_peer = self
            .peers
            .values()
            .any(|p| p.best_height <= self.local_height + 100);
        has_close_peer
    }
}
