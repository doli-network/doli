//! INC-I-120 Milestone 1 — Outbound sync-request governor (Layer 1).
//!
//! Structural fix for the mainnet fleet-collapse amplification storm: DOLI's
//! sync request/response subsystem had inbound serving caps but ZERO outbound
//! rate governance. A natural fork therefore self-amplified into a ~40 req/s
//! busy-retry loop (3.5M req/node/day) → resource collapse → fleet death.
//!
//! These tests FAIL before the fix (the governor API does not exist) and PASS
//! after. They pin two contracts:
//!   1. The exemption classifier (`SyncRequest::is_rate_governed`) — G1 guardrail.
//!   2. The outbound governor (`RateLimiter::check_sync_request`) bounds the
//!      retry-storm classes.

use network::protocols::sync::SyncRequest;
use network::{RateLimitConfig, RateLimiter};

use crypto::Hash;
use libp2p::PeerId;

// OUTPUT CONTRACT: fn SyncRequest::is_rate_governed(&self) -> bool
// O1: true  — bulk-catchup/retry-storm class (GetHeaders, GetBodies, GetBlockByHeight)
// O2: false — recovery/canonical-critical class, NEVER throttled
//             (GetStateSnapshot, GetStateRoot, GetHeadersByHeight, GetBlockByHash, DirectAttestation)
// PATHS: one match arm per SyncRequest variant (8 variants).
// MATRIX: governed = {GetHeaders, GetBodies, GetBlockByHeight};
//         exempt   = {GetBlockByHash, GetStateSnapshot, GetStateRoot, GetHeadersByHeight, DirectAttestation}
//
// INPUT PARTITIONS:
//   - P-gov-1: governed variant (GetHeaders / GetBodies / GetBlockByHeight) → expect true
//   - P-gov-2: exempt variant (GetBlockByHash / GetStateSnapshot / GetStateRoot /
//              GetHeadersByHeight / DirectAttestation) → expect false
//   Each variant exercises a distinct match arm; both classes are covered by the
//   two tests below (`governed_classes_are_rate_governed`, `recovery_and_canonical_classes_are_exempt`).

/// INC-I-120 M1: the three bulk-catchup classes that drive the busy-retry
/// amplification storm MUST be governed (rate-limited).
#[test]
fn governed_classes_are_rate_governed() {
    assert!(
        SyncRequest::get_headers(Hash::ZERO, 100).is_rate_governed(),
        "GetHeaders is the primary busy-retry offender — must be governed"
    );
    assert!(
        SyncRequest::get_bodies(vec![Hash::ZERO]).is_rate_governed(),
        "GetBodies bulk catchup must be governed"
    );
    assert!(
        SyncRequest::get_block_by_height(1).is_rate_governed(),
        "GetBlockByHeight catchup must be governed"
    );
}

/// INC-I-120 M1 / G1 (from INC-I-049): recovery + canonical-critical classes
/// MUST bypass the governor entirely — throttling these is exactly the
/// INC-I-049 failure mode (limiter dropped a canonical block → 9-min fork).
#[test]
fn recovery_and_canonical_classes_are_exempt() {
    assert!(
        !SyncRequest::get_state_snapshot(Hash::ZERO).is_rate_governed(),
        "GetStateSnapshot (snap recovery) must be exempt"
    );
    assert!(
        !SyncRequest::get_state_root(Hash::ZERO).is_rate_governed(),
        "GetStateRoot (snap quorum) must be exempt"
    );
    assert!(
        !SyncRequest::get_headers_by_height(1, 100).is_rate_governed(),
        "GetHeadersByHeight (post-snap anchor recovery) must be exempt"
    );
    assert!(
        !SyncRequest::get_block_by_hash(Hash::ZERO).is_rate_governed(),
        "GetBlockByHash (orphan-chase canonical fetch) must be exempt"
    );
    assert!(
        !SyncRequest::DirectAttestation { data: vec![] }.is_rate_governed(),
        "DirectAttestation (causal push, not a fetch retry) must be exempt"
    );
}

// OUTPUT CONTRACT: fn RateLimiter::check_sync_request(&mut self, peer: &PeerId) -> bool
// O1: true  — a governed outbound sync request is permitted (tokens available)
// O2: false — the per-peer OR global sync-request bucket is empty → request dropped
// PATHS: P1 disabled config → always true (bypass);
//        P2 enabled + tokens available → true (caller then records);
//        P3 enabled + per-peer bucket exhausted → false.
// MATRIX: {enabled, disabled} × {tokens available, per-peer exhausted}
//
// INPUT PARTITIONS:
//   - P-rl-1: enabled config, repeated requests to ONE peer until drained
//             → relationship: sent count is bounded AND next check == false
//             (test `governor_bounds_outbound_per_peer`)
//   - P-rl-2: disabled config, arbitrarily many requests
//             → relationship: check always == true (test `governor_pass_through_when_disabled`)

/// INC-I-120 M1: the governor bounds the outbound sync-request RATE per peer.
/// Under the busy-retry storm a single node hammered one peer ~40 req/s; the
/// governor must reject once the per-peer bucket is drained.
#[test]
fn governor_bounds_outbound_per_peer() {
    let config = RateLimitConfig {
        max_sync_requests_per_second: 5, // small → small burst bucket
        ..Default::default()
    };
    let mut limiter = RateLimiter::new(config);
    let peer = PeerId::random();

    // Drain the per-peer burst bucket.
    let mut sent = 0u32;
    for _ in 0..1000 {
        if limiter.check_sync_request(&peer) {
            limiter.record_sync_request(&peer);
            sent += 1;
        } else {
            break;
        }
    }

    // The governor MUST have cut us off well below the 1000 attempts — the
    // whole point is that an unbounded retry loop cannot send unbounded requests.
    assert!(
        sent < 1000,
        "governor must bound outbound sync requests; sent={sent} (unbounded!)"
    );
    // And the next check is rejected (bucket empty).
    assert!(
        !limiter.check_sync_request(&peer),
        "per-peer sync-request bucket must be empty after draining"
    );
}

/// INC-I-120 M1: when rate limiting is disabled, the governor is a pass-through
/// (operator override / tests). Mirrors existing block/tx limiter behavior.
#[test]
fn governor_pass_through_when_disabled() {
    let config = RateLimitConfig {
        enabled: false,
        ..Default::default()
    };
    let mut limiter = RateLimiter::new(config);
    let peer = PeerId::random();

    for _ in 0..1000 {
        assert!(
            limiter.check_sync_request(&peer),
            "disabled governor must always permit"
        );
    }
}
