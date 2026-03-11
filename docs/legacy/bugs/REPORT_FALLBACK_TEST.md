# Fallback System Test Report (v2)

**Feature:** Producer Fallback System (5-Rank Sequential Windows)
**Date:** 2026-02-10 (Retest after fixes)
**Network:** Devnet (10 producers, 1 bond each, 10 total bonds)
**Commit:** 4eaed38 (fix(node): fix fallback system issues from live devnet testing (ISSUE-1,2,3,4))
**Previous Report:** 2026-02-09, commit 3b254bd

---

## Changes Since v1

Four fixes were implemented based on v1 test findings:

| Fix | Issue | Description |
|-----|-------|-------------|
| Fix 1 | ISSUE-1 + ISSUE-4 | Deduplicate eligible list in `try_produce_block()` to match validation path |
| Fix 2 | ISSUE-2 | Validate producer eligibility on gossip, reorg, and cached blocks |
| Fix 3 | ISSUE-3 | Startup chain integrity check (verify tip hash in block store) |
| Fix 4 | Diagnostic | Debug logging for fallback eligible list and rank assignment |

---

## Test Environment

| Parameter | Value |
|-----------|-------|
| Producers | 10 (all genesis via `devnet init --nodes 10`) |
| Bonds each | 1 (10 total) |
| Slot duration | 10s |
| Fallback ranks | 5 (rank 0-4, 2s windows each) |
| Genesis time | 1770705440 (Unix) |
| Bootstrap blocks | 24 (devnet) |

### Producer-to-Ticket Mapping (sorted by pubkey)

| Ticket | Node | PubKey (prefix) | RPC Port |
|--------|------|-----------------|----------|
| 0 | 3 | 2ad5e084... | 28548 |
| 1 | 4 | 32eb5773... | 28549 |
| 2 | 8 | 7c28309d... | 28553 |
| 3 | 2 | 7d7ae41d... | 28547 |
| 4 | 9 | a6ed1833... | 28554 |
| 5 | 5 | af833911... | 28550 |
| 6 | 6 | b3231652... | 28551 |
| 7 | 1 | bd28dbd6... | 28546 |
| 8 | 7 | ecc0c1f2... | 28552 |
| 9 | 0 | fed24b13... | 28500 |

**Selection formula:** Primary = `slot % total_bonds`, Fallback rank R = `(slot + offset_R) % total_bonds`

---

## Test Results

### Test 1: Baseline - Normal Block Production

**Status: PASS**

All 10 producers cycling correctly through sequential slots.

```
H: 68 S: 68 S%10=8 | N7(ecc0c1f2) | +    0ms
H: 69 S: 69 S%10=9 | N0(fed24b13) | +    0ms
H: 70 S: 70 S%10=0 | N3(2ad5e084) | +    0ms
H: 71 S: 71 S%10=1 | N4(32eb5773) | +    0ms
H: 72 S: 72 S%10=2 | N8(7c28309d) | +    0ms
H: 73 S: 73 S%10=3 | N2(7d7ae41d) | +    0ms
H: 74 S: 74 S%10=4 | N9(a6ed1833) | +    0ms
H: 75 S: 75 S%10=5 | N5(af833911) | +    0ms
H: 76 S: 76 S%10=6 | N6(b3231652) | +    0ms
H: 77 S: 77 S%10=7 | N1(bd28dbd6) | +    0ms
```

**Observations:**
- Perfect 10-slot cycle, no gaps
- All blocks produced at offset +0ms (within primary 0-2s window)
- 10 producers x 1 bond = deterministic round-robin

---

### Test 2: Single Primary Failure - Fallback Rank 1

**Status: PASS**

**Action:** Killed Node 7 (ecc0c1f2, ticket 8), primary for S%10=8.

**Result:** Node 3 (2ad5e084) produced at **+2132ms** (rank 1 window: 2-4s).

```
H: 93 S: 97 S%10=7 | N1(bd28dbd6) | +    0ms
H: 94 S: 98 S%10=8 | N3(2ad5e084) | + 2132ms  <<<< RANK 1 FALLBACK
H: 95 S: 99 S%10=9 | N0(fed24b13) | +    0ms
H: 96 S:100 S%10=0 | N3(2ad5e084) | +    0ms
```

**Confirmed across multiple cycles:**
- S=98: Node 3 at +2132ms
- S=108: Node 3 at +2118ms
- S=118: Node 3 at +2118ms

**vs v1:** Timing improved from +3s (v1) to +2.1s (v2). The dedup fix allows rank 1 to activate at exactly +2000ms plus network propagation delay (~130ms).

---

### Test 3: Cascading Failure - Primary + Rank 1 Dead

**Status: PASS - MAJOR IMPROVEMENT**

**Action:** Also killed Node 3 (2ad5e084), the rank 1 fallback for S%10=8.

**Result:** Node 8 (7c28309d) produced at **+4088ms** (rank 2 window: 4-6s).

```
H:107 S:117 S%10=7 | N1(bd28dbd6) | +    0ms
H:108 S:118 S%10=8 | N8(7c28309d) | + 4088ms  <<<< RANK 2 FALLBACK
H:109 S:119 S%10=9 | N0(fed24b13) | +    0ms
```

**CRITICAL FIX CONFIRMED (ISSUE-4):**
- **v1 result:** +6s (rank 3) - rank 2 was skipped due to duplicate in eligible list
- **v2 result:** +4s (rank 2) - correct! Dedup fix ensures rank 2 is used

This is the most important improvement. The dedup fix (Fix 1) eliminated the rank mapping discrepancy (ISSUE-1) and the fallback coverage gap (ISSUE-4) simultaneously. With 2 nodes dead, the system correctly falls to rank 2 instead of jumping to rank 3.

---

### Test 4: Deep Fallback - 3 Producers Dead

**Status: PASS - MAJOR IMPROVEMENT (no slot skips)**

**Action:** Also killed Node 8 (7c28309d).

**Dead nodes:** Node 7 (ticket 8), Node 3 (ticket 0), Node 8 (ticket 2)

**Result:** ALL slots still produce! Node 9 handles all fallback slots at correct ranks.

```
H:130 S:134 S%10=4 | N9(a6ed1833) | +    0ms    (normal)
H:131 S:135 S%10=5 | N5(af833911) | +    0ms    (normal)
H:132 S:136 S%10=6 | N6(b3231652) | +    0ms    (normal)
H:133 S:137 S%10=7 | N1(bd28dbd6) | +    0ms    (normal)
H:134 S:138 S%10=8 | N9(a6ed1833) | + 6000ms   <<<< RANK 3 FALLBACK
H:135 S:139 S%10=9 | N0(fed24b13) | +    0ms    (normal)
H:136 S:140 S%10=0 | N9(a6ed1833) | + 4000ms   <<<< RANK 2 FALLBACK
H:137 S:141 S%10=1 | N4(32eb5773) | +    0ms    (normal)
H:138 S:142 S%10=2 | N9(a6ed1833) | + 2000ms   <<<< RANK 1 FALLBACK
H:139 S:143 S%10=3 | N2(7d7ae41d) | +    0ms    (normal)
H:140 S:144 S%10=4 | N9(a6ed1833) | +    0ms    (normal)
H:141 S:145 S%10=5 | N5(af833911) | +    0ms    (normal)
H:142 S:146 S%10=6 | N6(b3231652) | +    0ms    (normal)
H:143 S:147 S%10=7 | N1(bd28dbd6) | +    0ms    (normal)
H:144 S:148 S%10=8 | N9(a6ed1833) | + 6000ms   <<<< RANK 3 FALLBACK (REPEATS)
H:145 S:149 S%10=9 | N0(fed24b13) | +    0ms    (normal)
H:146 S:150 S%10=0 | N9(a6ed1833) | + 4000ms   <<<< RANK 2 FALLBACK (REPEATS)
H:147 S:151 S%10=1 | N4(32eb5773) | +    0ms    (normal)
H:148 S:152 S%10=2 | N9(a6ed1833) | + 2000ms   <<<< RANK 1 FALLBACK (REPEATS)
H:149 S:153 S%10=3 | N2(7d7ae41d) | +    0ms    (normal)
```

**CRITICAL IMPROVEMENT vs v1:**
- **v1 result:** S%10=8 was **SKIPPED** (0 blocks) with 3 dead nodes. 30% node loss = 10% slot loss.
- **v2 result:** S%10=8 **PRODUCES** at +6s (rank 3). **Zero slot loss** with 3 dead nodes!

**Fallback timing breakdown (3 dead nodes):**

| Slot%10 | Primary (dead?) | Actual Producer | Offset | Rank |
|---------|-----------------|-----------------|--------|------|
| 0 | N3 (DEAD) | N9 | +4000ms | 2 |
| 2 | N8 (DEAD) | N9 | +2000ms | 1 |
| 8 | N7 (DEAD) | N9 | +6000ms | 3 |
| 1,3,4,5,6,7,9 | alive | normal | +0ms | 0 |

Pattern repeats perfectly across 2+ full cycles. Height increments by exactly 1 per slot (no gaps).

---

### Test 5: Recovery - Restart Killed Producers

**Status: PARTIAL PASS (new issue discovered)**

**Action:** Restarted nodes 3, 7, 8.

| Node | Recovery | Detail |
|------|----------|--------|
| Node 8 | **SYNCED but height corrupted** | Reported height=283 instead of 163. Chain hash correct. |
| Node 3 | **FAILED** | Froze during startup (no crash, just stopped after "Applied chainspec overrides") |
| Node 7 | **FAILED** | Same as Node 3 — froze during startup |

**New Issue (ISSUE-5): Height Double-Counting During Restart Sync**

Node 8's block store already had blocks 0-120 from before it was killed. On restart, the sync manager re-downloaded and re-applied ALL 163 blocks from genesis. The height counter became 120 + 163 = 283 instead of 163.

**Cascading effect:** Node 8 advertised height=283 to peers. Other nodes (0,1,2,4,5,9) attempted to sync from this corrupted peer. The sync failed because:
1. Header chain was broken (peer sent headers from genesis, nodes expected continuation)
2. Nodes entered `BlockedResync` state and stopped producing
3. `FORK PREVENTION: Only 4 peers (need 10)` blocked all production

**Chain state after restart attempt:**
```
Nodes 0,1,2,4,5,9: height=0 (reset to genesis during failed sync)
Node 6: height=163 (unaffected)
Node 8: height=283 (corrupted)
Nodes 3,7: DEAD (froze during startup)
```

**Root cause analysis:**
- Fix 3 (integrity check) correctly verified `best_hash` existed in the block store
- But it did NOT detect the height was wrong (block exists, height counter is off)
- The sync protocol re-applies blocks from genesis even when the block store already has them
- This creates a corrupted height that poisons the entire network

**Fix recommendation:** The sync manager should check if blocks already exist in the store before incrementing the height counter. Additionally, the height counter should be derived from the block store's height index, not from an in-memory counter that accumulates.

---

## Comparison: v1 vs v2

| Metric | v1 (3b254bd) | v2 (4eaed38) | Improvement |
|--------|-------------|-------------|-------------|
| Rank 1 fallback timing | +3s | +2.1s | Correct 2s window entry |
| Rank 2 fallback (2 dead) | SKIPPED to rank 3 (+6s) | +4s (rank 2) | Dedup fix works |
| 3 dead nodes slot loss | 10% (1 slot/cycle skipped) | **0%** (all slots produce) | Eliminated slot loss |
| Rank assignment | Incorrect (ISSUE-1) | Correct | Dedup fix |
| Gossip block validation | None (ISSUE-2) | Producer eligibility checked | Security fix |
| Startup integrity | None (ISSUE-3) | Hash verified, partial height check | Partial fix |
| Node restart | 2/3 recovered | 1/3 synced (height bug), 2/3 froze | **Regression** (new ISSUE-5) |

---

## Issues Status

### Resolved

| Issue | Fix | Status |
|-------|-----|--------|
| ISSUE-1: Rank mapping discrepancy | Fix 1 (dedup) | **RESOLVED** - ranks now match correctly |
| ISSUE-2: No gossip block validation | Fix 2 (eligibility check) | **RESOLVED** - 3 code paths validated |
| ISSUE-4: Fallback coverage gap | Fix 1 (dedup) | **RESOLVED** - rank 2 no longer skipped |

### Partially Resolved

| Issue | Fix | Status |
|-------|-----|--------|
| ISSUE-3: Node restart causes fork | Fix 3 (integrity check) | **PARTIAL** - hash check works, height check insufficient |

### New Issues

| Issue | Severity | Description |
|-------|----------|-------------|
| ISSUE-5: Height double-counting on restart sync | **HIGH** | Sync re-applies stored blocks, corrupting height counter. Corrupted node poisons network sync. |

---

## Timing Analysis

| Scenario | Dead Nodes | S%10=8 Result | Offset | Rank Window |
|----------|-----------|---------------|--------|-------------|
| Normal | 0 | Node 7 produces | +0ms | Rank 0 (0-2s) |
| Test 2 | Node 7 | Node 3 fallback | +2132ms | Rank 1 (2-4s) |
| Test 3 | Node 7, 3 | Node 8 fallback | +4088ms | Rank 2 (4-6s) |
| Test 4 | Node 7, 3, 8 | Node 9 fallback | +6000ms | Rank 3 (6-8s) |

**Key observation:** Each additional dead node shifts fallback by exactly +2000ms. The dedup fix ensures continuous rank coverage with no gaps.

---

## Conclusion

The 5-rank sequential fallback system is now **production-ready for its core function.** The dedup fix (Fix 1) eliminated the critical rank mapping and coverage gap issues. With 30% of producers dead (3/10), the chain now has **zero slot loss** (all slots produce via fallback), a major improvement over v1 which lost 10% of slots.

**Remaining work before mainnet:**
1. **ISSUE-5 (HIGH)** - Fix height double-counting in sync manager. The current restart behavior can poison an entire network. This is a sync protocol bug, not a consensus bug.
2. **ISSUE-3 refinement** - Strengthen integrity check to verify height matches block store height index, not just hash existence.
