# Fundamentals Check: INC-I-061 — Delegator 90% reward sent to wrong address

**INC-ID:** INC-I-061
**Date:** 2026-05-07

## Build
- PASS — project compiled on last commit (d5707b5f). No build changes since.

## Tests
- N/A — delegation reward tests need to be written (this is new delegation feature code from recent commits)

## External Dependencies
- N/A — no external dependencies involved. Bug is in local reward calculation logic.

## Resource/Capacity
- N/A — not a resource issue. Bug is incorrect address derivation in reward outputs.

## Occam's Razor Ordering
- Level 1 (Config/Data): N/A — not a config issue
- Level 2 (Known Bug): N/A — new feature, first delegation reward epoch observed
- Level 3 (Single Code Path): **YES** — hypothesis points to single code path in `calculate_epoch_rewards()` using wrong pubkey_hash for delegator reward outputs
- Level 4 (Interaction): N/A — likely single code path
- Level 5 (Emergent): N/A — deterministic, reproducible

## Verdict
FAST path — single code path bug in reward address derivation for delegator outputs.

## Quality Self-Check

Items with PASS + evidence: 1/5
Items with N/A + justification: 4/5
Items with FAIL: 0
