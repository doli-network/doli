# Domain Investigation Brief — Mainnet fleet-wide RSS step change (4.2×) + CPU shape change

- **INC_ID**: new (propose INC-I-146)
- **RUN_ID**: 473
- **Command**: `/omega-swarm --deep`
- **Date**: 2026-07-30
- **Scope**: full investigation (no `--scope` given)

---

## Problem description (refined prompt)

On DOLI **MAINNET**, per-node resident memory made a **STEP CHANGE** from a stable ~450 MB baseline to
~1.9 GB (≈4.2×), and CPU utilization **changed shape** at approximately the same time.

The step is **FLEET-WIDE** — it affected *all* nodes, including nodes that were already running and
otherwise unchanged, not only the two nodes that were started at that moment. The temporal boundary
coincides with starting node **N6** and node **N12** and registering both as **producers**.

The chain was restarted from **fresh genesis on 2026-07-22** (v6.24.0, commit `9647b809`) with **3 seeds
+ 5 producers**, and ran stably at ~450 MB for roughly a week before this boundary.

### What must be determined

1. **What is consuming the additional ~1.45 GB per node**, and **why fleet-wide** rather than local to
   the two new nodes.
2. **The causal input.** Candidates (not exhaustive, do not anchor):
   - (a) producer-set size 5→7: scheduler working set, attestation bitfield/aggregation, EpochState,
     per-producer bookkeeping;
   - (b) peer/connection count growth: per-peer buffers, gossip mesh degree, concurrent sync sessions,
     Yamux/QUIC window buffers;
   - (c) a workload change that only manifests above a producer or peer threshold;
   - (d) bounded, benign allocator/cache growth (RocksDB block cache, mempool, block cache,
     epoch-state rebuild) that simply had not been touched at 5 producers;
   - (e) an unrelated concurrent change in the deployed binary or environment.
3. **Bounded-scaling vs unbounded leak.** These have OPPOSITE remediations. A bounded steady state that
   scales with N is a *capacity/design* finding; an unbounded leak is a *defect*. Misclassifying this is
   the most expensive error available here.
4. **The CPU shape change**, with the same rigor: which work increased (validation, gossip
   forward/dedup, attestation aggregation, state-root computation, sync, RocksDB compaction) and whether
   it scales with producer count, peer count, or block content.

### Explicit non-anchoring instruction

Classify the problem from symptoms, not from the user's framing. A "memory problem" may have a
consensus, gossip, or sync root cause. State explicitly whether the observed steady state is **EXPECTED**
for a 7-producer / N-peer fleet — a correct-but-costly design is a different finding than a bug and must
not be reported as a bug.

---

## Chain context

| Item | Value |
|---|---|
| Chain | DOLI mainnet |
| Consensus | PoS, 10s slots, bond-weighted epoch rewards, attestation bitfields |
| Genesis | fresh, 2026-07-22 (`db05c2c5` set genesis_time 1783532348; `9647b809` release v6.24.0) |
| Binary at genesis | v6.24.0 |
| Topology at genesis | 3 seeds + 5 producers |
| Topology now | 3 seeds + 7 producers (N6, N12 added) + external producers |
| State model | UTXO (not accounts); 3 states must be byte-identical across nodes: ChainState, UtxoSet, ProducerSet |

### Server layout (mainnet)

- `ai1` — seed1, n1–n3
- `ai2` — seed2, n4–n5, **explorer** (queries local seed2)
- `ai3` — seed3, ivan, santiago
- `ai4` — n6–n8
- `ai5` — n9–n12, **monitoring stack** (Prometheus + Grafana + Alertmanager,
  https://monitor.doli.network)

All reachable via `ssh <alias>` (aliases already in `~/.ssh/config` — just `ssh <ai4>`).
Services are **systemd** with **per-service binary copies** (`doli-node-seed`, `doli-node-n1`, …).

---

## User evidence

| Type | Evidence |
|---|---|
| [image] | Grafana time-series dashboard: clear step in per-node memory; CPU signal shape changed at ~same boundary |
| [metric] | Memory ~450 MB → ~1.9 GB steady state (≈4.2×). **Step, not a linear leak ramp.** |
| [metric] | CPU behavior/shape changed; magnitude not stated by user |
| [event] | N6 and N12 started + registered as producers on the same day |
| [scope] | **"This issue has affected all nodes"** — fleet-wide, incl. unchanged nodes. Most discriminating fact in the report. |
| [baseline] | ~1 week stable at ~450 MB post-genesis with 3 seeds + 5 producers |

### Data sources available to investigators

- **Prometheus/Grafana on ai5** — https://monitor.doli.network (admin credentials in
  `~/.claude/projects/.../memory/reference_monitoring_stack.md`). Prometheus is the authority for the
  exact step timestamp and for per-metric decomposition. **Query Prometheus directly** (HTTP API on ai5)
  rather than eyeballing the dashboard.
- **App logs in FILES** — e.g. `/var/log/doli/mainnet/seed.log` on ai1/ai2/ai3 (root-owned, large —
  use `sudo` + date-filtered grep). Producer nodes: look in the node data dir or the `--log-file` flag in
  the systemd unit. **journalctl only has systemd lifecycle events, NOT app logs.**
- **RPC** — seed RPC and per-node RPC; `getChainInfo`, producer/bond queries, peer lists.
- **Source** — this repo; the deployed binary corresponds to v6.24.0 / HEAD-ish. Verify the actual
  deployed version per node (`doli-node --version` or the release commit) before attributing behavior.

---

## ⚠️ HARD CONSTRAINTS (binding on every investigator)

1. **MAINNET. READ-ONLY.** No restarts, no deploys, no config changes, no wipes, no `systemctl stop`,
   no RPC write/admin calls, no genesis touches. Diagnostics only: read files, read logs, read metrics,
   read-only RPC queries, read source.
2. **Never `pkill`/`kill` any node.**
3. **Never wipe or delete any data directory.**
4. Do not propose a genesis reset. If your analysis appears to require one, say so explicitly as a
   finding and stop — that is a user decision, and #0 RULE forbids resets for storage/feature changes.
5. Log reads on seeds require `sudo` and **must be date-filtered** — the seed log is hundreds of MB.
6. Respect the 60% context budget. Summarize; do not dump raw logs into your report.

---

## Regression context (MANDATORY git archeology)

⚠️ **REGRESSION CONTEXT DETECTED**

- **Baseline**: fresh mainnet genesis `9647b809` (v6.24.0, 2026-07-22). Stable-at-450 MB window = the
  ~1 week after genesis with 3 seeds + 5 producers.
- **Deployed range**: `9647b809..HEAD`, **plus** the pre-genesis fix batch that is *inside* the deployed
  binary and therefore untested at >5 producers:
  - INC-I-139 snap-admission rework (`2d13b847`, `622c373c`, `2930c80b`, `64080417`, `dcdd8be3`, `015342ed`, `89bfc9c0`)
  - INC-I-142 unified gossip staleness gate (`1c510919`, `b7f13756`)
  - INC-I-143 fork-guard wedge-escape / SiblingFetch / snap-anchor gates (`560b2af7`, `d19de7ee`, `0de4c536`, `6738e660`, `56cefb63`, `9f821b81`)
  - INC-I-144 height-index purge on rollback/reorg (`b57b1869`, `fa9835dc`)
  - INC-I-145 archive repair RPC (`5ece4f76`)
  - Lazy state-root commitment (`63fc90b1`, `df974e06`, `0a0016e1`)
  - BLS attestation aggregate removal (`86bac138`, `427d5050`)
  - Disk-guardian: fail-safe writes + logrotate (`cde5215c`, `ea5d495c`, `ec6afc52`)

**Before forming any root-cause hypothesis:**

1. Identify suspected affected code paths from the symptoms. Memory-resident structures to consider:
   UTXO set (in-memory + RocksDB), ProducerSet / EpochState, block cache, gossip dedup + staleness
   caches, sync-manager buffers, per-peer buffers, RocksDB block cache / write buffers, mempool,
   attestation aggregation, archiver.
2. Run `git log 9647b809..HEAD -- <suspected_paths>` **and** review the pre-genesis batch above.
   Enumerate EVERY commit touching those paths.
3. For each commit, read the actual change and assess whether it could plausibly cause a **fleet-wide
   4.2× step in RSS** that manifests only once producer count crosses 5→7 (or peer count crosses a
   threshold).
4. **NO investigator may conclude "pre-existing defect" OR "expected behavior" without explicitly
   reviewing this diff and ruling each commit out with evidence.**

Findings without git-archeology evidence are incomplete → PRELIMINARY (not VERDICT) under the
evidence-floor protocol.

---

## Time window

- Genesis: 2026-07-22
- Stable window: ~2026-07-22 → boundary
- Boundary: the day N6 + N12 were started and registered as producers (**exact timestamp must be
  established from Prometheus / logs — do NOT accept the user's approximate date as the boundary**)
- Symptom persists to: 2026-07-30 (today)

---

## Evidence Assembly

Full detail in `docs/.workflow/evidence-assembly.md` (24 incidents, invariant table, hotspots).
Compressed summary — **read this before forming any hypothesis**:

### ⚠️ CRITICAL — the binary did NOT change

`git log 9647b809..HEAD` = **1 commit, OMEGA tooling only**. The deployed binary is effectively
**byte-identical to fresh genesis**. None of the 11 memory-relevant invariant-guarded files were touched
post-genesis.

**Consequence: no post-genesis source change can explain the step.** The root cause is therefore one of:
- (i) a **workload/topology threshold effect** in unchanged code (something scales badly with producer
  count, peer count, or fleet size, and 5→7 crossed a knee);
- (ii) a latent defect in the **pre-genesis batch** (INC-I-139 snap admission, INC-I-142 gossip staleness
  gate, INC-I-143 fork-guard/SiblingFetch/snap-anchor, INC-I-144 height-index purge, INC-I-145 archive
  repair, lazy state-root commitment, BLS-aggregate removal, disk-guardian) that had never been exercised
  above 5 producers;
- (iii) an environmental/operational change (config, systemd unit, data volume, diagnostic_ledger growth)
  rather than code.

Do **not** spend effort diffing `9647b809..HEAD`. Spend it on (i)/(ii)/(iii).

### Escalation signal

26–31 prior doctor/diagnose/bugfix/swarm runs on this exact problem shape (mainnet resource spike,
RAM-or-CPU, code-vs-scale ambiguity). **INC-I-070, 101, 102, 110, 117, 141, 143 are all still OPEN.**
This symptom class has never fully closed. Treat "we already fixed that" with suspicion.

### Prior memory incidents — highest-signal

| INC | Status | Root cause | Relevance now |
|---|---|---|---|
| **I-104** | resolved | RocksDB uncapped memtables → node-density-dependent OOM | Caps are **static ~152 MB total** — they do **not** scale with producer count. Verify CF count hasn't grown. |
| **I-114** | resolved+verified | Gossip flood → unbounded `VecDeque` → 7.44 GB OOM | Version-independent. Same shape could recur through a different queue. |
| **I-102** | **OPEN** | ai4/ai5 OOM after `diagnostic_ledger` deploy — **root cause never filled in** | ai4 hosts N6–N8, ai5 hosts N9–N12 — exactly the servers hosting the new producers. |
| **I-101** | **OPEN** | `diagnostic_ledger` scan latent risk — **fix deferred, never deployed** | Live risk in the running binary. |
| **I-117** | **OPEN** | Memory watchdog found **dormant / unarmed fleet-wide**, never fixed | We may have no automatic protection right now. |
| **I-141** | **OPEN** | Attestation/vote verification claimed **O(N) per block**, ~45 % CPU at ~300 producers | **Direct candidate mechanism for the CPU-shape change at 5→7.** |
| **I-143** | **OPEN** | Fleet SnapSync cascade — **root cause NOT established**, damage still present | Pre-genesis batch; snap paths are in the deployed binary. |

### Count-scaling precedents (this fleet has a documented history of knees)

- **INC-I-009 / 011 / 014** — RAM cliffs at specific node counts (103 → 112 → 136+).
- **INC-I-038** — gossip mesh scoring becomes unreachable **exactly at 38 producers**.
- **INC-I-104** — identical hardware: ai4 fine, ai5 OOMs. Pure density threshold.
- **INC-I-141** — O(N) attestation verification cost.

**Pattern: DOLI has repeatedly exhibited discrete, threshold-triggered resource knees.** A 4.2× step at a
count boundary is *consistent with prior behavior of this system*, not anomalous.

### Do-not-retry list (failed / already-settled)

- INC-I-143 hypotheses **H1–H4** — falsified.
- INC-I-073 checkpoint growth — ruled **NOT a bug** (5/5 convergence). Don't re-litigate.
- INC-I-070 storage growth — never resolved; do **not** assume "normal growth" as an explanation.
- Commit `1c510919`'s claim that the gossip `SeenCache` is bounded at ~2 MB — **verify by measurement**;
  do not simply restate the claim, and do not propose "unbound it" without evidence.

### Recorded intentional RAM tradeoff: NONE

**No decision anywhere states that ~1.9 GB at 7 producers is expected.** The only capacity figure on
record is the static ~152 MB RocksDB memtable cap (INC-I-104), which does not scale with producer count.
**The absence of a recorded budget is itself a finding** — nobody has ever sized this fleet's steady state.

### Active invariants in scope (all confirmed untouched post-genesis)

`INV-STORAGE-001` (RocksDB caps) · `INV-NETWORK-002/003/004`, `INV-NET-002` (gossip bounding/dedup) ·
`INV-PERIODIC-001` (no 1 s full-UTXO scan) · `INV-APPLY-012` (no O(N) scans in `apply_block`) ·
`INV-SYNC-011/014` (snap admission/backend) · `INV-CONSENSUS-001` (deterministic producer-set-at-height).

**Prometheus metrics to check first**: `doli_rocksdb_db_write_buffer_size_bytes`,
`memory_watchdog_trips_total`, `process_resident_bytes`.

### Hotspots

`sync/manager/{mod,production_gate,cleanup}.rs` — critical (16–23 touches).
⚠️ `crates/storage/`, `crates/mempool/`, `crates/core/src/validation.rs` have **zero hotspot entries
despite heavy incident history** (INC-I-071/104/105/108/109/111). That is a **registration gap, not a
low-risk signal** — do not read their absence as evidence of safety.

### Open questions the DB cannot answer → route to runtime evidence

1. Reconcile the 450 MB baseline against INC-I-104's pre/post-cap numbers — capped correctly, or has the
   column-family count grown since?
2. Is the memory watchdog **armed** on this fleet right now? (INC-I-117 says it was dormant.)
3. **Exact step timestamp vs. exact N6/N12 registration timestamp** — do NOT trust the user's approximate
   date. Establish both independently from Prometheus and logs, then test coincidence.
4. Does `diagnostic_ledger` volume correlate with producer count? (INC-I-101/102 are open on this.)
5. Is INC-I-142's staleness gate deployed fleet-wide, or still partial?

### Recommended read

`docs/postmortems/2026-07-21-inc-i-143-snapsync-cascade.md` — not yet read in full by any agent.
