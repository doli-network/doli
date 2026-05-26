# Investigation Brief — INC-I-090 observability gap RCA

## Refined prompt
See `docs/.workflow/prompt-refinement.md`.

**Bottom line**: A real production fork happened on N3 mainnet at h=284677 (2026-05-25 22:54:37 → 23:04:30 UTC). No automated surface (dashboard, alert, monitoring) flagged it — the user noticed visually. The observability-fork subsystem has 4 layers (Emit → Persist → Classify → Surface) and possibly a 5th cross-cutting issue (version skew). Determine WHICH layer(s) failed.

## Ground truth (do not re-derive — already established)

- **Incident**: INC-I-090, status `investigating`, root_cause for underlying fork = recovery.rs:312 fencepost (OUT OF SCOPE — that's about WHY the node got stuck, not about WHY observability missed it).
- **Fork shape**: 1-block, self-produced minority by N3, same-parent (`cefa9950...` at h=284676), N3 block `8ede1526` (slot 291216, producer `54323cef` = N3) vs canonical `150b4a7b` (slot 291215, producer `50fd1758`).
- **N3 outcome**: stuck 9 min, sync_fails 0→253, recovered by snap-sync at ~23:04:30.
- **Detection mode**: user visual notice, post-hoc.

## Subsystem map (from architect blueprint — `docs/.workflow/system-blueprint.md`)

```
[L0: Deploy]    Mainnet binary /mainnet/bin/doli-node-n3 must contain observability code
                       ↓
[L1: Emit]      bins/node/src/node/block_handling.rs  (forks on gossip arrival)
                bins/node/src/node/fork_recovery.rs   (recovery path)
                crates/network/src/sync/manager/recovery.rs  (sync state machine)
                bins/node/src/node/apply_block.rs     (BlockApplied)
                  → DiagnosticEmitter::record(event)  -- fire-and-forget
                       ↓
[L2: Persist]   AsyncChannelEmitter ring buffer (drop-oldest on overflow)
                  → writer task drains → RocksDB cf_events
                  Health counters: events_written_total, events_dropped_total, ledger_available, last_heartbeat_ms
                       ↓
[L3: Classify]  classifier.rs 8 rules, pure function, ON-DEMAND via RPC only
                  → ForkType + confidence + recommended_action
                       ↓
[L4: Surface]   getForkDiagnostic / getFleetForkDiagnostic / getStateRootDebug / getUtxoDiff (RPC)
                fork-monitor.sh (polls getChainInfo — not getForkDiagnostic)
                (no documented dashboard / explorer / metrics consumer)
```

## Architect verdict
CODE-FIXABLE at conf(0.80). Top 3 architectural smells flagged:
1. **No automated consumer of `recommended_action`.** Classifier produces actionable output but no code reads it.
2. **Recovery coordinator emission gap (unverified).** `RecoveryClassifyCall` event kind exists, but skill does not document an emit call in recovery.rs. If 253 recovery iterations produced zero diagnostic events, ChainBreakLoop rule cannot fire.
3. **Classification is lazy (on-demand only).** No background classifier; nobody knows the recommended_action unless they manually call the RPC.

## Log sufficiency verdict
SUFFICIENT. Load-bearing evidence channels:
1. FORK_GUARD textual log lines in `/var/log/doli/mainnet/n3.log.1` — colocated with emit call sites; prove emit code path was reached (L1).
2. `getForkDiagnostic` RPC on N3:8503 — returns events + health.events_dropped_total + classification.fork_type + recommended_action (L1/L2/L3/L0).
3. `systemctl list-unit-files` + `crontab -l` on ai1 — resolves whether fork-monitor.sh is deployed (L4).
4. Classifier is a pure function — inputs+code+output is reconstructable offline (L3).

## Evidence assembly (memory.db)
See `docs/.workflow/evidence-assembly.md`.
- 0 prior failed approaches in this domain specifically (clean slate).
- 20 prior runs in broader fork/sync domain — high incident frequency, but the observability gap is a fresh investigation.
- 6 open P0/P1 findings in fork/sync subsystems (suggesting compound architectural stress).
- Hotspot: `bins/node/src/node/block_handling.rs` marked CRITICAL.
- INV-OBS-001: diagnostic RPC hardcoding anti-pattern (in invariants).
- Constraint table EMPTY — investigators must not impose prior failures that don't exist.

## Scope, INC_ID, RUN_ID

- SCOPE: observability-fork (the diagnostic-ledger + RPC + alerting pipeline). NOT the underlying fork bug.
- INC_ID: INC-I-090
- RUN_ID: 372

## Live access (read-only)

- ssh ai1 → mainnet host (N3 runs here as `doli-mainnet-n3.service`)
- /var/log/doli/mainnet/n3.log + n3.log.1 (rotated, contains incident window)
- /mainnet/n3/data → RocksDB (diagnostics CF inside diagnostics/)
- /mainnet/bin/doli-node-n3 → deployed binary
- RPC: 127.0.0.1:8500 (seed1), :8501 (n1), :8502 (n2), :8503 (n3)
- All ai1-ai5 servers reachable via SSH aliases — `ssh aiN` (no further config needed).

**MAINNET IS LIVE. READ-ONLY. No restarts, no wipes, no config changes.**

## Falsifiable questions per layer

| Layer | Question | Discriminator |
|-------|----------|---------------|
| L0 | Was the deployed binary built from a commit including the observability subsystem? | `doli-node-n3 --version` + `strings` for diagnostic_ledger symbols; commit date vs feature merge date |
| L1 | Did N3's executed code path call `DiagnosticEmitter::record` for the fork at h=284677? | FORK_GUARD log lines + `getForkDiagnostic` for window 284670-284685 |
| L2 | Were events emitted but dropped? | `health.events_dropped_total > 0` from RPC |
| L3 | What `recommended_action` does the classifier return for this fork shape? | Read classifier.rs; check live RPC response; trace which of 8 rules matched |
| L4-script | Is fork-monitor.sh deployed as a systemd unit or cron on ai1? | `systemctl list-unit-files \| grep fork`; `crontab -l` |
| L4-consumer | Does any dashboard/explorer/metrics consume getForkDiagnostic? | grep repo for callers; ssh ai1-ai5 + scan running processes |
| L4-fleet | Did seed1/n1/n2 see N3's fork block in their ledgers? | `getForkDiagnostic` on RPC 8500/8501/8502 for same window |

## Anti-anchoring directive

DO NOT collapse on a single hypothesis. Multi-layer compound failure is plausible (e.g. classification returns "normal_operation" AND fork-monitor not deployed AND no dashboard consumer — three independent failures). Treat each layer as independently falsifiable. The synthesizer will determine convergence.

## Anti-hedging directive (HARD)

Do NOT recommend "add more logging" or "add more observability" as a primary finding. Per Log Sufficiency Verdict, existing evidence is SUFFICIENT. Your job is RCA, not telemetry shopping.

## Constraints
- MAINNET read-only.
- Code is SoT. Skill claims must be verified against code.
- Hash::ZERO is not a fork signal.
- Diagnostic ledger is per-node — must query each node separately.
