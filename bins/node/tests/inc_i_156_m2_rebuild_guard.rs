//! INC-I-156 / M2 — R2: `rebuild_producer_set_from_blocks` is DESTRUCTIVE-THEN-FALLIBLE.
//!
//! covers: bins/node/src/node/rewards.rs (rebuild_producer_set_from_blocks)
//! covers: bins/node/src/node/rollback.rs (the unguarded call site at rollback.rs:144)
//!
//! Requirements under test: REQ-I156-005 (red), REQ-I156-006 (unchanged-after-refusal),
//! REQ-I156-007 (happy-path regression locks).
//!
//! ## The defect, as measured on this branch (cba23389)
//!
//! `Node::rebuild_producer_set_from_blocks` (`rewards.rs:1105`) calls `producers.clear()` at
//! `rewards.rs:1110` — a REAL clear (`producer/set_core.rs:66-71`: drops `producers`, drops
//! `pending_updates`, invalidates `active_cache`) — and only THEN walks
//! `for height in 1..=target_height` (`rewards.rs:1115`), aborting at the first missing height
//! with `"Producer set rebuild: missing block at height {} (store corrupted)"`
//! (`rewards.rs:1116-1124`). On a HOLED block store it therefore **destroys the caller's
//! ProducerSet and then aborts** — destroy-then-abort, the exact mirror of the correction
//! INC-I-152 had to apply to P1-003's framing. It does NOT return a plausible-looking wrong
//! set; it returns `Err` over wreckage.
//!
//! Three of the four call sites are guarded by an upstream `ensure_blocks_present` over the
//! SAME range (`rollback.rs:175-187` for the legacy branch; `block_handling.rs:605-607` for
//! both reorg sites). The fourth — `rollback.rs:144`, the snapshot-deserialize-failure
//! fallback inside the UNDO branch — has no such incidental protection, and it is reached
//! AFTER the UTXO undo at `rollback.rs:111-123` has already been applied durably.
//!
//! Neither caller repairs the emptied set (`periodic.rs` propagates; `production/mod.rs` logs
//! `[BLOCK_POISON]` and returns), and `apply_block/mod.rs:316-318`
//! (`batch.write_full_producer_set`) makes the emptied set DURABLE on the very next applied
//! block. That escalation is the operational harm; it is documented here rather than executed,
//! because post-fix the emptied set is unreachable and an escalation test would have no
//! post-fix meaning. The durability surface is instead asserted at O4 below.
//!
//! ## OUTPUT CONTRACT — `Node::rebuild_producer_set_from_blocks(&self, producers: &mut ProducerSet, target_height: u64) -> Result<()>`
//!
//! OUTPUTS (every way the function can change observable state):
//!   O1  `producers` — the `&mut ProducerSet` parameter. The ONLY thing the function is
//!       supposed to write. Content, not counts (REQ-I156-006): `serialize_canonical()`
//!       bytes + producer count + sorted pubkeys + per-producer (bond_amount, bond_count) +
//!       `pending_update_count()`. `pending_updates` is NOT part of `serialize_canonical`
//!       (`producer/set_persistence.rs:77-110`), so it is asserted separately — a count-only
//!       or canonical-only assertion would miss the `pending_updates.clear()` inside
//!       `ProducerSet::clear()`.
//!   O2  receiver `self` — taken by `&self`. The one interior-mutability surface is
//!       `self.cached_genesis_producers` (`OnceLock`), initialised by
//!       `derive_genesis_producers_from_chain()` at `rewards.rs:1336`, reachable ONLY at
//!       `height == genesis_blocks + 1` (devnet: h=41, `network_params/defaults.rs:450`).
//!       Every partition below runs at `target_height <= 13`, so this path is
//!       UNREACHABLE-BY-CONSTRUCTION here and is asserted as such (`cached` stays uninit) in
//!       the dense partition rather than left unexamined.
//!   O3  return value `Result<()>` — `Ok(())` or `Err(msg)`; the message text is itself an
//!       output (an operator acts on it), so it is asserted, not just its variant.
//!   O4  persistent store writes — NONE. The function only READS `block_store`
//!       (`rewards.rs:1117`). Asserted positively by reading `state_db.load_producer_set()`
//!       back before and after (Rule AQ-5), so a "fix" that started persisting mid-rebuild
//!       state is caught.
//!   O5  global/static state — none.
//!   O6  channels/events — none (tracing only; not asserted).
//!
//! PATHS through the function:
//!   PA  refusal-before-mutation (post-fix only): range not dense -> `Err`, nothing cleared.
//!   PB  clear-then-abort (pre-fix behaviour on the same input): `clear()` runs, the loop dies
//!       at the first missing height -> `Err` over an emptied set.
//!   PC  success: dense range -> `Ok(())`, set = f(blocks in `1..=target_height`).
//!   PD  empty range: `target_height == 0` -> loop body never runs.
//!
//! INPUT PARTITIONS:
//!   I1  holed store, hole strictly INSIDE `1..=target_height`, block 1 and block
//!       `target_height` both present, ProducerSet non-empty. Direct call.      [RED]
//!   I2  = I1, observing the ERROR MESSAGE (O3 text).                            [RED]
//!   I3  = I1 reached through the real call site `rollback.rs:144`: undo data present, its
//!       `producer_snapshot` non-empty and NOT deserializable as a `ProducerSet`.[RED]
//!   I4  dense store, non-empty ProducerSet, `target_height = 13`.               [PASS-LOCK]
//!   I5  dense store, GUARDED caller (legacy rollback branch, `rollback.rs:267`).[PASS-LOCK]
//!   I6  `target_height == 0`, block 1 PRESENT — the guard must not over-refuse. [PASS-LOCK]
//!   I7  `target_height == 0`, block 1 MISSING — `.max(1)` must keep the "block 1 must
//!       exist" strictness INC-I-152 established (`rollback.rs:175-187`).        [RED]
//!
//! MATRIX (outputs x partitions; `-` = not applicable, `n/r` = unreachable by construction):
//!   I1: O1 ✓  O2 n/r  O3 ✓  O4 ✓
//!   I2: O1 ✓  O2 n/r  O3 ✓ (message text)  O4 -
//!   I3: O1 ✓  O2 n/r  O3 ✓  O4 ✓
//!   I4: O1 ✓  O2 ✓    O3 ✓  O4 ✓
//!   I5: O1 ✓  O2 n/r  O3 ✓ (caller's `Ok(RolledBack)` + landed height)  O4 ✓
//!   I6: O1 ✓  O2 n/r  O3 ✓  O4 -
//!   I7: O1 ✓  O2 n/r  O3 ✓  O4 -
//!
//! ## MANDATORY CAVEAT — REQ-I156-007 "state root identical pre- and post-fix"
//!
//! That acceptance criterion is **unachievable verbatim**, for a reason ORTHOGONAL to
//! INC-I-156 and already pinned by the M1-committed test
//! `inc_i_156_req007_undo_based_rollback_state_root_unchanged`
//! (`inc_i_156_m1_rocksdb_clear_leak.rs:667`): every rollback path assigns
//! `chain_state.best_height` DIRECTLY (`rollback.rs:240-245`; `block_handling.rs:748-752`,
//! `:798-802`) instead of going through `ChainState::update_tip`, which is what maintains the
//! documented `total_work == height` invariant (`chain_state.rs:128-131`). `total_work` is
//! byte 44..52 of the canonical `ChainState` encoding (`chain_state.rs:148`) and IS part of
//! the consensus state root, so `total_work` diverges after ANY rollback — before this fix and
//! after it, identically.
//!
//! That pre-existing defect is **OUT OF SCOPE FOR M2**: this file does not fix it, does not
//! extend the M1 pin, and never asserts on a whole-encoding state root. Every REQ-I156-007
//! lock below is **component-wise** — UtxoSet content, ProducerSet content, and
//! `best_height` / `best_hash` — exactly the workaround M1 adopted.

mod inc_i_156_m1_harness;
use inc_i_156_m1_harness as h;

use doli_node::node::Node;
use doli_node::node::RollbackOutcome;
use storage::ProducerSet;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Chain tip / `local_height` seen by `rollback_one_block` (`rollback.rs:11-14`).
///
/// TRAP — DO NOT read a rebuild's emptiness on this fixture as evidence of anything
/// (INC-I-156 M2 QA F2). Devnet `genesis_blocks = 40` (`network_params/defaults.rs:450`), the
/// replay loop SKIPS every `Registration` tx while `height <= genesis_blocks`
/// (`rewards.rs:1177-1179`), and the boundary registration runs only at
/// `height == genesis_blocks + 1` (`rewards.rs:1381`). `CHAIN_LEN = 14` therefore sits
/// entirely inside the genesis phase, so EVERY rebuild in THIS file legitimately returns an
/// EMPTY set (measured: live count=3 / 742 canonical bytes vs rebuilt count=0 / 16 bytes).
/// That is fine for the partitions here — they are about the GUARD, and the red tests assert
/// `Err` AND intactness of the caller's NON-EMPTY live set, which emptiness cannot fake — but
/// it makes any "byte-identical rebuild" comparison inside this file a comparison of empty to
/// empty. The non-vacuous reconstruction lock lives in
/// `inc_i_156_m2_dense_reconstruction.rs`, which crosses the boundary with real on-chain
/// genesis registrations. Raising `CHAIN_LEN` alone would NOT fix it:
/// `derive_genesis_producers_from_chain` (`genesis.rs:24-45`) needs `Registration` txs in
/// blocks `1..=genesis_blocks` and the devnet fallback is empty (`genesis.rs:65`).
const CHAIN_LEN: u64 = 14;
/// `target_height = local_height - 1` (`rollback.rs:38`) — the range the rebuild walks.
const TARGET_HEIGHT: u64 = CHAIN_LEN - 1;

/// The hole. STRICTLY INSIDE `1..=TARGET_HEIGHT` and strictly above 1, so that:
///   * block 1 is present  -> the pre-INC-I-152 "does block 1 exist?" style check waves it
///     through and control reaches the destructive section (the enabler);
///   * block `TARGET_HEIGHT` is present -> `rollback_one_block`'s parent lookup at
///     `rollback.rs:78` succeeds and the rollback proceeds to the undo branch;
///   * the hole is inside the rebuild range -> `ensure_blocks_present(1, TARGET_HEIGHT)`
///     reports it, and so does the pre-fix loop.
const HOLE_LOW: u64 = 6;
const HOLE_HIGH: u64 = 8;

/// Non-empty, NOT deserializable as a `ProducerSet`: 4 bytes cannot even supply bincode's
/// leading 8-byte map length, so deserialization fails deterministically without attempting
/// a length-driven allocation. This is the ONLY route to `rollback.rs:144`
/// (`producer_snapshot` empty -> sentinel skip; deserializable -> restore).
const CORRUPT_SNAPSHOT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

// ==================== Observation surface (REQ-I156-006) ====================

/// Full CONTENT snapshot of a `ProducerSet`. REQ-I156-006 forbids count-only assertions, and
/// `serialize_canonical` alone is not sufficient either: it covers `producers` +
/// `exit_history` only (`producer/set_persistence.rs:77-110`) and would be blind to the
/// `pending_updates.clear()` that `ProducerSet::clear()` also performs
/// (`producer/set_core.rs:66-71`). Both are captured.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProducerContent {
    /// Byte-exact canonical encoding — the same bytes the state root is computed over.
    canonical: Vec<u8>,
    count: usize,
    /// Sorted `(pubkey, bond_amount, bond_count)` — a readable diff when `canonical` differs.
    bonds: Vec<(Vec<u8>, u64, u32)>,
    /// NOT covered by `canonical`; `clear()` drops it, so it must be asserted separately.
    pending: usize,
}

impl ProducerContent {
    fn of(ps: &ProducerSet) -> Self {
        let mut bonds: Vec<(Vec<u8>, u64, u32)> = ps
            .all_producers()
            .iter()
            .map(|p| {
                (
                    p.public_key.as_bytes().to_vec(),
                    p.bond_amount,
                    p.bond_count,
                )
            })
            .collect();
        bonds.sort();
        Self {
            canonical: ps.serialize_canonical(),
            count: ps.total_count(),
            bonds,
            pending: ps.pending_update_count(),
        }
    }

    async fn live(node: &Node) -> Self {
        Self::of(&*node.producer_set.read().await)
    }

    /// What is persisted in `state_db` — O4. `rebuild_producer_set_from_blocks` writes to no
    /// store, so this must be invariant across every partition (Rule AQ-5: read the store
    /// back rather than trusting the writer's own view).
    fn persisted(node: &Node) -> Self {
        Self::of(&node.state_db.load_producer_set())
    }

    fn summary(&self) -> String {
        format!(
            "count={} pending={} canonical={}B first_pubkeys=[{}]",
            self.count,
            self.pending,
            self.canonical.len(),
            self.bonds
                .iter()
                .take(3)
                .map(|(k, amt, n)| format!("{}(bond={amt},n={n})", hex::encode(&k[..4])))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

// ==================== Fixture ====================

/// A node in the R2 pre-call shape: the PRODUCTION `RocksDb` UTXO backend, a real
/// `CHAIN_LEN`-block chain applied through `apply_block`, a DENSE block store, and a
/// NON-EMPTY `ProducerSet` (the `n` genesis producers `Node::new_for_test` registers,
/// `init.rs:1141-1145`).
///
/// The chain carries coinbase-only blocks. That is deliberate and load-bearing for honesty:
/// with devnet `genesis_blocks = 40` (`network_params/defaults.rs:450`) the rebuild's genesis
/// boundary crossing at `height == genesis_blocks + 1` is never reached, so a COMPLETED
/// rebuild over this chain legitimately yields an EMPTY set. An emptied set is therefore NOT
/// evidence of the defect on its own — which is exactly why the red tests below assert
/// `Err` FIRST and then intactness: the pairing "`Err` returned AND the set was mutated" is
/// the thing that cannot be explained by correct behaviour.
async fn build_node() -> (Node, TempDir) {
    let (mut node, producers, temp) = h::make_node(3).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: the chain must reach CHAIN_LEN"
    );
    assert_eq!(
        node.sync_manager.read().await.local_tip().0,
        CHAIN_LEN,
        "fixture: sync_manager local tip must be CHAIN_LEN — `rollback_one_block` reads \
         `local_height` from there (rollback.rs:11-14), not from chain_state"
    );

    let before = ProducerContent::live(&node).await;
    assert!(
        before.count > 0,
        "fixture: the ProducerSet must be NON-EMPTY before the call, otherwise the red tests \
         could not distinguish destruction from a correctly-empty rebuild. Got {}",
        before.summary()
    );

    (node, temp)
}

/// Punch the hole: drop the canonical entries for `low..=high`.
///
/// `get_block_by_height` resolves height -> hash -> body (`block_store/queries.rs:171-177`)
/// and `ensure_blocks_present` reads the SAME height index (`queries.rs:193-209`), so
/// dropping the canonical entry makes those heights invisible to BOTH the guard and the
/// rebuild loop — the same observable shape as a node that never fetched them. Technique
/// recorded at `inc_i_152_p1_003_rollback_holed_store.rs:569`.
fn punch_hole(node: &Node, low: u64, high: u64) {
    for height in low..=high {
        let hash = node
            .block_store
            .get_hash_by_height(height)
            .expect("block_store get_hash_by_height failed")
            .unwrap_or_else(|| panic!("setup: expected a canonical entry at h={height}"));
        node.block_store
            .remove_canonical_entry(height, hash)
            .expect("remove_canonical_entry failed");
        assert!(
            node.block_store
                .get_block_by_height(height)
                .expect("block_store read failed")
                .is_none(),
            "setup: h={height} must be invisible to get_block_by_height after the hole"
        );
    }
}

/// The enabling precondition shared by I1/I2/I3: the hole is ALREADY visible to
/// `ensure_blocks_present` over the rebuild's own range, and the two heights that keep the
/// destructive path reachable (block 1, block `TARGET_HEIGHT`) are present.
fn assert_hole_precondition(node: &Node) {
    let err = node
        .block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .expect_err(
            "precondition: `ensure_blocks_present(1, TARGET_HEIGHT)` must ALREADY report the \
             hole. This is the oracle the fix hoists into rewards.rs:1105 — if it does not \
             fire here, the red test proves nothing about the guard.",
        );
    assert!(
        err.to_string().contains(&format!("height {HOLE_LOW}")),
        "precondition: the guard must name the FIRST missing height ({HOLE_LOW}); got {err}"
    );
    for h in [1u64, TARGET_HEIGHT] {
        assert!(
            node.block_store
                .get_block_by_height(h)
                .expect("block_store read failed")
                .is_some(),
            "precondition: block h={h} must be PRESENT — block 1 keeps the destructive path \
             reachable, block {TARGET_HEIGHT} keeps `rollback.rs:78`'s parent lookup alive"
        );
    }
}

fn assert_store_is_dense(node: &Node) {
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .unwrap_or_else(|e| {
            panic!("precondition: the block store must be DENSE over 1..={TARGET_HEIGHT}: {e}")
        });
}

/// O1 — the REQ-I156-006 assertion, in one place so every refusal path holds the identical
/// contract: full content snapshot before vs after, never counts alone.
fn assert_producer_set_intact(before: &ProducerContent, after: &ProducerContent, partition: &str) {
    assert_eq!(
        after.count,
        before.count,
        "[{partition}] REQ-I156-005/006 / O1: a rebuild that returned `Err` MUST NOT have \
         mutated the caller's ProducerSet. Producer COUNT moved {} -> {}. \
         `rewards.rs:1110` calls `producers.clear()` BEFORE `rewards.rs:1115` has established \
         that the block range `1..=TARGET_HEIGHT` is even walkable, so a holed store destroys \
         the live set and then aborts (destroy-then-abort). The fix hoists \
         `ensure_blocks_present(1, target_height.max(1))` to be the FIRST statement of \
         `rebuild_producer_set_from_blocks`. BEFORE: {} | AFTER: {}",
        before.count,
        after.count,
        before.summary(),
        after.summary()
    );
    assert_eq!(
        after.bonds, before.bonds,
        "[{partition}] REQ-I156-006 / O1: same pubkeys and same per-producer \
         (bond_amount, bond_count) required — a count-only match is not sufficient"
    );
    assert_eq!(
        after.pending, before.pending,
        "[{partition}] REQ-I156-006 / O1: `pending_updates` length must be unchanged. \
         `ProducerSet::clear()` drops `pending_updates` too (producer/set_core.rs:66-71) and \
         that field is NOT covered by `serialize_canonical`, so it needs its own assertion"
    );
    assert!(
        after.canonical == before.canonical,
        "[{partition}] REQ-I156-006 / O1: the ProducerSet must be BYTE-IDENTICAL to its \
         pre-call value ({} bytes expected, {} found)",
        before.canonical.len(),
        after.canonical.len()
    );
}

/// O4 — `rebuild_producer_set_from_blocks` writes to NO store. Invariant on every partition.
fn assert_no_store_write(node: &Node, before: &ProducerContent, partition: &str) {
    let persisted = ProducerContent::persisted(node);
    assert_eq!(
        persisted.canonical, before.canonical,
        "[{partition}] O4: `rebuild_producer_set_from_blocks` only READS block_store \
         (rewards.rs:1117) and must never write the ProducerSet to state_db. The persisted \
         set moved. (The emptied set becoming durable is `apply_block/mod.rs:316-318`'s doing, \
         on the NEXT applied block — not this function's.)"
    );
}

// ==========================================================================
//  I1 — THE RED TEST. Must FAIL pre-fix on O1 (intactness), not on O3.
// ==========================================================================

/// Requirement: REQ-I156-005 (Must) — path PA vs PB, partition I1.
///
/// Acceptance: "Post-fix: returns `Err` with the `ProducerSet` **identical** to its pre-call
/// value — same producer count, same pubkeys, same bond totals, same `pending_updates`
/// length."
///
/// PRE-FIX this test FAILS on the INTACTNESS assertion (O1): `Err` is returned either way —
/// `rewards.rs:1119` aborts at h={HOLE_LOW} — but pre-fix the set has already been emptied by
/// `rewards.rs:1110`. The `Err` assertion is deliberately FIRST so the failure cannot be
/// mistaken for "the function stopped erroring".
#[tokio::test]
async fn inc_i156_005_holed_store_rebuild_must_not_destroy_producer_set() {
    let (node, _tmp) = build_node().await;
    punch_hole(&node, HOLE_LOW, HOLE_HIGH);
    assert_hole_precondition(&node);

    let before = ProducerContent::live(&node).await;

    // The call under test. `&self` + a separate `&mut ProducerSet` binding — the exact shape
    // `rollback.rs:141-145` uses.
    let result = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
    };

    // O3 — unchanged by the fix: a holed store must still be refused, loudly.
    let err = result.expect_err(
        "REQ-I156-005 / O3: a rebuild over a HOLED range must return `Err`. Pre-fix this comes \
         from the loop abort at rewards.rs:1119; post-fix from the hoisted \
         `ensure_blocks_present`. Either way it is never `Ok`.",
    );

    // O1 — THE RED ASSERTION. Pre-fix: `before.count == 3`, `after.count == 0`.
    let after = ProducerContent::live(&node).await;
    assert_producer_set_intact(&before, &after, "I1 holed/direct");

    // O4 — nothing was persisted on the way through.
    assert_no_store_write(&node, &before, "I1 holed/direct");

    assert!(
        !err.to_string().is_empty(),
        "O3: the refusal must carry a non-empty diagnostic"
    );
}

// ==========================================================================
//  I2 — RED: the refusal must name the first missing height, in the GUARD's words.
// ==========================================================================

/// Requirement: REQ-I156-005 (Must) — output O3 (message text), partition I2.
///
/// Acceptance: "Post-fix: the error message names the first missing height
/// (`ensure_blocks_present` already does this, `block_store/queries.rs:201-205`) so an
/// operator can act."
///
/// Both messages name the height, so a "contains the height" assertion alone would pass
/// pre-fix and prove nothing. The assertion is therefore written against the POST-FIX guard's
/// message: `[FORK_GUARD_BACKFILL] block_store missing canonical block at height {h}
/// (range checked: {lo}..={hi})`. Pre-fix the message is `"Producer set rebuild: missing
/// block at height 6 (store corrupted)"` (`rewards.rs:1120-1123`) and this FAILS on the
/// marker — which is the point: the marker is what fleet log-scrapers key on.
///
/// AUDIT-P2-206: the two markers are DIFFERENT tokens and both must be present. The wrapped
/// `StorageError` carries the bare `[FORK_GUARD_BACKFILL]` form
/// (`crates/storage/src/block_store/queries.rs:201-205`), while the runbook
/// (`.claude/skills/node/SKILL.md:62,192`) and the sibling rollback/reorg log lines document
/// the `[FORK_GUARD_BACKFILL_REQUIRED]` form. Note `"[FORK_GUARD_BACKFILL]"` is NOT a
/// substring of `"[FORK_GUARD_BACKFILL_REQUIRED]"` (the `]` differs), so the two assertions
/// below are independent and neither can satisfy the other by accident.
#[tokio::test]
async fn inc_i156_005_holed_store_rebuild_error_names_first_missing_height() {
    let (node, _tmp) = build_node().await;
    punch_hole(&node, HOLE_LOW, HOLE_HIGH);
    assert_hole_precondition(&node);

    let err = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
            .expect_err("O3: a holed range must be refused")
    };
    let msg = err.to_string();

    assert!(
        msg.contains(&format!("height {HOLE_LOW}")),
        "REQ-I156-005 / O3: the refusal must name the FIRST missing height ({HOLE_LOW}) so an \
         operator knows what to backfill. Got: {msg}"
    );
    assert!(
        msg.contains("[FORK_GUARD_BACKFILL]"),
        "REQ-I156-005 / O3: post-fix the refusal must be the HOISTED GUARD's message \
         (`block_store/queries.rs:201-205`), which carries the `[FORK_GUARD_BACKFILL]` marker \
         and the checked range — the same marker the rollback/reorg sites already emit and \
         the one the runbook and fleet log-scrapers key on. Pre-fix the message is the \
         loop-abort text from rewards.rs:1120-1123, which means the clear() at rewards.rs:1110 \
         has ALREADY run. Got: {msg}"
    );
    assert!(
        msg.contains("[FORK_GUARD_BACKFILL_REQUIRED]"),
        "AUDIT-P2-206 / O3: the refusal must ALSO carry the `_REQUIRED` token in its own outer \
         text, so the token the runbook documents (`.claude/skills/node/SKILL.md:62,192`) and \
         the sibling rollback/reorg sites emit survives the `?` chain to whatever finally \
         reports this error. The wrapped StorageError only supplies the bare \
         `[FORK_GUARD_BACKFILL]` form, which an operator grepping the documented token misses. \
         Got: {msg}"
    );
}

// ==========================================================================
//  I3 — RED: the same harm through the REAL unguarded call site, rollback.rs:144.
// ==========================================================================

/// Requirement: REQ-I156-005 (Must) — path PA vs PB, partition I3.
///
/// Acceptance: "Undo data for the tip **exists** but its `producer_snapshot` is non-empty and
/// **not** deserializable as a `ProducerSet` — this is the only route to `rollback.rs:144`."
///
/// This is the integration half of I1: it proves the destroy-then-abort is reachable from
/// production control flow, not just by calling the helper directly. The route is narrow by
/// construction — `rollback.rs:135` short-circuits on the empty-snapshot sentinel (INC-I-071)
/// and `rollback.rs:138` takes the restore branch whenever the snapshot deserializes — so the
/// undo record is corrupted deliberately here.
///
/// PRE-FIX this FAILS on O1 exactly as I1 does; `rollback_one_block` returns `Err` on both
/// sides (the `?` at `rollback.rs:144`).
#[tokio::test]
async fn inc_i156_005_rollback_corrupt_snapshot_over_holed_store_must_not_destroy_producer_set() {
    let (mut node, _tmp) = build_node().await;

    // Route selection: keep the undo record (so the UNDO branch is taken) but make its
    // producer_snapshot non-empty and undeserializable (so `rollback.rs:141`'s else fires).
    let mut undo = node
        .state_db
        .get_undo(CHAIN_LEN)
        .unwrap_or_else(|| panic!("fixture: undo data at h={CHAIN_LEN} must exist"));
    undo.producer_snapshot = CORRUPT_SNAPSHOT.to_vec();
    node.state_db
        .put_undo(CHAIN_LEN, &undo)
        .expect("fixture: put_undo failed");
    {
        let reread = node
            .state_db
            .get_undo(CHAIN_LEN)
            .expect("fixture: undo must survive the rewrite");
        assert!(
            !reread.producer_snapshot.is_empty(),
            "fixture: an EMPTY producer_snapshot is the INC-I-071 sentinel — rollback.rs:135 \
             would skip the restore entirely and never reach rewards.rs:1105"
        );
        assert!(
            bincode::deserialize::<ProducerSet>(&reread.producer_snapshot).is_err(),
            "fixture: the snapshot must FAIL to deserialize, otherwise rollback.rs:138 \
             restores it and the rebuild fallback at rollback.rs:144 is never reached"
        );
    }

    punch_hole(&node, HOLE_LOW, HOLE_HIGH);
    assert_hole_precondition(&node);

    let before = ProducerContent::live(&node).await;

    let result = node.rollback_one_block().await;

    // O3 — the caller propagates the rebuild's `Err` either way (`?` at rollback.rs:144).
    let err = result.expect_err(
        "REQ-I156-005 / O3: `rollback_one_block` must surface the rebuild failure. Pre-fix the \
         `?` at rollback.rs:144 propagates the loop abort; post-fix it propagates the hoisted \
         guard. Neither may be swallowed.",
    );

    // O1 — THE RED ASSERTION at the real call site.
    let after = ProducerContent::live(&node).await;
    assert_producer_set_intact(&before, &after, "I3 holed/rollback.rs:144");

    // O4 — the rebuild persisted nothing. (The chain_state <-> cf_utxo inconsistency this
    // error path leaves behind pre-dates this fix and is analysis §7 out-of-scope; it is NOT
    // asserted here so this test cannot be read as blessing or condemning it.)
    assert_no_store_write(&node, &before, "I3 holed/rollback.rs:144");

    assert!(
        !err.to_string().is_empty(),
        "O3: the propagated refusal must carry a non-empty diagnostic"
    );
}

// ==========================================================================
//  I7 — RED: `.max(1)` keeps INC-I-152's "block 1 must exist" strictness.
// ==========================================================================

/// Requirement: REQ-I156-005 (Must) — path PD, partition I7.
///
/// The fix's range is `1..=target_height.max(1)`, not `1..=target_height`. Without `.max(1)`
/// the `target_height == 0` case (rolling block 1 back to genesis) would make the guard a
/// no-op — strictly WEAKER than the unconditional "block 1 must exist" pre-check INC-I-152
/// established at `rollback.rs:175-187`. This test is what makes `.max(1)` non-optional.
///
/// PRE-FIX this FAILS on O3: with `target_height == 0` the loop `1..=0` never runs, so
/// `rewards.rs:1105` clears the set and returns `Ok(())` — a silent wipe with no error at all,
/// the worst shape of the defect.
#[tokio::test]
async fn inc_i156_005_target_height_zero_with_block_one_missing_must_refuse() {
    let (node, _tmp) = build_node().await;
    punch_hole(&node, 1, 1);
    assert!(
        node.block_store
            .get_block_by_height(1)
            .expect("block_store read failed")
            .is_none(),
        "precondition: block 1 must be MISSING for this partition"
    );

    let before = ProducerContent::live(&node).await;

    let result = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, 0)
    };

    // O3 — THE RED ASSERTION for this partition.
    assert!(
        result.is_err(),
        "REQ-I156-005 / O3: `target_height == 0` with block 1 MISSING must be REFUSED. The \
         guard range is `1..=target_height.max(1)`; without `.max(1)` it degenerates to the \
         empty range `1..=0`, `ensure_blocks_present` returns `Ok(())` for `low > high` \
         (block_store/queries.rs:194-196), and the function silently clears the set and \
         returns Ok — strictly weaker than the unconditional block-1 check INC-I-152 \
         established at rollback.rs:175-187. Got: {result:?}"
    );

    // O1 — and the refusal must be non-destructive, same contract as every other refusal.
    let after = ProducerContent::live(&node).await;
    assert_producer_set_intact(&before, &after, "I7 target=0/block-1 missing");
}

// ==========================================================================
//  I4 — PASS-LOCK: dense store, happy path. GREEN pre- AND post-fix.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — path PC, partition I4. NOT a red test.
///
/// Acceptance: "happy path unchanged" — a dense range must still produce exactly the set the
/// blocks imply, with `Ok(())`.
///
/// The oracle is the function's own contract rather than a hand-predicted set: the rebuild is
/// a PURE FUNCTION of `1..=target_height`, so rebuilding into the LIVE (non-empty) set and
/// into a FRESH `ProducerSet::new()` must yield byte-identical results. That is the strongest
/// statement available without inventing an independent reimplementation, and it locks BOTH
/// halves of the contract the fix must preserve: (a) a dense range still returns `Ok(())`
/// — the hoisted guard must not over-refuse; (b) prior content still does not survive into
/// the output — the fix reorders `clear()`, it does not remove it.
///
/// Per this file's REQ-I156-007 caveat, this is asserted COMPONENT-WISE (ProducerSet content)
/// and never against a whole-encoding state root.
#[tokio::test]
async fn inc_i156_007_dense_store_rebuild_happy_path_unchanged() {
    let (node, _tmp) = build_node().await;
    assert_store_is_dense(&node);

    let before = ProducerContent::live(&node).await;
    assert!(
        before.canonical != ProducerSet::new().serialize_canonical(),
        "precondition: the live set must differ from an empty one, otherwise the \
         input-independence assertion below is vacuous. Got {}",
        before.summary()
    );

    // (a) Into the LIVE, non-empty set.
    let from_live = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
            .expect(
                "REQ-I156-007 / O3: a DENSE range must still return Ok(()). If this fails, the \
                 hoisted `ensure_blocks_present(1, target_height.max(1))` is over-refusing on a \
                 store it must admit — the range it checks is identical to the one the loop at \
                 rewards.rs:1115 walks.",
            );
        ProducerContent::of(&producers)
    };

    // (b) Into a FRESH set. Same blocks, same range, different starting content.
    let from_empty = {
        let mut scratch = ProducerSet::new();
        node.rebuild_producer_set_from_blocks(&mut scratch, TARGET_HEIGHT)
            .expect("REQ-I156-007 / O3: the same dense range must return Ok(()) for any input");
        ProducerContent::of(&scratch)
    };

    // O1 — output is a pure function of the block range.
    assert_eq!(
        from_live.canonical,
        from_empty.canonical,
        "REQ-I156-007 / O1: `rebuild_producer_set_from_blocks` must be a PURE FUNCTION of \
         `1..=target_height` — no residue of the caller's prior content may survive. The fix \
         REORDERS `clear()` behind the guard; it must not weaken or drop it. \
         from_live: {} | from_empty: {}",
        from_live.summary(),
        from_empty.summary()
    );
    assert_eq!(
        (from_live.count, from_live.pending),
        (from_empty.count, from_empty.pending),
        "REQ-I156-007 / O1: producer count and pending_updates length must also be \
         input-independent (`pending_updates` is outside `serialize_canonical`)"
    );

    // F2 (INC-I-156 M2 QA): this partition is BELOW the genesis boundary, so the two rebuilds
    // above are both legitimately EMPTY and the byte comparison, while true, is weak — see the
    // TRAP note on CHAIN_LEN. Assert that emptiness EXPLICITLY, with its cause, so the
    // comparison is never mistaken for a reconstruction lock; the non-vacuous one lives in
    // `inc_i_156_m2_dense_reconstruction.rs`.
    assert_eq!(
        from_live.canonical,
        ProducerSet::new().serialize_canonical(),
        "F2: on this fixture the rebuild is EMPTY BY DESIGN (TARGET_HEIGHT={TARGET_HEIGHT} <= \
         genesis_blocks={}), so the input-independence assertion above compares two empty \
         encodings. If this ever stops being empty the fixture crossed the genesis boundary \
         and the comparison above became a real lock — move the reconstruction assertions here \
         and delete this one. Got {}",
        node.config.network.genesis_blocks(),
        from_live.summary()
    );

    // O2 — the one interior-mutability surface, asserted UNREACHED at this height.
    assert!(
        TARGET_HEIGHT < node.config.network.genesis_blocks(),
        "O2: this partition must stay BELOW the genesis boundary (devnet genesis_blocks={}), \
         so `derive_genesis_producers_from_chain()` at rewards.rs:1336 — the only \
         interior-mutability write on `&self` — is unreachable and cannot confound the \
         input-independence assertion above",
        node.config.network.genesis_blocks()
    );

    // O4 — a completed rebuild still persists nothing by itself.
    assert_no_store_write(&node, &before, "I4 dense/direct");
}

// ==========================================================================
//  I5 — PASS-LOCK: a GUARDED caller on a dense store still completes.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — path PC through the legacy rollback branch, partition
/// I5. NOT a red test.
///
/// The legacy branch (`rollback.rs:267`) already sits behind
/// `ensure_blocks_present(1, target_height.max(1))` at `rollback.rs:175-187`. After the fix
/// the SAME range is checked twice — once by the caller, once inside the helper. This test is
/// the evidence that the now-redundant second scan changes nothing observable: the rollback
/// must still COMPLETE.
///
/// The discriminator matters. Before INC-I-204 M3 the guard's REFUSAL also returned `Ok(true)`, so
/// the return value alone could not distinguish "rolled back" from "refused and skipped". The landed
/// `best_height` / `best_hash` are what separate them, so both are asserted — component-wise,
/// per this file's REQ-I156-007 caveat (`total_work` drift is out of scope; see the module
/// doc and the M1 pin).
#[tokio::test]
async fn inc_i156_007_dense_store_guarded_legacy_rollback_still_completes() {
    let (mut node, _tmp) = build_node().await;
    assert_store_is_dense(&node);

    let target_hash = node
        .block_store
        .get_hash_by_height(TARGET_HEIGHT)
        .expect("block_store read failed")
        .expect("fixture: a canonical entry must exist at TARGET_HEIGHT");
    let utxo_before = h::utxo_content(&node).await;

    // Force the LEGACY branch: no undo -> `rollback.rs:147` falls through to the
    // rebuild-from-genesis path that carries the INC-I-152 guard.
    node.state_db.prune_undo_above(0);
    assert!(
        node.state_db.get_undo(CHAIN_LEN).is_none(),
        "fixture: undo at h={CHAIN_LEN} must be absent so the legacy branch is taken"
    );

    let rolled = node
        .rollback_one_block()
        .await
        .expect("REQ-I156-007 / O3: a dense-store legacy rollback must not error")
        == RollbackOutcome::RolledBack;
    assert!(
        rolled,
        "REQ-I156-007 / O3: `rollback_one_block` must report Ok(RolledBack)"
    );

    // Second discriminator, kept after M3 made the return value itself honest.
    let (height_after, hash_after) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        (height_after, hash_after),
        (TARGET_HEIGHT, target_hash),
        "REQ-I156-007 / O3: the rollback must have COMPLETED, not been refused. \
         the INC-I-152 guard fires on the same branch, so the \
         landed (best_height, best_hash) is the only discriminator. Landing anywhere else \
         means the now-redundant in-helper scan is refusing a store the caller's identical \
         scan already admitted."
    );

    // O1 — the ProducerSet the rebuild produced is the one that is live.
    let ps_after = ProducerContent::live(&node).await;
    let expected = {
        let mut scratch = ProducerSet::new();
        node.rebuild_producer_set_from_blocks(&mut scratch, TARGET_HEIGHT)
            .expect("oracle: the same dense range must rebuild cleanly");
        ProducerContent::of(&scratch)
    };
    assert_eq!(
        ps_after.canonical,
        expected.canonical,
        "REQ-I156-007 / O1: after a completed legacy rollback the live ProducerSet must equal \
         the rebuild over `1..={TARGET_HEIGHT}`. live: {} | expected: {}",
        ps_after.summary(),
        expected.summary()
    );

    // Component-wise UTXO lock + the shared INV-GUARD-001 / INV-SYNC-014 post-conditions.
    assert!(
        h::utxo_content(&node).await.len <= utxo_before.len,
        "REQ-I156-007: a rollback may only shrink or preserve the UTXO set at this depth \
         (before={}, after={})",
        utxo_before.len,
        h::utxo_content(&node).await.len
    );
    h::assert_utxo_invariants(&node, "I5 dense/legacy-rollback").await;
}

// ==========================================================================
//  I6 — PASS-LOCK: `target_height == 0` with block 1 PRESENT is not over-refused.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — path PD, partition I6. NOT a red test.
///
/// The twin of I7, and the reason `.max(1)` is exactly right rather than merely strict: when
/// block 1 IS present, rolling back to genesis must still be admitted. `ensure_blocks_present`
/// skips height 0 by construction (`block_store/queries.rs:197-199`), so `1..=1` is the
/// strongest range that is still satisfiable here.
///
/// GREEN on both sides: pre-fix the loop `1..=0` never runs and the function returns `Ok(())`
/// over a cleared set; post-fix the guard passes and the same thing happens. Identical
/// observable behaviour — which is precisely what a happy-path lock must pin.
#[tokio::test]
async fn inc_i156_007_target_height_zero_with_block_one_present_is_admitted() {
    let (node, _tmp) = build_node().await;
    assert!(
        node.block_store
            .get_block_by_height(1)
            .expect("block_store read failed")
            .is_some(),
        "precondition: block 1 must be PRESENT for this partition"
    );

    let mut scratch = ProducerSet::new();
    scratch
        .register_genesis_producer(*crypto::KeyPair::generate().public_key(), 1, 100_000_000)
        .expect("fixture: register_genesis_producer failed");
    let seeded = ProducerContent::of(&scratch);
    assert_eq!(
        seeded.count, 1,
        "fixture: the scratch set must be non-empty so the post-call emptiness below is an \
         observation, not a tautology"
    );

    node.rebuild_producer_set_from_blocks(&mut scratch, 0)
        .expect(
            "REQ-I156-007 / O3: `target_height == 0` with block 1 PRESENT must be ADMITTED. \
             `.max(1)` widens the guard's range to `1..=1`, which this store satisfies; a \
             refusal here would turn a legitimate rollback-to-genesis into a wedge.",
        );

    // O1 — behaviour identical pre- and post-fix: the empty range clears and rebuilds nothing.
    let after = ProducerContent::of(&scratch);
    assert_eq!(
        (after.count, after.pending),
        (0, 0),
        "REQ-I156-007 / O1: an admitted empty range must still clear the set and add nothing — \
         the fix REORDERS `clear()` behind the guard, it does not change what a successful \
         call produces. Got {}",
        after.summary()
    );
    assert_eq!(
        after.canonical,
        ProducerSet::new().serialize_canonical(),
        "REQ-I156-007 / O1: the resulting set must be byte-identical to an empty ProducerSet"
    );
}
