# Log Sufficiency Assessment: INC-I-090

**Incident**: INC-I-090 (observability gap -- fork not surfaced)
**Run**: 372
**Agent**: log-specialist (diagnostic sufficiency mode)
**Assessed**: 2026-05-26

## 1. Framework + Log Locations Detected

- **Language**: Rust
- **Logging framework**: `tracing` crate (info!, warn!, error!, debug!, trace! macros)
- **Log destinations**:
  - Textual logs: `/var/log/doli/mainnet/n3.log` + `.log.1` (file-based, rotated; per CLAUDE.md: "app logs are written to files, not journalctl")
  - Diagnostic ledger: RocksDB at `<data_dir>/diagnostics/` (per-node, `cf_events` column family)
- **RPC-exposed counters** (via `getForkDiagnostic` response `.health` field):
  - `events_written_total` -- cumulative events persisted to RocksDB
  - `events_dropped_total` -- cumulative events evicted by ring buffer overflow
  - `ledger_available` -- boolean, ledger DB opened successfully
  - `last_heartbeat_ms` -- wall-clock ms of last `WriterHeartbeat` event
- **INV-OBS-001**: "DiagnosticHealth block returned by getForkDiagnostic RPC must reflect live writer counters -- never hardcoded literals on the live production path." (Fixed in workflow #353-354 per evidence-assembly.md; previously hardcoded zeros.)

## 2. Per-Layer Coverage Assessment

### L1 (Emit) -- Did DiagnosticEmitter::record get called?

**Existing evidence channels**:
- **Textual logs**: FORK_GUARD signals are confirmed present in `/var/log/doli/mainnet/n3.log.1` (per refined prompt: "FORK_GUARD signals visible in /var/log/doli/mainnet/n3.log.1"). These textual log lines are emitted independently of the diagnostic ledger -- they are `info!` / `warn!` statements in `block_handling.rs` that fire when fork conditions are detected.
- **Diagnostic ledger**: `ForkBlockReceived` events in RocksDB at `cf_events` with kind byte `3`, queryable via `getForkDiagnostic(min_height=284670, max_height=284685)`.
- **Emit sites per skill**: `block_handling.rs:168-190` (fire-and-forget `let _ = self.diagnostic_emitter.record(...)`) for every non-tip gossip block; `block_handling.rs:195-258` for `HeightOccupied` fork classification.

**What the channel CAN distinguish**:
- Textual logs (FORK_GUARD lines) prove the code path that would call `record()` was entered -- the fork was detected by the node's block classification logic.
- If `getForkDiagnostic` for height range [284670, 284685] returns `ForkBlockReceived` events, L1 is confirmed working.
- If no events exist in that range but FORK_GUARD lines exist in the text log, L1 emit was likely called but L2 persist failed (or emit site is after the textual log but before the `record()` call).

**What the channel CANNOT distinguish**:
- The fire-and-forget pattern (`let _ = ...`) means the `record()` call's return value is discarded. There is no independent textual log that says "diagnostic event emitted successfully" or "diagnostic emit failed". However, the presence of FORK_GUARD textual log lines at the SAME code site establishes that the code path was reached, and the RPC health counters (`events_written_total`) confirm whether events arrived in the ledger.

**Verdict for L1**: SUFFICIENT. FORK_GUARD textual logs prove code path entry. `getForkDiagnostic` query proves event arrival (or non-arrival). The combination resolves H1.

### L2 (Persist) -- Did events survive the ring buffer to RocksDB?

**Existing evidence channels**:
- **RPC health field**: `events_dropped_total` -- if nonzero, ring buffer overflow occurred (H2).
- **RPC health field**: `events_written_total` -- total events that reached RocksDB.
- **RPC health field**: `ledger_available` -- confirms RocksDB diagnostics DB is open.
- **RPC health field**: `last_heartbeat_ms` -- confirms writer task is alive (WriterHeartbeat events).
- **Direct ledger query**: `getForkDiagnostic(min_height=284670, max_height=284685)` returns actual events from RocksDB.

**What the channel CAN distinguish**:
- `events_dropped_total > 0` proves overflow occurred at some point (though not necessarily during the incident window).
- `events_written_total` combined with ledger query count confirms persistence.
- `ledger_available = false` would prove H7-adjacent failure (subsystem not initialized).
- `last_heartbeat_ms` proves writer task liveness.

**What the channel CANNOT distinguish**:
- `events_dropped_total` is cumulative since node start, not per-window. Cannot pinpoint WHEN drops occurred. However, for mainnet steady-state (one event per ~10s), overflow is architecturally unlikely unless the writer task is wedged, which `last_heartbeat_ms` would reveal.

**Verdict for L2**: SUFFICIENT. The four health counters collectively resolve H2. A single `getForkDiagnostic` RPC call provides all needed evidence.

### L3 (Classify) -- Does the classifier recognize this fork shape?

**Existing evidence channels**:
- **Code is the evidence**: `classifier.rs` is a pure function with 8 rules (per LEDGER-SCHEMA.md). Its inputs are the `Vec<DiagnosticEvent>` from L2 and its outputs are `Classification{fork_type, confidence, recommended_action}`.
- **RPC response**: `getForkDiagnostic` includes `classification` field with the computed result. Calling the RPC for the incident window directly shows what the classifier produces for THIS event set.
- **No separate logs needed**: Because classify() is pure (no I/O, no system clock per CONSTRAINT in skill), the investigator can reconstruct its behavior from inputs (the events in the ledger) + code (the 8 rules). No runtime introspection is needed.

**What the channel CAN distinguish**:
- Reading `classification.fork_type` and `classification.recommended_action` from the RPC response tells exactly which rule matched and what action was recommended.
- Comparing the actual fork shape (1-block self-produced minority, snap-sync resolved) against each rule's preconditions determines whether the classification is correct or a gap.
- Rule priority order is documented (LEDGER-SCHEMA.md lines 105-114). The investigator can trace first-match-wins logic.

**What the channel CANNOT distinguish**:
- Nothing. For a pure function, inputs + code = full determinism. There is no hidden state.

**Verdict for L3**: SUFFICIENT. RPC query returns the live classification. Code review of the 8 rules determines correctness. No logging needed for a pure function.

### L4 (Surface) -- Was there an active consumer?

**Existing evidence channels**:
- **Filesystem**: `scripts/fork-monitor.sh` exists in the repo. Whether it runs on mainnet is a systemd/cron question, not a logging question.
- **systemd journal**: `journalctl --list-units | grep fork` or `systemctl list-unit-files | grep fork` on ai1 would show whether a service exists.
- **cron**: `crontab -l` on ai1 would show whether fork-monitor runs periodically.
- **Process table**: `pgrep -f fork-monitor` (if the investigation catches it running).
- **Script itself**: `fork-monitor.sh` polls `getChainInfo` (not `getForkDiagnostic`), groups by `bestHash`, exits 0/1/2. It has no persistent output log of its own invocations unless output is redirected by a systemd unit or cron.
- **Analyst assessment**: "The skill does NOT document a systemd unit, cron job, or container that runs fork-monitor.sh on mainnet" (analyst report line 48).

**What the channel CAN distinguish**:
- systemd unit existence/absence proves or refutes H4 (deployment).
- If a unit exists, its `OnUnitActiveSec` or timer interval determines cadence.
- Absence of a unit file is definitive evidence of H4.

**What the channel CANNOT distinguish**:
- If fork-monitor.sh was run manually (ad-hoc) during the incident window, there may be no record unless the operator's shell history is checked. However, the investigation question is about automated detection, and the systemd/cron check resolves that.

**Verdict for L4**: SUFFICIENT. systemd/cron inventory on ai1 is definitive for H4. No logging is needed -- the question is "does the service exist," not "did it produce the right output."

### L0 (Version Skew) -- Does the deployed binary contain the observability subsystem?

**Existing evidence channels**:
- **Binary introspection**: `strings /mainnet/bin/doli-node-n3 | grep -i diagnostic` or `grep -i getForkDiagnostic` would confirm compiled-in symbols.
- **RPC probe**: Calling `getForkDiagnostic` on N3 RPC 8503. If the method exists and returns a response (even empty), the subsystem is compiled in. If it returns "method not found", the binary predates the feature.
- **`getNodeInfo` / `--version`**: Per RPC reference, `getNodeInfo` returns build metadata. Whether it includes a commit hash or build date depends on the binary's compilation flags.
- **Deployment history**: `stat /mainnet/bin/doli-node-n3` shows file modification time, which correlates with deployment date.

**What the channel CAN distinguish**:
- A single `getForkDiagnostic` RPC call to port 8503 definitively resolves H7. Method exists = feature compiled in. Method not found = version skew confirmed.
- Binary file date vs feature merge date provides corroborating evidence.

**What the channel CANNOT distinguish**:
- Nothing material. The RPC probe is definitive.

**Verdict for L0**: SUFFICIENT. One RPC call resolves H7.

## 3. Cross-Layer Assessment

### Q1: Can investigators distinguish "emit was called but write failed" from "emit was never called"?

**YES.** The resolving signals are:

1. **FORK_GUARD textual log lines in `/var/log/doli/mainnet/n3.log.1`** -- these are emitted at the same code site as `DiagnosticEmitter::record()` calls in `block_handling.rs`. Their presence proves the code path was entered (= emit was called). Their absence would prove emit was never called.

2. **`getForkDiagnostic(min_height=284670, max_height=284685)` on N3 RPC 8503** -- presence of `ForkBlockReceived` events in the response proves write succeeded. Absence, combined with FORK_GUARD lines in the text log, proves emit was called but write failed.

3. **`health.events_dropped_total`** in the same RPC response -- if nonzero, confirms ring buffer overflow as the write-failure mechanism.

The combination of (1) + (2) + (3) cleanly separates "never called" from "called but lost."

### Q2: Can investigators distinguish "fork-monitor not deployed" from "fork-monitor deployed but cadence too slow"?

**YES.** The resolving signals are:

1. **`systemctl list-unit-files | grep -i fork`** on ai1 -- presence/absence of a systemd unit is definitive for deployment.

2. **If a unit exists**: `systemctl cat <unit-name>` reveals `OnUnitActiveSec` or `ExecStart` with `--loop` interval, resolving cadence.

3. **If no unit exists**: `crontab -l` on ai1 checks the alternative scheduling mechanism.

4. **If neither exists**: fork-monitor.sh is not deployed, and H4 is confirmed as a categorical gap (script exists in repo but never runs).

No logging is needed for this -- it is an infrastructure inventory question.

## 4. Log Sufficiency Verdict

```
━━━ LOG SUFFICIENCY VERDICT ━━━
Verdict: SUFFICIENT

Existing instrumentation across four evidence channels covers the entire
L0-L4 investigation surface. No logging changes are needed for this
investigation. The load-bearing evidence is:

  L0 (Version skew): Single `getForkDiagnostic` RPC call to N3:8503.
     Method exists = feature compiled in. Method not found = H7 confirmed.

  L1 (Emit): FORK_GUARD textual log lines in /var/log/doli/mainnet/n3.log.1
     prove code path entry. getForkDiagnostic(min_height=284670, max_height=
     284685) proves event arrival in ledger.

  L2 (Persist): health.events_written_total + health.events_dropped_total +
     health.ledger_available + health.last_heartbeat_ms in getForkDiagnostic
     response collectively prove or refute ring buffer overflow (H2).

  L3 (Classify): classification.fork_type + classification.recommended_action
     in getForkDiagnostic response show exact rule match. classifier.rs is a
     pure function -- inputs + code = full determinism. No logs needed.

  L4 (Surface): systemctl list-unit-files + crontab -l on ai1 definitively
     resolve fork-monitor deployment (H4). No application logs needed --
     this is an infrastructure inventory question.

  Cross-layer: FORK_GUARD textual logs (independent of ledger) + ledger
     query results + health counters distinguish all three failure modes:
     "never emitted" vs "emitted but dropped" vs "persisted but not surfaced."
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
