# QA Report: INC-I-180 M1 — consensus withdrawal-holdings gate (ROUND 3, FINAL)

**Run:** 525 · **Incident:** INC-I-180 · **Milestone:** M1 · **Round:** 3 of 3 (final iteration)
**Branch:** `bugfix/inc-i-180-withdrawal-holdings-gate` (uncommitted working tree)
**Spec:** `docs/.workflow/inc-i-180-M1-brief.md` (F1..F7 binding)
**Date:** 2026-08-20
**Supersedes:** the round-1 and round-2 reports (condensed below as *Resolved history*)

---

## Verdict

**PASS** — REQ-I180-001, REQ-I180-002 and REQ-I180-003 (all Must) are met.

Both round-2 blockers are genuinely closed, proven by re-executing the exact round-2 probes that
found them. ISSUE-006: the cross-owner withdrawal that round 2 got ADMITTED is now rejected with
`ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`. ISSUE-005: the epoch-boundary block that round 2 got
ADMITTED in Full mode and REJECTED in Light is now rejected identically in Full, Light and Replay.

The owner-derivation claim — the one item that could have turned a fix into a liveness break — was
verified **non-circularly**, against bonds built by the production `Transaction::new_add_bond` and
`Transaction::new_registration` constructors rather than by the test fixture. Both are accepted.
This mattered: the delivered fixture hardcodes the same expression the gate uses, so the
developer's own ACCEPT control could not have distinguished "gate matches production" from "gate
matches fixture" (see OBS-R3-001).

The hoist introduced nothing. `git diff` on `validation_checks.rs` is **196 insertions, 0
deletions** — no pre-existing line was removed or altered. The gate still runs in all three modes,
the lock discipline survives the move, and INC-I-080's cap still sits below the early return.
Pre-activation bit-identity holds on every rule, including the two new ones.

A real `cargo test --workspace --no-fail-fast` was executed this round (round 2 skipped it):
**3688 passed / 3 failed / 43 ignored** across 176 targets. All three failures are pre-existing —
two are the known `test_cluster_10x100` EMFILE case, and the third
(`inc_i_096_below_gate_rejects_remove_liquidity`) was **not** on the known-issues list, so it was
proven pre-existing with a fresh `HEAD` detached-worktree control rather than assumed. Zero failures
are attributable to this diff. `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --check` both exit 0.

Two non-blocking test-quality defects were found: the delivered ownership fixture is circular
(OBS-R3-001) and the new positional guard test is self-satisfying (OBS-R3-002). Neither is a defect
in shipped behaviour — the gate's derivation and the code position they are meant to lock are both
correct today, verified independently above.

---

## Round-3 scope

- Re-execution of the two round-2 blocker probes (R2-B8 cross-owner, R2-E1 Full-vs-Light).
- A **non-circular** proof of the owner derivation against the production transaction builders.
- Verification that the hoist changed no mode-reachability, no lock discipline, no INC-I-080
  behaviour, and nothing pre-existing.
- Re-confirmation of the invariants: pre-AH bit-identity, no version bump, no activation height
  moved or reused, flush order untouched, no test weakened.
- A real, complete `cargo test --workspace --no-fail-fast` run.

Probes were added as `bins/node/tests/it/qa_r3_probe.rs`, executed, then **removed**;
`bins/node/tests/it/` is restored to its four delivered modules + `main.rs`.

---

## Resolved history — ISSUE-001..006 final verdicts

| ID | Round opened | Finding | Final verdict |
|---|---|---|---|
| **ISSUE-001** | 1 | `TxType::Exit` invisible to the gate; `[Exit(p), RequestWithdrawal(p,434)]` admitted, apply half-applied, producer left `Active, weight=1` | **FIXED** (round 2) |
| **ISSUE-002** | 1 | Declared `bond_count` never bound to the Bond UTXOs destroyed; `bond_count=1` + 434 Bond inputs ⇒ 434 UTXOs gone, 1 bond removed, `weight=433` | **FIXED** (count+type round 2, owner round 3 via ISSUE-006) |
| **ISSUE-003** | 1 | `rebuild_producer_set_from_blocks` lacked the post-AH `pending_addbond_count` term ⇒ reorg/rollback fork | **FIXED** (round 2) |
| **ISSUE-004** | 1 | `specs/protocol.md` and the `tx_processing.rs` comment asserted guarantees 001/002/003 disproved | **FIXED** (round 2, residual drift closed round 3) |
| **ISSUE-005** | 2 | Gate sat below an over-broad `return Ok(())`; same epoch-boundary block ADMITTED in Full, REJECTED in Light | **FIXED** (round 3) |
| **ISSUE-006** | 2 | Bond-input binding was owner-agnostic; spend `A`'s bonds while naming `B` ⇒ `A` keeps unbacked reward-earning weight (the n11 shape) at zero cost | **FIXED** (round 3) |

Round-1 observations OBS-001..OBS-005 and round-2 OBS-R2-001..OBS-R2-005 stand as written and are
not re-litigated. OBS-001 (mempool/builder holdings parity) is agreed **M2 scope**.

---

## Round-3 requirement results

| ID | Priority | R1 | R2 | R3 | Evidence |
|---|---|---|---|---|---|
| **REQ-I180-001** | Must | FAIL | FAIL | **PASS** | Gate at `validation_checks.rs:599-793`, above the EpochReward section (`:795`). Owner-bound input resolution `:645-666`; allowance `:737-759`; count binding `:776-787`. Both round-2 bypasses closed and re-probed: R3-B8, R3-E1, R3-E2 |
| **REQ-I180-002** | Must | PASS | PASS | **PASS** | `validation_checks.rs:739` `.saturating_add(producers.pending_addbond_count(pk))`; `tx_processing.rs:388-402` mirrors it; `rewards.rs:1379-1393` mirrors it in the replay path. `req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable` green |
| **REQ-I180-003** | Must | PASS | PASS | **PASS** | `defaults.rs` diff = 3 insertions / 0 deletions; mainnet `u64::MAX`, testnet `230_000`, devnet `20`; no version bump of any kind; `crates/storage/src/` unmodified. R3-B8p, R3-E4 confirm pre-AH verdicts unchanged for the two NEW rules as well |

---

## PRIORITY 1 — the two round-2 blockers, re-verified by execution

### ISSUE-006 — **FIXED**

`bins/node/src/node/validation_checks.rs:645-666`. An input counts toward `bond_inputs` only if its
pre-block UTXO satisfies **both** predicates:

- `e.output.output_type == OutputType::Bond` (`:660-661`), and
- `e.output.pubkey_hash == owner_hash` (`:662`), where
  `owner_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, wd.producer_pubkey.as_bytes())`
  (`:646-649`).

A transaction spending `A`'s bonds while naming `B` therefore counts **zero** owned inputs, and the
pre-existing declared-count comparison at `:776-787` rejects it with
`ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`. No new error code, no new activation height, no change to
the allowance rule.

**Executed** (round-2 probe R2-B8, re-run as R3-B8 against bonds built by the production
`Transaction::new_add_bond`, both producers registered at 433 so 100 is well inside `B`'s allowance
and the allowance rule cannot be what rejects):

```
R3-B8   declare B=100, spend 100 of A's REAL bonds  -> [ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]
                                                       (round 2: ADMITTED)
R3-B8c  control: declare A=100, same 100 REAL bonds -> ADMITTED
R3-B8p  same cross-owner block at PRE_AH=5          -> ADMITTED   (legacy verdict preserved)
```

R3-B8c is the discriminating control: the same block, same UTXOs, same producer set, differing only
in **which producer the withdrawal names**. Rejection is caused by ownership, not by broken wiring.

#### The owner-derivation claim — proven NON-CIRCULARLY

This was the milestone's real risk. If `hash_with_domain(ADDRESS_DOMAIN, pubkey)` were not the
derivation actually used for Bond output `pubkey_hash`, the predicate would silently never match,
every post-AH withdrawal would be rejected, and the fix would be a liveness break worse than the
bug.

The delivered fixture cannot prove this. `inc_i_180_common.rs:128`, `:156` and `:356`
(`seed_bond_utxos`, `seed_bond_utxos_split`, `bond_output`) each construct the Bond output with a
hardcoded `crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes())` — the same
expression the gate evaluates. The developer's `..._same_owner_bond_inputs_are_accepted` control
therefore proves only that the gate agrees with the fixture.

Round 3 severed that circularity. R3-OWN1 and R3-OWN2 build the bonds with the **production**
constructors, seed the resulting `Output` values **verbatim** at the transaction's real hash
(nothing re-derives a `pubkey_hash`), and then withdraw them:

```
R3-OWN1  Transaction::new_add_bond(12 bonds)      -> withdrawal declaring 12 -> ADMITTED
R3-OWN2  Transaction::new_registration(7 bonds)   -> withdrawal declaring  7 -> ADMITTED
```

Both ACCEPT. The gate's derivation matches the real `AddBond` and `Registration` paths.

Static corroboration of the same claim across every Bond-creating site:

| Path | Site | Derivation |
|---|---|---|
| `Transaction::new_registration` | `crates/core/src/transaction/core.rs:231-232` | `hash_with_domain(ADDRESS_DOMAIN, public_key.as_bytes())` |
| `Transaction::new_add_bond` | `crates/core/src/transaction/core.rs:378-379` | `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey.as_bytes())` |
| genesis bond seeding | `bins/node/src/node/genesis.rs:150,158-161` | `hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes())` |
| CLI / GUI | build the transaction through the two constructors above | inherited |

`crypto::PublicKey::address_hash` (`crates/crypto/src/keys.rs:129`) and
`crypto::address::encode` (`address.rs:113`) use the identical expression, so the derivation is
the single canonical address function, not a local convention.

### ISSUE-005 — **FIXED**

The whole INC-I-180 gate block was hoisted to `validation_checks.rs:599`, above the
`=== EpochReward validation ===` section at `:795`. The Full-mode
`[INC_I_081_MISSING_CHECK_SKIP]` `return Ok(())` at `:1030-1034` is now **below** the gate and can
no longer skip it.

**Executed** (round-2 probe R2-E1, re-run as R3-E1 at the devnet epoch boundary h=44 with an empty
block store, so `calculate_epoch_rewards` returns `IncompleteEpochStoreError` — the freshly
snap-synced shape that triggers the early return):

```
R3-E1  h=44 Full   -> [ECON_WITHDRAWAL_OVER_HOLDINGS]      (round 2: ADMITTED)
R3-E1  h=44 Light  -> [ECON_WITHDRAWAL_OVER_HOLDINGS]
R3-E1  h=44 Replay -> [ECON_WITHDRAWAL_OVER_HOLDINGS]
```

The count-binding rule was probed at the same boundary, so **both** new rules are proven
mode-independent there:

```
R3-E2  h=44 Full / Light / Replay -> [ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]  (all three)
```

The hoist also, as a side effect, moved the gate above a **second** early return I had not found in
round 2 — `[INC_I_081_VALIDATION_SKIP]` at `:930-936`, which fires in Full mode for an
epoch-boundary block that *does* carry an `EpochReward` transaction when the local store cannot
recompute it. That path would have skipped the gate too. Both are now closed by position.

---

## PRIORITY 2 — the hoist introduced nothing

| Check | Result | Evidence |
|---|---|---|
| No pre-existing line removed or altered | **PASS** | `git diff --numstat -- bins/node/src/node/validation_checks.rs` = `196  0` — 196 insertions, **0 deletions**. The hoist is relative to the round-2 working tree; against `HEAD` the entire gate is new insertion, so nothing canonical was touched |
| Gate runs in Full / Light / Replay | **PASS** | R3-MODE at `POST_AH=1_000_007`: all three → `[ECON_WITHDRAWAL_OVER_HOLDINGS]`. R3-E1/E2 at the epoch boundary: all three agree. Gate body references `mode` nowhere and now has no early return above it |
| Lock discipline survives the move | **PASS** | utxo read guard scoped to the block expression `:627-670`; producer read guard taken only at `:671`. Executed R3-LOCK (biased `tokio::select!`, future parked not cancelled): `gate_completed_while_producer_write_held=false`, `utxo_try_write_ok=Some(true)` — the utxo guard was genuinely released while the gate waited on the producer guard. Then `gate finished after guard release, ok=true`. No overlap with the opposing pair `rollback.rs:324-326` (producers→utxo) / `apply_block/mod.rs:197-198` (utxo→producers) |
| Nothing the gate depends on is computed later | **PASS** | The gate's only inputs are `height`, `self.config.network.params()`, `self.utxo_set`, `self.producer_set` and `block.transactions` — all parameters or fields. Every local it uses (`withdrawal_gate_ah`, `bond_inputs_by_tx`, `producers`, the two tallies) is defined inside the block. The coinbase rules it now follows (`:578-597`) are unchanged and were already above it; the EpochReward section it now precedes operates on `TxType::EpochReward` only — disjoint from `RequestWithdrawal`/`Exit`/`AddBond` |
| INC-I-080 behaviour unchanged | **PASS** | Cap block at `:1046`, still **below** the real early return at `:1030-1034` (byte offsets confirmed programmatically). R3-E3: a clean epoch-boundary block in Full mode on an incomplete store is still **ADMITTED**, exactly as before the hoist. Devnet pins `addbond_cap_enforcement_activation_height` to `u64::MAX`, so no behavioural fixture can observe the cap itself — the source-position guard is the only executable lock available (see OBS-R3-002 for its weakness) |
| Pre-AH bit-identity, ALL rules (INV-CONSENSUS-002) | **PASS** | Whole block skipped at `:621` (`if height >= withdrawal_gate_ah`). R3-B8p: cross-owner block at h=5 → ADMITTED. R3-E4: over-allowance withdrawal at the pre-AH epoch boundary h=4 → ADMITTED in Full, Light **and** Replay. `in_flight` forced to 0 below the gate at `tx_processing.rs:388-393` and `rewards.rs:1379-1384`. Mainnet is `u64::MAX`, so mainnet replay is untouched by value as well as by code |
| Version bump of any kind | **None** | `git diff --stat -- '*Cargo.toml'` empty. No `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION` / schema line added — the only textual match is a doc sentence in `specs/protocol.md` stating no bump was needed |
| Activation height moved / reused / bundled | **No** | `defaults.rs` diff = 3 insertions, 0 deletions; the field is dedicated. `network_params/mod.rs` is 7/7 but every deletion is a stale **doc comment** on `delegation_auth_activation_height` and `addbond_cap_enforcement_activation_height` being corrected to match their actual defaults — no value changed. `env_loader.rs` locks mainnet to the compiled default, matching the existing pattern |
| Testnet gate is genuinely future | **PASS** | Local testnet `getChainInfo` reports `bestHeight = 216453`; the gate is pinned at `230_000` — ~13.5k blocks ahead, not yet crossed |
| Epoch-boundary flush order | **Untouched** | `git status --short crates/storage/src/` empty |
| Test weakened / deleted / `#[ignore]`-d since round 2 | **No** | Zero `#[ignore]` across the three `tests/it/` trees. All round-2 test names still present. The `..._validation_and_apply_never_disagree` sweep still carries its 9 cases. `..._over_allowance_block_is_rejected` and `..._u32_max_saturates_and_rejects` still reject through the allowance rule, not incidentally through the new owner binding. Suite grew 24 → 34 in `doli-node` |

---

## Test results (executed this round)

| Suite | Command | Result |
|---|---|---|
| `doli-node` deliverable | `cargo test -p doli-node --test it` | **34 passed / 0 failed / 0 ignored** |
| QA round-3 probes | `cargo test -p doli-node --test it qa_r3_probe` | **11 passed / 0 failed** (then removed) |
| Full workspace | `cargo test --workspace --no-fail-fast` (fd limit raised to 65536) | **3688 passed / 3 failed / 43 ignored**, across 176 test targets |
| Build gate | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0, clean** |
| Format gate | `cargo fmt --check` | **exit 0, clean** |

Round 2 did not run the full workspace; this is the real number the milestone green-gate needs.
Note it does **not** match the developer's claim of `1938 passed / 1 failed / 12 ignored` — that
count appears to come from a fail-fast run that stopped at the first failing target. The executed
truth is 3688 / 3 / 43. All three failures are pre-existing; none is attributable to this diff.

### Workspace failure analysis — all 3 failures proven NOT caused by this diff

| Failing test | Occurrences | Cause | Pre-existing? |
|---|---|---|---|
| `test_network::test_cluster_10x100` | 2 (the file is compiled into two targets: `checkpoint_rotation` and `test_network`) | EMFILE. Panic at `bins/node/tests/test_network.rs:55:37`: `Node 69 init failed: database error: IO error: DB::Open() failed … Too many open files`, after successfully completing **7 of 10** clusters of 100 nodes each | **Yes** — environmental, proven at `HEAD` by a detached-worktree control in round 1. Explicitly out of scope for this round |
| `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` | 1 | Assertion at `crates/mempool/src/contention_tests.rs:1108`: `Below inc_i_096 gate, RemoveLiquidity DOLI-outflow must be rejected` — the transaction is admitted where the test expects rejection | **Yes** — see the control below |

⚠ **CONTRADICTION, investigated and resolved.** I expected exactly one known failure and got three.
The third, `inc_i_096_below_gate_rejects_remove_liquidity`, is **not** EMFILE and is not on the
known-issues list, so I did not assume it was pre-existing — I proved it:

1. It reproduces **deterministically in isolation** (`cargo test -p mempool --lib` on the single
   test), so it is not a parallel-execution or contention artifact despite the module name.
2. **HEAD control**: a detached worktree was created at `HEAD` (`ca475a01`, containing none of the
   INC-I-180 changes) and the same test was executed there. It fails **identically**, same
   assertion, same message.

```
git worktree add --detach <tmp> HEAD        # ca475a01, clean tree
cargo test -p mempool --lib inc_i_096_below_gate_rejects_remove_liquidity
  -> Below inc_i_096 gate, RemoveLiquidity DOLI-outflow must be rejected
  -> test result: FAILED. 0 passed; 1 failed
```

The INC-I-180 diff does not touch `crates/mempool/` at all, and the only shared surface —
`crates/core/src/network_params/` — gained one new field whose testnet value (`230_000`) is far
above the height 25_000 this test uses, and which no AMM path reads. **Not caused by this
milestone.** It is nevertheless a genuine main-line defect (a DeFi activation-gate parity test
failing on `main`) and deserves its own ticket — recorded as OBS-R3-006.

**Failures attributable to this diff: zero.**

---

## Failure mode validation (round 3)

| Scenario | Triggered | Detected | Degraded OK | Notes |
|---|---|---|---|---|
| Cross-owner Bond inputs (real bonds) | Yes | **Yes** | Yes | R3-B8 → `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`. Round 2: ADMITTED |
| Same-owner control on the same shape | Yes | n/a | Yes | R3-B8c → ADMITTED. Isolates ownership as the cause |
| Bonds from the real `AddBond` path | Yes | n/a | Yes | R3-OWN1 → ADMITTED. No liveness break |
| Bonds from the real `Registration` path | Yes | n/a | Yes | R3-OWN2 → ADMITTED |
| Epoch boundary, Full mode, incomplete store | Yes | **Yes** | Yes | R3-E1/E2 — all three modes agree. Round 2: Full and Light disagreed |
| Clean epoch-boundary block, Full, incomplete store | Yes | n/a | Yes | R3-E3 → ADMITTED. INC-I-081 skip and INC-I-080's position intact |
| Pre-AH epoch boundary, all modes | Yes | n/a | n/a | R3-E4 → ADMITTED ×3. No leak below the activation height |
| Pre-AH cross-owner | Yes | n/a | n/a | R3-B8p → ADMITTED. Legacy verdict preserved |
| Non-boundary mode divergence | Yes | n/a | Yes | R3-MODE → identical verdict ×3 |
| Lock cycle under contention, post-hoist | Yes | n/a | Yes | R3-LOCK — utxo guard free while parked on producers |

## Security validation (round 3)

| Attack surface | Test performed | Result | Notes |
|---|---|---|---|
| Declared count vs **owner** of the spend | Declare `B`=100, spend 100 of `A`'s real `AddBond` bonds | **PASS** | Rejected. ISSUE-006 closed |
| Consensus-rule bypass via validation mode | Same epoch-boundary block, Full vs Light vs Replay, two distinct rules | **PASS** | Identical verdicts. ISSUE-005 closed |
| Self-declared count vs actual spend | `bond_count=1` with 400 Bond inputs, at an epoch boundary | **PASS** | `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` in all modes |
| Retroactive consensus change | Pre-AH cross-owner and pre-AH epoch-boundary over-allowance | **PASS** | Both still ADMITTED — no already-canonical verdict flips |
| Lock-order denial of service, post-hoist | Park the gate on the producer guard, probe utxo `try_write` | **PASS** | No guard overlap |
| Error-message leakage | Read the three `bail!` strings | **PASS** | Hashed pubkeys and counts only; no keys, paths or stack traces |
| Mempool/builder admission parity | Static | Out of scope | OBS-001, milestone M2 |

## Specs/Docs Drift

| File | Documented behaviour | Actual behaviour | Severity |
|---|---|---|---|
| `specs/protocol.md:608-637` | "rejected at block validation … before any state mutation, so no Bond UTXO is ever spent"; count binding now specified as inputs resolving to a Bond output "whose `pubkey_hash` equals `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey)`"; "the gate is evaluated **before** the EpochReward section … so its verdict is identical in Full, Light and Replay" | **Accurate** — the two round-2 drift lines are closed. Verified against `validation_checks.rs:599-793` and probes R3-B8, R3-E1, R3-E2, R3-MODE | — |
| `docs/error-codes.md:37-39` | Three codes; `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` now states "**owned by the named producer**" with the derivation | **Accurate** — matches the `bail!` strings and the implemented predicate | — |
| `bins/node/src/node/validation_checks.rs:599-615` | The hoisted block's own comment explains why the position is load-bearing and why INC-I-080 stays below | **Accurate** — the round-2 "all modes" comment that was false is gone | — |
| `bins/node/src/node/apply_block/tx_processing.rs:419-431` | Corrected comment on the shortfall branch | **Accurate** (closed round 2) | — |
| `crates/core/src/network_params/mod.rs:327-357` | Doc defaults for `delegation_auth_*` and `addbond_cap_*` corrected to `mainnet 0 / testnet 0 / devnet u64::MAX` | **Accurate** — matches `defaults.rs`. Pre-existing drift, fixed in passing | — |

`CLAUDE.md` and `specs/maintainer-authorization-architecture.md` also appear in `git diff` but
contain zero references to INC-I-180; they are pre-existing unrelated working-tree changes and were
not assessed.

---

## Blocking issues

**None.**

---

## Non-blocking observations (round 3)

- **OBS-R3-001 (MEDIUM, test quality)** — the delivered ownership fixture is circular.
  `inc_i_180_common.rs:128`, `:156`, `:356` build Bond outputs with the same
  `hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes())` expression the gate evaluates, so
  `req_i180_001_post_ah_same_owner_bond_inputs_are_accepted` cannot detect a derivation drift
  between the gate and the production builders. If `Transaction::new_add_bond` ever changed its
  Bond `pubkey_hash` derivation, the fixture would change with it only if someone remembered.
  **Recommendation:** port R3-OWN1/R3-OWN2 into the deliverable suite — build the bonds with
  `Transaction::new_add_bond` / `new_registration` and seed the resulting `Output` values verbatim.
  The gate is correct today (proven this round); this is about keeping it provably correct.

- **OBS-R3-002 (MEDIUM, test quality)** — the new positional guard
  `inc_i_080_addbond_cap_stays_below_the_epoch_reward_return`
  (`inc_i_180_withdrawal_holdings_gate.rs:655-686`) is **self-satisfying**. It computes
  `early_return = SRC.find("[INC_I_081_MISSING_CHECK_SKIP]")`, and `find` returns the **first**
  occurrence — which since the hoist is the marker inside the new INC-I-180 comment at
  **line 602**, not the real early return at **line 1030**. Measured:

  ```
  find(SKIP)            -> line  602   (the comment)
  rfind(SKIP)           -> line 1030   (the real return)
  INC-I-080 cap block   -> line 1046
  assert cap > early_return  ->  1046 > 602  ->  passes trivially
  ```

  The assertion would still pass if the real `return Ok(())` were moved *below* the cap — the exact
  regression it exists to catch. The `gate_block < epoch_section` half is sound. **Recommendation:**
  use `rfind`, or anchor on the enclosing `return Ok(());` rather than the log marker.
  The code position is correct today (verified independently above and by R3-E3), so this is a
  latent test gap, not a live defect.

- **OBS-R3-003 (LOW, accepted)** — post-AH, the owner binding refuses a withdrawal whose Bond UTXOs
  sit at a non-canonical address. That is only reachable through a hand-crafted `AddBond` that
  places Bond outputs somewhere other than
  `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey)`; no production path does. Refusing is the
  conservative direction (the alternative is the ISSUE-006 ledger split) and was accepted for this
  milestone.

- **OBS-R3-004 (LOW)** — the hoist changes error **precedence**, not verdicts. A post-AH block that
  violates both a withdrawal rule and an EpochReward rule now reports the withdrawal code instead of
  the epoch code. Both are rejections, so accept/reject is unchanged on every input and no node can
  diverge; only operator-facing error strings shift.

- **OBS-R3-005 (LOW, pre-existing, noted not blocking)** — `validation_checks.rs` is now 1,506
  lines, over the 500-line module budget. The file was already ~1,400 lines before this milestone;
  splitting a live consensus validator is not a fix-round activity.

- **OBS-R3-006 (MEDIUM, pre-existing, needs its own ticket)** —
  `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity`
  (`crates/mempool/src/contention_tests.rs:989-1115`) fails on `main`. Below the INC-I-096 gate at
  height 25_000 on testnet params, a `RemoveLiquidity` transaction with DOLI outflow is **admitted**
  to the mempool where the naive-conservation rule should reject it with `MPTX008`. Proven
  pre-existing by a `HEAD` detached-worktree control (see the workspace failure analysis). Unrelated
  to INC-I-180 — the diff does not touch `crates/mempool/` — but it is a live DeFi admission-parity
  gap on the default branch and should not stay silent inside a red test suite. DeFi gates are
  frozen at `u64::MAX` on mainnet, so it is not currently exploitable in production.

Round-1 OBS-001..OBS-005 and round-2 OBS-R2-001..OBS-R2-005 remain open as written. Of these,
OBS-R2-004 (`rebuild_producer_set_from_blocks` does not queue
`PendingProducerUpdate::RevokeDelegation`, INC-I-058) is pre-existing and still deserves its own
ticket.

---

## Known items explicitly NOT blocked on

- `test_network::test_cluster_10x100` EMFILE — environmental, proven pre-existing at `HEAD` by a
  detached-worktree control in round 1. See the workspace failure analysis above.
- `inc_i_096_below_gate_rejects_remove_liquidity` — proven pre-existing at `HEAD` by a fresh
  detached-worktree control **this round** (OBS-R3-006). Not caused by this diff.
- **OBS-001** mempool/builder holdings parity — agreed M2 scope.
- `validation_checks.rs` module size — pre-existing (OBS-R3-005).
- Non-canonical Bond address refusal — accepted (OBS-R3-003).
- **FIND-I081-EPOCH-SKIP-001** — the INC-I-080 half of the epoch-boundary skip, already banked as a
  standing P1 for its own separately AH-gated fix. Not re-reported here.

## Modules not validated

- Live devnet/testnet execution past the activation height (static + harness only). The gate is
  `u64::MAX` on mainnet and 13.5k blocks ahead on testnet, so nothing is live yet. Recommend a
  devnet run past h=20 before the testnet gate is approached.

---

## Final verdict

**PASS** — REQ-I180-001, REQ-I180-002 and REQ-I180-003 are met. Both round-2 blockers are closed
and were re-verified by re-executing the exact probes that found them: the cross-owner withdrawal
that was ADMITTED in round 2 is now rejected with `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`, and the
epoch-boundary block that got opposite verdicts in Full and Light now gets the same verdict in all
three modes. The owner-derivation claim — the one way this fix could have become a liveness break —
was proven non-circularly against the production `AddBond` and `Registration` builders, not against
the fixture. The hoist removed nothing (196 insertions, 0 deletions), preserved the lock discipline
under direct empirical challenge, left INC-I-080's cap below the early return, and leaks nothing
below the activation height. No version bump, no activation height moved or reused, no flush-order
change, no weakened test. The full workspace suite runs at **3688 passed / 3 failed / 43 ignored**,
with all three failures proven pre-existing at `HEAD` — including one that was not on the
known-issues list and was verified by control rather than assumed — so zero failures are
attributable to this diff; clippy and fmt are clean. Three MEDIUM observations (OBS-R3-001 circular
ownership fixture, OBS-R3-002 self-satisfying positional guard, OBS-R3-006 pre-existing DeFi
admission-parity failure on `main`) are recorded for follow-up; none is a defect in the behaviour
this milestone ships. **Approved for review.**
