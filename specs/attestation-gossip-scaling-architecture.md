# Attestation Gossip-Volume Scaling Architecture

> **PROPOSAL + MEASUREMENT** (2026-07-18; Measurement Addendum 2026-07-19) — 5-evaluator
> convergence synthesis. Incident INC-I-141.
> **VERDICT (unchanged): CONDITIONAL NO-GO on structural work — DEFER behind a measured tripwire,
> conf(0.95, measured). Producer-count tripwire relaxable from N ≥ 2000 toward N ≥ 5000 on
> measured CPU headroom (see Measurement Addendum). STATUS: Tier 0a (dead validate path, spec
> T0-C/M1) APPLIED to main 2026-07-19 (uncommitted, conf 0.97); Tier 0b (T0-A) and Tier 0c
> (T0-B) proven-safe, DEFERRED in worktree reference.**

## Problem Statement

The redesign began as "per-block attestation *verification* is O(N), ~45% of the block CPU
budget" — **falsified in Step 0** (2 independent agents): the production block-validate path
(`validate_block_with_mode`, `validation_checks.rs:418` ← `apply_block/mod.rs:110`) performs
**zero** attestation signature verification; the BLS aggregate verify path is unreachable dead
code. The corrected question: **does the N-attestation-messages-per-slot gossip flood wall the
Law #3 target of "1000s of producers in 10s slots", and what is the best structural response?**

Verified mechanism (synthesizer-ground-truthed 2026-07-18, correcting the brief):
each active producer publishes ONE `Attestation` per 10s slot via
`NetworkCommand::BroadcastAttestation` → gossipsub topic **`ATTESTATION_TOPIC` =
`/doli/attestations/1`** (`crates/network/src/gossip/mod.rs:44`,
`service/command_handling.rs:220-221`), received at `behaviour_events.rs:411` →
`NetworkEvent::NewAttestation` → `on_new_attestation` (`bins/node/src/node/network_events.rs:536`,
1 Ed25519 verify per unique message). **NOT `VOTES_TOPIC`** — `/doli/votes/1`
(`gossip/mod.rs:30`) carries the governance auto-update veto (`behaviour_events.rs:385` →
`NewVote` → `on_new_vote`, `event_loop.rs:356`). The brief and the analyst doc §1.2 are wrong on
this point; three evaluators independently caught it and the synthesizer confirmed by grep.

The flood is amplified by **`flood_publish(true)`** (`gossip/config.rs:224`, comment: *"At 42
nodes the bandwidth cost is negligible"*; second site `:355`) — the origin sends every published
message to ALL connected peers (up to max_peers=50), not just the mesh subset.
**MEASURED 2026-07-19: `flood_publish` affects ONLY the publisher's own egress; relaying is
mesh-bounded (adaptive D, cap `MESH_N_CAP=50`, `config.rs:154-160,30`) plus a 60s BLAKE3
duplicate cache (`config.rs:185,220`), so unique ingress is N×wire — NOT N×D×wire. The feared
"flood_publish → O(N²)" amplification is defused (see Measurement Addendum).** A parallel
point-to-point channel exists: `SyncRequest::DirectAttestation` unicast to the slot+1 producer
(`startup.rs:647`, receive `network_events.rs:326-343`, version-gated `protocols/status.rs:38`).
The on-chain attestation bitfield (committed via `presence_root`, `validation_checks.rs:395-402`)
is the third, authoritative dissemination channel — all consensus consumers (liveness
`post_commit.rs:60-66`, rewards `rewards.rs:139-145`, RPC `schedule.rs:305-311`) read the block,
not the gossip.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      -1 BLS sign/attestation/slot per producer and -O(N) G1 aggregation per produced block; 0 on all other paths (observed)
  Memory:   -104B per tracked attestation; 0 otherwise (observed)
  IO:       0 (observed)
  Network:  -96B per gossiped attestation (276B to 180B bincode wire, -34.8%, MEASURED 2026-07-19) x N producers x mesh-bounded fanout per slot; -104B per propagated block (measured)
  Disk:     -104B per stored block body once T0-B ships (observed)
  Latency:  -negligible on gossip and block assembly (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: pure status-quo (leave dead BLS pipeline in place) — cheaper in effort, permanently more expensive in bytes/CPU on every mesh edge
Why this proposal anyway: the retired pipeline is verified write-only (zero live readers); the subtraction shrinks the exact O(N·fanout) term this redesign targets at zero new abstraction cost, while ALL structural (cost-adding) work is deferred behind the tripwire
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Retire the write-only BLS pipeline (gossip field + block blob + dead validate path) | conf(0.7, observed) | Complete write-only BLS subsystem, D1-D8 inventory; wire shrink per attestation, rolling-safe |
| Restructurer | boundaries | Producer is ALREADY the aggregator; block is already the aggregate; flood partly redundant with unicast + bitfield | conf(0.62-0.68, observed) | Corrected the topic (ATTESTATION_TOPIC); found DirectAttestation unicast; O(N²·D) network math (defused by measurement — see Addendum); rejected relay aggregation |
| Pattern Matcher | patterns | Producer-as-aggregator directed delivery (Ethereum-subnet analog via DOLI's own 0xAA/unicast primitives) — deferred | conf(0.68, observed) | Committee sampling 2/10 fit (breaks per-minute reward attribution); Narwhal 1/10; flood_publish is global, not per-topic |
| Failure Analyst | failures | F1-F9 hard filters; MAX_EXCLUSIONS_PER_BLOCK does NOT exist in code — only MIN_PRODUCERS_FLOOR=3 | conf(0.65-0.7, measured) | Delivery reliability feeds exclusion with only a floor-3 backstop; INC-I-114 flood_publish OOM is new negative evidence |
| Radical Simplifier | minimal | **DEFER — status-quo is the minimum viable architecture to N≈3000; install tripwire** | conf(0.7, measured) | Byte math (2026-07-18 modeled): ~1% slot CPU at N=1000 — CONFIRMED and improved by measurement: 0.42% at N=1000, comfort to ~N=10000 |

*Cost figures inside this table are the evaluators' original 2026-07-18 estimates, preserved as
an evaluation record; the authoritative numbers are the MEASURED constants in the cost model
below and the Measurement Addendum.*

## Convergence Matrix

Deletions/decisions × evaluators (Sub=Subtractionist, Res=Restructurer, Pat=Patterns, Fail=Failure, Rad=Radical):

```
                                            Sub  Res  Pat  Fail Rad   Score
DEFER structural aggregation now:            ~    Y    Y    Y    Y    4.5/5 → DEFINITE
Retire per-attestation BLS gossip field:     Y    -    Y*   -    Y    3/5   → DEFINITE (after synth verification)
Delete dead validate path (D1/D2/D3):        Y    -    -    -    ~    1.5/5 + analyst + audit CORE-C1 → DEFINITE (independent sources)
Empty block.aggregate_bls_signature:         Y    -    Y*   -    Y    3/5   → RECOMMENDED (INC-I-062 sign-off pending)
flood_publish is the real amplifier:         -    -    Y    Y    Y    3/5   → REVISED by measurement: publisher-only egress; see Addendum
REJECT committee sampling:                   -    -    Y    Y    -    2/5, zero dissent → REJECTED
REJECT relay/infra-node aggregation:         -    Y    -    Y    -    2/5, zero dissent → REJECTED
Endgame = producer-as-aggregator/subnets:    ~    Y    Y    ~    Y    4/5   → DEFERRED Tier 2 shape
RegionAggregate: retain vs delete:           defer RETAIN DELETE -  RETAIN → CONTRADICTION resolved: RETAIN (annotated)
Drop attester_weight from wire:              Y    -    -    ~    -    1/5   → OPTION (security signal)
```
`Y*` = Patterns P4/P5 propose it conditionally (sequenced). `~` = implied/partial support.

**Convergence independence check (per protocol):**
- *DEFER*: Radical (byte math, measured), Failure (incident-derived risk ranking), Patterns
  (pattern-fit costing), Restructurer (kill-test caps on own proposals) — four INDEPENDENT
  evidence bases → true convergence, confidence elevated to 0.85; **elevated again to 0.95 by
  the 2026-07-19 benchmark + live-testnet measurement pass** (the modeled curve was re-derived
  from measured constants and came out MORE comfortable, not less).
- *BLS gossip field retirement*: Subtractionist (consumer-chain grep) and Radical (wire-size
  math) used independent methods; both flagged the same open question ("does anything live read
  the field?"), which the **synthesizer discharged by direct verification** (see Resolved
  Contradiction B below): every consumer terminates in the dead aggregate. Elevation to 0.85
  is verification-backed, per the graph/blast elevation rule; **elevated to 0.92 by executable
  proof (2026-07-19 worktree build + test pass).**
- *Dead validate path*: Subtractionist grep + analyst §3 + prior audit CORE-C1
  (`docs/audits/audit-2026-02-24.md:37`) + synthesizer grep (zero live callers) — four sources;
  **elevated to 0.97 by executable proof: deletion APPLIED, full workspace build exit 0,
  doli-core 972 + doli-node 52 tests green (2026-07-19).**

## Resolved Contradictions (synthesizer ground-truth, 2026-07-18)

- **A — Topic:** live attestation topic is `ATTESTATION_TOPIC` `/doli/attestations/1`
  (`gossip/mod.rs:44`; publish `command_handling.rs:220-221`; receive `behaviour_events.rs:411`
  → `event_loop.rs:378` → `on_new_attestation`). `VOTES_TOPIC` is governance veto
  (`behaviour_events.rs:385` → `on_new_vote`). Brief + analyst §1.2 WRONG; Restructurer/
  Patterns/Radical RIGHT. Any future patch targeting `publish_vote` would edit the wrong channel.
- **B — BLS field safety:** gossiped `Attestation.bls_signature` (`attestation.rs:31-33`,
  `#[serde(default)]`) has exactly four non-test readers (`network_events.rs:334,340,556,562`),
  all feeding `MinuteAttestationTracker::record_with_bls` → `bls_sigs` map, whose SOLE consumer
  is `bls_sigs_for_minute` (`assembly.rs:618`) → `aggregate_bls_signatures` →
  `block.aggregate_bls_signature`. That blob's only verify readers are
  `validation/registration.rs:267` and `validation/block.rs:174` — both inside bare
  `validate_block()`, which has **zero live call sites** (synthesizer grep). Remaining readers
  are storage round-trip (`block_store/types.rs`, `writes.rs:36`), legacy format compat
  (`transaction/legacy.rs`), and display-only RPC (`rpc/types/block.rs:82` emits `None` when
  empty; `schedule.rs:320`). `Block::hash()` = `header.hash()` (`block.rs:188`); the header
  hash (`block.rs:76-84`) covers version/prev_hash/merkle_root/presence_root/genesis_hash/
  missed_producers — **NOT** the body blob. Both receive sites already branch on
  `is_empty()`. → **Dropping the gossiped field is rolling-safe; the block blob is uncommitted.**
  (Dead-path readers deleted 2026-07-19 with Tier 0a — see Migration Path M1 status.)
- **C — Exclusion caps:** `MAX_EXCLUSIONS_PER_BLOCK` and `max_excluded_total` — **0 hits in
  the tree** (synthesizer grep, all `*.rs`). The live backstop is the epoch-boundary attestation
  filter `compute_live_producer_list` (`epoch_state/mod.rs:44-111`) with
  `MIN_PRODUCERS_FLOOR = 3` (`consensus/constants.rs:155`) post-activation (2/3-of-effective
  pre-activation) — this caps the **floor**, not the per-epoch exclusion **delta**. The Failure
  Analyst is CORRECT; CLAUDE.md/MEMORY.md and the brief carry doc drift from the INC-I-016-era
  fix. **Consequence: any delivery-reliability change is MORE dangerous than the brief assumed**
  — a correlated delivery failure can prune the scheduler set toward 3 in one boundary. This
  hardens filter F4 and is a primary reason Tier 1/Tier 2 are deferred. ⚠ DOC-DRIFT: register
  a MEMORY.md hotfix for the stale cap references.
- **D — Delivery model:** the slot+1 `DirectAttestation` unicast EXISTS and is live
  (`startup.rs:647` send; `network_events.rs:326-343` receive-and-record; re-broadcast
  `event_loop.rs:496-505`; capability version-gated `status.rs:38`). Three overlapping channels
  carry attestation data: (1) global flood, (2) next-producer unicast, (3) on-chain bitfield.
  The flood is partially redundant TODAY — but no metric exists on unicast delivery share, so
  flood removal stays measurement-gated (Patterns P5 kill test: UNRESOLVED without a metric).
  **MEASURED 2026-07-19: confirmed — delivery-share is NOT observable from current telemetry
  (byte counters unwired; no flood-receive marker). A new receive-path counter is a prerequisite
  to exercising Tier 1.**

## The Honest Cost Model f(N) — MEASURED 2026-07-19

Per node per 10s slot. Constants MEASURED on Apple Silicon against the real code paths
(benchmark + live-testnet + executable-proof pass; supersedes the 2026-07-18 modeled constants
of ~430B on-wire / ~50µs×2 verify / D_dup≈8):

| Constant | MEASURED value | Source |
|----------|----------------|--------|
| Attestation wire size (with BLS) | **276 B** | bincode, `attestation.rs:120` |
| Attestation wire size (without BLS, T0-A) | **180 B** (−34.8%) | bincode, same path |
| Ed25519 verify | **20.98 µs** | benchmark |
| Effective verify per attestation | **~42 µs** | gossipsub `ValidationMode::Strict` double verify — envelope + app (`config.rs:195`) |
| BLS `fast_aggregate_verify` | **0.44 ms, O(1) in N** | `bls.rs:640` — confirms pairings were NEVER the bottleneck |
| Gossipsub mesh degree D | adaptive, caps at `MESH_N_CAP=50` | `config.rs:154-160,30` |
| Dedup | 60s BLAKE3 duplicate cache | `config.rs:185,220` |

`flood_publish=true` affects ONLY the publisher's egress; relaying is mesh-bounded + deduped,
so **unique ingress ≈ N × wire, NOT N × D × wire**. Recomputed curve (10s slot, conservative
2× verify):

| N | Unique ingress/slot | Verify CPU/slot | % of 10s slot | Assessment |
|------|---------------------|-----------------|---------------|------------|
| 33 (today) | ~14 KB (~1.4 KB/s) | ~1.4 ms | 0.014% | trivial |
| 300 | ~125 KB (~12.5 KB/s) | ~13 ms | 0.13% | trivial |
| **1000 (Law #3)** | **~416 KB (~42 KB/s)** | **~42 ms** | **0.42%** | **comfortable** |
| 3000 | ~1.25 MB (~125 KB/s) | ~126 ms | 1.26% | comfortable |
| 10000 | ~4.2 MB (~420 KB/s) | ~420 ms | 4.2% | still comfortable |
| 30000 | ~12.5 MB (~1.25 MB/s) | ~1.26 s | 12.6% | discomfort |

The prior modeled "~1% at N=1000, comfortable to ~3000" **HOLDS under measurement**, and the
CPU comfort margin now extends to **~N=10000**. RAM: dup-cache ≈6N msg-ids ≈ low-single-digit
MB at N=3000. The INC-I-009 86GB blowup was connection-buffer-driven (fixed by max_peers=50),
orthogonal to attestation message count; INC-I-114 was flood_publish-amplification-driven (now
guarded by INV-NETWORK-002, `gossip/config.rs:92-134`). **The O(N) term is real; its measured
coefficient is trivial. Nothing breaks at the stated target.**

**Residual scaling variable (MEASURED 2026-07-19 — this replaces "flood_publish → O(N²)" and
BLS as the thing to fear): pre-dedup gossip-ENVELOPE re-verification.** Worst case is up to
~N×D envelope verifies per slot (≈10% of slot at N=1000) IF the dedup cache does not
short-circuit the Strict envelope check. **This — not BLS, not the flood — is the metric to
instrument before scaling past ~N=3000.**

**The single most important number: at Law #3's N=1000, the flood costs 0.42% of slot CPU and
~42 KB/s (MEASURED) — the comfort runway now extends to ~N=10000 before discomfort.**

## Definite Changes (High Convergence)

- ARCHITECTURAL: Defer ALL structural aggregation — the flat single-topic flood REMAINS the
  attestation dissemination architecture until a measured tripwire trips.
    Convergence: Radical P1 + Failure archetype ranking + Patterns (all additive patterns
    deferred/rejected) + Restructurer (self-capped confidence on P1/P2) — 4.5/5, independent
    evidence bases (see independence check).
    Evidence: MEASURED cost table above (2026-07-19 benchmark pass against `attestation.rs:120`,
    `config.rs:154-160,185,195,220`, `bls.rs:640`); resolved contradiction C (floor-3-only
    backstop makes premature structural change strictly riskier than the flood it would
    replace); INC-I-016/062/054 incident record.
    Confidence: conf(0.95, measured) — raised from 0.85 (converged) by the 2026-07-19
    measurement pass: the re-derived curve is MORE comfortable than the modeled one.
    What changes architecturally: the decision itself is codified — no subnet topics, no
    aggregator roles, no committee sampling, no relay aggregation, no delivery-model change
    below the tripwire. Tripwire thresholds (any one trips → open a Tier 1/Tier 2 design
    session under full activation-height discipline): (a) active producers ≥ 2000 —
    **MEASURED 2026-07-19: CPU headroom to ~N=10000 supports relaxing this toward ≥ 5000;
    ingress/occupancy alarms stay as-is** — (b) sustained attestation ingress > 20 Mbit/s,
    (c) swarm-loop gossip occupancy > 20% of slot. Register as `monitoring_signals` rows for
    INC-I-141. Non-foreclosure is preserved: the `RegionAggregate` scaffold, subnet topic
    plumbing, the retained (empty) `bls_signature` wire field, and the on-chain bitfield all
    remain, so both Tier 2 endgame shapes stay buildable. No code change; status-quo is 0.42%
    slot CPU / ~42 KB/s at N=1000 (MEASURED 2026-07-19, cost table).

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
      Why this proposal anyway: doing nothing structural IS the floor; the tripwire converts a speculative redesign into a measured one
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Retire the write-only per-attestation BLS gossip field — producers emit
  `Attestation` with empty `bls_signature` (Ed25519-only), deleting the wire half of the dead
  BLS pipeline. **STATUS 2026-07-19: session name "Tier 0b" — proven-safe by executable proof,
  DEFERRED, sitting in worktree reference (not applied to main).**
    Convergence: Subtractionist P2 + Radical P2 + Patterns P4 (conditional) — 3/5; the shared
    open question ("any live reader?") discharged by synthesizer verification (Resolved
    Contradiction B: full consumer chain terminates in dead code; both receive sites already
    handle empty; `#[serde(default)]`).
    Evidence: `attestation.rs:28-33` (field), `startup.rs:599-611` (attach point),
    `network_events.rs:334,556` (empty-tolerant receivers), dead sink chain per Contradiction B;
    MEASURED wire sizes 276B → 180B (bincode, `attestation.rs:120`).
    Confidence: conf(0.92, measured) — raised from 0.85 (converged) by the 2026-07-19
    executable proof (worktree build + tests green with the change).
    What changes architecturally: the dual-signature attestation collapses to single-signature
    on the live path; attestation wire 276B→180B (−34.8%, MEASURED 2026-07-19); the
    `bls_signature` field REMAINS in the struct (bincode-positional, empty Vec) so old and new
    nodes interoperate and Tier 2 can repopulate it later without a format change — this is
    what voids the foreclosure objection Radical raised.

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      -1 BLS sign per attestation per producer per slot (observed)
        Memory:   -96B per tracked attestation (observed)
        IO:       0 (observed)
        Network:  -96B x N producers x mesh-bounded fanout per slot (276B to 180B bincode, MEASURED 2026-07-19) (measured)
        Disk:     0 (observed)
        Latency:  -negligible (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: status quo — keep signing/gossiping BLS bytes nothing verifies
      Why this proposal anyway: pays the 96B tax on every mesh edge per slot for a signature verified by zero nodes; subtraction attacks the exact O(N·fanout) term at zero new complexity
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete the dead validate path — bare `validate_block()`
  (`validation/block.rs:104`), `validate_bls_aggregate()` (`validation/registration.rs:262`),
  `producer_bls_keys` field (`validation/types.rs:152,271`), and their re-exports
  (`lib.rs:301`, `validation/mod.rs:55`). **STATUS 2026-07-19: session name "Tier 0a" —
  APPLIED to main tree (uncommitted): 5 files, 151 deletions in crates/core. Build/clippy/fmt
  green; doli-core 972 tests + doli-node 52 tests green with the dead path removed;
  `producer_bls_keys` already carried `#[allow(dead_code)]`. No consensus, block-content, or
  activation-height impact.**
    Convergence: Subtractionist P3 + analyst §3/REQ-ATT-004 + prior audit CORE-C1
    (`docs/audits/audit-2026-02-24.md:37`) + synthesizer grep (zero live callers) — four
    independent sources.
    Evidence: grep `validate_block(` excluding `_with_mode`/`_for_apply`/tests → zero live
    call sites (verified 2026-07-18); `producer_bls_keys` init `Vec::new()`, never populated;
    executable proof 2026-07-19 (full workspace build exit 0, all affected test suites green).
    Confidence: conf(0.97, measured) — raised from 0.85 (converged) by the applied deletion
    passing the full build/test gauntlet.
    What changes architecturally: ONE live validation entry point remains
    (`validate_block_with_mode`). This dead path generated the false premise that launched this
    redesign (two agents burned effort disproving it) — deletion removes the recurring
    maintenance hazard at the source. Future verified-finality (REQ-ATT-005, Option O3) would
    wire into the LIVE path; the O(1) primitive `crypto::bls_verify_aggregate` (`bls.rs:621`)
    survives independently. Net −151 lines deleted (measured; was estimated −120), 0 additions,
    0 runtime behavior change (dead code, never executed; marginally smaller binary).

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed)
        Memory:   0 (observed)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: a DEAD-marker comment pointing at INC-I-141 instead of deleting
      Why this proposal anyway: a comment did not stop this redesign's false premise from being derived off wired-looking code; deletion removes the hazard at the source
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: Stop populating `block.aggregate_bls_signature` — `aggregate_bls_signatures()`
  (`assembly.rs:616`) returns empty; own-block BLS recording (`assembly.rs:658`,
  `post_commit.rs:385`) drops its BLS arm; the FIELD stays on `Block` and in `block_store`
  types for historical-block deserialization. **STATUS 2026-07-19: session name "Tier 0c" —
  proven-safe by executable proof, DEFERRED, sitting in worktree reference. This is the only
  Tier 0 block-content touch; rolling-safe because `Block::hash()` is header-only — blocks
  hash identically across a mixed fleet, so the INC-I-062 fork mechanism cannot fire.**
    Convergence: Subtractionist P1 + Radical P2 (consequence) + Patterns P2 (BLS demoted to
    non-scale-lever) — 3/5.
    Evidence: blob uncommitted (`Block::hash()` header-only, `block.rs:76-84,188`); zero live
    verifiers (Contradiction B); RPC handles empty (`rpc/types/block.rs:82` → `None`).
    Confidence: conf(0.9, measured) — raised from 0.75 (converged) by the 2026-07-19 executable
    proof + the header-only-hash rolling-safety argument; held below the applied Tier 0a ONLY
    for the INC-I-062 user sign-off below, not for technical doubt.
    What changes architecturally: blocks stop carrying ~104B of write-only body data; producer
    drops per-block G1 aggregation CPU. ⚠ This changes what a producer puts INTO a block, which
    the CLAUDE.md INC-I-062 rule blanket-flags for synchronized deploy. The blanket rule's fork
    mechanism (mixed-version content → divergent STATE) does not apply here — the blob feeds no
    validation, no state, no hash, and every block is internally consistent as produced — so
    rolling deploy is safe. Per Rule #0b this exception requires explicit USER sign-off before
    shipping; see Migration Path M3 and the INC-I-075 checklist below.

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      -O(N) G1-aggregation per produced block (observed)
        Memory:   -small aggregation buffers (observed)
        IO:       0 (observed)
        Network:  -104B per propagated block (observed)
        Disk:     -104B per stored block body (observed)
        Latency:  -small at block assembly (inferred)
      Inevitability: AVOIDABLE
      Cheaper alternative: ship only the gossip-field retirement (T0-A) and let the blob shrink to the producer's own sig
      Why this proposal anyway: leaves no half-retired pipeline — the write-only subsystem ends cleanly at both ends, and block wire/disk stop carrying dead bytes
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Retain `RegionAggregate` (`attestation.rs:135-215`) and the subnet/region
  topic plumbing (`gossip/mod.rs:47`, `publish.rs:72-106`) as ANNOTATED deferred scaffold —
  do not wire, do not delete.
    Convergence: contradiction between Patterns P3 (delete: "models Ethereum's leaderless
    world") and Radical P3 / Restructurer P2 (retain: it is the Tier 2 batching seed).
    RESOLUTION: retain. Deleting buys zero runtime (dead code, negligible) while the Tier 2
    endgame — whichever shape wins at the tripwire — reuses either its message shape
    (Ed25519 batch, Radical b1) or its subnet plumbing (Restructurer P2). Patterns' anchoring
    concern is met by annotation: a doc comment stating "DEFERRED SCAFFOLD — do NOT wire
    without the Tier 2 design session; see specs/attestation-gossip-scaling-architecture.md.
    DOLI is single-leader: the block producer is the natural aggregator (Pattern A), not
    per-region elected aggregators."
    Evidence: zero production callers (three evaluators' greps agree); struct comment already
    sizes it for "~1000 attestors/region".
    Confidence: conf(0.7, converged)

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (measured)
        Memory:   0 (measured)
        IO:       0 (measured)
        Network:  0 (measured)
        Disk:     0 (measured)
        Latency:  0 (measured)
      Inevitability: AVOIDABLE
      Cheaper alternative: delete outright (git-recoverable)
      Why this proposal anyway: retention preserves the non-foreclosure Should at zero runtime cost; the annotation neutralizes the wrong-pattern-anchoring risk deletion was meant to solve
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

**O1 — Tier 1 lever: de-amplify `flood_publish` for attestations (AT the tripwire, not now).**
`flood_publish(true)` (`config.rs:224,355`) makes the origin send to ALL ≤50 peers instead of
the mesh subset — implicated in the INC-I-114 OOM. **MEASURED 2026-07-19: the flag affects
ONLY the publisher's egress (relaying is mesh-bounded + deduped), so this lever buys less than
the 2026-07-18 model assumed; the residual variable it does NOT address is pre-dedup envelope
re-verification (see cost model).** BUT: (i) the flag is **global** to the gossipsub behaviour
— it also covers BLOCKS, and its stated purpose is defensive block delivery ("ensures our
blocks/attestations reach everyone regardless of mesh topology"); per-topic control in the
pinned libp2p is UNVERIFIED (Patterns U4); (ii) with the exclusion backstop being floor-3-only
(Contradiction C), any delivery-reliability reduction feeds the INC-I-016 shape; (iii) at N=33
it saves nothing that matters. Conditions to exercise: **wire a NEW receive-path counter first
— delivery-share is NOT observable from current telemetry (byte counters unwired, no
flood-receive marker; MEASURED 2026-07-19)**; verify per-topic capability or scope via a second
gossipsub behaviour; INV-NETWORK-002 compliance (`config.rs:92-134`); testnet soak + gauntlet.
Confidence: conf(0.7, measured — flood mechanics now measured, but the safe off-path remains
unvalidated and gated on the new counter). Complexity: 0 modules (config) or +1 behaviour
(scoped).

**O2 — Drop `attester_weight` from the wire; receivers recompute from local `ProducerSet`
(LOW-EVIDENCE).** Subtractionist P5 only, conf(0.55, observed). Independent of byte savings
(-8B), this is a **security signal**: the receiver trusts a forgeable u64 that feeds finality
weight (`network_events.rs:552` → `production_gate.rs:496` → reorg-depth gating). NOT
backward-compatible (fixed-layout bincode field) → needs a coordinated/gated rollout unlike
Tier 0. Recommended routing: security review (`/omega-doctor` or audit), not this redesign.

**O3 — Verified BLS finality (analyst REQ-ATT-005, Should).** Orthogonal to the gossip wall
(the aggregate rides inside the block — zero volume relief; Patterns P2 kill test CONFIRMED
misdirected for scale; BLS `fast_aggregate_verify` MEASURED at 0.44 ms O(1) — cheap, but not a
scale lever). If DOLI wants cryptographically verified finality: populate producer BLS keys
once per epoch, verify via `bls_verify_aggregate` from `validate_block_with_mode` behind a NEW
forward-only activation height, `pks_validate=false`. NOTE: adopting O3 REVERSES Tier 0's
T0-A/T0-B (the field/blob must be repopulated — compatible, since both are retained empty).
Decide O3 before or independently of shipping Tier 0; conf(0.6, observed — analyst SSF, demoted
by 2 evaluators as non-scale).

**O4 — Delete `RegionAggregate` instead of retaining (Patterns P3, conf 0.66).** Counter-option
to the Recommended retention. Choose this only if the fleet forecloses region/subnet batching
as a Tier 2 shape permanently.

## Rejected (2+ evaluators, zero dissent — do not re-derive)

- **Committee sampling / sortition**: breaks bond-weighted per-minute reward attribution
  (≥54/60 minutes threshold, `attestation.rs:229-235`, `rewards.rs:128-157`) — a non-sampled
  producer loses rewards through no fault; frontally violates F1/F2/F4/F8; highest fork risk;
  re-arms INC-I-016 by design. (Patterns fit 2/10; Failure rank 5/5.)
- **Relay/seed/designated-infra aggregation**: relays are transport, not consensus
  (MEMORY.md); concentrates O(N) ingress on few nodes (INC-I-009/014 RAM shape); creates a
  censorship + liveness dependency — a Byzantine aggregator can selectively drop producers to
  deny rewards/trigger exclusion. Admissible ONLY as deterministic-from-consensus + rotating +
  fail-open-to-flood, with a periodic fallback canary (Failure P4/F5; Restructurer P3).

## Constraints — Failure Filters F1-F9 (from the Failure Analyst; binding on ALL tiers)

| # | Filter | Tier 0 | Tier 1 (O1) | Tier 2 (deferred) |
|---|--------|--------|-------------|-------------------|
| F1 | Canonical-bitfield equivalence (bit-identical bitfield or NEW forward-only AH) | PASS — bitfield built from presence, not BLS; bits unchanged | CONDITIONAL — reduced delivery may change observable attester set → must prove or gate | BINDING — the central obligation; prove input-set equivalence, not just encoding |
| F2 | Index-parity lock (encoder `[base_list \| extra sorted]` = all 4 decoders) | PASS — no index change | PASS — no index change | BINDING — aggregates must carry GLOBAL indices |
| F3 | Single deterministic canonicalizer (one producer folds gossip → consensus content) | PASS — unchanged | PASS — unchanged | BINDING — sealing producer stays sole bitfield writer; per-node aggregate merges into exclusion input = fork by construction |
| F4 | Delivery ≠ exclusion (backstop is ONLY MIN_PRODUCERS_FLOOR=3 — verified) | PASS — delivery unchanged | **HARDENED** — any reliability cut needs a per-epoch exclusion DELTA cap or grace FIRST | BINDING — correlated-failure unit grows to a subnet; delta cap REQUIRED before aggregation feeds `compute_live_producer_list` |
| F5 | No new censorship/liveness dependency (deterministic + rotating + fail-open + canary) | PASS — no new role | PASS — no new role | BINDING — kills naive designated-aggregator variants |
| F6 | History-robust determinism (snap-sync reproducibility; don't enlarge `silent_bitfield_count` gap, `rewards.rs:151,163`) | PASS — block content shrinks by inert bytes; field retained for old-block decode | PASS | BINDING — moving attestation data into blocks enlarges the INC-I-082 gap surface |
| F7 | Deterministic aggregation order (sort discipline per `fingerprint()`, x86/ARM byte-identical) | PASS — removes an aggregation | N/A | BINDING on any BLS/batch aggregate |
| F8 | Reward-attribution invariance (`calculate_epoch_rewards` reads bitfield only) | PASS | CONDITIONAL (via F1) | BINDING |
| F9 | RAM/volume budget modeled at N∈{300,1000,3000} vs flood_publish reality | PASS — strictly reduces bytes | PURPOSE of the lever; must model, not assume | BINDING — an aggregate topic still under flood_publish semantics may not relieve RAM |

## Architecture Maps

### Current Architecture
```
producer i (per 10s slot):
  attest_own_block ──► Attestation{Ed25519 64B + BLS 96B + weight 8B} (276B bincode wire, MEASURED)
    ├─► BroadcastAttestation ─► ATTESTATION_TOPIC flood (flood_publish=true → publisher egress to ALL ≤50 peers;
    │                            relaying mesh-bounded, cap MESH_N_CAP=50, + 60s BLAKE3 dedup)  [channel 1]
    └─► DirectAttestation unicast ─► slot+1 producer (version-gated)                            [channel 2]
every node, every attestation: on_new_attestation → verify (~42µs effective: envelope + app) → finality_tracker (weight)
                                                                     → minute_tracker (+ dead bls_sigs map)
slot+1 producer: minute_tracker → bitfield (presence_root-committed) + aggregate_bls_signature (WRITE-ONLY)
every node at apply: bitfield decode → liveness/rewards/RPC                                     [channel 3]
DEAD (validate_block/validate_bls_aggregate/producer_bls_keys — DELETED from main 2026-07-19, Tier 0a);
      RegionAggregate + region topics (zero production callers, retained scaffold)
```

### Proposed Architecture (Definite + Recommended)
```
IDENTICAL dissemination topology (flood + unicast + bitfield). Differences:
  - Attestation gossips Ed25519-only (180B bincode wire, -34.8% MEASURED); bls_signature field retained, empty
  - block.aggregate_bls_signature emitted empty (field retained; historical blocks decode unchanged)
  - bare validate_block / validate_bls_aggregate / producer_bls_keys DELETED (one validation entry point) [DONE 2026-07-19]
  - RegionAggregate retained, annotated as deferred Tier 2 scaffold
  - Tripwire monitoring: N≥2000 (relaxable →≥5000 per measured headroom) OR ingress>20Mbit/s
    OR gossip-occupancy>20% → opens Tier 1/2 design session
```

## Consensus Migration Plan (per tier)

### Tier 0 (Definite + Recommended subtractions)
- **Activation height:** NOT required. No consensus rule changes; accepted/rejected block set,
  bitfield, presence_root, state roots all bit-identical (the retired data is uncommitted and
  unverified — Contradiction B).
- **INC-I-075 three-question checklist:**
  Q1 Can a user tx trigger the path? NO (attestation production/gossip only).
  Q2 Can a producer-action/attestation pattern trigger it? YES — every attestation and every
  produced block traverses it.
  Q3 Bit-identical for ALL reachable inputs? YES for every consensus-visible output (block
  hash, bitfield, presence_root, exclusion, rewards, 3 state roots). The gossiped bytes and the
  uncommitted block-body blob change, but neither feeds any validation/state computation
  (verified: zero live readers). → Q2=YES with Q3=YES ⇒ **no activation height required.**
- **Deploy:** ROLLING. T0-A/T0-C are unambiguous (gossip + dead code). T0-B alters block
  CONTENT → INC-I-062 blanket rule flags synchronized deploy; the fork mechanism does not apply
  (blob feeds nothing; blocks internally consistent; `Block::hash()` is header-only, so blocks
  hash identically across a mixed fleet — INC-I-062 cannot fire) ⇒ rolling is safe, but this
  exception needs **explicit user sign-off** (Rule #0b) and devnet→testnet canary first.
- **Snap-sync:** SAFE — struct/storage fields retained; historical blocks with populated blobs
  deserialize unchanged; 3-state convergence untouched (no state input changes).
- **Rollback:** revert the binary. Both directions are compatible: empty and populated
  BLS fields/blobs are BOTH already-valid inputs on every node today (`#[serde(default)]`,
  `is_empty()` branches, pre-BLS-block acceptance at `validation/block.rs:172-174`).

### Tier 1 (O1 — flood de-amplification; DEFERRED to tripwire)
- **Activation height:** not required IF purely transport (F1 partial-escape: changes how bytes
  travel, not what the sealing producer observes). MUST prove delivery-equivalence first —
  otherwise the observable attester set changes → C3 Q3=NO → AH required.
- **INC-I-075 checklist:** Q1 NO / Q2 YES (attestation delivery) / Q3 UNPROVEN — delivery
  reliability may change which attestations the sealing producer observes within the minute
  window. Until Q3 is proven YES by measurement, treat as consensus-visible and gate.
- **Deploy:** rolling (config), but treated as a delivery-reliability change: **wire the new
  receive-path counter first (delivery-share unobservable today — MEASURED 2026-07-19)** →
  INV-NETWORK-002 compliance → testnet soak → gauntlet → canaried rollout.
- **Rollback:** config revert. **Snap-sync:** unaffected.

### Tier 2 (structural aggregation; DEFERRED behind tripwire)
- **Activation height:** REQUIRED (NEW forward-only NetworkParams height; never reuse/move a
  crossed one) the moment aggregation can change which bits the sealing producer sets (F1).
- **INC-I-075 checklist (pre-answered for the future session):** Q1 NO / Q2 YES / Q3 NO by
  construction (delivery topology changes the observable set) ⇒ AH mandatory. "No producer uses
  subnets yet" is NEVER a valid skip justification.
- **Deploy:** rolling with mixed-version window; **mixed-version parity harness is mandatory
  BEFORE the height** (Failure P5): old-path and new-path producers must seal identical
  bitfields over identical input sets, INCLUDING a degraded-delivery partition case — encoding
  parity alone is false comfort. Old flood retained as fail-open fallback + periodic canary (F5).
- **Prerequisite:** per-epoch exclusion DELTA cap (or delivery-confidence grace) restoring the
  INC-I-016 protection BEFORE aggregation feeds `compute_live_producer_list` (F4 + Contradiction
  C). **Snap-sync:** model against INV-SYNC-007/INC-I-082 if any attestation data moves into
  block bodies (F6). **Rollback:** forward-only; fail-open flood is the operational fallback.

## Migration Path

Ordered; each step independently shippable and revertible. BRIDGE entries are transitional.

1. **M1 (T0-C, session "Tier 0a") — ✅ APPLIED to main 2026-07-19 (uncommitted):** delete bare
   `validate_block` + `validate_bls_aggregate` + `producer_bls_keys` + re-exports. 5 files, 151
   deletions in crates/core. Full workspace build exit 0; clippy/fmt green; doli-core 972 +
   doli-node 52 tests green. Zero behavior change confirmed executably. No deploy urgency.
2. **M2 (T0-A, session "Tier 0b") — DEFERRED, proven-safe (worktree reference):**
   `create_and_broadcast_attestation` uses Ed25519-only construction; field stays, empty
   (wire 276B→180B, −34.8% MEASURED). Rolling deploy; mixed fleet emits mixed messages — both
   already-valid today.
3. **M3 (T0-B, session "Tier 0c") — DEFERRED, proven-safe (worktree reference); ships only
   after user sign-off on the INC-I-062 exception:** `aggregate_bls_signatures()` → empty; drop
   own-BLS recording arms. Only Tier 0 block-content touch; rolling-safe because `Block::hash()`
   is header-only (mixed fleet hashes identically → INC-I-062 cannot fire). Devnet → local
   testnet canary (verify block acceptance, bitfield parity, RPC
   `aggregate_bls_signature: null` cosmetics) → rolling deploy.
4. **M4 (tripwire):** register `monitoring_signals` (INC-I-141): active-producer count ≥2000
   (relaxable toward ≥5000 per measured CPU headroom — Measurement Addendum); attestation
   ingress >20 Mbit/s; swarm-loop gossip occupancy >20%. Grafana panel on the existing
   Prometheus stack (ai5). No consensus code. **NEW prerequisite for any Tier 1 exercise:
   a receive-path counter (delivery-share + flood-receive marker) — current byte counters are
   unwired (MEASURED 2026-07-19).**
5. **M5 (annotation):** doc comment on `RegionAggregate` + region topics per the Recommended
   change; fix MEMORY.md/CLAUDE.md doc drift on `MAX_EXCLUSIONS_PER_BLOCK` (Contradiction C).
6. **BRIDGE: none required.** Tier 0 is pure subtraction; no transitional shims exist to remove
   later. (Recorded explicitly so nothing patch-shaped ships under this spec's banner.)

## Complexity Comparison

| Metric | Current | Radical Minimum (pure DEFER) | Proposed (Tier 0 + tripwire) |
|--------|---------|------------------------------|------------------------------|
| Gossip topics (attestation) | 1 | 1 | 1 |
| Dissemination channels | 3 (flood, unicast, bitfield) | 3 | 3 |
| Live signature schemes on attestation wire | 2 (Ed25519 + write-only BLS) | 2 | **1** (Ed25519) |
| Validation entry points | 2 (1 live + 1 dead) | 2 | **1** (achieved 2026-07-19, Tier 0a) |
| Dead abstractions in-tree | 4 (validate path, BLS pipeline, RegionAggregate, region topics) | 4 | 2 (RegionAggregate + region topics, annotated as deliberate scaffold) |
| New modules/interfaces/roles | — | 0 | **0** |
| Consensus-rule changes / activation heights | — | 0 | **0** |
| Bytes per gossiped attestation (bincode wire, MEASURED) | 276 | 276 | **180 (−34.8%)** |

Radical tiebreaker: the proposed Tier 0 is STRICTLY SIMPLER than the radical floor (it deletes;
the floor merely abstains) and the radical evaluator's own P2 endorses it as the
if-forced-to-act move. No complex proposal came within 0.1 confidence of the floor — nothing
additive survives filtering; SSF satisfied.

## Milestones

Tier 0 touches 2 crates (core, node) across 5 small steps (M1-M5) — below the 4-module
milestone threshold; M1-M5 above serve as the execution checklist. M1 is DONE (2026-07-19,
uncommitted). Tier 2, if ever triggered, REQUIRES its own design session, spec, and milestones.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     3 (DEFER decision, BLS gossip field, dead validate path — 3+/5 or multi-source equivalent)
Restructuring convergence:      1 (producer-as-aggregator endgame shape, 4/5 — DEFERRED)
Addition options presented:     4 (O1 flood lever, O2 attester_weight, O3 verified finality, O4 delete scaffold)
Failure modes identified:       9 (F1-F9)
Failure modes applied as filters: 9/9 (per-tier table)
Radical floor gap:              current → radical minimum (=current, 0 changes) → proposed (= current MINUS dead pipeline; net -1 sig scheme, -1 validate path, 0 additions)
Contradictions found:           5 (A topic, B BLS safety, C exclusion caps, D delivery model, E RegionAggregate fate)
Contradictions resolved:        5/5 (A-D by synthesizer grep; E by non-foreclosure argument)
Evidence independence verified: YES (per-cluster checks in Convergence Matrix)
Measurement pass (2026-07-19):  benchmark + live-testnet + executable proof; all headline
                                constants upgraded assumed/modeled → measured; verdict unchanged
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Measurement Addendum (2026-07-19)

A benchmarking + live-testnet + executable-proof pass replaced this spec's theoretical cost
model with MEASURED evidence (Apple Silicon, real code paths). **The verdict is unchanged:
CONDITIONAL NO-GO / DEFER — now at conf(0.95, measured).**

### Measured constants

| Constant | MEASURED 2026-07-19 | Source |
|----------|---------------------|--------|
| Attestation wire (with BLS) | 276 B | bincode, `attestation.rs:120` |
| Attestation wire (without BLS) | 180 B (−34.8%) | bincode |
| Ed25519 verify | 20.98 µs | benchmark |
| Effective verify/attestation | ~42 µs | double verify under `ValidationMode::Strict` (envelope + app), `config.rs:195` |
| BLS `fast_aggregate_verify` | 0.44 ms, O(1) in N | `bls.rs:640` — pairings were NEVER the bottleneck |
| Mesh degree D | adaptive, cap `MESH_N_CAP=50` | `config.rs:154-160,30` |
| Dedup | 60s BLAKE3 cache | `config.rs:185,220` |

### Recomputed curve (10s slot, conservative 2× verify)

N=1000 → ~416 KB/slot (~42 KB/s), 42 ms verify, **0.42% of slot**. N=3000 → **1.26%**.
N=10000 → **4.2%**. The prior "~1% at N=1000, comfortable to ~3000" HOLDS; CPU comfort now
extends to **~N=10000**.

### Defused failure mode

The "flood_publish → O(N²)" alarm is mitigated: `flood_publish` hits ONLY the publisher's
egress; unique ingress is N×wire (not N×D×wire) thanks to the 60s dedup cache. **The REAL
residual scaling variable is pre-dedup gossip-ENVELOPE re-verification** — up to ~N×D
(≈10% of slot at N=1000) IF dedup does not short-circuit it. THIS (not BLS, not the flood) is
the metric to instrument before scaling past ~N=3000.

### Tripwire update

Given measured CPU headroom to ~N=10000, the Tier-2 revisit tripwire can move from **N ≥ 2000
out toward N ≥ 5000**. Ingress (>20 Mbit/s) and occupancy (>20% of slot) alarms stay as-is.

### Tier 0 status — proven and shipped

- **Executable proof:** full workspace build exit 0; doli-core 972 tests + doli-node 52 tests
  green with the dead path removed; `producer_bls_keys` already carried `#[allow(dead_code)]`.
- **SHIPPED: Tier 0a (spec T0-C/M1) APPLIED to main tree (uncommitted)** — deleted dead
  `validate_block()`, `validate_bls_aggregate`, `producer_bls_keys` + re-exports (5 files, 151
  deletions in crates/core). Build/clippy/fmt/tests green. conf(0.97). No
  consensus/block-content/activation-height impact.
- **Tier 0b (T0-A, empty gossiped BLS, −35% wire) and Tier 0c (T0-B, empty
  `block.aggregate_bls_signature`) = DEFERRED, proven-safe, sitting in worktree reference.**
  0c is the only block-content touch; rolling-safe because `Block::hash()` is header-only
  (blocks hash identically across a mixed fleet → INC-I-062 cannot fire).

### Confidence deltas

| Item | 2026-07-18 | 2026-07-19 | Basis of change |
|------|-----------|-----------|-----------------|
| DEFER verdict | 0.85 | **0.95** | measured curve confirms + improves the modeled one |
| Tier 0a (dead validate path, T0-C) | 0.85 | **0.97** | applied + full build/test proof |
| Tier 0b (gossip BLS field, T0-A) | 0.85 | **0.92** | executable proof in worktree |
| Tier 0c (block blob, T0-B) | 0.75 | **0.9** | executable proof + header-only-hash rolling-safety |
| Tier 1 (flood lever, O1) | 0.7 | **0.7 (held)** | gated on a NEW receive-path counter — delivery-share NOT observable (byte counters unwired, no flood-receive marker) |
| Tier 2 (structural aggregation) | 0.6 | **0.6 (held)** | un-prototyped; nothing measured changes its risk profile |

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
Why this proposal anyway: documentation-only addendum recording measured evidence; the only code change it records (Tier 0a) is pure dead-code deletion with zero runtime delta
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
