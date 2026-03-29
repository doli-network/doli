//! Peer scoring and reputation system
//!
//! This module implements a peer scoring system to track peer behavior
//! and protect the network from misbehaving nodes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, warn};

use crypto::Hash;
use doli_core::Slot;

/// An infraction committed by a peer
#[derive(Clone, Debug)]
pub enum Infraction {
    /// Peer sent an invalid block
    InvalidBlock { slot: Slot },
    /// Peer sent an invalid transaction
    InvalidTransaction { hash: Hash },
    /// Peer timed out on a request
    Timeout { count: u32 },
    /// Peer is spamming the network
    Spam { msg_type: String },
    /// Peer sent duplicate data
    Duplicate,
    /// Peer sent data that doesn't deserialize
    MalformedMessage,
    /// Peer is running an incompatible protocol version
    IncompatibleVersion { their_version: u32 },
}

impl Infraction {
    /// Get the score penalty for this infraction
    fn penalty(&self) -> i32 {
        match self {
            Infraction::InvalidBlock { .. } => -100,
            Infraction::InvalidTransaction { .. } => -20,
            Infraction::Timeout { count } => -5 * (*count as i32).min(10),
            Infraction::Spam { .. } => -50,
            Infraction::Duplicate => -5,
            Infraction::MalformedMessage => -30,
            Infraction::IncompatibleVersion { .. } => -200,
        }
    }
}

/// Score information for a single peer
#[derive(Clone, Debug)]
pub struct PeerScore {
    /// Current score value (-1000 to +1000)
    pub value: i32,
    /// When the score was last updated
    pub last_update: Instant,
    /// Recent infractions
    pub infractions: Vec<(Instant, Infraction)>,
    /// Count of valid blocks received
    pub valid_blocks: u32,
    /// Count of valid transactions received
    pub valid_transactions: u32,
}

impl Default for PeerScore {
    fn default() -> Self {
        Self {
            value: 0,
            last_update: Instant::now(),
            infractions: Vec::new(),
            valid_blocks: 0,
            valid_transactions: 0,
        }
    }
}

impl PeerScore {
    /// Create a new peer score
    pub fn new() -> Self {
        Self::default()
    }

    /// Add points to the score (clamped to -1000..1000)
    fn add(&mut self, points: i32) {
        self.value = (self.value + points).clamp(-1000, 1000);
        self.last_update = Instant::now();
    }

    /// Record an infraction
    fn record_infraction(&mut self, infraction: Infraction) {
        let penalty = infraction.penalty();
        self.add(penalty);
        self.infractions.push((Instant::now(), infraction));

        // Keep only recent infractions (last hour)
        let cutoff = Instant::now() - Duration::from_secs(3600);
        self.infractions.retain(|(time, _)| *time > cutoff);
    }

    /// Decay score towards zero over time
    pub fn decay(&mut self, decay_rate: f64) {
        let elapsed = self.last_update.elapsed().as_secs_f64();
        if elapsed > 60.0 {
            // Decay 1 point per minute towards zero
            let decay = (elapsed / 60.0 * decay_rate) as i32;
            if self.value > 0 {
                self.value = (self.value - decay).max(0);
            } else if self.value < 0 {
                self.value = (self.value + decay).min(0);
            }
            self.last_update = Instant::now();
        }
    }
}

/// Configuration for the peer scorer
#[derive(Clone, Debug)]
pub struct PeerScorerConfig {
    /// Score threshold below which peers are disconnected
    pub disconnect_threshold: i32,
    /// Score threshold below which peers are banned
    pub ban_threshold: i32,
    /// Duration to ban a peer
    pub ban_duration: Duration,
    /// Decay rate (points per minute towards zero)
    pub decay_rate: f64,
    /// Maximum number of infractions to keep per peer
    pub max_infractions: usize,
}

impl Default for PeerScorerConfig {
    fn default() -> Self {
        Self {
            disconnect_threshold: -200,
            ban_threshold: -500,
            ban_duration: Duration::from_secs(3600), // 1 hour
            decay_rate: 1.0,
            max_infractions: 100,
        }
    }
}

/// Peer reputation scorer
#[derive(Debug)]
pub struct PeerScorer {
    /// Configuration
    config: PeerScorerConfig,
    /// Scores for each peer
    scores: HashMap<PeerId, PeerScore>,
    /// Banned peers with ban expiry time
    banned: HashMap<PeerId, Instant>,
}

impl PeerScorer {
    /// Create a new peer scorer
    pub fn new(config: PeerScorerConfig) -> Self {
        Self {
            config,
            scores: HashMap::new(),
            banned: HashMap::new(),
        }
    }

    /// Get or create a score for a peer
    fn get_or_create_score(&mut self, peer: &PeerId) -> &mut PeerScore {
        self.scores.entry(*peer).or_default()
    }

    /// Get the score for a peer
    pub fn get_score(&self, peer: &PeerId) -> Option<&PeerScore> {
        self.scores.get(peer)
    }

    /// Record a valid block from a peer (+10 points)
    pub fn record_valid_block(&mut self, peer: &PeerId) {
        let score = self.get_or_create_score(peer);
        score.add(10);
        score.valid_blocks += 1;
        debug!(peer = %peer, score = score.value, "Recorded valid block");
    }

    /// Record an invalid block from a peer (-100 points)
    pub fn record_invalid_block(&mut self, peer: &PeerId, slot: Slot) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::InvalidBlock { slot });
        warn!(peer = %peer, slot, score = score.value, "Recorded invalid block");
    }

    /// Record a valid transaction from a peer (+1 point)
    pub fn record_valid_tx(&mut self, peer: &PeerId) {
        let score = self.get_or_create_score(peer);
        score.add(1);
        score.valid_transactions += 1;
    }

    /// Record an invalid transaction from a peer (-20 points)
    pub fn record_invalid_tx(&mut self, peer: &PeerId, hash: Hash) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::InvalidTransaction { hash });
        debug!(peer = %peer, score = score.value, "Recorded invalid transaction");
    }

    /// Record a timeout from a peer (-5 points)
    pub fn record_timeout(&mut self, peer: &PeerId) {
        let score = self.get_or_create_score(peer);
        let count = score
            .infractions
            .iter()
            .filter(|(_, inf)| matches!(inf, Infraction::Timeout { .. }))
            .count() as u32
            + 1;
        score.record_infraction(Infraction::Timeout { count });
        debug!(peer = %peer, score = score.value, "Recorded timeout");
    }

    /// Record spam from a peer (-50 points)
    pub fn record_spam(&mut self, peer: &PeerId, msg_type: &str) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::Spam {
            msg_type: msg_type.to_string(),
        });
        warn!(peer = %peer, msg_type, score = score.value, "Recorded spam");
    }

    /// Record a malformed message from a peer (-30 points)
    pub fn record_malformed(&mut self, peer: &PeerId) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::MalformedMessage);
        debug!(peer = %peer, score = score.value, "Recorded malformed message");
    }

    /// Record an incompatible protocol version from a peer (-200 points, instant disconnect)
    pub fn record_incompatible_version(&mut self, peer: &PeerId, their_version: u32) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::IncompatibleVersion { their_version });
        warn!(peer = %peer, their_version, score = score.value, "Recorded incompatible version");
    }

    /// Record a duplicate message from a peer (-5 points)
    pub fn record_duplicate(&mut self, peer: &PeerId) {
        let score = self.get_or_create_score(peer);
        score.record_infraction(Infraction::Duplicate);
    }

    /// Check if a peer should be disconnected
    pub fn should_disconnect(&self, peer: &PeerId) -> bool {
        if let Some(score) = self.scores.get(peer) {
            score.value < self.config.disconnect_threshold
        } else {
            false
        }
    }

    /// Check if a peer should be banned
    pub fn should_ban(&self, peer: &PeerId) -> bool {
        if let Some(score) = self.scores.get(peer) {
            score.value < self.config.ban_threshold
        } else {
            false
        }
    }

    /// Ban a peer
    pub fn ban(&mut self, peer: &PeerId) {
        let expiry = Instant::now() + self.config.ban_duration;
        self.banned.insert(*peer, expiry);
        warn!(peer = %peer, duration_secs = self.config.ban_duration.as_secs(), "Banned peer");
    }

    /// Check if a peer is banned
    pub fn is_banned(&self, peer: &PeerId) -> bool {
        if let Some(expiry) = self.banned.get(peer) {
            *expiry > Instant::now()
        } else {
            false
        }
    }

    /// Remove a peer's score (on disconnect)
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.scores.remove(peer);
    }

    /// Decay all scores and clean up expired bans
    pub fn tick(&mut self) {
        // Decay scores
        for score in self.scores.values_mut() {
            score.decay(self.config.decay_rate);
        }

        // Remove expired bans
        let now = Instant::now();
        self.banned.retain(|_, expiry| *expiry > now);
    }

    /// Get all peers that should be disconnected
    pub fn peers_to_disconnect(&self) -> Vec<PeerId> {
        self.scores
            .iter()
            .filter(|(_, score)| score.value < self.config.disconnect_threshold)
            .map(|(peer, _)| *peer)
            .collect()
    }

    /// Get statistics
    pub fn stats(&self) -> ScorerStats {
        let scores: Vec<i32> = self.scores.values().map(|s| s.value).collect();
        let avg = if scores.is_empty() {
            0.0
        } else {
            scores.iter().sum::<i32>() as f64 / scores.len() as f64
        };

        ScorerStats {
            peer_count: self.scores.len(),
            banned_count: self.banned.len(),
            average_score: avg,
            min_score: scores.iter().min().copied().unwrap_or(0),
            max_score: scores.iter().max().copied().unwrap_or(0),
        }
    }
}

/// Statistics from the peer scorer
#[derive(Clone, Debug)]
pub struct ScorerStats {
    /// Number of peers with scores
    pub peer_count: usize,
    /// Number of banned peers
    pub banned_count: usize,
    /// Average peer score
    pub average_score: f64,
    /// Minimum peer score
    pub min_score: i32,
    /// Maximum peer score
    pub max_score: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_peer_id() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn test_peer_score_default() {
        let score = PeerScore::new();
        assert_eq!(score.value, 0);
        assert!(score.infractions.is_empty());
    }

    #[test]
    fn test_peer_score_add() {
        let mut score = PeerScore::new();
        score.add(10);
        assert_eq!(score.value, 10);

        score.add(-15);
        assert_eq!(score.value, -5);
    }

    #[test]
    fn test_peer_score_clamped() {
        let mut score = PeerScore::new();
        score.add(2000);
        assert_eq!(score.value, 1000);

        score.add(-3000);
        assert_eq!(score.value, -1000);
    }

    #[test]
    fn test_valid_block_increases_score() {
        let mut scorer = PeerScorer::new(PeerScorerConfig::default());
        let peer = test_peer_id();

        scorer.record_valid_block(&peer);

        let score = scorer.get_score(&peer).unwrap();
        assert_eq!(score.value, 10);
        assert_eq!(score.valid_blocks, 1);
    }

    #[test]
    fn test_invalid_block_decreases_score() {
        let mut scorer = PeerScorer::new(PeerScorerConfig::default());
        let peer = test_peer_id();

        scorer.record_invalid_block(&peer, 100);

        let score = scorer.get_score(&peer).unwrap();
        assert_eq!(score.value, -100);
        assert_eq!(score.infractions.len(), 1);
    }

    #[test]
    fn test_should_disconnect() {
        let config = PeerScorerConfig {
            disconnect_threshold: -200,
            ..Default::default()
        };
        let mut scorer = PeerScorer::new(config);
        let peer = test_peer_id();

        // Not disconnected initially
        scorer.record_valid_block(&peer);
        assert!(!scorer.should_disconnect(&peer));

        // Record enough invalid blocks to trigger disconnect
        scorer.record_invalid_block(&peer, 1);
        scorer.record_invalid_block(&peer, 2);
        scorer.record_invalid_block(&peer, 3);

        assert!(scorer.should_disconnect(&peer));
    }

    #[test]
    fn test_should_ban() {
        let config = PeerScorerConfig {
            ban_threshold: -500,
            ..Default::default()
        };
        let mut scorer = PeerScorer::new(config);
        let peer = test_peer_id();

        // Record many invalid blocks
        for i in 0..6 {
            scorer.record_invalid_block(&peer, i);
        }

        assert!(scorer.should_ban(&peer));
    }

    #[test]
    fn test_ban_peer() {
        let mut scorer = PeerScorer::new(PeerScorerConfig::default());
        let peer = test_peer_id();

        assert!(!scorer.is_banned(&peer));
        scorer.ban(&peer);
        assert!(scorer.is_banned(&peer));
    }

    #[test]
    fn test_stats() {
        let mut scorer = PeerScorer::new(PeerScorerConfig::default());
        let peer1 = test_peer_id();
        let peer2 = test_peer_id();

        scorer.record_valid_block(&peer1);
        scorer.record_valid_block(&peer1);
        scorer.record_invalid_block(&peer2, 1);

        let stats = scorer.stats();
        assert_eq!(stats.peer_count, 2);
        assert_eq!(stats.min_score, -100);
        assert_eq!(stats.max_score, 20);
    }
}
