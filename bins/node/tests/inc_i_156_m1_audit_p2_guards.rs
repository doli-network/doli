//! INC-I-156 / M1 / AUDIT-FIX-2 — the two node-side residual findings of the second
//! 5-auditor sweep (`docs/.workflow/security-audit-report-M1-iter2.md`, `AUDIT GATE: PASS`).
//!
//! Both are properties of the SAME mechanism the first fix added — the durable
//! `CF_META[rebuild_in_progress]` halt — so they share one fixture: a healthy node holding a
//! snapshot of its own state, plus the marker armed by hand.
//!
//! * **AUDIT-P2-101** (4/5 + 1 signal) — `apply_snap_snapshot` installs a complete,
//!   root-re-verified state through `atomic_replace` (`fork_recovery.rs:348-364`) and never
//!   clears the marker. `atomic_replace` deliberately excludes `CF_META` from its
//!   `deletable_cfs` (`state_db/writes.rs:216-221`, "Fix #10"), so the halt SURVIVES the very
//!   operation that repairs the ledger. The crypto auditor's sequence makes this the likely
//!   AUTOMATIC recovery, not an edge case: an emptied `cf_utxo` at a live `chain_state` height
//!   fails every subsequent `apply_block` on missing inputs, which is exactly the stuck-fork
//!   condition that escalates to snap sync. A node that has verifiably self-healed would stay
//!   production-halted and keep refusing `GetStateSnapshot` until an operator wiped its disk.
//! * **AUDIT-P3-101** (4/5) — `serve_state_root` is not gated by the halt, so a halted node
//!   still publishes a root computed over its TRUNCATED set into the snap-sync quorum
//!   (`snap_sync.rs:83-103`). The node is barred from serving the snapshot but not from voting
//!   on which root the snapshot must match. Bounded to stall/quorum dilution — never
//!   accepted-bad-state, because the downloaded snapshot is independently root-re-verified —
//!   which is why it is P3 and not P2.
//!
//! ## Implementation surface this file constrains
//!
//! covers: bins/node/src/node/fork_recovery.rs:352-364   (the snap-install Ok(()) arm)
//! covers: bins/node/src/node/state_root_serve.rs:33      (the GetStateRoot refusal)
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! OUTPUT CONTRACT: fn apply_snap_snapshot(&mut self, snapshot: network::VerifiedSnapshot)
//!                    -> Result<()>   (`fork_recovery.rs:268`, `pub async`)
//!   plus fn Node::serve_state_root(&self) -> SyncResponse (`state_root_serve.rs:33`).
//!
//! OUTPUTS — full enumeration:
//!   O1: `CF_META[rebuild_in_progress]` read back through `state_db.get_rebuild_in_progress()`.
//!       The durable half — this is what a restart sees.
//!   O2: `Node::rebuild_halt_reason()` — the shared refusal predicate. The in-process half;
//!       enumerated separately from O1 because it is a distinct function and a fix that
//!       cleared the key without the predicate agreeing would be invisible to O1 alone.
//!   O3: `Node::serve_state_snapshot()`'s `SyncResponse` — the service the halt withholds.
//!       Must return to `StateSnapshot` once the ledger is repaired.
//!   O4: `Node::serve_state_root()`'s `SyncResponse` — `StateRoot` when healthy, `Error`
//!       tagged `[STATE_CORRUPT]` while halted. **AUDIT-P3-101's whole contract.**
//!   O5: `apply_snap_snapshot`'s `Result<()>`. Asserted so the enumeration is complete; it is
//!       NOT discriminating for either finding — the function returns `Ok(())` on the reject
//!       path too (`fork_recovery.rs:293`, `:302`, `:323`), which is precisely why O1 rather
//!       than O5 is what separates the install arm from the reject arm.
//!
//! PATHS through `apply_snap_snapshot`:
//!   S1: root re-verification PASSES and `atomic_replace` returns `Ok(())` — the install arm.
//!       **THE PATH UNDER TEST for AUDIT-P2-101.**
//!   S2: root re-verification FAILS (`:296-303`) — nothing is installed, early `Ok(())`.
//!       **Tested as the NOT-TAKEN arm**: the halt must SURVIVE. This is what makes the fix a
//!       disarm-on-repair rather than a disarm-on-attempt.
//!   S3: `atomic_replace` itself returns `Err` (`:365-367`) — no state installed, halt must
//!       survive. Not constructible without a fault injector (it needs a RocksDB write
//!       failure); named as a known gap. S2 and S3 share the "nothing was installed"
//!       relationship, and S2 covers it executably.
//!   S4: `recovery_mode` gate at `:270-273` — early `Ok(())` before any of this. Out of scope;
//!       the fixture leaves `recovery_mode` false, asserted below.
//!
//! PATHS through `serve_state_root`:
//!   R1: memo HIT (`state_root_serve.rs:37-46`) — returns the cached tuple WITHOUT touching
//!       the UTXO set. **The path the AUDIT-P3-101 test drives**, deliberately: a gate placed
//!       after the fast path would leave a halted node still serving the root it memoized
//!       while it was healthy. Warming the memo first is what makes this test discriminating.
//!   R2: memo COLD/STALE — recompute under read locks. Covered by the healthy control below
//!       (first call on a fresh node is cold).
//!   R3: `compute_state_root` returns `Err` — `SyncResponse::Error("State root error: …")`,
//!       not memoized. Untouched by this change; not re-tested here.
//!
//! INPUT PARTITIONS:
//!   P1 healthy node, marker ABSENT — relationship: snapshot SERVED, root SERVED, no halt.
//!      The NOT-TAKEN arm of both guards. **Control.**
//!   P2 marker ARMED by hand on an otherwise healthy node — relationship: both services
//!      refused. **RED for AUDIT-P3-101** (`serve_state_root` still answers pre-fix).
//!   P3 marker ARMED, then a VALID self-snapshot installed — relationship: marker gone, both
//!      services restored. **RED for AUDIT-P2-101** (marker survives pre-fix).
//!   P4 marker ARMED, then a snapshot whose `state_root` does NOT match its bytes — the
//!      relationship is the INVERSE of P3: nothing installed, so the halt must persist.
//!
//! MATRIX — 5 outputs × 4 partitions:
//!   P1: O1 None | O2 None | O3 StateSnapshot | O4 StateRoot | O5 N/A
//!       -> `inc_i_156_audit_p2_101_healthy_node_serves_both_seams` [control]
//!   P2: O1 Some | O2 Some | O3 Error | O4 Error[STATE_CORRUPT] | O5 N/A
//!       -> `inc_i_156_audit_p3_101_halted_node_refuses_to_serve_state_root` [RED]
//!   P3: O1 None | O2 None | O3 StateSnapshot | O4 StateRoot | O5 Ok
//!       -> `inc_i_156_audit_p2_101_successful_snap_install_disarms_the_halt` [RED]
//!   P4: O1 Some (unchanged) | O2 Some | O3 Error | O4 Error | O5 Ok
//!       -> `inc_i_156_audit_p2_101_rejected_snapshot_must_not_disarm_the_halt` [not-taken arm]
//!
//! PRE-FIX VERDICT — MEASURED on this branch, not predicted. Recorded in memory.db
//! (INC-I-156, run 493).

mod inc_i_156_m1_harness;

use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use network::protocols::SyncResponse;
use network::VerifiedSnapshot;
use tempfile::TempDir;

/// Chain length before the snapshot is taken. Small — none of these assertions scale with it;
/// what matters is that the state is non-trivial (6 coinbases into the reward pool).
const CHAIN_LEN: u64 = 6;
const N_PRODUCERS: usize = 3;
/// The target height stamped into the hand-armed marker. Distinct from `CHAIN_LEN` so the
/// operator message provably carries the REBUILD's target, not the tip.
const MARKER_TARGET: u64 = 4;

// ==================== Fixture ====================

/// A node on the PRODUCTION RocksDb UTXO backend at `CHAIN_LEN`, plus a `VerifiedSnapshot` of
/// its own state captured while it is provably healthy.
///
/// Self-snapshot rather than a second node: the install arm's precondition is a snapshot whose
/// re-computed root matches its envelope, and the node's own state is the only such snapshot
/// obtainable without standing up a second chain. It is also the operationally honest shape —
/// a real snap sync installs a state the node has verified to be complete, which is exactly the
/// property that makes disarming the halt correct.
async fn healthy_node_with_self_snapshot() -> (Node, VerifiedSnapshot, TempDir) {
    let (mut node, producers, temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert!(
        !node
            .recovery_mode
            .load(std::sync::atomic::Ordering::Relaxed),
        "fixture: recovery_mode must be false, otherwise apply_snap_snapshot returns at \
         fork_recovery.rs:270-273 and path S1 is never reached"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let snap = storage::StateSnapshot::create(&cs, &utxo, &ps)
            .expect("fixture: StateSnapshot::create over a healthy node must succeed");
        VerifiedSnapshot {
            block_hash: snap.block_hash,
            block_height: snap.block_height,
            chain_state: snap.chain_state_bytes,
            utxo_set: snap.utxo_set_bytes,
            producer_set: snap.producer_set_bytes,
            state_root: snap.state_root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            // Take the M7 fast path (fork_recovery.rs:410) so this fixture exercises the
            // install arm rather than the legacy epoch reconstruction.
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };
    assert_eq!(
        snapshot.block_height, CHAIN_LEN,
        "fixture: the self-snapshot must be taken at the tip"
    );

    (node, snapshot, temp)
}

/// Arm the halt by hand, and assert the arming actually took on all three observation points.
///
/// Hand-armed rather than driven through a real aborted rebuild: that path is already proven by
/// `inc_i_156_m1_audit_p1_guards.rs`, and re-deriving it here would couple these two findings to
/// the body-gap injection instead of to the marker, which is the thing under test.
async fn arm_halt(node: &Node) {
    node.state_db
        .set_rebuild_in_progress(MARKER_TARGET)
        .expect("fixture: arming the marker must succeed");
    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "fixture / O1: the marker must be armed before the assertions that depend on it"
    );
    assert!(
        node.rebuild_halt_reason().is_some(),
        "fixture / O2: the refusal predicate must agree with the durable marker"
    );
}

async fn assert_snapshot_served(node: &Node, scenario: &str) {
    let best_hash = node.chain_state.read().await.best_hash;
    let response = node.serve_state_snapshot(best_hash).await;
    assert!(
        matches!(response, SyncResponse::StateSnapshot { .. }),
        "[{scenario}] / O3: a node with no armed halt must SERVE GetStateSnapshot — got {}",
        response.type_name()
    );
}

async fn assert_snapshot_refused(node: &Node, scenario: &str) {
    let best_hash = node.chain_state.read().await.best_hash;
    let response = node.serve_state_snapshot(best_hash).await;
    match response {
        SyncResponse::Error(msg) => assert!(
            msg.contains("STATE_CORRUPT"),
            "[{scenario}] / O3: the refusal must stay tagged [STATE_CORRUPT]; got {msg:?}"
        ),
        other => panic!(
            "[{scenario}] / O3: GetStateSnapshot must be refused while the halt is armed — got {}",
            other.type_name()
        ),
    }
}

// ==================== AUDIT-P3-101 ====================

/// **AUDIT-P3-101 (P3, 4/5 auditors).**
///
/// `serve_state_root` must refuse while the halt is armed, with the same predicate and the same
/// refusal shape as `serve_state_snapshot` (`state_snapshot_serve.rs:51-54`).
///
/// The memo is deliberately WARMED first. `serve_state_root`'s fast path (`:37-46`) returns the
/// cached tuple keyed only on `best_hash`, and arming the halt does not move the tip — so a gate
/// placed anywhere after that fast path would let a halted node keep serving the root it
/// computed while it was still healthy. Driving the cold path instead would let a
/// wrongly-placed gate pass. R1 is therefore the discriminating path, not merely a convenient
/// one.
#[tokio::test]
async fn inc_i_156_audit_p3_101_halted_node_refuses_to_serve_state_root() {
    let (node, _snapshot, _temp) = healthy_node_with_self_snapshot().await;

    // ---- Warm the memo on a provably healthy node (path R2 -> memo populated). ----
    let warm = node.serve_state_root().await;
    let healthy_root = match warm {
        SyncResponse::StateRoot {
            block_hash,
            block_height,
            state_root,
        } => {
            assert_eq!(
                block_height, CHAIN_LEN,
                "precondition / O4: the healthy root must be reported at the tip"
            );
            (block_hash, state_root)
        }
        other => panic!(
            "precondition / O4: a healthy node must SERVE GetStateRoot — got {}",
            other.type_name()
        ),
    };

    arm_halt(&node).await;

    // The tip has not moved, so the memo is still a HIT for `healthy_root.0`. That is the
    // point: the pre-fix function answers from the memo without consulting anything.
    assert_eq!(
        node.chain_state.read().await.best_hash,
        healthy_root.0,
        "precondition / R1: arming the marker must not move the tip, otherwise the memo would \
         miss and this test would stop being a memo-path test"
    );

    // ---- O4: the whole contract. ----
    match node.serve_state_root().await {
        SyncResponse::Error(msg) => assert!(
            msg.contains("STATE_CORRUPT"),
            "[AUDIT-P3-101] / O4: the refusal must be tagged [STATE_CORRUPT] so a peer's log \
             names the real cause, exactly as the snapshot seam does; got {msg:?}"
        ),
        SyncResponse::StateRoot { state_root, .. } => panic!(
            "[AUDIT-P3-101] / O4: a node holding the rebuild halt must NOT publish a state \
             root. It served {state_root} — a root computed over a set the node itself has \
             declared truncated — into the snap-sync quorum tally (snap_sync.rs:83-103), while \
             being simultaneously barred from serving the snapshot that root is supposed to \
             describe. Note this call is a memo HIT: a gate placed after the fast path at \
             state_root_serve.rs:37-46 does NOT fix this."
        ),
        other => panic!(
            "[AUDIT-P3-101] / O4: expected SyncResponse::Error, got {}",
            other.type_name()
        ),
    }

    // O3 stays refused — the two seams must agree, that is the whole point of sharing
    // `rebuild_halt_reason()`.
    assert_snapshot_refused(&node, "AUDIT-P3-101 halted").await;
}

// ==================== AUDIT-P2-101 ====================

/// **AUDIT-P2-101 (P2, 4/5 auditors + 1 cross-perspective signal).**
///
/// A snap install that reaches `atomic_replace`'s `Ok(())` arm has replaced the ENTIRE durable
/// set with a state whose root was re-verified two steps earlier (`fork_recovery.rs:296-303`).
/// That genuinely REPAIRS a truncation rather than laundering one — which is exactly why the
/// two rebuild sites (`rollback.rs:329`, `block_handling.rs:951`) keep their CONDITIONAL disarm
/// and this one does not need it: an undo-based rollback reconstructs nothing and could
/// silently launder a halt raised by an earlier interrupted rebuild, while a root-verified
/// whole-set install cannot.
///
/// Pre-fix the marker survives, so the node stays production-halted and keeps refusing
/// `GetStateSnapshot` after it has verifiably healed — converting the automatic recovery into a
/// mandatory manual disk wipe, fleet-correlated because the trigger (a > `UNDO_KEEP_DEPTH`
/// reorg) is.
#[tokio::test]
async fn inc_i_156_audit_p2_101_successful_snap_install_disarms_the_halt() {
    let (mut node, snapshot, _temp) = healthy_node_with_self_snapshot().await;
    arm_halt(&node).await;
    assert_snapshot_refused(&node, "pre-install").await;

    // ---- O5: the install arm. ----
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("[AUDIT-P2-101] / O5: installing a self-consistent snapshot must succeed");

    // ---- O1: the durable half. ----
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "[AUDIT-P2-101] / O1: a COMPLETED snap install must DISARM CF_META[rebuild_in_progress]. \
         `atomic_replace` replaced every UTXO row with a snapshot whose state root was \
         re-verified at fork_recovery.rs:296-303, so the truncation the marker describes no \
         longer exists — but `atomic_replace` deliberately excludes CF_META from its \
         deletable_cfs (state_db/writes.rs:216-221), so the marker survives unless this arm \
         clears it explicitly. Left armed, the node stays production-halted and keeps refusing \
         GetStateSnapshot forever on a ledger that is in fact complete."
    );

    // ---- O2: the in-process half. ----
    assert!(
        node.rebuild_halt_reason().is_none(),
        "[AUDIT-P2-101] / O2: the refusal predicate must agree with the disarmed marker — the \
         production gate (production/mod.rs:42) reads this, not the raw key"
    );

    // ---- O3 + O4: both withheld services must come back. ----
    assert_snapshot_served(&node, "post-install").await;
    assert!(
        matches!(
            node.serve_state_root().await,
            SyncResponse::StateRoot { .. }
        ),
        "[AUDIT-P2-101] / O4: a repaired node must serve GetStateRoot again — a disarm that \
         left either seam refusing would only half-restore the node"
    );

    // The node must still be attached to state_db after the install (INV-SYNC-014): a disarm
    // is worthless if the set it re-authorizes is a detached InMemory copy.
    h::assert_utxo_invariants(&node, "AUDIT-P2-101 post-install").await;
}

/// **AUDIT-P2-101, the NOT-TAKEN arm (path S2).**
///
/// The disarm must be conditional on a SUCCESSFUL install, not on `apply_snap_snapshot` having
/// been called. A snapshot whose re-computed root does not match its envelope is rejected at
/// `fork_recovery.rs:296-303` — nothing is written, the truncation the marker describes is
/// still there, and the halt must persist. Note `apply_snap_snapshot` returns `Ok(())` on that
/// path, so O5 cannot distinguish it from a real install; O1 is what does.
#[tokio::test]
async fn inc_i_156_audit_p2_101_rejected_snapshot_must_not_disarm_the_halt() {
    let (mut node, mut snapshot, _temp) = healthy_node_with_self_snapshot().await;
    arm_halt(&node).await;

    // Break the envelope's root so step 1 refuses. The BYTES stay self-consistent, so the
    // rejection is provably the root check and not a deserialization failure.
    snapshot.state_root = crypto::hash::hash(b"inc_i_156_audit_p2_101_wrong_root");

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("fixture: the reject path returns Ok(()) after snap_fallback_to_normal");

    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "[AUDIT-P2-101] / O1 (S2): a REJECTED snapshot installed nothing, so the halt must \
         SURVIVE. A disarm placed at function scope instead of inside the atomic_replace Ok(()) \
         arm would clear the marker here and hand a still-truncated ledger back to the fleet — \
         the exact laundering the two rebuild sites' conditional disarm exists to prevent."
    );
    assert!(
        node.rebuild_halt_reason().is_some(),
        "[AUDIT-P2-101] / O2 (S2): the refusal predicate must still report the halt"
    );
    assert_snapshot_refused(&node, "post-reject").await;
}

// ==================== Control ====================

/// **P1 — the NOT-TAKEN arm of both guards on a provably healthy node.**
///
/// Doubles as the regression lock on the `serve_state_root` gate: adding a refusal at the top
/// of a memoize-on-compute function is exactly the kind of change that can accidentally poison
/// the healthy path, so the healthy path is asserted on BOTH the cold call and the memo hit.
#[tokio::test]
async fn inc_i_156_audit_p2_101_healthy_node_serves_both_seams() {
    let (node, _snapshot, _temp) = healthy_node_with_self_snapshot().await;

    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "[control] / O1: a node that never rebuilt must hold no marker"
    );
    assert!(
        node.rebuild_halt_reason().is_none(),
        "[control] / O2: no marker, no halt reason"
    );

    // Cold (R2) then warm (R1) — both must serve, and both must agree.
    let cold = node.serve_state_root().await;
    let warm = node.serve_state_root().await;
    match (cold, warm) {
        (
            SyncResponse::StateRoot {
                state_root: a,
                block_height: ha,
                ..
            },
            SyncResponse::StateRoot {
                state_root: b,
                block_height: hb,
                ..
            },
        ) => {
            assert_eq!(
                a, b,
                "[control] / O4: the memo hit must return the same root as the cold compute"
            );
            assert_eq!(
                (ha, hb),
                (CHAIN_LEN, CHAIN_LEN),
                "[control] / O4: at the tip"
            );
        }
        (c, w) => panic!(
            "[control] / O4: a healthy node must serve GetStateRoot on both the cold and the \
             memo path — got {} then {}",
            c.type_name(),
            w.type_name()
        ),
    }

    assert_snapshot_served(&node, "control").await;
}
