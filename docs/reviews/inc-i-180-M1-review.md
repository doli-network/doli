━━━ FINDINGS — 8 total (Major:3 Minor:5) ━━━

  [F1] MAJOR conf(0.85, observed) — bins/node/src/node/production/assembly.rs:242-291 — the gate adds three block-invalidity rules with no block-builder counterpart, turning a mempool-admissible RequestWithdrawal into a free, infinitely repeatable block-poison + rollback trigger post-AH
  [F2] MAJOR conf(0.72, inferred) — bins/node/src/node/validation_checks.rs:621 — the two ADMISSION-only rules are enforced with equal strictness in ValidationMode::Replay, contradicting INC-I-064's replay tolerance (tx_processing.rs:116-131) and the milestone's own "already canonical" reasoning (rewards.rs diff)
  [F3] MAJOR conf(0.99, observed) — bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs:655-686 — OBS-R3-002 confirmed by offset measurement: the positional guard cannot fail on the regression it exists to catch; BLOCKING, 2-line fix
  [F4] MINOR conf(0.95, observed) — bins/node/tests/it/inc_i_180_common.rs:128,156,356 — OBS-R3-001 confirmed: the ownership fixture hardcodes the same derivation the gate evaluates, so it cannot detect gate/builder drift
  [F5] MINOR conf(0.90, observed) — crates/storage/src/producer/info.rs:312-317 — the allowance counts pending_addbond_count() that add_bonds() may clip (!is_active, MAX_BONDS_PER_PRODUCER); direction is conservative, record as a residual (observation, no fix proposed)
  [F6] MINOR conf(0.85, observed) — bins/node/src/node/apply_block/tx_processing.rs:395-412 — post-AH the INC-I-058 auto-revoke trigger point moves silently and the milestone suite has zero delegation coverage
  [F7] MINOR conf(0.99, observed) — bins/node/tests/it/, crates/core/tests/it/, crates/storage/tests/it/ — the milestone's ENTIRE regression suite is untracked and Cargo-auto-discovered, so a commit that omits `git add` ships REQ-I180-001/002/003 with no in-tree coverage and nothing fails
  [F8] MINOR conf(0.90, observed) — bins/node/src/node/validation_checks.rs:627-671 — two lock acquisitions + two HashMap allocations on EVERY post-AH block regardless of content; PM-180-01's registry scale_assumptions understates this as "the existing producer_set read lock"

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-180 M1 — consensus withdrawal-holdings gate

**Run:** 525 · **Incident:** INC-I-180 · **Milestone:** M1 · **Date:** 2026-08-20
**Branch:** `bugfix/inc-i-180-withdrawal-holdings-gate` (uncommitted working tree)
**Reviewer input:** `docs/.workflow/inc-i-180-M1-brief.md` (F1..F7 binding), `docs/qa/inc-i-180-M1-qa-report.md` (round 3, PASS)

## Summary

**REQUIRES CHANGES** — one blocking item (F3, ~2 lines, test-only) plus two Major findings that
are inert on mainnet (`u64::MAX`) but arm on testnet the moment the fleet passes `230_000`.

The fix is **not** superficial. It closes the defect class at the correct layer, in the correct
direction, with the correct gate. The developer moved the decision from a `()`-returning
post-mutation pass into a `Result`-returning pre-mutation one, bound the declared count to the
*named producer's own* Bond UTXOs, charged the allowance for in-block `Exit`s, and mirrored the
allowance in both the live enqueue and the reorg/rollback replay. Pre-activation bit-identity holds
by construction (the whole block is skipped at `validation_checks.rs:621`, and `in_flight` is forced
to `0` at `tx_processing.rs:385-393` and `rewards.rs:1379-1384`). No version bump, no Cargo.toml
change, no existing activation height moved, reused or bundled, flush order untouched.

What the milestone did not do is trace its own new `bail!`s outward. Three new ways to make a block
INVALID were added to a function the block builder does not consult and to a `ValidationMode` the
codebase deliberately made tolerant. Both surfaces are documented in the repo — one of them ten
lines above the builder's own tx filter, citing the incident it caused. That is F1 and F2.

## Scope Reviewed

| Area | Files |
|---|---|
| The gate | `bins/node/src/node/validation_checks.rs:599-793` (196 insertions, 0 deletions) |
| Apply parity | `bins/node/src/node/apply_block/tx_processing.rs:377-447`, `:263-297` (Exit arm), `:315-373` (AddBond arm) |
| Replay parity | `bins/node/src/node/rewards.rs:1355-1400` |
| Activation height | `crates/core/src/network_params/{mod.rs,defaults.rs,env_loader.rs}` |
| Consumed invariants | `crates/storage/src/producer/{set_core.rs,info.rs}` (`pending_addbond_count`, `apply_pending_updates_with_cap`, `add_bonds`, `apply_withdrawal`) |
| Blast radius outward | `bins/node/src/node/apply_block/mod.rs:110-198`, `bins/node/src/node/production/{mod.rs,assembly.rs}`, `bins/node/src/operations/chain.rs`, `crates/core/src/validation/{tx_types.rs,utxo.rs}`, `crates/mempool/src/pool.rs` |
| Specs/docs | `specs/protocol.md:597-652`, `docs/error-codes.md:37-39`, `docs/bugfixes/inc-i-180-n11-zero-bond-active-set-analysis.md:360-403` |
| Tests | `bins/node/tests/it/` (4 modules), `crates/core/tests/it/`, `crates/storage/tests/it/` |

Not re-run: the workspace test suite, clippy, fmt. QA executed them this round and I accepted the
stated results per the review brief.

---

## 1. Root-cause completeness — does this close the defect class?

**Yes, at the layer where the class lives.** The defect was structural: two ledger effects with
independent success conditions, the destructive one running first and unconditionally
(`apply_block/mod.rs:202` before `:216`), the compensating one unable to fail
(`process_transaction_producer_effects` returns `()`). Any fix inside the second pass would have
been a symptom patch. The gate is at `apply_block/mod.rs:113`, before the tx loop opens at `:196`,
so a rejection produces no mutation at all — the two effects now genuinely succeed or fail together.

**The three post-AH rules are jointly sufficient for REQ-I180-001**, and each closes a distinct
bypass of the other two:

- Allowance alone bounds the declared count from ABOVE only — an under-declared request still
  destroys every Bond input (`process_transaction_utxos` spends inputs, not `bond_count`). Closed by
  the count binding at `validation_checks.rs:776-787`.
- Count binding alone is owner-agnostic — spend A's bonds, name B, and A keeps unbacked weight at
  zero cost. Closed by the owner predicate at `:645-666`
  (`e.output.pubkey_hash == hash_with_domain(ADDRESS_DOMAIN, wd.producer_pubkey)`).
- Both together still miss the `[Exit(p), RequestWithdrawal(p, n)]` shape, because apply bumps
  `withdrawal_pending_count += bond_count` for an Exit immediately (`tx_processing.rs:271`) while an
  Exit carries no inputs and no outputs. Closed by the Exit charge at `:685-706`.

I verified the two parity claims that the sufficiency argument rests on, rather than accepting them:

- **AddBond count parity.** Validation counts `tx.outputs.filter(Bond).count()`
  (`validation_checks.rs:676-681`); apply queues `outpoints` built by the identical filter
  (`tx_processing.rs:337-343`); `pending_addbond_count` sums `outpoints.len()`
  (`set_core.rs:214-226`). Same quantity, three sites. Apply additionally *skips* the enqueue for an
  unregistered producer with no pending Register (`tx_processing.rs:319-330`), which validation does
  not model — but a withdrawal naming such a producer is rejected first by
  `ECON_WITHDRAWAL_UNKNOWN_PRODUCER`, so the divergence is unreachable.
- **Exit charge parity.** Validation charges `get_by_pubkey(pk).bond_count` per Exit and accumulates
  with `saturating_add`; apply reads the same unchanged `bond_count` (`:266`) and accumulates with
  `+=` (`:271`). `bond_count` cannot move mid-block — only `withdrawal_pending_count` does — so the
  deliberately "untidy" double-charge for two Exits is correct parity, not a bug.

**Nothing in the milestone's own defect class is still reachable post-AH.** The failure mode
"Bond UTXO spent, weight kept" requires the enqueue at `tx_processing.rs:442` to be skipped, i.e.
`bond_count > remaining`. Post-AH `remaining` is computed from exactly the terms the gate bounded,
and `delegated_bonds` — the only other subtrahend — is applied to `available`, not to `remaining`,
so it cannot suppress the enqueue. The residual paths I traced all fail in the *safe* direction:
`apply_withdrawal` saturates at `count.min(bond_entries.len())` (`info.rs:489`) and auto-exits at
`bond_count == 0` (`:518-520`), so an over-queued withdrawal drains weight to zero rather than
leaving it unbacked. See F5.

## 2. Unintended behaviour changes

| Check | Verdict | Evidence |
|---|---|---|
| Pre-AH bit-identity (INV-CONSENSUS-002) | **Holds** | Whole gate skipped at `validation_checks.rs:621`; `in_flight` forced to `0` at `tx_processing.rs:385-393` and `rewards.rs:1379-1384`. Independently: `git diff --numstat` on `validation_checks.rs` is `196 0` — no canonical line altered |
| No early return above the gate | **Confirmed** | `sed -n '476,600p' | grep return` returns nothing; only `bail!`s (rejections) sit above it. The gate is unreachable-past in every mode |
| Version bump of any kind | **None** | `git diff -U0 | grep -E '^\+.*(CURRENT_PROTOCOL_VERSION|EPOCH_STATE_FORMAT_VERSION|MIN_PEER_PROTOCOL_VERSION)'` matches ONE line — prose in `specs/protocol.md:754` stating no bump was needed. `git diff --name-only | grep -c Cargo.toml` = `0` |
| AH moved / reused / bundled | **No** | `defaults.rs` diff is 3 insertions, 0 deletions, one dedicated field. `mod.rs` deletions are stale doc comments on `delegation_auth_*` and `addbond_cap_*` corrected to match `defaults.rs` — pre-existing drift fixed in passing, no value changed |
| `env_loader` mainnet lock | **Correct** | `env_loader.rs:369-376` locks mainnet to the compiled default, matching the `addbond_cap_*` pattern immediately above it |
| Flush order | **Untouched** | `crates/storage/src/` carries no modification in `git status --short` |
| New technical debt | **None** | `git diff -U0 -- bins/node/src crates/core/src | grep -E '^\+.*(unwrap\(\)|unsafe|TODO|FIXME|HACK|XXX|panic!|expect\()'` returns nothing. `u32::try_from(n).unwrap_or(u32::MAX)` is saturating, not a panic |
| Injection-pattern scan (mandatory) | **Clean** | Rust, no SQL/shell/eval surface in the diff. No f-string/format/concat-into-query analogue exists |
| Out-of-scope semantic change | **None** | `CLAUDE.md` and `specs/maintainer-authorization-architecture.md` are modified in the tree but `git diff` on both contains zero occurrences of "180" — pre-existing unrelated work |
| Lock discipline | **Correct** | Gate takes `utxo.read()` scoped to the block expression `:627-670`, drops it, then `producer_set.read()` at `:671`. Apply is utxo→producers (`mod.rs:197-198`), rollback is producers→utxo (`rollback.rs:325`); never co-held, so no cycle |

One gated semantic change went unremarked by both developer and QA — see **F6** (INC-I-058
auto-revoke). One error-precedence change is correctly identified by QA as OBS-R3-004 and is not a
verdict change: the gate is at `:599`, the EpochReward section at `:795`, so a block violating both
now reports the withdrawal code. Accept/reject is unchanged on every input.

## 3. Specs/docs accuracy

Verified line by line against the code, not against the QA report.

| File | Verdict |
|---|---|
| `specs/protocol.md:600-652` | **Accurate.** The allowance formula, the double-Exit charge rationale, the `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey)` derivation, the pre-block UTXO qualifier, the position argument (gate above the EpochReward section, INC-I-080 cap deliberately below the early return), and the three AH values all match the implementation exactly. It even states which two rules the replay path deliberately does NOT mirror — which is the reasoning F2 finds is applied inconsistently, but the spec's *description of the code* is correct |
| `docs/error-codes.md:37-39` | **Accurate.** Three codes, each matching a `bail!` string at `validation_checks.rs:729-735`, `:748-759`, `:778-787`. The `BOND_COUNT_MISMATCH` row correctly states "owned by the named producer" with the derivation |
| `crates/core/src/network_params/mod.rs:327-360` | **Accurate**, and the two doc-comment deletions correct pre-existing drift (`delegation_auth_*` and `addbond_cap_*` doc defaults now match `defaults.rs:150,423,676`) |
| `docs/bugfixes/inc-i-180-n11-zero-bond-active-set-analysis.md:373-403` | **Accurate.** I spot-checked 7 of the matrix's test names against the three `tests/it/` trees; all 7 resolve to a real `fn`. REQ-I180-001's third acceptance criterion (`status=exited, selection_weight=0` after the boundary) is genuinely asserted — `inc_i_180_withdrawal_holdings_gate.rs:333-337` and `crates/storage/tests/it/inc_i_180_withdrawal_holdings.rs:230-240` |
| Contradiction check | **None found.** No document claims X and then builds on not-X |

Note for the incident, not a review finding: REQ-I180-004 ("the shortfall MUST be observable as an
error, not only a WARN") is a **Must** and the diff leaves `tx_processing.rs:414-428` a `warn!` with
no metric. The matrix correctly marks 004..012 as out of M1 scope, so this is tracked, not dropped.

## 4. Did it go too far, or not far enough?

**Not too far.** The milestone did not silently expand into mempool admission or builder parity.
`crates/mempool/` carries zero modification; `production/assembly.rs` carries zero modification.
Every changed line is inside the three files the brief names plus the params crate.

**Not far enough — and the seam is load-bearing, not cosmetic.** The brief classifies
mempool/builder parity as an INV-VALIDATION-001 residual, "C8", deferred to M2 because "the mempool
and builder contexts carry no bond fields today." That framing describes a missing feature. What the
diff actually creates is a new *attacker-reachable* asymmetry: a transaction class that the mempool
admits and the block validator rejects. That is F1, and the M1 boundary does not leave it clean — it
leaves it armed at testnet height 230_000.

---

## Critical / Major Findings

### F1 — MAJOR: the gate has no block-builder counterpart, making it a free, repeatable block-poison trigger post-AH

- **Location:** `bins/node/src/node/production/assembly.rs:242-291`; `bins/node/src/node/production/mod.rs:619-668`; `crates/core/src/validation/tx_types.rs:548-607`; `crates/mempool/src/pool.rs:1127-1130`
- **Severity:** Major
- **Confidence:** `conf(0.85, observed)` — static trace of all four sites; not executed against a live post-AH node
- **Evidence:**
  - The builder's entire per-tx filter is `validation::validate_transaction_with_utxos` plus the
    NFT/Pool unique-id checks (`assembly.rs:242-291`). It holds `utxo` but never reads
    `producer_set`, so none of the three new rules is consultable there.
  - Ten lines above that filter, `assembly.rs:250-254`: *"validate_transaction_with_utxos doesn't
    check this — the check only existed in apply_block, causing a fatal mismatch where the builder
    includes a TX that apply_block rejects, freezing the chain. See: testnet incident 2026-03-25."*
    The milestone reintroduces exactly that class and the warning sits in the file it did not touch.
  - Structural validation for a RequestWithdrawal requires only `!tx.inputs.is_empty()`
    (`tx_types.rs:550-554`) — **not** that the inputs are `Bond`-typed. Its own doc comment at `:547`
    says "Bond UTXO ownership, producer bond holdings … done at node level." An ordinary signed
    `Normal` UTXO satisfies it.
  - The mempool's only RequestWithdrawal-specific behaviour is *relaxing* the lock check
    (`pool.rs:1127-1130`). There is no holdings check to relax.
  - `production/mod.rs:619` self-applies the built block; on `Err` it logs `[BLOCK_POISON]`, calls
    `rollback_one_block()` (`:629`), purges every tx of the failed block from the mempool
    (`:648-663`) and returns `Ok(())` (`:667`) — the block is discarded.
- **Impact:** post-AH, any user can submit a structurally valid, signature-valid,
  mempool-admissible `RequestWithdrawal` that names an unregistered producer, or declares a
  `bond_count` that does not match its owned Bond inputs, using ordinary `Normal` UTXOs as inputs.
  The transaction **never confirms**, so the attacker never pays a fee and never spends the input —
  the attack is free and infinitely repeatable. Every producer that selects it burns a block build
  and executes `rollback_one_block()`, the heaviest and historically most fragile path in the node
  (INC-I-156 class), on unauthenticated demand. The tx survives in every other node's mempool and
  re-propagates. This is a *new* surface: pre-AH the same transaction is admitted and mined, costing
  the attacker an input, one-shot.
- **Reachability:** mainnet `u64::MAX` — inert. Testnet `230_000` against a measured tip of
  `216_453` — roughly 13,500 blocks, about 1.6 days at 10 s slots.
- **Suggested fix:** make M2 a hard prerequisite of the testnet activation, not a follow-up
  milestone. Minimum M2 content: replicate the three predicates in `assembly.rs` tx selection as a
  **skip** (`continue`, like the unique-id checks at `:255-291`), never a build failure; and add the
  same predicates to mempool admission and `revalidate` so the tx is evicted rather than re-offered.
  If M2 cannot land before the testnet fleet approaches `230_000`, re-pin the testnet height — it has
  **not** been crossed, so moving it forward is legal under INV-PARAMS-001, which forbids only moving
  a *crossed* height. That is a decision for the user, not a unilateral edit.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(withdrawal_txs × inputs) per block build (observed — same predicate the gate already runs at validation_checks.rs:627-670, executed once more in the builder)
  Memory:   +constant (observed — one producer_set read guard + one HashMap per build, mirroring the gate)
  IO:       0 (observed — both reads are in-memory; no RocksDB access added)
  Network:  -unbounded (inferred — removes the re-gossip/re-selection loop of a permanently unminable tx across the whole fleet)
  Disk:     -per-poisoned-block (observed — each avoided BLOCK_POISON avoids a rollback_one_block undo read plus its WriteBatch)
  Latency:  +sub-ms per build; -one full slot per avoided poison (inferred — block-build deadline at assembly.rs:235 is seconds, the predicate is microseconds)
Inevitability: AVOIDABLE
Cheaper alternative: re-pin the testnet activation height beyond M2's landing date and ship M1 alone
Why this proposal anyway: the cheaper path only defers the exposure — the height must eventually be crossed, and until parity exists the chain carries a zero-cost, unauthenticated, repeatable trigger for rollback_one_block on every producer
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F2 — MAJOR: the two admission-only rules are enforced strictly in `ValidationMode::Replay`, contradicting INC-I-064 and the milestone's own reasoning

- **Location:** `bins/node/src/node/validation_checks.rs:621` (gate condition — `mode` appears nowhere in the gate body); `bins/node/src/node/apply_block/tx_processing.rs:116-131`; `bins/node/src/operations/chain.rs:344-356`
- **Severity:** Major
- **Confidence:** `conf(0.72, inferred)` — the contradiction is proven textually; the *reachability* of a degraded replay UTXO view past the AH is inferred from the INC-I-064 comment, not executed. QA explicitly lists live post-AH execution as unvalidated
- **Evidence:**
  - `tx_processing.rs:116-131`: per-tx UTXO validation failure is **tolerated** in Replay —
    *"INC-I-064: Replay mode tolerates UTXO validation failures. Historical blocks (e.g., E362) have
    EpochReward inputs referencing already-consumed pool UTXOs that no longer exist during replay."*
    So the codebase asserts, from a mainnet incident, that the replayed UTXO view can legitimately
    disagree with history.
  - The gate's count binding is built on exactly that view (`validation_checks.rs:645-666`,
    `utxo.get(...)`), and `spend_transaction` **removes** entries (`in_memory.rs:163-168`), so an
    unresolvable input counts `0` and the block is rejected with
    `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` — a hard `bail!` with no Replay carve-out.
  - The milestone applies the opposite reasoning one file over. `rewards.rs` diff:
    *"The other two INC-I-180 rules are deliberately NOT mirrored here … this replay reads blocks
    that are already canonical."* The identical argument is not applied to `ValidationMode::Replay`
    in `validate_block_economics`, which serves the same purpose.
  - Blast radius is bounded and I measured it: the only non-test caller passing `Replay` is
    `operations/chain.rs:354`, the operator `recover`/reindex tool, which wipes `state_db`
    (`:320-323`) and re-applies every block `1..=tip_height`, aborting the whole recovery on the
    first `Err` (`:356`). Gossip-received blocks never use `Replay`.
- **Impact:** on a post-AH chain, the last-resort repair tool gains a hard-abort mode at the exact
  class of block INC-I-064 made tolerable. It also applies two *admission* rules —
  `ECON_WITHDRAWAL_UNKNOWN_PRODUCER` and the Bond-input binding — to blocks that are already
  canonical, which is a category error the milestone itself identified and avoided elsewhere.
- **Suggested fix:** give the two admission-only rules the same Replay carve-out the UTXO layer has
  (`warn!` + continue when `mode == ValidationMode::Replay`), keeping the **allowance** rule strict
  in all three modes. This does not re-open ISSUE-005: that was a Full-vs-Light divergence between
  two *admission* modes; Replay is not an admission mode and, per the grep above, is unreachable
  from the network path. Add one test replaying a post-AH block whose Bond inputs do not resolve.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — adds one enum comparison on an already-taken branch)
  Memory:   0 (observed — no allocation change)
  IO:       0 (observed)
  Network:  0 (observed — Replay is never reached from the network path)
  Disk:     0 (observed)
  Latency:  0 (observed — offline operator tool, no SLO)
Inevitability: AVOIDABLE
Cheaper alternative: document the strictness as intentional and add a post-AH replay test proving the recovery tool survives a tolerated UTXO failure
Why this proposal anyway: the cheaper path leaves the recovery tool able to abort on a chain it is the only remedy for, and leaves the milestone applying two contradictory rules to the same "already canonical" argument
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F3 — MAJOR (BLOCKING): OBS-R3-002 — the positional guard cannot fail on the regression it exists to catch

- **Location:** `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs:655-686`
- **Severity:** Major — **blocking**
- **Confidence:** `conf(0.99, observed)` — measured directly against the source
- **Evidence:** `grep -n` on `bins/node/src/node/validation_checks.rs` gives:
  - `:602` — `[INC_I_081_MISSING_CHECK_SKIP]` inside the **new INC-I-180 comment**
  - `:1030` — the real early return's log marker
  - `:1046` — `=== INC-I-080: per-producer AddBond cap`

  The test computes `SRC.find("[INC_I_081_MISSING_CHECK_SKIP]")`, and `find` returns the **first**
  occurrence — the byte offset of line 602. The assertion therefore evaluates
  `cap_block(1046) > early_return(602)`, which is trivially true and **stays true if the real
  `return Ok(())` is moved anywhere below the cap** — the exact regression the guard exists to catch.
  The milestone's own hoist created the second occurrence, so the guard was already dead on delivery.

- **Explicit verdict — BLOCKING, not an acceptable M2 non-blocker.** Three reasons, and none of them
  is severity of the underlying risk:
  1. **The fix is two lines.** There is no engineering argument for deferring a two-line test edit to
     a future milestone; deferral costs more in tracking than in doing.
  2. **It is the ONLY executable lock on the constraint.** QA established that devnet pins
     `addbond_cap_enforcement_activation_height` to `u64::MAX` (`defaults.rs:676`), so no behavioural
     fixture on `Node::new_for_test` can observe the cap at all. If this guard cannot fail, the
     constraint has zero coverage.
  3. **A guard that cannot fail is worse than no guard.** It converts "uncovered, and everyone knows
     it" into "covered, green, and wrong" on a constraint whose violation is a *retroactive consensus
     change on a live mainnet chain* (the cap is enforced from height 0 on mainnet and testnet). The
     next engineer who moves that return will see a passing suite. The milestone's own hoist is proof
     that the code around that return moves.
- **Suggested fix:**
  1. Replace `.find(` with `.rfind(` for the early-return marker.
  2. Add a positive anchor so a third occurrence cannot recreate the bug — assert that the located
     marker is followed by `return Ok(());` within a small byte window, or assert
     `early_return > gate_block` so a match inside the INC-I-180 comment fails loudly rather than
     passing silently.
- **Test strategy for the fix:** temporarily reorder the two blocks in a scratch copy of the source
  string and assert the guard fails. A guard that has never been observed failing has not been tested.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — compile-time `include_str!` scan in a test binary only)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (observed — test-only change, no runtime code touched)
  Disk:     0 (observed)
  Latency:  0 (observed — `rfind` over a 1,506-line file, sub-millisecond, test-only)
Inevitability: AVOIDABLE
Cheaper alternative: delete the guard and record the constraint as uncovered
Why this proposal anyway: deleting it is honest but loses the only available lock on a constraint whose violation is a retroactive consensus change on mainnet; two lines restore it
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Minor Findings

### F4 — MINOR: OBS-R3-001 confirmed — the ownership fixture is circular

- **Location:** `bins/node/tests/it/inc_i_180_common.rs:128`, `:156`, `:356`
- **Confidence:** `conf(0.95, observed)`
- **Evidence:** all three Bond-output constructors hardcode
  `crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes())` — the same expression
  the gate evaluates at `validation_checks.rs:646-649`. `req_i180_001_post_ah_same_owner_bond_inputs_are_accepted`
  (`inc_i_180_gate_bindings.rs`) therefore proves only that the gate agrees with the fixture.
  I confirm QA's reasoning and its resolution: R3-OWN1/R3-OWN2 severed the circularity **this round**,
  against `Transaction::new_add_bond` and `new_registration` — but those probes were removed.
- **Impact:** if the production Bond `pubkey_hash` derivation ever changes, the fixture changes with
  it only if someone remembers, and the gate silently rejects every post-AH withdrawal — a liveness
  break, the worst outcome available to this milestone.
- **Suggested fix:** port R3-OWN1/R3-OWN2 into the deliverable suite. Build the bonds with the
  production constructors and seed the resulting `Output` values verbatim at the transaction's real
  hash, so nothing in the test re-derives a `pubkey_hash`. Pair it with F3 — one file, one pass.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — two additional test cases, no runtime code touched)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (observed — test-only)
  Disk:     0 (observed)
  Latency:  +milliseconds of test-suite wall time (observed — two fixtures on an existing 34-test target)
Inevitability: AVOIDABLE
Cheaper alternative: keep the circular fixture and rely on the round-3 QA probe as a one-time proof
Why this proposal anyway: a one-time proof in a deleted probe does not survive the next refactor; the derivation is the single point at which this fix becomes a liveness break
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F5 — MINOR (observation, no fix proposed): the allowance counts pending AddBonds that the flush may not fully land

- **Location:** `crates/storage/src/producer/info.rs:312-317`, consumed at
  `bins/node/src/node/validation_checks.rs:739`
- **Confidence:** `conf(0.90, observed)`
- **Evidence:** `add_bonds` returns `0` outright when `!self.is_active()` (`:312-314`) and otherwise
  clips to `bonds_to_add = min(outpoints.len(), MAX_BONDS_PER_PRODUCER - bond_count)` (`:316-317`),
  whereas `pending_addbond_count` returns the unclipped `outpoints.len()` (`set_core.rs:214-226`).
  The gate's allowance therefore over-counts what the epoch flush will actually add. On mainnet and
  testnet `addbond_cap_enforcement_activation_height` is `0` (`defaults.rs:150,423`), so over-cap
  AddBonds are rejected at validation and the clip is unreachable; on **devnet** the cap is
  `u64::MAX` (`:676`) while this gate is live from height `20`, so the clip path is live there.
- **Why this is not a defect:** the direction is conservative. `apply_withdrawal` saturates at
  `count.min(bond_entries.len())` (`info.rs:489`) and auto-exits at `bond_count == 0` (`:518-520`),
  so the outcome is at most an orphaned Bond UTXO (INC-I-085 class) and never unbacked weight — the
  inverse of the defect under repair. The developer already documented the exact counterfactual in
  `crates/storage/tests/it/inc_i_180_withdrawal_holdings.rs:259-262`.
- **No fix recommended.** Record as a known residual on PM-180-01 so the devnet-only asymmetry is
  visible to whoever eventually pins the mainnet height.

### F6 — MINOR: post-AH the INC-I-058 auto-revoke trigger point moves, with zero delegation coverage

- **Location:** `bins/node/src/node/apply_block/tx_processing.rs:395-412`
- **Confidence:** `conf(0.85, observed)`
- **Evidence:** the diff redefines `remaining` as `held + in_flight - withdrawal_pending`. The
  auto-revoke branch condition is unchanged text — `delegated > 0 && data.bond_count == remaining`
  (`:397`) — but `remaining` is now a different number post-AH. A "full exit" withdrawal computed
  against the pre-AH `remaining` no longer satisfies the equality, so the auto-revoke does **not**
  fire on inputs where it previously did. `grep -rl "delegat" bins/node/tests/it/ crates/storage/tests/it/`
  returns no files: the milestone's suite has no delegation coverage at all.
- **Impact:** bounded and self-healing — the withdrawal still enqueues (`bond_count <= remaining`),
  and when the flush drains the producer to zero, `set_core.rs` calls `cleanup_all_delegations` on
  the `Exited` status. The delegation simply survives one epoch longer than it used to. It is a
  gated behaviour change nobody noted, on a code path with no test.
- **Suggested fix:** one post-AH test with `delegated_bonds > 0` and a pending AddBond, asserting
  the enqueue fires and the delegation end-state after the boundary.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one test case, no runtime code changed by the fix)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (observed — test-only)
  Disk:     0 (observed)
  Latency:  +milliseconds of test-suite wall time (observed)
Inevitability: AVOIDABLE
Cheaper alternative: document the shifted trigger point in the tx_processing.rs comment and leave it untested
Why this proposal anyway: INC-I-058 exists because this exact branch failed before; a gated change to its trigger with zero coverage is how it fails again
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F7 — MINOR: the milestone's entire regression suite is untracked

- **Location:** `bins/node/tests/it/`, `crates/core/tests/it/`, `crates/storage/tests/it/`
- **Confidence:** `conf(0.99, observed)`
- **Evidence:** `git status --short` lists all three as `??`. `git diff --name-only | grep -c Cargo.toml`
  is `0`, so the targets exist only through Cargo's directory auto-discovery of `tests/<dir>/main.rs` —
  there is no manifest entry whose absence would break the build.
- **Impact:** a commit that stages only the modified files ships REQ-I180-001/002/003 with **zero**
  in-tree regression coverage, and nothing fails to reveal it: the workspace builds, clippy passes,
  and the 34+5+9 tests simply cease to exist. Given F3 and F4, the suite is also where the two
  outstanding fixes land.
- **Suggested fix:** `git add` the three directories explicitly, and verify with `git show --stat`
  after committing that `bins/node/tests/it/main.rs`, `crates/core/tests/it/main.rs` and
  `crates/storage/tests/it/main.rs` are present in the commit.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +test-suite compile time for 3 new targets (observed — already paid locally; the change is only that CI pays it too)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~120 KB of source in the repository (observed — `ls -la` on the three directories)
  Latency:  0 (observed — no runtime path affected)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: tests that are not committed do not exist; there is no cheaper way to have the regression suite in the repository
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F8 — MINOR: unconditional lock acquisitions and allocations per post-AH block; PM-180-01's registry entry understates them

- **Location:** `bins/node/src/node/validation_checks.rs:627-671`; registry row `PM-180-01`
- **Confidence:** `conf(0.90, observed)`
- **Evidence:** post-AH, **every** block — including the ~359 of every 360 that mutate no producer
  state — takes `self.utxo_set.read().await` (`:628`), builds a `HashMap` from an iterator over all
  transactions (`:627-670`), drops the guard, takes `self.producer_set.read().await` (`:671`), and
  allocates two more `HashMap`s (`:672-675`) before the match loop finds nothing to do. The 359/360
  figure is the project's own measurement, recorded at `apply_block/mod.rs:157-161` (INC-I-071).
  PM-180-01's `scale_assumptions` says *"O(txs-per-block) hash-map lookups under the existing
  producer_set read lock"* — but neither guard existed in `validate_block_economics` before this
  diff, and the utxo guard is not "existing" in that function at all.
- **Impact:** small in absolute terms at 10 s slots, but it is a per-block cost on the block-validation
  hot path introduced with an upstream cost statement that understates it, which is itself the signal
  the resource-cost protocol asks reviewers to flag. The utxo read guard also briefly contends with
  the `atomic_replace` writers on the rollback/snap paths.
- **Suggested fix:** guard the whole block with a cheap pre-scan —
  `if !block.transactions.iter().any(|t| matches!(t.tx_type, TxType::RequestWithdrawal | TxType::Exit | TxType::AddBond)) { skip }`
  — which is one pass over an already-in-cache slice and eliminates both guards and all three
  allocations on the overwhelming majority of blocks. It cannot change any verdict: with none of
  those three tx types present, the match loop has no reachable arm. Separately, correct
  PM-180-01's `scale_assumptions` to state two new read-guard acquisitions and three HashMap
  allocations per post-AH block.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -one HashMap build + -two HashMap allocations on ~359/360 blocks; +O(txs) enum comparisons on all blocks (observed — measurement source apply_block/mod.rs:157-161)
  Memory:   -3 HashMap allocations per non-producer block (observed — validation_checks.rs:627, :672, :674)
  IO:       0 (observed — both sets are in-memory; no RocksDB access in the gate)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  -two RwLock read acquisitions per non-producer block on the block-validation path (observed — validation_checks.rs:628, :671)
Inevitability: AVOIDABLE
Cheaper alternative: leave it — at 10 s slots the absolute cost is small
Why this proposal anyway: the cheaper path also leaves the registry's scale assumption wrong, and PM-180-01 is the row a future engineer will read when pinning the mainnet height on a chain with far more transactions per block than testnet
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## System Impact — protection-mechanism interaction review

`PM-180-01` is registered and active (`SELECT ... FROM protection_mechanisms WHERE status='active'`
returns 37 rows; `PM-180-01` carries trigger, action, scale assumptions and an `interacts_with`
entry). Its trigger surface is *block validation reading the ProducerSet for a named producer*.
Enumerating every active mechanism that shares any part of that surface:

| Mechanism | Shares surface? | Both fire on one event? | One creates the other's trigger? | One starves the other? |
|---|---|---|---|---|
| **INC-I-080 AddBond cap** (`validation_checks.rs:1046`, same fn, same producer entry, same lock, same pass) | **Yes — fully** | **Yes.** A block with an over-cap AddBond *and* an over-allowance withdrawal for the same producer trips both. PM-180-01 runs first (`:599` vs `:1046`), so it reports; accept/reject is identical either way (QA OBS-R3-004) | **No.** Both actions are "reject the block before any mutation" (`apply_block/mod.rs:113`, before the tx loop at `:196`). A rejection produces no state change, so neither action can construct the other's trigger. Structurally impossible, not merely unobserved | **No — and the direction is protective.** Both read `pending_addbond_count(pk)`. The cap *bounds* the term PM-180-01 adds to the allowance, so it can only tighten PM-180-01, never inflate it. PM-180-01 cannot be starved: a producer can always withdraw `bond_count - withdrawal_pending`, which requires no AddBond at all. No livelock, no mutual disarm |
| **PM-013 exclusion caps** (`max_excluded_total = active/3`) — listed in PM-180-01's `interacts_with` | **Yes, indirectly** | No — different pass | **Yes, weakly, and it is the diff's *intent*.** The apply-parity half of this diff makes full-exit withdrawals land where they previously silently did not, so affected producers now correctly leave the active set. `active` shrinks ⇒ PM-013's cap tightens. Only via *correct* exits, and inert on mainnet at `u64::MAX` | No. `MIN_PRODUCERS_FLOOR = 3` bounds the shrink; on a 12–20 node testnet this is worth watching after the gate activates but cannot deadlock |
| **PM-025 rebuild-precondition dense guard** (`rewards.rs`, `rebuild_producer_set_from_blocks`) | **Yes — the diff modifies the function PM-025 fronts** (`rewards.rs:1379-1393`) | No — PM-025 gates *entry*, INC-I-180 changes the *loop body* | No. PM-025 returns `Err` before `producers.clear()`; if it fires, the loop body never runs | No. Cleanly composed: entry guard, then body | 
| PM-001/002/003/008 (gossip), PM-005/006/007/009/014–019 (sync), PM-020–024, PM-172-*/173-*/174-*/176-* (maintainer/release) | No shared surface — none reads the ProducerSet bond ledger during block validation | — | — | — |

**Unanswered interactions: none.** No blocker from this dimension.

**Two registry corrections recommended** (documentation, not behaviour):
1. `PM-180-01.interacts_with` names the INC-I-080 cap as free text and lists `PM-013`, but omits
   `PM-025`, whose fronted function this diff modifies.
2. `PM-180-01.scale_assumptions` says the work happens "under the existing producer_set read lock."
   Neither the utxo read guard nor the producer read guard existed in `validate_block_economics`
   before this diff — see **F8**. It also does not record the devnet-only asymmetry from **F5**
   (cap `u64::MAX` on devnet while this gate is live from height 20).

**Scale sensitivity.** Every new numeric constant is an activation height, not a tuning threshold:
`u64::MAX` / `230_000` / `20`. The testnet value was derived from an observed tip (`216_453`,
measured this round), which satisfies the "derived from observed system size" rule. No cap, quota,
timeout or rate constant is introduced. The allowance arithmetic is fully saturating
(`saturating_add`/`saturating_sub` throughout `:737-759`), and `u32::MAX` saturation is covered by
`req_i180_001_post_ah_u32_max_saturates_and_rejects`.

---

## Latent hazard assessment — the TOCTOU window (review item 7)

**Verdict: ACCEPTABLE as a recorded hazard, with one condition.**

The gate resolves Bond-input types under `utxo_set.read()` (`validation_checks.rs:628`), drops that
guard, then takes `producer_set.read()` (`:671`). Co-holding is genuinely forbidden: `apply_block`
locks utxo→producers (`mod.rs:197-198`) while `rollback.rs:325` locks producers→utxo, and holding
both here would join those orders into a cycle. The release is not a claim — QA proved it
empirically with a parked-future probe. The design is correct.

That leaves a window between the gate's decision (`mod.rs:113`) and apply's `producer_set.write()`
(`:198`). I verified the mitigating property rather than accepting it: `grep -rn "producer_set.write()"`
over `bins/node/src` and `crates/rpc/src` returns ten sites, all inside the node's block-processing
paths (`rollback.rs:147,151,274`, `fork_recovery.rs:337`, `apply_block/mod.rs:198`,
`genesis_completion.rs:46`, `state_update.rs:184`, `block_handling.rs:747,751,927`) and **zero**
inside the RPC crate. With a serial event loop no concurrent writer exists, so the window is
unreachable today.

**The condition:** that property is a convention, not an assertion. Nothing in the code fails if a
future change moves any one of those ten writers onto a spawned task — and at that moment the gate's
verdict becomes stale in a way that produces exactly the ledger split this milestone exists to
prevent. Record it as an invariant (`INV-*`: "every `producer_set` writer executes on the node's
serial event loop; the INC-I-180 gate's decision-to-apply window depends on it") with the ten sites
linked, so a `v_regression_map` query trips on the next edit to any of them. That converts an
unreachable hazard into a *guarded* unreachable hazard, which is the difference between this and the
INC-I-075 "currently unused" class.

---

## Improvement suggestions (non-blocking, no fix proposed)

- `validation_checks.rs` is now ~1,506 lines against a 500-line module budget (QA OBS-R3-005).
  Pre-existing and growing; splitting a live consensus validator is not a fix-round activity, but
  the gate block (`:599-793`, 196 lines) is a self-contained, well-commented unit that would extract
  cleanly into `validation_checks/withdrawal_holdings.rs` when someone does take that on.
- Two idioms for the same narrowing conversion sit 40 lines apart in the new code: saturating
  `u32::try_from(n).unwrap_or(u32::MAX)` at `:668` versus lossy `as u32` at `:681`. Neither is
  reachable at overflow, but on a consensus path one idiom is better than two.
- `OBS-R3-006` (`inc_i_096_below_gate_rejects_remove_liquidity` failing on `main`) is correctly
  excluded from this milestone and correctly proven pre-existing by a `HEAD` control. It deserves
  its own incident — a red DeFi admission-parity test on the default branch will eventually be read
  as background noise.

## Modules not reviewed

None within the milestone scope. The blast radius outward (builder, mempool, replay, storage
substrate) was reviewed and produced F1, F2, F5. Live devnet/testnet execution past the activation
height was not performed by QA and not by me — the gate is `u64::MAX` on mainnet and ~13,500 blocks
ahead on testnet, so nothing is live. A devnet run past `h=20` before the testnet height is
approached remains the right next step, and it is the cheapest way to falsify F1 and F2 empirically.

## Final Verdict

**Requires iteration.** Fix **F3** before committing — two lines, test-only, and until it lands the
milestone ships a green guard that cannot fail on a consensus-critical positional constraint. Fold
**F4**, **F6** and **F7** into the same pass; they are all in files F3 already touches, or are a
`git add`.

The consensus code itself is sound and I would approve it on its own merits: the root cause is
closed at the right layer, the three rules are jointly sufficient, parity with apply and with the
rebuild replay is real and verified, and pre-activation bit-identity holds by construction.

**F1 and F2 do not block the commit — they block the testnet activation.** Both are inert while the
height is `u64::MAX` on mainnet and un-crossed on testnet. Neither is inert once the fleet passes
`230_000`, which is roughly 1.6 days of block production away. The user should decide explicitly
between (a) landing M2 before the testnet fleet approaches the height, or (b) re-pinning the testnet
height — legal today, since it has not been crossed — to buy that room. What should not happen is
M1 deploying to testnet with the height where it is and M2 treated as an ordinary follow-up.

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: consensus-visible block-validity rules added (three new rejection paths in `validate_block_economics`); attacker-submittable fields parsed and trusted for control flow (`bond_count`, `inputs`, `producer_pubkey` from `extra_data`); external-data read across a trust boundary (UTXO set resolution of unauthenticated transaction inputs); state-integrity surface (bond ledger, selection weight, epoch reward eligibility); QA already found two exploitable-shaped defects in this exact code across rounds 1-2 (cross-owner bond spend at zero cost, ISSUE-006; mode-dependent admission, ISSUE-005); this review adds F1, a free and infinitely repeatable unauthenticated liveness trigger, and F2, a hard-abort path in the operator recovery tool
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
