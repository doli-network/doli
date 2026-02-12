# Merge Resolution Report: fork + doli-ivan

**Date**: 2026-02-09
**Branches**: `fork` (23 commits) + `origin/doli-ivan` (12 commits)
**Common Ancestor**: `7781210`
**Safety Tag**: `backup-pre-merge-20260209` (restore: `git reset --hard backup-pre-merge-20260209`)

---

## Branch Summary

| Branch | Focus | Key Changes |
|--------|-------|-------------|
| **fork** (ours) | Sync stability, networking, devnet ops | Auto-resync, gossip watchdog, MeshConfig, consecutive fork detection, min_peers tuning, chainspec injection |
| **doli-ivan** (theirs) | Consensus, scalability, auto-update | Tiered architecture (T1/T2/T3), finality gadget, fork recovery, bond adjustments, auto-update system (M1-M8) |

## Merge Stats

- **Total files in merge**: 45 files, +5307/-240 lines
- **Auto-merged (no conflicts)**: 35 files
- **Conflicts requiring manual resolution**: 10 files

---

## Conflict Resolutions (10 files)

### 1. `crates/core/src/consensus.rs`

**Conflict Type**: Test-only
**Resolution Strategy**: Keep both test sets

Both branches added new test functions to the same `#[cfg(test)]` module:
- **fork**: `test_chainspec_override_*` (4 tests) — verify chainspec parameter loading
- **Ivan**: `test_tier_*` (4 tests) — verify tier computation logic

**Action**: Kept all 8 tests. No production code conflict.

---

### 2. `crates/network/src/gossip.rs`

**Conflict Type**: Structural (major)
**Resolution Strategy**: Merge both feature sets

- **Ivan added**: Tiered topic system (`TIER1_BLOCKS_TOPIC`, `HEADERS_TOPIC`, `ATTESTATION_TOPIC`), `new_gossipsub_for_tier()`, `subscribe_to_topics_for_tier()`, `region_topic()`
- **Fork added**: `MeshConfig` struct for configurable gossipsub mesh parameters, `new_gossipsub(keypair, &MeshConfig)`

**Action**: Kept Ivan's tiered topic infrastructure (Tier 1/2/3 with different subscriptions) alongside fork's `MeshConfig` struct and configurable `new_gossipsub()`. Both features are complementary — tier-aware mesh reconfiguration can use `MeshConfig` in the future.

---

### 3. `crates/network/src/sync/reorg.rs`

**Conflict Type**: Test-only
**Resolution Strategy**: Keep both test sets

- **fork**: `test_tiebreak_*` (2 tests) — verify tiebreak logic
- **Ivan**: `test_finality_*` (2 tests) — verify finality rejection logic

**Action**: Kept all 4 tests. No production code conflict.

---

### 4. `bins/node/src/config.rs`

**Conflict Type**: Struct fields + constructor
**Resolution Strategy**: Keep both additions

- **fork added**: `chainspec: Option<ChainSpec>` field — passes full chainspec into `Node` for in-memory access
- **Ivan added**: `slot_duration_override: Option<u64>` field — explicit slot duration override from chainspec

**Action**: Kept BOTH fields in the `NodeConfig` struct and its `for_network()` constructor. Both serve different purposes: `chainspec` carries the full spec, `slot_duration_override` is a pre-extracted convenience value.

---

### 5. `crates/network/src/service.rs`

**Conflict Type**: Gossipsub initialization
**Resolution Strategy**: Keep fork's implementation + Ivan's comment

- **fork**: Uses `MeshConfig` from `NetworkParams` for configurable gossipsub initialization
- **Ivan**: Uses default gossipsub with comment about future tier-aware reconfiguration

**Action**: Kept fork's `MeshConfig` usage (already wired to `NetworkParams`). Added Ivan's comment noting that tier-aware gossipsub reconfiguration is planned for future implementation.

---

### 6. `specs/architecture.md`

**Conflict Type**: Documentation sections
**Resolution Strategy**: Combine both additions

- **fork added**: Parameter loading order documentation (Parent ENV > .env > Chainspec > defaults)
- **Ivan added**: Chainspec overrides explanation

**Action**: Combined both sections. The parameter loading order and chainspec override documentation are complementary and both needed.

---

### 7. `bins/node/src/main.rs` (4 conflict regions)

**Conflict Type**: Startup logic
**Resolution Strategy**: Combine both features

| Region | fork | Ivan | Resolution |
|--------|------|------|------------|
| Chainspec loading | Type annotation on `load()` | Better comment | Kept both: Ivan's comment + fork's type annotation |
| Config setup | `config.chainspec = chainspec.clone()` | `config.slot_duration_override = ...` | Kept BOTH assignments |
| Slot duration | (none) | Slot duration override propagation | Added Ivan's slot duration handling |
| Producer set init | Flat if/else if/else per network | Nested if-let with match | Used fork's flat structure (cleaner): chainspec-first, then legacy network-specific fallback |

---

### 8. `bins/node/src/node.rs` (6 conflict regions)

**Conflict Type**: Major structural (largest conflicts)
**Resolution Strategy**: Combine both recovery mechanisms

| Region | fork | Ivan | Resolution |
|--------|------|------|------------|
| Struct fields | `consecutive_fork_blocks: u32` | `cached_scheduler`, `our_tier`, `last_tier_epoch`, `vote_tx` | Kept ALL fields from both |
| Min peers | Network-specific (devnet=1, others=2) | Comment about tier-aware override | Kept fork's values + Ivan's comment |
| Struct init | fork's initializers | Ivan's initializers | Combined both sets |
| **Production auth** | `maybe_auto_resync()` on AheadOfPeers/ChainMismatch with consecutive fork tracking | `BlockedConflictsFinality` + `try_trigger_fork_recovery()` | **Combined both**: fork's auto-resync (nuclear: wipe+resync) + Ivan's fork recovery (surgical: walk orphan parents) + new `BlockedConflictsFinality` variant |
| Heartbeat delay | Scaled: 20% of slot, capped 700ms | Fixed value | Kept fork's scaled approach |
| VDF safety | Detailed stale parent detection comment | Shorter comment | Kept fork's detailed version |

**Key Decision — Production Authorization**: The two recovery strategies are complementary:
- Ivan's `try_trigger_fork_recovery()` attempts surgical recovery (download orphan parents)
- Fork's `maybe_auto_resync()` is the nuclear option (wipe and resync from genesis) after 50+ consecutive failures
- Both are called on `AheadOfPeers` and `ChainMismatch` — surgical first, nuclear as last resort

---

### 9. `crates/network/src/sync/manager.rs` (6 conflict regions)

**Conflict Type**: Major structural
**Resolution Strategy**: Layer both feature sets

| Region | fork | Ivan | Resolution |
|--------|------|------|------------|
| Default fields | Gossip tracking + sync failure counters | (none) | Kept fork's fields |
| `can_produce()` layers | Layers 7-10: SyncFailures, ChainMismatch, GossipWatchdog, AutoResync | Finality conflict check | Kept fork's 10 layers + added Ivan's finality check as **Layer 11** |
| Tier setter | `set_gossip_timeout()` | `set_tier()` | Kept BOTH methods |
| Helper methods | `network_tip()` accessors, `sync_state_name()` | (none) | Kept fork's helpers |
| Force resync reset | Gossip tracking field reset | Network tip reset | Kept BOTH resets |
| Stall recovery | Synchronized stall detection | Stuck Processing detection via `last_block_applied` | Combined both: fork's Synchronized stall + Ivan's Processing stall |

**Production Authorization Layers** (unified, 11 total):
1. Not syncing
2. Has peers
3. Min peers met
4. Valid chain tip
5. Not currently downloading
6. Peer heights consistent
7. No excessive sync failures (fork)
8. No chain mismatch (fork)
9. Gossip watchdog OK (fork)
10. Auto-resync not in progress (fork)
11. No finality conflicts (Ivan)

---

### 10. `.claude/skills/network-setup/SKILL.md` (10 conflict regions)

**Conflict Type**: Documentation/skill definition
**Resolution Strategy**: Fork base + Ivan's unique content

Used fork's detailed version (v3.2.0) as the base, which includes:
- Zombie process cleanup procedures
- Devnet reinitialization checks
- Add-producer automation
- Producer removal guide
- Chain-halt troubleshooting

Incorporated Ivan's unique additions:
- Broader network description
- DHT peer discovery guidance
- Additional troubleshooting scenarios

---

## Merge Philosophy

The guiding principle was **"keep both, lose nothing"**:

1. **Complementary features** (most cases): Both branches added different capabilities. We kept everything.
2. **Overlapping logic**: Where both touched the same function, we layered fork's approach (more production-hardened from devnet testing) with Ivan's additions (forward-looking scalability).
3. **Style conflicts**: Preferred fork's style when structurally cleaner (e.g., flat if/else vs nested match), but kept Ivan's logic regardless.
4. **Tests**: All tests from both branches preserved — zero test loss.

## Rollback Instructions

If anything is wrong after commit:
```bash
git reset --hard backup-pre-merge-20260209
```

This restores the `fork` branch to its exact pre-merge state.
