<!--
OUTPUT CONTRACT: N/A — requirements document (not a test file)
INPUT PARTITIONS: N/A — requirements document (not a test file)
-->

# Requirements: Node-Level Low-Disk Self-Protection ("Disk Guardian")

> Origin: INC-class live failure — external mainnet producer "nano" ABRT core-dumped
> (signal 6) in a systemd crash-loop; root cause a 100%-full 38G volume (29G unrotated
> `/var/log/doli/mainnet.log`). Beneficiaries: ~17 unmonitored external VPS producers.
> Structural N1–N12 + seeds already have Prometheus/Grafana (gain near-zero).
> Feature evaluation: GO, FVS 4.3 (`docs/.workflow/feature-evaluation.md`).

---

## ⚠ SCOPE PIVOT (2026-07-17) — REQ-DISK-001..012 SUPERSEDED, Option 1 is ACTIVE

An anti-anchoring skeptic pass + orchestrator code verification found the watchdog scope
below **insufficient and misdirected**: a production halt does not stop the writes that
actually ENOSPC-abort the process (`apply_block` fires on every *received* block, and the
abort mechanism is `panic = "abort"` (`Cargo.toml:120`) + `.expect()` on RocksDB writes —
not anything production-specific). The scope pivoted to **Option 1: make disk-full a
clean, non-crashing condition** (M1 fail-safe foreground writes + M2 bounded log growth).

- **SUPERSEDED (never implemented):** REQ-DISK-001..012 (watchdog poll, production halt,
  hysteresis, NetworkParams thresholds, metrics). Retained below unmodified for
  traceability. They may return later as a *complementary* operator-early-warning feature,
  not as a prerequisite for crash safety.
- **REVERSED:** REQ-DISK-014 ("log rotation is out of scope") — log bounding is now M2,
  shipped as a deployment artifact by the CLI service installer (the node still never
  deletes data itself; logrotate does).
- **ACTIVE requirements:** REQ-DISK-101..106 (M1) and REQ-DISK-201..205 (M2) — see the
  "Option 1 Requirements (ACTIVE)" section at the end of this file.
- **Architecture:** `specs/disk-guardian-architecture.md`.

Everything between this banner and the "Option 1 Requirements (ACTIVE)" section documents
the superseded watchdog scope.

---

## Scope

**In scope (this iteration):** in-process, node-level self-protection inside `doli-node`.
A cheap periodic poll of the **`data_dir` mount's free space**; a **graceful, voluntary
halt of block production** when free space falls below a configurable floor; a **clear,
structured log/metric signal**; and **automatic resume** (with hysteresis) when space is
reclaimed. All thresholds live in `NetworkParams`. Non-consensus, no activation height,
rolling-deploy safe.

**Modules touched (blast radius — see Architecture Context):**
- `bins/node/src/node/periodic.rs` — additive periodic poll (TTL-cached), isolated.
- `bins/node/src/node/production/gates.rs` and/or the production authorization path — new
  early-return gate that consults an isolated disk-halt signal.
- `bins/node/src/node/mod.rs` — one new `Node` field (isolated halt signal + last-poll cache).
- `crates/core/src/network_params/{mod.rs,defaults.rs}` — new tunable fields + per-network defaults.
- `fs2` crate — already a dependency; `available_space()` primitive (currently unused).

**Explicitly NOT in scope:** pausing block application/validation ingest; automatic log
rotation; automatic pruning; any consensus-rule, block-content, or wire-format change.
(See "Out of Scope" for rationale — these are deliberate deferrals, not omissions.)

## Anchor / Skeptic Reconciliation

No `docs/.workflow/skeptic-analysis.md` was present at scoping time, so I ran the fallback
anchor-detection.

- **First read:** "halt block production when disk is low."
- **Contradicting second read:** "production is a small fraction of the node's disk writes
  — `apply_block()` writes on *every* block (self-produced OR received). Halting production
  alone does not stop the write that actually ENOSPC-aborts, so it is theater unless we also
  halt writes/apply."
- **Reconciliation (chosen position, with evidence):** The production halt is the
  consensus-safe, isolation-respecting **core**. It (a) is provably bit-identical to a
  missed slot (`production/mod.rs` L280–296: an unfilled slot is "ALWAYS deterministic",
  no ProducerSet mutation), and (b) fires at a **threshold with margin** — *before* the
  disk reaches 100% — giving the operator a window to intervene and emitting a loud signal
  the unmonitored tail otherwise never gets. The second read is correct that production
  halt does not by itself stop `apply_block()` writes; therefore a critical-level
  apply/ingest pause is a **real candidate** — but it touches the apply/sync hot path,
  which (i) violates the mandated additive-isolation constraint during active INC-I-139 /
  INC-I-138 churn, and (ii) needs its own consensus-safety + sync-state-resume analysis. It
  is deferred as **REQ-DISK-013 (Won't, this iteration)** with the residual risk stated
  explicitly (see Identified Risks R1), not silently dropped.

### The skeptic's four challenges — explicit decisions

1. **Production-only vs also-validation halt** → **Production-only** this iteration
   (REQ-DISK-002 Must). Apply/ingest pause = REQ-DISK-013 (Won't) with rationale + residual
   risk. Halting production is provably consensus-neutral and additive; halting apply strands
   the node from convergence and touches churning hot paths.
2. **Threshold vs ENOSPC reaction** → **Proactive absolute-free-bytes threshold**
   (REQ-DISK-002, REQ-DISK-006 Must). Reactive ENOSPC-catch is post-hoc: the write already
   failed and RocksDB may already be mid-corruption — exactly what we are preventing. A
   free-**bytes** floor (not a percentage) is mount-size-independent and can be sized to
   cover pending writes + compaction headroom.
3. **Flapping** → **Two-watermark hysteresis + minimum dwell** (REQ-DISK-004 Must): halt
   below `low_watermark`, resume only above `high_watermark` (> low) AND after `min_dwell`.
4. **Right layer** → **In-process, node-level** (REQ-DISK-001 Must). External tooling
   (systemd, watchdog scripts, Prometheus) is exactly what the target population does not
   install; only in-node logic reaches the "installs nothing" tail and can pre-empt the
   write.

## Summary (plain language)

Right now, if a `doli-node`'s disk fills up, the process crashes hard (signal 6) and
systemd restarts it into the same full disk — an endless crash-loop that can corrupt the
database mid-write. This feature teaches the node to watch its own data disk. When free
space gets low, the node politely stops producing blocks (which, to the rest of the
network, looks exactly like a producer that happened to miss its turn — nothing breaks and
no consensus rule changes) and writes a loud, clear log message so the operator knows to
free space. When space comes back, the node resumes on its own. It never deletes data and
never writes anything big itself.

## User Stories

- As an **operator of an unmonitored VPS producer**, I want my node to warn me and degrade
  gracefully instead of crash-looping when the disk fills, so that I don't lose my node to
  a corrupted database and an expensive wipe-and-resync.
- As a **producer**, I want a low-disk pause to look exactly like a missed slot to the rest
  of the network, so that my node never forks the chain or shrinks the active producer set
  by pausing.
- As an **operator**, I want the node to auto-resume once I free space, so that recovery is
  hands-off after I fix the disk.
- As a **network maintainer**, I want this to ship as a plain binary upgrade with no
  activation height and no synchronized deploy, so that paused and producing nodes always
  interoperate.
- As an **operator with Prometheus** (structural fleet), I want a gauge for free bytes and
  halt state, so that existing dashboards can alert on it too.

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|--------------------|
| REQ-DISK-001 | Periodic poll of the **`data_dir`** filesystem's available bytes, on the existing periodic tick, TTL-cached so it adds negligible CPU/IO | Must | - [ ] Polls the mount containing `config.data_dir`, NOT `/` nor the log mount<br>- [ ] Uses `fs2::available_space()` (O(1) `statvfs`); no full-FS walk, no new thread<br>- [ ] Result cached ≥30s (TTL pattern like `DEFI_HEALTH_CACHE_TTL`); poll is skipped while cache is warm<br>- [ ] Added additively/isolated in `periodic.rs`; no edit to sync/apply/reorg logic |
| REQ-DISK-002 | Graceful production halt when free bytes `< low_watermark_bytes`, **bit-identical to a missed slot** | Must | - [ ] When below floor, `try_produce_block()` emits **no block** for the slot<br>- [ ] `ProducerSet` / `active_producers` / `excluded_producers` are **unchanged** by the halt<br>- [ ] No Register/AddBond/Exit/Slash/Withdrawal/Delegation is emitted or implied<br>- [ ] From peers' view the node's state is indistinguishable from ordinary missed slots |
| REQ-DISK-003 | Emit a **structured, clear** signal on the halt transition | Must | - [ ] A single structured `tracing::warn!` fires on the **transition** into halted (not per-tick)<br>- [ ] Message includes: data_dir path, free bytes, threshold, and "production halted"<br>- [ ] Field is machine-parseable (structured fields, not only prose) |
| REQ-DISK-004 | **Auto-resume with hysteresis** to prevent flap | Must | - [ ] Resume only when free bytes `≥ high_watermark_bytes` AND `min_dwell` elapsed since halt<br>- [ ] No resume while free bytes are in `[low, high)` (the dead band)<br>- [ ] Repeated crossings within one `min_dwell` window do not toggle production more than once<br>- [ ] Resume emits one structured `info!` on the transition |
| REQ-DISK-005 | Disk-halt signal is **isolated** from other production blocks | Must | - [ ] Disk halt uses a dedicated signal (e.g. `Node`-owned `AtomicBool` / typed gate), NOT the shared `SyncManager.production_blocked` string<br>- [ ] Clearing the disk halt does NOT resume production if an unrelated explicit block (invariant violation) is active<br>- [ ] An unrelated `unblock_production()` does NOT clear the disk halt<br>- [ ] Order of gate checks is deterministic and documented |
| REQ-DISK-006 | Thresholds are **tunable in `NetworkParams`**, per network, no consensus coupling | Must | - [ ] `disk_low_watermark_bytes`, `disk_high_watermark_bytes`, `disk_min_dwell_secs` (or equivalents) exist on `NetworkParams`<br>- [ ] Present in `defaults()` for mainnet, testnet, devnet with sane values (high > low, floor ≥ headroom for pending writes + compaction)<br>- [ ] Changeable via new binary without genesis reset or activation height<br>- [ ] Constructor/validation rejects `high ≤ low` |
| REQ-DISK-007 | **No consensus change, no activation height, rolling-deploy safe** | Must | - [ ] No consensus rule, block content, tx ordering, coinbase, bitfield, or wire format changes<br>- [ ] No `HardForkSchedule` entry, no `*_activation_height`, no protocol/epoch version bump<br>- [ ] A halted node and a producing node on the same binary interoperate; a mixed old/new fleet interoperates (old nodes simply never halt)<br>- [ ] Answers the INC-I-075 three-question checklist: Q1 no user tx path, Q2 no producer/attestation path alters consensus output, Q3 behavior bit-identical for all reachable consensus inputs |
| REQ-DISK-008 | The guardian **must not itself worsen disk usage** | Must | - [ ] No new large/unbounded writes by the guardian<br>- [ ] Alert logging is transition-triggered (plus at most a bounded periodic reminder — REQ-DISK-010), never per-1s-tick spam<br>- [ ] The poll itself performs no disk writes |
| REQ-DISK-009 | Prometheus gauges for free bytes + halt state | Should | - [ ] A gauge exposes current `data_dir` free bytes<br>- [ ] A gauge/bool exposes halted state (0/1)<br>- [ ] Metric update reuses the cached poll value (no extra syscalls) |
| REQ-DISK-010 | Bounded periodic reminder while halted | Should | - [ ] While halted, re-emit the warning at a bounded cadence (e.g. ≤ every 5 min)<br>- [ ] Reminder stops on resume<br>- [ ] Cadence is not the 1s tick |
| REQ-DISK-011 | Surface disk-guardian status via RPC/CLI health output | Could | - [ ] An existing status/health RPC includes free bytes + halted flag |
| REQ-DISK-012 | Configurable **override to disable** the guardian (belt-and-suspenders) | Could | - [ ] A NetworkParams/config value (e.g. `low_watermark = 0`) disables the halt while keeping the metric |
| REQ-DISK-013 | Pause **block application / sync ingest** at a critical threshold | Won't | N/A (deferred — see Out of Scope R1; touches churning apply/sync hot path, needs own consensus-safety analysis) |
| REQ-DISK-014 | Automatic **log rotation / disk cleanup** by the node | Won't | N/A (node must never delete data; log rotation is an OS/logrotate concern) |
| REQ-DISK-015 | Automatic **pruning trigger** under disk pressure | Won't | N/A (pruning is a deliberate operator action; auto-pruning risks unintended data loss) |

## Acceptance Criteria (detailed)

### REQ-DISK-002: Graceful production halt == missed slot
- [ ] Given free bytes `< low_watermark`, when the producer's slot arrives, then no block is
      produced and the slot is left empty (identical code outcome to the existing missed-slot
      path at `production/mod.rs` L280–296).
- [ ] Given the node halts for N consecutive slots, when peers evaluate its state, then they
      observe only missed slots — no Exit/Withdrawal, no change to `active_producers_at_height`.
- [ ] Given the node is halted, when an epoch boundary is crossed, then the node's on-chain
      active producer set derivation is byte-identical to a node that merely missed those
      slots (no divergence, no fork).
- [ ] Edge: halting exactly at a slot the node was scheduled to produce → empty slot, next
      scheduled producer proceeds normally.

### REQ-DISK-004: Hysteresis / anti-flap
- [ ] Given free bytes oscillate rapidly across `low_watermark`, when polled, then production
      does not toggle on each crossing (dead band `[low, high)` holds the state).
- [ ] Given the node halted at T0, when free bytes exceed `high_watermark` at T0+ε (< min_dwell),
      then the node stays halted until `min_dwell` has elapsed.
- [ ] Given the node halted, when free bytes recover above `high_watermark` and `min_dwell`
      passes, then production resumes and one `info!` transition event fires.

### REQ-DISK-005: Signal isolation
- [ ] Given an invariant-violation explicit block is active AND disk is low, when disk recovers,
      then production stays blocked (the invariant block still holds).
- [ ] Given disk is low (halted) AND some subsystem calls `unblock_production()`, when the next
      slot arrives, then the node still does not produce (disk gate independent).
- [ ] Given both gates clear, then and only then does production resume.

### REQ-DISK-007: Rolling-deploy / no-consensus safety
- [ ] Given a mixed fleet (some nodes upgraded, some not), when an upgraded node halts on low
      disk, then un-upgraded nodes treat it as a normal missed-slot producer — no rejection, no fork.
- [ ] Given the change, the INC-I-075 three-question checklist is answerable: no user
      transaction and no producer/attestation pattern can alter any consensus-visible output;
      the pause changes only *whether* this node emits its own block, which the scheduler
      already treats as a deterministic empty slot.

## Architecture Context (modification workflow — gate checkpoint)

### Module Boundaries
- **`periodic.rs :: run_periodic_tasks()`** — Responsibility: cheap, cached, read-only health
  polls on the 1 Hz tick. Depends on: `self.config.data_dir`, `fs2`, `Instant` TTL cache.
  Depended by: the production gate reads the halt signal it sets. Precedent: `DEFI_HEALTH_CACHE_TTL`
  (30s TTL, INC-I-111), `utxo_size_monitor.rs`, `checkpoint_health.rs`. **This is the direct
  idiom to clone.**
- **`production/gates.rs` + `handle_production_authorization()`** — Responsibility: chain of
  early-return production gates (sync, peers, explicit, bootstrap, canonical). Depends on:
  `SyncManager::can_produce()`. Depended by: `try_produce_block()`. The disk gate is a new
  peer of the ~6 existing gates.
- **`SyncManager` production gate (`crates/network/.../production_gate.rs`)** — owns
  `production_blocked: Option<String>` (a **single shared slot**) via `block_production()` /
  `unblock_production()` / `production_block_reason()`. **CONSTRAINT:** this is shared and
  single-valued — the disk guardian must NOT reuse it (would clobber invariant-violation
  blocks and be clobbered by `unblock_production()`). Hence REQ-DISK-005 mandates an isolated
  `Node`-owned signal.
- **`NetworkParams` (`crates/core/src/network_params/`)** — Responsibility: per-network
  tunables + `defaults()`. Home of the three thresholds. Accessed on `Node` via `self.params`
  (type `ConsensusParams`) — architect to confirm the exact field wiring; `self.config.data_dir`
  holds the mount to poll.

### Data Flows Through Affected Area
- **New flow (additive):** 1 Hz tick → (TTL-gated) `fs2::available_space(data_dir)` → compare
  to watermarks with hysteresis/dwell → set/clear isolated `Node` halt flag + update metric +
  transition log. No data leaves the node; nothing is written.
- **Consumed by:** production gate reads the flag → returns "no block this slot" (empty slot).
- **Downstream consumers unchanged:** scheduler, epoch boundary, `active_producers_at_height`,
  peers — all see only "this producer didn't emit," a path they already handle deterministically.

### Architectural Constraints & Invariants
- **INV-CONSENSUS-001** (active): the eligible/active producer set MUST be a deterministic
  function of (ProducerSet state, NetworkParams). — The disk halt MUST NOT feed into that
  function. Since it only suppresses this node's own block emission (a missed slot), it does
  not. *Violated only if* someone routed the halt through `active_producers`/`excluded` — which
  REQ-DISK-002 forbids.
- **Missed-slot determinism** (`production/mod.rs` L280–296): an unfilled slot is always
  deterministic; emergency-equalization was removed as the #1 fork source. The halt MUST land
  on exactly this path — never a new "special" empty-slot variant.
- **Additive-isolation constraint** (INC-I-139 / INC-I-138 churn in `periodic.rs` and sync):
  the poll must be a self-contained block; must not restructure the surrounding tick or touch
  sync/apply/reorg. Merge-friction is the only flagged caution.
- **INV-APPLY-012 spirit** (no O(N) in hot path): the poll is O(1) `statvfs` and TTL-cached —
  compliant. It runs in the periodic loop, not `apply_block()`.
- **#0 RULE (no genesis reset):** thresholds are runtime `NetworkParams`, activate on binary
  swap, need no activation height and no reset.

### Blast Radius (graph-verified)
- `blast.py block_production` → 1 dependent: `SyncManager` (`production_gate.rs`). Confirms the
  shared-slot risk is contained to one owner (reinforces REQ-DISK-005 isolation).
- `blast.py run_periodic_tasks` → dependents are `Node` + 3 unit tests only. Adding to it does
  not ripple beyond `Node`.
- **Direct impact:** `periodic.rs`, `production/gates.rs`, `node/mod.rs`, `network_params/*`.
- **Indirect impact:** none in consensus — the halt output merges into the pre-existing empty-slot
  path. Grep confirms **no** existing free-disk check anywhere → net-new, no collision.

### Failure-Mode Matrix (gauntlet.conf present — system-impact protocol)
| Recorded mode | Source | Behavior of this change in that mode |
|---------------|--------|--------------------------------------|
| Liveness-exclusion cascade | INC-I-016 / PM-013 | A self-halted producer misses the **same** slots a crash-looping node misses today → **not a new** exclusion source; capped by `MAX_EXCLUSIONS_PER_BLOCK` / `max_excluded_total`. Mitigation: loud alert + fast operator recovery + hysteresis (avoid needless halts). |
| Epoch-boundary liveness prune | INC-I-116 | A paused producer is non-participating like any missed-slot producer; pruning derivation is deterministic and unchanged. No new pruning trigger, no divergence. |
| Scheduler / excluded divergence | INC-I-026 | The halt writes NO local HashSet feeding the scheduler (REQ-DISK-002/005). `active_producers` untouched → no divergence, no fork. |
| Disk I/O spike / event-loop freeze | INC-I-072 | The poll is O(1) `statvfs`, TTL-cached (≥30s) → adds negligible IO; must not add load (REQ-DISK-001/008). |
| Snap/sync admission churn | INC-I-139 / INC-I-138 | Change is additive and isolated to `periodic.rs`; touches no sync/apply path → cannot interact with snap-admission logic. |
| Production flap | new (this feature) | Two-watermark hysteresis + `min_dwell` (REQ-DISK-004) bounds toggling to ≤1 per dwell window. |
| Prolonged self-halt strands node | new (this feature) | Recoverable: auto-resume on space return; residual documented in R1. Strictly better than ENOSPC abort + wipe/resync. |

## Impact Analysis

### Existing Code Affected
- `bins/node/src/node/periodic.rs` — add one cached poll block. **Risk: low** (additive), medium
  merge-friction (active INC-I-139/138 churn).
- `bins/node/src/node/production/gates.rs` (or the auth path) — add one early-return gate.
  **Risk: low-medium** (a new gate in the most consensus-sensitive chain; must land on the
  existing empty-slot path).
- `bins/node/src/node/mod.rs` — one new `Node` field. **Risk: low.**
- `crates/core/src/network_params/{mod.rs,defaults.rs}` — new fields + 3 default sets. **Risk: low.**

### What Breaks If This Changes Incorrectly
- If the halt is routed through `active_producers`/`excluded`/`production_blocked` → **fork /
  active-set shrink** (INV-CONSENSUS-001, INC-I-026). Mitigation: REQ-DISK-002 + REQ-DISK-005.
- If the poll is not cached / walks the FS → event-loop freeze (INC-I-072). Mitigation:
  REQ-DISK-001 (O(1) + TTL).
- If it polls `/` or the log mount instead of `data_dir` → wrong mount, false negatives, and it
  fails to protect the write path. Mitigation: REQ-DISK-001 acceptance criteria.
- If it reuses `production_blocked` → clobbers/gets-clobbered by invariant-violation blocks.
  Mitigation: REQ-DISK-005.

### Regression Risk Areas
- Production gate ordering (must not change existing gate outcomes for non-low-disk nodes).
- Epoch-boundary determinism for a node that paused across a boundary.
- Metric hot path (must reuse cached value).

## Traceability Matrix
| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|----------------------|
| REQ-DISK-001 | Must | (test-writer) | (architect) | periodic.rs |
| REQ-DISK-002 | Must | (test-writer) | (architect) | production/gates.rs |
| REQ-DISK-003 | Must | (test-writer) | (architect) | periodic.rs |
| REQ-DISK-004 | Must | (test-writer) | (architect) | periodic.rs / node state |
| REQ-DISK-005 | Must | (test-writer) | (architect) | node/mod.rs + gates.rs |
| REQ-DISK-006 | Must | (test-writer) | (architect) | network_params/* |
| REQ-DISK-007 | Must | (test-writer) | (architect) | (cross-cutting) |
| REQ-DISK-008 | Must | (test-writer) | (architect) | periodic.rs |
| REQ-DISK-009 | Should | (test-writer) | (architect) | metrics |
| REQ-DISK-010 | Should | (test-writer) | (architect) | periodic.rs |
| REQ-DISK-011 | Could | (test-writer) | (architect) | rpc |
| REQ-DISK-012 | Could | (test-writer) | (architect) | network_params/* |
| REQ-DISK-013 | Won't | N/A | N/A | N/A (deferred) |
| REQ-DISK-014 | Won't | N/A | N/A | N/A (deferred) |
| REQ-DISK-015 | Won't | N/A | N/A | N/A (deferred) |

## Security Gate Evaluation
Trust-boundary triggers checked: external data ingestion, multi-user sharing, network comms,
shell/SQL/path construction from external input, untrusted deserialization. **None apply.** The
only input is the OS `statvfs` reading of the node's **own** `data_dir` mount; the threshold
comes from local `NetworkParams`. No external/untrusted data crosses into this feature → **no
`REQ-DISK-SEC-*` requirements required.** (If a future iteration exposes threshold override via
RPC/CLI, re-run this gate for input validation on that value.)

## Specs Drift Detected
- None in the affected area — grep confirms no prior free-disk logic and no spec describing one.
  This document is the first spec for the domain.

## Assumptions
| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|------------------------|------------------------------|-----------|
| 1 | `self.config.data_dir` is the RocksDB/state mount to protect (the write path that ENOSPC-aborts) | We watch the folder where the node stores its blockchain data, not the log folder or the OS root | No — arch to confirm data_dir == state_db mount |
| 2 | An absolute **free-bytes** floor (not a %) is the right metric, sized to cover pending writes + compaction headroom | We measure "how many gigabytes are left," which works the same on a tiny or huge disk | No — arch/economist to pick default values |
| 3 | The halt lands on the existing missed-slot path (`production/mod.rs` L280–296) with no new empty-slot variant | A paused node looks to everyone else exactly like one that missed its turn | Yes — code-verified |
| 4 | The disk halt must use a `Node`-owned isolated signal, not `SyncManager.production_blocked` | The node uses a separate on/off switch so it can't accidentally cancel other safety pauses | Yes — code-verified (shared single-slot string) |
| 5 | `NetworkParams` field access on `Node` is via `self.params` (`ConsensusParams`) | The tunable knobs live in the node's config object | No — arch to confirm exact field |
| 6 | `fs2::available_space()` is available and O(1) on the target platforms (Linux VPS = statvfs) | The "check free space" call is cheap and instant | Yes — fs2 already a dependency |

## Identified Risks
- **R1 (residual, primary):** Production-halt alone does not stop `apply_block()` writes from
  *received* blocks. If the disk reaches 100% before the operator acts, `apply_block()` can
  still ENOSPC-abort — the very crash we target. **Mitigation:** the free-bytes floor is set
  with margin (operator window) + loud repeated alert (REQ-DISK-003/010). **Deferred full fix:**
  critical-level apply/ingest pause (REQ-DISK-013), out of scope this iteration by the
  additive-isolation constraint. This risk is *reduced*, not eliminated — and even the current
  scope is strictly safer than today (clean warning + degraded mode vs. immediate crash-loop).
- **R2:** Over-aggressive threshold → false-positive halts → recoverable **reward loss** (missed
  slots). Mitigation: conservative defaults + hysteresis (REQ-DISK-004/006).
- **R3:** Prolonged self-halt could contribute to another node's liveness exclusion (INC-I-016).
  Mitigation: not a *new* risk (a crashed node misses the same slots), capped by exclusion
  limits; fast auto-resume.
- **R4:** Merge friction with in-flight INC-I-139/138 work in `periodic.rs`. Mitigation: keep
  the poll a self-contained, additive block.

## Out of Scope (Won't — this iteration)
- **REQ-DISK-013 — apply/ingest pause at a critical threshold.** Deferred because it touches the
  apply/sync hot path (violates additive-isolation during INC-I-139/138 churn) and requires its
  own consensus-safety + clean-resume analysis. Strong candidate for a follow-up iteration; R1
  documents the residual risk it would close.
- **REQ-DISK-014 — node-driven log rotation / cleanup.** The node must never delete data; log
  rotation is an OS/`logrotate` concern (and was the proximate cause at "nano", solvable
  operationally).
- **REQ-DISK-015 — auto-pruning under pressure.** Pruning is a deliberate operator action;
  triggering it automatically under disk pressure risks unintended, unrecoverable data loss.

## What I Don't Fully Understand (to resolve with architect)
- Whether `config.data_dir` and the RocksDB `state_db`/`block_store` mounts can ever differ
  (bind mounts / symlinks). If they can, the poll target must follow the actual DB path
  (Assumption 1).
- The exact `NetworkParams` vs `ConsensusParams` field wiring on `Node` (Assumption 5).
- Sensible default watermark values per network — needs sizing against real block/compaction
  write bursts (input for architect + any economics review of reward-loss tradeoff).

---

# Option 1 Requirements (ACTIVE — 2026-07-17 scope pivot)

> Design: `specs/disk-guardian-architecture.md`. Non-consensus, no activation height,
> rolling-deploy safe (INC-I-075 checklist answered in the architecture doc).
> M1 = fail-safe foreground writes; M2 = bound log growth. Milestones are independent.

## Requirements (M1 — Fail-safe foreground writes)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|--------------------|
| REQ-DISK-101 | All `StateDb` direct disk-write methods return `Result<_, StorageError>`; no `.expect()`/`.unwrap()` on RocksDB write ops in non-test `state_db` code | Must | - [ ] `insert_utxo`, `remove_utxo`, `import_utxos`, `add_transaction`, `clear_and_write_genesis`, `put_undo`, `clear_utxos` return `Result` (see architecture D1 change list with file:line)<br>- [ ] `rg '\.expect\(' crates/storage/src/state_db` shows no RocksDB write-op matches in non-test code (bincode serialization expects allowlisted)<br>- [ ] TDD: Linux tmpfs ENOSPC reproduction test FAILS (panics) pre-change, PASSES (returns `Err`) post-change |
| REQ-DISK-102 | Success path is **bit-identical**: same WriteBatch contents, same write order, same state root | Must | - [ ] Diff per method is line-local (`.expect(...)` → `?`/`Ok`)<br>- [ ] Existing state-root convergence + storage test suites pass with zero success-path expectation changes<br>- [ ] No consensus/version constant touched (no `CURRENT_PROTOCOL_VERSION`/`EPOCH_STATE_FORMAT_VERSION`/`MIN_PEER_PROTOCOL_VERSION` bump, no activation height) |
| REQ-DISK-103 | All non-test callers propagate the error (no new swallow sites) | Must | - [ ] `crates/storage/src/utxo/set.rs` RocksDb wrapper arms propagate via `?` (signatures already `Result`)<br>- [ ] `bins/node/src/node/init.rs:233,369` and `bins/node/src/operations/chain.rs:133,138` propagate via `?`<br>- [ ] Rollback path surfaces `Err` instead of aborting (fault-injection test) |
| REQ-DISK-104 | Process behavior on ENOSPC: clean error, never SIGABRT | Must | - [ ] Gauntlet disk-full scenario: fill data mount → zero SIGABRT / core dumps; structured error logged; systemd not crash-looping<br>- [ ] Free space → node restarts/resumes cleanly and re-converges (state root matches fleet) |
| REQ-DISK-105 | `clear_utxos` no longer silently swallows write errors (`let _ =` at writes.rs:92) | Should | - [ ] Returns `Result<(), StorageError>`; callers propagate |
| REQ-DISK-106 | Startup ENOSPC produces an actionable error message | Could | - [ ] Error names the data_dir mount and suggests freeing space / checking `/var/log/doli` |

## Requirements (M2 — Bound log growth)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|--------------------|
| REQ-DISK-201 | `doli service install` (systemd path) writes a size-capped logrotate drop-in for the log file its unit redirects to | Must | - [ ] Writes `/etc/logrotate.d/doli-{network}` with `maxsize 200M`, `rotate 5`, `copytruncate`, `compress`, `missingok`, `notifempty`<br>- [ ] `copytruncate` present (systemd holds the append fd; rename rotation would be bypassed)<br>- [ ] Re-install is idempotent (overwrites)<br>- [ ] Unit test asserts generated content byte-exactly |
| REQ-DISK-202 | Steady-state log usage is bounded and the ceiling is documented | Must | - [ ] Bound ≈ (rotate+1)×maxsize (~1.2G) + one inter-rotation burst day, stated in docs<br>- [ ] `logrotate -d` accepts the generated config; forced rotation truncates while the node keeps logging to the same path |
| REQ-DISK-203 | `doli service uninstall` removes the drop-in | Should | - [ ] Drop-in file removed on uninstall; absent-file case tolerated |
| REQ-DISK-204 | Docs tell existing (nano-class) operators how to adopt without waiting for reinstall | Should | - [ ] `docs/troubleshooting.md` disk-full section + copy-paste drop-in snippet<br>- [ ] `docs/producer_node_quickstart.md` note |
| REQ-DISK-205 | In-node log rotation (`tracing-appender`) | Won't | N/A — rejected: rotates by time not size, duplicates/bypasses the systemd append model, adds a dependency + operator migration (architecture D2) |

## Traceability Matrix (Option 1)

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|----------------------|
| REQ-DISK-101 | Must | `disk_guardian_failsafe_test.rs`: insert/remove/add_transaction/import_utxos/clear_and_write_genesis/put_undo `_success_*` + `_on_failing_db_returns_err_not_panic` | disk-guardian-architecture.md §D1 | `state_db/{writes,undo}.rs` |
| REQ-DISK-102 | Must | `disk_guardian_failsafe_test.rs`: `*_success_is_ok_*` round-trip + `state_root_is_bit_identical_across_equivalent_sequences` + `remove_utxo_absent_returns_ok_none` + `import_utxos_empty_iterator_is_ok_noop` | §D1 bit-identity + §INC-I-075 | `state_db/writes.rs` (diff shape) |
| REQ-DISK-103 | Must | `disk_guardian_failsafe_test.rs`: `caller_propagates_err_via_question_mark`, `rollback_wrapper_insert_and_remove_surface_err` | §D1 callers + §Integration Points | `utxo/set.rs`, `init.rs`, `operations/chain.rs` |
| REQ-DISK-104 | Must | `disk_guardian_failsafe_test.rs`: `*_on_failing_db_returns_err_not_panic` (unit proxy for ENOSPC via read-only handle); gauntlet disk-full = system-level | §Failure-Mode Matrix | (system-level) |
| REQ-DISK-105 | Should | `disk_guardian_failsafe_test.rs`: `clear_utxos_success_is_ok_and_wipes`, `clear_utxos_on_failing_db_returns_err_not_panic` | §D1 #7 | `state_db/writes.rs` |
| REQ-DISK-106 | Could | (test-writer) | §Failure-Mode Matrix (startup row) | `init.rs` |
| REQ-DISK-201 | Must | (test-writer) | §D2 | `bins/cli/src/cmd_service.rs` |
| REQ-DISK-202 | Must | (test-writer/ops) | §D2 ceiling | `cmd_service.rs` + docs |
| REQ-DISK-203 | Should | (test-writer) | §D2 | `cmd_service.rs` |
| REQ-DISK-204 | Should | N/A (docs) | §D2 adoption | `docs/troubleshooting.md`, `docs/producer_node_quickstart.md` |
| REQ-DISK-205 | Won't | N/A | §D2 rejection of (a) | N/A |
