# Fork Analysis: Finality Bypass in Fork Recovery Causes Reorg of Finalized Blocks

## Event Summary
- **Chain**: DOLI (custom PoS blockchain)
- **Consensus**: Slot-based PoS with weight-based fork choice + attestation-based finality
- **Client(s)**: DOLI Node v4.0.4
- **Fork detected**: 2026-03-26 ~17:48:05 UTC (after chain reset at ~17:44)
- **Fork depth**: 1 block (height 2 — two competing blocks)
- **Fork type**: Consensus-level (finality bypass bug)
- **Severity**: **Critical** — finalized blocks are reorged, violating consensus safety
- **Status**: At risk of recurrence (code bug not fixed)

## Fork Point
- **Common ancestor**: Block height 1 — hash `e1457f733fb44267290bc2c4628761fc6132f23455201e471de87d9fc1b5c5ed`
- **Chain A head**: Block height 2, slot 7011 — hash `c6edb3cdc375e6bf0e022749d356d16e7fccf23e5ba2419c8d3790611d66b850`
- **Chain B head**: Block height 2, slot 7012 — hash `c63e59dfda9632a13019015dd9fcb8aa50e280792142054a3fc855459b48e567`
- **Canonical chain**: Both chains were accepted by different nodes. The network is inconsistent at end of logs.

## Competing Blocks at Fork Height

| Field | Chain A Block | Chain B Block | Match? |
|-------|-------------|-------------|--------|
| Hash | `c6edb3cd...` | `c63e59df...` | No (confirms fork) |
| Parent Hash | `e1457f73...` | `e1457f73...` | Yes |
| Height | 2 | 2 | Yes |
| Slot | 7011 | 7012 | No (consecutive slots) |
| Producer | `effe88fe...` (n4) | `54323cef...` (n3) | Different |
| Produced at | 17:48:05 | 17:48:18 | 13s apart |
| VDF proof | `2d27fdcc...` | `54323cef...` | Different |

## Context: Chain Reset Preceded the Fork

All nodes were restarted with wiped data as part of a deployment procedure:
- **Seed**: restarted at 17:44:02 (was at h=17 s=6952)
- **n1**: restarted at 17:44:11 (was at h=17 s=6952)
- **n63**: restarted at 17:45:50 (was at h=16 s=6947)
- **n143**: restarted at 17:47:31
- **n163**: first start at 17:48:35 (new node)

The rolling restart meant nodes came online at different times over a ~4 minute window. This created conditions for competing block production: n4 produced a block at slot 7011 (17:48:05) and n3 produced at slot 7012 (17:48:18), both at height 2, both building on the same parent.

This initial fork is **normal and expected** — two producers at consecutive slots on a young network. The fork choice rule (weight + deterministic hash tie-break) should resolve it.

## Diagnosis Steps

| Step | Hypothesis | Diagnostic | Result | Conclusion |
|------|-----------|-----------|--------|------------|
| 1 | Fork caused by snap sync state divergence | Searched logs for snap sync events around fork time | No snap sync activity — all nodes started fresh from genesis | **Eliminated** |
| 2 | Fork caused by client version mismatch | Checked startup logs for version | All nodes run DOLI Node v4.0.4 | **Eliminated** |
| 3 | Fork caused by staggered restart creating competing producers | Checked BLOCK_PRODUCED logs for height 2 | n4 produced `c6edb3cd` at slot 7011, n3 produced `c63e59df` at slot 7012 — 13 seconds apart | **Confirmed** as trigger, but this is normal behavior |
| 4 | Fork should have resolved via weight-based fork choice | Checked check_reorg_weighted behavior | Weight tie (both weight=1), deterministic hash tie-break applied | Working correctly at reorg level |
| 5 | Finality should have prevented reorg | Checked for "FINALITY: Rejecting reorg" messages | ALL 13 core nodes log finality rejection followed by fork_recovery bypassing it | **Confirmed** — this is the root cause bug |
| 6 | plan_reorg respects finality | Read `reorg.rs` plan_reorg function (lines 341-420) | NO finality check in plan_reorg | **Confirmed** — missing finality guard |
| 7 | fork_recovery solo check allows the bypass | Read `fork_recovery.rs` line 112 | `weight_delta == 0 && our_fork_is_solo` allows reorg despite finality rejection | **Confirmed** — solo check overrides finality |

## Root Cause

**Category**: Consensus-level (code bug)

**Cause**: `plan_reorg()` in `crates/network/src/sync/reorg.rs` has NO finality check, while `check_reorg_weighted()` does. The fork recovery code path in `bins/node/src/node/fork_recovery.rs` first tries `check_reorg_weighted` (which correctly rejects the reorg due to finality), then falls through to `plan_reorg` (which has no finality check), allowing the reorg to proceed.

**Evidence**: Every node (seed, n1-n12) logs the exact same pattern:
```
WARN  FINALITY: Rejecting reorg past finalized height 2 (ancestor at 1)
WARN  FINALITY: Rejecting reorg past finalized height 2 (ancestor at 1)
INFO  Fork recovery: switching to network chain (delta=0, solo=true) -- rollback=1, new=1
```

The reorg module correctly rejects the reorg twice (once per code path attempt), but the fork recovery code's `plan_reorg` fallback bypasses this protection.

**Why it happened**: Two independent code paths for reorg planning exist:
1. `check_reorg_weighted()` (line 224, reorg.rs) — has finality check at line 291
2. `plan_reorg()` (line 341, reorg.rs) — has NO finality check

The fork recovery code (fork_recovery.rs, line 74-96 and 98-133) treats `check_reorg_weighted` returning `None` as "try the deeper fallback" rather than "reorg was rejected for safety reasons."

**Code location**:
- Bug: `crates/network/src/sync/reorg.rs` — `plan_reorg()` missing finality check
- Aggravating factor: `bins/node/src/node/fork_recovery.rs` lines 110-122 — `our_fork_is_solo` condition allows zero-delta reorgs without checking WHY `check_reorg_weighted` returned None

## Impact Assessment
- **Transactions affected**: Blocks at height 2 on the reorged chain (1 block per affected node)
- **Finality impact**: **CRITICAL** — finalized blocks were reorged, violating the core safety guarantee of finality. If finalized blocks can be reorged, the finality mechanism provides no meaningful protection.
- **Economic impact**: Low on testnet (no real value), but this bug on mainnet could enable double-spend attacks against users who rely on finality.
- **Duration**: Fork persists at end of logs. Nodes are split across 3 states:
  - Some on Chain A hash `c6edb3cd...` (slot 7011)
  - Some on Chain B hash `c63e59df...` (slot 7012)
  - Some advanced to h=3 hash `1da959d4...` (slot 7014)
- **Nodes affected**: All 13 core nodes (seed + n1-n12) confirmed. Likely all ~165 nodes affected.

## Remediation Recommendations

1. **Immediate (Critical)**: Add finality check to `plan_reorg()` in `crates/network/src/sync/reorg.rs`. The fix is surgical — add the same finality check that `check_reorg_weighted` has (lines 291-303) to `plan_reorg` before returning `Some(ReorgResult)`:
   ```rust
   // Before returning Some(ReorgResult), check finality
   if let Some(finality_height) = self.last_finality_height {
       let ancestor_height = self.block_weights.get(&common_ancestor)
           .map(|w| w.height).unwrap_or(0);
       if ancestor_height <= finality_height {
           warn!("FINALITY: Rejecting plan_reorg past finalized height {} (ancestor at {})",
               finality_height, ancestor_height);
           return None;
       }
   }
   ```

2. **Short-term**: In `fork_recovery.rs`, distinguish between "check_reorg_weighted returned None because no reorg needed" and "returned None because finality blocked it." If finality blocked it, do NOT fall through to `plan_reorg`. Consider adding a return type that distinguishes rejection reasons.

3. **Long-term**: Centralize finality enforcement. Every code path that can trigger a reorg must go through a single finality gate. Currently there are at least two paths (check_reorg_weighted and plan_reorg) and only one has the guard. Consider making finality a pre-condition on `execute_reorg()` itself as a defense-in-depth measure.

4. **Network recovery**: After deploying the fix, all nodes should be restarted to ensure they converge on the same canonical chain. The current network state is inconsistent.

## Evidence Appendix

### Block Headers
- **Height 1 (common ancestor)**: `e1457f733fb44267290bc2c4628761fc6132f23455201e471de87d9fc1b5c5ed` — finalized at attestation 4/5
- **Height 2 Chain A**: `c6edb3cdc375e6bf0e022749d356d16e7fccf23e5ba2419c8d3790611d66b850` — slot 7011, produced by n4 (`effe88fe...`), finalized at attestation 4/5
- **Height 2 Chain B**: `c63e59dfda9632a13019015dd9fcb8aa50e280792142054a3fc855459b48e567` — slot 7012, produced by n3 (`54323cef...`), finalized at attestation 4/5 (after reorg)
- **Height 3**: `1da959d4d2fae2b23f272f5ed2e5c94b9f5de88d8f0fec500080bac036d16bf3` — slot 7014 (some nodes reached this)

### Log Excerpts — Finality Bypass Pattern (seed.log)
```
17:48:43.907 FINALITY: Block c6edb3cd... finalized at height 2 (attestation 4/5)
17:48:44.368 FINALITY: Rejecting reorg past finalized height 2 (ancestor at 1)
17:48:44.464 FINALITY: Rejecting reorg past finalized height 2 (ancestor at 1)
17:48:44.476 Fork recovery: switching to network chain (delta=0, solo=true) -- rollback=1, new=1
17:48:44.479 Executing reorg: rolling back 1 blocks, applying 1 new blocks
17:48:44.597 Applying block c63e59df... at height 2
17:48:45.030 FINALITY: Block c63e59df... finalized at height 2 (attestation 4/5)
```

### Affected Nodes Confirmation
All 13 core nodes (seed + n1-n12) show identical pattern:
- Finality correctly rejects reorg
- Fork recovery bypasses finality via plan_reorg fallback
- Reorg executes despite finalization

### Code References
- **Bug location**: `crates/network/src/sync/reorg.rs` — `plan_reorg()` function (line 341)
- **Correct implementation**: `crates/network/src/sync/reorg.rs` — `check_reorg_weighted()` finality check (lines 290-303)
- **Bypass trigger**: `bins/node/src/node/fork_recovery.rs` — solo detection override (lines 110-122)

## Timeline
| Time | Event |
|------|-------|
| 17:29:14 | Network started (seed + original nodes) |
| 17:30:26 | First block finalized at height 1 |
| 17:39:02 | Network reached h=17 s=6952 (finalized) |
| 17:44:02 | Seed restarted with wiped data (chain reset begins) |
| 17:44:11-17:47:31 | Rolling restart of all nodes with wiped data |
| 17:48:05 | n4 produces block `c6edb3cd...` at height 2, slot 7011 |
| 17:48:13 | Seed applies chain A block at height 2 |
| 17:48:18 | n3 produces competing block `c63e59df...` at height 2, slot 7012 |
| 17:48:43 | Seed finalizes chain A block at height 2 (4/5 attestations) |
| 17:48:44 | **BUG**: Seed reorgs PAST finalization — rolls back chain A, applies chain B |
| 17:48:44 | Reorg module logs "Rejecting reorg past finalized height" but fork_recovery ignores it |
| 17:48:28-17:49:14 | Same finality bypass reorg occurs on ALL core nodes (n1-n12) |
| 17:50:00 | Network in inconsistent state — nodes on 3 different chain states |
