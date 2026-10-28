//! Decode-boundary shape check and per-peer report throttle for gossip votes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crypto::{PublicKey, Signature};
use network::{PeerId, PeerScorer, PeerScorerConfig};
use updater::VoteMessage;

const MAX_TRACKED: usize = 1024;
const MAX_VOTE_VERSION_LEN: usize = 64;

/// Minimum interval between two malformed-vote reports for one peer.
pub(crate) const MALFORMED_VOTE_WINDOW: Duration = Duration::from_secs(60);

/// The voter key, when the version is 1..=64 ASCII graphic bytes, the voter id
/// is a public key and the signature field is hex of the signature length;
/// `None` marks the vote malformed.
pub(crate) fn parse_voter(vote: &VoteMessage) -> Option<PublicKey> {
    let v = vote.version.as_bytes();
    if v.is_empty() || v.len() > MAX_VOTE_VERSION_LEN || !v.iter().all(u8::is_ascii_graphic) {
        return None;
    }
    let voter = PublicKey::from_hex(&vote.producer_id).ok()?;
    Signature::from_hex(&vote.signature).ok()?;
    Some(voter)
}

/// Record a malformed vote from `peer`; the scorer is reset once it tracks
/// `MAX_TRACKED` peers.
pub(crate) fn record_malformed_bounded(scorer: &mut PeerScorer, peer: &PeerId) {
    if scorer.stats().peer_count >= MAX_TRACKED {
        *scorer = PeerScorer::new(PeerScorerConfig::default());
    }
    scorer.record_malformed(peer);
}

/// Last malformed-vote report time per peer. Bounded: entries older than the
/// window are pruned on every decision, and the map is reset at `MAX_TRACKED`
/// (a reset can only re-allow a report, never suppress one).
#[derive(Default)]
pub(crate) struct MalformedVoteThrottle {
    last: HashMap<PeerId, Instant>,
}

impl MalformedVoteThrottle {
    /// True when a malformed vote from `peer` should be recorded and logged now.
    pub(crate) fn should_report(&mut self, peer: PeerId, window: Duration) -> bool {
        let now = Instant::now();
        self.last.retain(|_, t| now.duration_since(*t) < window);
        if self.last.contains_key(&peer) {
            return false;
        }
        if self.last.len() >= MAX_TRACKED {
            self.last.clear();
        }
        self.last.insert(peer, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_within_window_is_suppressed_and_expired_is_reallowed() {
        let mut t = MalformedVoteThrottle::default();
        let (peer, other) = (PeerId::random(), PeerId::random());
        assert!(t.should_report(peer, MALFORMED_VOTE_WINDOW));
        assert!(!t.should_report(peer, MALFORMED_VOTE_WINDOW));
        assert!(t.should_report(other, MALFORMED_VOTE_WINDOW));
        assert!(t.should_report(peer, Duration::ZERO));
    }

    #[test]
    fn map_stays_bounded_under_distinct_peers() {
        let mut t = MalformedVoteThrottle::default();
        for _ in 0..(MAX_TRACKED * 3) {
            assert!(t.should_report(PeerId::random(), MALFORMED_VOTE_WINDOW));
            assert!(t.last.len() <= MAX_TRACKED);
        }
    }
}
