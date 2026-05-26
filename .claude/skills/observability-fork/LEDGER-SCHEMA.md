# Diagnostic Ledger Schema

Source of truth: `crates/storage/src/diagnostic_ledger/types.rs`

---

## Storage layout

- **RocksDB path**: `<data_dir>/diagnostics/`
- **Column family**: `cf_events`
- **Key**: 25 bytes — `[event_kind u8][height u64 BE][ulid 16 bytes]`
- **Value**: `[0x01 format_marker][schema_version u16 LE][bincode payload]`
- **Current schema version**: `CURRENT_SCHEMA_VERSION = 1` (`types.rs:23`)
- **Compression**: Lz4 (`mod.rs:47`)

Key layout ensures prefix scan by `EventKind` byte is efficient (all events of the same kind are contiguous). Within a kind, events are ordered by height (big-endian), then by ULID (time-ordered within the same height).

---

## EventKind variants (u8 prefix bytes)

| Kind | u8 | Description |
|---|---|---|
| `BlockApplied` | 1 | Block validated and applied to chain state |
| `BlockRejected` | 2 | Block failed validation |
| `ForkBlockReceived` | 3 | Non-tip block arrived (orphan, height-occupied, reorg candidate, or rejected) |
| `RollbackStarted` | 4 | Rollback operation began |
| `RollbackCompleted` | 5 | Rollback finished |
| `ReorgExecuted` | 6 | Chain reorganization executed |
| `RecoveryClassifyCall` | 7 | Recovery state machine's classifier was invoked |
| `SnapSyncAttempted` | 8 | Snap-sync attempt started |
| `SnapSyncCompleted` | 9 | Snap-sync succeeded |
| `SnapSyncFailed` | 10 | Snap-sync failed |
| `ChainBreakDetected` | 11 | Parent-hash mismatch during header sync |
| `WriterHeartbeat` | 12 | Periodic health tick from writer task |

Fork-relevant kinds (used in `fork_summary` counts and baseline rate):
`ForkBlockReceived`, `BlockRejected`, `RollbackStarted`, `ReorgExecuted`, `RecoveryClassifyCall`, `SnapSyncFailed`

---

## DiagnosticEvent structure

```
DiagnosticEvent {
  event_id: String,                     // ULID — unique, time-ordered
  kind: EventKind,
  timestamp_ms: u64,                    // wall-clock ms since UNIX epoch
  height: Option<u64>,                  // block height (None for non-block events)
  correlation_key: Option<CorrelationKey>,  // fork episode grouping
  caused_by_event_id: Option<String>,   // causal predecessor ULID
  is_cascade_origin: bool,              // first event in a cascade
  payload: EventPayload,                // kind-specific data
}
```

---

## EventPayload variants (key fields only)

**BlockApplied**: `slot, block_hash, producer_pubkey, from_peer_id, received_at_ms, applied_at_ms, validation_duration_ms, mode, tx_count`
- `validation_duration_ms > 2000` triggers `TipRaceHighLatency` classifier rule (e)
- `validation_duration_ms < 500` enables `TipRaceNatural` classifier rule (f)

**BlockRejected**: `slot, block_hash, producer_pubkey, from_peer_id, rejection_reason, mode`
- `rejection_reason.contains("EpochReward")` at epoch boundary triggers `EpochBoundaryInvalid` rule (b)

**ForkBlockReceived**: `block_hash, block_slot, block_height_estimate, producer_pubkey, from_peer_id, classification, fork_kind, local_tip_hash, local_tip_height`
- `classification`: `"Rejected"` / `"ForkBlock"` / `"Orphan"`
- `fork_kind`: `"HeightOccupied"` / `"ReorgCandidate"` (or absent)

**RollbackStarted**: `from_height, to_height, trigger, cumulative_depth`
- > 3 in a 60s window triggers `RollbackLoop` rule (c)
- > 10 in 1h window contributes to `ChainBreakLoop` signal_c

**ReorgExecuted**: `old_tip_hash, new_tip_hash, rollback_depth, applied_count, weight_delta, trigger_block_hash, trigger_from_peer_id`

**RecoveryClassifyCall**: `local_height, network_tip_height, peer_count, last_applied_secs, shallow_rollback_count, snap_attempts, last_rollback_local_height, in_grace_period, last_finality_height, action_returned, rule_matched`
- > 20 in 1h window contributes to `ChainBreakLoop` signal_d

**SnapSyncCompleted** + **ForkBlockReceived** within 300s triggers `PostSnapDeadTip` rule (d)

**ChainBreakDetected**: `expected_prev_hash, actual_prev_hash, header_slot, valid_so_far_count, from_peer_id`
- > 3 in 1h window triggers `ChainBreakLoop` signal_a (highest priority after explicit rules)

---

## CorrelationKey

Links events in the same fork episode:
```
CorrelationKey {
  divergence_height: Option<u64>,
  canonical_hash: Option<String>,
  fork_hash: Option<String>,
}
```

Serialized as `"<divergence_height>|<canonical_hash>|<fork_hash>"` for HashMap keys in `build_fork_groups()`.

**Note**: At emission time in `block_handling.rs`, `canonical_hash` is always `None` — only `fork_hash` is set. The fleet aggregator determines canonical vs fork by comparing `BlockApplied` hashes from different peers.

---

## Classifier rule priority order

1. **(a) ProducerEquivocation** — 2x `BlockApplied` same height+producer, different hash → conf 0.95
2. **(b) EpochBoundaryInvalid** — `BlockRejected` at height % 360 == 0 with "EpochReward" → conf 0.90
3. **(c) RollbackLoop** — > 3 `RollbackStarted` in any 60s window → conf 0.85
4. **(d) PostSnapDeadTip** — `SnapSyncCompleted` then `ForkBlockReceived` within 300s → conf 0.80
5. **(h) ChainBreakLoop** — any of 4 signals in 1h window → conf 0.85 (fires BEFORE e/f — Workflow #349)
6. **(e) TipRaceHighLatency** — `ForkBlockReceived` where `BlockApplied` at same height has `validation_duration_ms > 2000` → conf 0.75
7. **(f) TipRaceNatural** — `ForkBlockReceived` with latency < 500ms, no other signals in correlation group → conf 0.70
8. **(g) Unknown** — fallback, conf 0.0, no recommended_action

`ChainBreakLoop` `recommended_action_args` includes wipe path template:
```json
{
  "approach": "stop_node + rm -rf <data_dir>/{blocks,state_db,utxo,diagnostics} + restart with --no-snap=false",
  "preserve": ["wallet.json", "producer.seed.txt"],
  "verify_after": "doli forks --explain --human after 10 minutes of sync"
}
```

---

## Pruning policy

`DiagnosticLedger::prune(retention_secs, max_events)`:
1. Age prune: remove events with `timestamp_ms < now - retention_secs * 1000`
2. Count cap: if remaining > `max_events`, evict oldest non-pinned
3. Pin protection: first event per unique `CorrelationKey` is never evicted by count-cap

Production default retention and max_events: NOT set in this codebase (caller-determined). Writer task responsible for scheduling pruning.
