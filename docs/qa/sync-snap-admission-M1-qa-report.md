# QA Report: INC-I-139 M1 — Snap-Admission Regression Suite (TESTS-ONLY, run_id 455)

## Scope Validated
- New test file `crates/network/src/sync/manager/tests_inc_i139.rs` (10 tests) + registration in `mod.rs`.
- Requirements REQ-SNAP-001/002/003/007/010 (`docs/redesigns/sync-snap-admission-redesign-analysis.md` L205-266).
- Regression classes 1-8, Extended Invariant INV-SYNC-011 (`specs/sync-snap-admission-architecture.md` L262-274).
- This is a redesign TESTS-ONLY milestone: the deliverable is the test suite, not a feature. QA verifies the tests correctly encode the acceptance contract.

## Summary
**PASS.** The suite exhibits the exact FAIL/PASS profile the milestone defines as done. The 7 behavior-locking tests pass against current unchanged code; the 3 FAIL-by-design tests are committed `#[ignore]` and, when de-ignored, each fails on its REAL DC assertion (not a setup artifact), with cited source lines verified to exist in the codebase. No consensus/source code outside test scaffolding was modified.

## System Entrypoint
- Non-ignored: `cargo test -p network --lib -- sync::manager::tests_inc_i139` → 7 passed, 3 ignored.
- Ignored (de-ignored run): `cargo test -p network --lib -- --ignored sync::manager::tests_inc_i139` → 3 failed (by design).

## Acceptance-Contract Verification

### 1. FAIL/PASS profile (VERIFIED)
| Test | Class | Expected | Observed |
|------|-------|----------|----------|
| class1_n4_wedge_parks_no_snap_below_gap_50 | 1a | PASS | PASS |
| class1_evidence_gated_snap_only_at_gap_50_plus_empties | 1b | PASS | PASS |
| class5_inc_i138_replay_gap28_no_genesis_resync | 5 | PASS | PASS |
| class6_fresh_bootstrap_snaps_via_route_c | 6 | PASS | PASS |
| class7_epoch_boundary_wedge_no_snap | 7a | PASS | PASS |
| class7_epoch_boundary_gap28_no_genesis_resync | 7b | PASS | PASS |
| class8_b7_height_offset_floor_gated_refused | 8 | PASS | PASS |
| class2_n1_bare_gap_51_must_not_snap | 2 | FAIL-by-design, ignored (DC-1/M3) | ignored; FAILS when run |
| class3_counter_not_starved_by_dispatch84_reset | 3 | FAIL-by-design, ignored (DC-4/M5) | ignored; FAILS when run |
| class4_floor_gap500_coordinator_snap_passes_gate1 | 4 | FAIL-by-design, ignored (DC-2/M2) | ignored; FAILS when run |

7 non-ignored PASS + 3 ignored — matches the contract exactly. `#[ignore]` reason strings correctly name the landing milestone (M3/M5/M2).

### 2. Ignored tests fail on their REAL DC assertion (VERIFIED)
- **Class 2** panics at `tests_inc_i139.rs:185` — the `!snapped` assertion — because Route A (`sync_engine/decision.rs:168 || gap > self.snap.threshold`) admits snap at bare gap 51. Fails on the load-bearing assertion, not a precondition `.expect()`.
- **Class 3** panics at `:244` with `consecutive_empty_headers maxed at 1 across 15 cycles` — proving `sync_engine/dispatch.rs:84` unconditionally resets the counter each cycle. The "1" is the genuine runtime behavior of the DC-4 target, not a spurious artifact.
- **Class 4** panics at `:291` — `request_genesis_resync` returned false — because Gate 1 (`production_gate.rs:674 confirmed_height_floor>0 && !is_emergency`) refuses `CoordinatorSnapEscalation` (not in the emergency set). Real DC-2 gate.

All three cited source locations were independently confirmed to exist: `sync_engine/decision.rs:168`, `sync_engine/dispatch.rs:84`, `production_gate.rs:674`. None of the three fails on a setup/quorum artifact.

### 3. Output Contract & citation hygiene (VERIFIED)
Every test carries an accurate `// OUTPUT CONTRACT:` block (outputs O1/O2 × paths × input partitions × matrix) and cites its class number + REQ-SNAP id + incident (INC-I-138/139/012). Contract targets match the asserted observable (e.g., Class 2 documents `pipeline_data` as the reachable proxy for the SnapCollecting X1 transition, with the rationale noted).

### 4. Traceability (one labeling gap — non-blocking)
| REQ | Covered by | Status |
|-----|-----------|--------|
| REQ-SNAP-002 | Classes 1, 2, 5, 7 | Covered, cited |
| REQ-SNAP-003 | Classes 4, 6 | Covered, cited |
| REQ-SNAP-007 | Class 3 | Covered, cited |
| REQ-SNAP-010 | Class 8 | Covered, cited |
| REQ-SNAP-001 | (Class 2 de facto) | **No test header cites REQ-SNAP-001** |

**OBS-001 (non-blocking):** REQ-SNAP-001 ("single guarded admission chokepoint") is not explicitly cited by any test. Its behavioral acceptance criterion — "Route A bare-gap no longer admits snap without the same guard as Route B" — is exercised by Class 2 (which locks exactly that Route-A kill). Its primary AC, however, is an *enumeration proof* ("exactly one code path sets `SnapCollecting`"), which is a static/architectural verification that lands with the DC-1 implementation (M3), not a runtime unit test. This is a defensible boundary for a TESTS-ONLY M1, but the traceability link would be clearer if Class 2's header also cited REQ-SNAP-001.

### 5. Class 7 epoch-boundary honesty (VERIFIED)
The `CLASS 7 GAP` comment (`tests_inc_i139.rs:386-394`) honestly states that the SyncManager unit layer cannot simulate the true trigger — `GetHeaders(canonical-start-hash)` returning 0 headers at an epoch boundary — because header generation / block_store lookups live at the node/block_store layer. It reproduces the resulting evidence shape (empty headers + gap at epoch-boundary heights 36/64) and explains why FAIL-class variants are not duplicated (gap/evidence-driven admission is epoch-position-invariant). Accurate, no overclaim.

### 6. Source-code containment (VERIFIED)
Only Rust changes are `crates/network/src/sync/manager/mod.rs` (a single `#[cfg(test)] mod tests_inc_i139;` line) and the new `tests_inc_i139.rs`. `docs/.workflow/test-m3-contract.rs` is an uncompiled workflow scratch file outside any crate. No consensus/validation/production source file was touched. Remaining diff entries (`.claude/skills/*`, `CLAUDE.md`, `docs/DOCS.md`, `specs/SPECS.md`) are documentation/index artifacts, not source.

## Specs/Docs Drift
None introduced by this milestone. Pre-existing INV-SYNC-011 incompleteness is already tracked in the analysis doc (REQ-SNAP-010) and is the target of M3, not an M1 regression.

## Blocking Issues
None. No Must-requirement test regressed; the FAIL-by-design profile is intended and correctly quarantined by `#[ignore]`.

## Non-Blocking Observations
- **OBS-001**: REQ-SNAP-001 has no test that explicitly cites it (covered behaviorally by Class 2; its enumeration-proof AC is a static M3 concern). Recommend adding a `REQ-SNAP-001` citation to Class 2's header.

## Final Verdict
**PASS** — All behavior-locking tests pass, all FAIL-by-design tests fail on their genuine DC assertions with verified source citations, contracts and incident traceability are accurate, and no source outside test code was modified. The suite meets the milestone's definition of done. Sole caveat: REQ-SNAP-001 is covered only behaviorally (via Class 2) and not explicitly cited — a labeling refinement, not a coverage gap.
