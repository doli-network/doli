# Security Synthesis Reasoning Trace — INC-I-139 M6 (RUN 455)

## Mode
No `security-conclusion-*.md` files existed → fallback mode: read all 5 full reports before forming any conclusion (anti-anchoring). Synthesizer independently re-verified the four load-bearing claims by grep against the working tree.

## Auditor Reports Summary
| Perspective | Findings | Key evidence | Gaps declared |
|---|---|---|---|
| Injection | 2 P3 (dead-code inversion; 10→50 inert verification) | repo-wide caller search; read-site enumeration; single-authority setter check | discv5 grace arm sites; downstream `needs_genesis_resync` consumers |
| Auth | 1 P2 (one-way latch, self-classified PRE-EXISTING via git log -S), 1 negative verification | write/read trace of threshold; git archeology 39d500e7 | post-resync restart orchestration (would neutralize latch) |
| Crypto | 1 P3 (dead-code inversion, conf 0.6) + 3 guard-untouched verifications | git diff on consensus_target_hash/floor/finality = 0 lines | plurality-vs-majority Sybil model (pre-existing) |
| Logic | 3 P3 (10→50 verified; min(10) latent; dead-code) + race/integer/liveness analyses | exhaustive read-site enumeration; no snap-state serialization | did not execute tests; ShallowRollback completion out of scope |
| Config | 2 P3 (grace-skip benign; reset-while-disabled deviation) | governor diff empty; is_rate_governed classification; Sybil amplification directional analysis | no live-node rate measurement |

## Synthesizer Verification (grep, 2026-07-10)
1. `is_deep_fork_detected` invocations in crates/+bins/: ZERO (definition + 2 doc comments only) — cluster A confirmed.
2. Non-test `snap.threshold` reads: decision.rs:163 (`< u64::MAX`), production_gate.rs:630/732/740 (`== u64::MAX`), block_lifecycle.rs:261 (log arg); writes only 497 (MAX)/508 (50); dispatch.rs:262 is a comment — cluster B (5/5 inertness) confirmed.
3. `disable_snap_sync` non-test callers: init.rs:696 only; `enable_snap_sync` callers: production_gate.rs:750 only — latch (cluster C) mechanics confirmed.
4. `git log -S "snap.threshold = 10"` → 39d500e7 — pre-M6 emergency enable existed with identical persistence → P2-001 classified PRE-EXISTING.
(graphify not provisioned in this session; grep fallback per protocol.)

## Deduplication Log
- SEC-INJECTION-001 + SEC-CRYPTO-M6-001 + SEC-LOGIC-003 (+ config cross-signal #1) → AUDIT-P3-001. Merged evidence; severity P3 unanimous; conf lifted to 0.9 (converged, 4/5).
- SEC-INJECTION-002 + SEC-AUTH-001 + SEC-LOGIC-001 + crypto "core claims" + config pattern row → merged into Verified-Safe V1 (not a vulnerability; 5/5).
- SEC-AUTH-002 + injection residual note + logic cosmetic note → AUDIT-P2-001 (auth's grading kept; injection/logic notes counted as independent convergence on existence + pre-existing classification).
- SEC-LOGIC-002 + config dispatch pattern row + crypto min(10) analysis + auth FP-check → AUDIT-P3-003.
- SEC-CONFIG-001 + injection cross-signal (rate governor) + logic cross-signal → AUDIT-P3-004 (question raised by 2, resolved by config with cited chokepoint evidence).
- SEC-CONFIG-002 → AUDIT-P3-002 (single-auditor; synthesizer verification raised conf 0.6→0.7, keeping it out of the speculative bucket).

## Convergence Independence
All clusters: each auditor performed its own repo search/trace in its own lane; no shared derived artifact. True convergence → boosts applied (A: 0.9; B: 0.95; C: 0.8; D: 0.9; E: 0.75).

## Contradiction Analysis
Logic ("gaps<50 h>0 snap-unreachable except corroborated evidence") vs Auth+Crypto (`AllPeersBlacklistedDeepFork` emergency at `gap > 12`, cleanup.rs:445-453). Evidence quality: auth line-cited the guard and crypto independently mapped the flow (observed) vs logic's enumeration which omitted this RecoveryReason (incomplete enumeration, not counter-evidence). Resolved in auth's favor on the code fact; the INV-SYNC-011-compliance interpretation is unresolvable from code alone → SPEC-001 manual review. Both sides agree pre-existing/outside M6 diff → no gate impact.

## Coverage Analysis
All 5 perspectives substantive. Shared blind spot: node-level recovery orchestration (out of the 5-file brief scope) — material only to P2-001 real-world severity. No runtime measurement anywhere (all `observed`).

## Gate Reasoning
Gate blocks only on M6-INTRODUCED P0/P1. M6-introduced findings: P3-001 (dead code), P3-002 (inert deviation), P3-003 (bit-identical), P3-004 (intended/governed) — max P3. The only P2 is pre-existing (git-archeology-verified, 3/5 concurrence, 0 dissent). → PROCEED.
