// INC-I-281 M2.4 — SEC-FIXVERIFY-001 (P1, R-3a Must): a walked block after a boundary block
// that only the walked branch holds skips the door's round-robin arm but counts in fork-choice
// weight; execute_reorg rolls the node back, applies the prefix, then refuses the block.
//
// covers: block_handling reorg_schedule reorg_restore fork_recovery fork_walk wedge_escape force_reorg door_epoch validation_checks mod init network_events rollback_memory fork_cache
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis) handles a heavier walked branch
//   [C_B, A_{B+1}, A_{B+2}] connected at F.
// Functions: Node::handle_completed_fork_recovery (walk door -> fork choice -> execute_reorg ->
//   in-reorg schedule refusal -> restore), Node::handle_new_block (wedge door).
// OUTPUTS:
//   O1 return value (Ok; no panic)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at B, B+1, B+2; body presence of the refused block
//   O4 state_db persisted chain-state tip
//   O5 epoch state (hash over every field, effective list)
//   O6 sync manager local tip
//   O7 LB-9 memo: is_fork_branch_rejected(branch tip, node tip)
//   O8 rollback memory: removed_by_rollback(D_B | D_{B+1} | C_B)
//   O9 logs: `[REORG_RESTORE] restored` / `failed`; rollback-path lines on a later feed
// PATHS: refusal + restore ok | refusal + restore fails | later feed of the refused branch
//   extended by one block (walk door, wedge door) | no refusal (honest straddle)
// INPUT PARTITIONS:
//   A_{B+1} = C'_{B+1}: registered, L_new(D_B)[s1], not L_new(C_B)[s1] -> refused in the reorg
//   A_{B+1} = C_{B+1}:  L_new(C_B)[s1]                                 -> lands
//   node store: intact | D_{B+1} body replaced by a stub (header, empty body)
// MATRIX: restores_prior_tip O1-O6 O8 O9 | rejects_branch O7 | extension_no_rollback O2 O9
//   (walk + wedge) | restore_failure O1-O4 O7 O9 + later feed | honest_straddle O1-O5 O9.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::{Hash, PublicKey};
use doli_core::Block;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::Logs;
use network::PeerId;

/// Lines only a rollback, a fork apply or a restore emits.
const ROLLBACK_LINES: [&str; 7] = [
    "Rolling back from height",
    "Undo-based rollback",
    "[ROLLBACK] Initiating",
    "ROLLBACK_DONE",
    "new blocks from fork",
    "is not scheduled at its parent",
    "[REORG_RESTORE]",
];

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE refusedreorg={cell} wrong={}", wrong as u8);
}

fn rollback_lines(logs: &Logs) -> Vec<(&'static str, usize)> {
    ROLLBACK_LINES
        .iter()
        .map(|l| (*l, logs.mentioning(l)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Every output a restore returns to its pre-reorg value.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    st: State,
    epoch_hash: Hash,
    list: Vec<PublicKey>,
    sync_tip: (u64, Hash),
}

async fn snapshot(fx: &Fx) -> Snapshot {
    let (h, hash, _) = fx.node.sync_manager.read().await.local_tip();
    Snapshot {
        st: fx.state().await,
        epoch_hash: fx.node.epoch_state.hash(),
        list: active(&fx.node.epoch_state),
        sync_tip: (h, hash),
    }
}

/// Post-genesis fixture and [C_B, C'_{B+1}, C'_{B+2}], C'_{B+1} registered but unscheduled.
async fn refused_setup() -> (Fx, Vec<Block>) {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    let br = fx.ineligible();
    let by = br[1].header.producer;
    assert_ne!(
        by,
        at(&fx.l_canon, fx.s1),
        "precondition: C'_B+1 is not L_new(C_B)[s1]"
    );
    assert!(
        fx.node
            .producer_set
            .read()
            .await
            .get_by_pubkey(&by)
            .is_some(),
        "precondition: C'_B+1's producer is registered (membership arm passes)"
    );
    (fx, br)
}

async fn walk_logged(fx: &mut Fx, br: &[Block]) -> (anyhow::Result<()>, Logs) {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(logs.subscriber());
    let f = fx.f_hash;
    let result = run_walk(fx, br, f).await;
    (result, logs)
}

/// `br` plus one block on its tip at its own next slot.
fn extend(fx: &Fx, br: &[Block]) -> Vec<Block> {
    let tip = br.last().unwrap();
    let s3 = fx.s2 + 1;
    let by = fx.key(&at(&fx.l_canon, s3));
    let mut ext = br.to_vec();
    ext.push(fx.mk(tip.hash(), fx.b() + 3, s3, by));
    ext
}

/// Feeds `ext` through the walk door, then block by block through gossip (wedge door).
/// Returns (rollback-path lines seen, outputs that moved).
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

async fn rejected(fx: &Fx, branch_tip: Hash, local_tip: Hash) -> bool {
    fx.node
        .sync_manager
        .read()
        .await
        .is_fork_branch_rejected(branch_tip, local_tip)
}

// R-3a / SEC-FIXVERIFY-001 (Must) — Decision: a failure means a reorg refused by the in-reorg schedule check leaves the node off the tip it had before (chain state, index, persisted tip, epoch state or sync tip), so a door-skipped unscheduled tail rewrites unfinalized blocks.
#[tokio::test]
async fn refused_reorg_restores_prior_tip() {
    let (mut fx, br) = refused_setup().await;
    let before = snapshot(&fx).await;
    let (result, logs) = walk_logged(&mut fx, &br).await;
    let after = snapshot(&fx).await;
    let applied = fx.applied(&br[1]) || fx.applied(&br[2]);
    let restored = logs.mentioning("[REORG_RESTORE] restored");
    let failed = logs.mentioning("[REORG_RESTORE] failed");
    let mut held = Vec::new();
    for b in [&fx.d[0], &fx.d[1], &fx.c_b] {
        if fx
            .node
            .sync_manager
            .read()
            .await
            .removed_by_rollback(&b.hash())
        {
            held.push(b.hash());
        }
    }
    let wrong = result.is_err()
        || after != before
        || applied
        || restored == 0
        || failed > 0
        || !held.is_empty();
    probe("tip_restored", wrong);
    assert!(
        !applied,
        "the unscheduled C'_B+1 (or its child) was applied ({after:?})"
    );
    assert_eq!(
        after.st, before.st,
        "tip / height / canonical B..B+2 / persisted tip not restored ({result:?})"
    );
    assert_eq!(
        (after.epoch_hash, &after.list),
        (before.epoch_hash, &before.list),
        "epoch state not restored to L_new(D_B)"
    );
    assert_eq!(
        after.sync_tip, before.sync_tip,
        "sync manager tip not restored"
    );
    assert!(
        restored >= 1 && failed == 0,
        "expected `[REORG_RESTORE] restored` and no `failed` line ({restored}, {failed})"
    );
    assert!(
        held.is_empty(),
        "restore rollbacks were recorded as fork-remedy removals: {held:?}"
    );
    result.expect("walk: Ok");
}

// R-3a / SEC-FIXVERIFY-001 (Must) — Decision: a failure means the refused walked branch is not remembered on the restored tip, so the next walk door re-walks it and re-runs the rollback.
#[tokio::test]
async fn refused_reorg_rejects_branch() {
    let (mut fx, br) = refused_setup().await;
    let (result, _logs) = walk_logged(&mut fx, &br).await;
    let tip = fx.state().await.best_hash;
    let branch_tip = br.last().unwrap().hash();
    let on_restored = rejected(&fx, branch_tip, fx.d[1].hash()).await;
    probe("branch_rejected", !on_restored);
    assert!(
        on_restored,
        "LB-9 memo does not hold branch tip {branch_tip:.16} on the restored tip D_B+1 (node tip {tip:.16}, {result:?})"
    );
}

// R-3a / SEC-FIXVERIFY-001 (Must) — Decision: a failure means the refused block is not remembered, so the same branch extended by a new tip re-wins fork choice and rolls the node back every slot.
#[tokio::test]
async fn refused_branch_extension_causes_no_rollback() {
    let (mut fx, br) = refused_setup().await;
    let original = fx.state().await;
    let (first, _logs) = walk_logged(&mut fx, &br).await;
    let ext = extend(&fx, &br);
    let (lines, moved) = feed_again(&mut fx, &ext).await;
    let end = fx.state().await;
    probe(
        "extension_no_rollback",
        !lines.is_empty() || !moved.is_empty() || end != original,
    );
    assert!(
        lines.is_empty(),
        "the extended refused branch reached a rollback / fork apply again: {lines:?} (first walk {first:?})"
    );
    assert!(moved.is_empty(), "the later feed moved the node: {moved:?}");
    assert_eq!(
        end, original,
        "the node is not on its own tip D_B+1 after both feeds"
    );
    assert!(!fx.applied(&ext[1]), "C'_B+1 applied by a later feed");
}

// R-3a / SEC-FIXVERIFY-001 (Must) — Decision: a failure means a restore that cannot complete panics, errors, leaves an inconsistent tip, or forgets the refused branch, so a reorg loop or a corrupt state follows.
#[tokio::test]
async fn refused_reorg_restore_failure_is_logged_and_branch_rejected() {
    let (mut fx, br) = refused_setup().await;
    // Injection: D_B+1's stored body becomes the M3a stub (its header, empty body); the
    // restore's Light re-apply of D_B+1 fails InvalidMerkleRoot (inc_i_281_m3a_stub_serve).
    let stub = Block {
        header: fx.d[1].header.clone(),
        transactions: vec![],
        aggregate_bls_signature: vec![],
        attestation_bitfield: vec![],
    };
    assert!(
        !stub.verify_merkle_root(),
        "precondition: stub unapplicable"
    );
    fx.node
        .block_store
        .put_block(&stub, fx.b() + 1)
        .expect("store the stub body");
    let stored = fx
        .node
        .block_store
        .get_block(&fx.d[1].hash())
        .expect("store");
    assert_eq!(
        stored.map(|b| b.transactions.len()),
        Some(0),
        "precondition: D_B+1 is stored with the stub body"
    );

    let (result, logs) = walk_logged(&mut fx, &br).await;
    let st = fx.state().await;
    let store = &fx.node.block_store;
    let at_tip = store.get_hash_by_height(st.best_height).expect("store");
    let above = store.get_hash_by_height(st.best_height + 1).expect("store");
    let failed = logs.mentioning("[REORG_RESTORE] failed");
    let on_tip = rejected(&fx, br.last().unwrap().hash(), st.best_hash).await;
    let applied = fx.applied(&br[1]);
    let ext = extend(&fx, &br);
    let (lines, moved) = feed_again(&mut fx, &ext).await;
    let consistent =
        at_tip == Some(st.best_hash) && above.is_none() && st.persisted == Some(st.best_hash);
    probe(
        "restore_failure",
        result.is_err()
            || failed == 0
            || !consistent
            || applied
            || !on_tip
            || !lines.is_empty()
            || !moved.is_empty(),
    );
    assert!(result.is_ok(), "handling returned {result:?}");
    assert!(
        failed >= 1,
        "no error line `[REORG_RESTORE] failed` when D_B+1 cannot be re-applied ({st:?})"
    );
    assert!(
        consistent,
        "inconsistent tip after a failed restore: {st:?}, canonical at tip {at_tip:?}, above {above:?}"
    );
    assert!(!applied, "the unscheduled C'_B+1 was applied");
    assert!(
        on_tip,
        "LB-9 memo does not hold the refused branch on the node tip {:.16}",
        st.best_hash
    );
    assert!(
        lines.is_empty() && moved.is_empty(),
        "the extended refused branch rolled the node back again: {lines:?} {moved:?}"
    );
}

// R-3a (Must) — Decision: a failure means the refusal/restore also catches an honest straddling branch whose post-boundary block is scheduled under the branch's own list, so a minority can no longer walk back.
#[tokio::test]
async fn honest_straddle_still_reorgs() {
    let mut fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    let canon = fx.canonical();
    let (result, logs) = walk_logged(&mut fx, &canon).await;
    let st = fx.state().await;
    let tip = canon[2].hash();
    let landed = st.best_hash == tip
        && st.best_height == fx.b() + 2
        && st.at_b1 == Some(canon[1].hash())
        && st.persisted == Some(tip);
    probe("honest_straddle", !landed);
    result.expect("walk: Ok");
    assert_landed(&fx, &canon, fx.b(), "honest straddle").await;
    assert_eq!(
        active(&fx.node.epoch_state),
        fx.l_canon,
        "the node re-derived L_new(C_B)"
    );
    assert_eq!(
        logs.mentioning("[REORG_RESTORE]"),
        0,
        "a fully scheduled branch ran a restore"
    );
}
