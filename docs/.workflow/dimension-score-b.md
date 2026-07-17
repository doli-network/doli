# Dimension Scores: Group B

## Feature
Node-level disk-space self-protection: when free disk runs low, `doli-node` halts production and emits a structured "disk 98% full, production halted" log/error instead of ABRT/ENOSPC crash-looping (risking mid-write state corruption). Auto-recovers when space is reclaimed. Non-consensus, no activation height.

## Scores

### D2: Impact — Score: 4
**Assessment**: Prevents a real, already-materialized failure (nano ABRT) and its expensive downstream cost (state corruption → wipe+resync), with high per-incident value but a bounded beneficiary set.
**Evidence**:
- Origin incident is not hypothetical: external mainnet producer "nano" ABRT core-dumped in a crash-loop from a 100%-full disk (29G unrotated log). Impact is proven, not speculative.
- Downstream cost avoided is large: an ENOSPC abort mid-write to RocksDB risks state divergence; DOLI's established remediation for integrity divergence is a full wipe + snap-resync + backfill from one canonical seed (MEMORY: `feedback_deploy_integrity`, `feedback_cascade_recovery`). Turning signal-6 into a clean halt removes that tail risk.
- Diagnosability gain is concrete: the incident required manual digging to find the 29G log; a structured "disk 98% full, production halted" message collapses operator diagnosis time from spelunking to a single log line.
- Beneficiary scope is bounded: ~17-node external `/producers` fleet on small unmonitored VPSs. The structural fleet (N1-N12 + seeds) already has Prometheus/Grafana/Alertmanager, so the incremental value there is near zero.
**Reasoning**: Value is immediate (addresses an incident that already happened) and prevents a genuinely expensive corruption-driven recovery path, so this is clearly above midpoint. It is capped below 5 because it is defensive tail-risk mitigation for a subset of operators rather than a multiplier/platform capability others build on, and the highest-value nodes are already covered by external monitoring. Solid, targeted robustness value → 4.

### D5: Alignment — Score: 5
**Assessment**: Textbook fit — the "gracefully stop producing to protect self/network" idiom already exists in the exact files this feature touches, and the non-consensus voluntary-non-production model maps onto existing safe-degradation behavior.
**Evidence**:
- `bins/node/src/node/production/mod.rs::try_produce_block` already implements two self-protecting halts of exactly this shape: version-enforcement halt (`node_updater::is_production_allowed`, rate-limited warn) and hardfork-schedule halt (`should_stop_producing`, "stop producing to avoid poisoning the network"). A disk-health gate is a third instance of an established pattern, not a new paradigm.
- `bins/node/src/node/production/gates.rs` documents a "Single source of truth for production safety" with a `BlockedExplicit { reason }` authorization variant — a natural, idiomatic insertion point for a "disk low" reason.
- Periodic resource-health monitoring is a precedent: `crates/storage/src/utxo_size_monitor.rs` (F1 snap monitor, cached gauge with 30/60s TTL) and `bins/node/src/node/checkpoint_health.rs` establish `periodic.rs` as the home for cached health checks — the exact place a disk-space poll belongs.
- Ethos match: CLAUDE.md's Stability Pillars and defensive-robustness posture, plus MEMORY's explicit acknowledgment that the network runs many external, unmonitored producers (`feedback_external_producers_need_activation_height`), make node-level self-protection squarely on-vision for a decentralized producer set.
- Architecturally clean: voluntary non-production reuses existing missed-slot behavior; no consensus rule change, no activation height, no block-content change — it stays entirely inside the observability/robustness envelope and cannot fork the chain.
**Reasoning**: The feature does not merely fit the architecture; it clones an idiom already present in the target file and reuses an existing periodic-monitor precedent, so integration risk to project direction is minimal. Because it also advances the stated vision (robustness for external operators) without touching consensus, alignment is maximal → 5.

## Notes
- D2 and D5 are decoupled here: alignment is near-perfect (5) but impact is scope-bounded (4) because the most valuable nodes already have external monitoring — the orchestrator should not let the strong D5 fit inflate perceived reach.
- Cost/risk (D3/D6) and timing (D7) are out of my group; I note only that the existing gate patterns suggest a low-surface-area implementation, but I did not score that.
- The CLAUDE.md phrase "plan graceful degradation" was not found verbatim, but the equivalent ethos (Stability Pillars, "stop producing to avoid poisoning the network") is present and directly on point.
