// INC-I-281 M2.5 — SEC-CONS-001 (P1, P1-A Must): a door-admitted fork block that fails apply_block
// after the rollback leaves the node rolled back; only the schedule refusal restores the prior tip.
//
// covers: reorg_restore block_handling reorg_schedule door_epoch validation_checks mod init wedge_escape fork_walk force_reorg fork_recovery post_commit floor_window fork_cache reorg_refusals
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis) walks a heavier branch connected at F.
// Functions: Node::handle_completed_fork_recovery (walk door -> fork choice -> execute_reorg ->
//   apply failure -> restore), Node::handle_new_block (wedge door, later feed).
// OUTPUTS:
//   O1 return value (Ok)
//   O2 chain_state best_hash / best_height; O3 canonical hash at B..B+2; O4 persisted tip
//   O5 epoch state hash + effective list; O6 sync manager local tip
//   O7 producer set canonical bytes (memory + persisted). UTXO set not asserted: new_for_test
//   keeps an empty in-memory UtxoSet, so any reorg's atomic_replace persists it (45 -> 2 after
//   the m24 restore on HEAD); the harness cannot observe it.
//   O8 LB-9 memo: is_fork_branch_rejected(branch tip, D_{B+1})
//   O9 logs: rollback / fork-apply / restore lines
// PATHS: apply failure at index 0 (boundary block) | at index 1 after an applied prefix |
//   later feed of the failed branch extended by one block (walk + gossip) |
//   junk-body copy in the fork cache (no rollback), then the honest copy by gossip (lands)
// INPUT PARTITIONS: failing block = C_B or C_{B+1} plus a spend of a missing UTXO (valid merkle
//   root, re-signed); junk copy = C_B's header with an empty body.
// MATRIX: boundary O1-O9 | tail O1-O9 | remembered O2-O4 O9 | junkbody O2-O4 O8 (memo must NOT
//   hold the honest tip) O9 + landed.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::{Hash, PublicKey};
use doli_core::Block;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::{junk_body, ledger, rollback_lines, with_missing_spend, Ledger, Logs};
use network::PeerId;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE applyfail={cell} wrong={}", wrong as u8);
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    st: State,
    epoch_hash: Hash,
    list: Vec<PublicKey>,
    sync_tip: (u64, Hash),
    ledger: Ledger,
}

async fn snapshot(fx: &Fx) -> Snapshot {
    let (h, hash, _) = fx.node.sync_manager.read().await.local_tip();
    Snapshot {
        st: fx.state().await,
        epoch_hash: fx.node.epoch_state.hash(),
        list: active(&fx.node.epoch_state),
        sync_tip: (h, hash),
        ledger: ledger(&fx.node).await,
    }
}

async fn setup() -> Fx {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    fx
}

/// [C_B*, Y_{B+1}, Y_{B+2}]: C_B* = C_B plus a missing spend; the tail is scheduled under L_new(C_B).
fn boundary_branch(fx: &Fx) -> Vec<Block> {
    let bad = with_missing_spend(&fx.c_b, fx.key(&fx.c_b.header.producer));
    let y1 = fx.mk(
        bad.hash(),
        fx.b() + 1,
        fx.s1,
        fx.key(&at(&fx.l_canon, fx.s1)),
    );
    let y2 = fx.mk(
        y1.hash(),
        fx.b() + 2,
        fx.s2,
        fx.key(&at(&fx.l_canon, fx.s2)),
    );
    vec![bad, y1, y2]
}

/// [C_B, C_{B+1}*, Y_{B+2}]: C_B applies, C_{B+1}* (scheduled, plus a missing spend) fails.
fn tail_branch(fx: &Fx) -> Vec<Block> {
    let canon = fx.canonical();
    let bad = with_missing_spend(&canon[1], fx.key(&canon[1].header.producer));
    let y2 = fx.mk(
        bad.hash(),
        fx.b() + 2,
        fx.s2,
        fx.key(&at(&fx.l_canon, fx.s2)),
    );
    vec![canon[0].clone(), bad, y2]
}

async fn walk_logged(fx: &mut Fx, br: &[Block]) -> (anyhow::Result<()>, Logs) {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(logs.subscriber());
    let f = fx.f_hash;
    let result = run_walk(fx, br, f).await;
    (result, logs)
}

async fn rejected(fx: &Fx, branch_tip: Hash, local_tip: Hash) -> bool {
    fx.node
        .sync_manager
        .read()
        .await
        .is_fork_branch_rejected(branch_tip, local_tip)
}

/// Walks `br` (block `bad` fails apply) and returns what differs from a full restore.
async fn failed_walk_wrongs(fx: &mut Fx, br: &[Block], bad: &Block) -> Vec<String> {
    let before = snapshot(fx).await;
    let (result, logs) = walk_logged(fx, br).await;
    let after = snapshot(fx).await;
    let mut wrong = Vec::new();
    if result.is_err() {
        wrong.push(format!("result {result:?}"));
    }
    if after.st != before.st {
        wrong.push(format!(
            "tip/index/persisted not restored: {:?} (before {:?})",
            after.st, before.st
        ));
    }
    if (after.epoch_hash, &after.list) != (before.epoch_hash, &before.list) {
        wrong.push("epoch state not restored".to_string());
    }
    if after.sync_tip != before.sync_tip {
        wrong.push(format!("sync tip {:?}", after.sync_tip));
    }
    let (a, b) = (&after.ledger, &before.ledger);
    if a != b {
        wrong.push("producer set (memory or persisted) differs from before the reorg".to_string());
    }
    if fx.applied(bad) {
        wrong.push("the failing block is stored".to_string());
    }
    if logs.mentioning("[REORG_RESTORE] restored") == 0 {
        wrong.push("no `[REORG_RESTORE] restored` line".to_string());
    }
    let tip = br.last().unwrap().hash();
    if rejected(fx, tip, fx.d[1].hash()).await {
        wrong.push(
            "LB-9 memo holds the branch on D_B+1 after a body-level apply failure".to_string(),
        );
    }
    if inc_i_236_m2_reorg_support::cached(&fx.node, &bad.hash())
        .await
        .is_some()
    {
        wrong.push("the failing body is still in the fork cache".to_string());
    }
    wrong
}

/// `br` plus one block on its tip at its own next slot.
fn extend(fx: &Fx, br: &[Block]) -> Vec<Block> {
    let tip = br.last().unwrap();
    let s3 = fx.s2 + 1;
    let mut ext = br.to_vec();
    ext.push(fx.mk(tip.hash(), fx.b() + 3, s3, fx.key(&at(&fx.l_canon, s3))));
    ext
}

/// Feeds `ext` through the walk door, then by gossip; (rollback-path lines, outputs that moved).
async fn feed_again(fx: &mut Fx, ext: &[Block]) -> (Vec<(&'static str, usize)>, Vec<String>) {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(logs.subscriber());
    let before = fx.state().await;
    let mut moved = Vec::new();
    let f = fx.f_hash;
    let result = run_walk(fx, ext, f).await;
    let st = fx.state().await;
    if st != before {
        moved.push(format!("walk ({result:?}): {st:?}"));
    }
    for b in ext {
        let result = fx.node.handle_new_block(b.clone(), PeerId::random()).await;
        let st = fx.state().await;
        if st != before {
            moved.push(format!("gossip {:.16} ({result:?}): {st:?}", b.hash()));
        }
    }
    (rollback_lines(&logs), moved)
}

// P1-A / SEC-CONS-001 (Must) — Decision: a failure means a boundary block that passes the walk door but fails apply_block after the rollback leaves the node rolled back to F (tip, ledger, epoch state or memo not restored), so one bad block rewrites unfinalized canonical blocks.
#[tokio::test]
async fn apply_failure_at_boundary_restores_prior_tip() {
    let mut fx = setup().await;
    let br = boundary_branch(&fx);
    let bad = br[0].clone();
    let wrong = failed_walk_wrongs(&mut fx, &br, &bad).await;
    probe("boundary", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "apply failure of C_B* after the rollback: {wrong:?}"
    );
}

// P1-A / SEC-CONS-001 (Must) — Decision: a failure means an apply failure after an applied fork prefix (C_B landed, C_B+1* fails) leaves the node on the fork prefix instead of its prior tip.
#[tokio::test]
async fn apply_failure_in_tail_restores_prior_tip() {
    let mut fx = setup().await;
    let br = tail_branch(&fx);
    let bad = br[1].clone();
    let wrong = failed_walk_wrongs(&mut fx, &br, &bad).await;
    probe("tail", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "apply failure of C_B+1* after C_B applied: {wrong:?}"
    );
}

// P1-A / SEC-CONS-001 (Must) — Decision: a failure means the apply-failed block is not remembered, so the same branch extended by a new tip wins fork choice again and rolls the node back every slot.
#[tokio::test]
async fn apply_failed_block_refused_before_rollback() {
    let mut fx = setup().await;
    let br = boundary_branch(&fx);
    let original = fx.state().await;
    let (first, _logs) = walk_logged(&mut fx, &br).await;
    let ext = extend(&fx, &br);
    let (lines, moved) = feed_again(&mut fx, &ext).await;
    let end = fx.state().await;
    probe(
        "remembered",
        !lines.is_empty() || !moved.is_empty() || end != original,
    );
    assert!(
        lines.is_empty(),
        "the extended failed branch reached a rollback / fork apply again: {lines:?} (first walk {first:?})"
    );
    assert!(moved.is_empty(), "the later feed moved the node: {moved:?}");
    assert_eq!(
        end, original,
        "the node is not on its own tip D_B+1 after both feeds"
    );
}

// P1-A / SEC-CONS-001 profile A (Must) — Decision: a failure means a junk-body copy of an honest header rolls the node back (no merkle check before the rollback) or makes the honest block refused, so a zero-stake relay forces rollbacks or blocks the canonical branch.
#[tokio::test]
async fn junk_body_copy_does_not_refuse_honest_block() {
    let mut fx = setup().await;
    let canon = fx.canonical();
    let mut walked = canon.clone();
    walked[0] = junk_body(&canon[0]);
    let before = fx.state().await;
    let (r1, logs) = walk_logged(&mut fx, &walked).await;
    let mid = fx.state().await;
    let lines = rollback_lines(&logs);
    let memo = rejected(&fx, canon[2].hash(), fx.d[1].hash()).await;
    for b in &canon {
        gossip(&mut fx, b).await;
    }
    let st = fx.state().await;
    let landed = st.best_hash == canon[2].hash()
        && st.best_height == fx.b() + 2
        && st.at_b == Some(canon[0].hash())
        && st.persisted == Some(canon[2].hash());
    probe(
        "junkbody",
        mid != before || !lines.is_empty() || memo || !landed || r1.is_err(),
    );
    assert!(r1.is_ok(), "junk walk returned {r1:?}");
    assert!(
        lines.is_empty(),
        "the junk-body copy of C_B reached a rollback / fork apply: {lines:?}"
    );
    assert_eq!(mid, before, "the junk-body walk moved the node");
    assert!(
        !memo,
        "the junk copy put the honest branch tip C_B+2 in the LB-9 memo on D_B+1"
    );
    assert_landed(&fx, &canon, fx.b(), "honest copy after the junk copy").await;
}
