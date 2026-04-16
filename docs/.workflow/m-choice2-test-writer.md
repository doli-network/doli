# M-Choice2 / INC-I-034 — Test Writer Report (TDD step 1)

**Agent**: test-writer
**Milestone**: M-Choice2 — RUNTIME PERIODIC block-store integrity check (Phase 1, observability-only)
**Incident**: INC-I-034
**Spec**: `specs/scheduler-state-architecture.md`, §"Block-store integrity contract" + "CHOICE 2 — LOCKED: RUNTIME PERIODIC"
**Scope**: ONE pure scheduling helper `should_run_integrity_check` + ONE constant + ONE `Node` field. No async, no I/O, no Node instantiation in these tests — pure-function assertions only.
**Test-file**: `bins/node/src/node/periodic.rs` (appended `#[cfg(test)] mod integrity_check_tests`)

---

## 1. Output Contract enumeration (per CLAUDE.md Rule #21)

```
OUTPUT CONTRACT: fn should_run_integrity_check(
    current_tip: u64,
    last_checked_tip: Option<u64>,
    min_interval_blocks: u64,
) -> bool

Outputs (Rust identification, per .claude/protocols/output-contract.md):
  O1 mutable params     — NONE. All three parameters are by-value (`u64`, `Option<u64>`, `u64`).
                          No `&mut T`; no interior mutability (`RefCell`, `Mutex`, `RwLock`,
                          `AtomicXxx`). Nothing passed in can be mutated.
  O2 receiver / self    — NONE. Free function (no `&self`, no `&mut self`).
  O3 return             — bool: `true` => caller SHOULD run the scan; `false` => skip this tick.
                          This is the ONLY observable output.
  O4 persistent stores  — NONE. No RocksDB, no file I/O, no cache writes — the helper is a
                          pure predicate.
  O5 global / static    — NONE. No `static mut`, no `once_cell`, no env-var access.
  O6 channels / events  — NONE. No `send`, no callback, no `tokio::sync::mpsc`.

Summary: 1 observable output (O3). Helper is pure — the enumeration is exhaustive and
short by design. All mutable state (scan dispatch, log emission, updating
`last_integrity_check_tip`) is the async caller's responsibility and is out of scope for
this TDD step.
```

---

## 2. Paths × Outputs matrix

| # | Path | `current_tip` | `last_checked_tip` | `min_interval_blocks` | O3 `return` | Test name |
|---|------|---------------|--------------------|-----------------------|-------------|-----------|
| P1 | Genesis — cold start, nothing to check | `0` | `None` | `1000` | **`false`** | `p1_genesis_tip_zero_no_prior_scan_returns_false` |
| P2 | First-ever run after tip crossed threshold | `1500` | `None` | `1000` | **`true`** | `p2_first_run_tip_past_interval_returns_true` |
| P3 | Last scan too recent — skip | `1500` | `Some(1499)` | `1000` | **`false`** | `p3_last_scan_recent_returns_false` |
| P4 | Exact boundary (inclusive `>=`) | `2000` | `Some(1000)` | `1000` | **`true`** | `p4_exact_boundary_returns_true` |
| P5 | Just past boundary | `2001` | `Some(1000)` | `1000` | **`true`** | `p5_just_past_boundary_returns_true` |
| P6 | Pathological `interval=0`, no advance — busy-loop guard | `5` | `Some(5)` | `0` | **`false`** | `p6_zero_interval_no_advance_returns_false` |
| P7 | Defensive: tip moved backward (e.g. mid-rollback) | `100` | `Some(500)` | `1000` | **`false`** | `p7_tip_backward_returns_false_no_underflow` |
| P8 | Adversarial: `u64::MAX` boundary — no overflow | `u64::MAX` | `Some(u64::MAX - 1000)` | `1000` | **`true`** | `p8_u64_max_boundary_returns_true` |
| P9 | Adversarial: `u64::MAX` no advance | `u64::MAX` | `Some(u64::MAX)` | `1000` | **`false`** | `p9_u64_max_no_advance_returns_false` |

Plus a sanity test pinning the documented default constant:
`default_interval_constant_is_1000` — asserts `INTEGRITY_CHECK_INTERVAL_BLOCKS == 1000`.

**Matrix completeness**: 1 output × 9 paths = 9 cells, each with a dedicated `#[test]`. All 7 mandatory paths plus the 2 adversarial `u64::MAX` paths are covered (both were marked "trivial, add if time" in the brief; both are trivial so both shipped).

No "partial tests" — every cell has an assertion on the bool return. There are no other outputs to skip.

---

## 3. FAIL evidence (TDD red)

Tests were added to `bins/node/src/node/periodic.rs` and the module was compiled with:

```
cargo test -p doli-node --bin doli-node integrity_check_tests --no-run
```

The compilation FAILS today because neither `should_run_integrity_check` nor
`INTEGRITY_CHECK_INTERVAL_BLOCKS` exist yet — which is the expected TDD-red state.

Unique compile errors (truncated output):

```
error[E0425]: cannot find function `should_run_integrity_check` in this scope
error[E0425]: cannot find value `INTEGRITY_CHECK_INTERVAL_BLOCKS` in this scope

error: could not compile `doli-node` (bin "doli-node" test) due to 11 previous errors; 1 warning emitted
```

Sample error (P4 test site):

```
error[E0425]: cannot find function `should_run_integrity_check` in this scope
   --> bins/node/src/node/periodic.rs:868:19
    |
868 |         let ran = should_run_integrity_check(2000, Some(1000), 1000);
    |                   ^^^^^^^^^^^^^^^^^^^^^^^^^^ not found in this scope
```

Error count breakdown:
- 9 x `should_run_integrity_check` not found (one per path-test P1..P9)
- 1 x `should_run_integrity_check` not found in P8/P9 is covered by the 9 tests above
- 1 x `INTEGRITY_CHECK_INTERVAL_BLOCKS` not found in `default_interval_constant_is_1000`
- (P1 also references `INTEGRITY_CHECK_INTERVAL_BLOCKS` in its argument list, which is where the 11th error comes from.)

FAIL evidence: **CONFIRMED** — the test suite cannot compile without the helper + constant. Once the developer implements both, the suite must compile AND all 10 `#[test]` functions must pass.

---

## 4. Handoff to the Developer

The developer MUST add, in `bins/node/src/node/periodic.rs`, the following items — with these EXACT signatures and names (the tests pin them):

### 4.1 Constant (module-scope, above the impl blocks)

```rust
/// Minimum number of blocks between periodic integrity scans.
/// Default: 1000 blocks (~3h at 10s slot time on mainnet).
pub(crate) const INTEGRITY_CHECK_INTERVAL_BLOCKS: u64 = 1000;
```

### 4.2 Pure helper (module-scope, free function — NOT a method)

```rust
/// Phase-1 scheduling predicate for the periodic integrity check.
///
/// Returns `true` iff (a) we've never scanned AND the tip has already
/// advanced `min_interval_blocks` or more past genesis, OR (b) the tip has
/// advanced `min_interval_blocks` or more since the last scan.
///
/// Genesis (tip=0) always returns `false` (nothing to scan yet — no cold-start
/// log spam). A backward tip (last > current) also returns `false` — must not
/// underflow and must not re-trigger on a rollback.
///
/// Pure: no I/O, no locks, no time source. Safe to call from anywhere.
pub(crate) fn should_run_integrity_check(
    current_tip: u64,
    last_checked_tip: Option<u64>,
    min_interval_blocks: u64,
) -> bool {
    // implementation left to developer; must satisfy the 9 path rows in the
    // test matrix above (P1..P9).
}
```

The test matrix is the executable spec — the developer writes the body, runs
`cargo test -p doli-node --bin doli-node integrity_check_tests`, iterates until
green.

### 4.3 Node struct field (`bins/node/src/node/mod.rs`, in `struct Node { ... }`)

```rust
/// Last tip height at which `run_periodic_integrity_check` last executed.
/// `None` before the first scan. Updated after each scan completes (Ok or Err).
pub last_integrity_check_tip: Option<u64>,
```

Initialise to `None` in `Node::new()` and `Node::new_for_test()`.

**NOTE** — this field is NOT exercised by this TDD step's tests (the helper is pure). It is listed here so the Developer can wire the async glue in step 2 without re-scoping. The tests for the async glue (and for the CRITICAL-log emission) belong to a follow-up TDD cycle outside M-Choice2 step 1.

### 4.4 Out of scope for this step (step 2 work)

- The async method `Node::run_periodic_integrity_check()` that
  1. reads `self.chain_state.best_height`,
  2. calls `should_run_integrity_check(tip, self.last_integrity_check_tip, INTEGRITY_CHECK_INTERVAL_BLOCKS)`,
  3. on `true`, calls `self.block_store.ensure_blocks_present(1, tip)`,
  4. on `Err(StorageError::NotFound(msg))`, emits a `CRITICAL` log with the operator-action message (Phase 1 — no `HALT_PRODUCTION`, no backfill auto-dispatch),
  5. writes `self.last_integrity_check_tip = Some(tip)` regardless of scan result.
- Wiring the call site inside `run_periodic_tasks()` as a sibling of the other periodic blocks.
- Integration tests for the async glue (those belong in `bins/node/tests/`).

---

## 5. Specs gaps found

None for this scope. The spec (§"Block-store integrity contract" + "CHOICE 2 — LOCKED: RUNTIME PERIODIC") is clear about the three trigger points (startup, chain_state advance, background every 1000 blocks) and the Phase-1 observability-only stance. No contradictions with the existing `BlockStore::ensure_blocks_present` contract at `crates/storage/src/block_store/queries.rs:193`.

---

## 6. Files modified

- `bins/node/src/node/periodic.rs` — appended `mod integrity_check_tests` (10 `#[test]` functions), plus an explanatory block comment above the existing `mod tests`. No production logic changed.

## 7. Confidence

- Contract pinning: **1.0** — tests will not compile unless the exact helper signature + constant name ship. No room for Developer drift.
- Path coverage: **0.95** — 9/9 enumerated paths covered. The only thing the pure helper cannot catch (by design) is what the async caller does with the `true`; that is step 2's problem.
- FAIL→PASS evidence for M-Choice2 at conf(0.7): **READY** — FAIL state captured above; PASS will be verified after the Developer implements §4.1+§4.2 and re-runs the same command.
