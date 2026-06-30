# INC-I-120 — Sync-protocol amplification storm: structural fix design

**Status:** ✅ IMPLEMENTED (Layers 1 + 2), session 2026-06-30 — Layer 3 GSet TTL = fast-follow, Layer 4 = ops action
**Implementation:** Run 444. Tests: `crates/network/tests/inc_i_120_sync_governor.rs` (4, FAIL→PASS) + `recovery.rs::tests::stuck_fork_*` (4, FAIL→PASS). Invariants INV-SYNC-009 + INV-FORK-001. Build gate green (release build, clippy `-D warnings`, fmt, network 416 + node tests). NO activation height (no consensus-rule / block-content change), rolling-deploy safe.
**Severity:** critical (mainnet fleet collapse, recurrent ~06/24–06/28)
**Domain:** network/sync
**Diagnosis source:** workflow 442 (4/4 domain convergence) + code re-verification (session 2026-06-30)

---

## 0. FRESH SESSION — START HERE

Resume with: `/omega-doctor --incident INC-I-120`

**State at handoff:** diagnosis complete and code-verified; solution designed and **approved by the user (structural approach, not a patch)**; **no code written yet.** TDD: write failing tests first.

**Do, in order:**
1. Read this whole doc + the incident record (`sqlite3 .omega/memory.db "SELECT * FROM incidents WHERE incident_id='INC-I-120'"`) + findings (`SELECT * FROM findings WHERE finding_id LIKE 'INC-I-120%'`).
2. Implement **Fix 1 + Fix 2** below (the two halves of this incident). **Fix 3 is an ops action** the user runs on servers. GSet pruning is a **fast-follow incident**, not this one.
3. TDD — each fix gets a test that **FAILS before, PASSES after** (see §6).
4. Honor the two guardrails from our own history (§2a) — they are non-negotiable.
5. **Testnet first.** Rolling-deploy safe; **no activation height** (no consensus-rule / block-content change). Confirm both deploy-safety questions before shipping.
6. Build gate: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check && cargo test -p network -p node`.
7. Copy + codesign binary for testnet; ask before commit/push; never mainnet without explicit OK.

**Approved scope:** Fix 1 + Fix 2 now · Fix 3 ops in parallel · GSet pruning = separate fast-follow.

---

## 1. Structural diagnosis (code-verified, code is SOT)

The fleet death is **one structural disease with two faces**, not a collection of unrelated bugs:

> **DOLI's sync request/response subsystem has inbound serving limits but no outbound
> rate governance, and no recovery action when a node is stuck on a fork.**
> A natural fork (inevitable in PoS) therefore self-amplifies into fleet-wide resource collapse.

This is the **same architectural class** INC-I-114 hardened on the *gossip* path (flood control + validation gate). INC-I-120 is that same shape on the **adjacent unhardened path** — sync.

### Verified facts

| # | Fact | Location (verified) |
|---|------|---------------------|
| A | **Inbound** serving cap: single global counter, 24 requests/interval, emits `"busy: sync serving limit reached"` when tripped. Under fleet-wide fork every node trips it instantly. | `bins/node/src/node/network_events.rs:300-322` |
| B | **Outbound busy handling**: on `"busy"` the client blacklists the peer, sets `Idle`, then **immediately** calls `start_sync()` → defeats `start_sync`'s own `is_syncing()` guard → ~40 req/s self-amplified loop = **3.5M req/node/day**. (KILL MECHANISM) | `crates/network/src/sync/manager/sync_engine/response.rs:111-121` |
| C | **No outbound rate governor anywhere.** Every outbound sync request — sync-manager `next_request()`, orphan-chase, silence-pull, attest-fetch, block-by-hash — funnels through the **single chokepoint** `NetworkCommand::RequestSync → sync.send_request()`, which has zero rate limiting. | `crates/network/src/service/command_handling.rs:76-82` |
| D | **Stuck-fork never recovers**: small-gap (≤1000) stuck-sync deliberately does NOT signal fork recovery. | `crates/network/src/sync/manager/cleanup.rs:623-625` |
| E | **Signal is logged-only**: `consume_stuck_fork_signal()` emits a WARN and takes no recovery action. | `bins/node/src/node/periodic.rs:399-407` |
| F | **Reusable infra exists**: per-peer + global `TokenBucket` (blocks/tx/requests/bandwidth). | `crates/network/src/rate_limit.rs` |
| G | **Amplifier (secondary)**: producer GSet CRDT is grow-only, no TTL → re-syncs 9-day-old announcements forever → 8.5M log-lines/day, 7.68 GB/day. | `bins/node/src/node/network_events.rs:471-500` |

**Why a patch is not enough:** fixing only B (busy-retry circuit breaker) removes the *largest* single contributor, but the structural gap (C) means the *next* fork can still amplify through orphan-chase / silence-pull / block-by-hash (collectively ~670K+ req/day, all ungoverned). The responsible fix governs the **chokepoint**, not one call site.

---

### 1a. History linkage (why this recurred — read before coding)

| Prior incident | Relationship to INC-I-120 |
|----------------|---------------------------|
| **INC-I-090** | BUILT the stuck-fork detector (`signal_stuck_fork` / `consume_stuck_fork_signal`) but wired it ONLY to a WARN log — code comment: *"take_stuck_fork_signal() had ZERO non-test callers — the signal sat unread."* **Detector built, response never connected.** INC-I-120 RC-2 (Layer 2) is the unfinished half of this — not a regression. Also fixed a finality-guard fencepost (`<=`→`<`, `recovery.rs:312`). |
| **INC-I-049** | A per-peer rate limiter once **dropped a canonical block → 9-min fork.** This is the cautionary tale for Layer 1: a blunt limiter can CAUSE the fork it tries to prevent. → Guardrail G1. |
| **INC-I-040** | *"Recurring fork problem — 55+ fix attempts."* Long history of symptom-patching without closing the structural gap — evidence the structural fix (not another patch) is correct. |

### 2a. Guardrails from our own history (NON-NEGOTIABLE)

- **G1 (from INC-I-049):** the Layer 1 governor throttles ONLY redundant/retry chatter. It MUST NOT drop or delay (a) canonical block delivery/propagation or (b) a genuine fork-recovery request. Exempt/prioritize critical traffic; budget the retry storm.
- **G2 (from INC-I-090 + INV-SYNC-001/004/008):** Layer 2 recovery may roll back TO finality but NEVER below it (strict `<`). Reset in-memory `last_finality_height` if a rollback ever lands at/below it.
- **G3 (from INC-I-090 reason-for-disable + lessons):** the Layer 2 "stuck" trigger fires ONLY on a sustained, genuinely divergent stall (e.g. ≥300s no block applied AND gap stable AND local tip hash ∉ peer-majority hash) — never on transient gossip lag. Over-triggering is why small-gap signalling was disabled originally.

## 2. The responsible structural solution (layered)

### Layer 1 — Outbound sync-request governor (THE structural fix; subsumes B)
Add a token-bucket governor at the single outbound chokepoint `NetworkCommand::RequestSync`
(`command_handling.rs:76-82`), reusing `rate_limit::TokenBucket`:
- **Per-peer** bucket + **global** bucket. When empty → **drop** the request (do not queue, do not tight-loop). The sync state machine already re-derives needed requests on the next tick, so a dropped request is re-attempted at a governed rate.
- Caps the *rate* of **all** outbound sync request classes regardless of origin — one governor, not N call-site patches.
- Complementary fix at `response.rs:111-121`: on `"busy"`, apply cooperative backoff (do **not** immediately `start_sync()`); the governor is the hard cap, the backoff is the polite behavior.

### Layer 2 — Make fork recovery actually recover (co-equal RC; faces D + E)
- `cleanup.rs`: re-enable small-gap stuck-fork signalling, **guarded** so it fires only on a *sustained* divergent stall (e.g. no block applied for ≥300s AND gap stable AND local tip hash ∉ peer-majority hash), never on transient gossip lag (the original reason it was disabled).
- `periodic.rs`: `consume_stuck_fork_signal()` must invoke the real recovery path (`resolve_shallow_fork()` / bounded rollback), not just WARN.

### Layer 3 — GSet pruning/TTL (secondary amplifier; face G) — *separable*
Bound the grow-only producer GSet (TTL / prune inactive producers / cap delta size) so anti-entropy stops re-syncing 9-day-old announcements. Distinct subsystem; did not drive the kill. Recommend a **fast-follow incident**.

### Layer 4 — Operational backstop (ops config, not repo logic) — *do regardless*
Arm `DOLI_MEMORY_WATCHDOG_BYTES` + add systemd `MemoryMax=` on all 10 mainnet units. The INC-I-114 M2 watchdog code already exists but was never armed (absent on every unit). Last-resort: even an unforeseen amplifier cannot OOM the host.

---

## 3. Deploy safety (MEMORY.md #0b — both questions)

| Layer | Consensus *rules* changed? | Block *content* changed? | Activation height? | Rolling deploy? |
|-------|---------------------------|--------------------------|--------------------|-----------------|
| 1 governor | No (outbound throttle) | No | **Not needed** | **Safe** (each node governs its own outbound) |
| 2 recovery | No (local reorg/rollback behavior, validation unchanged) | No | Not needed | Safe |
| 3 GSet TTL | No | No (producer announcements, re-forward only) | Not needed | Safe (verify convergence) |
| 4 ops | n/a | n/a | n/a | per-node config |

INC-I-075 three-question check (Layer 1): not reachable by user tx as a consensus computation; not consensus-visible (no block/scheduler/validation change) → **no activation height required**.

---

## 4. Recommended scope for THIS incident

**Layers 1 + 2** (kill mechanism + stall recovery) as the core responsible fix, **+ Layer 4** as a required parallel ops action. **Layer 3** as a fast-follow incident.

## 5. Resource cost (Layer 1 governor)

- **CPU:** negligible — one token-bucket check per outbound sync request (a few arithmetic ops + a time read), on a path that today issues millions/day. Net CPU **drops** (fewer requests sent/served).
- **Memory:** one `TokenBucket` per peer (~tens of bytes) + globals. Bounded by peer count (≤50). Negligible.
- **IO/Disk:** strongly **reduced** (the storm is what drove 7.68 GB/day logs + 2.1K read IOps).
- **Network:** strongly **reduced** (the entire point).
- **Latency:** sync requests may be delayed by up to one token-refill interval under load — acceptable; sync is already eventually-consistent and gossip delivers ~94% of blocks.
- **Inevitability:** required — an ungoverned outbound path is the structural defect.
- **Cheaper alternative:** busy-only circuit breaker (face B alone) — rejected as insufficient (leaves C open).

## 6. Test-first (TDD, Output Contract)

1. **RC-1 governor (must FAIL before, PASS after):** drive repeated `SyncResponse::Error("busy")` / repeated request derivation through the chokepoint; assert outbound `send_request` count is bounded by the token-bucket rate over a window.
2. **RC-2 recovery (must FAIL before):** simulate sustained small-gap divergent stall; assert a recovery action (signal → rollback/resolve) is invoked, not just a log.
3. Regression tests linked to new invariants `INV-SYNC-xxx` (outbound governed) and `INV-FORK-xxx` (sustained stall triggers recovery).
