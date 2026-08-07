# QA Report: INC-I-156 M2 — `rebuild_producer_set_from_blocks` guard hoist

> **Iteration 2 (post-fix re-validation).** Iteration 1 issued CONDITIONAL APPROVAL with
> OBS-001 (F1), OBS-002 (F3) and OBS-003 (F2) open. All three are now closed and independently
> re-verified. The iteration-1 findings block is preserved verbatim in
> [Appendix A](#appendix-a--iteration-1-findings-block-superseded).

```
━━━ FINDINGS — 6 total (0 blocking / 0 medium / 2 minor / 1 info / 3 resolved) ━━━

  [F1] RESOLVED conf(1.00, md5 line compare + control-flow trace + independent RED/GREEN) — bins/node/src/node/block_handling.rs:621 — the reorg caller's guard is now `ensure_blocks_present(1, target_height.max(1))`, byte-identical to bins/node/src/node/rewards.rs:1148 and bins/node/src/node/rollback.rs:177 (all three normalize to md5 0cea56af45852e01e49d6fb36716d09c). The refusal at :620-637 precedes the marker arm at :829-838 and `utxo.clear()` at :852. RED re-verified by QA: reverting only `.max(1)` makes inc_i156_f1_reorg_target_zero_missing_block_one_must_not_set_rebuild_marker FAIL on the O1 marker assertion ("Marker found armed at target_height=Some((0, 1786074944))"), not on O5.
  [F2] RESOLVED conf(1.00, measured at runtime) — bins/node/tests/inc_i_156_m2_dense_reconstruction.rs:307 — the dense happy-path lock now reconstructs a genuinely non-empty ProducerSet. QA-measured: `rebuilt(K2): count=3 pending=0 canonical=886B` against `empty_canonical=16B`, and `live == rebuilt` byte-for-byte. The vacuous comparison (rebuilt count=0 / 16B) is gone; the CHAIN_LEN=14 sibling file now names its own genesis-phase limitation at :118-131 and delegates the oracle.
  [F3] RESOLVED conf(1.00, file-read) — specs/engine-parts.md:2761, :2793, :2796 — REQ-I156-009 drift corrected. :2793 no longer claims "clears and fully replays" in the falsified order (now "dense pre-check FIRST, then clear, then replay"); :2796 documents `ensure_blocks_present(1, target_height.max(1))`; all three sibling register entries carry the identical guard string.
  [F4] MINOR conf(0.95, code-read) — bins/node/src/node/rollback.rs:111-123 and bins/node/src/node/block_handling.rs:691-715 — at two call sites the in-memory UtxoSet is already rewound BEFORE the helper is reached, so a refusal is not "UTXO byte-for-byte unchanged" at the call-site level; pre-existing and identical pre-M2, not introduced by this diff. Carried, not re-litigated.
  [F6] MINOR conf(1.00, wc -l) — bins/node/tests/inc_i_156_m2_rebuild_guard.rs:846 — 846 lines against the 800-line test-file budget (Global Rule 19). Deliberately left unsplit by the developer; the overrun is 46 lines of docstring added to close F2 honestly. Recommendation below; not a blocker.
  [F5] INFO conf(1.00, prior pinned test) — crates/core ChainState::update_tip / total_work — the orthogonal pre-existing `total_work` divergence after any rollback is carried out-of-scope per the M2 brief and stays pinned by `inc_i_156_req007_undo_based_rollback_state_root_unchanged`.

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**VERDICT: PASS** — all three **Must** requirements (REQ-I156-005, -006, -007) and the one
**Should** requirement (REQ-I156-009) are met. The iteration-1 regression F1 is closed with an
independently re-verified RED, the vacuous F2 oracle is replaced by a measured non-empty
reconstruction (count=3 / 886 canonical bytes), and the specs drift F3 is corrected. Build,
clippy `-D warnings`, and `fmt --check` all exit 0; the workspace suite is 3189 passed / 3
failed / 43 ignored, which is the 3183 baseline plus exactly the 6 new tests, with the 3
failures unchanged and pre-existing. One MINOR test-file size-budget overrun (F6) and two
carried out-of-scope items (F4, F5) remain, none blocking.

---

## Scope Validated

Worktree `/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/bugfix+inc-i-156-p2-residual-guards`, branch `bugfix/inc-i-156-p2-residual-guards`, HEAD `cba23389` + uncommitted M2 changes.

Re-measured this iteration with `git status --porcelain` and `git diff --numstat` (not taken from the task description):

| File | Status | Change |
|---|---|---|
| `bins/node/src/node/block_handling.rs` | modified | +21 / -6 |
| `bins/node/src/node/rewards.rs` | modified | +48 / -0 (purely additive — 0 deleted lines) |
| `specs/engine-parts.md` | modified | +3 / -3 |
| `bins/node/tests/inc_i_156_m2_rebuild_guard.rs` | untracked (new) | 846 lines, 7 tests |
| `bins/node/tests/inc_i_156_m2_reorg_range_parity.rs` | untracked (new) | 460 lines, 3 tests |
| `bins/node/tests/inc_i_156_m2_dense_reconstruction.rs` | untracked (new) | 450 lines, 3 tests |
| `docs/qa/inc-i-156-M2-qa-report.md` | untracked | this file |

Nothing else. `rollback.rs` is **unmodified** (`git diff --name-only -- bins/node/src/node/rollback.rs` returns 0 lines), so the iteration-1 structural analysis of S1/S2 stands unchanged.

## System Entrypoint

DOLI is a library + binary workspace; M2 touches no runtime entrypoint. Validation surface is the cargo test harness, exercised in the worktree:

```
cargo test -p doli-node --test inc_i_156_m2_rebuild_guard --test inc_i_156_m2_reorg_range_parity --test inc_i_156_m2_dense_reconstruction
cargo test -p doli-node --test <8 regression suites>
cargo test --workspace --no-fail-fast
cargo build --release ; cargo clippy --workspace --all-targets -- -D warnings ; cargo fmt --check
```

All executed by QA in this session. No node process was started: the changed function is reachable only from `rollback_one_block` / `execute_reorg`, both of which the integration tests drive directly through `Node::new_for_test`.

## Traceability Matrix Status — FINAL

| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Final verdict |
|---|---|---|---|---|---|
| REQ-I156-005 | **Must** | Yes (4 in rebuild_guard) | Yes | Yes | **PASS** — guard at `rewards.rs:1148` precedes `producers.clear()` at `:1158` and the loop at `:1163`; RED independently re-verified in iteration 1 |
| REQ-I156-006 | **Must** | Yes (full-content snapshot; extended by dense_reconstruction K3) | Yes | Yes | **PASS** — `ProducerContent` byte-compares canonical bytes + bonds + pending; K3 now proves "intact" over a range whose successful rebuild would have been *different non-empty* content. Call-site caveat F4 carried (pre-existing) |
| REQ-I156-007 | **Must** | Yes (3 pass-locks + 3 parity + 3 dense + 8 suites + workspace) | Yes | Yes | **PASS** — dense oracle is now non-vacuous (count=3 / 886B); `.max(1)` proven inert for every `target_height > 0` (parity J3); the one input that changed at the reorg sites is now guarded *before* any mutation |
| REQ-I156-009 | Should | n/a (doc) | n/a | Yes | **PASS** — `specs/engine-parts.md:2761/:2793/:2796` corrected; register is internally consistent |

### Gaps Found

**None open.** Both iteration-1 gaps are closed:

- ~~REQ-I156-009 has no artifact~~ → `specs/engine-parts.md` +3/-3, three register entries corrected.
- ~~REQ-I156-007's producer-content oracle is vacuous~~ → `inc_i_156_m2_dense_reconstruction.rs` (3 tests) reconstructs 3 producers / 886 canonical bytes.

## Iteration 2 — What Changed and How It Was Re-Verified

### F1 / OBS-001 — closed

**Range parity is now textual, not approximate.** All three guard call sites were extracted by
line number and md5'd after stripping leading whitespace:

```
bins/node/src/node/rewards.rs:1148          .ensure_blocks_present(1, target_height.max(1))   0cea56af45852e01e49d6fb36716d09c
bins/node/src/node/block_handling.rs:621    .ensure_blocks_present(1, target_height.max(1))   0cea56af45852e01e49d6fb36716d09c
bins/node/src/node/rollback.rs:177          .ensure_blocks_present(1, target_height.max(1))   0cea56af45852e01e49d6fb36716d09c
```

Identical hash — the ranges are byte-identical, not merely equivalent.

**Ordering re-confirmed at the shifted line numbers.** `execute_reorg`'s control flow on the
current tree:

| Line | Statement |
|---|---|
| `block_handling.rs:572` | `if rollback_count > 0 {` — the guarded block opens |
| `block_handling.rs:578` | `let mut rebuild_marker_armed = false;` |
| **`:620-637`** | **the dense guard + `?` — the refusal point** |
| `:680` / `:683` | `let has_undo = …` / `if has_undo {` |
| `:739` | S3 helper call (inside the `if`) |
| `:803` | `} else {` — the legacy branch opens |
| `:830` / `:838` | `set_rebuild_in_progress(target_height)` / `rebuild_marker_armed = true` |
| `:852` | `utxo.clear()` |
| `:910` | S4 helper call |
| `:964-966` | the conditional disarm |

The refusal at `:620-637` therefore precedes the marker arm (`:830/:838`) and `utxo.clear()`
(`:852`) unconditionally, on both branches. The `:739` / `:830-838` mutual exclusion is
structural: `:739` is inside `if has_undo` (`:683`–`:803`), the arm is inside the `else`
(`:803`–`:927`).

**RED independently re-verified — the developer's log was NOT trusted.** QA neutralized the
fix by reverting `block_handling.rs:621` from `target_height.max(1)` to `target_height` (one
token; the `map_err` message arms were left in place so only the guard range changed) and re-ran
the parity suite:

| Guard at `:621` | Result |
|---|---|
| Neutralized (`1, target_height`) | `2 passed; 1 failed` — the failure is `inc_i156_f1_reorg_target_zero_missing_block_one_must_not_set_rebuild_marker`, panicking at `inc_i_156_m2_reorg_range_parity.rs:296` on the **O1 marker assertion**, with `Marker found armed at target_height=Some((0, 1786074944))`. Not on O5 (`Err`) — the test's ordering discipline holds |
| Restored | `3 passed; 0 failed` |

`block_handling.rs` was restored **byte-identically** — md5 `6ce8f5d3f17fcebbb27017c2479ac398`
before and after, and `git diff --numstat` back to `21 6 bins/node/src/node/block_handling.rs`.

**Test genuinely covers it.** `inc_i_156_m2_reorg_range_parity.rs` drives the real
`Node::execute_reorg` through `ReorgResult` (not the helper directly), forces the legacy branch
with `prune_undo_above(0)` (`:156-162`), installs the **production RocksDb** UTXO backend
(`:123`) so `utxo.clear()` is a durable mutation, asserts non-empty preconditions for the UTXO
set (`:262`) and ProducerSet (`:273`) so O2/O4 cannot be vacuous, and asserts the two guard
ranges genuinely disagree on the constructed store *before* the call (`:234-253`). Six outputs
are asserted on the refusal (marker, in-memory UTXO, persisted UTXO, chain_state, ProducerSet,
height index), with O5 (`Err`) asserted **first** precisely so the RED cannot be misread.

### F2 / OBS-003 — closed

`bins/node/tests/inc_i_156_m2_dense_reconstruction.rs` (new, 450 lines, 3 tests) crosses the
genesis boundary with **real on-chain `Registration` transactions** (`:162-185`, 32-byte
`vdf_output` so `derive_genesis_producers_from_chain`'s length filter at `genesis.rs:32` accepts
them, plus a real BLS proof-of-possession).

QA measured the rebuilt set directly via a temporary instrumented probe appended to the file,
run with `--nocapture`, then removed (file restored byte-identically, md5
`ea5631c2670f654ee8b098a7785a170e`, 450 lines):

```
QA-ITER2 PROBE: genesis_blocks=40 CHAIN_LEN=44 TARGET_HEIGHT=43
  live:                          count=3 pending=0 canonical=886B
  rebuilt(K2):                   count=3 pending=0 canonical=886B
  rebuilt(K1 @genesis_blocks):   count=0 pending=0 canonical=16B
  empty_canonical=16B
```

| | Iteration 1 | Iteration 2 |
|---|---|---|
| Rebuilt producer count | **0** | **3** |
| Rebuilt canonical bytes | **16** (= the empty encoding) | **886** |
| What the byte comparison proves | empty == empty | 886 real bytes of reconstructed producer state, live-rebuild == fresh-rebuild |

The oracle is no longer vacuous: K2 asserts non-emptiness (`:321`), the exact
`(pubkey, bond_amount, bond_count)` triples against the fixture's own keys (`:345`), and
byte-identical purity across a live-set rebuild and a fresh-set rebuild (`:362`). K1 pins the
below-boundary emptiness as a **measured, attributed** property (`:259-284`) so the trap cannot
be re-set silently, and K3 re-proves REQ-I156-006's "intact" over a range whose successful
rebuild would have replaced the content with different non-empty content (`:398-450`).

`inc_i_156_m2_rebuild_guard.rs` grew 813 → 846 lines. The delta is **documentation, not new
tests** (test count unchanged at 7): `:118-131` now states outright that `CHAIN_LEN = 14` sits
inside the genesis phase, that any byte-identical-rebuild comparison inside that file is
therefore empty-vs-empty, and that the non-vacuous lock lives in the new file. That is the
correct remediation shape — the weak file labels its own limit rather than pretending otherwise.

### F3 / OBS-002 — closed

`git diff -U0 specs/engine-parts.md` touches exactly three lines: `2761`, `2793`, `2796`.

| Line | Entry | Verified |
|---|---|---|
| `:2761` | `execute_reorg()` | Now documents `ensure_blocks_present(1, target_height.max(1))` and names why the range matters at `target_height == 0` |
| `:2793` | `rebuild_producer_set_from_blocks()` | The falsified order claim is **gone** — grep for "clears and fully replays producer state by iterating" returns 0 hits. Replaced by "**dense pre-check FIRST, then clear, then replay**" plus the guard call and the `.max(1)` rationale |
| `:2796` | `rollback_one_block()` | Now documents `ensure_blocks_present(1, target_height.max(1))` and "**refuses first** … NO state mutated", which was the original §8 drift |

Register consistency: all three sibling entries now carry the **identical** guard string
`ensure_blocks_present(1, target_height.max(1))`. **REQ-I156-009 reads PASS.**

## Four-Call-Site Probe Matrix — POST-FIX

Call sites re-enumerated **per root, separately** (an aggregate scan is not a per-root fact):
`bins/` → 6 hits, of which 4 are call sites + 1 definition + 1 comment; `crates/` → grep exits 1
(**zero** call sites).

| # | Site | Path reached | Upstream guard | Upstream range | Marker armed when helper runs? | Observable behaviour change |
|---|---|---|---|---|---|---|
| S1 | `rollback.rs:144` | undo path, `producer_snapshot` present but non-deserializable | **NONE** | — | **No** — arm at `:210` is inside the `else` at `:146`; `:144` is inside the `if let Some(ref undo)` at `:106` | **YES — the intended fix.** Was destroy-then-abort; now refuse-intact |
| S2 | `rollback.rs:267` | legacy rebuild-from-genesis | `rollback.rs:177` | `(1, target_height.max(1))` — **IDENTICAL** (md5 match) | **Yes** (`:210`) | **None** except TOCTOU |
| S3 | `block_handling.rs:739` | reorg, undo path, non-deserializable snapshot | `block_handling.rs:621` | `(1, target_height.max(1))` — **IDENTICAL** (md5 match; was divergent in iter-1) | **No** — arm at `:838` is inside the `else` at `:803`; `:739` is inside the `if has_undo` at `:683` | **None** except TOCTOU |
| S4 | `block_handling.rs:910` | reorg, legacy rebuild | `block_handling.rs:621` | `(1, target_height.max(1))` — **IDENTICAL** (md5 match; was divergent in iter-1) | **Yes** (`:830`, `:838`) | **None** except TOCTOU |

### Verdict on the milestone premise

**The three previously-guarded sites (S2, S3, S4) are now provably unchanged.** Iteration 1
refuted this for S3/S4; the `.max(1)` alignment restores it. The remaining exposure at all three
is TOCTOU only — a height-index entry deleted between the caller's guard and the helper — and
pre-M2 that same TOCTOU aborted inside the helper's own replay loop at the same `?` with the same
armed marker (and at S2/S4 the UTXO replay would hit it first). No new leak.

S1 remains the single site with an observable behaviour change, which is exactly the defect M2
exists to fix, and it never arms the marker.

## Acceptance Criteria Results

### REQ-I156-005 (Must) — guard before any mutation — **PASS**

- [x] **Guard is the first statement.** `rewards.rs:1148` (`ensure_blocks_present(1, target_height.max(1))`) → `:1158` (`producers.clear()`) → `:1163` (`for height in 1..=target_height`). Verified by line number on the current tree.
- [x] **RED verified by fix neutralization** (iteration 1: guard block replaced by a bare `producers.clear();` → `3 passed; 4 failed`, the 4 failures exactly the REQ-I156-005 tests; restored → `7 passed`).
- [x] **Error names the first missing height** and carries the `[FORK_GUARD_BACKFILL]` marker (`queries.rs:201-205`).

### REQ-I156-006 (Must) — refusal leaves state byte-for-byte unchanged — **PASS**

- [x] **Full-content snapshot, not counts** — canonical bytes + `total_count()` + sorted `(pubkey, bond_amount, bond_count)` + `pending_update_count()`.
- [x] **Now proven on a non-trivial range** — dense_reconstruction K3 (`:398`) refuses over a range whose successful rebuild would have been *different non-empty* content, so "unchanged" cannot be satisfied by empty-in/empty-out.
- [x] **No store writes** — `state_db.load_producer_set()` read back on both the refusal and the success path (`:378`, `:445`).
- [!] **Call-site caveat (F4, carried, pre-existing).** At `rollback.rs:144` / `block_handling.rs:739` the in-memory UtxoSet is already rewound before the helper is reached. Identical pre-M2. Not introduced by this diff.

### REQ-I156-007 (Must) — no behaviour change on the dense/happy path — **PASS**

- [x] **Dense rebuild returns `Ok` and reconstructs real producers** — K2, measured count=3 / 886B.
- [x] **Real legacy rollback still COMPLETES** — `inc_i156_007_dense_store_guarded_legacy_rollback_still_completes`.
- [x] **`.max(1)` is inert for every `target_height > 0`** — parity J3, asserted as a precondition (`:439-443`) then driven through a real shallow `execute_reorg`.
- [x] **`target_height == 0` with block 1 present is still admitted at BOTH the helper and the reorg caller** — rebuild_guard `:770` and parity J2 (`:375`), the latter asserting the reorg actually lands on `(0, genesis_hash)` rather than being refused-and-skipped.
- [x] **8-suite regression — 33 passed / 0 failed / 1 ignored** (re-executed this iteration):

  | Suite | Result |
  |---|---|
  | `inc_i_156_m1_audit_p1_guards` | 3 passed |
  | `inc_i_156_m1_audit_p2_guards` | 4 passed |
  | `inc_i_156_m1_reorg_clear_leak` | 2 passed |
  | `inc_i_156_m1_rocksdb_clear_leak` | 3 passed |
  | `inc_i_152_p1_003_rollback_holed_store` | 3 passed |
  | `inc_i_071_undo_snapshot_sentinel` | 3 passed |
  | `fork_recovery` | 11 passed |
  | `recover_replay` | 4 passed, 1 ignored |

- [x] **M2 suites — 13 passed / 0 failed**: `inc_i_156_m2_rebuild_guard` 7, `inc_i_156_m2_reorg_range_parity` 3, `inc_i_156_m2_dense_reconstruction` 3.
- [x] **Full workspace — 3189 passed / 3 failed / 43 ignored.** Arithmetic confirmed: 3183 (iteration-1 baseline) + 6 new tests = 3189, no other delta. The 3 failures are exactly the known pre-existing set, attributed to their binaries:

  | Failure | Binary | Cause |
  |---|---|---|
  | `test_network::test_cluster_10x100` | `tests/checkpoint_rotation.rs` | `Too many open files` — macOS fd limit, environmental |
  | `test_cluster_10x100` | `tests/test_network.rs` | `Too many open files` — macOS fd limit, environmental |
  | `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` | `mempool` lib unittests | pre-existing, unrelated to M2 |

### REQ-I156-009 (Should) — specs drift corrected — **PASS**

See [F3 above](#f3--obs-002--closed). All three register entries corrected and mutually consistent.

## Build / Lint Gates

| Gate | Command | Exit |
|---|---|---|
| Build | `cargo build --release` | **0** |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **0** |
| Format | `cargo fmt --check` | **0** |

## Out-of-Scope Discipline — CONFIRMED CLEAN

`git diff --name-only` shows exactly three files. Every prohibited surface was checked, and the
two symbol hits were attributed **per file** rather than left as an aggregate:

| Surface | In diff? | Evidence |
|---|---|---|
| `crates/storage/src/state_db/writes.rs` (AUDIT-P2-105 `.flatten()`) | **No** | Not in `git diff --name-only`; `git diff --stat -- crates/storage/src/state_db/writes.rs` is empty; `.flatten()` still present at `writes.rs:160, 226` (plus 6 other sites) — untouched, correctly carried |
| `rebuild_epoch_state_from_blocks` | **No** | 0 hits in `git diff -U0 -- bins/`, 0 in `-- crates/`; the 2 hits are **prose in `specs/` only** (carried through the `:2793`/`:2796` rewrite) |
| `rebuild_producer_liveness` | **No** | 0 hits in `git diff -U0 -- bins/`, 0 in `-- crates/`; the 4 hits are **prose in `specs/` only** |
| `ChainState::update_tip` / `total_work` | **No** | 0 hits anywhere in the diff |

`rewards.rs` is **purely additive** — `git diff` shows 0 deleted lines, so the replay loop
itself is provably untouched.

## Exploratory Testing Findings — Iteration 2

| # | What was tried | Expected | Actual | Severity |
|---|---|---|---|---|
| QA2-P1 | md5 the three guard lines after whitespace normalization | Identical hashes if the ranges are truly byte-identical | All three `0cea56af45852e01e49d6fb36716d09c` | pass |
| QA2-P2 | Neutralize `block_handling.rs:621` (`.max(1)` → bare), run the parity suite | The marker test must FAIL on **O1**, not O5 | Failed at `:296` on O1, `Marker found armed at Some((0, 1786074944))`; O5 passed first, as designed | pass |
| QA2-P3 | Restore and compare md5 + `--numstat` | Byte-identical restoration | md5 `6ce8f5d…` matches; numstat back to `21 6` | pass |
| QA2-P4 | Instrumented probe printing rebuilt canonical byte length + count | Non-trivial (≫ 16 bytes, count > 0) | `count=3 canonical=886B` vs `empty=16B` | pass |
| QA2-P5 | Probe the below-boundary partition in the same run | Empty, with a named cause (not an accident) | `count=0 canonical=16B` at `target_height == genesis_blocks`, asserted explicitly by K1 | pass |
| QA2-P6 | Per-root call-site enumeration (`bins/` and `crates/` scanned separately) | 4 call sites in `bins/`, 0 in `crates/` | `bins/` 6 lines (4 calls + 1 def + 1 comment); `crates/` grep exit 1 | pass |
| QA2-P7 | Grep the diff per-file for the four prohibited symbols | 0 hits in Rust, prose-only in specs | Confirmed: 0/0 in `bins/` and `crates/`, 2 and 4 in `specs/` | pass |

## Failure-Mode Validation

| Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Holed block store mid-range (INC-I-152 shape) | Yes (rebuild_guard I1/I2/I3, dense K3) | Yes | n/a — refuses | Yes | `Err` naming the first hole, `[FORK_GUARD_BACKFILL]` marker, ProducerSet intact |
| Block store fully empty | Yes (iter-1 QA-P2) | Yes | n/a | Yes | Names height 1 |
| `target_height` beyond tip | Yes (iter-1 QA-P1) | Yes | n/a | Yes | Names tip+1 |
| Corrupt `producer_snapshot` over a holed store (real `rollback_one_block`) | Yes (rebuild_guard I3) | Yes | n/a | Yes | Reaches `rollback.rs:144` through production control flow |
| `target_height == 0`, block 1 absent, **at the helper** | Yes (rebuild_guard I7) | Yes | n/a | Yes | `.max(1)` keeps INC-I-152 strictness |
| `target_height == 0`, block 1 absent, **through real `execute_reorg`** | **Yes (parity J1 — NEW)** | Yes | n/a — refuses before any mutation | Yes | Marker DISARMED, in-memory + persisted UTXO byte-identical, chain_state intact, height index intact |
| `target_height == 0`, block 1 **present**, through real `execute_reorg` | **Yes (parity J2 — NEW)** | n/a | Completes | n/a | Lands on `(0, genesis_hash)`; the fix is an alignment, not a new refusal |
| Dense shallow reorg (`.max(1)` inert) | **Yes (parity J3 — NEW)** | n/a | Completes | n/a | Lands on the common ancestor; UTXO invariants hold |
| Rebuild crossing the genesis boundary (non-empty reconstruction) | **Yes (dense K2 — NEW)** | n/a | Completes | n/a | 3 producers / 886 canonical bytes, live-rebuild == fresh-rebuild |
| TOCTOU: hole appears between the caller's guard and the helper's | **Partially** — surrogate only (iter-1 QA-P5); a true concurrent index deletion is not constructible in-process | Yes | n/a | Yes | Guard re-evaluated per call, no caching. Ranges now identical at all three guarded sites, so this is the **only** residual divergence — and pre-M2 it failed at the same `?` with the same armed marker |
| Marker left armed after a refusal at a **guarded** site | **No longer reachable** | n/a | n/a | n/a | The iteration-1 S4 path is closed by the range alignment; verified by QA2-P2 RED/GREEN |
| Deadlock against the `producer_set` write guard | Not triggered (static) | n/a | n/a | n/a | Guard touches only `BlockStore`'s RocksDB height index — an independent lock domain. No new lock acquired |

## Security Validation

M2 changes no trust boundary: the helper ingests no external data, parses nothing, and the added
code is a read-only completeness check on locally-stored data. The iteration-2 fix strictly
improves the security posture — the one path where a node could be driven into a self-inflicted
permanent halt (S4) is removed.

| Attack surface | Test performed | Result | Notes |
|---|---|---|---|
| Attacker-induced ProducerSet destruction via a holed store | rebuild_guard I1/I3, dense K3, iter-1 QA-P2/P4 — refuse and leave the set intact | **PASS** | The vulnerability M2 closes |
| Attacker-induced production on a truncated ledger | Static trace of `rebuild_halt_reason()` + `init.rs:333`; parity J1 asserts the durable UTXO set is never emptied on a refusal | **PASS** | Strengthened — `cf_utxo` is no longer emptied on the refused path at all |
| Attacker-induced **self-inflicted halt** (durable marker armed by a refusal at a guarded site) | parity J1 (O1), RED/GREEN re-verified by QA2-P2 | **PASS** | Was the iteration-1 F1 exposure; now unreachable |
| Error-message information leak | Read the refusal text | **PASS** | Discloses only a local block height and a range — no keys, paths, or peer data |
| Injection / parsing surfaces | n/a | **Out of Scope** | The diff parses no input; no new external data path |

## Specs/Docs Drift

**None open.** The two iteration-1 drift items are corrected in this diff; re-verified by
reading `specs/engine-parts.md:2761`, `:2793`, `:2796` on the current tree.

| File | Documented behaviour | Actual behaviour | Status |
|---|---|---|---|
| `specs/engine-parts.md:2793` | "dense pre-check FIRST, then clear, then replay … `ensure_blocks_present(1, target_height.max(1))` is the first statement" | `rewards.rs:1148` guard → `:1158` `clear()` → `:1163` loop | **MATCHES** |
| `specs/engine-parts.md:2796` | "refuses first via `ensure_blocks_present(1, target_height.max(1))` … NO state mutated" | `rollback.rs:177` guard → `:210` arm → `:224` `utxo.clear()` | **MATCHES** |
| `specs/engine-parts.md:2761` | "`ensure_blocks_present(1, target_height.max(1))` … refusal precedes the marker arm and the UTXO clear even at `target_height == 0`" | `block_handling.rs:621` guard → `:830/:838` arm → `:852` clear | **MATCHES** |

## Carried Out-of-Scope Items (not re-litigated)

1. **F5 — `total_work` divergence after any rollback.** Pre-existing; every rollback path assigns `state.best_height` directly, bypassing `ChainState::update_tip`'s `total_work == height` invariant. Pinned by the M1-committed `inc_i_156_req007_undo_based_rollback_state_root_unchanged`. The M2 tests correctly work around it component-wise. Correctly classified; unchanged.
2. **F4 — in-memory UtxoSet already rewound before the helper** at `rollback.rs:144` / `block_handling.rs:739`. Pre-existing, identical pre-M2. Correctly classified; unchanged.
3. **AUDIT-P2-105** `.flatten()` in `crates/storage/src/state_db/writes.rs:160, 226` — deliberately untouched and verified untouched.

## Blocking Issues

**None.** No **Must** requirement failed, and the one **Should** requirement now passes.

## Non-Blocking Observations

- **[OBS-001] CLOSED** — `block_handling.rs:621` now uses `ensure_blocks_present(1, target_height.max(1))`. Verified by md5 parity, control-flow ordering, and an independent RED/GREEN.
- **[OBS-002] CLOSED** — `specs/engine-parts.md:2761/:2793/:2796` corrected. REQ-I156-009 passes.
- **[OBS-003] CLOSED** — `inc_i_156_m2_dense_reconstruction.rs` reconstructs 3 producers / 886 canonical bytes; the sibling file documents its own genesis-phase limit.
- **[OBS-004] OPEN (carried)** — consider restating REQ-I156-006's contract as *"unchanged by the helper"* rather than *"by any refused operation"*, since two call sites mutate the in-memory UtxoSet before the helper is reached. (F4)
- **[OBS-005] NEW, MINOR** — `bins/node/tests/inc_i_156_m2_rebuild_guard.rs` is **846 lines against the 800-line test-file budget** (Global Rule 19). The developer left it unsplit deliberately, and the 46-line overrun is the F2 remediation docstring — content that has to live next to the fixture it explains. **QA recommendation: accept as-is for this milestone, and split at the next edit** by extracting the shared `ProducerContent` observation surface and the hole-punching helpers into the existing `inc_i_156_m1_harness` module, which all three M2 files already import. That reclaims ~90 lines from `rebuild_guard.rs` and removes triplicated code across the three M2 files at the same time. Splitting now would churn a file whose RED evidence has already been captured, for no correctness gain. **Not a blocker.**
  - Related, informational only: `bins/node/src/node/rewards.rs` (1435 lines) and `bins/node/src/node/block_handling.rs` (1056 lines) both exceed the 500-line source budget, but both did so **before** M2 (1387 and 1041 respectively). Pre-existing; out of M2's scope.

## Modules Not Validated

None within M2's scope. The one probe that could not be fully executed is a **true concurrent**
TOCTOU (a height-index deletion racing between the caller's guard and the helper) — not
constructible in-process without a second writer thread; covered by a surrogate (iter-1 QA-P5)
plus static trace. With the ranges now identical at all three guarded sites, TOCTOU is the sole
residual divergence and it is not a regression.

## Final Verdict

**PASS** — All three **Must** requirements (REQ-I156-005, REQ-I156-006, REQ-I156-007) and the
**Should** requirement (REQ-I156-009) are met.

Iteration-1 findings F1, F2 and F3 are all closed, each with independent QA evidence rather than
a reading of the developer's log: F1 by md5 range parity plus a neutralize-and-restore RED that
fails on the marker assertion (`Marker found armed at Some((0, 1786074944))`) and a byte-identical
restoration (md5 `6ce8f5d3f17fcebbb27017c2479ac398`, numstat `21 6`); F2 by a runtime measurement
of the rebuilt set (`count=3 / 886 canonical bytes` against an empty encoding of 16 bytes); F3 by
reading the three corrected register entries. The milestone premise that the three
previously-guarded call sites are provably unchanged — refuted in iteration 1 — now holds, with
TOCTOU the only residual divergence and no regression against pre-M2 behaviour there.

Test evidence: M2 suites 13/13, 8-suite regression 33/33 (+1 ignored), full workspace 3189 passed
/ 3 failed / 43 ignored (= baseline 3183 + exactly the 6 new tests) with the 3 failures unchanged
and pre-existing. `cargo build --release`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo fmt --check` all exit 0. No scope creep: the diff is three files, and all four
prohibited surfaces are absent from the Rust diff in both `bins/` and `crates/`.

One MINOR observation remains (OBS-005, the 846-line test file against the 800-line budget) with
a concrete split recommendation for the next edit, plus two correctly-classified carried
out-of-scope items (F4, F5). **Approved for review.**

---

## Appendix A — Iteration 1 findings block (superseded)

Preserved verbatim for audit continuity.

```
━━━ FINDINGS — 5 total (0 blocking / 2 medium / 2 minor / 1 info) ━━━

  [F1] MEDIUM conf(0.92, code-read + control-flow trace) — bins/node/src/node/block_handling.rs:607 vs bins/node/src/node/rewards.rs:1146 — the reorg caller's guard range is `ensure_blocks_present(1, target_height)` while the hoisted helper's is `(1, target_height.max(1))`; the ranges DIVERGE at `target_height == 0`, so block_handling.rs:724/:895 are NOT provably unchanged, and at :895 a refusal leaves the durable `rebuild_in_progress` marker ARMED (disarm at :949 skipped by `?`).
  [F2] MEDIUM conf(0.99, measured at runtime) — bins/node/tests/inc_i_156_m2_rebuild_guard.rs:116 (CHAIN_LEN=14) vs genesis_blocks=40 — every dense rebuild in the M2 suite reconstructs an EMPTY ProducerSet (live count=3 / 742 canonical bytes vs rebuilt count=0 / 16 bytes), so the REQ-I156-007 "pure function / byte-identical" oracle compares empty == empty and never exercises the replay loop's producer reconstruction.
  [F3] MINOR conf(1.00, file-read) — specs/engine-parts.md:2793 and :2796 — REQ-I156-009 drift is NOT corrected and M2 made :2793 newly false ("clears and fully replays" no longer describes the order); :2796 still omits `ensure_blocks_present` that the sibling entry at :2761 documents.
  [F4] MINOR conf(0.95, code-read) — bins/node/src/node/rollback.rs:111-123 and bins/node/src/node/block_handling.rs:676-700 — at two call sites the in-memory UtxoSet is already rewound BEFORE the helper is reached, so a refusal is not "UTXO byte-for-byte unchanged" at the call-site level; pre-existing and identical pre-M2, not introduced by this diff.
  [F5] INFO conf(1.00, prior pinned test) — crates/core ChainState::update_tip / total_work — the orthogonal pre-existing `total_work` divergence after any rollback is carried out-of-scope per the M2 brief and stays pinned by `inc_i_156_req007_undo_based_rollback_state_root_unchanged`.

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Iteration 1 verdict: CONDITIONAL APPROVAL.** Superseded by the PASS above.
