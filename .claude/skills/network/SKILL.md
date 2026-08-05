# network — DOLI P2P Network Layer (`crates/network`)

<!-- @INDEX
ENTRY-POINTS    12-53
OPERATIONS      55-64
DATA-FLOW       66-77
DEPENDENCIES    79-94
CONSTRAINTS     96-129
PATTERNS        131-145
@/INDEX -->

## ENTRY POINTS

### Service / lifecycle

| Function/Struct | Location | Signature | Description |
|-------------------|----------|-----------|--------------|
| `NetworkService::new(config)` | `service/mod.rs:81` | `async fn new(config: NetworkConfig) -> Result<Self, NetworkError>` | Builds libp2p swarm+behaviour, dials bootstrap BEFORE cached peers (INC-I-048), spawns `run_swarm` task on tokio |
| `NetworkService::next_event/try_next_event()` | `service/mod.rs:434,443` | `async fn next_event(&mut self) -> Option<NetworkEvent>` / `fn try_next_event(&mut self) -> Option<NetworkEvent>` | Node event loop drains events; `try_next_event` used to flush pending blocks before production |
| `NetworkService::broadcast_block/header/attestation/transaction()` | `service/mod.rs:448-484` | `async fn broadcast_block(&self, block: Block) -> Result<(), NetworkError>` | Sends `NetworkCommand` over mpsc to swarm loop |
| `NetworkService::update_best_slot(slot)` | `service/mod.rs:424` | `fn update_best_slot(&self, slot: u32)` | Node calls after every `apply_block()`; feeds rate-limiter INC-I-049 exemption |
| `NetworkService::gossip_shed_metrics()` | `service/mod.rs:430` | `fn gossip_shed_metrics(&self) -> Arc<GossipShedMetrics>` | INC-I-114 M2: exposes block load-shed drop counter to node/RPC |
| `run_swarm()` | `service/swarm_loop.rs:34` | `async fn run_swarm(swarm, command_rx, event_tx, peers, config, ...)` | `tokio::select!` loop: swarm events, commands, DHT refresh (60s), rate-limit cleanup (5min), bootstrap-TTL cleanup (5s) |
| `handle_swarm_event()` | `service/swarm_events.rs:30` | `async fn handle_swarm_event(event, swarm, ...)` | Connection lifecycle: eviction (LIFO+grace), genesis/version mismatch cooldowns, dial backoff |
| `handle_behaviour_event()` | `service/behaviour_events.rs:37` | `async fn handle_behaviour_event(event, swarm, ...)` | Gossipsub message classify+report, Kademlia routing filter, Identify, status/sync req-resp |
| `handle_command()` | `service/command_handling.rs:22` | `async fn handle_command(command, swarm, config, rate_limiter)` | Dispatches `NetworkCommand` → gossipsub publish / req-resp send / dial; RequestSync passes through INC-I-120 governor |
| `enqueue_or_shed()` | `service/backpressure.rs:74` | `fn enqueue_or_shed(tx, event, metrics)` | INC-I-114 M1: synchronous `try_send`, sheds gossip blocks on full channel (never `.await`s) |
| `MemoryWatchdog::tick()/should_shed()` | `watchdog.rs:103,156` | `fn tick(&mut self)` | INC-I-114 M2: trips shared flag at RSS soft threshold; fail-open on non-Linux/sampler-None |
| `plan_startup_dials()` | `service/helpers.rs:129` | `fn plan_startup_dials(bootstrap_nodes, cached_addrs, pending_limit) -> (Vec<Multiaddr>, Vec<Multiaddr>)` | INC-I-048: bootstrap addrs get priority pending slots over cached addrs |

### Sync manager (`sync/manager/` — 8 submodules: mod, types, recovery, production_gate, cleanup, snap_sync, peers, sync_engine/{decision,dispatch,response})

| Function | Location | Signature | Description |
|----------|----------|-----------|--------------|
| `SyncManager::new(config, genesis_hash)` | `sync/manager/mod.rs:184` | `fn new(config: SyncConfig, genesis_hash: Hash) -> Self` | Standalone state machine, no network handle; node feeds it events |
| `SyncManager::update_local_tip(h,hash,slot)` | `sync/manager/mod.rs:296` | `fn update_local_tip(&mut self, height: u64, hash: Hash, slot: u32)` | Call after every `apply_block()`; maintains 200-entry canonical hash ring buffer, flips to `Synchronized` when caught up |
| `SyncManager::can_produce(slot)` | `sync/manager/production_gate.rs:41` | `fn can_produce(&mut self, current_slot: u32) -> ProductionAuthorization` | 3-check gate: syncing/AwaitingCanonicalBlock, min-peers (echo-chamber), gossip-silence (non-blocking, INC-I-050) |
| `SyncManager::classify_and_dispatch(shallow_rollback_count)` | `sync/manager/block_lifecycle.rs:623` | `fn classify_and_dispatch(&mut self, u32) -> (RecoveryAction, Option<RecoveryContext>)` | Authoritative (M2) recovery decision: builds `RecoveryContext`, calls coordinator `classify()`, records action |
| `RecoveryCoordinator::classify(ctx)` | `sync/manager/recovery.rs:270` | `fn classify(&self, ctx: &RecoveryContext) -> RecoveryAction` | Pure function; 7 ordered rules, least-severe-that-fits (see CONSTRAINTS INC-I-138) |
| `SyncManager::report_orphan_gossip/empty_headers/apply_failure/stale_tip/stuck_fork()` | `sync/manager/block_lifecycle.rs:668-704` | `fn report_orphan_gossip(&mut self, slot: u32, gap: u64)` | Detectors REPORT evidence only; never decide (recovery.rs owns decision) |
| `SyncManager::note_orphan_gossip_block(h, slot)` | `sync/manager/peers.rs:465` | `fn note_orphan_gossip_block(&mut self, block_height: u64, block_slot: u32)` | Legacy anti-cascade path (pre-M2); INC-I-138 D5 gates suppression on `peer_block_applied_since_rollback`, not raw height comparison |
| `SyncManager::next_request()` | `sync_engine/dispatch.rs:15` | `fn next_request(&mut self) -> Option<(PeerId, SyncRequest)>` | Fork-recovery-first, then Headers (height-based fallback INC-I-012 F1), escalation ladder |
| `SyncManager::handle_response(peer, resp)` | `sync_engine/response.rs:16` | `fn handle_response(&mut self, peer: PeerId, response: SyncResponse) -> Vec<Block>` | Discards stale sync-epoch responses; feeds headers/bodies pipeline |
| `SyncManager::start_sync()` | `sync_engine/decision.rs:117` | `fn start_sync(&mut self)` (crate-visible) | Idempotent; decides snap-vs-header-first; uses `consensus_target_hash()` majority-vote to avoid partition steering |
| `SyncManager::best_peer()` | `sync_engine/decision.rs:81` | `fn best_peer(&self) -> Option<PeerId>` | Load-distributing pseudo-random pick seeded by `(local_height, sync_epoch)` — avoids thundering herd (INC-I-014/017) |
| `SyncManager::handle_snap_state_root/snapshot/download_error()` | `sync/manager/snap_sync.rs:19,137,216` | `fn handle_snap_state_root(&mut self, peer, block_hash, block_height, state_root)` | Vote quorum (majority of total peers, capped 5), height-window reject (±100), root-verify deferred to node |
| `SyncManager::take_snap_snapshot()` | `sync/manager/snap_sync.rs:257` | `fn take_snap_snapshot(&mut self) -> Option<VerifiedSnapshot>` | Transitions to `Synchronized`, sets `AwaitingCanonicalBlock` gate (block store empty post-snap) |
| `SyncManager::cleanup()` | `sync/manager/cleanup.rs:13` | `fn cleanup(&mut self)` | Per-tick: tip decay, stuck-sync/body-stall/deadlock detection, blacklist expiry, snap-retry, height-offset detection, grace timeouts |
| `SyncManager::request_genesis_resync(reason)` | `sync/manager/production_gate.rs:660` | `fn request_genesis_resync(&mut self, reason: RecoveryReason) -> bool` | Single gated entry point for all resync triggers: floor, concurrency, rate-limit, snap-availability, snap-attempt-limit (emergency bypass for 3 reasons) |
| `SyncManager::reset_local_state(genesis_hash)` | `sync/manager/block_lifecycle.rs:281` | `fn reset_local_state(&mut self, genesis_hash: Hash)` | Refuses to reset below `confirmed_height_floor` (INC-I-005 Fix C) |
| `PeerScorer::new/tick()` | `scoring.rs:155,284` | `fn tick(&mut self)` | Call periodically: decay scores, expire bans |
| `RateLimiter::check_block/check_block_global_only/check_sync_request()` | `rate_limit.rs:209,232,333` | `fn check_sync_request(&mut self, peer: &PeerId) -> bool` | INC-I-120: outbound sync-request governor (per-peer 10/s burst 100, global 60/s burst 300) |
| `classify_block_gossip()` / `classify_producer_gossip()` | `gossip/validation.rs:96,157` | `fn classify_block_gossip(data, genesis_time, slot_duration, now_unix) -> (MessageAcceptance, Option<Block>)` | INC-I-114: staleness verdict for `validate_messages` gate; `Ignore` (no penalty) vs `Reject` (P4 penalty) vs `Accept` |

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Join the network (bootstrap) | 1. `NetworkService::new()` builds swarm 2. dials bootstrap addrs first, then capped cached-peer addrs 3. discv5 UDP walk (if enabled) discovers more peers 4. status handshake exchanges `fork_id`/`genesis_hash`/version | `NetworkService::new(config)`, `plan_startup_dials()`, `Discv5Service::new()` | `NetworkConfig` (bootstrap_nodes, max_peers, genesis_hash, fork_id) | `peer_count() > 0`, `SyncManager::is_bootstrap_ready()==true` |
| Sync a node from behind (header-first) | 1. peer status reveals gap ≥3 (or ≥1 at height 0) 2. `start_sync()` picks load-balanced peer 3. `GetHeaders`/`GetHeadersByHeight` → validate chain link 4. `GetBodies` parallel download 5. apply blocks sequentially, `update_local_tip()` | `SyncManager::should_sync()`, `start_sync()`, `next_request()`, `handle_response()` | connected peers with `best_height > local_height` | state reaches `Synchronized`, `pipeline_stats()==(0,0,0)` |
| Snap sync a far-behind node | 1. gap > 50 (threshold) + ≥3 peers 2. `GetStateRoot` broadcast, collect votes (15s timeout) 3. majority-agreeing group ≥ quorum(capped 5) 4. `GetStateSnapshot` from quorum peer, node re-verifies state_root independently 5. `take_snap_snapshot()` → `AwaitingCanonicalBlock` gate until first peer block applies | `SyncManager::handle_snap_state_root()`, `handle_snap_snapshot()`, `take_snap_snapshot()` | ≥3 connected peers, `snap.attempts < 3` | `Synchronized` + first canonical gossip block clears `AwaitingCanonicalBlock` |
| Recover from a detected fork | 1. detector reports evidence (`OrphanGossip`/`EmptyHeaders`/`ApplyFailure`/`StuckFork`/`StaleTip`) to `RecoveryCoordinator` 2. `classify_and_dispatch()` returns least-severe action 3. caller executes: `ShallowRollback{depth}` / `HeaderFirstSync` / `SnapSync` / `GenesisResync` | `report_*()` fns, `classify_and_dispatch()`, `RecoveryCoordinator::classify()` | evidence within 120s TTL window, ≥2 matching events (INC-I-138 D2) | gap closes, `consecutive_empty_headers` resets to 0 |
| Diagnose peer scoring / eviction thrash | 1. check `PeerScorer::get_score(peer)` 2. check `eviction_cooldown` / grace-period (`DOLI_EVICTION_GRACE_SECS`, default 30s) 3. inspect `peers_to_disconnect()` | `PeerScorer::stats()`, `peers_to_disconnect()`, log grep `[MEM-CONN]`/eviction lines | peer_id or log window | root cause identified: score<-200 disconnect, score<-500 ban, or grace/LIFO tiebreak |
| Add a new gossip topic (dev task) | 1. const in `gossip/mod.rs` 2. `publish_<name>()` in `gossip/publish.rs` 3. subscribe in `gossip/config.rs::subscribe_to_topics()` 4. `NetworkEvent::New<Name>`/`NetworkCommand::Broadcast<Name>` in `service/types.rs` 5. wire in `behaviour_events.rs` + `command_handling.rs` | edits across 5 files | new topic name | node publishes+receives on new topic without breaking existing ones |

## DATA FLOW

| Input | Transform | Output | Location |
|-------|-----------|--------|----------|
| Peer gossip block bytes | `validate_messages` hold → `classify_block_gossip()` (deserialize once, staleness check vs wall-clock slot) → `report_message_validation_result` | `NetworkEvent::NewBlock(block, peer_id)` via `enqueue_or_shed()` (bounded, load-shed) | `service/behaviour_events.rs:82`, `gossip/validation.rs:96`, `service/backpressure.rs:74` |
| `try_produce_block()` output | `NetworkService::broadcast_block()` → `NetworkCommand::BroadcastBlock` mpsc → gossipsub publish | wire bytes on `/doli/blocks/1` topic | `service/mod.rs:448` → `command_handling.rs:29` |
| New peer connection | `StatusRequest`/`StatusResponse` exchange (version, network_id, genesis_hash, fork_id, producer_pubkey) | `SyncManager::add_peer()`/`update_peer()`, version/fork_id mismatch → disconnect | `protocols/status.rs`, `sync/manager/peers.rs:19,73` |
| Sync gap detected | `SyncManager::start_sync()` → `SyncRequest::GetHeaders`/`GetHeadersByHeight` → outbound governor (`is_rate_governed()`) → `SyncResponse::Headers` → `SyncResponse::Bodies` → sequential apply | node's `apply_block()` per downloaded block | `sync_engine/decision.rs:117`, `protocols/sync.rs:195` |
| Snap sync gap (>50, ≥3 peers) | `GetStateRoot` votes → majority-quorum group → `GetStateSnapshot` from quorum peer → node `compute_state_root_from_bytes()` independent verify | `VerifiedSnapshot{chain_state, utxo_set, producer_set, epoch_state_bytes}` applied wholesale | `sync/manager/snap_sync.rs:19-207` |
| Tx announce-request (bandwidth opt.) | `encode_tx_announce(hashes)` (0xAA prefix) gossip publish → peer `TxFetchRequest{hashes}` → `TxFetchResponse{txs}` (max 50 hashes/req) | full `Transaction` objects delivered only to interested peers | `gossip/publish.rs:216`, `protocols/txfetch.rs` |
| Orphan/apply-failure/stale-tip signal | detector `report_*()` → `RecoveryCoordinator` evidence window (120s TTL, 256 cap) → `classify()` (7 ordered rules, 30s action cooldown) | `RecoveryAction` executed by caller (periodic.rs / node layer) | `sync/manager/recovery.rs:270-437` |
| Fork orphan block | `ForkRecoveryTracker::start()` walks `prev_hash` backward via `GetBlockByHash` (10s per-request timeout, failover to alternate peer) until connects to local block_store | `CompletedRecovery{blocks (forward order), connection_point}` → reorg evaluation | `sync/fork_recovery.rs:88-239` |

## DEPENDENCIES

| This Domain Uses | Skill File | What For |
|------------------|-----------|----------|
| `doli_core::{Block, BlockHeader, Transaction, ProducerAnnouncement, ProducerBloomFilter, FinalityTracker, decode_producer_set}` | consensus/core skill (if present) | Block/tx types, finality tracking, producer-set CRDT decode used in gossip validation |
| `crypto::{Hash, PublicKey, hash::hash}` (BLAKE3) | — | Message IDs (gossip dedup), status/sync wire hashes, fork_id computation |
| `libp2p` (gossipsub, kad, request_response, relay, dcutr, autonat, noise, yamux, tcp/dns transport) | — | Full P2P transport stack; this crate is the sole consumer |
| `discv5` crate | — | UDP peer discovery (`discovery/discv5_service.rs`), primary discovery mechanism |
| `NetworkParams` (mesh_n, activation heights, block size constants) | consensus/core skill | `GOSSIP_MAX_TRANSMIT_SIZE = BASE_BLOCK_SIZE + GOSSIP_ENVELOPE_MARGIN` (INC-I-091 large-block support) |

| Used By | Skill File | What For |
|---------|-----------|----------|
| `bins/node/src/node/*` (init.rs, event_loop.rs, block_handling.rs, fork_recovery.rs, production.rs, periodic.rs, rollback.rs) | node/core skill (if present) | Drives `NetworkService`/`SyncManager` as the network I/O and sync-state backbone; `apply_block()` calls `update_local_tip()`/`block_applied_with_weight()`; recovery actions executed here |
| `crates/rpc/src/methods/*` | rpc skill / doli-network skill | `getPeers`, `getNetworkStatus`, gossip-shed metrics, guardian pause/resume touch `NetworkService` handle |
| `crates/mempool` | — | Consumes `NetworkEvent::NewTransaction`/`TxFetchResponse`, produces txs for `broadcast_transaction()` |
| `crates/updater` (hardfork.rs) | — | `CURRENT_PROTOCOL_VERSION`/`fork_id` interplay with `HardForkSchedule` (constant-gate discipline, NOT schedule entries, for rolling deploys) |

## CONSTRAINTS

| Constraint | Type | Location | Detail |
|-----------|------|----------|--------|
| `max_established_per_peer=1`, `total_conn_limit=max_peers+10`, `pending_limit=5` (env-overridable) | performance | `service/mod.rs:159-186` | INC-I-014: independent per-direction limits double effective total if not capped via `with_max_established()`; pending conns allocate ~456KB pre-handshake |
| Eviction grace period 30s + LIFO age tiebreaker | invariant | `service/swarm_events.rs:144-166` | INC-I-016: non-producer peers (seeds/relays) sit at gossipsub score 0.0 — without grace+tiebreak, eviction is effectively random and fragments the mesh |
| Eviction rate limit: max 5/60s | invariant | `service/swarm_events.rs:118-125` | Prevents evict→reconnect→evict churn from exhausting RAM at >max_peers network sizes |
| Bootstrap dial priority over cached peers | invariant | `service/mod.rs:292-330`, `service/helpers.rs:129` | INC-I-048: with `pending_limit=5`, stale cached-peer dials previously starved bootstrap dials entirely (nodes stuck at 0 peers) |
| `check_block_global_only()` bypasses per-peer limit | invariant | `rate_limit.rs:232` | INC-I-049: candidate-next-slot blocks must never be silently dropped by per-peer rate limits |
| Outbound sync-request governor: 10/s per-peer (burst 100), 60/s global (burst 300) | performance/security | `rate_limit.rs:333-362`, `protocols/sync.rs:183-206` | INC-I-120: throttles ONLY bulk-catchup classes (`GetHeaders`/`GetBodies`/`GetBlockByHeight`); recovery+canonical-critical classes (`GetStateSnapshot`,`GetStateRoot`,`GetHeadersByHeight`,`GetBlockByHash`,`DirectAttestation`) bypass entirely via `is_rate_governed()==false` — throttling these caused a 9-min fork (INC-I-049 lesson) |
| `CURRENT_PROTOCOL_VERSION=8`, `EPOCH_STATE_FORMAT_VERSION=1`, `MIN_PEER_PROTOCOL_VERSION=1` | invariant/security | `protocols/status.rs:49,68,83` | INC-I-054: bumping `CURRENT_PROTOCOL_VERSION` unnecessarily triggers `delete_epoch_state()` on every restart → non-deterministic rebuild → fork. Only bump when `EpochState` wire format changes; use `EPOCH_STATE_FORMAT_VERSION` for that |
| `fork_id` mismatch → immediate disconnect | security | `protocols/status.rs:90-131` | `fork_id = BLAKE3(genesis_hash \|\| sorted active fork heights)`; old peers send `Hash::ZERO` (`#[serde(default)]`), tolerated |
| `GENESIS_MISMATCH_COOLDOWN_SECS=86400` | performance | `service/types.rs:14` | Genesis-mismatch peers silently ignored 24h, no retry |
| `MAX_SYNC_SIZE=16MB`, `MAX_STATUS_SIZE=64KB`, `MAX_TXFETCH_SIZE=256KB`/`MAX_TXFETCH_HASHES=50` | security | `protocols/sync.rs:22`, `status.rs:86`, `txfetch.rs:20,23` | INC-I-012 F13: sync size reduced from 64MB — length-prefix DoS mitigation |
| `GOSSIP_MAX_TRANSMIT_SIZE = BASE_BLOCK_SIZE + GOSSIP_ENVELOPE_MARGIN` | invariant | `gossip/config.rs:26` | INC-I-091: gossip cap must be ≥ largest producible block or valid blocks can't propagate; production block size gated separately by `large_block_activation_height` |
| Gossip message ID = BLAKE3(data)[0..20] | invariant | `gossip/config.rs:185-189` | INC-I-012 F9: `DefaultHasher` is NOT platform-independent (x86 vs ARM) — would break cross-platform dedup |
| `validate_messages=true` + `enqueue_or_shed()` bounded queue REQUIRED together with `flood_publish=true` / `duplicate_cache_time≤30s` | security/invariant (INV-NETWORK-002) | `gossip/config.rs:89-138` | `assert_gossip_hardening_invariant()` fails fast at construction if aggressive gossip config lacks BOTH mitigations. Lineage: INC-I-009/014/114/118/120 all share unbounded-queue + aggressive-propagation root shape |
| `STALE_BLOCK_SLOT_THRESHOLD=6` slots (60s) | invariant | `gossip/validation.rs:35` | Stale gossip blocks → `Ignore` (no penalty, INC-I-016 safety), not `Reject` — avoids eviction cascades on honest catch-up peers |
| `PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS=3600` | invariant | `gossip/validation.rs:135` | INC-I-137: only forward producer-gossip if ≥1 announcement is within TTL (GSet convergence preserved); all-stale → `Ignore` |
| `DOLI_IP_COLOCATION_THRESHOLD` (default 5) | performance | `gossip/config.rs:308-317` | Devnet with many 127.0.0.1 nodes MUST raise to ~500 or co-located peers exceed graylist (-16K) at 33+ peers |
| Memory watchdog soft threshold (default 0=disabled), fail-open on non-Linux/no-sampler | performance | `watchdog.rs:41-118`, `config.rs:87-98` | INC-I-114 M2: trips shared flag to shed ALL gossip blocks above RSS threshold; edge-triggered logging only |
| `SyncState` 3 variants, ALL 9 transitions valid | invariant | `sync/manager/mod.rs:368-410` | `Idle\|Syncing{phase}\|Synchronized`; `is_valid_transition()` always true — invalid states made unrepresentable by design, not by transition-blocking |
| Snap sync quorum = `max(3, min(total_peers/2+1, 5))` | security | `sync/manager/snap_sync.rs:67-75` | Cap at 5 regardless of peer count (uncapped formula unreachable within 15s root_timeout at 1K+ nodes); real trust boundary is independent `compute_state_root_from_bytes()` re-verification, not quorum size |
| Snapshot/state-root height window: reject if `block_height < target_height - 100` | security | `sync/manager/snap_sync.rs:35,162` | INC-I-004: prevents stale/freshly-wiped peers from contaminating quorum or serving outdated snapshots |
| `RecoveryCoordinator` thresholds: `MIN_MINOR_FORK_EVIDENCE=2` (was 3), `MINOR_FORK_GAP_MAX=50`, `SNAP_SYNC_GAP_MIN=500`, `SHALLOW_ROLLBACK_MAX=10`, `SNAP_ATTEMPTS_MAX=3`, `SNAP_MIN_PEERS=3`, `STALE_TIP_SECS=300`, `EVIDENCE_TTL=120s`, `ACTION_COOLDOWN=30s` | invariant | `sync/manager/recovery.rs:196-248` | INC-I-138 D2: threshold lowered 3→2 so 2 events within one 30s coordinator tick satisfy minor-fork evidence (was requiring cross-tick accumulation, permanently unreachable under a dominant counter-reset). D4: `deep_fork_confirmed` gap-guards the empty-header branch (`gap ≥ MINOR_FORK_GAP_MAX`) after a gap=28 SnapSync misfire lost blocks 37-63 |
| `StuckFork` evidence variant + INV-SYNC-001/002/004/008 finality guard | invariant/security | `sync/manager/recovery.rs:96-104,325-378` | INC-I-120 RC-2: closes the "genuinely forked + stale tip never satisfies `recently_synced()`" gap that looped `HeaderFirstSync` forever; ShallowRollback refuses to land BELOW `last_finality_height` (fencepost: landing ON finality is legal, `<` not `<=`, INC-I-090) |
| `peer_block_applied_since_rollback` gate (not raw `local_height > rb_h`) | invariant | `sync/manager/peers.rs:534-567`, `types.rs:558-587` | INC-I-138 D5: raw height comparison fired on self-produced fork blocks too (measured 21× false suppressions → 325s stall); only block-handling layer (gossip/sync/reorg), never self-production, sets this flag |
| Rollback state TTL = 300s | invariant | `sync/manager/recovery.rs:174-187` | INC-I-049: stale `last_rollback_local_height` from hours ago permanently suppressed fork detection; now expires |
| `confirmed_height_floor` monotonic progress floor | invariant | `sync/manager/mod.rs:151`, `production_gate.rs:673-688` | INC-I-005 Fix C: once Synchronized + 10 stable blocks, `reset_local_state()`/`request_genesis_resync()` refuse to go below this height (breaks infinite snap-sync death spiral); emergency reasons (deep fork, apply-failure-threshold) bypass |
| `MAX_CONSECUTIVE_RESYNCS=5` | invariant | `sync/manager/mod.rs:55` | Hard stop on repeated forced resyncs; requires manual `recover --yes` after |
| Fork recovery `MAX_RECOVERY_DEPTH=1000`, per-request timeout 10s + failover, session timeout 120s, cooldown 30s | performance | `sync/fork_recovery.rs:18-27` | Bounded parent-chain walk; genesis-hash cross-chain rejection built in |
| `PeerCache` never caches loopback (127.x / ::1) | invariant | `peer_cache.rs:74-77` | Prevents self-dial loops when cache shared/copied across co-located test nodes |
| Kademlia replication factor = 8 (was 20) | security | `discovery/kademlia.rs:22` | INC-I-050: factor 20 in a 26-node net (77%) let a single bad DHT entry reach nearly every node in one 60s bootstrap cycle |
| `messages.rs::Message` enum | dead-code (unclear) | `messages.rs` | Declared in `lib.rs` (`pub mod messages`) but not re-exported/used by wire protocols (status/sync/txfetch have their own typed request/response enums) — appears legacy/vestigial. `[UNCLEAR]` whether still referenced by tests only |

## PATTERNS

| Pattern | Example Location | Usage |
|---------|-------------------|-------|
| Recovery evidence report → coordinator classify → caller executes | `sync/manager/block_lifecycle.rs:623-704`, `recovery.rs:270` | Detectors NEVER decide actions; centralizes fork-recovery policy in one pure-function classifier for testability (see recovery.rs's 30+ unit tests) |
| Escalation ladder, least-severe-first | `recovery.rs:311-437` | `None < ShallowRollback < HeaderFirstSync < SnapSync < GenesisResync`; each rule gate checked in order, first match wins |
| Single outbound chokepoint for governed requests | `service/command_handling.rs:78-98` | ALL `NetworkCommand::RequestSync` funnels through one `if request.is_rate_governed()` check before `swarm.send_request()` — add new SyncRequest variants by declaring their governed-ness in `SyncRequest::is_rate_governed()`, not by scattering checks |
| Bounded-queue + validate-then-forward gossip hot path | `service/backpressure.rs:74`, `gossip/validation.rs:58-124` | `try_send` (never `.await`s) + pure classify function decouples I/O from decision logic; same shape required for any new "aggressive propagation" gossip feature per INV-NETWORK-002 |
| Sub-struct grouping to prevent illegal states | `sync/manager/types.rs` (`ForkState`, `NetworkState`, `SnapSyncState`, `SyncPipeline`) | Independent booleans (`resync_in_progress`, `post_recovery_grace`, `awaiting_canonical_block`) replaced by single `RecoveryPhase` enum — illegal combinations become unrepresentable |
| Load-distributing peer selection seeded by local state | `sync_engine/decision.rs:81-109`, `snap_sync.rs` quorum group `max_by_key` | Avoid thundering-herd on a single "best" peer at scale — hash `(local_height, sync_epoch)` into peer index instead of always picking absolute max height |
| Diagnosing sync stuck | check in order: `sync_state_name()` → `pipeline_stats()` → `progress()` → `consecutive_empty_headers()` (≥10→deep fork) → `fork.use_height_based_headers` (post-snap fallback active) → `recovery_phase` | `production_gate.rs`, `mod.rs:437-457` |
| Diagnosing production blocked | `can_produce()` → match `ProductionAuthorization` variant (`BlockedSyncing`/`BlockedInsufficientPeers`/`BlockedBootstrap`/`BlockedAwaitingCanonicalBlock`/`BlockedExplicit`) | `production_gate.rs:41-221`, `types.rs:183-209` |
| Adding a new gossip topic | const in `gossip/mod.rs` → `publish_<name>()` in `publish.rs` → subscribe in `config.rs` → `NetworkEvent`/`NetworkCommand` variant in `service/types.rs` → wire in `behaviour_events.rs`+`command_handling.rs` | `gossip/mod.rs:20-44`, `config.rs:384-418` |
| Adding a new `SyncRequest` variant | enum variant + constructor in `protocols/sync.rs` → declare `is_rate_governed()` → handle in node's `handle_sync_request()` (outside this crate) → no codec change needed (bincode + length-prefix generic) | `protocols/sync.rs:26-207` |
| Transport wire encoding | `[u32 LE length][bincode payload]`, reject-then-read pattern, all 3 protocols (status 64KB, sync 16MB, txfetch 256KB) | `protocols/status.rs:225-315`, `sync.rs:256-347`, `txfetch.rs` |
