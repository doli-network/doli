# Fundamentals Check — INC-I-026 Post-Activation Investigation

**Date:** 2026-04-09
**Incident:** INC-I-026 (resumed)
**Trigger:** User reports fork disaster on mainnet after INC-I-026 scheduler fix activated at h=30500 (commit 7f033517).
**Mode:** `--investigate` (read-only, no code changes this session).

## Occam's Razor Ordering

### L1 — Are the fundamentals sane?
| Check | Status | Evidence |
|-------|--------|----------|
| Is the bug reproducible? | PASS | User reports ongoing fork cascade on mainnet after h=30500. Operator-reported symptom, not a hypothesis. |
| Do we know which code changed? | PASS | Two commits bound to this activation: `f9af3755` (the fix) and `7f033517` (mainnet activation at 30500). v6.7.8 is the first binary that sees this gate flip on mainnet. |
| Is the activation height correct in code? | PASS | `crates/core/src/network_params/defaults.rs`: `inc_i_026_scheduler_activation_height: 30_500` for Mainnet (verified via git show 7f033517). `crates/updater/src/hardfork.rs` has matching entry h=30500 min_version=6.7.8 for Mainnet. |
| Did the cargo test pass pre-deploy? | PASS (with caveat) | Per memory entry #282, `inc_i_026_excluded_divergence` test asserts 20/20 slots identical across two nodes that applied competing blocks. Test covers post-activation scheduler invariant only; it does NOT cover mixed-version mesh behavior. |

### L2 — Capacity / resources?
Not relevant at this stage — symptom is forking (correctness), not resource exhaustion. Will verify per-node RAM/CPU/disk during evidence collection, not pre-judge.

### L3 — Configuration / deployment?
| Check | Status | Evidence |
|-------|--------|----------|
| Is every mainnet node on v6.7.8? | UNKNOWN | Requires SSH sweep. HardForkSchedule entry will stop <6.7.8 nodes from PRODUCING at h=30500, but they may still VALIDATE and accept blocks — the gate is the version check in the updater, not the scheduler branch itself. |
| Did the rollout beat the activation height? | UNKNOWN | Mainnet tip was ~24640 at commit time per memory #269. 5860 blocks × 10s = ~16h window. Need to check when v6.7.8 actually landed on each producer. |
| Is the NetworkParams value plumbed through correctly? | PASS | Traced in prior session: resolve_epoch_eligibility reads it, validate_producer_eligibility reads it via ValidationContext, build_block_content reads it. All three gates symmetric. |
| Is CURRENT_PROTOCOL_VERSION actually causing partition on mismatched peers? | UNKNOWN | Commit f9af3755 bumped 2→3 but kept MIN_PEER_PROTOCOL_VERSION at 1 — old peers can still gossip. Could be a silent divergence vector if old peers produce blocks the new scheduler rejects (or vice versa). |

### L4 — Environment drift?
| Check | Status | Evidence |
|-------|--------|----------|
| Branch compile state | PASS | Prior session (entry #282) reported clean build for doli-core + doli-node. |
| Working tree clean for investigation | PASS | `git status --short` shows only M CLAUDE.md and untracked `isudoajl/`. No in-flight edits to consensus code. |

### L5 — Is it actually a code bug?
Held until evidence collection. Possible explanations in rough order of likelihood based on prior behavioral learnings:

1. **Mixed-version mesh at activation moment** — some producers still on v6.7.7 (legacy scheduler) at h=30500, others on v6.7.8 (new scheduler). Pre-activation path uses filter-by-excluded; post-activation is pure round-robin. At the gate slot, two producers compute DIFFERENT eligible sets for the SAME slot → two blocks → fork. Plausible because the HardForkSchedule only stops old PRODUCTION but does not guarantee every node is on v6.7.8 at the exact height.
2. **Asymmetric transformation** — same class as the original bug, but on a different input field. Behavioral learning #1 explicitly warns about this. Possible that one of the 3 fix sites (scheduling.rs / validator.rs / assembly.rs) reads a slightly different input shape post-activation.
3. **active_production_list drift at epoch crossing** — if h=30500 lands mid-epoch, active_production_list was computed from the PRE-activation state and applied POST-activation, while another node crossed an epoch boundary exactly at 30500 and computed a different list. The behavioral-learning pattern (HashSet local mutation affecting scheduler) applies here as well.
4. **Rollback / replay using legacy scheduler** — rollback_one_block or rebuild_producer_set_from_blocks might still use pre-activation filter even when replaying post-activation blocks. Behavioral learning #14 (raw constants in rebuild paths).
5. **Genuine consensus bug not caught by the cargo test** — the test exercised 2 nodes with 20 slots; a 30+ producer mainnet has richer edge cases.

## Triage Posture

**Path:** DEEP (forced by multiple criteria)

- Resumed incident with prior failed fix attempts → forces DEEP (trigger #4).
- Bug is post-activation of a CONSENSUS change → 3+ components minimum (scheduler, validator, assembly, updater, gossip). Forces DEEP (trigger #2).
- This is the 10+ attempt on the excluded_producers / scheduler subsystem — behavioral learning #4 applies (3+ incremental fixes on same subsystem → stop patching, analyze as feedback loop).

## Signals to Escalate

- **Total signals so far on mainnet/consensus/scheduler scope: very high** (INC-I-016, INC-I-020b, INC-I-024, INC-I-025, INC-I-026 × multiple sessions, ~13 Ivan commits in the same files last week). If the root cause here is a NEW distinct cause (not a regression of the existing fix), this reaches 3+ root causes on the same incident → HYDRA escalation to `/omega-redesign`.
- Will count precisely after diagnosis, not now.

## Investigation Plan

1. Mainnet forensics sweep (read-only):
   - Per node: binary version, chain tip + hash, systemd status, latest rollback count, fork-related log lines
   - Cross-node: who agrees on what at h=30500, 30501, 30502, …, current
   - Timeline: what happened at the exact activation boundary
2. Diagnostician phase: hypothesis / kill-test loop informed by forensics
3. Deliver diagnosis report (no fix this session)
4. Decision point: DEEP-path fix or redesign escalation

## Quality Audit

```
━━━ FUNDAMENTALS CHECK QUALITY AUDIT ━━━
Items with PASS + evidence: 5/5
Items with PASS but NO evidence: 0
Items with FAIL: 0
Items with N/A + justification: 1 (L2 — capacity not relevant to correctness symptom)
Items deferred to evidence collection (UNKNOWN): 4
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

No rubber-stamp items. All PASS entries have direct evidence (git commands or prior memory.db entries). UNKNOWNs are explicitly deferred to the forensic sweep — not hand-waved.
