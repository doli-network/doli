//! INC-I-237 M6 (audit F7, INV-ATTEST-001) — the genuine BLS half is forwarded
//! even when a junk-BLS copy spent (attester, slot) first on the unknown-block
//! path; a mismatched-key producer's first copy is still forwarded once and
//! still attends.
//!
//! OUTPUT CONTRACT: Node::on_new_attestation(&mut self, data, source_peer) -> Verdict
//!   O1: none (data by value)   O2: self.attest_seen (relay state),
//!   self.parent_sig_pool + self.minute_tracker (ingestion)   O3: Verdict
//!   O4: block_store read only   O5: none   O6: finality weight — DECLARED,
//!   NOT OBSERVABLE (private tracker)
//! PATHS: P1 unknown block, first copy (junk BLS) -> Accept; P2 unknown block,
//!   genuine BLS after junk -> Ignore at V (audit F7); P3 unknown block, third
//!   copy with the genuine BLS -> Ignore (bound 2); P4 known block, first copy,
//!   BLS invalid for the on-chain key -> Accept + attendance, no pool entry;
//!   P5 known block, second copy of the invalid half -> Ignore
//! INPUT PARTITIONS: bls {junk 96 B, genuine, valid-but-wrong-key}; block
//!   {unknown, known}; height {h, h + 1}
//! MATRIX: O3 x P1-P5, O2 x P4 (attendance, pool) — asserted below.

use crypto::{bls_verify, BlsKeyPair, BlsSignature, Hash};
use doli_core::attestation::{attestation_minute, bls_attest_msg};
use network::service::Verdict;

use super::inc_i_178_m0_common::{dual, make_node, safe_build_height, N_SMALL};
use super::inc_i_237_m6_common::{
    bls_member, is_known, set_wall, store_block, variant, AcceptLog, WALL,
};

// REQ-GRV-023 — Decision: a failure means a peer that delivers a junk-BLS copy first stops this node from ever forwarding the genuine BLS half of an attestation for a block it does not have yet.
#[tokio::test]
async fn m6_f7_unknown_block_genuine_copy_after_junk_bls_copy_is_accepted() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    set_wall(&mut node, WALL);
    let kp = &producers[3];
    let bls = bls_member(&node, kp).await;
    let unknown = Hash::from_bytes([0xF7; 32]);
    let genuine = dual(kp, &bls, unknown, WALL, 7);
    let junk = variant(&genuine, 7, vec![0xA5; 96]);
    let third = variant(&genuine, 8, genuine.bls_signature.clone());
    let mut log = AcceptLog::default();
    let vj = log.feed(&mut node, &junk).await;
    let vg = log.feed(&mut node, &genuine).await;
    let vt = log.feed(&mut node, &third).await;
    println!(
        "M6_PROBE genuine_after_junk_accepted={}",
        u8::from(vg == Verdict::Accept)
    );

    let sig = BlsSignature::try_from_slice(&genuine.bls_signature).expect("BLS bytes");
    assert!(
        bls_verify(&bls_attest_msg(&unknown), &sig, bls.public_key()).is_ok(),
        "non-vacuity: the genuine BLS half verifies against the on-chain key"
    );
    assert!(
        junk.verify().is_ok() && genuine.verify().is_ok(),
        "non-vacuity: both copies carry a valid Ed25519 signature"
    );
    assert!(
        !is_known(&node, &unknown),
        "non-vacuity: unknown-block path"
    );
    assert_eq!(
        vj,
        Verdict::Accept,
        "non-vacuity: the junk-BLS copy arrived first"
    );
    assert_eq!(
        vg,
        Verdict::Accept,
        "REQ-GRV-023: the genuine BLS half after a junk-BLS copy must be forwarded"
    );
    assert_eq!(
        vt,
        Verdict::Ignore,
        "bound 2: a third copy of (attester, slot) on the unknown-block path"
    );
    assert!(log.max() <= 2);
}

// INV-ATTEST-001 — Decision: a failure means the M6 relay rule stops forwarding (or attending) a member whose BLS key does not match its on-chain key, the INC-I-217 mismatched-key producers.
#[tokio::test]
async fn m6_inv_attest_001_invalid_bls_first_copy_accepted_once_and_attends() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    set_wall(&mut node, WALL);
    let h = safe_build_height(&node);
    let kp = &producers[3];
    let _onchain = bls_member(&node, kp).await;
    let wrong = BlsKeyPair::generate();
    let slot = WALL - 1;
    let hash = store_block(&node, slot, h, 0xE1);
    let first = dual(kp, &wrong, hash, slot, h);
    let copy = variant(&first, h + 1, first.bls_signature.clone());
    let mut log = AcceptLog::default();
    let v1 = log.feed(&mut node, &first).await;
    let v2 = log.feed(&mut node, &copy).await;

    assert!(is_known(&node, &hash), "non-vacuity: known-block path");
    assert_eq!(
        node.parent_sig_pool.get(&hash, kp.public_key()),
        None,
        "non-vacuity: the BLS half is invalid for the on-chain key (never pools)"
    );
    assert!(
        node.minute_tracker
            .attested_in_minute(attestation_minute(slot))
            .contains(&kp.public_key()),
        "INV-ATTEST-001: attendance is recorded whatever the BLS verdict"
    );
    assert_eq!(
        [v1, v2],
        [Verdict::Accept, Verdict::Ignore],
        "INV-ATTEST-001: the first copy is forwarded once per (attester, slot)"
    );
}
