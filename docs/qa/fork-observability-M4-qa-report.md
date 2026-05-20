<!--
OUTPUT CONTRACT: N/A — QA report (not a test file)
INPUT PARTITIONS: N/A — QA report (not a test file)
-->

# M4 QA Report: CLI + Agent-Facing Docs

## Verdict: APPROVED

Commit `899721e9` on `main`. All M4 acceptance criteria met. 36/36 M4 tests pass. Full regression (storage, doli-node, rpc, doli-cli) passes with 0 failures. No consensus files modified. CLI works end-to-end (exits non-zero on RPC failure with clear error). Docs are schema-first and agent-readable.

## Acceptance Criteria (REQ-by-REQ)

| REQ | Status | Evidence |
|-----|--------|----------|
| CLI-001 (--human, JSON default) | MET | `doli forks` outputs JSON via `serde_json::to_string_pretty`. `--human` renders 4 sections (Health, Events, Classification, Baseline). Verified in source and help text. |
| CLI-002 (--last duration) | MET | `parse_duration` handles h/m/s, rejects invalid. Default "1h" -> 3600s. Unit tests confirm. |
| CLI-003 (--explain) | MET | Fetches recent events, finds most recent ForkBlockReceived/BlockRejected, re-queries with fork_event_id. Prints message when no fork events found. |
| CLI-004 (--by-producer) | MET | `aggregate_by_producer` reads `fork_summary.by_producer`, sorts by `Reverse(count)`. Both JSON and --human output paths. |
| DOC-001 (rpc_reference.md) | MET | getForkDiagnostic appears 3 times in rpc_reference.md. |
| DOC-002 (troubleshooting.md) | MET | 4 references to fork diagnosis / `doli forks` in troubleshooting.md. |
| DOC-003 (fork_observability.md) | MET | Covers all 12 EventKinds, DiagnosticBundle schema (TypeScript), 9 classification types, retention env vars, cascade-origin pin. |
| DOC-004 (3-question checklist) | MET | Commit message contains Q1=NO, Q2=NO, Q3=YES with justification. |
| RETRO-001 (INC-I-083) | MET | Architecture spec has dedicated "INC-I-083 Schema Adequacy" section (line 593). fork_observability.md references it (line 158). |
| RETRO-002 (INC-I-081) | MET | Architecture spec has "INC-I-081 Schema Adequacy" section (line 632). fork_observability.md references it (line 167). |

## CLI Usability Spot-Check

- `doli forks`: Calls `getForkDiagnostic` RPC with `window_secs=3600`. Outputs `DiagnosticBundle` as pretty-printed JSON. PASS.
- `doli forks --human`: `render_human()` covers Health (ledger_available, written/dropped), Events (list or "no events"), Classification (None/Named/Unknown with reason), Baseline (rates + delta%). PASS.
- `doli forks --last 1h`: `parse_duration("1h")` -> 3600. Passed as `window_secs`. PASS.
- `doli forks --explain`: Two-phase: first fetches window to find most recent fork event ID, then re-queries with `fork_event_id`. Graceful "No fork events found" on empty. PASS.
- `doli forks --by-producer`: Reads `by_producer` map, sorts by `Reverse(count)`, outputs JSON or text. PASS.
- `doli forks --rpc <url>`: Overrides default RPC endpoint. PASS.

## Docs Quality Audit

- **Schema-first**: YES. TypeScript interface block with field types, not prose.
- **All 12 EventKinds**: YES. Table with discriminant, description, trigger location, key payload fields.
- **Retention env vars**: YES. `DOLI_DIAG_RETENTION_DAYS` and `DOLI_DIAG_MAX_EVENTS` documented.
- **Cascade-origin pin**: YES. Documented in retention section.
- **Phase 2 deferrals**: NO. Not mentioned in fork_observability.md. See OBS-001.

## Test Results

- M4 tests: **36/36 passed**, 0 failed.
- Regression (storage + doli-node + rpc + doli-cli): **all passed**, 0 failed.

## End-to-End Smoke

Node running on `127.0.0.1:8500` (older binary without M3 RPC). CLI correctly calls `getForkDiagnostic`, receives `-32601 Method not found`, exits with code 1 and clear error message. `--help` output confirms all 5 flags.

## Exit Code Semantics

PASS. RPC failure -> `anyhow::anyhow!("RPC unavailable: ...")` -> `?` propagation -> exit code 1.

## Modular Discipline

PASS. `cmd_forks.rs`: 365 lines (under 500). `fork_observability.md`: 173 lines (under 500).

## DO-NOT-MODIFY: PASS

Files changed: `bins/cli/src/cmd_forks.rs`, `bins/cli/src/commands.rs`, `bins/cli/src/main.rs`, `bins/cli/tests/cmd_forks_test.rs`, `docs/fork_observability.md`, `docs/rpc_reference.md`, `docs/troubleshooting.md`, `docs/.workflow/milestone-progress.md`. No consensus, validation, snapshot, network_params, or apply_block files touched.

## Issues Found

None blocking.

## Non-Blocking Observations

- **OBS-001**: `docs/fork_observability.md` does not mention Phase 2 deferrals (replay tool, fleet RPC, schemars export). An agent reading this doc has no signal about what features are intentionally absent. Low severity — requirements don't mandate it, but it improves agent utility.

## Overall Verdict

**APPROVED**. All Must and Should requirements met. No blocking issues. M4 is complete.
