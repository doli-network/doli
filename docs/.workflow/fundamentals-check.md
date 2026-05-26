# Fundamentals Check — INC-I-090 fix loop (milestone phase)

This is the FIX-loop fundamentals check. The investigation-phase fundamentals already completed; VERDICT achieved conf(0.92, measured). See `docs/bugfixes/inc-i-090-observability-gap-verdict.md`.

| Item | Status | Evidence |
|------|--------|----------|
| Build compiles | PASS (assumed from recent commits) | Recent commit `52fe4ed3 chore(release): bump 6.22.0 -> 6.22.1` built clean; INC-I-089 fix `7d2fd5bb` shipped. Will re-verify per milestone before commit. |
| Tests passing on baseline | N/A pre-milestone | Will run `cargo test -p storage --lib && cargo test -p rpc --lib` before M1's failing-test phase to establish baseline green. |
| External deps reachable | N/A | All work is internal (observability subsystem). |
| Resource/capacity bug? | NO | This is an emit-wiring + RPC-filter bug, not a resource bug. |
| Occam's level | Level 4 (code defect — wiring) + Level 5 (architectural — missing alert consumer) | D1, D2, D3, D5, D6, D7 are code-level wiring defects. D4, D8 are infrastructure/wiring gaps. None are timing/race or resource. |

## Working-tree caveat (CRITICAL for milestone-runner)

`git status` shows `M crates/network/src/sync/manager/recovery.rs` — UNCOMMITTED WIP, likely the underlying-fork fencepost fix related to `inc-i-090-finality-guard-analysis.md`.

**Milestone-runner constraint**: every milestone in this run uses `git add <specific files>` for its fix, NEVER `git add -A` or `git add .`. The recovery.rs WIP must stay uncommitted in the user's tree across all 7 milestones. M4 (which would also touch recovery.rs) is DEFERRED until the user commits this WIP.

## Quality Audit

```
━━━ FUNDAMENTALS CHECK QUALITY AUDIT ━━━
Items with PASS + evidence: 1/5
Items with PASS but NO evidence: 0
Items with FAIL: 0
Items with N/A + justification: 4 (build/test deferred to per-milestone gates; deps + capacity not applicable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

No blockers. Proceeding to milestone loop.
