# Feature Evaluation: Agent-Consumable Fork-Diagnostic Subsystem

## Feature Description
Emitter + RocksDB-CF ledger + 3 bundle RPCs + deterministic classifier + JSON CLI + historical-log replay tool + schema export. Designed for agent consumption (Claude sub-agents as primary consumer). Workflow #346.

## Evaluation Summary

| Dimension | Score (1-5) | Assessment |
|-----------|-------------|------------|
| D1: Necessity | 5 | 10+ fork incidents in 3 months, 2-4h each, proven recurring pain |
| D2: Impact | 4 | 16-32h/month diagnostic labor reduced to minutes; narrow audience (1 operator + agents) |
| D3: Complexity Cost | 2 | 6 crates, ~3-4k LoC, 2 new deps, novel retention infra, replay parser |
| D4: Alternatives | 4 | Existing tools partially work but at 100x time cost; simpler intermediate possible |
| D5: Alignment | 4 | Fits RocksDB CF + RPC module patterns; JsonSchema is novel but non-blocking |
| D6: Risk | 4 | Read-only instrumentation, no consensus impact; verify emit latency before merge |
| D7: Timing | 4 | INC-I-082 just landed, INC-I-083 proves need, no conflicting work |

**Feature Viability Score: 4.0 / 5.0**

## Verdict: CONDITIONAL

The feature is genuinely needed (5) and well-timed (4), but the full scope (3-4k LoC, 6 crates, replay tool, schema export, compaction filter) is disproportionate for the first delivery. The FVS sits exactly at the GO threshold; one scope cut to reduce D3 from 2 to 3 would make this a clean GO.

## Cross-Dimension Synthesis

**Primary tension: D1(5) + D2(4) vs D3(2)** -- Classic "must have, too expensive" pattern. Resolution: phase the delivery so the high-value core ships first and the expensive tail (replay tool, schema export, compaction filter) ships only after the core proves itself.

**Secondary tension: D4(4) indicates a simpler alternative exists** -- The scorers noted that structured JSONL + one aggregation RPC delivers 60-70% of value at 30% cost. The full system is optimal but the phased approach starts from that simpler alternative and grows.

## Conditions

- [ ] **Phase 1 boundary**: Ship ONLY emitter + RocksDB CF + `getForkEvents` RPC + classifier. Defer replay tool, schema export, and `getFleetForkDiagnostic` to Phase 2.
- [ ] **Emit latency gate**: Benchmark emit overhead in apply_block hot path; must be <50us per block or switch to async channel.
- [ ] **No compaction filter in Phase 1**: Use a periodic-task pruner (existing pattern in `periodic.rs`) for retention. Compaction filter is novel infra with no codebase precedent -- defer.
- [ ] **Skip `schemars` dep in Phase 1**: Document the JSON shape in code comments + a test fixture. Schema-first discipline can come in Phase 2 after the core is validated.

## Alternatives Considered
- **Structured JSONL + grep scripts**: 60% value at 30% cost. Good Phase 0 but insufficient for agent-speed queries.
- **Single aggregation RPC only**: Solves the "5s answer" goal but without persistence loses history for pattern detection.

## Recommendation
Proceed with a phased scope. Phase 1 (~1.5-2k LoC, 3-4 crates) delivers the core diagnostic loop: emit structured events, store in CF, expose via RPC, classify. This alone eliminates the 2-4h diagnosis problem. Phase 2 (replay, fleet query, schema) adds polish once the core is battle-tested. Workflow #346.

## User Decision

**ACCEPTED — Phased scope (Recommended option).** 2026-05-20.

Phase 1 (this workflow #346):
- Emitter (instrument apply_block, block_handling, fork_recovery, rollback, recovery.classify, sync_engine response, snap_sync — read-only, no decision-logic changes)
- RocksDB column family ledger with bounded retention via a periodic-task pruner (existing pattern in `periodic.rs`)
- One operator-grade RPC method (`getForkEvents` or equivalent — name decided by architect) returning a self-contained DiagnosticBundle
- Deterministic typed classifier producing a `Classification {type, confidence, evidence_event_ids, recommended_action, recommended_action_args}` enum
- JSON-default CLI subcommand (`doli forks`)
- Emit-latency benchmark in apply_block hot path; gate: <50us per emit (or async channel)
- Unit tests for emitter, ledger I/O, RPC, classifier
- One unit-level tip-race reproduction test
- Docs: `docs/rpc_reference.md` + `docs/troubleshooting.md` + new `docs/fork_observability.md`

Phase 2 (separate future workflow — NOT in scope of #346):
- Historical-log replay tool (`doli forks replay --log <file>`)
- Retroactive INC-I-083 + INC-I-081 fixture replay tests
- `schemars`/JsonSchema export (`docs/fork_observability_schema.json`)
- `getFleetForkDiagnostic` cross-fleet correlation RPC

Phase 1 retroactive validation: the schema must be DESIGNED such that, if Phase 2's replay tool were applied to the n10/n14 logs from INC-I-083 and the broken-producer logs from INC-I-081, the resulting DiagnosticBundle would carry the correct verdict. The schema must be defensible against these two incidents on paper, even though the replay tool itself ships in Phase 2.
