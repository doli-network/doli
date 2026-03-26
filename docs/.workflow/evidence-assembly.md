# Evidence Assembly: INC-I-012

## Incident Timeline

**INC-I-012: 50-node stress test watchman - network health monitoring**
Status: investigating

Timeline of attempts and discoveries:
- **Discovery 1**: Fix 1 (conn_headroom max_peers*2+20) solved 15 stuck nodes but re-exposed RAM explosion (4.9GB→8.5GB in 4 min) and fork storms when 50 non-producing nodes connect. REVERTED.
- **Discovery 2**: Fix 2 (Event drain before production - drain 50 pending events before try_produce_block) prevents stale-tip forks but insufficient — GSet flooding consumes entire event loop.
- **ROOT CAUSE**: GSet gossip flooding. 106 nodes × mesh_n=12 × gossip_lazy=12 = ~1200 producer announcement messages per round. Each carries 103 duplicates. Seed last 30 log lines were 100% GSet processing — zero block events.
- **Discovery 3**: Seed fork recovery deadlocked: Cannot rollback h=229→228 (missing block 228). Cached chain rejected at slot 387 (invalid producer for slot — ProducerSet divergence). Common ancestor search failed after 1000 blocks.
- **Attempt 1**: conn_headroom increase to max_peers*2+20 (developer)
- **Attempt 2**: Event drain before production (50 events) (developer)

## Prior System Model
No prior model — diagnostician will build from scratch

## Constraint Table
| # | Fix Attempted | What It Changed | Result | What This Eliminates |
|---|---|---|---|---|
| 1 | conn_headroom to max_peers*2+20 | Network connection management | Solved 15 stuck nodes but caused RAM explosion 4.9GB→8.5GB + fork storms | Root cause NOT solely in connection headroom — creates new problems |
| 2 | Event drain before production (50 events) | Event loop processing order | Prevents stale-tip forks but GSet flooding still consumes event loop | Event processing order helps but NOT sufficient alone |

## Scope-Specific Context

### Hotspots
- **crates/network/src/sync/manager/mod.rs** (critical, 23 touches): M1-M3 redesign: 67→25 fields, 14→3 states, 11→4 production checks. Structurally simplified.
- **crates/network/src/sync/manager/production_gate.rs** (critical, 18 touches): M2 redesign: 590→130 lines. 4 checks replace 11 layers.
- **crates/network/src/sync/manager/cleanup.rs** (critical, 16 touches): Simplified after M1: fork_sync watchdog paths removed.
- **crates/network/src/sync/manager/block_lifecycle.rs** (high, 2 touches): reset_sync_for_rollback() resets consecutive_empty_headers unconditionally, preventing escalation

### Critical Open P0 Findings
- **Fork sync weight check**: Uses <= instead of <, rejecting equal-weight canonical chains during remedial fork recovery. On young networks (all producer weights=1), causes 1-block fork reorgs to always be rejected with delta=0.
- **--no-snap-sync deadlock**: Unconditionally blocks needs_genesis_resync() when snap_sync_threshold=u64::MAX. Recovery path reset_state_only() preserves block data and is compatible with --no-snap-sync.
- **Hardcoded SLOTS_PER_EPOCH**: rebuild_producer_set_from_blocks() uses hardcoded SLOTS_PER_EPOCH (360) instead of network-aware blocks_per_reward_epoch() for epoch boundary detection.

### Open P1 Findings
- **Empty headers blacklist**: Handler always blacklists the responding peer (blaming peer for fork). After 3-4 cycles, all canonical peers are blacklisted.
- **Peer selection degradation**: best_peer_for_recovery() selects max height peer without quality threshold. Under load, canonical peers disconnect.
- **GSet gossip O(N²)**: Creates O(N*mesh_n*announcements) events per round. At 112 nodes with 13 producers: ~100+ events per node per round, almost all DUPLICATE.

### Patterns
GSet gossip flooding is a recurring pattern across multiple incidents — the combinatorial explosion of producer announcements in mesh networks.

## Shared Incidents (Cortex)
No shared incidents found

## Related Incidents
**Active investigating incidents**:
- INC-I-010: Fork-stuck node cannot recover: snap exhausted + no rollback path (ProducerSet divergence issue)

**Recently resolved incidents**:
- INC-I-011: Eviction thrashing RAM explosion at 112 nodes (30s cooldown fix)
- INC-I-008: Production gate blocks majority chain (hash agreement + rebuild ordering)
- INC-I-006: Chain divergence during reorg at epoch boundary (hardcoded SLOTS_PER_EPOCH)
- INC-I-007: Invalid transition Synchronized->DownloadingBodies (state explosion fix)
- INC-I-005: Sync regression during mass sync (recovery gate redesign)

## Key Behavioral Learnings
- **High confidence (0.95)**: When diagnosing consensus fork cascades, check for asymmetric transformations between production and validation scheduler inputs. The DeterministicScheduler is deterministic -- non-determinism comes from its INPUTS.
- **High confidence (0.9)**: Body downloaders must have per-hash failure limits and per-peer tracking. Never retry the same hash from the same peer indefinitely.
- **High confidence (0.9)**: should_sync() must require a minimum gap (>=3 blocks) before triggering a full sync cycle. A 1-2 block gap is normal gossip propagation delay.
- **High confidence (0.9)**: When reactive scheduling modifies ProducerSet flags, those flags must be rebuilt from on-chain block history after ANY state restoration.
- **High confidence (0.9)**: When auditing consensus-critical code, grep for raw constants like SLOTS_PER_EPOCH. Network-aware paths use config.network.*() but fallback/rebuild paths may still use hardcoded mainnet values.

## Key Decisions
- **Sync domain (0.95 confidence)**: Root cause is combinatorial state explosion in SyncManager: 60+ fields with 10+ overloaded interaction points. Each fix adds fields, growing the state space.
- **Scaling domain (0.9 confidence)**: Tier 2 scaling: 4 changes implemented (parallel gossip handlers, always-on delta gossip, scaled channels, fast-path dup check).
- **Sync domain (0.85 confidence)**: Three minimum structural changes needed: (1) decouple production to own tokio::spawn task, (2) replace 9 boolean flags with RecoveryState enum, (3) add global sync serving semaphore.