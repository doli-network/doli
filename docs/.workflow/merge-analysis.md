# Merge Analysis: fix/sync-state-explosion-root-causes vs main

**Date**: 2026-03-28
**Analyst**: Branch Strategist (Opus 4.6)

---

## 1. Executive Summary

**Verdict**: These branches have **deeply diverged** over 9 days with fundamentally different architectural directions. A direct `git merge main` into the source branch would produce **50+ conflict files** with **HIGH resolution difficulty** in the core node modules. The recommended strategy is **selective porting from source into main**, organized by domain, with the sync/network layer being the primary transplant target.

**Key tension**: Main has moved forward with features (DeFi, testing infrastructure, CI, per-byte fees, epoch bond snapshots, excluded_producers round-robin scheduling) while source has done deep structural work on sync, networking, and connection management. Both branches made **incompatible changes** to the production scheduler, fork recovery, and event loop.

---

## 2. Branch Topology

| Dimension | Source (`fix/sync-state-explosion-root-causes`) | Main |
|-----------|------------------------------------------------|------|
| **Commits since divergence** | 96 | 162 |
| **Common ancestor** | `103fc31a` (2026-03-19, "unify mainnet gossip mesh") | same |
| **Last commit** | `f62ca06a` (2026-03-28) — INC-I-016 eviction fixes | `6c54a1c5` (2026-03-27) — v4.9.8 CI validation |
| **Files changed** | 187 (28,361+, 13,733-) | 2,374 (49,332+, 2,067-) |
| **Files unique to branch** | 129 (mostly new network/sync modules, docs) | 2,316 (mostly testnet keys, tests, new crates) |
| **Overlapping files** | 57 | 57 |

**Main's file count is inflated**: 1,024 of the 2,316 unique files are testnet key files. The real unique code delta is ~1,292 files (tests, CLI features, new crates, docs).

---

## 3. Domain Categorization

### Source Branch: Change Domains

| Domain | Commits | Nature | Key Changes |
|--------|---------|--------|-------------|
| **Networking/Sync** | ~45 | Root-cause fixes, structural refactors | Sync manager redesign (M1-M3), recovery gate, state transition validation, fork sync, chain follower, modularized sync_engine into 4 sub-modules, modularized service into 5 files |
| **Connection Management** | ~20 | Root-cause fixes | Yamux buffer tuning, conn_limit scaling, bootstrap-only connections, eviction cooldown, grace period, age tiebreaker, per-peer limits, total caps |
| **Event Loop / Production** | ~10 | Root-cause fixes | Production escape hatch (event flooding starvation), drain-before-produce, genesis hardcoded producers |
| **Fork Recovery** | ~8 | Root-cause fixes | Rejected fork tips set, force_recover_from_peers, snap sync improvements |
| **Rollback** | ~5 | Root-cause fixes | Removed cumulative rollback cap, removed genesis rollback guard, scheduled flag rebuild |
| **Consensus Params** | ~5 | Configuration | testnet epoch=36, bond_unit=1 DOLI, max_peers=25, gossip mesh retuned |
| **Modularization** | ~2 | Structural refactor | 14 modules split to <500 lines (P1+P2), network_events.rs, tx_announcements.rs |
| **Docs/Specs** | ~6 | Documentation | Bugfix docs, redesign specs, troubleshooting |

**Source branch profile**: ~75% root-cause fixes and structural refactors, ~15% configuration, ~10% documentation. The core work addresses incidents INC-I-005 through INC-I-016.

### Main Branch: Change Domains

| Domain | Commits | Nature | Key Changes |
|--------|---------|--------|-------------|
| **DeFi/Features** | ~40 | Feature additions | Bridge commands, lending, pools, NFT batch ops, token management, per-byte fee system, faucet |
| **Testing Infrastructure** | ~30 | Feature additions | TestNetwork, ClusterNetwork (10K nodes), gossip simulation, fork recovery tests, 2213 test files |
| **Production/Scheduling** | ~15 | Root-cause fixes + features | Epoch bond snapshots, excluded_producers round-robin, rank 1 guard, try-apply pattern, scheduler fingerprint |
| **Fork Recovery** | ~8 | Root-cause fixes | Deterministic hash tiebreak (replaces solo-fork heuristic), excluded_producers cleared on rollback, post_rollback flag management |
| **CLI Expansion** | ~15 | Feature additions | Chain commands, service commands, wallet multi-address, parsers, paths module |
| **CI/Release** | ~10 | Infrastructure | GitHub Actions, clippy/fmt fixes, version bumps (v4.9.0-4.9.8) |
| **Consensus Params** | ~5 | Configuration | GENESIS_TIME updated, covenants activation lowered, per-byte fee constants, max_peers=50 |
| **Node Visibility** | ~10 | Refactor | `pub(super)` -> `pub` on all Node methods (for TestNetwork), lib.rs exposure |
| **Testnet Infrastructure** | ~20 | Configuration | Genesis keys (512 per network), scripts, explorer, swap bot |

**Main branch profile**: ~40% feature additions, ~25% test infrastructure, ~20% root-cause fixes, ~15% configuration/CI.

---

## 4. Conflict Assessment — All 57 Files

### CRITICAL (High Risk, Major Manual Work Required) — 10 files

| File | Source Change | Main Change | Risk | Reason |
|------|-------------|-------------|------|--------|
| `bins/node/src/node/mod.rs` | +87/-45: New fields (rejected_fork_tips, had_peers_last_tick, snap_sync_height, bootstrap_peer_ids, etc.), removed cumulative_rollback_depth/seen_blocks_for_slot, changed last_producer_list_change to Arc<RwLock<>> | +158/-55: All fields changed from `pub(super)` to `pub`, added excluded_producers, epoch_bond_snapshot, last_broadcast_gset_len, kept cumulative_rollback_depth/seen_blocks_for_slot | **CRITICAL** | Both add new fields in overlapping regions. Main keeps fields source removes. Visibility changes conflict with structural changes. Every other node file depends on this struct. |
| `bins/node/src/node/event_loop.rs` | +694/-225: Production escape hatch, drain-before-produce, removed mempool registration purge, delta gossip always-on, extracted handle_network_event dispatch to network_events.rs | +98/-45: GSet broadcast tracker, stable sequence numbers, conditional gossip broadcasting, improved toxic TX purge, pub visibility | **CRITICAL** | Both rewrite the gossip timer section differently. Source removes code main improves. Source extracts handle_network_event; main keeps it inline. Core event loop logic diverged. |
| `bins/node/src/node/production/scheduling.rs` | +65/-30: Genesis hardcoded producers (RC-5A fix), last_producer_list_change.read().await | +75/-55: **Complete replacement** of resolve_epoch_eligibility — bond-weighted DeterministicScheduler replaced with round-robin + excluded_producers filter | **CRITICAL** | **Mutually exclusive algorithms.** Source patches the existing bond-weighted scheduler. Main replaces it entirely with round-robin exclusion. Cannot merge — must choose one. |
| `bins/node/src/node/fork_recovery.rs` | +368/-190: rejected_fork_tips tracking, force_recover_from_peers fallback, apply_snap_snapshot (replaced apply_checkpoint_state), finality check in plan_reorg | +87/-55: Deterministic hash tiebreak (replaced solo-fork heuristic), excluded_producers cleared on rollback, post_rollback flag fix | **CRITICAL** | Both rewrite the fork recovery logic. Source adds ~200 new lines for snap snapshot application. Main changes the reorg decision algorithm. The solo-fork removal and hash tiebreak conflict with source's fork tip tracking. |
| `bins/node/src/node/apply_block/mod.rs` | +174/-8: Reactive round-robin scheduling inline (missed slot detection, fallback detection, attestation re-scheduling), prev_slot capture | +2/-2: `pub` visibility only | **HIGH** | Source adds ~140 lines of reactive scheduling logic. Main adds the same conceptual feature (excluded_producers) but in a completely different file (production/mod.rs post_commit_actions). Different implementations of the same idea. |
| `bins/node/src/node/rollback.rs` | +240/-150: Removed cumulative rollback cap, removed genesis rollback guard, new resolve_shallow_fork with stuck_fork_signal, scheduled flag rebuild | +59/-20: excluded_producers cleared on rollback, progress tracking (LAST_ROLLBACK_HEIGHT static), gap expanded to 50, pub visibility | **HIGH** | Both modify resolve_shallow_fork differently. Source rewrites it entirely (removes fork_sync_active check, adds stuck_signal). Main adds progress tracking and gap expansion. Rollback guards removed in source but kept in main. |
| `bins/node/src/node/rewards.rs` | +128/-5: ATTESTATION_QUALIFICATION_THRESHOLD constant, rebuild_scheduled_from_blocks (130-line function for INC-I-006) | +22/-10: epoch_bond_snapshot in reward calculation, pub visibility | **HIGH** | Source adds the rebuild_scheduled_from_blocks function and changes threshold source. Main changes reward calculation to use epoch bond snapshot. Different domains but overlapping file regions (reward calculation uses different bond sources). |
| `bins/node/src/node/periodic.rs` | +286/-130: Removed checkpoint_state consumption, removed seen_blocks cleanup, added tx_announcements expiry, force_recover_from_peers on fork exceeded, bootstrap backoff reset on mass eviction, production escape hatch integration | +15/-5: pub visibility, HEALTH log includes snap_epoch/snap_bonds | **HIGH** | Source removes large chunks of code that main still references (checkpoint_state, seen_blocks_for_slot). Source adds new features (tx_announcements, force_recover). Main's changes are minor but depend on the removed code paths. |
| `bins/node/src/node/init.rs` | +58/-35: New fields (rejected_fork_tips, had_peers_last_tick, snap_sync_height, bootstrap_peer_ids), removed cumulative_rollback_depth/seen_blocks, changed last_producer_list_change to Arc, snap-sync disable, min_peers=2 during genesis | +282/-20: epoch_bond_snapshot init (30 lines), excluded_producers rebuild from blocks (60 lines), announcement_sequence recovery, last_broadcast_gset_len, all new fields from main's mod.rs | **HIGH** | Both add blocks to Node::new(). Source removes fields main keeps. Source changes constructor arguments that main also changes. The Self {} literal at the end of new() will have heavy conflicts. |
| `crates/network/src/sync/manager/sync_engine.rs` | **DELETED** (moved to sync_engine/ directory with 4 sub-modules: decision.rs, dispatch.rs, mod.rs, response.rs) | Modified in place (+34/-32): gap thresholds, log format changes, empty headers handling | **HIGH** | Source deletes the file. Main modifies it. Git will flag this as a delete-vs-modify conflict. Source's changes are the correct resolution (file was restructured), but main's logic changes need to be manually verified against the new sub-module structure. |

### MEDIUM (Requires Careful Merging) — 15 files

| File | Source Change | Main Change | Risk | Reason |
|------|-------------|-------------|------|--------|
| `bins/node/src/node/block_handling.rs` | +332/-200: Removed pre-apply eligibility check, added reorg floor check + finality guard, simplified reorg error handling, scheduled flag rebuild in execute_reorg | +24/-20: Re-added eligibility check, improved error logging, cleared post_rollback flag, pub visibility | **MEDIUM** | Source removes eligibility check that main re-adds. Reorg error handling differs. Both add code to execute_reorg but at different points. |
| `bins/node/src/node/production/mod.rs` | +127/-95: Removed UTXO-based bond derivation (uses bond_weights_for_scheduling), removed inactivity leak comments, cleaned scheduling | +153/-60: bond_weights_for_scheduling via epoch snapshot, scheduler fingerprint logging, rank 1 guard (30 lines), try-apply block pattern (50 lines), pub visibility | **MEDIUM** | Both change how bond weights are sourced. Main adds rank 1 guard and try-apply pattern that source doesn't have. Same direction but different implementations. |
| `bins/node/src/node/startup.rs` | +99/-40: Config env overrides (DOLI_MAX_PEERS, DOLI_CONN_LIMIT, DOLI_YAMUX_WINDOW_SIZE, etc.), snap_sync_height restoration | +15/-5: pub visibility only | **MEDIUM** | Source's env overrides are additive. Main's visibility changes are mechanical. Low semantic conflict but line-level conflicts likely in function signatures. |
| `bins/node/src/node/validation_checks.rs` | +490/-420: Heavy refactoring of validation logic, reactive scheduling integration | +109/-55: Eligibility validation improvements, new validation helpers | **MEDIUM** | Large diff on both sides. Source refactors the structure; main adds new checks. Need line-by-line comparison but likely in different functions. |
| `crates/network/src/service/mod.rs` | +128/-30: Connection limits, peer eviction with age tiebreaker, grace period, DOLI_EVICTION_GRACE_SECS env, bootstrap-only connection type | +13/-9: Minor fixes and adjustments | **MEDIUM** | Source's changes are much larger and mostly additive. Main's changes are small. Likely resolvable but need verification that main's small fixes aren't in regions source rewrote. |
| `crates/core/src/consensus/constants.rs` | Checkpoint comment cleanup, SEED_CONFIRMATION_DEPTH constant, MAX_FALLBACK_RANKS comment cleanup | GENESIS_TIME changed, MAX_BONDS docs improved, per-byte fee constants (BASE_FEE, FEE_PER_BYTE) | **MEDIUM** | Different regions mostly, but GENESIS_TIME and comment changes may overlap at line level. |
| `crates/core/src/consensus/params.rs` | testnet slots_per_reward_epoch=36 (from default) | covenants_activation_height changed for testnet and mainnet | **MEDIUM** | Different fields modified. Low semantic conflict but file is small — hunks may overlap. |
| `crates/core/src/network_params/defaults.rs` | max_peers=25 (testnet), genesis_time=1774721476, gossip mesh retuned (mesh_n varies by network size), bond_unit=1 DOLI | max_peers=50 (both networks), genesis_time=1774083167, gossip mesh unchanged, bond_unit=1 DOLI | **MEDIUM** | Both change max_peers and genesis_time to different values. bond_unit converges. Gossip mesh params conflict. Must decide whose network params win (source is more recent and stress-tested). |
| `bins/node/src/node/production/gates.rs` | +13/-5: Minor gate adjustments | +2/-1: Minor adjustment | **LOW-MED** | Small changes on both sides, likely in different functions. |
| `bins/node/src/node/production/assembly.rs` | +4/-2: Minor changes | +80/-15: Significant additions (try-apply pattern) | **LOW-MED** | Source is trivial; main adds substantial code. Take main's version. |
| `bins/node/src/run.rs` | +16/-30: Removed checkpoint setup code | +25/-2: Added lib exposure code | **MEDIUM** | Source removes code; main adds to same region. |
| `bins/node/src/config.rs` | +22/-1: Env override configs | +28/-3: New config options | **MEDIUM** | Both add to NodeConfig struct. Likely compatible but may conflict at struct definition. |
| `bins/node/src/main.rs` | +9/-13: Cleanup | +41/-9: New command-line args | **MEDIUM** | Both modify arg parsing, may overlap. |
| `crates/mempool/src/pool.rs` | -484/+13: Removed add_transaction, made fields pub(crate) | +160/-2: Added remove_by_error_pattern, new methods | **MEDIUM** | Source deletes method; main adds new methods using the old structure. Source's pub(crate) change is structural. |
| `crates/core/src/lib.rs` | +4/-2: Minor re-exports | +35/-5: More re-exports and new modules | **LOW-MED** | Both add exports. Likely compatible with minor conflicts. |

### LOW (Mostly Auto-Mergeable) — 32 files

| File | Source Change | Main Change | Risk | Reason |
|------|-------------|-------------|------|--------|
| `chainspec.testnet.json` | genesis_time=1774721476 (v94), bond_amount=100000000 | genesis_time=1774083167 (v48b), bond_amount=100000000 | **LOW** | Both change bond_amount identically. Timestamps differ — take source's (more recent). |
| `Cargo.lock` | +17/-16 | +230/-158 | **LOW** | Auto-generated. Rebuild after merge. |
| `.gitignore` | Minor | Minor | **LOW** | Likely non-overlapping additions. |
| `CLAUDE.md` | Minor updates | Minor updates | **LOW** | Documentation, non-functional. |
| `WHITEPAPER.md` | Updates | Updates | **LOW** | Documentation, non-functional. |
| `docs/architecture.md` | Updates | Updates | **LOW** | Documentation. |
| `bins/cli/src/cmd_bridge.rs` | fee 1500->1 (3 lines) | +1141 (major feature expansion) | **LOW** | Source's 3-line change is trivial. Take main's version, apply fee fix. |
| `bins/cli/src/cmd_channel.rs` | 2 line change | 2 line change | **LOW** | Likely identical change. |
| `bins/cli/src/cmd_nft/*.rs` (3 files) | 1-2 line fee changes | Feature additions | **LOW** | Source's changes trivial. Take main's + apply fee fixes. |
| `bins/cli/src/cmd_producer/*.rs` (4 files) | 3-20 line changes (fee reduction, cleanup) | 3-22 line changes (fee reduction, cleanup) | **LOW** | Likely similar/identical changes (both reduce fees). |
| `bins/cli/src/cmd_snap.rs` | +42/-11 | +228/-58 | **LOW** | Main has much more work. Take main's, verify source's snap changes are covered. |
| `bins/cli/src/cmd_token.rs` | 1 line | 9 lines | **LOW** | Take main's. |
| `bins/cli/src/cmd_wallet.rs` | 4 lines | 148 lines | **LOW** | Take main's; source's 4 lines are trivial. |
| `bins/cli/src/commands.rs` | +13 (snap-sync flag) | +425 (many new commands) | **LOW** | Main subsumes source. Verify snap-sync flag exists in main. |
| `bins/cli/src/main.rs` | +7/-2 | +132/-13 | **LOW** | Main subsumes source. |
| `bins/gui/src/commands/producer.rs` | 1 line | 1 line | **LOW** | Likely identical. |
| `bins/gui/src/commands/transaction.rs` | 1 line | 10 lines | **LOW** | Take main's. |
| `bins/node/Cargo.toml` | +1 (libc dep) | +7 (multiple deps) | **LOW** | Both add deps. Merge both. |
| `crates/core/src/network/tests.rs` | Minor | Minor | **LOW** | Tests, non-overlapping likely. |
| `crates/core/src/transaction/core.rs` | Minor | Minor | **LOW** | Small change on both sides. |
| `crates/core/src/validation/transaction.rs` | +2/-6 (fee simplification) | +115/-10 (per-byte fee system) | **LOW** | Source simplifies; main adds new system. Take main's. |
| `crates/crypto/src/lib.rs` | +3/-1 | +1 | **LOW** | Minor re-exports. |
| `crates/mempool/src/policy.rs` | 1 line | 1 line | **LOW** | Likely identical or trivial. |
| `crates/storage/src/utxo_rocks.rs` | +5 | +193/-8 | **LOW** | Source is trivial. Take main's. |
| `crates/network/src/sync/manager/block_lifecycle.rs` | Modularization changes | +7 | **LOW** | Source restructured file. Take source's structure, verify main's 7-line addition is included. |
| `crates/network/src/sync/manager/cleanup.rs` | Modularization | -1 line | **LOW** | Take source's. |
| `crates/network/src/sync/manager/production_gate.rs` | Modularization | +10/-2 | **LOW** | Source restructured. Verify main's changes are in source's version. |

---

## 5. Fundamental Architectural Conflicts

### 5A. Production Scheduler: Bond-Weighted vs Round-Robin

**Source**: Keeps the `DeterministicScheduler` (bond-weighted slot assignment). Adds genesis hardcoded producers for deterministic bootstrap.

**Main**: **Replaces** `DeterministicScheduler` with simple `slot % sorted_producers.len()` round-robin. Adds `excluded_producers` set for liveness filtering. Adds `epoch_bond_snapshot` for deterministic bond counts.

**Impact**: These are **mutually exclusive approaches**. The round-robin approach on main is a deliberate architectural choice (simpler, avoids bond-weight divergence). Source's bond-weighted approach is the original design.

**Resolution**: Must decide which scheduler to use. Main's round-robin + exclusion system is newer and integrated with its testing infrastructure. Source's bond-weighted approach would need the epoch_bond_snapshot from main to be deterministic.

### 5B. Node Struct Visibility: pub(super) vs pub

**Source**: Keeps `pub(super)` on all Node fields (encapsulation).

**Main**: Changes **every field** to `pub` and **every method** to `pub`. This is for `TestNetwork` integration testing, which creates Node instances and manipulates their internals.

**Impact**: Every overlapping file in `bins/node/src/node/` will have visibility conflicts. This is a global find-replace on main that touches every function signature.

**Resolution**: If keeping TestNetwork, must accept `pub` visibility. Mechanical but tedious.

### 5C. Sync Engine Structure

**Source**: Completely restructured `sync_engine.rs` into a directory with 4 sub-modules. Added 6+ new sync modules (headers.rs, bodies.rs, chain_follower.rs, fork_recovery.rs, fork_sync.rs, adversarial_tests.rs). This is the core architectural contribution.

**Main**: Made incremental fixes to the original single-file `sync_engine.rs`.

**Impact**: Main's sync_engine changes are **obsoleted** by source's restructuring. The logic changes from main need to be verified against source's new structure.

### 5D. Event Loop Architecture

**Source**: Adds production escape hatch, drain-before-produce, removes old mempool purge. Extracts handle_network_event to separate `network_events.rs` module.

**Main**: Keeps event loop structure, adds GSet broadcast tracking, improves mempool purge (toxic TX pattern matching), adds `last_broadcast_gset_len`.

**Impact**: Source's event loop is fundamentally different (escape hatch changes scheduling fairness). Main's GSet tracking is valuable but implemented differently from source's approach (source makes delta gossip always-on).

---

## 6. Merge Strategy Recommendation

### GATING DECISION (2026-03-28): Keep Main's Round-Robin Scheduler

**Decision**: Main's round-robin scheduler (`slot % N`) with epoch bond-weighted reward distribution is the production design. Source's `DeterministicScheduler` (bond-weighted slot assignment) is not being ported.

**Rationale**: Both approaches produce identical economic outcomes (rewards proportional to stake). Main's design decouples block production (equal turns) from reward distribution (bond-weighted at epoch boundary). This is simpler to audit, distributes production load evenly, and matches the Ethereum validator model. Confirmed by project owner.

**Impact on merge**: Eliminates Risk #1 (scheduler divergence), eliminates Risk #1 from independent verification (ProducerInfo.scheduled serialization break), and removes ~15 source files/changes from the port scope.

### Recommended: Selective Port from Source into Main (4 layers)

A direct `git merge main` into the source branch is **NOT recommended** due to:
- 57 conflict files with 10 CRITICAL conflicts
- Global visibility changes on main
- Delete-vs-modify on sync_engine.rs
- Wire protocol changes in sync message types
- Estimated resolution effort: **3-5 days** of careful manual conflict resolution with high risk of regression

Instead, port source branch's networking/sync improvements into main in 4 prioritized layers:

#### Layer 0: Wire Protocol + Sync Message Types (PREREQUISITE)
Port the sync request/response type changes that all sync modules depend on.

**Changes**:
- Remove old variants: `GetStateAtCheckpoint`, `GetBlocksByHeightRange`, `StateAtCheckpoint`, `Block(Box<Option<Block>>)`
- Add new variants: `GetStateSnapshot`, `GetStateRoot`, `GetHeadersByHeight`, `StateSnapshot`, `StateRoot`, `Block(Option<Block>)`
- Update all handlers in validation_checks.rs that process sync messages

**Risk**: MEDIUM. Wire protocol changes require all nodes to upgrade simultaneously. Must coordinate with testnet deploy.

#### Layer 1: Network/Sync Core (HIGHEST VALUE, HIGHEST RISK)
Port the sync manager redesign and connection management fixes.

**Files to port** (unique to source — no conflicts):
- `crates/network/src/sync/manager/sync_engine/` — the 4 sub-modules (decision.rs, dispatch.rs, mod.rs, response.rs) replacing sync_engine.rs
- `crates/network/src/sync/manager/peers.rs`, `snap_sync.rs`, `types.rs`, `tests.rs`
- `crates/network/src/sync/reorg/` — mod.rs + tests.rs (replaces reorg.rs)
- `crates/network/src/sync/adversarial_tests.rs`
- `crates/network/src/service/` — 5 new sub-modules
- `crates/network/src/behaviour.rs`, `config.rs`, `gossip/`, `lib.rs`, `peer.rs`, `protocols/`, `rate_limit.rs`

**Files requiring careful merge** (overlapping):
- `crates/network/src/service/mod.rs` — merge source's connection limits + eviction into main's version
- `crates/network/src/sync/manager/block_lifecycle.rs`, `cleanup.rs`, `production_gate.rs` — verify main's small fixes exist in source's restructured versions

**CRITICAL: Back-port main's 4 sync fixes into source's new structure:**
1. Gap threshold 12→50 in `dispatch.rs` (source has 50 in response.rs but 12 in dispatch.rs)
2. Don't inflate sync_failures on empty headers (source still increments)
3. Genesis height guard in production gate (source lacks this)
4. `set_post_rollback()` method (source doesn't have it)

**Structural integration required** (NOT a file copy):
- Source uses `self.fork.*` (ForkState sub-struct), main uses `self.*` (flat fields)
- Must add `ForkState`, `SyncPipelineData` types to main's manager
- Must add `set_state()`, `state_label()` methods
- Source deleted `chain_follower.rs` and `fork_sync.rs` that main still has — remove after port

**Risk**: HIGH. The sync restructuring is the most valuable change but requires coordinated integration, not file copying.

#### Layer 2: Node-Level Fixes (HIGH VALUE, MEDIUM RISK)
Port incident fixes from source, adapted to main's scheduler model.

**Cherry-pick candidates** (by incident):
- **INC-I-014**: Rejected fork tips, connection caps, drain-before-produce (fork_recovery.rs, event_loop.rs, periodic.rs)
- **INC-I-013**: Bootstrap-only connections, Yamux tuning, env overrides (startup.rs, config.rs)
- **INC-I-012**: Production escape hatch — anti-starvation for biased select! under 100+ peer load (event_loop.rs)
- **INC-I-016**: Eviction grace period, age tiebreaker (network service)
- **INC-I-006**: rebuild_scheduled_from_blocks (rewards.rs)
- **INC-I-005**: Rollback safety improvements, sync cascade prevention

**Conflicts to resolve**:
- `mod.rs` — add source's new fields (`rejected_fork_tips`, `had_peers_last_tick`, `snap_sync_height`, `bootstrap_peer_ids`) while keeping main's `pub` visibility, `excluded_producers`, `epoch_bond_snapshot`, `last_broadcast_gset_len`
- `event_loop.rs` — integrate production escape hatch + drain-before-produce while keeping main's GSet broadcast tracking, announcement sequence stabilization, and toxic TX purge (`remove_by_error_pattern`)
- `fork_recovery.rs` — integrate rejected_fork_tips + apply_snap_snapshot while keeping main's deterministic hash tiebreak + excluded_producers clearing
- `rollback.rs` — keep main's cumulative rollback cap (50) + LAST_ROLLBACK_HEIGHT progress tracking + excluded_producers clearing. Port source's `rebuild_scheduled_from_blocks` call.
- `init.rs` — merge both sets of new fields into Node::new()
- `network_events.rs` — new file from source (524 lines), adapt all `pub(super)` to `pub`
- `tx_announcements.rs` — new file from source (189 lines), adapt visibility

**What NOT to port** (dead code under main's scheduler):
- Source's `DeterministicScheduler` rebuild in scheduling.rs
- Source's genesis hardcoded producers (RC-5A) — only needed for bond-weighted
- Source's `cached_scheduler` field changes
- Source's `last_producer_list_change` type change (`Option<Instant>` → `Arc<RwLock<>>`)
- Source's `scheduled` flag on `ProducerInfo` (would break serialization)
- Source's removal of `cumulative_rollback_depth` and `seen_blocks_for_slot` — main uses these
- Source's reactive round-robin scheduling in apply_block/mod.rs (~150 lines) — main uses excluded_producers instead

**What to port from source that main lacks**:
- `gset.purge_stale(14400)` — removes ghost producers after 4 hours
- Delta gossip always-on (`producer_count > 0` threshold)
- `rebuild_scheduled_from_blocks()` after rollback (INC-I-006)

**What to keep from main that source lacks**:
- `epoch_bond_snapshot` for deterministic reward calculation (N5 testnet fix)
- `remove_by_error_pattern()` toxic TX purge (improved over source's removal)
- `LAST_ROLLBACK_HEIGHT` progress-based rollback skip
- Shallow fork window expansion (12→50)
- `excluded_producers.clear()` after rollback

#### Layer 3: Configuration (LOW RISK)
Network parameter decisions — all resolved.

| Parameter | Source | Main | **Decision** |
|-----------|--------|------|-------------|
| `max_peers` | 25 (stress-tested at 200 nodes) | 50 | **25** — battle-tested |
| `genesis_time` | 1774721476 (v94, 2026-03-28) | 1774083167 (v48b, 2026-03-21) | **source** — active testnet |
| `gossip mesh` | Retuned for network size | Default | **source** — stress-tested |
| `slots_per_reward_epoch` | 36 (testnet) | default | **source** — fast epochs for testing |
| `chainspec.testnet.json` | v94 genesis | v48b genesis | **source** |

---

## 7. Prioritized Merge Order

```
Step 1: Create integration branch from main
        git checkout -b integrate/sync-fixes main

Step 2: Port Layer 0 (wire protocol) + Layer 3 (configuration) — lowest risk, establishes baseline
        - Sync message type changes
        - chainspec.testnet.json
        - network_params/defaults.rs
        - consensus/constants.rs, params.rs
        Run: cargo build && cargo test

Step 3: Port Layer 1 (sync/network) — highest value, coordinated integration
        - Add types.rs (ForkState, SyncPipelineData)
        - Restructure manager/mod.rs (add fork sub-struct, set_state method)
        - Replace sync_engine.rs with sync_engine/ directory
        - Add new sync modules (peers.rs, snap_sync.rs, reorg/)
        - Back-port main's 4 sync fixes into new structure
        - Merge network service changes (connection limits, eviction)
        - Remove obsoleted files (chain_follower.rs, fork_sync.rs)
        Run: cargo build && cargo test

Step 4: Port Layer 2 (node-level fixes) — requires most judgment
        - mod.rs (add source's fields, keep main's visibility + fields)
        - init.rs (merge both constructors)
        - event_loop.rs (escape hatch + main's GSet tracking + toxic TX purge)
        - network_events.rs + tx_announcements.rs (new files, adapt to pub)
        - fork_recovery.rs (rejected tips + snap snapshot + main's hash tiebreak)
        - rollback.rs (keep main's guards + add rebuild_scheduled_from_blocks)
        - block_handling.rs (reorg floor + finality guard)
        - periodic.rs (new periodic tasks + keep main's HEALTH log fields)
        - rewards.rs (rebuild_scheduled_from_blocks + keep epoch_bond_snapshot)
        - validation_checks.rs (largest diff — adapt source's changes to main's scheduler model)
        Run: cargo build && cargo clippy -- -D warnings && cargo test

Step 5: Verify no regressions
        - Run TestNetwork/ClusterNetwork tests (from main)
        - Run stress test scenarios
        - Verify all pub visibility intact for TestNetwork
```

---

## 8. Risk Assessment (Updated Post-Gating Decision)

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ~~Scheduler divergence~~ | ~~HIGH~~ | ~~Chain fork~~ | **ELIMINATED** — keeping main's round-robin |
| ~~ProducerInfo serialization break~~ | ~~HIGH~~ | ~~State root divergence~~ | **ELIMINATED** — not porting scheduled flag |
| Sync regression from incomplete port | MEDIUM | Nodes stuck | Port entire sync layer as a unit + back-port main's 4 fixes |
| Visibility conflicts cause compile errors | HIGH | Build failure | Resolve mechanically — all `pub(super)` from source → `pub` to match main |
| Missing field initialization in Node::new() | HIGH | Runtime panic | Carefully merge init.rs — every field in mod.rs must be initialized |
| Wire protocol break during rollout | MEDIUM | Network partition | Coordinate testnet deploy — all nodes must upgrade simultaneously |
| Main's 4 sync fixes lost in restructuring | MEDIUM | Sync regression | Explicit back-port checklist (gap threshold, empty headers, genesis guard, post_rollback) |
| Event loop behavior mismatch | LOW-MEDIUM | Production starvation or TX leaks | Keep BOTH escape hatch (source) AND toxic TX purge (main) |
| Network crate API divergence | MEDIUM | Compile failure | Source exports ForkAction, VerifiedSnapshot etc. that main must import; source removes ForkSync that main must stop importing |

---

## 9. What Source Makes Obsolete on Main

- Main's incremental sync_engine.rs fixes (gap thresholds, log formatting) — **obsoleted** by source's complete restructuring (but main's logic changes must be verified/back-ported)
- Main's solo-fork heuristic in fork recovery — **obsoleted** by source's rejected_fork_tips + snap recovery approach (main's hash tiebreak is independently valuable and KEPT)
- Main's inline handle_network_event — **obsoleted** by source's network_events.rs extraction

## 10. What Main Makes Obsolete on Source

- Source's bond-weighted DeterministicScheduler patches — **OBSOLETED** (gating decision: main's round-robin wins)
- Source's `scheduled` flag on ProducerInfo — **OBSOLETED** (main's ephemeral excluded_producers wins)
- Source's `cached_scheduler` field and genesis hardcoded producers — **OBSOLETED**
- Source's `last_producer_list_change` Arc<RwLock<>> change — **OBSOLETED**
- Source's reactive round-robin in apply_block/mod.rs (~150 lines) — **OBSOLETED**
- Source's flat fee (fee_units=1) in CLI commands — **obsoleted** by main's per-byte fee system
- Source's mempool field visibility changes — **obsoleted** by main's more complete mempool expansion
- Source's incremental CLI changes — **obsoleted** by main's massive CLI expansion
- Source's toxic TX purge removal — **OBSOLETED** by main's improved `remove_by_error_pattern()`

---

## 11. Files That Can Be Safely Ignored (Identical or Trivially Compatible Changes)

These 15 files have changes that are either identical on both branches or where one side's change is a strict subset:
- `bins/cli/src/cmd_channel.rs` — likely identical
- `bins/cli/src/cmd_producer/exit.rs` — likely identical
- `bins/cli/src/cmd_producer/withdrawal.rs` — likely identical
- `bins/gui/src/commands/producer.rs` — likely identical
- `crates/mempool/src/policy.rs` — likely identical
- `crates/crypto/src/lib.rs` — compatible additions
- `.gitignore` — compatible additions
- All CLI files where source only changes fee values — take main's version (per-byte fee system supersedes)

---

## 12. Estimated Effort (Updated)

| Approach | Effort | Risk | Confidence |
|----------|--------|------|------------|
| ~~Direct git merge~~ | ~~3-5 days~~ | ~~HIGH~~ | ~~LOW~~ |
| **Selective port source→main (Layer 0+1+2+3)** | **3-4 days** | **MEDIUM** | **MEDIUM-HIGH** |
| Selective port Layer 0+1 only (sync/network) | 1.5 days | LOW-MEDIUM | HIGH |
| Abandon source, cherry-pick only network config | 0.5 days | LOW | HIGH (but loses sync work) |

**Recommendation**: Selective port (all 4 layers) into main. The sync/network restructuring (4,281 lines addressing INC-I-005 through INC-I-016) is too valuable to lose. Main's 162 commits of feature work + round-robin scheduler are the base. Estimated 3-4 days (prior estimate of 2-3 was low — Layer 1 requires coordinated integration, not file copying, and main's 4 sync fixes must be back-ported into source's restructured modules).
