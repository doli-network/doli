# Analyst — Observability gap RCA for INC-I-090

**INC**: INC-I-090
**RUN_ID**: 372
**Mode**: `/omega-doctor --investigate` (read-only, no fix loop)
**Scope**: observability-fork subsystem (skill `.claude/skills/observability-fork/`)
**Note**: Underlying fork root cause (recovery.rs:312 fencepost) is OUT OF SCOPE — already diagnosed in `docs/.workflow/domain-diagnosis-report.md`. This investigation is about WHY the observability subsystem did not surface the fork before the user noticed visually.

## Bug statement (de-anchored)

A real production fork occurred on N3 on mainnet:
- Time window: 2026-05-25 22:54:37 → 23:04:30 UTC (~9 minutes)
- Height: h=284677, fork hash `8ede1526...` (N3 self-produced, slot 291216, producer `54323cef` = N3 itself) vs canonical `150b4a7b...` (slot 291215, producer `50fd1758`)
- Both extended same parent `cefa9950...` at h=284676 → 1-block divergence
- Resolution: snap-sync at ~23:04:30, sync_fails escalated 0→253
- **Detection**: USER visual notice. No dashboard alert, no monitoring page, no automated alarm fired.

The observability-fork subsystem documented in the skill provides four sequential layers:

```
[L1: Emit]      block_handling.rs / fork_recovery.rs / sync/manager/recovery.rs
                  → DiagnosticEmitter::record(event)
                       ↓
[L2: Persist]   AsyncChannelEmitter ring buffer → writer task → RocksDB cf_events
                       ↓
[L3: Classify]  classifier.rs 8 rules → ForkType + recommended_action
                       ↓
[L4: Surface]   RPC methods (getForkDiagnostic, getFleetForkDiagnostic, etc.)
                + fork-monitor.sh polling + (any operator-facing surface)
```

Any one or several of these layers may have failed. The investigation must NOT collapse on a single hypothesis before evidence rules out the others.

## Architecture Context

The observability subsystem is **per-node and decoupled from consensus**. Specifically:

1. **Emission is fire-and-forget** (`block_handling.rs:168-190`, per skill): `let _ = self.diagnostic_emitter.record(...)`. Any panic/error inside the emitter is silently discarded. This is necessary for hot-path safety but means emission failure is invisible to consensus.

2. **Ring buffer is drop-oldest on overflow** (`emitter.rs:175-178`): if the writer task can't drain fast enough, events for the very incident we want to diagnose can be evicted before reaching RocksDB. Health counter `events_dropped_total` is the canary.

3. **Persistence is per-node** — there is NO gossip of diagnostic events between nodes. To see N3's perspective of the fork, you MUST query N3 specifically. seed1/n1/n2 each record their own perspective independently. The fleet view requires polling all of them (`getFleetForkDiagnostic`).

4. **Classification is offline + pure** — `classify(events: &[DiagnosticEvent])` is called only when `getForkDiagnostic` RPC is invoked. There is no background classifier that proactively flags or alerts. Classification result is computed on-demand per RPC call. **This is a structural property: classification produces a `recommended_action` field but no code path consumes that field automatically.** A `recommended_action: "manual_intervention"` will sit in a JSON response forever unless a human or external tool reads it.

5. **Surface layer is minimal**:
   - `fork-monitor.sh` polls `getChainInfo` (NOT `getForkDiagnostic`) and groups by `bestHash`. It detects only ACTIVE fleet divergence visible in the chain tip — a 1-block transient fork on ONE node that the rest of the fleet treats as canonical will produce a TRANSIENT divergence that may or may not coincide with a poll.
   - The skill does NOT document a systemd unit, cron job, or container that runs `fork-monitor.sh` on mainnet. This is suspicious.
   - There is no documented dashboard, explorer page, or metrics scrape that consumes any of the four diagnostic RPCs.

**Architectural implication**: even if L1+L2+L3 work perfectly, if no L4 consumer exists, the data sits in RocksDB until someone manually runs a curl. That matches the symptom — "user noticed visually" without any alert.

## Probable cause area (not yet a verdict)

Multiple layers are suspect, with rough prior weights based on architecture review alone:

| Hypothesis | Architecture-only prior | Why |
|-----------|------------------------|-----|
| H1 (no emit) | Low | Skill cites concrete emit sites for ForkBlockReceived on every non-tip block. N3 received `150b4a7b` (canonical) while holding `8ede1526` — that's `HeightOccupied`, which the skill says emits. |
| H2 (ring overflow) | Low | Mainnet steady-state event rate is low (one BlockApplied per 10s + occasional sync events). Ring should not be full unless writer task wedged. Worth checking but unlikely as primary cause. |
| H3 (classify shape unknown) | Medium-Low | Classifier has 8 rules + Unknown fallback. INC-I-090 shape likely matches (e) TipRaceHighLatency or (f) TipRaceNatural — those rules exist. Classification probably succeeds but with low-actionability output. |
| H4 (fork-monitor cadence/deployment) | **HIGH** | Skill documents `fork-monitor.sh` as a manual command, not a service. If it isn't running as a systemd unit on ai1-ai5, the script existing in the repo is meaningless. Even if running, polling `getChainInfo` only catches divergence visible at the chain tip, not historical 1-block races. |
| H5 (low-priority recommended_action) | **HIGH** | Per LEDGER-SCHEMA.md, the natural classification for "ForkBlockReceived where local applied has slightly different latency" is `TipRaceNatural` with conf 0.70 and (per skill OPERATIONS table line 50) `recommended_action: normal_operation`. A 1-block self-produced fork resolved by snap-sync is **architecturally indistinguishable from a benign tip race UNTIL the node gets stuck**. The "stuck" signal is post-hoc (sync_fails 0→253), and it's not clear the classifier consumes that. |
| H6 (no operator surface) | **HIGH** | Strong architecture evidence: no dashboard/explorer/metrics integration documented. The four RPC methods are read-only by design (CONSTRAINT in skill) and have no documented consumer beyond human curl. |
| H7 (binary version skew) | Medium | If the mainnet binary on ai1 was built before the observability feature merged, none of L1-L4 work. Requires `ssh ai1 + check build artifact`. The skill mentions Workflow #349 changed classifier rule order, so the feature is mature in source — but mature in source ≠ deployed. |

The hypothesis weights are NOT the diagnosis. They are starting points for the investigators to falsify.

## Impact analysis

The "fix" downstream of this investigation (out of scope for `--investigate`) would land in one or more of:
- `bins/node/src/node/block_handling.rs`, `fork_recovery.rs`, `crates/network/src/sync/manager/recovery.rs` (if H1)
- `crates/storage/src/diagnostic_ledger/emitter.rs` (if H2)
- `crates/storage/src/diagnostic_ledger/classifier.rs` (if H3 or H5)
- `scripts/fork-monitor.sh` + new systemd unit (if H4)
- New dashboard/explorer/metrics integration (if H6)
- Mainnet redeploy (if H7)

Blast radius is significant for L1 (touches consensus-adjacent code paths) and small for L4 (operator tooling only). The investigation must identify WHICH layer to fix before any code is written.

## Specs/docs drift flags (preliminary)

- The skill SKILL.md confidently claims `block_handling.rs:154-418` emits `ForkBlockReceived`. Investigators must verify by reading the code — code is SoT.
- The skill OPERATIONS table (row "Post-fork action routing") implies that `recommended_action` is wired into guardian procedures. The skill does not document any code path that consumes the `recommended_action` field automatically. Either the skill is aspirational, or there's a consumer not documented. Investigators must determine which.

## Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.95, measured)
Reasoning: 7 candidate hypotheses across 4 architectural layers (emit, persist, classify, surface) + 1 cross-cutting (version skew). Investigation is cross-module (block_handling, diagnostic_ledger, classifier, rpc methods, scripts, infrastructure). Mainnet probes required (RPC + binary + systemd inventory). Multi-layer compound failure plausible. None of the DEEP-PATH disqualifiers apply.
━━━━━━━━━━━━━━━━━━━━━━
```

**DEEP path triggers matched**:
- (2) Bug involves 3+ interacting components — yes (L1 emit sites, L2 ring buffer, L3 classifier, L4 surface, L0 deploy)
- (5) Architectural issues detected — yes (no surface consumer of `recommended_action` documented; possibly aspirational design)
- (6) Fundamentals check flagged systemic concerns — yes (silent absences: no systemd unit, no pruning policy documented, version skew unknown)

## Milestones (deferred — `--investigate` mode)

Not applicable — `--investigate` stops after the synthesizer. Milestone planning for the fix is a separate `/omega-doctor` (no `--investigate`) run, conducted on local testnet, after this diagnosis lands.

The diagnosis SHOULD output a milestone sketch as part of the repro recipe (STEP 7 of the refined prompt), but the orchestrator must NOT execute it in this run.

## Outstanding questions for the investigation

These are the falsifiable questions the parallel investigators must answer with evidence:

1. **L1 (Code investigator)**: For the actual code path N3 executed (FORK_GUARD signals in n3.log.1), does that path call `DiagnosticEmitter::record`? Cite file:line for every emit-or-not site.
2. **L2 (State investigator)**: Query getForkDiagnostic on N3 RPC 8503 for window [284670, 284685]. Capture events_written_total, events_dropped_total, ledger_available. NONZERO dropped → H2.
3. **L3 (Code investigator)**: Read classifier.rs and identify which rule a 1-block self-produced minority fork resolved by snap-sync matches. Capture the literal `recommended_action` string. If low-priority, that's the H5 finding.
4. **L4 (State + Log investigator)**: ssh ai1; `systemctl list-units | grep -iE 'fork|monitor'`; check if `scripts/fork-monitor.sh` is deployed as a service. If yes, what's its `OnUnitActiveSec` / loop interval? If no, that's the H4 finding.
5. **L0 (State investigator)**: Confirm the mainnet `/mainnet/bin/doli-node-n3` binary contains the observability subsystem. Strings/symbols check vs source. If a deployed binary predates the feature merge, that's H7.
6. **Cross-cutting (Wildcard investigator)**: What did seed1/n1/n2 each see? If they have `BlockApplied(8ede1526)` from N3 in their ledgers but no fleet aggregator polls them, that's a confirmation that L1+L2 work but L4 is the bottleneck.
7. **Cross-cutting (Constraints investigator)**: Apply the elimination matrix — given prior fork investigations succeeded post-hoc using these same RPCs, the data must be reaching RocksDB at least sometimes. What's structurally different about THIS incident shape that bypassed each layer?

These questions become the framing for the investigation brief in Step 2c.
