# Prompt Refinement — INC-I-090 (observability gap RCA)

Original:
INVESTIGATE: The DOLI observability-fork system did not surface a real mainnet fork that occurred on N3 on 2026-05-25 22:54-23:04 UTC. I want root cause analysis of WHY this happened and a concrete repro path.

The fork-observability subsystem (skill: .claude/skills/observability-fork/) is supposed to:
  1. Record DiagnosticEvent rows (kind=ForkBlockReceived, ReorgExecuted, etc.) to a RocksDB ledger every time a node sees a divergent or orphan block.
  2. Expose getForkDiagnostic / getFleetForkDiagnostic / getStateRootDebug / getUtxoDiff RPC methods that surface those events with classification and recommended_action.
  3. fork-monitor.sh polls getChainInfo across nodes and flags divergent bestHash groups.

KNOWN GROUND-TRUTH FORK:
  Incident: INC-I-090
  Time:     2026-05-25 22:54:37 UTC → 23:04:30 UTC (UTC)
  Node:     N3 on mainnet (ai1)
  Height:   h=284677
  N3 block: hash=8ede1526... slot=291216 producer=54323cef (N3 itself)
  Canon:    hash=150b4a7b... slot=291215 producer=50fd1758
  Parent:   cefa9950... at h=284676 (both extend same parent → 1-block fork)
  Outcome:  N3 stuck 9 min (sync_fails 0→253), snap-sync recovered ~23:04:30.

Hypotheses H1-H7 (do not anchor on any single one):
  H1: Events were never emitted (instrumentation gap in block_handling.rs / fork_recovery.rs / sync state machine).
  H2: Events emitted but dropped (AsyncChannelEmitter ring-buffer overflow; events_dropped_total).
  H3: Events written but classifier doesn't recognize this fork shape (1-block self-produced minority resolved by snap-sync).
  H4: Events exist but no alerting layer consumes them — fork-monitor.sh only polls getChainInfo at unknown cadence.
  H5: Classifier returns wrong recommended_action for this case (normal_operation when it should be auto_recover or alert).
  H6: RPC methods work but no operator-facing surface (dashboard / explorer / metrics) consumes them.
  H7: Mainnet binary on ai1/n3 doesn't have observability subsystem compiled / wired in (version skew).

Anchors detected:
- "observability is broken" framing → STRIP/REFRAME — symptom not diagnosis; user explicitly warned against anchoring.
- 7 hypotheses provided → PRESERVED as candidate space — user requested confirm/refute per hypothesis, did not commit to one.
- Prior fork investigation cited recovery.rs:312 fencepost bug → ISOLATED — separate concern; observability investigation is independent.
- "add more logging" framing → STRIP — user explicitly forbids this as primary finding.

Domain context preserved:
- [incident] INC-I-090 already filed; root cause of FORK itself = recovery.rs:312 fencepost (out of scope here).
- [git] Source-of-truth code paths: bins/node/src/node/block_handling.rs, fork_recovery.rs; crates/network/src/sync/manager/recovery.rs; crates/storage/src/diagnostic_ledger/{emitter.rs, mod.rs, classifier.rs}; crates/rpc/src/methods/{dispatch.rs, diagnostics.rs, diagnostics_fleet.rs, stats.rs}; scripts/fork-monitor.sh, scripts/health-check.sh.
- [skill] .claude/skills/observability-fork/{SKILL.md, LEDGER-SCHEMA.md, RPC-CHEATSHEET.md}.
- [live] Mainnet N3 = ai1, RPC 127.0.0.1:8503, log /var/log/doli/mainnet/n3.log{,.1}, data /mainnet/n3/data, binary /mainnet/bin/doli-node-n3.
- [live] Cluster RPCs: seed1=8500 n1=8501 n2=8502 n3=8503; all on ai1-ai5 via ssh aliases.
- [prior] docs/.workflow/domain-diagnosis-report.md and 4 domain-investigation-*.md cover the underlying fork bug — use as evidence, do not re-derive.
- [ground-truth] Fork shape: 1-block, self-produced minority by N3, slot-conflict at h=284677 with canonical, resolved by snap-sync at ~23:04:30.
- ⚠️ CONSTRAINT: Mainnet is LIVE — read-only investigation. No restarts, no data wipes, no config changes, no deploys, no "fixes". Diagnose only.
- ⚠️ CONSTRAINT: Code is SoT. If doc/skill says X and code says Y, code wins.
- ⚠️ CONSTRAINT: Do NOT recommend "add more logging" as a primary finding.
- ⚠️ CONSTRAINT: Investigation MUST distinguish between (a) was the event emitted, (b) was it persisted, (c) was it classified actionably, (d) did any consumer surface it — these are 4 separable failures.

Refined:
Determine why the observability-fork subsystem did not surface INC-I-090 on N3 (h=284677, 2026-05-25 22:54:37→23:04:30 UTC) before the user noticed visually.

The system has four sequential layers — emit → persist → classify → surface — and the failure could be in any one or several. Determine the failure layer(s) by:

(L1) Emit: For the actual code path N3 executed during the incident (FORK_GUARD signals visible in /var/log/doli/mainnet/n3.log.1), trace whether `DiagnosticEmitter::record` is called. Walk block_handling.rs (orphan handling, divergent parent), fork_recovery.rs (recovery entry), and sync/manager/recovery.rs (the file containing the underlying fencepost — does that path emit?). Cite file:line for every emit-or-not call site that the actual code path crossed.

(L2) Persist: Query getForkDiagnostic on N3 (RPC 8503) for window [284670, 284685]. Capture events_written_total, events_dropped_total, ledger_available. NONZERO events_dropped_total implies AsyncChannelEmitter ring-buffer overflow (H2). Also query seed1/n1/n2 — they should have ForkBlockReceived for N3's 8ede1526 block; their absence narrows to L1 vs L2 on the receiver side.

(L3) Classify: Read crates/storage/src/diagnostic_ledger/classifier.rs (8 rules, first-match-wins). Identify which rule matches a 1-block self-produced minority fork resolved by snap-sync. Capture the fork_type and recommended_action. If recommended_action == "normal_operation" or equivalent low-priority, that is a classification coverage gap (H3/H5).

(L4) Surface: Check fork-monitor.sh — is it running on mainnet (systemd unit / cron)? At what cadence? A 9-minute 1-block divergence with 60s polling should catch it; 600s would not (H4). Check if any dashboard, explorer, or metrics consumer reads the RPC outputs (H6).

(L0) Version skew: As a parallel check, confirm the mainnet doli-node-n3 binary on ai1 contains the observability subsystem — grep symbols / check build date vs feature merge (H7).

Produce a unified diagnosis identifying the failure layer(s), specific defects (file:line, severity, fix sketch), a minimal local-testnet repro recipe, and per-hypothesis confidence (low/medium/high).

Investigation directive: this is a /omega-doctor --investigate run. Treat each layer L0-L4 as independently falsifiable. Do not collapse failures into one anchor before evidence is gathered — multiple layers may have failed independently.
