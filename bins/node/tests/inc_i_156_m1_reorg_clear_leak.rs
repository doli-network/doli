//! INC-I-156 / M1 / R1 — SECOND LEAK SITE: `execute_reorg`'s legacy no-undo rebuild
//! replays on top of an un-cleared UTXO set, leaking the whole rolled-back RANGE's outputs.
//!
//! Requirements: REQ-I156-004 (Must, THE red test), REQ-I156-007 (Must, happy-path lock),
//! REQ-I156-010 (Should, INV-SYNC-014). Milestone M1, run 493.
//! Analysis: `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md` §2 ("Falsifying call
//! site #2, NOT in the brief"), §5 REQ-I156-004, §6 (why M1 must include this site).
//!
//! Primary file: `inc_i_156_m1_rocksdb_clear_leak.rs` — same defect, `rollback_one_block`,
//! ONE block. Shared fixture: `inc_i_156_m1_harness/mod.rs`.
//!
//! ## Why this file exists separately from the rollback proof
//!
//! The analysis (§6) records that splitting `block_handling.rs:803` out of M1 would be
//! INCORRECT, not merely inconvenient: the `clear()` signature change forces that file to be
//! edited in the same commit anyway, so leaving its behaviour unfixed would mean knowingly
//! compiling a call site whose semantics just changed under it. One commit, both leak sites
//! — therefore one milestone, both red tests.
//!
//! The reorg site is also STRICTLY WORSE than the rollback site in two ways:
//!   * its residual spans the whole range `target+1..=current`, not one block;
//!   * its replay loop silently SKIPS absent blocks (`block_handling.rs:805-807` uses
//!     `.ok().flatten()`) where `rollback.rs:196-201` hard-errors. It relies entirely on the
//!     upstream density guard at `block_handling.rs:599`. The R1 fix does not change that
//!     reliance, but an honest `clear()` makes the consequence of a guard bypass worse
//!     (empty instead of stale) — so `assert_store_is_dense` below is not decoration, it is
//!     the assertion that the guard is confirmed intact (analysis §4.5).
//!
//! ## Implementation surface this file constrains
//!
//! covers: bins/node/src/node/block_handling.rs:803 (execute_reorg legacy no-undo rebuild — `utxo.clear()`, the second leak site)
//! covers: crates/storage/src/utxo/set.rs           (UtxoSet::clear — the RocksDb arm is the shared root cause)
//! covers: crates/storage/src/state_db/writes.rs    (StateDb::clear_utxos — the delegate)
//! covers: bins/node/src/node/rollback.rs:191       (the sibling leak site; proven in inc_i_156_m1_rocksdb_clear_leak.rs)
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! OUTPUT CONTRACT: fn execute_reorg(&mut self, reorg_result: ReorgResult,
//!                                   triggering_block: Block) -> Result<()>
//!   (`bins/node/src/node/block_handling.rs:498`, `pub async`. `target_height` is derived at
//!    `:567` as `current_height - reorg_result.rollback.len()`. Branch under test: the legacy
//!    no-undo rebuild at `:781-852`, reached when `has_undo` is false at `:658-659`.)
//!
//! OUTPUTS — full enumeration:
//!   O1: return value `Result<()>`. NOTE the discrimination problem this creates: the
//!       function returns `Ok(())` on many refusal paths (`:532`, `:551`, `:561`, `:938`),
//!       so an "is_ok" assertion cannot distinguish "reorg ran correctly" from "reorg bailed
//!       out". O3 is what proves the rollback actually executed.
//!   O2: `self.utxo_set` + the PERSISTENT `cf_utxo` / `cf_utxo_by_pubkey` column families.
//!       `utxo.clear()` at `:803`, replay at `:804-830`, `atomic_replace` at `:882-884`.
//!       **THE load-bearing output.** Read back INDEPENDENTLY through `node.state_db`
//!       (Rule AQ-5).
//!   O3: `self.chain_state.{best_height,best_hash,best_slot}` (`:798-802` on this branch).
//!   O4: `self.producer_set` (`:833-836`) + its persisted form via atomic_replace.
//!   O5: `block_store` canonical entries for EVERY height in `target+1..=current` —
//!       persistent store DELETEs (`:863-867`, INC-I-144). Range, not a single entry.
//!   O6: `self.sync_manager` finality marker (`:903-906`).
//!   O7: `StateDb.utxo_count` — INV-GUARD-001.
//!   O8: `self.utxo_set` VARIANT identity — INV-SYNC-014 / REQ-I156-010.
//!   O9: `self.mempool` — revalidated at `:960-965`. Empty in this fixture; named for
//!       completeness, and asserted empty so the enumeration is not silently incomplete.
//!
//! PATHS through the function:
//!   P0: `rollback_count == 0` (`:572`) — the whole rollback block is skipped.
//!   P1: missing fork block (`:532`) / broken fork chain (`:543`, `:555`) — early `Ok(())`.
//!   P2: holed store — `ensure_blocks_present(1, target)` fails at `:599` → `Err`
//!       `[FORK_GUARD_BACKFILL_REQUIRED]`, nothing mutated. Owned by
//!       `m_rc11_fork_guard_backfill_regression.rs`; asserted as a PRECONDITION here (the
//!       guard must ADMIT this store) so it cannot be what refuses this test.
//!   P3: undo-based reorg — `has_undo == true` (`:661`). `clear()` never called.
//!   P4: legacy no-undo rebuild — `has_undo == false` (`:781`). Calls `utxo.clear()` at
//!       `:803` then replays `1..=target_height`. **THE DEFECTIVE PATH.**
//!
//! INPUT PARTITIONS:
//!   P4a DENSE store, RocksDb variant, reorg depth 2, range creates outputs that survive
//!       past the range — relationship: `post == canonical(target)`. Pre-fix it is
//!       `canonical(target) ∪ {created in target+1..=current, unspent within that range}`.
//!       **THE RED PARTITION.** Depth ≥ 2 is required by REQ-I156-004 and is what makes
//!       this distinct from the rollback file: the residual is a RANGE, so a per-block fix
//!       at `rollback.rs` alone would leave this site broken.
//!   P4b same, but the range ALSO creates an output that it SPENDS internally (OUT_C:
//!       created at target+1, spent at target+2) — a distinct relationship: this output must
//!       be absent BOTH pre- and post-fix, so it is the control that shows the leak set is
//!       exactly "created AND unspent within the range" and not "everything the range
//!       touched". Without it, "all range-created outputs are gone" would be an untested
//!       over-claim.
//!   P4c same, and the range SPENDS an output created BEFORE the range (OUT_B at target) —
//!       that output must be RESTORED. A fix that merely deleted range-created outputs
//!       would pass P4a and fail P4c.
//!   P3a undo present over the whole range, DENSE store — `clear()` unreachable; the UTXO
//!       component must be byte-identical to `canonical(target)` pre- and post-fix.
//!       REQ-I156-007.
//!
//! MATRIX — 9 outputs × 4 partitions = 36 cells:
//!   P4a: O1 Ok(()) | O2 == canonical(target), range-created-unspent ABSENT | O3 h=TARGET,
//!        hash=ancestor | O4 serializable | O5 entries at target+1..=current all purged
//!        | O6 finality cleared | O7 INV-GUARD-001 | O8 is_rocksdb() | O9 mempool empty
//!        -> `inc_i_156_req004_legacy_reorg_must_not_leak_rolled_back_range_outputs` [RED]
//!   P4b: the internally-spent output is absent — same test.              [control, same fn]
//!   P4c: the pre-range output the range spent is RESTORED — same test.   [RED, same fn]
//!   P3a: O1 Ok(()) | O2 == canonical(target) byte-exact | O3..O9 as P4a
//!        -> `inc_i_156_req007_undo_based_reorg_utxo_state_unchanged`         [PASS-LOCK]
//!
//! ## `new_blocks` is deliberately EMPTY
//!
//! `execute_reorg` does rollback-then-apply in one function. If the fixture supplied fork
//! blocks, the post-call UTXO set would be `rollback_result ∪ applied(fork)` and no
//! assertion could attribute a difference to the rollback half — the observation would be
//! ambiguous, which is exactly the failure mode this protocol exists to prevent. With
//! `new_blocks: vec![]` the collect loop at `:514-535` and the apply loop at `:914-940` are
//! both no-ops, the chain-validation guards at `:542-563` are skipped (`first()` is `None`,
//! `1..0` is empty), and every output observed below is produced by the rollback half alone.
//! `rollback_count` still comes from `reorg_result.rollback.len()` at `:503`, so the reorg
//! DEPTH under test is a genuine 2.
//!
//! PRE-FIX VERDICT — MEASURED on this branch (base f4e6ea69), not predicted. See the
//! recorded failure text in memory.db (INC-I-156, run 493).

mod inc_i_156_m1_harness;

use crypto::Hash;
use doli_node::node::Node;
use inc_i_156_m1_harness as h;
use network::sync::ReorgResult;
use storage::Outpoint;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Pre-reorg tip (`current_height` at block_handling.rs:566).
const CHAIN_LEN: u64 = 8;
/// `reorg_result.rollback.len()` — REQ-I156-004 requires depth >= 2.
const REORG_DEPTH: u64 = 2;
/// `target_height = current_height - rollback_count` (block_handling.rs:567).
const TARGET_HEIGHT: u64 = CHAIN_LEN - REORG_DEPTH;

/// Height whose Transfer spends the synthetic funding UTXO and creates OUT_A.
const FUND_SPEND_AT: u64 = 3;

const FUNDING_AMOUNT: u64 = 100_000;
const OUT_A_AMOUNT: u64 = 99_000;
/// Created at TARGET_HEIGHT, SPENT inside the rolled-back range — must be RESTORED (P4c).
const OUT_B_AMOUNT: u64 = 98_000;
/// Created at TARGET+1 and SPENT at TARGET+2 — inside the range on both ends (P4b control).
const OUT_C_AMOUNT: u64 = 97_000;
/// Created at TARGET+2 (the tip) and never spent — part of the leak set (P4a).
const OUT_D_AMOUNT: u64 = 96_000;

// ==================== Fixture ====================

struct Fixture {
    /// Canonical UTXO set at `TARGET_HEIGHT`, captured through the REAL `apply_block` path
    /// before the rolled-back range existed. Ground truth, observed not predicted.
    canonical_at_target: h::UtxoContent,
    /// Outputs created in `target+1..=current` that are STILL UNSPENT at `current`. This is
    /// exactly the residual the analysis (§2) characterises, and it is never empty because
    /// every block carries a coinbase.
    range_created_unspent: Vec<(Outpoint, u64)>,
    /// OUT_C — created at `target+1`, spent at `target+2`. Absent both pre- and post-fix.
    range_created_then_spent: Outpoint,
    /// OUT_B — created at `target`, spent inside the range. Must be RESTORED.
    pre_range_spent: Outpoint,
    /// Hash of the common ancestor (the block at `TARGET_HEIGHT`).
    ancestor_hash: Hash,
    /// Hashes of the blocks being rolled back, in ascending height order.
    rollback_hashes: Vec<Hash>,
    /// The tip block, passed as `triggering_block` (unused when `new_blocks` is empty).
    tip_block: doli_core::Block,
}

/// Build the `execute_reorg` pre-reorg shape. Same production backend and same
/// spend-chain discipline as the rollback fixture; see
/// `inc_i_156_m1_harness::install_production_utxo_backend` for why the variant is
/// load-bearing, and the rollback file's `build_fixture` for why the synthetic funding UTXO
/// must be spent below `TARGET_HEIGHT`.
async fn build_fixture(n_producers: usize) -> (Node, Fixture, TempDir) {
    let (mut node, producers, temp) = h::make_node(n_producers).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;

    h::apply_plain_up_to(&mut node, &producers, FUND_SPEND_AT - 1, &params).await;

    let owner = crypto::KeyPair::generate();
    let owner_pkh = h::address_of(&owner);
    let funding = h::fund(
        &node,
        owner_pkh,
        FUNDING_AMOUNT,
        FUND_SPEND_AT - 1,
        b"inc_i_156_m1_reorg_funding",
    )
    .await;

    // h=3: funding -> OUT_A.
    let tx_a = h::signed_transfer(funding, OUT_A_AMOUNT, &owner, owner_pkh);
    let out_a = Outpoint::new(tx_a.hash(), 0);
    h::apply_block_with_transfer(&mut node, &producers, FUND_SPEND_AT, &params, tx_a).await;

    h::apply_plain_up_to(&mut node, &producers, TARGET_HEIGHT - 1, &params).await;

    // h=TARGET: OUT_A -> OUT_B. OUT_B is unspent at the common ancestor and is what the
    // rolled-back range consumes (partition P4c).
    let tx_b = h::signed_transfer(out_a, OUT_B_AMOUNT, &owner, owner_pkh);
    let out_b = Outpoint::new(tx_b.hash(), 0);
    h::apply_block_with_transfer(&mut node, &producers, TARGET_HEIGHT, &params, tx_b).await;

    // ---- GROUND TRUTH at the common ancestor. ----
    assert_eq!(
        node.chain_state.read().await.best_height,
        TARGET_HEIGHT,
        "fixture: the chain must pause at TARGET_HEIGHT to capture ground truth"
    );
    let canonical_at_target = h::utxo_content(&node).await;
    assert!(
        canonical_at_target.contains(&out_b),
        "fixture: OUT_B must be UNSPENT at TARGET_HEIGHT"
    );
    assert!(
        !canonical_at_target.contains(&funding),
        "fixture: the synthetic funding UTXO must be SPENT by TARGET_HEIGHT, otherwise it \
         would appear in canonical(TARGET) but never in a rebuild-from-genesis and every set \
         equality below would be a false failure"
    );
    let ancestor_hash = node.chain_state.read().await.best_hash;

    // ---- The rolled-back RANGE. ----
    // h=TARGET+1: OUT_B -> OUT_C. Consumes a pre-range output, creates an in-range one.
    let tx_c = h::signed_transfer(out_b, OUT_C_AMOUNT, &owner, owner_pkh);
    let out_c = Outpoint::new(tx_c.hash(), 0);
    let block_lo =
        h::apply_block_with_transfer(&mut node, &producers, TARGET_HEIGHT + 1, &params, tx_c).await;

    // h=TARGET+2 (= CHAIN_LEN): OUT_C -> OUT_D. Consumes an in-range output, creates the
    // in-range output that survives to the tip.
    let tx_d = h::signed_transfer(out_c, OUT_D_AMOUNT, &owner, owner_pkh);
    let block_hi =
        h::apply_block_with_transfer(&mut node, &producers, CHAIN_LEN, &params, tx_d).await;

    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: the chain must be at CHAIN_LEN before the reorg"
    );

    // The residual, computed from the blocks themselves rather than assumed: every outpoint
    // the range created, minus the one it spent internally.
    let range_created_unspent: Vec<(Outpoint, u64)> = h::created_outpoints(&block_lo)
        .into_iter()
        .chain(h::created_outpoints(&block_hi))
        .filter(|(op, _)| *op != out_c)
        .collect();
    assert_eq!(
        range_created_unspent.len(),
        3,
        "fixture: the range must leave exactly 3 unspent creations (coinbase@{}, \
         coinbase@{CHAIN_LEN}, OUT_D), got {}",
        TARGET_HEIGHT + 1,
        range_created_unspent.len()
    );
    {
        let utxo = node.utxo_set.read().await;
        for (op, amt) in &range_created_unspent {
            assert!(
                utxo.contains(op),
                "fixture: range-created outpoint {:.8}#{} ({amt} doli) must be live at the tip",
                op.tx_hash,
                op.index
            );
        }
        assert!(
            !utxo.contains(&out_c),
            "fixture: OUT_C must be created AND spent inside the range — it is the control \
             that shows the leak set is exactly 'created and unspent within the range'"
        );
        assert!(
            !utxo.contains(&out_b),
            "fixture: OUT_B must be spent by the range — the reorg has to RESTORE it"
        );
    }

    let rollback_hashes = vec![block_lo.hash(), block_hi.hash()];

    (
        node,
        Fixture {
            canonical_at_target,
            range_created_unspent,
            range_created_then_spent: out_c,
            pre_range_spent: out_b,
            ancestor_hash,
            rollback_hashes,
            tip_block: block_hi,
        },
        temp,
    )
}

/// Erase the undo log so `has_undo` at block_handling.rs:658-659 is FALSE over the whole
/// rollback range and the legacy branch at `:781` is taken. `prune_undo_above(0)` is the
/// production truncation API (`state_db/undo.rs:50`); a snap-synced node arrives in exactly
/// this shape.
fn force_legacy_branch(node: &Node) {
    node.state_db.prune_undo_above(0);
    for h in (TARGET_HEIGHT + 1)..=CHAIN_LEN {
        assert!(
            node.state_db.get_undo(h).is_none(),
            "fixture: undo at h={h} must be absent so `has_undo` is false over the whole \
             rollback range (block_handling.rs:658-659)"
        );
    }
}

/// The dense-store precondition — and the REQ-I156-004 / analysis §4.5 confirmation that the
/// FORK_GUARD density check at block_handling.rs:599 is intact and ADMITS this store, so it
/// cannot be what refuses this test. R1 is the dense-store residual.
fn assert_store_is_dense(node: &Node) {
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .unwrap_or_else(|e| {
            panic!(
                "precondition: the block store must be DENSE over 1..={TARGET_HEIGHT} so the \
                 FORK_GUARD check at block_handling.rs:599 admits this reorg — got {e}"
            )
        });
}

/// Build the `ReorgResult` that drives the legacy branch. `new_blocks` is empty on purpose —
/// see the "`new_blocks` is deliberately EMPTY" section of the module doc.
fn reorg_result(fx: &Fixture) -> ReorgResult {
    ReorgResult {
        rollback: fx.rollback_hashes.clone(),
        common_ancestor: fx.ancestor_hash,
        new_blocks: Vec::new(),
        weight_delta: 1, // execute_reorg only logs it
    }
}

/// Assert every non-O2 output of a COMPLETED reorg rollback.
async fn assert_reorg_completed(node: &Node, fx: &Fixture, scenario: &str) {
    // ---- O3 ----
    let (cs_height, cs_hash) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        cs_height, TARGET_HEIGHT,
        "[{scenario}] / O3: chain_state.best_height must be {TARGET_HEIGHT} (the common \
         ancestor) after the reorg. This is what proves the rollback actually RAN — \
         execute_reorg returns Ok(()) on several refusal paths too."
    );
    assert_eq!(
        cs_hash, fx.ancestor_hash,
        "[{scenario}] / O3: chain_state.best_hash must be the common ancestor"
    );

    // ---- O4 ----
    let producer_bytes = {
        let ps = node.producer_set.read().await;
        bincode::serialize(&*ps).expect("[O4] ProducerSet must serialize")
    };
    assert!(
        !producer_bytes.is_empty(),
        "[{scenario}] / O4: producer_set must still serialize after \
         rebuild_producer_set_from_blocks (block_handling.rs:833-836)"
    );

    // ---- O5 (INC-I-144): EVERY rewound height's canonical entry, not just the tip. ----
    for height in (TARGET_HEIGHT + 1)..=CHAIN_LEN {
        assert!(
            node.block_store
                .get_hash_by_height(height)
                .expect("[O5] block_store read")
                .is_none(),
            "[{scenario}] / O5: the canonical entry at h={height} must be purged by a \
             completed reorg (block_handling.rs:863-867). Unlike rollback_one_block this is a \
             RANGE delete, which is part of what makes this the wider-blast-radius site."
        );
    }

    // ---- O6 ----
    assert!(
        node.sync_manager.read().await.local_tip().0 >= TARGET_HEIGHT,
        "[{scenario}] / O6: sync_manager must remain in a consistent state after the reorg"
    );

    // ---- O9 ----
    assert_eq!(
        node.mempool.read().await.size(),
        0,
        "[{scenario}] / O9: the mempool is empty in this fixture and must stay empty after \
         revalidate (block_handling.rs:960-965)"
    );

    // ---- O7 + O8 ----
    h::assert_utxo_invariants(node, scenario).await;
}

// ==========================================================================
//  P4a + P4b + P4c — THE RED TEST (REQ-I156-004). Must FAIL pre-fix, on O2.
// ==========================================================================

/// Requirement: REQ-I156-004 (Must) — path P4, partitions P4a / P4b / P4c.
///
/// Acceptance criteria (analysis §5, REQ-I156-004):
///   * Same construction as REQ-I156-003 over `execute_reorg`'s legacy branch: `RocksDb`
///     variant, dense store, `has_undo == false` over the rollback range
///     (block_handling.rs:658-659), reorg depth >= 2.
///   * Pre-fix: outputs created in `target+1..=current` and unspent within that range
///     SURVIVE the reorg.
///   * Post-fix: they do not; the set equals the canonical set at `target_height`.
///
/// PRE-FIX (the RED signal): `utxo.clear()` at block_handling.rs:803 is a no-op on this
/// variant, so the replay of `1..=target_height` lands on top of the state at the tip. Every
/// output the RANGE created and did not itself spend survives — here the two coinbases plus
/// OUT_D — and `atomic_replace` at `:882-884` makes it durable.
#[tokio::test]
async fn inc_i_156_req004_legacy_reorg_must_not_leak_rolled_back_range_outputs() {
    let (mut node, fx, _tmp) = build_fixture(3).await;
    force_legacy_branch(&node);
    assert_store_is_dense(&node);

    let reorg = reorg_result(&fx);
    assert_eq!(
        reorg.rollback.len() as u64,
        REORG_DEPTH,
        "precondition: REQ-I156-004 requires a reorg depth of at least 2 — the residual must \
         be a RANGE, not a single block, so that a per-block fix cannot satisfy it"
    );

    // ---- Drive the REAL entry point. ----
    let result = node.execute_reorg(reorg, fx.tip_block.clone()).await;

    // ---- O1 ----
    result.unwrap_or_else(|e| {
        panic!(
            "REQ-I156-004 / O1: execute_reorg must not error on a DENSE store with the RocksDb \
             backend — got {e}. (An `[FORK_GUARD_BACKFILL_REQUIRED]` error here would mean the \
             density precondition above is wrong.)"
        )
    });

    // ---- O2, read back from the PERSISTENT store (AQ-5). THE assertions. ----
    let persisted = h::persisted_utxo_content(&node);
    let leaked: Vec<(Outpoint, u64)> = fx
        .range_created_unspent
        .iter()
        .filter(|(op, _)| persisted.contains(op))
        .copied()
        .collect();

    assert!(
        leaked.is_empty(),
        "REQ-I156-004 / O2 — THE LEAK (second call site): {} of {} outputs CREATED by the \
         rolled-back RANGE h={}..={CHAIN_LEN} and unspent within it are STILL PRESENT in \
         cf_utxo after reorging back to h={TARGET_HEIGHT}: [{}].\n\
         Mechanism: `utxo.clear()` at bins/node/src/node/block_handling.rs:803 dispatches to \
         the `UtxoSet::RocksDb(_)` arm of crates/storage/src/utxo/set.rs:71-76, which is an \
         EMPTY BLOCK. The replay at block_handling.rs:804-830 therefore runs ON TOP of the \
         state at the tip: it restores what the range SPENT but removes nothing it CREATED. \
         `atomic_replace` at block_handling.rs:882-884 makes the result durable. This is the \
         SAME root cause as rollback.rs:191 (see inc_i_156_m1_rocksdb_clear_leak.rs) but over \
         a RANGE rather than one block, so a per-call-site fix at rollback.rs alone would \
         leave this site broken — which is why the analysis §6 requires both in M1.\n\
         Fix: delegate the RocksDb arm to `StateDb::clear_utxos()` \
         (crates/storage/src/state_db/writes.rs:80-102) and return `Result<(), StorageError>` \
         so this call site must propagate it.\n\
         total_value: canonical({TARGET_HEIGHT}) = {} | after reorg = {} | inflation = {} || \
         utxo count: {} -> {}",
        leaked.len(),
        fx.range_created_unspent.len(),
        TARGET_HEIGHT + 1,
        h::describe(&leaked, 4),
        fx.canonical_at_target.total_value,
        persisted.total_value,
        persisted
            .total_value
            .saturating_sub(fx.canonical_at_target.total_value),
        fx.canonical_at_target.len,
        persisted.len,
    );

    // ---- O2 / P4c: the pre-range output the range SPENT must be back. ----
    assert!(
        persisted.contains(&fx.pre_range_spent),
        "REQ-I156-004 / O2 / P4c: OUT_B ({:.8}#{}) was created at h={TARGET_HEIGHT} and spent \
         inside the rolled-back range; the rebuild must RESTORE it. A fix that merely deleted \
         the range's created outputs instead of genuinely clearing and replaying would pass \
         the leak assertion and fail here.",
        fx.pre_range_spent.tx_hash,
        fx.pre_range_spent.index,
    );

    // ---- O2 / P4b: the CONTROL — created and spent inside the range, absent either way. ----
    assert!(
        !persisted.contains(&fx.range_created_then_spent),
        "REQ-I156-004 / O2 / P4b: OUT_C ({:.8}#{}) was created at h={} and spent at \
         h={CHAIN_LEN}, both inside the rolled-back range, so it must be absent from \
         canonical({TARGET_HEIGHT}). Its presence would mean the replay is resurrecting \
         outputs the range itself consumed — a different defect from the leak above.",
        fx.range_created_then_spent.tx_hash,
        fx.range_created_then_spent.index,
        TARGET_HEIGHT + 1,
    );

    // ---- O2: exact equality with the canonical set at the common ancestor. ----
    assert_eq!(
        persisted.total_value, fx.canonical_at_target.total_value,
        "REQ-I156-004 / O2: total supply after the reorg must EQUAL the canonical supply at \
         h={TARGET_HEIGHT}"
    );
    assert_eq!(
        persisted.pairs, fx.canonical_at_target.pairs,
        "REQ-I156-004 / O2: the persisted UTXO set must be EXACTLY the canonical set at \
         h={TARGET_HEIGHT} — same outpoints, same amounts. {} entries expected, {} found.",
        fx.canonical_at_target.len, persisted.len,
    );
    assert!(
        persisted.canonical == fx.canonical_at_target.canonical,
        "REQ-I156-004 / O2: `serialize_canonical()` must be byte-identical to \
         canonical(h={TARGET_HEIGHT}) — a difference means the node's state root diverges from \
         every peer that did not take this path"
    );

    // ---- O3..O9 ----
    assert_reorg_completed(&node, &fx, "P4a/P4b/P4c REQ-I156-004").await;
}

// ==========================================================================
//  P3a — REQ-I156-007 happy-path REGRESSION LOCK. Passes pre- AND post-fix.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — path P3, partition P3a. NOT a red test.
///
/// Acceptance: "Dense-store reorg with undo data: state root identical pre- and post-fix."
///
/// The undo branch (block_handling.rs:661-780) never calls `clear()`, so the M1 fix must not
/// move its result. The assertion is on the UTXO component of the state root — byte-exact
/// against `canonical(TARGET_HEIGHT)` — because that is the one and only component the M1
/// change can touch. (The full state root is NOT asserted here for the reason documented on
/// `inc_i_156_req007_undo_based_rollback_state_root_unchanged` in the sibling file: a
/// pre-existing `total_work` drift, orthogonal to INC-I-156, is pinned there rather than
/// re-pinned in every file.)
///
/// This is the guard against a fix that makes `clear()` honest but perturbs the common
/// reorg path.
#[tokio::test]
async fn inc_i_156_req007_undo_based_reorg_utxo_state_unchanged() {
    let (mut node, fx, _tmp) = build_fixture(3).await;
    // NO prune_undo_above — that is the whole point of this partition.
    for height in (TARGET_HEIGHT + 1)..=CHAIN_LEN {
        assert!(
            node.state_db.get_undo(height).is_some(),
            "precondition: undo at h={height} must be PRESENT so `has_undo` is true \
             (block_handling.rs:658-659) and the undo branch is taken"
        );
    }
    assert_store_is_dense(&node);

    node.execute_reorg(reorg_result(&fx), fx.tip_block.clone())
        .await
        .expect("REQ-I156-007 / O1: an undo-based reorg must not error");

    // ---- O2: the component M1 could move, byte-exact. ----
    let utxo_canonical_after = node.utxo_set.read().await.serialize_canonical();
    assert!(
        utxo_canonical_after == fx.canonical_at_target.canonical,
        "REQ-I156-007 / O2: the UTXO component of the state root after an UNDO-BASED reorg \
         must be byte-identical to canonical(h={TARGET_HEIGHT}). The undo branch \
         (block_handling.rs:661-780) never calls `UtxoSet::clear()`, so M1 must not move this \
         by a single byte. {} bytes expected, {} found.",
        fx.canonical_at_target.canonical.len(),
        utxo_canonical_after.len(),
    );

    let persisted = h::persisted_utxo_content(&node);
    assert_eq!(
        persisted.pairs, fx.canonical_at_target.pairs,
        "REQ-I156-007 / O2: the undo branch must land on canonical(h={TARGET_HEIGHT}) exactly"
    );
    for (op, amt) in &fx.range_created_unspent {
        assert!(
            !persisted.contains(op),
            "REQ-I156-007 / O2: the undo branch must already remove range-created outpoint \
             {:.8}#{} ({amt} doli) — it iterates `undo.created_utxos` at \
             block_handling.rs:686-688. This is the behaviour the legacy branch is SUPPOSED to \
             reproduce.",
            op.tx_hash,
            op.index
        );
    }
    assert!(
        persisted.contains(&fx.pre_range_spent),
        "REQ-I156-007 / O2: the undo branch must restore OUT_B (block_handling.rs:690-693)"
    );

    // ---- O3..O9 ----
    assert_reorg_completed(&node, &fx, "P3a REQ-I156-007").await;
}
