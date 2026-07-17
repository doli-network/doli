<!--
OUTPUT CONTRACT: N/A — architecture document (not a test file)
INPUT PARTITIONS: N/A — architecture document (not a test file)
-->

# Architecture: Disk-Full as a Clean Condition (Option 1)

> Supersedes the "Disk Guardian" production-halt watchdog scope in
> `specs/disk-guardian-requirements.md` (REQ-DISK-001..012, never implemented — see the
> SCOPE PIVOT banner in that file). Origin: mainnet producer "nano" SIGABRT crash-loop on a
> 100%-full 38G volume (29G unrotated `/var/log/doli/mainnet.log`).
> Status: **APPROVED SCOPE — Option 1** (anti-anchoring skeptic pass + orchestrator code
> verification, 2026-07-17). Non-consensus, no activation height, rolling-deploy safe.

## SSF Design (one sentence)

**Make a full disk return an error instead of aborting the process (M1: convert the
RocksDB-write `.expect()` panics in `state_db` to propagated `Result`s), and stop the disk
from filling in the first place (M2: the systemd service installer ships a size-capped
logrotate drop-in for the log file it already redirects to).**

No watchdog, no thresholds, no production gate, no new `Node` state, no `NetworkParams`
fields, no new dependencies, no node-binary change in M2.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +small (inferred)   # ops-layer logrotate compress pass ≤1x/day; node process: 0
  Memory:   0 (observed)        # no new allocations on any success path
  IO:       +small (inferred)   # copytruncate copy of ≤200MB at rotation; node hot paths: 0
  Network:  N-A (observed)      # no wire-visible change of any kind
  Disk:     -large (inferred)   # log growth capped at ~1.2GB vs unbounded 29G+ observed at "nano"
  Latency:  0 (observed)        # success paths bit-identical; no added hot-path work
Inevitability: AVOIDABLE
Cheaper alternative: docs-only operator guidance for logrotate + leave the panics in place
Why this proposal anyway: the unmonitored operator population demonstrably installs nothing (nano ran months unrotated), and a SIGABRT core-dump crash-loop risks DB corruption that only M1's error propagation removes at the source
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Root-Cause Chain (code-verified)

1. `Cargo.toml:120` — `[profile.release] panic = "abort"`. **Every Rust panic in the release
   binary is a SIGABRT core dump.** This is the abort mechanism behind nano's signal-6
   crash-loop. `conf(0.95, observed)`.
2. `crates/storage/src/state_db/writes.rs` — direct (non-batch) StateDb write methods call
   `.expect("RocksDB write batch")` on `db.write()`: `insert_utxo` (L43), `remove_utxo`
   (L66), `import_utxos` (L322, L329), `add_transaction` (L404), plus
   `clear_and_write_genesis` (L134) and `put_undo` (`undo.rs:15`, `.expect("RocksDB put
   undo")`). On ENOSPC these panic → abort. `conf(0.95, observed)`.
   - **Correction to the pivot brief:** `StateDb::spend_transaction` (writes.rs:410) already
     returns `Result` and propagates via `?` (L452) — it is NOT a panic site. The panic in
     the rollback spend path is in its *sibling* `remove_utxo`, which the `UtxoSet` wrapper
     calls (`utxo/set.rs:180,208`).
3. `apply_block` (production AND sync) is **already fail-safe**: state persists via
   `BlockBatch` (`state_db/batch.rs` — `put_undo` at L473 is serialization-only into the
   batch) committed at `bins/node/src/node/apply_block/mod.rs:329-332` with
   `.commit().map_err(...)?`. The `UtxoSet::RocksDb` wrappers are "used only in rollback
   paths; normal block processing uses BlockBatch" (`utxo/set.rs:126-128`). **No change to
   apply_block.** `conf(0.9, observed)`.
4. `bins/cli/src/cmd_service.rs:337-338` — the CLI's own systemd installer writes
   `StandardOutput=append:/var/log/doli/{network}.log` with **no rotation of any kind**
   anywhere in the repo. This is the exact template nano-class external producers use, and
   the unbounded file is nano's actual trigger. `conf(0.95, observed)`.

## RocksDB Background-Error Finding (the "849ms open question")

**Question:** does a RocksDB *background* compaction/flush hitting ENOSPC abort the process
internally, bypassing app-level `?`? Does startup WAL recovery abort?

**Finding — NO process abort originates inside RocksDB; the abort vector is exclusively
Rust-side (`panic="abort"` + `.expect()`).** Derivation:

- `state_db/open.rs` (read in full): default `Options` — no custom `Env`, no
  `paranoid_checks` override, `set_wal_recovery_mode(PointInTime)` (L72). If WAL recovery
  or its recovery-flush hits ENOSPC, `DB::open_cf_descriptors` (L185) returns
  `Err(Status)`, which propagates: `init.rs:181` `StateDb::open(...)?` → `Node::new` →
  `run_node` → `main` → anyhow error, **exit code 1, not SIGABRT**. `conf(0.85, observed)`
  for the propagation chain.
- Runtime background flush/compaction ENOSPC: RocksDB's ErrorHandler sets a background
  error and places the DB in an errored/read-only state; **subsequent foreground writes
  return `Err(Status)`** — RocksDB release builds return `Status`, they do not `abort()`
  on I/O errors (C++ `assert` compiles out; no `std::terminate` path for ENOSPC).
  Whether RocksDB auto-resumes after space frees (Status::NoSpace soft-error recovery via
  SstFileManager) is version/config-dependent; without it a restart clears the latched
  error — and post-M1 that restart is clean, not a crash-loop. `conf(0.75, inferred)` from
  RocksDB documented error-handling semantics + the observed default options.
  **Verification hook:** gauntlet disk-full scenario must observe zero SIGABRT (below).
- **Nano's 849ms-after-start abort is therefore a Rust panic on an early-startup-reachable
  disk write**, not a RocksDB-internal abort. Panic sites reachable in the first second on
  a full disk include the migration `import_utxos` (`init.rs:233`), genesis-mismatch
  `clear_and_write_genesis` (`init.rs:369`), and the startup UTXO-rebuild / rollback writes
  that go through `UtxoSet::RocksDb` → `insert_utxo`/`remove_utxo`. The exact site is
  indeterminable without the core dump; **all candidates are inside the M1 conversion
  set.** `conf(0.7, inferred)`.

**Design consequence:** M1 needs **no RocksDB options tweak**. If the gauntlet disk-full
scenario ever observes a process abort NOT attributable to a Rust panic, that is a new
incident (follow-up: explicit `SstFileManager`/recovery tuning), not part of this scope.

## Design Decisions

### D1 — M1: Convert StateDb direct-write panics to `Result<_, StorageError>`

Change list (all in `crates/storage/`):

| # | Method | File:Line (panic) | New signature |
|---|--------|-------------------|---------------|
| 1 | `StateDb::insert_utxo` | writes.rs:43 | `Result<(), StorageError>` |
| 2 | `StateDb::remove_utxo` | writes.rs:66 | `Result<Option<UtxoEntry>, StorageError>` |
| 3 | `StateDb::import_utxos` | writes.rs:322,329 | `Result<(), StorageError>` |
| 4 | `StateDb::add_transaction` | writes.rs:404 | `Result<(), StorageError>` |
| 5 | `StateDb::clear_and_write_genesis` | writes.rs:134 | `Result<(), StorageError>` |
| 6 | `StateDb::put_undo` | undo.rs:15 | `Result<(), StorageError>` |
| 7 | `StateDb::clear_utxos` | writes.rs:92 (silent `let _ =` swallow) | `Result<(), StorageError>` |

Non-test callers to adapt (all already error-shaped — **no signature cascade**):
- `crates/storage/src/utxo/set.rs:163,180,196,208` — the four `UtxoSet::RocksDb` wrapper
  arms (`add_transaction`/`spend_transaction`/`insert`/`remove`) already return
  `Result<_, StorageError>` and today *swallow* the panic; add `?`.
- `bins/node/src/node/init.rs:233,369` — `Node::new` returns `Result`; add `?`.
- `bins/node/src/operations/chain.rs:133,138` — truncate rollback loop; enclosing fn
  returns `anyhow::Result`; add `?` (StorageError: std::error::Error via thiserror).
- Remaining call sites are tests (`state_db/tests.rs`, `utxo/tests.rs`,
  `phase3/phase5/tuning` integration tests, `inc_i_064` node test) — mechanical `.unwrap()`.

**Kept as panics (deliberate):** the bincode *serialization* `.expect()`s (writes.rs:33,
125,310,391; undo.rs:14; batch.rs:51,352,372,383,476). They serialize in-memory structs to
a `Vec` — no disk I/O, cannot ENOSPC; failure there is a programmer error for which
abort-on-panic is the correct response. Converting them would widen the diff for zero
robustness gain (Subtraction Principle: smallest change that removes the failure class).

**Success-path bit-identity (hard constraint):** each method builds the identical
`WriteBatch` in the identical order; the ONLY textual change is `.expect(...)` →
`?`/`Ok(...)`. The post-write counter updates stay after the write; on error they are now
skipped — previously the process died before reaching them, so error-path counter behavior
is equivalent. Same writes, same state root, same disk bytes on success.
`conf(0.9, observed)` — the diff is line-local per method.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)   # same WriteBatch construction; Result return is a status RocksDB already produces
  Memory:   0 (observed)   # no new allocations; StorageError only materializes where the process previously aborted
  IO:       0 (observed)   # identical write ops, identical order
  Network:  N-A (observed) # single-node storage plumbing
  Disk:     0 (observed)   # identical bytes written on success
  Latency:  0 (observed)   # no added branches in the success path beyond the pre-existing status check
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: converts fatal SIGABRT+core-dump into a clean propagated error at zero runtime cost; there is no cheaper way to remove the abort class than deleting the panics themselves
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### D2 — M2: Bound log growth via installer-shipped logrotate drop-in (Option b)

**Decision: (b) logrotate drop-in, written by the existing systemd installer**
(`bins/cli/src/cmd_service.rs::install_systemd`), **not** (a) in-node `tracing-appender`.
`conf(0.85, observed)`.

`install_systemd` additionally writes `/etc/logrotate.d/doli-{network}`:

```
/var/log/doli/{network}.log {
    maxsize 200M
    daily
    rotate 5
    copytruncate
    compress
    delaycompress
    missingok
    notifempty
}
```

Why (a) loses (each point code-grounded):
- `tracing-appender` rotates by **time, not size** — it does not bound bytes, which is the
  actual failure dimension (29G in one file).
- The real deployment model is a **systemd stdout append redirect**
  (`cmd_service.rs:337-338`; same pattern in `scripts/install-services.sh:81-173` and the
  launchd/GUI paths). An in-node file writer either **duplicates** the log (node writes
  file A, systemd still appends stdout to file B) or requires every operator to edit their
  unit to drop the redirect — a fleet-wide migration for the exact population that installs
  nothing. Incoherent with the shipped model.
- (a) adds a dependency + node-binary change + operator migration; (b) is ~30 lines in the
  installer the target population already runs, and `doli service logs`
  (`cmd_service.rs:720-728`) keeps working because `copytruncate` keeps the path stable.
  (`copytruncate` is **required**: systemd holds the append fd, so rename-based rotation
  would leave the process writing to the rotated inode forever.)

**Tradeoff + scalability ceiling (one sentence):** steady-state log usage is bounded to
≈ (rotate+1)×maxsize (~1.2G, less with compression) at the cost of accepting the standard
copytruncate race (a few lines lost at rotation instant) and a worst-case single
*inter-rotation* burst (logrotate's timer is daily on stock distros, so one pathological
>200M day can transiently exceed the cap by that day's volume — months-scale unbounded
growth, the nano mode, is eliminated).

**Scale assumptions (system-impact protocol):** maxsize=200M / rotate=5 is calibrated for
the 38G-class VPS fleet (nano's volume) from the observed rate (29G over multiple months →
well under 200M/day at info level; basis=inferred from the incident report). On larger
disks the cap is simply more conservative; there is no small-scale regime where 1.2G of
logs endangers a disk that can hold the chain. Thresholds are plain text in the drop-in —
operator-tunable without any binary change.

**Adoption for existing installs (nano-class):** re-running `sudo doli service install` is
idempotent and now also writes the drop-in; docs (M2 deliverable) give the copy-paste
drop-in for operators who won't re-run the installer. `doli service uninstall` removes it.
**Out of scope:** macOS/launchd (`newsyslog`, no external-producer population) and the
raw-script devnet/testnet paths (dev machines, wiped regularly).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +small (inferred)   # one daily/size-triggered logrotate compress pass, ops-layer cron, not in the node process
  Memory:   0 (observed)        # no node-process change
  IO:       +small (inferred)   # copytruncate copy of ≤200MB at rotation, ≤1x/day steady state
  Network:  N-A (observed)      # local file rotation only
  Disk:     -large (inferred)   # caps log growth at ~1.2GB vs unbounded 29G+ observed; net large savings
  Latency:  0 (observed)        # node hot paths untouched
Inevitability: AVOIDABLE
Cheaper alternative: docs-only guidance telling operators to configure logrotate themselves (zero repo change)
Why this proposal anyway: the target population is unmonitored operators who demonstrably do not install ops tooling (nano ran months without rotation); the installer is the only channel that reliably reaches them
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### D3 — Explicitly rejected alternatives (exploration trace, condensed)

| Alternative | Verdict |
|---|---|
| Do nothing (rely on operator monitoring) | ELIMINATED — nano proves the unmonitored tail exists; crash-loop risks DB corruption + wipe/resync. |
| Original watchdog scope (REQ-DISK-001..012: threshold poll + production halt) | SUPERSEDED — production halt does not stop the writes that actually abort (`apply_block` fires on every *received* block); it adds Node state, NetworkParams fields, and a gate in the most consensus-sensitive chain to mitigate a symptom M1 removes at the source. Skeptic verdict: complexity without covering the failure. May return later as a *complement* (operator early-warning), not a prerequisite. |
| Global `panic = "unwind"` + catch | ELIMINATED — changes crash semantics for every panic in the binary (consensus code included); masking programmer errors is worse than aborting on them. |
| RocksDB options tweak (SstFileManager / recovery mode) in M1 | REJECTED for this scope — no evidence RocksDB itself aborts (see finding); would be speculative tuning. Revisit only if the gauntlet disk-full scenario observes a non-panic abort. |
| In-node log rotation (tracing-appender) — option (a) | ELIMINATED — see D2. |

## Integration Points

| Milestone | Files touched | What changes |
|---|---|---|
| M1 | `crates/storage/src/state_db/writes.rs`, `crates/storage/src/state_db/undo.rs` | 7 method signatures → `Result`; `.expect()` → `?` |
| M1 | `crates/storage/src/utxo/set.rs` | 4 wrapper arms: add `?` (signatures unchanged) |
| M1 | `bins/node/src/node/init.rs` (L233, L369), `bins/node/src/operations/chain.rs` (L133, L138) | add `?` |
| M1 | test files listed in D1 | mechanical `.unwrap()` |
| M2 | `bins/cli/src/cmd_service.rs` | `install_systemd` writes + `uninstall` removes the logrotate drop-in |
| M2 | `docs/troubleshooting.md`, `docs/producer_node_quickstart.md` | disk-full section + adoption note for existing installs |

**NOT touched:** `apply_block`/sync/reorg internals (INC-I-139/138 churn), `production/`,
`periodic.rs`, `NetworkParams`, any consensus code, any version constant.

## INC-I-075 Three-Question Consensus-Shape Checklist (M1)

1. **Can any user-submittable transaction trigger this code path?** The changed methods run
   only on node-local events (rollback/undo replay, startup migration, genesis-mismatch
   wipe, operator truncate). A user tx cannot select these paths, and on the success path
   the code is bit-identical anyway. **NO.**
2. **Can any producer-action or attestation pattern trigger it?** Reorgs (which producers
   can influence) reach the rollback wrappers — but the wrappers' consensus-visible output
   (the resulting state) is bit-identical on success. **NO** (no consensus-visible change is
   triggerable).
3. **Is the new behavior bit-identical to the old behavior for ALL reachable inputs?**
   **YES for every input the consensus computation can observe**: identical WriteBatch
   contents and order, identical state root. The only divergent input class is an OS-level
   I/O error (ENOSPC), where old = process abort, new = propagated `Err` — an OS error is
   not a consensus input, and "abort" was never a consensus-visible output.

→ **No activation height required.** Deploy-safety Q#0b: (1) consensus rules unchanged;
(2) block CONTENT unchanged → **rolling deploy safe**, no synchronized stop. No bump of
`CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION`
(INV: a needless protocol bump forks the chain — INC-I-054).

## Failure-Mode Matrix (system-impact protocol — gauntlet.conf present)

**Headline: disk-full becomes an error the node reports, never a signal the kernel reports.**

| Mode | Today (v6.23.9) | After M1 + M2 |
|---|---|---|
| Disk full at **foreground state write** (rollback/undo/migration/truncate) | `.expect()` panic → `panic="abort"` → SIGABRT core dump → systemd crash-loop (nano) | `Err(StorageError)` propagates; operation aborts with a structured error log; no core dump; process exits non-zero (startup) or surfaces the failed operation (runtime) |
| Disk full at **apply_block** (produce or sync) | Already safe: `BlockBatch.commit()` → `?` (`apply_block/mod.rs:329-332`); block application fails cleanly, retried when space frees | Unchanged — verified, no modification |
| Disk full at **RocksDB background flush/compaction** | RocksDB latches bg error (no internal abort); DB errored/read-only; the NEXT foreground write's `Err` hits `.expect()` → SIGABRT | Same RocksDB behavior; foreground `Err` now propagates cleanly; operator frees space; restart (clean, no crash-loop) clears any latched bg error. Gauntlet must confirm zero SIGABRT |
| Disk full at **startup / WAL recovery** | `StateDb::open` Err already propagates cleanly (init.rs:181 `?` → exit 1), BUT migration (init.rs:233) / genesis-wipe (init.rs:369) writes panic → SIGABRT + crash-loop | All startup writes return `Result`; deterministic clean-error exit with actionable message; systemd restart loop is calm (10s, no core dumps eating more disk) |
| Disk full **during rollback** | Wrapper calls panicking `insert_utxo`/`remove_utxo` → SIGABRT mid-rollback | Rollback returns `Err` at the failed op; each op is its own atomic WriteBatch (fully applied or not at all); node reports rollback failure via the existing error path instead of dying mid-flight |
| **Partial write / crash mid-batch** | WriteBatch atomicity + WAL `PointInTime` recovery: batch fully applied or not at all | Unchanged — M1 does not alter batch composition or WAL settings |
| **Disk recovers** (space freed) | Crash-loop persists until operator intervenes; core dumps may consume MORE disk | Foreground writes succeed again; a latched RocksDB bg error clears on (clean) restart; no operator wipe needed |
| **Log file fills the volume** (nano's actual trigger) | `/var/log/doli/{network}.log` grows unbounded (29G observed) until 100% | logrotate caps steady state at ≈(rotate+1)×maxsize ≈ 1.2G; residual: one inter-rotation burst day (documented ceiling, D2) |

**Protection-mechanism registration (M2 logrotate cap):** trigger = log size ≥ maxsize at
the logrotate timer tick; action = copytruncate rotation, keep 5 compressed; scale
assumptions = per D2; trigger-surface interactions = none with any consensus/protection
mechanism (`v_protection_surface` — disk/log is a new surface; the only co-resident is the
`doli service logs` reader, which is path-stable under copytruncate). M1 is error-path
plumbing, not a triggered mechanism — nothing to register. **Registration in
`protection_mechanisms` + gauntlet disk-full run are close-out gates for the implementing
workflow** (this session is design-only and has no DB access).

**Gauntlet requirement before close:** a disk-full chaos scenario (small tmpfs or
quota-bound data_dir on a devnet node): fill → assert no SIGABRT, structured error logged,
node/systemd not crash-looping; free space → assert node resumes/restarts cleanly and
re-converges (state root matches fleet).

## Performance / Resource Budget

- M1: zero success-path delta (see D1 cost block). Error path allocates one `StorageError`
  string — irrelevant (process previously aborted).
- M2: zero node-process delta; ops-layer ≤1 copytruncate copy of ≤200MB/day (see D2 block).
- Budget assertion for QA: existing storage benchmarks (`testing/benchmarks`) and the
  state-root convergence suite must show no regression; no new hot-path work exists to
  measure.

## Test Strategy (TDD — test BEFORE fix)

### M1 — Fail-safe foreground writes
Output contract: every changed method × {success, disk-write-failure} × {empty/populated DB}.

1. **Reproduction test first (FAIL→PASS evidence):** Linux-only integration test
   (`crates/storage/tests/disk_full_failsafe.rs`, `#[cfg(target_os = "linux")]`, ignored
   elsewhere): open a `StateDb` on a tiny tmpfs (~8MB), write until ENOSPC, assert the
   method returns `Err(StorageError)` **and the test process observes no panic/abort**.
   Written against the CURRENT code it must FAIL (panic) — that is the reproduction.
2. **Success-path bit-identity:** existing state-root convergence + storage suites pass
   unchanged (`cargo test -p storage`, fork-recovery integration tests). No test may need
   success-path expectation changes — if one does, bit-identity is violated (STOP).
3. **Propagation tests:** rollback wrapper (`utxo/set.rs`) arms return `Err` when the
   underlying StateDb errors (tmpfs-based, same harness); `operations/chain.rs` truncate
   surfaces the error.
4. **Grep gate (regression guard):** `rg '\.expect\(|\.unwrap\(' crates/storage/src/state_db`
   returns no RocksDB *write-op* matches in non-test code (serialization expects allowlisted).

### M2 — Bound log growth
Output contract: installer output × {fresh install, re-install (idempotent), uninstall}.

1. **Unit test first:** the drop-in generator function returns byte-expected logrotate
   content for a given network (string assertion, same style as the unit-template tests).
2. **Install/uninstall behavior:** `install_systemd` writes `/etc/logrotate.d/doli-{network}`
   (path assertion via injectable root or dry-run), re-install overwrites idempotently,
   `uninstall` removes it.
3. **Ops validation (gauntlet/manual):** `logrotate -d` (dry-run) accepts the generated
   config; on the local testnet, forcing `logrotate -f` truncates a >maxsize file while the
   node keeps logging to the same path.

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|--------------|
| M1 | Fail-safe foreground writes | `storage/state_db/{writes,undo}.rs`, `storage/utxo/set.rs`, node call sites | REQ-DISK-101..106 | S | None |
| M2 | Bound log growth (installer logrotate) | `bins/cli/src/cmd_service.rs`, docs | REQ-DISK-201..204 | S | None (independent of M1) |

Per-milestone acceptance criteria live on the requirement rows in
`specs/disk-guardian-requirements.md` (Option 1 ACTIVE section). Milestone close-out for
the pipeline additionally requires: build gate (`cargo build --release && cargo clippy --
-D warnings && cargo fmt --check`), full `cargo test -p storage` (M1), and the gauntlet
disk-full scenario (workflow close gate).

## Security Model

No trust boundary is created or crossed: M1 changes error handling on the node's own
storage; M2 writes a root-owned config file from an already-root installer with a
fixed, non-user-interpolated network name (`mainnet|testnet|devnet` validated upstream by
`Network::from_str`). No external/untrusted data enters either path → no REQ-DISK-SEC-*
items (unchanged from the analyst's gate evaluation).

## Architecture Constraint Table (institutional memory, basis=documented incidents)

| Constraint (source) | How this design complies |
|---|---|
| INC-I-054 — needless protocol-version bump forks the chain | No version constants touched (hard invariant above) |
| INC-I-062 / MEMORY #0b — block-content change needs synchronized deploy | Block content untouched → rolling deploy |
| INC-I-075 — consensus-shape checklist before merge | Answered in-spec (all NO/YES as required) |
| INC-I-111 / MEMORY feedback — model hardware impact before proposing | Resource cost blocks per decision; zero success-path delta |
| INC-I-138/139 — apply/sync churn, additive-isolation | apply/sync/reorg logic untouched; changes are storage-plumbing + installer |
| MEMORY #0 — no genesis reset | Nothing here touches state encoding or activation |
| Modular size budget | All touched files remain within existing sizes; no new modules |

## Design Decision Quality Audit

```
━━━ DESIGN DECISION QUALITY AUDIT ━━━
Major decisions identified:            3 (D1 scope of Result conversion, D2 log bounding layer, RocksDB-tweak-or-not)
Alternatives per decision (avg):       3.7
  basis=measured:                      0
  basis=observed:                      7 (code reads: writes.rs, undo.rs, batch usage, open.rs, set.rs, cmd_service.rs, Cargo.toml)
  basis=inferred:                      3 (RocksDB bg-error semantics, nano panic site, log-rate sizing)
  basis=assumed:                       0
Confidence range for winner:           0.75–0.95
Decisions with flat distribution:      0
Decisions with conf >= 0.8 + assumed:  0
Constraint table entries used:         7
━━━ SIMPLICITY AUDIT ━━━
Subtraction alternatives explored:     2 (do-nothing; supersede/delete the watchdog scope — chosen)
"Do nothing" alternatives explored:    1
Winner complexity cost:                0 new modules, 0 new dependencies, 0 new Node state (M1 = signature changes; M2 = ~30 installer lines + 1 config artifact)
Simpler alternative that was close:    none (docs-only M2 rejected with evidence: nano population installs nothing)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Traceability

| Requirement | Architecture Section | Module |
|---|---|---|
| REQ-DISK-101..104 | D1, INC-I-075 checklist, Test Strategy M1 | `state_db/{writes,undo}.rs`, `utxo/set.rs`, call sites |
| REQ-DISK-105 | D1 (#7 clear_utxos) | `state_db/writes.rs` |
| REQ-DISK-106 | RocksDB finding + Failure-Mode Matrix (startup row) | `init.rs` error message |
| REQ-DISK-201..203 | D2, Test Strategy M2 | `bins/cli/src/cmd_service.rs` |
| REQ-DISK-204 | D2 adoption note | `docs/troubleshooting.md`, `docs/producer_node_quickstart.md` |
| REQ-DISK-205 (Won't) | D2 rejection of option (a) | N/A |
| REQ-DISK-001..012 (superseded) | D3 row 2 | N/A — never implemented |
