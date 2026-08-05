# State-Root / 3-State Commitment Architecture (PROPOSAL-ONLY)

> 5-evaluator convergence synthesis (2026-07-18). RUN_ID = none (proposal-only), INC_ID = none.
> Evaluator reports: `docs/.workflow/design-{subtraction,restructure,patterns,failures,radical}.md`.
> Reasoning trace: `docs/.workflow/architecture-reasoning.md`.
> Analyst scoping: `docs/redesigns/state-root-redesign-analysis.md`.
> 2026-07-18 addendum 1: a focused read-only verification pass closed all 3 Tier-0 correctness
> gaps from the repo and widened the Tier-0 diff scope by one file — Tier 0 re-scored 0.85 → 0.94.
> 2026-07-18 addendum 2: a live-network check (repo Phase 1 + read-only ai5 Phase 2) CLOSED the
> final operational question — the per-block divergence LOG is not load-bearing for detection —
> Tier 0 re-scored 0.94 → **conf(0.97, converged) — VERDICT: GO**.

## Problem Statement

DOLI's 3-state commitment (`compute_state_root`, `crates/storage/src/snapshot.rs:24-59` —
`H(H(cs) ‖ H(utxo) ‖ H(ps))`, BLAKE3) is recomputed eagerly once per applied block
(`bins/node/src/node/apply_block/state_update.rs:135-146`) by fully re-serializing the entire
3-state. The dominant cost is NOT hashing: it is the full `CF_UTXO` RocksDB scan + N bincode
deserializations + full canonical re-serialization + a fresh multi-MB `Vec` allocation in
`serialize_canonical_utxo` (`crates/storage/src/state_db/queries.rs:473-491`), paid every ~10 s
slot on every node. The original ask ("replace with an Ethereum-style Merkle-Patricia trie") was
identified at prompt-refinement as a solution anchor; the evaluation mandate was SSF-first.

**Premise-altering facts (code-verified, all 5 evaluators + synthesizer):**
- The root is NOT in `BlockHeader` (`crates/core/src/block.rs:19`) and is NEVER consensus-compared.
  It is a snap-sync integrity anchor (quorum-voted, `sync/manager/snap_sync.rs:52-125`; verified at
  install `fork_recovery.rs:281-303`) plus a diagnostic log/RPC. A formula change cannot fork block
  production — only degrade cross-version snap-sync to header-first fallback.
- Cost is a projected hypothesis, not a measured bottleneck: ~single-digit ms of a 10 000 ms slot,
  ~15–21× below the 16 MB `doli_utxo_canonical_size_bytes` threshold (`metrics.rs:243,522`; 12 MB
  warn at `metrics.rs:222,714`).
- Actual blast radius: 6 non-test call-sites (the "~15" was stale comment text).
- The unwired `mmr.rs IncrementalStateRoot` is the WRONG primitive (creation-order → snap-unsafe).

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      -1 O(state) BLAKE3 hash and N bincode deserializations per block per node, steady-state, Tier 0; call-count 1/block at state_update.rs:139 (measured)
  Memory:   -1 multi-MB transient Vec alloc/free per block, alloc site queries.rs:484 (measured)
  IO:       -1 full CF_UTXO RocksDB scan per block per node, scan site queries.rs:478 — the dominant saving (measured)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  -scan-plus-serialize removed from the block-apply critical path; first on-demand read per height pays the same bounded cost (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: status quo — keep the eager per-block compute (zero implementation cost, but retains a guaranteed per-block full-state scan nothing on the hot path consumes)
Why this proposal anyway: net-negative runtime cost with zero new machinery; root value stays byte-identical at every height, so there is no activation height, no migration, and no mixed-fleet risk
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## ⚠ UNIVERSAL FILTER — HIGHEST SEVERITY (applies to every tier, forever)

**NEVER bump `CURRENT_PROTOCOL_VERSION` (currently 8, `crates/network/src/protocols/status.rs:49`)
for a state-root formula change.** The `EPOCH_SNAPSHOT_HF` scaffolding carries a stale planned bump
("3 → 4", `crates/updater/src/hardfork.rs:199`) — an INC-I-054 landmine: any bump triggers
`delete_epoch_state()` on every node restart → non-deterministic rebuild → guaranteed fork at the
next epoch boundary, and the `!=` check re-triggers on rollback (`status.rs:61-62`). The root
formula does NOT change the `EpochState` serialization format (kill test run by the Failure
Analyst: NOT FOUND — `compute_state_root_with_epoch_state` only appends `H(EpochSnapshot)` as a
4th hash input; the struct is untouched). Any formula change (Tier 1) is gated by a NEW
`NetworkParams` activation height ONLY, forward-only, mirroring `amm_activation_height`
discipline. This filter is also what keeps the Tier-1 revert path benign: a buggy formula is
reverted by rolling FORWARD to a corrected formula at a second, higher AH — never by lowering the
first AH, and never via a version bump.

---

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | P1: delete eager per-block compute; root lazy on-demand | conf(0.62, observed) | Eager compute feeds only a cosmetic log + RPC handling that already has fresh-compute fallbacks |
| Restructurer | boundaries | P2/P3: order-independent multiset digest folded at the mutation site, owned by `state_db`/`BlockBatch` | conf(0.6, observed) | Commitment computed at a READ site that rediscovers what `BlockBatch` already knows (`utxo_count` precedent, `batch.rs:485`); `mmr.rs` DISQUALIFIED (creation-order, snap-unsafe) |
| Pattern Matcher | patterns | P5 now; P1 LtHash multiset when scan proven binding | conf(0.7, measured) | Incremental multiset hash (Bitcoin Core muhash family) is THE canonical pattern for this class; XOR/AdHash rejected (algebraic cancellation) |
| Failure Analyst | failures | 6 filters; F-VERSION-BUMP highest severity | conf(0.72, measured) | Formula change cannot fork production (root not on block path); INC-I-081-style cascade DISPROVEN; naive lazy loses the divergence canary (F-D0-2) |
| Radical Simplifier | minimal | P1 (THE SSF): lazy + memoized, same formula | conf(0.7, observed) | Eager compute serves only a diagnostic RPC + a forensic log; neither needs per-block freshness; ~15–30 LOC, 0 new structures, 0 AH |

## Convergence Matrix

Independence verified per cluster (see `docs/.workflow/architecture-reasoning.md` for the full
evidence-independence checks; conclusions were read via the Conclusion-First Protocol before any
full report, so the matrix was formed from independent summaries).

```
                                              Subtr  Restr  Pattern  Failure  Radical   Score
Remove eager per-block root compute (Tier 0):   Y      Y*     (~)      (F)      Y      3/5 direct + 1 permit → DEFINITE
No new authenticated machinery NOW:             Y      (~)     Y       (~)      Y      3 direct + 2 partial → DEFINITE
No PROTOCOL_VERSION bump for formula change:    Y      Y       Y        Y       (~)    4/5 → DEFINITE (filter)
mmr.rs is the WRONG primitive (snap-unsafe):    -      Y       Y        Y       -      3/5 → RECOMMEND retirement
Keep EPOCH_SNAPSHOT_HF parked (no wire/delete): Y      -       N        Y       (~)    2/5 + 1 contra → RECOMMEND (contradiction resolved, see below)
Tier-1 shape = commutative multiset (LtHash):   -      Y       Y       (F)      Y      3/5 + filter agreement → CONDITIONAL option
MPT/SMT now:                                    N      N       N        N       N      5/5 NO-GO now
Collapse duplicate GetStateRoot handlers:       Y      -       -        -       -      1/5 → OPTION (kill test since resolved: one handler is dead code)

Y = proposed/agreed · Y* = variant (periodic instead of lazy) · (~) = partial/implicit
(F) = permitted-with-filter · N = rejected · - = not addressed
```

**Contradictions found: 4 — all resolved** (full log in the reasoning trace):
1. **EPOCH_SNAPSHOT_HF delete-vs-keep** — Pattern Matcher P5 said "delete or land"; Subtractionist
   P2 DISPROVED deletion with concrete evidence (`hardfork.rs:431-478` test-enforces its presence
   on every production network; `PROTOCOL_VERSION 3→4` already spent). Resolved: KEEP PARKED,
   never wire for this redesign, never bump (measured evidence beats an unverified suggestion).
2. **XOR acceptable-vs-reject** — Restructurer called XOR "adequate while non-consensus";
   Pattern Matcher rejected it outright (algebraic cancellation → collisions on a quorum-verified
   anchor). Resolved: REJECT XOR/AdHash. The cost delta vs LtHash is negligible, the anchor is
   quorum-verified today, and REQ-SROOT-008 non-foreclosure forbids baking in a primitive that
   becomes an attack surface if the root ever turns consensus-visible.
3. **Per-block `[STATE_ROOT]` log dispensable-vs-load-bearing** — Subtractionist found zero
   script/doc references ("telemetry regression, not correctness"); Failure Analyst F-D0-2 proved
   per-block fingerprints were what caught the 2026-04-15 scheduler-divergence incidents
   (`snapshot.rs:136-141`), and block hash carries no state → silent divergence is possible.
   Resolved in favor of the adversarial lens at synthesis time: Tier 0 MUST carry a divergence
   canary (below). FINAL RESOLUTION (live-network check): all fork/divergence DETECTION is
   RPC/metric-based, not log-based — the per-block log is a post-detection HUMAN forensic aid;
   epoch-cadence logging is safe. See "Tier-0 Verification" item 5.
4. **Sequencing: "restructure now" vs "defer"** — Restructurer framed the mutation-site digest as
   "the real fix"; Subtractionist/Pattern/Radical defer it. Resolved by the Radical Tiebreaker +
   measured cost (analyst §4: ~15–21× headroom, no flamegraph, no incident cites this cost):
   Tier 0 now; the Restructurer's P2/P3/P4 becomes the fully-specified Tier 1, trigger-gated.

## Tier-0 Verification (Phase 1 repo read-only + Phase 2 live ai5 read-only — ALL gaps CLOSED)

1. **GAP1 — reader census: SAFE.** `cached_state_root` = `Arc<RwLock<Option<(Hash,Hash,u64)>>>`
   (`mod.rs:179`), inits to `None` (`init.rs:1015,1218,1404`). Exactly 3 readers: R1 the
   per-block `[STATE_FP]` log (`apply_block/mod.rs:428`, tolerates `None` → prints "none");
   R2 the `event_loop.rs:509` GetStateRoot handler — **DEAD CODE** (`handle_sync_request_bg`,
   `#[allow(dead_code)]` at `event_loop.rs:392`, confirmed by the comment at
   `validation_checks.rs:960` "handle_sync_request_bg() in event_loop.rs is dead code");
   R3 the `validation_checks.rs:1098` GetStateRoot handler — **LIVE**, compute-on-miss fallback.
   ZERO type-(c) readers (none assume eager freshness). The snap-sync quorum path
   (`snap_sync.rs:19` handle_snap_state_root) consumes PEERS' `StateRoot` responses served via
   R3, never the local cache. `getStateRootDebug` (`crates/rpc/src/methods/stats.rs:66-104`)
   computes independently. Block production never reads the cache.
2. **GAP2 — lock safety: RACE-FREE by construction.** The live R3 handler runs on the SAME
   single event-loop actor with `&mut self` as block apply (dispatch: `event_loop.rs:60`
   `select!` → handle_network_event → `network_events.rs:355` → `validation_checks.rs:962`).
   Apply-write and RPC-memoize-write are therefore mutually exclusive; `cached_state_root` is a
   leaf lock. IMPLEMENTATION REQUIREMENT: the memoize-write must drop the
   chain_state/utxo/producer read guards BEFORE taking the cache write guard (mirror the guard
   ordering of `state_update.rs:135-146`).
3. **GAP3 — single live handler.** Only R3 (`validation_checks.rs:1093-1122`) needs
   cache-on-compute; the `_bg` handler (`event_loop.rs:394-531`) is dead and needs no edit
   (see Option C for its disposition). Doc-drift flag (fix at implementation time):
   `specs/engine-parts.md:2738,2812` claims the reverse liveness — stale, code is SoT.
4. **SCOPE-COMPLETENESS (widens the Tier-0 diff by one file):** deleting the eager compute
   orphans R1 — the per-block `[STATE_FP]` fork-detection log reads the cache immediately after
   today's eager write (write `apply_block/mod.rs:210` → read `mod.rs:428`, `sr=` field in the
   format string at `mod.rs:438`). Under naive lazy it would print a STALE or "none" `sr=` at the
   wrong height — actively misleading during a divergence incident. Tier 0 MUST also update
   `apply_block/mod.rs:427-435` (drop the `sr=` field, re-label it, or feed it the
   memoized-or-`None` value explicitly). Folded into the Definite change and F-D0-2.
5. **OPERATIONAL RATIFICATION (Phase 2, live ai5 read-only) — the final open question CLOSED:
   the per-block divergence LOG is NOT load-bearing for detection.**
   - Fork/divergence DETECTION across the toolchain is RPC/metric-based, not log-based:
     `scripts/fork-monitor.sh:53-56` uses `getChainInfo` (tip-hash grouping); the gauntlet and
     guardian divergence checks use the `getStateRootDebug` RPC
     (`scripts/gauntlet-collect.py:103-105`,
     `.claude/skills/guardian/reference/procedures.md:297,391,510`), which recomputes on demand
     from live state, independent of `cached_state_root` and the per-block path
     (`crates/rpc/src/methods/stats.rs:66-104`) — preserved UNTOUCHED by the SSF.
   - No state-root Prometheus metric exists (`bins/node/src/metrics.rs`, 854 lines — no
     state_root/fingerprint/divergence gauge).
   - Live ai5: no Loki/Promtail (node logs are not shipped → a log-based alert is impossible);
     zero `state_root`/`STATE_FP` references in `/etc/prometheus`, `/etc/alertmanager`,
     `/etc/grafana`, `/var/lib/grafana`; the only consensus-liveness alert is
     `BlockHeightStalled: increase(doli_chain_height[5m])==0`
     (`/etc/prometheus/rules/doli.yml:47`), keyed on the `doli_chain_height` metric.
   - The per-block `[STATE_FP]`/`[STATE_ROOT]` log is a HUMAN forensic aid used AFTER detection
     (code comments `snapshot.rs:39-42`, `apply_block/mod.rs:348-355`); its detection substitute
     (`getStateRootDebug`) is unaffected.
   - `getStateRoot` cadence: no continuous polling loop exists; only the gauntlet calls it once
     per run — so the lazy+memo saving over per-block full-UTXO hashing (every ~10 s) is real.
   → Epoch-cadence logging is SAFE; the ≥0.95 ratification is met.

---

## Tiered Proposal (SSF-first, per Rule 18)

### TIER 0 — SSF: lazy, memoized state-root computation — **VERDICT: GO — conf(0.97, converged); ≥0.95 threshold MET**

Stop computing the root eagerly per block; compute on demand (snap-sync build + `GetStateRoot`),
memoized keyed on `best_hash`, with a divergence canary and an explicit `[STATE_FP]` `sr=` fix.
Root value byte-identical at every height → NO activation height, NO migration, NO mixed-fleet
risk, maximally non-foreclosing (formula and structure untouched — multiset/merkle/trie all stay
open). Detail under Definite Changes.

**Verification chain (how confidence was earned, not asserted):** 0.85 at synthesis (3/5
independent convergence + failure-mode filters) → 0.94 after the repo pass closed all 3
correctness gaps (no type-(c) reader; single-actor leaf-lock race-freedom; single live handler)
→ **0.97 after the live-network check closed the one operational question**: every fork/divergence
DETECTOR in the toolchain is RPC/metric-based (`fork-monitor.sh` via `getChainInfo`, gauntlet +
guardian via `getStateRootDebug` — both untouched by the SSF); no log-shipping/alerting consumes
the per-block line (no Loki/Promtail on ai5, zero refs in Prometheus/Alertmanager/Grafana
configs). The per-block log is a post-detection human forensic aid, so epoch-cadence logging is
safe. The residual ~0.03 is ordinary implementation risk (the widened diff scope + tests), not an
open design question.

### TIER 1 — CONDITIONAL / DEFERRED: commutative multiset digest of the UTXO component — **VERDICT: CONDITIONAL (do NOT build now)** — conf(0.75, converged) on the design shape

LtHash-style (BLAKE3-XOF → fixed integer-lane vector, add/sub mod 2^16), order-independent,
folded at the mutation site (`BlockBatch.add_utxo`/`spend_utxo`, `batch.rs:46,71`), owned by
`state_db` (persisted as ONE META-key value per block inside the same atomic `WriteBatch`,
`batch.rs:481` — the `utxo_count`/`chain_commitment` precedent, `batch.rs:485`, `queries.rs:648`).
cs/ps stay full-recompute (cheap: 140 B fixed / O(P≈14–30)). AH-gated via a NEW `NetworkParams`
field; pre-activation bit-identical to legacy (reuse the `compute_state_root_with_epoch_state`
None/Some gate SHAPE, `snapshot.rs:86` — the shape only, never its version bump). NOT XOR/AdHash
(cancellation), NOT MMR (creation-order), NOT ECMH (non-BLAKE3 hash-to-curve, unaudited).
MUST ship with the F-DRIFT triad (see Constraints). Migration path below.

**Trigger (tied to existing instrumentation):** initiate a Tier-1 design review when
`doli_utxo_canonical_size_bytes` (`metrics.rs:234`) sustains ≥ **4 MB (25% of the 16 MB threshold,
`metrics.rs:243`)** — ~5× the cited mainnet-scale figure — AND a flamegraph of the apply path
confirms the `CF_UTXO` scan (not BLAKE3) is the binding cost. Target landing the change well
before the 12 MB warn (`metrics.rs:222`). DOLI is currently ~15–21× below threshold: **Tier 0 is
the only warranted action now.** Note: Tier 0 removes the per-block scan entirely, so the Tier-1
trigger should also weigh the residual on-demand cost (snap-serve + RPC frequency — measured
today: no continuous poller; gauntlet calls `getStateRoot` once per run), which is the only place
the scan survives.

### TIER 2 — full MPT/SMT (proof-based, per-key) — **VERDICT: NO-GO NOW** — conf(0.9, converged — 5/5 rejection)

Documented ONLY as the non-foreclosure endpoint (REQ-SROOT-008 evolution path: multiset →
merkelized-per-component → full trie). Adopt only if proof-based per-key snap-sync (REQ-SROOT-009,
a COULD) ever becomes a hard requirement, AND write-amp is measured at mainnet set size + ~300 TPS
large-block throughput first (F-WRITE-AMP: O(log N) trie-node writes per changed key is a direct
INC-I-111 replay). Highest write-amplification of any option; its defining benefit (per-key
proofs) has zero consumers today.

---

## Definite Changes (High Convergence)

- **ARCHITECTURAL: Invert the state-root computation model from eager-per-block to lazy
  on-demand with memoization, preserving a divergence canary and fixing the orphaned
  `[STATE_FP]` `sr=` reader (TIER 0).**
    Convergence: Subtractionist P1 (0.62) + Radical Simplifier P1 (0.7) + Restructurer P1
    (0.6, hot-path-removal variant); Pattern Matcher P5 (0.7) converges on the enclosing
    no-new-machinery decision; Failure Analyst permits D0 subject to F-D0-2. Independence
    verified: reader-table grep vs first-principles consumer trace vs mutation/read-site data-flow
    analysis (three distinct evidence bases). All correctness gaps closed by the repo
    verification pass; the operational question closed by the live-network check (see Tier-0
    Verification above).
    Evidence: `state_update.rs:135-146` (sole eager compute); `queries.rs:473-491` (dominant
    cost); `block.rs:19` (no header field, zero consensus comparisons); reader census —
    `apply_block/mod.rs:428` ([STATE_FP], None-tolerant but stale-prone under lazy — see
    component e), `event_loop.rs:509` (DEAD handler, `#[allow(dead_code)]` at `event_loop.rs:392`),
    `validation_checks.rs:1098` (the ONE live handler, compute-on-miss); `snapshot.rs:215` +
    `fork_recovery.rs:281,341` (snap build/install compute fresh, independent of the cache);
    `snap_sync.rs:19` (quorum votes consume peers' `StateRoot` responses served via the live
    handler — never the local cache); `stats.rs:66-104` (`getStateRootDebug` computes
    independently — the detection tool, untouched); single-actor dispatch chain
    `event_loop.rs:60` → `network_events.rs:355` → `validation_checks.rs:962` (race-freedom by
    construction); live ai5 — no log-shipping, no state-root metric/alert
    (`/etc/prometheus/rules/doli.yml:47` is height-stall only).
    Confidence: conf(0.97, converged) — VERDICT: GO; residual ~0.03 is implementation risk only.
    What changes architecturally: the root ceases to be a per-block product of `apply_block` and
    becomes an on-demand derived value of the read path. The eager-compute-for-observation
    pattern is eliminated; the memo (`cached_state_root` tuple keyed on `best_hash`, `mod.rs:179`)
    becomes the single freshness contract. Components: (a) delete the eager compute+publish block
    in `state_update.rs:135-146`; (b) cache-on-compute write-back in the ONE live `GetStateRoot`
    handler (`validation_checks.rs:1093-1122`) — mandatory: quorum-vote collection is served by
    this handler, so during fleet snap-sync bursts an unmemoized lazy node would recompute
    O(state) per incoming vote request (the dead `_bg` handler at `event_loop.rs:394-531` needs
    no edit — disposition in Option C); (c) lock-ordering requirement for the write-back: drop
    the chain_state/utxo/producer read guards BEFORE taking the `cached_state_root` write guard
    (leaf lock, mirroring `state_update.rs:135-146`; mutual exclusion with apply is guaranteed by
    the single event-loop actor); (d) F-D0-2 canary — an epoch-cadence full `[STATE_ROOT]`
    per-component log (fork-detection cadence, INC-I-054 pattern) PLUS a per-block line logging
    the memoized-root-or-`None` without forcing a scan (variant choice in Options, Option B;
    optional cheap hardening: retain a per-block `scheduler_root` one-liner — already computed at
    `apply_block/mod.rs:417`, no UTXO serialization — preserving INC-I-016-class
    scheduler-divergence forensics); (e) update `apply_block/mod.rs:427-435` — the per-block
    `[STATE_FP]` log reads the cache right after today's eager write (write `mod.rs:210` → read
    `mod.rs:428`, `sr=` at `mod.rs:438`); under lazy it would print a stale/none `sr=` at the
    wrong height — drop the field, re-label it, or feed it the memoized-or-`None` value
    explicitly; (f) golden identity test: lazy root == legacy root at every height (formula
    untouched ⇒ REQ-SROOT-001/002/003/004 hold by construction).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -1 O(state) BLAKE3 hash and N deserializations per block steady-state; on-demand computes memoized to at most 1/height (measured)
      Memory:   -1 multi-MB transient Vec per block, queries.rs:484; memo tuple already exists at mod.rs:179 (measured)
      IO:       -1 full CF_UTXO scan per block, queries.rs:478; the scan survives only on actual snap-serve or first RPC read per height (measured)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  -scan-plus-serialize off the block-apply critical path; plus the same bounded cost on the first on-demand read per height; worst case equals today, never worse (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: status quo (keep eager compute) — zero implementation cost, retains the guaranteed per-block scan
    Why this proposal anyway: removes 100% of the per-block cost REQ-SROOT-007 targets with 0 new modules, 0 persisted structures, 0 consensus surface, and byte-identical root value at all heights
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Bind any future root-formula change to a NEW `NetworkParams` activation
  height ONLY; `CURRENT_PROTOCOL_VERSION` frozen at 8 for this domain (the Universal Filter,
  adopted as a standing architectural decision).**
    Convergence: Failure Analyst F-VERSION-BUMP (0.72, measured) + Subtractionist constraint 6 +
    Restructurer constraint 4 + Pattern Matcher constraint 4 — 4/5, independent evidence
    (landmine trace at `hardfork.rs:199` vs EpochState-format non-change vs REQ-SROOT-005 reading).
    Evidence: `hardfork.rs:199` (stale planned bump); `status.rs:49,61-62` (current value 8;
    INC-I-054 `!=` re-trigger); kill test "format genuinely changes" NOT FOUND
    (`snapshot.rs:86-130` touches no `EpochState` field; `EPOCH_STATE_FORMAT_VERSION` independent,
    `init.rs:774`).
    Confidence: conf(0.85, converged)
    What changes architecturally: removes a fork-cascade class by construction — decouples the
    root-formula evolution surface from the peer-handshake version, making the Tier-1 revert path
    (roll FORWARD to a corrected formula at a second higher AH) structurally benign.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (measured)
      Memory:   0 (measured)
      IO:       0 (measured)
      Network:  0 (measured)
      Disk:     0 (measured)
      Latency:  0 (measured)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: a zero-cost standing rule that converts the single highest-severity failure mode in the design space (INC-I-054 replay) into a structurally unreachable state
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- **ARCHITECTURAL: Retire `mmr.rs IncrementalStateRoot` as a state-root candidate (tombstone
  with a disqualification comment, or delete — disposition is Option D).**
    Convergence: Restructurer kill test (CONFIRMED FATAL) + Failure Analyst F-BOOTSTRAP (0.7) +
    Pattern Matcher landscape table (MMR "poor") — 3/5, independent evidence (code trace of
    `mmr.append()` vs first-principles history-dependence argument vs pattern taxonomy).
    Evidence: `crates/storage/src/mmr.rs:35,110-167,131` — commits every UTXO ever created in
    CREATION ORDER + XOR spent-set; a snap-synced node holds only the live set
    (`snapshot.rs:209`, `utxo/set.rs:432`) and cannot reconstruct it → violates REQ-SROOT-006
    outright. Fully implemented + tested, zero non-test callers (grep-verified by two evaluators).
    Confidence: conf(0.75, converged)
    What changes architecturally: deletes (or explicitly disqualifies) an entire unwired
    abstraction whose presence invites the "just wire mmr.rs" mistake — the naive path is DEAD
    and the codebase should say so at the source.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (measured)
      Memory:   0 (measured)
      IO:       0 (measured)
      Network:  0 (measured)
      Disk:     0 (measured)
      Latency:  0 (measured)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave it unwired with only a warning comment (no deletion)
    Why this proposal anyway: a proven-wrong primitive left looking ready-to-wire is a latent REQ-SROOT-006 violation waiting for a future implementer; the disqualification must be recorded at the code, not only in this spec
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Keep `EPOCH_SNAPSHOT_HF` / `compute_state_root_with_epoch_state` PARKED —
  do not wire it for this redesign, do not delete it, and never execute its planned version bump.**
    Convergence: Subtractionist P2 (0.65, deletion DISPROVEN) + Failure Analyst landmine finding —
    2/5, independent evidence (test-enforcement trace vs version-bump trace). Pattern Matcher's
    contrary "delete or land" suggestion resolved against (Contradiction 1).
    Evidence: `hardfork.rs:431-478` (live test asserts the entry exists on every production
    network — deletion breaks the build gate); `hardfork.rs:199` (already-spent 3→4 bump);
    `snapshot.rs:79-85,86-130` (scheduler-state folding for INC-I-034 — a value-CHANGING concern
    orthogonal to this value-PRESERVING cost fix). Tier 1 may reuse its None/Some gate SHAPE only.
    Confidence: conf(0.75, converged)
    What changes architecturally: pins the boundary between two distinct evolution surfaces —
    the scheduler-divergence HF (parked, version-coupled, orthogonal) and the cost redesign
    (value-preserving, AH-only) — preventing a future implementer from conflating them.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (measured)
      Memory:   0 (measured)
      IO:       0 (measured)
      Network:  0 (measured)
      Disk:     0 (measured)
      Latency:  0 (measured)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: prevents both a wasteful deletion (breaks tests, strands a paid protocol bump) and a catastrophic mis-wiring (inheriting the INC-I-054 landmine)
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

- **OPTION A — Tier 1 multiset digest (DEFERRED; approve the trigger, not the build).**
  Source: Restructurer P2/P3/P4 + Pattern Matcher P1 (LtHash) + Radical P2. conf(0.75, converged)
  on the shape; the SEQUENCING verdict is defer. What is being decided now: accept the stated
  trigger (≥4 MB sustained on `doli_utxo_canonical_size_bytes` + flamegraph confirming the
  `CF_UTXO` scan binding) and the recorded design shape, so the future migration is calm, not
  rushed. Complexity cost when built: +1 module (~150–300 LOC), +1 persisted META value, +1 AH,
  +the F-DRIFT triad. Failure-mode filter: passes F-BOOTSTRAP (order-independent, seedable by one
  scan) and F-WRITE-AMP (one 32 B–2 KB write/block, not per-mutation); vulnerable to F-DRIFT
  unless the triad ships (mandated in Constraints).
- **OPTION B — Canary form for Tier 0** (pick one; default = B1+B2 combined):
  B1: epoch-cadence full `[STATE_ROOT]` per-component log (Radical's mitigation — fork-detection
  cadence, preserves the component-divergence grep at ~1/epoch cost). B2: per-block line logging
  the memoized root or `None` (Subtractionist's cheap alternative — per-block breadcrumb, zero
  scans; this also serves as the `[STATE_FP]` `sr=` replacement, component e). B3 (upgrade):
  cheap per-block per-component digests without the utxo re-serialization (Failure Analyst's
  preference — strongest signal, small extra code). B4 (OPTIONAL hardening, cheap): retain a
  per-block `scheduler_root` one-liner — it is already computed at `apply_block/mod.rs:417` with
  NO UTXO serialization — preserving INC-I-016-class scheduler-divergence forensics at per-block
  cadence while the expensive 3-state root goes lazy. Recommendation: B1+B2 (+B4 if desired) now.
  The operational safety question is CLOSED: all detectors are RPC/metric-based
  (`fork-monitor.sh:53-56` getChainInfo; gauntlet/guardian `getStateRootDebug`); no log-based
  alerting exists (no Loki/Promtail; zero refs in Prometheus/Alertmanager/Grafana on ai5) — the
  canary choice is a forensics-richness preference, not a detection-safety decision.
- **OPTION C — Dispose of the DEAD duplicate `GetStateRoot` handler.** Verification resolved the
  Subtractionist's open kill test: `handle_sync_request_bg` (`event_loop.rs:394-531`, containing
  the second handler at `:507-531`) is `#[allow(dead_code)]` (`event_loop.rs:392`) and explicitly
  documented dead at `validation_checks.rs:960`; only `validation_checks.rs:1093-1122` is live.
  conf(0.65, observed) — the annotation + doc comment suggest DELIBERATE retention (it was the
  background-I/O isolation variant), so deletion vs keep-with-comment is a user cleanup choice;
  either way, Tier 0 edits ONLY the live handler.
- **OPTION D — `mmr.rs` disposition**: tombstone-with-comment vs full deletion (retirement itself
  is Recommended above). Deletion is the stronger subtraction (~203 LOC + tests) but is a
  1-evaluator preference — **low-evidence** on disposition, converged on disqualification.
- **HOUSEKEEPING (non-architectural, bundle with Tier 0):** fix the stale "15 call-sites" comment
  (`snapshot.rs:81`, `hardfork.rs:203` — actual: 6, analyst §1d); fix the `specs/engine-parts.md:2738,2812`
  doc-drift at implementation time (it claims the reverse handler liveness — code is SoT); record
  in the code comment at `state_update.rs` (post-edit) that snap build/install compute fresh and
  must never route through the memo.

## Constraints (from the Failure Analyst — filters any chosen path MUST satisfy)

1. **F-VERSION-BUMP (conf 0.72, HIGHEST SEVERITY):** no `CURRENT_PROTOCOL_VERSION` bump for the
   root formula — see Universal Filter. Adopted as a Definite change.
2. **F-BOOTSTRAP (conf 0.7):** any incremental scheme MUST be a pure, order-independent
   (set-commutative) function of the current on-disk 3-state (REQ-SROOT-006). Chained/delta-log
   running hashes are CONFIRMED FATAL (f(sequence) is unrecoverable from the final set). Kills
   MMR and any append-order accumulator. The "running-hash accumulator" wording in older docs is
   a trap.
3. **F-DRIFT triad (conf 0.68) — mandatory for Tier 1:** (a) exact undo mirror in `rollback.rs`
   (fold-out must invert fold-in; mirrors the `utxo_count` ± symmetry, `batch.rs:485-493`);
   (b) epoch-boundary deferred-producer mutation mirror (Register/AddBond/Exit/Slash/Withdrawal/
   Delegation are DEFERRED per CLAUDE.md); (c) periodic full-recompute audit with a defined
   halt/rebuild action on mismatch (a drifted node otherwise becomes a silent non-server; if the
   drift bug is systematic, the fleet stops serving valid snapshots → new-joiner starvation).
   A Tier-1 implementation missing any leg is incomplete and MUST be rejected.
4. **F-WRITE-AMP (conf 0.66):** persist ONE accumulator value per block, never one write per
   mutation; no MPT unless REQ-SROOT-009 becomes a hard requirement AND write-amp is measured at
   mainnet size + large-block throughput (INC-I-111).
5. **F-D0-2 (conf 0.6 → RESOLVED by live-network check):** any cost reduction MUST preserve a
   divergence-forensics signal and must not leave a MISLEADING one. Detection was proven
   RPC/metric-based (Tier-0 Verification item 5), so the per-block LOG cadence is a forensics
   preference, not a detection requirement — Tier 0 satisfies this via Option B (B1+B2 default;
   B4 optional per-block `scheduler_root` one-liner from `apply_block/mod.rs:417` preserves
   INC-I-016-class scheduler forensics at zero UTXO-serialization cost). The orphaned
   `[STATE_FP]` `sr=` field (`apply_block/mod.rs:428,438`) must still be fixed (component e) —
   a stale root printed at the wrong height is worse than silence.
6. **F-D-mixed (conf 0.68):** a Tier-1 formula split is benign degradation (snap-reject →
   3 retries → header-first sync → convergence; INC-I-081-style cascade DISPROVEN), EXCEPT the
   starvation caveat: fresh old-binary nodes joining a mostly snap-synced-and-pruned fleet may
   find no deep block history (INC-I-012 class). Mitigate with forward-only AH + operator upgrade
   lead time (~30 external producers — no synchronized stop exists).
7. **Determinism:** integer-only, BLAKE3-only (reject ECMH/hash-to-curve unless separately
   audited); x86/ARM golden vectors; preserve RocksDB-key-order == in-memory-sort parity for the
   WIRE format (`queries.rs:466` vs `in_memory.rs:361-362`) — note a commutative digest REMOVES
   this coupling from the root VALUE (a structural bonus), but the sorted wire format still
   requires it for snapshot transfer.
8. **Boundary:** `crates/network` stays storage-free — only a `Hash` crosses; verification stays
   node-side (`fork_recovery.rs`). Tier-1 digest lives in `state_db`/`BlockBatch` (atomicity with
   the WriteBatch forces placement).

## Architecture Maps

### Current Architecture
```
apply_block (EVERY block, state_update.rs:139)
  └─► compute_state_root (snapshot.rs:24)
        └─► serialize_canonical_utxo (queries.rs:473): full CF_UTXO scan + N deser + re-ser + multi-MB alloc
  └─► cached_state_root (write, state_update.rs:144) + per-block [STATE_ROOT] log (snapshot.rs:43)

Readers (3): R1 [STATE_FP] log (mod.rs:428, sr= at :438 — reads right after the eager write) ·
  R2 GetStateRoot handler in event_loop.rs:509 — DEAD CODE (#[allow(dead_code)] :392) ·
  R3 GetStateRoot handler in validation_checks.rs:1098 — LIVE (cache-first, fresh-compute
  fallback that discards its result) — R3 also serves snap-sync quorum votes (snap_sync.rs:19)
Independent fresh computes: snap BUILD (snapshot.rs:215) · snap INSTALL verify+recache
  (fork_recovery.rs:281,341) · CLI verify (cmd_snap.rs:226) · getStateRootDebug (stats.rs:66-104)
Detection toolchain (all RPC/metric-based, none read the log): fork-monitor.sh:53-56
  (getChainInfo) · gauntlet-collect.py:103-105 + guardian procedures (getStateRootDebug) ·
  Prometheus BlockHeightStalled (doli.yml:47, doli_chain_height)
Block acceptance: NEVER touches the root (no header field, block.rs:19)
```

### Proposed Architecture (Definite + Recommended — Tier 0)
```
apply_block: ZERO root work; [STATE_FP] sr= field fixed (dropped, re-labeled, or explicit
  memoized-or-None) so it can never print a stale root at the wrong height; optional per-block
  scheduler_root one-liner retained (mod.rs:417, no UTXO serialization — Option B4).
GetStateRoot — ONE live handler (validation_checks.rs:1093-1122; serves RPC + quorum votes):
  memo hit (best_hash match) → O(1)
  memo miss → compute fresh (existing fallback) → WRITE BACK to memo (new; drop state read
  guards before taking the cache write guard — leaf lock, single-actor mutual exclusion)
Dead _bg handler (event_loop.rs:394-531): untouched (disposition = Option C)
snap BUILD / INSTALL / CLI: unchanged (compute fresh; install still re-caches, fork_recovery.rs:341)
Detection toolchain: UNTOUCHED (getChainInfo / getStateRootDebug / doli_chain_height)
Canary: epoch-cadence full [STATE_ROOT] component log + per-block memo-or-None line (Option B)
mmr.rs IncrementalStateRoot: retired (tombstoned or deleted)
EPOCH_SNAPSHOT_HF: parked, explicitly not the vehicle for any of this
Formula, root value, wire format: BYTE-IDENTICAL at every height
```

## Migration Path

**Tier 0 — no migration required.** Root value is unchanged at all heights: no AH, no golden-epoch
replay, no fleet coordination; rolling deploy safe (a mixed fleet computes identical roots).
Implementation order (for the future implementer — NOT part of this proposal):
1. Add cache-on-compute write-back to the ONE live `GetStateRoot` handler
   (`validation_checks.rs:1093-1122`), with the lock-ordering rule: release the
   chain_state/utxo/producer read guards BEFORE taking the `cached_state_root` write guard
   (behavior-neutral, ships first).
2. Add the Option-B canary (epoch-cadence log + per-block memo line; optional B4
   `scheduler_root` one-liner).
3. Fix the `[STATE_FP]` `sr=` field (`apply_block/mod.rs:427-435`) so it reports
   memoized-or-`None` honestly (or is dropped/re-labeled) — must land in the SAME change as
   step 4, never after it.
4. Delete the eager block at `state_update.rs:135-146`.
5. Tests: golden identity (lazy == legacy per height), memo keyed-on-`best_hash` staleness test,
   quorum-vote serve path test (vote request on cold memo computes fresh and memoizes),
   `[STATE_FP]` stale-`sr=` regression test (must never print a root from a previous height as
   if current).
6. Housekeeping: stale "15 call-sites" comments; `specs/engine-parts.md:2738,2812` handler-liveness
   doc drift (fix at implementation time).
No BRIDGE entries are required for Tier 0 — there is no transitional state to maintain.

**Tier 1 — when (and only when) the trigger fires:**
1. Flamegraph gate: confirm `CF_UTXO` scan dominance (analyst Assumption 2) — the entire
   escalation hinges on this measurement.
2. Crypto review pins the primitive (LtHash recommended; MuHash3072 is the proven-pedigree
   fallback; XOR/ECMH rejected per Constraints).
3. New `NetworkParams` AH field (devnet=0, testnet next, mainnet with operator lead time —
   `amm_activation_height` discipline). NO version bump (Universal Filter).
4. Pre-activation: digest code ships dormant; root remains legacy-identical (REQ-SROOT-001
   golden-vector across ≥3 live epochs; None/Some gate shape from `snapshot.rs:86`).
5. At AH: seed the accumulator by ONE full scan of the on-disk 3-state (REQ-SROOT-006 — snap-safe
   by order-independence; scan-once == fold-incrementally by commutativity, which is also what
   collapses the three root-derivation paths onto one primitive, Restructurer P4).
6. F-DRIFT triad ships in the same change (undo mirror, epoch-boundary mirror, periodic audit
   with halt/rebuild on mismatch) — non-negotiable.
7. BRIDGE: dual-formula posture during the mixed-fleet window — old-binary nodes fall back to
   header-first sync by design (F-D-mixed); monitor external-producer upgrade progress before
   pinning the mainnet AH. Transitional only; the posture ends once the fleet crosses the AH.
8. Revert path: roll FORWARD to a corrected formula at a second, higher AH — never lower the
   first, never bump the version.

## Complexity Comparison

| Metric | Current | Radical Minimum (Tier 0) | Proposed (this spec) | Tier 1 (deferred) | Tier 2 MPT (NO-GO) |
|--------|---------|--------------------------|----------------------|-------------------|---------------------|
| New modules | 0 | 0 | **0** | 1 | 3–5 |
| New persisted structures | 0 | 0 | **0** | 1 (META value) | 1 large CF + proof cache |
| New consensus surface (AH) | 0 | 0 | **0** | 1 | 1 (+header field if consensus) |
| LOC (est.) | — | ~15–30 | **~25–50 (incl. canary + [STATE_FP] fix)** | ~150–300 | ~2,000–5,000 |
| Per-block ops | 1 full-CF scan + N deser + re-ser + multi-MB alloc + 3× BLAKE3 | 0 | **0** | O(changed) folds + 1 small persist | O(changed·log N) node writes |
| Reorg-reversal invariant | none (self-healing) | none | **none** | digest must invert (F-DRIFT triad) | trie must invert (harder) |
| Migration (REQ-SROOT-006) | n/a | none | **none** | 1 seed scan at AH | full trie build at AH |
| Mixed-fleet snap risk | n/a | none | **none** | window until fleet upgrades | window until fleet upgrades |

The proposed architecture IS the radical minimum plus the mandatory canary and the `[STATE_FP]`
scope-completeness fix — the radical floor gap is zero.

## Acceptance Criteria Mapping (REQ-SROOT-001..010 → tiers)

| REQ | Priority | Tier 0 | Tier 1 | Tier 2 |
|-----|----------|--------|--------|--------|
| 001 identical root pre-activation | Must | **By construction** (value unchanged at ALL heights) | Golden-vector ≥3 epochs, None/Some gate | same as T1 |
| 002 bit-identical convergence (INV-SYNC-007) | Must | **By construction** | commutativity + golden vectors | order-canonical leaves |
| 003 x86/ARM determinism | Must | **Inherited** (BLAKE3 unchanged) | integer-only LtHash + cross-arch CI vector | tree vectors |
| 004 snap build+install intact | Must | **Untouched** (both compute fresh) | one shared digest primitive, install-seed scan | proof path new |
| 005 forward-only AH, no version bump | Must | **N/A (no AH needed)** | NEW NetworkParams AH; version frozen at 8 | same |
| 006 initial root from on-disk 3-state | Must | **Trivially** (no formula change) | seed-by-one-scan; order-independence guarantees equality | full trie build from sorted state |
| 007 update ∝ changed entries | Should | **Dissolved** (no per-block computation exists) | O(changed) folds | O(changed·log N) |
| 008 non-foreclosure | Should | **Maximal** (structure untouched; all paths open) | scalar digest does not foreclose merkelization | is the endpoint |
| 009 proof-based snap-sync | Could | not addressed (correctly — no consumer) | not addressed | the only tier that delivers it |
| 010 no genesis reset / no sync-stop | Won't | **Satisfied** (rolling-safe) | satisfied (AH + lead time) | satisfied (AH + lead time) |

## Milestones

Tier 0 is small (≤4 files: `state_update.rs`, `validation_checks.rs`, `apply_block/mod.rs`,
`snapshot.rs` canary) — two milestones suffice:
- **M1 — Memo + canary (behavior-additive):** write-back in the live handler (with the
  lock-ordering rule); epoch-cadence + per-block-memo canary (optional B4 scheduler_root
  one-liner); tests (golden identity, memo staleness, vote-serve). No behavior removed.
- **M2 — Eager-compute removal (the subtraction):** delete `state_update.rs:135-146` + fix the
  `[STATE_FP]` `sr=` field in the SAME change; comment fixes (stale "15 call-sites",
  `engine-parts.md` handler liveness); `mmr.rs` retirement + `EPOCH_SNAPSHOT_HF` "parked — not
  the cost-redesign vehicle" comment per Recommended changes; full test gate incl. the
  stale-`sr=` regression test.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     2 (eager per-block compute 3/5+1; mmr.rs disqualification 3/5)
Restructuring convergence:      2 (lazy inversion 3/5 → Definite; Tier-1 mutation-site digest 3/5 → deferred Option)
Addition options presented:     4 (A: Tier-1 digest · B: canary form incl. B4 scheduler_root · C: dead-handler disposition · D: mmr disposition)
Failure modes identified:       6 (F-VERSION-BUMP, F-D-mixed, F-BOOTSTRAP, F-DRIFT, F-WRITE-AMP, F-D0-2) + 4 determinism traps
Failure modes applied as filters: 6/6 (every proposal filtered; log in reasoning trace)
Radical floor gap:              eager O(state)/block → Tier 0 (0 new structures) → proposed == radical minimum + canary + [STATE_FP] fix (gap: 0)
Contradictions found:           4
Contradictions resolved:        4/4 (EPOCH_SNAPSHOT_HF · XOR · canary [closed by live check] · sequencing)
Evidence independence verified: YES (per-cluster, Conclusion-First Protocol; repo pass closed
                                3/3 correctness gaps + 1 scope fix → 0.94; live ai5 pass closed
                                the operational question → Tier 0 = conf(0.97) VERDICT: GO)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Open Questions

1. ~~Is epoch-cadence logging an acceptable replacement for the per-block fingerprint?~~
   **RESOLVED/CLOSED (2026-07-18, repo Phase 1 + live ai5 Phase 2):** all fork/divergence
   DETECTION is RPC/metric-based — `fork-monitor.sh:53-56` (getChainInfo tip grouping),
   gauntlet + guardian via `getStateRootDebug` (`gauntlet-collect.py:103-105`, guardian
   procedures 297/391/510; recomputes independently, `stats.rs:66-104`, untouched by the SSF);
   no state-root Prometheus metric exists (`metrics.rs`, 854 lines); live ai5 has no
   Loki/Promtail (logs not shipped → log alerts impossible) and zero `state_root`/`STATE_FP`
   references in Prometheus/Alertmanager/Grafana; the only consensus-liveness alert is
   `BlockHeightStalled` (`doli.yml:47`). The per-block log is a post-detection human forensic
   aid (`snapshot.rs:39-42`, `apply_block/mod.rs:348-355`); `getStateRoot` has no continuous
   poller (gauntlet: once per run). → Epoch-cadence logging is SAFE; Tier 0 = 0.97, VERDICT: GO.
2. Flamegraph of the apply path at mainnet scale (required by the Tier-1 trigger; analyst
   Assumption 2 — the whole escalation hinges on it). Does not block Tier 0.
3. Roadmap intent: will the 3-state root ever become a header-validated consensus field? Code
   says no; if intent says yes, Tier 0 remains valid but Tier 1 becomes mandatory and its
   primitive-strength bar rises (Constraint on XOR already anticipates this). Does not block
   Tier 0.
