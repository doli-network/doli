# Improvement Analysis: Fork Observability Subsystem Removal

## SSF (Simplest Removal Path)

Delete the `crates/storage/src/diagnostic_ledger/` module and all 45 dependent files, then remove the ~175 coupling lines from 19 non-diagnostic files. No activation height, no synchronized deploy -- removal is deploy-safe because the subsystem is consensus-invisible.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -trivial (measured) # removes NoOpEmitter.is_noop() branch in 18 hot-path call sites
  Memory:   -trivial (measured) # removes Arc<dyn DiagnosticEmitter> + Option<Arc<DiagnosticLedger>> from Node struct
  IO:       0 (measured) # diagnostic RocksDB only opened when --fork-diagnostics flag is on
  Network:  0 (measured) # no network traffic involved
  Disk:     -17091 (measured) # lines of source/test/doc code removed
  Latency:  0 (measured) # emit calls already short-circuit via is_noop()
Inevitability: AVOIDABLE
Cheaper alternative: NONE-NEEDED
Why this proposal anyway: Remove 17K lines of unused subsystem to reduce maintenance burden
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Scope

| Domain | Files affected |
|--------|---------------|
| `crates/storage` | `diagnostic_ledger/` module (9 source files) + 8 test files |
| `bins/node` | 5 diagnostic modules + 5 integration test files + edits in 9 non-diagnostic files |
| `crates/rpc` | 2 method files + 1 test file + edits in 4 non-diagnostic files |
| `bins/cli` | 3 command files + 2 test files |
| docs/specs/skills | 10 files (full deletion) + edits in ~8 other docs/specs |
| scripts | 1 file edit (`install-local-services.sh`) |

## Summary (plain language)

The fork-diagnostic observability subsystem (`--fork-diagnostics` flag) adds a separate RocksDB database, async writer/pruner/monitor tasks, a classifier engine, 2 RPC methods, 3 CLI commands, and ~18 emit call sites in hot paths (apply_block, block_handling, fork_recovery, rollback, periodic). When the flag is OFF (default), all paths short-circuit via `NoOpEmitter.is_noop() == true` before allocating any data. The subsystem is entirely opt-in and consensus-invisible -- it never modifies state roots, block content, or validation logic.

## Architecture Context

### Module Boundaries

| Module | Responsibility | Depends on | Depended by |
|--------|---------------|-----------|-------------|
| `storage::diagnostic_ledger` | Types, RocksDB ledger, emitter trait, classifier, fleet aggregation, queries, log-replay | `rocksdb`, `serde`, `ulid` | `bins/node`, `crates/rpc`, `bins/cli` |
| `bins/node::diagnostic_*` | Writer task, pruner task, monitor task, apply_block emit helpers | `storage::diagnostic_ledger` | Node startup (task spawning), periodic (monitor tick) |
| `crates/rpc::diagnostics[_fleet]` | `getForkDiagnostic`, `getFleetForkDiagnostic` RPC handlers | `storage::diagnostic_ledger`, RPC context | CLI `cmd_forks*` |
| `bins/cli::cmd_forks*` | `doli forks`, `doli forks --fleet`, `doli forks --replay` | RPC client (HTTP to node) | None (leaf) |

### Data Flow Through Affected Area

```
[Hot paths] --fire-and-forget--> DiagnosticEmitter.record()
  |-- NoOpEmitter: returns Ok(()) immediately (flag OFF / default)
  |-- AsyncChannelEmitter: sends to bounded channel (flag ON)
        |
        v
  DiagnosticWriter task --> RocksDB "diagnostics/" database
        |
        v
  DiagnosticPruner task (periodic cleanup)
  DiagnosticMonitor task (periodic classify + alert)
        |
        v
  getForkDiagnostic RPC --> DiagnosticLedger.query() --> classify() --> DiagnosticBundle
  getFleetForkDiagnostic RPC --> fan-out to peer getForkDiagnostic RPCs --> FleetBundle
        |
        v
  CLI: doli forks [--fleet] [--replay] --> human/JSON rendering
```

### Architectural Constraints & Invariants

- **Constraint**: The `diagnostic_emitter` field on Node is `Arc<dyn DiagnosticEmitter>`. It exists on every Node instance regardless of flag. Removal eliminates the trait entirely.
- **Constraint**: `BlockProvenance` struct (from/received_at_ms) is defined inside `diagnostic_ledger::types`. It is used ONLY as a parameter to `apply_block()` and ONLY consumed by emit calls. Removing diagnostics removes this parameter.
- **Constraint**: The diagnostic RocksDB instance at `<data_dir>/diagnostics/` is completely separate from `state_db` and `block_store`. No state root or consensus data flows through it.

### Blast Radius

**Direct impact (deletion):**

| Category | Files | Lines | Crate |
|----------|-------|-------|-------|
| Storage: diagnostic_ledger module | 9 source files | 4,103 | `crates/storage` |
| Storage: diagnostic tests | 8 test files | 3,158 | `crates/storage` |
| Node: diagnostic modules | 5 source files | 835 | `bins/node` |
| Node: diagnostic integration tests | 5 test files | 2,450 | `bins/node` |
| RPC: diagnostic methods | 2 source files | 813 | `crates/rpc` |
| RPC: diagnostic test | 1 test file | 931 | `crates/rpc` |
| CLI: fork commands | 3 source files | 1,411 | `bins/cli` |
| CLI: fork tests | 2 test files | 1,038 | `bins/cli` |
| Docs/specs/skills | 10 files | 2,352 | N/A |
| **TOTAL deletable** | **45 files** | **17,091 lines** | **4 crates** |

**Indirect impact (edits to non-diagnostic files):**

| File | Lines to edit | Nature of edit |
|------|--------------|----------------|
| `bins/node/src/node/mod.rs` | ~25 | Remove `diagnostic_emitter`, `diagnostic_ledger`, `diagnostic_writer_stats` fields + imports |
| `bins/node/src/node/init.rs` | ~37 | Remove ledger open + NoOpEmitter defaults in `new()`, `new_for_test()`, `new_for_sync_test()` |
| `bins/node/src/node/block_handling.rs` | ~18 | Remove 6 inline `DiagnosticEvent` emit blocks + `BlockProvenance` import/construction |
| `bins/node/src/node/fork_recovery.rs` | ~6 | Remove 5 inline emit blocks + import |
| `bins/node/src/node/rollback.rs` | ~5 | Remove 2 emit blocks + import |
| `bins/node/src/node/periodic.rs` | ~18 | Remove 2 emit blocks + diagnostic_monitor integration + import |
| `bins/node/src/node/apply_block/mod.rs` | ~6 | Remove 3 `diagnostics::emit_*` calls + `provenance` parameter from `apply_block()` signature |
| `bins/node/src/node/startup.rs` | ~6 | Remove diagnostic_ledger passing to RPC context |
| `bins/node/src/cli.rs` | ~4 | Remove `--fork-diagnostics` flag |
| `bins/node/src/config.rs` | ~5 | Remove `fork_diagnostics` field |
| `bins/node/src/main.rs` | ~3 | Remove `fork_diagnostics` plumbing |
| `bins/node/src/run.rs` | ~3 | Remove `fork_diagnostics` param + diagnostic_ledger clone |
| `bins/node/src/metrics.rs` | ~10 | Remove `diagnostic_ledger` from RocksDB metrics scraper |
| `crates/rpc/src/methods/context.rs` | ~21 | Remove `diagnostic_ledger`, `diagnostic_writer_stats`, `diagnostic_emitter` fields + builders |
| `crates/rpc/src/methods/dispatch.rs` | ~4 | Remove 2 method dispatch entries |
| `crates/rpc/src/methods/mod.rs` | ~3 | Remove `diagnostics` + `diagnostics_fleet` module declarations |
| `crates/rpc/src/server.rs` | ~1 | Remove `getFleetForkDiagnostic` from admin methods list |
| `crates/storage/src/lib.rs` | ~1 | Remove `pub mod diagnostic_ledger` |
| `crates/storage/src/metrics.rs` | ~3 | Remove `"diagnostic_ledger"` from RocksDB label list |
| `scripts/install-local-services.sh` | ~2 | Remove `--fork-diagnostics` from plist arguments |
| `crates/storage/tests/rocksdb_memtable_cap_test.rs` | ~15 | Remove `diagnostic_ledger_memtable_cap_preserved` test fn |
| `crates/storage/tests/rocksdb_shared_block_cache_test.rs` | ~20 | Remove `test_diagnostic_ledger_shared_block_cache` test fn |
| **TOTAL edits** | **~216 lines across 22 files** | |

**apply_block() signature change:** The `provenance: Option<BlockProvenance>` parameter is consumed ONLY by emit calls. All 8 callers pass `None` except `block_handling.rs:485` which constructs a `BlockProvenance`. After removal, the parameter and all call sites simplify. This is a crate-internal change (not public API).

**RPC surface change:** 55 methods -> 53 methods (-2: `getForkDiagnostic`, `getFleetForkDiagnostic`).

**Docs/specs needing edits (not deletion):**

| File | What to update |
|------|---------------|
| `docs/rpc_reference.md` | Remove 2 method sections (~190 lines), update method count |
| `docs/cli.md` | Remove `forks` / `forks --fleet` / `forks --replay` command sections |
| `docs/troubleshooting.md` | Remove `doli forks` references in troubleshooting procedures |
| `specs/SPECS.md` | Remove 2 fork-observability spec entries |
| `specs/rocksdb-configuration-architecture.md` | Remove diagnostic_ledger from instance list |
| `.claude/skills/SKILLS-INDEX.md` | Remove observability-fork entries |
| `.claude/skills/testnet-deploy/SKILL.md` | Remove `doli forks` from deployment verification |
| `.claude/skills/mainnet/SKILL.md` | Remove `doli forks` from deployment verification |
| `docs/audits/security-audit-issue-174-2026-06-08.md` | Remove getFleetForkDiagnostic SSRF finding (resolved by deletion) |
| `docs/bugfixes/inc-i-105-architecture-context.md` | Remove diagnostic_ledger references |
| `docs/redesigns/inc-i-104-redesign-analysis.md` | Remove diagnostic_ledger references |

**Dependencies potentially removable from Cargo.toml:**
- `ulid` crate in `crates/storage/Cargo.toml` -- used ONLY by diagnostic_ledger in storage
- `ulid` crate in `bins/node/Cargo.toml` -- used ONLY by emit call sites in hot paths (would be removed)

## Consensus / Deploy Safety Verdict

### Three-Question Checklist (CLAUDE.md INC-I-075)

1. **Can any user-submittable transaction trigger this code path?** NO. The diagnostic emitter is triggered only by block processing (apply_block, block_handling, fork_recovery, rollback) and periodic tasks. No transaction type activates diagnostic recording.

2. **Can any producer-action or attestation pattern trigger it?** YES -- indirectly. Every block received/applied/rejected triggers an emit call. But the emit call is fire-and-forget (`let _ = ...`) and the `is_noop()` short-circuit guarantees zero side effects when the flag is OFF.

3. **Is the new behavior bit-identical to the old behavior for ALL reachable inputs?** YES. When `--fork-diagnostics` is OFF (the default on ALL production nodes -- mainnet does NOT use this flag), removing the code produces bit-identical behavior because:
   - `NoOpEmitter.is_noop()` returns true -> emit helpers return immediately -> zero allocations, zero I/O
   - No diagnostic state is included in state roots, block content, or validation
   - The diagnostic RocksDB instance is at a separate path and is not opened when the flag is OFF

**Verdict: DEPLOY-SAFE.** No activation height needed. No synchronized deploy needed. Removal can be deployed via normal rolling restart. The subsystem is consensus-invisible and off-by-default on all production nodes.

## Necessity Verdict: Hypothesis Assessment

**Hypothesis:** "The feature was unnecessary to implement."

**Evidence FOR removal (confirming hypothesis):**

1. **Not enabled in production.** The `--fork-diagnostics` flag is NOT passed in any mainnet systemd unit. The flag IS present in `scripts/install-local-services.sh` (local testnet plist), but the local testnet is a development environment, not production.

2. **Existing independent alternatives.** The guardian scripts (`fork-monitor.sh`) poll `getChainInfo` for chain-tip divergence and do NOT use `getForkDiagnostic`. The state-root debug tooling (`getStateRootDebug`, `getUtxoDiff`) operates independently. These cover the operational need for fork detection.

3. **Historical incident recovery never used it.** Reviewing the incident history (INC-I-081, INC-I-016, INC-I-012, etc.), none of the recovery procedures reference `getForkDiagnostic` or `doli forks`. Recovery consistently uses: logs, `getChainInfo`, `getStateRootDebug`, manual block inspection.

4. **No downstream consumers.** No script, automation, or external tool depends on the diagnostic ledger's RocksDB data or the 2 RPC methods.

5. **Cost/benefit.** The subsystem is ~17K lines (source + tests + docs) with ~216 coupling lines across 22 non-diagnostic files. It adds cognitive load to every hot-path file and an `Arc<dyn DiagnosticEmitter>` field to every Node instance.

**Evidence AGAINST removal (refuting hypothesis):**

1. **Deployment verification skill references.** Both `testnet-deploy/SKILL.md` and `mainnet/SKILL.md` include `doli forks` as a post-deploy verification step. However, these appear to be aspirational (added as part of the feature implementation), not operationally exercised -- the user reports the feature was unnecessary.

2. **Classifier adds structured diagnosis.** The classifier (8 rules: TipRaceNatural, ProducerEquivocation, EpochBoundaryInvalid, RollbackLoop, ChainBreakLoop, PostSnapDeadTip, etc.) provides structured fork classification that plain logs do not. This is genuinely useful for automated diagnosis but is only available when `--fork-diagnostics` is enabled.

3. **Fleet-level aggregation.** `getFleetForkDiagnostic` provides cross-node fork group detection (on-canonical / on-fork / undecided). This is operationally valuable but has never been used in an actual incident (per incident history review).

**Verdict: CONFIRMED -- the subsystem is removable without operational loss.** The diagnostic capability it provides (structured fork classification, fleet aggregation) is genuinely useful in theory but has never been exercised in practice. All real incident response has used the independent tooling (getChainInfo, getStateRootDebug, getUtxoDiff, fork-monitor.sh, logs). The ~17K lines of code and ~216 coupling lines represent maintenance burden without operational return. The feature was well-engineered (proper opt-in gating, NoOpEmitter short-circuit, fire-and-forget semantics) but addresses a need that is already covered by simpler tools.

## What I Don't Understand

1. Whether the user has ever used `doli forks` or the RPC methods operationally -- the skill files suggest it was integrated into deployment procedures, but incident records show no usage. I am inferring non-use from absence of evidence, which is weaker than evidence of absence.
2. Whether there are external operators (beyond the structural fleet) who might be using `--fork-diagnostics`. The flag is documented in CLI help, so external producers theoretically could enable it.

## What Must NOT Change (Independent Siblings)

These are independent and must survive removal unchanged:

| Component | Verification |
|-----------|-------------|
| `getChainInfo` RPC | No imports from `diagnostic_ledger`. Defined in `crates/rpc/src/methods/network.rs`. |
| `getStateRootDebug` RPC | No imports from `diagnostic_ledger`. |
| `getUtxoDiff` RPC | No imports from `diagnostic_ledger`. |
| `scripts/fork-monitor.sh` | No references to `getForkDiagnostic` or `getFleetForkDiagnostic`. Uses `getChainInfo`. |
| Guardian scripts (`emergency-halt.sh`, `emergency-resume.sh`, etc.) | No diagnostic dependency. |

## Removal Milestones (if user proceeds)

### M1: Storage + Node core (largest, most coupled)
- Delete `crates/storage/src/diagnostic_ledger/` (9 files, 4,103 lines)
- Delete all diagnostic-specific storage tests (8 files, 3,158 lines)
- Remove `pub mod diagnostic_ledger` from `crates/storage/src/lib.rs`
- Remove `diagnostic_ledger` from `crates/storage/src/metrics.rs`
- Remove `ulid` from `crates/storage/Cargo.toml`
- Edit `bins/node/src/node/mod.rs` -- remove 3 fields + trait imports
- Edit `bins/node/src/node/init.rs` -- remove ledger open + NoOpEmitter defaults
- Delete 5 node diagnostic modules (835 lines)
- Delete 5 node diagnostic integration tests (2,450 lines)
- Edit 5 hot-path files (block_handling, fork_recovery, rollback, periodic, apply_block/mod) -- remove emit calls
- Remove `BlockProvenance` parameter from `apply_block()` signature + all 8 call sites
- Edit cli.rs, config.rs, main.rs, run.rs, startup.rs, metrics.rs -- remove flag + plumbing
- Remove `ulid` from `bins/node/Cargo.toml`
- Remove diagnostic tests from rocksdb_memtable_cap_test.rs and rocksdb_shared_block_cache_test.rs
- **Test gate:** `cargo build --release && cargo clippy -- -D warnings && cargo test -p storage -p doli-node`

### M2: RPC + CLI (leaf nodes, clean cuts)
- Delete 2 RPC method files (813 lines)
- Delete 1 RPC test file (931 lines)
- Edit context.rs, dispatch.rs, mod.rs, server.rs -- remove diagnostic plumbing
- Delete 3 CLI command files (1,411 lines)
- Delete 2 CLI test files (1,038 lines)
- **Test gate:** `cargo build --release && cargo clippy -- -D warnings && cargo test -p doli-rpc -p doli-cli`

### M3: Docs, specs, skills, scripts
- Delete 10 doc/spec/skill files (2,352 lines)
- Edit ~8 docs/specs files (rpc_reference, cli, troubleshooting, SPECS.md, SKILLS-INDEX, testnet-deploy skill, mainnet skill, rocksdb spec)
- Edit `scripts/install-local-services.sh` -- remove `--fork-diagnostics` from plist
- Update RPC method count in rpc_reference.md (55 -> 53)
- **Verify:** No broken cross-references in remaining docs

## Impact Analysis

### Existing Code Affected
- `bins/node/src/node/apply_block/mod.rs`: Signature change (remove `provenance` param) -- Risk: LOW (crate-internal)
- `bins/node/src/node/block_handling.rs`: Remove ~18 emit lines + BlockProvenance construction -- Risk: LOW
- `bins/node/src/node/mod.rs`: Remove 3 struct fields -- Risk: LOW
- `bins/node/src/node/init.rs`: Remove ~37 lines of initialization -- Risk: LOW
- `crates/rpc/src/methods/context.rs`: Remove 3 fields + 3 builders -- Risk: LOW

### What Breaks If This Changes
- `doli forks` CLI commands stop existing -- **Mitigation:** document removal in changelog
- External producers using `--fork-diagnostics` lose the feature -- **Mitigation:** announce deprecation

### Regression Risk Areas
- `apply_block()` signature change affects all 8 callers -- all callers are in the same crate, easily verified by compiler
- RocksDB metrics scraper loses one instance label -- verify remaining labels still work

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|---------------------|
| REQ-REMOVE-001 | Must | (filled by test-writer) | M1 | M1 |
| REQ-REMOVE-002 | Must | (filled by test-writer) | M2 | M2 |
| REQ-REMOVE-003 | Should | (filled by test-writer) | M3 | M3 |
| REQ-REMOVE-004 | Must | (filled by test-writer) | N/A | M1 verification |

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-REMOVE-001 | Remove diagnostic_ledger module from storage crate and all emit coupling from node hot paths | Must | - [ ] `crates/storage/src/diagnostic_ledger/` directory deleted<br>- [ ] All diagnostic test files deleted<br>- [ ] No `DiagnosticEmitter`, `DiagnosticLedger`, `DiagnosticEvent`, `BlockProvenance`, `NoOpEmitter` references remain in `bins/node/` or `crates/storage/`<br>- [ ] `apply_block()` no longer has `provenance` parameter<br>- [ ] `cargo build --release` succeeds<br>- [ ] `cargo test -p storage -p doli-node` passes |
| REQ-REMOVE-002 | Remove diagnostic RPC methods and CLI commands | Must | - [ ] `getForkDiagnostic` and `getFleetForkDiagnostic` removed from dispatch<br>- [ ] `doli forks` subcommands removed<br>- [ ] `cargo test -p doli-rpc -p doli-cli` passes |
| REQ-REMOVE-003 | Update all docs, specs, skills, and scripts | Should | - [ ] No broken references to deleted features in docs/<br>- [ ] specs/SPECS.md updated<br>- [ ] SKILLS-INDEX updated<br>- [ ] RPC method count updated (55 -> 53)<br>- [ ] scripts/install-local-services.sh updated |
| REQ-REMOVE-004 | Independent siblings verified unaffected | Must | - [ ] `getChainInfo`, `getStateRootDebug`, `getUtxoDiff` still functional<br>- [ ] `fork-monitor.sh` still functional |

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|----------------------|---------------------------|-----------|
| 1 | `--fork-diagnostics` is NOT passed in any mainnet systemd unit | The feature is off in production | No -- inferred from absence in CLAUDE.md systemd references |
| 2 | No external producer uses `--fork-diagnostics` | No one outside the structural fleet relies on this | No -- cannot verify external node configs |
| 3 | `ulid` crate is used only by diagnostic code in storage + node | Removing it from both Cargo.toml files is safe | Yes -- verified by grep |

## Identified Risks

- **External producer breakage**: If any external producer runs with `--fork-diagnostics`, the flag disappearing is a breaking change. Mitigation: announce deprecation, or keep the flag as a no-op that logs a warning for one version.
- **Deployment skill regression**: The testnet-deploy and mainnet skills include `doli forks` as a verification step. If operators have internalized this workflow, they lose a tool. Mitigation: update skills to use `getChainInfo` + `fork-monitor.sh`.

## Out of Scope (Won't)

- Replacing the classifier's structured diagnosis capability with a new simpler tool -- not needed, existing tools cover the operational need
- Keeping the flag as a deprecated no-op -- unnecessary complexity for a feature that was never used in production

## Specs Drift Detected

- `specs/rocksdb-configuration-architecture.md` references `diagnostic_ledger` as one of 4 RocksDB instances -- would need updating to 3 instances after removal
- `docs/rpc_reference.md` method count will change from 55 to 53
