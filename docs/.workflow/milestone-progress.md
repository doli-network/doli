# Milestone Progress — INC-I-116 Epoch-Boundary Liveness Prune (RUN_ID=436)

Spec: `specs/epoch-liveness-prune-architecture.md`
Decisions locked at User Gate: **A1** (pure absolute `MIN_PRODUCERS_FLOOR=3`), **C1** (keep ghost exclusion).
Mode: `--fix`. Commit LOCAL only (no push). NO version bumps.

| Milestone | Scope | Behavioral change | Activation gated | Status |
|-----------|-------|-------------------|------------------|--------|
| M1 | Absolute floor (A1) + `epoch_prune_activation_height` gate + rewards.rs lockstep | YES (post-activation) | YES | COMPLETE (2026-06-18) |
| M2 | Extract `compute_live_producer_list()` + fix `rewards.rs:777` decode-list bug | NO (pure refactor) | NO | PENDING |
| M3 | Mainnet activation-height pinning | YES | — | DEFERRED (separate decision session per HC-6/INC-I-075) |
| M4 | Post-activation dead-branch cleanup | NO | — | DEFERRED (after all networks cross AH) |

In-scope this run: **M1, M2**. M3/M4 deferred.
