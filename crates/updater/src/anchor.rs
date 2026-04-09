//! Canonical anchor — hardcoded checkpoints for catastrophic recovery.
//!
//! A canonical anchor pins a `(height, block_hash, state_root)` tuple at
//! compile time. Blocks at the anchor height with a mismatched hash are
//! rejected, reorgs whose common ancestor sits below the highest anchor
//! are rejected, and snap sync snapshots with a mismatched state_root at
//! the anchor height are rejected.
//!
//! Canonical anchors are a **recovery** tool, not a prevention tool — added
//! only after catastrophic consensus failure (e.g. the INC-I-026 cascade of
//! 2026-04-09) to force the network onto a known-healthy prefix. A new
//! binary ships with a new anchor and every node on that binary becomes
//! immune to the hostile chain. Append-only: once shipped, an anchor is
//! never removed — only superseded by a newer anchor at a higher height.
//! Each anchor carries a `min_version` that mirrors `HardForkInfo` so that
//! partial fleet upgrades partition off the old half automatically.
//!
//! All `AnchorSchedule::for_network` schedules are currently empty. This
//! module is plumbing + enforcement only — real anchors land in a separate
//! PR after testnet cycle and two-reviewer approval.

use crypto::Hash;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// A single canonical anchor — the expected block at a specific height.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalAnchor {
    /// Block height at which this anchor is pinned.
    pub height: u64,
    /// The only valid block hash at `height`.
    pub hash: Hash,
    /// State root after applying the anchored block.
    pub state_root: Hash,
    /// Minimum binary version that knows about this anchor.
    ///
    /// Producers below this version stop producing at `height`, identical
    /// to `HardForkInfo::min_version` semantics. This makes partial fleet
    /// upgrades a forcing function — the old half cannot produce past the
    /// anchor and will be partitioned off the anchored chain.
    pub min_version: String,
    /// Human-readable reason (logs, audit, incident tracking).
    pub reason: String,
    /// Incident identifier that motivated this anchor (e.g. "INC-I-026").
    pub incident: Option<String>,
}

impl CanonicalAnchor {
    /// Return true if the given `(height, hash)` pair matches this anchor.
    pub fn matches(&self, height: u64, hash: Hash) -> bool {
        self.height == height && self.hash == hash
    }

    /// Return true if the given height is at or above this anchor's height.
    pub fn is_active(&self, current_height: u64) -> bool {
        current_height >= self.height
    }
}

/// Violation of a canonical anchor. Produced by `AnchorSchedule::validate_*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorViolation {
    /// The anchor height at which the violation occurred.
    pub height: u64,
    /// The hash or state_root that the anchor expected.
    pub expected: Hash,
    /// The hash or state_root that was actually observed.
    pub got: Hash,
    /// Kind of violation (block hash vs state root).
    pub kind: AnchorViolationKind,
    /// Human-readable reason from the anchor entry.
    pub reason: String,
}

/// Kind of canonical-anchor violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorViolationKind {
    /// Block hash at the anchor height did not match.
    BlockHash,
    /// State root at the anchor height did not match.
    StateRoot,
}

impl std::fmt::Display for AnchorViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field = match self.kind {
            AnchorViolationKind::BlockHash => "block hash",
            AnchorViolationKind::StateRoot => "state_root",
        };
        write!(
            f,
            "canonical anchor violated at h={}: expected {} {}, got {} ({})",
            self.height, field, self.expected, self.got, self.reason
        )
    }
}

impl std::error::Error for AnchorViolation {}

/// A sorted, append-only schedule of canonical anchors for a network.
///
/// Mirrors `HardForkSchedule` — entries are compile-time baked into the
/// binary via `AnchorSchedule::for_network`. All nodes on the same binary
/// share the same view. Zero coordination overhead.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnchorSchedule {
    anchors: Vec<CanonicalAnchor>,
}

impl AnchorSchedule {
    /// Create an empty schedule.
    pub fn new() -> Self {
        Self {
            anchors: Vec::new(),
        }
    }

    /// Add an anchor to the schedule (maintains sorted order by height).
    ///
    /// If an anchor already exists at the same height, the new anchor replaces
    /// it and a warn log is emitted. In production, anchors should only be
    /// added via `for_network` at compile time — runtime `add` is used by
    /// tests and by the guardian `anchor propose` command.
    pub fn add(&mut self, anchor: CanonicalAnchor) {
        if self.anchors.iter().any(|a| a.height == anchor.height) {
            warn!(
                "Canonical anchor at height {} already scheduled, replacing",
                anchor.height
            );
            self.anchors.retain(|a| a.height != anchor.height);
        }
        info!(
            "Scheduled canonical anchor at height {}: hash={} reason={:?} incident={:?}",
            anchor.height, anchor.hash, anchor.reason, anchor.incident
        );
        self.anchors.push(anchor);
        self.anchors.sort_by_key(|a| a.height);
    }

    /// Return the anchor at exactly `height`, if any.
    pub fn anchor_at(&self, height: u64) -> Option<&CanonicalAnchor> {
        self.anchors.iter().find(|a| a.height == height)
    }

    /// Return the highest anchor in the schedule (the rollback floor), if any.
    pub fn highest(&self) -> Option<&CanonicalAnchor> {
        self.anchors.last()
    }

    /// Return the highest anchor at or below `height`, if any.
    ///
    /// Useful when deciding whether a given chain tip is past any anchor.
    pub fn active_at(&self, height: u64) -> Option<&CanonicalAnchor> {
        self.anchors.iter().rev().find(|a| a.height <= height)
    }

    /// Return all anchors (sorted by height).
    pub fn all(&self) -> &[CanonicalAnchor] {
        &self.anchors
    }

    /// Return the number of anchors in the schedule.
    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    /// Return `true` if the schedule has no anchors.
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// Validate a block against any anchor at its height.
    ///
    /// Returns `Ok(())` if no anchor exists at the given height, or if the
    /// block's hash matches the anchor. Returns `Err(AnchorViolation)` if
    /// an anchor exists at that height and the hash does not match.
    pub fn validate_block(&self, height: u64, hash: Hash) -> Result<(), AnchorViolation> {
        if let Some(anchor) = self.anchor_at(height) {
            if hash != anchor.hash {
                return Err(AnchorViolation {
                    height,
                    expected: anchor.hash,
                    got: hash,
                    kind: AnchorViolationKind::BlockHash,
                    reason: anchor.reason.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validate a state_root against any anchor at its height.
    ///
    /// Used by snap sync to verify downloaded snapshots: if the peer delivers
    /// a snapshot at `anchor.height`, its state_root must match the anchor's.
    pub fn validate_state_root(
        &self,
        height: u64,
        state_root: Hash,
    ) -> Result<(), AnchorViolation> {
        if let Some(anchor) = self.anchor_at(height) {
            if state_root != anchor.state_root {
                return Err(AnchorViolation {
                    height,
                    expected: anchor.state_root,
                    got: state_root,
                    kind: AnchorViolationKind::StateRoot,
                    reason: anchor.reason.clone(),
                });
            }
        }
        Ok(())
    }

    /// Return `true` if a reorg with the given common-ancestor height is
    /// allowed by the anchor schedule.
    ///
    /// A reorg is allowed iff its common ancestor sits at or above the
    /// highest anchor's height. An empty schedule allows all reorgs.
    pub fn reorg_allowed(&self, common_ancestor_height: u64) -> bool {
        match self.highest() {
            Some(anchor) => common_ancestor_height >= anchor.height,
            None => true,
        }
    }

    /// Return `true` if `height` is strictly below the highest anchor
    /// (and therefore immutable — cannot be rolled back).
    pub fn height_is_anchored(&self, height: u64) -> bool {
        match self.highest() {
            Some(anchor) => height <= anchor.height,
            None => false,
        }
    }

    /// Return `true` if the binary should stop producing at `current_height`
    /// because the active anchor requires a newer minimum version.
    ///
    /// Mirrors `HardForkSchedule::should_stop_producing`. Used by the
    /// production gate to refuse producing when the node is running a
    /// binary too old to enforce the anchor.
    pub fn should_stop_producing(&self, current_height: u64, current_version: &str) -> bool {
        self.anchors.iter().any(|a| {
            a.is_active(current_height) && crate::is_newer_version(&a.min_version, current_version)
        })
    }

    /// Compile-time canonical anchor schedule for the given network.
    ///
    /// All networks currently return empty schedules — enforcement paths are
    /// live but no-op. Real anchors are added via a separate PR after a
    /// testnet cycle and two-reviewer approval. See module docs.
    pub fn for_network(_network: doli_core::Network) -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        Hash::from_bytes(bytes)
    }

    fn sample_anchor(height: u64, hash_byte: u8, min_version: &str) -> CanonicalAnchor {
        CanonicalAnchor {
            height,
            hash: h(hash_byte),
            state_root: h(hash_byte.wrapping_add(100)),
            min_version: min_version.to_string(),
            reason: format!("test anchor at h={}", height),
            incident: Some(format!("INC-TEST-{:03}", height)),
        }
    }

    #[test]
    fn empty_schedule_allows_everything() {
        let schedule = AnchorSchedule::new();
        assert!(schedule.is_empty());
        assert_eq!(schedule.len(), 0);
        assert!(schedule.highest().is_none());
        assert!(schedule.anchor_at(100).is_none());
        assert!(schedule.active_at(100).is_none());
        assert!(!schedule.height_is_anchored(100));
        assert!(schedule.reorg_allowed(0));
        assert!(schedule.reorg_allowed(u64::MAX));
        assert!(schedule.validate_block(100, h(42)).is_ok());
        assert!(schedule.validate_state_root(100, h(42)).is_ok());
        assert!(!schedule.should_stop_producing(1_000_000, "0.0.1"));
    }

    #[test]
    fn for_network_returns_empty_for_all_networks() {
        assert!(AnchorSchedule::for_network(doli_core::Network::Mainnet).is_empty());
        assert!(AnchorSchedule::for_network(doli_core::Network::Testnet).is_empty());
        assert!(AnchorSchedule::for_network(doli_core::Network::Devnet).is_empty());
    }

    #[test]
    fn add_maintains_sorted_order() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(200, 2, "6.8.0"));
        schedule.add(sample_anchor(100, 1, "6.7.9"));
        schedule.add(sample_anchor(300, 3, "6.9.0"));

        let all = schedule.all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].height, 100);
        assert_eq!(all[1].height, 200);
        assert_eq!(all[2].height, 300);
    }

    #[test]
    fn add_duplicate_height_replaces() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 1, "6.7.9"));
        schedule.add(sample_anchor(100, 2, "6.8.0"));

        assert_eq!(schedule.len(), 1);
        let anchor = schedule.anchor_at(100).unwrap();
        assert_eq!(anchor.hash, h(2));
        assert_eq!(anchor.min_version, "6.8.0");
    }

    #[test]
    fn highest_returns_last_anchor() {
        let mut schedule = AnchorSchedule::new();
        assert!(schedule.highest().is_none());

        schedule.add(sample_anchor(100, 1, "6.7.9"));
        assert_eq!(schedule.highest().unwrap().height, 100);

        schedule.add(sample_anchor(500, 5, "6.9.0"));
        assert_eq!(schedule.highest().unwrap().height, 500);

        schedule.add(sample_anchor(300, 3, "6.8.0"));
        assert_eq!(schedule.highest().unwrap().height, 500);
    }

    #[test]
    fn active_at_returns_highest_anchor_at_or_below() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 1, "6.7.9"));
        schedule.add(sample_anchor(300, 3, "6.8.0"));
        schedule.add(sample_anchor(500, 5, "6.9.0"));

        assert!(schedule.active_at(50).is_none());
        assert_eq!(schedule.active_at(100).unwrap().height, 100);
        assert_eq!(schedule.active_at(150).unwrap().height, 100);
        assert_eq!(schedule.active_at(300).unwrap().height, 300);
        assert_eq!(schedule.active_at(450).unwrap().height, 300);
        assert_eq!(schedule.active_at(500).unwrap().height, 500);
        assert_eq!(schedule.active_at(1_000_000).unwrap().height, 500);
    }

    #[test]
    fn validate_block_matches_anchor() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 42, "6.7.9"));

        // Match — OK.
        assert!(schedule.validate_block(100, h(42)).is_ok());
        // Different height — OK (no anchor there).
        assert!(schedule.validate_block(101, h(99)).is_ok());
        assert!(schedule.validate_block(99, h(99)).is_ok());
    }

    #[test]
    fn validate_block_rejects_anchor_mismatch() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 42, "6.7.9"));

        let err = schedule.validate_block(100, h(99)).unwrap_err();
        assert_eq!(err.height, 100);
        assert_eq!(err.expected, h(42));
        assert_eq!(err.got, h(99));
        assert_eq!(err.kind, AnchorViolationKind::BlockHash);
    }

    #[test]
    fn validate_state_root_matches_anchor() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 42, "6.7.9"));
        // sample_anchor sets state_root = hash_byte + 100 = 142
        assert!(schedule.validate_state_root(100, h(142)).is_ok());
    }

    #[test]
    fn validate_state_root_rejects_mismatch() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 42, "6.7.9"));

        let err = schedule.validate_state_root(100, h(99)).unwrap_err();
        assert_eq!(err.height, 100);
        assert_eq!(err.expected, h(142)); // sample_anchor: hash_byte + 100
        assert_eq!(err.got, h(99));
        assert_eq!(err.kind, AnchorViolationKind::StateRoot);
    }

    #[test]
    fn reorg_allowed_respects_highest_anchor() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 1, "6.7.9"));
        schedule.add(sample_anchor(500, 5, "6.9.0"));

        // Common ancestor below highest anchor — reject.
        assert!(!schedule.reorg_allowed(0));
        assert!(!schedule.reorg_allowed(99));
        assert!(!schedule.reorg_allowed(100));
        assert!(!schedule.reorg_allowed(499));
        // At highest anchor — allow.
        assert!(schedule.reorg_allowed(500));
        // Above highest anchor — allow.
        assert!(schedule.reorg_allowed(501));
        assert!(schedule.reorg_allowed(1_000_000));
    }

    #[test]
    fn height_is_anchored_returns_true_at_or_below_highest() {
        let mut schedule = AnchorSchedule::new();
        assert!(!schedule.height_is_anchored(0));

        schedule.add(sample_anchor(100, 1, "6.7.9"));
        assert!(schedule.height_is_anchored(0));
        assert!(schedule.height_is_anchored(99));
        assert!(schedule.height_is_anchored(100));
        assert!(!schedule.height_is_anchored(101));
        assert!(!schedule.height_is_anchored(u64::MAX));
    }

    #[test]
    fn should_stop_producing_respects_min_version() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 1, "6.8.0"));

        // Before anchor activates — no effect regardless of version.
        assert!(!schedule.should_stop_producing(99, "6.7.9"));
        // At anchor with old version — stop.
        assert!(schedule.should_stop_producing(100, "6.7.9"));
        // At anchor with current version — OK.
        assert!(!schedule.should_stop_producing(100, "6.8.0"));
        // Above anchor with old version — still stop.
        assert!(schedule.should_stop_producing(200, "6.7.9"));
        // Above anchor with newer version — OK.
        assert!(!schedule.should_stop_producing(200, "6.9.0"));
    }

    #[test]
    fn canonical_anchor_matches() {
        let anchor = sample_anchor(100, 42, "6.7.9");
        assert!(anchor.matches(100, h(42)));
        assert!(!anchor.matches(100, h(99)));
        assert!(!anchor.matches(101, h(42)));
    }

    #[test]
    fn canonical_anchor_is_active() {
        let anchor = sample_anchor(100, 1, "6.7.9");
        assert!(!anchor.is_active(0));
        assert!(!anchor.is_active(99));
        assert!(anchor.is_active(100));
        assert!(anchor.is_active(1_000_000));
    }

    #[test]
    fn violation_display_includes_context() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 42, "6.7.9"));
        let err = schedule.validate_block(100, h(99)).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("h=100"));
        assert!(msg.contains("test anchor"));
    }

    #[test]
    fn serde_roundtrip() {
        let mut schedule = AnchorSchedule::new();
        schedule.add(sample_anchor(100, 1, "6.7.9"));
        schedule.add(sample_anchor(500, 5, "6.8.0"));

        let json = serde_json::to_string(&schedule).unwrap();
        let decoded: AnchorSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded.anchor_at(100).unwrap().hash, h(1));
        assert_eq!(decoded.anchor_at(500).unwrap().hash, h(5));
    }
}
