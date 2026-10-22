//! INC-I-255 M5 (AUDIT-P1-517) — the detector stores and pairs only headers its
//! `admit` predicate accepts, pays at most one check per distinct header before
//! a proof and a fixed budget after it, and keeps the smaller stored header.
//!
// OUTPUT CONTRACT: fn EquivocationDetector::check_block_with(&mut self, &Block, impl FnOnce(&BlockHeader) -> bool) -> Option<EquivocationProof>
//   O1 return: Some(proof) only for a pair of admitted headers that fits a SlashData
//   O2 calls of `admit` (counted per header hash by the closure)
//   O3 pending proofs (receiver), via take_pending_proofs()
//   O4 stored headers (receiver), via tracked_count() and which pair forms
// PATHS: before a proof | after a proof | re-delivery of the stored header | admit=false
//        | stored header oversize | many offenders
// MATRIX
//   O1,O2 x re-delivered stored header x10 -> None, 0 extra calls (inc_i_255_m5_detector_redelivered_stored_*)
//   O2 x 5 rejected + A + A x3 + B -> <= 1 call per header (inc_i_255_m5_detector_distinct_headers_*)
//   O2 x 10 headers after a proof -> <= MAX_EVIDENCE_CHECKS_AFTER_PROOF; other slot unaffected (inc_i_255_m5_detector_post_proof_*)
//   O1,O3,O4 x F rejected -> A -> B -> one proof (A,B) (inc_i_255_m5_detector_rejected_header_*)
//   O3 x 4 admitted headers, one (P,S) -> 1 pending; 200 offenders -> <= 64 (inc_i_255_m5_detector_pending_*)
//   O1 x oversize O with A then B (both orders) -> proof (A,B) (inc_i_255_m5_detector_keeps_smaller_*)
// INPUT PARTITIONS:
//   admit: true | false; delivery: once | repeated; size: small | oversize (2,100 missed producers)
//   offenders: 1 (one or two slots) | 200

use std::collections::{HashMap, HashSet};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, BlockHeader};
use network::{EquivocationDetector, EquivocationProof, MAX_EVIDENCE_CHECKS_AFTER_PROOF};
use vdf::{VdfOutput, VdfProof};

const SLOT: u32 = 100;
const PENDING_BOUND: usize = 64;
const OVERSIZE_MISSED: usize = 2_100;

fn block(slot: u32, producer: &PublicKey, salt: u8, missed: usize) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes([salt; 32]),
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: *producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: vec![*producer; missed],
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// Detector plus a per-header-hash count of `admit` calls.
#[derive(Default)]
struct Probe {
    d: EquivocationDetector,
    calls: HashMap<Hash, u32>,
}

impl Probe {
    fn check(&mut self, b: &Block, verdict: bool) -> Option<EquivocationProof> {
        let calls = &mut self.calls;
        self.d.check_block_with(b, |h| {
            *calls.entry(h.hash()).or_default() += 1;
            verdict
        })
    }

    fn total(&self) -> u32 {
        self.calls.values().sum()
    }

    fn of(&self, b: &Block) -> u32 {
        self.calls.get(&b.hash()).copied().unwrap_or(0)
    }
}

fn pair_of(p: &EquivocationProof) -> HashSet<Hash> {
    [p.block_header_1.hash(), p.block_header_2.hash()].into()
}

fn want(a: &Block, b: &Block) -> HashSet<Hash> {
    [a.hash(), b.hash()].into()
}

fn pk() -> PublicKey {
    *KeyPair::generate().public_key()
}

// REQ-SLASH-004 — Decision: a failure means a re-delivered stored header costs a check or pairs with itself (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_detector_redelivered_stored_header_never_admits_or_pairs() {
    let p = pk();
    let mut x = Probe::default();
    let a = block(SLOT, &p, 1, 0);
    assert!(
        x.check(&a, true).is_none(),
        "first header is stored, not paired"
    );
    for i in 0..10 {
        assert!(
            x.check(&a, true).is_none(),
            "re-delivery #{i} must not pair"
        );
    }
    assert_eq!(
        x.of(&a),
        1,
        "the stored header is checked once, re-delivery costs nothing"
    );
    assert_eq!(x.d.tracked_count(), 1, "one stored header");
}

// REQ-SLASH-004 — Decision: a failure means one header is evidence-checked more than once before a proof (unbounded VDF cost per header) (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_detector_distinct_headers_admit_at_most_once_before_proof() {
    let p = pk();
    let mut x = Probe::default();
    let decoys: Vec<Block> = (10..15).map(|s| block(SLOT, &p, s, 0)).collect();
    for f in &decoys {
        assert!(
            x.check(f, false).is_none(),
            "a rejected header yields no proof"
        );
    }
    let (a, b) = (block(SLOT, &p, 1, 0), block(SLOT, &p, 2, 0));
    x.check(&a, true);
    for _ in 0..3 {
        x.check(&a, true);
    }
    let proof = x.check(&b, true).expect("A then B must pair");
    assert_eq!(pair_of(&proof), want(&a, &b));
    for (h, n) in &x.calls {
        assert!(*n <= 1, "header {h} checked {n} times before the proof");
    }
    assert_eq!(
        (x.of(&a), x.of(&b)),
        (1, 1),
        "A and B each checked exactly once"
    );
}

// REQ-SLASH-004 — Decision: a failure means an offence keeps paying evidence checks after its proof, or its budget throttles another slot (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_detector_post_proof_admits_bounded() {
    const EXTRA: u8 = 10;
    assert!(
        u32::from(EXTRA) > MAX_EVIDENCE_CHECKS_AFTER_PROOF,
        "NON-VACUITY: exceed the budget"
    );
    let p = pk();
    let mut x = Probe::default();
    x.check(&block(SLOT, &p, 1, 0), true);
    assert!(
        x.check(&block(SLOT, &p, 2, 0), true).is_some(),
        "PRECONDITION: proof"
    );
    let before = x.total();
    for s in 0..EXTRA {
        x.check(&block(SLOT, &p, 100 + s, 0), true);
    }
    let after = x.total() - before;
    assert!(
        after <= MAX_EVIDENCE_CHECKS_AFTER_PROOF,
        "{after} admit calls after the proof exceed {MAX_EVIDENCE_CHECKS_AFTER_PROOF}"
    );
    let other = block(SLOT + 1, &p, 1, 0);
    x.check(&other, true);
    assert_eq!(
        x.of(&other),
        1,
        "another slot of the same producer is still checked"
    );
}

// REQ-SLASH-004 — Decision: a failure means a rejected decoy is stored or paired and blocks the valid pair (the AUDIT-P1-517 shape).
#[test]
fn inc_i_255_m5_detector_rejected_header_never_stored_or_paired() {
    let p = pk();
    let mut x = Probe::default();
    let f = block(SLOT, &p, 9, 0);
    assert!(x.check(&f, false).is_none());
    assert_eq!(
        x.d.tracked_count(),
        0,
        "a rejected header must not be stored"
    );
    let (a, b) = (block(SLOT, &p, 1, 0), block(SLOT, &p, 2, 0));
    assert!(
        x.check(&a, true).is_none(),
        "A must not pair with the rejected decoy"
    );
    let proof = x
        .check(&b, true)
        .expect("A then B must pair after a rejected decoy");
    assert_eq!(pair_of(&proof), want(&a, &b), "the proof is (A, B)");
    let pending = x.d.take_pending_proofs();
    assert_eq!(pending.len(), 1, "one pending proof");
    assert_eq!(
        pair_of(&pending[0]),
        want(&a, &b),
        "the pending proof is (A, B)"
    );
}

// REQ-SLASH-004 — Decision: a failure means one offence queues several pending proofs once the reported latch is gone.
#[test]
fn inc_i_255_m5_detector_pending_one_per_offence() {
    let p = pk();
    let mut x = Probe::default();
    for s in 1..=4 {
        x.check(&block(SLOT, &p, s, 0), true);
    }
    assert_eq!(
        x.d.take_pending_proofs().len(),
        1,
        "one (producer, slot) holds one pending proof"
    );
}

// REQ-SLASH-004 — Decision: a failure means mass equivocation through check_block_with grows pending proofs past the M2 cap.
#[test]
fn inc_i_255_m5_detector_pending_cap_holds() {
    let mut x = Probe::default();
    for _ in 0..200 {
        let p = pk();
        x.check(&block(SLOT, &p, 1, 0), true);
        x.check(&block(SLOT, &p, 2, 0), true);
    }
    let n = x.d.take_pending_proofs().len();
    assert!(
        (1..=PENDING_BOUND).contains(&n),
        "pending proofs {n} outside 1..={PENDING_BOUND}"
    );
}

// REQ-SLASH-004 — Decision: a failure means a VDF-valid oversize decoy kept as the stored header blocks the pair that fits (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_detector_keeps_smaller_header_so_oversize_does_not_block() {
    let p = pk();
    let o = block(SLOT, &p, 7, OVERSIZE_MISSED);
    let (a, b) = (block(SLOT, &p, 1, 0), block(SLOT, &p, 2, 0));
    for (first, second, order) in [(&o, &a, "O then A"), (&a, &o, "A then O")] {
        let mut x = Probe::default();
        assert!(x.check(first, true).is_none());
        assert!(
            x.check(second, true).is_none(),
            "{order}: a pair that cannot fit is no proof"
        );
        assert!(!x.d.has_pending_proofs(), "{order}: nothing pending");
        let proof = x
            .check(&b, true)
            .expect("the pair that fits must still form");
        assert_eq!(
            pair_of(&proof),
            want(&a, &b),
            "{order}: the proof is (A, B)"
        );
    }
}
