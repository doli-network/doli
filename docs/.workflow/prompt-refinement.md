━━━ PROMPT REFINEMENT ━━━

Original:
> Mission: End the Fork-Diagnosis Suffering — Build Trivial-Mode Observability for DOLI
> (See `docs/.workflow/prompt-fork-observability.md` — 157-line brief, referenced verbatim.)
>
> Pain: every fork/BEHIND incident costs ~4 hours of grep across 18 log files (some
> 1.9 GB), manual timestamp correlation, RPC-polling state-root debug, re-reading
> `recovery.rs:252-363`, and reconstructing causality from "chain break valid_so_far=0"
> lines that don't say which producer caused the fork or which peer the local block
> came from. 30+ incidents in the log (INC-I-009 → INC-I-083). Most recent: INC-I-083
> (2026-05-19, 5/18 testnet nodes frozen, required 4 parallel investigators +
> synthesizer + ~2h, and the investigator still initially mis-routed because no
> per-node fork-event counter exists).
>
> User mandate (verbatim, given when picking scope): "Your solution needs to be focused
> to be used and consumed only by agents, fuck humans! So, let the data form be your
> own language; from now on, it will be extremely easy to detect behinds, forks,
> regressions... we need the most effective solution! what you recommend?"
>
> Earlier user mandate (also verbatim): "the sky's the limit, enough of this when you
> know it's always too difficult to figure out what the problem is, doing thousands of
> greps, thousands of logs, and the analyses are always horribly depressing."
>
> The original "5-second human operator" framing in prompt-fork-observability.md is
> SUPERSEDED by the agent-consumption framing above. The success test is now: a
> diagnostic SUB-AGENT (future Claude session) can call ONE RPC and receive a complete,
> self-contained, machine-parseable diagnostic bundle that lets it issue a verdict
> without spawning sub-investigators or grepping logs.
>
> Starting-point ideas explicitly framed as "the floor, not the ceiling":
> (1) Single-line structured FORK_EVENT log entry with full context per divergence
> (2) Append-only per-node fork ledger at <data_dir>/fork_log.jsonl, bounded
> (3) RPC method getForkForensics(window_secs) returning ledger window + aggregations
>
> Provocations the brief explicitly invites the agent to consider, accept, reject, or
> replace: Time-machine RPC (getApplySequence), pairwise divergence query
> (compareWithPeer), causality DAG / fork tree visualization, producer fork-attribution
> scoring, deterministic auto-classifier of fork type, reorg play-by-play emitter,
> recovery decision trace, cross-fleet correlation RPC, baseline auto-capture, fork
> honeypot debug mode, block provenance trail, witness-mode markdown report,
> inverse-thinking pre-fork warnings.
>
> Retroactive validation requirement: the schema must be able to diagnose INC-I-083 and
> INC-I-081 from captured logs in <5 seconds (now: in ONE RPC call by an agent).
>
> Deliverables: design doc, modular implementation (no file >500 lines), unit tests
> (emitter, ledger I/O, RPC, classifier, retroactive INC-I-083 fixture replay),
> docs/rpc_reference.md + docs/troubleshooting.md updates + new docs/fork_observability.md,
> CLI skill update for `testnet forks`. Quickstart one-pager DEFERRED (target is agent
> consumers, not human operators).
>
> Codebase pointers given: recovery.rs (~720 lines), production_gate.rs:608-688,
> sync_engine/response.rs (Empty headers + Chain break origin), apply_block/ multi-file
> module, block_handling.rs, fork_recovery.rs, rollback.rs, periodic.rs, RPC method
> registry at crates/rpc/src/methods/, storage (jsonl-vs-RocksDB-CF decided: RocksDB CF),
> explorer at testnetlinux/explorer/. Evidence on disk: ~/testnet/logs/n10.log (~1 GB),
> ~/testnet/logs/n14.log, docs/.workflow/inc-i-083-session-handoff.md and the four
> domain-investigation-{fork,connectivity,parameters,code}.md files.

Anchors detected:
  - "Single-line structured log entry" → REJECT as primary interface. Human-grep ergonomics. Logs become a SECONDARY emission for replay tooling only. Primary interface is the RocksDB-CF ledger queried via RPC.
  - "fork_log.jsonl in <data_dir>" → REJECT as primary store. Agent consumption needs indexed range queries by (height, slot, producer, peer, event_kind, correlation_key). jsonl loses to RocksDB CF on every dimension. Optional jsonl EXPORT for offline log forensics, not the primary store.
  - "getForkForensics(window_secs)" → REFRAME as one of three bundle RPCs (`getRecentForks`, `getForkDiagnostic(fork_id)`, `getFleetForkDiagnostic(peer_rpcs, window)`). The atomic unit returned is a self-contained DiagnosticBundle, not aggregation summaries.
  - "5-second human operator" framing → SUPERSEDED. Success test rewritten for agent consumption: ONE RPC call returns ONE self-contained bundle that an LLM-driven sub-agent can parse and act on without further investigation.
  - Provocations list → CONVERTED to KEEP/REJECT/DEFER positions (see scope below); architect must defend each.

Domain context preserved:
  - [constraint] LOCAL testnet only (~/testnet/, launchd, 127.0.0.1) — NEVER ssh ai1–ai5
  - [constraint] ⚠️ CONSTRAINT: No consensus impact — read-only instrumentation only. No change to apply_block decision logic, validation, state-root computation, peer scoring, fork choice, attestation. Adding new observability CALLS from these modules is allowed; changing what they DECIDE is not.
  - [constraint] ⚠️ CONSTRAINT: Safe for rolling deploy — both INC-I-062 (block content) and INC-I-075 (consensus rules) questions answer NO. No activation height. No HardForkSchedule entry.
  - [constraint] ⚠️ CONSTRAINT: No mainnet contamination — testnet first, mainnet adopts at its own pace.
  - [constraint] ⚠️ CONSTRAINT: Bounded storage — ledger MUST self-cap. Default: 30 days OR 100k events per record_kind (configurable). RocksDB CF rotation via TTL or compaction filter.
  - [constraint] ⚠️ CONSTRAINT: No PII — libp2p PeerId OK to log, raw IP addresses are NOT.
  - [constraint] ⚠️ CONSTRAINT: Test discipline — every new RPC unit-tested; emitter has tip-race reproduction test; retroactive INC-I-083 + INC-I-081 fixture-replay tests; `cargo test -p doli-core -p storage -p network -p rpc` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --check` ALL clean.
  - [constraint] ⚠️ CONSTRAINT: Docs in sync — docs/rpc_reference.md, docs/troubleshooting.md, new docs/fork_observability.md (agent-facing schema doc, not human one-pager); `/sync-docs` at end; commit message includes three-question consensus-shape checklist with NO/NO/YES written out.
  - [constraint] ⚠️ CONSTRAINT: No genesis reset (per CLAUDE.md #0).
  - [constraint] ⚠️ CONSTRAINT: DO NOT MODIFY decision logic in — crates/core/src/consensus.rs, crates/core/src/network_params/defaults.rs activation heights, bins/node/src/node/apply_block/* (READ only; adding new emit calls FROM apply_block is allowed), crates/storage/src/snapshot.rs, crates/core/src/validation/*.
  - [constraint] ⚠️ CONSTRAINT: Modular — no source file >500 lines (800 for tests) per CLAUDE.md.
  - [constraint] ⚠️ CONSTRAINT: Default-ON for observability; default-OFF for any debug/honeypot mode.
  - [constraint] ⚠️ CONSTRAINT: CLI default output is machine-readable JSON. `--human` flag is the opt-in for pretty rendering, NOT the default.
  - [evidence] docs/.workflow/prompt-fork-observability.md — the full 157-line brief.
  - [evidence] docs/.workflow/inc-i-083-session-handoff.md + domain-investigation-{fork,connectivity,parameters,code}.md + domain-diagnosis-report.md — the most recent fork incident; retroactive validation target.
  - [evidence] ~/testnet/logs/n10.log (~1 GB, PID changed at 23:09, snap-synced to canonical in 90s), ~/testnet/logs/n14.log (stuck h=110,596 due to wipe missing nodes1/n14/data), ~/testnet/logs/seed.log (healthy advance) — live forensic data.
  - [evidence] docs/postmortems/ + MEMORY.md incident table — INC-I-009 through INC-I-083 history.
  - [evidence] Codebase pointer table in the brief — pre-validated entry points.
  - [scope] User mandate explicitly broad and agent-first: most-effective solution, sky's the limit, focus on agent consumption.
  - [success-test] ONE RPC call by a diagnostic sub-agent returns a self-contained DiagnosticBundle that contains every datum needed to issue a verdict on the most recent fork. No sub-investigators, no log grep, no timestamp correlation, no second RPC. If the next fork still requires multi-agent investigation, work failed.
  - [retroactive-test] Schema MUST diagnose INC-I-083 (5 frozen nodes, sync_fails climbing 200–360) AND INC-I-081 (broken producer emitted invalid epoch-boundary block) from captured logs via the historical-log replay tool in ONE bundle each. If not, schema is wrong.

Refined:

Build an agent-consumable fork-diagnostic subsystem for DOLI. The atomic unit of
agent consumption is a self-contained `DiagnosticBundle` returned by a single RPC.
The bundle contains every datum a diagnostic sub-agent needs to issue a verdict on a
fork or BEHIND event — no second RPC, no log grep, no timestamp correlation across
nodes, no sub-investigators required.

Four layers, each requiring independent architect justification:

  (a) EMITTER LAYER — records every fork-relevant event with full causal context.
      Event kinds (initial set; architect adds/removes):
        block_received, block_applied, block_rejected,
        orphan_buffered, orphan_parent_requested, orphan_parent_received,
        chain_break_detected, reorg_planned, reorg_executed,
        rollback_started, rollback_completed, rollback_aborted,
        recovery_classify_call, recovery_action_dispatched,
        snap_sync_attempted, snap_sync_completed, snap_sync_failed,
        fork_event_finalized.
      Each event carries: event_id (ULID), correlation_key (div_height, canonical_hash,
      fork_hash), caused_by_event_id, timestamp_ms, full kind-specific payload.

  (b) LEDGER LAYER — RocksDB column family per record_kind (justified vs JSONL because
      agents need indexed range scans by (height, slot, producer, peer, kind,
      correlation_key) and atomicity with existing block storage). Bounded retention
      via compaction filter. Optional JSONL export tool for offline log forensics.

  (c) RPC LAYER — minimum three bundle endpoints:
        - getRecentForks(window_secs, limit) → [DiagnosticBundle]
        - getForkDiagnostic(fork_event_id) → DiagnosticBundle
        - getFleetForkDiagnostic(peer_rpc_urls[], window_secs) → FleetBundle
      DiagnosticBundle is a typed Rust struct (derive Serialize, Deserialize,
      JsonSchema) containing: the FORK_EVENT, the causal event chain (all linked
      events with full payloads), the typed Classification {type, confidence,
      evidence_event_ids, recommended_action, recommended_action_args}, the baseline
      comparison {fork_events_per_hour_current, fork_events_per_hour_baseline_24h_avg,
      delta_pct, verdict_enum}, and the schema_version. Names not pre-decided.

  (d) CLI / TOOL LAYER — `doli forks` subcommand. JSON output is default. `--explain`
      flag returns the bundle for the most recent fork. `--by-producer` returns
      attribution aggregates. `doli forks replay --log <file>` ingests a historical
      log and emits the bundle that WOULD have been produced (retroactive validation
      AND debugging tool).

Agent-first design directives (apply to analyst, architect, test-writer):

  - The schema is the API. The Rust struct definitions in `crates/core/src/fork_observability/`
    (or wherever architect lands them) generate a JSON Schema document published at
    `docs/fork_observability_schema.json`. Agents read the schema once, then know how
    to parse every bundle without re-reading prose docs.

  - Stable field names across schema versions. Every record carries `schema_version`.
    Additive changes only. Removal requires a deprecation cycle.

  - The deterministic classifier returns a TYPED enum. The agent does NOT have to
    re-decide. Variants: tip_race_natural, tip_race_high_latency, producer_equivocation,
    epoch_boundary_invalid, post_snap_dead_tip, validation_disagreement, rollback_loop,
    snap_sync_to_minority_fork, unknown. Each variant carries variant-specific evidence
    fields. `unknown` is allowed but must carry a `reason_unknown` string AND
    `evidence_event_ids` for human escalation.

  - Causal chains are explicit (`caused_by_event_id`), not inferred from timestamps.
    The agent never has to correlate by timestamp ordering.

  - Cross-node correlation key is `(div_height, canonical_hash, fork_hash)` — a tuple
    that JOINs the same fork event across multiple nodes' ledgers without timestamp
    ambiguity.

  - Default-ON: emitter, ledger, RPC, CLI. Honeypot mode (if architect keeps it) is
    default-OFF.

For every brainstorm provocation in the original brief, take an EXPLICIT position
and defend it. Initial proposed positions (architect may overrule):

  - Time-machine RPC (getApplySequence) → KEEP (folds into getForkDiagnostic as the
    surrounding-events list)
  - Pairwise divergence query (compareWithPeer) → KEEP (folds into
    getFleetForkDiagnostic)
  - Causality DAG / fork tree viz → DEFER (human view; agents use the chain field)
  - Producer fork-attribution scoring → KEEP (folds into bundle aggregations)
  - Deterministic auto-classifier → KEEP (this IS the verdict)
  - Reorg play-by-play → KEEP (one of the record_kinds)
  - Recovery decision trace (recovery_classify_call event) → KEEP (high leverage)
  - Cross-fleet fork-correlation RPC → KEEP (getFleetForkDiagnostic)
  - Baseline auto-capture → KEEP (needed for verdict_enum)
  - Fork honeypot mode → DEFER (test-infra; separate workflow)
  - Block provenance trail (from_peer_id, received_at_ms) → KEEP (foundational
    for half the other features)
  - Witness-mode markdown report → REJECT in favor of the JSON bundle
  - Pre-fork warning stream → DEFER (separate observability stream; bundle answers
    "what happened", not "what is about to happen")

Validation gate: the design is invalid if it cannot retroactively diagnose INC-I-083
AND INC-I-081 from captured log fixtures via the historical-log replay tool, with each
incident producing ONE DiagnosticBundle that contains the verdict. Build the replay
tool and the historical-log fixtures FIRST. Lock the schema only after both fixtures
yield correct verdicts.

⚠️ CONSTRAINT: All hard constraints from the brief apply — no consensus impact, safe
for rolling deploy, bounded storage, no PII, default-ON observability + default-OFF
debug, modular (no file >500 lines), DO NOT modify decision logic in consensus.rs /
activation-height defaults / apply_block / snapshot.rs / validation/*. New
observability CALLS from those modules are allowed; changing their DECISIONS is not.

Feature directive: Analyze how the feature integrates with existing architecture
before implementing. The user's description defines WHAT, not HOW.

━━━ PHASE 1 BOUNDARY (locked by user 2026-05-20) ━━━

IN SCOPE for workflow #346:
  - Emitter (read-only instrumentation in apply_block, block_handling, fork_recovery,
    rollback, recovery.classify, sync_engine response, snap_sync)
  - RocksDB column family ledger + periodic-task pruner for retention
  - ONE operator-grade RPC returning a self-contained DiagnosticBundle
  - Deterministic typed Classification enum {type, confidence, evidence_event_ids,
    recommended_action, recommended_action_args}
  - JSON-default `doli forks` CLI subcommand
  - Emit-latency benchmark, gate <50us per emit (or async channel)
  - Unit tests + one tip-race reproduction test
  - Docs sync (rpc_reference.md, troubleshooting.md, new fork_observability.md)

DEFERRED to Phase 2 (separate future workflow — DO NOT IMPLEMENT in #346):
  - Historical-log replay tool (`doli forks replay --log <file>`)
  - Retroactive INC-I-083 / INC-I-081 fixture replay tests
  - `schemars` / JsonSchema export
  - `getFleetForkDiagnostic` cross-fleet correlation RPC

PHASE 1 RETROACTIVE-VALIDATION DISCIPLINE (still required in #346):
  The DiagnosticBundle schema MUST be designed such that, if Phase 2's replay tool
  were applied to INC-I-083 (n10/n14 logs) and INC-I-081 (broken-producer logs), the
  resulting bundle would carry the correct verdict. Defend the schema against these
  two incidents on paper in the architecture doc. The replay tool itself ships in
  Phase 2; the schema's adequacy must be argued NOW.

━━━━━━━━━━━━━━━━━━━━━━━━━
