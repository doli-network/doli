# network — DOLI P2P Network Layer
<!-- @INDEX
ENTRY-POINTS: lines 16-43
DATA-FLOWS: lines 45-72
STRUCTS: lines 74-133
FUNCTIONS: lines 135-201
CONSTANTS: lines 203-237
CONSTRAINTS: lines 239-290
PATTERNS: lines 292-340
-->

## ENTRY-POINTS

`NetworkService::new(config)` → `service/mod.rs:75` — builds libp2p swarm, dials bootstrap nodes (priority before cached peers — INC-I-048), spawns `run_swarm` task. Returns handle.

`NetworkService::next_event()` / `try_next_event()` → `service/mod.rs:417,426` — node event loop drives network by polling these. `try_next_event` drains pending blocks before production.

`NetworkService::broadcast_block/transaction/attestation/header()` → `service/mod.rs:431-458` — sends `NetworkCommand` over mpsc channel to swarm loop.

`SyncManager::new(config, genesis_hash)` → `sync/manager/mod.rs:176` — standalone sync state machine, does NOT hold network handle. Node drives it by feeding events.

`SyncManager::update_local_tip()` → `sync/manager/mod.rs:278` — called after every `apply_block()`. Updates height/hash/slot, checks if synchronized, completes resync if in progress.

`PeerScorer::new(config)` / `PeerScorer::tick()` → `scoring.rs:155,284` — call `tick()` periodically to decay scores and clean bans.

`RateLimiter::new(config)` / `RateLimiter::check_block()` / `check_block_global_only()` → `rate_limit.rs:159,184,207` — `check_block_global_only` bypasses per-peer limit for candidate-next blocks (INC-I-049).

`NetworkService::update_best_slot(slot)` → `service/mod.rs:412` — node calls after every `apply_block()` to feed best slot to rate limiter for priority exemption.

## DATA-FLOWS

**Block gossip path:**
```
peer gossip → swarm event → behaviour_events.rs → NetworkEvent::NewBlock(block, peer_id) →
event_rx → node event loop → handle_new_block() → apply_block() → update_local_tip()
```

**Block broadcast path:**
```
node.try_produce_block() → NetworkService::broadcast_block() →
NetworkCommand::BroadcastBlock → command_handling.rs → gossip::publish_block()
```

**Status handshake:**
```
peer connected → StatusRequest received → NetworkEvent::StatusRequest{peer_id, request, channel} →
node sends StatusResponse → SyncManager::update_peer_status() → decides if sync needed
```

**Sync pipeline (header-first):**
```
SyncManager detects gap → NetworkCommand::RequestSync{GetHeaders} →
SyncResponse::Headers → SyncManager → NetworkCommand::RequestSync{GetBodies} →
SyncResponse::Bodies → SyncManager → node applies blocks sequentially
```

**Snap sync pipeline:**
```
gap > snap_threshold (50) → SyncPipelineData::SnapCollecting{votes} →
GetStateRoot from N peers → quorum (≥3) agrees on root →
SyncPipelineData::SnapDownloading → GetStateSnapshot from quorum peer →
SyncPipelineData::SnapReady{VerifiedSnapshot} → node applies snapshot
```

**Tx announce-request (bandwidth optimization):**
```
node has tx → encode_tx_announce(hashes) → gossip publish (0xAA prefix) →
peer receives TxAnnouncement → RequestTxFetch{hashes} → TxFetchResponse{txs}
```

**Fork detection path:**
```
orphan gossip block → RecoveryEvidence::OrphanGossip →
RecoveryCoordinator::report_evidence() → classify_and_dispatch() →
RecoveryAction::{ShallowRollback|HeaderFirstSync|SnapSync|GenesisResync}
```

## STRUCTS

### NetworkService — `service/mod.rs:52`
```
peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>  — shared with swarm loop
command_tx: mpsc::Sender<NetworkCommand>        — drives swarm
event_rx: mpsc::Receiver<NetworkEvent>          — node polls
best_slot: Arc<AtomicU32>                       — INC-I-049 rate limit exemption
```

### NetworkConfig — `config.rs:15`
Key fields: `max_peers`, `bootstrap_nodes`, `mesh_n/n_low/n_high`, `fork_id`, `enable_discv5`, `seed_mode`, `tx_announce_enabled`
Defaults: `max_peers=50`, `mesh_n=12/8/24`, `enable_discv5=true`

### SyncManager — `sync/manager/mod.rs:50`
Sub-structs: `pipeline: SyncPipeline`, `pipeline_data: SyncPipelineData`, `network: NetworkState`, `fork: ForkState`, `snap: SnapSyncState`, `recovery: RecoveryCoordinator`
Key defaults: `resync_grace_period_secs=30`, `max_slots_behind=2`, `gossip_activity_timeout_secs=180`, `min_peers_for_production=2`, `bootstrap_grace_period_secs=15`

### SyncState — `sync/manager/types.rs:62`
3 variants only: `Idle`, `Syncing{phase: SyncPhase, started_at}`, `Synchronized`

### SyncPhase — `sync/manager/types.rs:89`
`DownloadingHeaders | DownloadingBodies | ProcessingBlocks | SnapCollecting | SnapDownloading`

### SyncPipelineData — `sync/manager/types.rs:106`
`None | Headers{target_slot, peer, headers_count} | Bodies{pending, total} | Processing{height} | SnapCollecting{votes, asked} | SnapDownloading{quorum_root, peer, alternates} | SnapReady{snapshot}`

### VerifiedSnapshot — `sync/manager/types.rs:153`
Contains `chain_state`, `utxo_set`, `producer_set` (bincode), `state_root`, optional `epoch_state_bytes`. Node re-verifies state_root before applying.

### ProductionAuthorization — `sync/manager/types.rs:183`
`Authorized | BlockedSyncing | BlockedInsufficientPeers | BlockedExplicit | BlockedBootstrap | BlockedAwaitingCanonicalBlock`

### RecoveryPhase — `sync/manager/types.rs:328`
`Normal | ResyncInProgress | PostRecoveryGrace{started, blocks_applied} | AwaitingCanonicalBlock{started}`
Lifecycle: `Normal → ResyncInProgress → PostRecoveryGrace → Normal`

### ForkState — `sync/manager/types.rs:491`
`consecutive_empty_headers` (≥10 → genesis resync), `consecutive_apply_failures` (≥3 → resync), `needs_genesis_resync`, `use_height_based_headers` (INC-I-012 post-snap deadlock fix)

### SnapSyncState — `sync/manager/types.rs:429`
`threshold=50`, `quorum=3`, `root_timeout=15s`, `download_timeout=60s`

### RecoveryEvidence — `sync/manager/recovery.rs:72`
`EmptyHeaders{peer, gap} | OrphanGossip{slot, gap} | ApplyFailure{height} | DeepForkSuspected{empty, gap} | StaleTip{last_applied_secs, gap}`

### RecoveryAction — `sync/manager/recovery.rs:103`
`None | ShallowRollback{depth} | HeaderFirstSync | SnapSync | GenesisResync`
Ordered by severity. Classifier returns LEAST severe that fits evidence.

### PeerScore — `scoring.rs:51`
Range `-1000..1000`. Decay towards 0 (1 pt/min). Ban threshold: `-500`, disconnect threshold: `-200`.

### PeerLimits / RateLimiter — `rate_limit.rs:72,144`
Token bucket per peer. `max_blocks_per_minute=10`, `max_txs_per_second=50`, global `100 blocks/min`.

### StatusRequest / StatusResponse — `protocols/status.rs:90,111`
Wire: `version`, `network_id`, `genesis_hash`, `fork_id` (BLAKE3 of genesis + active fork heights)
`fork_id=Hash::ZERO` from old peers is tolerated (backward compat).

### SyncRequest — `protocols/sync.rs:26`
`GetHeaders{start_hash, max_count} | GetBodies{hashes} | GetBlockByHeight{height} | GetBlockByHash{hash} | GetStateSnapshot{block_hash} | GetStateRoot{block_hash} | DirectAttestation{data} | GetHeadersByHeight{start_height, max_count}`

### SyncResponse — `protocols/sync.rs:90`
`Headers(Vec<BlockHeader>) | Bodies(Vec<Block>) | Block(Option<Block>) | StateSnapshot{...} | StateRoot{block_hash, block_height, state_root} | Error(String)`

### PeerInfo — `peer.rs:14`
`id, address, version, best_height, best_hash, is_producer`
`is_producer` protects producer peers from eviction (SCALE-T2-004).

### PeerCache — `peer_cache.rs:23`
`MAX_CACHED_PEERS=100`. Never caches loopback addresses (self-dial loop prevention).

## FUNCTIONS

### gossip/publish.rs
- `new_gossipsub(keypair, mesh)` → `gossip/config.rs:56` — builds Gossipsub with BLAKE3 message IDs (platform-independent dedup), peer scoring, flood_publish=true
- `subscribe_to_topics(gossipsub)` → `gossip/config.rs:191` — subscribes to: blocks, txs, producers, votes, heartbeats, headers, attestations
- `publish_block/transaction/producer/vote/heartbeat/header/attestation(gossipsub, data)` → `gossip/publish.rs:12-93`
- `compute_dynamic_mesh(total_peers)` → `gossip/config.rs:21` — √N scaling for large networks, clamp(8, 50)
- `encode_tx_batch(txs)` → `gossip/publish.rs:120` — prefix 0xBA, u32 count LE, then [u32 len LE + bytes] per tx
- `encode_tx_announce(hashes)` → `gossip/publish.rs:216` — prefix 0xAA, u32 count LE, then 32-byte hashes
- `decode_tx_gossip(data)` → `gossip/publish.rs:147` — dispatches on prefix byte, returns `TxGossipMessage`
- `decode_tx_message(data)` → `gossip/publish.rs:167` — returns full txs only (skips announce messages)

### sync/manager/mod.rs
- `SyncManager::new(config, genesis_hash)` → `:176`
- `SyncManager::update_local_tip(height, hash, slot)` → `:278` — MUST call after every apply_block
- `SyncManager::best_peer_for_recovery()` → `:485` — prefers peers near network tip (within 10 blocks)
- `SyncManager::take_needs_mass_status_refresh()` → `:269` — consumes flag, node triggers status burst
- `SyncManager::pipeline_stats()` → `:433` — returns (pending_headers, pending_blocks, active_requests)

### sync/manager/types.rs
- `ForkState::recommend_action(gap, consecutive_rollbacks, max_rollback_depth, best_peer)` → `:587`
  Rules: ≥10 empty headers OR needs_genesis → NeedsGenesisResync; gap > max_depth → NeedsGenesisResync; <max_depth AND ≥3 empty → RollbackOne

### scoring.rs
- `PeerScorer::record_valid_block(peer)` → +10 pts
- `PeerScorer::record_invalid_block(peer, slot)` → -100 pts
- `PeerScorer::record_incompatible_version(peer, version)` → -200 pts (instant disconnect)
- `PeerScorer::record_spam(peer, msg_type)` → -50 pts
- `PeerScorer::record_malformed(peer)` → -30 pts
- `PeerScorer::peers_to_disconnect()` → `:296` — all peers below disconnect_threshold (-200)
- `PeerScorer::tick()` → `:284` — decay all scores, clean expired bans

### rate_limit.rs
- `RateLimiter::check_block(peer)` → `:184` — per-peer AND global check
- `RateLimiter::check_block_global_only()` → `:207` — INC-I-049: bypasses per-peer (use for candidate-next slots)
- `RateLimiter::record_block(peer, size)` → `:221` — consume tokens from both buckets
- `RateLimiter::cleanup(max_age)` → `:325` — LRU eviction, max 1000 tracked peers

### peer_cache.rs
- `PeerCache::load(path)` → `:29` — bincode decode, None if corrupt
- `PeerCache::save(path)` → `:47` — atomic (write .tmp then rename)
- `PeerCache::add(peer_id, address)` → `:73` — upsert, trim to 100, rejects loopback
- `PeerCache::addresses()` → `:105` — returns parsed Multiaddrs for dialing

### nat.rs
- `NatConfig::client()` → `:54` — seeds as relay servers, no relay server mode
- `NatConfig::relay_server()` → `:65` — enables relay server, 256 reservations

## CONSTANTS

### Version constants — `protocols/status.rs`
```rust
CURRENT_PROTOCOL_VERSION: u32 = 8     // peer handshake only — DO NOT bump for non-EpochState changes
EPOCH_STATE_FORMAT_VERSION: u32 = 1   // EpochState serialization — bump ONLY when struct changes
MIN_PEER_PROTOCOL_VERSION: u32 = 1    // peers below this are disconnected
```

**CRITICAL**: Bumping `CURRENT_PROTOCOL_VERSION` triggers `delete_epoch_state()` on every restart → non-deterministic rebuild → fork (INC-I-054). Only bump if `EpochState` serialization format changes.

### Protocol identifiers
```
PROTOCOL_ID = "/doli/1.0.0"           — lib.rs:123
STATUS_PROTOCOL = "/doli/status/1.0.0" — protocols/status.rs:14
SYNC_PROTOCOL = "/doli/sync/1.0.0"    — protocols/sync.rs:15
```

### Gossip topics — `gossip/mod.rs`
```
BLOCKS_TOPIC        = "/doli/blocks/1"
TRANSACTIONS_TOPIC  = "/doli/txs/1"
PRODUCERS_TOPIC     = "/doli/producers/1"
VOTES_TOPIC         = "/doli/votes/1"
HEARTBEATS_TOPIC    = "/doli/heartbeats/1"
HEADERS_TOPIC       = "/doli/headers/1"       — lightweight pre-announce
ATTESTATION_TOPIC   = "/doli/attestations/1"  — finality gadget
TIER1_BLOCKS_TOPIC  = "/doli/t1/blocks/1"     — dense mesh, validators only
```

### Tx gossip prefixes — `gossip/publish.rs`
```
TX_MSG_BATCH    = 0xBA   — full tx batch format
TX_MSG_ANNOUNCE = 0xAA   — hash announcement format
```

### Connection limits — `service/mod.rs:150-176`
```
total_conn_limit = max_peers + 10  (override: DOLI_CONN_LIMIT env)
pending_limit    = 5               (override: DOLI_PENDING_LIMIT env)
max_established_per_peer = 1
idle_timeout: mainnet=24h, testnet/devnet=5min (override: DOLI_IDLE_TIMEOUT_SECS)
```

### Recovery thresholds — `sync/manager/recovery.rs:182`
```
MIN_MINOR_FORK_EVIDENCE = 3      — events before escalation
MINOR_FORK_GAP_MAX      = 50     — gap separating shallow rollback vs header-first
SNAP_SYNC_GAP_MIN       = 500    — gap above which snap is preferred
SHALLOW_ROLLBACK_MAX    = 10     — rollbacks before escalating to snap
```

### Scoring penalties — `scoring.rs:36`
```
InvalidBlock     = -100
InvalidTx        = -20
Timeout(n)       = -5 * min(n, 10)
Spam             = -50
Duplicate        = -5
MalformedMessage = -30
IncompatibleVersion = -200
```

### Sync defaults — `sync/manager/types.rs:39`
```
max_headers_per_request = 500
max_bodies_per_request  = 128
max_concurrent_body_requests = 8
request_timeout         = 30s
stale_timeout           = 300s
```

### MAX_SYNC_SIZE — `protocols/sync.rs:22`
`16MB` per message. Reduced from 64MB (INC-I-012 F13: length-prefix DoS mitigation).

## CONSTRAINTS

**INC-I-014: Connection RAM limits**
`max_established_per_peer=1` — old value 2 caused ~86GB RAM at 156 nodes (1MB Yamux each). Independent in/out per-direction limits can double the effective total — always use `with_max_established()` for the cap. `pending_limit=5` — each pending (unfinished handshake) allocates ~456KB.

**INC-I-016: Gossip mesh eviction**
Non-producer nodes have all gossipsub scores at 0.0. Without LIFO tiebreaker and 30s grace period, every eviction tick randomly disconnects them, fragmenting the mesh. `DOLI_EVICTION_GRACE_SECS` (default 30) controls grace period.

**INC-I-012 F1: Post-snap header deadlock**
After snap sync, `local_hash` may be unrecognizable to canonical peers (forked peer snapshot). `ForkState::use_height_based_headers=true` triggers `GetHeadersByHeight` instead of `GetHeaders`, bypassing the hash lookup. Cleared after first successful response.

**INC-I-048: Bootstrap dial priority**
Bootstrap nodes are dialed BEFORE cached peers, directly on the swarm BEFORE `run_swarm` spawns. With `pending_limit=5`, cached peer dials previously starved the bootstrap dial completely (nodes stuck at 0 peers).

**INC-I-049: Rate limit exemption for candidate blocks**
`NetworkService::update_best_slot()` must be called after every `apply_block()`. `check_block_global_only()` exempts candidate-next-slot blocks from per-peer rate limits so they are never silently dropped.

**INC-I-054: Protocol version bump rule**
`CURRENT_PROTOCOL_VERSION` bump triggers `delete_epoch_state()` on restart → non-deterministic rebuild from local blocks → snap-synced nodes have incomplete history → different results → fork at next epoch boundary. Only bump when `EpochState` wire format changes.

**fork_id mismatch = immediate disconnect**
Both `StatusRequest` and `StatusResponse` carry `fork_id = BLAKE3(genesis_hash || sorted active fork heights)`. Mismatch on new-vs-new → disconnect. Old peers send `Hash::ZERO` (tolerated).

**GENESIS_MISMATCH_COOLDOWN_SECS = 86400**
Genesis-mismatch peers are silently ignored for 24h. No retry. (`service/types.rs:14`)

**Gossip dedup uses BLAKE3**
Message ID function uses `hash(data)[0..20]`. DefaultHasher is NOT platform-independent — using it would cause duplicate dedup failures across x86/ARM nodes. (`gossip/config.rs:62`)

**GossipSub ip_colocation_factor**
Default threshold=5. Devnet (many nodes on 127.0.0.1) MUST set `DOLI_IP_COLOCATION_THRESHOLD=500` or nodes exceed the graylist threshold (-16K score at 33+ co-located peers).

**SyncState transitions: all 9 valid**
All transitions between Idle/Syncing/Synchronized are valid. `set_state()` logs invalid transitions but code makes all combos legal (`is_valid_transition` always returns true). (`sync/manager/mod.rs:385`)

**Snap sync quorum: 3 peers minimum**
`SnapSyncState::quorum=3`. Fewer agreeing peers means no snapshot download — falls back to header-first. Blacklisted peers (bad snapshots) are excluded.

**RecoveryCoordinator is authoritative since M2 (2026-04-24)**
3 legacy detector→action paths replaced. Evidence is REPORTED via `report_evidence()`, action RETURNED via `classify_and_dispatch()`. Caller executes. Pure read — no direct side effects.

**`PeerCache` never caches loopback addresses**
127.x.x.x or ::1 are silently dropped. Prevents self-dial loops when cache is shared across nodes. (`peer_cache.rs:74`)

## PATTERNS

### Adding a new gossip topic
1. Define `const TOPIC: &str = "/doli/<name>/1"` in `gossip/mod.rs`
2. Add `publish_<name>(gossipsub, data)` in `gossip/publish.rs`
3. Add subscribe in `subscribe_to_topics()` in `gossip/config.rs`
4. Add `NetworkEvent::New<Name>(Vec<u8>)` in `service/types.rs`
5. Add `NetworkCommand::Broadcast<Name>(Vec<u8>)` in `service/types.rs`
6. Handle in `service/behaviour_events.rs` (decode + send event) and `service/command_handling.rs` (publish)

### Adding a new SyncRequest variant
1. Add variant to `SyncRequest` enum in `protocols/sync.rs`
2. Add constructor method on `SyncRequest` impl
3. Handle in node's `handle_sync_request()` (rpc/node level — not in network crate)
4. No codec changes needed (bincode + length prefix handles it automatically)

### Tuning mesh for network size
Call `compute_dynamic_mesh(total_peers)` → returns `MeshConfig`. Pass to `new_gossipsub(keypair, &mesh)`. Key: √N scaling for >20 peers, capped at 50. Devnet uses larger mesh for star topology.

### Diagnosing sync stuck
Check in order:
1. `SyncManager::sync_state_name()` — what state?
2. `SyncManager::pipeline_stats()` — (pending_headers, pending_blocks, active_requests)
3. `SyncManager::progress()` — percentage complete
4. `ForkState::consecutive_empty_headers` — ≥10 means deep fork, trigger genesis resync
5. `ForkState::use_height_based_headers` — true = post-snap deadlock recovery active
6. `RecoveryPhase` — which phase are we in?

### Diagnosing production blocked
`SyncManager::authorize_production()` returns `ProductionAuthorization`. Check variant:
- `BlockedSyncing` — wait for sync to complete
- `BlockedInsufficientPeers` — need ≥2 peers (default `min_peers_for_production`)
- `BlockedBootstrap` — waiting for first peer status response
- `BlockedAwaitingCanonicalBlock` — post-snap sync, waiting for first gossip block

### Connection RAM explosion (INC-I-014 pattern)
Symptoms: RSS grows without bound, especially at 100+ nodes.
Checks: `DOLI_CONN_LIMIT` and `DOLI_PENDING_LIMIT` env vars. `max_peers+10` total, `5` pending.
Root: per-direction limits (in/out) are INDEPENDENT — 60in + 60out = 120 total. Always use `with_max_established()`.

### Peer scoring flow
```
Block received → validate → record_valid_block(+10) OR record_invalid_block(-100)
Periodic tick → PeerScorer::tick() → decay + clean bans
Periodic tick → peers_to_disconnect() → disconnect if < -200
Score < -500 → should_ban() → ban() → 1h ban
```

### Fork recovery escalation ladder
```
OrphanGossip × N → RecoveryEvidence → classify_and_dispatch() →
  gap ≤ 50 + recently synced → ShallowRollback{1}
  medium gap or stale tip    → HeaderFirstSync
  gap ≥ 500 or empty headers ≥ 10 → SnapSync
  snap failed + apply failed → GenesisResync
```

### Transport wire encoding
All request-response protocols (status, sync, txfetch) use:
`[u32 LE length][bincode payload]`
Max sizes: status=64KB, sync=16MB. Read length prefix, reject if over limit, then read exactly that many bytes.
