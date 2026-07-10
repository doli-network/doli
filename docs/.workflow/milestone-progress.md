# Milestone Progress — INC-I-139 Phase 1 (RUN 455, redesign --fix)

Spec: specs/sync-snap-admission-architecture.md · Analysis: docs/redesigns/sync-snap-admission-redesign-analysis.md
Workflow type: redesign · Scope: crates/network/src/sync/ + bins/node production_gate/block_lifecycle
Phase 2 (M8-M10, AH-gated wedge-escape) explicitly OUT of this run — separate decision session.

| ID | Name | Scope | Depends | Status |
|----|------|-------|---------|--------|
| M1 | Regression test suite (classes 1-8; 2, 3 & 4 FAIL-by-design against current code) | tests only | — | COMPLETE (2026-07-10) |
| M2+M3 | DC-2 floor-gate forward exemption + DC-1 delete Route A (ATOMIC, one commit) | production_gate.rs:666-681, decision.rs:164-169 | M1 | PENDING |
| M4 | DC-3 delete A1 redirect (keep regime guards :118-129, :144-152) | dispatch.rs:96-117 | M2+M3 | PENDING |
| M5 | DC-4 counter single-owner (remove dispatch.rs:84 reset, keep :83) + co-test suite | dispatch.rs:84 | M4 | PENDING |
| M6 | RC-1 threshold demotion + discv5-grace h==0 gate + RC-2 emergency taxonomy sentinel | types.rs:468, decision.rs:163/179/204, production_gate.rs:729 | M5 | PENDING |
| M7 | Close-out: extend INV-SYNC-011, register regression_tests + protection_mechanisms, docs, gauntlet run | memory.db, docs/ | M6 | PENDING |

## M1 outcome (RUN 455, 2026-07-10)
- Deliverable: `crates/network/src/sync/manager/tests_inc_i139.rs` (10 tests / 8 classes) + `mod.rs` registration. TESTS-ONLY, zero source-logic change.
- FAIL/PASS profile (verified against current code): 7 non-ignored PASS (classes 1×2, 5, 6, 7×2, 8 — lock current good behavior); 3 `#[ignore]` FAIL-by-design reproductions:
  - class2 (DC-1 → de-ignored in M3): Route A bare-gap `decision.rs:168` admits snap at gap 51.
  - class3 (DC-4 → de-ignored in M5): `dispatch.rs:84` unconditional counter reset starves the evidence counter.
  - class4 (DC-2 → de-ignored in M2): Gate 1 `production_gate.rs:674` refuses CoordinatorSnapEscalation — proves DC-2 is load-bearing.
- Full `network` crate suite green: 440 passed, 0 failed, 4 ignored. fmt + clippy clean.
- Class 7 GAP: SyncManager unit layer cannot simulate true epoch-boundary GetHeaders-returns-empty (node/block_store layer); feasible PASS-variants covered, gap documented in the test file.
- QA: PASS (`docs/qa/sync-snap-admission-M1-qa-report.md`). Review: APPROVED, AUDIT-SKIP (`docs/reviews/sync-snap-admission-M1-redesign-review.md`).
