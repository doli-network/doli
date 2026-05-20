# INC-I-082 Analysis — Snap-Sync Safety Contract on Reorg/Rollback Paths

> Source: Analyst subagent (read-only), persisted by orchestrator. Branch HEAD: `hotfix/inc-i-078-delegation-auth-and-cap`. INVESTIGATE-ONLY.

## 1. Git History Reconstruction

**Commit 3ee3a7c6 — drift confirmed (framing, not factual):** git subject is `fix(consensus): decouple EPOCH_STATE_FORMAT_VERSION from protocol version (INC-I-054)`. On HEAD: the INC-I-054 safety guard IS present at `rewards.rs:530-558`; `delete_epoch_state()` call IS removed from init.rs (`init.rs:742-766` logs version mismatch, does NOT delete). `delete_epoch_state()` still exists at `crates/storage/src/state_db/queries.rs:402` with ZERO production callers.

**Four regression commits — all present & active on HEAD:**
| Commit | Fix | Evidence on HEAD |
|--------|-----|-----------------|
| b3e368d3 | #4A 3-epoch lookback | `rewards.rs:672-682` cites "Fix #4A (2026-04-15, synmgrefactor)" |
| 1f85c965 | #4B tier accumulators | `rewards.rs:617-627` cites "Fix #4B" |
| 084dcb89 | #4C current-epoch replay | `rewards.rs:894-909` cites "Fix #4C" |
| f677e9b5 | #5 rollback wiring | `rollback.rs:243-277` epoch_state undo restore + rebuild fallback |

User line-number hints accurate (≤8 line drift, functionally irrelevant).

## 2. Architecture

`rebuild_epoch_state_from_blocks()` @ `rewards.rs:513`, async, DEPRECATED doc comment. Upfront guard @ `rewards.rs:530-558`: `has_incomplete_history = get_block_by_height(1).is_none() || get_block_by_height(lookback_start).is_none()`. On snap-synced nodes block 1 is missing → `has_incomplete_history = true` always.

**Fallback branches (rewards.rs:647-770):**
1. `epoch<=1` → all active producers. SAFE.
2. `has_incomplete_history && !have_inmem_accum` → INC-I-054 guard: `snap_sync_height=Some(current_h)` (Light validation), all active producers. SAFE.
3. `have_inmem_accum` → uses in-memory `attested_sets`, NO block scan. **BYPASSES the INC-I-054 guard** but does not read incomplete history.
4. else → full block scan + secondary net @ `rewards.rs:751-769`.

**6 call sites (all reorg/rollback fallback):**
| # | Site | Trigger |
|---|------|---------|
| 1 | block_handling.rs:673 | reorg undo deserialize fail |
| 2 | block_handling.rs:678 | reorg undo no epoch_state_snapshot (pre-upgrade) |
| 3 | block_handling.rs:754 | reorg legacy fallback (no undo) |
| 4 | rollback.rs:266 | rollback undo deserialize fail |
| 5 | rollback.rs:272 | rollback undo no epoch_state_snapshot |
| 6 | rollback.rs:276 | rollback no undo |

Snap-sync pre-guard @ `rollback.rs:150-153`: if block 1 missing, skip legacy rebuild entirely. `apply_block/mod.rs:327` always writes `epoch_state_snapshot: Some(...)`. `UndoData.epoch_state_snapshot` has `#[serde(default)]` (`state_db/types.rs:15-34`).

## 3. Hypotheses

**H1 (reachable?):** Reachable but GUARDED. Sites 2,5 not reachable on snap-synced (always Some). Sites 1,4 require bincode break (code bug, negligible). Site 6 pre-guarded at rollback.rs:150-153. Site 3 hits has_incomplete_history guard. **Unsafe block-scan branch (4) NOT reachable on snap-synced nodes.**

**H2 (silent-wrong vs safe?):** Safe degradation in all reachable scenarios. 4 defense layers: INC-I-054 guard, have_inmem_accum bypass, reorg Light mode (block_handling.rs:796), next-epoch post_commit self-correction.

**H3 (INC-I-081 intersection?):** Structural intersection EXISTS (ShallowRollback → rollback_one_block → rebuild fallback) but functionally benign post-INC-I-081 (MAX_CUMULATIVE_ROLLBACK=50, SHALLOW_ROLLBACK_MAX=10) + the guards. Needs 4 simultaneous improbable conditions to reach unsafe path.

## 4. Specs/Docs Drift

| Doc | Drift | Severity |
|-----|-------|----------|
| `specs/engine-parts.md:2790` | rebuild description omits INC-I-054 guard | Medium |
| `.claude/skills/storage/SKILL.md:561-605` | shows delete_epoch_state() as active in init.rs | Low |
| `.claude/skills/network/SKILL.md:208,292` | says protocol bump triggers delete_epoch_state() (no longer true) | Medium |

## 5. Self-Flagged Uncertainties (load-bearing — escalation basis)

1. Cannot determine mainnet height where `epoch_state_snapshot` was first added to UndoData.
2. Cannot verify whether any mainnet node still has pre-upgrade undo entries in its active rollback window.
3. Could not `git show 3ee3a7c6` to confirm single vs bundled commit.

## 6. User Narrative Divergences

| # | User Claim | Code Reality |
|---|-----------|-------------|
| 1 | 3ee3a7c6 = safety check commit | Subject = EPOCH_STATE_FORMAT_VERSION decoupling; effects present but broader purpose |
| 2 | unsafe rebuild reachable on snap-synced via pre-INC-I-040 undo | Snap-synced nodes only have current-binary undo (always Some); scenario impossible by construction *on snap-synced nodes* — only possible on full-sync nodes |
| 3 | revert f677e9b5 + block_handling.rs:673,678,754 | Reverting would regress full-sync nodes (stale epoch_state after rollback) while fixing a non-existent snap-sync problem |
| 4 | rewards.rs:530-558,:649-653,:759-767 | Actual 530-558 (exact), 649-654, 751-769 |

## 7. Triage Verdict (Analyst)

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.92, basis=code_trace)
Reasoning: All reachable paths on snap-synced nodes guarded by INC-I-054 check or have_inmem_accum bypass; no code change required; only specs need doc update.
━━━━━━━━━━━━━━━━━━━━━━
```

## 8. Orchestrator Triage Override → DEEP

FAST overridden to **DEEP**. Basis: (a) objective hard trigger "3+ interacting components" present in the analysis's own 6-call-site cross-module map; (b) the verdict's load-bearing reachability argument is self-flagged uncertain (§5.1, §5.2); (c) `--investigate` mode + maximum cost-if-wrong (wrong "leave as-is" on mainnet consensus = fork) mandates independent parallel verification. Analyst analysis becomes input to the Deep Investigation Path.

## 9. Recommended Action (Analyst, pending DEEP verification)

LEAVE AS-IS (code) + docs-only drift fixes. **To be confirmed or refuted by parallel investigation + synthesizer before final presentation.**
