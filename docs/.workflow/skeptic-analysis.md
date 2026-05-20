# Skeptic Analysis — Workflow #346

> Independent contradicting interpretation of the agent-consumable fork-diagnostic
> subsystem. Read in parallel with the analyst's requirements; the analyst must
> incorporate or explicitly defend against each reframe.

## What I Don't Understand
1. Whether the operator has ever used the existing `fork-monitor.sh` script successfully and what specifically it lacked beyond "THAT vs WHY" — the gap might be narrower than assumed.
2. How often the diagnostic sub-agent (future Claude session) would actually operate in a zero-human-in-the-loop mode vs. the current pattern where the user explicitly invokes `/omega-doctor` or `/omega-swarm`.
3. Whether the existing `classify_gossip_block()` in `block_handling.rs` already constitutes a deterministic classifier that the observability layer is trying to reinvent at a higher abstraction level.

## Obvious Interpretation (what the analyst will likely produce)
The analyst will read this as a 4-layer instrumentation project: emit structured events from hot paths, store in RocksDB CF, expose via RPC bundle, classify deterministically. Primary consumer is a Claude sub-agent. The scope is Phase 1 (emitter + ledger + one RPC + classifier + CLI). The user's "fuck humans" mandate means JSON-only, no human ergonomics.

## Contradicting Interpretation
The real consumer is the human operator verifying the agent's verdict. The "self-contained bundle" is an illusion because the most diagnostic forks (INC-I-083) required cross-domain context the bundle cannot contain. RocksDB CF is over-engineering for a time-series event log. The classifier will produce "unknown" for the incidents that actually matter. The emitter misses the real decision points by instrumenting `apply_block` when the fork decision happens upstream in `classify_gossip_block`.

---

## Reframe 1: The Real Consumer Is the Human Auditing the Agent's Verdict
- **Counter-claim**: The user said "fuck humans" but the behavioral pattern across 30+ incidents is: agent produces diagnosis, user reads it, user decides whether to wipe/restart/deploy. The human-in-the-loop never disappears — it shifts from "do the forensics" to "verify the verdict." A JSON-only interface with no `--human` summary makes verification harder, not easier. The user already has `feedback_trust_user_diagnosis.md` — when the user HAS diagnosed, the agent should act. But when the agent diagnoses, the user still reviews. The system becomes self-referential if no human can audit what the agent concluded.
- **Evidence**: INC-I-083 session handoff, section 7: the investigator's "regression does not reproduce" conclusion was WRONG, and the human caught it after 2 hours. If the agent trusts its own classifier output without human review, wrong verdicts propagate. MEMORY.md has `feedback_verify_crash_before_routing.md` — "verify before routing" is a human audit step the agent itself learned.
- **Risk if analyst's framing wins**: The system optimizes for agent speed but creates an audit gap. When the classifier says `tip_race_natural` and it is actually `epoch_boundary_invalid` (INC-I-081), nobody catches it because the human cannot easily read the bundle.

## Reframe 2: The "Self-Contained Bundle" Is Not Self-Contained
- **Counter-claim**: INC-I-083's root cause required knowing: (a) which nodes had `--no-snap-sync`, (b) the `SNAP_ATTEMPTS_MAX` counter state, (c) the `confirmed_height_floor` value, (d) whether `recently_synced()` was false, (e) the peer mesh topology. None of these are in `apply_block` or `block_handling`. They live in `recovery.rs` classify context (`RecoveryContext` struct) and `production_gate.rs` dispatch gates. The DiagnosticBundle spec includes `recovery_classify_call` events, but those capture the OUTPUT of classify, not the input state (peer_count, snap_attempts, confirmed_height_floor, `--no-snap-sync` flag). A diagnostic agent receiving the bundle would still need to query `getStateRootDebug` for state hashes and inspect node configuration to diagnose INC-I-083.
- **Evidence**: `recovery.rs:252-363` — `classify()` reads a `RecoveryContext` with 12+ fields including `in_grace_period`, `shallow_rollback_count`, `snap_attempts`, `peer_count`, `last_finality_height`. The domain-diagnosis-report.md needed ALL of these plus the 3 dispatch gates in `production_gate.rs:614-688`. The proposed event schema captures `recovery_classify_call` but the prompt-refinement.md does not specify capturing the full `RecoveryContext` as part of the event payload.
- **Risk if analyst's framing wins**: The "one RPC, no second call" promise fails on the first real incident, and the operator falls back to grepping logs anyway — exactly the problem this feature was supposed to solve.

## Reframe 3: JSONL Beats RocksDB CF for This Use Case
- **Counter-claim**: The primary diagnostic pattern is "show me events in the last N minutes sorted by time." JSONL serves this with `tail -n 1000 fork_log.jsonl | jq 'select(.timestamp_ms > X)'`. The "indexed range scans by (height, slot, producer, peer, kind)" argument assumes queries the agent will never actually issue. INC-I-083 was diagnosed by TIME RANGE (what happened between 21:30 and 22:51), not by producer lookup or slot index. RocksDB CF introduces: (a) a migration story when schema changes (the codebase has `create_missing_column_families(true)` in `state_db/open.rs:17` but zero compaction filter precedent), (b) coupling to the state_db lifecycle (snap sync wipes state_db — would it wipe the fork ledger too?), (c) complexity for retention (no TTL or compaction filter exists anywhere in this codebase per grep). JSONL gives: trivial offline copy, grep-compatible, no DB migration, independent of state_db wipe.
- **Evidence**: `state_db/open.rs` has 6 CFs with `create_missing_column_families(true)` — adding a 7th is mechanically easy but creates a coupling where snap-sync wipes (`state_db/writes.rs:164` iterates deletable CFs) could destroy the fork ledger. Zero compaction filter hits in the entire codebase. The dimension-score-c.md explicitly flags this: "No existing compaction filter or TTL pattern exists in the codebase."
- **Risk if analyst's framing wins**: A schema change in Phase 2 requires a migration strategy that does not exist. The fork ledger gets wiped during snap sync because it shares the state_db. Novel retention infrastructure adds 400+ LoC for something `logrotate` handles for free.

## Reframe 4: The Classifier's "Unknown" Will Dominate
- **Counter-claim**: The proposed variants are: `tip_race_natural`, `tip_race_high_latency`, `producer_equivocation`, `epoch_boundary_invalid`, `post_snap_dead_tip`, `validation_disagreement`, `rollback_loop`, `snap_sync_to_minority_fork`, `unknown`. INC-I-082 was a bit-level rebuild defect in `rebuild_epoch_state_from_blocks` — no variant covers "epoch state rebuild produces non-identical results." INC-I-083's actual root cause was "classify() has no escalation path for dead-fork nodes" — no variant covers "recovery state machine coverage hole." The classifier classifies SYMPTOMS (what kind of fork), not ROOT CAUSES (why recovery failed). Every novel incident — which is every incident that matters — will return `unknown`.
- **Evidence**: The domain-diagnosis-report.md explicitly states: "The root cause is NOT the natural tip race. It is a trigger — any event that puts a node on a minority fork produces the same cascade. The root cause is the classify() coverage hole." The classifier would see the tip race and return `tip_race_natural` or `post_snap_dead_tip` while the actual problem is a code defect. The classifier gives false confidence.
- **Risk if analyst's framing wins**: The agent trusts `tip_race_natural`, recommends "wait for self-recovery," and the node stays frozen for hours — exactly what happened when INC-I-083 was initially dismissed.

## Reframe 5: The Emitter Instruments the Wrong Layer
- **Counter-claim**: The Phase 1 emitter list targets `apply_block`, `block_handling`, `fork_recovery`, `rollback`, `recovery.classify`, `sync_engine response`, `snap_sync`. But the fork DECISION happens in `classify_gossip_block()` at `block_handling.rs:42-94` BEFORE apply_block is called. The `apply_block` path only runs for `ExtendsTip` blocks. The critical fork signals — `ForkBlock(HeightOccupied)`, `Orphan`, `Rejected`, `ReorgCandidate` — all return BEFORE apply_block. Instrumenting apply_block captures "block applied successfully" but misses "fork block dropped at height X because canonical had better slot" (line 168-202) and "orphan chased from peer Y" (line 205-227). The `[FORK_GUARD]` log at line 184 is the actual fork signal — and it is a free-text `info!()` line, not a structured event.
- **Evidence**: `block_handling.rs:162-321` — the `match class` dispatch handles all four `BlockClass` variants. Only `ExtendsTip` falls through to `apply_block`. The `ForkBlock(HeightOccupied { is_better: true })` branch at line 178-188 calls `signal_stuck_fork()` — this is the EXACT signal that INC-I-083's domain-diagnosis-report.md identifies as "dead code" (line 14 of the report: "signal_stuck_fork() fires but has ZERO production callers"). The emitter MUST capture classify_gossip_block decisions, not just apply_block outcomes.
- **Risk if analyst's framing wins**: The emitter records that blocks were applied but not that fork blocks were dropped, orphans were chased, or reorg candidates were evaluated. The most diagnostic events are invisible.

## Reframe 6: The User's Mandate Is Rhetorical, Not Architectural
- **Counter-claim**: "Sky's the limit" and "fuck humans" are emotional responses to INC-I-083 frustration, not architectural directives. The user also has behavioral learning `feedback_genesis_reset_needs_approval.md`: "Even under 'full autonomy on local testnet,' genesis reset ALWAYS needs explicit approval." The user grants broad mandates then enforces specific constraints. The feature-evaluation.md already down-scoped the "sky's the limit" framing from a 3-4k LoC maximalist system to a phased delivery with Phase 1 at ~1.5-2k LoC. The real need is incremental: structured log lines + one aggregation RPC would solve 70% of the pain.
- **Evidence**: The feature-evaluation.md scored D3 (Complexity) at 2/5 and recommended phasing specifically because the full scope was "disproportionate." The user accepted the phased scope, not the maximalist one.
- **Risk if analyst's framing wins**: The analyst builds for the rhetorical mandate instead of the accepted scope, over-engineers Phase 1, and delays the 70% solution while pursuing the 100% solution.

## Reframe 7: Push Beats Pull — The Right Metric Is "Alert Without Asking"
- **Counter-claim**: The "5-second answer via RPC" success test assumes the operator KNOWS to ask. But INC-I-083 ran for ~2 hours before diagnosis began. The real pain is not query latency — it is detection latency. The `[HEALTH]` line in `periodic.rs:898` already runs every tick. If it emitted a structured `FORK_DETECTED` event when `sync_fails > N` or state-root divergence is detected, the system would TELL the operator instead of waiting to be asked. The existing `fork-monitor.sh` is a pull script. The existing guardian system (`guardian.rs`) already does fleet polling. Neither pushes alerts.
- **Evidence**: INC-I-083 timeline: deploy at 20:16, deadlocks emerged ~21:30-22:00, snapshot at 22:51. That is 1-2 hours of undetected degradation. A push alert at the 60-second mark (when `recently_synced()` fails) would have saved the entire investigation. The periodic task at `periodic.rs` already runs every ~1s and has the health data.
- **Risk if analyst's framing wins**: The system answers fast when asked but nobody asks for 2 hours. The 5-second RPC is irrelevant if the 2-hour detection gap is not closed.

## Recommended Scope Adjustments (if I were the analyst)
1. Instrument `classify_gossip_block` in `block_handling.rs` as a FIRST-CLASS emit point — not just apply_block. Every `BlockClass::ForkBlock` and `BlockClass::Orphan` decision must emit.
2. Store the fork ledger in a SEPARATE RocksDB database (not a CF in state_db) or use JSONL, to avoid snap-sync wipe coupling and migration complexity.
3. Include the full `RecoveryContext` (all 12+ fields) in every `recovery_classify_call` event, not just the action output — otherwise the bundle is not self-contained.
4. Add a push-alert mechanism (even just a log line checked by the periodic task) that fires when fork-event rate exceeds baseline — do not rely solely on pull RPC.
5. Require `--human` flag to produce a summary the operator can audit after the agent produces its verdict.

## What the Analyst Will Likely Miss
1. **The snap-sync wipe problem**: adding a CF to state_db means fork history gets destroyed whenever a node snap-syncs — exactly when you need it most.
2. **classify_gossip_block as the primary emit site**: the brief says "apply_block" but the fork decisions happen upstream. The analyst will instrument the wrong function.
3. **RecoveryContext capture**: the bundle spec says "recovery_classify_call" but does not specify capturing the INPUT context, only the OUTPUT action. Without the 12-field context, INC-I-083 is undiagnosable from the bundle alone.
4. **Detection latency vs. query latency**: the 5-second metric solves the wrong problem. The 2-hour gap between fork and diagnosis is the real cost driver.

---

Key files referenced:
- `bins/node/src/node/block_handling.rs` (lines 42-94: classify_gossip_block; lines 162-321: dispatch)
- `crates/network/src/sync/manager/recovery.rs` (lines 250-363: classify with 12+ field RecoveryContext)
- `crates/storage/src/state_db/open.rs` (6 CFs, create_missing_column_families, no TTL/compaction filter)
- `crates/storage/src/state_db/writes.rs` (line 164: deletable_cfs list that snap-sync wipes)
- `bins/node/src/node/periodic.rs` (line 898: [HEALTH] structured log)
- `docs/.workflow/domain-diagnosis-report.md` (root cause: classify() coverage hole, not tip race)
