//! INC-I-294 M-4: every `process_block` exit after `[APPLY_START]` logs one `[APPLY_END]`;
//! a drop says `status=dropped reason=<reason>`.
// covers: bins/node/src/node/block_handling.rs (process_block)
//
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()> (-> process_block)
//   O1 INFO `[APPLY_START]` events (count)
//   O2 INFO `[APPLY_END]` events (count, slot/hash, status, reason)
//   O3 chain tip (best_height / best_hash) — moves only on the applied path
//   O4 return: ignored (drops return Ok(()))
// PATHS: P1 auth drop (unsigned header, devnet gate 0) | P2 classify Rejected (genesis mismatch)
//   | P3 orphan (parent unknown, cached) | P4 ExtendsTip applied
// MATRIX
//   P1 x O1,O2 -> 1 START, 1 END `status=dropped reason=unauthenticated`, tip unchanged
//   P2 x O1,O2 -> 1 START, 1 END `status=dropped reason=` + classify text "different chain", tip unchanged
//   P3 x O1,O2 -> 1 START, 1 END `status=dropped reason=`, tip unchanged
//   P4 x O1,O2,O3 -> 1 START, 1 END `status=applied` (control, passes today), tip advanced
// INPUT PARTITIONS: exit before classify (P1) | classify verdict (P2) | dispatch arm (P3) | apply (P4)
//   probe: each test prints `unmatched_apply_start=<starts - ends>`

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::Block;
use tracing_subscriber::layer::SubscriberExt;

use super::inc_i_182_m1_common::Logs;
use super::inc_i_236_m4_common::{header_block, wall_slot};
use super::inc_i_255_m1_common::{chain_genesis, full_block_with, tip, world, World};
use crate::block_sig::sign_header;

struct Seen {
    starts: Vec<String>,
    ends: Vec<String>,
}

/// A second live dispatcher keeps tracing-core out of its one-dispatcher mode, where an
/// uncaptured thread's first hit of a callsite caches `never` for every thread.
fn keep_interest_rebuilt() {
    static KEEP: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEP.get_or_init(|| tracing::Dispatch::new(tracing_subscriber::registry()));
}

async fn feed_logged(w: &mut World, block: Block) -> Seen {
    let logs = Logs::default();
    {
        keep_interest_rebuilt();
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        let _ = w
            .node
            .handle_new_block(block, network::PeerId::random())
            .await;
    }
    let seen = Seen {
        starts: logs.mentioning("[APPLY_START]"),
        ends: logs.mentioning("[APPLY_END]"),
    };
    eprintln!(
        "unmatched_apply_start={}",
        seen.starts.len() as i64 - seen.ends.len() as i64
    );
    seen
}

fn signer(w: &World) -> (PublicKey, KeyPair) {
    (w.accused_pk(), w.accused.clone())
}

async fn assert_genesis_fixture(w: &World) {
    let g = w.node.chain_state.read().await.genesis_hash;
    assert_eq!(
        g,
        chain_genesis(),
        "FIXTURE: node genesis is the devnet chainspec genesis"
    );
}

fn assert_one_dropped(seen: &Seen, block: &Block, case: &str) -> String {
    assert_eq!(
        seen.starts.len(),
        1,
        "FIXTURE {case}: one [APPLY_START]; got {:?}",
        seen.starts
    );
    assert_eq!(
        seen.ends.len(),
        seen.starts.len(),
        "{case}: exactly one [APPLY_END] per [APPLY_START]; starts={:?} ends={:?}",
        seen.starts,
        seen.ends
    );
    let end = &seen.ends[0];
    assert!(
        end.starts_with("INFO "),
        "{case}: [APPLY_END] must be INFO; got {end:?}"
    );
    let want = format!(
        "[APPLY_END] slot={} hash={} status=dropped reason=",
        block.header.slot,
        block.hash()
    );
    assert!(end.contains(&want), "{case}: want {want:?} in {end:?}");
    end.clone()
}

// INC-I-294 M-4 (Must) — Decision: a failure means an unauthenticated gossip block still vanishes after [APPLY_START] with no INFO verdict, so a dropped tip block is invisible in production logs.
#[tokio::test]
async fn inc_i_294_m4_unauthenticated_drop_logs_apply_end_dropped() {
    let mut w = world().await;
    let (best_h, genesis) = tip(&w.node).await;
    let (pk, _) = signer(&w);
    let slot = wall_slot(&w.node);
    let block = header_block(&mut w.node, 1, slot, genesis, &pk, None, 0).await;
    let seen = feed_logged(&mut w, block.clone()).await;
    assert_eq!(
        tip(&w.node).await.0,
        best_h,
        "FIXTURE: the unsigned block is not applied"
    );
    let end = assert_one_dropped(&seen, &block, "unauthenticated");
    assert!(
        end.contains("reason=unauthenticated"),
        "unauthenticated: reason text; got {end:?}"
    );
}

// INC-I-294 M-4 (Must) — Decision: a failure means a classify reject (other genesis / fork_id) leaves [APPLY_START] unmatched, so operators cannot tell a dropped block from a hung apply.
#[tokio::test]
async fn inc_i_294_m4_classify_reject_logs_apply_end_dropped_with_reason() {
    let mut w = world().await;
    assert_genesis_fixture(&w).await;
    let best_h = tip(&w.node).await.0;
    let (pk, kp) = signer(&w);
    let slot = wall_slot(&w.node);
    let elsewhere = crypto::hash::hash(b"m4 other chain parent");
    let mut block = header_block(&mut w.node, 1, slot, elsewhere, &pk, Some(&kp), 0).await;
    block.header.genesis_hash = Hash::from_bytes([0x94; 32]);
    sign_header(&mut block.header, &kp);
    let seen = feed_logged(&mut w, block.clone()).await;
    assert_eq!(
        tip(&w.node).await.0,
        best_h,
        "FIXTURE: the foreign block is not applied"
    );
    let end = assert_one_dropped(&seen, &block, "classify reject");
    assert!(
        end.contains("different chain"),
        "classify reject: the reason carries the classify text; got {end:?}"
    );
}

// INC-I-294 M-4 (Must) — Decision: a failure means an orphan gossip block (parent unknown) still leaves [APPLY_START] unmatched, the path a dropped tip block takes.
#[tokio::test]
async fn inc_i_294_m4_orphan_logs_apply_end_dropped() {
    let mut w = world().await;
    assert_genesis_fixture(&w).await;
    let best_h = tip(&w.node).await.0;
    let (pk, kp) = signer(&w);
    let slot = wall_slot(&w.node);
    let missing = crypto::hash::hash(b"m4 missing parent");
    let block = header_block(&mut w.node, 2, slot, missing, &pk, Some(&kp), 0).await;
    let seen = feed_logged(&mut w, block.clone()).await;
    assert_eq!(
        tip(&w.node).await.0,
        best_h,
        "FIXTURE: the orphan is not applied"
    );
    let cached = w
        .node
        .fork_block_cache
        .read()
        .await
        .contains_key(&block.hash());
    assert!(
        cached,
        "FIXTURE: the block reached the orphan arm (cached for its parent)"
    );
    assert_one_dropped(&seen, &block, "orphan");
}

// INC-I-294 M-4 (Must) — Decision: a failure means the drop logging changed the applied path (a second [APPLY_END] or a lost status=applied), breaking the latency correlation scripts read.
#[tokio::test]
async fn inc_i_294_m4_applied_block_logs_single_apply_end_applied() {
    let mut w = world().await;
    let before = tip(&w.node).await.0;
    let (block, _) = full_block_with(&mut w.node, &[]).await;
    let seen = feed_logged(&mut w, block.clone()).await;
    assert_eq!(
        tip(&w.node).await,
        (before + 1, block.hash()),
        "FIXTURE: the block applies"
    );
    assert_eq!(
        seen.starts.len(),
        1,
        "one [APPLY_START]; got {:?}",
        seen.starts
    );
    assert_eq!(
        seen.ends.len(),
        1,
        "exactly one [APPLY_END]; got {:?}",
        seen.ends
    );
    let applied = seen
        .ends
        .iter()
        .filter(|l| l.contains("status=applied"))
        .count();
    assert_eq!(
        applied, 1,
        "status=applied exactly once; got {:?}",
        seen.ends
    );
    assert!(
        !seen.ends[0].contains("status=dropped"),
        "applied block must not say dropped; got {:?}",
        seen.ends[0]
    );
}
