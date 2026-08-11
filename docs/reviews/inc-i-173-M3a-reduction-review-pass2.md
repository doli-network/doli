━━━ FINDINGS — 4 total (MINOR:4) ━━━

  [P1] MINOR conf(0.85, observed) — crates/core/src/maintainer/digest.rs:1 vs crates/rpc/src/methods/governance.rs:93 — the [F6] fix declares "M3" = the SUPERSEDED six-item milestone, yet 24 shipped artifacts self-label "INC-I-173 M3"; the same F6 feature is labelled M3 in one file and M3a in another
  [P2] MINOR conf(0.95, measured) — crates/core/src/validation/tx_types.rs:1129, bins/node/src/node/validation_checks.rs:1310, crates/mempool/src/pool.rs:1724 — three already-over-budget source files grew further this milestone (+58 / +34 / +29 over 32e0a650); Global Rule 19 limit is 500
  [P3] MINOR conf(0.90, observed) — crates/core/tests/inc_i_173_m3_maintainer_digest.rs:386-388 — the exclusion pin's doc comment quotes docs/rpc_reference.md verbatim as saying "two nodes holding the same trust root always return the same digest"; the same [F2] fix rewrote that sentence (rpc_reference.md:1069-1070), so the quotation is now a dead attribution
  [P4] MINOR conf(0.80, observed) — .omega/memory.db protection_mechanisms PM-173-01.interacts_with = '[]' — the [F1] repair dropped the dangling ids but recorded NO edge to PM-172-05, which is UNGATED and fires on the same AddMaintainer/RemoveMaintainer trigger surface

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review (pass 2): INC-I-173 M3a — scoped verification of review-iteration-1 fixes

Scoped verification pass, not a fresh review. Branch `bugfix/inc-i-173-state-only-fee-gate`,
`git rev-parse HEAD` = `32e0a650d4694a99d378c04c9d6ff5b965036438`, working tree dirty, nothing
committed. Iteration 1 (`docs/reviews/inc-i-173-M3a-reduction-review.md`) approved the code and
raised 8 findings; F4/F5/F6 routing, check order and additivity were cleared there and are NOT
re-reviewed here.

Gate results were consumed from the developer's record, not re-run. I re-ran two targets myself
(cheap, and they are the two the fixes edited most): `inc_i_173_m3_maintainer_digest` **10 passed**,
`inc_i_173_m3_payload_bounds` **19 passed**, both 0 failed.

## Fix-by-fix verification

### [F2] `last_updated` removed from the digest preimage — LANDED, and it was the right call

**(a) Preimage.** `crates/core/src/maintainer/digest.rs:81-88` now hashes exactly four terms:
`MAINTAINER_SET_DIGEST_DOMAIN` (`:82`), `genesis_hash` (`:83`), `(threshold as u64).to_le_bytes()`
(`:84`), then the members sorted ascending (`:78-79`, `:85-87`). No `last_updated` update remains;
`grep -rn last_updated crates/core/src/maintainer/digest.rs` returns only doc-comment lines
(`:47,49,58,64`) explaining the exclusion.

**(b) Exclusion pinned.** `audit_p1_003_digest_is_independent_of_last_updated`
(`crates/core/tests/inc_i_173_m3_maintainer_digest.rs:401-440`) drives `last_updated` over
`{0, 88_289, 172_000, u64::MAX}` against a `last_updated = 1` reference, across three genesis
partitions, asserting EQUALITY. The `88_289` / `1` pair is the measured fleet shape, so the test
reproduces the exact false mismatch. It carries an anti-vacuity tail (`:422-439`): a member swap and
a threshold change must still move the digest, so the test cannot pass by the digest having gone
insensitive. A second pin exists one layer up at the RPC boundary
(`crates/rpc/tests/inc_i_173_m3_maintainer_set_rpc.rs:383-435`), which additionally asserts
`last_change_block` still DIFFERS between the two responses — that is what makes "the operator loses
nothing" checkable rather than asserted. Both go beyond what iteration 1 asked for.

**(c) Docs.** `docs/rpc_reference.md:1065` — the preimage formula no longer contains
`last_updated_le_u64`. `:1068-1092` is a rewritten block: the headline is now "Two nodes that accept
the same release signatures always return the same digest", followed by an explicit excluded-terms
list (member ORDER, `last_change_block`) each carrying its measurement, closing with ":1090 a changed
MEMBER, a changed `threshold` or a different `genesis_hash` each change the digest; member order and
`last_change_block` do not." `grep -rn "last_updated_le_u64\|M3 landed" specs docs crates bins`
(excluding review/qa/workflow artifacts) → **zero hits**. No surviving claim contradicts the measured
divergence. The stale example digest was elided to `f99d3e79....` (`:1049`), matching the truncation
style of the neighbouring fields — correct, since a literal computed under the old preimage would be
a value no node can reproduce.

**(d) Per-term sensitivity retained, not deleted.** `audit_p1_003_digest_changes_when_a_member_changes`
(`:328`), `audit_p1_003_digest_changes_when_threshold_changes` (`:366`),
`audit_p1_003_digest_differs_across_genesis_hashes` (`:452`) all still exist and pass. The exact-preimage
pin (`:168-203`) independently recomputes the digest in `expected_digest` (`:144-156`) rather than
calling the function under test, so a re-added term fails there too. Domain separation, determinism,
insertion-order independence, empty set and duplicate members are also still covered — 10 tests, all
green.

**Independent judgement on whether removing the term was right: YES.** Two reasons that stand without
reference to the developer's argument. First, the digest's stated question is answered by
`MaintainerSet::verify_multisig`, which consults members and threshold and never reads `last_updated`
— so the term was in the preimage of a function that models a predicate that does not depend on it.
Second, the blast radius of the subtraction is nil: `grep -rn maintainer_set_digest --include='*.rs'`
per-root gives, in `crates`, only the definition (`digest.rs:77`), the re-export (`mod.rs:75`) and four
RPC read sites (`rpc/methods/governance.rs:90,127-128,204-205`); in `bins`, only a log line
(`apply_block/governance.rs:56,93,167-174`). No consensus path, no persisted format, no updater/trust-root
consumer. The digest is a pure observability scalar, and an observability scalar that reports a
divergence which does not exist is strictly worse than one term poorer. Keeping the `-V1` domain tag is
also correct: nothing is committed or deployed, so no published digest exists to be compatible with.

### [F3] encoded-size arithmetic deleted — LANDED, and the surviving guarantee is INTACT

`crates/core/src/maintainer/mod.rs` no longer defines `MAX_MAINTAINER_CHANGE_ENCODED_BYTES`,
`BINCODE_LEN_PREFIX_BYTES`, `ENCODED_PUBLIC_KEY_BYTES`, `ENCODED_SIGNATURE_BYTES` or
`ENCODED_MAINTAINER_SIGNATURE_BYTES`; a repo-wide grep for all five names across `crates`, `bins`,
`specs` and `docs` (excluding review artifacts) returns **zero hits** — no dead reference, no dead
public API.

The load-bearing check for this milestone is whether the sole surviving caps-consistency guarantee
was weakened. It was not:

- `crates/core/tests/inc_i_173_m3_payload_bounds.rs:253-264` still asserts
  `encoded.len() <= MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES` where `encoded = maximal.to_bytes()`
  (`:243`) — the **real** bincode encoder, not a restatement.
- The fixture is still MAXIMAL, and maximal by construction rather than by literal: `:239-242` builds
  `change_data(MAX_MAINTAINER_CHANGE_SIGNATURES, Some("z".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES)))`,
  and `change_data` (`:157-165`) emits `sig_count` distinct full signature entries plus a target key.
  Raising either cap automatically enlarges the fixture; adding a field to `MaintainerChangeData`
  enlarges the encoding. Both fail the assertion rather than silently passing.
- `:245-252` additionally pins `signatures.len() == MAX_MAINTAINERS`, so the fixture cannot silently
  stop being 5-of-5.
- The 873/151 figures moved to prose on `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES`
  (`maintainer/mod.rs:117-145`), which names the test that protects them and preserves the OBS-5
  history. `:265-271` in the test leaves a NOTE recording why the second assertion is gone.

19/19 payload-bounds tests pass after the deletion, including
`req_173_014_maximal_legal_payload_is_accepted_above_the_gate` (the behavioural twin). **No
regression.**

### [F4] 785 → 873 — LANDED

`crates/core/tests/inc_i_173_m3_payload_bounds.rs:232-236` no longer restates the arithmetic at all:
"The worst case is 873 bytes — 5 signatures plus a 256-byte `reason` — which leaves 151 bytes of
headroom … NOT restated arithmetically anywhere; it is asserted below against the real bincode
encoder." Deleting the derivation instead of correcting its digits is the stronger fix — the figure
was already wrong once by 88 bytes in the unsafe direction.

### [F5] `specs/SPECS.md` index — LANDED

`specs/SPECS.md:43` now reads "M1 + M3a IMPLEMENTED = F1-F7 / Options A-E PROPOSAL, Option E
built-then-WITHDRAWN, 2026-08-11" and its tail names what M3a shipped, including the `last_updated`
exclusion and the withdrawal of the rotation journal + Option E. In step with the spec body.

### [F6] spec status honest — LANDED

`specs/state-only-fee-gate-architecture.md:17-24` — "M1 LANDED (F1+F2+F3, commit `32e0a650`). M3a
IMPLEMENTED IN THE WORKING TREE, PENDING COMMIT", plus a labelling rule that names M3 as the
superseded six-item milestone. `:43-45` — "base `32e0a650`, which is `HEAD` — nothing of M3a is
committed yet". Both statements are true against `git rev-parse HEAD` = `32e0a650…` and a dirty tree
with zero commits. `grep -n "M3 landed"` over `specs` → zero hits. The Option E section (`:466-517`)
is consistent with the status header and still states plainly that C7 replay is an unaddressed open
risk.

### [F7] test renames — LANDED, traceability preserved

`crates/core/src/transaction/tests_price_attestation.rs:112` → `price_attestation_is_not_zero_flow`,
with AUDIT-P3-002 and the old name recorded in the doc comment (`:93-110`).
`bins/node/tests/oracle_integration.rs:61` → `audit_p1_003_price_attestation_is_not_system_routed`,
AUDIT-P1-003 retained in the name and AUDIT-P3-002 in the adjacent comment block (`:41-59`). Both
bodies assert `!tx.is_zero_flow()`, which now matches the names. A repo-wide grep for either old name
across `crates`, `bins`, `specs`, `docs` (excluding review artifacts) returns **zero hits** — no dead
reference in a test harness, CI filter or doc.

## Binding constraints — all HOLD

| Constraint | Result |
|---|---|
| `git status --short crates/updater crates/storage` | **empty** — both reverts stay reverted |
| `git diff 32e0a650 -- crates/core/src/network_params/` | **empty** — no activation height touched |
| `git diff 32e0a650 --stat -- crates/core/src/consensus Cargo.toml` | **empty** — no version constant touched |
| L1/L2 at `crates/core/src/validation/transaction.rs:39-88` | `shasum` of lines 39-88 at `32e0a650` and in the tree are **both** `b6341c35c4be15483514012c6c4a64cebdaac0da` — character-identical |
| `git diff 32e0a650 -- crates/core/src/validation/transaction.rs` | a **single** hunk at `@@ -168,10 +168,10 @@` (the `ctx` threading) |
| [F1] registry repair | `PM-173-02..05` are `status='removed'` and absent from `v_protection_surface`; `PM-173-01`/`PM-173-06` `interacts_with = '[]'` — no dangling id survives |
| Test-file budget (800) | max is `inc_i_173_m3_payload_bounds.rs` at **728** — inside budget |

## Findings

### [P1] MINOR — the new labelling rule is contradicted by 24 shipped self-labels

- **Location:** `crates/core/src/maintainer/digest.rs:1`; `crates/rpc/src/methods/governance.rs:93`;
  `docs/rpc_reference.md:1061`; `specs/engine-parts.md:479`.
- **Evidence:** the [F6] fix added `specs/state-only-fee-gate-architecture.md:22-24` — "the six-item
  milestone is called **M3** and was SUPERSEDED; what is in the tree is **M3a**". A grep for
  `INC-I-173 M3` across `crates`, `bins`, `docs/rpc_reference.md` and `specs` (excluding review/qa
  artifacts) returns **24 lines**. The same F6 feature carries both labels in two files:
  `digest.rs:1` "INC-I-173 M3 / F6" versus `rpc/methods/governance.rs:93` "INC-I-173 M3a / F6".
  `engine-parts.md:479` attributes the shipped `is_state_only` deletion to "INC-I-173 M3", a
  milestone the spec now declares superseded and never committed.
- **Confidence:** conf(0.85, observed).
- **Impact:** cosmetic today; the cost lands on the next agent, who reads a rule saying M3 was
  withdrawn and then finds live code attributing itself to M3. Iteration 1's F6 asked for one label
  for one body of work; the rule was added but not applied outward.
- **Suggested fix:** either narrow the rule's wording to "M3 in prose means the superseded milestone;
  `INC-I-173 M3` in code headers is historical and reads as M3a", or sweep the 24 self-labels. The
  first is one sentence and loses nothing.

### [P2] MINOR — three over-budget source files grew further

- **Location:** `crates/core/src/validation/tx_types.rs` (1129), `bins/node/src/node/validation_checks.rs`
  (1310), `crates/mempool/src/pool.rs` (1724).
- **Evidence:** `wc -l` now versus `git show 32e0a650:<path> | wc -l`: 1071→1129 (+58),
  1276→1310 (+34), 1695→1724 (+29). Global Rule 19 caps source files at 500.
- **Confidence:** conf(0.95, measured).
- **Impact:** none behavioural. All three were already 2-3x over budget at the base commit, so this is
  inherited debt that M3a added to rather than created. Recording it keeps the condition visible
  instead of silently compounding.
- **Suggested fix:** none inside this milestone — a split of `pool.rs` or `tx_types.rs` is its own
  change with its own blast radius, and doing it inside a consensus-touching milestone is worse than
  the debt. File it as a follow-up.

### [P3] MINOR — the exclusion pin quotes a doc sentence the same fix deleted

- **Location:** `crates/core/tests/inc_i_173_m3_maintainer_digest.rs:386-388`.
- **Evidence:** the doc comment reads: makes "the operator-facing claim in `docs/rpc_reference.md`
  (\"two nodes holding the same trust root always return the same digest\") true rather than
  aspirational". That sentence was the [F2] finding's own quotation of the OLD text; the fix rewrote
  it, and `docs/rpc_reference.md:1069-1070` now reads "Two nodes that accept the same release
  signatures always return the same digest". The quotation marks make it a verbatim attribution that
  no longer resolves.
- **Confidence:** conf(0.90, observed).
- **Impact:** the claim is semantically preserved, so nothing is misleading about the digest. It is a
  dead citation of exactly the kind this review pass was asked to catch.
- **Suggested fix:** requote the current sentence, or drop the quotation marks and cite
  `docs/rpc_reference.md:1068-1092` by range.

### [P4] MINOR — PM-173-01 now records no interaction, but shares a trigger surface with PM-172-05

- **Location:** `.omega/memory.db`, `protection_mechanisms.interacts_with` on `PM-173-01`.
- **Evidence:** `SELECT mechanism_id, status, interacts_with FROM protection_mechanisms WHERE
  mechanism_id LIKE 'PM-173%'` → `PM-173-01|active|[]`. `SELECT … FROM v_protection_surface` lists
  `PM-172-05` ("un-authorizable maintainer set refusal, **UNGATED**"), whose recorded trigger is
  "reached from AddMaintainer, RemoveMaintainer and ProtocolActivation — all user-submittable" — the
  same two tx types as PM-173-01's trigger.
- **Confidence:** conf(0.80, observed).
- **Impact:** the interaction is real but benign, and I answer it here rather than leaving it open, so
  it is **not** a blocker: (1) both mechanisms can fire on the same event, and the order is fixed and
  total — PM-173-01's caps run inside `validate_transaction`
  (`crates/core/src/validation/tx_types.rs:784,804,817`) strictly before any node-level
  `verify_multisig`; (2) neither action can create the other's trigger — both actions are refusals of
  the same transaction, producing no new input; (3) neither starves the other — PM-172-05's trigger
  surface is the persisted set, not the tx stream, so PM-173-01 rejecting a transaction cannot
  disarm it. `[]` records "no interaction", which is one degree stronger than the truth.
- **Suggested fix:** set `PM-173-01.interacts_with = '["PM-172-05"]'` with the three answers above as
  the note, in the same act as the commit.

## Speculative Findings (low-confidence, not actionable)

- **S1 — duplicate members are a theoretical counterexample to the headline doc claim.**
  conf(0.55, inferred). `docs/rpc_reference.md:1069-1070` asserts two nodes accepting the same release
  signatures always digest the same. A set `[A,A,B,C]` and a set `[A,B,C]` at the same threshold accept
  exactly the same signatures (`count_distinct_signers` counts distinct signers among current members)
  but digest differently — `audit_p1_003_duplicate_members_do_not_collide_with_the_deduplicated_set`
  asserts precisely that non-collision. It is unreachable in practice: `PM-172-06` is an UNGATED
  well-formedness gate that fails closed on any duplicate entry at every `MaintainerState::load`, on
  both decode branches, and `crates/core/src/validation/tx_types.rs:831` records that
  "target is not already a maintainer" is enforced at the node level. Worth knowing, not worth changing.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 — [P1] and [P3] are comment/doc text; [P4] is a memory.db row; [P2] proposes no change (observed)
  Memory:   0 — no allocation, no data structure, no constant is added or removed by any proposed fix (observed)
  IO:       0 — one UPDATE against .omega/memory.db, executed once at commit time, not on any node path (observed)
  Network:  0 — no RPC field, no wire format and no gossip payload is touched (observed)
  Disk:     0 — no persisted node format is touched; the memory.db row is ~60 bytes (measured)
  Latency:  0 — none of the four fixes reaches a runtime path; the digest itself is unchanged by them (observed)
Inevitability: AVOIDABLE
Cheaper alternative: ship as is and carry [P1]–[P4] as follow-ups — none is behavioural, and the code
  is already green under the full gate
Why this proposal anyway: [P4] costs one UPDATE and closes the only remaining unanswered entry in the
  protection surface for this area, and [P1]/[P3] cost one sentence each while the context is loaded;
  deferring text corrections to a later session is how the 785-byte figure survived two milestones
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Modules Not Reviewed

Everything iteration 1 cleared — F4 routing correctness, F5 check order, F6 additivity, reduction
cleanliness — was deliberately not re-reviewed, per the scope of this pass. The full workspace suite
and workspace clippy were not re-run; the runner re-runs them before the commit.

## Security Audit Verdict

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: unchanged from iteration 1 — F5 is consensus-visible above an activation height testnet has
already crossed and bounds an attacker-controlled payload on a fee-exempt transaction; F6 publishes a
release-verification trust-root scalar over RPC and to the log; F4 changes mempool admission routing.
The DELTA this pass adds to the sweep's scope is narrow and fully enumerated: the digest preimage lost
one term (`crates/core/src/maintainer/digest.rs`), and every other edit was comment, doc, spec, test
name or test assertion. The digest has no consensus, persistence or updater consumer — per-root greps
give 4 RPC read sites in `crates` and 1 log site in `bins` — so the subtraction cannot move a
consensus-visible value.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Final Verdict

All six developer fixes landed as specified, plus the runner's [F1] registry repair. Nothing was
weakened: the sole surviving caps-consistency guarantee still runs a maximal fixture through the real
bincode encoder, the digest is still pinned as sensitive to member, threshold and genesis hash, and
the L1/L2 expressions are byte-identical to the base commit. The four findings above are text and
registry corrections; none blocks the commit, and none is behavioural.

VERDICT: APPROVED
