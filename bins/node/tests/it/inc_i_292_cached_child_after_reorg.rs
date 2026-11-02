//! INC-I-292: a reorg that lands on a parent applies that parent's cached child.
//! The node sits on its own branch D on F. A competing branch C_B, C_{B+1},
//! C_{B+2} (B = F + 1) arrives out of order: C_B, then C_{B+2} (an orphan, cached),
//! then C_{B+1}, whose door reorgs onto C_{B+1}. C_{B+2} is a valid child of the
//! new tip with its body in the fork cache, so the node must end on C_{B+2}.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (Orphan arm with a retained
//   parent -> wedge door -> execute_reorg -> run_reorg), then fn Node::run_periodic_tasks()
//   O1: chain_state best_hash / best_height, canonical entries at B, B+1, B+2,
//     persisted tip, sync tip
//   O2: fork_block_cache no longer holds C_{B+2}; block_store holds C_{B+2}
//   O3: return values Ok(()) (handle_new_block / run_periodic_tasks, via helpers)
//   PATHS: P1 at/above the INC-I-288 AH: D = 2 blocks by 2 producers, C by the
//     node's key, C_B attested over gossip by both other producers, so C_B+C_{B+1}
//     out-supports D at equal length; P2 below the AH (child re-exec): D = 1 block,
//     C_B+C_{B+1} out-counts it
//   INPUT PARTITIONS: N1A above AH, arrival C_B, C_{B+2}, C_{B+1} -> C_{B+2};
//     N1B below AH, same arrival order, D one block -> C_{B+2}
//   MATRIX: N1A O1-O3 · N1B O1-O3
//   covers: block_handling wedge_escape fork_cache body_retry

use crypto::KeyPair;
use doli_core::{Attestation, Block};
use network::PeerId;

use super::inc_i_289_common::{self as c, block, leader, Rig, F};

const B: u64 = F + 1;

struct Shape {
    r: Rig,
    /// C_B, C_{B+1}, C_{B+2}.
    c: [Block; 3],
}

/// The first slot after `after` whose leader is the node's own key (keys[0]).
fn own_slot_after(r: &Rig, after: u32) -> u32 {
    let own = r.keys[0].public_key();
    (after + 1..=after + 3)
        .find(|s| leader(&r.keys, *s).public_key() == own)
        .expect("fixture: an own-key slot within 3 slots")
}

/// Gossip attestations of `b` by every non-own producer; they back `b` at the
/// doors because the node already holds `b` as a fork block.
async fn attest_by_others(r: &mut Rig, b: &Block, height: u64) {
    let others: Vec<KeyPair> = r.keys[1..].to_vec();
    for k in &others {
        let a = Attestation::new(
            b.hash(),
            b.header.slot,
            height,
            1,
            k.private_key(),
            *k.public_key(),
        );
        r.node
            .on_new_attestation(a.to_bytes(), PeerId::random())
            .await;
    }
}

/// D = `d_slots` applied on F. C_B, C_{B+1}, C_{B+2} on F, all by the node's key.
/// C_B arrives (retained fork block), both other producers attest it, then C_{B+2}
/// arrives before its parent (an orphan, cached). C_{B+1} is NOT delivered yet.
async fn arrange(d_slots: &[u32]) -> Shape {
    let mut r = c::rig(d_slots).await;
    let d_top = r.ours.last().expect("fixture: D").clone();
    let d_height = F + r.ours.len() as u64;
    let p = r.node.params.clone();
    let s0 = own_slot_after(&r, d_top.header.slot);
    let own = r.keys[0].clone();
    let c_b = block(B, s0, r.f_hash, &own, &p);
    let c_b1 = block(B + 1, s0 + 3, c_b.hash(), &own, &p);
    let c_b2 = block(B + 2, s0 + 6, c_b1.hash(), &own, &p);
    for (i, b) in [&c_b, &c_b1, &c_b2].iter().enumerate() {
        assert_eq!(
            leader(&r.keys, b.header.slot).public_key(),
            own.public_key(),
            "fixture: C[{i}] is by its slot leader"
        );
    }

    c::gossip(&mut r, &c_b).await;
    assert_eq!(
        c::weighed(&r, &c_b).await,
        (true, true, true),
        "precondition: C_B recorded, cached and retained"
    );
    attest_by_others(&mut r, &c_b, B).await;
    c::assert_on(&r, &d_top, d_height, "precondition: still on D after C_B").await;

    c::gossip(&mut r, &c_b2).await;
    assert!(
        c::cached(&r, &c_b2.hash()).await,
        "precondition: C_B+2 cached as an orphan"
    );
    assert!(!c::stored(&r, &c_b2), "precondition: C_B+2 not applied yet");
    c::assert_on(&r, &d_top, d_height, "precondition: still on D after C_B+2").await;

    Shape {
        r,
        c: [c_b, c_b1, c_b2],
    }
}

/// Deliver C_{B+1} (its door reorgs onto C_{B+1}), tick, then require C_{B+2}.
async fn assert_child_applied(s: &mut Shape, label: &str) {
    let [c_b, c_b1, c_b2] = s.c.clone();
    c::gossip(&mut s.r, &c_b1).await;
    c::tick_until(&mut s.r, &c_b2.hash(), 5).await;

    // Shape check: the reorg took the C branch at least through C_{B+1}.
    let canon = |h: u64| s.r.node.block_store.get_hash_by_height(h).unwrap();
    assert_eq!(
        canon(B),
        Some(c_b.hash()),
        "{label} shape: C_B canonical at B"
    );
    assert_eq!(
        canon(B + 1),
        Some(c_b1.hash()),
        "{label} shape: C_B+1 canonical at B+1 (the reorg landed on the parent)"
    );

    let (tip, height) = {
        let cs = s.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    let stuck = if tip == c_b1.hash() {
        " (stuck on the parent C_B+1)"
    } else {
        ""
    };
    assert_eq!(
        (tip, height),
        (c_b2.hash(), B + 2),
        "{label}: INC-I-292 cached child C_B+2 not applied; tip at h={height}{stuck}"
    );
    c::assert_on(&s.r, &c_b2, B + 2, label).await;
    assert!(c::stored(&s.r, &c_b2), "{label}: C_B+2 in the block store");
    assert!(
        !c::cached(&s.r, &c_b2.hash()).await,
        "{label}: C_B+2 left in the fork cache after it applied"
    );
}

// N1A INC-I-292 — Decision: a failure means a reorg that lands on a parent leaves that parent's cached child unapplied, so the node stops one block short of the branch it chose and waits for a fetch that never comes.
#[tokio::test]
async fn n1a_reorg_landing_on_parent_applies_cached_child_above_gate() {
    // D = 2 blocks by 2 distinct producers (support 2). C alone = the node's key
    // (support 1, C_B is retained, not heavier); with both others' attestations on
    // C_B, C_B+C_{B+1} has support 3 > 2 at equal length: C_{B+1}'s door lands there.
    let mut s = arrange(&[F as u32 + 1, F as u32 + 2]).await;
    assert_child_applied(&mut s, "N1A above AH").await;
}

// N1B INC-I-292 — Decision: a failure means the gap is not limited to the bond-support rule: a count-rule reorg below the AH that lands on a parent also strands its cached child, so the fix cannot be gated.
#[tokio::test]
async fn n1b_reorg_landing_on_parent_applies_cached_child_below_gate() {
    if !c::in_below_ah_child(
        "inc_i_292_cached_child_after_reorg::n1b_reorg_landing_on_parent_applies_cached_child_below_gate",
    ) {
        return;
    }
    // Below the AH one block weighs 1, so N1A's D (2 blocks) ties C_B+C_{B+1} and
    // the strict door never lands on C_{B+1}. With D = 1 block, C_B ties D (retained)
    // and C_B+C_{B+1} out-counts it: C_{B+1}'s door lands on the parent of C_{B+2}.
    let mut s = arrange(&[F as u32 + 1]).await;
    assert_child_applied(&mut s, "N1B below AH").await;
}
