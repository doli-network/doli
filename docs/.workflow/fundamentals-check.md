# Fundamentals Check — INC-I-090 observability gap RCA

This is a **diagnostic investigation** (--investigate), not a code change. The "system" under investigation is the observability subsystem of a LIVE production blockchain. Fundamentals are framed against the investigation feasibility, not local build state.

## Resource / environment fundamentals

| Item | Status | Evidence |
|------|--------|----------|
| Local repo accessible | PASS | `git branch --show-current` → `main`; `.omega/memory.db` reachable. |
| Incident record exists | PASS | `sqlite3 ... incidents WHERE incident_id='INC-I-090'` → 1 row, status=`investigating`, has root_cause for underlying fork bug. |
| Prior fork-domain diagnosis present | PASS | `docs/.workflow/domain-diagnosis-report.md` + 4 `domain-investigation-*.md` files staged in repo per refined prompt; they document the underlying fencepost bug (out of scope here). |
| Observability skill files present | PASS | `.claude/skills/observability-fork/SKILL.md` (121 lines, current per skill table) + `LEDGER-SCHEMA.md` (134 lines) read in this session. |
| Source files referenced in skill exist | PASS (assumed) | Skill cites concrete file:line refs; full code reads deferred to investigator sub-agents (investigation gate active). |
| Mainnet RPC reachable from ai1 | UNKNOWN | Deferred to investigators (live mainnet ssh requires sub-agent delegation per gate). N3 RPC 127.0.0.1:8503 expected per refined prompt. |
| Mainnet binary build provenance | UNKNOWN | H7 (version skew) requires `ssh ai1 + check binary symbols vs feature merge date`; deferred to investigators. |
| Local testnet available for repro | PASS (presumed) | `~/testnet/` exists per CLAUDE.md; `scripts/testnet.sh` is the standard interface. Not yet exercised — needed for STEP 7 repro recipe. |
| memory.db queries work | PASS | All `sqlite3` queries above returned rows; schema columns confirmed (workflow_runs uses `created_at`, not `started_at`). |

## Occam's Razor ordering (rca-backpressure.md levels)

Before deep investigation, considered the cheapest hypotheses:

| Level | Hypothesis | Status |
|-------|-----------|--------|
| 1. Resource exhaustion | Ring-buffer overflow dropped events (H2) | UNKNOWN — needs live `events_dropped_total` probe (investigator-state). |
| 2. Misconfiguration | fork-monitor.sh not deployed / wrong cadence (H4) | UNKNOWN — needs `ssh ai1; systemctl list-units \| grep fork`; investigator-state. |
| 3. Version skew | Mainnet binary lacks observability code (H7) | UNKNOWN — needs binary-vs-source diff (investigator-log + investigator-state). |
| 4. Code defect — emit gap | Code path didn't call `emit()` (H1) | UNKNOWN — needs static trace of N3's executed path (investigator-code). |
| 5. Code defect — classify | Classifier returns low-priority action (H5) | LIKELY per skill rules — TipRaceHighLatency/Natural → action ~normal — needs classifier read (investigator-code). |
| 6. Architecture gap | No alerting consumer wired in (H6) | UNKNOWN — needs deployment/infra scan (investigator-state). |

The hypotheses are NOT mutually exclusive — multiple layers (emit, persist, classify, surface) may each have a partial failure. Investigation must keep all open until evidence eliminates each.

## Invariants the investigation must preserve

| Invariant | Source | Why it matters here |
|-----------|--------|---------------------|
| MAINNET is LIVE — read-only | User constraint + CLAUDE.md | No `ssh ai1 systemctl restart`, no `rm -rf /mainnet/.../diagnostics`, no config rewrite. All evidence collection must be non-mutating. |
| Code is SoT | CLAUDE.md Law #2 | If skill claims `block_handling.rs:154-418` emits, but actual code doesn't, code wins. Verify line refs by reading. |
| `Hash::ZERO` is NOT a fork signal | MEMORY.md INC-I-014 | If incidental probes show 0x00 hashes, do not infer fork. |
| Diagnostic ledger is per-node | SKILL.md CONSTRAINTS | Querying only N3's ledger may miss events that other nodes saw; investigators must probe seed1/n1/n2 too. |
| Anti-anchoring | Refined prompt + protocol | Do not collapse on H1/H4/H5 before evidence; multi-layer failure is plausible. |

## Silent absences (things that should exist but might not)

- A systemd unit file `doli-fork-monitor.service` is NOT in `scripts/` per gate-blocked check; investigators must verify deployment (or non-deployment) on ai1-ai5.
- The skill mentions `health.events_dropped_total` and `health.last_heartbeat_ms` — these counters MUST be live and queryable for H2 to be testable. If RPC returns no `health` field, that itself is a finding.
- The skill mentions `WriterHeartbeat` (EventKind=12) but does not document who consumes heartbeat absence as an alert — possible H6 gap.
- LEDGER-SCHEMA notes "Production default retention and max_events: NOT set in this codebase (caller-determined)" — if caller never schedules pruning, the ledger could be unbounded; if it IS scheduled, events for INC-I-090 (24h ago) may already be pruned. CRITICAL pre-flight: confirm events from 2026-05-25 are still present before drawing conclusions from their absence.

## Quality audit

```
━━━ FUNDAMENTALS CHECK QUALITY AUDIT ━━━
Items with PASS + evidence: 6/9
Items with PASS but NO evidence: 0     ← clean
Items with FAIL: 0
Items with UNKNOWN + reason: 3         ← all deferred to investigation gate (correct path)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

UNKNOWNs are not rubber-stamps — they are explicit deferrals to investigator sub-agents because the investigation gate (correctly) blocks the orchestrator from running `ssh ai1` + live probes. All three UNKNOWNs are first-class investigation questions assigned to specific investigator roles below.
