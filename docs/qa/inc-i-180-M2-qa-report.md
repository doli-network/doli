# QA Report — INC-I-180 M2 (mempool + builder parity for the withdrawal-holdings gate)

**Run 525 · incident INC-I-180 · branch `bugfix/inc-i-180-withdrawal-holdings-gate` · base `e6c066c7` (M1).**
Uncommitted milestone: `git diff e6c066c7` plus the untracked files listed in the implementation report §8b.

---

# CURRENT VERDICT (round 3 of max 3) — **PASS**

**No blocking issue remains.** ISSUE-003 and ISSUE-004 are both closed, and I closed them by
re-deriving each replacement sentence against the code rather than by reading the new wording and
agreeing with it. The parity lock the developer introduced is real: I reproduced **both** gate
mutations myself and each turned the exact rows red, with the exact numbers, that the developer
recorded — then restored `validation_checks.rs` byte-for-byte (`sha256 b411b0bb…`,
`git diff --numstat e6c066c7` back to `27 0`). The ISSUE-004 general rule ships in **five** places, I
found all five independently, and my round-2 `PROBE-CONT2` shape re-runs unchanged and is correctly
accounted for by it. No regression: **177 targets, 3728 passed, 3 failed, 43 ignored, zero compile
errors**, and the failing SET is byte-identical to the declared baseline with no fourth failure.

What remains is five non-blocking observations, all of them precision or coverage notes in prose. The
largest is that three shipped files attribute "eight allowance shapes" to a test function that drives
seven of them (the eighth is its sibling `…_at_the_ceiling`, whose name the attributed name is a
strict prefix of, so the claim is true when the name is used as the `cargo test` filter it was
measured with). None of them asserts a false safety property and none would make a reader less
careful. Blocking on them would be disproportionate.

Rounds 1 and 2 below are retained verbatim as history. The round-3 section is at
[Round 3](#round-3--final-verification-of-the-developers-qa-round-2-fixes).

| Item | Round-3 status |
|---|---|
| ISSUE-001 (HIGH, blocking) | **CLOSED** in round 2; re-verified structurally in round 3 |
| ISSUE-002 (MEDIUM, blocking) | **CLOSED** via ISSUE-003 + ISSUE-004 |
| ISSUE-003 (MEDIUM, blocking) | **CLOSED** — resolution (b) complete; every replacement sentence re-derived TRUE; the lock is real and I drove both mutations |
| ISSUE-004 (MEDIUM, blocking) | **CLOSED** — five copies found independently; rule TRUE; PROBE-CONT2 re-run and accounted for |
| OBS-001, OBS-002, OBS-003, OBS-005 | closed in round 2, unchanged |
| OBS-004, OBS-006, OBS-007, OBS-008, OBS-009 | carried forward, non-blocking |
| OBS-010 … OBS-014 | **NEW**, non-blocking (R3.7) |

---

## Verdict (round 2 — superseded, kept as history)

**FAIL.**

**ISSUE-001 (HIGH) is genuinely closed at the root**, verified by my own mutation testing rather than
on the implementer's word. **ISSUE-002's chosen resolution (a) is not**: the round-1 fix removed one
false claim from `specs/protocol.md` and `docs/architecture.md` and put **two new false claims** into
the same two paragraphs of the same two shipped files. Both are provable against the code — one by
`grep`, one by execution. Neither has any runtime consequence and neither needs a code change; each
is one to two sentences. **No regression: the workspace failing-target set is byte-identical to the
declared baseline, all round-1 PASS items still hold, and pre-activation byte-identity was re-probed
after the shared-arithmetic change and is intact.**

| Round-1 item | Round-2 status |
|---|---|
| ISSUE-001 (HIGH, blocking) | **CLOSED** — verified by MUTATION-1 / MUTATION-2 |
| ISSUE-002 (MEDIUM, blocking) | **NOT CLOSED** — replacement claims are false → ISSUE-003, ISSUE-004 |
| OBS-001 | **CLOSED** — verified by execution (PROBE-EMPTY) |
| OBS-002 | **CLOSED** — verified by source; move is behaviour-preserving |
| OBS-003 | **CLOSED** — third deploy answer present at §9 |
| OBS-004 | coverage note, unchanged; no action was required |
| OBS-005 | **CLOSED** — the overstated sentence is gone |

---

## Verdict (round 1 — superseded, kept as history)

**FAIL.**

The milestone's primary acceptance criterion — *"the builder must refuse to place a `RequestWithdrawal`
in a candidate block when the post-AH rule set would reject the assembled block … Enforce R0, R1, R4,
R3, R2"* (brief §S1) — is **not met for R1**. A reachable ledger shape makes the builder's allowance
strictly larger than the gate's, and the builder then assembles a block that the same node rejects.
This is the exact `[BLOCK_POISON]` / `rollback_one_block()` condition M2 exists to close, reproduced by
execution against the M2 tree (ISSUE-001). A second executed probe disproves the containment relation
that both `specs/protocol.md` and `docs/architecture.md` now state as fact (ISSUE-002).

Everything else in the milestone validated clean: S3, S5, S6, pre-AH invariance, INV-PROD-002,
lock-order, deadlock-freedom, and the S4 routing decision.

**Nothing is broken on any live network today.** AH #23 is `u64::MAX` on mainnet and `230_000` on
testnet (unreached), so both issues are post-activation only. Both fixes are node-local (no activation
height, no version bump, no synchronized deploy).

---

## Scope validated

| Item | Covered |
|---|---|
| S1 builder parity (R0, R1, R4, R3, both R2 shapes, F6) | yes — by execution |
| S2 mempool admission parity + `revalidate` eviction | yes — by execution |
| S3 `ValidationMode::Replay` carve-out (Full/Light invariance, R1/R4 strictness, allowance charge) | yes — by execution |
| S4 AUDIT-P2-004 routing decision | yes — judged, not re-litigated |
| S5 AUDIT-P1-004 rebuild mirror (both height forms) | yes — by execution + term-by-term source comparison |
| S6 OBS-001 | yes — discharged by S1+S2 modulo ISSUE-001/002 |
| INV-VALIDATION-001 / INV-PROD-003 three-path parity | yes — **failed**, see ISSUE-001 |
| INV-PROD-002 (skip never becomes failure/abort/rollback) | yes — pass |
| Pre-AH byte-identity | yes — pass for the gate/builder/mempool; see OBS-003 for the S5 arm |
| REQ-I180-003 acceptance criteria | yes — pass |

Not validated: live-network behaviour (AH unreached on every network; nothing to observe), and the
oracle/DeFi/NFT surfaces untouched by this milestone.

## System entrypoint

Validation was executed against the workspace test harness, not a running fleet: AH #23 is unreached on
every network, so no live node exercises any code path this milestone adds. Commands used:

```
cargo test --workspace --no-fail-fast          # full suite, 177 targets
cargo test -p doli-node --test it              # milestone suite
cargo test -p mempool --test it                # milestone suite
cargo test -p doli-node --test it qa_probe -- --nocapture --test-threads=1
```

`Node::new_for_test` (devnet, `withdrawal_holdings_gate_activation_height = 20`) is the harness; the
probe drives `build_block_content`, `validate_block_economics` and `Mempool::add_transaction` on one
node, which is the only configuration in which "the builder built a block this node rejects" is
directly observable.

---

## Test-suite results

| Suite | Result |
|---|---|
| `cargo test -p doli-node --test it` | **57 passed / 0 failed** |
| `cargo test -p mempool --test it` | **6 passed / 0 failed** |
| `cargo test --workspace --no-fail-fast` | 177 targets, **3707 passed / 43 ignored** |

Failing-target set on the workspace run: `doli-node --test checkpoint_rotation`,
`doli-node --test test_network`, `mempool --lib` — **exactly the declared baseline set, no fourth
target**. Re-run in isolation, each yields exactly one failure:

```
mempool --lib            : contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity
checkpoint_rotation      : test_network::test_cluster_10x100
test_network             : test_cluster_10x100
```

Under full-workspace parallelism the two `doli-node` network targets shed additional
`test_network::*` cases; `grep -ci "Too many open files"` on the run log returns **12**, so those are
EMFILE resource exhaustion in the same environmental class as the declared baseline, not M2 regressions.
**No M2-attributable failure exists.**

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo build --release`
were measured clean by the runner and were not re-established.

---

## Blocking issues

### ISSUE-001 — the builder's R1 allowance saturates in a different order than the gate's, so it still assembles blocks the gate rejects · **HIGH** · blocking

**Location:** `bins/node/src/node/production/withdrawal_holdings.rs:84-92` vs
`bins/node/src/node/validation_checks.rs:771-776`.

The two formulas use the same terms in a different saturating order:

```
gate    : bond_count .sat_add(pending_addbond) .sat_add(in_block_addbond) .sat_sub(withdrawal_pending) .sat_sub(in_block_withdrawn)
builder : ProducerHoldings::allowance()  // = bond_count.sat_add(pending_addbond).sat_sub(withdrawal_pending)
          .sat_add(in_block_addbond) .sat_sub(in_block_withdrawn)
```

`saturating_sub` does not commute with `saturating_add` across a clamp. Whenever
`withdrawal_pending > bond_count + pending_addbond` **and** an `AddBond` for the same producer sits at a
lower index in the candidate block, the builder's allowance is larger than the gate's by exactly
`withdrawal_pending - (bond_count + pending_addbond)`. The builder then selects a withdrawal the gate
rejects with `[ECON_WITHDRAWAL_OVER_HOLDINGS]`.

**The precondition is reachable on chain, not synthetic.** `apply_block/tx_processing.rs:266-271` does
`producer.withdrawal_pending_count += all_bonds` while re-reading an **unchanged** `bond_count`, so two
`Exit` transactions for one producer inside one epoch drive `withdrawal_pending` to `2 × bond_count`.
The M1 gate documents that double charge as deliberate parity
(`validation_checks.rs:723-728`). `validate_exit_data`
(`crates/core/src/validation/tx_types.rs:11-40`) checks structure only.

**Consequence.** This is the defect the milestone exists to close, at one remove: the block never
confirms, so no fee is paid and no input is spent, while every producer that selects it burns a block
build and runs `rollback_one_block()` — INV-VALIDATION-001's `[BLOCK_POISON]` monitoring signal, and
INV-PROD-003 verbatim.

**Observed** (`bins/node/tests/it/qa_i180_m2_probe.rs::qa_probe_builder_selects_a_block_the_gate_rejects`):

```
PROBE-SAT: addbond_selected=true withdrawal_selected=true
gate_verdict=Err([ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at height=1000007
  producer=2333100a… requests 8 bonds but allowance is 0
  (held=12, pending_addbond=0, in_block_addbond=10, withdrawal_pending=24, in_block_withdrawn=0))
```

The builder placed **both** the `AddBond` and the withdrawal; `validate_block_economics` on the block
this node just built returned `Err`.

**Reproduction:**
1. add `mod qa_i180_m2_probe;` to `bins/node/tests/it/main.rs`
2. `cargo test -p doli-node --test it qa_probe_builder_selects_a_block_the_gate_rejects -- --nocapture`

**Expected fix (developer's call, not QA's):** make the builder use the gate's saturating order
verbatim, e.g. give `ProducerHoldings` an `allowance_with(in_block_addbond, in_block_withdrawn)` that
adds every credit before subtracting any debit, and have both the builder and the mempool call it.
`ProducerHoldings::allowance()` with both terms at zero is already order-equivalent to the gate, so the
mempool needs no change for this issue. NODE-LOCAL; no activation height, no version bump, no
synchronized deploy.

**Coverage gap that let it through:** the S1 suite has no partition in which the builder's
`in_block_addbond` term is non-zero — `bins/node/tests/it/inc_i_180_builder_parity.rs` contains no
`AddBond` and no `Exit` transaction, so the in-block accounting the brief calls out as mandatory
(§S1, third hard constraint) is exercised only through `RequestWithdrawal`. Any fix must add an
`AddBond`-bearing and an `Exit`-bearing partition.

---

### ISSUE-002 — `mempool-reject ⊆ builder-skip` is false; admission censors a withdrawal a legal block carries · **MEDIUM** · blocking (documented claim is wrong in two shipped files)

**Location:** `crates/mempool/src/withdrawal_holdings.rs:45`;
claimed as fact in `specs/protocol.md` ("the containment relation is
`mempool-reject ⊆ builder-skip ⊆ consensus-reject` — weaker is correct, stronger would evict
transactions a real block can carry") and in `docs/architecture.md`.

The mempool evaluates the M1 rule table "with every `in_block_*` term at zero". That is the correct
choice for `in_block_withdrawn` and for R4, which only ever **reduce** the allowance or add a
constraint. It is the wrong choice for `in_block_addbond`, which **raises** the allowance: setting it to
zero makes admission STRICTER than the block rule, not weaker. Admission therefore rejects the
`[AddBond(P, +n), RequestWithdrawal(P, d)]` shape whenever `d` exceeds the flushed allowance — a shape
the gate accepts and a real block can carry.

**Observed** (`qa_probe_admission_rejects_what_a_legal_block_carries`), producer with
`bond_count = 1`, block `[coinbase, AddBond(P, +5), RequestWithdrawal(P, 4, 4 owned Bond inputs)]`:

```
PROBE-CONT:
admission = Err("invalid transaction: [ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal
                 at height=1000007 producer=66f8ed1e… requests 4 bonds but allowance is 1
                 (held=1, pending_addbond=0, withdrawal_pending=0, in_mempool_withdrawn=0)")
gate      = Ok(())
```

**Consequence.** Post-AH liveness/censorship: an operator who submits `AddBond` and
`RequestWithdrawal` in the same batch has the withdrawal dropped by every honest node's mempool, so it
never reaches a builder. Bounded — the operator can resubmit after the `AddBond` confirms, at which
point `pending_addbond_count` covers it, which is the M1 in-window-operator path
(`req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable`). No safety impact, no poison.

**Why the existing test does not catch it.** `req_i180_003_admission_is_contained_in_the_gate` compares
admission against `single_tx_gate_reference`, "an independent re-implementation of the decidable subset
of the M1 rule table … with every `in_block_*` term at zero" (test plan §4.2). That oracle is the
mempool's own model, so the assertion is `mempool == mempool-model`, not
`mempool-reject ⊆ builder-skip`. The containment relation is **asserted in prose and in two shipped
docs, and tested nowhere**.

**Reproduction:**
1. add `mod qa_i180_m2_probe;` to `bins/node/tests/it/main.rs`
2. `cargo test -p doli-node --test it qa_probe_admission_rejects_what_a_legal_block_carries -- --nocapture`

**Two acceptable resolutions:** (a) accept the over-rejection and correct both docs to state the true
relation (`mempool-reject ⊄ builder-skip` for allowance-raising in-block terms) plus the operator
consequence; or (b) make admission ignore the `AddBond`-window shortfall. (a) is the cheaper and
honest choice; either way the false claim must leave `specs/protocol.md` and `docs/architecture.md`.

---

## Non-blocking observations

- **OBS-001 — the `Unavailable` fallback is fail-open only where the snapshot is seeded.**
  `HoldingsSources::lookup` (`crates/mempool/src/holdings.rs:59-75`) falls back to the published
  snapshot when `try_read()` fails, and **absence from the snapshot is the R0 condition**. Only
  `Node::new` seeds the snapshot (`init.rs:740-752`); `Node::new_for_test` (`init.rs:1141`) and
  `Node::new_for_replay` (`init.rs:1353`) wire an empty `Vec` that is never seeded before the first
  `apply_block`. In those two constructors a contended `try_read()` makes **every** producer read as
  `Unregistered` — over-rejection, i.e. fail-**closed**, contradicting the implementation report §3
  ("Fail-open: over-rejection at this layer is censorship"). Not production-reachable through
  `Node::new`. Recommend seeding the snapshot in all three constructors, or distinguishing
  "snapshot empty" from "producer absent".

- **OBS-002 — `revalidate` runs one block before the snapshot it may fall back to is refreshed.**
  `apply_block/mod.rs:274` calls `mempool.revalidate(...)`; `refresh_mempool_producer_snapshot` is
  called at `apply_block/mod.rs:382`. The producer write guard taken at `:198` is released before
  `:274`, so `try_read()` normally succeeds and the live set answers. But whenever it does not, the
  fallback snapshot at that call site is **guaranteed** to be one block stale — exactly the state
  `revalidate` exists to shed. This is the same window that widens ISSUE-001's exploitation surface.
  Recommend moving the refresh above the `revalidate` block.

- **OBS-003 — the S5 rebuild mirror changes pre-activation output, and it is declared but not gated.**
  The `rewards.rs` auto-revoke mirror is not height-gated (correctly — the live branch is not either),
  so it changes `rebuild_producer_set_from_blocks` output **below** AH #23, where the mirror runs on
  every network today (mainnet AH = `u64::MAX`). REQ-I180-003's second criterion reads
  *"pre-activation behaviour bit-identical to `ca0b3093`"*. The implementation report §1 states this
  plainly ("Below AH #23 the fix still changes rebuild output — toward live") and the disproof attempt
  is sound: `received_delegations` is inside `serialize_canonical()`, so a rebuilt node holding the
  un-revoked value already diverges from every live node on `psHash`, and the rebuild runs only on
  reorg/recovery, so the fleet never converged on the pre-fix value. **Assessment: correct, but it is a
  pre-AH-visible change to a state-root-feeding path and should be surfaced explicitly at the deploy
  gate rather than folded into "no consensus change".**

- **OBS-004 — the S5 mirror's `in_flight > 0` cell is untested.**
  `audit_p1_004_rebuild_matches_live_for_a_delegated_full_exit` runs with `pending_addbond_count = 0`,
  so the arithmetic it exercises is identical to the pre-AH form. Both forms are nevertheless
  reproduced: `in_flight` is height-gated by the same expression in both live
  (`tx_processing.rs:388-398`) and rebuild (`rewards.rs:1379-1389`), and the branch condition is
  algebraically identical to live in both (see the S5 verification below). No defect; a coverage note.

- **OBS-005 — the S4 residual-exposure headline overstates, then corrects itself.**
  `docs/.workflow/inc-i-180-M2-audit-p2-004-routing.md` writes "A hostile snapshot peer already
  controls the ENTIRE `ProducerSet` and `UtxoSet` the syncing node installs." Read alone that is
  false — those objects ARE cross-checked by the state root, which is what makes `pending_updates`'
  exclusion the finding. The very next clause states the accurate form ("minus the state-root
  cross-check the object's other fields get … one field lacks the integrity check its siblings have").
  Recommend deleting the strong sentence; the corrected form carries the whole argument.

---

## Acceptance criteria — REQ-I180-003 (Must)

Source: `docs/bugfixes/inc-i-180-n11-zero-bond-active-set-analysis.md:362`.

| Criterion | Result | Evidence |
|---|---|---|
| New AH field in `crates/core/src/network_params/`, mainnet above tip; no existing AH reused or moved | **PASS** | `git diff --stat e6c066c7 -- crates/core/src/network_params/` is empty. M2 adds no height and moves none; every consensus-visible change rides AH #23 from M1. |
| Pre-activation behaviour bit-identical to `ca0b3093` | **PASS** (with OBS-003) | `git diff --numstat e6c066c7 -- validation_checks.rs` = `27 0` — zero deletions; both S3 guards sit inside `if height >= withdrawal_gate_ah`. Builder `WithdrawalParity::new(false, h)` short-circuits; `withdrawal_holdings_verdict` returns `Ok` below the AH. Probe `qa_probe_pre_activation_still_selects` selects the withdrawal at `PRE_AH` even on the ISSUE-001 ledger. The one pre-AH-visible change is the S5 recovery-path repair (OBS-003). |
| Three-question checklist answered in the commit body | **PASS (material ready)** | Implementation report §9 carries the three-question checklist, both deploy questions, `Path-Coverage:` and `Failure-Modes:`. Not yet committed — the runner must transcribe it. |

## Traceability

`REQ-I180-*` is not collected by the coverage gate, so this was checked manually against the
implementation report §1 and the test-plan §2 matrix.

| Brief item | Tests exist | Tests pass | Acceptance met |
|---|---|---|---|
| S1 builder parity | yes (6 tests) | yes | **NO** — R1 hole (ISSUE-001); no `AddBond`/`Exit` partition |
| S2 mempool admission | yes (6 tests) | yes | **NO** — containment claim false (ISSUE-002) |
| S3 Replay carve-out | yes (4 tests) | yes | yes |
| S4 AUDIT-P2-004 | routed out, no test (correct) | n/a | yes |
| S5 rebuild mirror | yes (1 new + 2 existing) | yes | yes (OBS-003/004) |
| S6 OBS-001 | discharged by S1+S2 | — | follows S1/S2 |

Gaps found:
- No test drives the builder's `in_block_addbond` or `in_block_withdrawn`-via-`Exit` accounting.
- The `mempool-reject ⊆ builder-skip` inclusion is documented in two shipped files and asserted by no
  test (the existing test compares admission against a model of admission).

---

## Detailed validation results

### 1. S3 — Replay carve-out (PASS)

- Guard placement verified by source read: the R0 guard is inside the `let ... else`
  (`validation_checks.rs:750-760`), the second guard sits **after** R4 and **before** R3
  (`:822-837`). Both are `mode == ValidationMode::Replay`, so `Full` and `Light` are untouched by
  construction and are locked by `req_i180_003_full_and_light_verdicts_are_unchanged_by_the_carve_out`.
- Carve-out covers R0, R3 and both R2 shapes; **R1 and R4 stay strict in all three modes** — R1 reads
  only the ProducerSet allowance, R4 reads `earlier_tx_hashes`, which Replay has in full. Correct split.
- **The developer's claim that the second guard must charge `in_block_withdrawn` is verified by
  execution.** Probe `qa_probe_replay_charges_the_allowance` replays
  `[wd(P,2), wd(P,3)]` against `bond_count = 4` with unresolvable inputs:

  ```
  PROBE-CHRG: Err([ECON_WITHDRAWAL_OVER_HOLDINGS] … requests 3 bonds but allowance is 2
    (held=4, pending_addbond=0, in_block_addbond=0, withdrawal_pending=0, in_block_withdrawn=2))
  ```

  `in_block_withdrawn=2` proves the charge survives the `continue`. Without it the second withdrawal
  would have seen allowance 4 and been admitted — R1 drift, exactly as claimed.
- The R0 carve-out does **not** charge, and correctly so: an unregistered producer never reaches the
  allowance computation, so no drift is possible.

### 2. S5 — rebuild parity, both height forms (PASS)

The mirror (`rewards.rs:1406-1416`) is term-for-term the live branch
(`tx_processing.rs:401-406`), with the live outer `if` folded into the condition:

| live | rebuild |
|---|---|
| `remaining = held.sat_add(in_flight).sat_sub(wp)` | `available = bond_count.sat_add(in_flight).sat_sub(wp)` (same value) |
| outer `if data.bond_count > available_live` where `available_live = remaining.sat_sub(delegated)` | third conjunct `data.bond_count > available.saturating_sub(delegated)` |
| `delegated > 0` | `delegated > 0` |
| `data.bond_count == remaining` | `data.bond_count == available` |

The seemingly redundant third conjunct is precisely the live outer guard, so the two conditions are
identical, including the `delegated > 0, bond_count == remaining == 0` edge (neither fires).
`in_flight` is computed by the same height-gated expression on both sides, so **both the pre-AH form
(`in_flight = 0`) and the post-AH form are reproduced**, with no gate of the mirror's own — which is
right, because the live branch has none either. `audit_p1_004_rebuild_matches_live_for_a_delegated_full_exit`
asserts identical `delegated_bonds`, `received_delegations` **and** `serialize_canonical()` bytes, and
passes. See OBS-003/004.

### 3. INV-PROD-002 — refusal is a skip (PASS)

- `assembly.rs:324-333` is `warn!` + `continue`, sitting after the data-budget check and before
  `cumulative_user_bytes += tx_size`, so the in-block accounting counts only included transactions.
- `git diff --stat e6c066c7 -- bins/node/src/node/rollback.rs bins/node/src/node/block_handling.rs
  bins/node/src/node/apply_block/` is **empty**: the poison-rollback path — the fleet's only escape
  from a finality wedge — was not read, moved or altered.
- Every probe run through `build_at` `expect`s on `Err`; `build_block_content` returned `Ok` in every
  partition including the ISSUE-001 one. No build failure, no abort.
- `req_i180_003_skip_never_fails_aborts_or_rolls_back` additionally locks the receiver cells
  (producer-set canonical bytes, UTXO-set length, mempool length) and passes.

### 4. Item 8 — the `share_producer_set` / `try_read()` design deviation (ACCEPTED, with OBS-001/002)

The developer rejected the test plan's push-only snapshot and added a live handle read with
`try_read()`. Assessment:

- **Deadlock / blocking / livelock: none.** `tokio::sync::RwLock::try_read()` never blocks and never
  queues, so no admission call site can be parked behind an `apply_block` writer, and the mempool's
  synchronous context cannot deadlock against it. Verified by source
  (`crates/mempool/src/holdings.rs:60-64`). This is the correct primitive for the job.
- **Lock-order:** the builder resolves holdings under the producer guard alone
  (`assembly.rs:192-196`), drops it, then takes the UTXO guard. Only one guard is ever held, so
  selection does not join `apply_block`'s `utxo → producers` (`apply_block/mod.rs:197-198`) to
  `rollback`'s `producers → utxo` (`rollback.rs:324-326`). The developer's stated conclusion is
  correct and I verified both line references.
- **Sustained write contention degrades to the snapshot, not to no checking** — provided the snapshot
  is wired and non-empty, which is true for `Node::new`. See OBS-001 for the other two constructors.
- **The one-block staleness bound is enforced**: `refresh_mempool_producer_snapshot` is called from
  `apply_block/mod.rs:382`, i.e. once per applied block, under an awaited (not `try_`) read, so it
  cannot be starved. The bound holds, with the ordering caveat in OBS-002.
- **Fail-open on `Unavailable` is safe**: under-rejection at admission is caught by the builder and by
  consensus; over-rejection would be censorship. The direction chosen is the right one.

### 5. Item 9 — S4 routing decision quality (SOUND)

- **(a) add `pending_updates` to `serialize_canonical()`** — blast radius stated accurately: state root
  of every block on every network, a new AH of its own, a synchronized deploy, a new canonical
  ordering rule for an order-sensitive queue, and re-verification of `snapshot.rs` / snap-sync
  install+re-verify / checkpoint compare / fork choice. Correctly sized as its own incident.
- **(b) re-derive from the block range** — rejected for *availability*, not cost. Correct: a
  snap-synced node's store starts above the last epoch boundary, so it would refetch the range from
  the same untrusted peer. This is INV-EPOCH-002 applied verbatim.
- **(c) restructure the read** — claimed unavailable because dropping the `pending_addbond` term breaks
  M1's green `req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable`
  (`bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs:177` — verified present) and re-opens the
  in-window-operator liveness hole. I probed the remaining variant the doc dismisses in one line —
  keep the term but source it from inside the state root — and confirm it does not exist: no field in
  `serialize_canonical()` carries in-flight AddBonds, and treating an installed snapshot's
  `pending_updates` as empty would make a snap-synced node reject blocks the fleet accepts, which is
  strictly worse. **(c) is genuinely unavailable.** Fairly assessed.
- **Residual exposure**: honest in substance, overstated in one sentence — see OBS-005. The conclusion
  that this is a prerequisite for pinning a real mainnet value for AH #23 is correct and should be
  carried into the height-pinning session (AUDIT-P2-001/002).

### 6. Exploratory testing

| # | Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Builder at post-AH with `withdrawal_pending (24) > bond_count (12)` and an in-block `AddBond(+10)`, withdrawal declaring 8 | builder skips, or gate accepts | builder selected; gate rejected `[ECON_WITHDRAWAL_OVER_HOLDINGS]` | **high** → ISSUE-001 |
| 2 | Same ledger, build **below** AH #23 | withdrawal still selected (no censorship) | selected | none — pass |
| 3 | Admission of `RequestWithdrawal(P,4)` alongside `AddBond(P,+5)`, `bond_count = 1` | admitted (the block is legal) | rejected `[ECON_WITHDRAWAL_OVER_HOLDINGS]`, gate `Ok` | **medium** → ISSUE-002 |
| 4 | Two same-producer withdrawals in one block under `ValidationMode::Replay` | R1 stays strict and sees the charged allowance | `Err(… allowance is 2 … in_block_withdrawn=2)` | none — pass |
| 5 | `RequestWithdrawal` declaring 0 bonds with zero inputs, post-AH | some verdict | admission rejects earlier, at generic tx validation: `[ERRTX003] output 0 has zero amount (type=Normal)`; the gate itself returns `Ok` | none — no free no-op path |
| 6 | Withdrawal spending an output of the same block's coinbase (builder's R4 set omits the coinbase, the gate's includes it) | builder must not select it | unreachable: the coinbase outpoint does not exist in the pre-block UTXO view, so `validate_transaction_with_utxos` (which runs first, `assembly.rs:259`) skips it. Sound, not a gap | none |

Findings 1 and 3 were recorded to `.omega/memory.db` (`outcomes`, run 525) at the moment of
observation.

### 7. Failure-mode validation

| Scenario | Triggered | Detected | Degraded OK | Notes |
|---|---|---|---|---|
| Mempool holdings source contended (`try_read` fails) | Not triggered — no deterministic hook | n/a | **partly** | Falls back to a ≤1-block-stale snapshot (bound enforced at `apply_block/mod.rs:382`), then to `Unavailable` → admit. Safe direction, but see OBS-001 (empty snapshot ⇒ fail-closed) and OBS-002 (fallback guaranteed stale inside `apply_block`). |
| Builder skips a transaction the gate would accept | Yes (ISSUE-002 shape, at admission) | Yes | Yes | The transaction stays in the mempool — `req_i180_003_skip_never_fails_aborts_or_rolls_back` asserts the builder never evicts — and is re-offered next slot. |
| Builder selects a transaction the gate rejects | **Yes** | **No** | **No** | ISSUE-001. This is the failure mode the milestone was written to make impossible. |
| Mempool evicts a withdrawal that later becomes valid | Not triggered | n/a | Yes | Owner resubmits; no funds move, no input spent; eviction logged with the bracketed reason (`pool.rs:1293-1305`). |
| Replay carve-out on a genuinely invalid historical block | Yes (probe 4) | Yes | Yes | R1 and R4 still abort the reindex; the UTXO-bound rules log `[REPLAY_SKIP]` and continue. |
| Rebuild mirror fires where live did not | Not triggerable | n/a | Yes | Conditions proven identical term-by-term (§2 above). |

### 8. Security validation

| Surface | Test performed | Result |
|---|---|---|
| Free, unauthenticated block poison via `RequestWithdrawal` (Reviewer F1) | Drove the R0/R1/R4/R3/R2F/R2P/F6 partitions through admission → builder → gate; then drove the R1 saturation partition | **FAIL** for R1 (ISSUE-001); PASS for R0, R4, R3, both R2 shapes and F6 |
| Precondition reachability for ISSUE-001 | Source read of `tx_processing.rs:266-271` and `validation/tx_types.rs:11-40` | Reachable: `Exit` carries no signature requirement at the transaction layer, and apply's `+=` against an unchanged `bond_count` lets two Exits set `withdrawal_pending = 2 × bond_count` |
| Mempool amplification (a rejected tx costing more than it should) | `withdrawal_holdings_verdict` placement in `add_transaction` | PASS — placed after signature checks and **before** `calculate_inputs`/fee work, so a rejected withdrawal costs less, not more |
| Deadlock / lock-cycle on the producer↔utxo pair | Source read of `assembly.rs:180-199`, `apply_block/mod.rs:197-198`, `rollback.rs:324-326`, `holdings.rs:60-64` | PASS — one guard at a time in the builder; `try_read()` never queues |
| Snapshot-peer trust (AUDIT-P2-004) | Not probed — routed out of M2 | Out of scope; residual exposure recorded in the routing document and gated behind the AH-pinning session |

### 9. Specs / docs drift

| File | Documented behaviour | Actual behaviour | Severity |
|---|---|---|---|
| `specs/protocol.md` (§ "Admission and selection parity") | "the containment relation is `mempool-reject ⊆ builder-skip ⊆ consensus-reject` — weaker is correct, stronger would evict transactions a real block can carry" | The left inclusion is false: admission rejects the `[AddBond(P,+n), RequestWithdrawal(P,d)]` shape the gate accepts (ISSUE-002) | **medium** |
| `docs/architecture.md` (Builder parity paragraph) | same containment claim | same | **medium** |
| `specs/protocol.md` (§ builder bullet) | "**Block builder** … applies the whole table — including … the in-block `AddBond`/`Exit`/`RequestWithdrawal` accounting" | true in structure, but the R1 arithmetic diverges from the gate for `in_block_addbond` (ISSUE-001) | **high** |

Everything else added to `specs/protocol.md` and `docs/architecture.md` in this milestone matched the
code as read: the S3 mode split, the `[REPLAY_SKIP]` behaviour, the S5 auto-revoke mirror and its
height-dependence, the `revalidate` eviction, the fallback-vs-primary holdings ordering, and the
INV-PROD-002 skip semantics.

---

## Modules not validated

None within the M2 scope. Out-of-scope items (`AUDIT-P2-003`, `AUDIT-P2-001/002`, `AUDIT-P0-001`,
`AUDIT-P1-003`, `AUDIT-P1-005`, `FIND-I081-EPOCH-SKIP-001`, `AUDIT-P3-007`) were not touched, per brief
§3.

## QA artefacts left in the tree

`bins/node/tests/it/qa_i180_m2_probe.rs` — five probe tests, **not declared** in
`bins/node/tests/it/main.rs` (that file was restored to its M2 state and verified at `2 0` against
`e6c066c7`). Add `mod qa_i180_m2_probe;` to re-arm. This file is a QA artefact: it must **not** be
staged with the milestone, and ISSUE-001/002 need proper partitions inside
`inc_i_180_builder_parity.rs` and `crates/mempool/tests/it/inc_i_180_admission_parity.rs` as part of
the fix. No existing assertion was weakened or deleted; no version was bumped; no activation height was
added, moved or reused; nothing was committed.

## Final verdict

**FAIL** — REQ-I180-003 passes, but the milestone's own binding scope item S1 is not met: R1 parity
between the block builder and `validate_block_economics` is still open (ISSUE-001), reproducible by
execution, and it re-creates the free `rollback_one_block()` poison the milestone was written to close.
ISSUE-002 additionally makes a containment claim now shipped in `specs/protocol.md` and
`docs/architecture.md` false. Both are node-local fixes with no activation-height, version or
synchronized-deploy consequence, and both are post-AH only — no live network is affected today. S3, S4,
S5, S6, INV-PROD-002, pre-activation invariance, lock-order and deadlock-freedom all pass and need no
rework.

*(End of round-1 report. Round 2 follows.)*

---
---

# Round 2 — verification of the developer's QA round-1 fixes

**Run 525 · fix round 1 of max 3 reviewed · QA round 2 of max 3 · same branch, same base, still
uncommitted.** Scope: the fix only, per the round-2 brief. Source read: the developer's
`docs/.workflow/inc-i-180-M2-implementation.md` § "QA round 1 fixes" (lines 451-730).

## R2.0 — Gates and suites re-measured by me on the current tree

| Gate | Result |
|---|---|
| `cargo fmt --check` | **clean** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **clean** (no error, no warning) |
| `cargo test -p doli-node --test it` | **66 passed / 0 failed** |
| `cargo test -p mempool --test it` | **6 passed / 0 failed** |
| `cargo test --workspace --no-fail-fast` | 177 targets, **3716 passed / 13 failed / 43 ignored** |

**The 13 is an environment artefact, not a regression, and the failing-TARGET set is byte-identical
to the declared baseline.** The run reports `error: 3 targets failed:` — `doli-node --test
checkpoint_rotation`, `doli-node --test test_network`, `mempool --lib`. There is **no fourth target**.
Twelve of the thirteen failures are `Too many open files` panics at `bins/node/tests/test_network.rs:55`
(`Node {n} init failed: database error: IO error: … Too many open files`), i.e. RocksDB file-descriptor
exhaustion during cluster spin-up. Re-run single-threaded with a raised limit
(`ulimit -n 10240; cargo test -p doli-node --test test_network -- --test-threads=1`) the target drops
to **12 passed / 1 failed**, and the survivor is `test_onchain_liveness_10k_nodes`, again EMFILE.
Notably `test_cluster_10x100` — the brief's declared baseline failure — **passes** under
single-threading, which shows *which* network test loses the descriptor race is nondeterministic and
that none of them is an assertion failure.

The one non-environmental failure is `mempool --lib ::
contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity`, panicking at
`crates/mempool/src/contention_tests.rs:1108` with *"Below inc_i_096 gate, RemoveLiquidity DOLI-outflow
must be rejected"* — a DeFi contention gate, unrelated to withdrawals.
`git diff --numstat e6c066c7 -- crates/mempool/src/contention_tests.rs` is empty. **Declared baseline,
not M2-attributable.** Verdict on brief item 5: **PASS — the failing set is the baseline set and there
is no fourth failure.**

*(My round-1 run recorded 12 EMFILE lines and my round-2 run recorded 12 as well; the developer's run
recorded 2. The count tracks machine load at the time, not the tree.)*

---

## R2.1 — ISSUE-001: closed at the root, and I proved it myself

### The fix, read

`ProducerHoldings::allowance_with(in_block_addbond, in_block_withdrawn)`
(`crates/mempool/src/holdings.rs:34-40`) is every-credit-then-every-debit:

```rust
self.bond_count
    .saturating_add(self.pending_addbond)
    .saturating_add(in_block_addbond)
    .saturating_sub(self.withdrawal_pending)
    .saturating_sub(in_block_withdrawn)
```

`bins/node/src/node/production/withdrawal_holdings.rs:84-87` now calls it; the two chained saturating
ops are gone. `crates/mempool/src/withdrawal_holdings.rs:50` calls `allowance_with(0,
in_mempool_withdrawn)`.

### MUTATION-1 — FAIL→PASS evidence, established independently

I reverted **only** the builder line to the pre-fix chained form and re-ran the whole node `it` target:

```
$ cargo test -p doli-node --test it --no-fail-fast
test result: FAILED. 65 passed; 1 failed
  RED: inc_i_180_in_block_parity::inc_i180_m2_builder_skips_when_the_allowance_clamps

panicked at bins/node/tests/it/inc_i_180_in_block_parity.rs:333:5:
O3/INV-PROD-003: the builder assembled a block this same node's gate rejects. … :
Err([ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at height=1000007 producer=744ffb9f…
requests 8 bonds but allowance is 0 (held=12, pending_addbond=0, in_block_addbond=10,
withdrawal_pending=24, in_block_withdrawn=0))
```

That is **my round-1 probe's message, term for term** (`held=12, in_block_addbond=10,
withdrawal_pending=24, allowance 0`). The new partition drives exactly the shape I reported, it is RED
against the pre-fix arithmetic, and it is GREEN against the fix. File restored byte-for-byte
(`shasum` match) afterwards.

### MUTATION-2 — the fix cannot be degraded into an over-fix

Replacing the builder's `in_block_addbond` argument with a literal `0`:

```
test result: FAILED. 65 passed; 1 failed
  RED: inc_i_180_in_block_parity::inc_i180_m2_builder_credits_the_in_block_addbond
```

The IP-CREDIT row is a genuine mutation detector, exactly as the developer claims.

### Reachability of the precondition is now EXECUTED, not asserted

`inc_i180_m2_two_exits_charge_the_allowance_twice` derives `withdrawal_pending = 24` against
`bond_count = 12` by pushing two real `Exit` transactions through
`process_transaction_producer_effects`. This is the right fix for my round-1 complaint that the
partition rested on a hand-written ledger.

**ISSUE-001: CLOSED.**

---

## R2.2 — ISSUE-003 (NEW, MEDIUM, blocking) — "one function at every layer" is false: the gate is still a second transcription

**This is the exact check the round-2 brief asked for, and it fails.**

`grep -rn "allowance_with\|\.allowance()" --include='*.rs' bins crates` returns **four** lines, and the
consensus gate is not among them:

```
bins/node/src/node/production/withdrawal_holdings.rs:84    info.allowance_with(...)   ← builder
crates/mempool/src/holdings.rs:34                          fn allowance_with(...)     ← definition
crates/mempool/src/holdings.rs:44                          self.allowance_with(0, 0)  ← allowance()
crates/mempool/src/withdrawal_holdings.rs:50               holdings.allowance_with(…) ← admission
```

`validate_block_economics` still computes R1 inline at
`bins/node/src/node/validation_checks.rs:771-776`:

```rust
let allowance = info
    .bond_count
    .saturating_add(producers.pending_addbond_count(pk))
    .saturating_add(prior_add)
    .saturating_sub(info.withdrawal_pending_count)
    .saturating_sub(prior_wd);
```

`git diff --numstat e6c066c7 -- bins/node/src/node/validation_checks.rs` is `27 0` — the gate was not
touched this round, so **two transcriptions that currently agree** is precisely the state ISSUE-001's
lesson warns about. Two of three layers share the function; the third — the one that owns the
reference semantics — does not.

**What is false, and where it ships:**

| File | Sentence | Truth |
|---|---|---|
| `specs/protocol.md` (builder bullet) | "All three layers compute the allowance through the single `ProducerHoldings::allowance_with` function, never by re-expressing its terms" | Two layers do. The gate re-expresses the terms. |
| `docs/architecture.md` (Builder parity paragraph) | "The builder computes the allowance through the SAME function the gate calls (`ProducerHoldings::allowance_with`)" | The gate calls no such function. |
| `docs/.workflow/inc-i-180-M2-implementation.md:479-481, 649-652` | "the ONLY place the allowance is computed at any layer"; "CLOSED by construction, not by agreement: both call `ProducerHoldings::allowance_with`, so there is no second expression that can drift" | The safety property is not construction-level. It is agreement-level, backed by one test row. |

**MUTATION-3 — how much protection actually exists.** I drifted the **gate's** transcription into the
pre-fix order (`…add(pending).sub(wp).add(prior_add).sub(prior_wd)`) and re-ran the target:

```
test result: FAILED. 65 passed; 1 failed
  RED: inc_i_180_in_block_parity::inc_i180_m2_the_gate_rejects_the_clamped_withdrawal
```

Exactly one row catches it, and it is the **harness self-check** (`assert!(msg.contains("allowance is
0"))`), not a parity assertion. So the risk is real but bounded: gate drift in the clamping direction
is caught; gate drift in any shape that row does not cover is not. `validation_checks.rs` restored
byte-for-byte afterwards (verified: `git diff --numstat` back to `27 0`, lines 771-776 re-read).

**Severity MEDIUM, blocking by the same standard round-1 ISSUE-002 was blocked under** ("a documented
claim is wrong in two shipped files"). No runtime defect today.

**Two acceptable resolutions (developer's call):**
(a) route the gate through `allowance_with` too — feasible (`bins/node` already depends on `mempool`,
and `mempool::holdings::of_producer_set` already returns the struct), behaviour-identical by
construction, but it edits consensus code and must be re-measured; or
(b) correct the three statements to what is true: *the builder and the mempool share
`ProducerHoldings::allowance_with`; the gate holds the reference expression; parity between them is
locked by `inc_i180_m2_builder_skips_when_the_allowance_clamps` and
`inc_i180_m2_the_gate_rejects_the_clamped_withdrawal`.* (b) is one sentence per file and costs nothing.

---

## R2.3 — ISSUE-004 (NEW, MEDIUM, blocking) — the replacement containment claim is also false, and the premise under it is wrong

The round-1 ISSUE-002 resolution (a) was the right choice — the reasoning for rejecting (b) at
implementation-report lines 505-511 is sound, and the operator consequence is now stated. But the
replacement text is not true.

**Shipped claim** (`specs/protocol.md`, mempool bullet; mirrored in `docs/architecture.md` and in the
`crates/mempool/src/withdrawal_holdings.rs:1-12` module header):

> "Zeroing an in-block term is weaker than the block rule when the term LOWERS the allowance
> (`in_block_withdrawn`) or adds a constraint (R4), and **stricter** when it RAISES the allowance
> (`in_block_addbond`). So `mempool-reject ⊆ consensus-reject` holds for every shape except one:
> `[AddBond(P, +n), RequestWithdrawal(P, d)]` with `d` above the flushed allowance…"

**Both halves are wrong.**

1. **The premise.** Admission does **not** zero `in_block_withdrawn`. `pool.rs:566-571` calls
   `withdrawal_holdings_verdict(..., count_residents = true)`, which substitutes
   `withdrawal_holdings::resident_withdrawn(...)` — the sum over **every** same-producer
   `RequestWithdrawal` this mempool holds. That is not a zeroed term; it is a stand-in that can
   **exceed** any block's `in_block_withdrawn`, because a block need not carry the residents.
2. **The enumeration.** "Except one" is wrong. Here is a second exception with **no `AddBond`
   anywhere**, executed on one node:

```
PROBE-CONT2  (P: bond_count=6, withdrawal_pending=0, pending_addbond=0, 6 owned Bond UTXOs)
  wd1(P, declares 4, spends UTXO 0..4)  admission = Ok(())
  wd2(P, declares 2, spends UTXO 4..6)  admission = Err("[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]
      RequestWithdrawal at height=1000007 producer=104e5a5d… declares its full allowance of
      2 bonds but spends 2 of the 6 Bond UTXOs it owns")
  gate on a block carrying wd2 ALONE   = Ok(())
  EXCEPTION_WITHOUT_ADDBOND = true
```

Mechanism: the resident charge drops the admission allowance to `6 − 4 = 2`, which equals the declared
count, which flips R2 from its partial shape into its **full-exit** shape and measures the declared
count against all 6 still-live Bond UTXOs. The gate, evaluating a block that carries wd2 alone, reads
allowance 6, takes the partial branch, and accepts. Bounded and safe in exactly the same way as the
`AddBond` window (wd1 confirms or expires, then the operator resubmits; no fee, no input spent, no
poison) — but it is not the shape the spec names, and a reader who trusts "except one" will
mis-reason about mempool liveness.

**Resolution:** correct the sentence in `specs/protocol.md`, `docs/architecture.md` and the
`crates/mempool/src/withdrawal_holdings.rs` module header to state the general rule — *admission
substitutes mempool-wide state for block-local state, so it over-rejects whenever the substitute
raises the block's allowance (`in_block_addbond → 0`) or exceeds the block's debit
(`in_mempool_withdrawn > in_block_withdrawn`); every such over-rejection is one confirmation deep and
costs no fee and no input* — rather than enumerating a single shape. No code change is required, and
none is recommended: the resident charge is what stops this mempool from ever offering a builder the
`[partial(P), full-exit(P)]` pair (SEC-FIXVERIFY2-001), which is a property worth keeping.

*(Probe file `bins/node/tests/it/qa_i180_m2_r2_probe.rs` was written, run, and **deleted**;
`bins/node/tests/it/main.rs` was restored and verified byte-identical by `shasum`. Nothing dangles.)*

---

## R2.4 — the nine new partitions: which are regression detectors and which are locks

Brief item 2 asks me to confirm each partition would FAIL against pre-fix code. Measured, not assumed:

| Test | RED under MUT-1 (pre-fix order) | RED under MUT-2 (credit dropped) | RED under MUT-3 (gate drifts) | What it really is |
|---|---|---|---|---|
| `inc_i180_m2_builder_skips_when_the_allowance_clamps` | **YES** | no | no | **the ISSUE-001 regression** |
| `inc_i180_m2_builder_credits_the_in_block_addbond` | no | **YES** | no | over-fix detector |
| `inc_i180_m2_the_gate_rejects_the_clamped_withdrawal` | no | no | **YES** | harness self-check + the only gate-drift detector |
| `inc_i180_m2_two_exits_charge_the_allowance_twice` | no | no | no | reachability derivation (apply's Exit arm, unchanged code) |
| `inc_i180_m2_pre_activation_still_selects_the_clamped_withdrawal` | no | no | no | pre-AH invariance lock |
| `inc_i180_m2_an_in_block_exit_never_splits_builder_from_gate` | no | no | no | `allows_empty_io` lock + Exit coverage |
| `inc_i180_m2_admission_over_rejects_the_addbond_window` | no | no | no | ISSUE-002 truth lock, both layers driven |
| `inc_i180_m2_the_operator_resubmits_once_the_addbond_flushes` | no | no | no | bound lock |
| `inc_i180_m2_admission_reject_implies_gate_reject_without_a_credit` | no | no | no | containment-half lock, both layers driven |

**Assessment: legitimate.** Three of the nine are mutation detectors. The other six are locks on
statements that were previously asserted only in prose — which is what round 1 asked for. None is
vacuous: the two IP-CONT/IP-LOWER rows drive `Mempool::add_transaction` **and**
`validate_block_economics` on one node (my round-1 complaint was that
`req_i180_003_admission_is_contained_in_the_gate` compared admission against a model of admission —
that row is untouched and its doc comment now states its true scope, `crates/mempool/tests/it/
inc_i_180_admission_parity.rs:624-632`). The `AddBond`-bearing coverage the brief's §S1 third hard
constraint requires is genuinely exercised: MUTATION-2 proves `in_block_addbond` is load-bearing.

**`Exit`-bearing coverage — accepted with the developer's disclosure, verified.** IP-EXIT does not
drive the builder's `Exit` charge, because `TxType::Exit.allows_empty_io()` is `false`, so
`validate_transaction_with_utxos` refuses a bare `Exit` at `assembly.rs:259` — the first skip gate,
before the withdrawal gate at `:324`. I confirmed the mechanism at
`crates/core/src/transaction/types.rs` and `crates/core/src/validation/utxo.rs`. Recording the
disproved assumption rather than retuning the test is the right call, and the assertion
`!TxType::Exit.allows_empty_io()` converts a future `allows_empty_io` change into a test failure. The
gate-side `[Exit(P), RequestWithdrawal(P,d)]` order stays locked by M1's `inc_i_180_gate_bindings.rs`.
**Consequence, restated so it is not lost: `WithdrawalParity::accept`'s `Exit` arm is dead code
through mempool selection today.**

**Probe cleanup verified.** `bins/node/tests/it/qa_i180_m2_probe.rs` is gone; `main.rs` is `3 0`
against `e6c066c7` (three module declarations, one of them this round's) and no `qa_*` file remains in
`bins/node/tests/it/`.

---

## R2.5 — OBS-001, OBS-002, OBS-003, OBS-005

**OBS-001 — CLOSED, verified by execution.** `HoldingsSources::lookup`
(`crates/mempool/src/holdings.rs:90-92`) now returns `Unavailable` for an EMPTY published snapshot.
I drove the fail-open path directly on a `Node::new_for_test` node by holding the live
`producer_set` write guard across admission, forcing `try_read()` to fail:

```
PROBE-EMPTY: contended_live + empty_snapshot verdict=Ok(())
```

The transaction declares 99 bonds against 1 held — it would be rejected outright by any source that
answered. It was admitted, so no source answered: `Unavailable`, i.e. fail-**open**, which is what §3
claims. Pre-fix this was `Unregistered` → reject → fail-closed censorship under `new_for_test` /
`new_for_replay`. The choice of `is_empty()` over seeding all three constructors is well argued and I
agree with it. The mempool `it` suite still exercises the R0 partition against a **populated**
snapshot (`wired()` at `crates/mempool/tests/it/inc_i_180_admission_parity.rs:348-352` seeds
`case.holdings`, and the R0 row still asserts `[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]` and passes) — so
the new branch did not silently disable an existing assertion. See OBS-006 below for the residual.

**OBS-002 — CLOSED, verified by source, and the one behavioural delta is smaller than stated.** The
`remove_for_block` + `revalidate` block moved from `apply_block/mod.rs:265` to `:375`, immediately
after `refresh_mempool_producer_snapshot(height)` at `:372`. I checked the two things that could have
gone wrong:
- *Does `revalidate`'s `height` argument change?* **No.** `update_chain_state_for_block` is called at
  `:241`, above **both** positions, so `self.chain_state.read().await.best_height` is the new height
  at either site. No AH-boundary shift, no maturity-window shift.
- *Does anything between the two positions read the mempool?* **No.** `grep -rn "self\.mempool"
  bins/node/src/node/apply_block/` returns exactly one line — `:379`, the moved block itself.

The declared delta (on a `put_block` / `batch.commit()` error the block's transactions now stay in the
mempool) is correct and is the safe direction. Rejecting the "move the refresh up" alternative on the
AUDIT-P1-001 measurement is sound.

**OBS-003 — CLOSED.** Implementation report §9 now carries a third deploy answer (lines 407-415)
stating that the `rewards.rs` auto-revoke mirror changes `rebuild_producer_set_from_blocks` output
**below** AH #23 on every network today, why it is deliberately ungated, and that it must be weighed
at the deploy gate rather than waved through. That is exactly what I asked for.

**OBS-005 — CLOSED.** `grep -n "ENTIRE\|entire"
docs/.workflow/inc-i-180-M2-audit-p2-004-routing.md` returns nothing.

**OBS-004** was a coverage note only; nothing was required and nothing changed.

---

## R2.6 — round-1 PASS items re-checked after the shared-arithmetic change

Brief item 6. The fix touched arithmetic shared across layers, which is exactly the change that can
leak above the AH into below it.

**Pre-activation byte-identity — re-probed by execution, intact.** Same producer, same ledger
(`bond_count = 1`), same 6-input withdrawal declaring 6, one node below the AH and a fresh node above
it:

```
PROBE-PREAH: admission_at_PRE_AH  = Ok(())
PROBE-PREAH: gate_at_PRE_AH       = Ok(())
PROBE-PREAH: admission_at_POST_AH = Err([ECON_WITHDRAWAL_OVER_HOLDINGS] … allowance is 1
                                    (held=1, pending_addbond=0, withdrawal_pending=0,
                                     in_mempool_withdrawn=0))
PROBE-PREAH: gate_at_POST_AH      = Err([ECON_WITHDRAWAL_OVER_HOLDINGS] … allowance is 1
                                    (held=1, pending_addbond=0, in_block_addbond=0,
                                     withdrawal_pending=0, in_block_withdrawn=0))
```

Below the AH both layers are strict no-ops on a ledger both reject above it, and above the AH the two
layers report the **same allowance**. The structural guards are all still in place and none was
weakened:
- gate: the entire withdrawal section is inside `if height >= withdrawal_gate_ah`
  (`validation_checks.rs:621`); `numstat` is `27 0` — zero deletions;
- builder: `WithdrawalParity::new(active, height)` with `active = height >= AH`; `load()` and `allow()`
  both return early when `!active`, so `allowance_with` is **never reached** below the AH;
- mempool: `withdrawal_holdings_verdict` returns `Ok` when `current_height < AH` (`pool.rs`).
- Selection below the AH is additionally locked by the new
  `inc_i180_m2_pre_activation_still_selects_the_clamped_withdrawal`, which builds on the ISSUE-001
  ledger at `PRE_AH` and asserts the withdrawal is still carried.

**S3 Replay carve-out, S5 both height forms, INV-PROD-002 skip-not-abort — all still hold.**
`git diff --numstat e6c066c7` shows `validation_checks.rs 27 0` and `rewards.rs 22 0` unchanged from
round 1, `rollback.rs` / `block_handling.rs` still absent from the diff, and the whole node `it`
target is green at 66/66 including every S3 and S5 row. `wd_parity.accept(tx)` at `assembly.rs:332`
is still the last statement before `included_txs.push(tx)` with no `continue` after it, so the
in-block accounting still counts exactly the included set.

---

## R2.7 — adversarial pass on the fix diff (brief item 7)

I assumed a third defect and went looking. Two hypotheses were disproved by the code; two more
findings are nits. The two that survived are ISSUE-003 and ISSUE-004 above.

**⚠ Hypothesis 1, DISPROVED — gate/apply divergence on `delegated_bonds`.** The gate's R1 allowance
ignores `delegated_bonds`; live apply computes `available = remaining.saturating_sub(delegated)`
(`apply_block/tx_processing.rs:401-404`). I expected a partial withdrawal with
`available < declared < remaining` to be accepted by the gate and silently dropped by apply — the n11
shape surviving M1. **It does not.** The enqueue is gated on `if data.bond_count <= remaining`
(`:437`), **not** on `available`; `delegated` only selects the auto-revoke branch versus the warn.
Gate and apply agree. Hypothesis withdrawn.

**⚠ Hypothesis 2, DISPROVED — the builder's R4 set is a strict subset of the gate's.**
`WithdrawalParity::accept` inserts `tx.hash()` into `earlier_hashes` for every **mempool** transaction,
but the builder also adds protocol-generated transactions outside the loop — the epoch-reward coinbase
(`assembly.rs:96`) and the genesis registration (`:144`) — and `accept` is never called for those. The
gate's `earlier_tx_hashes` **does** include them. So the builder is structurally weaker on R4. It is
nevertheless unreachable: the genesis `reg_tx` is constructed with `outputs: vec![]` (nothing to
spend), and any input naming an outpoint created inside this block fails
`validate_transaction_with_utxos` against the pre-block UTXO view at `assembly.rs:259` — the **first**
skip gate, before the withdrawal gate at `:324`. Same mechanism as round-1 exploratory finding 6.
Real asymmetry, no reachable exploit. Worth a one-line comment, not a fix.

**OBS-006 (new, non-blocking) — the new `is_empty()` early return has no Path-Coverage entry and no
test in the tree.** Implementation report §9's `Path-Coverage` block (line 422) still lists only three
`HoldingsSources::lookup` outcomes (live hit / snapshot hit / `Unavailable` unwired). The round-1 fix
added a **fourth** early return — `if guard.is_empty() { return Unavailable }` — which is a new
early-return branch in non-test Rust and therefore falls under the Path-Coverage protocol. No test in
the tree drives it: the `crates/mempool` cases all seed a populated snapshot, and the `bins/node` cases
all resolve through an uncontended live handle. My PROBE-EMPTY covered it and has been deleted.
Recommend a `Path-Coverage:` line plus one small test that write-holds `node.producer_set` across
`add_transaction` and asserts `Ok` — three lines, and it is the branch that decides fail-open versus
fail-closed.

**OBS-007 (new, non-blocking, nit) — `pending_addbond` is itself transcribed twice.**
`ProducerSet::pending_addbond_count` (`crates/storage/src/producer/set_core.rs:214-226`, live path)
uses `outpoints.len() as u32` + `.sum()`; `holdings_of_every_producer`
(`bins/node/src/node/holdings.rs:16-25`, snapshot path) uses `u32::try_from(len).unwrap_or(u32::MAX)`
+ `fold(saturating_add)`. Same value for every reachable input, different overflow semantics. The
snapshot form is the safer one. Unreachable divergence; listed because it is the same pattern
ISSUE-001 came from, and because the snapshot is the source the mempool falls back to.

**OBS-008 (new, non-blocking, nit) — the in-block AddBond count is transcribed twice with different
overflow semantics.** Builder: `u32::try_from(count).unwrap_or(u32::MAX)`
(`production/withdrawal_holdings.rs:142-148`). Gate: `count() as u32` — a truncating cast
(`validation_checks.rs:705-709`). Block size limits make the difference unreachable. Same class.

**Not a finding, verified safe:** `allowance_with`'s two credit `saturating_add`s cannot saturate for
any reachable bond count; `x.sat_sub(a).sat_sub(b) == x.sat_sub(a+b)` for `u32` when `a+b` does not
overflow, so apply's single-subtraction form and the gate's two-subtraction form agree; and the
builder's `holdings` map, populated only on `HoldingsLookup::Found`, yields `0` for an unregistered
producer in `accept`'s `Exit` arm — matching the gate's `get_by_pubkey(...).unwrap_or(0)` exactly.

---

## R2.8 — updated specs/docs drift table

| File | Documented behaviour | Actual behaviour | Severity |
|---|---|---|---|
| `specs/protocol.md` (builder bullet) | "All three layers compute the allowance through the single `ProducerHoldings::allowance_with` function" | The gate re-expresses the terms inline at `validation_checks.rs:771-776` and calls nothing (ISSUE-003) | **medium** |
| `docs/architecture.md` (Builder parity paragraph) | "the SAME function the gate calls (`ProducerHoldings::allowance_with`)" | The gate calls no such function (ISSUE-003) | **medium** |
| `specs/protocol.md` (mempool bullet) | "`mempool-reject ⊆ consensus-reject` holds for every shape except one: `[AddBond(P,+n), RequestWithdrawal(P,d)]`…" | At least two exception shapes; the second needs no `AddBond` and comes from `in_mempool_withdrawn` exceeding `in_block_withdrawn` (ISSUE-004, executed) | **medium** |
| `docs/architecture.md` (Admission is not contained… paragraph) | same enumeration | same (ISSUE-004) | **medium** |
| `crates/mempool/src/withdrawal_holdings.rs:1-12` (module header) | same enumeration | same (ISSUE-004) | **medium** |

Round-1 drift rows are **all cleared**: the old `mempool-reject ⊆ builder-skip ⊆ consensus-reject`
sentence is gone from both files, and the "builder applies the whole table" bullet is now true of the
arithmetic as well as the structure. Everything else added to `specs/protocol.md` and
`docs/architecture.md` in this round matched the code as read — the S3 mode split, the `[REPLAY_SKIP]`
behaviour, the S5 auto-revoke mirror and its height-dependence, the `revalidate` eviction, the
fallback-vs-primary holdings ordering, the empty-snapshot rule, and the INV-PROD-002 skip semantics.

---

## R2.9 — round-2 blocking issues

- **[ISSUE-003] MEDIUM** — `specs/protocol.md`, `docs/architecture.md`,
  `docs/.workflow/inc-i-180-M2-implementation.md`. The claim that all three layers share
  `ProducerHoldings::allowance_with` is false; `validate_block_economics` is still an independent
  transcription. Fix: either route the gate through the function, or correct the three statements. One
  sentence each for the doc route.
- **[ISSUE-004] MEDIUM** — `specs/protocol.md`, `docs/architecture.md`,
  `crates/mempool/src/withdrawal_holdings.rs` module header. The containment claim's premise
  (`in_block_withdrawn` is zeroed) and its enumeration ("except one shape") are both false, disproved
  by execution. Fix: state the general rule instead of enumerating one shape. No code change.

## R2.10 — round-2 non-blocking observations

- **[OBS-006]** `HoldingsSources::lookup`'s new `is_empty()` early return: missing from the
  `Path-Coverage:` block and driven by no test in the tree.
- **[OBS-007]** `pending_addbond` transcribed twice (`set_core.rs:214` vs `node/holdings.rs:16`) with
  different overflow semantics.
- **[OBS-008]** in-block AddBond count transcribed twice (`production/withdrawal_holdings.rs:142` vs
  `validation_checks.rs:705`) with different overflow semantics.
- **[OBS-009]** the builder's R4 `earlier_hashes` omits protocol-generated transactions the gate's
  `earlier_tx_hashes` includes. Unreachable (see R2.7 hypothesis 2); worth a code comment so the next
  reader does not have to re-derive the unreachability.
- Carried from round 1: **OBS-004** (S5 `in_flight > 0` cell untested) still open as a coverage note.

## R2.11 — QA artefacts left in the tree

**None.** `bins/node/tests/it/qa_i180_m2_r2_probe.rs` was created, executed and deleted;
`bins/node/tests/it/main.rs`, `bins/node/src/node/production/withdrawal_holdings.rs` and
`bins/node/src/node/validation_checks.rs` were each mutated and then restored, and each was verified
byte-identical afterwards (`shasum` for the first two, re-read of lines 769-777 plus `numstat 27 0`
for the third). Final state re-measured: `cargo fmt --check` clean, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `doli-node --test it` 66/66, `mempool --test it` 6/6, and
`git diff --numstat e6c066c7` identical to the state I received. **No assertion was weakened or
deleted, no version was bumped, no activation height was added, moved or reused, and nothing was
committed.**

---

## Round-2 final verdict

**FAIL** — round 2 of max 3.

The blocking HIGH is closed and I proved it independently: MUTATION-1 turns
`inc_i180_m2_builder_skips_when_the_allowance_clamps` RED with my own round-1 message and GREEN with
the fix, MUTATION-2 shows the fix cannot be degraded into an over-fix, and the ledger the partition
rests on is now derived by applying two real `Exit` transactions instead of hand-written. OBS-001,
OBS-002, OBS-003 and OBS-005 are all closed, three of them verified by execution. No regression: the
workspace failing-TARGET set is byte-identical to the declared baseline with no fourth target, the
only non-environmental failure is the declared DeFi-gate baseline, and pre-activation byte-identity
was re-probed after the shared-arithmetic change and is intact at both the admission and the gate
layer.

What blocks is ISSUE-002's resolution, not ISSUE-001's. Round 1 blocked on "a documented claim is
wrong in two shipped files"; the round-1 fix removed that claim and put two new false claims into the
same two paragraphs of the same two files — one disprovable by `grep`, one disproved here by
execution. Applying the same standard to the same defect class in the same files, the milestone
cannot ship on documentation that a reader can falsify in one command. **Neither issue requires a code
change; both are one to two sentences.** Everything else in M2 — S1 builder parity, S2 admission, S3,
S4, S5, S6, INV-PROD-002, lock-order, deadlock-freedom, REQ-I180-003 — passes and needs no rework.

---

# Round 3 — final verification of the developer's QA round-2 fixes

**Round 3 of a hard maximum of 3.** Scope was narrow by instruction: ISSUE-003's resolution (b),
the new parity lock, ISSUE-004's general rule, no regression, no dangling artefacts, and a spot-check
of what already passed. Every factual sentence below carries the command that produced it.

## R3.0 — What I measured myself on the current tree

| Measurement | Result |
|---|---|
| `cargo fmt --check` | clean (exit 0) |
| `cargo test -p doli-node --test it` | **68 passed / 0 failed / 0 ignored** |
| `cargo test -p mempool --test it` | **6 passed / 0 failed** |
| `cargo test --workspace --no-fail-fast` | **177 targets · 3728 passed · 3 failed · 43 ignored · 0 compile errors** |
| `git diff --numstat e6c066c7 -- bins/node/src/node/validation_checks.rs` | `27 0` |
| `shasum -a 256 bins/node/src/node/validation_checks.rs` | `b411b0bb50f6769b58c56691d38f6d61990159f3dae420c8095e3a927ceeb2ba` |
| `shasum -a 256 bins/node/tests/it/main.rs` | `3a04639b8a11cb0f7185b9f451f4282e06ad20929fd315a16d351bebbfb0083a` |
| `#[tokio::test]` count across `bins/node/tests/it/*.rs` | 2+6+9+19+9+3+4+16 = **68**, equals the run count |

`grep -c "error\[E" ws.log` → **0**. Clippy was re-measured by the runner and is clean; I did not
re-run it, because I edited no source file that survives this round.

---

## R3.1 — ISSUE-003: resolution (b) is complete, and every replacement sentence is TRUE · **CLOSED**

I assumed a third false claim and went looking for it with commands, not with reading.

### The structural claim, re-derived

```
$ grep -rn "allowance_with\|\.allowance()" --include='*.rs' bins crates | wc -l        13
$ grep -rn "allowance_with\|\.allowance()" --include='*.rs' bins crates \
      | grep -v "/tests/"
bins/node/src/node/production/withdrawal_holdings.rs:84   info.allowance_with(...)     ← builder
crates/mempool/src/holdings.rs:36                         fn allowance_with(...)       ← definition
crates/mempool/src/holdings.rs:46                         self.allowance_with(0, 0)    ← allowance()
crates/mempool/src/withdrawal_holdings.rs:51              holdings.allowance_with(…)   ← admission
$ sed -n '771,776p' bins/node/src/node/validation_checks.rs
        let allowance = info
            .bond_count
            .saturating_add(producers.pending_addbond_count(pk))
            .saturating_add(prior_add)
            .saturating_sub(info.withdrawal_pending_count)
            .saturating_sub(prior_wd);
```

Four non-test call sites, none of them in `validation_checks.rs`; the gate holds the inline
expression at exactly the lines the docs cite. The developer's note that the **unfiltered** grep now
returns 13 — because the new lock names the function 9 times — is also correct (13 − 4 = 9).

### Each corrected sentence, verified

| File | Corrected statement | Verdict |
|---|---|---|
| `specs/protocol.md` (builder bullet) | "The builder and the mempool compute the allowance through `ProducerHoldings::allowance_with`… The gate does NOT call it: it holds the reference expression inline in `validate_block_economics`, because routing consensus validation through the mempool crate would invert the layering." | **TRUE** — grep above; the layering argument is sound (`crates/mempool` is not a dependency direction consensus should acquire) |
| `docs/architecture.md` (Builder-parity paragraph) | "The builder computes the allowance through `ProducerHoldings::allowance_with`, the function the mempool also calls. The gate does not call it — it holds a second transcription of the same five terms inline…" | **TRUE** — same grep |
| `crates/mempool/src/holdings.rs:33-35` (doc on `allowance_with`) | "The builder and this crate call it; the gate holds the same expression inline, and the two are locked by `inc_i180_m2_the_gate_allowance_equals_the_shared_function`." | **TRUE** |
| `docs/.workflow/inc-i-180-M2-implementation.md` §ISSUE-001 fix | "…is the function the **builder and the mempool** compute the allowance through. The **gate does not call it**" | **TRUE** |
| same, Failure-Modes delta row | "CLOSED by an explicit assertion, NOT by construction… Measured RED under two independent gate mutations (re-ordered terms; dropped `in_block_addbond` credit)." | **TRUE** — I reproduced both, R3.2 |
| round-1 §ISSUE-002 block | marked **SUPERSEDED**, points forward to the round-2 section | correct handling; the round-1 record is preserved rather than rewritten |

The routed residual `FIND-I180-M2-TRANSCRIPTION-001` (relocate `ProducerHoldings` into `crates/storage`
so all three layers share one definition) is banked, not claimed as done. That is the honest framing:
a lock fails *after* someone writes the drift, not instead of it.

---

## R3.2 — The parity lock is real: I drove both mutations myself

`bins/node/tests/it/inc_i_180_allowance_parity.rs`, 326 lines, `grep -c 'name: "IP-'` → **8**,
2 test functions, additive (`main.rs` +1 `mod` line, zero deletions anywhere).

### Pre-derivation (done before running anything, so the run could disprove me)

| Row | `bond` | `pending` | `wp` | `in_block_add` | `in_block_wd` | allowance | MUT-A (pre-fix order) | MUT-B (credit dropped) |
|---|---|---|---|---|---|---|---|---|
| IP-ZERO | 4 | 0 | 4 | 0 | 0 | 0 | same | same |
| IP-DEFICIT | 12 | 0 | 24 | 0 | 0 | 0 | same | same |
| **IP-DEFICIT-CREDIT** | 12 | 0 | 24 | 10 | 0 | 0 | **10 — differs** | 0 (same) |
| **IP-BOTH** | 20 | 3 | 5 | 7 | 4 | 21 | 21 (same) | **14 — differs** |
| IP-PENDING | 1 | 6 | 0 | 0 | 0 | 7 | same | same |
| IP-EXIT | 9 | 2 | 0 | 0 | 9 | 2 | same | same |
| IP-DEBIT-CLAMP | 5 | 0 | 5 | 0 | 5 | 0 | same | same |
| **IP-CEILING** | MAX−5 | 4 | 10 | 3 | 0 | MAX−10 | **MAX−8 — differs** | **MAX−11 — differs** |

Predicted RED rows: MUT-A → IP-DEFICIT-CREDIT and IP-CEILING; MUT-B → IP-BOTH and IP-CEILING.

### MUT-A — the gate re-ordered into the pre-fix shape

`.add(pending) .sub(withdrawal_pending) .add(prior_add) .sub(prior_wd)`

```
test result: FAILED. 0 passed; 2 failed; 66 filtered out

IP-DEFICIT-CREDIT: expected tail: but allowance is 0 (held=12, pending_addbond=0,
                   in_block_addbond=10, withdrawal_pending=24, in_block_withdrawn=0)
                   gate said: [ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] … declares 1 bonds
                   but spends 0 Bond UTXO inputs OWNED BY IT …
IP-CEILING:        expected tail: but allowance is 4294967285 (held=4294967290, …)
                   gate said: [ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] … declares 4294967286 …
```

Exactly the two predicted rows, and the failure mode is the predicted one: the gate's allowance rose
**above** the shared function, so R1 never fired and the probe fell through to R2. That is the
two-sidedness the file claims — an allowance that drifts UP is caught by non-rejection, not only one
that drifts DOWN.

### MUT-B — the credit dropped: `.saturating_add(prior_add)` → `.saturating_add(0)`

```
test result: FAILED. 0 passed; 2 failed; 66 filtered out

IP-BOTH:    expected: allowance is 21 (held=20, pending_addbond=3, in_block_addbond=7,
                      withdrawal_pending=5, in_block_withdrawn=4)
            gate said: … allowance is 14 (held=20, pending_addbond=3, in_block_addbond=7,
                      withdrawal_pending=5, in_block_withdrawn=4)
IP-CEILING: expected 4294967285, gate reported 4294967284
```

IP-BOTH is the row that proves the **terms half** of the assertion is load-bearing: the gate echoed
`in_block_addbond=7` while computing as if it were 0. An assertion on the number alone would still
have caught this one, but an assertion on the number alone could not catch a gate that reaches the
right number from the wrong terms — which is why the echoed tail is asserted whole.

### Restore, verified

```
$ shasum -a 256 bins/node/src/node/validation_checks.rs
b411b0bb50f6769b58c56691d38f6d61990159f3dae420c8095e3a927ceeb2ba
$ git diff --numstat e6c066c7 -- bins/node/src/node/validation_checks.rs
27      0
$ cargo test -p doli-node --test it
test result: ok. 68 passed; 0 failed
```

Byte-identical to the tree I received. **No consensus source file was left edited.**

### Coverage of the eight shapes — the brief's four questions, answered

- **Both clamp directions.** Lower clamp: IP-DEFICIT (`wp=24 > 12+0`), IP-DEFICIT-CREDIT,
  IP-DEBIT-CLAMP (ledger debit 5 **plus** in-block debit 5 against 5 held). Upper clamp: IP-CEILING,
  where `(MAX−5)+4+3` saturates inside the credit chain — asserted explicitly by the fixture's own
  `assert_eq!(u32::MAX - 10, …)` guard so the row cannot silently stop engaging it. **Covered.**
- **`withdrawal_pending > bond_count + pending_addbond` with and without a lower-index same-producer
  `AddBond`.** IP-DEFICIT is the without-case; IP-DEFICIT-CREDIT is the with-case (a real
  `add_bond_tx` at a lower block index). **Covered — and the pair is the load-bearing one**:
  IP-DEFICIT stays GREEN under MUT-A and IP-DEFICIT-CREDIT goes RED, which is the executed proof that
  the deficit alone is not what splits the orders; the credit term is.
- **Zero and saturation boundaries.** IP-ZERO lands the allowance exactly on 0 with no in-block term;
  IP-CEILING is the saturation boundary. **Covered.**
- **"A shape that cannot distinguish the two orders is decoration."** Three of the eight
  (IP-DEFICIT-CREDIT, IP-BOTH, IP-CEILING) are order- or credit-discriminating. The other five are
  **term-isolation** rows, not decoration, and I checked each against a drop-one-term mutation on
  paper: dropping `pending_addbond` moves IP-PENDING 7→1; dropping `withdrawal_pending` moves IP-ZERO
  0→4 and IP-DEFICIT 0→12; dropping the in-block debit moves IP-EXIT 2→11; and IP-DEBIT-CLAMP is the
  only row where a `saturating_sub` degraded to a plain `-` underflows in a debug build. The weakest
  row is IP-DEBIT-CLAMP, which yields 0 under several mutations — recorded as a coverage note
  (OBS-014), not a defect.

`withdrawal_pending` is **derived**, not hand-written, in seven of the eight rows: the fixture applies
real `RequestWithdrawal` / `Exit` transactions through `process_transaction_producer_effects`. Only
IP-CEILING writes the ledger directly, and it documents why inline (`register_genesis_producer`
allocates one `StoredBondEntry` per bond, so a `u32::MAX` ledger cannot be built through it) — sound,
because every probe bails at R1 and nothing downstream of R1 reads the bond entries.

---

## R3.3 — ISSUE-004: the general rule is TRUE, and it ships in five places · **CLOSED**

### I found the five places myself, before reading the developer's count

```
$ grep -rln "over-reject\|OVER-reject" --include='*.rs' --include='*.md' bins crates specs docs
```

| # | Place | Carries the general rule? |
|---|---|---|
| 1 | `specs/protocol.md`, mempool bullet | yes — both halves, plus the UNDER-reject direction |
| 2 | `docs/architecture.md`, "Admission is not contained in builder-skip" | yes — both halves |
| 3 | `crates/mempool/src/withdrawal_holdings.rs:1-13`, module header | yes — both halves |
| 4 | `bins/node/tests/it/inc_i_180_in_block_parity.rs:506-508`, doc comment on `inc_i180_m2_admission_over_rejects_the_addbond_window` | partial — states the "raises the allowance" half and names the row as one **instance** (OBS-014) |
| 5 | `docs/.workflow/inc-i-180-M2-implementation.md`, round-2 §ISSUE-004 | yes — both halves, quoted |

Round 1 had shipped it in 1–3 only; 4 and 5 are the two it missed. **The count is right.**

### No surviving copy of the false enumeration

```
$ grep -rn "except one\|for every shape except\|Except one" \
      --include='*.rs' --include='*.md' bins crates specs docs | grep -v "docs/qa/"
docs/.workflow/inc-i-180-M2-implementation.md:865:**"Except one", corrected.** …
```

One hit, and it is the heading of the correction itself, not a claim.

### The premise is now right, verified against the code

```
$ grep -n "withdrawal_holdings_verdict" crates/mempool/src/pool.rs
314:    fn withdrawal_holdings_verdict(          ← definition
573:        self.withdrawal_holdings_verdict(&tx, utxo_set, current_height, true)     ← admission
1301:                    self.withdrawal_holdings_verdict(tx, utxo_set, current_height, false)  ← revalidate
```

`pool.rs:333-341` selects `withdrawal_holdings::resident_withdrawn(self.entries.values().map(|e| &e.tx), &pubkey)`
when `count_residents`, and `crates/mempool/src/withdrawal_holdings.rs:51` is
`holdings.allowance_with(0, in_mempool_withdrawn)` — **first argument zeroed, second SUBSTITUTED**.
`resident_withdrawn` sums `bond_count` over every same-producer `RequestWithdrawal` the mempool holds;
the candidate is not yet inserted, so it never self-counts. Three of the five copies (spec,
architecture, module header) state explicitly that this substitute **can exceed** a block's
`in_block_withdrawn` "since a block need not carry the residents". **The premise is correct.**

### PROBE-CONT2, re-run on the round-3 tree

Probe wired into `bins/node/tests/it/main.rs`, executed, then **deleted** and `main.rs` restored
(sha256 verified identical, see R3.6).

```
PROBE-CONT2 wd1 admission          = Ok(())
PROBE-CONT2 wd2 admission          = Err("[ECON_WITHDRAWAL_INCOMPLETE_DRAIN] RequestWithdrawal at
                                     height=1000007 producer=4aa7e180… declares its full allowance
                                     of 2 bonds but spends 2 of the 6 Bond UTXOs it owns")
PROBE-CONT2 gate(wd2 alone)        = Ok(())
PROBE-CONT2 EXCEPTION_WITHOUT_ADDBOND = true
```

Fixture: `bond_count = 6`, `withdrawal_pending = 0`, `pending_addbond = 0`, six owned Bond UTXOs, **no
`AddBond` anywhere**; `wd1` declares 4 and spends UTXOs 0..4, `wd2` declares 2 and spends UTXOs 4..6.
Byte-for-byte the round-2 result — round 2's doc-only fixes changed no behaviour.

**Does the shipped rule account for it?** Yes, and by the second clause specifically:
`in_mempool_withdrawn = 4` (the resident `wd1`) versus `in_block_withdrawn = 0` on a block carrying
`wd2` alone, so the substitute **exceeds the block's debit**. That drops the admission allowance to
`6 − 4 = 2`, which equals the declared count, which flips R2 from its partial branch into its
full-exit branch and measures 2 against all 6 still-live Bond UTXOs. The gate reads allowance 6, takes
the partial branch, accepts. This is precisely the instance the spec names second — "a resident
`RequestWithdrawal(P, d1)` that drops the admission allowance far enough to push a second
`RequestWithdrawal(P, d2)` out of R2's partial branch and into its full-exit branch … with no
`AddBond` anywhere". **The rule as written covers the shape that disproved its predecessor.**

The boundedness claim also holds under check: once `wd1` confirms, `withdrawal_pending` becomes 4 and
the owned Bond UTXO count becomes 2, so `wd2` re-admits on the full-exit branch (2 declared, 2 owned).
`specs/protocol.md` states the bound as "the resident confirms **or expires**, then the operator
resubmits", which is the accurate form; `docs/architecture.md` shortens it to "one confirmation deep"
without the expiry alternative (OBS-013).

The developer's decision to leave the code alone is correct: the resident charge is what keeps this
mempool from ever offering a builder the `[partial(P), full-exit(P)]` pair (SEC-FIXVERIFY2-001), and
that property is worth more than the one-confirmation liveness cost.

---

## R3.4 — No regression

```
$ cargo test --workspace --no-fail-fast
targets=177   passed=3728   failed=3   ignored=43
$ grep -c "error\[E" ws.log        →  0
$ grep -ci "Too many open files"   →  2
```

**Failing set, byte-identical to the declared baseline. No fourth distinct failure.**

| Target | Test | Class |
|---|---|---|
| `doli-node --test checkpoint_rotation` | `test_network::test_cluster_10x100` | environmental (EMFILE) |
| `doli-node --test test_network` | `test_cluster_10x100` | same file, second binary |
| `mempool --lib` | `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` | pre-existing baseline |

The four `^error` lines in the log are cargo's three "test failed, to rerun pass …" summaries plus
"error: 3 targets failed" — not compile errors; `error[E…]` count is zero.

Baseline at `e6c066c7` per the brief: 176 targets, 3700 passed, 3 failed, 43 ignored. This tree is
**+28 passing tests and +1 target** — the extra target is `crates/mempool/tests/it`, compile-red at
baseline because it names two symbols M2 introduced. Same 3 failures, same 43 ignored. The
developer's claimed figures — **177 / 3728 / 3** — reproduce exactly. (The "targets" figure counts
`test result:` lines and is noisy by ±1 across runs, as both earlier rounds recorded; the load-bearing
facts are the zero `error[E…]` and the byte-identical failing set.)

---

## R3.5 — Spot-check of what already passed (no re-derivation)

| Item | Check | Result |
|---|---|---|
| Pre-activation byte-identity | gate section still wholly inside `if height >= AH`; `WithdrawalParity::new(active, h)` short-circuits; `withdrawal_holdings_verdict` returns `Ok` below AH; `req_i180_003_pre_activation_selection_is_unchanged`, `inc_i180_m2_pre_activation_still_selects_the_clamped_withdrawal`, `req_i180_003_pre_activation_is_mode_invariant` all green | **HOLDS** |
| S3 Replay carve-out | both guards present (`validation_checks.rs:753`, `:828`), both `mode == ValidationMode::Replay`, `numstat 27 0` | **HOLDS** |
| S5 both height forms | `rewards.rs 22 0`; mirror condition `delegated > 0 && bond_count == available && bond_count > available.saturating_sub(delegated)` is live's `if bond_count > available { if delegated > 0 && bond_count == remaining }` flattened — the rebuild's `available` **is** live's `remaining`, and `available.saturating_sub(delegated)` **is** live's `available`. Term-for-term equivalent | **HOLDS** |
| INV-PROD-002 skip-not-abort | `assembly.rs:324-334` — `warn!` + `continue`, then `wd_parity.accept(tx)` as the last statement before `included_txs.push(tx)` with no `continue` between. No `Err`, no eviction, no rollback | **HOLDS** |
| ISSUE-001 closed | builder's only allowance site is `info.allowance_with(in_block_addbond, in_block_withdrawn)` at `production/withdrawal_holdings.rs:84`; no chained saturating pair remains | **HOLDS** |
| OBS-002 ordering | `apply_block/mod.rs` — the hygiene block now sits after `refresh_mempool_producer_snapshot(height)`, with the load-bearing reason in a comment | **HOLDS** |
| OBS-005 | `docs/.workflow/inc-i-180-M2-audit-p2-004-routing.md` carries the corrected "one field lacks the integrity check its siblings have" framing | **HOLDS** (but see OBS-012) |
| No assertion weakened | every tracked test file in the diff is **additive**: `inc_i_180_rebuild_parity.rs 194 0`, `main.rs 4 0`. Zero deletions in any tracked test file | **HOLDS** |
| Module-size budget | new modules 41 / 114 / 135 / 209 lines; `apply_block/mod.rs` exactly 500; `allowance_parity.rs` 326 of the 800-line test budget | **HOLDS** |

---

## R3.6 — Nothing dangles

- `bins/node/tests/it/main.rs` declares exactly nine modules; `ls bins/node/tests/it/` shows exactly
  those nine files plus `main.rs`. No unwired module, no undeclared file.
- `crates/mempool/tests/it/` — `main.rs` declares `inc_i_180_admission_parity`; that is the only other
  file present.
- `find bins crates -name "*qa_i180*" -o -name "*probe*"` → **empty**.
- My own artefacts: `bins/node/tests/it/qa_i180_m2_r3_probe.rs` was created, executed and **deleted**;
  `main.rs` was wired and unwired and is back to `sha256 3a04639b…`;
  `bins/node/src/node/validation_checks.rs` was mutated twice and restored to
  `sha256 b411b0bb…` / `numstat 27 0`. `cargo fmt --check` clean and
  `cargo test -p doli-node --test it` 68/68 **after** the restore.
- **No assertion was weakened or deleted. Nothing was committed. No version was bumped. No activation
  height was added, moved or reused.**

---

## R3.7 — Round-3 observations (all NON-BLOCKING)

None of these asserts a false safety property, and every one of them errs toward making a reader more
careful rather than less. They are recorded for the reviewer and for the follow-up milestone.

- **[OBS-010] "eight allowance shapes" is attributed to a test function that drives seven.**
  `specs/protocol.md`, `docs/architecture.md` and the implementation report's Failure-Modes row all
  say `inc_i180_m2_the_gate_allowance_equals_the_shared_function` "drives the gate over eight
  allowance shapes". That function iterates a 7-row table; the eighth (IP-CEILING) lives in
  `inc_i180_m2_the_gate_allowance_equals_the_shared_function_at_the_ceiling`. Because the attributed
  name is a strict **prefix** of the sibling's, `cargo test … <name>` selects both — which is how the
  developer measured it (`2 passed`) and how I measured it. Suggested remedy: "…locked by the two
  `inc_i180_m2_the_gate_allowance_equals_the_shared_function*` rows, eight shapes in total". Low.
- **[OBS-011] Two over-broad `whenever` quantifiers.** Both state a **necessary** condition as if it
  were sufficient.
  (a) `specs/protocol.md`: "a second order silently raises one layer's allowance above the other's
  whenever `withdrawal_pending > bond_count + pending_addbond`". The divergence also needs
  `in_block_addbond > 0` — and this report's own MUT-A run is the proof: IP-DEFICIT **is** that ledger
  shape with `in_block_addbond = 0` and it stayed GREEN.
  (b) `specs/protocol.md` and `docs/architecture.md`: "admission OVER-rejects whenever the substitute
  raises the block's allowance … or exceeds the block's debit". With an ample allowance neither
  substitution rejects anything. Suggested remedy in both: "can over-reject only when…". Low.
- **[OBS-012] The OBS-005 overstatement survives in a second file.** The routing doc was corrected,
  but `docs/.workflow/inc-i-180-M2-implementation.md` §5 still reads "the peer already controls the
  entire `ProducerSet` and `UtxoSet` the node installs". The routing doc's corrected form — "a field
  inside an object the syncing node installs from a peer, minus the state-root cross-check the
  object's other fields get" — is the accurate one and should replace it. Low; a workflow doc, and the
  conclusion it supports is unchanged.
- **[OBS-013] `docs/architecture.md` shortens the boundedness claim** to "one confirmation deep"
  without the expiry alternative that `specs/protocol.md` states ("the resident confirms **or
  expires**"). A resident that no builder ever selects holds the second withdrawal out until the
  14-day mempool expiry, not until a confirmation. Low.
- **[OBS-014] Two coverage notes on the new lock.** (a) The doc comment at
  `inc_i_180_in_block_parity.rs:506-508` states only the "raises the allowance" half of the general
  rule; the "exceeds the block's debit" half — the half PROBE-CONT2 exercises — is not named there,
  and no row in the node `it` target asserts that instance end-to-end (it is measured only by this
  report's probe). (b) `crates/mempool/tests/it/inc_i_180_admission_parity.rs:630` still calls the
  AddBond row "(the exception)", a residual of the enumeration framing; "(one instance)" would match
  the corrected rule. (c) IP-DEBIT-CLAMP is the weakest of the eight shapes — it yields 0 under most
  mutations, and its unique value is only that a `saturating_sub` degraded to plain `-` underflows
  there. Low.

**Carried forward from earlier rounds, unchanged and still non-blocking:** OBS-004 (S5 `in_flight > 0`
cell untested), OBS-006 (`HoldingsSources::lookup`'s `is_empty()` early return is still absent from
the `Path-Coverage:` block and driven by no test), OBS-007 / OBS-008 (`pending_addbond` and the
in-block AddBond count each transcribed twice with different overflow semantics), OBS-009 (builder R4
`earlier_hashes` omits protocol-generated transactions — unreachable, wants a comment).

**Routed residual, banked not closed:** `FIND-I180-M2-TRANSCRIPTION-001` — relocate `ProducerHoldings`
into `crates/storage` so builder, mempool and gate share ONE definition instead of two transcriptions
bound by a test. It touches consensus code and needs its own measurement pass. It is a prerequisite
worth naming before any real mainnet value is pinned for AH #23, alongside AUDIT-P2-004.

---

## R3.8 — Specs / docs drift after round 3

| File | Documented behaviour | Actual behaviour | Severity |
|---|---|---|---|
| `specs/protocol.md`, `docs/architecture.md`, implementation report | "…drives the gate over eight allowance shapes" attributed to one test function | That function drives seven; the eighth is its `_at_the_ceiling` sibling, selected by the same `cargo test` filter | low (OBS-010) |
| `specs/protocol.md` | "a second order raises one layer's allowance … **whenever** `wp > bond + pending`" | Also requires `in_block_addbond > 0`; IP-DEFICIT is the counter-example and is green under MUT-A | low (OBS-011a) |
| `specs/protocol.md`, `docs/architecture.md` | "admission OVER-rejects **whenever** the substitute raises the allowance or exceeds the debit" | Necessary, not sufficient — an ample allowance rejects nothing | low (OBS-011b) |
| `docs/.workflow/inc-i-180-M2-implementation.md` §5 | "the peer already controls the **entire** `ProducerSet` and `UtxoSet`" | Corrected in the routing doc; this copy was missed | low (OBS-012) |
| `docs/architecture.md` | "one confirmation deep" | "confirms **or expires**", as `specs/protocol.md` correctly states | low (OBS-013) |

**All round-2 drift rows are cleared.** Everything else I checked in `specs/protocol.md` and
`docs/architecture.md` matched the code as read: the four non-test `allowance_with` call sites, the
inline gate expression at `771-776`, the substitution premise (`count_residents = true` at `pool.rs:573`,
`false` at `:1301`), the `revalidate` eviction, the fallback-vs-primary holdings ordering, the
empty-snapshot rule, the refresh-before-revalidate ordering, the single-guard lock discipline in
`assembly.rs:192-199`, the S3 mode split, and the S5 auto-revoke mirror's height-dependence.

---

## Round-3 final verdict

**PASS** — round 3 of max 3.

Both round-2 blocking issues are closed on evidence I produced myself rather than on the developer's
word. ISSUE-003's resolution (b) is complete: the three false "one function at every layer" statements
are gone, their replacements are true under `grep` and under `sed`, and — the part that actually
matters — the hazard they were papering over is now closed by an executed assertion. I drove **both**
gate mutations and both went red on exactly the rows and with exactly the numbers predicted from the
arithmetic before the run, then restored `validation_checks.rs` byte-for-byte. ISSUE-004's general
rule ships in five places, I found all five independently, its premise about `resident_withdrawn` is
correct against the code, the false enumeration survives nowhere, and my round-2 PROBE-CONT2 shape
re-runs unchanged and falls squarely inside the rule's second clause.

No regression: 177 targets, 3728 passed, 3 failed, 43 ignored, zero compile errors, and the failing
SET is byte-identical to the declared baseline. Nothing dangles. Nothing was weakened, bumped, moved
or committed.

Five new observations remain, all low, all prose-precision or coverage. The one I weighed hardest is
OBS-010 — three shipped files attribute eight shapes to a seven-shape test function. I am not blocking
on it: the eighth shape exists, is asserted, and is selected by the same test filter the claim's name
denotes, so the safety property the sentence describes is real and no reader is misdirected about what
is protected. Blocking a milestone on that would be blocking on phrasing, which is not what this gate
is for.

**Approved for review.**
