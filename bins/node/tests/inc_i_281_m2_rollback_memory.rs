// INC-I-281 M2.2 — R-3b (Must), root cause RC-R: after a fork-remedy rollback the
// removed dead blocks are re-adopted from the store, outside fork choice, so the
// canonical sibling that arrives next finds its height occupied.
//
// OUTPUT CONTRACT: non-fork-choice intake of a block after rollback_one_block
//   (CoordinatorApproved, the coordinator's ShallowRollback).
// Functions: Node::handle_new_block (gossip; ExtendsTip after the REAPPLY check),
//   Node::on_sync_response (orphan-chase Block reply; header-first Bodies),
//   Node::drain_cached_children (behind an ExtendsTip apply),
//   Node::run_periodic_tasks (pending-sync apply loop).
// OUTPUTS:
//   O1 return value (Ok on every path)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at F+1 and F+2
//   O4 state_db persisted chain-state tip
//   O5 sync_manager local tip height
// PATHS: gossip/chase | cached drain | synced bodies | fork choice (walk/wedge/reorg)
// INPUT PARTITIONS: block = {removed by the rollback (D1, D2), never removed (C1)}
//   x {heavier sibling cached, no sibling} (INC-I-281 M2.3: no time window; a lighter
//   sibling is in inc_i_281_m23_false_rollback.rs)
// MATRIX (O1-O5 in every built cell):
//   gossip/chase  x removed x heavier sibling -> stay F; then C1 -> tip C1 (test 1)
//   cached drain  x removed x heavier sibling -> stay F; then C1 -> tip C1 (test 2)
//   synced bodies x removed x heavier sibling -> stay F; then C1 -> tip C1 (test 3)
//   gossip        x removed x no sibling      -> tip D1, D2 at once (test 6)
//   gossip        x removed x heavier sibling, after 1 slot -> stay F (test 7)
//   fork choice   x removed x heavier sibling -> tip D2 (test 7)
//   cached drain  x removed x heavier sibling -> D2 held, back in cache (test 8)
// O6 fork cache content and O7 the reported-hold mark are read in test 8 only.
// refs: R-3b, INC-I-281; INC-I-147 D4 stays in inc_i_147_d4_rollback_reapply.rs.
// covers: block_handling fork_cache rollback rollback_memory block_lifecycle peers recovery sync_manager periodic network_events
// covers: bins/node/src/node/block_handling.rs
// covers: bins/node/src/node/fork_cache.rs
// covers: bins/node/src/node/rollback.rs
// covers: bins/node/src/node/periodic.rs
// covers: bins/node/src/node/network_events.rs
// covers: crates/network/src/sync/manager/rollback_memory.rs
// covers: crates/network/src/sync/manager/block_lifecycle.rs
// covers: crates/network/src/sync/manager/mod.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_m2_support;

use inc_i_236_m2_reorg_support::seed_cache;
use inc_i_281_m2_support::*;
use network::protocols::SyncResponse;
use network::sync::CompletedRecovery;
use network::PeerId;

// R-3b (Must) — Decision: a failure means a gossip or orphan-chase copy of a block the fork-remedy rollback removed is re-applied at once, outside fork choice, so the canonical sibling a few ms later finds its height occupied.
#[tokio::test]
async fn inc_i_281_rollback_not_undone_by_gossip_of_removed_block() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let c1 = r.canon[0].clone();
    // INC-I-281 M2.3: held because a heavier sibling is cached, not by a time window.
    cache_sibling(&r, &c1, F + 1, HEAVY).await;
    let d1 = r.dead[0].clone();
    gossip(&mut r, &d1).await;
    let after_gossip = obs(&r).await;
    r.node
        .on_sync_response(PeerId::random(), SyncResponse::Block(Some(d1)))
        .await
        .expect("chase reply: Ok");
    let after_chase = obs(&r).await;
    let held = at_fork_point(&r, &after_gossip) && at_fork_point(&r, &after_chase);
    probe_reapply("gossip", !held);
    assert!(
        at_fork_point(&r, &after_gossip),
        "gossip of the removed D1 re-adopted it outside fork choice: {after_gossip:?}"
    );
    assert!(
        at_fork_point(&r, &after_chase),
        "the orphan-chase reply of the removed D1 re-adopted it: {after_chase:?}"
    );
    gossip(&mut r, &c1).await;
    assert_on(&r, &c1, F + 1, "canonical C1 after the held D1").await;
}

// R-3b (Must) — Decision: a failure means the cached dead child is drained back onto the tip behind its re-applied removed parent, re-adopting the whole dead branch in one intake.
#[tokio::test]
async fn inc_i_281_rollback_not_undone_by_cached_child_drain() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    seed_cache(&r.node, &r.dead[1..2]).await;
    let c1 = r.canon[0].clone();
    // INC-I-281 M2.3: held because a heavier sibling is cached, not by a time window.
    cache_sibling(&r, &c1, F + 1, HEAVY).await;
    let d1 = r.dead[0].clone();
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    let d2 = r.dead[1].hash();
    let drained = after.at_f2 == Some(d2) || after.best_hash == d2;
    probe_reapply("cached_drain", drained || !at_fork_point(&r, &after));
    assert!(
        !drained,
        "the fork-cache drain re-applied the cached removed D2: {after:?}"
    );
    assert!(
        at_fork_point(&r, &after),
        "the removed branch came back through the drain intake: {after:?}"
    );
    gossip(&mut r, &c1).await;
    assert_on(&r, &c1, F + 1, "canonical C1 with the dead D2 still cached").await;
}

// R-3b (Must) — Decision: a failure means header-first sync from a dead-branch peer re-applies the removed bodies outside fork choice, through the node intake or the periodic pending-sync loop.
#[tokio::test]
async fn inc_i_281_rollback_not_undone_by_synced_bodies() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let c1 = r.canon[0].clone();
    // INC-I-281 M2.3: held because a heavier sibling is cached, not by a time window.
    cache_sibling(&r, &c1, F + 1, HEAVY).await;
    // The dead peer is one block ahead (D3): header-first sync needs a gap of 3.
    let d3 = r.dead[2].clone();
    let peer = PeerId::random();
    r.node
        .sync_manager
        .write()
        .await
        .add_peer(peer, F + 3, d3.hash(), d3.header.slot);
    let dead = r.dead.clone();
    let served = serve_sync(&mut r.node, peer, &dead).await;
    let after_intake = obs(&r).await;
    let tick = r.node.run_periodic_tasks().await;
    let after_tick = obs(&r).await;
    assert!(
        served.contains(&"headers") && served.contains(&"bodies"),
        "precondition: header-first sync fetched the dead headers and bodies; served {served:?}"
    );
    assert!(tick.is_ok(), "run_periodic_tasks: {:?}", tick.err());
    let held = at_fork_point(&r, &after_intake) && at_fork_point(&r, &after_tick);
    probe_reapply("synced_bodies", !held);
    assert!(
        at_fork_point(&r, &after_intake),
        "header-first bodies re-adopted the removed branch: {after_intake:?}"
    );
    assert!(
        at_fork_point(&r, &after_tick),
        "the periodic pending-sync loop re-adopted the removed branch: {after_tick:?}"
    );
    gossip(&mut r, &c1).await;
    assert_on(&r, &c1, F + 1, "canonical C1 after the held sync bodies").await;
}

// R-3b (Must) — Decision: a failure means a falsely removed block with no competing sibling does not return at once (INC-I-147 D4 recovery lost; the lab majority wedge).
#[tokio::test]
async fn inc_i_281_false_rollback_readopts_removed_block_at_once() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let d1 = r.dead[0].clone();
    // INC-I-281 M2.3: no competing sibling cached, so re-adopted at once (no hold window).
    gossip(&mut r, &d1).await;
    assert_on(&r, &d1, F + 1, "removed D1 at once, no sibling known").await;
    let d2 = r.dead[1].clone();
    gossip(&mut r, &d2).await;
    assert_on(&r, &d2, F + 2, "removed D2 at once, no sibling known").await;
}

// R-3b (Must) — Decision: a failure means a removed block is re-adopted outside fork choice after the window although a competing sibling is known, or the hold leaks into fork choice and blocks a walked branch.
#[tokio::test]
async fn inc_i_281_removed_block_held_while_sibling_known_after_window() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    // INC-I-281 M2.3: the cached sibling holds D1 only because it would win fork choice.
    let c1 = r.canon[0].clone();
    cache_sibling(&r, &c1, F + 1, HEAVY).await;
    tokio::time::sleep(past_window(&r)).await;
    let d1 = r.dead[0].clone();
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    probe_reapply("sibling_known", !at_fork_point(&r, &after));
    assert!(
        at_fork_point(&r, &after),
        "removed D1 re-adopted after the window while its sibling C1 is cached: {after:?}"
    );
    let walk = CompletedRecovery {
        blocks: r.dead[..2].to_vec(),
        connection_point: r.f_hash,
    };
    r.node
        .handle_completed_fork_recovery(walk)
        .await
        .expect("walk result: Ok");
    assert_on(
        &r,
        &r.dead[1],
        F + 2,
        "fork choice adopts the removed branch",
    )
    .await;
}

// R-3b (Must) — Decision: a failure means the fork-cache drain applies a removed child whose sibling is known, or drops it instead of putting it back.
#[tokio::test]
async fn inc_i_281_cached_drain_holds_removed_child() {
    let mut r = rig().await;
    roll_back_to_f(&mut r).await;
    let (d1, d2) = (r.dead[0].clone(), r.dead[1].clone());
    // Higher slot than D2, so the drain offers D2 first.
    let d2b = child_of(&r, &d1, F + 2, 208);
    seed_cache(&r.node, std::slice::from_ref(&d2)).await;
    // INC-I-281 M2.3: D2 is held only because the cached D2b would win fork choice.
    cache_sibling(&r, &d2b, F + 2, HEAVY).await;
    tokio::time::sleep(past_window(&r)).await;
    gossip(&mut r, &d1).await;
    let after = obs(&r).await;
    let cached = r
        .node
        .fork_block_cache
        .read()
        .await
        .contains_key(&d2.hash());
    // State, not log capture: tracing callsite interest races across parallel tests.
    let hold_reported = !r
        .node
        .sync_manager
        .write()
        .await
        .note_removed_block_held(&d2.hash());
    probe_reapply("drain_sibling_known", after.at_f2 == Some(d2.hash()));
    assert_eq!(
        after.at_f1,
        Some(d1.hash()),
        "D1 has no known sibling: applied"
    );
    assert!(hold_reported, "the drain never offered D2 to the hold");
    assert_ne!(
        after.at_f2,
        Some(d2.hash()),
        "the drain applied the removed D2: {after:?}"
    );
    assert!(cached, "the held D2 was dropped from the fork cache");
    assert_on(&r, &d2b, F + 2, "the drain moves on to the next sibling").await;
}
