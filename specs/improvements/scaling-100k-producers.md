# Requirements: Scaling DOLI to 100K+ Producers

> Analysis of the "Unlock DOLI for 100K+ Producers" implementation plan.
> Every file path, line number, and assumption verified against post-modularization codebase.
> Date: 2026-03-15

## Code Verification Summary

All 7 items had **incorrect file references** (pre-modularization paths). Corrected paths and findings below.

## Scope

| Domain | Affected Files |
|--------|---------------|
| Block Production | `bins/node/src/node/production/mod.rs`, `production/assembly.rs`, `production/scheduling.rs` |
| Network Layer | `crates/network/src/service/swarm_events.rs`, `service/swarm_loop.rs` |
| Consensus | `crates/core/src/consensus/constants.rs`, `consensus/selection.rs`, `scheduler.rs` |
| Gossip | `crates/network/src/gossip/config.rs` |
| Sync | `bins/node/src/node/fork_recovery.rs` |
| Startup | `bins/node/src/node/startup.rs` |
| Validation | `crates/core/src/validation/producer.rs` |

---

## Plan vs Reality

### Item 1: Block Building Deadline

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `production.rs:1024-1048` | `production/assembly.rs` lines 117-140 |
| Code | "mempool TX validation loop" | `build_block_content()` method. Mempool loop calls `mempool.select_for_block(1_000_000)` (size cap only). TX validation at lines 129-138. |
| Assumption | "No deadline exists" | **CONFIRMED.** Size cap (1MB) but no time deadline. Slot boundary check at lines 154-161 aborts AFTER all TX processing (safety net, not proactive). |

**Verdict:** Plan is correct. Implementation point is `assembly.rs` line 117.

### Item 2: Dead Peer Exponential Backoff

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `service.rs` | `service/swarm_events.rs` lines 64-85 (ConnectionClosed) |
| Code | "ConnectionClosed branch" | Correct event, wrong file. Currently: `peers.remove()`, no backoff tracking. |
| Reconnection | Not mentioned | `periodic.rs` lines 196-213: redials bootstrap every `slot_duration` (10s), fixed interval, no backoff. |

**Verdict:** Plan partially correct. Must span TWO locations: swarm_events.rs (tracking) + periodic.rs (backoff-aware redial). `mismatch_redial_cooldown` in `swarm_loop.rs` line 57 is partial precedent.

### Item 3: Liveness-Aware Epoch Scheduler (CONSENSUS-CRITICAL)

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `production.rs:746-801` | `production/scheduling.rs` lines 408-454 (epoch mode) |
| `producer_liveness` exists? | Assumed yes | **YES.** `Node.producer_liveness: HashMap<PublicKey, u64>` at `mod.rs` line 146. |
| How populated? | Not checked | `apply_block/mod.rs` line 68: inserts producer+height. Rebuilt from block_store on startup at `init.rs` lines 318-340. |
| Deterministic? | Assumed yes | **PARTIALLY.** Derived from chain data, BUT after snap sync, block_store is EMPTY → liveness map is empty. Full-sync nodes have populated maps. |
| Already applied? | Assumed no | **CONFIRMED** not applied to epoch scheduling. Liveness filter only in bootstrap mode (lines 326-373). |

**CRITICAL FINDING: `producer_liveness` is NOT safe for consensus-critical scheduling.**

After snap sync, the map is empty. Two nodes at the same height with the same state root would compute different scheduling if one snap-synced and the other full-synced. **This causes an immediate fork.**

**Recommendation:** Must use on-chain data — either `presence_root` bitfield in block headers (already exists for attestation) or a new `last_active_epoch` field in ProducerSet (transferred in snap sync).

### Item 3b: Inactivity Leak — Whale Bond Weight Decay (CONSENSUS-CRITICAL)

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `consensus.rs` + `production.rs:306-318` | `consensus/constants.rs` + `production/mod.rs` lines 154-164 |
| Constants | "New INACTIVITY_LEAK_*" | **Do not exist.** No inactivity leak mechanism in codebase. |
| active_with_weights | "Apply decay formula" | Currently builds from UTXO bond counts only (lines 158-163). No decay. |

**CRITICAL FINDING:** Same consensus-safety issue as Item 3. The plan uses `producer_liveness` HashMap which differs between snap-synced and full-sync nodes. **The plan's approach is UNSAFE for consensus.**

For consensus safety:
1. Decay formula MUST use ONLY on-chain data
2. "Last active height" MUST be stored in ProducerSet (persisted in StateDb, transferred in snap sync)
3. NOT derived from block_store scanning

### Item 4: Producer Peer Reservation in GossipSub

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `gossip/config.rs` | **CORRECT.** Lines 27-66 and 271-306. |
| TopicScoreParams | "Add scoring" | **Not present.** No `PeerScoreParams`, no `TopicScoreParams`. |
| Dynamic mesh | Already exists | `compute_dynamic_mesh()` removed — replaced with universal static mesh (mesh_n=12, mesh_n_high=24). |

**Verdict:** Plan is correct. Network-layer only, no consensus impact. Universal mesh params (12/8/24/12) scale from small to 1000+ node networks without runtime adaptation.

### Item 5: Auto-Backfill After Snap Sync

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `fork_recovery.rs` | **CORRECT.** `apply_snap_snapshot()` at lines 371-493. |
| Current behavior | "No auto-backfill" | **CONFIRMED.** Block_store is empty after snap sync except seeded canonical index. |

**Verdict:** Plan is correct. However, `backfillFromPeer` requires an RPC URL, and peer RPC endpoints are not discoverable from P2P. Should use sync protocol (GetBlocks) instead.

### Item 6: DHT Warning

| Aspect | Plan Said | Actual Code |
|--------|-----------|-------------|
| File | `startup.rs` | **CORRECT.** `no_dht` set at line 162. No warning. |

**Verdict:** Plan is correct. Simple, zero-risk addition.

---

## Requirements

| ID | Requirement | Priority | Risk |
|----|------------|----------|------|
| REQ-OPS-001 | Warn when --no-dht used with >5 producers | Must | Low |
| REQ-NET-001 | Dead peer exponential backoff for reconnection | Should | Low |
| REQ-PROD-001 | Block building time deadline (60% of slot) | Should | Low |
| REQ-NET-002 | GossipSub TopicScoreParams for producer peer priority | Could | Low |
| REQ-SYNC-001 | Auto-backfill block_store after snap sync | Should | Medium |
| REQ-SYNC-002 | Disable snap sync by default for mainnet | Must | Low |
| REQ-SYNC-003 | Validate StateDb against chainspec on startup | Must | Medium |
| REQ-SCHED-001 | Liveness-aware epoch scheduling using on-chain data | Must | **HIGH** |
| REQ-SCHED-002 | Inactivity leak — whale bond weight decay | Could | **HIGHEST** |

## Acceptance Criteria

### REQ-OPS-001: DHT Warning
- [ ] `--no-dht` && active_producers > 5 → WARN log during `start_network()`
- [ ] Warning includes producer count and recommendation
- [ ] Does not prevent startup

### REQ-NET-001: Dead Peer Backoff
- [ ] Failed attempts per peer tracked in HashMap
- [ ] Backoff: 1s, 2s, 4s, ... capped at 300s (60s for bootstrap nodes)
- [ ] Counter resets on successful connection

### REQ-PROD-001: Block Building Deadline
- [ ] Deadline = `slot_duration_ms * 6 / 10` from block building start
- [ ] Coinbase + epoch rewards always included before deadline check
- [ ] Partial TX sets produce valid blocks
- [ ] WARN log when TXs dropped due to deadline

### REQ-NET-002: GossipSub Producer Scoring
- [ ] TopicScoreParams on BLOCKS_TOPIC with `first_message_deliveries_weight: 10.0`
- [ ] No consensus behavior change
- [ ] Works with universal mesh config (mesh_n=12)

### REQ-SYNC-001: Auto-Backfill After Snap Sync
- [ ] Background task starts after `apply_snap_snapshot()` completes
- [ ] Uses sync protocol (GetBlocks), not RPC
- [ ] Rate-limited (10 blocks/s), non-blocking
- [ ] Persists progress across restarts

### REQ-SYNC-002: Disable Snap Sync by Default
- [ ] Default `snap_sync_threshold` changed from 1000 to `u64::MAX` (disabled)
- [ ] `--snap-sync` CLI flag explicitly enables it (sets threshold back to 1000)
- [ ] If StateDb has stale state and `--snap-sync` not passed, refuse to start with error
- [ ] Error message tells operator to wipe data or pass `--snap-sync`
- [ ] Mainnet nodes always full-sync from genesis via headers by default

### REQ-SYNC-003: Validate StateDb Against Chainspec on Startup
- [ ] On startup, if StateDb has producers, validate genesis hash matches embedded chainspec
- [ ] If genesis hash mismatch detected, refuse to start with clear error
- [ ] Error identifies the stale StateDb as the source (not the chainspec)
- [ ] Applies to all sync modes (full-sync and snap-sync)

### REQ-SCHED-001: Liveness-Aware Epoch Scheduler
- [ ] Uses on-chain attestation data (presence_root), NOT local producer_liveness
- [ ] Snap-synced and full-sync nodes produce identical scheduling at same height
- [ ] Inactive producers get reduced weight, not removed
- [ ] Deadlock safety: if all inactive, restore full weights
- [ ] Validation mirrors production logic exactly

### REQ-SCHED-002: Whale Bond Weight Decay
- [ ] `last_active_epoch` field added to ProducerSet (persisted, snap-sync safe)
- [ ] Decay: configurable rate per missed epoch, floor at 10% of original
- [ ] Weight fully recovers after 1 epoch of active production
- [ ] Optional/defaulted field for backward compatibility
- [ ] Gated behind `is_protocol_active()` for safe activation

## Implementation Priority (Adjusted)

The plan's priority order is adjusted based on risk analysis:

| Phase | Order | ID | Rationale |
|-------|-------|-----|-----------|
| 1 | 1 | REQ-OPS-001 | Trivial, zero risk. Ship independently. |
| 1 | 2 | REQ-NET-001 | Low risk, network-only, no consensus impact. |
| 1 | 3 | REQ-PROD-001 | Low risk, important for large mempools. |
| 1 | 4 | REQ-NET-002 | Low risk, gossip optimization. |
| 2 | 5 | REQ-SYNC-002 | Prerequisite for liveness scheduling. Disable snap sync default. |
| 2 | 6 | REQ-SYNC-003 | Prerequisite for liveness scheduling. Validate StateDb. |
| 2 | 7 | REQ-SYNC-001 | Medium risk, operational health. |
| 3 | 8 | REQ-SCHED-001 | **HIGH risk.** Must use on-chain data, test exhaustively, deploy simultaneously. |
| 3 | 9 | REQ-SCHED-002 | **HIGHEST risk.** Depends on REQ-SCHED-001 + storage schema change. Separate phase. |

Phase 1 items can be implemented **in parallel**. Phase 2 must complete before Phase 3.

## Impact Analysis

| Module | How Affected | Risk |
|--------|-------------|------|
| `production/assembly.rs` | Add deadline (REQ-PROD-001) | Low — additive |
| `service/swarm_events.rs` | Add failure tracking (REQ-NET-001) | Low — network only |
| `periodic.rs` | Backoff-aware redial (REQ-NET-001) | Low — timing only |
| `production/mod.rs` lines 154-164 | Modify active_with_weights (REQ-SCHED-001/002) | **HIGH** — consensus-critical |
| `consensus/constants.rs` | New leak constants (REQ-SCHED-002) | **HIGH** — simultaneous deploy |
| `storage/producer/` | Add `last_active_epoch` (REQ-SCHED-002) | **HIGH** — schema change |
| `validation/producer.rs` | Mirror scheduling logic (REQ-SCHED-001) | **HIGH** — mismatch = fork |
| `gossip/config.rs` | Add TopicScoreParams (REQ-NET-002) | Low — network only |
| `fork_recovery.rs` | Auto-backfill trigger (REQ-SYNC-001) | Medium — must not interfere |
| `startup.rs` | DHT warning (REQ-OPS-001) | Low — log only |

## Key Finding

**Items 3 and 3b (REQ-SCHED-001/002) CANNOT safely use the `producer_liveness` HashMap for consensus-critical scheduling.**

After snap sync, this map is empty, causing scheduling divergence between snap-synced and full-sync nodes. Implementation MUST use on-chain data:
- Option A: `presence_root` bitfield (already in block headers, used for attestation)
- Option B: New `last_active_epoch` field in ProducerSet (persisted in StateDb, transferred in snap sync)

## Corrected File Paths

| Plan Reference | Actual Path (post-modularization) |
|---------------|----------------------------------|
| `production.rs:1024-1048` | `production/assembly.rs:117-140` |
| `production.rs:746-801` | `production/scheduling.rs:408-454` |
| `production.rs:306-318` | `production/mod.rs:154-164` |
| `service.rs` (ConnectionClosed) | `service/swarm_events.rs:64-85` |
| `consensus.rs` | `consensus/constants.rs` |
| `gossip/config.rs` | `gossip/config.rs` (correct) |
| `fork_recovery.rs` | `fork_recovery.rs` (correct) |
| `startup.rs` | `startup.rs` (correct) |
