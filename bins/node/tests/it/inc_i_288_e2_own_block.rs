//! INC-I-288 E-2(b): the behind side. After a coordinator rollback to h, the node
//! produces its OWN block at h+1 on its stale branch. Gate 1 of the coordinator
//! (`applied_since_rollback()` + orphan/empty-headers evidence -> HeaderFirstSync)
//! reads that block as "reconnected to canonical", so it returns HeaderFirstSync on
//! every tick, before the cooldown and before Rule 1b, while no peer can extend the
//! tip. The node waits out the 300 s rollback TTL before it walks down.
//! Contract at the gate (devnet 0): the node's own block at h+1 does not open Gate 1;
//! a block a peer delivered at h+1 (pending-sync apply or gossip) still does, so the
//! fix keeps Gate 1 for real progress.
//! Own-block signal: the own cell runs the production path's local steps
//! (production/mod.rs: `apply_block(Light)` of a block whose producer is the node's
//! `producer_key`, then `last_produced_slot`). The pending-sync apply
//! (periodic.rs) makes the SAME `apply_block(Light)` call for a peer's block, so a
//! fix keys on the producer key or on `last_produced_slot`, not on the call site.
//! Below-gate behavior: `inc_i_288_e2_below_ah.rs`.
//!
//! OUTPUT CONTRACT: fn SyncManager::classify_and_dispatch(shallow_rollback_count)
//!   -> (RecoveryAction, Option<RecoveryContext>), the coordinator call of the node
//!   tick (periodic.rs), after the tick's ShallowRollback { depth: 1 } and one apply
//!   at h+1, inside the 30 s cooldown (so only Gate 1 can return an action)
//!   O1: return action — Gate 1 HeaderFirstSync, or the Gate 2 hold / a walk-down
//!   O2: return ctx — gap > 0 (a peer reports a taller chain)
//!   O3: node tip after the apply (chain_state, index, persisted, sync tip)
//!   O4: coordinator last_action (record_action) — N/A as its own cell: it is O1
//! PATHS (one per apply origin; parent A1, height F+2, evidence and window equal):
//!   P1: own block (production apply, producer = own key, last_produced_slot)
//!   P2: peer block by the pending-sync apply (periodic.rs apply_block(Light))
//!   P3: peer block by gossip (handle_new_block -> apply_peer_body + peer mark)
//! INPUT PARTITIONS: the apply origin is the only input that differs; one per path.
//! MATRIX: P1 O1-O3 · P2 O1-O3 · P3 O1-O3 (O4 N/A)
//!
//! covers: recovery.rs (Gate 1, applied_since_rollback) rollback_memory.rs (gate1 height)
//! covers: block_lifecycle.rs (classify_and_dispatch, block_applied_with_weight)
//! covers: apply_block (mod.rs, own producer key) production (mod.rs) periodic.rs
//! covers: block_handling.rs (peer mark) types.rs init.rs (SyncConfig gate)

use network::RecoveryAction;

use super::inc_i_288_e2_common::*;

// E-2(b) INC-I-288 — Decision: a failure means the node's own unbroadcast block at h+1 still opens Gate 1, so a forked node loops HeaderFirstSync until the 300 s rollback TTL instead of walking down (the measured 46.8-slot hold).
#[tokio::test]
async fn e2_b_inc_i_288_own_block_does_not_open_gate1() {
    let (mut r, peer) = after_coordinator_rollback().await;
    let own = block_on_a1(&r, true);
    apply_as_produced(&mut r, &own).await;
    assert_on(&r, &own, F + 2, "precondition: own block is the tip").await;
    empty_headers(&r, peer, 2).await;

    let (action, gap) = dispatch(&mut r).await;

    assert!(
        matches!(
            action,
            RecoveryAction::None | RecoveryAction::ShallowRollback { depth: 1 }
        ),
        "E-2(b): after a coordinator rollback the node's OWN block at h+1 opened Gate 1 \
         (action={action:?}, gap={gap}); inside the cooldown a forked node must hold or \
         walk down, not loop HeaderFirstSync"
    );
    assert!(gap > 0, "E-2(b): a peer reports a taller chain (gap {gap})");
}

// E-2(b) INC-I-288 — Decision: a failure means the fix also closed Gate 1 for a peer block applied by the pending-sync path, so a node that reconnected after a rollback is rolled back again (the rollback cascade Gate 1 exists to stop).
#[tokio::test]
async fn e2_b_inc_i_288_control_synced_peer_block_opens_gate1() {
    let (mut r, peer) = after_coordinator_rollback().await;
    let theirs = block_on_a1(&r, false);
    apply_as_synced(&mut r, &theirs).await;
    assert_on(
        &r,
        &theirs,
        F + 2,
        "precondition: synced peer block is the tip",
    )
    .await;
    empty_headers(&r, peer, 2).await;

    let (action, gap) = dispatch(&mut r).await;

    assert_eq!(
        action,
        RecoveryAction::HeaderFirstSync,
        "E-2(b) control: a synced peer block at h+1 is progress; Gate 1 must still \
         return HeaderFirstSync (gap={gap})"
    );
    assert!(gap > 0, "E-2(b) control: a peer reports a taller chain");
}

// E-2(b) INC-I-288 — Decision: a failure means the fix also closed Gate 1 for a gossip block from a peer, so peer-confirmed progress after a rollback no longer stops a second rollback.
#[tokio::test]
async fn e2_b_inc_i_288_control_gossip_peer_block_opens_gate1() {
    let (mut r, peer) = after_coordinator_rollback().await;
    let theirs = block_on_a1(&r, false);
    deliver(&mut r, &theirs, peer).await;
    assert_on(
        &r,
        &theirs,
        F + 2,
        "precondition: gossip peer block is the tip",
    )
    .await;
    empty_headers(&r, peer, 2).await;

    let (action, gap) = dispatch(&mut r).await;

    assert_eq!(
        action,
        RecoveryAction::HeaderFirstSync,
        "E-2(b) control: a gossip peer block at h+1 is progress; Gate 1 must still \
         return HeaderFirstSync (gap={gap})"
    );
    assert!(gap > 0, "E-2(b) control: a peer reports a taller chain");
}
