# DOLI Sync Architecture: Branch Decision Report

**Date**: 2026-03-24
**Author**: Antonio Lozada
**For**: Ivan D. Lozada — Decision Maker
**Subject**: Technical assessment of `main` vs `fix/sync-state-explosion-root-causes` for the sync manager architecture going forward

---

## Executive Summary

Two branches have diverged since March 19 (commit `103fc31a`), each taking a fundamentally different approach to the sync state explosion problem. Main avoids triggering the bugs through network-layer improvements. The fix branch addresses the bugs at their root through architectural rework. Both approaches work today. They carry different risk profiles for the future.

This report presents the facts so that the final decision is informed and documented.

---

## 1. Branch Overview

| | `main` | `fix/sync-state-explosion-root-causes` |
|---|---|---|
| **Commits since fork** | 85 | 70 |
| **Primary author** | Ivan (75 commits) | Antonio (34) + Ivan (35) |
| **Current version** | 4.5.3 | 4.0.4 |
| **Strategy** | Reduce trigger frequency | Fix root causes |
| **Reverts** | 5 | 1 |
| **Chain resets** | ~3 | ~24 |
| **Sync test lines** | 1,035 | 3,853 |
| **Total sync module lines** | 8,568 | 10,595 |

---

## 2. What Main Did (Ivan's Work)

Main focused on **production features and network-layer hardening**:

### Features Shipped
- Round-robin block production (eliminates chain freeze on producer death)
- Reconstructible liveness filter (3 attempts — 2 reverted — working on 3rd)
- Chained transactions (spend pending mempool change outputs)
- Zero-config producer UX (`doli init`, `doli service`)
- forceProduceBlock RPC for testnet fork testing
- deb/rpm packaging with post-install scripts
- Scaled testnet to 500 nodes

### Sync Approach
- **Did not modify the core sync state machine**
- Kept `chain_follower.rs` (876 lines) and `fork_sync.rs` (723 lines) intact
- Fixed network-layer triggers: decoupled transport limits, stopped GSet flooding, limited connections per peer, deterministic fork tiebreak
- Result: sync cascades happen less frequently because the network is calmer

### Stability Indicators
- 5 reverts in 85 commits (liveness filter: 2 reverts, forceProduceBlock: 1 revert, genesis: 1 revert, producer set: 1 revert)
- Liveness filter required 3 implementation attempts before finding a snap-sync-compatible design
- `snap_sync.rs` exists as an empty file (0 lines)

---

## 3. What the Fix Branch Did (Antonio's Work)

The fix branch focused on **systematic root-cause elimination**:

### Root Causes Found and Fixed
- **INC-I-001**: 10 root causes of sync state explosion identified and resolved
- **INC-I-002**: ProducerSetDigest decode path missing in gossip handler
- **INC-I-003**: `scheduled` field in bincode serialization causing divergence
- **INC-I-004**: 3 sync root causes + testnet genesis key alignment
- **INC-I-005**: Sync cascade feedback loop (7 fixes: peak_height tracking, snap quorum cap at 15, reactive scheduling decoupled from block store, chain_state.best_slot fallback, reset_state_only guard, cascade loop breaker)
- **INC-I-006**: Network-aware epoch length in rebuild + tier code (hardcoded mainnet constants in replay paths)
- **INC-I-008**: Production gate hash agreement removed + rebuild ordering fixed

### Architectural Changes
- **Deleted** `chain_follower.rs` (876 lines) and `fork_sync.rs` (723 lines) — the tangled fork recovery code
- **Created** `snap_sync.rs` (327 lines) — dedicated snap sync module
- **Added** state transition validation matrix (M1) — prevents invalid sync state transitions
- **Added** recovery gate (M2) — routes all 9 genesis resync sites through single controlled path
- **Added** hard enforcement + ForkAction wiring (M3) — reorg floor check
- **Redesigned** sync manager substruct extraction with idempotent `start_sync`
- 3,853 lines of sync tests (3.7x main's coverage)

### Stability Indicators
- 1 revert in 70 commits
- 24 testnet genesis resets (intensive testing cycle)
- Fell behind main by 13 production features
- Version frozen at 4.0.4

---

## 4. The Core Technical Risk

### What Main Carries Forward

Main still contains the code that was proven buggy by the fix branch's 8 incidents:

| File | Lines | Status on Main |
|------|-------|----------------|
| `chain_follower.rs` | 876 | **Untouched — contains bugs identified in INC-I-001** |
| `fork_sync.rs` | 723 | **Untouched — contains bugs identified in INC-I-001** |
| `snap_sync.rs` | 0 | **Empty — no dedicated snap sync logic** |
| Sync manager tests | 1,035 | **Missing coverage for 10 identified failure modes** |

The network-layer fixes (GSet flooding, connection limits, transport decoupling) reduce how often these bugs trigger. They do not eliminate the bugs.

### When the Bugs Will Surface Again

The sync state explosion is triggered by concurrent fork recovery across multiple nodes. This probability increases with:

- **More nodes**: 500 → 5,000 → 50,000 (quadratic growth in partition combinations)
- **Network partitions**: geographic distribution, unreliable links, cloud provider outages
- **Adversarial conditions**: deliberate fork injection, eclipse attacks
- **Long chains**: more block history = deeper potential reorgs = longer recovery paths through buggy code

At current scale (500 nodes, single testnet operator, no adversaries), main's band-aid holds. The question is how long.

---

## 5. Options

### Option A: Continue with Main

**Pros**:
- Working in production today at 500 nodes
- 13 features already shipped
- Simpler path — no merge needed
- Version already at 4.5.3

**Cons**:
- 1,599 lines of proven-buggy sync code (`chain_follower.rs` + `fork_sync.rs`) remain
- 0 lines of dedicated snap sync logic
- 10 identified root causes remain unfixed
- 2,818 fewer test lines covering the most critical subsystem
- Liveness filter took 3 attempts — indicates fragility in the area it touches
- Technical debt compounds: every feature built on top of the buggy sync layer makes future fixes harder

**Risk**: At scale or under adversarial conditions, the sync state explosion will recur. The fix will be harder then because more code depends on the current (broken) architecture.

### Option B: Rebase Fix Branch onto Main

**Pros**:
- 10 root causes fixed
- Clean sync architecture (state transition matrix, recovery gate, dedicated snap sync)
- 3,853 lines of sync tests
- Tangled code deleted, not patched
- Picks up all of main's 13 features via rebase

**Cons**:
- Significant merge effort (5,383 insertions / 3,356 deletions in sync alone)
- Risk of merge regressions (this already happened once — commit `b0b7c185` "restore all Antonio INC-001 fixes lost during main merge")
- Requires thorough testing after merge
- Delays next release

**Risk**: The merge itself is the danger point. Once completed and tested, the codebase is on stronger architectural footing.

### Option C: Cherry-Pick Fix Branch's Root-Cause Fixes into Main

**Pros**:
- Lower merge risk than full rebase
- Gets the critical fixes without the full architectural rework
- Keeps main's history clean

**Cons**:
- Cherry-picking 70 commits with interdependencies is error-prone
- The architectural fixes (M1/M2/M3) depend on each other — partial cherry-pick may not work
- `chain_follower.rs` and `fork_sync.rs` deletions require all replacement code to come with them
- Effectively re-doing the fix branch's work piecemeal

---

## 6. Recommendation

From a technical standpoint, Option B (rebase fix branch onto main) is the correct long-term choice for a blockchain that is "just starting out." The architectural fixes should be integrated early, before the network grows to a scale where the latent bugs become critical and the codebase is too entangled to fix safely.

However, this is a **project decision, not just a technical one**. The merge effort is real and carries its own risks. If the priority is shipping features fast, Option A works until it doesn't.

What I can say with certainty: **the bugs in `chain_follower.rs` and `fork_sync.rs` are real, documented across 8 incidents, and will resurface at scale.** The only question is when.

---

## 7. Decision Record

| Decision | |
|----------|---|
| **Date** | |
| **Decided by** | |
| **Option chosen** | |
| **Rationale** | |
| **Accepted risks** | |

---

*This report was prepared from git history analysis of both branches. All claims are verifiable via `git log`, `git diff`, and the incident documentation on the fix branch.*
