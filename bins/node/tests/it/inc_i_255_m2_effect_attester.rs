//! INC-I-255 M2 — a Slashed member keeps its record but gives no attester
//! weight: its attestation earns no attendance and no pooled BLS half.
//!
// OUTPUT CONTRACT: fn Node::on_new_attestation(&mut self, Vec<u8>, PeerId) -> Verdict
//   O1 self.minute_tracker attendance for (attester, minute)
//   O2 self.parent_sig_pool entry (block hash, attester)
//   O3 return — relay verdict, not asserted (gossip relay rules are outside this milestone)
// PATHS: P-SLASHED attester status Slashed (record kept) | P-ACTIVE control member
// MATRIX
//   O1,O2 x P-SLASHED -> no attendance, nothing pooled (inc_i_255_m2_probe_attester)
//   O1,O2 x P-ACTIVE  -> attendance and pooled half (same test, non-vacuity)
// INPUT PARTITIONS:
//   attester: Slashed with an on-chain BLS key and a valid dual signature | Active control with the same shape

use crypto::KeyPair;
use doli_core::attestation::attestation_minute;
use network::PeerId;

use super::inc_i_178_m0_common::{
    assemble, build_via_production, dual, make_node, safe_build_height, N_SMALL,
};
use super::inc_i_237_m6_common::bls_member;

async fn slashed_member_gives_no_weight() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let block = assemble(header, txs, bf);
    let (hash, slot) = (block.hash(), block.header.slot);
    node.block_store
        .put_block_canonical(&block, height)
        .expect("put_block_canonical");
    let minute = attestation_minute(slot);

    let (slashed, control): (&KeyPair, &KeyPair) = (&producers[3], &producers[7]);
    let bls_s = bls_member(&node, slashed).await;
    let bls_c = bls_member(&node, control).await;
    node.producer_set
        .write()
        .await
        .slash_producer(slashed.public_key(), height - 1)
        .expect("PRECONDITION: slash the member");
    assert!(
        node.producer_set
            .read()
            .await
            .get_by_pubkey(slashed.public_key())
            .is_some(),
        "PRECONDITION: the Slashed record is kept"
    );

    for (kp, bls) in [(slashed, &bls_s), (control, &bls_c)] {
        node.on_new_attestation(
            dual(kp, bls, hash, slot, height).to_bytes(),
            PeerId::random(),
        )
        .await;
    }

    let attended = node.minute_tracker.attested_in_minute(minute);
    assert!(
        attended.contains(&control.public_key()),
        "NON-VACUITY: an Active member attends"
    );
    assert!(
        node.parent_sig_pool
            .get(&hash, control.public_key())
            .is_some(),
        "NON-VACUITY: an Active member's BLS half is pooled"
    );
    assert!(
        !attended.contains(&slashed.public_key()),
        "B11: a Slashed member must earn no attendance"
    );
    assert!(
        node.parent_sig_pool
            .get(&hash, slashed.public_key())
            .is_none(),
        "B11: a Slashed member's BLS half must not enter the aggregate pool"
    );
}

// REQ-SLASH-003 — Decision: a failure means a slashed producer still feeds attendance and the block aggregate, so its exclusion is incomplete (INC-I-244).
#[tokio::test]
async fn inc_i_255_m2_probe_attester() {
    slashed_member_gives_no_weight().await;
}
