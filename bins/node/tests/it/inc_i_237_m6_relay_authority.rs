//! INC-I-237 M6 (AUDIT-P1-506 residual, Q1) — the attestation relay verdict
//! may not depend on residency in the 8-parent BLS pool: at most 2 Accepts per
//! (attester, slot), however many known parents an attacker cycles.
//!
//! OUTPUT CONTRACT: Node::on_new_attestation(&mut self, data, source_peer) -> Verdict
//!   O1: none (data by value)   O2: self.attest_seen (relay state),
//!   self.parent_sig_pool + self.minute_tracker (ingestion, must not change)
//!   O3: Verdict   O4: block_store read only   O5: none
//!   O6: sync_manager finality weight — DECLARED, NOT OBSERVABLE (private tracker)
//! PATHS: P1 first (attester, slot), valid BLS -> Accept; P2 seen slot, parent
//!   evicted from the pool, genuine BLS re-pools -> Accept at V (the residual);
//!   P3 seen slot, a different known block, genuine BLS newly pools -> Accept
//!   at V (keyed variant and Q1 fork switch)
//! INPUT PARTITIONS: parents {10 > MAX_PARENTS}; cycles {3 after the originals};
//!   attesters {2 keyless-captured, 1 keyed}; block {known, same slot fork pair};
//!   height {original, original + cycle}
//! MATRIX: O3 x P1-P3 (Accepts per (attester, slot) <= 2; Q1 second = Ignore),
//!   O2 x P3 (pool + attendance unchanged by the verdict). REQ-GRV-030
//!   (consensus code untouched) is verified by the runner diff check, not here.

use crypto::KeyPair;
use doli_core::attestation::attestation_minute;
use doli_core::ParentSignaturePool;
use network::service::Verdict;
use network::PeerId;

use super::inc_i_178_m0_common::{dual, make_node, safe_build_height, N_SMALL};
use super::inc_i_237_m6_common::{
    bls_member, is_known, set_wall, store_block, variant, wall_slot, AcceptLog, WALL,
};

const PARENTS: u32 = 10;
const CYCLES: u64 = 3;

// REQ-GRV-SEC-006 / REQ-GRV-SEC-002 — Decision: a failure means a keyless peer replaying height-mutated copies of captured genuine attestations over more known parents than the BLS pool holds still mints unbounded relay Accepts per (attester, slot).
#[tokio::test]
async fn m6_audit_p1_506_keyless_parent_cycle_accepts_at_most_two_per_attester_slot() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    set_wall(&mut node, WALL);
    let base = safe_build_height(&node);
    let attesters: [&KeyPair; 2] = [&producers[3], &producers[4]];
    let mut keys = Vec::new();
    for kp in attesters {
        keys.push(bls_member(&node, kp).await);
    }
    let mut blocks = Vec::new();
    let mut captured = Vec::new();
    for i in 0..PARENTS {
        let (slot, height) = (WALL - PARENTS + i, base + u64::from(i));
        let hash = store_block(&node, slot, height, i);
        blocks.push(hash);
        for (kp, bls) in attesters.iter().zip(&keys) {
            captured.push(dual(kp, bls, hash, slot, height));
        }
    }
    let mut log = AcceptLog::default();
    let mut originals = Vec::new();
    for g in &captured {
        originals.push(log.feed(&mut node, g).await);
    }
    let last = captured.last().expect("captured");
    let pooled = node
        .parent_sig_pool
        .get(&last.block_hash, &last.attester)
        .map(|s| s.to_vec());
    for cycle in 1..=CYCLES {
        for g in &captured {
            let copy = variant(g, g.height + cycle, g.bls_signature.clone());
            log.feed(&mut node, &copy).await;
        }
    }
    let worst = log.max();
    println!("M6_PROBE keyless_cycle_accepts_per_attester_slot={worst}");

    assert_eq!(wall_slot(&node), WALL, "non-vacuity: node clock pinned");
    assert!(PARENTS as usize > ParentSignaturePool::MAX_PARENTS);
    assert!(
        blocks.iter().all(|h| is_known(&node, h)),
        "non-vacuity: every cycled parent is a known block"
    );
    assert!(
        originals.iter().all(|v| *v == Verdict::Accept),
        "non-vacuity: the first copy of each (attester, slot) is Accepted: {originals:?}"
    );
    assert_eq!(
        pooled,
        Some(last.bls_signature.clone()),
        "non-vacuity: the captured BLS halves are genuine (they pool)"
    );
    assert!(
        worst <= 2,
        "AUDIT-P1-506: {worst} Accepts for one (attester, slot) after {CYCLES} cycles over {PARENTS} known parents (bound 2)"
    );
}

// REQ-GRV-SEC-006 — Decision: a failure means one member key signing one current slot over many distinct known old blocks earns one relay Accept per block instead of at most 2 per (attester, slot).
#[tokio::test]
async fn m6_audit_p1_506_keyed_old_blocks_same_slot_accept_at_most_two() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    set_wall(&mut node, WALL);
    let base = safe_build_height(&node);
    let kp = &producers[3];
    let bls = bls_member(&node, kp).await;
    let mut log = AcceptLog::default();
    let (mut known, mut verdicts, mut pooled) = (Vec::new(), Vec::new(), 0);
    for i in 0..PARENTS {
        let height = base + u64::from(i);
        let hash = store_block(&node, WALL - 40 + i, height, 0x100 + i);
        known.push(is_known(&node, &hash));
        let att = dual(kp, &bls, hash, WALL, height);
        verdicts.push(log.feed(&mut node, &att).await);
        if node.parent_sig_pool.get(&hash, kp.public_key()) == Some(&att_blob(&att)) {
            pooled += 1;
        }
    }
    let n = log.get(kp.public_key(), WALL);
    println!("M6_PROBE keyed_cycle_accepts_per_attester_slot={n}");

    assert!(
        known.iter().all(|k| *k),
        "non-vacuity: every signed old block is known: {known:?}"
    );
    assert_eq!(
        verdicts[0],
        Verdict::Accept,
        "non-vacuity: first (attester, slot)"
    );
    assert_eq!(
        pooled, PARENTS,
        "non-vacuity: every signed BLS half is valid (it pools)"
    );
    assert!(
        n <= 2,
        "AUDIT-P1-506: one member key earned {n} Accepts for (attester, slot {WALL}) over {PARENTS} known blocks (bound 2): {verdicts:?}"
    );
}

fn att_blob(a: &doli_core::Attestation) -> [u8; 96] {
    a.bls_signature
        .clone()
        .try_into()
        .expect("96-byte BLS half")
}

// REQ-GRV-SEC-006 (Q1, review F7) — Decision: a failure means a hop that already forwarded one valid BLS half for (attester, slot) forwards a second one for a competing block, or the relay change stopped pooling / attendance for it.
#[tokio::test]
async fn m6_q1_fork_switch_second_honest_attestation_same_slot_gets_no_second_bls_forward() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    set_wall(&mut node, WALL);
    let h = safe_build_height(&node);
    let kp = &producers[3];
    let bls = bls_member(&node, kp).await;
    let slot = WALL - 1;
    let (b1, b2) = (
        store_block(&node, slot, h, 1),
        store_block(&node, slot, h, 2),
    );
    let (first, second) = (dual(kp, &bls, b1, slot, h), dual(kp, &bls, b2, slot, h));
    let v1 = node
        .on_new_attestation(first.to_bytes(), PeerId::random())
        .await;
    let v2 = node
        .on_new_attestation(second.to_bytes(), PeerId::random())
        .await;

    assert!(
        b1 != b2 && is_known(&node, &b1) && is_known(&node, &b2),
        "non-vacuity: two distinct known blocks in one slot"
    );
    assert_eq!(
        v1,
        Verdict::Accept,
        "the first honest attestation is forwarded"
    );
    assert_eq!(
        node.parent_sig_pool.get(&b1, kp.public_key()),
        Some(&att_blob(&first)),
        "first BLS half pooled"
    );
    assert_eq!(
        node.parent_sig_pool.get(&b2, kp.public_key()),
        Some(&att_blob(&second)),
        "REQ-GRV-030: the second honest attestation still pools (relay verdict only)"
    );
    assert!(
        node.minute_tracker
            .attested_in_minute(attestation_minute(slot))
            .contains(&kp.public_key()),
        "REQ-GRV-030: attendance is recorded"
    );
    assert_eq!(
        v2,
        Verdict::Ignore,
        "Q1: a second honest attestation for a different block in the same slot gets no second BLS forward"
    );
}
