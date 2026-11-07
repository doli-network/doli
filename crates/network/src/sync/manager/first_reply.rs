//! First-reply latch: one bounded hold on building per connected session.

use std::time::{Duration, Instant};

use tracing::info;

use super::SyncManager;

/// A peer more than this many blocks above the local tip starts the hold.
const TALLER_GAP: u64 = 5;
const HOLD_BOUND: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub(super) struct FirstReplyLatch {
    pub(super) spent: bool,
    pub(super) armed_at: Option<Instant>,
}

impl FirstReplyLatch {
    pub(super) fn spend(&mut self, cause: &str) {
        if !self.spent && self.armed_at.is_some() {
            info!("[FIRST_REPLY] build hold released: {cause}");
        }
        self.spent = true;
    }
}

impl SyncManager {
    /// The height from which `can_produce` lets a Syncing state build.
    pub fn fork_choice_gate(&self) -> u64 {
        self.config.inc_i_288_fork_choice_activation_height
    }

    /// True while the hold applies. Peer input can start it, never extend it.
    pub fn first_reply_holds(&mut self, now: Instant) -> bool {
        if self.first_reply.spent {
            return false;
        }
        let best = self.best_peer_height();
        if best <= self.local_height.saturating_add(TALLER_GAP) {
            return false;
        }
        let armed_at = match self.first_reply.armed_at {
            Some(armed_at) => armed_at,
            None => {
                info!(
                    "[FIRST_REPLY] build hold started: local={} best_peer={}",
                    self.local_height, best
                );
                *self.first_reply.armed_at.insert(now)
            }
        };
        if now.saturating_duration_since(armed_at) >= HOLD_BOUND {
            self.first_reply.spend("bound reached");
            return false;
        }
        true
    }
}
