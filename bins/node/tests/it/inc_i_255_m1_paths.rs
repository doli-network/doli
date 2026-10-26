//! INC-I-255 M1 — authenticated double-production evidence becomes one
//! canonical, fee-free slash that the handler creates and that admission,
//! relay, the builder and apply (every mode) all accept.
//!
// covers: network_params defaults env_loader ordering lib slash_evidence slash_canonical slash_state slash_fee utxo transaction block types data core pool slash_admission equivocation validation_checks assembly tx_processing rewards slash_submit init block_handling event_loop
//
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()> (reaches handle_equivocation)
//   O1 mempool SlashProducer entries naming the accused (count and id)
// OUTPUT CONTRACT: fn Node::handle_new_transaction(&self, Transaction) -> bool
//   O1 return; O2 mempool residency
// OUTPUT CONTRACT: fn Node::handle_gossip_txs(&self, Vec<Transaction>) -> Verdict
//   O1 relay verdict
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions the builder selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 chain tip; O3 block_store; O4 producer-set slash record; O5 state root
// PATHS: P-ON devnet slash gate (0) | P-OFF child process with the gate env-raised far above the tip
//        | P-HDL handle_equivocation at P-ON with the handler's slash withheld (resident or refused)
// MATRIX
//   handle_new_block O1 x P-ON  -> exactly [canonical id]   (inc_i_255_probe_handler_builds_tx)
//   handle_new_block O1 x P-OFF -> []                       (same test, child)
//   handle_new_transaction O1,O2 x canonical -> true, resident (inc_i_255_probe_mempool_admits)
//   build_block_content O1 x resident canonical -> selected   (inc_i_255_probe_builder_includes)
//   apply_block O1-O4 x Full -> Ok, tip = block, stored, recorded (inc_i_255_probe_apply_full_accepts)
//   apply_block O1,O2,O4,O5 x {Full, Light, Replay} -> Ok, identical tip and root (inc_i_255_b2_*)
//   handle_new_transaction x {reporter A, reporter B, swapped, trailing, canonical} -> resident = [canonical] (inc_i_255_b8_*)
//   handle_gossip_txs x {reporter-signed -> Ignore, canonical -> Accept} (inc_i_255_b3_*)
//   handle_equivocation O1 x P-HDL {slash for the accused resident -> count unchanged (inc_i_255_handler_skips_*),
//     admission refuses (unregistered accused) -> nothing resident (inc_i_255_handler_refused_*)}
// INPUT PARTITIONS:
//   evidence: canonical | reporter-signed (two reporter keys) | swapped header order | trailing byte
//   gate: devnet default | env-raised in a re-exec'd child
//   mode: Full | Light | Replay

use std::collections::HashSet;

use crypto::{Hash, KeyPair, Signature};
use doli_core::validation::ValidationMode;
use network::Verdict;

use super::inc_i_255_m1_common::*;

const BELOW: u64 = 1_000_000_000;
const MOD: &str = "inc_i_255_m1_paths::";

// REQ-SLASH-001 — Decision: a failure means detected double production still creates no slash (B1), or the handler creates one off-gate or with a non-canonical id.
#[tokio::test]
async fn inc_i_255_probe_handler_builds_tx() {
    if in_child() {
        let (resident, _) = handler_slashes(&mut world().await).await;
        assert!(
            resident.is_empty(),
            "below the slash gate the handler must stay log-only; mempool holds {resident:?}"
        );
        return;
    }
    run_in_child(&format!("{MOD}inc_i_255_probe_handler_builds_tx"), BELOW);

    let mut w = world().await;
    let (resident, expected) = handler_slashes(&mut w).await;
    assert_eq!(
        resident,
        vec![expected],
        "B1: at/above the gate one equivocation must leave exactly the canonical slash in the local mempool"
    );
}

// REQ-SLASH-001 — Decision: a failure means the normal lane still refuses authenticated fee-0 evidence (B3), so no node admits or relays a slash.
#[tokio::test]
async fn inc_i_255_probe_mempool_admits() {
    let w = world().await;
    let tx = valid_slash(&w);
    let admitted = w.node.handle_new_transaction(tx.clone()).await;
    let why = if admitted {
        None
    } else {
        Some(admit(&w.node, &tx).await)
    };
    assert!(
        admitted,
        "B3: normal-lane admission must accept a valid fee-0 slash; verdict {why:?}"
    );
    assert!(
        w.node.mempool.read().await.contains(&tx.hash()),
        "B3: the admitted slash must be resident"
    );
}

// REQ-SLASH-001 — Decision: a failure means the builder skips a resident valid slash (B2 fee floor), so it is never mined.
#[tokio::test]
async fn inc_i_255_probe_builder_includes() {
    let mut w = world().await;
    let tx = valid_slash(&w);
    assert!(
        inject(&w.node, &tx).await,
        "PRECONDITION: the slash must be resident before the builder runs"
    );
    let (_, selected) = full_block_with(&mut w.node, &[]).await;
    assert!(
        selected.contains(&tx.hash()),
        "B2: the builder must select the resident valid slash"
    );
}

// REQ-SLASH-001 — Decision: a failure means Full apply rejects a block that carries valid evidence (B2), so a producer forks off its own slash block.
#[tokio::test]
async fn inc_i_255_probe_apply_full_accepts() {
    let mut w = world().await;
    let tx = valid_slash(&w);
    let (block, _) = full_block_with(&mut w.node, std::slice::from_ref(&tx)).await;
    let pre = w
        .node
        .validate_block_for_apply(&block, 1, ValidationMode::Full)
        .await;
    assert!(
        pre.is_ok(),
        "PRECONDITION: block-level validation accepts the slash block; got {pre:?}"
    );

    let r = w
        .node
        .apply_block(block.clone(), ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "B2: Full apply must accept a block with a valid slash; got {:?}",
        r.as_ref().err().map(|e| format!("{e:#}"))
    );
    assert_eq!(
        tip(&w.node).await,
        (1, block.hash()),
        "tip must be the slash block"
    );
    let stored = w
        .node
        .block_store
        .get_block_by_height(1)
        .expect("block_store read")
        .expect("applied block must be stored");
    assert!(
        stored.transactions.iter().any(|t| t.hash() == tx.hash()),
        "stored block must carry the slash"
    );
    assert!(
        slash_recorded(&w.node, &w.accused_pk()).await,
        "the applied slash must be recorded against the accused"
    );
}

// REQ-SLASH-002 — Decision: a failure means Light or Replay reaches another verdict or state than Full on one slash block (B12), a sync-vs-live split.
#[tokio::test]
async fn inc_i_255_b2_apply_light_and_replay_match_full() {
    let (reporter, accused) = (KeyPair::generate(), KeyPair::generate());
    let mut full = world_with(&reporter, &accused).await;
    let tx = valid_slash(&full);
    let (block, _) = full_block_with(&mut full.node, std::slice::from_ref(&tx)).await;
    let mut light = world_with(&reporter, &accused).await;
    let mut replay = world_with(&reporter, &accused).await;
    assert_eq!(
        tip_root(&full.node).await,
        tip_root(&light.node).await,
        "PRECONDITION: identical genesis state"
    );

    let mut results = Vec::new();
    for (mode, w) in [
        (ValidationMode::Full, &mut full),
        (ValidationMode::Light, &mut light),
        (ValidationMode::Replay, &mut replay),
    ] {
        let r = w
            .node
            .apply_block(block.clone(), mode)
            .await
            .map_err(|e| format!("{e:#}"));
        let state = tip_root(&w.node).await;
        let recorded = slash_recorded(&w.node, accused.public_key()).await;
        results.push((mode, r, state, recorded));
    }
    for (mode, r, (h, hash, _), recorded) in &results {
        assert!(r.is_ok(), "B2/B12: {mode:?} apply must accept; got {r:?}");
        assert_eq!((*h, *hash), (1, block.hash()), "{mode:?}: tip");
        assert!(*recorded, "{mode:?}: slash must be recorded");
    }
    assert_eq!(
        results[1].2, results[0].2,
        "B12: Light state differs from Full"
    );
    assert_eq!(
        results[2].2, results[0].2,
        "B12: Replay state differs from Full"
    );
}

// REQ-SLASH-002 — Decision: a failure means one evidence pair still yields several admissible tx ids (reporter, order, padding), the INC-I-237 flood shape (B8).
#[tokio::test]
async fn inc_i_255_b8_one_canonical_id_per_evidence_pair() {
    let w = world().await;
    let pk = w.accused_pk();
    let (a, b) = pair(&w.accused, chain_genesis());
    let (lo, hi) = by_hash(a.clone(), b.clone());
    let canon = slash_tx(pk, lo.clone(), hi.clone(), Signature::default());
    let proof = proof_of(&w, a, b);
    let mut trailing = canon.clone();
    trailing.extra_data.push(0);
    let variants = [
        proof.to_slash_transaction(&w.reporter),
        proof.to_slash_transaction(&KeyPair::generate()),
        slash_tx(pk, hi, lo, Signature::default()),
        trailing,
    ];
    let mut ids: HashSet<Hash> = variants.iter().map(|t| t.hash()).collect();
    ids.insert(canon.hash());
    assert_eq!(ids.len(), 5, "PRECONDITION: five distinct ids for one pair");

    for v in &variants {
        w.node.handle_new_transaction(v.clone()).await;
    }
    w.node.handle_new_transaction(canon.clone()).await;
    for v in &variants {
        w.node.handle_new_transaction(v.clone()).await;
    }
    assert_eq!(
        resident_slashes(&w.node, &pk).await,
        vec![canon.hash()],
        "B8: exactly one variant may be admitted and it must be the canonical id"
    );
}

// REQ-SLASH-002 — Decision: a failure means relay disagrees with admission for slashes (valid evidence never forwarded, or invalid evidence forwarded).
#[tokio::test]
async fn inc_i_255_b3_gossip_relay_verdict_follows_admission() {
    let w = world().await;
    let (a, b) = pair(&w.accused, chain_genesis());
    let reporter_signed = proof_of(&w, a.clone(), b.clone()).to_slash_transaction(&w.reporter);
    let valid = canonical(w.accused_pk(), a, b);

    let bad = w.node.handle_gossip_txs(vec![reporter_signed]).await;
    assert!(
        matches!(bad, Verdict::Ignore),
        "a reporter-signed slash must not be relayed"
    );
    let good = w.node.handle_gossip_txs(vec![valid]).await;
    assert!(
        matches!(good, Verdict::Accept),
        "B3: a valid slash admitted locally must be relayed"
    );
}

// REQ-SLASH-001 — Decision: a failure means a second equivocation by an accused whose slash is already resident adds another slash, the per-offender flood.
#[tokio::test]
async fn inc_i_255_handler_skips_when_slash_already_resident() {
    let mut w = world().await;
    let pk = w.accused_pk();
    let first = valid_slash(&w);
    assert!(
        w.node.handle_new_transaction(first.clone()).await,
        "PRECONDITION: a slash for the accused is resident"
    );
    let g = chain_genesis();
    let c = evidence_header(&w.accused, g, SLOT, 0xC3, 0);
    let d = evidence_header(&w.accused, g, SLOT, 0xD4, 0);
    let second = canonical(pk, c.clone(), d.clone());
    assert_ne!(second.hash(), first.hash(), "FIXTURE: a distinct pair");
    let before = w.node.mempool.read().await.iter().count();

    w.node.handle_equivocation(proof_of(&w, c, d)).await;

    assert_eq!(
        w.node.mempool.read().await.iter().count(),
        before,
        "the handler must not add a second slash for a resident accused"
    );
    assert_eq!(resident_slashes(&w.node, &pk).await, vec![first.hash()]);
    w.node
        .mempool
        .write()
        .await
        .remove_transaction(&first.hash());
    let r = admit(&w.node, &second).await;
    assert!(
        r.is_ok(),
        "NON-VACUITY: once the first leaves, admission accepts the withheld tx; got {r:?}"
    );
}

// REQ-SLASH-001 — Decision: a failure means the handler lands a slash that its own admission refuses (unregistered accused).
#[tokio::test]
async fn inc_i_255_handler_refused_by_admission_lands_nothing() {
    let mut w = world().await;
    let stranger = KeyPair::generate();
    let spk = *stranger.public_key();
    let (a, b) = pair(&stranger, chain_genesis());
    let mut proof = proof_of(&w, a.clone(), b.clone());
    proof.producer = spk;
    let before = w.node.mempool.read().await.iter().count();

    w.node.handle_equivocation(proof).await;

    assert_eq!(
        w.node.mempool.read().await.iter().count(),
        before,
        "a refused slash must not land in the mempool"
    );
    assert!(resident_slashes(&w.node, &spk).await.is_empty());
    let r = admit(&w.node, &canonical(spk, a, b)).await;
    assert!(
        mempool_slash_msg(&r).is_some_and(|m| m.contains("not a registered producer")),
        "PRECONDITION: admission refuses the handler's tx; got {r:?}"
    );
    let (c, d) = pair(&w.accused, chain_genesis());
    let expected = canonical(w.accused_pk(), c.clone(), d.clone()).hash();
    w.node.handle_equivocation(proof_of(&w, c, d)).await;
    assert_eq!(
        resident_slashes(&w.node, &w.accused_pk()).await,
        vec![expected],
        "NON-VACUITY: the same armed handler admits for a registered accused"
    );
}
