# Fork Observability — Agent-Facing Schema Reference

The fork-diagnostic observability subsystem captures structured events at every
consensus-relevant decision point in the DOLI node. Its primary consumers are
diagnostic sub-agents that triage fork incidents without grepping logs. The
`getForkDiagnostic` RPC returns a self-contained `DiagnosticBundle` with events,
classification, baseline rate, and writer health.

---

## Event Kinds

| Kind | Discriminant | Description | Trigger Location | Key Payload Fields |
|------|-------------|-------------|------------------|--------------------|
| `BlockApplied` | 1 | Block validated and applied to chain state | `apply_block/mod.rs` | slot, block_hash, producer_pubkey, from_peer_id, validation_duration_ms, mode, tx_count |
| `BlockRejected` | 2 | Block failed validation | `apply_block/mod.rs` | slot, block_hash, producer_pubkey, rejection_reason, mode |
| `ForkBlockReceived` | 3 | Fork block received (height occupied or reorg candidate) | `block_handling.rs` | block_hash, block_slot, producer_pubkey, from_peer_id, classification, fork_kind, local_tip_hash, local_tip_height |
| `RollbackStarted` | 4 | Rollback operation began | `rollback.rs` | from_height, to_height, trigger, cumulative_depth |
| `RollbackCompleted` | 5 | Rollback operation finished | `rollback.rs` | from_height, to_height, duration_ms, success |
| `ReorgExecuted` | 6 | Chain reorganization executed | `fork_recovery.rs` | old_tip_hash, new_tip_hash, rollback_depth, applied_count, weight_delta |
| `RecoveryClassifyCall` | 7 | Recovery classifier invoked | `periodic.rs` | local_height, network_tip_height, peer_count, last_applied_secs, action_returned, rule_matched |
| `SnapSyncAttempted` | 8 | Snap-sync attempt started | `sync/manager.rs` | local_height, target_height, source_peer_id |
| `SnapSyncCompleted` | 9 | Snap-sync completed | `sync/manager.rs` | result, duration_ms |
| `SnapSyncFailed` | 10 | Snap-sync failed | `sync/manager.rs` | error, duration_ms |
| `ChainBreakDetected` | 11 | Parent-hash mismatch during header sync | `sync/manager.rs` | expected_prev_hash, actual_prev_hash, header_slot |
| `WriterHeartbeat` | 12 | Periodic health canary from writer task | `diagnostic_writer.rs` | events_written_total, events_dropped_total |

---

## DiagnosticBundle Schema

```typescript
interface DiagnosticBundle {
  schema_version: 1;                       // u16
  node_peer_id: string;                    // PeerId of reporting node
  query_timestamp_ms: number;              // wall-clock ms
  events: DiagnosticEvent[];               // events in query window
  fork_summary: ForkSummary;               // aggregate stats
  classification: Classification | null;   // classifier output or null
  baseline: BaselineComparison;            // rate comparison
  health: DiagnosticHealth;                // writer status
}

interface DiagnosticEvent {
  event_id: string;                        // ULID
  kind: EventKind;                         // enum discriminant
  timestamp_ms: number;
  height: number | null;
  correlation_key: CorrelationKey | null;
  caused_by_event_id: string | null;
  is_cascade_origin: boolean;
  payload: EventPayload;                   // kind-specific fields
}

interface ForkSummary {
  fork_events_in_window: number;
  by_producer: Record<string, number>;     // pubkey_hex -> count
  by_event_kind: Record<string, number>;   // kind_name -> count
  first_fork_height: number | null;
  last_fork_height: number | null;
}

interface Classification {
  fork_type: ForkType;                     // named variant or Unknown
  confidence: number;                      // 0.0 to 1.0
  evidence_event_ids: string[];
  recommended_action: string | null;
  recommended_action_args: object | null;
}

interface BaselineComparison {
  fork_events_per_hour_current: number;
  fork_events_per_hour_24h_avg: number;
  delta_pct: number;                       // (current - avg) / avg * 100
}

interface DiagnosticHealth {
  ledger_available: boolean;
  events_written_total: number;
  events_dropped_total: number;
  last_heartbeat_ms: number | null;
}

interface CorrelationKey {
  divergence_height: number | null;
  canonical_hash: string | null;
  fork_hash: string | null;
}
```

---

## Classification Types

| ForkType | Meaning | Recommended Action |
|----------|---------|-------------------|
| `TipRaceNatural` | Two producers in adjacent slots, low-latency race; benign | None (self-resolving) |
| `TipRaceHighLatency` | Tip race with validation_duration > 2000ms | Investigate network latency |
| `ProducerEquivocation` | Same producer emitted two blocks for the same height | `investigate_producer` — potential slashing |
| `EpochBoundaryInvalid` | Block rejected at epoch boundary (e.g., missing EpochReward) | `investigate_producer` — broken producer binary |
| `PostSnapDeadTip` | Dead tip after snap sync (INC-I-012 pattern) | Retry snap sync or shallow rollback |
| `ValidationDisagreement` | Nodes disagree on block validation | Investigate validation rules divergence |
| `RollbackLoop` | >3 rollbacks in 60 seconds | Investigate stuck recovery |
| `SnapSyncToMinorityFork` | Snap-synced to a peer on a minority fork | Re-snap from a healthy seed |
| `Unknown` | Novel pattern; carries `reason_unknown` + `evidence_event_ids` | Human review of evidence |

---

## Retention Policy

- **Default retention**: 30 days
- **Maximum events**: 100000 (100,000)
- **Env vars**:
  - `DOLI_DIAG_RETENTION_DAYS` — override retention period (default: 30)
  - `DOLI_DIAG_MAX_EVENTS` — override max event count (default: 100000)
- **Cascade-origin pin**: the first event per unique `correlation_key` is preserved
  even when pruning by age or count, ensuring trigger events survive cascades
- **Pruner cadence**: every 60 seconds


---

## RPC

### `getForkDiagnostic` (per-node)

**Params**: `{ window_secs: u64, limit?: u64, fork_event_id?: string }`

**Returns**: `DiagnosticBundle` (see schema above)

**Error codes**: `-32603` when the diagnostic ledger is unavailable

See [docs/rpc_reference.md](rpc_reference.md) for full specification and examples.

### `getFleetForkDiagnostic` (fleet-wide)

**Params**: `{ peer_rpcs: string[], window_secs?: u64, limit?: u64 }`

- `peer_rpcs` (required): RPC URLs to query. Max 50 (env `DOLI_FLEET_MAX_PEERS`).
- `window_secs` (optional, default 3600): forwarded to each per-peer `getForkDiagnostic`.
- `limit` (optional, capped at 10,000): forwarded to each per-peer query.

**Returns**: `FleetBundle` — aggregates per-peer `DiagnosticBundle` results into:

| Field | Description |
|-------|-------------|
| `schema_version` | Fleet bundle format version (currently 1) |
| `query_timestamp_ms` | Wall-clock ms when the fleet query started |
| `queried_peers[]` | Per-peer `PeerStatus` (redacted URL, optional bundle or error, latency) |
| `fleet_summary` | Reachable peers, total fork events, majority/minority classifications |
| `fork_groups[]` | Fork events grouped by `CorrelationKey`; peers partitioned into canonical/fork/undecided |
| `divergence_table[]` | Heights where peers disagree on block hash, with recommended actions |

**Timeouts**: 5s per peer (`DOLI_FLEET_PEER_TIMEOUT_SECS`), 30s total wall-clock.

**Error codes**: `-32602` (too many peers / invalid params), `-32603` (total timeout).

**PII guard**: `peer_rpcs` URLs are redacted to `peer-N` labels in the output. No IP addresses appear in the serialized `FleetBundle`.

See [docs/rpc_reference.md](rpc_reference.md) for the full `FleetBundle` JSON example.

---

## CLI

```bash
# JSON output (default, last 1 hour)
doli forks --last 1h

# Human-readable output
doli forks --last 1h --human

# Explain the most recent fork event
doli forks --explain

# Attribution by producer
doli forks --by-producer

# Fleet-wide query (Phase 2a)
doli forks --fleet http://127.0.0.1:8501,http://127.0.0.1:8502 --human

# Offline replay from a historical log file (Phase 2a)
doli forks --replay ~/testnet/logs/n10.log --human

# Replay with JSON output to file
doli forks --replay ~/testnet/logs/n10.log --out bundle.json
```

See [docs/cli.md](cli.md) for full CLI reference.

---

## Replay Tool

The replay tool (`doli forks --replay <LOG_FILE>`) ingests a raw node log file
offline, parses diagnostic events from log lines, runs the classifier, and
outputs a `DiagnosticBundle`. No running node or RPC connection is needed.

**Output bundle shape**: identical to `getForkDiagnostic` except:
- `node_peer_id` = `"(log-replay)"`
- `health.ledger_available` = `false`
- `baseline` = zeros (no 24h history available from a log file)

**Recognized log patterns** (13 parsers in `crates/storage/src/diagnostic_ledger/log_replay/parsers.rs`):

| Parser | Log pattern | EventKind |
|--------|-------------|-----------|
| `parse_block_applied` | `[BLOCK] Applied h=... hash=... producer=...` | `BlockApplied` |
| `parse_block_rejected` | `Block rejected: ...` | `BlockRejected` |
| `parse_block_reject_structured` | `[BLOCK] REJECT slot=S h=H producer=P error=R` | `BlockRejected` |
| `parse_rollback_initiating` | `[ROLLBACK] Initiating: ...` | `RollbackStarted` |
| `parse_rolling_back_from` | `Rolling back from h=...` | `RollbackStarted` |
| `parse_reorg_complete` | `[REORG] Complete: ...` | `ReorgExecuted` |
| `parse_health` | `[HEALTH] h=... s=... hash=...` | `WriterHeartbeat` |
| `parse_stuck_sync` | `[RECOVERY] Stuck sync detected ...` | `RecoveryClassifyCall` |
| `parse_snap_attempted` | `[SNAP] Attempting snap sync ...` | `SnapSyncAttempted` |
| `parse_snap_completed` | `[SNAP] Completed ...` | `SnapSyncCompleted` |
| `parse_snap_failed` | `[SNAP] Failed ...` | `SnapSyncFailed` |
| `parse_chain_break` | `[SYNC] Chain break ...` | `ChainBreakDetected` |
| `parse_fork_guard` | `[FORK_GUARD] ...` | `ForkBlockReceived` |

**Note**: `parse_block_reject_structured` was added in M4 after the INC-I-081
fixture campaign discovered that production nodes emit `[BLOCK] REJECT slot=S h=H
producer=P error=R` (structured format) rather than the plain-text `Block rejected:`
format. Both parsers are active; the structured parser fires first in dispatch order.

---

## Empirical Schema Validation

### INC-I-083 (REQ-FORKOBS-RETRO-001) — Validated via replay fixture

**Fixture**: `crates/storage/tests/fixtures/inc_i083_replay.log` (178 lines from
real `~/testnet/logs/n10.log`, captured 2026-05-19).

**Result**: Classifier verdict **Unknown** with chain-break events as evidence.
The repeating header chain breaks during a stuck sync recovery loop have no named
`ForkType` variant — `Unknown` with `evidence_event_ids` pointing at
`ChainBreakDetected` events is the correct and actionable output.

**Coverage gap identified**: a named variant (e.g., `HeaderRecoveryStuck` or
`ChainBreakLoop`) is a Phase 2b candidate.

### INC-I-081 (REQ-FORKOBS-RETRO-002) — Validated via replay fixture

**Fixture**: `crates/storage/tests/fixtures/inc_i081_replay.log` (12 lines
synthesized to production log format).

**Result**: Classifier verdict **EpochBoundaryInvalid** (rule b, confidence 0.90).
Schema adequate — the classifier correctly identifies the broken epoch-boundary
block pattern without log grep.

**Parser gap closed**: M4 added `parse_block_reject_structured` to handle the
actual production `[BLOCK] REJECT` format discovered during fixture testing.

---

## Phase 2b — Deferred

The following capabilities are explicitly deferred to future workflows:

| Capability | Status | Notes |
|------------|--------|-------|
| `schemars` / JSON Schema export (`docs/fork_observability_schema.json`) | DEFERRED | Machine-readable schema for agent validation |
| Causality DAG / fork tree visualization | DEFERRED | Human UI; agents use the `caused_by_event_id` chain field instead |
| Dashboard / explorer integration | DEFERRED | Pending stable RPC schema |
| Fork honeypot debug mode | DEFERRED | Test-infra; separate workflow |
| Pre-fork warning stream / push alerts | DEFERRED | Different observability domain (prediction vs diagnosis) |
| Performance optimization beyond stream-parse + parallel queries | DEFERRED | Current implementation handles 1.9 GB logs and 50-peer fleets |
| authn/authz for the fleet RPC | DEFERRED | Currently operator-side only (no external exposure) |
| Named `ForkType` variant for header chain-break recovery loops | DEFERRED | Currently classifies as `Unknown` — INC-I-083 is the canonical example. Candidate names: `HeaderRecoveryStuck`, `ChainBreakLoop` |

**Note on `classification` field**: the RPC handler always populates `classification`
with at least `ForkType::Unknown` when no fork-specific rules match. The JSON
schema declares it as `Classification | null` for forward compatibility, but
the current implementation always returns a non-null value.
