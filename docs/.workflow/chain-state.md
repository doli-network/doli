# Chain State — INC-I-139 RUN 455 M7 close-out (BLOCKED on system-impact gate)

Status: M7 institutional work DONE; the multi-node system-impact gate did NOT go green → cannot commit close-out or close runs.

## Completed (persisted, correct, ungated)
- memory.db: INV-SYNC-011 updated to extended all-paths text (protection_level=3, incident=INC-I-139, M5 reset-writer enumeration amended).
- memory.db: 19 regression_tests registered (tests_inc_i139.rs class1-9 + tests_inc_i139_m6.rs RC-1/RC-2) linked INV-SYNC-011/INC-I-139.
- memory.db: 4 protection_mechanisms registered PM-016..PM-019 (domain=sync).
- docs/troubleshooting.md: 7.3 SnapSync Admission (INC-I-139) added — UNCOMMITTED.
- docs/architecture.md: 4.3 Snap-Admission Authority funnel diagram added — UNCOMMITTED.
- testnet: rolling-restarted to sha dcdd8be3 (new binary), converged h=14340 (rolling-safe validated).

## BLOCKER — system-impact suite 7/8 (row id=11, sha dcdd8be3)
- Only GS-001 fresh-genesis-boot missed: "single-block1-hash: distinct genesis=1 block1=7 (want 1/1)".
- Root cause (verified by direct getBlockByHeight(1) across nodes): nodes hold DIFFERENT stored block-1 hashes (ea1f5563, 40d83837, 9236b1cc) and some have none ("Block not found") — persisted artifact of past genesis resets + snap-sync historical-block pruning (troubleshooting 1.9). NOT caused by INC-I-139 (snap admission cannot alter a block written 14340 blocks ago). Live tip fully converged.
- All 7 INC-I-139-domain scenarios green (GS-002 no-spurious-escalation/no-empty-headers-loop, GS-003 snap epoch crossing, GS-004 fork recovery inj, GS-005 late-join, GS-006 stale-flood, GS-007 rollback-rejoin, GS-008 scale-mismatch).

## To resume / close M7 (needs a decision — NOT a code fix in M7 scope)
Option A: refine GS-001 block-1 assertion in scripts/gauntlet.sh to compare only among nodes that HAVE block 1 (exclude snap-pruned / absent), re-run → expect 8/8 → then commit close-out + close runs 455/456 + resolve INC-I-139.
Option B: genesis reset of local testnet to a single clean block-1 (REQUIRES EXPLICIT USER APPROVAL — destructive), then re-run.
Then: commit docs (message prefix "docs(sync): INC-I-139 M7 — close-out...") with Failure-Modes block; close run 455 + run 456; UPDATE incidents INC-I-139 resolved with the resolution text in the M7 task step 7.

## Deferred follow-ups
- INC-I-139 replay system-impact scenario seed (spec asks for it): NOT a <30-line seed-only add — needs a matching evaluator in scripts/gauntlet.sh, so recorded as follow-up, not added.
- AUDIT-P2-001 (emergency-enable one-way latch) — future hardening.
- Phase 2 (M8-M10, fork_choice_weight_tiebreak_activation_height wedge-escape) — separate decision session.
