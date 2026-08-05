━━━ VERDICT — conf(0.96, converged) ━━━

Root cause: The reported "fleet-wide per-node 450 MB → 1.9 GB memory step" does not exist as described — the observed step is HOST-level memory on ai4/ai5 only, and equals (to within 0.4–5 % on three independent measurements) the summed RSS of seven planned node starts (n6–n12, down since Jul 22 11:29 UTC, started 2026-07-30 15:17–15:31 UTC, each snap-syncing 70,753 blocks). The root cause of the incident AS FILED is an observability defect: every `doli_*` application gauge permanently emits 0 because the gauges declared at `bins/node/src/metrics.rs:97` are never updated, and no DOLI process exports `process_resident_memory_bytes` — so host-level and mislabeled-aggregate panels were the only memory signals available, making "fleet-wide per-node leak" the natural reading of a benign, planned event. The reporter's observation was correct; the instrument made correct attribution impossible. The chain itself is healthy: zero fork events since genesis, one converged tip, producer activation 5→12 executed cleanly and memory-neutrally 2 h 15 m AFTER the step.

Evidence:
  [E1] bins/node/src/metrics.rs:97 — `doli_*` gauges declared but never updated anywhere; the application exporter is structurally dead (root anchor).
  [E2] measure:doli_chain_height=0 on all 15 targets at every sample 2026-07-22→07-30 (Prometheus range query; live scrape of n6 shows all doli_* = 0 while node is at h=72,572 with 14 peers; no process_resident_memory_bytes series exists for any DOLI process) — the exporter defect is fleet-wide and predates the event.
  [E3] measure:grafana_per_node_rocksdb_panel=398MB→774MB at 15:30 (Grafana API, doli-rocksdb-health "Per-node RocksDB resident estimate": a fleet sum() with no `by (instance)`) — a second instrument-untruth surface that manufactured the "fleet-wide" appearance when 7 scrape targets reappeared.
  [E4] log:2026-07-30T15:17:50.869857Z n6 "Starting node with data directory /mainnet/n6/data" — first process start of the boundary window (UTC-stamped app log, authoritative).
  [E5] measure:ActiveEnterTimestamp=2026-07-30 15:31:13 UTC, NRestarts=0 (systemctl show doli-mainnet-n6/n7/n8 on ai4; up{job="doli_node"} = 0 since 07-22 10:46, transitions to 1 at 07-30 15:46 for all of n6–n12; n6.log 15:17:51 "[SNAP_SYNC] Fresh node … gap=70753") — seven nodes, down for the chain's whole life, started together today.
  [E6] measure:host_delta_vs_sum_new_process_rss=ai4 +1.08GB vs 1.084GB (residual −0.4%), ai5 +1.43GB vs 1.474GB (−3%) (Prometheus MemTotal−MemAvailable at 15:16/20:16 paired with ps -eo rss at 20:17; independently: 0.48+4×0.37=1.96 vs 2.04 GB "Apps" expression, and code-lens closure <5%) — the entire step is process-placement arithmetic, not growth in any existing process.
  [E7] measure:max_per_process_rss=378MB fleet-wide; ai1 +32MB/node, ai2 +60MB/node, ai3 +50MB across the boundary (ps -eo rss on all 5 hosts, 20:17 UTC; Prometheus host series) — no node is near 1.9 GB and the step is NOT fleet-wide.
  [E8] log:2026-07-30T17:45:31.483245Z seed "[EPOCH] Frozen producer list for epoch 199: 12 producers, active_list=12 (was: 5 producers)" — producer-set activation is 2 h 15 m AFTER the 15:17–15:31 step; and it is 5→12 (bonds 5→720), not 5→7.
  [E9] measure:host_mem@17:30→18:00=ai4 1474→1475MB, ai5 2128→2141MB, ai1 1401→1261MB (Prometheus, 15-min series) — zero memory inflection at producer activation; producer-set growth is decoupled from the memory step.
  [E10] measure:post_snap_residual=n6–n12 at 343–378MB vs settled nodes 137–201MB, vs ~204MB configured per-process ceiling; smaps_rollup n6-vs-n2 delta = +165,008kB Anonymous of +165,476kB Rss (99.7% anon heap); ~10 64-MiB-aligned glibc arena mappings on n6 vs 3–4 on n1/n2 at identical peer count/bonds/binary (sudo smaps/smaps_rollup) — a real, in-budget-violating residual; trigger correlates with sync volume, not uptime (n1, restarted same window with no backlog, sits at 155MB).
  [E11] crates/network/src/config.rs:137 — memory watchdog threshold defaults to 0 (disabled); only enable path is DOLI_MEMORY_WATCHDOG_BYTES (bins/node/src/node/startup.rs:267), unset in all 11 mainnet systemd units; no MemoryMax/MemoryHigh anywhere; no memory_watchdog_trips_total series in Prometheus — zero automatic memory protection fleet-wide (INC-I-117 confirmed still open).
  [E12] measure:drift=ai5 1.89→2.29GB, ai4 1.30→1.54GB over 16:15→20:15 (~20–25MB/h/node), decelerating at fine grain (ai4 Σ +21.2 → +6.9 MB per 5-min; ai5 flat 19:16–20:16) (Prometheus + repeated ps; dmesg -T on ai4: Jun 29 pre-genesis OOM-kills of doli-node-n6/n8 at anon-rss 1.37–2.12GB) — unclassified: window too short to distinguish post-snap settling from a slow leak.
  [E13] git:dc178d70 — the ONLY commit in 9647b809..HEAD, touching only .claude/scripts/blast.py (+47/−4, OMEGA tooling); md5 0782278a61e12c37ec3b6481c5f343df identical across all 11 per-service binary copies, all reporting 6.24.0 (9647b809) — the deployed binary is byte-identical to fresh genesis; no post-genesis code change can be causal.
  [E14] measure:fork_events=0 (FORK_GUARD/WEDGE_ESCAPE/REORG/ROLLBACK/STUCK_FORK/EQUIVOCATION all = 0 in n7.log 07-22→07-30 whole chain life and seed.log; byte-identical hashes at h=1/70991/72531 between newest node n12 and oldest seed1; fork-retention structures bounded <11MB worst case) — the fork domain is ruled out entirely.
  [E15] measure:per_peer_cost≤8.6MB vs 207MB/peer required (unrestarted controls: ai3 +50MB÷7 peers, ai2 +60MB/node÷7; bottom-up code ceiling ~1.7MB/peer from crates/network/src/transport.rs:18 yamux 262,144B/stream ×6 streams + 12 per-peer maps) — connectivity is 24–120× too small to explain the step.
  [E16] measure:rocksdb_total=n6 45.9MB < n2 53.5MB while n6's RSS is 165MB HIGHER (live doli_rocksdb_* gauges; caps verified DB-scoped: memtable_cap 67,108,864 + 50,331,648; CF counts unchanged at 7 (state_db) + 9 (block_store)) — RocksDB/INC-I-104 regression ruled out; RocksDB is not the variable.
  [E17] git:98650be2 — "remove fork-diagnostics subsystem entirely" (2026-07-07) is a verified ancestor of genesis release 9647b809 (git merge-base --is-ancestor = true) — the diagnostic_ledger implicated by open INC-I-101/I-102 does not exist in the deployed binary.
  [E18] bins/node/src/node/state_root_serve.rs:36 — lazy state-root memo is a single Copy tuple Option<(Hash,Hash,u64)>, overwritten in place; O(1) forever — cannot grow with chain height.

Causal chain:
  1. Fleet-wide memory-observability untruth: the application exporter is dead and no per-process RSS is exported  (supported by [E1], [E2])
  2. Any Grafana memory panel can only bind host-level node_memory_* series, and the one "per-node" memory panel is a mislabeled fleet sum  (supported by [E2], [E3])
  3. Seven nodes (n6–n12), down since Jul 22 11:29 UTC, are started 15:17–15:31 UTC on the two 4 GB hosts, each snap-syncing 70,753 blocks  (supported by [E4], [E5])
  4. A genuine HOST-level memory step appears on ai4/ai5 only, equal to the new processes' summed RSS; all other hosts move +31–60 MB/node  (supported by [E6], [E7])
  5. The reporter — reading the only panels that exist — interprets the host step as a per-node, fleet-wide 4.2× leak caused by producer registration, and files INC-I-146 (the observed symptom)  (supported by [E3], [E6], [E7]; the attributed cause is independently falsified by [E8], [E9])
  6. Residual live exposure remains: freshly snap-synced nodes sit 1.7× over the configured memory budget with an unclassified drift, on 4 GB hosts with zero automatic memory protection  (supported by [E10], [E11], [E12])

Counter-hypotheses ruled out:
  - Producer-set growth (5→12) as the causal input: ruled out by [E8], [E9] — activation occurred 2 h 15 m AFTER the memory step and produced zero memory inflection; nothing in configuration scales with producer count.
  - Fork/sibling/reorg retention (INC-I-143 F2 wedge-escape, sibling caches): ruled out by [E14] — zero fork events in the entire chain life; retention paths never executed; worst-case fork structures <11 MB (≤1 % of the step).
  - Per-peer connection/gossip state (yamux windows, SeenCache, mesh): ruled out by [E15] — 207 MB/peer would be required; measured upper bound is 8.6 MB/peer, code ceiling 1.7 MB/peer; all network caps verified enforced.
  - RocksDB cap regression / CF-count growth (INC-I-104 recurrence): ruled out by [E16] — caps intact, DB-scoped, live-verified; the high-RSS nodes have SMALLER RocksDB footprints than settled nodes.
  - diagnostic_ledger retention (INC-I-101/I-102, the brief's top-ranked lead): ruled out by [E17] — the subsystem was removed before genesis and is not in the deployed binary; recommend closing both incidents as obsoleted-by-removal.
  - Lazy state-root memo growth: ruled out by [E18] — O(1) single tuple, cannot ramp.
  - Post-genesis regression / config or binary drift: ruled out by [E13] — one tooling-only commit since genesis; md5-identical binaries on all 11 units.

Regression check: git log 9647b809..HEAD = exactly 1 commit — dc178d70 "chore(omega): redeploy blast.py — dotted method labels + lower-bound caveats" (2026-07-27, touches only .claude/scripts/blast.py, +47/−4) — verified first-hand during synthesis. The deployed mainnet binary is byte-identical to the fresh-genesis release v6.24.0 (9647b809); md5 0782278a61e12c37ec3b6481c5f343df identical across all 11 per-service copies ([E13]). No post-genesis source change exists that could cause the symptom; the pre-genesis fix batch's fork-domain members were ruled out by runtime evidence ([E14]) and its snap-admission members are absorbed by the arithmetic closure ([E6] — no unexplained mass remains for them to explain).

Shape-Recurrence: RECURS
  Checked: INC-I-009, INC-I-014, INC-I-084, INC-I-100, INC-I-101, INC-I-102, INC-I-104, INC-I-105, INC-I-106, INC-I-107, INC-I-109, INC-I-117 (observability/memory domains, last 180 days, memory.db queried 2026-07-30).
  Same architectural shape as: INC-I-106 (metrics.rs sum_cf aggregation inflated reported block-cache usage N×; resolved 2026-06-02), INC-I-107 (stale HELP text + missing capacity gauge in the same metric family; resolved 2026-06-02) — this is the 3rd occurrence of the shape "memory-observability instrument reports untruthful values → memory state misjudged". The 3rd-recurrence hard gate applies: a root-cause FIX at [E1] is mandatory (provided below). The pattern indicates the metrics layer has no truth-gate — three consecutive incidents where the exporter said something false or nothing at all.

Recommended Fixes:
  - FIX: Wire the application metrics exporter in bins/node/src/metrics.rs — populate every doli_* gauge (chain height, peers, mempool, UTXO size, active producers, sync state) from live node state and export process_resident_memory_bytes (self-RSS) from doli-node; gate it with a regression test asserting doli_chain_height > 0 against a running test node so exporter truth is enforced, not assumed.
      Breaks chain at: [E1]
      Removes the root instrument untruth at bins/node/src/metrics.rs:97 that made correct attribution impossible; also un-blinds defects B/C/D for all future incidents (mandatory root FIX under the 3rd-recurrence gate).
  - DEFENSE-IN-DEPTH: Correct the Grafana doli-rocksdb-health "Per-node RocksDB resident estimate" panel to `sum by (instance)` (or relabel it as a fleet total), and label host-vs-process memory panels explicitly.
      Breaks chain at: [E3]
      Downstream instrument surface — the mislabeled aggregate amplified the "fleet-wide" misreading; the root remains the dead exporter.
  - DEFENSE-IN-DEPTH: Arm the memory watchdog (DOLI_MEMORY_WATCHDOG_BYTES) and add MemoryMax=/MemoryHigh= to all 11 mainnet units — closes INC-I-117 / defect C. Mainnet deploy: explicit user approval required, one host at a time per standing rules.
      Breaks chain at: [E11]
      Caps the blast radius of the unprotected-4GB-host exposure; does not address the root observability defect.
  - DEFENSE-IN-DEPTH: Execute the +24 h / +48 h re-measure of ai4/ai5 (host series AND per-process ps -eo rss) to classify the drift; if it continues linear, run the discriminator on a testnet 70k-block snap-sync replica (jemalloc-stats build or MALLOC_ARENA_MAX=2 canary: RSS high while allocated low ⇒ allocator retention; allocated high ⇒ genuine retained structure in the snap path).
      Breaks chain at: [E12]
      Converts the unclassified residual into a decided state before it can become the next OOM; downstream of the root.
  - DEFENSE-IN-DEPTH: Review ai4/ai5 process placement — 3–4 producers per 4 GB host, with ai5 additionally carrying Prometheus + Grafana + Alertmanager at ~60 % used, is the exact INC-I-102/I-104 density configuration that has OOM'd twice before.
      Breaks chain at: [E10]
      Reduces the probability that the post-snap residual (whatever it turns out to be) lands on a host with no headroom; not a root action.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Domain Diagnosis Report: Mainnet "fleet-wide 4.2× RSS step" (INC-I-146 proposed, RUN 473)

## Problem Profile

- **What happened (as reported)**: per-node RSS stepped ~450 MB → ~1.9 GB (≈4.2×) fleet-wide, CPU shape changed, coinciding with starting "nodes 6 and 12" and registering producers, ~a week after the 2026-07-22 fresh genesis.
- **What actually happened (measured)**: seven nodes (n6–n12) that had been **down for the entire 8-day life of this chain** were started 2026-07-30 15:17–15:31 UTC on the two 4 GB hosts (ai4, ai5). Host memory on those two hosts rose by exactly the new processes' RSS ([E6]). No node process exceeds 378 MB ([E7]). Chain is healthy: no fork ([E14]), all nodes converged on one tip, producer set activated 5→12 cleanly and memory-neutrally at 17:45:31 UTC ([E8], [E9]).
- **Reporter reading, charitably and correctly re-read**: "nodes 6 and 12" almost certainly means the **range n6–n12** — under that reading the reporter's account of *events* is substantially accurate: exactly those seven nodes were started that day and registered as producers. The week-of-stability-then-step is also literally what the host series shows (flat all week — because the hosts were empty of nodes). The premises that fail are the *attributions* (per-node, fleet-wide, leak), and they failed because of a broken instrument, not observer error.
- **When**: step window 2026-07-30 15:17–15:31 UTC (host memory ramping over the following ~hour as 7 processes snap-synced); investigation same day 20:16–21:10 UTC.
- **Affected nodes**: hosts ai4 (n6–n8) and ai5 (n9–n12) stepped; ai1/ai2/ai3 moved +31–60 MB/node only.
- **Chain**: DOLI mainnet, PoS 10 s slots, genesis 2026-07-22 10:45 UTC, v6.24.0 (`9647b809`) on all structural nodes, tip ~72,530–72,590 during investigation, byte-identical hashes at all sampled heights.

## Domain Relevance Matrix

| Domain | Relevance | Top Hypothesis | Confidence | Key Finding |
|--------|-----------|----------------|------------|-------------|
| Fork | MINIMAL | — (ruled out) | fork hypotheses dead at conf 0.02–0.05 | Zero fork/reorg/sibling events in entire chain life; fork retention <11 MB worst case (≤1 % of step) [E14]. Latent: `pending_proofs` never drained (0 B today). |
| Connectivity | LOW | Bounded ≤8.6 MB/peer contribution on unchanged nodes; not causal | conf(0.9, measured) rule-out arithmetic | 207 MB/peer required vs 8.6 measured vs 1.7 from code — 24×/120× short [E15]. All network caps verified enforced. Flagged glibc-arena residual to code lens. |
| Parameters | HIGH (as absent limits + measurement defects, NOT mis-tuning) | Reported figure = ai5 host "Apps" panel 0.48→2.04 GB (4.25×) | conf(0.70, measured) | No parameter scales with producer count; configured per-process ceiling ~204 MB; watchdog unarmed + no MemoryMax anywhere [E11]; mislabeled Grafana fleet-sum panel [E3]. |
| Code | LOW (for the symptom) — but owns the root-cause defect | Process lifecycle explains step; dead metrics exporter explains misreading | conf(0.7, measured) both | No unbounded collection with missing eviction found; O(N) attestation verify confirmed real but negligible at N=12; exporter emits zeros fleet-wide [E1], [E2]. |

**All four investigators independently falsified the reporter's premises and converged on the same account.** No report was thin (all ≥300 substantive lines with Domain Relevance Assessments); synthesis NOT compromised.

## Domain Classification

- **Primary domain (root cause)**: **Observability/instrumentation** — a code defect (`metrics.rs:97` exporter never updated, no process-RSS export) plus a dashboard facet (fleet `sum()` labeled "per-node"). In the 4-domain taxonomy: **Code** primary, **Parameters/ops** facet.
- **Presenting domain**: Memory/resource (reported as a fleet-wide RSS leak).
- Presenting ≠ primary — the signature cross-domain case: a "memory problem" that is an instrumentation problem wearing a memory costume.

## Cross-Domain Causal Chain

```
Root cause domain:   observability (code: dead exporter [E1] + params: mislabeled panel [E3])
  + independent op:  7 planned node starts on ai4/ai5, snap-syncing 70,753 blocks [E4][E5]
    -> produced:     genuine HOST-level step on ai4/ai5 = Σ new-process RSS [E6][E7]
      -> read as:    per-node fleet-wide 4.2× leak (only host-level series exist)
        -> filed as: INC-I-146 "fleet-wide RSS step + CPU shape change" (symptom)

Direction: Observability defect → misattribution of a real, benign event
Primary domain: CODE/OBSERVABILITY      Presenting domain: MEMORY/RESOURCE
```

**Causal direction tests applied**:
1. *Counterfactual*: could the misreading have occurred WITHOUT [E1]? No — with per-process RSS exported, a per-node panel would show flat ≤378 MB lines and the host panel would be identifiable as host-level. Causation confirmed.
2. *Temporal precedence*: gauges have emitted zeros since genesis ([E2] — 8-day range query) — the defect predates the event. Confirmed.
3. *Mechanism*: with zero `doli_*` series, any Grafana memory panel can only bind `node_memory_*` (host) or the mislabeled fleet-sum panel that stepped 398→774 MB purely from 7 scrape targets reappearing ([E3]). Both manufactured the "fleet-wide per-node" appearance. Confirmed.
4. *Reverse test on the reporter's causal candidate* (producers 5→12): the step **precedes** activation by 2 h 15 m and activation is memory-flat ([E8], [E9]). Cannot be causal. The CPU-side input (attestations/block 120→128) moved at the epoch-199 boundary exactly where memory did NOT move — a clean measured decoupling.

**Separate genuine thread (NOT part of the misdiagnosis chain — must not be buried)**: the post-snap-sync heap residual [E10]. Real, in-budget-violating, PRELIMINARY (trigger unproven). See Genuine Defects, item B.

**Graph note**: graphify is not provisioned in this environment (checked). The code-level root is an *absence-of-call-sites* defect (gauges declared at `metrics.rs:97`, never updated) — no structural "X calls Y" link is asserted in the chain, and the defect's effect is confirmed by direct runtime measurement ([E2]), which supersedes structural inference (additionally unreliable here given graphify's documented Rust-method blind spot).

## Convergence Matrix

| Claim | Fork | Conn | Params | Code | Convergence |
|---|---|---|---|---|---|
| Step is TODAY 15:17–15:31 UTC, not week-old | Y (Prometheus + n6.log) | Y (systemd + `up{}`) | Y (n6.log + 8-day series absence) | Y (journalctl) | **4/4** |
| NOT fleet-wide (ai1/2/3 +31–60 MB only) | Y | Y | Y | Y | **4/4** |
| Grafana series is HOST memory; max per-process RSS ≈378 MB | Y (inferred + ps) | Y (ps ×5 hosts) | Y (ps + smaps) | Y (ps + reconciliation) | **4/4** |
| Δhost = Σ new-process RSS (arithmetic, not leak) | consistent (not closed) | Y (0.4 %/3 %) | Y (+4 %/exact) | Y (<5 %) | **3/4 + 1 consistent** |
| Producer growth 5→12 decoupled from memory | Y (timing, measured) | Y (observed) | Y (no config scales with N) | Y (control hosts flat) | **4/4** |
| Metrics exporter dead / no process-RSS export | Y | Y | Y | Y | **4/4** |
| Post-snap residual: n6–n12 ~2× settled, ~99.7 % anon heap | Y (observed) | Y (smaps: arenas) | Y (smaps_rollup: budget violation) | Y (sync-volume correlation + OOM precedent) | **4/4** |
| Watchdog unarmed + no MemoryMax (INC-I-117 open) | flagged | corroborated | Y (source + 11 units) | Y (unit grep) | **3/4 measured** |

**Independence check — VERIFIED.** Convergent claims were reached through distinct evidence channels: fork = Prometheus range series + UTC app logs + RPC hash comparison; connectivity = systemd state + paired host-Δ/Σ-RSS sampling + /proc/smaps + RPC; parameters = ps + smaps_rollup + unit-file grep + Grafana API + live RocksDB gauges; code = journalctl + dmesg + git merge-base + source reads. No convergence rests on a shared log line. 4/4 convergence on 5+ independent claims ⇒ base 0.85+; confirmed cross-domain chain ⇒ +0.1; arithmetic closure <5 % via three independent expressions ⇒ **conf(0.96, converged)**.

## Contradictions — 6 found, 6 resolved, 0 unresolved touching root cause

1. **Step minute: 15:17 vs 15:31 vs "16:31".** RESOLVED. Two process generations: first start **15:17:50 UTC** ([E4], UTC-stamped app log — authoritative), two stop/start cycles, final generation **15:31:13 UTC** ([E5]). Every "16:31" figure (fork's `ps -o lstart`, code's journalctl excerpts) is the SAME 15:31 UTC event rendered in local WEST/BST (UTC+1); the connectivity investigator's explicit conversion (`16:31:13 WEST = 15:31:13 UTC, NRestarts=0`) is authoritative. This also dissolves the fork report's open question #5 ("why did 7 nodes restart at 16:31, an hour after the step") — there was no second restart; it is a timezone artifact. **Authoritative step window: 2026-07-30 15:17–15:31 UTC**, memory ramping over the following ~hour as processes snap-synced.
2. **Byte deltas: ai4 0.48→1.56 vs 0.49→1.54 vs 0.13→1.21; ai5 0.85→2.28 vs 0.86→2.29 vs 0.48→2.04 (and fork's 0.86→2.42).** RESOLVED — two different PromQL expressions plus sample-time skew on a ramping series. Fork/conn/code used `MemTotal − MemAvailable`; parameters used the Node-Exporter-Full "Apps" expression (`MemTotal − MemFree − Buffers − Cached − Slab − PageTables − SwapCached`), which subtracts reclaimable cache and reads lower baselines (ai5 0.48 vs 0.85). Fork's ai5 2422 MB @20:00 vs conn's 2280 @20:16 is reclaimable-page-cache wiggle inside `MemAvailable` (~6 %) — no bearing on any conclusion. **Authoritative for the closure claim**: connectivity's paired 15:16/20:16 sampling ([E6] — same expression, host Δ measured simultaneously with Σ process RSS; residual 0.4 %/3 %). **Authoritative for what the reporter saw**: parameters' ai5 "Apps" series **0.48 → 2.04 GB = 4.25×** — the only single series reproducing both endpoints AND the ratio of "450 MB → 1.9 GB ≈ 4.2×" to two significant figures — held at conf(0.70, measured) because the reporter's screen was not directly observed (code's fallback: "450 MB" = ai4 idle baseline, "1.9 GB" = ai5 @16:15 — requires mixing two hosts, less parsimonious).
3. **Producer transition epoch: 197→198 (conn: seed3 `snap_bonds` 5→720 at 16:46) vs 199 (fork: frozen list at 17:45:31).** RESOLVED — not contradictory. Registrations landed h=70,991–70,999 (~16:04–16:10 UTC), entered the epoch-state snapshot at the 198 boundary and the frozen active-production list at 199 ([E8]). Both boundaries postdate the step; neither shows a memory inflection ([E9]).
4. **diagnostic_ledger (INC-I-101/102) as the brief's top lead vs structurally impossible (code).** RESOLVED in code's favor: [E17] — the subsystem was removed pre-genesis. Fork's "not cleared by my lens" was lane discipline, not a counter-claim. **Consequence: close INC-I-101/I-102 as obsoleted-by-removal.**
5. **INC-I-143 root-cause status: brief says "NOT established"; fork cites postmortem §12 (run 466) establishing it at conf 0.95** (unserialized rolling restart shipping block-content commit `427d5050`). RESOLVED — the postmortem supersedes the stale memory.db record; the INC-I-143 damage signature (`BLOCK 1 ✗ / INTEGRITY −1`) is also gone from this chain (n12 has block 1 matching seed1). Update the DB record.
6. **Drift: linear +20–25 MB/h/node (code, 4 h coarse) vs decelerating/flat (conn, 5-min fine grain; ai5 flat final hour).** RESOLVED as not-a-contradiction: both series are consistent with a decelerating post-snap asymptote — the ramp concentrated in 16:15–19:16, flattening after ([E12]). **Stated plainly: the observation window (≤5 h total, minutes at fine grain) is too short to distinguish (i) post-snap settling toward an asymptote, (ii) a genuine slow leak — the June 29 pre-genesis OOM cadence at these rates would reach ~1.9 GB in ~2.5–3 days — or (iii) host-level noise from non-DOLI processes.** Unclassified by design, not by negligence. See defect D and the scheduled re-measure.

### Reporter premises — what was seen vs. what was measured (no blame; the instrument was broken)

| Premise | What the reporter actually saw | Why that reading was reasonable | Measured truth |
|---|---|---|---|
| "Per-node ~450 MB → ~1.9 GB (4.2×)" | ai5 host "Apps" memory 0.48 → 2.04 GB (4.25×) — best single-series match | Defect A means **no per-process memory series exists anywhere**; a memory panel on this Grafana can only be host-level, and nothing labels it as such | Max per-process RSS fleet-wide = 378 MB [E7]; host step = Σ of new processes + pre-existing base [E6] |
| "Fleet-wide, incl. unchanged nodes" | Both 4 GB hosts stepped simultaneously; the "Per-node RocksDB resident estimate" panel (a fleet sum) stepped 398→774 MB when 7 targets reappeared [E3] | Two mislabeled/aggregated instruments manufactured a fleet-event appearance; and a genuine small fleet-wide component (+31–60 MB/node from peers 7→14 at ≤8.6 MB/peer) sat under the instinct | ai1/ai2/ai3 did not step [E7]; effect confined to the two hosts that gained processes |
| "Nodes 6 and 12 started, ~a week after genesis" | Read charitably as **the range n6–n12** — exactly right: those seven nodes, down since Jul 22 11:29, all started 2026-07-30 [E5] | The host series was genuinely flat all week, then stepped the day those nodes started — the *event narrative* is accurate | Seven nodes; step today 15:17–15:31 UTC [E4][E5]; the "stable week at 450 MB" was hosts running **zero** DOLI nodes |
| "Caused by producer registration (5→7)" | Registrations and the step happened the same afternoon | Without app metrics there is no way to decompose co-occurring changes | Producers went 5→**12** (bonds 5→**720**); activation 17:45:31 UTC — 2 h 15 m after the step — memory-neutral [E8][E9] |

## Root Cause

See the VERDICT block. Summary: the observed event is process-placement arithmetic, not a leak — seven planned node starts on two 4 GB hosts, closure to 0.4–5 % across three independent measurements. The root cause of the incident as filed is the dead application metrics exporter ([E1], [E2]) plus a mislabeled dashboard aggregate ([E3]), which together made a host-level reading of a benign event look like a fleet-wide per-node leak. The reporter — the system's architect — observed a real step on a real dashboard and escalated correctly; the instrument was broken, so correct attribution was impossible. One genuine technical residual survives falsification (defect B below).

## Causal Chain

**Convention: link 1 is the root cause; the final observed symptom is link 5; link 6 is the residual-risk tail.**

| # | Item | Domain | Derived? | Derivation |
|---|------|--------|----------|------------|
| 1 | [E1][E2] Memory-observability instrument untruth: doli_* exporter dead, no process-RSS export | code (observability) | — | (the root) |
| 2 | [E2][E3] Only host-level series can populate Grafana; the one "per-node" panel is a mislabeled fleet sum | parameters (dashboard) | YES | from #1 — nothing else exists to bind |
| 3 | [E4][E5] 7 nodes started 15:17–15:31 UTC on ai4/ai5, each snap-syncing 70,753 blocks | operations (independent input) | — | planned operator action, independent of #1 |
| 4 | [E6][E7] Genuine host step on ai4/ai5 only = Σ new-process RSS; other hosts +31–60 MB/node | resource/host | YES | from #3 — placement arithmetic |
| 5 | [E3][E6][E7] Host step read as per-node fleet-wide 4.2× leak; INC-I-146 filed (**symptom**) | presenting | YES | from #2 + #4; attributed cause falsified by [E8][E9] |
| 6 | [E10][E11][E12] Residual exposure: fresh nodes 1.7× over budget, unclassified drift, zero memory protection on 4 GB hosts with June-29 OOM history | parameters/code | YES | from #3 + defects B/C — the genuine risk tail the misattribution nearly buried |

## Genuine Defects Surfaced (do not let "premise falsified" bury these)

**A — Metrics exporter dead fleet-wide. Severity: HIGH (P1). Status: VERDICT (measured, 4/4).** All `doli_*` gauges permanently 0 ([E1], [E2]); no process-RSS export. **The causal defect behind the misdiagnosis** and the highest-value finding: it is why an 8-investigator swarm was needed to establish what `ps` answers in one second. Also blinds the INC-I-117 watchdog question and makes memory-budget rows (mempool, UtxoSet, peers) unobservable in production. Aggravating facet: the Grafana fleet-sum panel labeled "per-node" ([E3]).

**B — Post-snap-sync heap residual. Severity: P1 open question. Status: PRELIMINARY (mechanism measured, trigger unproven).** Freshly snap-synced n6–n12 hold 343–378 MB vs 137–201 MB settled; configured per-process ceiling ≈204 MB (+~20 MB file-backed) ⇒ **~1.7× over budget — at least one consumer exists outside every configured cap** ([E10]). Excess is 99.7 % anonymous heap in ~10 64-MiB-aligned glibc arena mappings; discriminator is **sync volume, not uptime** (n1, restarted same window with no backlog: 155 MB). RocksDB ruled out ([E16]). Cannot distinguish live-but-unbounded structure from retained-free allocator arenas from outside the process — no allocator stats exist and heap profiling is forbidden on read-only mainnet. **The primary open technical question.** Resolve by: (a) the +24/+48 h re-measure; (b) testnet 70k-block snap-sync replica under jemalloc-stats or `MALLOC_ARENA_MAX=2` — RSS high while allocated low ⇒ allocator retention (benign, fixable); allocated high ⇒ genuine retained structure in the snap path (INC-I-139/143 territory).

**C — Zero automatic memory protection fleet-wide. Severity: P1 operational. Status: VERDICT (measured).** Watchdog disabled by default and never armed; no cgroup limits anywhere ([E11]). INC-I-117 confirmed still open. On 4 GB hosts at 1.5–2.3 GB used with a June-29 dmesg history of OOM-kills at 1.4–2.1 GB anon-RSS ([E12]) — live exposure with no tripwire.

**D — Unclassified drift, window too short. Status: OPEN (explicitly not classified).** ~+20–25 MB/h/node on ai4/ai5 over 16:15–20:15, decelerating at fine grain, ai5 flat the final hour ([E12]). Same phenomenon as B (arenas/compaction settling), a separate ramp, or noise? **The window cannot distinguish these — we decline to over-claim.** Resolve by re-measuring at +24 h and +48 h (2026-07-31 and 2026-08-01, ~20:00 UTC): host series AND `ps -eo rss` on ai4/ai5. Settling ⇒ B is an asymptote; linear ⇒ escalate as a leak (June-29 cadence: ~2.5–3 days to 1.9 GB at this rate).

**E — Latent: `pending_proofs` never drained. P3.** `crates/network/src/sync/equivocation.rs:95`, pushed at `:157`; `take_pending_proofs()` (`:189`) has zero non-test callers; unbounded by construction, retains two full BlockHeaders per event. **0 bytes today** (zero equivocations ever on this chain). Register as invariant-gap follow-up.

**F — INC-I-141 O(N) attestation verification confirmed real. P3 now / P2 at scale.** `crates/core/src/attestation.rs:203-206`: sequential, unbatched per-signature loop — genuinely O(N) in attester count. At 12 producers ≈1.2 verify/s: negligible (CPU 1 %→4–5 % on ai4/ai5 = process starts; control hosts flat). Headroom to the ~300-producer ceiling INC-I-141 records is ~25× on producer count — a live scaling ceiling, not today's problem. The DirectAttestation unicast fan-out observed in logs also scales with producer count. Keep INC-I-141 open.

**Minor latents to register**: `fork_block_cache` TTL-only bound (flood-rate × 300 s — INC-I-114 shape; cold today); sync-manager `.clear()` retains capacity (no `shrink_to_fit`; single-digit MB at observed high-water); `ContentStore` fully uncapped if ever wired (currently dead code). **Ops questions for the user**: n6's `blocks/` SSTs have mtime 09:47, predating its 15:17:50 first start — data dir pre-populated out of band (confirm what was copied); all external-producer scrape targets down since 2026-07-22 — externals appear not to be on this chain.

## Shape-Recurrence

**RECURS — 3rd occurrence** of "memory-observability instrument reports untruthful values → memory state misjudged". Matches: INC-I-106 (metrics.rs aggregation inflated block-cache figure N×), INC-I-107 (stale HELP + missing gauge), both resolved 2026-06-02, domain `observability`. Checked: INC-I-009, I-014, I-084, I-100, I-101, I-102, I-104, I-105, I-106, I-107, I-109, I-117. Adjacent, not counted: INC-I-100 (instrument false-positive, fork domain), INC-I-117 (open, excluded by status), plus the recorded behavioral learning about stacked Grafana panels showing the operator-level misreading has itself recurred. Hard gate satisfied: root FIX at [E1] proposed. The pattern's lesson: the metrics layer needs a truth-gate (regression test against a live node), not another patch.

## Recommended Fixes

See the VERDICT block for the canonical entries: **1 × FIX at [E1]** (wire the `bins/node/src/metrics.rs` exporter + process-RSS export + live-node regression test) and **4 × DEFENSE-IN-DEPTH** ([E3] Grafana panel, [E11] watchdog + MemoryMax, [E12] +24/+48 h drift re-measure with testnet allocator discriminator, [E10] ai4/ai5 placement review). Latents E/F and the minor latents are follow-up registrations, not fixes to this chain.

**Explicitly NOT recommended**: any memory "leak fix" to node code (no leak is measured), any node restarts/wipes, any genesis action. The chain is healthy; nothing about this event requires touching mainnet state.

## Routing Recommendation

```
PRIMARY DOMAIN ROUTING:
Domain:          Code (observability — dead metrics exporter)
Route to:        /omega-doctor  (code bug pipeline; testnet first, standard TDD gate)
Reasoning:       Root cause of the incident-as-filed is a code defect (metrics.rs:97 gauges
                 never updated + missing process-RSS export). 3rd shape recurrence demands a
                 gated, tested root fix, not another patch.
Cross-domain:    After the exporter fix is live, the misreading channel self-resolves.
                 Secondary (parameters/ops, explicit user approval, mainnet rules apply):
                 arm watchdog + MemoryMax + Grafana panel fix + ai4/ai5 placement review.
                 Evidence thread (no agent needed): +24/+48 h re-measure for defects B/D;
                 if linear, re-enter /omega-doctor --incident INC-I-146 with the testnet
                 heap-profiling plan (jemalloc stats or MALLOC_ARENA_MAX=2 canary).
NOT routed:      Fork (ruled out, zero events), Connectivity (ruled out, 24–120× short),
                 producer-set scaling (decoupled by measurement).
Memory hygiene:  Close INC-I-101/I-102 (obsoleted by pre-genesis removal 98650be2);
                 update INC-I-143 record (root cause established per postmortem §12);
                 register latents E, F, fork_block_cache TTL-only bound, ContentStore
                 uncapped-if-wired; surface n6 data-dir provenance + external-producer
                 absence to the user.
```

## Quality Gate

```
DOMAIN SYNTHESIS QUALITY GATE
Domain reports completed:           4/4 (all >= 300 substantive lines, all with relevance sections)
Domain relevance distribution:      Fork: MINIMAL, Conn: LOW, Params: HIGH, Code: LOW
Primary domain:                     Code/Observability (dead exporter) + Parameters facet (dashboard)
Presenting domain:                  Memory/resource (different from primary)
Cross-domain causation:             YES (observability defect -> misattribution of benign ops event)
Convergence on root cause:          4/4 domains (5+ independent convergent claims)
Evidence independence:              VERIFIED (app logs / systemd / Prometheus / smaps / dmesg /
                                    git / RPC / dashboard API — distinct channels per claim)
Contradictions found:               6
Contradictions resolved:            6/6 (0 unresolved touching root cause)
COMPROMISED flag:                   NO
Evidence status:                    VERDICT for diagnosis + defects A/C; PRELIMINARY for defect B;
                                    OPEN (declared, scheduled) for defect D
Regression check:                   git log 9647b809..HEAD = 1 commit (dc178d70, OMEGA tooling
                                    only) — binary byte-identical to genesis; verified first-hand
Shape-recurrence:                   RECURS (3rd occurrence; hard gate satisfied by FIX at [E1])
Routing recommendation:             /omega-doctor (metrics exporter), then ops hardening w/ user gate
```
