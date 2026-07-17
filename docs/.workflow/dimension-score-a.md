# Dimension Scores: Group A

## Feature
Node-level low-disk self-protection for `doli-node`: when free disk space runs low, halt production and emit a structured error/log (graceful) instead of ABRT/ENOSPC crash (signal 6) with mid-write corruption risk; auto-resume when space is reclaimed. Observability/robustness only — NOT a consensus-rule change, no activation height.

## Scores

### D1: Necessity — Score: 4
**Assessment**: Proven real failure with a real (if narrow) beneficiary population; the failure MODE (uncontrolled abort/corruption) is the genuine problem, not just the log cause.
**Evidence**: Origin incident — external mainnet producer "nano" ABRT core-dumped in a 5-restart systemd crash-loop after a 29G unrotated `/var/log/doli/mainnet.log` filled a 38G volume; near wipe+resync. Beneficiaries: ~17-node `/producers` external fleet on small VPSs with NO Prometheus/Grafana (structural N1-N12 + seeds already monitored). Current code has NO free-disk check anywhere (grep for `available_space`/`fs2::`/`statvfs` finds only `fs2::FileExt` file-locking in `producer/guard.rs:3`).
**Reasoning**: The problem is real and demonstrated in-session, not hypothetical. Log rotation fixes only the log cause, but chain growth on small VPSs makes ENOSPC eventually inevitable, and the true defect is aborting mid-write (ABRT/signal 6) which risks state corruption — a clean halt is strictly safer. Held to 4 rather than 5 because network-level severity is contained: a voluntary producer simply misses slots (existing behavior, no consensus/liveness threat to the fleet), the structural monitored fleet is unaffected, and the scope is operator self-protection rather than network survival.

### D4: Alternatives — Score: 4
**Assessment**: Several partial mitigations exist and would cover many cases, but none reach the target unmonitored-operator population, and none convert the abort failure-mode into a clean halt — that capability is only achievable in-node.
**Evidence**: Alternatives assessed — (a) logrotate fixes only the log cause, not inevitable chain growth; (b) host disk monitoring/Prometheus is exactly what the target population lacks by definition; (c) systemd OnFailure/quotas can restart but restart-into-full-disk reproduces the crash-loop actually observed and does not prevent the unclean abort; (d) external watchdog scripts require the operator to install them — the same operators who install nothing. In-node approach reuses existing infrastructure: `sm.block_production(reason)` / `ProductionAuthorization::BlockedExplicit` clean-halt gate (`bins/node/src/node/production/gates.rs`, RPC `guardian.rs:26`) already halts-with-reason and clears on resume; `fs2` (already a dependency) exposes `available_space()`; `utxo_size_monitor.rs` is an existing precedent for a cached in-node threshold gauge on a 60s TTL.
**Reasoning**: The external alternatives are structurally unable to serve the beneficiaries — every one of them requires operator setup that the unmonitored VPS operators, by definition, will not perform, whereas an in-node watchdog ships in the binary they already run (zero operator action). Additionally, no external tool can turn an ENOSPC write-abort into a graceful pre-write halt; only in-node logic can prevent the mid-write corruption path. Held to 4 rather than 5 because disciplined ops hygiene (logrotate + a disk alert) genuinely prevents the common case, so the feature is incremental defense-in-depth for the "installs nothing" tail rather than the sole possible solution.

## Notes
- D1/D4 both favor building, but as a robustness/observability improvement for a specific unmonitored tail — not a network-critical fix. Orchestrator should weight against Group C cost: the halt/resume machinery and disk-measurement primitive already exist, so the marginal build is thin.
- No new dependency required (`fs2` already present). Clear precedent (`utxo_size_monitor.rs`) for the exact monitor shape.
- Neither D1 nor D4 touches consensus; the CONSTRAINT (voluntary non-production = existing missed-slot behavior, no activation height) is consistent with the code — `BlockedExplicit` is already a non-consensus production gate.
