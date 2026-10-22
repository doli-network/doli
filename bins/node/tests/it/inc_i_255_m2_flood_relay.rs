//! INC-I-255 M2 — independent nodes agree on one slash per offence, and the
//! gossip relay verdict never forwards invalid or duplicate slash evidence.
//!
// OUTPUT CONTRACT: fn Node::handle_equivocation(&mut self, EquivocationProof)
//   O1 resident slash tx (bytes, id) per node
// OUTPUT CONTRACT: fn Node::handle_gossip_txs(&self, Vec<Transaction>) -> Verdict
//   O1 relay verdict; O2 mempool residency
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 slashes naming the accused the builder selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 producer-set slash record
// PATHS: three reporter nodes -> one builder node over gossip | relay verdict per variant
// MATRIX
//   handle_equivocation O1 x 3 nodes x {(a,b), (b,a)} -> byte-identical tx, one id; builder O1 -> 1; apply O1,O2 -> Ok, recorded
//     (inc_i_255_m2_determinism_one_id_one_mined)
//   two offences (slots) first seen by different nodes -> builder mempool holds 1, selects 1 (inc_i_255_m2_determinism_two_offences_*)
//   handle_gossip_txs O1,O2 x every invalid evidence variant -> Ignore, not resident; control valid -> Accept
//   handle_gossip_txs O1 x {second evidence for a resident accused, accused queued, resident tx re-gossiped} -> Ignore
// INPUT PARTITIONS:
//   arrival order: (a, b) | (b, a); offence: slot S | slot S + 2
//   evidence: bad signature | same header twice | different slots | wrong genesis | reporter-signed
//             | trailing byte | descending order | unknown producer | valid
//   accused state: Active | slash resident | Slash queued

use crypto::{Hash, KeyPair, Signature};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use network::{EquivocationProof, Verdict};
use storage::PendingProducerUpdate;

use super::inc_i_255_m1_common::*;
use super::inc_i_255_m2_flood_common::*;
use crate::block_sig::sign_header;

const NODES: usize = 3;

async fn reported_tx(w: &mut World, proof: EquivocationProof) -> Transaction {
    w.node.handle_equivocation(proof).await;
    let ids = resident_slashes(&w.node, &w.accused_pk()).await;
    assert_eq!(ids.len(), 1, "each reporter node holds exactly one slash");
    let pool = w.node.mempool.read().await;
    pool.get(&ids[0]).expect("resident entry").tx.clone()
}

// REQ-SLASH-004 — Decision: a failure means nodes seeing one equivocation build different tx ids (reporter, order), so N reports become N mineable slashes.
#[tokio::test]
async fn inc_i_255_m2_determinism_one_id_one_mined() {
    let accused = KeyPair::generate();
    let pk = *accused.public_key();
    let (a, b) = pair(&accused, chain_genesis());
    let expected = canonical(pk, a.clone(), b.clone()).hash();
    let mut txs = Vec::new();
    for i in 0..NODES {
        let mut w = world_with(&KeyPair::generate(), &accused).await;
        let (h1, h2) = if i % 2 == 0 {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        let proof = proof_of(&w, h1, h2);
        txs.push(reported_tx(&mut w, proof).await);
    }
    for (i, tx) in txs.iter().enumerate() {
        assert_eq!(
            tx.hash(),
            expected,
            "node #{i}: tx id differs from the canonical id"
        );
        assert_eq!(
            tx.serialize(),
            txs[0].serialize(),
            "node #{i}: tx bytes differ"
        );
    }

    let mut builder = world_with(&KeyPair::generate(), &accused).await;
    for tx in &txs {
        builder.node.handle_gossip_txs(vec![tx.clone()]).await;
    }
    assert_eq!(resident_slashes(&builder.node, &pk).await, vec![expected]);
    let (block, selected) = full_block_with(&mut builder.node, &[]).await;
    assert_eq!(slashes_naming(&block, &selected, &pk), 1, "one slash mined");
    let r = builder.node.apply_block(block, ValidationMode::Full).await;
    assert!(r.is_ok(), "the builder's block applies; got {:?}", r.err());
    assert!(
        slash_recorded(&builder.node, &pk).await,
        "the slash is recorded"
    );
}

// REQ-SLASH-004 — Decision: a failure means two nodes that first saw different offences by one producer get two slashes resident and mined for it.
#[tokio::test]
async fn inc_i_255_m2_determinism_two_offences_one_mined() {
    let accused = KeyPair::generate();
    let pk = *accused.public_key();
    let g = chain_genesis();
    let mut txs = Vec::new();
    for slot in [SLOT, SLOT + 2] {
        let mut w = world_with(&KeyPair::generate(), &accused).await;
        let h1 = evidence_header(&accused, g, slot, 0xA1, 0);
        let h2 = evidence_header(&accused, g, slot, 0xB2, 0);
        let mut proof = proof_of(&w, h1, h2);
        proof.slot = slot;
        txs.push(reported_tx(&mut w, proof).await);
    }
    assert_ne!(
        txs[0].hash(),
        txs[1].hash(),
        "FIXTURE: two offences, two ids"
    );

    let mut builder = world_with(&KeyPair::generate(), &accused).await;
    for tx in &txs {
        builder.node.handle_gossip_txs(vec![tx.clone()]).await;
    }
    assert_eq!(
        resident_slashes(&builder.node, &pk).await.len(),
        1,
        "the builder's mempool must hold one slash for the accused"
    );
    let (block, selected) = full_block_with(&mut builder.node, &[]).await;
    assert_eq!(
        slashes_naming(&block, &selected, &pk),
        1,
        "exactly one slash mined"
    );
}

fn invalid_variants(w: &World) -> Vec<(&'static str, Transaction)> {
    let g = chain_genesis();
    let pk = w.accused_pk();
    let (a, b) = pair(&w.accused, g);
    let (lo, hi) = by_hash(a.clone(), b.clone());
    let mut forged = b.clone();
    sign_header(&mut forged, &KeyPair::generate());
    let mut trailing = canonical(pk, a.clone(), b.clone());
    trailing.extra_data.push(0);
    let other_slot = evidence_header(&w.accused, g, SLOT + 1, 0xB2, 0);
    let foreign = Hash::from_bytes([0x5A; 32]);
    let stranger = KeyPair::generate();
    let (sa, sb) = pair(&stranger, g);
    vec![
        ("one bad signature", canonical(pk, a.clone(), forged)),
        (
            "same header twice",
            slash_tx(pk, a.clone(), a.clone(), Signature::default()),
        ),
        ("different slots", canonical(pk, a.clone(), other_slot)),
        (
            "wrong genesis",
            canonical(
                pk,
                evidence_header(&w.accused, foreign, SLOT, 0xA1, 0),
                evidence_header(&w.accused, foreign, SLOT, 0xB2, 0),
            ),
        ),
        (
            "reporter-signed",
            proof_of(w, a, b).to_slash_transaction(&w.reporter),
        ),
        ("trailing byte", trailing),
        (
            "descending order",
            slash_tx(pk, hi, lo, Signature::default()),
        ),
        (
            "unknown producer",
            canonical(*stranger.public_key(), sa, sb),
        ),
    ]
}

// REQ-SLASH-004 — Decision: a failure names an invalid evidence variant this node would forward to its peers (INV-NETWORK-003).
#[tokio::test]
async fn inc_i_255_m2_relay_invalid_evidence_never_forwarded() {
    let w = world().await;
    for (case, tx) in invalid_variants(&w) {
        let v = w.node.handle_gossip_txs(vec![tx.clone()]).await;
        assert!(
            matches!(v, Verdict::Ignore),
            "{case}: relay verdict must be Ignore"
        );
        assert!(
            !w.node.mempool.read().await.contains(&tx.hash()),
            "{case}: must not be resident"
        );
    }
    let v = w.node.handle_gossip_txs(vec![valid_slash(&w)]).await;
    assert!(
        matches!(v, Verdict::Accept),
        "NON-VACUITY: valid evidence is forwarded"
    );
}

// REQ-SLASH-004 — Decision: a failure means a second slash for an accused already resident is forwarded, so duplicates fan out across the network.
#[tokio::test]
async fn inc_i_255_m2_relay_duplicate_accused_not_forwarded() {
    let w = world().await;
    assert!(
        w.node.handle_new_transaction(valid_slash(&w)).await,
        "PRECONDITION: resident"
    );
    let v = w
        .node
        .handle_gossip_txs(vec![slash_of(&w.accused, 0xC3, 0xD4)])
        .await;
    assert!(
        matches!(v, Verdict::Ignore),
        "a second slash for a resident accused must not be forwarded"
    );
}

// REQ-SLASH-004 — Decision: a failure means a slash for an already-queued accused, or a re-gossiped resident slash, is forwarded again.
#[tokio::test]
async fn inc_i_255_m2_relay_queued_or_resident_not_forwarded() {
    let w = world().await;
    let tx = valid_slash(&w);
    assert!(
        w.node.handle_new_transaction(tx.clone()).await,
        "PRECONDITION: resident"
    );
    let again = w.node.handle_gossip_txs(vec![tx.clone()]).await;
    assert!(
        matches!(again, Verdict::Ignore),
        "a resident slash re-gossiped must not be forwarded"
    );

    let q = world().await;
    q.node
        .producer_set
        .write()
        .await
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: q.accused_pk(),
            height: 0,
        });
    let v = q.node.handle_gossip_txs(vec![valid_slash(&q)]).await;
    assert!(
        matches!(v, Verdict::Ignore),
        "a slash for a queued accused must not be forwarded"
    );
}
