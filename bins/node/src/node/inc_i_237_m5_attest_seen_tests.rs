//! INC-I-237 M5 (AUDIT-P1-506) — `AttestSeen`, the (attester, slot) newness set
//! behind the attestation relay verdict, and its 12-slot prune window.
//!
//! OUTPUT CONTRACT: AttestSeen::insert(&mut self, (PublicKey, u32)) -> bool;
//!   AttestSeen::prune(&mut self, best_slot: u32); AttestSeen::len(&self)
//!   O3: bool (first seen)   O5: the set (observed via insert / len)
//! PATHS: P1 new key -> true; P2 same key -> false; P3 other attester, same
//!   slot -> true; P4 prune drops slot < best - ATTEST_STALE_SLOTS, keeps the
//!   window; P5 best below the window -> keeps all (no underflow)
//! INPUT PARTITIONS: slot {0..=30, 17, 18, 30}; best {5, 30}; attester {a, b}
//! MATRIX: O3 x P1-P4, O5 x P4/P5 — asserted below.

use crypto::KeyPair;
use network::gossip::staleness::ATTEST_STALE_SLOTS;

use super::attestation::ingress::AttestSeen;

// REQ-GRV-SEC-002 (AUDIT-P1-506) — Decision: a failure means the newness set either grows without bound (no prune) or forgets a slot still inside the swarm's 12-slot attestation window, re-opening one Accept per variant.
#[test]
fn m5_audit_p1_506_attest_seen_is_first_seen_per_attester_slot_and_prunes_to_the_window() {
    let (a, b) = (
        *KeyPair::generate().public_key(),
        *KeyPair::generate().public_key(),
    );
    let mut seen = AttestSeen::default();
    assert!(seen.insert((a, 7)), "first (a, 7)");
    assert!(!seen.insert((a, 7)), "second (a, 7)");
    assert!(seen.insert((b, 7)), "another attester, same slot");
    seen.prune(5);
    assert_eq!(seen.len(), 2, "best below the window keeps everything");
    for s in 0..=30 {
        seen.insert((a, s));
    }
    seen.prune(30);
    let floor = 30 - ATTEST_STALE_SLOTS;
    assert_eq!(
        seen.len(),
        (30 - floor + 1) as usize,
        "window [best-12, best]"
    );
    assert!(
        !seen.insert((a, floor)),
        "slot best-12 is still inside the window"
    );
    assert!(seen.insert((a, floor - 1)), "slot best-13 was pruned");
}
