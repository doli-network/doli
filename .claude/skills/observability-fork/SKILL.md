---
name: observability-fork
description: "DOLI fork observability — detect, measure, and diagnose chain divergence. Use when: fork detected, are nodes diverging, is there a fork, state root mismatch, find divergence point, compare state between nodes, reorg detection, show diagnostic events, fleet health, getForkDiagnostic, getFleetForkDiagnostic, getStateRootDebug, getUtxoDiff, fork-monitor, health-check, diagnostic ledger, fork classifier. For recovery procedures see guardian skill."
---

<!-- @INDEX
ENTRY-POINTS    15-32
OPERATIONS      34-49
DATA-FLOW       51-66
DEPENDENCIES    68-86
CONSTRAINTS     88-106
PATTERNS        108-120
SUPPLEMENTARY   RPC-CHEATSHEET.md (curl payloads for all methods), LEDGER-SCHEMA.md (event types, classifier rules, storage layout)
@/INDEX -->

## ENTRY POINTS

| Function/Endpoint | Location | Signature | Description |
|---|---|---|---|
| `getForkDiagnostic` | `crates/rpc/src/methods/dispatch.rs:74` + `diagnostics.rs:43` | `async fn get_fork_diagnostic(&self, params: Value) -> Result<Value, RpcError>` | Returns `DiagnosticBundle`: events, fork_summary, classification, baseline, health. Params: `window_secs` (default 3600), `fork_event_id` (causal chain mode), `limit` (cap 10000), `kind`, `min_height`, `max_height` |
| `getFleetForkDiagnostic` | `crates/rpc/src/methods/dispatch.rs:76` + `diagnostics_fleet.rs:65` | `async fn get_fleet_fork_diagnostic(&self, params: Value) -> Result<Value, RpcError>` | Fans out `getForkDiagnostic` to `peer_rpcs[]` in parallel, aggregates into `FleetBundle` (fork_groups, divergence_table, fleet_summary). Params: `peer_rpcs` (required), `window_secs`, `limit` |
| `getStateRootDebug` | `crates/rpc/src/methods/dispatch.rs:48` + `stats.rs:66` | `async fn get_state_root_debug(&self) -> Result<Value, RpcError>` | Returns `{height, bestHash, stateRoot, csHash, utxoHash, psHash, utxoCount, producerCount, totalMinted, registrationSeq}` — per-component hashes for cross-node comparison |
| `getUtxoDiff` | `crates/rpc/src/methods/dispatch.rs:49` + `stats.rs:112` | `async fn get_utxo_diff(&self, params: Value) -> Result<Value, RpcError>` | Full UTXO dump or targeted diff. With `{"referenceHashes": ["h1",...]}`: returns only entries that differ. Each entry: `{outpoint, hash, detail}` |
| `getChainStats` | `crates/rpc/src/methods/dispatch.rs:43` + `stats.rs:14` | `async fn get_chain_stats(&self) -> Result<Value, RpcError>` | Returns `{total_supply, address_count, utxo_count, active_producers, total_staked, height, reward_pool_balance, total_confirmed}` |
| `getChainInfo` | `crates/rpc/src/methods/dispatch.rs:28` | `async fn get_chain_info(&self) -> Result<Value, RpcError>` | Returns `{bestHeight, bestHash, genesisHash, ...}` — primary poll target of fork-monitor.sh |
| `getStateSnapshot` | `crates/rpc/src/methods/dispatch.rs:50` + `snapshot.rs:24` | `async fn get_state_snapshot(&self) -> Result<Value, RpcError>` | Full serialized state at current height — used for snap sync source verification |
| `DiagnosticLedger::record` | `crates/storage/src/diagnostic_ledger/mod.rs:84` | `fn record(&self, event: &DiagnosticEvent) -> Result<(), StorageError>` | Write a single event to RocksDB `cf_events` |
| `DiagnosticLedger::query_recent` | `crates/storage/src/diagnostic_ledger/mod.rs:100` | `fn query_recent(&self, window_secs: u64, limit: usize) -> Result<Vec<DiagnosticEvent>, StorageError>` | Time-window scan, oldest-first, limit capped at 10000 |
| `DiagnosticLedger::query_range` | `crates/storage/src/diagnostic_ledger/mod.rs:113` | `fn query_range(&self, kind: Option<EventKind>, min_height: u64, max_height: u64, limit: usize) -> Result<Vec<DiagnosticEvent>, StorageError>` | Prefix scan by kind + height range, limit capped at 10000 |
| `DiagnosticLedger::query_causal_chain` | `crates/storage/src/diagnostic_ledger/mod.rs:127` | `fn query_causal_chain(&self, start_event_id: &str, max_depth: usize) -> Result<Vec<DiagnosticEvent>, StorageError>` | Follows `caused_by_event_id` links, oldest-first, cycle-safe |
| `classify` | `crates/storage/src/diagnostic_ledger/classifier.rs:36` | `fn classify(events: &[DiagnosticEvent]) -> Classification` | Pure function — 8 rules, first-match-wins. Returns `ForkType` + confidence + recommended_action |
| `fork-monitor.sh` | `scripts/fork-monitor.sh:1` | `bash fork-monitor.sh [--testnet] [--loop [SECS]] [--endpoints FILE]` | Polls `getChainInfo` on all nodes, groups by `bestHash`, exits 0=OK/1=FORK/2=error |
| `health-check.sh` | `scripts/health-check.sh:1` | `bash health-check.sh [mainnet\|testnet\|all]` | 7-check suite: RPC responding, peer count, genesis hash, height delta, peer-ID errors, service file flags. Requires `DOLI_AI{1-5}` env vars |

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|---|---|---|---|---|
| **Detect fork — local devnet** | 1. Run script against devnet ports (28500-28550). 2. Interpret output. | `scripts/fork-monitor.sh` | Nodes running on 127.0.0.1:28500+ | `OK — N nodes, height=H, hash=<prefix>...` printed in green; exit 0 |
| **Detect fork — local testnet** | 1. Run script against testnet ports (8500-8512). | `scripts/fork-monitor.sh --testnet` | Nodes running on 127.0.0.1:8500-8512 | Same OK output |
| **Continuously monitor for forks** | 1. Start loop mode with optional interval. 2. Ctrl-C to stop. | `scripts/fork-monitor.sh --loop 30` | Same port range | Prints OK/FORK line every 30s |
| **Fork confirmed — get diagnostic bundle from one node** | 1. POST `getForkDiagnostic` with a time window. 2. Inspect `classification.fork_type` + `recommended_action`. 3. Check `health.events_dropped_total` — if > 0, some events were lost. | `curl -s -X POST http://127.0.0.1:28500 -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"window_secs":3600},"id":1}'` | Node RPC URL | JSON with `schema_version:1`, `classification.fork_type` set, `fork_summary.fork_events_in_window > 0` |
| **Fleet-wide fork diagnostic** | 1. POST `getFleetForkDiagnostic` with list of all node RPC URLs. 2. Inspect `divergence_table` for heights with competing hashes. 3. Inspect `fork_groups` for canonical/fork partition. 4. Check `fleet_summary.majority_classification`. | `curl -s -X POST http://127.0.0.1:28500 -d '{"jsonrpc":"2.0","method":"getFleetForkDiagnostic","params":{"peer_rpcs":["http://127.0.0.1:28500","http://127.0.0.1:28501"],"window_secs":3600},"id":1}'` | List of RPC URLs (max 50; override with `DOLI_FLEET_MAX_PEERS` env) | `FleetBundle` with `divergence_table` populated at fork height |
| **Find divergence point — compare state roots** | 1. Call `getStateRootDebug` on both nodes. 2. Compare `stateRoot`. If different: 3. Compare `csHash`, `utxoHash`, `psHash` to isolate which component diverged. | `curl -s -X POST http://127.0.0.1:28500 -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}'` on each node | Two node RPC endpoints at the SAME height | `stateRoot` matches → nodes agree; differing sub-hash identifies component |
| **Find first divergent UTXO** | 1. Dump UTXO hashes from node A (full dump). 2. Send hash list to node B as `referenceHashes`. 3. B returns only differing entries. | Step 1: `curl ... getUtxoDiff params:{}` → collect `entries[].hash` array. Step 2: `curl ... getUtxoDiff params:{"referenceHashes":[...]}` to node B | Two nodes at same height; node A's hash array | Response has `diffCount > 0` with specific `outpoint` + `detail` fields identifying divergent UTXOs |
| **Read diagnostic ledger for an incident window** | 1. `getForkDiagnostic` with `window_secs` covering incident. 2. Or `query_range` for a specific height band and kind. | `{"method":"getForkDiagnostic","params":{"window_secs":7200}}` or `ledger.query_range(Some(EventKind::ForkBlockReceived), min_h, max_h, 1000)` | Time window or height range | `events[]` array, oldest-first |
| **Trace causal chain from a specific event** | 1. `getForkDiagnostic` with `fork_event_id` set to the starting event's ULID. | `{"method":"getForkDiagnostic","params":{"fork_event_id":"<ULID>"}}` | A known event ULID from a prior query | Events array containing the ancestor chain, oldest first |
| **Check diagnostic writer health** | 1. `getForkDiagnostic` any params. 2. Read `health` field: `ledger_available`, `events_written_total`, `events_dropped_total`, `last_heartbeat_ms`. | Same as above | Any | `health.ledger_available=true`; `events_dropped_total=0` means no overflow |
| **Reproduce fork locally for testing** | 1. Run `testing/integration/reorg_test.rs::test_single_block_reorg` for 1-block reorg. 2. `test_deep_reorg_10_blocks` for deep reorg. 3. `partition_heal.rs::test_partition_separate_chains` for partition-then-heal. 4. `equivocation_slashing.rs` for double-signing. 5. `malicious_peer.rs` for invalid block injection. | `cargo test -p testing --test reorg_test` | Test binary | Tests pass; observe `ForkBlockReceived`/`ReorgExecuted` events in ledger |
| **Post-fork action routing** | 1. If classifier returns `recommended_action`: map to guardian procedure. 2. `manual_intervention` → divergence table `ProducerEquivocation` or `EpochBoundaryInvalid`. 3. `auto_recover` → `PostSnapDeadTip`. 4. `restart_with_resync` → `ChainBreakLoop`. 5. `normal_operation` → `TipRaceNatural`. | Check `classification.recommended_action` in `getForkDiagnostic` response | Classification result | See `../guardian/SKILL.md` for recovery procedures |

## DATA FLOW

| Input | Transform | Output | Location |
|---|---|---|---|
| Network gossip block (any classification) | `classify_gossip_block()` → `ForkBlock`/`Orphan`/`Rejected` | `DiagnosticEmitter::record(ForkBlockReceived)` | `bins/node/src/node/block_handling.rs:154-418` |
| `BlockClass::ForkBlock(HeightOccupied)` | Extract `fork_height`, `canonical_hash=None`, `fork_hash=block_hash` | `ForkBlockReceived` event with `CorrelationKey{divergence_height,fork_hash}` | `block_handling.rs:195-258` |
| `DiagnosticEmitter::record()` | Non-blocking push to `AsyncChannelEmitter` ring buffer (capacity-bounded, drop-oldest on overflow) | `DiagnosticEvent` in VecDeque | `crates/storage/src/diagnostic_ledger/emitter.rs:171-181` |
| Ring buffer drain (writer task) | `DiagnosticLedger::record()` → bincode serialize → RocksDB `cf_events` write | On-disk event keyed by `[kind u8][height u64 BE][ulid 16B]` | `diagnostic_ledger/mod.rs:84-93`, `types.rs:429-436` |
| `getForkDiagnostic` RPC call | `query_recent(window_secs, limit)` or `query_causal_chain(event_id, depth)` | `Vec<DiagnosticEvent>` from RocksDB | `diagnostics.rs:72-80` |
| `Vec<DiagnosticEvent>` | `classifier::classify(events)` — 8 rules, first-match-wins | `Classification{fork_type, confidence, evidence_event_ids, recommended_action}` | `diagnostics.rs:86`, `classifier.rs:36-60` |
| `Classification` + events | `build_fork_summary()` + `build_baseline()` + `DiagnosticWriterStats` read | `DiagnosticBundle{schema_version:1, node_peer_id, events, fork_summary, classification, baseline, health}` | `diagnostics.rs:83-119` |
| `getFleetForkDiagnostic` call with `peer_rpcs[]` | Parallel `reqwest` POST to each peer's `getForkDiagnostic` (per-peer 5s timeout, 30s total) | `Vec<PeerStatus>` — each has `bundle: Option<DiagnosticBundle>` or `error` | `diagnostics_fleet.rs:120-146` |
| `Vec<PeerStatus>` | `build_fork_groups()` — groups events by `CorrelationKey` string, partitions peers into canonical/fork/undecided | `Vec<ForkGroup>` | `fleet.rs:155-271` |
| `Vec<PeerStatus>` + `Vec<ForkGroup>` | `build_divergence_table()` — finds heights with >1 distinct `BlockApplied` hash across fleet | `Vec<DivergencePoint{height, competing_hashes, recommended_action}]` | `fleet.rs:276-338` |
| `fork-monitor.sh` poll | `getChainInfo` on each port → group by `bestHash` | `OK` (1 group) or `FORK` (N groups) + per-group node list | `scripts/fork-monitor.sh:88-115` |
| `getStateRootDebug` | `chain_state.serialize_canonical()` + `utxo_set.serialize_canonical()` + `producer_set.serialize_canonical()` → BLAKE3 each, combine, hash again | `{stateRoot, csHash, utxoHash, psHash}` | `stats.rs:70-104` |

## DEPENDENCIES

| This Domain Uses | Skill File | What For |
|---|---|---|
| `storage::DiagnosticLedger` (RocksDB backend) | `crates/storage/` | Persistence of diagnostic events at `<data_dir>/diagnostics/` |
| `storage::ChainState`, `UtxoSet`, `ProducerSet` | `crates/storage/` | `getStateRootDebug` state serialization; `getChainInfo` height/hash |
| `crates/rpc` dispatch + RpcContext | (this domain) | Method registration at `dispatch.rs:74-76`; `RpcContext::diagnostic_ledger` field at `context.rs:105` |
| `crypto::Hash` | `crates/crypto/` | BLAKE3 hashing for state root computation |
| `network::EquivocationDetector` | `crates/network/` | Double-signing detection feeding `ForkBlockReceived` events |
| `classifier::classify()` | (this domain) | Called inline by `getForkDiagnostic` handler — no separate crate |
| `ulid` crate | external | ULID generation for event IDs and RocksDB key ordering |
| `reqwest` crate | external | HTTP client for fleet peer queries in `diagnostics_fleet.rs` |

| Used By | Skill File | What For |
|---|---|---|
| Guardian recovery | `.claude/skills/guardian/SKILL.md` | `recommended_action` from `Classification` drives recovery procedures |
| `bins/node/src/node/block_handling.rs` | (node domain) | Emits `ForkBlockReceived` events on every non-tip gossip block |
| `bins/node/src/node/apply_block.rs` | (node domain) | Emits `BlockApplied` events (M2 — `BlockProvenance` passed as `Option`) |
| Operator scripts (`fork-monitor.sh`) | (this domain) | Polls `getChainInfo`; redirects to `getForkDiagnostic` on FORK |

## CONSTRAINTS

| Constraint | Type | Location | Detail |
|---|---|---|---|
| Logs are in FILES, not journalctl | invariant | CLAUDE.md + `scripts/health-check.sh:7` | `health-check.sh` reads `tail -200 ${log_path}` from files; app logs never in journalctl |
| `Hash::ZERO` is NOT a fork signal | security | MEMORY.md (INC-I-014) | A peer reporting `bestHash=0x000...0` is a sync state issue, not a fork. Do not react as fork. |
| Diagnostic ledger is per-node, not consensus-bound | invariant | `diagnostic_ledger/mod.rs:1-10` | Each node's `<data_dir>/diagnostics/` is independent. Fleet view requires polling each node. No events are gossiped. |
| Max query limit: 10,000 events | security | `queries.rs:15` `diagnostics.rs:22` | `REQ-FORKOBS-SEC-003`. Any `limit` param above 10000 is silently clamped. |
| `getForkDiagnostic` is strictly read-only | security | `diagnostics.rs` header `REQ-FORKOBS-SEC-002` | No writes to ledger. Verified by test `test_rpc_method_is_readonly` in `diagnostics_rpc_test.rs:567` |
| Fleet max peers: 50 default | performance | `diagnostics_fleet.rs:27` | Override with `DOLI_FLEET_MAX_PEERS` env. Exceeding returns `code:-32602`. Per-peer timeout: `DOLI_FLEET_PEER_TIMEOUT_SECS` (default 5s). Total: 30s wall-clock. |
| `AsyncChannelEmitter` drops OLDEST on overflow | invariant | `emitter.rs:175-178` | Ring buffer full → oldest event evicted, `dropped_count` incremented. Check `health.events_dropped_total` to detect loss. |
| `getUtxoDiff` only works with in-memory UTXO set | invariant | `stats.rs:143-147` | Returns `RpcError::internal_error("RocksDb UTXO set not supported for diff")` if node uses RocksDb UTXO backend. |
| RPC IP addresses redacted in fleet output | security | `fleet.rs:140-142` | `rpc_url` field in `PeerStatus` is always replaced with `"peer-N"` via `redact_rpc_url()`. No dotted-quad IPs in serialized `FleetBundle`. Enforced by test `test_no_ipv4_in_serialized_bundle`. |
| `classify()` is a pure function — no system clock | invariant | `classifier.rs:1-3` | Rule (h) `ChainBreakLoop` anchors "now" to `max(event.timestamp_ms)` in the slice, not `Instant::now()`. Safe to call in tests with fabricated timestamps. |
| Classifier rule (h) `ChainBreakLoop` fires BEFORE (e)/(f) | invariant | `classifier.rs:49-53` | Workflow #349: prevents stuck nodes from being mis-labelled `TipRaceNatural`. `ChainBreakLoop` thresholds: `chain_break_count>3` OR `fork_recv>100 && fork/applied>10` OR `rollback>10` OR `recovery_attempts>20` in last 1h. |
| Schema version check is `>` not `!=` | invariant | `types.rs:449-452` | Decoder accepts any version `<= CURRENT_SCHEMA_VERSION=1`. Future versions gracefully rejected. Forward-compatible. |
| `getStateRootDebug` state root uses `csHash||utxoHash||psHash → BLAKE3` | invariant | `stats.rs:87-91` | Combined 96-byte input hash. Must match `apply_block` state root computation for comparison to be valid. Stale reads between lock acquisitions can produce transient mismatches. |
| Prune preserves cascade-origin pins | invariant | `mod.rs:136-238` | For each unique `CorrelationKey`, the first (earliest ULID) event is pinned and never evicted by count-based pruning. Age-based pruning (`retention_secs`) removes stale regardless. |
| `ForkBlockReceived` from `HeightOccupied` sets `canonical_hash=None` | invariant | `block_handling.rs:209-213` | Only `fork_hash` is populated in `CorrelationKey` at emit time. `canonical_hash` is not populated inline — fleet aggregator uses `BlockApplied` events to determine canonical side. |

## PATTERNS

| Pattern | Example Location | Usage |
|---|---|---|
| **Diagnostic event emission in hot path** | `block_handling.rs:168-190` | `self.diagnostic_emitter.record(DiagnosticEvent{event_id: ulid::Ulid::new().to_string(), kind: EventKind::ForkBlockReceived, timestamp_ms: SystemTime::now()...})`— wrap in `let _ =` to discard result (non-blocking, fire-and-forget) |
| **CorrelationKey construction at emit site** | `block_handling.rs:209-213` | `correlation_key: Some(CorrelationKey{divergence_height: Some(fork_height), canonical_hash: None, fork_hash: Some(block_hash.to_hex())})` — always use at least one non-None field for grouping |
| **Classifier rule structure (add a new rule)** | `classifier.rs:36-59` | Add `if let Some(c) = rule_X_new_type(events) { return c; }` in `classify()` at desired priority position. Rule function returns `Option<Classification>`. All rules are pure functions with no I/O. |
| **RPC dispatch registration** | `dispatch.rs:73-76` | Add `"methodName" => self.handler_fn(request.params).await` to the `match` in `handle_request()`. Handler lives in `crates/rpc/src/methods/<name>.rs`. |
| **Fork-monitor polling loop** | `scripts/fork-monitor.sh:64-79` | For each port in range: `rpc_call $port getChainInfo` → extract `bestHeight` + `bestHash` via python3 JSON parse → accumulate `name|height|hash` lines → single python3 grouping pass |
| **Fleet peer redaction** | `fleet.rs:140-142` + `diagnostics_fleet.rs:186` | Always call `redact_rpc_url(&url, index)` before putting URL into any struct field that will be serialized. Never log or serialize raw RPC URLs. |
| **Test ledger construction** | `diagnostics_rpc_test.rs:140-144` | `DiagnosticLedger::open(tempdir.path())` — opens at `<tmpdir>/diagnostics/`. Keep `TempDir` alive for test duration. |
| **Emitter factory pattern** | `emitter.rs:154-163` | `AsyncChannelEmitter::new(capacity)` returns `(emitter, receiver)` sharing same buffer. Writer task holds receiver; node holds `Arc<dyn DiagnosticEmitter>` (the emitter). `NoOpEmitter` for graceful degradation when ledger is unavailable. |
