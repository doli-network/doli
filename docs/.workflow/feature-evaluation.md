# Feature Evaluation: Node-Level Low-Disk Self-Protection

## Feature Description
Node-level self-protection for `doli-node`: when free disk space runs low, the node
gracefully **halts block production** and emits a structured error/log (e.g. "disk 98%
full, production halted") instead of ABRT/ENOSPC crash-looping (signal 6) and risking
mid-write RocksDB state corruption. Auto-recovers (resumes production) when space is
reclaimed. NON-consensus, no activation height — voluntary non-production is provably
equal to an existing missed slot.

**Origin**: Live mainnet external producer "nano" ABRT core-dumped in a systemd
crash-loop; root cause was a 100%-full disk (29G unrotated log on a 38G volume).
**Beneficiaries**: ~17 unmonitored external VPS producers (structural N1-N12 + seeds
already have Prometheus/Grafana).

*Evaluated in Parallel Mode from three independent dimension-scorer outputs
(`dimension-score-a/b/c.md`), eliminating halo effects between dimension groups.*

## Evaluation Summary

| Dimension | Score (1-5) | Assessment |
|-----------|-------------|------------|
| D1: Necessity | 4 | Proven live failure; the abort/corruption failure-MODE is the real defect, contained network severity |
| D2: Impact | 4 | Prevents a materialized crash + expensive wipe+resync tail; bounded to unmonitored fleet |
| D3: Complexity Cost | 4 | Small, `Node`-local; reuses gate + periodic-monitor patterns; residual = hysteresis + correct mount + gauntlet |
| D4: Alternatives | 4 | Partial external mitigations exist but none reach the "installs nothing" tail or prevent the unclean abort |
| D5: Alignment | 5 | Clones an idiom already in the target file; reuses `utxo_size_monitor` precedent; zero consensus coupling |
| D6: Risk | 4 | Net risk-REDUCING (crash→clean missed-slot); only downside is recoverable reward loss from aggressive threshold |
| D7: Timing | 5 | Live recurring cause, all prerequisites met, no activation/version/cross-crate dependency |

**Feature Viability Score: 4.3 / 5.0**

Calculation: `[(D1+D2+D5)×2 + (D3+D4+D6+D7)] / 10 = [(4+4+5)×2 + (4+4+4+5)] / 10 = [26+17]/10 = 4.3`

## Verdict: GO

Build it. FVS 4.3 clears the GO threshold (≥4.0), no dimension scored 1 (no override
cap), and no NO-GO override applies. Three independent scorers converged on 4-5 across
all seven dimensions with cited code evidence — the feature converts a signal-6
crash-loop that can corrupt state mid-write into an already-traveled, deterministic
missed-slot path, using machinery (`BlockedExplicit` halt gate, `fs2` primitive,
`utxo_size_monitor.rs` precedent) that already exists.

## Detailed Analysis

### What Problem Does This Solve?
A real, in-session, mainnet-materialized failure: an unmonitored external producer
crash-looped and core-dumped because its disk filled. The true defect is not the log —
it is `doli-node` aborting mid-write under ENOSPC (risking RocksDB corruption) rather
than degrading gracefully. Log rotation fixes only the proximate cause; chain growth on
small VPSs makes ENOSPC eventually inevitable, so a clean pre-write halt is strictly
safer for the ~17-node unmonitored tail.

### What Already Exists?
Everything structural is already present. The clean-halt gate
(`sm.block_production(reason)` / `ProductionAuthorization::BlockedExplicit`) in
`bins/node/src/node/production/gates.rs` already halts-with-reason and clears on resume.
`fs2` (already a dependency, used in `producer/guard.rs`) exposes `available_space()`.
`crates/storage/src/utxo_size_monitor.rs` and `periodic.rs` (DEFI_HEALTH_CACHE_TTL=30s)
are direct precedents for a TTL-cached in-node health poll. Grep confirms **no** existing
free-disk check anywhere — this is net-new logic, but every ingredient is copy-adaptable.

### Complexity Assessment
Blast radius is `Node`-only. Implementation shape: poll the `data_dir` filesystem in
`periodic.rs` behind a TTL cache + `AtomicBool`; gate in `production/gates.rs`; threshold
tunable in `NetworkParams`; structured `tracing::warn!`. Resume is implicit — the next
poll re-clears the flag when space returns, so no new state machine. Residual cost above
trivial: (1) hysteresis / min-dwell to avoid flap near threshold, (2) polling the correct
mount (the RocksDB `data_dir`, which may differ from the log mount that filled "nano"),
(3) the `gauntlet.conf`-mandated failure-mode matrix + gauntlet run before close.
**Maintenance burden: low and ongoing-negligible** — one cached poll and one early-return
gate mirroring ~6 sibling gates; no new dependency, no schema, no cross-crate surface.

### Risk Assessment
Dominant effect is risk REDUCTION. Voluntary production-pause is provably identical to a
missed slot (`production/mod.rs` L280-296: "the slot is empty… ALWAYS deterministic") and
does NOT deregister the bond or mutate the on-chain active producer set (derived at epoch
boundary via `active_producers_at_height`) — from other nodes' view it is missed slots,
not a Withdrawal/Exit, so it cannot shrink the active set or fork the chain. Residual,
non-consensus risks: (a) false-positive halt → recoverable lost reward (mitigate with
conservative default + hysteresis); (b) prolonged self-halt could trip other nodes'
liveness exclusion (INC-I-016) — but a crash-looping node misses those same slots today,
so this is not a NEW risk and is capped by `MAX_EXCLUSIONS_PER_BLOCK`; (c) cross-mount
subtlety — must poll `data_dir`, which is precisely what prevents corruption.

## Conditions
None — feature approved for pipeline entry.

The following are **implementation guidance for downstream agents** (not gating
conditions), surfaced by the scorers and worth carrying into requirements:
- [ ] Poll the `data_dir` (RocksDB) filesystem specifically — NOT the log mount that
      filled "nano" — since preventing mid-write corruption is the load-bearing goal.
- [ ] Add hysteresis / minimum dwell time to prevent production flap near the threshold.
- [ ] Keep the periodic poll additive and isolated in `periodic.rs` — that file is under
      active churn (INC-I-139 snap-admission, in-flight INC-I-138/120 recovery work), so
      merge-friction is the only caution flagged. Different concern, no consensus coupling.
- [ ] Threshold tunable lives in `NetworkParams`, not a global constant.
- [ ] Run the mandated gauntlet failure-mode matrix + gauntlet before workflow close
      (`gauntlet.conf` present).

## Alternatives Considered
- **logrotate**: fixes only the proximate log cause, not inevitable chain growth on small
  volumes; does not convert the abort into a clean halt.
- **Host disk monitoring / Prometheus**: exactly what the target population lacks by
  definition — the structural monitored fleet already has it and gains near-zero.
- **systemd OnFailure / quotas**: can restart, but restart-into-full-disk reproduces the
  observed crash-loop and does not prevent the unclean abort.
- **External watchdog script**: requires the operator to install it — the same operators
  who, by definition, install nothing.
- Conclusion: only in-node logic reaches the unmonitored tail AND converts ENOSPC
  write-abort into a graceful pre-write halt. External tools are structurally unable to do
  the latter. Feature is incremental defense-in-depth for the "installs nothing" tail.

## Recommendation
Proceed to the pipeline. This is a low-cost, low-risk, high-alignment robustness fix for a
real, already-materialized mainnet failure. It is not a network-critical or platform
feature (hence 4s, not 5s, on Necessity/Impact/Complexity/Alternatives/Risk), but it
cleanly clears the GO bar and reuses existing machinery, so the marginal build is thin and
the safety upside (avoiding mid-write RocksDB corruption → wipe+resync) is concrete. Hand
to the Analyst with the implementation-guidance checklist above.

Note on scoring: all seven dimensions landed at 4-5. This was checked against the
auto-approval anti-pattern — the scores come from three INDEPENDENT scorer contexts
(the structural safeguard against halo inflation), and each Group-A/B held its dimensions
to 4 with explicit "why not 5" reasoning, so the absence of a ≤3 score reflects a genuinely
low-controversy feature, not score inflation.

## User Decision
[Awaiting user: PROCEED / ABORT / MODIFY]
