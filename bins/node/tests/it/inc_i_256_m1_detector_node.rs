//! INC-I-256 M1 / R-3 — the equivocation detector seen through
//! `Node::handle_new_block`: a huge-slot sync block cannot wipe its window, and
//! a header stored below the slashing gate cannot block the pair after it.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>
//   (the sync-response intake: no gossip SLOT_SANITY in front of it)
//   O1 self.equivocation_detector: stored headers (tracked_count; survival shown by a later pair)
//   O2 mempool: SlashProducer entries naming each accused (ids), via handle_equivocation
//   O3 return: ignored — block verdicts are not under test, only the detector
// PATHS:
//   P1 headers H_i stored -> sync block at slot u32::MAX -> conflicting H'_i
//   P2 child, slashing gate G above the tip: decoy D (signed, bad VDF) stored below G
//      -> chain crosses G -> A then B (valid, same producer and slot as D)
// INPUT PARTITIONS:
//   stored headers: 2 producers (all wiped, one wiped, none wiped are distinguishable),
//                   plus a control pair completed before the huge block (pipeline non-vacuity)
//   gate: devnet default (P1) | raised to G = 3 (P2)
//   stored header: valid (P1) | decoy failing the real admission rules (P2);
//                  P2 control: a second producer's clean pair after the crossing
// MATRIX: P1 x {O1 precondition 2 stored, metric wiped, O2 one slash per producer = canonical(H, H')}
//         P2 x {O1 precondition decoy stored, O2 = [canonical(A, B)], never (D, A) or (D, B)}
// covers: slash_submit slash_redeliver periodic mod block_handling network_events header_auth equivocation slash_admission pool transaction validation_checks init

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::validation::{evidence_header_admissible, verify_block_signature};
use doli_core::{Block, Network};
use doli_node::node::Node;

use super::inc_i_236_m4_common::{feed, header_block};
use super::inc_i_255_m1_common::{
    admit, canonical, chain_genesis, in_child, resident_slashes, run_in_child,
};
use super::inc_i_255_m2_common::{rig, Keys};
use super::inc_i_255_m2_flood_common::flock;
use crate::block_sig::sign_header;

const MOD: &str = "inc_i_256_m1_detector_node::";
const CROSS_GATE: u64 = 3;

fn orphan_parent() -> Hash {
    Hash::from_bytes([0x0D; 32])
}

/// Local wall-clock slot, as the gossip SLOT_SANITY computes it.
fn wall_slot(node: &Node) -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    node.params.timestamp_to_slot(now)
}

async fn tracked(node: &Node) -> usize {
    node.equivocation_detector.read().await.tracked_count()
}

/// Two signed, VDF-valid conflicting orphan blocks by `kp` at `slot`.
async fn sibling_pair(node: &mut Node, kp: &KeyPair, slot: u32) -> (Block, Block) {
    let pk = *kp.public_key();
    let a = header_block(node, 1, slot, orphan_parent(), &pk, Some(kp), 0).await;
    let b = header_block(node, 1, slot, orphan_parent(), &pk, Some(kp), 1).await;
    assert_ne!(a.hash(), b.hash(), "FIXTURE: the siblings conflict");
    (a, b)
}

fn slash_id(pk: PublicKey, x: &Block, y: &Block) -> Hash {
    canonical(pk, x.header.clone(), y.header.clone()).hash()
}

/// Signed by `kp`, same producer and slot, garbage VDF: fails the real admission rules.
async fn decoy(node: &mut Node, kp: &KeyPair, slot: u32) -> Block {
    let pk = *kp.public_key();
    let base = header_block(node, 1, slot, orphan_parent(), &pk, Some(kp), 7).await;
    let mut h = base.header.clone();
    h.vdf_output.value = vec![0xE0; h.vdf_output.value.len()];
    sign_header(&mut h, kp);
    let d = Block::new(h, base.transactions.clone());
    assert!(
        verify_block_signature(&d.header).is_ok(),
        "FIXTURE: the decoy carries a valid producer signature"
    );
    assert!(
        evidence_header_admissible(&d.header, &chain_genesis(), Network::Devnet).is_err(),
        "FIXTURE: the decoy fails the real admission rules"
    );
    d
}

// REQ-R-3 — Decision: a failure means one sync block with a huge slot erases every stored header, so a later double production is never slashed (P2-541).
#[tokio::test]
async fn inc_i_256_m1_detector_huge_slot_sync_block_keeps_window() {
    let mut f = flock(3).await;
    let slot = wall_slot(&f.node);
    let ctl = f.accused[2].clone();
    let (c1, c2) = sibling_pair(&mut f.node, &ctl, slot).await;
    feed(&mut f.node, c1.clone()).await;
    feed(&mut f.node, c2.clone()).await;
    let ctl_pk = *ctl.public_key();
    assert_eq!(
        resident_slashes(&f.node, &ctl_pk).await,
        vec![slash_id(ctl_pk, &c1, &c2)],
        "NON-VACUITY: these headers pair and the slash is admitted when nothing intervenes"
    );
    let mut seen = Vec::new();
    for kp in &f.accused[..2] {
        let (h, h2) = sibling_pair(&mut f.node, kp, slot).await;
        feed(&mut f.node, h.clone()).await;
        seen.push((*kp.public_key(), h, h2));
    }
    assert_eq!(
        tracked(&f.node).await,
        seen.len() + 1,
        "PRECONDITION: the detector stores one header per producer"
    );
    let stranger = KeyPair::generate();
    let huge = header_block(
        &mut f.node,
        1,
        u32::MAX,
        orphan_parent(),
        stranger.public_key(),
        Some(&stranger),
        0,
    )
    .await;
    feed(&mut f.node, huge).await;
    for (_, _, h2) in &seen {
        feed(&mut f.node, h2.clone()).await;
    }
    let mut kept = 0;
    let mut got = Vec::new();
    for (pk, h, h2) in &seen {
        let ids = resident_slashes(&f.node, pk).await;
        let want = slash_id(*pk, h, h2);
        kept += usize::from(ids == vec![want]);
        got.push((ids, want));
    }
    println!("B2M1_METRIC DETECTOR_ENTRIES_WIPED={}", seen.len() - kept);
    for (i, (ids, want)) in got.iter().enumerate() {
        assert_eq!(
            ids,
            &vec![*want],
            "producer {i}: a sync block at slot u32::MAX must not wipe the stored header; \
             the later conflicting header must yield exactly one slash"
        );
    }
}

async fn gate_crossing_body() {
    let mut r = rig(&Keys::new()).await;
    let gate = r
        .node
        .config
        .network
        .params()
        .effective_slashing_activation_height();
    assert_eq!(gate, CROSS_GATE, "FIXTURE: child slashing gate");
    let x = r.keys.accused.clone();
    let pk = *x.public_key();
    let slot = wall_slot(&r.node);
    let d = decoy(&mut r.node, &x, slot).await;
    let (a, b) = sibling_pair(&mut r.node, &x, slot).await;
    assert!(
        r.best().await.0 + 1 < gate,
        "FIXTURE: the decoy arrives below the gate"
    );
    feed(&mut r.node, d.clone()).await;
    assert_eq!(
        tracked(&r.node).await,
        1,
        "PRECONDITION: below the gate the detector stores the decoy"
    );
    r.advance_to(gate).await;
    let ctl = r.keys.delegator.clone();
    let ctl_pk = *ctl.public_key();
    let (c1, c2) = sibling_pair(&mut r.node, &ctl, slot).await;
    feed(&mut r.node, c1.clone()).await;
    feed(&mut r.node, c2.clone()).await;
    assert_eq!(
        resident_slashes(&r.node, &ctl_pk).await,
        vec![slash_id(ctl_pk, &c1, &c2)],
        "NON-VACUITY: above the gate a clean pair is slashed on this rig"
    );
    feed(&mut r.node, a.clone()).await;
    feed(&mut r.node, b.clone()).await;
    let got = resident_slashes(&r.node, &pk).await;
    for (what, id) in [
        ("(D, A)", slash_id(pk, &d, &a)),
        ("(D, B)", slash_id(pk, &d, &b)),
    ] {
        assert!(!got.contains(&id), "the decoy pair {what} was admitted");
    }
    if got.is_empty() {
        let direct = admit(&r.node, &canonical(pk, a.header.clone(), b.header.clone())).await;
        panic!(
            "after the gate: no slash for (A, B); direct admission of the same tx gives {direct:?}, \
             so the decoy stored below the gate blocked the pair"
        );
    }
    assert_eq!(
        got,
        vec![slash_id(pk, &a, &b)],
        "after the gate: exactly one admitted slash naming (A, B); the decoy stored below the gate must not block it"
    );
}

// REQ-R-3 — Decision: a failure means a header stored while the detector admitted everything still blocks or poisons the valid pair after the gate (P2-542).
#[tokio::test]
async fn inc_i_256_m1_detector_decoy_below_gate_dropped_at_crossing() {
    if in_child() {
        gate_crossing_body().await;
        return;
    }
    run_in_child(
        &format!("{MOD}inc_i_256_m1_detector_decoy_below_gate_dropped_at_crossing"),
        CROSS_GATE,
    );
}
