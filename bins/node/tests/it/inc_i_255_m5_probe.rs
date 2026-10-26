//! INC-I-255 M5 probes (AUDIT-P1-517) — one signed decoy header from the
//! accused must not stop the node's automatic slash for the real pair (A, B).
//!
// covers: equivocation header_auth block_handling rewards slash_submit slash_evidence validation/mod network lib sync mod protocol security_model architecture dev-notes
//
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>
//   (reaches EquivocationDetector -> handle_equivocation -> submit_equivocation_slash)
//   O1 mempool SlashProducer entries naming the accused (ids, count)
//   O2 return: ignored (intake verdicts unchanged by M5; side effects fire first)
// PATHS: P-ON devnet default slash gate, every block via gossip intake
// MATRIX
//   O1 x decoy foreign genesis -> A -> B -> [canonical(A,B)] (inc_i_255_m5_probe_decoy_genesis)
//   O1 x decoy garbage VDF -> A -> B -> [canonical(A,B)] (inc_i_255_m5_probe_decoy_vdf)
//   O1 x decoy VDF-valid but oversize -> A -> B -> [canonical(A,B)] (inc_i_255_m5_probe_decoy_oversize)
//   O1 x 5 garbage-VDF decoys -> A -> B -> [canonical(A,B)] (inc_i_255_m5_probe_many_bad_vdf)
// INPUT PARTITIONS:
//   decoy: foreign genesis | garbage VDF | oversize (missed_producers) | 5 x garbage VDF
//   order: decoys first, then A, then B (the accused controls all three)

use super::inc_i_255_m5_common::*;

async fn assert_one_ab_slash(s: &Scene, case: &str) {
    let ids = s.slashes().await;
    assert_eq!(
        ids,
        vec![s.ab_id()],
        "{case}: decoy then A then B must leave exactly one admitted canonical (A,B) slash; mempool holds {ids:?}"
    );
}

// REQ-SLASH-005 — Decision: a failure means one foreign-genesis decoy switches off the automatic slash for the real pair (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_probe_decoy_genesis() {
    let mut s = scene().await;
    let decoy = s.decoy_foreign_genesis().await;
    let (a, b) = (s.a.clone(), s.b.clone());
    for blk in [&decoy, &a, &b] {
        s.feed(blk).await;
    }
    assert_one_ab_slash(&s, "foreign-genesis decoy").await;
}

// REQ-SLASH-005 — Decision: a failure means one garbage-VDF decoy switches off the automatic slash for the real pair (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_probe_decoy_vdf() {
    let mut s = scene().await;
    let decoy = s.decoy_bad_vdf(0).await;
    let (a, b) = (s.a.clone(), s.b.clone());
    for blk in [&decoy, &a, &b] {
        s.feed(blk).await;
    }
    assert_one_ab_slash(&s, "garbage-VDF decoy").await;
}

// REQ-SLASH-005 — Decision: a failure means a VDF-valid decoy too large for any SlashData blocks the pair that fits (AUDIT-P1-517).
#[tokio::test]
async fn inc_i_255_m5_probe_decoy_oversize() {
    let mut s = scene().await;
    let decoy = s.decoy_oversize().await;
    let (a, b) = (s.a.clone(), s.b.clone());
    for blk in [&decoy, &a, &b] {
        s.feed(blk).await;
    }
    assert_one_ab_slash(&s, "oversize decoy").await;
}

// REQ-SLASH-005 — Decision: a failure means N cheap bad-VDF decoys exhaust the detector before A and B; a pre-proof count cap is rejected because it re-opens AUDIT-P1-517.
#[tokio::test]
async fn inc_i_255_m5_probe_many_bad_vdf() {
    const DECOYS: u8 = 5;
    let mut s = scene().await;
    let mut decoys = Vec::new();
    for i in 0..DECOYS {
        decoys.push(s.decoy_bad_vdf(i).await);
    }
    let (a, b) = (s.a.clone(), s.b.clone());
    for blk in decoys.iter().chain([&a, &b]) {
        s.feed(blk).await;
    }
    assert_one_ab_slash(&s, "5 garbage-VDF decoys").await;
}
