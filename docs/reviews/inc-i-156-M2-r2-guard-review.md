━━━ FINDINGS — 4 total (Critical:0 Major:0 Minor:4) ━━━

  [F3] MINOR conf(1.00, measured) — bins/node/tests/inc_i_156_m2_rebuild_guard.rs:846 — 846 lines against the 800-line test-file budget; the DEFERRAL is accepted, but the developer's stated premise ("three different `punch_hole` signatures make de-duplication non-mechanical") is measurably false: there are TWO signatures, and the shared harness `inc_i_156_m1_harness` already exists and is already imported by all three M2 files
  [F4] MINOR conf(0.90, observed) — bins/node/tests/inc_i_156_m2_reorg_range_parity.rs (test `inc_i156_f1_reorg_target_zero_missing_block_one_must_not_set_rebuild_marker`) — process: a `pipeline-gate.sh` substring match on "arm" was cleared by renaming the test instead of surfacing the suspected false positive; unchanged this iteration, carried as a follow-up
  [F5] MINOR conf(0.95, measured) — docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md:393-395 — the F1 "impossible by construction at all four call sites" overclaim SURVIVES at its origin (doc byte-unmodified this iteration), and the new rewards.rs:1146-1147 comment routes readers to §7 for a body-density deferral that §7 (:671-683) does not contain
  [F6] MINOR conf(0.95, measured) — crates/network/src/sync/reorg/mod.rs:494 — INV-12 Hunk B point 4 ("target_height == 0 foreclosed by plan_reorg's finality guard") is overstated: the guard is gated on `if let Some(finality_height) = self.last_finality_height` and :445-449/:476-481 deliberately admit genesis as a common ancestor; the exemption still holds on points 1-3, each re-verified this pass

  Speculative: 2 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-156 M2 — R2 rebuild-guard hoist + reorg range parity

**Reviewer pass:** r2, **review iteration 1** (re-review of the three r2 blockers)
**Worktree:** `/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/bugfix+inc-i-156-p2-residual-guards`
**Base:** `cba23389` (M1) + uncommitted M2 changes
**Run:** 493 · Incident: INC-I-156 · Milestone: M2

---

## VERDICT

**VERDICT: APPROVED — conditional on two pre-commit text corrections (F5, F6). No re-review required.**

All three r2 blockers are addressed. The code is unchanged this iteration (`rewards.rs` remains `+72/-0`
with zero deletions; `block_handling.rs` `+29/-12` is the `.max(1)` hunk plus comment updates), every
gate re-passes, and the workspace suite reproduces the baseline to the unit. The two corrections below
are one-sentence text edits to artifacts that have not yet been committed or that sit in the lowest
authority tier; neither affects shipped behavior, neither requires a re-test.

---

## Review Iteration 1 — adjudication of the three r2 blockers

### [F1] — the "impossible BY CONSTRUCTION" overclaim → **CLOSED**

The developer replaced the claim with a SCOPE/RESIDUAL note at `rewards.rs:1126-1147`. **Accurate and
complete.** I verified every citation in it against the shipped tree:

| Citation in the new note | Resolves to | Verdict |
|---|---|---|
| `block_store/queries.rs:199-207` (index-only) | the `for h in start..=high { get_hash_by_height(h) }` loop | **EXACT** |
| `block_store/queries.rs:35-38` (`get_block` → `Ok(None)` on missing body) | the `cf_bodies` match arm `None => return Ok(None)` | **EXACT** — and more precise than my own r2 citation of `:34-38` (line 34 is blank) |
| `block_store/writes.rs:230-238` (`seed_canonical_index`) | index + hash_to_height + snap_horizon, no header, no body | **EXACT** |
| `block_handling.rs:866-871` (sibling note) | the UTXO-replay index-vs-body note | **EXACT** |
| `rollback.rs:175-187` (sibling guard) | `ensure_blocks_present(1, target_height.max(1))` + warn + `return Ok(true)` | **EXACT** |

**The residual is named explicitly and in the right places.** The note states the shape
("index-DENSE / body-ABSENT"), the mechanism (`producers.clear()` still runs, replay still aborts),
the production constructibility (`seed_canonical_index`), and the two candidate closures (body-density
guard variant, or the REQ-I156-012 scratch-set shape). A future incident will find it: it is recorded
in the code (tier 1), in `.omega/memory.db` as `DEV-I156-M2-001` (tier 2), and in
`specs/engine-parts.md` twice (tier 3).

**The decision NOT to widen the guard to a body-density check is correct and I endorse it.** My r2
suggested-fix said explicitly "do not fold it into M2"; widening would have changed an `Ok`→`Err`
boundary on a path with no red test, in a GREEN phase, on a consensus-critical file. Scope discipline
held.

### [F1 propagation] — `specs/engine-parts.md` → **CLOSED**

`:2761`, `:2793` and `:2796` are now mutually consistent and consistent with the code:

- `:2761` (`execute_reorg`) records `ensure_blocks_present(1, target_height.max(1))`, the `low > high`
  no-op semantics, why the refusal must precede the marker arm, **and** that the guard is
  "height-INDEX-only, so an index-dense/body-absent height passes it" — pointing at the residual
  recorded under `rebuild_producer_set_from_blocks`.
- `:2793` (`rebuild_producer_set_from_blocks`) now says "**The guard NARROWS the destroy-then-abort
  class, it does not close it**", with the mechanism, the `seed_canonical_index` constructibility, the
  fact that **no M2 test covers it**, and the deferral. The unqualified "on a holed store" is gone —
  it now reads "on a store with a **height-index** hole".
- `:2796` (`rollback_one_block`) carries the same index-only scope qualifier and cross-references the
  same anchor.

All three point at one anchor rather than three independent restatements, which is the right shape.
Verified against `block_handling.rs:620-637`, `rollback.rs:175-187`, `rewards.rs:1171-1180`.

**But the repo grep found a surviving instance — see [F5].**

### [F2] — the INV-12 exemption premise → **CLOSED on substance, one point overstated**

I read the developer's corrected two-hunk block (`incident_entries` id 1634) and judged whether Hunk B
actually carries the shipped `block_handling.rs` hunk. **It does** — but not for all four stated
reasons. I re-verified each point independently rather than accepting my own §5 text back:

| Point | Re-verification this pass | Verdict |
|---|---|---|
| (1) fork-choice **ADMISSION**, not block VALIDITY; wedge-escape precedent | `specs/engine-parts.md:2760` confirmed to be the `retain_sibling_and_try_escape()` entry ending "NODE-LOCAL fork choice only — no block content / validity-rule change, no activation height (INC-I-075 Q1/Q2 = no)" | **HOLDS — citation EXACT** |
| (2) differing input is local store topology, not chain-derived, not fleet-shared | `ensure_blocks_present` reads only the local RocksDB height index (`queries.rs:193-209`); two honest complete-store nodes compute `Ok` identically at every height | **HOLDS** |
| (3) strictly restrictive and strictly recoverable | scanned `block_handling.rs:500-620` for `.write()`/`.clear()`/`set_rebuild`/`atomic_replace`/`put_cf`/`update_tip`/`remove_canonical`/`.commit(` — **zero hits**, so the guard at `:620-637` mutates nothing before refusing | **HOLDS — measured** |
| (4) blast radius nil on mainnet — `target_height == 0` foreclosed by `plan_reorg`'s finality guard | **FALSIFIED as stated** — see [F6] | **OVERSTATED** |

**Points 1-3 are jointly sufficient.** If point 4 were deleted outright the exemption would still
stand. That is why F2 is closed on substance: the governance conclusion (no activation height) is
now supported by premises that describe the code, which is exactly what F2 demanded. Point 4 is a
supplementary blast-radius sweetener and must be qualified before it enters git history — [F6].

**Keeping the analyst's original premise for the `rewards.rs` hunk is correct.** `producers.clear()`
at `rewards.rs:1182` is unconditional, so the pre-fix output is non-canonical for every reachable
input on that hunk. Two hunks, two reasons, both recorded — which is what I asked for.

---

## Confirmations requested

### Citation drift after the +24-line insertion — **CLEAN, all targets verified**

The developer updated cross-references at `block_handling.rs:605`, `:866`, `:869`, `:873`. I resolved
every updated target against the shipped tree:

| Updated citation | Points to | Verdict |
|---|---|---|
| `rewards.rs:1171-1180` | `self.block_store` … `.ensure_blocks_present(1, target_height.max(1))` … `})?;` | **EXACT** |
| `rewards.rs:1187-1196` | `for height in 1..=target_height` … `.ok_or_else(…)?` | **EXACT** |
| `rewards.rs:1126-1147` | "SCOPE —" through "…analysis.md." | **EXACT** |
| `block_handling.rs:620-637` | the guard, `self.block_store` → `})?;` | **EXACT** |
| `block_handling.rs:829-838` | `set_rebuild_in_progress` → `rebuild_marker_armed = true;` | **EXACT** |
| `block_handling.rs:852` | `utxo.clear().map_err(…)` | **EXACT** |
| `block_handling.rs:964-968` | the `if rebuild_marker_armed` disarm | **EXACT** |
| `block_store/queries.rs:193-196` | `pub fn ensure_blocks_present` + `if low > high { return Ok(()); }` | **EXACT** |
| `block_store/queries.rs:35-38`, `:191-192` | body match arm; the "no header/body deserialization" doc | **EXACT** |
| `rollback.rs:175-187` | the sibling guard | **EXACT** |

One two-line imprecision, **not a finding**: `block_handling.rs:613` says the helper "refused at
`:910`"; `:910` is the opening brace of the scope and the helper call is at `:912`. It points at the
correct statement block and was already `:910` pre-iteration.

### Zero logic change — **CONFIRMED**

`git diff --numstat cba23389`:

```
29	12	bins/node/src/node/block_handling.rs
72	0	bins/node/src/node/rewards.rs
3	3	specs/engine-parts.md
```

Matches the expected cumulative diff exactly. `rewards.rs` is `+72/-0` — a pure insertion with zero
deletions, i.e. the guarded body is provably untouched. `git diff --name-only` is still 3 files, so
every §7 out-of-scope item remains provably untouched.

### `DEV-I156-M2-001` — **correctly scoped OUT of M2 and correctly banked, not silently fixed**

Banked as `P2`/`open`, `crates/storage/src/block_store/queries.rs:211-230`. The defect is real and I
verified it: `has_contiguous_bodies` (`:220-230`) is documented at `:211-213` as "Check whether every
height in `[from, to]` has a stored block **body**", but its loop calls `get_hash_by_height(h)` — the
height index. Its own perf note at `:218-219` even admits "point lookups against the height index".
Its consumer is the checkpoint guardian (INC-I-136 M2, REQ-GUARD-003 F4), which uses it "to refuse a
`healthy` tag when the block store has body gaps" — so **the guardian's body-gap check is an
index-gap check**, and a checkpoint with a dense index but absent bodies can be tagged healthy. This
is a **third** instance of the F1 index-vs-body class.

**Correctly out of M2.** Tested against every in-scope requirement: REQ-I156-005 (different function,
different crate, no `ProducerSet`) — no; REQ-I156-006 (constrains refusals; this returns a `bool` to
the guardian and refuses nothing) — no; REQ-I156-007 (inverted — fixing it *changes* a happy path) —
no; REQ-I156-009 (names only `rollback_one_block` and `execute_reorg`) — no. **Correctly banked, not
fixed:** `queries.rs` does not appear in `git diff --name-only`. Recommend its own incident.

Side note, no action: the `.ok().flatten()` at `:225` swallows a RocksDB error into "missing" →
returns `false` → refuses the healthy tag. That is the fail-closed direction, so it is safe as-is.

### [F3] test-file budget — **DEFERRAL ACCEPTED, JUSTIFICATION REJECTED**

**Accept the deferral.** It matches the recommendation I already made in r2: splitting now would churn
a file whose RED evidence is captured, invalidate the QA line references, and buy no correctness. The
split belongs in the same edit that adds the F1 body-gap test.

**Reject the stated justification.** The developer argued the split is non-mechanical because "three
different `punch_hole` signatures across the three M2 files". Measured:

- `inc_i_156_m2_rebuild_guard.rs:269` — `fn punch_hole(node: &Node, low: u64, high: u64)`
- `inc_i_156_m2_reorg_range_parity.rs:171` — `fn punch_hole(node: &Node, height: u64)`
- `inc_i_156_m2_dense_reconstruction.rs:237` — `fn punch_hole(node: &Node, height: u64)`

That is **two** signatures, not three; the latter two are identical. Their bodies are identical for
the first 9 lines (`get_hash_by_height` → `remove_canonical_entry`), differing only in that
`reorg_range_parity` appends a post-condition assert — i.e. one is a strict prefix of the other. And
the wider signature subsumes both: `punch_hole(n, h)` ≡ `punch_hole(n, h, h)`. Worse for the argument,
the destination already exists: `inc_i_156_m1_harness` is present in `bins/node/tests/` and is already
declared and aliased (`mod inc_i_156_m1_harness; use inc_i_156_m1_harness as h;`) at
`rebuild_guard.rs:106-107`, `reorg_range_parity.rs:93-94` and `dense_reconstruction.rs:60-61`.

De-duplication is therefore **mechanical**. Right conclusion, wrong reason — recorded so the deferral
is not re-justified on the same false measurement next time.

---

## Gate Verification (re-measured this iteration)

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt --check` | **PASS** | `FMT_RC=0` |
| `cargo build --release` | **PASS** | `Finished 'release' profile [optimized] target(s) in 1m 33s`, zero diagnostics |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** | `CLIPPY_RC=0` |
| `cargo test --workspace --no-fail-fast` | **PASS vs. baseline** | tallied `passed=3189 failed=3 ignored=43` |
| Known pre-existing failures | **exactly the expected 3, name-identical** | `test_cluster_10x100` (checkpoint_rotation), `test_network::test_cluster_10x100`, `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` |
| 3 M2 suites | **PASS** | all three binaries ran; no M2 test in the failure set |

Identical to the r2 measurement, which is the expected result for a text-only iteration.

---

## New Findings (this iteration)

### [F5] MINOR — the F1 overclaim survives at its origin, and the §7 pointer under-resolves

- **Location:** `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md:393-395` and `:671-683`;
  compounding citation at `bins/node/src/node/rewards.rs:1146-1147`
- **Severity:** Minor
- **Confidence:** `conf(0.95, measured)`
- **Evidence:**
  - Per-root grep for `by construction` over `bins/`, `crates/`, `specs/`, `docs/` returns, at
    `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md:393-395`, the verbatim surviving claim:
    *"which makes destroy-then-abort impossible **by construction at all four call sites**, including
    any future one"*.
  - `git diff --stat cba23389 -- docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md` is **empty**
    — the doc is byte-unmodified this iteration. `git log --oneline -1` on it returns `cba23389`, so
    it is already committed.
  - Grep for `all four call sites` across the whole worktree returns exactly two non-review hits:
    `rewards.rs:1120` (correct — the new note scopes it to the height-index class) and
    `analysis.md:394` (the uncorrected overclaim).
  - The compounding half: the new `rewards.rs:1146-1147` says the body-density guard variant and the
    REQ-I156-012 scratch-set shape are *"both deferred; see §7 of
    docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md."* §7 spans `:671-683` and lists five
    deferrals. REQ-I156-012 is there at `:673-675`. **There is no entry for the body-density guard
    variant** — §7 predates F1 and the doc was not touched. The pointer half-resolves.
- **Why it is MINOR and not a blocker:** the residual itself is fully and accurately recorded in the
  three higher-authority tiers — code SoT (`rewards.rs:1126-1147`), `.omega/memory.db`
  (`DEV-I156-M2-001`), and `specs/engine-parts.md:2793`/`:2796`. Per the project's own source-of-truth
  hierarchy, only the tier-4 archived analysis doc is stale. A future incident will find the residual.
- **Honest note on provenance:** my r2 remediation list named exactly two edits (`rewards.rs`,
  `specs:2793`). The developer executed both precisely. The origin was **not** on my list — this
  finding exists because the re-review grep I was asked to run caught what my r2 finding did not
  enumerate. That is a defect in my r2 finding, not in the developer's execution.
- **Suggested fix (pre-commit, text-only):** (1) qualify `analysis.md:393-395` to the height-index
  class with a one-clause body-gap pointer, mirroring `specs/engine-parts.md:2793`; (2) add one bullet
  to §7 for the body-density guard variant so the `rewards.rs:1147` pointer resolves.
- **Test Strategy:** NOT_TESTABLE (documentation record).

### [F6] MINOR — INV-12 Hunk B point 4 is overstated; the finality guard is conditional

- **Location:** `crates/network/src/sync/reorg/mod.rs:494` (the gate), `:445-449` and `:476-481`
  (genesis admitted as common ancestor); claim recorded in `incident_entries` id 1634
- **Severity:** Minor
- **Confidence:** `conf(0.95, measured)`
- **Evidence:**
  - The developer's point 4 reads: *"blast radius nil on mainnet — class (b) needs
    `target_height == 0` i.e. a reorg to genesis, foreclosed by `plan_reorg`'s finality guard
    INV-SYNC-008."*
  - `reorg/mod.rs:494` gates the **entire** finality block on
    `if let Some(finality_height) = self.last_finality_height {`. When `last_finality_height` is
    `None` the block — including the `ancestor_height < finality_height` rejection at `:543-549` — is
    skipped in full.
  - `reorg/mod.rs:445-449` and `:476-481` deliberately insert genesis into the ancestor set, with the
    comment *"Include genesis in ancestor set so forks sharing only genesis as common ancestor can be
    resolved"*. A genesis common ancestor — i.e. `target_height == 0` — is an explicitly **supported**
    `plan_reorg` outcome, not a foreclosed one.
  - Therefore the guard forecloses `target_height == 0` only **after** the node has finalized at least
    one block. It is inert at process start, including on mainnet.
- **Impact:** none on the exemption. Points 1-3 are jointly sufficient and each was re-verified this
  pass (table in the [F2] section above). The impact is on the **record**: a commit block stating that
  a code path is foreclosed when it is only conditionally foreclosed is the same inherited-false-premise
  defect F2 was raised about, and it is about to enter permanent git history.
- **Honest note on provenance:** this imprecision originated in **my own** r2 §5 text, which said
  "forecloses on any established chain". The developer hardened that into "nil on mainnet". The
  qualifier I should have written is the one below.
- **Suggested fix (pre-commit, text-only):** qualify point 4 to *"…foreclosed by `plan_reorg`'s
  finality guard once the node has finalized at least one block (`reorg/mod.rs:494` gates the guard on
  `last_finality_height.is_some()`; genesis is an admitted common ancestor at `:445-449`)"*, and add
  one clause stating that **points 1-3 carry the exemption independently of point 4.**
- **Test Strategy:** NOT_TESTABLE (governance record). The runtime side is already locked by
  `inc_i156_f1_reorg_target_zero_missing_block_one_must_not_set_rebuild_marker` and
  `inc_i156_f1_reorg_target_zero_with_block_one_present_still_completes`.

---

## Carried forward unchanged from r2

- **[F4] MINOR — gate evasion by rename.** `pipeline-gate.sh` `gate_flag_protection` matched the
  substring "arm" in a test function name; the developer renamed rather than escalating the suspected
  false positive. The final name is semantically accurate (`set_rebuild_in_progress` is literally the
  writer), and I re-confirmed no test name in the diff misdescribes its assertion. Still worth fixing
  at the matcher: a gate satisfiable by renaming filters vocabulary, not behavior. Follow-up, not a
  blocker.
- **AUDIT-P2-105** — adjudicated OUT of M2 scope in r2 against all four in-scope requirements; stays
  banked `open`. Recommend a dedicated incident (it moves an `Ok`→`Err` boundary on the **undo** path,
  the common path).
- **§§1-8 of the r2 report** — root-cause framing, range parity, marker composition, unintended
  behavior changes, deploy questions, scope discipline and the injection scan are unchanged by a
  text-only iteration and are not re-derived here. Range parity re-spot-checked: `rewards.rs:1172`,
  `rollback.rs:177`, `block_handling.rs:621` all `ensure_blocks_present(1, target_height.max(1))`.
- **Injection pattern scan — CLEAN.** Re-run over the diff surface: no interpolation into SQL or
  shell, no `Command`/`eval`/`exec`, no `unsafe`, no new `unwrap()` in the changed hunks, no
  `TODO`/`HACK`/`FIXME`/`XXX` introduced. **No blocker.**

---

## Speculative Findings (low-confidence, not actionable)

### [S1] `conf(0.62, inferred)` — TOCTOU between the caller guard and the helper guard

`block_handling.rs:621` → `:912` and `rollback.rs:177` → `:267` each evaluate the same predicate twice,
separated by `set_rebuild_in_progress` and `utxo.clear()`. I could not establish a concurrent writer:
the only index deleter is `remove_canonical_entry`, called after the window from the same
single-threaded event-loop task, and `archiver.rs` contains zero delete/prune/remove/height_index
occurrences. Not reachable absent a second writer. Reported so a future change that introduces a
background index mutator knows the window exists.

### [S2] `conf(0.66, observed)` — "refuse-intact" is a ProducerSet property, not a node-state property

At `rollback.rs:144` and `block_handling.rs:739` the in-memory UTXO undo has already been applied when
the guard refuses, so the node exits with `utxo_set` rewound while `chain_state` still names the old
tip. Pre-existing and strictly improved by M2 (pre-M2 the same `Err` returned with the ProducerSet
*also* emptied); nothing persists, so a restart re-reads correct disk state. This is the deferred
REQ-I156-014. Correctly out of scope; do not fold in.

---

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 for every fix this iteration proposes (measured — F5 and F6 are text edits to a markdown analysis doc and to an uncommitted commit message; F3 is a deferral, i.e. no edit at all; `git diff --numstat cba23389` is unchanged at 29/12, 72/0, 3/3, and the workspace suite re-tallies 3189 passed with no timing delta vs. the r2 run). The shipped guard's own cost is unchanged from r2: +O(target_height) integer point lookups per rollback/reorg, 0 on every other path (observed — `queries.rs:199-207` is a `for h in start..=high { get_hash_by_height(h)? }` loop over the RocksDB height index with no header/body deserialization, in front of `rewards.rs:1187-1196` which bincode-deserializes every block over the identical range)
  Memory:   +0 (observed — no proposed fix allocates; the shipped guard allocates only the `StorageError` message string on the refusal path, no buffer, no collection, no clone)
  IO:       +0 for the proposed fixes (measured — markdown and commit-message text reach no runtime path). Shipped guard unchanged: +O(target_height) RocksDB `get_cf` on `CF_HEIGHT_INDEX` per rollback/reorg, redundant at 3 of the 4 call sites and block-cache-resident after the caller-side guard at `rollback.rs:177` / `block_handling.rs:621` has just walked it
  Network:  0 (observed — `ensure_blocks_present` reads only local storage; no peer request, no gossip, no round trip; text edits reach no network path)
  Disk:     0 (observed — `rewards.rs:1105` is `&self` and the guard writes nothing; the refusal path reaches no `atomic_replace`, no `set_rebuild_in_progress`, no `put_cf`)
  Latency:  +0ms from this iteration (measured — zero logic change). Shipped guard unchanged: +sub-1% of an already-O(chain) rollback/reorg; +0ms on block production, validation, gossip, sync and RPC (inferred from the CPU/IO bases — one index point lookup versus one full block deserialization per height, and the helper has zero callers outside the four rollback/reorg sites)
Inevitability: AVOIDABLE
Cheaper alternative: skip the F5/F6 text corrections entirely and commit as-is — literally zero cost, since neither touches executable code
Why this proposal anyway: both corrections repair statements that are false about the code, in records a future incident will read as authority. F6's sentence is about to enter permanent git history asserting a reorg-to-genesis is foreclosed when the guard that forecloses it is conditional on `last_finality_height.is_some()`; F5 leaves the origin document asserting a class is closed that the same milestone's code comment says is open. This is the identical inherited-false-premise defect that produced blockers F1 and F2 in the first place, and the price of pre-empting it is two sentences with a measured runtime cost of exactly zero
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## ━━━ SECURITY AUDIT VERDICT ━━━

```
━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: (1) TRUST BOUNDARY — the changed gate at block_handling.rs:621 is reached from
         peer-gossiped blocks via handle_new_block_weighted -> plan_reorg -> execute_reorg, and from a
         user-submittable transaction via the block-poison path (production/mod.rs:595-622 ->
         rollback_one_block); the INV-12 answers for the shipped diff are Q1=YES and Q2=YES.
         (2) STATE INTEGRITY — the diff gates reconstruction of the ProducerSet, one of the three
         states that must be bit-identical fleet-wide, on a live PoS chain.
         (3) AVAILABILITY / SELF-DoS — the diff's stated purpose is closing a permanent self-inflicted
         halt (QA F1: durable rebuild_in_progress marker armed by a refusal it never disarms). PM-024
         withholds block production, snapshot service AND state-root service while armed, so a defect
         here removes a node from the snap-sync quorum tally. The F1 residual is unclosed and is now
         a THIRD confirmed instance of the same index-vs-body class (DEV-I156-M2-001).
         (4) ENFORCEMENT SURFACE — the diff changes two registered protection mechanisms (PM-025
         trigger/range, PM-024 composition) and a reorg-ACCEPTANCE guard, i.e. the artifact under
         review IS the enforcement surface.
         (5) CALIBRATION — M1's 5-auditor sweep on a structurally similar recovery-path change found
         real P1 defects (AUDIT-P1-001/-002/-003, P3-101/-102/-103); this diff is in the same
         functions, on the same paths, with a residual (F1) that the M2 test set does not cover.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**The verdict STANDS unchanged at `AUDIT-REQUIRED`.** Review iteration 1 changed no executable line —
`git diff --numstat cba23389` is byte-identical to the r2 measurement — so not one security signal was
removed. Nothing in this iteration could downgrade the verdict, and the parallel 5-auditor sweep should
run to completion.

Recommended auditor focus is **widened** by this iteration: (a) the F1 index-vs-body asymmetry across
**all** guards derived from `ensure_blocks_present`; (b) `DEV-I156-M2-001` confirms a second consumer
already mis-assumes body density — `has_contiguous_bodies` backs the checkpoint guardian's healthy
tag, so **auditors should enumerate every caller that treats an index check as a body check**, not
just the two in this diff; (c) adversarial reachability of `target_height == 0` in `execute_reorg`,
now sharper given [F6]: `plan_reorg`'s finality guard is inert while `last_finality_height` is `None`,
and genesis is an explicitly admitted common ancestor.

---

## Final Verdict

**APPROVED.** The code is correct, minimal, unchanged this iteration, and strictly reduces harm. All
three r2 blockers are closed: the overclaim is replaced with an accurate and well-cited SCOPE/RESIDUAL
note, the spec projection is qualified and internally consistent across `:2761`/`:2793`/`:2796`, and
the INV-12 exemption now rests on premises that describe the code. Every citation the developer
touched resolves exactly. Scope discipline held — he correctly declined to widen the guard.

**Two mandatory pre-commit text corrections** (neither requires re-test or re-review):

1. **F6** — `crates/network/src/sync/reorg/mod.rs:494`: qualify INV-12 Hunk B point 4 to "once the
   node has finalized at least one block", and state that points 1-3 carry the exemption
   independently. Do not commit the block as written.
2. **F5** — `docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md:393-395`: qualify the surviving
   "impossible by construction at all four call sites" overclaim to the height-index class, and add a
   §7 bullet for the body-density guard variant so the `rewards.rs:1147` pointer resolves.

Then: `AUDIT-REQUIRED` → complete the in-flight 5-auditor sweep → commit.

Follow-ups to file as their own work (do **not** fold into M2): the body-density residual (F1, prefer
the REQ-I156-012 scratch-set shape, which closes both sub-classes); `DEV-I156-M2-001`
(`has_contiguous_bodies` index-vs-body, plus an enumeration of every `ensure_blocks_present` /
`has_contiguous_bodies` caller that assumes body density); the `gate_flag_protection` matcher false
positive (F4); the `inc_i_156_m2_rebuild_guard.rs` split (F3, bundle with the F1 body-gap test — and
note it is mechanical, since `inc_i_156_m1_harness` is already imported by all three files);
`AUDIT-P2-105` (stays banked, `open`).

> **Deferral premise correction (pre-commit pass, AUDIT-P3-217 / F3).** The "it is mechanical"
> premise above is **false** and the split stays deferred on the corrected premise: the shared import
> of `inc_i_156_m1_harness` does not cover the helpers the split would actually have to move. Each M2
> file defines its **own** local `punch_hole` with **divergent arity** — `punch_hole(&Node, u64)` in
> `inc_i_156_m2_dense_reconstruction.rs:237` and `inc_i_156_m2_reorg_range_parity.rs:171` vs
> `punch_hole(&Node, u64, u64)` in `inc_i_156_m2_rebuild_guard.rs:269` (and
> `inc_i_152_p1_003_rollback_holed_store.rs:572`). Splitting `rebuild_guard.rs` therefore requires
> either duplicating the 3-arg `punch_hole` plus `build_node` / `assert_hole_precondition` and the
> `HOLE_LOW` / `HOLE_HIGH` / `TARGET_HEIGHT` constants into the new file, or hoisting one arity into
> the shared harness where it collides with the 2-arg variants. Neither is quick or mechanical, so it
> is not done in a text-correction pass. The file is **862 lines** after the AUDIT-P2-206 assertion
> added here (was 846), i.e. still over the 800-line test budget — carried as a known, recorded
> exception to be discharged by the F1 body-gap bundle, which has to touch these helpers anyway.
