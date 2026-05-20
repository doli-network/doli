# Dimension Scores: Group A

## Feature
Agent-consumable fork-diagnostic subsystem (emitter, RocksDB-CF ledger, bundle RPCs, classifier, CLI, replay tool) — workflow #346

## Scores

### D1: Necessity — Score: 5
**Assessment**: The pain is systemic, recurring (30+ incidents over 3 months), high-cost per occurrence (~2-4h each), and actively blocking project velocity.
**Evidence**:
- MEMORY.md incident table: INC-I-009 through INC-I-083 (74 incident IDs across 3 months; at least 10 are explicitly fork/BEHIND/sync-related: INC-I-009, -010, -012, -014, -016, -041, -054, -068, -081, -082, -083)
- INC-I-083 (`docs/.workflow/inc-i-083-session-handoff.md`): required 4 parallel investigators + synthesizer + ~2h, and STILL initially misdiagnosed ("bundle is innocent"). The domain-diagnosis-report.md is 287 lines of cross-domain causal chain analysis for what should be a trivial diagnostic.
- INC-I-081 (`MEMORY.md`): 5-commit hotfix, fleet-wide fork, required multi-investigator cascade analysis.
- Current workaround cost: grep across 18 log files (some 1.9 GB), manual timestamp correlation across nodes, multiple RPC polls, re-reading 360-line `recovery.rs`, reconstructing causality from free-text lines missing producer/peer attribution.
- What breaks if NOT built: every future fork incident continues to cost 2-4h of multi-agent investigation. The operator stated: "the analyses are always horribly depressing."
**Counter-evidence (anti-anchoring)**: The INC-I-083 domain-diagnosis system (4 parallel investigators) DID eventually produce correct root-cause analysis. The problem is time/cost, not impossibility. Some incidents are code bugs that observability alone cannot prevent.
**Reasoning**: This is a clear 5. The pain is not hypothetical — it is measured at 30+ occurrences over 3 months with documented multi-hour costs per incident. The workaround (multi-agent grep + timestamp correlation) exists but is explicitly failing (wrong conclusions, 2h latency, operator burnout). The frequency is roughly weekly, not monthly.

### D4: Alternatives — Score: 4
**Assessment**: Existing alternatives are individually weak and collectively insufficient, but the gap between "better scripts" and "full subsystem" deserves acknowledgment.
**Existing partial solutions**:
- `scripts/fork-monitor.sh` — detects tip divergence across nodes (exit code 1 = fork), but only reports THAT a fork exists, not WHY/WHO/WHEN
- `getStateRootDebug` RPC — returns height/hashes per node but no causal context, no event history, no classification
- `getChainInfo` / `getEpochInfo` RPCs — state queries with no fork attribution
- `scripts/health-check.sh` / `scripts/status.sh` — fleet status snapshots
- Guardian system (`crates/rpc/src/methods/guardian.rs`) — peer chain-tip polling but no fork event history
- Free-text log lines (`[FORK_GUARD]`, `[HEADER_DEBUG] Chain break`, classify() logs) — present but unstructured, missing producer/peer/causality fields
- `/omega-swarm --deep` multi-investigator pipeline — actually works (produced correct INC-I-083 diagnosis) but costs ~2h and spawns 4+ agents
**Why proposed feature IS strictly better**:
- No existing tool answers "which producer caused this fork" or "how many forks per hour vs baseline" in a single query
- The existing log lines lack: `from_peer_id`, `caused_by_event_id`, correlation keys, classification enum
- The multi-agent workaround costs 2h; the proposed system targets 5s (single RPC)
- RocksDB-CF indexed queries replace grep across 1.9GB log files
**Counter-evidence**: A simpler version (structured JSONL log + better grep scripts + a small "fork summary" RPC) could deliver 60-70% of the value at 30% of the cost. The full 4-layer architecture (emitter + RocksDB CF + 3 bundle RPCs + deterministic classifier + CLI + replay tool) may be overengineered relative to a "structured log + single aggregation RPC" approach. The `/omega-swarm --deep` pipeline, while slow, does produce correct answers — the subsystem converts a 2h task to 5s, which is transformative but not the difference between possible and impossible.
**Reasoning**: Score of 4 (not 5) because a simpler intermediate solution exists — even just adding structured fields to existing log lines plus one aggregation RPC would significantly reduce the pain. The full proposed system is the optimal solution, but it's not the ONLY solution. The existing fork-monitor.sh + getStateRootDebug + manual grep pipeline partially works; it just costs 100x more time.

## Notes
- The frequency evidence is strong: ~10+ fork-specific incidents documented in 3 months = roughly one per week requiring multi-hour diagnosis.
- The "agent-first" framing (target consumer is Claude sub-agents, not humans) is a force multiplier — the same observability infrastructure that helps the operator also makes automated incident response possible.
- The necessity score would drop to 4 only if the fork-recovery code fixes (INC-I-083 root cause in classify()) reduced fork frequency by 90%+, but the brief explicitly notes that natural tip races are routine PoS behavior and will continue to occur.
