//! Active parent chain download for fork recovery
//!
//! When a node receives an orphan block whose parent is unknown,
//! this tracker walks backward through the fork chain by requesting
//! parent blocks from peers. Once the chain connects to a block in
//! our store, the fork can be evaluated for a potential reorg.
//!
//! This is the Bitcoin-like "request missing ancestors" mechanism.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Block;
use libp2p::PeerId;
use tracing::{info, warn};

/// Maximum parent chain walk depth (matches MAX_REORG_DEPTH in reorg.rs)
const MAX_RECOVERY_DEPTH: usize = 1000;

/// Cooldown between recovery attempts
const RECOVERY_COOLDOWN: Duration = Duration::from_secs(30);

/// Walk session timeout, measured from the last accepted walk block
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout for a single block request before trying a different peer
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Bound on remembered (branch tip, local tip) verdicts that did not win fork choice.
const MAX_REJECTED_BRANCHES: usize = 32;

/// Active recovery session state
struct ActiveRecovery {
    /// Blocks collected so far, newest first (tip → ancestor direction)
    blocks: Vec<Block>,
    /// Next parent hash to fetch
    next_parent: Hash,
    /// Whether we're waiting for a response
    pending: bool,
    /// Peer to query
    peer: PeerId,
    /// When the last walk block was accepted (or the session started)
    last_progress_at: Instant,
    /// When the current request was sent (for per-request timeout / F11 failover)
    request_sent_at: Option<Instant>,
    /// Alternate peers to try if primary fails or times out
    alternate_peers: Vec<PeerId>,
}

/// Result of a completed fork recovery
pub struct CompletedRecovery {
    /// Blocks in FORWARD order (ancestor-child → fork tip)
    pub blocks: Vec<Block>,
    /// Hash where the fork connects to our chain (exists in block_store)
    pub connection_point: Hash,
}

/// Tracks active fork recovery via parent chain walking
pub struct ForkRecoveryTracker {
    /// Currently active recovery, if any
    active: Option<ActiveRecovery>,
    /// Cooldown: earliest time a new recovery can start
    cooldown_until: Option<Instant>,
    /// Set when recovery is cancelled because the fork exceeds MAX_RECOVERY_DEPTH.
    /// The node should escalate to force_resync_from_genesis().
    exceeded_max_depth: bool,
    /// Our chain's genesis hash — blocks from other chains are rejected.
    genesis_hash: Hash,
    /// (branch tip, local tip) pairs already walked and not adopted (LB-9).
    rejected: VecDeque<(Hash, Hash)>,
}

impl ForkRecoveryTracker {
    pub fn new() -> Self {
        Self {
            active: None,
            cooldown_until: None,
            exceeded_max_depth: false,
            genesis_hash: Hash::default(),
            rejected: VecDeque::new(),
        }
    }

    /// Set the genesis hash for cross-chain block rejection.
    pub fn set_genesis_hash(&mut self, hash: Hash) {
        self.genesis_hash = hash;
    }

    /// Get the genesis hash.
    pub fn genesis_hash(&self) -> Hash {
        self.genesis_hash
    }

    /// Start recovery for an orphan block.
    /// Returns false if already recovering or on cooldown.
    pub fn start(&mut self, orphan_block: Block, peer: PeerId) -> bool {
        self.expire_stale_session();
        if self.active.is_some() {
            return false;
        }
        if let Some(until) = self.cooldown_until {
            if Instant::now() < until {
                return false;
            }
        }
        // Reject blocks from a different chain before starting recovery
        if !self.genesis_hash.is_zero() && orphan_block.header.genesis_hash != self.genesis_hash {
            warn!(
                "Fork recovery rejected: orphan block from different chain (genesis {})",
                &orphan_block.header.genesis_hash.to_hex()[..16]
            );
            return false;
        }
        let next_parent = orphan_block.header.prev_hash;
        info!(
            "Fork recovery: starting parent walk from block {} (parent={})",
            orphan_block.hash(),
            &next_parent.to_string()[..16]
        );
        self.active = Some(ActiveRecovery {
            next_parent,
            blocks: vec![orphan_block],
            pending: false,
            peer,
            last_progress_at: Instant::now(),
            request_sent_at: None,
            alternate_peers: Vec::new(),
        });
        true
    }

    /// Set alternate peers for failover during recovery (INC-I-012 F11).
    pub fn set_alternate_peers(&mut self, peers: Vec<PeerId>) {
        if let Some(recovery) = self.active.as_mut() {
            recovery.alternate_peers = peers.into_iter().filter(|p| *p != recovery.peer).collect();
        }
    }

    /// Get next fetch request. Returns (peer, hash_to_fetch) if a request should be sent.
    pub fn next_fetch(&mut self) -> Option<(PeerId, Hash)> {
        let recovery = self.active.as_mut()?;
        // Check session timeout
        if recovery.last_progress_at.elapsed() > RECOVERY_TIMEOUT {
            self.cancel("session timeout");
            return None;
        }
        // INC-I-012 F11: Per-request timeout with peer failover.
        // If the current request has been pending for >10s, switch to an
        // alternate peer and retry. This prevents a single slow/disconnected
        // peer from stalling the walk until the progress deadline.
        if recovery.pending {
            let sent_at = recovery.request_sent_at?;
            if sent_at.elapsed() <= REQUEST_TIMEOUT {
                return None; // Still waiting, not timed out yet
            }
            // No alternates left (`?` = None) — let the session timeout handle it
            let next_peer = recovery.alternate_peers.pop()?;
            warn!(
                "Fork recovery: request to {} timed out after {}s — failing over to {}",
                recovery.peer,
                sent_at.elapsed().as_secs(),
                next_peer
            );
            recovery.peer = next_peer;
            recovery.pending = false;
            // Fall through to send new request below
        }
        recovery.pending = true;
        recovery.request_sent_at = Some(Instant::now());
        Some((recovery.peer, recovery.next_parent))
    }

    /// Feed a block response. Returns true if consumed by recovery.
    pub fn handle_block(&mut self, peer: PeerId, block: Option<Block>) -> bool {
        let recovery = match self.active.as_mut() {
            Some(r) if r.pending && r.peer == peer => r,
            _ => return false,
        };
        // Content-keyed: a block that is not the requested parent is some other
        // response (e.g. an orphan chase) — pass it through, keep waiting.
        if matches!(&block, Some(blk) if blk.hash() != recovery.next_parent) {
            return false;
        }
        recovery.pending = false;

        match block {
            Some(blk) => {
                // Reject blocks from a different chain
                if !self.genesis_hash.is_zero() && blk.header.genesis_hash != self.genesis_hash {
                    warn!(
                        "Fork recovery cancelled: block from different chain (genesis {})",
                        &blk.header.genesis_hash.to_hex()[..16]
                    );
                    self.cancel("genesis hash mismatch");
                    return true;
                }
                recovery.next_parent = blk.header.prev_hash;
                recovery.blocks.push(blk);
                recovery.last_progress_at = Instant::now();
                // Depth check
                if recovery.blocks.len() > MAX_RECOVERY_DEPTH {
                    self.cancel("exceeded max depth");
                }
                true
            }
            None => {
                self.cancel("peer does not have block");
                true
            }
        }
    }

    /// Check if the chain connected to our store.
    /// Node calls this with the result of block_store.has_block(current_parent).
    /// Returns completed recovery if chain is connected.
    pub fn check_connection(&mut self, parent_known: bool) -> Option<CompletedRecovery> {
        if !parent_known {
            return None;
        }
        let recovery = self.active.take()?;
        let connection_point = recovery.next_parent;
        self.cooldown_until = Some(Instant::now() + RECOVERY_COOLDOWN);

        // Reverse blocks to forward order (ancestor-child first, fork tip last)
        let mut blocks = recovery.blocks;
        blocks.reverse();

        info!(
            "Fork recovery: chain connected at {} ({} blocks)",
            &connection_point.to_string()[..16],
            blocks.len()
        );

        Some(CompletedRecovery {
            connection_point,
            blocks,
        })
    }

    /// Get current parent hash being sought (for Node to check block_store)
    pub fn current_parent(&self) -> Option<Hash> {
        let recovery = self.active.as_ref()?;
        if recovery.pending {
            // Still waiting for response, don't check yet
            return None;
        }
        Some(recovery.next_parent)
    }

    /// Is recovery active?
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Can a new recovery start? (not active and not on cooldown)
    pub fn can_start(&self) -> bool {
        if matches!(&self.active, Some(r) if r.last_progress_at.elapsed() <= RECOVERY_TIMEOUT) {
            return false;
        }
        if let Some(until) = self.cooldown_until {
            if Instant::now() < until {
                return false;
            }
        }
        true
    }

    /// Cancel recovery with reason, start cooldown
    pub fn cancel(&mut self, reason: &str) {
        if reason == "exceeded max depth" {
            self.exceeded_max_depth = true;
            warn!(
                "Fork exceeds {} blocks — node should escalate to genesis resync",
                MAX_RECOVERY_DEPTH
            );
        }
        if self.active.is_some() {
            warn!("Fork recovery cancelled: {}", reason);
            self.active = None;
            self.cooldown_until = Some(Instant::now() + RECOVERY_COOLDOWN);
        }
    }

    /// Drop a session past RECOVERY_TIMEOUT even if `next_fetch` never ran.
    fn expire_stale_session(&mut self) {
        if matches!(&self.active, Some(r) if r.last_progress_at.elapsed() > RECOVERY_TIMEOUT) {
            self.cancel("session timeout");
        }
    }

    /// Remember that the branch ending at `branch_tip` lost fork choice against `local_tip`.
    pub fn mark_branch_rejected(&mut self, branch_tip: Hash, local_tip: Hash) {
        if self.is_branch_rejected(branch_tip, local_tip) {
            return;
        }
        if self.rejected.len() >= MAX_REJECTED_BRANCHES {
            self.rejected.pop_front();
        }
        self.rejected.push_back((branch_tip, local_tip));
    }

    /// Was this branch already walked and not adopted while on this local tip?
    pub fn is_branch_rejected(&self, branch_tip: Hash, local_tip: Hash) -> bool {
        self.rejected.contains(&(branch_tip, local_tip))
    }

    /// Check and consume the exceeded-max-depth flag.
    /// Returns true exactly once after a max-depth cancellation.
    pub fn take_exceeded_max_depth(&mut self) -> bool {
        std::mem::take(&mut self.exceeded_max_depth)
    }
}

impl Default for ForkRecoveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "fork_recovery_tests.rs"]
mod tests;
