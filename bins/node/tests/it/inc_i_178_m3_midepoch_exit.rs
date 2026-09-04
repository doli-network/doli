//! INC-I-178 M3 — REQ-BLS-014 (Must): the latent honest-block rejection, driven
//! through the REAL node-side validator.
//!
//! IGNORED because it pins a state the tree is expected to LEAVE: it asserts
//! today's REJECTION of an honestly built block. M4 un-ignores it and flips the
//! verdict to acceptance behind `inc_i_178_attestation_bls_activation_height`.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::validate_block_for_apply(&self, &Block, height, ValidationMode)
//!      -> Result<(), ValidationError>`
//!   O1 return value — Err carrying `attestation_bitfield has bits set beyond
//!      producer_count` (`validation_checks.rs:430-443`)
//!   O2 mutable params — NONE; the block is taken by shared reference. Asserted
//!      negatively: the bitfield is byte-identical after the call.
//!   O3 receiver/self — the validator holds `&self`; no node state may change.
//!   O4 persistent store writes / O5 statics / O6 channels — NONE on the reject path.
//!   Paths:
//!     P1 `epoch_state.producer_list` holds a producer INACTIVE at h (an exit),
//!        so the honest encoder width `base + extras` exceeds
//!        `active_producers_at_height(h).len()` AND an honest attester owns a bit
//!        in the excess region -> REJECT (today's behaviour)
//!   INPUT PARTITIONS: the exited producer sorts strictly BELOW the byte-largest
//!     active producer, so a genuine active attester — not the exited one — is the
//!     holder of the out-of-range bit. Every attester in this block is honest.
//!   MATRIX 6 outputs x 1 path: O1 asserted by error text; O2 and O3 asserted
//!     after the call; O4/O5/O6 are unreachable on a validation reject.

use crypto::{KeyPair, PublicKey};
use doli_core::validation::ValidationMode;

use crate::inc_i_178_m0_common::{
    active_at, assemble, build_via_production, err_text, make_node, record_attesters,
    safe_build_height, test_universe, unix_now, N_REALISTIC,
};

/// REQ-BLS-014 (Must) — Decision: a failure means the honest-block rejection this milestone documents is no longer reachable, so M4's activation-height gate would be guarding a defect that has already moved somewhere the gate does not cover.
#[tokio::test]
#[ignore = "INC-I-178 M4: green only at/after inc_i_178_attestation_bls_activation_height"]
async fn inc_i_178_m3_midepoch_exit_rejects_an_honest_block() {
    let (mut node, producers, _tmp) = make_node(N_REALISTIC).await;
    let height = safe_build_height(&node);
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();

    // P1 setup: a producer that is frozen into this epoch's producer_list but has
    // left the active set. It must sort BELOW the byte-largest active producer so
    // the out-of-range bit belongs to a genuine attester, not to the exited key.
    let largest = *all
        .iter()
        .max_by(|a, b| a.as_bytes().cmp(b.as_bytes()))
        .expect("45 producers");
    let exited = loop {
        let cand = *KeyPair::generate().public_key();
        if cand.as_bytes() < largest.as_bytes() {
            break cand;
        }
    };
    let pos = node
        .epoch_state
        .producer_list
        .partition_point(|p| p.as_bytes() < exited.as_bytes());
    node.epoch_state.producer_list.insert(pos, exited);

    let active = active_at(&node, height).await;
    assert_eq!(
        active.len(),
        N_REALISTIC,
        "the exited key is NOT active at h"
    );
    assert!(
        !active.iter().any(|p| p == &exited),
        "P1: producer_list \\ active_at(h) != {{}}"
    );

    // The honest encoder width, from the shipped `[base | extras]` shape.
    let universe = test_universe(&node.epoch_state.producer_list, &active);
    assert_eq!(universe.len(), N_REALISTIC + 1, "45 active + 1 exited");
    assert!(
        universe.len() > active.len(),
        "the precondition: honest width {} exceeds the validator denominator {}",
        universe.len(),
        active.len()
    );

    // Every attester below is a genuine active producer.
    let slot = node.params.timestamp_to_slot(unix_now());
    record_attesters(&mut node, slot, &all);
    let (header, txs, bf) = build_via_production(&mut node, height).await;

    assert_eq!(
        bf.len(),
        universe.len().div_ceil(8),
        "the builder sized the bitfield by the universe, not by active.len()"
    );
    let honest = doli_core::decode_attestation_bitfield_vec(&bf, universe.len());
    assert_eq!(
        honest.len(),
        N_REALISTIC,
        "all 45 honest attesters credited"
    );
    assert!(
        honest.iter().any(|i| *i >= active.len()),
        "an honest attester must own a bit at or beyond {}; got {honest:?}",
        active.len()
    );

    let block = assemble(header, txs, bf.clone());
    let verdict = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;

    // O1: today's behaviour — the honest block is rejected by name.
    assert!(
        err_text(&verdict).contains("attestation_bitfield has bits set beyond producer_count"),
        "P1: expected the stray-bit rejection, got {:?}",
        err_text(&verdict)
    );

    // O2: the validator did not mutate the block it was handed.
    assert_eq!(block.attestation_bitfield, bf, "O2: bitfield untouched");
    // O3: a rejected block leaves the epoch list it read unchanged.
    assert_eq!(
        node.epoch_state.producer_list.len(),
        N_REALISTIC + 1,
        "O3: no node state change on the reject path"
    );
}
