use std::collections::HashMap;
use std::time::{Duration, Instant};

use network::PeerId;

const MAX_TRACKED: usize = 1024;

/// Last ORPHAN_CHASE request time per (peer, height). Bounded: entries older
/// than the window are pruned on every decision, and the map is reset at
/// `MAX_TRACKED` (a reset can only re-allow a chase, never suppress one).
#[derive(Default)]
pub(crate) struct OrphanChaseGuard {
    last: HashMap<(PeerId, u64), Instant>,
}

impl OrphanChaseGuard {
    /// True when a by-height chase to `peer` for `height` should be sent now.
    pub(crate) fn should_chase(&mut self, peer: PeerId, height: u64, window: Duration) -> bool {
        let now = Instant::now();
        self.last.retain(|_, t| now.duration_since(*t) < window);
        if self.last.contains_key(&(peer, height)) {
            return false;
        }
        if self.last.len() >= MAX_TRACKED {
            self.last.clear();
        }
        self.last.insert((peer, height), now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_stays_bounded_under_distinct_keys() {
        let mut g = OrphanChaseGuard::default();
        let w = Duration::from_secs(60);
        for h in 0..(MAX_TRACKED as u64 * 3) {
            assert!(g.should_chase(PeerId::random(), h, w));
            assert!(g.last.len() <= MAX_TRACKED);
        }
    }
}
