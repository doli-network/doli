//! INC-I-255 M2 — the equivocation handler submits at most one slash per
//! accused, ever: re-delivered or new evidence for a reported accused creates
//! no tx, below the gate nothing is created or marked, and the node's
//! detector does not accumulate re-delivered proofs.
//!
// OUTPUT CONTRACT: fn Node::handle_equivocation(&mut self, EquivocationProof)
//   O1 mempool SlashProducer entries naming the accused (count, id)
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()> (reaches the detector)
//   O1 mempool slash entries for the accused; O2 detector pending proofs
// PATHS: P-ON devnet gate 0 | P-OFF child, gate env-raised to 2, then crossed by one applied block
//        | P-EVICT the handler's own slash left the mempool after admission
// MATRIX
//   handle_equivocation O1 x P-EVICT x same proof -> [] (inc_i_255_m2_probe_handler)
//   handle_equivocation O1 x P-EVICT x another proof, same accused -> [] (inc_i_255_m2_handler_new_evidence_after_report_*)
//   handle_equivocation O1 x P-ON x {same proof x5, another proof} -> exactly [first id] (inc_i_255_m2_handler_repeat_*)
//   handle_equivocation O1 x P-ON x two accused -> one each (inc_i_255_m2_handler_marks_per_accused)
//   handle_equivocation O1 x P-OFF -> [] then, after the gate is crossed, [canonical id] (inc_i_255_m2_handler_gate_*)
//   handle_new_block O1,O2 x sibling re-delivered K times -> one slash, <= 1 pending proof (inc_i_255_m2_handler_redelivered_sibling_*)
// INPUT PARTITIONS:
//   evidence: pair A (salts A1/B2) | pair B (salts C3/D4), same slot | header order swapped
//   delivery: once | repeated | after the local slash left the mempool
//   gate: devnet default | env-raised then crossed

use crypto::KeyPair;
use doli_core::validation::ValidationMode;

use super::inc_i_236_m4_common::{feed, header_block};
use super::inc_i_255_m1_common::*;
use super::inc_i_255_m2_flood_common::*;

const MOD: &str = "inc_i_255_m2_flood_handler::";
const RAISED_GATE: u64 = 2;

fn second_proof(w: &World) -> (network::EquivocationProof, crypto::Hash) {
    let g = chain_genesis();
    let c = evidence_header(&w.accused, g, SLOT, 0xC3, 0);
    let d = evidence_header(&w.accused, g, SLOT, 0xD4, 0);
    let id = canonical(w.accused_pk(), c.clone(), d.clone()).hash();
    (proof_of(w, c, d), id)
}

fn first_proof(w: &World) -> (network::EquivocationProof, crypto::Hash) {
    let (a, b) = pair(&w.accused, chain_genesis());
    let id = canonical(w.accused_pk(), a.clone(), b.clone()).hash();
    (proof_of(w, a, b), id)
}

async fn report_then_evict(w: &mut World) -> network::EquivocationProof {
    let (proof, id) = first_proof(w);
    w.node.handle_equivocation(proof.clone()).await;
    assert_eq!(
        resident_slashes(&w.node, &w.accused_pk()).await,
        vec![id],
        "PRECONDITION: the first report lands the canonical slash"
    );
    w.node.mempool.write().await.remove_transaction(&id);
    proof
}

// REQ-SLASH-004 — Decision: a failure means a re-delivered equivocation re-submits a slash once the first left the mempool, the INC-I-237 re-batching flood.
#[tokio::test]
async fn inc_i_255_m2_probe_handler() {
    let mut w = world().await;
    let proof = report_then_evict(&mut w).await;
    w.node.handle_equivocation(proof).await;
    let resident = resident_slashes(&w.node, &w.accused_pk()).await;
    assert!(
        resident.is_empty(),
        "a reported accused must not get a second submitted slash; mempool holds {resident:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means new evidence for an already-reported accused creates a fresh tx id, so one offender yields many slash txs.
#[tokio::test]
async fn inc_i_255_m2_handler_new_evidence_after_report_creates_nothing() {
    let mut w = world().await;
    report_then_evict(&mut w).await;
    let (proof, id) = second_proof(&w);
    w.node.handle_equivocation(proof).await;
    assert!(
        resident_slashes(&w.node, &w.accused_pk()).await.is_empty(),
        "another proof for a reported accused must create no tx"
    );
    let tx = slash_of(&w.accused, 0xC3, 0xD4);
    assert_eq!(
        tx.hash(),
        id,
        "FIXTURE: the handler's tx for the second proof"
    );
    let r = admit(&w.node, &tx).await;
    assert!(
        r.is_ok(),
        "NON-VACUITY: admission alone accepts that evidence; got {r:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means repeated or alternative proofs for one resident accused add mempool entries (per-offence flood on one node).
#[tokio::test]
async fn inc_i_255_m2_handler_repeat_proofs_leave_one_tx() {
    let mut w = world().await;
    let (proof, id) = first_proof(&w);
    for _ in 0..5 {
        w.node.handle_equivocation(proof.clone()).await;
    }
    let (other, _) = second_proof(&w);
    w.node.handle_equivocation(other).await;
    let swapped = network::EquivocationProof {
        block_header_1: proof.block_header_2.clone(),
        block_header_2: proof.block_header_1.clone(),
        ..proof
    };
    w.node.handle_equivocation(swapped).await;
    assert_eq!(resident_slashes(&w.node, &w.accused_pk()).await, vec![id]);
    assert_eq!(
        resident_slash_total(&w.node).await,
        1,
        "exactly one slash tx in the mempool"
    );
}

// REQ-SLASH-004 — Decision: a failure means the reported-set is keyed too coarsely and a second offender is never slashed.
#[tokio::test]
async fn inc_i_255_m2_handler_marks_per_accused() {
    let mut f = flock(2).await;
    let mut want = Vec::new();
    for kp in f.accused.clone() {
        let (a, b) = pair(&kp, chain_genesis());
        want.push((
            *kp.public_key(),
            canonical(*kp.public_key(), a.clone(), b.clone()).hash(),
        ));
        let proof = network::EquivocationProof {
            producer: *kp.public_key(),
            block_header_1: a,
            block_header_2: b,
            slot: SLOT,
        };
        f.node.handle_equivocation(proof).await;
    }
    for (i, (pk, id)) in want.iter().enumerate() {
        assert_eq!(
            resident_slashes(&f.node, pk).await,
            vec![*id],
            "offender #{i} must get its own slash"
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means the handler creates a tx below the gate, or marks the accused below the gate so the slash is lost once the gate is crossed.
#[tokio::test]
async fn inc_i_255_m2_handler_gate_crossing_creates_after_refusal() {
    if !in_child() {
        run_in_child(
            &format!("{MOD}inc_i_255_m2_handler_gate_crossing_creates_after_refusal"),
            RAISED_GATE,
        );
        return;
    }
    let mut w = world().await;
    let (proof, id) = first_proof(&w);
    w.node.handle_equivocation(proof.clone()).await;
    assert!(
        resident_slashes(&w.node, &w.accused_pk()).await.is_empty(),
        "below the gate (next height 1 < {RAISED_GATE}) the handler must create no tx"
    );
    let (block, _) = full_block_with(&mut w.node, &[]).await;
    let r = w.node.apply_block(block, ValidationMode::Full).await;
    assert!(
        r.is_ok(),
        "FIXTURE: an empty block applies; got {:?}",
        r.err()
    );
    w.node.handle_equivocation(proof).await;
    assert_eq!(
        resident_slashes(&w.node, &w.accused_pk()).await,
        vec![id],
        "once the gate is crossed the same proof must create the slash"
    );
}

// REQ-SLASH-004 — Decision: a failure means a re-delivered sibling header grows the node's pending proofs or its slash count without bound.
#[tokio::test]
async fn inc_i_255_m2_handler_redelivered_sibling_bounded() {
    const REDELIVERIES: usize = 12;
    let mut w = world().await;
    let genesis = w.node.chain_state.read().await.best_hash;
    let pk = w.accused_pk();
    let accused: KeyPair = w.accused.clone();
    let slot = wall_slot(&w.node);
    let a = header_block(&mut w.node, 1, slot, genesis, &pk, Some(&accused), 0).await;
    let b = header_block(&mut w.node, 1, slot, genesis, &pk, Some(&accused), 1).await;
    assert_ne!(a.hash(), b.hash(), "FIXTURE: sibling headers must conflict");
    feed(&mut w.node, a).await;
    for _ in 0..REDELIVERIES {
        feed(&mut w.node, b.clone()).await;
    }
    assert_eq!(
        resident_slashes(&w.node, &pk).await.len(),
        1,
        "one slash for the accused after {REDELIVERIES} re-deliveries"
    );
    let pending = w
        .node
        .equivocation_detector
        .write()
        .await
        .take_pending_proofs()
        .len();
    assert!(
        pending <= 1,
        "the detector holds {pending} pending proofs after {REDELIVERIES} re-deliveries of one sibling"
    );
}
