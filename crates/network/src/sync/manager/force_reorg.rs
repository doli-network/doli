//! INC-I-204 M4.1 / REQ-FORK-012 — the `forceReorgTo` operator directive.
//!
//! The directive lives here and nowhere else: memory-only, one slot, three
//! independent bounds (wall clock, height span, single-shot). `SyncManager` is
//! already shared by `Node` and `RpcContext`, so the RPC arms and the node
//! consumes without new plumbing.

use std::time::{Duration, Instant};

use crypto::Hash;
use tracing::{info, warn};

use super::branch_verdict::BRANCH_VERDICT_TTL;
use super::recovery::thresholds::MINOR_FORK_GAP_MAX;
use super::SyncManager;

/// Wall-clock lifetime of an armed directive, anchored to `BRANCH_VERDICT_TTL`.
pub const FORCE_REORG_TTL_SECS: u64 = BRANCH_VERDICT_TTL.as_secs();

/// Blocks the local tip may advance past `armed_at_height` before the node is
/// judged to have resumed on its own. Anchored to `MINOR_FORK_GAP_MAX`, the
/// band the wedge lives in.
pub const FORCE_REORG_MAX_HEIGHT_SPAN: u64 = MINOR_FORK_GAP_MAX;

/// One operator-named reorg target, with the bounds that end it.
pub struct ForceReorgDirective {
    target: Hash,
    armed_at: Instant,
    armed_at_height: u64,
}

/// Result of inspecting the directive slot. `Armed` is non-destructive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceReorgPoll {
    Idle,
    Expired,
    Armed(Hash),
}

impl SyncManager {
    /// Arm a force-reorg directive. Replaces any previous one; never queues.
    pub fn arm_force_reorg(&mut self, target: Hash) {
        let replaced = self.force_reorg.as_ref().map(|d| d.target);
        self.force_reorg = Some(ForceReorgDirective {
            target,
            armed_at: Instant::now(),
            armed_at_height: self.local_height,
        });
        info!(
            "[FORCE_REORG] armed target={} armed_at_height={} ttl_secs={} max_height_span={} replaced={:?}",
            target, self.local_height, FORCE_REORG_TTL_SECS, FORCE_REORG_MAX_HEIGHT_SPAN, replaced
        );
    }

    /// Non-consuming peek at the armed target.
    pub fn force_reorg_target(&self) -> Option<Hash> {
        self.force_reorg.as_ref().map(|d| d.target)
    }

    /// Inspect the directive against both bounds. Clears the slot on expiry;
    /// leaves it untouched otherwise.
    pub fn poll_force_reorg(&mut self, now: Instant, local_height: u64) -> ForceReorgPoll {
        let Some(directive) = self.force_reorg.as_ref() else {
            return ForceReorgPoll::Idle;
        };

        let age = now.saturating_duration_since(directive.armed_at);
        let advanced = local_height.saturating_sub(directive.armed_at_height);
        let stale = age > Duration::from_secs(FORCE_REORG_TTL_SECS);
        let overtaken = advanced > FORCE_REORG_MAX_HEIGHT_SPAN;

        if stale || overtaken {
            warn!(
                "[FORCE_REORG] expired target={} age_secs={} advanced={} stale={} overtaken={}",
                directive.target,
                age.as_secs(),
                advanced,
                stale,
                overtaken
            );
            self.force_reorg = None;
            return ForceReorgPoll::Expired;
        }

        ForceReorgPoll::Armed(directive.target)
    }

    /// Spend the single shot. Returns the target exactly once.
    pub fn consume_force_reorg(&mut self) -> Option<Hash> {
        self.force_reorg.take().map(|d| d.target)
    }
}
