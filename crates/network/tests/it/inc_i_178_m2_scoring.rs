//! INC-I-178 M2 — the peer-scoring budget for a bad BLS attestation half.
//!
//! The penalty is a NETWORK-SAFETY number, not a taste question: an invalid BLS
//! signature is relayed by honest peers, so the cost per event decides whether one
//! misconfigured producer can partition the mesh (INV-NETWORK-002).
//!
//! OUTPUT CONTRACT
//!
//! F1: `PeerScorer::record_invalid_bls_attestation(&mut self, peer: &PeerId)`
//!   Observable outputs:
//!     O1 return — unit
//!     O2 `self` mutation — `get_score(peer).value` and `.infractions`
//!     O3 derived reads — `should_disconnect(peer)`, `should_ban(peer)`,
//!        `peers_to_disconnect()`
//!     O4 persistent store writes — NONE (the scorer is in-memory)
//!   Paths:
//!     P1 first infraction for an unseen peer -> score created at -10
//!     P2 repeated infractions               -> linear accumulation
//!     P3 accumulation reaching exactly the disconnect threshold
//!     P4 accumulation past the threshold
//!   INPUT PARTITIONS: 0, 1, 20 and 21 infractions from ONE peer; a second,
//!     untouched peer as the control (scoring must be per-peer).
//!
//! F2: `Infraction::InvalidBlsAttestation` — the variant itself. `penalty()` is
//!   private, so the -10 is observed through F1, which is the only way production
//!   ever reaches it.

use network::{Infraction, PeerId, PeerScorer, PeerScorerConfig};

const PENALTY: i32 = -10;

fn scorer() -> PeerScorer {
    PeerScorer::new(PeerScorerConfig::default())
}

fn value(s: &PeerScorer, p: &PeerId) -> i32 {
    s.get_score(p).map(|x| x.value).unwrap_or(0)
}

// REQ-BLS-006 — Decision: the per-event cost. Too high and one honest relay of one
// misconfigured producer is ejected within a few blocks; too low and a deliberate
// forgery flood costs the attacker nothing.
#[test]
fn m2_scoring_an_invalid_bls_attestation_costs_exactly_ten_points() {
    let mut s = scorer();
    let peer = PeerId::random();
    let control = PeerId::random();
    assert_eq!(value(&s, &peer), 0, "an unseen peer starts at zero");

    s.record_invalid_bls_attestation(&peer);

    assert_eq!(value(&s, &peer), PENALTY);
    assert_eq!(
        s.get_score(&peer).map(|x| x.infractions.len()),
        Some(1),
        "the infraction is retained for the decay window"
    );
    assert_eq!(value(&s, &control), 0, "scoring is per-peer, not global");
    let _variant = Infraction::InvalidBlsAttestation;
}

// REQ-BLS-010 — Decision: the liveness bound. If ONE relayed forgery could
// disconnect a peer, an attacker would only need to gossip a single mutated
// attestation per victim to shred the mesh.
#[test]
fn m2_scoring_one_invalid_bls_attestation_is_never_disconnectable_or_bannable() {
    let mut s = scorer();
    let peer = PeerId::random();

    s.record_invalid_bls_attestation(&peer);

    assert!(!s.should_disconnect(&peer), "-10 is far above -200");
    assert!(!s.should_ban(&peer));
    assert!(s.peers_to_disconnect().is_empty());
}

// REQ-BLS-010 — Decision: pins the exact budget the design was sized against. The
// threshold test is strict `<`, so 20 infractions land ON -200 and do NOT eject;
// the 21st does. A change to either number silently re-sizes the attack budget.
#[test]
fn m2_scoring_twenty_infractions_land_on_the_threshold_and_the_twenty_first_crosses_it() {
    let mut s = scorer();
    let peer = PeerId::random();

    for _ in 0..20 {
        s.record_invalid_bls_attestation(&peer);
    }
    assert_eq!(value(&s, &peer), 20 * PENALTY, "linear accumulation");
    assert_eq!(
        value(&s, &peer),
        PeerScorerConfig::default().disconnect_threshold
    );
    assert!(
        !s.should_disconnect(&peer),
        "the threshold test is strict `<`: exactly -200 is not yet disconnectable"
    );

    s.record_invalid_bls_attestation(&peer);
    assert_eq!(value(&s, &peer), 21 * PENALTY);
    assert!(s.should_disconnect(&peer), "the 21st infraction crosses it");
    assert!(
        !s.should_ban(&peer),
        "-210 is still above the -500 ban line"
    );
    assert_eq!(s.peers_to_disconnect(), vec![peer]);
}
