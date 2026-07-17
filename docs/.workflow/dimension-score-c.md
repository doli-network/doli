# Dimension Scores: Group C

## Feature
Node-level self-protection in `doli-node`: when free disk space runs low, gracefully stop producing blocks + emit structured error/log instead of ABRT/ENOSPC crash (mid-write corruption risk). Auto-recover when space is reclaimed. NOT a consensus-rule change; voluntary non-production == existing missed-slot behavior; no activation height.

## Scores

### D3: Complexity Cost — Score: 4
**Assessment**: Small, localized, and matches an already-established gate+periodic-monitor pattern; only modest extra work from hysteresis, cross-mount correctness, and the mandatory gauntlet run.
**Evidence**:
- `bins/node/src/node/production/mod.rs` `try_produce_block()` already chains ~6 sequential early-return gates that each do `return Ok(())` (version enforcement L40-54, hard-fork stop L59-84, `handle_production_authorization` L101-104, behind-network L174-189, invariant-violation L226-240, startup grace L272-278). A disk-space gate is one more entry of the identical shape — a dedicated `gates.rs` already exists (`production/gates.rs`).
- `bins/node/src/node/periodic.rs` already runs cached, TTL-throttled monitoring reads in `run_periodic_tasks()`: `DEFI_HEALTH_CACHE_TTL = 30s` (L7, L107-129) explicitly modeled on `crates/storage/src/utxo_size_monitor.rs` (30/60s cadence). A `statvfs`/`statfs` free-space poll caching an `AtomicBool` slots directly into this loop.
- Threshold config → `NetworkParams` (per MEMORY: "New activation heights / tunables go directly into NetworkParams"); structured logging via `tracing::warn!` is already pervasive here. Resume logic is implicit: the periodic poll re-clears the flag each tick when space returns — no new state machine required.
**Reasoning**: Grep confirms NO existing disk-space check anywhere in the workspace, so this is net-new, but every ingredient (per-tick cached poll, production early-return gate, NetworkParams tunable, structured log) already exists and is copy-adaptable. The residual cost above "trivial" is: (1) hysteresis / min-dwell to prevent flap near the threshold, (2) checking the correct filesystem (`data_dir` mount where RocksDB writes, which differs from the log mount that filled "nano"), and (3) the `gauntlet.conf`-mandated failure-mode matrix + gauntlet run before close. Blast radius is `Node` only.

### D6: Risk — Score: 4
**Assessment**: Dominant effect is risk *reduction* (avoids mid-write ENOSPC corruption); the pause path is proven consensus-safe; the only genuine downside is recoverable reward loss from a too-aggressive threshold.
**Evidence**:
- Production-pausing is already the codebase's normal, safe behavior: `production/mod.rs` L280-296 documents "If a producer misses their slot, the slot is empty. The next slot's producer takes their turn via the normal deterministic scheduler... ALWAYS deterministic." Every existing gate that returns early == a missed slot. A voluntary disk-halt is byte-identical to those paths.
- Pausing does NOT deregister the bond or mutate the producer set — active-set membership is derived at the epoch boundary from on-chain state (`active_producers_at_height`, L196-201), independent of whether this node emits a block. So the "threshold too aggressive → shrinks the active producer set" concern does not hold from other nodes' view: it looks exactly like missed slots, not a Withdrawal/Exit.
- Hot-path latency is avoidable by construction: the check runs in `periodic.rs` (cached, like defi_health / utxo_size_monitor), and the production-path gate reads only a cached flag — negligible. `statvfs` itself is microsecond-scale.
**Reasoning**: The change is safety-additive: it converts a signal-6 crash-loop that can corrupt RocksDB mid-write into a clean missed-slot, which is an already-traveled, deterministic path. Residual risks are bounded and non-consensus: (a) false-positive halt → lost block reward (recoverable; mitigate with conservative default + hysteresis); (b) prolonged self-halt could eventually trip *other* nodes' liveness exclusion (MEMORY INC-I-016 post_commit exclusion) — but a crash-looping node misses those same slots today, so this is not a NEW risk and is capped by `MAX_EXCLUSIONS_PER_BLOCK`; (c) cross-filesystem subtlety — must poll the `data_dir` mount (RocksDB writes), which is precisely what prevents the corruption, though it won't catch a full *log* partition on a separate mount. None of these can fork the chain.

### D7: Timing — Score: 5
**Assessment**: A live mainnet producer just crash-looped from this exact cause; all prerequisites are met and there are no consensus/activation dependencies.
**Evidence**:
- Origin incident: external mainnet producer "nano" ABRT core-dumped, disk 100% full, 29G unrotated log — the precise failure this feature prevents.
- No prerequisites blocked: no activation height (constraint states voluntary non-production == existing missed-slot), no schema/version bump, no cross-crate coordination. Target files (`production/mod.rs`, `periodic.rs`, `NetworkParams`) are stable, mature, and already host the required patterns. `gauntlet.conf` exists, so the validation harness required to close safely is already in place.
**Reasoning**: The problem is live and recurring on unmonitored external VPS producers, and the fix is self-contained with no gating on other work. One caution, not a blocker: `periodic.rs` is under active churn from the in-flight sync line (recent commits INC-I-139 snap-admission; in-file INC-I-138/120 recovery-coordinator work), so the new block should be additive and isolated to avoid merge friction — but it touches a different concern (disk self-protection) and carries no consensus coupling.

## Notes
- Anti-double-counting honored: D3 scores only build/maintenance effort (new-but-pattern-matched, ~4); D6 scores only breakage/security (net risk-reducing, ~4). The shared "false-positive halt" concern is counted once, under D6, as recoverable reward loss.
- Key consensus-safety fact for the orchestrator: voluntary production-pause is provably equal to a missed slot (`production/mod.rs` L280-296) and does not alter the on-chain active producer set — this is the load-bearing reason D6 is not lower.
- Implementation-shape recommendation surfaced during scoring (not a score input): poll in `periodic.rs` with a TTL cache + `AtomicBool`, gate in `production/gates.rs`, threshold in `NetworkParams`, add hysteresis, and check the `data_dir` filesystem specifically.
