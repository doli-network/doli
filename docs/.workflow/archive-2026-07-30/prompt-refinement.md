# Prompt Refinement — /omega-swarm --deep (mainnet memory/CPU step-change)

Original:
```
/omega-swarm --deep [Image #1] look at this weird behavior, We've had problem after problem, and they
continue to be related to memory and similar behavior. We restarted Genesis about two weeks ago with
three seeds and five producer nodes, and it had been stable until the day I started nodes 6 and 12 and
registered them as producers. That's when the memory usage spiked from approximately 450MB to about
1.9GB. This issue has affected all nodes. If you review our history of related problems in the database
and in the commits, you'll see everything. What's happening now? And if you look closely, the CPU
behavior has also changed.
```

Anchors detected:
- `"they continue to be related to memory"` → REFRAME — cause anchor. Pre-classifies the root cause as a
  memory-subsystem defect. RSS growth is an OBSERVABLE, not a cause; it may be the consequence of a
  consensus/gossip/sync workload change. Restated as hypothesis H-MEM, not premise.
- `"That's when the memory usage spiked"` (implying node-6/12 registration caused it) → REFRAME — temporal
  correlation stated as causation. Restated as: "the step change is temporally coincident with N6/N12
  start + producer registration; establish whether the registration, the node count, the producer-set
  size change, or an unrelated concurrent event is the causal input."
- `"If you review our history of related problems in the database and in the commits, you'll see everything"`
  → REFRAME — layer/source anchor. Directs investigation to memory.db + git history as if the answer is
  already recorded there. Restated as: "prior incidents and the deployed commit range are MANDATORY
  starting evidence, but are not the boundary of the investigation. Runtime evidence from the live fleet
  is required to confirm or refute any historical match."
- `"look at this weird behavior"` / `"problem after problem"` → STRIP — affective framing, no diagnostic content.

Domain context preserved:
- [image] Grafana time-series dashboard (mainnet monitoring stack, ai5 / https://monitor.doli.network)
  showing a step change in per-node memory usage and a change in CPU signal shape.
- [metric] Memory: ~450 MB → ~1.9 GB steady state (≈4.2×) — step change, not a linear leak ramp.
- [metric] CPU: behavior/shape changed at (or near) the same boundary — magnitude unstated by user.
- [topology] Chain restarted from fresh genesis ~2026-07-22 (`9647b809 chore(release): v6.24.0 — fresh
  mainnet genesis 2026-07-22`), initially 3 seeds + 5 producer nodes.
- [event] Trigger boundary: nodes **N6** and **N12** started AND registered as producers on the same day.
- [scope] "This issue has affected all nodes" — the step change is fleet-wide, NOT confined to the two
  newly-started nodes. This is the single most discriminating fact in the report.
- [baseline] Stable for ~1 week+ post-genesis at ~450 MB before the boundary.
- ⚠️ CONSTRAINT: **This is MAINNET** (3 seeds = ai1/ai2/ai3; producers N1–N12 across ai1/ai2/ai4/ai5).
  Investigation is **READ-ONLY**. No config changes, no restarts, no deploys, no wipes, no mainnet writes.
- ⚠️ CONSTRAINT: Remote node app logs are in FILES (e.g. `/var/log/doli/mainnet/seed.log`), NOT journalctl.
- ⚠️ CONSTRAINT: All servers reachable via `ssh <alias>` (ai1–ai5) from `~/.ssh/config`.

Regression context: DETECTED — baseline = fresh mainnet genesis `9647b809` (v6.24.0, 2026-07-22);
                    range = `9647b809..HEAD` plus the pre-genesis fix batch `dc178d70..` and the
                    INC-I-139/142/143/144/145 series that shipped immediately before genesis.
                    Git archeology mandated for all investigators.

Refined:
```
On DOLI MAINNET, per-node resident memory made a STEP CHANGE from a stable ~450 MB baseline to ~1.9 GB
(≈4.2×), and CPU utilization changed shape at approximately the same time. The step is FLEET-WIDE — it
affected all nodes, including nodes that were already running and unchanged, not only the two nodes that
were started at that moment. The temporal boundary coincides with starting node N6 and node N12 and
registering both as producers. The chain was restarted from fresh genesis on 2026-07-22 (v6.24.0,
commit 9647b809) with 3 seeds + 5 producers, and ran stably at ~450 MB for roughly a week before this
boundary.

Determine what is actually consuming the additional ~1.45 GB per node and why the consumption is
fleet-wide rather than local to the two new nodes. Establish the causal input: is it (a) the producer-set
size / scheduler-and-attestation working set growing from 5→7 active producers, (b) the peer/connection
count growing (per-peer buffers, gossip mesh, sync sessions), (c) a workload change that only manifests
above a producer or peer threshold, (d) an allocator/cache growth that is bounded and benign
(RocksDB block cache, mempool, block cache, epoch-state rebuild), or (e) an unrelated concurrent change
in the deployed binary. Distinguish "unbounded growth / leak" from "bounded steady-state that scales with
N" — these have opposite remediations, and the wrong one costs a genesis reset.

Explain the CPU shape change with the same rigor: identify which work (validation, gossip
forward/dedup, attestation aggregation, state-root computation, sync, RocksDB compaction) increased,
and whether it scales with producer count, peer count, or block content.

Routing directive: Classify the problem from symptoms, not from the user's framing. A "memory problem"
may have a consensus, gossip, or sync root cause. A "code bug" may have an infrastructure root cause.
Note explicitly whether the observed steady state is EXPECTED for a 7-producer / N-peer fleet — a
correct-but-costly design is a different finding than a defect, and must not be reported as a bug.

⚠️ REGRESSION CONTEXT DETECTED
Baseline: fresh mainnet genesis 9647b809 (v6.24.0, 2026-07-22); prior stable-at-450MB window is the
          ~1 week after genesis with 3 seeds + 5 producers.
Deployed range: 9647b809..HEAD, plus the immediately-preceding fix batch that is running in the deployed
          binary (INC-I-139 snap admission, INC-I-142 gossip staleness gate, INC-I-143 fork-guard/wedge
          escape + SiblingFetch + snap-anchor gates, INC-I-144 height-index purge, INC-I-145 archive
          repair, lazy state-root commitment 0a0016e1/df974e06/63fc90b1, BLS aggregate removal
          427d5050/86bac138, disk-guardian ec6afc52).

MANDATORY before forming any root-cause hypothesis:
1. Identify the suspected affected code paths from the symptoms (memory-resident structures: UTXO set,
   ProducerSet/EpochState, block cache, gossip dedup/staleness caches, sync manager buffers, peer
   buffers, RocksDB caches, mempool, attestation aggregation).
2. Run `git log 9647b809..HEAD -- <suspected_paths>` AND review the pre-genesis fix batch listed above,
   and enumerate EVERY commit that touched those paths.
3. For each commit, read the actual change and assess whether it could plausibly cause a fleet-wide
   4.2× step in RSS that only manifests once producer count crosses 5→7 (or peer count crosses a
   threshold).
4. NO investigator may conclude "pre-existing defect" or "expected behavior" without explicitly
   reviewing this diff and ruling each commit out with evidence.

This constraint applies to ALL parallel investigators and the synthesizer. Findings without
git-archeology evidence are incomplete and trigger PRELIMINARY (not VERDICT) status under the
evidence-floor protocol.
```
