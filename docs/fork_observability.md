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

**Method**: `getForkDiagnostic`

**Params**: `{ window_secs: u64, limit?: u64, fork_event_id?: string }`

**Returns**: `DiagnosticBundle` (see schema above)

**Error codes**: `-32603` when the diagnostic ledger is unavailable

See [docs/rpc_reference.md](rpc_reference.md) for full specification and examples.

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
```

See [docs/cli.md](cli.md) for full CLI reference.

---

## Retroactive Validation

### INC-I-083 (REQ-FORKOBS-RETRO-001)

The schema captures the full `RecoveryClassifyCall` event with all 12 fields
from `RecoveryContext`. INC-I-083's root cause (classify() coverage hole causing
HeaderFirstSync loop) would produce an `Unknown` classification with
`reason_unknown` pointing directly at the repeating `action_returned=HeaderFirstSync`
pattern with climbing `last_applied_secs`. Evidence event IDs link to each
`RecoveryClassifyCall` that shows the loop. Estimated agent diagnosis time: <60s.

### INC-I-081 (REQ-FORKOBS-RETRO-002)

The schema captures `BlockRejected` events with `rejection_reason`. INC-I-081's
broken epoch-boundary block (missing EpochReward) triggers classifier rule 2,
producing an `EpochBoundaryInvalid` classification with confidence ~0.85 and
`recommended_action=investigate_producer`. The classification is correct and
specific without log grep.

## Phase 2 — Not Yet Implemented

The following capabilities are EXPLICITLY DEFERRED to a separate future workflow.
Agents reading this doc should NOT expect these features in Phase 1:

| Capability | Status | Notes |
|------------|--------|-------|
| Historical-log replay tool (`doli forks replay --log <file>`) | DEFERRED | Will ingest existing 1.9 GB log files offline and emit DiagnosticBundle for retroactive analysis |
| `getFleetForkDiagnostic` cross-fleet correlation RPC | DEFERRED | Will query multiple peers and synthesize a fleet-wide view |
| `schemars` / JSON Schema export (`docs/fork_observability_schema.json`) | DEFERRED | Will publish a machine-readable schema agents can validate against |
| Fork honeypot debug mode | DEFERRED | Test-infra; separate workflow |
| Pre-fork warning stream / push alerts | DEFERRED | Different observability domain (prediction vs diagnosis) |
| Causality DAG / fork tree visualization | DEFERRED | Human UI; agents use the `caused_by_event_id` chain field instead |
| Dashboard / explorer integration | DEFERRED | Pending stable RPC schema |

**Note on `classification` field**: the RPC handler always populates `classification`
with at least `ForkType::Unknown` when no fork-specific rules match. The JSON
schema declares it as `Classification | null` for forward compatibility, but
Phase 1 always returns a non-null value.
