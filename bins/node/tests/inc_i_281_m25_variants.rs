// INC-I-281 M2.5 — SEC-CONS-002 (P2, P1-B Must): after a refused reorg the same boundary block with
// a new unscheduled tail passes every door again (round-robin skipped) and costs a full rollback.
//
// covers: reorg_restore block_handling reorg_schedule door_epoch validation_checks mod init wedge_escape fork_walk force_reorg fork_recovery post_commit floor_window fork_cache reorg_refusals
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis) after the in-reorg refusal of
//   [C_B, C'_{B+1}, C'_{B+2}] (m24 refused_setup), then a variant or honest branch on C_B.
// Functions: Node::handle_completed_fork_recovery (walk door), Node::handle_new_block (wedge
//   door), Node::try_consume_force_reorg (forceReorgTo door).
// OUTPUTS:
//   O1 return value / ForceReorgOutcome
//   O2 chain_state best_hash / best_height; O3 canonical hash at B..B+2; O4 persisted tip
//   O5 reorg-handler weight of the variant tail block / memo-held tip
//   O6 logs: rollback / fork-apply / restore lines; `Executing reorg`
// PATHS: variant at walk door | at wedge door | at forceReorgTo | memo-held sibling at the wedge
//   door | honest tail under the refused anchor (control)
// INPUT PARTITIONS: variant X'_{B+1} at s1 by a registered producer that is neither L_new(C_B)[s1]
//   nor L_new(D_B)[s1] (new hash), tail scheduled under L_new(C_B); honest C_{B+1} = L_new(C_B)[s1].
// MATRIX: walk O1-O4 O6 | wedge O2-O6 | force O1-O4 O6 | memo O2-O6 | honest O1-O4 + landed.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::Hash;
use doli_core::Block;
use doli_node::node::ForceReorgOutcome;
use inc_i_236_m2_reorg_support::{seed_cache, weight_recorded};
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::{rollback_lines, Logs};
use network::PeerId;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE variants={cell} wrong={}", wrong as u8);
}

async fn setup() -> Fx {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    fx
}

/// The m24 refusal: [C_B, C'_{B+1}, C'_{B+2}] refused in the reorg, node restored on D_{B+1}.
async fn refused_first() -> Fx {
    let mut fx = setup().await;
    let br = fx.ineligible();
    let before = fx.state().await;
    let logs = Logs::default();
    let result = {
        let _g = tracing::subscriber::set_default(logs.subscriber());
        let f = fx.f_hash;
        run_walk(&mut fx, &br, f).await
    };
    assert!(result.is_ok(), "precondition: first walk Ok ({result:?})");
    assert!(
        logs.mentioning("is not scheduled at its parent") >= 1,
        "precondition: the in-reorg check refused C'_B+1"
    );
    assert_eq!(fx.state().await, before, "precondition: restored on D_B+1");
    fx
}

/// [C_B, X'_{B+1}, V_{B+2}, V_{B+3}, V_{B+4}]: X' unscheduled under L_new(C_B), V scheduled.
async fn variant(fx: &Fx) -> Vec<Block> {
    let (canon, dead) = (at(&fx.l_canon, fx.s1), at(&fx.l_dead, fx.s1));
    let by = fx
        .keys
        .iter()
        .find(|k| *k.public_key() != canon && *k.public_key() != dead)
        .expect("fixture: a third producer");
    assert!(
        fx.node
            .producer_set
            .read()
            .await
            .get_by_pubkey(by.public_key())
            .is_some(),
        "precondition: X'_B+1's producer is registered"
    );
    let x = fx.mk(fx.c_b.hash(), fx.b() + 1, fx.s1, by);
    let mut out = vec![fx.c_b.clone(), x];
    for (i, s) in (fx.s2..fx.s2 + 3).enumerate() {
        let prev = out.last().unwrap().hash();
        let v = fx.mk(prev, fx.b() + 2 + i as u64, s, fx.key(&at(&fx.l_canon, s)));
        out.push(v);
    }
    out
}

fn logged() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let g = tracing::subscriber::set_default(logs.subscriber());
    (logs, g)
}

/// Outputs that show the node rolled back or moved.
async fn no_rollback_wrongs(fx: &Fx, before: &State, logs: &Logs) -> Vec<String> {
    let mut wrong: Vec<String> = rollback_lines(logs)
        .into_iter()
        .map(|(l, n)| format!("{n} x `{l}`"))
        .collect();
    let st = fx.state().await;
    if st != *before {
        wrong.push(format!("tip moved: {st:?}"));
    }
    wrong
}

// P1-B / SEC-CONS-002 (Must) — Decision: a failure means a variant tail on the refused anchor C_B passes the walk door again, so every fresh tail hash costs a rollback, two atomic_replace and a restore.
#[tokio::test]
async fn variant_tail_refused_at_walk_door_without_rollback() {
    let mut fx = refused_first().await;
    let v = variant(&fx).await;
    let before = fx.state().await;
    let (logs, g) = logged();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &v, f).await;
    drop(g);
    let mut wrong = no_rollback_wrongs(&fx, &before, &logs).await;
    if result.is_err() {
        wrong.push(format!("result {result:?}"));
    }
    if fx.applied(&v[1]) {
        wrong.push("X'_B+1 applied".to_string());
    }
    probe("walk", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "variant [C_B, X'_B+1, ..] at the walk door: {wrong:?}"
    );
}

// P1-B / SEC-CONS-002 (Must) — Decision: a failure means a gossiped variant tail on the refused anchor records weight and rolls the node back through the wedge door.
#[tokio::test]
async fn variant_tail_refused_at_wedge_door_without_rollback() {
    let mut fx = refused_first().await;
    let v = variant(&fx).await;
    let before = fx.state().await;
    let (logs, g) = logged();
    for b in &v {
        let r = fx.node.handle_new_block(b.clone(), PeerId::random()).await;
        assert!(r.is_ok(), "handle_new_block {:.16}: {r:?}", b.hash());
    }
    drop(g);
    let mut wrong = no_rollback_wrongs(&fx, &before, &logs).await;
    if weight_recorded(&fx.node, &v[1].hash()).await {
        wrong.push("X'_B+1 weight recorded (passed the wedge door)".to_string());
    }
    if fx.applied(&v[1]) {
        wrong.push("X'_B+1 applied".to_string());
    }
    probe("wedge", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "variant [C_B, X'_B+1, ..] at the wedge door: {wrong:?}"
    );
}

// P1-B / SEC-CONS-002 (Should) — Decision: a failure means forceReorgTo onto a variant tip of the refused anchor rolls the node back before the in-reorg check refuses the tail.
#[tokio::test]
async fn variant_tail_refused_by_force_reorg_without_rollback() {
    let mut fx = refused_first().await;
    let v = variant(&fx).await;
    let before = fx.state().await;
    seed_cache(&fx.node, &v).await;
    let (logs, g) = logged();
    let target = v.last().unwrap().hash();
    fx.node.sync_manager.write().await.arm_force_reorg(target);
    let outcome = fx.node.try_consume_force_reorg().await;
    drop(g);
    let mut wrong = no_rollback_wrongs(&fx, &before, &logs).await;
    if outcome != Some(ForceReorgOutcome::Ineligible) {
        wrong.push(format!("outcome {outcome:?}, expected Ineligible"));
    }
    probe("force", !wrong.is_empty());
    assert!(wrong.is_empty(), "forceReorgTo the variant tip: {wrong:?}");
}

// P1-B / SEC-CONS-002 (Must) — Decision: a failure means the wedge door ignores the LB-9 memo, so a branch tip already rejected on this local tip records weight and re-runs a reorg from gossip.
#[tokio::test]
async fn wedge_door_consults_rejected_branch_memo() {
    let mut fx = setup().await;
    let canon = fx.canonical();
    let tip: Hash = canon[2].hash();
    fx.node
        .sync_manager
        .write()
        .await
        .mark_fork_branch_rejected(tip, fx.d[1].hash());
    let before = fx.state().await;
    let (logs, g) = logged();
    for b in &canon {
        gossip(&mut fx, b).await;
    }
    drop(g);
    let mut wrong = no_rollback_wrongs(&fx, &before, &logs).await;
    let attempts = logs.mentioning("Executing reorg");
    if attempts > 0 {
        wrong.push(format!("{attempts} x `Executing reorg`"));
    }
    if weight_recorded(&fx.node, &tip).await {
        wrong.push("memo-held tip C_B+2 weight recorded".to_string());
    }
    probe("memo", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "memo-held branch tip at the wedge door: {wrong:?}"
    );
}

// P1-B (Must, control) — Decision: a failure means the anchor memory also refuses the honest tail scheduled under the anchor's own list, so a minority can no longer walk back onto the canonical branch.
#[tokio::test]
async fn honest_tail_under_refused_anchor_still_reorgs() {
    let mut fx = refused_first().await;
    let canon = fx.canonical();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &canon, f).await;
    let st = fx.state().await;
    let landed = st.best_hash == canon[2].hash()
        && st.at_b == Some(canon[0].hash())
        && st.persisted == Some(canon[2].hash());
    probe("honest", !landed || result.is_err());
    result.expect("walk: Ok");
    assert_landed(&fx, &canon, fx.b(), "honest tail after the refusal").await;
    assert_eq!(
        active(&fx.node.epoch_state),
        fx.l_canon,
        "the node re-derived L_new(C_B)"
    );
}
