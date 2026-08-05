<!-- @INDEX
ENTRY-POINTS    13-50
OPERATIONS      52-68
DATA-FLOWS      70-152
STRUCTS         154-176
FUNCTIONS       178-280
DEPENDENCIES    282-296
CONSTRAINTS     298-342
PATTERNS        344-374
@/INDEX -->
# node — DOLI Node Binary

## ENTRY-POINTS

`bins/node/src/main.rs:36` — `async fn main()` — CLI dispatch: parses args, sets up logging, loads data_dir, calls run_node()/run_bootnode()/operations::*/commands::*

`bins/node/src/run.rs:18` — `async fn run_node(...)` — Full node startup: loads keys (producer + BLS), config, chainspec, spawns update service, constructs Node, runs archiver, blocks on SIGINT/SIGTERM with 30s graceful-shutdown timeout

`bins/node/src/run.rs:635` — `async fn run_bootnode(...)` — Lightweight UDP-only bootnode: no storage/libp2p/sync, Discv5 peer discovery only (~2MB RAM vs 200MB+ full node)

`bins/node/src/node/startup.rs:5` — `Node::run()` — Top-level lifecycle: placeholder-key guard (refuses mainnet) → start_network → self-register producer (testnet/devnet/genesis) → genesis VDF → start_rpc → run_event_loop → shutdown

`bins/node/src/node/startup.rs:141` — `Node::start_network()` — Builds NetworkConfig (dynamic/fixed gossip mesh, fork_id, discv5, peer cache path), starts NetworkService

`bins/node/src/node/startup.rs:295` — `Node::start_rpc()` — Builds RpcContext (56 methods), wires broadcast/vote/update-status callbacks, spawns RpcServer + WebSocket

`bins/node/src/node/event_loop.rs:5` — `Node::run_event_loop()` — Biased `tokio::select!`: network events (highest) → production timer (1s/200ms devnet) → gossip anti-entropy timer. Production escape-hatch forces production+periodic under event flood

`bins/node/src/node/event_loop.rs:274` — `Node::handle_network_event()` — Dispatches all `NetworkEvent` variants to `network_events.rs` / `block_handling.rs` handlers

`bins/node/src/node/init.rs:156` — `Node::new(...)` — Opens BlockStore + StateDb, migrates legacy files, wipes state on genesis-hash mismatch, loads EpochState (unified key → individual keys → UTXO rebuild fallback), builds full Node struct

`bins/node/src/node/init.rs:1056` — `Node::new_for_test(...)` — Minimal Node for integration tests: real RocksDB/ProducerSet/SyncManager, no networking

`bins/node/src/node/init.rs:1253` — `Node::new_for_replay(data_dir, network)` — Headless node for disaster-recovery replay: existing block_store + FRESH state_db, genesis producers from chainspec, no networking

`bins/node/src/lib.rs:11` — lib re-exports: `pub mod config`, `pub mod metrics`, `pub mod node`, `pub mod producer`, `pub mod updater` for integration-test access

Commands dispatched from `main.rs:150`:
- `Run{bootnode:true}` → `run::run_bootnode()`; else `run::run_node()`
- `Init` → `operations::init_data_dir()`
- `Status` → `operations::show_status()`
- `Import/Export` → `operations::import_blocks()`/`export_blocks()`
- `Restore{from_rpc|from,backfill}` → `operations::restore_from_rpc()`/`restore_from_archive()`/`backfill_from_archive()`
- `Truncate` → `operations::truncate_chain()`
- `Recover` → `operations::recover_chain_state()`
- `Reindex` → `operations::reindex_canonical_chain()`
- `Devnet` → `commands::handle_devnet_command()`
- `Update/Maintainer/Release/Upgrade` → `commands::handle_*_command()`
- `CheckpointInfo` → prints compiled-in `CHECKPOINT_HEIGHT/HASH/STATE_ROOT`

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Start a producing node | 1. `doli-node run --producer` 2. loads producer key + BLS key 3. `producer::startup_checks()` (lock file, signed-slots DB, duplicate-key detection) 4. `Node::run()` | `run::run_node()`, `producer::startup_checks()`, `Node::run()` | producer key file, `--force-start` to skip dup-key check | node reaches `run_event_loop`, RPC responds, peers connect |
| Start a non-producing node | 1. `doli-node run` (no `--producer`) | `run::run_node()` | none | syncs via gossip/sync, RPC live |
| Start a bootnode | 1. `doli-node run --bootnode` | `run::run_bootnode()` | p2p/discv5 ports | Discv5 ENR serves peer discovery |
| Produce a block | 1. production timer/escape-hatch fires 2. version+hardfork+auth gates 3. eligibility (bootstrap or epoch) 4. `build_block_content()` 5. VDF 6. `apply_block(Light)` 7. broadcast | `Node::try_produce_block()`, `build_block_content()`, `compute_block_vdf()` | producer_key, active bond weight >0 | block applied locally + broadcast; `[BLOCK_PRODUCED]` log |
| Apply a received block | 1. gossip/sync delivers block 2. `classify_gossip_block()` 3. route: ExtendsTip→apply / ForkBlock→cache-or-reorg / Orphan→chase parent | `Node::handle_new_block()`, `classify_gossip_block()`, `Node::apply_block()` | valid Block | height advances, state root cached, `[APPLY_END] status=applied` |
| Recover from a fork | 1. periodic tick reports evidence (empty headers, stale tip) 2. `SyncManager::classify_and_dispatch()` returns action 3. dispatch: ShallowRollback/HeaderFirstSync/SnapSync/GenesisResync | `Node::run_periodic_tasks()` RECOVERY COORDINATOR block, `Node::rollback_one_block()` | stuck-fork/stale-tip signal | chain converges on canonical tip |
| Execute a reorg | 1. reorg_result from SyncManager or fork_recovery 2. verify block_store completeness (`FORK_GUARD_BACKFILL_REQUIRED` else) 3. undo-based rollback to common ancestor 4. apply new_blocks in slot order | `Node::execute_reorg()` | `ReorgResult`, cached/stored fork blocks | tip = heavier chain, mempool revalidated |
| Roll back one block | 1. undo-based restore (UTXO+ProducerSet+EpochState from undo log) 2. safety caps (no genesis, max 50 cumulative) | `Node::rollback_one_block()` | undo data at height, or block-1 present for legacy rebuild | height-1, `[FORK] ROLLBACK_DONE` |
| Recover from snap sync | 1. SyncManager completes quorum-verified snapshot download 2. re-verify state_root 3. replace 3 states 4. load EpochState (fast-path bytes or legacy reconstruction) | `Node::apply_snap_snapshot()` | `VerifiedSnapshot` | `snap_sync_height` set, Light-mode window until next epoch boundary |
| Calculate epoch rewards | 1. scan epoch blocks 2. decode attestation bitfield per block 3. tier1/2/3 qualification 4. bond-weighted split incl. delegation | `Node::calculate_epoch_rewards(epoch)` | complete local block_store for the epoch range | `Ok(Vec<(amount,pubkey_hash)>)` or `Err(IncompleteEpochStoreError)` |
| Restore a node from a peer/archive | 1. `doli restore --from-rpc <URL>` or `--from <archive_path>` | `operations::restore_from_rpc()`, `restore_from_archive()`, `backfill_from_archive()` | RPC URL (never peer ID) or archive dir | data_dir rebuilt, node startable |
| Create a manual/auto checkpoint | 1. `auto_checkpoint_interval` elapses OR `createCheckpoint` RPC 2. RocksDB checkpoint (state_db + blocks) 3. `health.json` tagged via `decide_checkpoint_health()` 4. rotate keeping last 5 (protects most-recent healthy) | `Node::run_periodic_tasks()` AUTO-CHECKPOINT block, `checkpoint_health::decide_checkpoint_health()`, `select_checkpoint_evictions()` | `--auto-checkpoint N` | checkpoint dir + `health.json` written |
| Enter/exit recovery mode | 1. `enterRecoveryMode`/`exitRecoveryMode` RPC | RPC → `Node.recovery_mode: AtomicBool` | admin token | `apply_block`/`apply_snap_snapshot` silently drop inbound mutations until exit |

## DATA-FLOWS

**Block ingestion (gossip):**
```
NetworkEvent::NewBlock → on_new_block_event() [snap-sync/dup/stale/rejected-fork-tip filters]
  → handle_new_block() → classify_gossip_block() → [ExtendsTip | ForkBlock | Orphan | Rejected]
  → ExtendsTip: apply_block(Full|Light) → drain cached orphans on new tip (max 50)
  → ForkBlock(HeightOccupied,is_better): signal_stuck_fork() → RecoveryCoordinator
  → ForkBlock(ReorgCandidate): cache_block_with_eviction() → handle_new_block_weighted() → execute_reorg()
      or start_fork_recovery()/try_apply_cached_chain() fallback
  → Orphan: ORPHAN_CHASE — GetBlockByHeight(need_height) direct to sender
```

**Block application (`apply_block/mod.rs:16`):**
```
Block → recovery_mode/snap_height/duplicate guards
  → validate_block_for_apply() → validate_block_economics()
  → begin_batch() [StateDb.BlockBatch]
  → per-tx: process_transaction_utxos() [batch UTXO r/w, undo capture]
         + process_transaction_producer_effects() [ProducerSet deferred queue_update]
         + process_transaction_governance() [Maintainer immediate; ProtocolActivation deferred]
  → process_unbonding() → update_known_producers() → update_chain_state_for_block()
  → track_finality_and_apply_deferred() [epoch-0 or boundary: apply_pending_updates_with_cap()]
  → maybe_complete_genesis() [height == genesis_blocks+1 only]
  → post_commit_actions(): recompute_active_status, attestation bitfield decode+accumulate,
      batch.put_epoch_state() every block, epoch-boundary derive_at_boundary(),
      aggregate_oracle_prices_at_epoch_boundary() [pre-rotation, gated by oracle_activation_height],
      create_and_broadcast_attestation, archive buffering, WS broadcast
  → batch.put_undo() → batch.commit() [atomic: UTXO+ProducerSet+ChainState+EpochState+undo]
  → refresh_mempool_producer_snapshot() → prune_undo_before() → [STATE_FP] fingerprint log
```

**Block production (`production/mod.rs:31`):**
```
production_timer/escape-hatch → try_produce_block()
  → is_production_allowed() gate → hardfork_schedule.should_stop_producing() gate
  → last_produced_slot fast-path → handle_production_authorization() [SyncManager.can_produce()]
  → behind-network-height guard → resolve_bootstrap_eligibility() OR resolve_epoch_eligibility()
  → is_producer_eligible_ms() window check → rank-1 offline-guard → propagation-delay wait
  → build_block_content(): epoch-reward coinbase (at epoch start, aborts slot on
      IncompleteEpochStoreError), genesis VDF Registration TX, mempool select+validate,
      attestation bitfield encode [base(epoch_producer_list) | extra sorted]
  → drain_pending_events() [abort if chain advanced] → compute_block_vdf()
  → post-VDF stale-parent/dup-slot re-check → apply_block(Light) [BLOCK_POISON rollback+purge on fail]
  → broadcast (unless BEHIND_TIP_SUPPRESS) → attest_own_block() → flush_finalized_to_archive()
```

**Epoch boundary (height % blocks_per_epoch == 0):**
```
build_block_content() → calculate_epoch_rewards(epoch) → EpochReward TX (or slot-abort on Err)
apply_block() → post_commit_actions() → EpochDerivationInput → EpochState::derive_at_boundary()
  → aggregate_oracle_prices_at_epoch_boundary() [uses CLOSING epoch bond_snapshot, pre-rotation]
  → self.epoch_state = new_state (rotation) → minute_tracker.reset()
```

**Fork recovery (`fork_recovery.rs`):**
```
signal_stuck_fork()/report_stale_tip()/report_empty_headers() → run_periodic_tasks()
  RECOVERY COORDINATOR → SyncManager::classify_and_dispatch() → RecoveryAction
  → ShallowRollback{depth}: rollback_one_block() × depth
  → CompletedForkRecovery: handle_completed_fork_recovery()
      → record_fork_block_weight() → cache blocks → check_reorg_weighted() (simple)
        or plan_reorg() (deep) → execute_reorg() if weight_delta>0 or hash-tiebreak
        else try_apply_direct_successor() (INV-SYNC-003 fallback) or drop
```

**Snap sync:**
```
SyncManager completes quorum vote+download → take_snap_snapshot() (periodic)
  → apply_snap_snapshot(): verify state_root → deserialize 3 states → replace local
  → atomic_replace() to StateDb → UtxoSet::from_state_db() (INC-I-118: avoid frozen copy)
  → EpochState: fast-path from snapshot.epoch_state_bytes, else legacy reconstruction
      (bond snapshot 3-source fallback → attestation accumulators → derive_at_boundary)
  → snap_sync_height=Some(h) → clear fork_block_cache → set_store_floor()
```

**Rollback/reorg epoch-state restore (INC-I-071/082):**
```
undo.producer_snapshot: empty Vec = sentinel "ProducerSet unchanged this block" (skip restore)
undo.epoch_state_snapshot: always present, deserialize → self.epoch_state
  missing/legacy → rebuild_epoch_state_from_blocks(target_height) [explicit param,
    no chain_state/state_db read dependency — INC-I-082 Defect 2 fix]
```

## STRUCTS

`bins/node/src/node/mod.rs:79` — **`Node`** — ~60 fields. Key additions since last audit:
- `epoch_state: doli_core::EpochState` — producer_list, active_list, bond_snapshot, attested_sets[3], attestation_accum[3], blocks_produced (single source of truth for scheduler)
- `recovery_mode: Arc<AtomicBool>` — anti-poisoning gate, cleared only via RPC, non-persistent
- `oracle_sunset_triggered: Arc<AtomicBool>` — Phase 2.1 Oracle M8; wired into every ValidationContext; recomputed at each epoch boundary from `state_db`-persisted `OracleSunsetState`
- `mempool_active_producers_snapshot: Arc<std::sync::RwLock<Vec<(PublicKey,u64)>>>` — AUDIT-P1-001; shared with mempool for oracle-attestation auth; refreshed after every apply_block
- `health_window: VecDeque<bool>` — INC-I-055 rolling checkpoint-health samples (size `CHECKPOINT_HEALTH_WINDOW_SIZE`=20)
- `attest_fetch_tracker: HashMap<Hash,(Instant,u8,PeerId)>` — INC-I-049 deferred attestation-triggered fetch (500ms grace, max 3 peers/hash, 30s TTL)
- `defi_health_cache: std::sync::Mutex<Option<((u64,Option<(Hash,u64)>),Instant)>>` + `defi_health_refresh_counter: AtomicU64` — INC-I-111, 30s TTL cache for Prometheus DeFi gauges
- `peer_churn: HashMap<PeerId,VecDeque<Instant>>` — Fix 5a 2026-04-15, rate-limits flapping peers (5 events/30s)
- `rejected_fork_tips: HashSet<Hash>` — INC-I-014, capped at 1000
- `fork_block_cache: Arc<RwLock<HashMap<Hash,Block>>>` — max 100, slot-sorted eviction, 30-slot TTL sweep in periodic

`bins/node/src/node/block_handling.rs:11` — **`BlockClass`** (pure) — `ExtendsTip | ForkBlock(ForkBlockKind) | Orphan{need_height} | Rejected(String)`

`bins/node/src/node/block_handling.rs:25` — **`ForkBlockKind`** — `HeightOccupied{fork_height,canonical_slot,is_better} | ReorgCandidate`

`bins/node/src/node/rewards.rs:11` — **`IncompleteEpochStoreError`** — INC-I-081; distinct from `Ok(Vec::new())` ("no qualifiers, pool accumulates"). Fields: epoch, epoch_start/end_height, gap_count, silent_bitfield_count

`bins/node/src/node/checkpoint_health.rs:10` — **`CheckpointHealthDecision`** — `{healthy, isolated, self_consistent}`, produced by `decide_checkpoint_health()`

`bins/node/src/config.rs:11` — **`NodeConfig`** — network, data_dir, listen_addr, bootstrap_nodes, max_peers, rpc, no_dht, relay_server, genesis_time_override, chainspec, slot_duration_override, no_snap_sync, seed_mode, auto_checkpoint_interval, bootnode_enrs, no_discv5, discv5_port, external_address

## FUNCTIONS

### Node lifecycle (`startup.rs`, `event_loop.rs`)
`startup.rs:5` `Node::run()` — orchestrates full startup through shutdown
`startup.rs:141` `Node::start_network()` — NetworkConfig incl. mainnet-fixed eager mesh (mesh_n=12) vs `compute_dynamic_mesh()` elsewhere
`startup.rs:295` `Node::start_rpc()` — 56-method RpcContext wiring
`startup.rs:530` `Node::recompute_active_status(height)` — once/epoch, sets `is_active_producer`
`startup.rs:569` `Node::create_and_broadcast_attestation()` — signs + gossips + direct-delivers to next-slot producer (v5+ peers only)
`event_loop.rs:5` `Node::run_event_loop()` — biased select w/ production escape-hatch
`event_loop.rs:274` `Node::handle_network_event()` — full NetworkEvent dispatch table

### Block handling (`block_handling.rs`)
`block_handling.rs:42` `classify_gossip_block()` — pure, no side effects
`block_handling.rs:116` `Node::handle_new_block()` — classify+dispatch+drain-orphans
`block_handling.rs:429` `Node::execute_reorg()` — FORK_GUARD_BACKFILL_REQUIRED completeness check before mutating chain_state; undo-based (has_undo all-heights) or legacy rebuild fallback

### Block application (`apply_block/`)
`apply_block/mod.rs:16` `Node::apply_block(block,mode)` — full pipeline; INC-I-071 empty-snapshot optimization decided by `helpers::block_mutates_producer_set()`
`apply_block/tx_processing.rs:12` `process_transaction_utxos()` — batch-only (Phase 3, no utxo_store writes); NFT/Pool unique-ID rejection; ValidationMode::Replay tolerates historical spend failures
`apply_block/tx_processing.rs:193` `process_transaction_producer_effects()` — Register/Exit/Slash/AddBond/RequestWithdrawal/DelegateBond/RevokeDelegation, all epoch-deferred; INC-I-078 auth+cap gating on Delegate*
`apply_block/governance.rs:10` `process_transaction_governance()` — AddMaintainer/RemoveMaintainer immediate; ProtocolActivation verified against on-chain (or ad-hoc) MaintainerSet
`apply_block/genesis_completion.rs:14` `maybe_complete_genesis()` — runs exactly at `genesis_blocks+1`; consumes pool UTXOs via batch, creates real Bond UTXOs
`apply_block/state_update.rs:40` `update_chain_state_for_block()` — 3-phase lock (metadata write → state-root compute under read locks → publish cache)
`apply_block/state_update.rs:152` `track_finality_and_apply_deferred()` — finality weight tracking + `apply_pending_updates_with_cap()` at epoch-0/boundary
`apply_block/post_commit.rs:10` `post_commit_actions()` — attestation decode (base+extra full-decode post-activation) + epoch derivation + oracle aggregator call + attestation + archive + WS
`apply_block/oracle.rs:66` `aggregate_oracle_prices_at_epoch_boundary()` — Phase 2.1 Oracle M6; MUST run before `self.epoch_state` rotation; gated `< oracle_activation_height` no-op; aborts (AUDIT-P1-002) on missing closing-epoch block
`apply_block/helpers.rs:13` `block_mutates_producer_set()` — pure predicate for undo-snapshot decision

### Block production (`production/`)
`production/mod.rs:18` `should_defer_epoch_production()` — post-restart grace (5s devnet/15s else)
`production/mod.rs:31` `try_produce_block()` — full gate chain → build → VDF → apply → broadcast
`production/gates.rs:21` `handle_production_authorization()` — delegates to `SyncManager.can_produce()`, resets rollback counters on Authorized
`production/scheduling.rs:6` `resolve_bootstrap_eligibility()` — stability window, bootstrap-min-height, liveness split, `bootstrap_schedule_with_liveness()`
`production/scheduling.rs:422` `resolve_epoch_eligibility()` — pure `active_list[slot % len]`, symmetric with validation's `validate_producer_eligibility`
`production/assembly.rs:9` `build_block_content()` — coinbase+epoch-reward+genesis-VDF+mempool+bitfield assembly; `large_block_activation_height` gates block-data budget (LEGACY vs LARGE_BLOCK budgets)
`production/assembly.rs:507` `drain_pending_events()` — non-blocking network-channel drain before VDF
`production/assembly.rs:556` `compute_block_vdf()` — hash-chain VDF, calibrated iterations, devnet placeholder
`production/assembly.rs:616` `aggregate_bls_signatures()`
`production/assembly.rs:647` `attest_own_block()`

### Rewards (`rewards.rs`)
`rewards.rs:71` `calculate_epoch_rewards(epoch)` → `Result<Vec<(u64,Hash)>, IncompleteEpochStoreError>` — fail-fast on incomplete block_store (INC-I-081, was silent `Vec::new()` pre-fix); tier1(90%)/tier2(80%-of-median)/tier3(accumulate) qualification; delegation split via `DELEGATE_REWARD_PCT`
`rewards.rs:448` `handle_equivocation()` — builds+broadcasts slash TX
`rewards.rs:529` `rebuild_producer_liveness()` — post-rollback/reorg liveness rebuild from block_store
`rewards.rs:565` `rebuild_epoch_state_from_blocks(target_height)` — INC-I-082 Defect 2: explicit height param, no ordering dependency on chain_state/state_db; incomplete-history safe-default (Light validation fallback)
`rewards.rs:1105` `rebuild_producer_set_from_blocks()`

### Rollback (`rollback.rs`)
`rollback.rs:10` `rollback_one_block()` — undo-based O(1); safety caps (no genesis height>1, max 50 cumulative); INC-I-071 single `get_undo()` reuse for UTXO+epoch restore

### Fork recovery (`fork_recovery.rs`)
`fork_recovery.rs:8` `handle_completed_fork_recovery()` — simple check_reorg_weighted() then plan_reorg() fallback; deterministic hash-tiebreak on weight_delta==0
`fork_recovery.rs:156` `try_trigger_fork_recovery()`
`fork_recovery.rs:184` `try_apply_cached_chain()` — max 50-block backward chain build
`fork_recovery.rs:268` `apply_snap_snapshot()` — full snapshot application incl. EpochState fast-path/legacy
`fork_recovery.rs:697` `try_apply_direct_successor()` — INV-SYNC-003 (INC-I-081 Bug 3); applies candidate iff `prev_hash == local_tip`, Light mode

### Validation glue (`validation_checks.rs`)
`validation_checks.rs:12` `check_producer_eligibility()` — lightweight gossip pre-check (bootstrap+epoch ValidationContext)
`validation_checks.rs:165` `validate_block_for_apply()` — full ValidationContext; Light mode uses empty bootstrap list (post-genesis deterministic scheduler only)
`validation_checks.rs:446` `validate_block_economics()` — coinbase/EpochReward/PriceAttestation-dedup/AddBond-cap checks not expressible in `doli_core::validation` (needs UTXO/producer/block_store access); INC-I-080 per-producer AddBond cap runs in ALL modes
`validation_checks.rs:962` `handle_sync_request()` — GetHeaders/GetBodies/GetBlockBy*/GetHeadersByHeight/GetStateRoot/GetStateSnapshot/DirectAttestation responders

### Genesis (`genesis.rs`)
`genesis.rs:13` `derive_genesis_producers_from_chain()` — OnceLock-cached; falls back to hardcoded chainspec producers for snap-synced nodes
`genesis.rs:92` `genesis_bls_pubkeys()`
`genesis.rs:121` `consume_genesis_bond_utxos()` — shared by rollback/reorg UTXO-rebuild paths

### Periodic tasks (`periodic.rs`, 1370+ lines)
`periodic.rs:95` `run_periodic_tasks()` — defi-health cache (30s TTL) → integrity scan (every 100 blocks, auto-repair from archive) → attest-fetch sweep → cache-TTL sweep → apply pending sync blocks → snap-snapshot consumption → sync cleanup+stuck-fork consume+report → archive catch-up → **RECOVERY COORDINATOR** (report evidence → `classify_and_dispatch()` → ShallowRollback/HeaderFirstSync/SnapSync/GenesisResync) → sync-request dispatch → periodic status refresh → port-reachability warning → AUTO-CHECKPOINT (health.json + rotation) → 30s HEALTH log → seed release → integrity-check-Phase2
`periodic.rs:1137` `maybe_run_integrity_check()` — every 1000 blocks, blocking-task scan + auto-repair
`periodic.rs:1248` `parse_checkpoint_height()` (pure, tested)
`periodic.rs:1261` `integrity_scan()` (pure block-store scan)

### Network events (`network_events.rs`)
`network_events.rs:8` `on_peer_connected()` / `:52` `on_peer_disconnected()` — churn-rate-limited (Fix 5a)
`network_events.rs:87` `on_new_block_event()` — snap-sync/upstream-dup/stale/rejected-fork-tip filters before `handle_new_block()`
`network_events.rs:211` `on_peer_status()`, `:243` `on_status_request()`
`network_events.rs:536` `on_new_attestation()`, `:581` `maybe_fetch_attested_block()` — INC-I-049 deferred fetch record
`network_events.rs:709` `record_peer_churn_and_check()` — 5 events/30s window rate-limit

### Checkpoint health (`checkpoint_health.rs`, pure)
`checkpoint_health.rs:36` `decide_checkpoint_health()` — self-consistency gate → isolated-healthy (F2) → point-healthy → window-healthy fallback
`checkpoint_health.rs:91` `select_checkpoint_evictions()` — keep top-N by height + immunity for highest-height healthy checkpoint (F3/F5)

### Initialization (`init.rs`)
`init.rs:12` `cleanup_orphan_utxo_store()` — Phase 4 one-shot disk cleanup (state_db is sole UTXO store)
`init.rs:56` `recover_body_gaps()` — detects/undoes header-only blocks from interrupted header-first sync
`init.rs:156` `Node::new()` — see ENTRY-POINTS
`init.rs:1056` `new_for_test()`, `init.rs:1253` `new_for_replay()`

### Producer safety (`producer/mod.rs`)
`producer/mod.rs:31` `startup_checks()` — lock file + signed-slots DB + optional duplicate-key detection
`producer/guard.rs` `ProducerGuard` — RAII lock file
`producer/signed_slots.rs` `SignedSlotsDb` — double-sign prevention

### Operations (`operations/`)
`operations/chain.rs` — `truncate_chain()`, `recover_chain_state()`, `reindex_canonical_chain()`
`operations/init.rs` — `init_data_dir()`, `show_status()`, `import_blocks()`, `export_blocks()`
`operations/restore.rs` — `restore_from_rpc()`, `restore_from_archive()`, `backfill_from_archive()`, `bridge_checkpoint_to_archive()`

### Dead/reserved code
`tx_announcements.rs` — `PendingTxAnnouncements` struct, fully `#[allow(dead_code)]`; NOT wired up. `network_events.rs` uses a plain `HashMap<PeerId,Vec<Hash>>` (`pending_tx_announcements` field) instead. Do not assume this module is live.

## DEPENDENCIES

**Depends on (crates):**
- `doli_core` — Block, BlockHeader, Transaction, ConsensusParams, NetworkParams, validation, scheduler, EpochState/EpochDerivationInput, attestation codec, `oracle::*` (bond_weighted_median, compute_structural_share_bps, dedupe_latest_per_attester)
- `storage` — BlockStore, StateDb, UtxoSet, ProducerSet, ChainState, BlockBatch, UndoData, MaintainerState, archiver, `compute_scheduler_root`, `compute_state_root`
- `network` — NetworkService, SyncManager, PeerId, NetworkEvent/NetworkCommand, SyncRequest/Response, EquivocationDetector, `RecoveryAction`, `CompletedRecovery`, `ReorgResult`
- `rpc` — RpcServer, RpcContext, Mempool, WsEvent
- `crypto` — Hash, KeyPair, PublicKey, BlsKeyPair
- `updater` — HardForkSchedule, UpdateConfig, `is_production_allowed()`, MaintainerSet bootstrap keys
- `vdf` — VdfOutput, VdfProof (genesis registration only; production VDF is hash-chain in `doli_core::tpop`)

**What depends on this module:**
- Integration tests in `bins/node/tests/` (via `lib.rs` re-exports + `Node::new_for_test`)
- `bins/gui/` (GUI producer registration)
- No other crates depend on `bins/node` — top-level binary

## CONSTRAINTS

**Atomicity invariant** — every `apply_block()` commits UTXO+ProducerSet+ChainState+EpochState+undo in one `StateDb.BlockBatch`. Block store write deferred until AFTER all TX validation (`apply_block/mod.rs:275`) — see N4 2026-03-13 poisoning incident.

**3-state identity** — `ChainState`, `UtxoSet`, `ProducerSet` MUST be identical across nodes. Since Phase 3/4, `utxo_set` is a thin wrapper backed directly by `state_db` (no separate cache to diverge).

**Epoch state ordering** — Producer mutations (Register/Exit/Slash/AddBond/RequestWithdrawal/DelegateBond/RevokeDelegation) DEFERRED to epoch boundary, applied every block during epoch 0. Maintainer changes immediate.

**INC-I-071 undo sentinel** — `undo.producer_snapshot == Vec::new()` means "ProducerSet unchanged at this height — rollback/reorg MUST skip restore". Decided per-block by `block_mutates_producer_set()`. `epoch_state_snapshot` is NEVER covered by this optimization — always present.

**INC-I-082 explicit height param** — `rebuild_epoch_state_from_blocks(target_height)` and all rollback/reorg callers pass height explicitly; never read `chain_state`/`state_db` for the operating height inside a rebuild function (Defect 2 root cause).

**Bitfield encoder/decoder parity** — encode order `[epoch_state.producer_list (base) | extra sorted by pubkey]`. Encoder: `production/assembly.rs` (~line 358). Decoders: `post_commit.rs` (`use_full_decode` gated by `full_bitfield_decode_height`), `rewards.rs` (gated by `rewards_epoch_list_fix_height`). Misalignment = attestation index corruption (Stability Pillar #2).

**Rollback safety caps** — never rollback to genesis (height>1). Max 50 cumulative rollbacks (`MAX_CUMULATIVE_ROLLBACK`) before requiring manual intervention.

**Recovery mode gate** — `recovery_mode==true` → `apply_block()`/`apply_snap_snapshot()` silently drop all inbound mutations (except `ValidationMode::Replay`). Cleared only via `exitRecoveryMode` RPC; not persisted (cleared on restart).

**Snap sync height guard** — blocks at `height <= snap_sync_height` silently skipped in `apply_block`; used Light validation (no VDF, empty bootstrap list). Cleared at next epoch boundary once `epoch_producer_list` is attestation-filtered again.

**IncompleteEpochStoreError fail-fast (INC-I-081)** — `calculate_epoch_rewards()` returns `Err` (not `Ok(Vec::new())`) when local block_store has gaps or missing body-bitfields in the epoch window. Callers: production aborts the slot (`INC_I_081_SLOT_ABORT`); validation degrades Full→Light for that check (`INC_I_081_VALIDATION_SKIP` / `_MISSING_CHECK_SKIP`). Epoch 0 exempt (no attestation data by construction).

**Oracle gating (Phase 2.1, frozen)** — `oracle_activation_height = u64::MAX` on every network today. `aggregate_oracle_prices_at_epoch_boundary()` is a strict `<` gate no-op pre-activation. MUST run BEFORE `self.epoch_state = new_state` rotation in `post_commit_actions` — depends on CLOSING epoch's `bond_snapshot`. Aborts (does not partially aggregate) on any missing closing-epoch block (AUDIT-P1-002).

**INC-I-138 recovery classification** — `consecutive_empty_headers` counter semantics: only genuine progress (canonical `block_applied`, or post-snap height-based dispatch) may reset it. `HeaderFirstSync` action must NOT reset it — was the D2 root cause of a 325s stall (counter capped at ≤2, G3 rollback threshold of 3 permanently unreachable).

**Direct-successor fallback (INV-SYNC-003, INC-I-081 Bug 3)** — `try_apply_direct_successor()` applies a non-heavier fork tip directly when `candidate.prev_hash == local_tip`, avoiding permanent drop of valid-but-not-heavier blocks.

**Fork_id stability** — `current_fork_id()` always uses `u64::MAX` height to include ALL scheduled forks — "what forks does this binary know", not "what's active now".

**Chainspec security** — Mainnet/Testnet always use embedded (compiled-in) chainspec; disk files/`--chainspec` silently ignored. Devnet honors disk/CLI.

**Production gates (layered, in order)** — (1) `is_production_allowed()` pending-update grace, (2) `hardfork_schedule.should_stop_producing()`, (3) `SyncManager.can_produce()` (5 sub-checks), (4) `should_defer_epoch_production()` post-restart grace, (5) `resolve_bootstrap_eligibility()` stability+peer checks.

**Supply conservation (INC-I-064)** — UTXO spend/create failures now propagate as errors (not silently swallowed); Replay mode tolerates historical mismatches.

**Attest-fetch dedup (INC-I-049)** — deferred 500ms, max 3 peers/hash, 30s TTL — eliminates ~94% false-positive fetches where gossip beats the attestation by 1-2ms.

**Peer churn rate-limit** — `record_peer_churn_and_check()` drops expensive work (status request, sync_manager mutation) for peers exceeding 5 events/30s.

**Orphan recovery (Stability Pillar #1)** — orphan received → `GetBlockByHeight(need_height)` direct to sender. Do not modify without re-reading the pillar doc.

**Signed slots safety** — `SignedSlotsDb::check_and_mark()` prevents double-signing the same slot after restart; `ProducerGuard` lock file prevents two instances on one machine.

**Dead-code trap** — `tx_announcements.rs::PendingTxAnnouncements` is unused (`#[allow(dead_code)]` throughout); the live path is `network_events.rs`'s plain `HashMap`. Do not "fix" or extend the dead module expecting it to run.

## PATTERNS

**Biased select for production fairness** — `event_loop.rs` `tokio::select! { biased; ... }`: network events always processed before production timer, but a `last_production_check` escape-hatch forces production+periodic tasks under continuous event flood.

**ValidationMode routing** — `Full` (gossip/production; MAX_PAST_SLOTS+VDF checked), `Light` (sync/reorg/self-produced; time-checks + bootstrap-producer-list skipped), `Replay` (disaster recovery; bypasses recovery_mode + snap_height guards, tolerates historical UTXO mismatches).

**Undo log pattern** — snapshot spent UTXOs into `undo_spent_utxos` before mutation, record created outpoints after. ProducerSet uses full-serialize snapshot UNLESS `block_mutates_producer_set()==false` (INC-I-071 empty-sentinel). Rollback/reorg reverse: remove created, restore spent, restore-or-skip producer snapshot.

**Pure classification before action** — `classify_gossip_block()` is read-only/no-side-effects; decision made before any mutation. Same idiom in `checkpoint_health::decide_checkpoint_health()`/`select_checkpoint_evictions()` — both pure, both unit-tested without a `Node`/tokio runtime.

**OnceLock caching with invalidation** — `cached_genesis_producers: OnceLock<Vec<PublicKey>>` cleared (fresh OnceLock) on rollback/reorg crossing the genesis boundary.

**Atomic batch commit** — all state writes go through `StateDb.BlockBatch::begin_batch()`/`commit()`. Never write individual fields between begin and commit; `post_commit_actions` extends the same `undo`/`batch` the caller began.

**Epoch-deferred mutations** — ProducerSet mutations set pending flags via `queue_update()`; `EpochState::derive_at_boundary()` applies them atomically so all nodes compute the same producer list at the same height. Rebuild paths (`rebuild_producer_set_from_blocks`, `rebuild_epoch_state_from_blocks`) mirror this exactly, including height-gated auth/cap checks (INC-I-078).

**Explicit-height rebuild functions (INC-I-082 pattern)** — any function that reconstructs consensus-derived state from `block_store` takes `target_height`/`height` as an explicit parameter rather than reading `self.chain_state`. Removes an entire class of caller-ordering bugs.

**Gossip anti-entropy via GSet CRDT** — `ProducerGSet` grow-only set of signed `ProducerAnnouncement`s; merge idempotent/commutative; sequence resumes from persisted value on restart (avoids "Duplicate" rejection).

**Liveness split for scheduling** — bootstrap round-robin splits producers into `live`/`stale` by `LIVENESS_WINDOW_MIN` (or `3*num_bp`); all-stale deadlock-safety falls back to all-live. Mirrored identically in `check_producer_eligibility`/`validate_block_for_apply` so production and validation compute the same rank.

**Toxic TX purging** — on "already exists"/"already registered" production error, mempool purges matching TXs to prevent infinite-retry chain freeze (both `try_produce_block`'s BLOCK_POISON path and the event-loop's error-string match).

**Chain integrity scan** — full BLAKE3 commitment over `1..=round_to_100(tip)` every 100 blocks (never incremental — incremental broke on every alternate code path). Phase 2 auto-repairs from `--archive-to` on gap detection.

**Fail-fast over silent-empty (INC-I-081 idiom)** — distinguish "legitimately empty" (`Ok(Vec::new())`, Tier-3 accumulate) from "cannot compute" (`Err(IncompleteEpochStoreError)`). Applied to `calculate_epoch_rewards`; the oracle aggregator's AUDIT-P1-002 abort-on-missing-block is the same idiom.

**Devnet timing shortcuts** — production interval 200ms vs 1s; producer-list stability 3s vs 15s; bootstrap grace 5s vs 15s/90s; bootstrap timeout 60s vs 180s. Always network-gated via `self.config.network == Network::Devnet`.

**Test entry points** — `lib.rs` re-exports `pub mod node` plus `init::cleanup_orphan_utxo_store`/`init::recover_body_gaps`/`rewards::IncompleteEpochStoreError`. `Node::new_for_test()` (in-process RocksDB, no networking) and `Node::new_for_replay()` (existing block_store + fresh state_db, headless) are the two non-production construction paths — never wire networking into either.
