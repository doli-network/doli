//! INC-I-156 / M1 / AUDIT-FIX — the two BLOCKING findings of the 5-auditor security sweep
//! on the M1 diff (`docs/.workflow/security-audit-report-M1.md`, `AUDIT GATE: BLOCK`).
//!
//! Both findings share ONE reachable input class, so they share one fixture:
//! a canonical **height index entry whose block body is unreadable**, below the reorg target,
//! on a node driven through `execute_reorg`'s legacy (no-undo) rebuild-from-genesis.
//!
//! * **AUDIT-P1-002** — the replay's fetch at `block_handling.rs:815-817` is
//!   `get_block_by_height(h).ok().flatten()`, which maps BOTH `Err` and `Ok(None)` to a silent
//!   skip and then **continues the loop**. Post-M1 (`clear()` is real) that permanently deletes
//!   that height's outputs from the freshly-emptied durable set. The sibling replay at
//!   `rollback.rs:210-219` and the ProducerSet replay at `rewards.rs:1115-1124` both fail
//!   CLOSED over the identical range — only this loop fails open.
//! * **AUDIT-P1-001** — the now-real `clear()` commits durably BEFORE that multi-minute,
//!   abortable replay, and **nothing at startup detects the truncated ledger**
//!   (synthesizer-verified: zero `utxo_len() == 0` guards across `bins/` and `crates/`).
//!   The fleet's own watchdog (`scripts/doli-watchdog.sh:21-25`, 5s timeout on a 2-minute
//!   timer) restarts into that window with probability approaching 1.
//!
//! ## Why the body gap is the right injection for BOTH
//!
//! It is the ONLY way to abort a legacy rebuild mid-replay without killing the process, and it
//! is production-constructible, not synthetic: `seed_canonical_index`
//! (`block_store/writes.rs:230-244`) writes `height_index` + `hash_to_height` and **no header,
//! no body** — called from `fork_recovery.rs:377`, `init.rs:424`, `:461`. `recover_body_gaps`
//! (`init.rs:56-90`) exists solely because "header-first sync can leave gaps" and scans only
//! the last 100 heights, while both replays iterate `1..=target_height`.
//!
//! The FORK_GUARD pre-flight at `block_handling.rs:599` cannot refuse it: `ensure_blocks_present`
//! (`block_store/queries.rs:193-209`) checks the height INDEX and never deserializes a body.
//! `has_contiguous_bodies` (`:220-231`) is ALSO index-only despite its name — the audit
//! explicitly refutes substituting it (Contradictions #3). Both facts are asserted executably in
//! `assert_index_body_asymmetry` below, so this file re-proves them rather than citing them.
//!
//! ## Implementation surface this file constrains
//!
//! covers: bins/node/src/node/block_handling.rs:815 (the fail-OPEN fetch — AUDIT-P1-002)
//! covers: bins/node/src/node/block_handling.rs:809 (durable wipe with no marker — AUDIT-P1-001)
//! covers: bins/node/src/node/rollback.rs:201       (the sibling wipe site — same marker)
//! covers: crates/storage/src/state_db/writes.rs    (set_/clear_rebuild_in_progress)
//! covers: crates/storage/src/state_db/queries.rs   (get_rebuild_in_progress)
//! covers: bins/node/src/node/state_snapshot_serve.rs (the GetStateSnapshot refusal)
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! OUTPUT CONTRACT: fn execute_reorg(&mut self, reorg_result: ReorgResult,
//!                                   triggering_block: Block) -> Result<()>
//!   (`bins/node/src/node/block_handling.rs:498`, `pub async`. Branch under test: the legacy
//!    no-undo rebuild at `:781-864`, reached when `has_undo` is false at `:658-659`.)
//!   plus fn Node::rebuild_halt_reason(&self) -> Option<String>
//!   and  fn Node::serve_state_snapshot(&self, Hash) -> SyncResponse.
//!
//! OUTPUTS — full enumeration (the M1 red tests already own O3..O9 of a COMPLETED reorg;
//! this file owns the ABORTED reorg, where the completion outputs are by definition not
//! produced, so only the outputs that exist on an abort are enumerated):
//!   O1: `execute_reorg`'s `Result<()>` — specifically the Err TEXT. This is the whole of
//!       AUDIT-P1-002's contract: which call site refused, and at which height.
//!   O2: the PERSISTENT `cf_utxo` / `cf_utxo_by_pubkey` content. Read back INDEPENDENTLY
//!       through `node.state_db` (Rule AQ-5) — `utxo.clear()` and the RocksDb replay both
//!       write straight through, so this is durable even though `atomic_replace` never runs.
//!   O3: `CF_META[rebuild_in_progress]` — the AUDIT-P1-001 marker. Survives the abort AND a
//!       process restart (`atomic_replace` never deletes CF_META keys, `writes.rs:181-186`).
//!   O4: `Node::rebuild_halt_reason()` — the shared refusal predicate read by the production
//!       gate (`production/mod.rs`) and the snapshot server.
//!   O5: `Node::serve_state_snapshot()`'s `SyncResponse` — must be `Error`, not `StateSnapshot`.
//!       This is "refuses to serve GetStateSnapshot" observed directly, not inferred.
//!   O6: `chain_state.best_height` after a restarted node is asked to produce — must not
//!       advance. (Non-discriminating on its own: the fixture node has no peers and may not
//!       have been scheduled anyway. Asserted so the enumeration is not silently incomplete.)
//!
//! PATHS through the replay loop at `block_handling.rs:816`:
//!   L1: every height in `1..=target` readable — the healthy path. Owned by
//!       `inc_i_156_m1_reorg_clear_leak.rs`; NOT re-tested here.
//!   L2: some height's body is unreadable (`Ok(None)`) — **THE PATH UNDER TEST.**
//!   L3: some height's read raises `Err` (I/O, corrupt SST, `deserialize_body` failure) —
//!       same `.ok().flatten()` collapse, same remediation, not separately constructible
//!       without a fault injector. Named as a known gap; L2 and L3 differ only in which of
//!       `get_block`'s arms produces the `None`.
//!
//! INPUT PARTITIONS:
//!   G1 gap at `GAP_HEIGHT` strictly BELOW `TARGET_HEIGHT`, dense elsewhere, RocksDb variant,
//!      undo pruned. Relationship asserted: the replay must STOP at the gap, and the error must
//!      name the reorg UTXO rebuild and the gap height.  **THE RED PARTITION for both tests.**
//!   G2 the same store observed through the two pre-flight guards
//!      (`ensure_blocks_present`, `has_contiguous_bodies`) — relationship: both ADMIT it, while
//!      `get_block_by_height` returns `Ok(None)`. This is the index-vs-body asymmetry, asserted
//!      as a PRECONDITION so neither guard can be what refuses these tests.
//!   G3 restart: a second `Node` opened on the SAME data dir after the aborted rebuild —
//!      relationship: the marker, the halt reason and the snapshot refusal all survive.
//!
//! MATRIX — 6 outputs × 3 partitions:
//!   G2: O1..O6 N/A (precondition only)          -> `assert_index_body_asymmetry` [precondition]
//!   G1: O1 Err naming "Reorg UTXO rebuild: missing block at height {GAP}"
//!       | O2 contains NO output created above the gap | O3 marker SET | O4 Some | O5 Error
//!       -> `inc_i_156_audit_p1_002_reorg_replay_must_fail_closed_on_unreadable_block` [RED]
//!       -> `inc_i_156_audit_p1_001_aborted_legacy_rebuild_arms_a_durable_halt` [RED]
//!   G3: O3 marker SET | O4 Some | O5 Error | O6 height not advanced
//!       -> `inc_i_156_audit_p1_001_halt_survives_restart_and_refuses_to_serve` [RED]
//!
//! PRE-FIX VERDICT — MEASURED on this branch, not predicted. Recorded in memory.db
//! (INC-I-156, run 493).

mod inc_i_156_m1_harness;

use crypto::{Hash, KeyPair};
use doli_core::Block;
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use network::protocols::SyncResponse;
use network::sync::ReorgResult;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Pre-reorg tip (`current_height` at block_handling.rs:566).
const CHAIN_LEN: u64 = 8;
/// `reorg_result.rollback.len()`.
const REORG_DEPTH: u64 = 2;
/// `target_height = current_height - rollback_count` (block_handling.rs:567).
const TARGET_HEIGHT: u64 = CHAIN_LEN - REORG_DEPTH;
/// The height whose body the replay cannot read. Strictly BELOW `TARGET_HEIGHT` so the
/// common-ancestor fetch at `block_handling.rs:624` is unaffected, and strictly ABOVE 1 so the
/// replay has already written real state before it reaches the gap.
const GAP_HEIGHT: u64 = 4;

const N_PRODUCERS: usize = 3;

// ==================== Fixture ====================

struct Fixture {
    /// Hash of the common ancestor (the block at `TARGET_HEIGHT`).
    ancestor_hash: Hash,
    /// Hashes of the blocks being rolled back, ascending.
    rollback_hashes: Vec<Hash>,
    /// The tip block, passed as `triggering_block` (unused — `new_blocks` is empty).
    tip_block: Block,
    /// Outpoints the gapped block created. Present in the canonical set before the reorg.
    gap_created: Vec<(storage::Outpoint, u64)>,
    /// Outpoints created at `TARGET_HEIGHT` — i.e. ABOVE the gap but still inside the replay
    /// range. Their presence after the abort is what proves the replay walked PAST a height it
    /// could not read instead of stopping.
    above_gap_created: Vec<(storage::Outpoint, u64)>,
    /// Producer keys, so the restart leg can rebuild a `Node` on the same data dir.
    producers: Vec<KeyPair>,
}

/// Build a dense `1..=CHAIN_LEN` chain on the PRODUCTION RocksDb UTXO backend, then punch a
/// body gap at `GAP_HEIGHT` by repointing its canonical index entry at a hash that has neither
/// a header nor a body — exactly the shape `seed_canonical_index` leaves behind.
async fn build_gapped_fixture() -> (Node, Fixture, TempDir) {
    let (mut node, producers, temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;

    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;
    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: the chain must reach CHAIN_LEN before the reorg"
    );

    let read_block = |height: u64| -> Block {
        node.block_store
            .get_block_by_height(height)
            .expect("fixture: block_store read")
            .unwrap_or_else(|| panic!("fixture: block at h={height} must exist before the gap"))
    };
    let gap_block = read_block(GAP_HEIGHT);
    let target_block = read_block(TARGET_HEIGHT);
    let tip_block = read_block(CHAIN_LEN);
    let rollback_hashes: Vec<Hash> = ((TARGET_HEIGHT + 1)..=CHAIN_LEN)
        .map(|hgt| read_block(hgt).hash())
        .collect();
    assert_eq!(
        rollback_hashes.len(),
        REORG_DEPTH as usize,
        "fixture: the rollback range must be exactly REORG_DEPTH blocks"
    );

    let gap_created = h::created_outpoints(&gap_block);
    let above_gap_created = h::created_outpoints(&target_block);
    {
        let utxo = node.utxo_set.read().await;
        for (op, _) in gap_created.iter().chain(above_gap_created.iter()) {
            assert!(
                utxo.contains(op),
                "fixture: every coinbase outpoint of h={GAP_HEIGHT} and h={TARGET_HEIGHT} must \
                 be live before the reorg — they are what the replay has to reproduce"
            );
        }
    }

    // ---- The NOT-TAKEN arm of the snapshot refusal, on a provably healthy node. ----
    // Doubles as the regression lock on the `serve_state_snapshot` extraction: the body was
    // moved verbatim out of `handle_sync_request`, so a healthy node must still answer with a
    // real snapshot, not an Error.
    {
        let best_hash = node.chain_state.read().await.best_hash;
        let response = node.serve_state_snapshot(best_hash).await;
        assert!(
            matches!(response, SyncResponse::StateSnapshot { .. }),
            "fixture: with no rebuild marker set, GetStateSnapshot must be SERVED — got {}",
            response.type_name()
        );
    }

    // ---- Erase the undo log so the legacy branch at block_handling.rs:781 is taken. ----
    node.state_db.prune_undo_above(0);
    for hgt in (TARGET_HEIGHT + 1)..=CHAIN_LEN {
        assert!(
            node.state_db.get_undo(hgt).is_none(),
            "fixture: undo at h={hgt} must be absent so `has_undo` is false over the whole \
             rollback range (block_handling.rs:658-659)"
        );
    }

    // ---- Punch the body gap. ----
    let phantom = crypto::hash::hash(b"inc_i_156_audit_p1_002_phantom_block");
    node.block_store
        .seed_canonical_index(phantom, GAP_HEIGHT)
        .expect("fixture: seed_canonical_index must succeed");

    let ancestor_hash = target_block.hash();
    (
        node,
        Fixture {
            ancestor_hash,
            rollback_hashes,
            tip_block,
            gap_created,
            above_gap_created,
            producers,
        },
        temp,
    )
}

/// PRECONDITION + the audit's facts #1 and #3 as executable assertions: both pre-flight guards
/// ADMIT a store whose body the replay cannot read. If either guard refused, these tests would
/// be measuring the guard, not the fetch.
fn assert_index_body_asymmetry(node: &Node) {
    assert!(
        node.block_store
            .get_hash_by_height(GAP_HEIGHT)
            .expect("block_store read")
            .is_some(),
        "precondition: the height INDEX at h={GAP_HEIGHT} must be present"
    );
    assert!(
        node.block_store
            .get_block_by_height(GAP_HEIGHT)
            .expect("block_store read")
            .is_none(),
        "precondition: the BODY at h={GAP_HEIGHT} must be unreadable — this is the whole \
         index-vs-body asymmetry AUDIT-P1-002 rests on"
    );
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .expect(
            "precondition: the FORK_GUARD pre-flight at block_handling.rs:599 must ADMIT this \
             store — it is index-only (block_store/queries.rs:193-209), so it cannot be what \
             refuses this test",
        );
    assert!(
        node.block_store.has_contiguous_bodies(1, TARGET_HEIGHT),
        "precondition: `has_contiguous_bodies` must ALSO admit this store. Despite its name and \
         doc comment it calls get_hash_by_height (block_store/queries.rs:220-231) — substituting \
         it for the fail-closed fetch would ship a no-op fix (audit Contradictions #3)."
    );
}

fn reorg_result(fx: &Fixture) -> ReorgResult {
    ReorgResult {
        rollback: fx.rollback_hashes.clone(),
        common_ancestor: fx.ancestor_hash,
        new_blocks: Vec::new(),
        weight_delta: 1,
    }
}

// ==================== AUDIT-P1-002 ====================

/// **AUDIT-P1-002 (P1, BLOCKING, 3/5 auditors + reviewer F1).**
///
/// The legacy reorg replay must FAIL CLOSED when it cannot read a block in `1..=target`, with
/// the shape already present at the sibling site `rollback.rs:210-219`. Pre-fix the loop
/// silently skips the height and walks on, so the freshly-emptied durable set is left missing
/// that height's outputs forever while heights ABOVE the gap are replayed on top — a durable
/// subset presented as a completed rebuild.
///
/// O1 is the primary assertion: the error must name the REORG UTXO REBUILD and the gap height.
/// Pre-fix the reorg does still return `Err`, but from the ProducerSet replay at
/// `rewards.rs:1115-1124` — i.e. AFTER the UTXO damage is durable, naming a missing block and
/// saying nothing about UTXO loss. An `is_err()` assertion alone therefore proves nothing here;
/// the TEXT is what discriminates.
#[tokio::test]
async fn inc_i_156_audit_p1_002_reorg_replay_must_fail_closed_on_unreadable_block() {
    let (mut node, fx, _temp) = build_gapped_fixture().await;
    assert_index_body_asymmetry(&node);

    let err = node
        .execute_reorg(reorg_result(&fx), fx.tip_block.clone())
        .await
        .expect_err(
            "the reorg cannot complete over a store whose body at h={GAP_HEIGHT} is unreadable — \
             if this returned Ok the fixture failed to reach the legacy branch",
        );
    let text = err.to_string();

    // ---- O1: WHICH site refused, and at which height. ----
    let expected = format!("Reorg UTXO rebuild: missing block at height {GAP_HEIGHT}");
    assert!(
        text.contains(&expected),
        "[AUDIT-P1-002] / O1: the reorg UTXO replay must fail CLOSED at the unreadable height \
         with `?` + ok_or_else, exactly as rollback.rs:210-219 already does. Expected the error \
         to contain {expected:?}; got {text:?}. Pre-fix this text is the ProducerSet rebuild's \
         (`rewards.rs:1115-1124`), which fires only AFTER the UTXO set has already been emptied \
         and partially replayed — the damage is durable by then."
    );

    // ---- O2: the replay must have STOPPED at the gap, not walked past it. ----
    let persisted = h::persisted_utxo_content(&node);
    for (op, amt) in &fx.above_gap_created {
        assert!(
            !persisted.contains(op),
            "[AUDIT-P1-002] / O2: outpoint {:.8}#{} ({amt} doli) was created at \
             h={TARGET_HEIGHT}, ABOVE the unreadable h={GAP_HEIGHT}. Finding it in the durable \
             set proves the replay SKIPPED the gap and continued — leaving cf_utxo a permanent \
             subset of canonical({TARGET_HEIGHT}) with h={GAP_HEIGHT}'s outputs deleted. \
             Persisted set: {}",
            op.tx_hash,
            op.index,
            h::describe(&persisted.pairs, 8)
        );
    }
    for (op, _) in &fx.gap_created {
        assert!(
            !persisted.contains(op),
            "[AUDIT-P1-002] / O2: the gapped height's own outputs must be absent on a fail-closed \
             abort too — the point of the fix is that the rebuild does not PRETEND to have \
             produced canonical({TARGET_HEIGHT}), not that it recovers the unreadable block."
        );
    }
}

// ==================== AUDIT-P1-001 ====================

/// **AUDIT-P1-001 (P1, BLOCKING, 5/5 auditors).**
///
/// The now-real `clear()` commits durably before an abortable replay. When that replay does not
/// finish, the node must be left holding a DURABLE, self-describing halt marker instead of
/// silently rebooting onto a destroyed ledger.
///
/// This test observes the marker in-process, immediately after the abort. The restart leg is
/// the next test.
#[tokio::test]
async fn inc_i_156_audit_p1_001_aborted_legacy_rebuild_arms_a_durable_halt() {
    let (mut node, fx, _temp) = build_gapped_fixture().await;
    assert_index_body_asymmetry(&node);

    let _ = node
        .execute_reorg(reorg_result(&fx), fx.tip_block.clone())
        .await
        .expect_err("fixture: the rebuild must abort mid-replay for this test to mean anything");

    // ---- O3: the durable marker. ----
    let marker = node.state_db.get_rebuild_in_progress().unwrap_or_else(|| {
        panic!(
            "[AUDIT-P1-001] / O3: CF_META[rebuild_in_progress] must be SET. `utxo.clear()` \
             committed a full deletion of cf_utxo durably (state_db/writes.rs:99) and the replay \
             that was supposed to reconstitute it did not finish. With no marker, a restart \
             boots at the pre-reorg tip against a destroyed ledger and nothing detects it — \
             verified by the synthesizer as zero `utxo_len() == 0` guards anywhere in bins/ or \
             crates/."
        )
    });
    assert_eq!(
        marker.0, TARGET_HEIGHT,
        "[AUDIT-P1-001] / O3: the marker must carry the rebuild's target height so the operator \
         message names what the node was trying to reconstruct"
    );
    assert!(
        marker.1 > 0,
        "[AUDIT-P1-001] / O3: the marker must carry a wall-clock start timestamp"
    );

    // ---- O4 + O5 ----
    assert!(
        node.rebuild_halt_reason().is_some(),
        "[AUDIT-P1-001] / O4: the shared refusal predicate must report the halt while the marker \
         is set — it is what the production gate and the snapshot server both read"
    );
    assert_snapshot_refused(&node, "post-abort").await;
}

/// **AUDIT-P1-001, restart leg.** The marker's entire value is that it survives the process
/// death the fleet's own watchdog causes: `scripts/doli-watchdog.sh` polls `getChainInfo` with a
/// 5s timeout on a 2-minute timer and runs `systemctl restart` on failure, while the rebuild
/// holds the `chain_state` and `utxo_set` write guards that `getChainInfo` needs
/// (`rpc/src/methods/network.rs:47,51`) across the whole `1..=target` loop.
///
/// The abort is simulated by the body gap rather than by killing the process; what is exercised
/// here is the part that matters — a SECOND `Node` opened on the SAME data dir must still
/// refuse.
#[tokio::test]
async fn inc_i_156_audit_p1_001_halt_survives_restart_and_refuses_to_serve() {
    let (mut node, fx, temp) = build_gapped_fixture().await;
    let producers = fx.producers.clone();
    let data_dir = temp.path().to_path_buf();

    let _ = node
        .execute_reorg(reorg_result(&fx), fx.tip_block.clone())
        .await
        .expect_err("fixture: the rebuild must abort mid-replay");

    // Close every RocksDB handle — this IS the restart.
    drop(node);

    let restarted = Node::new_for_test(data_dir, producers)
        .await
        .expect("restart: reopening the node on the same data dir must succeed");

    // ---- O3 ----
    assert!(
        restarted.state_db.get_rebuild_in_progress().is_some(),
        "[AUDIT-P1-001] / O3: the marker must SURVIVE the restart. It lives in CF_META, which \
         `atomic_replace` deliberately does not iterate-delete (state_db/writes.rs:181-186), and \
         `atomic_replace` never ran anyway — so the only way it can be gone is if it was never \
         written before the destructive `clear()`."
    );

    // ---- O4 ----
    let reason = restarted.rebuild_halt_reason().unwrap_or_else(|| {
        panic!(
            "[AUDIT-P1-001] / O4: a restarted node holding the marker must report a halt reason. \
             `Restart=always` otherwise returns the corrupted node straight to the fleet."
        )
    });
    assert!(
        reason.contains("STATE_CORRUPT"),
        "[AUDIT-P1-001] / O4: the reason must be tagged [STATE_CORRUPT] so it is greppable in \
         the log; got {reason:?}"
    );
    assert!(
        reason.to_lowercase().contains("resync"),
        "[AUDIT-P1-001] / O4: the reason must NAME THE REMEDY (resync) — converting silent \
         permanent destruction into a loud, self-describing halt is the entire difference \
         between P1 and P2 here; got {reason:?}"
    );

    // ---- O5 ----
    assert_snapshot_refused(&restarted, "post-restart").await;

    // ---- O6 ----
    let mut restarted = restarted;
    let height_before = restarted.chain_state.read().await.best_height;
    restarted
        .try_produce_block()
        .await
        .expect("try_produce_block must not error — it refuses quietly, like the other gates");
    assert_eq!(
        restarted.chain_state.read().await.best_height,
        height_before,
        "[AUDIT-P1-001] / O6: a node holding the halt marker must not extend the chain"
    );
}

/// O5 shared assertion: `GetStateSnapshot` must be refused, not served. A corrupted node that
/// still answers this hands its truncated set to every bootstrapping peer.
async fn assert_snapshot_refused(node: &Node, scenario: &str) {
    let best_hash = node.chain_state.read().await.best_hash;
    match node.serve_state_snapshot(best_hash).await {
        SyncResponse::Error(msg) => assert!(
            msg.contains("STATE_CORRUPT"),
            "[{scenario}] / O5: the refusal must be tagged [STATE_CORRUPT] so a peer's log names \
             the real cause; got {msg:?}"
        ),
        other => panic!(
            "[{scenario}] / O5: GetStateSnapshot must be REFUSED while the rebuild marker is set. \
             Serving it hands a truncated UTXO set to every bootstrapping peer, and nothing \
             downstream can detect that: BlockHeader carries no state_root, so a wrong set is \
             never caught at block acceptance. Got {}.",
            other.type_name()
        ),
    }
}
