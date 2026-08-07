//! INC-I-156 / M1 — the `UtxoSet::clear()` CONTRACT, asserted against BOTH backends.
//!
//! Requirements: REQ-I156-001 (Must), REQ-I156-002 (Must), REQ-I156-006 (Must),
//! REQ-I156-008 (Should, doc-comment truth — verified indirectly by these
//! post-conditions). Analysis: `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md`
//! §2 (R1 verification), §5 (detailed acceptance criteria).
//!
//! ## The defect
//!
//! `crates/storage/src/utxo/set.rs:68-78` — the `UtxoSet::RocksDb` arm of `clear()` is an
//! EMPTY BLOCK. The façade therefore means two different things on its two backends:
//! "empty the set" on `InMemory`, "do nothing at all" on `RocksDb`. Its doc comment
//! asserts the divergence is safe because "UtxoSet.clear() on the RocksDb variant is only
//! called during genesis reset (init.rs)". That claim is false in all three of its clauses
//! (analysis §1.3): it is also called from `rollback.rs:191` and `block_handling.rs:803`;
//! the `init.rs:112` site is undo-gap recovery, not a genesis reset; and neither rollback
//! nor reorg replaces the variant afterwards.
//!
//! This file closes the "contract absence" brittleness signal (analysis §1.8 signal 5) by
//! running ONE shared post-condition body against BOTH variants. A façade whose method has
//! a single executable contract cannot silently diverge again.
//!
//! ## Covered implementation surface
//!
//! covers: crates/storage/src/utxo/set.rs        (UtxoSet::clear — the RocksDb arm is the defect)
//! covers: crates/storage/src/state_db/writes.rs (StateDb::clear_utxos — the correct primitive to delegate to)
//!
//! ## Pre-fix expectation (the RED signal, at RUNTIME not at compile time)
//!
//! `clear_empties_the_set_rocksdb_variant`         FAILS — the set is untouched.
//! `clear_returns_a_result_that_cannot_be_swallowed` FAILS — the return type is still `()`.
//! `clear_on_failing_rocksdb_returns_err_not_panic`  FAILS — a no-op cannot report failure.
//! `clear_empties_the_set_inmemory_variant`         PASSES pre- and post-fix (regression lock).
//!
//! The file deliberately COMPILES against both the pre-fix `-> ()` signature and the
//! post-fix `-> Result<(), StorageError>` signature (see the `ClearOutcome` adapter). That
//! is a deliberate choice: a compile error is a weak red signal — it proves only that a
//! type changed, and it hides every behavioural assertion behind it. These tests fail on
//! ASSERTIONS that name the leak, which is the evidence the fix must erase.
//!
//! ============================ OUTPUT CONTRACT ============================
//!
//! OUTPUT CONTRACT: fn UtxoSet::clear(&mut self) -> Result<(), StorageError>   [post-fix]
//!                  fn UtxoSet::clear(&mut self)                              [pre-fix, the defect]
//!
//! OUTPUTS (full enumeration):
//!   O1: receiver `self` — `UtxoSet::InMemory(store)`: the backing HashMap.
//!   O2: persistent store `cf_utxo` (RocksDb arm only) — observed via `iter_all()`,
//!       `get()`, `total_value()`.
//!   O3: persistent store `cf_utxo_by_pubkey` (RocksDb arm only) — the SECONDARY INDEX.
//!       Observed independently of `cf_utxo` via `address_count()`
//!       (`state_db/queries.rs:281-304` iterates `cf_utxo_by_pubkey` DIRECTLY and counts
//!       distinct 32-byte prefixes, so an orphaned index row is visible there even though
//!       `get_by_pubkey_hash` would hide it — that read path joins back to `cf_utxo`,
//!       `queries.rs:79`).
//!   O4: in-memory counter `StateDb.utxo_count` (RocksDb arm only) — observed via
//!       `utxo_count()` / `len()`. INV-GUARD-001 requires it to equal the distinct
//!       `cf_utxo` key count at all times.
//!   O5: return value — `Result<(), StorageError>` post-fix; `()` pre-fix (REQ-I156-002:
//!       the outcome must not be silently discardable).
//!
//! PATHS:
//!   P1: success, backend writes accepted.
//!   P2: failure, the underlying store rejects the write (read-only handle standing in for
//!       ENOSPC / a failing disk — the injection technique from
//!       `disk_guardian_failsafe_test.rs:99-108`). Only reachable on the RocksDb arm.
//!
//! INPUT PARTITIONS (a partition is an input class that changes the RELATIONSHIP between
//! the asserted quantities, not merely the branch taken):
//!   P1a InMemory + N>0 entries  — O1 empties; O2/O3/O4 do not exist for this variant, so
//!                                 the façade-level reads (`len`, `total_value`,
//!                                 `address_count`) all derive from O1. Passes pre-fix.
//!   P1b RocksDb  + N>0 entries  — O2, O3 and O4 are three INDEPENDENT stores that must
//!                                 fall to zero together. This is the partition that
//!                                 distinguishes a real wipe from a `cf_utxo`-only wipe
//!                                 (which would leave orphaned index rows and a stale
//!                                 counter). **The RED partition.**
//!   P1c RocksDb  + N==0 entries — the empty-set edge (worst-case scenario #1). Clearing an
//!                                 already-empty set must be an `Ok(())` no-op, not an
//!                                 error: the relationship is "0 -> 0", not "N -> 0", so a
//!                                 fix that asserts progress would break here.
//!   P2a RocksDb  + failing store + N>0 — O5 must be `Err`; O2/O3/O4 must be BYTE-FOR-BYTE
//!                                 what they were (REQ-I156-006: full content snapshot,
//!                                 not a count). Pre-fix, a no-op trivially preserves
//!                                 content but CANNOT report the failure — so this
//!                                 partition is red on O5 only, which is exactly the
//!                                 REQ-I156-002 harm.
//!
//! MATRIX — 5 outputs × 4 partitions = 20 cells. O1 is n/a on the three RocksDb
//! partitions and O2/O3/O4 are n/a on P1a (the variant has no such stores); those cells are
//! discharged by the shared body asserting the façade-level equivalent
//! (`len`/`total_value`/`address_count`), which is the only observable either variant has.
//!   P1a: O1 ✓ (via len/iter_all/total_value/get) | O5 ✓
//!        -> `clear_empties_the_set_inmemory_variant`                      [PASS-LOCK]
//!   P1b: O2 ✓ | O3 ✓ (address_count) | O4 ✓ (utxo_count == iter_all().len()) | O5 ✓
//!        -> `clear_empties_the_set_rocksdb_variant`                       [RED]
//!   P1c: O2 ✓ | O3 ✓ | O4 ✓ | O5 ✓
//!        -> `clear_on_already_empty_rocksdb_is_ok_noop`                   [PASS-LOCK]
//!   P2a: O2 ✓ | O3 ✓ | O4 ✓ | O5 ✓ (Err, not panic)
//!        -> `clear_on_failing_rocksdb_returns_err_not_panic`              [RED on O5]
//!   O5 type pin (REQ-I156-002), asserted on its own so that the behavioural failures above
//!   are not masked by it:
//!        -> `clear_returns_a_result_that_cannot_be_swallowed`             [RED]
//!
//! SCOPE BOUNDARY — deliberately NOT duplicated here:
//!   `StateDb::clear_utxos()`'s own success and failing-DB coverage already exists at
//!   `crates/storage/tests/disk_guardian_failsafe_test.rs:283` and `:401`. This file tests
//!   the FAÇADE (`UtxoSet::clear`), which is a different unit: the defect is that the
//!   façade does not reach the primitive at all. The failing-DB test below is therefore not
//!   a duplicate — it asserts that the façade PROPAGATES what the primitive already
//!   reports.

use std::sync::Arc;

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::transaction::Output;
use storage::{Outpoint, StateDb, StorageError, UtxoEntry, UtxoSet};
use tempfile::TempDir;

// ==========================================================================
//  Signature adapter — lets this file compile against BOTH the pre-fix
//  `-> ()` and the post-fix `-> Result<(), StorageError>` signatures.
//
//  Why: the RED evidence that matters is BEHAVIOURAL ("the set is still
//  full"), and a compile error would hide every such assertion behind a type
//  mismatch. `RETURNS_RESULT` still pins the signature requirement
//  (REQ-I156-002) — as a runtime assertion, in its own test.
//
//  Post-fix the `()` impl becomes unreachable; it is kept so the adapter
//  documents both sides of the transition.
// ==========================================================================

trait ClearOutcome {
    /// `false` for the pre-fix `()` return, `true` for `Result<(), StorageError>`.
    const RETURNS_RESULT: bool;
    fn into_result(self) -> Result<(), StorageError>;
}

impl ClearOutcome for () {
    const RETURNS_RESULT: bool = false;
    fn into_result(self) -> Result<(), StorageError> {
        Ok(())
    }
}

impl ClearOutcome for Result<(), StorageError> {
    const RETURNS_RESULT: bool = true;
    fn into_result(self) -> Result<(), StorageError> {
        self
    }
}

fn returns_result<T: ClearOutcome>(_probe: &T) -> bool {
    T::RETURNS_RESULT
}

// ============================ Fixtures ============================

const PK_A: &[u8] = b"inc_i_156_owner_a";
const PK_B: &[u8] = b"inc_i_156_owner_b";

fn entry(amount: u64, pk: Hash, height: u64) -> UtxoEntry {
    UtxoEntry {
        output: Output::normal(amount, pk),
        height,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// Five UTXOs spread over TWO distinct pubkey hashes. Two addresses matter: the
/// `cf_utxo_by_pubkey` index is keyed `[pubkey_hash || outpoint]`, so a single-address
/// fixture would make `address_count()` a 1-vs-0 assertion that a partial wipe could still
/// satisfy by accident.
fn seed_pairs() -> Vec<(Outpoint, UtxoEntry)> {
    let pk_a = crypto_hash(PK_A);
    let pk_b = crypto_hash(PK_B);
    (0..5u32)
        .map(|i| {
            let pk = if i % 2 == 0 { pk_a } else { pk_b };
            (
                Outpoint::new(crypto_hash(format!("inc_i_156_utxo_{i}").as_bytes()), i),
                entry(1_000 + i as u64, pk, i as u64),
            )
        })
        .collect()
}

fn populate(set: &mut UtxoSet, pairs: &[(Outpoint, UtxoEntry)]) {
    for (op, e) in pairs {
        set.insert(*op, e.clone())
            .expect("fixture: insert into UtxoSet must succeed");
    }
}

fn rocksdb_set() -> (UtxoSet, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = StateDb::open(dir.path()).unwrap();
    (UtxoSet::from_state_db(Arc::new(db)), dir)
}

/// Reopen a seeded DB READ-ONLY. Every subsequent `db.write(batch)` returns
/// `rocksdb::Error` -> `StorageError::Database`. Technique lifted verbatim from
/// `disk_guardian_failsafe_test.rs:99-108`.
fn failing_rocksdb_set(seed: impl FnOnce(&StateDb)) -> (UtxoSet, TempDir) {
    let dir = TempDir::new().unwrap();
    {
        let db = StateDb::open(dir.path()).unwrap();
        seed(&db);
        // dropped here -> RocksDB releases the exclusive lock
    }
    let ro = StateDb::from_readonly_for_test(dir.path()).unwrap();
    (UtxoSet::from_state_db(Arc::new(ro)), dir)
}

/// Full CONTENT snapshot — outpoint set plus amounts, sorted for determinism.
/// REQ-I156-006 requires content, not counts: a count-only comparison cannot
/// distinguish "unchanged" from "one entry swapped for another".
fn content(set: &UtxoSet) -> Vec<(Outpoint, u64, Hash)> {
    let mut v: Vec<(Outpoint, u64, Hash)> = set
        .iter_all()
        .into_iter()
        .map(|(op, e)| (op, e.output.amount, e.output.pubkey_hash))
        .collect();
    v.sort_by_key(|(op, _, _)| op.to_bytes());
    v
}

// ==========================================================================
//  THE SHARED POST-CONDITION BODY (REQ-I156-001, final acceptance bullet:
//  "the same post-condition test runs against BOTH variants from one
//  parameterized body").
// ==========================================================================

/// Post-condition of a successful `clear()`, expressed once and executed against every
/// variant. Asserts O1/O2/O3/O4/O5 for the success path.
fn assert_clear_empties(set: &mut UtxoSet, variant: &str, seeded: &[(Outpoint, UtxoEntry)]) {
    let expected_before = seeded.len();
    assert_eq!(
        set.len(),
        expected_before,
        "[{variant}] fixture precondition: the set must hold {expected_before} UTXOs before \
         clear() — otherwise the post-condition below is vacuous"
    );
    assert!(
        set.total_value() > 0,
        "[{variant}] fixture precondition: total_value must be non-zero before clear()"
    );

    #[allow(clippy::let_unit_value)]
    let outcome = set.clear();
    let result = outcome.into_result();

    // ---- O5: the call reported success. ----
    assert!(
        result.is_ok(),
        "REQ-I156-001 [{variant}] / O5: clear() over a writable backend must return Ok(()), \
         got {:?}",
        result.as_ref().err().map(|e| e.to_string())
    );

    // ---- O2 (+O1 on InMemory): the entries are gone. THE assertion. ----
    let remaining = set.iter_all();
    assert!(
        remaining.is_empty(),
        "REQ-I156-001 [{variant}] / O2: clear() returned Ok(()) but the UTXO set is NOT \
         empty — {} of {} entries survived. On the RocksDb variant this is the INC-I-156 R1 \
         defect: `crates/storage/src/utxo/set.rs:68-78` matches `UtxoSet::RocksDb(_)` to an \
         EMPTY BLOCK, so the call is a silent no-op while its caller \
         (`bins/node/src/node/rollback.rs:191`, `bins/node/src/node/block_handling.rs:803`) \
         proceeds to replay 1..=target_height ON TOP of the un-cleared set — durable \
         inflation, the INC-I-041 zombie-UTXO class. The correct implementation already \
         exists and is already tested: delegate to `StateDb::clear_utxos()` \
         (`crates/storage/src/state_db/writes.rs:80-102`). Surviving outpoints (first 3): \
         {:?} | total_value still {}",
        remaining.len(),
        expected_before,
        remaining
            .iter()
            .take(3)
            .map(|(op, e)| format!("{:?}#{} amount={}", op.tx_hash, op.index, e.output.amount))
            .collect::<Vec<_>>(),
        set.total_value(),
    );

    // ---- O2: every individually-named entry is unreachable. ----
    for (op, e) in seeded {
        assert!(
            set.get(op).is_none(),
            "REQ-I156-001 [{variant}] / O2: outpoint {:?}#{} (amount {}) is still readable \
             after clear()",
            op.tx_hash,
            op.index,
            e.output.amount
        );
    }

    // ---- O4: the counter agrees with the store (INV-GUARD-001). ----
    assert_eq!(
        set.utxo_count(),
        0,
        "REQ-I156-001 [{variant}] / O4: utxo_count() must be 0 after clear()"
    );
    assert_eq!(
        set.utxo_count() as usize,
        set.iter_all().len(),
        "INV-GUARD-001 [{variant}] / O4: utxo_count() must equal the number of live entries \
         after clear()"
    );
    assert_eq!(
        set.len(),
        0,
        "REQ-I156-001 [{variant}] / O4: len() must be 0 after clear()"
    );
    assert!(
        set.is_empty(),
        "REQ-I156-001 [{variant}] / O4: is_empty() must be true after clear()"
    );
    assert_eq!(
        set.total_value(),
        0,
        "REQ-I156-001 [{variant}] / O2: total_value() must be 0 after clear()"
    );

    // ---- O3: the SECONDARY INDEX is gone too — no orphaned cf_utxo_by_pubkey rows. ----
    // `address_count()` reads cf_utxo_by_pubkey directly (queries.rs:281-304) rather than
    // joining back to cf_utxo, so it is the one public read path that can SEE an orphan.
    assert_eq!(
        set.address_count(),
        0,
        "REQ-I156-001 [{variant}] / O3: address_count() must be 0 after clear() — a non-zero \
         value means cf_utxo was wiped but cf_utxo_by_pubkey was not, leaving orphaned index \
         rows that a later insert would double-count. `StateDb::clear_utxos()` \
         (writes.rs:80-102) deletes BOTH column families in one WriteBatch; any \
         reimplementation must do the same."
    );

    // ---- O3: the by-pubkey read path returns nothing for either seeded address. ----
    for pk_seed in [PK_A, PK_B] {
        let pk = crypto_hash(pk_seed);
        assert!(
            set.get_by_pubkey_hash(&pk).is_empty(),
            "REQ-I156-001 [{variant}] / O3: get_by_pubkey_hash returned entries after clear()"
        );
    }
}

// ==========================================================================
//  P1a — InMemory variant. PASS-LOCK: must pass pre-fix AND post-fix.
// ==========================================================================

/// Requirement: REQ-I156-001 (Must) — partition P1a.
/// Acceptance: "Given a `UtxoSet::InMemory`, behavior is unchanged (`len()==0` after clear)."
///
/// REGRESSION LOCK, not a red test. It must pass on the current (broken) tree and keep
/// passing after the fix. It is the guard against "fixing" R1 by breaking the honest arm,
/// and it is one half of the single shared body demanded by REQ-I156-001.
#[test]
fn clear_empties_the_set_inmemory_variant() {
    let mut set = UtxoSet::new();
    assert!(
        !set.is_rocksdb(),
        "fixture: UtxoSet::new() must build the InMemory variant"
    );
    let pairs = seed_pairs();
    populate(&mut set, &pairs);

    assert_clear_empties(&mut set, "P1a InMemory", &pairs);
}

// ==========================================================================
//  P1b — RocksDb variant. THE RED TEST of this file.
// ==========================================================================

/// Requirement: REQ-I156-001 (Must) — partition P1b. The production variant.
/// Acceptance: "Given a `UtxoSet::RocksDb` holding N > 0 UTXOs, when `clear()` returns
/// `Ok(())`, then `iter_all().is_empty()` and `utxo_count() == 0`" and "then `cf_utxo`
/// AND `cf_utxo_by_pubkey` are both empty (no orphaned index rows)".
///
/// PRE-FIX: FAILS on O2 — `set.rs:71-76` is an empty block, so all 5 entries survive.
/// POST-FIX: passes.
///
/// This is the same executable body as the InMemory test above; only the variant differs.
/// That is the point: one contract, two backends (analysis §1.8 signal 5).
#[test]
fn clear_empties_the_set_rocksdb_variant() {
    let (mut set, _dir) = rocksdb_set();
    assert!(
        set.is_rocksdb(),
        "fixture: UtxoSet::from_state_db must build the RocksDb variant — the variant every \
         production node holds (init.rs:311, fork_recovery.rs:363). An InMemory-variant test \
         PASSES on the broken code and proves nothing."
    );
    let pairs = seed_pairs();
    populate(&mut set, &pairs);

    // Precondition: the secondary index really was written, so asserting it falls to zero
    // is not vacuous.
    assert_eq!(
        set.address_count(),
        2,
        "fixture precondition: the 5 seeded UTXOs must span 2 distinct addresses in \
         cf_utxo_by_pubkey"
    );

    assert_clear_empties(&mut set, "P1b RocksDb", &pairs);

    // INV-SYNC-014 (INC-I-118): clearing must not swap the live backend out from under the
    // node. A "fix" that replaced `*self = UtxoSet::new()` would satisfy every assertion
    // above while silently detaching the node from state_db.
    assert!(
        set.is_rocksdb(),
        "INV-SYNC-014: the variant must still be RocksDb after clear() — clear() empties the \
         backend, it does not replace it"
    );
}

// ==========================================================================
//  P1c — RocksDb, already-empty edge (worst-case scenario #1: empty input).
// ==========================================================================

/// Requirement: REQ-I156-001 (Must) — partition P1c (edge: empty set).
/// Acceptance: clearing an already-empty RocksDb set is an `Ok(())` no-op — the
/// relationship is "0 -> 0", not "N -> 0". Guards against a fix that reports failure (or
/// panics on an empty iterator) when there is nothing to delete.
///
/// PASSES pre-fix (trivially — the arm does nothing) and must keep passing post-fix.
#[test]
fn clear_on_already_empty_rocksdb_is_ok_noop() {
    let (mut set, _dir) = rocksdb_set();
    assert_eq!(set.len(), 0, "fixture: the set starts empty");

    #[allow(clippy::let_unit_value)]
    let outcome = set.clear();
    let result = outcome.into_result();

    assert!(
        result.is_ok(),
        "REQ-I156-001 / P1c / O5: clear() on an EMPTY RocksDb set must be an Ok(()) no-op, \
         got {:?}",
        result.as_ref().err().map(|e| e.to_string())
    );
    assert_eq!(set.len(), 0, "P1c / O4: len() must still be 0");
    assert_eq!(
        set.utxo_count(),
        0,
        "P1c / O4: utxo_count() must still be 0"
    );
    assert!(set.iter_all().is_empty(), "P1c / O2: cf_utxo must be empty");
    assert_eq!(
        set.address_count(),
        0,
        "P1c / O3: cf_utxo_by_pubkey must be empty"
    );
    assert!(set.is_rocksdb(), "INV-SYNC-014: variant preserved");
}

// ==========================================================================
//  REQ-I156-002 — the signature pin, asserted on its own.
// ==========================================================================

/// Requirement: REQ-I156-002 (Must).
/// Acceptance: "`pub fn clear(&mut self) -> Result<(), StorageError>`" — the outcome must
/// not be silently discardable, because a `clear()` that cannot report failure is exactly
/// how a no-op arm survived three call sites for two releases.
///
/// PRE-FIX: FAILS — `clear()` returns `()`, so `RETURNS_RESULT` resolves to the `()` impl.
/// POST-FIX: passes via the `Result<(), StorageError>` impl.
///
/// Kept in its own test so that the behavioural failures above are attributable to the
/// LEAK rather than to the type change.
#[test]
fn clear_returns_a_result_that_cannot_be_swallowed() {
    let (mut set, _dir) = rocksdb_set();

    #[allow(clippy::let_unit_value)]
    let outcome = set.clear();

    assert!(
        returns_result(&outcome),
        "REQ-I156-002 / O5: `UtxoSet::clear()` still returns `()`. It must return \
         `Result<(), StorageError>` so no call site can silently discard a failed wipe. \
         Four production call sites must then propagate: rollback.rs:191, \
         block_handling.rs:803, init.rs:112 (behind the INC-I-136 `!is_rocksdb()` fence), \
         and the `set.rs` definition itself."
    );

    // Do not leave the outcome unconsumed — post-fix an unused Result is a warning.
    outcome
        .into_result()
        .expect("REQ-I156-002: clear() over a writable backend must be Ok");
}

// ==========================================================================
//  P2a — failing store. REQ-I156-002 (Err, not panic) + REQ-I156-006 (content
//  unchanged after a refusal).
// ==========================================================================

/// Requirement: REQ-I156-002 (Must) + REQ-I156-006 (Must) — path P2, partition P2a.
/// Acceptance: "Given a read-only / failing DB, `clear()` returns `Err` and does not
/// panic", and "any refused operation must leave the UTXO set byte-for-byte unchanged —
/// pre/post snapshots compared by full content, not just counts".
///
/// PRE-FIX: FAILS on O5 — a no-op arm returns `()`, which the adapter maps to `Ok(())`;
/// the façade is structurally incapable of reporting that the wipe did not happen. (O2/O3
/// pass trivially pre-fix: a no-op changes nothing. That asymmetry is deliberate — the
/// content assertions are here to constrain the POST-fix implementation, which really does
/// write, and must not half-write on failure.)
/// POST-FIX: `StateDb::clear_utxos` builds one WriteBatch and `?`s on `db.write` — nothing
/// is applied, and the `utxo_count.store(0)` at `writes.rs:100` is never reached because it
/// sits AFTER the `?`.
///
/// NOT a duplicate of `disk_guardian_failsafe_test.rs:401`: that test pins the PRIMITIVE
/// (`StateDb::clear_utxos`). This one pins the FAÇADE — that `UtxoSet::clear` reaches the
/// primitive at all and propagates what it reports. The INC-I-156 defect is precisely a
/// façade that never reaches its primitive.
#[test]
fn clear_on_failing_rocksdb_returns_err_not_panic() {
    let pairs = seed_pairs();
    let seeded = pairs.clone();
    let (mut set, _dir) = failing_rocksdb_set(move |db| {
        for (op, e) in &seeded {
            db.insert_utxo(op, e).expect("fixture: seed write");
        }
    });

    // Precondition: the read-only handle still SEES the seeded data (reads work, writes
    // fail), so the "unchanged" assertion below is meaningful.
    let before = content(&set);
    assert_eq!(
        before.len(),
        pairs.len(),
        "fixture precondition: the read-only handle must expose all {} seeded UTXOs",
        pairs.len()
    );
    let count_before = set.utxo_count();
    let total_before = set.total_value();
    let addresses_before = set.address_count();

    #[allow(clippy::let_unit_value)]
    let outcome = set.clear();
    let result = outcome.into_result();

    // ---- O5: the failure must SURFACE, and must not abort the process. ----
    assert!(
        matches!(result, Err(StorageError::Database(_))),
        "REQ-I156-002 / P2a / O5: clear() against a failing (read-only) backend must return \
         Err(StorageError::Database(_)), got {result:?}. A `clear()` that cannot report \
         failure is how `rollback.rs:191` came to believe the set had been emptied when it \
         had not — the caller has no way to distinguish 'wiped' from 'silently skipped'."
    );

    // ---- O2/O3/O4: REQ-I156-006 — FULL CONTENT unchanged, not merely the count. ----
    let after = content(&set);
    assert_eq!(
        after,
        before,
        "REQ-I156-006 / P2a / O2: a REFUSED clear() must leave the UTXO set byte-for-byte as \
         it found it — outpoints AND amounts AND owners. {} entries before, {} after.",
        before.len(),
        after.len()
    );
    assert_eq!(
        set.utxo_count(),
        count_before,
        "REQ-I156-006 / P2a / O4: utxo_count() must not be zeroed by a refused clear() — in \
         `StateDb::clear_utxos` the `utxo_count.store(0)` at writes.rs:100 sits AFTER the \
         `self.db.write(batch)?` at writes.rs:99 and must stay there"
    );
    assert_eq!(
        set.total_value(),
        total_before,
        "REQ-I156-006 / P2a / O2: total_value() must not move on a refused clear()"
    );
    assert_eq!(
        set.address_count(),
        addresses_before,
        "REQ-I156-006 / P2a / O3: cf_utxo_by_pubkey must not be partially deleted by a \
         refused clear()"
    );
    assert!(
        set.is_rocksdb(),
        "INV-SYNC-014 / P2a: the variant must survive a refused clear()"
    );
}

// ---------------------------------------------------------------------------
// REQ-I156-008 — the doc post-condition must be TRUE, including its SCOPE
// clause. Reviewer finding F2: the first replacement comment claimed "no
// secondary index row survives", which is false — `cf_unique_id` is NOT
// touched by `StateDb::clear_utxos` (writes.rs:81-98 covers `cf_utxo` and
// `cf_utxo_by_pubkey` only), and `InMemoryUtxoStore::clear` (in_memory.rs:54)
// likewise clears only `utxos`, not `unique_ids`. Both backends agree, so the
// surviving index is deliberate, not a divergence.
//
// This test locks the corrected post-condition against the type's own public
// API. Without it, a future "cleanup" that wipes `cf_unique_id` inside
// `clear_utxos` would silently make already-minted NFT/asset/pool ids
// re-mintable on every legacy rebuild — a supply defect, not a tidiness one.
// ---------------------------------------------------------------------------
#[test]
fn clear_does_not_wipe_the_unique_id_index_rocksdb_variant() {
    let dir = TempDir::new().unwrap();
    let db = Arc::new(StateDb::open(dir.path()).unwrap());

    let minted = crypto_hash(b"inc-i-156-m1-minted-nft-id");
    db.add_unique_id(storage::UID_PREFIX_NFT, &minted);

    let mut set = UtxoSet::from_state_db(Arc::clone(&db));
    let seeded = seed_pairs();
    populate(&mut set, &seeded);

    assert!(
        set.has_unique_id(storage::UID_PREFIX_NFT, &minted),
        "fixture: the unique id must be present before clear()"
    );
    assert!(!set.is_empty(), "fixture: the set must be non-empty");

    set.clear()
        .expect("REQ-I156-001: clear() must succeed on a healthy RocksDb store");

    // The documented post-condition: the SET is empty ...
    assert_eq!(
        set.len(),
        0,
        "REQ-I156-001: clear() must empty the RocksDb UTXO set"
    );
    assert_eq!(
        set.utxo_count(),
        0,
        "REQ-I156-001: utxo_count must be reset to 0"
    );

    // ... and the unique-id index SURVIVES, exactly as the SCOPE clause of the
    // `UtxoSet::clear` doc comment states (set.rs:71-77).
    assert!(
        set.has_unique_id(storage::UID_PREFIX_NFT, &minted),
        "REQ-I156-008 / F2: `cf_unique_id` must SURVIVE clear(). `UndoData` carries no \
         unique-id field, so this index is never rolled back; wiping it here would make an \
         already-minted id re-mintable after any legacy rebuild."
    );
    assert!(
        set.is_rocksdb(),
        "INV-SYNC-014: the variant must survive clear()"
    );
}
