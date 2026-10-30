//! INC-I-289 L-1 (ruling 2): a candidate block's bits count toward its branch's
//! support only after the aggregate BLS check passes at the door. Below
//! `inc_i_178_attestation_bls_activation_height`, or on a failed check, they add 0.
//! Ruling 1: the node's OWN attestation counts only as a bit in a block on the branch.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (gossip door / wedge door
//   -> SupportView::support -> execute_reorg), then fn Node::run_periodic_tasks()
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: return values Ok(()) (handle_new_block / run_periodic_tasks, via helpers)
//   O3: the finality tracker holds the node's own attestation for D (precondition only)
//   PATHS: P1 forged bits (garbage aggregate) on a rival child, BLS AH = MAX;
//     P2 the same, BLS AH = 0 (root = presence_commitment, aggregate fails);
//     P3 valid aggregate of 5 of 7 on a rival child, BLS AH = 0;
//     P4 own gossip attestation for our tip; P5 own attestation as a verified bit
//   INPUT PARTITIONS: L1A X = 2 blocks {s0,s1} vs Y = {s2,s3} + forged bits 7 of 7
//     (both AH legs) -> X; L1B X = 2 blocks vs Y = {s2,s3} + valid bits 5 of 7 -> Y;
//     L1C-1 D {k1} + own gossip vs S {k2,k1} -> S; L1C-2 D {k1} + own verified bit
//     vs S {k2,k1} -> D (2 vs 2 tie keeps ours)
//   MATRIX: L1A O1,O2 x P1,P2 · L1B O1,O2 x P3 · L1C O1,O2,O3 x P4,P5
//   covers: branch_support doors support wedge_escape fork_recovery block_handling gossip_verdicts attestation verify keys

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::Block;

use super::inc_i_289_common::{self as c, leader, F};
use super::inc_i_289_k1_bond_weight::{
    assert_tip, attested_block, give, nth, plain_at, register_bls_all, rig7_with, settle, tip,
    universe, verified_block, R7,
};

const GARBAGE_AGG: [u8; 96] = [0xA5u8; 96];

/// X: 2 blocks on F by sorted[0], sorted[1] (slots 7, 8), applied. Y1: slot 23
/// (sorted[2]) on F. Returns (X, Y1).
async fn on_x(r: &mut R7) -> (Vec<Block>, Block) {
    let p = r.node.params.clone();
    let ah = r.node.inc_i_178_attestation_bls_activation_height;
    let x1 = plain_at(ah, F + 1, 7, r.f_hash, leader(&r.keys, 7), &p);
    let x2 = plain_at(ah, F + 2, 8, x1.hash(), leader(&r.keys, 8), &p);
    for b in [&x1, &x2] {
        r.node
            .apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: X applied");
    }
    assert_tip(&r.node, &x2, F + 2, "L1 precondition: on X2").await;
    let y1 = plain_at(ah, F + 1, 23, r.f_hash, leader(&r.keys, 23), &p);
    assert_eq!(
        leader(&r.keys, 7).public_key(),
        nth(&r.keys, 0).public_key()
    );
    assert_eq!(
        leader(&r.keys, 8).public_key(),
        nth(&r.keys, 1).public_key()
    );
    assert_eq!(
        leader(&r.keys, 23).public_key(),
        nth(&r.keys, 2).public_key()
    );
    assert_eq!(
        leader(&r.keys, 24).public_key(),
        nth(&r.keys, 3).public_key()
    );
    (vec![x1, x2], y1)
}

/// One L1A leg: BLS AH `ah178`; Y2 (slot 24, sorted[3]) on Y1 names all 7 with
/// a garbage aggregate and the root the AH expects. Returns (tip is X2, tip is Y2).
async fn forged_leg(ah178: u64) -> (bool, bool) {
    let mut r = rig7_with(ah178).await;
    let (x, y1) = on_x(&mut r).await;
    let p = r.node.params.clone();
    let uni = universe(&r.node, F + 2).await;
    assert_eq!(uni.len(), 7, "fixture: universe is the 7 producers");
    let all: Vec<&KeyPair> = r.keys.iter().collect();
    let by = leader(&r.keys, 24);
    let y2 = attested_block(
        F + 2,
        24,
        y1.hash(),
        by,
        &p,
        &uni,
        &all,
        &GARBAGE_AGG,
        ah178,
    );
    give(&mut r.node, &y1).await;
    give(&mut r.node, &y2).await;
    settle(&mut r.node, &y2.hash(), 5).await;
    let t = tip(&r.node).await;
    eprintln!(
        "[L1A ah178={ah178}] tip={t} x2={} y2={} y2_cached={} y2_stored={}",
        x[1].hash(),
        y2.hash(),
        r.node
            .fork_block_cache
            .read()
            .await
            .contains_key(&y2.hash()),
        r.node.block_store.has_block(&y2.hash()).unwrap()
    );
    if t == x[1].hash() {
        assert_tip(&r.node, &x[1], F + 2, "L1A: X kept").await;
    }
    (t == x[1].hash(), t == y2.hash())
}

// L1A INC-I-289 — Decision: a failure means a peer can forge fork-choice weight by setting every bit with an aggregate that does not verify, and pull the node off its branch (ruling 2).
#[tokio::test]
async fn l1a_inc_i_289_forged_bits_without_valid_aggregate_do_not_win() {
    let below = forged_leg(u64::MAX).await;
    let above = forged_leg(0).await;
    assert_eq!(
        (below, above),
        ((true, false), (true, false)),
        "L1A: forged bits (garbage aggregate) must add 0; (on X, on Y) for [BLS AH MAX, BLS AH 0]"
    );
}

// L1B INC-I-289 — Decision: a failure means the BLS door rejects or zeroes VALID bits, so a 5-of-7 attested branch can no longer beat a 2-producer branch (ruling 2 control).
#[tokio::test]
async fn l1b_inc_i_289_verified_bits_count_above_the_bls_gate() {
    let mut r = rig7_with(0).await;
    let (_x, y1) = on_x(&mut r).await;
    let p = r.node.params.clone();
    let uni = universe(&r.node, F + 2).await;
    let five: Vec<&KeyPair> = (2..7).map(|i| nth(&r.keys, i)).collect();
    let by = leader(&r.keys, 24);
    let y2 = verified_block(&r.keys, &r.bls, F + 2, 24, y1.hash(), by, &p, &uni, &five);
    give(&mut r.node, &y1).await;
    give(&mut r.node, &y2).await;
    let ticks = settle(&mut r.node, &y2.hash(), 5).await;
    eprintln!("[L1B] ticks: {ticks}");
    assert_tip(
        &r.node,
        &y2,
        F + 2,
        "L1B: Y (5 of 7, verified) must beat X (2 of 7)",
    )
    .await;
}

/// The first slot after `after` whose leader is `k`.
fn slot_of(keys: &[KeyPair], k: &KeyPair, after: u32) -> u32 {
    (after + 1..after + 10)
        .find(|s| leader(keys, *s).public_key() == k.public_key())
        .expect("fixture: a slot led by k")
}

/// A rig whose applied D (one block on F) is by a producer other than the node's own key.
async fn rig_on_non_own() -> (c::Rig, u32) {
    for s in 210..230u32 {
        let r = c::rig(&[s]).await;
        if r.ours[0].header.producer != *r.keys[0].public_key() {
            return (r, s);
        }
    }
    panic!("fixture: no non-own leader in 20 rigs");
}

// L1C INC-I-289 — Decision: a failure means the node's own attestation is counted as gossip evidence for its own tip (echo), or is NOT counted when a verified bit on the branch carries it (ruling 1).
#[tokio::test]
async fn l1c_inc_i_289_own_attestation_does_not_count_at_the_door() {
    // P4 (verified_bit=false): own gossip attestation for D does not count.
    for verified_bit in [false, true] {
        let (mut r, s_d) = rig_on_non_own().await;
        let bls = register_bls_all(&mut r.node, &r.keys).await;
        let own_k = r.keys[0].clone();
        let d = r.ours[0].clone();
        let k1 = r
            .keys
            .iter()
            .find(|k| k.public_key() == &d.header.producer)
            .unwrap()
            .clone();
        let k2 = r
            .keys
            .iter()
            .find(|k| k.public_key() != own_k.public_key() && k.public_key() != k1.public_key())
            .unwrap()
            .clone();
        let p = r.node.params.clone();
        let our_top = if verified_bit {
            // P5: D2 (by k1) carries the own key's bit for D with a valid aggregate.
            let uni = universe(&r.node, F + 2).await;
            let s = slot_of(&r.keys, &k1, s_d);
            let d2 = verified_block(&r.keys, &bls, F + 2, s, d.hash(), &k1, &p, &uni, &[&own_k]);
            give(&mut r.node, &d2).await;
            assert_tip(&r.node, &d2, F + 2, "L1C-2 precondition: on D2").await;
            d2
        } else {
            (r.node.sync_manager.write().await).add_attestation_weight(
                &d.hash(),
                *own_k.public_key(),
                1,
            );
            let held = (r.node.sync_manager.read().await).pending_attesters();
            assert!(
                held.iter()
                    .any(|(h, v)| *h == d.hash() && v.contains(own_k.public_key())),
                "L1C-1 precondition: the tracker holds the own attestation for D: {held:?}"
            );
            d.clone()
        };
        let s1_slot = slot_of(&r.keys, &k2, s_d + 3);
        let s2_slot = slot_of(&r.keys, &k1, s1_slot);
        let s1 = plain_at(0, F + 1, s1_slot, r.f_hash, &k2, &p);
        let s2 = plain_at(0, F + 2, s2_slot, s1.hash(), &k1, &p);
        give(&mut r.node, &s1).await;
        give(&mut r.node, &s2).await;
        if verified_bit {
            for _ in 0..3 {
                c::tick(&mut r).await;
            }
            let t: Hash = c::tip_hash(&r).await;
            eprintln!("[L1C-2] tip={t} d2={} s2={}", our_top.hash(), s2.hash());
            assert_tip(
                &r.node,
                &our_top,
                F + 2,
                "L1C-2: D {k1} + own VERIFIED bit (2) ties S {k2,k1} (2) -> D kept",
            )
            .await;
        } else {
            settle(&mut r.node, &s2.hash(), 5).await;
            assert_tip(
                &r.node,
                &s2,
                F + 2,
                "L1C-1: D {k1} + own gossip (0) = 1 < S {k2,k1} = 2 -> S",
            )
            .await;
        }
    }
}
