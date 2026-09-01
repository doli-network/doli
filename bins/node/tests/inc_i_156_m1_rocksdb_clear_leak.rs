//! INC-I-156 / M1 / R1 — the legacy no-undo rollback rebuild REPLAYS ON TOP OF AN
//! UN-CLEARED UTXO SET on every production node, leaking the rolled-back block's outputs.
//!
//! Requirements: REQ-I156-003 (Must, THE red test), REQ-I156-007 (Must, happy-path lock),
//! REQ-I156-010 (Should, INV-SYNC-014 regression assertion). Milestone M1, run 493.
//! Analysis: `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md` §2 (R1 verified),
//! §3 (fix direction + honest failure-mode statement), §5 (acceptance criteria).
//!
//! Sibling files: `inc_i_156_m1_reorg_clear_leak.rs` (REQ-I156-004, the `execute_reorg`
//! twin of this defect), `crates/storage/tests/inc_i_156_clear_contract_test.rs`
//! (REQ-I156-001/-002, the `UtxoSet::clear()` contract itself), and the shared fixture at
//! `inc_i_156_m1_harness/mod.rs`.
//!
//! ## The defect, in one paragraph
//!
//! `UtxoSet::clear()` (`crates/storage/src/utxo/set.rs:68-78`) matches `UtxoSet::RocksDb(_)`
//! to an EMPTY BLOCK — a silent no-op — and its doc comment justifies that with a claim
//! about call sites ("only called during genesis reset") that is false. Two production call
//! sites depend on `clear()` meaning "empty the set" before a rebuild-from-genesis replay:
//! `rollback.rs:191` and `block_handling.rs:803`. Every node that can reach either one holds
//! the `RocksDb` variant (`init.rs:311` for continuous nodes; `fork_recovery.rs:363` converts
//! a snap-installed node straight back to it, INV-SYNC-014 / INC-I-118). So on every real
//! node the sequence is *no-op clear* -> *replay `1..=target_height` over the live set*. The
//! replay restores everything the rolled-back range SPENT but removes nothing it CREATED.
//! The residual is `{outputs created in target+1..=current, unspent within that range}`,
//! which always contains at least the rolled-back block's coinbase — so this fires on 100%
//! of executions, not on an edge case. `atomic_replace` at `rollback.rs:281` then re-reads
//! the same polluted `cf_utxo` and writes it back: it cannot heal the leak, it makes it
//! durable. That is silent inflation — the INC-I-041 zombie-UTXO harm class, and a
//! violation of INV-UTXO-001 (`total_after == total_before + coinbase_amount`).
//!
//! ## Why this was invisible to the existing suite
//!
//! `Node::new_for_test` builds the `InMemory` variant (`init.rs:1129`, `init.rs:1338`), on
//! which `clear()` is honest. An `InMemory`-variant test PASSES on the broken code and
//! proves nothing — exactly how INC-I-152's first P1-003 test failed to catch its bug
//! (analysis §5, REQ-I156-003 bullet 1). The fixture therefore installs the production
//! backend explicitly (`inc_i_156_m1_harness::install_production_utxo_backend`), the same
//! way `inc_i_152_p1_003_rollback_holed_store.rs:471` does.
//!
//! ## Relationship to INC-I-152 — orthogonal, not overlapping
//!
//! INC-I-152 fixed the HOLED-store variant of this path (`rollback.rs:169`,
//! `ensure_blocks_present`). R1 is the DENSE-store residual and is unconditional. The
//! fixture keeps the block store DENSE precisely so that the INC-I-152 guard is NOT what
//! refuses this test (analysis §5, REQ-I156-003 bullet 2): the leak must be shown on a store
//! that guard admits.
//!
//! ## Implementation surface this file constrains
//!
//! covers: crates/storage/src/utxo/set.rs           (UtxoSet::clear — the RocksDb arm is the defect; delegate to StateDb::clear_utxos, return Result<(), StorageError>)
//! covers: crates/storage/src/state_db/writes.rs    (StateDb::clear_utxos:80-102 — the already-tested primitive to delegate to; wipes cf_utxo AND cf_utxo_by_pubkey in one WriteBatch and stores utxo_count = 0)
//! covers: bins/node/src/node/rollback.rs:191       (legacy no-undo rebuild call site — must propagate the new Result with `?`)
//! covers: bins/node/src/node/block_handling.rs:803 (execute_reorg legacy rebuild call site — same signature change, same leak; proven in inc_i_156_m1_reorg_clear_leak.rs)
//! covers: bins/node/src/node/init.rs:112           (startup undo-gap recovery — behind the INC-I-136 `!utxo_set.is_rocksdb()` fence; the fence MUST stay intact, and this file's is_rocksdb assertions are its regression lock)
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! OUTPUT CONTRACT: fn rollback_one_block(&mut self) -> Result<RollbackOutcome>
//!   (`bins/node/src/node/rollback.rs:10`, `pub async`. It takes NO `target_height`
//!    argument — `local_height` is read from `sync_manager.local_tip()` at rollback.rs:11-14
//!    and `target_height = local_height - 1` is derived at rollback.rs:38. Branch under
//!    test: the legacy no-undo-data fallback, rollback.rs:140-230.)
//!
//! OUTPUTS — full enumeration (receiver mutations through interior mutability, return
//! value, persistent-store writes; there are no `&mut` params — the receiver carries all):
//!   O1: return value `Result<RollbackOutcome>` — RolledBack, or RefusedNoMutation at a
//!       precondition, Err = failed.
//!   O2: `self.utxo_set` (RwLock receiver mutation) AND, because the variant is
//!       `RocksDb(Arc<StateDb>)`, the PERSISTENT `cf_utxo` / `cf_utxo_by_pubkey` column
//!       families — `add_transaction` on this variant is a direct `StateDb::insert_utxo`
//!       (set.rs:146-168). Made doubly durable by `atomic_replace` at rollback.rs:281.
//!       **THE load-bearing output.** Read back INDEPENDENTLY through `node.state_db`
//!       (Rule AQ-5), not only through the façade.
//!   O3: `self.chain_state.{best_height,best_hash,best_slot}` (rollback.rs:240-245).
//!   O4: `self.producer_set` (rollback.rs:226-229) + its persisted form via atomic_replace.
//!   O5: `block_store` canonical entry at `local_height` — persistent store DELETE
//!       (`remove_canonical_entry`, rollback.rs:254-257, INC-I-144).
//!   O6: `self.sync_manager` local tip + fork signals (rollback.rs:259-272).
//!   O7: `StateDb.utxo_count` atomic counter — INV-GUARD-001 requires it to equal the
//!       distinct `cf_utxo` key count.
//!   O8: `self.utxo_set` VARIANT identity — INV-SYNC-014 / REQ-I156-010: the live set must
//!       still be the `state_db`-backed variant afterwards. (Distinct from O2: a fix that
//!       rebuilt into a scratch `InMemory` set and published it would satisfy O2 and
//!       violate O8, silently detaching the node from state_db.)
//!
//! PATHS through the function:
//!   P0: `local_height == 0` (rollback.rs:34) / rollback-to-genesis refusal (:44) /
//!       cumulative-depth cap (:58) / missing parent at target (:82) / holed-store refusal
//!       (:169) — early returns, nothing mutated. Not exercised here; the block-store
//!       refusals are owned by `inc_i_152_p1_003_rollback_holed_store.rs`. Named for
//!       completeness of the enumeration.
//!   P1: undo-based rollback — `get_undo(local_height) == Some(_)` (rollback.rs:100).
//!       `clear()` is NEVER called on this path. The overwhelmingly common case.
//!   P2: legacy no-undo rebuild — `get_undo(local_height) == None` (rollback.rs:140).
//!       Calls `utxo.clear()` at rollback.rs:191 then replays `1..=target_height`.
//!       **THE DEFECTIVE PATH.**
//!
//! INPUT PARTITIONS (an input partition is an input class that changes the RELATIONSHIP
//! between the asserted quantities, not merely the branch taken):
//!   P2a DENSE store, RocksDb variant, tip block CREATES outputs
//!       — relationship: `post == canonical(target)`. Pre-fix the relationship is instead
//!         `post == canonical(target) ∪ created(tip)`, and `total_value` is strictly
//!         GREATER. **THE RED PARTITION.**
//!   P2b DENSE store, RocksDb variant, tip block ALSO SPENDS an earlier output
//!       — a second, independent relationship inside the same code path: an output the tip
//!         block consumed must be RESTORED by the replay. A fix that satisfied P2a by
//!         merely deleting `created(tip)` (rather than genuinely clearing and replaying)
//!         would pass P2a and fail P2b. Carried by the same rollback call, asserted
//!         separately.
//!   P2c DENSE store, RocksDb variant, `clear_utxos()` already applied by the test
//!       — the ORACLE. Establishes that `canonical(target)` is exactly reachable by
//!         clear-then-replay, so P2a's expected values are MEASURED, not predicted.
//!         Passes pre- AND post-fix.
//!   P1a undo present, valid `producer_snapshot`, DENSE store
//!       — relationship: the UTXO and ProducerSet components of the state root at `target`
//!         are IDENTICAL pre- and post-fix, because `clear()` is never reached. The
//!         chain_state component carries a pre-existing `total_work` drift that is PINNED,
//!         not tolerated — see that test's doc comment. REQ-I156-007.
//!
//! MATRIX — 8 outputs × 4 partitions = 32 cells:
//!   P2a: O1 RolledBack | O2 == canonical(target), created(tip) ABSENT | O3 h=TARGET, hash=parent
//!        | O4 non-empty & serializable | O5 tip entry removed | O6 tip == TARGET
//!        | O7 utxo_count == iter_all().len() | O8 is_rocksdb()
//!        -> `inc_i_156_req003_legacy_rollback_must_not_leak_rolled_back_block_outputs` [RED]
//!   P2b: same 8 cells, plus the restored-spend assertion inside that test.  [RED, same fn]
//!   P2c: O1 RolledBack | O2 == canonical(target) byte-exact | O3..O8 as P2a
//!        -> `inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical` [PASS-LOCK]
//!   P1a: O1 RolledBack | O2 utxo component byte-identical to canonical(TARGET)
//!        | O3 h=TARGET + chain_state canonical [0..44] and [52..140] restored, with the
//!          pre-existing `total_work` drift at offset 44..52 PINNED (see that test's doc)
//!        | O4 producer component byte-identical to its TARGET value
//!        | O5 tip entry removed | O6 tip == TARGET | O7 INV-GUARD-001 | O8 is_rocksdb()
//!        -> `inc_i_156_req007_undo_based_rollback_state_root_unchanged`      [PASS-LOCK]
//!
//! PRE-FIX VERDICT — MEASURED on this branch (base f4e6ea69), not predicted:
//!   P2a/P2b FAIL at O2, verbatim: "2 of 2 outputs CREATED by the rolled-back block (h=20)
//!   are STILL PRESENT in cf_utxo after rolling back to h=19 ... total_value: canonical(19)
//!   = 1900098000 | after rollback = 2000195000 | inflation = 100097000 || utxo count:
//!   20 -> 22". The inflation decomposes exactly: 100000000 (the h=20 coinbase to the
//!   reward pool) + 97000 (OUT_C, the tip block's Transfer output).
//!   P2c and P1a PASS pre-fix and must keep passing post-fix; together they forbid the two
//!   degenerate "fixes" (refuse every legacy rollback; break the undo path).
//!
//! O1 NOTE: `RolledBack` is asserted rather than merely `is_ok()` because before INC-I-204 M3
//! the legacy branch's holed-store refusal returned the SAME value as success — only the
//! combination of the outcome AND the O3 height change proved the rebuild actually ran.

mod inc_i_156_m1_harness;

use crypto::Hash;
use doli_node::node::Node;
use doli_node::node::RollbackOutcome;
use inc_i_156_m1_harness as h;
use storage::Outpoint;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Chain tip / `local_height` seen by `rollback_one_block` (rollback.rs:11-14).
const CHAIN_LEN: u64 = 20;
/// `target_height = local_height - 1` (rollback.rs:38). The canonical state we must land on.
const TARGET_HEIGHT: u64 = CHAIN_LEN - 1;

/// Height whose Transfer creates OUT_A from the synthetic funding UTXO.
const FUND_SPEND_AT: u64 = 10;
/// Height whose Transfer spends OUT_A and creates OUT_B.
const CHAIN_SPEND_AT: u64 = 17;

const FUNDING_AMOUNT: u64 = 100_000;
const OUT_A_AMOUNT: u64 = 99_000;
const OUT_B_AMOUNT: u64 = 98_000;
/// Created by the TIP block (h = CHAIN_LEN) — must NOT survive the rollback.
const OUT_C_AMOUNT: u64 = 97_000;

// ==================== Fixture ====================

/// Everything the assertions need to name the harm precisely.
struct Fixture {
    /// The canonical UTXO set at `TARGET_HEIGHT`, captured through the REAL `apply_block`
    /// path BEFORE the tip block existed. This is the ground truth the rollback must land
    /// on — observed, not recomputed and not predicted.
    canonical_at_target: h::UtxoContent,
    /// Outpoints CREATED by the tip block (its coinbase output + its Transfer output).
    /// Post-fix these must all be gone; pre-fix they all survive.
    tip_created: Vec<(Outpoint, u64)>,
    /// The output the tip block SPENT. The rollback must RESTORE it (partition P2b).
    tip_spent: Outpoint,
    /// `block_store` hash at `TARGET_HEIGHT` — what `chain_state.best_hash` must become.
    target_hash: Hash,
    /// State root at `TARGET_HEIGHT` for REQ-I156-007.
    state_root_at_target: Hash,
    /// The `compute_state_root` components at `TARGET_HEIGHT`, captured SEPARATELY so a
    /// root mismatch can be ATTRIBUTED to a component instead of reported as an opaque hash
    /// diff (`snapshot.rs:30-32`). That decomposition is what found the `total_work` drift.
    cs_canonical_at_target: Vec<u8>,
    ps_canonical_at_target: Vec<u8>,
    total_minted_at_target: u64,
}

/// Drive a node into the exact R1 pre-rollback shape:
///   * the PRODUCTION `RocksDb` UTXO backend,
///   * a real `CHAIN_LEN`-block chain applied through `apply_block`, with a spend chain
///     (funding -> OUT_A at h=10 -> OUT_B at h=17 -> OUT_C at h=20) so the tip block both
///     CREATES and SPENDS, exercising partitions P2a and P2b,
///   * a DENSE block store over `1..=TARGET_HEIGHT` (the INC-I-152 guard at rollback.rs:169
///     must ADMIT this store — R1 is the dense-store residual).
///
/// ## Why the synthetic funding UTXO does not corrupt the oracle
///
/// The funding entry is written directly into the set (no block produces it), so a
/// rebuild-from-genesis can never recreate it. It is therefore SPENT at h=10, far below
/// `TARGET_HEIGHT`, which makes it absent from `canonical(TARGET)` too. Both sides of every
/// equality below agree on it by being agreed-absent. Nothing else in the chain is
/// synthetic.
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
        b"inc_i_156_m1_rollback_funding",
    )
    .await;

    // h=10: funding -> OUT_A.
    let tx_a = h::signed_transfer(funding, OUT_A_AMOUNT, &owner, owner_pkh);
    let out_a = Outpoint::new(tx_a.hash(), 0);
    h::apply_block_with_transfer(&mut node, &producers, FUND_SPEND_AT, &params, tx_a).await;

    h::apply_plain_up_to(&mut node, &producers, CHAIN_SPEND_AT - 1, &params).await;

    // h=17: OUT_A -> OUT_B. OUT_B is the output the TIP block will spend.
    let tx_b = h::signed_transfer(out_a, OUT_B_AMOUNT, &owner, owner_pkh);
    let out_b = Outpoint::new(tx_b.hash(), 0);
    h::apply_block_with_transfer(&mut node, &producers, CHAIN_SPEND_AT, &params, tx_b).await;

    h::apply_plain_up_to(&mut node, &producers, TARGET_HEIGHT, &params).await;

    // ---- GROUND TRUTH: the canonical set at TARGET_HEIGHT, before the tip exists. ----
    assert_eq!(
        node.chain_state.read().await.best_height,
        TARGET_HEIGHT,
        "fixture: the chain must pause at TARGET_HEIGHT to capture ground truth"
    );
    let canonical_at_target = h::utxo_content(&node).await;
    assert!(
        canonical_at_target.len > 0 && canonical_at_target.total_value > 0,
        "fixture: canonical(TARGET) must be populated (len={}, total={})",
        canonical_at_target.len,
        canonical_at_target.total_value
    );
    assert!(
        canonical_at_target.contains(&out_b),
        "fixture: OUT_B must be UNSPENT at TARGET_HEIGHT — it is the output the tip block \
         consumes and the rollback must restore"
    );
    assert!(
        !canonical_at_target.contains(&funding),
        "fixture: the synthetic funding UTXO must be SPENT by TARGET_HEIGHT, otherwise it \
         would appear in canonical(TARGET) but never in a rebuild-from-genesis and every \
         set equality below would be a false failure"
    );
    let target_hash = node.chain_state.read().await.best_hash;
    let (
        state_root_at_target,
        cs_canonical_at_target,
        ps_canonical_at_target,
        total_minted_at_target,
    ) = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        (
            storage::compute_state_root(&cs, &utxo, &ps).expect("fixture: state root at TARGET"),
            cs.serialize_canonical().to_vec(),
            ps.serialize_canonical(),
            cs.total_minted,
        )
    };

    // ---- The TIP block: creates a coinbase output + OUT_C, spends OUT_B. ----
    let tx_c = h::signed_transfer(out_b, OUT_C_AMOUNT, &owner, owner_pkh);
    let tip_block =
        h::apply_block_with_transfer(&mut node, &producers, CHAIN_LEN, &params, tx_c).await;
    let tip_created = h::created_outpoints(&tip_block);
    assert_eq!(
        tip_created.len(),
        2,
        "fixture: the tip block must create exactly 2 outputs (coinbase + OUT_C), got {}",
        tip_created.len()
    );
    assert_eq!(
        node.sync_manager.read().await.local_tip().0,
        CHAIN_LEN,
        "fixture: sync_manager local tip must be CHAIN_LEN — rollback_one_block reads \
         `local_height` from there (rollback.rs:11-14), not from chain_state"
    );

    (
        node,
        Fixture {
            canonical_at_target,
            tip_created,
            tip_spent: out_b,
            target_hash,
            state_root_at_target,
            cs_canonical_at_target,
            ps_canonical_at_target,
            total_minted_at_target,
        },
        temp,
    )
}

/// Erase the undo log (`prune_undo_above(0)`, the production truncation API used at
/// `inc_i_152_p1_003_rollback_holed_store.rs:544`) so `get_undo(local_height)` is `None` and
/// `rollback_one_block` takes the LEGACY branch at rollback.rs:140. This is what a
/// snap-synced or freshly-wiped node looks like: state installed at the tip, nothing to
/// undo — the exact INC-I-152 fleet scenario.
fn force_legacy_branch(node: &Node) {
    node.state_db.prune_undo_above(0);
    assert!(
        node.state_db.get_undo(CHAIN_LEN).is_none(),
        "fixture: undo at h={CHAIN_LEN} must be absent so the legacy branch is taken"
    );
}

/// The dense-store precondition. R1 is orthogonal to store density, and this assertion makes
/// that explicit: the INC-I-152 guard at rollback.rs:169 must ADMIT this store, so it cannot
/// be what any failure below is attributable to.
fn assert_store_is_dense(node: &Node) {
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .unwrap_or_else(|e| {
            panic!(
                "precondition: the block store must be DENSE over 1..={TARGET_HEIGHT} so the \
                 INC-I-152 guard at rollback.rs:169 admits this rollback — got {e}"
            )
        });
}

/// Assert every non-O2 output of a COMPLETED rollback. Shared by the red test, the oracle
/// and the happy-path lock so all three hold the same contract.
async fn assert_rollback_completed(node: &Node, fx: &Fixture, scenario: &str) {
    // ---- O3 ----
    let (cs_height, cs_hash) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        cs_height, TARGET_HEIGHT,
        "[{scenario}] / O3: chain_state.best_height must be {TARGET_HEIGHT} after a completed \
         rollback"
    );
    assert_eq!(
        cs_hash, fx.target_hash,
        "[{scenario}] / O3: chain_state.best_hash must be the block at {TARGET_HEIGHT}"
    );

    // ---- O4 ----
    let producer_bytes = {
        let ps = node.producer_set.read().await;
        bincode::serialize(&*ps).expect("[O4] ProducerSet must serialize")
    };
    assert!(
        !producer_bytes.is_empty(),
        "[{scenario}] / O4: producer_set must still serialize after \
         rebuild_producer_set_from_blocks (rollback.rs:226-229)"
    );

    // ---- O5 (INC-I-144) ----
    assert!(
        node.block_store
            .get_hash_by_height(CHAIN_LEN)
            .expect("[O5] block_store read")
            .is_none(),
        "[{scenario}] / O5: the canonical entry at h={CHAIN_LEN} must be purged by a completed \
         rollback (remove_canonical_entry, rollback.rs:254-257)"
    );

    // ---- O6 ----
    assert_eq!(
        node.sync_manager.read().await.local_tip().0,
        TARGET_HEIGHT,
        "[{scenario}] / O6: sync_manager local tip must follow the chain down to {TARGET_HEIGHT}"
    );

    // ---- O7 + O8 ----
    h::assert_utxo_invariants(node, scenario).await;
}

// ==========================================================================
//  P2a + P2b — THE RED TEST (REQ-I156-003). Must FAIL pre-fix, on O2.
// ==========================================================================

/// Requirement: REQ-I156-003 (Must) — path P2, partitions P2a (leak) and P2b (restore).
///
/// Acceptance criteria discharged, in the order the analysis lists them (§5):
///   * RocksDb variant (`UtxoSet::from_state_db`), not `new_for_test`'s InMemory — asserted
///     by `install_production_utxo_backend` and again as O8.
///   * Block store DENSE over `1..=target_height` — `assert_store_is_dense`.
///   * Undo for the tip ABSENT (`prune_undo_above(0)`) — `force_legacy_branch`.
///   * POST-FIX: the UTXO set is EXACTLY `canonical(TARGET_HEIGHT)` — same total supply,
///     same outpoint set, none of the rolled-back block's created outpoints remain.
///   * `utxo_count() == iter_all().len()` (INV-GUARD-001) — O7.
///   * `is_rocksdb()` still true (INV-SYNC-014) — O8.
///   * Asserted on the PERSISTED state, after `atomic_replace` — `persisted_utxo_content`
///     reads `node.state_db` directly rather than trusting the façade (Rule AQ-5).
///
/// PRE-FIX (the RED signal): `utxo.clear()` at rollback.rs:191 is a no-op on this variant,
/// so the replay of `1..=19` lands on top of the state at height 20. The tip block's
/// coinbase output and OUT_C both survive and `total_value()` is strictly GREATER than
/// `canonical(19)` — durable inflation, INV-UTXO-001 violated.
#[tokio::test]
async fn inc_i_156_req003_legacy_rollback_must_not_leak_rolled_back_block_outputs() {
    let (mut node, fx, _tmp) = build_fixture(3).await;
    force_legacy_branch(&node);
    assert_store_is_dense(&node);

    // Precondition: the tip block's outputs really are live right now, so their absence
    // afterwards is a state CHANGE and not a tautology (Rule AQ-3).
    {
        let utxo = node.utxo_set.read().await;
        for (op, amt) in &fx.tip_created {
            assert!(
                utxo.contains(op),
                "precondition: tip-created outpoint {:.8}#{} ({amt} doli) must be live before \
                 the rollback",
                op.tx_hash,
                op.index
            );
        }
        assert!(
            !utxo.contains(&fx.tip_spent),
            "precondition: OUT_B must be SPENT by the tip block before the rollback — the \
             rollback has to RESTORE it (partition P2b)"
        );
    }

    // ---- Drive the REAL entry point. ----
    let result = node
        .rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await;

    // ---- O1 ----
    let rolled = result.unwrap_or_else(|e| {
        panic!(
            "REQ-I156-003 / O1: rollback_one_block must not error on a DENSE store with the \
             RocksDb backend — got {e}"
        )
    }) == RollbackOutcome::RolledBack;
    assert!(
        rolled,
        "REQ-I156-003 / O1: rollback_one_block must report Ok(RolledBack)"
    );

    // ---- O2, read back from the PERSISTENT store (AQ-5). THE assertions. ----
    let persisted = h::persisted_utxo_content(&node);
    let leaked: Vec<(Outpoint, u64)> = fx
        .tip_created
        .iter()
        .filter(|(op, _)| persisted.contains(op))
        .copied()
        .collect();

    assert!(
        leaked.is_empty(),
        "REQ-I156-003 / O2 — THE LEAK: {} of {} outputs CREATED by the rolled-back block \
         (h={CHAIN_LEN}) are STILL PRESENT in cf_utxo after rolling back to h={TARGET_HEIGHT}: \
         [{}].\n\
         Mechanism: `utxo.clear()` at bins/node/src/node/rollback.rs:191 dispatches to the \
         `UtxoSet::RocksDb(_)` arm of crates/storage/src/utxo/set.rs:71-76, which is an EMPTY \
         BLOCK. Every production node holds that variant (init.rs:311; fork_recovery.rs:363 \
         converts a snap-installed node straight back to it). So the replay of \
         `1..={TARGET_HEIGHT}` at rollback.rs:192-223 runs ON TOP of the state at \
         h={CHAIN_LEN}: it restores what the rolled-back block SPENT but removes nothing it \
         CREATED. `atomic_replace` at rollback.rs:281 then re-reads the same polluted cf_utxo \
         and writes it back — it cannot heal the leak, it makes it durable. Every block \
         carries a coinbase, so the residual is NEVER empty: this fires on 100% of executions \
         of this path, not on an edge case. That is silent inflation (INC-I-041 zombie-UTXO \
         class) and it violates INV-UTXO-001.\n\
         Fix: delegate the RocksDb arm to `StateDb::clear_utxos()` \
         (crates/storage/src/state_db/writes.rs:80-102 — already correct, already tested at \
         crates/storage/tests/disk_guardian_failsafe_test.rs:283) and return \
         `Result<(), StorageError>` so rollback.rs:191 and block_handling.rs:803 must \
         propagate it.\n\
         total_value: canonical({TARGET_HEIGHT}) = {} | after rollback = {} | inflation = {} \
         || utxo count: {} -> {}",
        leaked.len(),
        fx.tip_created.len(),
        h::describe(&leaked, 4),
        fx.canonical_at_target.total_value,
        persisted.total_value,
        persisted
            .total_value
            .saturating_sub(fx.canonical_at_target.total_value),
        fx.canonical_at_target.len,
        persisted.len,
    );

    // ---- O2 / P2b: the output the tip block SPENT must be back. ----
    assert!(
        persisted.contains(&fx.tip_spent),
        "REQ-I156-003 / O2 / P2b: OUT_B ({:.8}#{}) was spent by the rolled-back block and must \
         be RESTORED by the rebuild. Its absence would mean the replay is not reconstructing \
         `canonical({TARGET_HEIGHT})` — a fix that merely deleted the tip's created outputs \
         instead of genuinely clearing and replaying would fail here.",
        fx.tip_spent.tx_hash,
        fx.tip_spent.index,
    );

    // ---- O2: total supply must land EXACTLY on canonical, not merely 'not higher'. ----
    assert_eq!(
        persisted.total_value, fx.canonical_at_target.total_value,
        "REQ-I156-003 / O2: total supply after the rollback must EQUAL the canonical supply at \
         h={TARGET_HEIGHT}. Pre-fix it is strictly GREATER by the rolled-back block's \
         created-and-unspent outputs. (A SMALLER value would mean the opposite failure — a \
         clear that ran but a replay that did not complete; analysis §3 accepts that as \
         fail-visible rather than fail-silent, but it is still a bug and this catches it.)"
    );

    // ---- O2: the outpoint SET must be exactly canonical. ----
    assert_eq!(
        persisted.pairs, fx.canonical_at_target.pairs,
        "REQ-I156-003 / O2: the persisted UTXO set must be EXACTLY the canonical set at \
         h={TARGET_HEIGHT} — same outpoints, same amounts. {} entries expected, {} found.",
        fx.canonical_at_target.len, persisted.len,
    );

    // ---- O2: byte-exact — the encoding consensus hashes. ----
    assert!(
        persisted.canonical == fx.canonical_at_target.canonical,
        "REQ-I156-003 / O2: `serialize_canonical()` (the exact encoding the state root is \
         computed over) must be byte-identical to canonical(h={TARGET_HEIGHT}). {} bytes \
         expected, {} found — a difference here means the node's state root diverges from \
         every peer that did not take this path.",
        fx.canonical_at_target.canonical.len(),
        persisted.canonical.len(),
    );

    // ---- O2 consistency: the façade view must match the persisted view. ----
    let facade = h::utxo_content(&node).await;
    assert_eq!(
        facade.pairs, persisted.pairs,
        "REQ-I156-003 / O2: the façade view and the persisted cf_utxo view must agree after \
         atomic_replace (they are the same store on the RocksDb variant; a disagreement would \
         mean the fix swapped the backend)"
    );

    // ---- O3..O8 ----
    assert_rollback_completed(&node, &fx, "P2a/P2b REQ-I156-003").await;
}

// ==========================================================================
//  P2c — ORACLE. Passes pre- AND post-fix. Makes the red test's expected
//  values MEASURED rather than predicted.
// ==========================================================================

/// Requirement: REQ-I156-003 (Must) — path P2, partition P2c. NOT a red test.
///
/// This test performs, by hand and immediately before the call, the ONE thing the fix will
/// make `UtxoSet::clear()` do: `StateDb::clear_utxos()`. The subsequent no-op `clear()` at
/// rollback.rs:191 then leaves an already-empty set, so the replay reconstructs the UTXO set
/// from genesis exactly as it will post-fix.
///
/// It therefore proves two things the red test depends on and cannot prove about itself:
///   1. `canonical(TARGET_HEIGHT)` really IS reachable by clear-then-replay, byte-for-byte —
///      so the red test's expected values are an OBSERVED state, not a prediction. Without
///      this, a failure of the red test could not be attributed to the leak rather than to a
///      rebuild that legitimately differs from incremental application.
///   2. `StateDb::clear_utxos()` is the correct primitive to delegate to (analysis §3, fix
///      direction (a)) — measured, not argued.
///
/// It must PASS both before and after the fix. Post-fix it becomes redundant with the red
/// test by construction; that redundancy is the proof the fix landed where it was aimed.
#[tokio::test]
async fn inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical() {
    let (mut node, fx, _tmp) = build_fixture(3).await;
    force_legacy_branch(&node);
    assert_store_is_dense(&node);

    // Stand in for the post-fix `UtxoSet::clear()`.
    node.state_db
        .clear_utxos()
        .expect("ORACLE: StateDb::clear_utxos must succeed on a writable DB");
    assert_eq!(
        node.state_db.utxo_len(),
        0,
        "ORACLE precondition: clear_utxos must have emptied cf_utxo"
    );

    let rolled = node
        .rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("ORACLE / O1: the rollback must complete")
        == RollbackOutcome::RolledBack;
    assert!(rolled, "ORACLE / O1: rollback must report Ok(RolledBack)");

    let persisted = h::persisted_utxo_content(&node);
    assert_eq!(
        persisted.pairs, fx.canonical_at_target.pairs,
        "ORACLE / O2: clear-then-replay must reproduce canonical(h={TARGET_HEIGHT}) EXACTLY. \
         If this fails, the red test's expected values are wrong and the failure it reports is \
         not attributable to the leak — investigate the harness before touching the fix. {} \
         entries expected, {} found.",
        fx.canonical_at_target.len, persisted.len,
    );
    assert_eq!(
        persisted.total_value, fx.canonical_at_target.total_value,
        "ORACLE / O2: total supply must match canonical(h={TARGET_HEIGHT})"
    );
    assert!(
        persisted.canonical == fx.canonical_at_target.canonical,
        "ORACLE / O2: serialize_canonical() must be byte-identical to canonical(h={TARGET_HEIGHT})"
    );
    for (op, amt) in &fx.tip_created {
        assert!(
            !persisted.contains(op),
            "ORACLE / O2: tip-created outpoint {:.8}#{} ({amt} doli) must be gone once the set \
             is genuinely cleared before the replay",
            op.tx_hash,
            op.index
        );
    }

    assert_rollback_completed(&node, &fx, "P2c ORACLE").await;
}

// ==========================================================================
//  P1a — REQ-I156-007 happy-path REGRESSION LOCK. Passes pre- AND post-fix.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — path P1, partition P1a. NOT a red test.
///
/// Acceptance: "Undo-based rollback with a valid `producer_snapshot`: state root at
/// `target_height` is identical pre- and post-fix."
///
/// The undo branch (rollback.rs:100-139) NEVER calls `clear()`, so the M1 fix must not move
/// this state root by a single byte. Rather than compare an opaque root hash — which cannot
/// say WHICH component moved — this test asserts the three `compute_state_root` components
/// SEPARATELY (`snapshot.rs:30-32`). The UTXO component is the one and only component M1 can
/// touch, so it carries the lock; the other two are pinned so a fix that leaked sideways is
/// attributable on sight.
///
/// ## PRE-EXISTING DRIFT, measured while writing this test (NOT an M1 defect)
///
/// The full state root after an undo-based rollback does NOT return to its `TARGET_HEIGHT`
/// value, and the cause is not INC-I-156. A byte-level diff of the `ChainState` canonical
/// encoding isolated exactly ONE differing byte, at offset 44 — the low byte of `total_work`
/// (`chain_state.rs:148`). `ChainState::update_tip` maintains the documented invariant
/// `total_work == height` (`chain_state.rs:128-131`), but every rollback path assigns
/// `state.best_height` DIRECTLY without going through `update_tip` (`rollback.rs:240-245`;
/// `block_handling.rs:748-752`, `:798-802`). `total_work` IS part of the consensus
/// state-root encoding, so after any rollback the node's chain_state component diverges from
/// a peer at the same height that never rolled back.
///
/// This is identical on both branches and before and after the M1 fix, so it is PINNED here
/// (with an assertion that fails loudly if it is ever repaired) rather than fixed here.
/// Flagged for the reviewer; it belongs with the analysis §7 out-of-scope items, alongside
/// REQ-I156-014's chain_state ↔ cf_utxo inconsistency.
#[tokio::test]
async fn inc_i_156_req007_undo_based_rollback_state_root_unchanged() {
    let (mut node, fx, _tmp) = build_fixture(3).await;
    // NO prune_undo_above here — that is the whole point of this partition.
    assert!(
        node.state_db.get_undo(CHAIN_LEN).is_some(),
        "precondition: undo data at h={CHAIN_LEN} must be PRESENT so rollback_one_block takes \
         the undo branch (rollback.rs:100), which never calls clear()"
    );
    assert_store_is_dense(&node);

    let rolled = node
        .rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("REQ-I156-007 / O1: an undo-based rollback must not error")
        == RollbackOutcome::RolledBack;
    assert!(
        rolled,
        "REQ-I156-007 / O1: rollback_one_block must report Ok(RolledBack)"
    );

    let (root_after, cs_after, ps_after, utxo_canonical_after, total_minted_after) = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        (
            storage::compute_state_root(&cs, &utxo, &ps).expect("state root after rollback"),
            cs.serialize_canonical().to_vec(),
            ps.serialize_canonical(),
            utxo.serialize_canonical(),
            cs.total_minted,
        )
    };

    // O2 — the UTXO component. THIS is the only one the M1 fix can move (it changes
    // `UtxoSet::clear()` and nothing else), so it is the load-bearing lock.
    assert!(
        utxo_canonical_after == fx.canonical_at_target.canonical,
        "REQ-I156-007 / O2: the UTXO component of the state root after an UNDO-BASED rollback \
         must be byte-identical to canonical(h={TARGET_HEIGHT}). The undo branch \
         (rollback.rs:100-139) never calls `UtxoSet::clear()`, so M1 must not move this by a \
         single byte. {} bytes expected, {} found.",
        fx.canonical_at_target.canonical.len(),
        utxo_canonical_after.len(),
    );

    // O4 — the ProducerSet component, restored from `undo.producer_snapshot`
    // (rollback.rs:130-134). M1 does not touch this path; pinned as a regression lock.
    assert!(
        ps_after == fx.ps_canonical_at_target,
        "REQ-I156-007 / O4: the ProducerSet component must be byte-identical to its value at \
         h={TARGET_HEIGHT} (restored from undo.producer_snapshot at rollback.rs:130-134). {} \
         bytes expected, {} found.",
        fx.ps_canonical_at_target.len(),
        ps_after.len(),
    );

    // O3 — the ChainState component: everything the rollback manages, restored exactly.
    assert_eq!(
        &cs_after[0..44],
        &fx.cs_canonical_at_target[0..44],
        "REQ-I156-007 / O3: the (best_hash, best_height, best_slot) prefix of the ChainState \
         canonical encoding (chain_state.rs:145-147) must be restored exactly by the rollback"
    );
    assert_eq!(
        &cs_after[52..140],
        &fx.cs_canonical_at_target[52..140],
        "REQ-I156-007 / O3: the ChainState tail (genesis_hash, genesis_timestamp, \
         last_registration_hash, registration_sequence, total_minted — chain_state.rs:149-153) \
         must be untouched by a rollback. total_minted drift measured: {}",
        total_minted_after.saturating_sub(fx.total_minted_at_target)
    );

    // The ONE field that drifts. Measured, not assumed — the diagnostic that found it
    // reported a single differing byte at canonical offset 44, which is `total_work`.
    let total_work_after = node.chain_state.read().await.total_work;
    assert_eq!(
        (total_work_after, node.chain_state.read().await.best_height),
        (CHAIN_LEN, TARGET_HEIGHT),
        "PRE-EXISTING DRIFT PIN (orthogonal to INC-I-156, out of M1 scope): \
         `ChainState::update_tip` maintains the documented invariant `total_work == height` \
         (chain_state.rs:128-131), but BOTH rollback branches assign `state.best_height` \
         directly (rollback.rs:240-245; block_handling.rs:748-752 and :798-802) without going \
         through `update_tip`. So after a rollback `total_work` still reads {CHAIN_LEN} while \
         `best_height` reads {TARGET_HEIGHT} — and `total_work` IS part of the consensus \
         state-root encoding (chain_state.rs:148, canonical offset 44..52). This is present on \
         BOTH branches and identical before and after the M1 fix, which is why it is pinned \
         here instead of fixed here. If a future change repairs it, this assertion fails \
         loudly and the whole test should collapse to the single stronger line \
         `assert_eq!(root_after, fx.state_root_at_target)`."
    );
    assert_ne!(
        root_after, fx.state_root_at_target,
        "REQ-I156-007 / O3: given the `total_work` drift pinned above, the FULL state root \
         cannot equal its h={TARGET_HEIGHT} value. This assertion exists so the drift cannot be \
         quietly absorbed: if it starts failing, `total_work` is being rewound and the three \
         component assertions above should be replaced by a single \
         `root_after == fx.state_root_at_target`."
    );

    // ---- O2: the tip's outputs are gone via the undo route too. ----
    let persisted = h::persisted_utxo_content(&node);
    for (op, amt) in &fx.tip_created {
        assert!(
            !persisted.contains(op),
            "REQ-I156-007 / O2: the undo branch must already remove tip-created outpoint \
             {:.8}#{} ({amt} doli) — it iterates `undo.created_utxos` at rollback.rs:109-111. \
             This is the behaviour the legacy branch is SUPPOSED to reproduce.",
            op.tx_hash,
            op.index
        );
    }
    assert_eq!(
        persisted.pairs, fx.canonical_at_target.pairs,
        "REQ-I156-007 / O2: the undo branch must land on canonical(h={TARGET_HEIGHT}) exactly"
    );

    // ---- O3..O8 ----
    assert_rollback_completed(&node, &fx, "P1a REQ-I156-007").await;
}
