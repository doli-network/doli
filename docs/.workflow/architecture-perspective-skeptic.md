# Architecture Perspective: Skeptic — Workflow #346

## Perspective
Skeptic — attack proposed architectures for failure modes, hidden assumptions, and edge cases.

## What I Don't Understand
1. The actual disk I/O profile of the slowest mainnet node (ai3). The 50us claim is benchmarked in CI, not on production hardware.
2. Whether `rocksdb::DB::open` for a SECOND RocksDB instance on the same disk creates file-lock contention with the existing `state_db` or `block_store` instances at the OS level (RocksDB uses `flock`).
3. How `periodic.rs` at 1793 lines will accommodate a pruner task without violating the 500-line module budget or requiring a split that touches the run_periodic_tasks dispatch table.
4. Whether the `ulid` crate's monotonic counter persists across process restarts or resets to zero (the spec says "per process" but the restart edge case needs verification).

---

## Attack 1: "No consensus impact" claim

### The claim
Emit calls inside apply_block are fire-and-forget (`let _ = ledger.record(...)`), cannot alter control flow, and therefore cannot affect consensus.

### Failure modes

**FM-1a: Synchronous RocksDB write blocks apply_block past slot deadline.** Sync `record()` writes (REQ-FORKOBS-LEDGER-006) at 50us target. RocksDB sync latency is disk-dependent and bursty. Under WAL fsync, a single write can spike to 5-20ms on HDD or degraded SSD. If a producer's apply_block for its OWN block takes >10s (slot period), it misses the slot. Not a consensus rule change, but a liveness impact caused by diagnostic code. INC-I-081 proved that missing a single slot at an epoch boundary can cascade. conf(0.5, inferred).

**FM-1b: OOM from ULID or bincode allocation in constrained environments.** `ulid` allocates; `bincode::serialize` allocates a Vec per event. Under memory pressure (INC-I-009: 86GB RAM from Yamux), these allocations push over the edge. `let _ =` catches Err but not a panic from the allocator. conf(0.2, inferred).

**FM-1c: The `last_block_source` side-channel creates temporal coupling.** `handle_new_block` sets the field BEFORE calling `apply_block`. If apply_block is called from ANY other path (sync, replay, execute_reorg at block_handling.rs:264), the field holds stale data. Emitter reads stale provenance, emits wrong from_peer_id. conf(0.6, observed) — execute_reorg at line 264 calls self.apply_block without going through handle_new_block.

### Severity
FM-1a: Medium (liveness, not safety). FM-1c: High for diagnostic correctness.

### Mitigation required
- **Mandatory: async mpsc as DEFAULT, not as fallback.** Sync path is the risk; make async primary.
- **Mandatory: audit ALL call sites to apply_block** and clear last_block_source to None before non-gossip calls. Evidence: ≥4 paths bypass handle_new_block (execute_reorg, try_apply_cached_chain, snap sync completion, replay mode).

---

## Attack 2: "Safe for rolling deploy" claim

### Failure modes

**FM-2a: Node struct gains new fields → restart required.** `diagnostic_ledger`, `last_block_source`. During rolling deploy, nodes restart one at a time. New binary opens a second RocksDB at `data/diagnostics/`. Crash during diagnostic DB open (corrupt FS, permissions) → node fails to start. Pre-deploy, this failure mode did not exist. conf(0.4, inferred) — mitigated by REQ-FORKOBS-LEDGER-009 IF graceful degradation actually works.

**FM-2b: RPC inconsistency during rolling deploy window.** Node A (new) responds; Node B (old) returns method-not-found. Automated diagnostic agent querying the fleet gets inconsistent results. Acceptable but undocumented. conf(0.5, observed).

### Severity
Low. Rolling deploy is genuinely safe for consensus. RPC inconsistency is cosmetic.

### Mitigation
- Document that `getForkDiagnostic` may return method-not-found during rolling deploy; CLI/agent must handle.

---

## Attack 3: "<50us emit latency" claim

### Failure modes

**FM-3a: RocksDB `put_cf` with WAL is not 50us on spinning disk.** 10-50us on NVMe, 100-500us on SATA SSD, 2-20ms on HDD. Benchmark runs on CI (likely NVMe). ai3 disk profile unknown. conf(0.5, inferred).

**FM-3b: The async mpsc fallback has undecided backpressure semantics.** REQ-FORKOBS-PERF-001 says "switch to async mpsc" but doesn't specify bounded/unbounded, capacity, full-channel policy (drop oldest/newest/block). Dropping silently makes ledger unreliable. Blocking defeats the purpose. **Underspecified — correctness question, not perf.** conf(0.6, inferred).

**FM-3c: 50us is per-emit, but apply_block has multiple emit sites.** REQ-FORKOBS-EMIT-001 (block_applied) + EMIT-002 (block_rejected). Epoch boundary blocks also trigger post_commit emits. 3 × 50us = 150us added to every epoch boundary. conf(0.4, inferred) — still small vs 10s slots.

### Severity
Medium. Real risk is FM-3b: undefined backpressure.

### Mitigation
- Architect MUST specify: bounded(1024), drop-oldest, dropped-event counter in health RPC.
- Default to async mpsc, not sync.

---

## Attack 4: "Graceful degradation" claim

### Failure modes

**FM-4a: DB opens OK, then fails mid-operation (disk full, corruption).** REQ-FORKOBS-LEDGER-009 covers open-failure case. Does NOT cover: `record()` returns Err on a specific write, DB handle becomes invalid after RocksDB internal corruption, disk fills up. Every single emit site must handle Err without panicking. ~13 emit sites across 5+ files — surface area problem. If even ONE uses `.unwrap()` or `?`, the node crashes. conf(0.5, inferred).

**FM-4b: Silent write failure (readonly mount, permission change).** DB opens read-write on startup. Later, filesystem permissions change. All writes silently fail. RPC returns empty bundles. Agent concludes "no forks observed" when forks ARE occurring. **Worse than no diagnostic system — provides false negative assurance.** conf(0.3, inferred).

### Severity
FM-4a: High. FM-4b: Medium (false negatives).

### Mitigation
- **"Write canary" mechanism**: pruner task writes a heartbeat event. If missing from last N minutes, RPC returns a WARNING field indicating "diagnostic writes may be failing."
- All emit sites MUST use `let _ =` or equivalent. Reviewer MUST verify as merge blocker.

---

## Attack 5: "ULID ordering" claim

### Failure modes attempted

**FM-5a: ULID counter reset on restart.** In-memory counter. On restart, counter resets. If restart within same millisecond (unlikely but possible), two events could have identical timestamps with counter ordering not reflecting causal order. conf(0.2, inferred) — vanishingly unlikely.

**FM-5b: Cross-node correlation_key JOIN fails under clock skew.** Correlation_key is `(divergence_height, canonical_hash, fork_hash)` — height-based, NOT timestamp-based. SURVIVES clock skew. ULID timestamp is only for intra-node ordering. conf(0.1, observed) — the design is actually correct here.

### Kill test
Tried to construct a clock-skew failure but correlation_key is hash-based. The spec is sound on this point.

### Severity
Low. The ULID ordering claim is essentially correct for the use case. **Note this as a STRENGTH.**

---

## Attack 6: "Classifier is deterministic" claim

### Failure modes

**FM-6a: Rule precedence is underspecified for overlapping conditions.** Rule (a): same height same producer = ProducerEquivocation. Rule (e): validation_duration > 2000ms = TipRaceHighLatency. A producer equivocates AND validation takes 3000ms. Both match. Spec says "deterministic, match-based" but doesn't say first-match-wins or most-specific-wins. conf(0.6, observed) — explicitly lists 7 rules without precedence.

**FM-6b: Rule (f) "no other signals" is undefined.** TipRaceNatural requires "validation_duration < 500ms and no other signals." What constitutes "other signals"? Implementer decides → different implementations produce different classifications for identical inputs. Violates "deterministic." conf(0.5, inferred).

**FM-6c: Rule (d) is temporally fragile.** "snap_sync_completed followed by fork_block_received" = PostSnapDeadTip. But "followed by" in what time window? 6 hours? Spec doesn't specify temporal proximity. conf(0.4, inferred).

### Severity
Medium. Classifier produces inconsistent results across implementations until precedence and temporal windows are nailed down.

### Mitigation
- Architect MUST specify: rules evaluated in listed order, first match wins (recovery.rs:classify() precedent at lines 252-363).
- Define "no other signals" as "no other fork-classified events within the same query window."
- Define temporal proximity for rule (d): "snap_sync_completed within the last 300 seconds."

---

## Attack 7: "Unknown variant escalates safely" claim

### Failure modes

**FM-7a: "Unknown" is the expected output for every novel incident.** INC-I-082 (bit-level rebuild defect), INC-I-083 (recovery classify() coverage hole) — neither maps to any of 8 named variants. Classifier returns Unknown with vague evidence. Agent reads "Unknown" and... does what? UX degrades to "call getForkDiagnostic, get Unknown, fall back to grepping logs." conf(0.6, inferred).

**FM-7b: Unknown is the CORRECT answer.** The alternative — misclassifying INC-I-083 as TipRaceNatural and recommending "wait" — is actively harmful. Unknown with evidence is strictly better than wrong specific verdict. Architect should explicitly state Unknown+evidence is the DESIGNED outcome for novel incidents; system's value is in evidence capture, not classification. conf(0.6, inferred).

### Severity
Low for safety. Medium for the "5-second diagnosis" promise.

### Mitigation
- Reframe success metric: primary value is structured evidence capture (eliminating grep), not automated classification. Classification is bonus for known patterns.

---

## Attack 8: "Retroactive validation against INC-I-083/081" claim

### Failure modes

**FM-8a: INC-I-083 would produce `Unknown`, not a specific verdict.** Root cause was classify() coverage hole — no recovery action for dead-fork nodes. Events captured: block_applied (stopped), recovery_classify_call (returning None repeatedly), fork_block_received (if fork blocks arrived). Classifier sees "recovery returning None" → nothing in 8 named variants. PostSnapDeadTip requires snap_sync_completed, but INC-I-083 nodes had `--no-snap-sync`. Correct output: `Unknown(reason="recovery_classify returning None repeatedly with deep_fork evidence and no escalation")`. Is that "correctly diagnosed"? **Correctly TRIAGED — evidence points to recovery.rs. But spec says "correct verdict" and Unknown is technically not a verdict.** conf(0.5, inferred).

**FM-8b: INC-I-081 maps cleanly to EpochBoundaryInvalid.** Broken producer emitted block missing EpochReward. Rejecting nodes emit block_rejected with reason "missing EpochReward." Rule (b) fires. **Genuine win.** conf(0.65, observed).

**FM-8c: Retroactive validation is unfalsifiable on paper.** Phase 2 (replay tool) does the real test. Paper defense argues what WOULD happen with data that doesn't exist. The real test is whether emitter captures enough context at each decision point. INC-I-083 needed full RecoveryContext (12 fields). REQ-FORKOBS-EMIT-007 captures all 12. **Adequate — but only because skeptic reframe 2 was incorporated.** conf(0.55, observed).

### Severity
Low for INC-I-081. Medium for INC-I-083 (Unknown is honest but undersells).

### Mitigation
- For INC-I-083: explicitly state expected output is `Unknown` with rich evidence; the evidence (recovery_classify_call with `deep_fork=true, snap_attempts=0, action=None`) points a diagnostic agent to recovery.rs in under 60 seconds, even without a named variant.
- Accept that "correctly diagnosed" means "correct triage direction" for novel incidents, not "named enum variant."

---

## Top 3 architecture decisions the architect MUST resolve to ship safely

1. **Async-first emit, not sync-with-fallback.** Sync RocksDB write is primary risk to apply_block liveness. Specify bounded channel(1024), drop-oldest, dropped-event counter in health RPC. Do not defer to "benchmark results." conf(0.65, inferred).

2. **Audit ALL apply_block call sites for last_block_source correctness.** At minimum 4 paths bypass handle_new_block: execute_reorg (block_handling.rs:264), try_apply_cached_chain, snap sync completion, replay mode. Each must clear or correctly set last_block_source, or emitter produces garbage provenance. conf(0.6, observed).

3. **Nail down classifier rule precedence and temporal windows.** First-match-wins (matching recovery.rs:classify() precedent). Define "no other signals." Define temporal proximity for PostSnapDeadTip. Without these, "deterministic" claim is false. conf(0.6, inferred).

---

## Key Evidence

- `bins/node/src/node/apply_block/mod.rs:37` — apply_block is async, takes `&mut self`, the hot consensus path.
- `bins/node/src/node/block_handling.rs:264` — execute_reorg calls self.apply_block directly, bypassing handle_new_block. Stale `last_block_source` risk.
- `bins/node/src/node/block_handling.rs:42-94` — classify_gossip_block is the actual fork decision point.
- `crates/network/src/sync/manager/recovery.rs:130-152` — RecoveryContext has 12 fields. REQ-FORKOBS-EMIT-007 correctly mandates capturing all of them.
- `crates/storage/src/state_db/open.rs:21` — WAL recovery mode PointInTime already set.
- `crates/storage/src/state_db/writes.rs:164-178` — snap sync's `atomic_replace` wipes 4 CFs but NOT cf_meta. Separate diagnostic DB correctly avoids this blast radius.
- `bins/node/src/node/periodic.rs` — 1793 lines. Pruner pushes further over the 500-line budget.

## Cross-Perspective Signals
- Explorer may propose JSONL as simpler alternative. Skeptic notes snap-sync wipe risk is already mitigated by SEPARATE RocksDB instance (not a CF). JSONL trades query capability for simplicity — tradeoff is real but RocksDB risk is lower than earlier skeptic analysis suggested.
- Analogist may find parallels to Prometheus-style metric recording. Note Prometheus uses custom TSDB, not RocksDB, and write path is fire-and-forget with bounded memory.

## Gaps
- Cannot verify actual disk IO latency on production nodes without access to mainnet servers.
- Cannot verify whether opening a second RocksDB instance on same volume causes OS-level file lock contention.
- periodic.rs module split strategy is unanalyzed.
- The `ulid` crate's specific monotonic counter behavior across restarts was inferred from documentation, not measured.
