# Mission: Phase 2a — Fleet Diagnostic RPC + Historical-Log Replay

> Fresh session. This brief is self-contained. Codebase is at
> `/Users/isudoajl/ownCloud/Projects/doli-network/doli`. Testnet is LOCAL
> (`~/testnet/`, launchd, 127.0.0.1 — NEVER ssh ai1–ai5). Read `CLAUDE.md` and
> `MEMORY.md` first. Phase 1 is on branch `feature/fork-observability-346`
> (8 commits, ~2500 LoC, 120+ tests). Phase 2a builds ON TOP of that branch.

---

## What Phase 1 Already Delivered (do NOT re-build)

Branch `feature/fork-observability-346` contains:

| Layer | What exists | Files |
|-------|-------------|-------|
| **Types** | `DiagnosticEvent`, 12 `EventKind` variants, `EventPayload` enum, `ForkType` (9 variants incl. `Unknown { reason_unknown, evidence_event_ids }`), `Classification`, `DiagnosticBundle`, `ForkSummary`, `BlockProvenance`, `CorrelationKey`, format-marker byte + schema_version | `crates/storage/src/diagnostic_ledger/types.rs` |
| **Ledger** | Separate RocksDB at `<data_dir>/diagnostics/` + `cf_events` CF + composite key `[kind][height][ulid]` + bounded retention via periodic pruner with **cascade-origin pin** | `crates/storage/src/diagnostic_ledger/{mod,queries}.rs` |
| **Emitter** | `trait DiagnosticEmitter` + `AsyncChannelEmitter` (bounded 1024, drop-oldest, dropped_count exposed) + `NoOpEmitter` + `MockEmitter` + writer task + pruner task | `crates/storage/src/diagnostic_ledger/emitter.rs`, `bins/node/src/node/diagnostic_writer.rs`, `bins/node/src/node/diagnostics_pruner.rs` |
| **Emit sites** | Production-wired emit calls at: `apply_block` (success/failure), `classify_gossip_block` (4 dispatch arms: Rejected, HeightOccupied, Orphan, ReorgCandidate), `execute_reorg`, `rollback_one_block`, `periodic.classify_and_dispatch` (with full 11-field `RecoveryContext`) | `bins/node/src/node/{apply_block/, block_handling.rs, fork_recovery.rs, rollback.rs, periodic.rs}` |
| **apply_block signature** | `apply_block(&mut self, block, mode, provenance: Option<BlockProvenance>)` — explicit provenance threading at 10 call sites (1 `Some`, 9 `None`) | `bins/node/src/node/apply_block/mod.rs` |
| **Classifier** | Pure `classify(events) -> Classification` with 7 rules in first-match-wins order: ProducerEquivocation, EpochBoundaryInvalid, RollbackLoop (>3 rollbacks in 60s), PostSnapDeadTip (within 300s), TipRaceHighLatency (>2000ms), TipRaceNatural (<500ms + no other signals in correlation_key group), Unknown(with evidence) | `crates/storage/src/diagnostic_ledger/classifier.rs` |
| **RPC** | `getForkDiagnostic(window_secs?, limit?, fork_event_id?)` returning `DiagnosticBundle` JSON. Limit clamped at 10_000. Read-only. -32603 when ledger unavailable. | `crates/rpc/src/methods/diagnostics.rs` |
| **CLI** | `doli forks` with flags `--last <1h\|30m\|24h>`, `--human`, `--explain`, `--by-producer`, `--rpc <url>`. JSON default, `--human` renders 4 sections (Health, Events, Classification, Baseline) | `bins/cli/src/cmd_forks.rs` |
| **Docs** | Agent-facing schema doc (`docs/fork_observability.md`), troubleshooting workflow (`docs/troubleshooting.md` §6b), RPC reference entry (`docs/rpc_reference.md`), requirements + architecture in `specs/` | (see paths) |

**~120 tests across 6 files. All gates clean (build, clippy, fmt, test, regression).**

---

## The Two Gaps Phase 1 Left Open

### GAP 1 — Cross-fleet diagnosis is still N×local

Phase 1 answers "what happened on THIS node." For fleet incidents (INC-I-083: 5 nodes frozen, 13 healthy), an operator/agent still walks the fleet one RPC at a time, manually correlates, and decides who's on which side of the fork. The 5-second-verdict promise is a *per-node* claim, not a fleet claim.

### GAP 2 — Schema-adequacy is paper-only

The architect's spec includes an "INC-I-083 Schema Adequacy" section (paper exercise: "if the emitter HAD been running, would the classifier output a correct verdict?"). Phase 1 has NO tool that proves this empirically. The replay tool — `doli forks replay --log <file>` — was deferred. Until it ships, the schema's adequacy claim is unverified against real captured log data.

---

## Phase 2a Mission (this workflow)

Close exactly these two gaps. Do not scope-creep into Phase 2b items (fork tree viz, dashboard integration, schemars/JsonSchema export, fork honeypot, pre-fork warnings — all explicitly deferred).

### Deliverable 1 — `getFleetForkDiagnostic` RPC + CLI

**RPC method** `getFleetForkDiagnostic(peer_rpcs: Vec<String>, window_secs: u64, limit: u64)` returns a `FleetBundle`:

```rust
pub struct FleetBundle {
    pub schema_version: u16,        // = 1
    pub query_timestamp_ms: u64,
    pub queried_peers: Vec<PeerStatus>,        // success/failure per peer
    pub fleet_summary: FleetSummary,
    pub fork_groups: Vec<ForkGroup>,           // grouped by correlation_key
    pub divergence_table: Vec<DivergencePoint>,
}

pub struct PeerStatus {
    pub rpc_url: String,
    pub node_peer_id: Option<String>,           // None if unreachable
    pub bundle: Option<DiagnosticBundle>,       // None if RPC failed
    pub error: Option<String>,                  // "timeout", "method-not-found", etc.
    pub query_latency_ms: u64,
}

pub struct FleetSummary {
    pub total_peers_queried: u32,
    pub peers_reachable: u32,
    pub peers_with_diagnostics: u32,            // bundle.diagnostic_ledger != null
    pub total_fork_events_across_fleet: u64,
    pub unique_fork_groups: u32,
    pub majority_classification: Option<ForkType>,
    pub minority_classifications: Vec<(ForkType, u32)>,
}

pub struct ForkGroup {
    pub correlation_key: CorrelationKey,        // (div_height, canonical_hash, fork_hash)
    pub peers_on_canonical: Vec<String>,        // PeerIds
    pub peers_on_fork: Vec<String>,
    pub peers_undecided: Vec<String>,           // saw the event but no classification
    pub representative_classification: Classification,
}

pub struct DivergencePoint {
    pub height: u64,
    pub competing_hashes: Vec<(String, Vec<String>)>,  // (hash, peers_endorsing)
    pub first_seen_ms: u64,
    pub recommended_action: String,             // "manual_intervention", "auto_recover", "watch"
}
```

**CLI extension**: `doli forks --fleet <peer1,peer2,peer3,...>` (or read seeds from a config file).
- Default JSON output.
- `--human` renders a fleet topology summary: which peers are on which fork, who's healthy, who's silent.

**Concurrency**: query all peers in parallel via `tokio::spawn` + `futures::future::join_all`. Per-peer timeout: 5 seconds (configurable). Total request capped at 30 seconds.

**Trust model**: the fleet RPC is invoked from the operator's host, NOT from a peer node calling another peer node. Phase 2a does NOT introduce P2P-layer authority — it's an operator-side aggregator. Existing per-node RPC auth (if any) is honored on each leg.

### Deliverable 2 — Historical-log replay tool

**CLI**: `doli forks replay --log <file> [--out <bundle.json>] [--human]`.

Reads a free-text log file (typically `~/testnet/logs/n10.log` or a mainnet log) line-by-line, parses known log patterns into synthetic `DiagnosticEvent`s, runs the M3 classifier over them, and emits a `DiagnosticBundle`.

**Log patterns to parse** (extend as needed; the architect's spec lists the primary ones):

| Log pattern | Maps to |
|-------------|---------|
| `[INFO] Block applied: height=N slot=S hash=H producer=P` (or whatever the existing format is — grep `bins/node/src/node/apply_block` for the exact `info!()` calls) | `EventKind::BlockApplied` with the parsed fields |
| `[WARN] Block rejected: ...` | `EventKind::BlockRejected` |
| `[FORK_GUARD]` line at `block_handling.rs:184` | `EventKind::ForkBlockReceived` (classification=ForkBlock, fork_kind=HeightOccupied) |
| `chain break valid_so_far=N` (from sync_engine/response.rs) | `EventKind::ChainBreakDetected` |
| `[HEALTH]` lines from periodic.rs:898 | `EventKind::RecoveryClassifyCall` (best-effort — older logs don't have the full 11 RecoveryContext fields, so populate what's available and leave `rule_matched: "log-replay-incomplete"`) |
| `Rollback from h=X to h=Y` | `EventKind::RollbackStarted` + `RollbackCompleted` pair |
| `Reorg complete: now at height H` (from `execute_reorg` log line) | `EventKind::ReorgExecuted` (best-effort: trigger_block_hash and weight_delta may be missing) |
| `Snap sync attempted/completed/failed` | `EventKind::SnapSyncAttempted` / `Completed` / `Failed` |

The parser is **best-effort**. Old logs are missing fields the M1 schema mandates. For each missing field, default to a sentinel (`from_peer_id: None`, `validation_duration_ms: 0`, `correlation_key: CorrelationKey::default()`). Mark the resulting `DiagnosticEvent` with a `replayed_from_log: true` flag (you may need to add this as an additive field to `DiagnosticEvent`; if so, do it in this workflow and bump nothing — the field is `#[serde(default)]`).

**Stretch goal**: when consecutive `ForkBlockReceived` events share a height and the canonical chain diverged afterward, REGEX-tie them together by populating their `correlation_key` with the inferred `divergence_height`.

### Deliverable 3 — INC-I-083 + INC-I-081 replay fixtures + tests

The replay tool is useless if it doesn't actually diagnose past incidents. Two fixtures, two tests:

1. **`crates/storage/tests/fixtures/inc-i-083-n10.log`** — a captured snippet (~500-2000 lines) of the actual `~/testnet/logs/n10.log` from the 2026-05-19 incident, anonymized if needed (PeerIds are public; no IPs). Snippet must include: the post-deploy startup, the snap-sync attempt, the chain-break detection, the repeating HeaderFirstSync recovery loop, the final stall.

2. **`crates/storage/tests/fixtures/inc-i-081-broken-producer.log`** — a captured snippet from the 2026-05-18 incident showing a BlockRejected at an epoch boundary with the "missing EpochReward" rejection_reason.

3. **`bins/cli/tests/fork_replay_e2e_test.rs`** with two tests:
   - `test_inc_i_083_replay_produces_actionable_verdict` — pipe the n10 fixture through `doli forks replay`, assert the bundle's classification is either `Unknown` with `reason_unknown` referencing the recovery loop OR a specific variant; assert `evidence_event_ids` contain the chain-break + recovery_classify_call events; assert the classification points an agent at `recovery.rs` (e.g., via recommended_action="investigate_recovery_classify" or via the evidence event_kind distribution).
   - `test_inc_i_081_replay_produces_epoch_boundary_invalid` — pipe the broken-producer fixture, assert classification is `EpochBoundaryInvalid` with `recommended_action="investigate_producer"`.

**If either fixture produces an incorrect verdict, the schema is wrong.** This is the empirical replacement for Phase 1's paper "schema adequacy" sections.

---

## Hard Constraints (non-negotiable)

Same as Phase 1 — verify before any code:

1. **No consensus impact.** Pure observability layer. Replay tool is offline (reads logs, runs classifier in-process). Fleet RPC is operator-side aggregation over existing per-node RPCs.
2. **Safe for rolling deploy.** TWO deploy-safety questions (MEMORY.md #0): Q1=NO (no user-submittable tx triggers replay or fleet RPC), Q2=NO (no producer/attestation action triggers them). No activation height. No HardForkSchedule entry. No protocol version bump.
3. **No mainnet contamination.** Testnet first.
4. **No new consensus-touching files.** Replay tool lives in `bins/cli/src/cmd_forks_replay.rs` (or split from cmd_forks.rs). Fleet RPC lives in `crates/rpc/src/methods/diagnostics_fleet.rs`. Neither writes to state_db, block_store, or any consensus path.
5. **No PII.** PeerIds OK. NO IP addresses in any event or bundle field. The fleet RPC takes RPC URLs as input (URLs may include IPs in localhost form like `http://127.0.0.1:8500`) — but DO NOT echo those URLs into the persisted `FleetBundle`. Either redact them in the bundle OR keep them only in the response transient (not stored).
6. **Bounded inputs.** Fleet RPC: cap peer_rpcs at 50 entries (configurable env: `DOLI_FLEET_MAX_PEERS`). Replay: cap log file size at 5 GB; stream-parse line-by-line (do NOT load entire file into memory).
7. **Test discipline.** Every new RPC unit-tested. Replay tool: 2 fixture-replay tests + 5 unit tests for the log-pattern parser (one per pattern). `cargo test -p storage -p doli-node -p rpc -p doli-cli` clean. `cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo fmt --check` clean.
8. **Documentation alignment.** Update `docs/rpc_reference.md` for `getFleetForkDiagnostic`. Update `docs/fork_observability.md` — move `getFleetForkDiagnostic` from "Phase 2 — Not Yet Implemented" to the live RPC table; update the Replay section. Update `docs/troubleshooting.md` §6b — add the replay workflow as Step 4. Update `bins/cli/src/cmd_forks.rs` and the CLI reference for the new `--fleet` and `replay` subcommands.
9. **`/sync-docs` at the end.** Commit message includes the three-question consensus-shape checklist with NO/NO/YES.
10. **No genesis reset.** Per CLAUDE.md #0.
11. **DO NOT MODIFY** decision logic in: `crates/core/src/consensus.rs`, `crates/core/src/network_params/defaults.rs` activation heights, `bins/node/src/node/apply_block/*` (Phase 1's emit calls are fixed), `crates/storage/src/snapshot.rs`, `crates/core/src/validation/*`. You may READ them. You MUST NOT modify M1-M4 production emit sites — Phase 2a is additive only.
12. **Modular.** No source file >500 lines (800 for tests).

---

## Architectural Decisions Carried Over From Phase 1 (still in effect)

| ID | Decision | Implication for Phase 2a |
|----|----------|-------------------------|
| O1 | Async-default emitter, bounded 1024, drop-oldest, dropped_count exposed | Fleet RPC and replay tool DO NOT emit new events into the live ledger — they consume/aggregate existing events |
| O2 | Bincode + format-marker byte (0x01) + schema_version u16 LE | Replay tool's synthetic events use the same encoding; bundle is JSON over the wire (no bincode in transport) |
| O3 | Cascade-origin pin in pruner | Replay tool does NOT touch the live pruner; it operates on synthetic events in-memory |
| O4 | Explicit `Option<BlockProvenance>` on `apply_block` | Replay tool's synthetic BlockApplied events use `from_peer_id: Some(parsed_peer_id)` if log had it, else None |
| O5 | First-match-wins classifier rules | Replay tool reuses the M3 `classify()` function as-is — no re-implementation |
| O6 | Pruner extracted from periodic.rs | Phase 2a does not modify periodic.rs |
| O7 | `ulid` dep accepted | Fleet RPC and replay synthetic events generate ULIDs the same way |

---

## Codebase Pointers

| Subsystem | Where |
|---|---|
| M1 types (the shapes Phase 2a serializes/deserializes) | `crates/storage/src/diagnostic_ledger/types.rs` |
| M3 classifier (Phase 2a reuses this verbatim) | `crates/storage/src/diagnostic_ledger/classifier.rs` |
| M3 RPC handler (mirror this for the fleet variant) | `crates/rpc/src/methods/diagnostics.rs` |
| M3 RPC dispatch (add new method arm) | `crates/rpc/src/methods/dispatch.rs` |
| M3 RpcContext (already has `diagnostic_ledger` field; may need an HTTP client) | `crates/rpc/src/methods/context.rs` |
| M4 CLI (extend with --fleet flag and replay subcommand) | `bins/cli/src/cmd_forks.rs`, `bins/cli/src/commands.rs`, `bins/cli/src/main.rs` |
| Existing fleet-query pattern (Guardian system polls multiple peers) | `crates/rpc/src/methods/guardian.rs` — borrow the HTTP-client pattern |
| Existing log patterns to parse | `bins/node/src/node/periodic.rs:898` (`[HEALTH]`), `bins/node/src/node/block_handling.rs:184,191` (`[FORK_GUARD]`), `crates/network/src/sync/manager/sync_engine/response.rs` (chain-break) |

---

## Evidence on Disk

- `~/testnet/logs/n10.log` (~1 GB; PID changed at 23:09 in the INC-I-083 timeline; snap-synced at 22:51) — primary source for INC-I-083 fixture
- `~/testnet/logs/n14.log` (stuck at h=110,596 — secondary fixture if needed)
- `~/testnet/logs/seed.log` (healthy advance — baseline reference for the parser to ensure it doesn't false-positive on normal operation)
- `docs/postmortems/2026-05-18-INC-I-081-broken-producer.md` (or grep the postmortems dir for the broken-producer block details) — primary source for INC-I-081 fixture
- `docs/.workflow/inc-i-083-session-handoff.md`, `domain-investigation-{fork,connectivity,parameters,code}.md`, `domain-diagnosis-report.md` — context on what the verdict should look like
- `specs/fork-observability-architecture.md` — sections "INC-I-083 Schema Adequacy" and "INC-I-081 Schema Adequacy" — the paper claims this workflow empirically validates

---

## Suggested Milestone Breakdown

| ID | Name | Est. LoC | Key Deliverables |
|----|------|----------|-----------------|
| **M1** | Fleet RPC types + HTTP client + handler | ~300 | `FleetBundle`/`PeerStatus`/`ForkGroup`/`DivergencePoint` structs (in `crates/storage/src/diagnostic_ledger/types.rs` as additive types); HTTP client wrapper that calls `getForkDiagnostic` on each peer with timeout; aggregation logic |
| **M2** | Fleet CLI integration | ~150 | `doli forks --fleet <peer1,peer2,...>` flag; `--human` renderer for fleet topology; tests with 3 mocked peers |
| **M3** | Log-pattern parser | ~250 | One sub-parser per log pattern (BlockApplied, BlockRejected, ForkGuard, ChainBreak, Health, Rollback, Reorg, SnapSync); streams line-by-line; unit tests per pattern |
| **M4** | Replay CLI + fixture tests | ~200 | `doli forks replay --log <file>` subcommand; 2 fixture files (INC-I-083, INC-I-081 captured snippets); 2 end-to-end fixture-replay tests; assertion that classifier produces actionable verdicts |
| **M5** | Docs + close-out | ~80 | Update `docs/fork_observability.md`, `docs/rpc_reference.md`, `docs/troubleshooting.md`; **enrich `.claude/skills/mainnet/SKILL.md` and `.claude/skills/testnet-deploy/SKILL.md`** — both currently have a "Fork-Diagnostic Observability (Phase 1)" section that references Phase 2a as NOT YET SHIPPED; update those sections to reflect the live fleet RPC + replay tool (CLI examples, ssh-from-mac fleet sweep one-liners, mainnet deploy verification using `--fleet`, troubleshooting entries for the new failure modes); `/sync-docs`; commit with 3-question checklist |

**Total estimated: ~950 LoC across 5 milestones.**

Each milestone is independently mergeable. M2 depends on M1. M4 depends on M3 (parser must exist before fixtures can be tested through it). M3 and M1 can run in parallel.

---

## Process Expectations

- **Branch off `feature/fork-observability-346`.** Name: `feature/fork-observability-346-phase2a`. Do NOT touch `main` directly; merge order is: Phase 1 branch → main first (separate user decision), THEN Phase 2a branch builds on the merged main.
- **Same TDD discipline as Phase 1.** test-writer → developer → QA → reviewer per milestone.
- **Reuse the classifier verbatim.** The fixture tests must produce correct verdicts using the EXACT classifier that ships in M3 of Phase 1. If a fixture test fails because the classifier outputs `Unknown` when you expected a named variant — investigate WHICH is wrong:
   - The classifier rules may be incomplete (extend them — but this means M3-of-Phase-1 was incomplete; document carefully).
   - The fixture may genuinely be `Unknown`-classifiable (e.g., INC-I-083 was a `classify()` coverage hole, not a fork-type — `Unknown` with rich evidence pointing to recovery.rs IS the correct verdict).
- **No multi-agent investigation.** Build it yourself. The whole point of Phase 1 + 2a is to make future fork diagnoses self-serve via the RPC + replay.

---

## Anti-Patterns to Avoid

- **Re-implementing the classifier in the replay tool.** Use M3's `classifier::classify()` directly.
- **Loading the whole log file into memory.** Stream-parse — `BufReader::lines()`.
- **Calling peer RPCs sequentially.** Use `futures::join_all` for parallelism.
- **Echoing peer IP addresses into the persisted FleetBundle.** PII constraint — only PeerIds may surface.
- **Allowing the fleet RPC to be invoked from a peer node.** The fleet RPC is operator-side. Document this — and if existing RPC auth is "open to anyone who can reach the port," flag that as a follow-up concern, NOT a Phase 2a fix.
- **Treating `Unknown` as a defect.** It's the safe answer for unrecognized fork types — the schema's value is the evidence_event_ids, not the named variant.

---

## The Success Test

After Phase 2a lands, the operator (or diagnostic sub-agent) does ONE of these:

```bash
# Fleet-level diagnosis — one RPC, full fleet verdict
$ doli forks --fleet http://127.0.0.1:8501,http://127.0.0.1:8502,...,8514 --explain --human

# Retroactive analysis of any past incident from its logs
$ doli forks replay --log ~/testnet/logs/n10.log --human
```

And gets a complete, structured, actionable verdict — for the whole fleet OR for a past incident — in under 10 seconds.

If the next fleet incident still requires walking nodes one at a time, or if INC-I-083 still requires manual log grep, Phase 2a failed.

---

## What This Workflow Will NOT Do (Phase 2b)

Explicit non-goals. Do not add these in Phase 2a. They are deferred to a separate future workflow:

- `schemars` / JsonSchema export of the bundle
- Fork tree / causality DAG visualization
- Dashboard / explorer integration (the explorer at `testnetlinux/explorer/` stays untouched)
- Fork honeypot debug mode
- Pre-fork warning stream / push alerts
- Performance optimization beyond "streams line-by-line, parallel peer queries"
- Authentication / authorization for the fleet RPC

---

## Final Reminder

Phase 1 proved the schema works on paper. Phase 2a proves it works on real data — INC-I-083 and INC-I-081 are the litmus tests. If the replay tool can't diagnose them correctly, the Phase 1 schema is wrong and Phase 2a's job is to surface that. Either outcome is useful.

Build the thing that makes the next fleet-level fork investigation a non-event. Then close the loop on the retroactive promise.

Go.
