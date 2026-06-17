# INC-I-114 Part B — Operator Handoff (TDD hardening complete; agent deployed nothing)

**Workflow run:** 434 · **Branch:** `main` · **Status:** committed, NOT pushed, NOT deployed.

## What shipped (3 commits on `main`, on top of cc1b9c77 baseline)

| Commit | Milestone | What |
|--------|-----------|------|
| `56fd1b82` | M1 | Bounded load-shedding gossip event queue (`service/backpressure.rs`). Block hot path uses non-blocking `enqueue_or_shed` (try_send + drop + count) so the swarm task never suspends → libp2p's internal event VecDeque drains → heap bounded. |
| `71031584` | M2 | Memory watchdog (`watchdog.rs`). Samples RSS (Linux `/proc/self/statm`; fail-open elsewhere), trips a shared shed-flag at a configurable SOFT threshold below the OOM ceiling, sheds all inbound gossip blocks until recovery. **Default-disabled** (`memory_watchdog_threshold_bytes=0`); opt in via `DOLI_MEMORY_WATCHDOG_BYTES`. |
| `48d12f16` | M3 | INV-NETWORK-002 construction-time gate (`gossip/config.rs`). `new_gossipsub()` **fails to start** if aggressive gossip config (flood_publish / short dedup) lacks `validate_messages=true` + bounded queue — blocks silent re-introduction of the OOM class. |

## Deploy artifact
- Build: `cargo build --release` (done — `target/release/doli-node`, 21.78 MB).
- Deploy **`doli-node`** to all structural nodes (N1–N12 + 3 seeds). Per-service copies on mainnet (`doli-node-seed`, `doli-node-n1`, …) — replace ALL.
- macOS local copies: `codesign --force --sign -` after `cp`.

## ⚠️ Synchronized-deploy reminder (REQUIRED)
These changes are **NOT consensus rules (Q1=NO)** and **do NOT change block content (Q2=NO)** — no activation height. **BUT** the INC-I-114 trigger *is* a rolling restart of a partially-vulnerable mesh. Deploy to a **clean fleet**: synchronized **stop-all → start-all** on the hardened binary. Do not roll one node at a time into a live storm.

## Behavior on deploy (defaults)
- M1 is **always active** (no config) — the heap-bounding fix is on by default.
- M2 watchdog is **off until you set `DOLI_MEMORY_WATCHDOG_BYTES`** to a soft threshold below each box's OOM ceiling (e.g. ~70–80% of RAM on the 3.8 GB boxes). Recommend enabling it given the lineage.
- M3 gate is passive (production config already satisfies it).

## Observability (metrics registered in memory.db as monitoring_signals; Prometheus export is a follow-up, NOT yet wired)
- `gossip_blocks_shed_total` — `>0 sustained` = ingestion overload / memory pressure.
- `memory_watchdog_trips_total` — `>0` = node hit the soft memory threshold.
- `process_resident_bytes` — alert well below the OOM ceiling.

## Verification done in-repo (no deploy)
`cargo build --release` ✓ · `cargo clippy -p network --all-targets -D warnings` ✓ · `cargo fmt --check` ✓ · `cargo test -p network --lib` = **412 passed / 0 failed** ✓ · `cargo check --workspace` ✓.
RED→GREEN evidence per milestone in each commit message.

## LOCAL testnet validation (2026-06-17, ~/testnet, scripts/testnet.sh — NO pkill)
Deployed new `doli-node` to `~/testnet/bin` (codesigned) and `restart`ed the live cohort (seed + n1–n5) via launchd.

| Check | Result |
|-------|--------|
| All 6 nodes reboot on new binary | ✅ — **proves M3**: production gossip config passes the INV-NETWORK-002 construction assert (else `new_gossipsub()` Errs → node won't boot) |
| Chain liveness | ✅ advanced **529 → 552** (23 blocks), all converged, **0 forks** |
| M1 spurious sheds under normal load | ✅ **0** `[GOSSIP_SHED]` lines across all nodes (inert until a real flood, by design) |
| Panics / `INV-NETWORK-002 violation` / GossipSub errors | ✅ **0** |
| M2 watchdog runtime path | ✅ proven on n5 via `DOLI_MEMORY_WATCHDOG_BYTES`: `[ENV]…` → `[MEM_WATCHDOG] Enabled: soft_threshold=16384 MB` → `Memory sampler unavailable on this platform — watchdog inactive (fail-open)`. n5 plist reverted byte-identical to baseline afterward. |

**Limitation (honest):** the macOS testnet RSS sampler returns `None` (Linux-only `/proc/self/statm`), so M2 *active shedding* cannot be triggered here — only construction + fail-open. Actual Healthy→Shedding behavior is covered by the 16 unit tests and will be live only on the Linux mainnet/seed fleet. Likewise M1's flood-shedding isn't exercised by normal testnet traffic (unit-test `stale_block_flood_sheds_and_stays_bounded` covers it).

**Verdict:** Part B is safe on a live network — no regression to liveness/convergence, gate passes, watchdog wires up fail-safe. Ready to propose for synchronized mainnet deploy (clean fleet, stop-all → start-all).

## Not done (by design / your call)
- No push, no deploy, no node restart, no version bump (operator owns 100% of deploy).
- Prometheus export of the 3 shed/watchdog metrics — follow-up.
