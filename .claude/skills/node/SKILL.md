# node — DOLI Node Binary
<!-- @INDEX
ENTRY-POINTS: lines 14-47
DATA-FLOWS: lines 49-99
STRUCTS: lines 101-163
FUNCTIONS: lines 165-307
DEPENDENCIES: lines 309-333
CONSTRAINTS: lines 335-395
PATTERNS: lines 397-445
-->

## ENTRY-POINTS

`bins/node/src/main.rs:36` — `async fn main()` — CLI dispatch: parses args, sets up logging, loads data_dir, calls run_node() or run_bootnode()

`bins/node/src/run.rs:18` — `async fn run_node()` — Full node startup: loads keys, config, chainspec, spawns update service, constructs Node, runs archiver, blocks on signal

`bins/node/src/run.rs:624` — `async fn run_bootnode()` — Lightweight UDP-only bootnode: no storage, no libp2p, no sync; just Discv5 peer discovery

`bins/node/src/node/startup.rs:5` — `Node::run()` — Top-level node lifecycle: start_network → self-register producer → compute genesis VDF → start_rpc → run_event_loop → shutdown

`bins/node/src/node/startup.rs:141` — `Node::start_network()` — Builds NetworkConfig (mesh params, fork_id, peer cache path), starts NetworkService

`bins/node/src/node/event_loop.rs:5` — `Node::run_event_loop()` — Biased select loop: network events first, then production timer (1s/200ms), then gossip anti-entropy timer

`bins/node/src/node/init.rs:195` — `Node::new()` — Opens BlockStore + StateDb, migrates legacy files, initializes UTXO set, loads EpochState, builds full Node struct

`bins/node/src/lib.rs:1` — lib re-exports: `pub mod config`, `pub mod node`, `pub mod producer`, `pub mod updater` for integration test access

Commands dispatched from `main.rs:150`:
- `Run` → `run::run_node()` or `run::run_bootnode()`
- `Init` → `operations::init_data_dir()`
- `Status` → `operations::show_status()`
- `Import/Export` → `operations::import_blocks()` / `export_blocks()`
- `Restore` → `operations::restore_from_rpc()` / `restore_from_archive()` / `backfill_from_archive()`
- `Truncate` → `operations::truncate_chain()`
- `Recover` → `operations::recover_chain_state()`
- `Reindex` → `operations::reindex_canonical_chain()`
- `Devnet` → `commands::handle_devnet_command()`
- `Update/Maintainer/Release/Upgrade` → `commands::handle_*_command()`
- `CheckpointInfo` → prints compiled-in `CHECKPOINT_HEIGHT/HASH/STATE_ROOT`

## DATA-FLOWS

**Block ingestion (gossip):**
```
NetworkEvent::NewBlock → handle_network_event() → handle_new_block()
  → classify_gossip_block() → [ExtendsTip | ForkBlock | Orphan | Rejected]
  → ExtendsTip: check_producer_eligibility() → apply_block()
  → ForkBlock(HeightOccupied, is_better): signal stuck_fork → sync recovery
  → ForkBlock(ReorgCandidate): cache_block_with_eviction() → try_apply_cached_chain()
  → Orphan: request parent via GetHeaders(need_height)
```

**Block application (apply_block):**
```
Block → validate_block_for_apply() → validate_block_economics()
  → process_transaction_utxos()      [UtxoSet write + batch]
  → process_transaction_producer_effects()  [ProducerSet write, deferred]
  → process_transaction_governance()  [MaintainerState, ProtocolActivation deferred]
  → maybe_complete_genesis()          [first post-genesis block only]
  → batch.commit()                    [atomic: UTXO + ProducerSet + ChainState + EpochState]
  → update_chain_state_for_block()
  → post_commit_actions(): recompute_active_status, accumulate_block, epoch snapshot
  → update_known_producers()
  → broadcast new block to network
```

**Block production:**
```
production_timer tick → try_produce_block()
  → version enforcement check (pending_update gate)
  → hardfork schedule check
  → handle_production_authorization() (SyncManager.can_produce())
  → resolve_bootstrap_eligibility() OR epoch mode check
  → build_block_content(): coinbase, epoch rewards (at epoch start), mempool txs, attestation bitfield
  → sign block → apply_block(ValidationMode::Full) → broadcast
```

**Epoch boundary (at height N where N % blocks_per_epoch == 0):**
```
build_block_content() → calculate_epoch_rewards(epoch) → EpochReward TX appended
apply_block() → post_commit_actions() → EpochState.derive_at_boundary()
  → new producer_list, bond_snapshot, reset accumulators
  → persist EpochState to batch (atomic with block)
```

**Fork recovery:**
```
sync_manager.signal_stuck_fork() → periodic run_periodic_tasks()
  → sync_manager RecoveryCoordinator → [ShallowRollback | Resync | Wait]
  → ShallowRollback: rollback_one_block() (undo-based, O(1)) → sync fills gap
  → CompletedForkRecovery event: handle_completed_fork_recovery()
    → record weights → move blocks to fork_block_cache → execute_reorg()
```

**Snap sync:**
```
SyncManager completes snap sync → NetworkEvent::SnapSyncComplete(VerifiedSnapshot)
  → apply_snap_snapshot(): verify state_root, deserialize 3 states, replace local
  → persist to StateDb → set snap_sync_height → post-snap header sync fills gaps
```

**UTXO init on startup (INC-I-027):**
```
Node::new() → init_utxo_set(data_dir, state_db):
  utxo_store.len() == state_db.utxo_len() → use as-is (steady state)
  utxo_store empty, state_db non-empty → migrate (first boot post-upgrade)
  utxo_store != state_db len → clear + rebuild from state_db (guardian-restore self-heal)
  utxo_store cannot open → in-memory fallback from state_db
```

## STRUCTS

`bins/node/src/node/mod.rs:78` — **`Node`** — Main node struct with ~50 fields:
- `config: NodeConfig` — network, data_dir, listen_addr, bootstrap_nodes, RPC config
- `params: ConsensusParams` — slot_duration, bond amounts, reward params
- `block_store: Arc<BlockStore>` — canonical block storage (RocksDB)
- `state_db: Arc<StateDb>` — unified atomic state (UTXO + ProducerSet + ChainState + EpochState)
- `utxo_set: Arc<RwLock<UtxoSet>>` — in-memory working copy (RocksDB-backed or in-memory fallback)
- `chain_state: Arc<RwLock<ChainState>>` — height, best_hash, slot, genesis_hash
- `producer_set: Arc<RwLock<ProducerSet>>` — registered producers, bonds, delegations
- `mempool: Arc<RwLock<Mempool>>` — pending transactions
- `network: Option<NetworkService>` — libp2p + gossipsub
- `sync_manager: Arc<RwLock<SyncManager>>` — sync state machine, fork recovery coordinator
- `epoch_state: EpochState` — producer_list, bond_snapshot, attestation accumulators (single source of truth for scheduling)
- `producer_key: Option<KeyPair>` — present only for producing nodes
- `bls_key: Option<BlsKeyPair>` — required for all producers (BLS aggregate attestation)
- `fork_block_cache: Arc<RwLock<HashMap<Hash, Block>>>` — temporary cache for reorg blocks (max 100, slot-sorted eviction)
- `producer_gset: Arc<RwLock<ProducerGSet>>` — CRDT for producer discovery via gossip
- `epoch_state: EpochState` — authoritative scheduler state (producer_list, bond_snapshot, accumulators)
- `recovery_mode: Arc<AtomicBool>` — anti-poisoning gate; blocks all inbound block application
- `snap_sync_height: Option<u64>` — blocks at/below this height use Light validation
- `rejected_fork_tips: HashSet<Hash>` — prevents re-requesting already-rejected fork tips
- `attest_fetch_tracker: HashMap<Hash, (Instant, u8, PeerId)>` — deferred attestation block fetch (INC-I-049)

`bins/node/src/node/block_handling.rs:11` — **`BlockClass`** — Result of classify_gossip_block():
- `ExtendsTip` — direct tip extension, apply now
- `ForkBlock(ForkBlockKind)` — parent known, height may be occupied
- `Orphan { need_height }` — parent unknown, request it
- `Rejected(String)` — wrong genesis, fork_id mismatch, etc.

`bins/node/src/node/block_handling.rs:25` — **`ForkBlockKind`** — Sub-classification:
- `HeightOccupied { fork_height, canonical_slot, is_better }` — canonical chain at that height
- `ReorgCandidate` — parent known, no height conflict

`bins/node/src/config.rs:11` — **`NodeConfig`** — Per-node runtime config: network, data_dir, listen_addr, bootstrap_nodes, max_peers, rpc: RpcConfig, no_dht, relay_server, genesis_time_override, chainspec, slot_duration_override, no_snap_sync, auto_checkpoint_interval, bootnode_enrs, no_discv5, discv5_port, external_address

`bins/node/src/producer/mod.rs:31` — **`startup_checks()`** — Returns `(ProducerGuard, SignedSlotsDb)` after: lock file acquisition, signed slots DB open, optional duplicate key detection

## FUNCTIONS

### Node lifecycle (`startup.rs`)
`startup.rs:5` — `Node::run()` — Orchestrates: check placeholder keys → start_network → self-register → genesis VDF → start_rpc → run_event_loop → shutdown

`startup.rs:141` — `Node::start_network()` — Parses listen_addr, builds NetworkConfig with mesh params, extracts seed PeerIds for post-bootstrap release

### Event loop (`event_loop.rs`)
`event_loop.rs:5` — `Node::run_event_loop()` — Biased select: network_event > production_timer > gossip_timer. Production escape hatch ensures production runs even under event flood

### Block handling (`block_handling.rs`)
`block_handling.rs:42` — `classify_gossip_block()` — **Pure function** (no side effects): classifies block into BlockClass from read-only state snapshot

`block_handling.rs:116` — `Node::handle_new_block()` — Checks duplicates, equivocation, classifies, routes to apply/cache/orphan-request

`block_handling.rs:101` — `Node::cache_block_with_eviction()` — Inserts block into fork cache; evicts oldest 50 by slot when cache exceeds 100

### Block application (`apply_block/`)
`apply_block/mod.rs:14` — `Node::apply_block(block, mode)` — Full pipeline: recovery_mode guard → snap_height guard → duplicate check → validate → process TXs → batch.commit() → post_commit_actions → broadcast

`apply_block/tx_processing.rs:8` — `Node::process_transaction_utxos()` — UTXO spend/create, undo log capture, EpochReward pool drain (pre-activation), covenant evaluation

`apply_block/tx_processing.rs` — `Node::process_transaction_producer_effects()` — Deferred producer mutations: Register, AddBond, Exit, Slash, Withdrawal, Delegation (applied at epoch boundary, never mid-epoch except epoch 0)

`apply_block/governance.rs:10` — `Node::process_transaction_governance()` — Immediate: MaintainerAdd/Remove. Deferred: ProtocolActivation (returns version+epoch for chain_state apply)

`apply_block/genesis_completion.rs:11` — `Node::maybe_complete_genesis()` — Runs exactly once at height `genesis_blocks + 1`: derives VDF-proven producers, creates real bond-backed registrations

`apply_block/state_update.rs:40` — `Node::update_chain_state_for_block()` — Updates best_height/hash/slot, clears snap sync marker, applies deferred protocol activation at epoch boundary

`apply_block/state_update.rs:5` — `Node::update_known_producers()` — Adds newly registered producers to known_producers (sorted deterministically)

`apply_block/post_commit.rs:7` — `Node::post_commit_actions()` — Runs AFTER batch.commit(): recompute_active_status, decode attestation bitfield (full or base depending on height), accumulate_block, persist EpochState in batch, archive buffering, WebSocket broadcast

### Block production (`production/`)
`production/mod.rs:31` — `Node::try_produce_block()` — Version gate → hardfork gate → handle_production_authorization → scheduling → build_block_content → sign → apply_block → broadcast

`production/mod.rs:18` — `Node::should_defer_epoch_production()` — Grace period after first peer (5s devnet / 15s others) to avoid producing on stale tip after restart

`production/gates.rs:21` — `Node::handle_production_authorization()` — Delegates to SyncManager.can_produce(); resets shallow_rollback_count on Authorized; logs BlockedSyncing/BlockedInsufficientPeers/BlockedExplicit

`production/scheduling.rs:3` — `Node::resolve_bootstrap_eligibility()` — Bootstrap round-robin: stability check (3s devnet / 15s), peer status wait, joining node gap guard, liveness split (live vs stale producers)

`production/assembly.rs:9` — `Node::build_block_content()` — Assembles header+txs+bitfield: coinbase, epoch rewards (at epoch start via calculate_epoch_rewards), genesis VDF registration, mempool TXs, attestation bitfield encoding

### Rewards (`rewards.rs`)
`rewards.rs:34` — `Node::calculate_epoch_rewards(epoch)` — Scans epoch blocks, decodes presence_root bitfield (using epoch_state.producer_list for post-fix, all active pre-fix), counts attested minutes per producer. Returns empty Vec if block_store incomplete (fail-fast to prevent silent divergence). ≥54/60 minutes qualifies. Pool distributed bond-weighted among qualifiers.

### Rollback (`rollback.rs`)
`rollback.rs:10` — `Node::rollback_one_block()` — Undo-based O(1) rollback: restore UTXO set and ProducerSet from undo log. Falls back to rebuild-from-genesis if no undo data. Safety caps: refuses rollback to genesis (height > 1), max 50 cumulative rollbacks

### Fork recovery (`fork_recovery.rs`)
`fork_recovery.rs:8` — `Node::handle_completed_fork_recovery()` — Evaluates recovered fork: record weights → move to fork_block_cache → simple reorg check (weight_delta or hash tiebreak) → execute_reorg or drop

`fork_recovery.rs:133` — `Node::try_trigger_fork_recovery()` — Initiates fork recovery from cached orphan blocks via SyncManager

`fork_recovery.rs:161` — `Node::try_apply_cached_chain()` — Walks fork_block_cache backward from latest to our tip, applies in order; max 50 depth

`fork_recovery.rs:244` — `Node::apply_snap_snapshot()` — Applies verified snap sync snapshot: re-verify state_root → deserialize 3 states → replace local state atomically → persist to StateDb

### Validation (`validation_checks.rs`)
`validation_checks.rs:12` — `Node::check_producer_eligibility()` — Lightweight gossip pre-check: producer in active set OR GSet; bond weights from epoch snapshot; builds ValidationContext with liveness split

`validation_checks.rs:156` — `Node::validate_block_for_apply()` — Full ValidationContext build; calls validation::validate_block_with_mode (Full: VDF verified; Light: VDF skipped for snap-synced blocks)

### Genesis (`genesis.rs`)
`genesis.rs:13` — `Node::derive_genesis_producers_from_chain()` — OnceLock-cached scan of genesis blocks for Registration TXs with valid VDF proofs. Falls back to hardcoded chainspec producers for snap-synced nodes

### Periodic tasks (`periodic.rs`)
`periodic.rs:89` — `Node::run_periodic_tasks()` — Chain integrity scan (every 100 blocks rounded, BLAKE3 commitment), auto-repair from archive, peer redial, seed release, health window update, attest_fetch_tracker processing, auto-checkpoint

`periodic.rs:27` — `Node::maybe_bootstrap_maintainer_set()` — One-shot: bootstrap MaintainerSet from first 5 registered producers (by registration height, deterministic)

`periodic.rs:5` — `Node::flush_finalized_to_archive()` — Sends finalized blocks to archive channel (blocks above finalized height held back)

### Network events (`network_events.rs`)
`network_events.rs:8` — `Node::on_peer_connected()` — Churn rate-limit, set first_peer_connected, enable SyncManager bootstrap gate, send StatusRequest

`network_events.rs:52` — `Node::on_peer_disconnected()` — Rate-limited reconnect to bootstrap (1 attempt per slot_duration)

### Initialization (`init.rs`)
`init.rs:40` — `init_utxo_set()` — **Public** (for integration tests). Resolves UTXO init from disk vs state_db with INC-I-027 self-heal

`init.rs:101` — `recover_body_gaps()` — **Public** (for integration tests). Detects and undoes header-only blocks from interrupted header-first sync

`init.rs:195` — `Node::new()` — Opens stores, migrates legacy formats, applies chainspec, calls init_utxo_set + recover_body_gaps, loads EpochState from StateDb (or rebuilds), initializes all node fields

### Node getters / misc (`mod.rs`)
`mod.rs:292` — `Node::current_fork_id()` — `hardfork_schedule.fork_id(genesis_hash, u64::MAX)` — represents all known forks regardless of current height

`mod.rs:304` — `Node::best_height()` — async read of chain_state.best_height

`mod.rs:316` — `Node::bond_weights_for_scheduling()` — Returns bond weights: epoch_state.bond_snapshot (post-epoch-0) or UTXO count (epoch-0 fallback)

`mod.rs:362` — `Node::shutdown()` — Sets shutdown flag; event loop exits on next iteration

### Producer safety (`producer/mod.rs`)
`producer/mod.rs:31` — `startup_checks()` — Lock file + signed slots DB + optional duplicate key detection

`producer/guard.rs` — `ProducerGuard` — RAII lock file; dropped on process exit

`producer/signed_slots.rs` — `SignedSlotsDb` — Prevents double-signing after restart

### Operations (`operations/`)
`operations/chain.rs` — `truncate_chain()`, `recover_chain_state()`, `reindex_canonical_chain()` — offline chain repair tools

`operations/init.rs` — `init_data_dir()`, `show_status()`, `import_blocks()`, `export_blocks()`

`operations/restore.rs` — `restore_from_rpc()`, `restore_from_archive()`, `backfill_from_archive()`, `bridge_checkpoint_to_archive()`

## DEPENDENCIES

**Depends on (crates):**
- `doli_core` — Block, BlockHeader, Transaction, ConsensusParams, NetworkParams, validation, scheduler, EpochState, attestation codec
- `storage` — BlockStore, StateDb, UtxoSet, ProducerSet, ChainState, undo log, MaintainerState, archiver
- `network` — NetworkService, SyncManager, PeerId, NetworkEvent, NetworkCommand, SyncRequest/Response, EquivocationDetector
- `rpc` — RpcServer, RpcContext, Mempool, WsEvent
- `crypto` — Hash, KeyPair, PublicKey, BlsKeyPair
- `updater` — HardForkSchedule, UpdateConfig, is_production_allowed()
- `vdf` — VdfOutput, VdfProof (for genesis registration)

**What depends on this module:**
- Integration tests in `bins/node/tests/` (via lib re-exports)
- `bins/gui/` (GUI producer registration)
- No other crates depend on `bins/node` — it is the top-level binary

## CONSTRAINTS

**Atomicity invariant**: Every `apply_block()` call MUST commit all state changes (UTXO + ProducerSet + ChainState + EpochState + undo log) in a single `StateDb.BlockBatch` commit. Partial writes cause divergence. Block store write is deferred until AFTER all TX validation completes (INC-I-013 pattern: N4 incident).

**3-state identity**: `ChainState`, `UtxoSet`, `ProducerSet` MUST be identical across all nodes. Any divergence causes snap sync or validation failures. state_db is authoritative; utxo_store is a cache that must stay in sync.

**Epoch state ordering**: Producer mutations (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are DEFERRED to epoch boundary — never mid-epoch except epoch 0. MaintainerAdd/Remove are immediate.

**EpochState persistence**: epoch_state is persisted to batch on EVERY block (not just epoch boundaries). Loss on restart causes attestation accumulator divergence.

**Bitfield encoder/decoder parity**: Encoder order is `[epoch_state.producer_list | extra sorted by pubkey]`. ALL decoders (post_commit, rewards.rs, RPC) MUST use the same order. Misalignment causes attestation index corruption. See Stability Pillar #2.

**Rollback safety caps**: Never roll back to genesis (height > 1). Max 50 cumulative rollbacks before requiring manual intervention. Undo-based O(1) preferred; genesis-rebuild fallback only when no undo data.

**Recovery mode gate**: When `recovery_mode == true`, `apply_block()` silently drops all inbound blocks (except ValidationMode::Replay). Only `exitRecoveryMode` RPC clears it. Not persisted — cleared on restart.

**Snap sync height guard**: After snap sync, blocks at `height <= snap_sync_height` are silently skipped. These heights used Light validation mode (no VDF) since state was verified by state_root quorum.

**Fork_id stability**: `current_fork_id()` always uses `u64::MAX` to include ALL scheduled forks — represents "what forks does this binary know about", NOT which forks are currently active at chain height.

**Chainspec security**: Mainnet and Testnet always use embedded chainspec (compiled into binary). Disk files and `--chainspec` flag are silently ignored for these networks. For devnet, disk or CLI flag applies.

**Production gates (layered)**:
1. `is_production_allowed()` — pending update grace period
2. `hardfork_schedule.should_stop_producing()` — binary too old for active hard fork
3. `SyncManager.can_produce()` — sync state machine (5 sub-checks)
4. `should_defer_epoch_production()` — grace period after first peer
5. `resolve_bootstrap_eligibility()` — producer list stability + peer count

**Supply conservation** (INC-I-064): `apply_block()` snapshot UTXO total value before TX processing; verifies it is unchanged after (inflation vector closed).

**Attest fetch deduplication** (INC-I-049): Attestation-triggered block fetches are DEFERRED 500ms via `attest_fetch_tracker`. Eliminates ~94% false-positive fetches where gossip delivers the block 1-2ms after the attestation arrives.

**Peer churn rate-limit** (INC-I-2026-04-15): `record_peer_churn_and_check()` drops expensive work for peers exceeding 5 events in 30s rolling window. Prevents flapping peers from starving the event loop.

**Orphan recovery**: When orphan received, request parent via `GetHeaders(need_height)`. This is the "Orphan Chase" Stability Pillar #1.

**Signed slots safety**: `SignedSlotsDb` prevents double-signing the same slot after node restart. `ProducerGuard` lock file prevents two instances on the same machine.

## PATTERNS

**Biased select for production fairness**: `event_loop.rs` uses `tokio::select! { biased; ... }` — network events always processed before production timer. But production escape hatch (line 68) forces production if it hasn't run for a full interval under event flood. Never use unbiased select for this loop.

**ValidationMode routing**: `apply_block()` takes `ValidationMode`: `Full` (gossip/production, checks MAX_PAST_SLOTS and VDF), `Light` (sync/reorg, skips time-based checks for old blocks), `Replay` (disaster recovery, bypasses recovery_mode gate and snap_height guard).

**Undo log pattern**: Before any UTXO mutation, snapshot spent UTXOs into `undo_spent_utxos`. After, record created outpoints in `undo_created_utxos`. Both stored in StateDb keyed by height. Rollback reverses: remove created, restore spent. ProducerSet uses full serialized snapshot in undo log.

**Pure classification before action**: `classify_gossip_block()` is a pure function — reads-only, no side effects. Decision made before any mutable operations. Pattern used for correctness testing.

**OnceLock caching with invalidation**: `cached_genesis_producers: OnceLock<Vec<PublicKey>>` is cleared on reorgs crossing the genesis boundary by replacing with a fresh OnceLock.

**Atomic batch commit**: All state writes go through `StateDb.BlockBatch`. The `begin_batch()` / `batch.commit()` pattern ensures all or nothing. Never write individual fields between begin and commit.

**Epoch-deferred mutations**: ProducerSet mutations (Register, AddBond, etc.) set pending flags on ProducerInfo. `EpochState.derive_at_boundary()` applies them at epoch boundary atomically. This ensures all nodes compute the same producer list at the same height.

**Gossip anti-entropy via GSet CRDT**: `ProducerGSet` is a grow-only set of signed `ProducerAnnouncement` messages. Merge is idempotent and commutative. Sequence numbers prevent stale announcements. On restart, sequence resumes from stored value to avoid "Duplicate" rejection by peers.

**Liveness split for scheduling**: Bootstrap round-robin splits known producers into `live` (produced within `max(LIVENESS_WINDOW_MIN, 3 * num_bp)` blocks) and `stale`. Stale producers get lower ranks. If all stale, all treated as live (deadlock prevention).

**Toxic TX purging**: On production error containing "already exists" or "already registered", mempool automatically purges matching TXs to prevent infinite retry loops freezing the chain.

**Chain integrity scan**: Periodic BLAKE3 commitment over heights `1..=round_to_100(tip)`. Full rescan (not incremental) is always correct — incremental broke on every alternate code path (fork replacement, sync, snap sync). Auto-repair from archive if configured.

**Devnet timing**: Production interval 200ms (5 checks/second) vs 1s on testnet/mainnet. Producer list stability window 3s vs 15s. Bootstrap grace period 5s vs 15s.

**Test entry points**: `lib.rs` re-exports `pub mod node` with `pub use init::init_utxo_set` and `pub use init::recover_body_gaps` for integration tests. `Node::new_for_test()` pattern in tests uses `new()` directly with test config.
