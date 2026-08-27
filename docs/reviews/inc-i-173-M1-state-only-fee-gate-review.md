━━━ FINDINGS — 11 total (Critical:0 Major:3 Minor:8) ━━━

  [F1] MAJOR conf(0.88, observed) — crates/core/src/validation/tx_types.rs:739-778 — REV-173-001: FM-4+FM-11 become REACHABLE above the gate; maintainer-change txs carry no bound on extra_data or signatures and become mineable at ZERO fee, with an O(members x signatures) Ed25519 loop running inside the maintainer_state write lock. F5 is M3.
  [F2] MAJOR conf(0.85, measured) — bins/node/src/node/rollback.rs (0 maintainer refs, grep -c) — REV-173-002: FM-6 becomes REACHABLE; chain data can now mutate the node-local, out-of-state-root MaintainerState, and no reorg/snap-sync path undoes or replays it. F6 is M3.
  [F3] MAJOR conf(0.95, measured) — crates/core/src/network_params/defaults.rs:480 — REV-173-003: testnet AH 133_000 is a DECAYING asset; live tip measured 130_432 at unix 1786383633, leaving 2_568 blocks ~= 7 h 08 m. This already decayed once (QA ISSUE-001); nothing in the tree prevents a third decay.
  [F4] MINOR conf(0.92, measured) — bins/node/tests/inc_i_173_state_only_fee_gate.rs:383,394,399,405 — REV-173-004: the C8 ValidationMode test is TAUTOLOGICAL; CONSENSUS_TEST_MODE has four occurrences, all self-referential, and never drives a validation.
  [F5] MINOR conf(0.99, observed) — specs/SPECS.md:43 — REV-173-005: the index row labels the spec "PROPOSAL-ONLY" while specs/state-only-fee-gate-architecture.md:17 states "M1 IMPLEMENTED (F1+F2+F3)".
  [F6] MINOR conf(0.90, measured) — crates/core/src/validation/transaction.rs:39-88 — REV-173-006: the F7 implication allows_empty_io(t) => t in L1 and L2 holds for all 5 exempt types (hand-verified this review) but is machine-checked NOWHERE in M1; an inert-fix regression is undetected.
  [F7] MINOR conf(1.00, measured) — crates/core/src/transaction/types.rs:500 — REV-173-007: the file sits at exactly 500/500 lines with zero headroom; the next arm added to the consensus authority breaks the module budget.
  [F8] MINOR conf(0.90, measured) — crates/core/tests/inc_i_173_activation_height.rs:208-262 — REV-173-008: the "no existing height moved" baseline pins 7 of ~20 heights, so it is a sample, not the total property it claims.
  [F9] MINOR conf(0.85, observed) — crates/mempool/src/pool.rs:161,933-934 — REV-173-009: FM-9 confirmed with fresh evidence; entries is a HashMap and sort_by_key is stable, so ties among 0-fee governance txs resolve in HashMap iteration order. The C9/BRIDGE purge of the 3 stuck testnet txs is UNEXECUTED.
  [F10] MINOR conf(0.90, measured) — crates/core/src/network_params/defaults.rs:164 — REV-173-010: spec Follow-up 4 over-states severity. defi_activation_height has 6 write sites and ZERO readers; the escalation target is the stale INV-DEFI-001 record and CLAUDE.md, not mainnet consensus.
  [F11] MINOR conf(0.80, observed) — crates/core/src/transaction/types.rs:182 — REV-173-011: allows_empty_io() is `pub`, so C2 is enforced by convention (exactly one production caller) rather than by visibility or type.

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-173 M1 — State-Only Fee Gate

**Agent:** reviewer · **run_id:** 511 · **Incident:** INC-I-173 · **Milestone:** M1
**Branch:** `bugfix/inc-i-173-state-only-fee-gate` (UNCOMMITTED working tree) · **Base:** `b5f68bba`
**Date:** 2026-08-10 · **Workflow:** redesign of a consensus-critical transaction validator.
Question answered: *is this the right change, is it complete, and did anything unintended slip in?*

---

## Summary

**APPROVED.** The code is correct, complete for F1+F2+F3, and nothing unintended slipped in.

The single strongest piece of evidence: `git diff b5f68bba` over the **whole tree** contains exactly
**six deleted lines** — the original `is_state_only_tx` expression — and those six lines are re-added
verbatim inside the `else` branch. Every other line in the change is an addition. There is no other
deletion, no reformat, no drive-by edit, in any of the 11 modified files.

All three MAJOR findings concern what the gate *opens*, not what the code *does*. Each is a
spec-acknowledged M3 deferral, each is fail-closed on mainnet (`u64::MAX`), and each becomes live on
**testnet at height 133_000**, currently ~7 hours away. They are deploy preconditions, not merge
blockers, and are stated as such.

---

## Scope Reviewed

9 source files + 5 test files (working tree vs `b5f68bba`), `specs/state-only-fee-gate-architecture.md`,
`docs/redesigns/state-only-fee-gate-redesign-analysis.md`, the QA report, the developer report, plus
the following independently re-derived from code: `TxType` declaration, L1/L2 structural chains, all
`ValidationContext::new` and `validate_transaction_with_utxos` call sites workspace-wide, the five
exempt types' apply-path authorization, both exclusions' cited reasons, `select_for_block`,
`NetworkParams` derive set, `defi_activation_height` readers, `rollback.rs` maintainer references,
and the live testnet tip.

---

## 1. Root cause vs patch — does the drift seam actually close?

**It closes, and I tried to defeat the claim three ways. All three failed.**

| Attack on the "new TxType = BUILD FAILURE" claim | Result |
|---|---|
| Is `TxType` `#[non_exhaustive]`? | **No.** `grep -rn non_exhaustive crates/core/src/transaction/` returns zero hits; `types.rs:5-7` derives only `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`. Irrelevant anyway — `allows_empty_io` is defined in the same crate, where `non_exhaustive` does not apply. |
| Could a new variant get a default? | **No.** `Default` is not derived and no `#[default]` attribute exists on any variant. |
| Is there a `_` arm, or an arm group that would silently absorb a new variant? | **No.** All 24 arms are written out individually (`types.rs:184-209`), one variant per arm, no `|` grouping, no `_`. |

The 24 arms match the 24 live variants exactly (`types.rs:9-140`; discriminants 24-30 are tombstoned
and unrepresentable). This is a genuine one-owner authority, not a third victim enumeration.

**Residual, honest:** the compiler forces *classification*, not *correct* classification. A developer
under build pressure who writes `=> true` for a new unauthenticated type gets no signal. F7 (M3) is the
only guard for that combination and is not shipped — see [F6].

## 2. Correctness of the exempt set (F3 / C1) — independently verified

Every claim below was re-derived from code this review, not taken from the spec.

**The five `true` arms — each has an authenticating apply path:**

| Type | Auth path | Evidence |
|---|---|---|
| `Registration` | VDF hash-chain; the 0-in/0-out form is the protocol-generated genesis one | already exempt below the gate — **not a widening** |
| `DelegateBond` | Ed25519 `verify_hash(signing_message, signature, delegator)`, height-gated INC-I-078 | `apply_block/tx_processing.rs:445-452` |
| `RevokeDelegation` | same INC-I-078 path | already exempt below the gate — **not a widening** |
| `AddMaintainer` | `ms.set.verify_multisig_at(&data.signatures, &message, ...)` before `add_maintainer` | `apply_block/governance.rs:38-44` |
| `RemoveMaintainer` | `ms.set.verify_multisig_excluding_at(...)` before `remove_maintainer` | `apply_block/governance.rs:71-77` |

**The delta above the gate is exactly two types** — `AddMaintainer`, `RemoveMaintainer`. The other
three are already in the frozen below-gate list, so C3 (strict superset) holds by construction and the
change surface is the minimum possible. QA's "16 intended flips" is consistent with 2 types across the
tested shape/height grid.

**The two exclusions — both cited reasons verified CORRECT, in both directions:**

- `Exit` — `ExitData` carries no signature; `validate_exit_data` (`validation/tx_types.rs:11-42`)
  performs no crypto check; the apply handler at `apply_block/tx_processing.rs:262-295` force-queues
  `RequestWithdrawal` for **all** bonds of the named pubkey with no ownership proof. Exclusion correct.
- `SlashProducer` — `grep -rn reporter_signature crates bins` returns **exactly one production writer**
  (`crates/network/src/sync/equivocation.rs:55`) and **zero verifiers**. The field is signed and never
  checked. Exclusion correct.

Had either exclusion been wrong the other way (i.e. the type actually authenticated), M1 would be
leaving a liveness bug in place; had either been wrongly included, M1 would ship a free unauthenticated
primitive. Neither happened.

**`ProtocolActivation` / `PriceAttestation` = `false` is correct M1 scope.** They are spec Options A and
B, explicitly deferred. `ProtocolActivation` additionally dies earlier at mempool `FeeTooLow`, so
classifying it `true` alone would be inert; `PriceAttestation` is unreachable
(`oracle_activation_height = u64::MAX`). **No landmine:** `false` is the fail-safe direction — it
preserves today's behavior exactly. The only cost is that INC-I-173 will re-fire for
`PriceAttestation` if oracle is ever activated without revisiting this arm.

## 3. Activation-height discipline — all six checks pass

| Check | Result | Evidence |
|---|---|---|
| No EXISTING activation height moved anywhere | **PASS** | `git diff b5f68bba -- defaults.rs \| grep "^-"` returns zero lines; the whole-tree removal set is the 6 gate lines only |
| No version constant bumped | **PASS** | zero diff on `crates/network/src/protocols/status.rs` (`CURRENT_PROTOCOL_VERSION: u32 = 8`, `EPOCH_STATE_FORMAT_VERSION = 1`, `MIN_PEER_PROTOCOL_VERSION = 1` all untouched); zero `Cargo.toml` diff |
| No `HardForkSchedule` entry | **PASS** | zero diff on `crates/updater/src/hardfork.rs` |
| No genesis reset / chain identity perturbation | **PASS** | `NetworkParams` derives **only** `Debug, Clone` (`network_params/mod.rs:49`) — it is never serialized, hashed, or wire-encoded, so adding a field cannot perturb `genesis_hash` or `fork_id` |
| NOT bundled onto `maintainer_derivation_activation_height` | **PASS** | new dedicated field; asserted at `inc_i_173_activation_height.rs:182-198` against 4 neighbours |
| testnet `133_000 > 127_200` | **PASS** | `defaults.rs:480` vs `:449`; assertion at `inc_i_173_activation_height.rs:145` |

**Re-pin-history comment (`defaults.rs:454-479`) — present, honest, complete.** It records BOTH pins
(`u64::MAX -> 130_400`, then `130_400 -> 133_000`), tip at each pin, the measured rate with the
sampled height range and raw timestamps, the lead in blocks and hours, and the decay reason. I
re-derived the second sample independently: `(1786382169 - 1786372169) / (130_286 - 129_286)` =
`10000/1000` = exactly `10.00 s/block`. The comment does not round in its own favour.

Env plumbing is mainnet-locked (`env_loader.rs:437-447`), matching the
`maintainer_derivation_activation_height` precedent exactly.

## 4. The both-or-neither constraint (F2) — complete call-site enumeration

`validate_transaction_with_utxos` has **exactly two** production callers workspace-wide:

- `bins/node/src/node/production/assembly.rs:242` — context built at `:186`, height wired at `:219-224`
- `bins/node/src/node/apply_block/tx_processing.rs:106` — context built at `:61`, height wired at `:94-100`

Both pass `self.config.network.params().inc_i_173_activation_height` — no literal. Every other hit in
the tree is a `pub use` re-export, a doc comment, or an in-crate test.

`ValidationContext::new` has **six** production sites. The four not wired are correct as-is:

| Site | Reaches the gate? | Verdict on leaving it at `u64::MAX` |
|---|---|---|
| `crates/mempool/src/pool.rs:363` (`add_transaction`) | No — never calls `validate_transaction_with_utxos` | **Correct.** The field is simply unread. Maintainer txs already reach the mempool through `add_system_transaction` via `is_state_only()` (`transaction/core.rs:463-476`, which DOES list both maintainer types), so admission needs nothing from M1. |
| `crates/mempool/src/pool.rs:742` (`add_system_transaction`) | No | **Correct**, same reason. |
| `bins/node/src/node/validation_checks.rs:103` | No — block-level context | **Correct.** |
| `bins/node/src/node/validation_checks.rs:283` | No — block-level context | **Correct.** |

The fail-closed `u64::MAX` default is the right choice for all four: a forgotten site stays *below* the
gate, which is a liveness bug, never a silent consensus divergence. Note the asymmetry is real and the
comments at both wired sites state it correctly — forgetting the *apply* site is the self-forking one.

**INV-VALIDATION-001 check:** the change is a strict RELAXATION above the gate (fewer rejects), so a
mempool that stays stricter cannot produce block poisoning. Direction is safe.

## 5. Below-gate bit-identity (INV-COMPAT-001) — textual proof

```
- let is_state_only_tx = tx.inputs.is_empty()          (b5f68bba:utxo.rs:222-227)
+ } else {                                              (working tree:utxo.rs:244-252)
+     tx.inputs.is_empty()
+         && tx.outputs.is_empty()
+         && matches!(tx.tx_type, TxType::Registration | TxType::DelegateBond | TxType::RevokeDelegation)
+ };
```

Token-for-token identical; the only difference is four spaces of rustfmt indentation inside the `else`.

- Comparison is `>=` (`utxo.rs:239`), matching the `inc_i_096` twin idiom six lines below at `:266`.
- Height source is `ctx.current_height` — the **BLOCK's** height. `chain_state.best_height` appears
  nowhere in `validation/utxo.rs`. Both call sites pass their local `height`
  (`assembly.rs:190`, `tx_processing.rs:65`). **C4 honored.**
- The gate is inside the shared validator, so INV-PROD-003 parity is a construction property. Two
  source-text tests forbid either node file from growing its own comparison or calling `is_zero_flow()`
  (`inc_i_173_state_only_fee_gate.rs:357-375`).

## 6. The mint guard (C2)

`is_zero_flow()` (`transaction/core.rs:500-502`) keeps `inputs.is_empty() && outputs.is_empty()` as a
conjunct **inside** the function. Verified by exhaustive grep across `crates/core/src`, `bins`,
`crates/mempool/src`, `crates/rpc/src`:

- `allows_empty_io()` — **exactly one** production call site: inside `is_zero_flow()` (`core.rs:501`).
- `is_zero_flow()` — **exactly one** production call site: `validation/utxo.rs:240`.

No consensus path evaluates the type half alone. `ClaimReward`/`ClaimBond` (0-in, value-output) cannot
ride the widened list into a mint. See [F11] for the residual.

## 7. Scope boundary — F4-F7 leakage: NONE; M1 omissions: NONE

| M3 item that must NOT be here | Status |
|---|---|
| F4 — delete `is_state_only()` | **Absent.** `transaction/core.rs:456-476` is byte-unchanged, still lists 9 types incl. the false doc claim. Correctly untouched. |
| F5 — bound `extra_data` / `signatures` | **Absent.** `validate_maintainer_change_data` (`validation/tx_types.rs:739-778`) has no length check. See [F1]. |
| F6 — maintainer digest RPC | **Absent.** No `crates/rpc` diff. |
| F7 — cross-list total test | **Absent.** See [F6]. |
| L1/L2 (`validation/transaction.rs:39-88`) | **Character-identical.** Zero diff on that file. |
| Options A-E | **Absent.** `ProtocolActivation`/`PriceAttestation` both `false`. |

Nothing M1 required was skipped: F1a (`allows_empty_io`), F1b (`is_zero_flow`), F1c (the gate), F2a
(`NetworkParams` field), F2b (3 per-network literals + env), F2c (`ValidationContext` field + builder),
F2d (both call sites), F3 (curated set) all landed in the exact files the spec named.

## 8. Audit of the four test-harness fixes — all four legitimate

I re-derived 6.1 and 6.2 from `git show b5f68bba:crates/core/src/network_params/defaults.rs` myself.

**6.1 `inc_i_147` transposition — CONFIRMED, fix correct.** Baseline `:251` is inside the
`Network::Mainnet =>` block (which starts at `:18`, ends where `Network::Testnet =>` begins at `:283`)
and reads `129_500`. Baseline `:430` is inside the Testnet block and reads `80_700`. The test as
written had them reversed. Making the *code* pass would have required moving a mainnet activation
height — the precise INC-I-054 failure the test exists to forbid. Both values remain pinned, one per
network; assertion strength unchanged.

**6.2 mainnet `defi_activation_height` — CONFIRMED, fix correct, and the escalation resolves DOWNWARD.**
Baseline `:164` (Mainnet block) is the literal `0`; testnet `:391` and devnet `:557` are `u64::MAX`.
CLAUDE.md and INV-DEFI-001 both claim `u64::MAX` on all three networks. Code is SoT, so pinning `0` is
right. **But I went further and traced the reader**, which the spec's Follow-up 4 did not:
`defi_activation_height` is written into `ValidationContext` at six sites (`pool.rs:374,753`,
`validation_checks.rs:141,333`, `tx_processing.rs:86`, `assembly.rs:211`) and **compared nowhere**;
`DefiNotActivated` and `DEFI_NOT_ACTIVATED` do not exist anywhere in `crates` or `bins`. The field is
dead. Mainnet `0` is therefore **inert**, not a live consensus drift. See [F10] for the real
escalation target.

**6.3 HardFork counter — CONFIRMED, fix strictly stronger.** `hardfork.rs` has zero diff; the third
`matches()` hit is the rustdoc example at `:158`. Filtering comment lines makes a genuine new entry
still fire while a doc example no longer does.

**6.4 clippy allow — CONFIRMED, narrowest possible.** `#[allow(clippy::assertions_on_constants)]` on
one test whose subject *is* const-evaluability; assertions unchanged and still execute at runtime.
Preferring this over `const { assert!(..) }` correctly keeps the failure at test time.

None of the four weakened an assertion. All five in-place `HARNESS FIX` annotations are present and
accurate.

## 9. Specs/docs drift

- `specs/state-only-fee-gate-architecture.md` — accurate. Header updated to "M1 IMPLEMENTED", the
  pinned heights and the two-entry re-pin history match `defaults.rs` exactly. Follow-up 4's severity
  is over-stated — see [F10].
- `docs/redesigns/state-only-fee-gate-redesign-analysis.md` — traceability matrix present.
- `specs/SPECS.md:43` — **stale**, see [F5].
- `docs/DOCS.md:77` — says "drift between 5 hand-maintained lists" where the spec settled on six
  (L1-L6). Cosmetic; the analysis doc genuinely said five before the synthesizer found L6. Not filed.
- `CLAUDE.md` — "Oracle + DeFi gates are `u64::MAX`" is false for mainnet DeFi. Folded into [F10].

## 10. The 500-line deviation

`crates/core/src/transaction/types.rs` is at **exactly 500/500**. The compressed documentation is
**adequate**: the three-line doc states the ownership claim, the authorization-not-shape rule, and the
no-`_`-arm requirement; both excluded arms carry their cited reason inline; the long-form rationale
lives in `is_zero_flow()`'s 19-line doc (`core.rs:478-499`) and the `NetworkParams` field doc
(`mod.rs:594-635`) — both of which a reader reaches from the arm comments. **No split needed for this
change.** But see [F7]: the headroom is zero, and this file is now a consensus authority, so the split
must happen before the next arm, not after.

## 11. Failure modes — FM-1..FM-12 for the commit's `Failure-Modes:` block

| FM | What it is | M1 posture |
|---|---|---|
| FM-1 | Forged `SlashProducer` evidence (`reporter_signature` 0 verifiers, VDF publicly computable) | **AVOIDED** — `SlashProducer=false` keeps the fee gate as the accidental block. Underlying auth gap OPEN, own incident. |
| FM-2 | Unbounded thread-per-tx VDF pre-pass (`validation/block.rs:145-180`), C10 | **NOT TRIGGERED** — no exempted type feeds it. Pre-existing, OPEN. |
| FM-3 | Unauthenticated forced-`Exit` primitive | **AVOIDED** — `Exit=false`. Underlying gap OPEN, own incident. |
| FM-4 | Free permanent storage via unbounded `extra_data` on a newly-exempt type | **NEWLY REACHABLE above the gate. OPEN.** F5 is M3. See [F1]. |
| FM-5 | Governance signature replay (`signing_message` has no nonce/height/chain-id/expiry) | **NEWLY REACHABLE above the gate. OPEN.** Option E deferred. A replayed `remove:X` set now lands on-chain instead of dying in the mempool. |
| FM-6 | Trust-root divergence via reorg / snap-sync / non-fatal `warn!` | **NEWLY REACHABLE above the gate. OPEN.** F6 is M3. See [F2]. |
| FM-7 | Activation-boundary partition; old binaries reject valid blocks | **CLOSED for mainnet** (`u64::MAX`); **MITIGATED for testnet** by the 133_000 lead. Residual is [F3]. |
| FM-8 | Slash re-enabling has retroactive reach | **NOT APPLICABLE** — `SlashProducer` excluded. |
| FM-9 | Governance outcome depends on builder-local map ordering | **OPEN.** Confirmed with fresh evidence, see [F9]. The C9/BRIDGE purge is unexecuted. |
| FM-10 | `evict_lowest_fee` preferentially evicts the 0-fee lane (free-relay amplification) | **OPEN, unchanged by M1.** F4 is M3. Liveness residual for governance txs. |
| FM-11 | Quadratic maintainer multisig verify over an attacker-sized `Vec` | **NEWLY REACHABLE above the gate. OPEN.** F5 is M3. See [F1]. |
| FM-12 | Genesis re-validation and fresh sync | **CLOSED.** `Registration` stays exempt at every height (C3 strict superset) and the below-gate branch is token-identical, so genesis and historical replay are unperturbed. |

Suggested commit block: `Closes FM-12; avoids FM-1/FM-2/FM-3/FM-8 by exclusion; mitigates FM-7 by
activation height (mainnet fail-closed); OPEN and newly reachable above the gate: FM-4, FM-5, FM-6,
FM-11 (F5/F6/Option E are M3); OPEN and unchanged: FM-9, FM-10.`

---

## Major Findings

### REV-173-001 [F1] — FM-4 + FM-11 become reachable; the newly-exempt lane is free and unbounded

**Location:** `crates/core/src/validation/tx_types.rs:739-778`; `bins/node/src/node/apply_block/governance.rs:34-44`
**Evidence:** `validate_maintainer_change_data` checks only `inputs.is_empty()`, `outputs.is_empty()`,
`!extra_data.is_empty()` and that `MaintainerChangeData::from_bytes` succeeds — **no bound on
`extra_data.len()`, no bound on `signatures.len()`, no signature verification.** Admission is already
free: `is_state_only()` (`transaction/core.rs:463-476`) routes both maintainer types to
`add_system_transaction`, which skips the fee and signature checks. Above the gate the builder no longer
skips them (`assembly.rs:242`) and the apply handler runs `verify_multisig_at` while holding
`maintainer_state.write().await` (`governance.rs:36-44`).
**Impact:** above the gate, any actor can have zero-fee maintainer-change transactions with junk
signatures mined into permanent chain history, and each one costs every node an O(members x signatures)
Ed25519 loop inside a write lock, on every apply and every replay. Per-block volume is bounded by
`max_block_user_data`, but the price is zero, so it is a block-space censorship lever as well.
**Confidence:** conf(0.88, observed)
**Severity:** Major
**Suggested Fix:** do not merge F5 into M1 — the deferral is correct. Instead make it a *hard*
precondition: (a) record a protection-registry entry stating that pinning any non-`u64::MAX`
`inc_i_173_activation_height` on mainnet REQUIRES F5 in the same binary; (b) state the same in the M4
step of the spec's Migration Path, which currently only implies it through milestone ordering.

### REV-173-002 [F2] — FM-6 becomes reachable; chain data can now mutate out-of-state-root node-local state

**Location:** `bins/node/src/node/rollback.rs`; `bins/node/src/node/apply_block/governance.rs:46-56,79-89`
**Evidence:** `grep -c maintainer bins/node/src/node/rollback.rs` returns **0**, and the same grep over
`block_handling.rs` returns nothing. The apply handler persists to `<data_dir>/maintainer_state.bin`
and logs failures as `warn!`, never fatally. INC-I-172 deliberately kept `MaintainerState` out of the
state root for fork safety — a premise that held only while nothing on-chain could mutate it. M1 breaks
that premise.
**Impact:** above the gate a reorged-out `RemoveMaintainer` is not undone, a snap-synced node never
replays it, and a failed persist is silent. Nodes can agree on every block while disagreeing about
which binaries they trust. This interacts directly with PM-172-03/PM-172-04: an unseeded or divergent
root is an absorbing state for governance.
**Confidence:** conf(0.85, measured)
**Severity:** Major
**Suggested Fix:** same precondition shape as [F1] — F6 (the chain-derived digest RPC, spec option "c")
must be in the binary before any non-`u64::MAX` mainnet pin. Also update PM-172-03's registry entry:
its trigger surface is no longer "the persisted file plus the chain height" alone, because chain
content can now change the set.

### REV-173-003 [F3] — the testnet activation height is a decaying asset with no guard

**Location:** `crates/core/src/network_params/defaults.rs:480`
**Evidence:** live testnet tip measured by this review at `getChainInfo` on `127.0.0.1:8500`:
`bestHeight = 130_432`, node version `6.24.1`, at unix `1786383633`. Pinned AH `133_000`. Remaining
lead = `2_568` blocks at the developer's and QA's independently measured `10.00 s/block` =
**~7 h 08 m**, expiring around unix `1786409313`. QA measured 130_364 at 18:29 WEST; my 130_432 is
consistent (68 blocks / ~11 min later).
**Impact:** this exact decay already consumed one full QA iteration (ISSUE-001: `130_400` decayed from
781 blocks of lead to ~120 before deployment). Nothing in the tree prevents a third occurrence. If the
fleet crosses 133_000 un-upgraded, the mixed-fleet purpose of the gate is void for testnet.
**Correction to the recorded rationale, for precision:** the claim that a decayed pin "would freeze a
wrong value permanently (INC-I-054)" is true only once a binary carrying the value is running as the
chain crosses it. Nothing is deployed today, so a third re-pin would still be legal. Do not let that
belief force a rushed deploy — it would be exactly the wrong reaction.
**Confidence:** conf(0.95, measured)
**Severity:** Major
**Suggested Fix:** state the absolute deadline (testnet height 133_000) in the commit message, and gate
the M2 deploy on a fresh `getChainInfo` read rather than on the pinned comment. If the remaining lead
at deploy time is under ~1_000 blocks, re-pin before deploying rather than after.

---

## Minor Findings

**REV-173-004 [F4] — the C8 ValidationMode test is tautological.**
Location `bins/node/tests/inc_i_173_state_only_fee_gate.rs:383,394,399,405`. Evidence: `grep -n
CONSENSUS_TEST_MODE` returns four hits — the declaration and three assertions *about the declaration*.
The constant never parameterises a call. `req_173_003_c8_the_declared_validation_mode_is_full_not_replay`
asserts a `const` equals the literal it was assigned. Confidence conf(0.92, measured). Severity Minor.
Mitigating fact, verified: the substantive below-gate work runs in `crates/core/tests/inc_i_173_fee_gate.rs`
against `validate_transaction_with_utxos` **directly**, where no `ValidationMode` exists and therefore no
swallow path exists — strictly stronger than `Full`. The companion source-text test at `:419-429` is a real
guard. Fix: delete the tautology or make the constant drive an actual `apply_block`-level assertion; M2's
REQ-173-008 will cover the real property.

**REV-173-005 [F5] — SPECS.md index says PROPOSAL-ONLY.** Location `specs/SPECS.md:43` vs
`specs/state-only-fee-gate-architecture.md:17`. Evidence: the index row opens "(INC-I-173,
PROPOSAL-ONLY, 2026-08-10)" while the spec header states "Status: M1 IMPLEMENTED (F1+F2+F3)".
conf(0.99, observed). Severity Minor. Fix: change the index row to "M1 IMPLEMENTED; F4-F7 + Options A-E
PROPOSAL".

**REV-173-006 [F6] — the F7 implication is unproven by machine.** Location
`crates/core/src/validation/transaction.rs:39-88`. Evidence: I hand-verified it this review — L1 (`:39-63`)
and L2 (`:67-88`) both carry `!tx.is_registration()`, `!tx.is_delegate_bond()`,
`!tx.is_revoke_delegation()`, `!tx.is_maintainer_change()`, so all five exempt types are structurally
permitted to be 0-in/0-out and the fix is **not inert**. conf(0.90, measured). Severity Minor. Fix: land
F7's one-way implication test in M3 as specified; nothing else machine-checks the inert-fix regression.

**REV-173-007 [F7] — zero headroom in a consensus authority file.** Location
`crates/core/src/transaction/types.rs:500`. Evidence: `wc -l` = 500, budget = 500. conf(1.00, measured).
Severity Minor. Fix: split `types.rs` (e.g. `TxType` classification methods into
`transaction/classification.rs`) as its own change, before the next arm is added — not bundled into a
consensus fix.

**REV-173-008 [F8] — "no existing height moved" is a sample, not a total.** Location
`crates/core/tests/inc_i_173_activation_height.rs:208-262`. Evidence: 7 assertions
(`maintainer_derivation` x3, `inc_i_147` x3, `oracle`, `defi`) against ~20 activation-height fields on
`NetworkParams`. conf(0.90, measured). Severity Minor. Mitigating: the whole-tree diff check is
stronger and passes. Fix: replace with a golden snapshot over every `*_activation_height` field per
network so the property is total.

**REV-173-009 [F9] — FM-9 confirmed; the BRIDGE purge is unexecuted.** Location
`crates/mempool/src/pool.rs:161,933-934`. Evidence: `entries: HashMap<Hash, MempoolEntry>` at `:161`;
`select_for_block` collects that map into a `Vec` at `:933` and applies `sort_by_key` at `:934`, which
is **stable** — so ties among equal `effective_fee_rate` (all 0-fee governance txs) preserve HashMap
iteration order, which is per-process randomised. `select_for_block` applies no fee-rate floor, so
0-fee txs *are* offered to the builder — the fix is not inert at the selection layer either.
conf(0.85, observed). Severity Minor. Fix: execute the spec's BRIDGE step — purge or expire the three
stuck INC-I-172 testnet governance txs **before** height 133_000, and verify the purge by RPC rather
than assuming `max_age` expiry.

**REV-173-010 [F10] — the escalation target is the invariant record, not mainnet.** Location
`crates/core/src/network_params/defaults.rs:164`. Evidence: six writers of `defi_activation_height` into
`ValidationContext`, **zero** readers; `DefiNotActivated` / `DEFI_NOT_ACTIVATED` do not exist in
`crates` or `bins`. conf(0.90, measured). Severity Minor. Assessment: **does NOT need escalating as a
live consensus drift** — mainnet `0` is inert because the field has no consensus reader. What *does*
need correcting is (a) `INV-DEFI-001` in `.omega/memory.db`, which is `status='active'` and describes
`ValidationError::DefiNotActivated` enforcement that no longer exists in code, (b) CLAUDE.md's
"Oracle + DeFi gates are `u64::MAX`" claim, and (c) spec Follow-up 4's "HIGH — possible live consensus
drift" wording. A stale active invariant is more dangerous than a dead field: it will be cited as
protection that is not there.

**REV-173-011 [F11] — C2 is enforced by convention, not by the type system.** Location
`crates/core/src/transaction/types.rs:182`. Evidence: `allows_empty_io` is `pub` and re-exported with
`TxType`; the conjunct lives only in `is_zero_flow`. conf(0.80, observed). Severity Minor. Mitigating:
`pub` is currently *required* because `crates/core/tests/inc_i_173_zero_flow_predicate.rs` is an
integration test in a separate crate. Fix: add a source-text test asserting `allows_empty_io(` has
exactly one production call site (the same idiom already used at `inc_i_173_state_only_fee_gate.rs:357-375`),
so a future bypass fails CI rather than review.

---

━━━ RESOURCE COST — SUMMARY — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 — the recommended fixes are one registry row, three doc/index string edits, two test additions and one operational purge; none is compiled into a hot path (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 — the BRIDGE purge is a one-time RPC/mempool operation on the local testnet (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: accept the three MAJOR findings as recorded-and-unmitigated, relying on milestone ordering (M3 precedes M4) to deliver F5/F6 before any mainnet pin
Why this proposal anyway: milestone ordering is a plan, not a gate — INC-I-054 and INC-I-153 both shipped through an ordering everyone believed held; a protection-registry row and a stated deploy precondition cost nothing at runtime and are the only artifacts that survive a session boundary
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

The reviewed change itself carries the spec's F1+F2+F3 cost: +1 `u64` compare and one exhaustive-match
dispatch per non-coinbase transaction (replacing an existing 3-arm `matches!`), +8 bytes per
`ValidationContext`, zero IO/disk/network below the gate. Hot-path impact is not a finding: the compare
sits on a path already performing at least one BLAKE3 hash and one signature verification per input,
and the upstream proposal declared it (`specs/state-only-fee-gate-architecture.md:189-200`).

---

## Preconditions before the testnet gate crosses (deploy gates, not merge blockers)

1. Execute the spec BRIDGE step — purge the three stuck INC-I-172 governance txs before height 133_000 ([F9]).
2. Re-read the live tip immediately before the M2 deploy; re-pin if the lead is under ~1_000 blocks ([F3]).
3. Watch for FM-4/FM-11 on testnet above 133_000 — an oversized or many-signature maintainer tx is now mineable ([F1]).

## Preconditions before any non-`u64::MAX` mainnet pin (M4)

1. F5 (footprint bound) in the same binary ([F1]).
2. F6 (trust-root digest RPC) in the same binary ([F2]).
3. F7 (cross-list implication test) landed ([F6]).
4. Fleet version sweep confirming coverage of the ~30 external auto-update producers.

## Modules Not Reviewed

None in scope. `crates/rpc`, `crates/storage` and `crates/network` carry no diff and were consulted
read-only for the `reporter_signature` and `NetworkParams` serialization checks.

---

REVIEW VERDICT: APPROVED

SECURITY AUDIT VERDICT: AUDIT-REQUIRED

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: (1) trust boundary — the change alters a consensus validator's fee/balance exemption for
user-submittable transactions reachable from the public `sendTransaction` / `submitMaintainerChange`
RPC; (2) authorization — the exempt set IS an authorization decision, and two adjacent types
(`Exit`, `SlashProducer`) were verified this review to have no actor authentication at all; (3) state
integrity — above the gate, chain content can mutate the node-local maintainer trust root that governs
which binaries the fleet installs, with no reorg undo and no snap-sync replay ([F2]); (4) external data
— a newly-mineable lane with no bound on `extra_data` or `signatures` feeds an O(n) Ed25519 loop inside
a write lock ([F1]); (5) enforcement surface — the maintainer trust root is the update-signing
authority, so this change touches the deploy-enforcement surface itself. The activation gate and the
authorization-curated set reduce likelihood; they do not remove any of these signals.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
