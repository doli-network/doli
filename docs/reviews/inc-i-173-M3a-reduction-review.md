━━━ FINDINGS — 8 total (MAJOR:2 MINOR:6) ━━━

  [F1] MAJOR conf(0.95, measured) — .omega/memory.db protection_mechanisms:PM-173-02..05 — four protection mechanisms remain status='active' after their code was reverted; PM-173-01 and PM-173-06 still declare interacts_with edges to them
  [F2] MAJOR conf(0.90, observed) — docs/rpc_reference.md:1068 — "two nodes holding the same trust root always return the same digest" is false on the only fleet it has been measured against: the digest binds node-local `last_updated`, measured divergent at an identical tip
  [F3] MINOR conf(0.88, observed) — crates/core/src/maintainer/mod.rs:145-182 — the encoded-size arithmetic is orphaned: zero production consumers, its original consumer (the deleted journal ceiling) is gone, and its one test consumer is redundant with the assertion 20 lines above it
  [F4] MINOR conf(0.95, observed) — crates/core/tests/inc_i_173_m3_payload_bounds.rs:232-233 — the test's own doc comment still states the 785-byte figure that the same file explicitly corrects at :263-269 as "wrong by 88 bytes in the UNSAFE direction"
  [F5] MINOR conf(0.95, observed) — specs/SPECS.md:43 — master index still reads "M1 IMPLEMENTED F1+F2+F3 / F4-F7 + Options A-E PROPOSAL"; the spec body now declares M3a IMPLEMENTED and Option E WITHDRAWN
  [F6] MINOR conf(0.90, observed) — specs/state-only-fee-gate-architecture.md:17,37 — "M3 landed 2026-08-11" claims a completion state the tree does not have (nothing committed, HEAD=b5f68bba) and collides with the "M3a IMPLEMENTED" label 20 lines above
  [F7] MINOR conf(0.95, observed) — crates/core/src/transaction/tests_price_attestation.rs:438 + bins/node/tests/oracle_integration.rs:152 — two test functions named `..._is_state_only` / `..._classified_state_only` now assert the exact opposite of their names
  [F8] MINOR conf(0.85, observed) — crates/core/src/validation/tx_types.rs:784,804,817 — three new non-test early-return guards and a gauntlet-armed repo: the commit owes a per-branch `Path-Coverage:` block and a `Failure-Modes:` block, neither of which can exist yet

  Speculative: 2 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-173 M3a — reduction of M3 to F4+F5+F6+F7

Reviewer pass over `git diff 32e0a650` plus the five untracked M3a test files, on branch
`bugfix/inc-i-173-state-only-fee-gate`, working tree dirty, nothing committed.

## Scope Reviewed

| Area | Files |
|---|---|
| F5 payload bounds | `crates/core/src/maintainer/mod.rs`, `crates/core/src/validation/tx_types.rs`, `crates/core/src/validation/transaction.rs` |
| F6 digest | `crates/core/src/maintainer/digest.rs` (new), `crates/rpc/src/methods/governance.rs`, `bins/node/src/node/apply_block/governance.rs`, `docs/rpc_reference.md` |
| F4 routing + AUDIT-P3-003 wiring | `crates/core/src/transaction/core.rs`, `crates/rpc/src/methods/transaction.rs`, `bins/node/src/node/validation_checks.rs`, `crates/mempool/src/pool.rs` |
| F7 + updated tests | 5 new test files, `crates/core/src/transaction/tests_price_attestation.rs`, `bins/node/tests/oracle_integration.rs` |
| Reduction cleanliness | per-root greps over `crates/`, `bins/`, `specs/`, `docs/`; `git status` on `crates/updater`, `crates/storage` |
| Specs/docs | `specs/state-only-fee-gate-architecture.md`, `specs/engine-parts.md`, `specs/SPECS.md`, `docs/rpc_reference.md` |

Gate results were consumed from the runner's captured logs, not re-run.

## Summary

⚠️ **Code APPROVED. Report/registry/doc corrections REQUIRED before the commit.**

The reduction is clean in code. F4, F5, F6 and F7 are correct as implemented and are test-pinned
(50 assertions across the five new targets, all passing in `/tmp/m3a-fulltest.log`). No finding in
this review is a behavioural defect in the reduced tree. The two MAJOR findings are an institutional-
memory record that outlived its code (F1) and an operator-facing claim that the project's own
measurement contradicts (F2). Neither blocks the build; both block "done".

## 1. Is the reduction CLEAN? — YES, in code

Verified per-root, by symbol, not by reading the surviving diff.

- `grep -rn 'rotation.journal|maintainer_rotations|rotation_journal|MAX_ROTATION_RECORDS|TrustRootResolver'`
  over `crates` and `bins`: **zero matches**. The only hits anywhere are in
  `docs/qa/inc-i-173-M3-qa-report.md` (a historical artifact of the superseded milestone — correct to
  leave as a record) and `specs/state-only-fee-gate-architecture.md`, where every mention is inside
  the explicit WITHDRAWN section.
- `crates/core/src/maintainer/journal.rs` and `crates/storage/src/maintainer_journal.rs`: **absent**.
- `git status --short crates/updater crates/storage`: **empty**. The INC-I-172 blanket-refusal guard
  is back as the untouched status quo, exactly as intended. `crates/updater/src/trust_root.rs` and
  `crates/storage/src/maintainer_wellformed.rs` still exist and are byte-identical to `b5f68bba` —
  they are INC-I-172 M1/M2 code, not M3 residue.
- No Option-E symbol survives. `MaintainerChangeData::signing_message` is unchanged on this branch,
  and the spec says so in plain terms (`specs/state-only-fee-gate-architecture.md:449-478`).
- `docs/cli.md` and `docs/troubleshooting.md` are untouched — correct, since the QA report's
  doc-update obligations for Item 4 evaporated with Item 4.
- No orphan test target: the only `inc_i_173_m3_*` targets in the test log are the five kept ones.

The one comment I specifically hunted for — a surviving description of a rotation journal that no
longer exists — is not there. The rewritten rationale in `crates/core/src/maintainer/mod.rs:94-124`
describes only F5's own caps.

**Spec-level residue is a different matter — see F5/F6 below.** The spec text is honest about the
withdrawal; the index and the status labels are not yet in step.

## 2. The surviving encoded-size arithmetic — the judgement call

**Finding F3. Verdict: it is now orphaned complexity. Recommend deletion.**

`crates/core/src/maintainer/mod.rs:145-182` retains five constants —
`BINCODE_LEN_PREFIX_BYTES`, `ENCODED_PUBLIC_KEY_BYTES`, `ENCODED_SIGNATURE_BYTES`,
`ENCODED_MAINTAINER_SIGNATURE_BYTES` and the `pub const MAX_MAINTAINER_CHANGE_ENCODED_BYTES` (873) —
plus ~20 lines of comment explaining them.

Evidence, in the order that settles it:

1. **Zero production consumers.** `grep -rn 'MAX_MAINTAINER_CHANGE_ENCODED_BYTES|ENCODED_MAINTAINER_SIGNATURE_BYTES|BINCODE_LEN_PREFIX_BYTES|ENCODED_PUBLIC_KEY_BYTES|ENCODED_SIGNATURE_BYTES'` over `crates` and `bins` returns only the definitions themselves plus two lines in one test file.
2. **The original consumer is gone.** The registered rationale for deriving the figure was the rotation journal's file-size ceiling (`911,388 B = header + MAX_ROTATION_RECORDS × maximal record`, recorded verbatim in the decisions table and in `docs/qa/inc-i-173-M3-qa-report.md:806-814`). Item 4 was reverted; the ceiling no longer exists.
3. **The surviving test consumer is redundant.** In `crates/core/tests/inc_i_173_m3_payload_bounds.rs`, the caps-are-consistent property is established at `:250-261` by
   `assert!(encoded.len() <= MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES)` — against the **real bincode
   encoder**, on a maximal payload the test builds itself. That assertion alone is the whole
   guarantee. The second assertion at `:270-277`
   (`assert_eq!(encoded.len(), MAX_MAINTAINER_CHANGE_ENCODED_BYTES)`) compares the real encoder to a
   hand-rolled restatement of bincode's encoding rules. It cannot catch a cap inconsistency the first
   assertion misses; it can only catch the hand-rolled copy drifting from bincode — a risk that
   exists solely because the copy exists.
4. **`pub` with no consumer** is dead public API on `doli-core`.

The rewritten rationale ("these exist so the claim is a DERIVED constant the compiler recomputes,
not prose") describes a real hazard — the OBS-5 arithmetic really was wrong by 88 bytes — but the
fix for prose that drifts is to delete the prose and assert against the encoder, which the test
already does. Keeping a second copy of bincode's rules to keep the first copy honest is the
duplication, not the cure.

**Recommendation:** delete the five constants, delete the `:270-277` assertion, and move the
"873 B, 151 B headroom" figure into the doc comment on `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES` as
prose that the surviving `<=` assertion protects behaviourally. Net −45 lines, no property lost.
Not a blocker.

## 3. F4 correctness — CORRECT

Routing moved from the type-based `is_state_only()` (deleted, `transaction/core.rs`) to the
shape-based `is_zero_flow()` = `inputs.is_empty() && outputs.is_empty() && tx_type.allows_empty_io()`
(`crates/core/src/transaction/core.rs:478-480`). Both production callers migrated:
`crates/rpc/src/methods/transaction.rs:210` and `bins/node/src/node/validation_checks.rs:949`. Both
are pinned by `bins/node/tests/inc_i_173_m3_f4_routing.rs:145-200`, which asserts by source scan that
neither file still mentions the deleted predicate — a cheap guard against a half-migration.

**AUDIT-P3-003 wiring is complete.** All six production `ValidationContext` construction sites carry
`.with_inc_i_173_activation_height(...)`: `crates/mempool/src/pool.rs:363,766`,
`bins/node/src/node/validation_checks.rs:103,289`, `bins/node/src/node/apply_block/tx_processing.rs:61`,
`bins/node/src/node/production/assembly.rs:186`. Enumerated per-root; there is no seventh site. This
matters more than it did before F5, because F5 puts a second gate evaluation behind the same field —
an unwired consensus-path site would now be silently *more permissive*, not merely inert.

**The `crates/mempool/src/pool.rs:737-764` comment is TRUE of the reduced tree.** I checked its load-
bearing claim directly: `TxType::Registration => true` in `allows_empty_io`
(`crates/core/src/transaction/types.rs:184`), so a 0-in/0-out `Registration` does reach
`add_system_transaction` — the fourth routing delta, and the only one toward more free relay. The
comment's bound is also true: `validation/registration.rs` takes its no-inputs branch only under
`is_in_genesis`, and rejects post-genesis with "registration must have inputs for bond". The comment
correctly labels itself a correction of the previous, now-false, `is_state_only`-citing text. The
same delta table in `specs/state-only-fee-gate-architecture.md:300-312` matches the code.

`PriceAttestation` losing the system lane is stated, tested and inert
(`oracle_activation_height = u64::MAX` everywhere). `ClaimReward`/`ClaimBond` losing it is stated
with the OBS-2 correction that they are rejected unconditionally rather than "now pay a fee" — I
read `validate_claim_data`/`validate_claim_bond_data` and the correction is right.

## 4. F5 correctness — CORRECT

`crates/core/src/validation/tx_types.rs:753-836`.

- **Check order is right.** The size cap at `:784-792` runs before
  `MaintainerChangeData::from_bytes` at `:795`. bincode never sees a buffer larger than 1024 bytes
  above the gate. Pinned behaviourally by `audit_p1_001_size_cap_runs_before_the_decoder`
  (`payload_bounds.rs:375`), not merely by reading order.
- **Below-gate inertness is total.** Every one of the three bounds sits inside a
  `ctx.current_height >= ctx.inc_i_173_activation_height` conjunct or block; the three historical
  structural refusals (`:762-781`) are untouched and run on both branches. Four dedicated tests
  assert that oversized `extra_data`, oversized signature counts, oversized `reason` and multi-byte
  `reason` are all still *accepted* below the gate (`payload_bounds.rs:591-664`), and one asserts the
  frozen refusals are unchanged on both branches (`:698`).
- **`reason` is bounded in BYTES** (`reason.len()`, `:818`), not chars. Pinned by
  `audit_p1_001_reason_cap_counts_bytes_not_chars`.
- **`MAX_MAINTAINER_CHANGE_SIGNATURES = MAX_MAINTAINERS` is still principled without Option E.** The
  justification never depended on Option E: `MaintainerSet::count_distinct_signers` counts a
  signature only when its pubkey is a *current member*, and membership is capped at `MAX_MAINTAINERS`
  (`crates/core/src/maintainer/mod.rs:92`), so entry six can never contribute a distinct signer. The
  bound removes no capability at any set size, and the 5-of-5 maximal payload is proven acceptable
  above the gate (`req_173_014_maximal_legal_payload_is_accepted_above_the_gate`). Option E would
  have changed the signed *message*, not the number of signers the set can hold.

## 5. F6 correctness — CORRECT

- **Purely additive.** Every hunk in `crates/rpc/src/methods/governance.rs` adds fields; the only
  edit to an existing line is a trailing comma. Two dedicated tests
  (`audit_p1_003_on_chain_branch_keeps_every_existing_field`,
  `..._derived_branch_keeps_every_existing_field`) assert the pre-existing field set survives, so the
  additivity is enforced and not merely observed.
- **The digest module is a LEAF.** `crates/core/src/maintainer/digest.rs` imports exactly
  `crypto::Hasher` and `super::MaintainerSet`. The genesis hash arrives as `&[u8]` (`:54`). No new
  edge from `crates/core/validation` toward node-local maintainer state, and none toward `chainspec`.
- **Digest construction is sound for its stated question.** Domain-separated preimage, members sorted
  ascending before hashing (neutralising the AUDIT-P3-014 insertion-order nondeterminism), fixed-width
  `threshold`/`last_updated` terms, genesis binding to defeat the mainnet/testnet key-array collision.
  Ten tests cover order independence, empty set, duplicate members, and per-term sensitivity.
- **The apply-side log line is on the success arm only** (`apply_block/governance.rs:56,93`), on a
  fixed grep anchor, after the set is mutated. Correct placement.

The defect is not in the code — see F2.

## 6. Consensus-shape checklist for F5 (CLAUDE.md INV-12 / INC-I-075)

**Q1 — can a user-submittable tx reach this path?** **YES.** `AddMaintainer`/`RemoveMaintainer` are
submittable through RPC `sendTransaction`; `validate_maintainer_change_data` runs on every one of
them, from `validate_transaction` (`crates/core/src/validation/transaction.rs:170-176`).

**Q2 — can a producer-action or attestation pattern reach it?** **YES.** A producer may include either
type in a block; every node runs the same validator during block validation
(`crates/core/src/validation/block.rs:237,314`) and during apply
(`apply_block/tx_processing.rs:61-105`).

**Q3 — is the new behaviour bit-identical for ALL reachable inputs?** **NO, above the gate.** A payload
over 1024 bytes, or with more than 5 signature entries, or with a `reason` over 256 bytes, is now
REJECTED where it was previously accepted structurally.

**(Q1|Q2) YES + Q3 NO → an activation height is REQUIRED. F5 has one.**

**Does F5 need a NEW height, or does it correctly ride `inc_i_173_activation_height`? It correctly
rides the existing one.** The retroactive-vacuity argument holds and I verified its premise rather
than accepting it: below the gate `validate_transaction_with_utxos` takes the frozen 3-type
`matches!` branch, which excludes both maintainer types, so a 0-in/0-out maintainer tx fails the fee
check (`fee = 0 < BASE_FEE = 1`) at every height below the gate — being unmineable IS the INC-I-173
bug. No block at any height below the gate on any network can contain either type, so tightening
their structural validation cannot invalidate a single existing block. The gate flips at exactly the
same height and from exactly the same field as the fee gate it rides, evaluated in the same shared
validator chain, so the two cannot disagree by construction.

**Deploy consequence, unchanged from M1 and re-affirmed here:** `inc_i_173_activation_height` is
`133_000` on testnet and the live tip is ~134,700 — **crossed**. The change activates on arrival, so
the testnet deploy is a **synchronized stop-all/start-all**, never a rolling restart (INV-8 /
INC-I-062). Mainnet `u64::MAX` and devnet `0` are unaffected. The in-code comment at
`bins/node/src/node/validation_checks.rs:308-333` states this correctly, including the correction of
its own earlier false claim.

## 7. Findings

### F1 — MAJOR — protection registry describes four mechanisms that no longer exist

- **Location:** `.omega/memory.db`, `protection_mechanisms` rows `PM-173-02`, `PM-173-03`,
  `PM-173-04`, `PM-173-05`; edges on `PM-173-01`, `PM-173-06`.
- **Evidence:** `SELECT mechanism_id, status, interacts_with FROM protection_mechanisms WHERE
  mechanism_id LIKE 'PM-173%'` returns all six rows `active`, including "Maintainer rotation journal
  bounds", "Trust-root resolution cache", "Quorum-verified rotation replay" and "Chain-bound
  maintainer authorization (Option E)". Their code is absent:
  `ls crates/core/src/maintainer/journal.rs crates/storage/src/maintainer_journal.rs` →
  "No such file or directory". `PM-173-01.interacts_with = ["PM-173-02","PM-173-04"]` and
  `PM-173-06.interacts_with = ["PM-173-04"]` — the two surviving mechanisms point at deleted ones.
- **Confidence:** conf(0.95, measured).
- **Impact:** the system-impact protocol makes `v_protection_surface` the mandatory input to the next
  change in this area. The next agent will reason about interactions with a rotation journal, a
  trust-root cache and a chain-bound authorization that do not exist, and will find F5's declared
  interaction partners missing. This is the same class as a stale doc, with a worse blast radius,
  because it is queried automatically rather than read by choice.
- **Suggested fix:** set `PM-173-02`, `PM-173-03`, `PM-173-04`, `PM-173-05` to a non-active status
  with a note naming the M3a reduction, and rewrite `interacts_with` on `PM-173-01` and `PM-173-06`
  to drop the dangling ids. Do it in the same act as the commit.

### F2 — MAJOR — the `getMaintainerSet` headline claim is contradicted by the project's own measurement

- **Location:** `docs/rpc_reference.md:1068`; mechanism at
  `crates/core/src/maintainer/digest.rs:62`.
- **Evidence:** the doc states "Two nodes holding the same trust root always return the same digest;
  two nodes holding different roots never do." The digest preimage includes
  `set.last_updated.to_le_bytes()` (`digest.rs:62`). `last_updated` is node-local and outside the
  state root, and the M3 security audit **measured** it divergent across the live testnet fleet at an
  identical tip: `docs/.workflow/chain-state.md:36-39` — "RPC 8512 `last_change_block = 88289` vs 12
  peers at `1`, identical tip 134,682". Those 13 nodes hold the same five members and the same
  threshold, and will publish two distinct digests.
- **Confidence:** conf(0.90, observed) — the divergence is measured, the consequence for the digest is
  read off the preimage.
- **Impact:** F6's entire stated purpose is "compare two nodes' trust roots with ONE scalar". On the
  only fleet where the inputs have been measured, that scalar reports a mismatch for a fleet that is
  in fact aligned on its release-verification root. A false-mismatch instrument is worse than no
  instrument: it is the same failure the sorted-members design was specifically introduced to avoid,
  reintroduced through a different term. The doc is also internally inconsistent — `:1077-1080`
  correctly says a changed `last_change_block` changes the digest, which cannot both be true and
  leave `:1068` true.
- **Suggested fix (subtraction preferred):** remove `last_updated` from the digest preimage, so the
  digest answers exactly its stated question — "do we hold the same release-verification trust root?"
  — over the terms `verify_multisig` actually consults (members + threshold + chain). `last_updated`
  is already published separately as `last_change_block` on the same RPC response and in the log
  line, so an operator loses nothing and gains a scalar that does not false-alarm. If the term is
  kept instead, `:1068` must be reworded to say the digest binds `last_change_block` and that a
  member-identical fleet can legitimately mismatch, and the digest domain tag should move to `-V2`.
- **Test strategy:** extend `crates/rpc/tests/inc_i_173_m3_maintainer_set_rpc.rs` with two harnesses
  holding identical members/threshold and different `last_updated`, asserting the intended verdict.

### F3 — MINOR — orphaned encoded-size arithmetic

- **Location:** `crates/core/src/maintainer/mod.rs:145-182`; sole consumer
  `crates/core/tests/inc_i_173_m3_payload_bounds.rs:270-277`.
- **Evidence:** see §2 — grep shows no production consumer; the deleted journal ceiling was the
  original one; the redundant assertion is 20 lines below one that already proves the property
  against the real encoder.
- **Confidence:** conf(0.88, observed).
- **Suggested fix:** delete the five constants and the `assert_eq!` at `:270-277`; keep the `<=`
  assertion; move the 873/151 figures into the doc comment on
  `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES`.

### F4 — MINOR — a test's doc comment carries the figure the same test corrects

- **Location:** `crates/core/tests/inc_i_173_m3_payload_bounds.rs:232-233`.
- **Evidence:** the doc comment reads "Contract's computed worst case: 32 target + 8 len + 5x96 sigs
  + 1 + 8 + 256 reason = 785 <= 1024". Thirty lines below, `:263-269` states that figure "used to
  live only in a comment that stated 785 — wrong by 88 bytes in the UNSAFE direction". The real
  figure is 873.
- **Confidence:** conf(0.95, observed).
- **Suggested fix:** replace the 785 line with the derived 873 (or, if F3 is applied, with "5 sigs +
  a 256-byte reason, asserted against the real encoder below").

### F5 — MINOR — `specs/SPECS.md` index not updated for the reduction

- **Location:** `specs/SPECS.md:43`.
- **Evidence:** the entry still reads "M1 IMPLEMENTED F1+F2+F3 / F4-F7 + Options A-E PROPOSAL,
  2026-08-10", while `specs/state-only-fee-gate-architecture.md:17` now declares "M3a IMPLEMENTED
  (F4+F5+F6+F7 only)" and `:449` marks Option E WITHDRAWN.
- **Confidence:** conf(0.95, observed).
- **Suggested fix:** update the index entry to name M3a's shipped set and Option E's withdrawal.

### F6 — MINOR — "M3 landed" overstates an uncommitted tree, and the M3/M3a labels collide

- **Location:** `specs/state-only-fee-gate-architecture.md:17` and `:37`.
- **Evidence:** `:17` "M3a IMPLEMENTED (F4+F5+F6+F7 only)"; `:37` "**M3 landed 2026-08-11** on the
  same branch (base `32e0a650`)". `git status` shows the tree dirty with nothing committed; `HEAD` is
  `b5f68bba`. "Landed" is a completion claim the repository does not support, and the same body of
  work carries two names 20 lines apart.
- **Confidence:** conf(0.90, observed).
- **Suggested fix:** use one label (M3a) throughout, and state "implemented, pending commit" until
  the commit exists.

### F7 — MINOR — two test functions assert the opposite of their names

- **Location:** `crates/core/src/transaction/tests_price_attestation.rs:438`
  (`test_price_attestation_is_state_only`) and `bins/node/tests/oracle_integration.rs:152`
  (`audit_p1_003_price_attestation_classified_state_only`).
- **Evidence:** both bodies now assert `!tx.is_zero_flow()` — that `PriceAttestation` is *not*
  system-routed. The doc comments explain the inversion; the names do not.
- **Confidence:** conf(0.95, observed).
- **Suggested fix:** rename to `price_attestation_is_not_zero_flow` /
  `audit_p1_003_price_attestation_is_not_system_routed`.

### F8 — MINOR — the commit owes two gate blocks that cannot exist yet

- **Location:** new guards at `crates/core/src/validation/tx_types.rs:784-792`, `:804-810`,
  `:817-825`; repo gate at `.omega/gauntlet.conf` (present).
- **Evidence:** three new early-return guards in non-test Rust trigger CLAUDE.md rule 24
  (`Path-Coverage:` per-branch commit block, blocking-enforced by `path-coverage-gate.sh`). F5
  registers a protection mechanism in a gauntlet-armed repo, triggering rule 29 (`Failure-Modes:`
  block, blocking-enforced by `gauntlet-gate.sh`). Nothing is committed, so neither block exists.
- **Confidence:** conf(0.85, observed).
- **Suggested fix:** author both blocks as part of the commit message; the `Failure-Modes:` block must
  answer the reduced tree's modes, not the six-item M3's.

## Speculative Findings (low-confidence, not actionable)

- **S1 — mempool/consensus one-block skew at the gate boundary.** conf(0.60, inferred). The mempool
  builds its `ValidationContext` with `current_height` = the node's chain tip
  (`crates/mempool/src/pool.rs:363,766`), while block validation uses the block's height. At tip
  `gate − 1` the mempool evaluates F5 on the below-gate branch and would admit an oversized maintainer
  payload that block validation at `gate` rejects. No fork risk: `production/assembly.rs:186` builds
  its context from the block height, so a producer will not include it. The effect is a stranded
  mempool entry for at most one block. This is the same skew M1's fee gate already carries, not an
  M3a regression.
- **S2 — `genesis_hash` is not length-prefixed in the digest preimage.** conf(0.50, inferred).
  `digest.rs:60` hashes the slice directly. Every current caller passes a fixed 32-byte
  `crypto::Hash`, so there is no ambiguity today; a future caller passing a variable-length slice
  could in principle shift bytes between the chain term and the member list. Adding a length prefix
  would cost a domain-tag bump for zero present benefit.

## Specs/Docs Drift

Covered as F2, F4, F5, F6. Two positives worth recording: `specs/engine-parts.md:479` correctly
tombstones `Transaction::is_state_only` with the reason and the replacement, and
`specs/state-only-fee-gate-architecture.md:449-478` gives an unusually honest account of why Option E
was withdrawn, including that C7 (replay) is now an unaddressed open risk rather than a solved one.
`docs/DOCS.md` needs no change. One cosmetic item inside F5's scope: the example JSON at
`docs/rpc_reference.md:1050` shows `"genesis_hash": "0000...."` next to a full 64-hex digest.

## Architecture Escalation

None. The design invalidation that stopped M3 lives entirely in the two reverted items. Nothing in
the reduced tree depends on the compiled bootstrap maintainer five as a trust anchor, and F5/F6 do
not assume a rotation journal exists. INC-I-175 remains open and independent; F2's recommended
subtraction does not touch it.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -small — F2's subtraction and F3's deletion both remove work; F2 drops one 8-byte hasher update per getMaintainerSet call and per applied rotation (observed)
  Memory:   0 — no allocation changes; F3 deletes compile-time constants only (observed)
  IO:       0 — no new syscalls; the F6 log line is unchanged by any proposed fix (observed)
  Network:  0 — F2 keeps the response field count identical; only the digest's preimage changes (observed)
  Disk:     0 — no persisted format is touched by any proposed fix (observed)
  Latency:  0 — getMaintainerSet is an operator-cadence RPC, not a hot path; the removed hasher update is sub-microsecond (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: for F2, leave the digest as built and only reword docs/rpc_reference.md:1068 to warn that a member-identical fleet can legitimately mismatch — a one-line doc edit with zero code change
Why this proposal anyway: the doc-only path leaves the fleet's primary trust-root comparison instrument returning a mismatch for 13 aligned nodes on the live testnet, so operators learn to ignore it, which is the failure mode F6 exists to prevent
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Modules Not Reviewed

None within the stated scope. `docs/qa/inc-i-173-M3-qa-report.md` and
`docs/reviews/inc-i-173-M3-hardening-review.md` were read for evidence only — they are artifacts of
the superseded six-item M3 and are correctly left as historical record, not updated.

## Final Verdict

**Code approved for commit. Blocking on commit: F1 (registry), F5 and F6 (spec index and status
labels), F8 (commit-message gate blocks) — all cheap. F2 must be resolved before the digest is
presented to operators as a divergence check. F3, F4, F7 are follow-ups.**

## Security Audit Verdict

AUDIT-REQUIRED

F5 is consensus-visible above an activation height that testnet has already crossed, and it bounds an
attacker-controlled payload on a fee-exempt, input-less transaction — a trust boundary by any reading.
F6 publishes a new RPC field and a new log line derived from the release-verification trust root. F4
changes mempool admission routing, and by the evidence in §3 it moves one shape (`Registration`,
0-in/0-out) toward *more* free relay. Any one of these is enough.

Mitigating context the sweep should use to scope itself: items 1/2/5/6 of the superseded M3 — which
are exactly F5/F6/F4/F7 — already went through the 5-auditor sweep and drew no P0 and no P1
(`docs/.workflow/chain-state.md:51-56`). The required sweep can therefore be scoped as a **delta
audit** over what changed since that sweep: the two reverts, the rewritten rationale comments in
`crates/core/src/maintainer/mod.rs` and `crates/mempool/src/pool.rs`, and F2's `last_updated` binding
in the digest preimage. A full re-sweep of unchanged, already-cleared code is not what makes this
verdict AUDIT-REQUIRED.
