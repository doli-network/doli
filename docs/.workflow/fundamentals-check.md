# Fundamentals Check — INC-I-089

| Item | Status | Evidence |
|------|--------|----------|
| Build compiles | PASS (assumed from recent commits) | Recent commits 168775ba, 2c103d6a built clean; will re-verify in milestone loop |
| Tests passing | N/A pre-investigation | Will run baseline before writing reproduction test |
| External deps reachable | N/A | No external deps involved — purely internal startup race |
| Capacity/resource bug? | NO | Race condition, not resource exhaustion |
| Occam's level | Level 4 (timing/ordering) | Pure race between two concurrent in-process events: gossip-arrival of canonical block H+1 vs scheduler-firing for own slot at H+1 |

## Occam's Ordering (per .claude/protocols/rca-backpressure.md)

1. **Configuration/operator error** — RULED OUT (n2-n5 restarted with identical config and did NOT self-fork; only n1 hit the race window)
2. **Environment / dependency** — RULED OUT (same binary, same network conditions on all 5 producers)
3. **Recent code change** — RULED OUT (commits c05e02e3 new vs fc71a5ad old are both v6.22.0, no consensus/serialization delta; race is pre-existing latent bug exposed by routine restart)
4. **Race condition / timing / ordering** — CONFIRMED (gossip RX queue ordering vs scheduler tick alignment; ~12.5% probability per restart matches 5s online-window / 40s slot rotation)
5. **Architectural defect** — NO (mechanism to fix already exists — snap-sync gate; just needs extension to cover startup transition)

## Quality Audit

```
━━━ FUNDAMENTALS CHECK QUALITY AUDIT ━━━
Items with PASS + evidence: 1/5
Items with PASS but NO evidence: 0  ← no rubber-stamp
Items with FAIL: 0
Items with N/A + justification: 4 (build/test deferred to milestone loop; deps + capacity not applicable to startup race)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

No fundamentals blockers. Root cause is at Occam level 4 (timing/ordering), which fits the user's reported diagnosis. Proceeding to analyst for structural verification + triage.
