//! INC-I-255 M2 — the equivocation detector's pending proofs are de-duplicated
//! per (producer, slot) and bounded, so re-delivered or mass equivocations
//! cannot grow node memory.
//!
// OUTPUT CONTRACT: fn EquivocationDetector::check_block(&mut self, &Block) -> Option<EquivocationProof>
//   O1 pending proofs (receiver), observed through take_pending_proofs()/has_pending_proofs()
//   O2 return: Some on the first conflicting header for a (producer, slot)
// PATHS: same sibling re-delivered | third conflicting header, same (producer, slot)
//        | one producer, two slots | many producers, one slot each
// MATRIX
//   O1 x sibling re-delivered 50x -> 1 pending (inc_i_255_m2_detector_redelivered_sibling_one_pending)
//   O1 x third conflicting header -> 1 pending (inc_i_255_m2_detector_third_header_same_offence_one_pending)
//   O1,O2 x two slots, one producer -> 2 pending, both Some (inc_i_255_m2_detector_distinct_slots_kept)
//   O1 x 200 producers equivocating -> 1..=64 pending (inc_i_255_m2_detector_pending_bounded)
// INPUT PARTITIONS:
//   delivery: once | repeated; conflict: 2 headers | 3 headers; offences: 1 | 2 slots | 200 producers

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, BlockHeader};
use network::EquivocationDetector;
use vdf::{VdfOutput, VdfProof};

const PENDING_BOUND: usize = 64;

fn block(slot: u32, producer: &PublicKey, salt: u8) -> Block {
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
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

// REQ-SLASH-004 — Decision: a failure means every re-delivery of one sibling header stores another proof (unbounded memory, repeated handler work).
#[test]
fn inc_i_255_m2_detector_redelivered_sibling_one_pending() {
    let pk = *KeyPair::generate().public_key();
    let mut d = EquivocationDetector::new();
    let (a, b) = (block(100, &pk, 1), block(100, &pk, 2));
    assert!(d.check_block(&a).is_none());
    assert!(
        d.check_block(&b).is_some(),
        "PRECONDITION: the first sibling is an equivocation"
    );
    for _ in 0..50 {
        d.check_block(&b);
    }
    assert_eq!(
        d.take_pending_proofs().len(),
        1,
        "one offence must hold one pending proof"
    );
}

// REQ-SLASH-004 — Decision: a failure means a third conflicting header for one (producer, slot) adds a second proof for the same offence.
#[test]
fn inc_i_255_m2_detector_third_header_same_offence_one_pending() {
    let pk = *KeyPair::generate().public_key();
    let mut d = EquivocationDetector::new();
    for salt in 1..=3 {
        d.check_block(&block(100, &pk, salt));
    }
    assert_eq!(
        d.take_pending_proofs().len(),
        1,
        "one (producer, slot) must hold one pending proof"
    );
}

// REQ-SLASH-004 — Decision: a failure means de-duplication is keyed per producer only and a second offence at another slot is lost.
#[test]
fn inc_i_255_m2_detector_distinct_slots_kept() {
    let pk = *KeyPair::generate().public_key();
    let mut d = EquivocationDetector::new();
    for slot in [100, 101] {
        d.check_block(&block(slot, &pk, 1));
        assert!(
            d.check_block(&block(slot, &pk, 2)).is_some(),
            "slot {slot}: detected"
        );
    }
    assert_eq!(
        d.take_pending_proofs().len(),
        2,
        "two slots are two offences"
    );
}

// REQ-SLASH-004 — Decision: a failure means mass equivocation grows pending proofs without bound (the INC-I-237 memory shape).
#[test]
fn inc_i_255_m2_detector_pending_bounded() {
    let mut d = EquivocationDetector::new();
    for _ in 0..200 {
        let pk = *KeyPair::generate().public_key();
        d.check_block(&block(100, &pk, 1));
        d.check_block(&block(100, &pk, 2));
    }
    assert!(d.has_pending_proofs(), "NON-VACUITY: proofs were recorded");
    let n = d.take_pending_proofs().len();
    assert!(
        n <= PENDING_BOUND,
        "pending proofs {n} exceed the bound {PENDING_BOUND}"
    );
}
