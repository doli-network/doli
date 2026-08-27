# QA Report: INC-I-173 M1 — State-Only Fee Gate

**Agent:** qa · **run_id:** 511 · **Incident:** INC-I-173 · **Milestone:** M1
**Branch:** `bugfix/inc-i-173-state-only-fee-gate` (UNCOMMITTED working tree)
**Baseline:** `b5f68bba`
**Iteration 1:** 2026-08-10 18:12 WEST — FAIL (1 blocker)
**Iteration 2:** 2026-08-10 18:29 WEST — **PASS** (blocker resolved and independently re-verified)
**Workflow type:** redesign of a consensus-critical validator.
Primary question: *did behavior change anywhere it was not supposed to?*

---

## Summary

**PASS — all eight in-scope Must requirements met; the single iteration-1 blocker is closed.**

The implementation is sound. I proved below-the-gate bit-identity **empirically**, not
textually: 840 verdicts (24 `TxType` × 7 tx shapes × 5 below-gate heights) produced by the
working tree are **byte-for-byte identical** to the verdicts produced by a detached worktree
built at `b5f68bba`. Above the gate exactly **16** verdicts change, all of them
`AddMaintainer`/`RemoveMaintainer` in the 0-in/0-out shape flipping `InsufficientFee → OK` —
precisely the defect INC-I-173 exists to fix, and nothing else. The mint guard survived every
attack I could construct. All four developer test-harness fixes are legitimate; none weakened
an assertion. Both §8 failures are confirmed not caused by this change.

Iteration 1's only blocker was the pinned testnet activation height `130_400`, whose safety margin
had decayed to ~136 blocks (≈23 min). The developer re-pinned it to `133_000`. I re-measured the
live tip and block rate **myself** at 18:29 WEST: tip **130,364**, **exactly 10.00 s/block**,
remaining lead **2,636 blocks ≈ 7.32 hours** — sufficient for the remainder of M1 plus the M2
deploy. The re-pin is surgical: only three files carry a new mtime, the six consensus-logic files
are bit-unchanged from what iteration 1 validated, no other activation height moved, every quoted
copy was updated consistently, and no assertion was weakened. Build gate and 55/55 tests pass.

---

## System Entrypoint

Consensus-library milestone; no long-running system start required or attempted.

| Action | Command | Result |
|---|---|---|
| Build | `cargo build --release` | PASS (exit 0) |
| Format | `cargo fmt --check` | PASS, clean |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS, clean |
| Full suite | `cargo test --workspace --no-fail-fast` | 145 ok targets, 4 failed (see §8 audit) |
| Live testnet (READ-ONLY) | `getChainInfo` / `getBlockByHeight` on `http://127.0.0.1:8500` | iter 1 tip 130,264; iter 2 tip 130,364 |
| Baseline comparison | `git worktree add … b5f68bba --detach` | built + run independently, then removed |

No commit, push, deploy or binary copy was performed. No node was written to. No SSH. No mainnet contact.

---

## Traceability Matrix Status

| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Evidence |
|---|---|---|---|---|---|
| REQ-173-001 | Must | Yes (13 ZFP + 4 FG) | Yes | **PASS** (M1 scope) | `crates/core/src/validation/utxo.rs:239-248`; `transaction/core.rs:499-501`; `transaction/types.rs:181-210` |
| REQ-173-002 | Must | Yes (4 FG) | Yes | **PASS** | `transaction/types.rs:183`; QA probe `qa_req_173_002_*` |
| REQ-173-003 | Must | Yes (3 FG + 2 ND) | Yes | **PASS** | `utxo.rs:242-247`; QA differential dump, 840 verdicts, 0 diffs |
| REQ-173-003b | Must | Yes (3 ZFP + 3 FG) | Yes | **PASS** | `transaction/types.rs:189,191`; QA probe `qa_req_173_003b_*` |
| REQ-173-004 | Must | Yes (3 ZFP + 4 FG) | Yes | **PASS** | `transaction/core.rs:500`; QA probes `qa_c2_*`, `qa_req_173_004_*` |
| REQ-173-005 | Must | Yes (9 AH) | Yes | **PASS** (iter 2) | mainnet `u64::MAX`@275 / testnet `133_000`@480 / devnet `0`@631; lead re-measured 7.32 h — ISSUE-001 CLOSED |
| REQ-173-006 | Must | Yes (7 ND) | Yes | **PASS** | `assembly.rs:219-221` + `tx_processing.rs:98-100`; gate at `utxo.rs:239` |
| REQ-173-007 | Must | Yes (6) | Yes | **PASS** | zero version/genesis/Cargo.toml lines in `git diff b5f68bba` |
| REQ-173-008 | Must | No | — | **M2** (out of M1 scope, test plan §1) | — |
| REQ-173-009 | Should | No | — | **M3/deferred** (Option A not taken) | — |

### Gaps Found

- **REQ-173-001 second criterion is NOT met in M1, by design.** The analyst wrote "the predicate …
  is the same symbol consulted by the structural allow-lists"; `is_state_only()`
  (`transaction/core.rs:463`, 9 types) is still a separate hand-maintained list. Unification is
  F4/M3 and the M1 test plan (line 24) forbids touching it. Recorded so M1 sign-off is not mistaken
  for closing REQ-173-001 in full.
- **REQ-173-001 first criterion is met only above the gate.** `utxo.rs:244-247` still holds the
  literal 3-type `matches!` — mandatory (INV-COMPAT-001), verified token-identical. Not a defect.
- No requirement lacks tests in M1 scope; no orphan test — every `req_173_*` name in the four
  INC-I-173 targets maps to a REQ row.

---

## Acceptance Criteria Results

### REQ-173-001 (Must) — exemption derives from `is_zero_flow()` — **PASS**

- [x] No literal state-only tx-type list above the gate — `utxo.rs:240` is a bare
  `tx.is_zero_flow()` call.
- [x] Predicate lives in `crates/core` — `Transaction::is_zero_flow` (`transaction/core.rs:499`),
  delegating the type half to `TxType::allows_empty_io` (`transaction/types.rs:181`).
- [x] `allows_empty_io` is exhaustive with **no `_` arm** — 24 arms read at `types.rs:182-209`.
  A new `TxType` is a build failure, not a silent defect.
- [ ] "same symbol consulted by the structural allow-lists" — **DEFERRED to M3/F4** (see Gaps).

### REQ-173-002 (Must) — genesis `Registration` valid on both branches — **PASS**

The highest-risk regression, so I tested it independently of the developer fixture. QA probe
`qa_req_173_002_genesis_registration_validates_below_and_above_the_gate` builds the genesis
registration tx (real BLS pubkey + proof-of-possession) at mainnet heights `1, 10, 300, 359`
(inside the 360-block genesis window) and asserts: `Ok` with the gate at `u64::MAX` (legacy),
`Ok` with the gate at `0` (new branch / real devnet shape), and the two verdicts **string-equal**
— i.e. the exempt set is a strict superset. `TxType::Registration => true` (`types.rs:183`);
corroborated by the differential dump (`h=300 Registration 0in_0out -> OK` in both trees).

### REQ-173-003 (Must) — bit-identity at `height = AH-1` for all 24 types — **PASS**

Strongest evidence in this report. Verdict tables from both trees, 1512 rows each:

| Height band | Rows | Differences |
|---|---|---|
| `0, 1, 300, AH-2, AH-1` (below gate) | 840 | **0** |
| `AH, AH+1, 500_000, u64::MAX` (above gate) | 672 | 16 |

- [x] Every one of the 24 types keeps its pre-change verdict at `AH-1`, across 7 tx shapes,
  including the exact `ValidationError` variant and its payload fields.
- [x] `Exit`, `ClaimReward`, `ClaimBond`, `SlashProducer`, `AddMaintainer`, `RemoveMaintainer`,
  `ProtocolActivation`, `PriceAttestation` all still fail below the gate (verified at `h=199_999`).
- [x] **C8 confirmed, not merely claimed.** `validate_transaction_with_utxos` takes no
  `ValidationMode` parameter at all (`utxo.rs:24`), so no assertion can accidentally run in
  `Replay`. The INC-I-064 tolerance lives at the apply layer keyed on
  `mode == ValidationMode::Replay` **alone** (`apply_block/tx_processing.rs:119`, read directly);
  `Full` and `Light` both propagate the error.
- [x] The below-gate `matches!` is token-identical to `b5f68bba` — whitespace-normalised match
  against `git show b5f68bba:…/utxo.rs`, exactly one occurrence in each tree. **Re-confirmed in
  iteration 2.**

### REQ-173-003b (Must) — `Exit`/`SlashProducer` rejected at every height — **PASS**

- [x] `allows_empty_io` returns `false` for both (`types.rs:189,191`), each with a cited
  authorization reason, so a 0-in/0-out `Exit`/`SlashProducer` is `!is_zero_flow()`.
- [x] Rejected at every point of the 4 gate values × 7 heights grid
  (`qa_req_173_003b_exit_and_slash_rejected_at_every_height`); the differential dump shows
  `InsufficientFee { actual: 0, minimum: 1 }` at all 9 heights in **both** trees — unchanged.

### REQ-173-004 (Must) — mint guard (C2) — **PASS**

- [x] `ClaimReward`/`ClaimBond` with a 1,000,000 output rejected at every height on both branches.
- [x] The conjunct is inside `is_zero_flow()` (`core.rs:500`), not at the call site, so a widened
  type list cannot on its own bypass it.
- [x] **Defeat attempt, exhaustive.** 5 exempt types × 5 output shapes (non-zero, zero-amount,
  extra-data-only, a `Bond` output with `lock_until = u64::MAX`, three outputs) × 4 above-gate
  heights = 100 cases; **every** case rejected
  (`qa_c2_mint_guard_cannot_be_defeated_by_any_output_shape`).
- [x] Corroborated by the differential dump: **not one** output-bearing shape changed verdict at
  any height. `AddMaintainer` + one non-zero output dies at `InvalidMaintainerChange("… must have
  no outputs")`; `Registration` + one non-zero output dies at
  `InsufficientFunds { inputs: 0, outputs: 1 }` — proof the balance check actually ran.

### REQ-173-005 (Must) — new dedicated activation height — **PASS** (as of iteration 2)

- [x] New named field on all 3 networks. Line numbers below are the **iteration-2** positions
  (the testnet re-pin history comment grew by 13 lines): mainnet `u64::MAX` at `defaults.rs:275`,
  testnet `133_000` at `:480`, devnet `0` at `:631`. Ownership machine-derived from the
  `Network::` match arms (`Mainnet`@18, `Testnet`@294, `Devnet`@502), not from line proximity.
- [x] **No existing activation height moved anywhere.** `git diff -U0 b5f68bba` on `defaults.rs`
  filtered to height lines yields **three `+` lines and zero `-` lines**. Independently
  re-derived, not taken from the report.
- [x] Mainnet value strictly greater than the mainnet tip (`u64::MAX`, fail-closed; real pin
  deferred to M4 per spec).
- [x] Threaded to `ValidationContext` (`validation/types.rs:255`), defaulting to `u64::MAX`
  (`:297`), with `with_inc_i_173_activation_height` (`:366`).
- [x] Set at **both** call sites (`production/assembly.rs:219-221`,
  `apply_block/tx_processing.rs:98-100`) — grep-confirmed as the only two non-test callers of
  `validate_transaction_with_utxos`.
- [x] All 4 `NetworkParams` struct literal sites updated; no `..Default` anywhere in `defaults.rs`,
  so the compiler enforces completeness.
- [x] `env_loader.rs:437-448` locks mainnet, structurally matching the
  `maintainer_derivation_activation_height` precedent at `:424-436`.
- [x] **"set to a future height on … testnet" with usable lead — PASS at iteration 2.**
  `133_000` vs a self-measured live tip of `130,364` (18:29 WEST) = **2,636 blocks ≈ 7.32 h**.
  Also `133_000 > 127_200` (the INC-I-172 testnet derivation gate, `defaults.rs:450`, inside the
  same `Testnet` arm), so maintainer txs cannot become mineable before the trust root they mutate
  is derived. See "Iteration 2" below.

### REQ-173-006 (Must) — builder/apply parity (INV-PROD-003) — **PASS**

- [x] The gate is evaluated inside `validate_transaction_with_utxos` (`utxo.rs:239`) and at no call
  site; neither `assembly.rs` nor `tx_processing.rs` compares the height, each only forwards it.
- [x] Both contexts read `self.config.network.params().inc_i_173_activation_height` — no literal.
- [x] `req_173_006_builder_and_apply_contexts_agree_on_every_verdict` drives the same tx at the
  same height through both context shapes, asserting equality of `is_ok()` **and** the
  `std::mem::discriminant` of the error, so parity cannot hold by both sides failing differently.
  Non-vacuity asserted separately by `req_173_006_both_shapes_flip_at_the_gate_so_parity_is_not_vacuous`.

### REQ-173-007 (Must) — no version bumps, genesis unchanged — **PASS**

- [x] `git diff b5f68bba --stat -- '*Cargo.toml'` is empty; the diff contains zero lines touching
  `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION`,
  `genesis_hash` or `genesis_message`.
- [x] `crates/updater/src/hardfork.rs` untouched (`git status` clean) — correct per INV-8, since
  `current_fork_id(u64::MAX)` would activate any entry immediately.
- [x] Chainspec untouched; the 11 modified files are the 9 source files plus one index row each in
  `docs/DOCS.md` and `specs/SPECS.md`.

---

## Verdict on the Four Test-Harness Fixes (dev report §6)

I re-derived 6.1 and 6.2 from `git show b5f68bba:…/defaults.rs`, not from the report.

| # | Fix | My independent finding | Verdict |
|---|---|---|---|
| 6.1 | `inc_i_147` mainnet/testnet transposed | Baseline mainnet block starts `:18`, testnet `:283`. `inc_i_147_activation_height: 129_500` is at `:251` (**inside** the mainnet block, same block as `maintainer_derivation: 172_000` at `:264`); `80_700` is at `:430` (**inside** the testnet block). The test's original literals were reversed. Making the code satisfy the test as written would have required moving a mainnet activation height — the INC-I-054 failure mode this very test exists to forbid. Both values remain pinned, one per network. | **LEGITIMATE — strength unchanged** |
| 6.2 | mainnet `defi_activation_height` | Baseline literal is `0` at `defaults.rs:164`, unchanged by this diff. The test asserted `u64::MAX`, which is what CLAUDE.md claims. Code is the source of truth; the test now pins the true baseline and still fires if INC-I-173 perturbs the neighbour. | **LEGITIMATE — strength unchanged**; the underlying doc drift is recorded below |
| 6.3 | `HardForkSchedule` counter | On the **unmodified** file the raw count is genuinely 3: `:158` is `/// schedule.add(HardForkInfo {`, a rustdoc example; the two real entries are `:231` and `:238`, both before the first `#[cfg(test)]` at `:253`. The filter `!l.trim_start().starts_with("//")` excludes `///` lines and cannot exclude a real entry, because live code never begins with `//`. | **LEGITIMATE — strictly stronger** |
| 6.4 | `#[allow(clippy::assertions_on_constants)]` | Scoped to one test. The two `const` bindings and both `assert!` calls are unchanged and still execute; the lint fires on the property under test (const-evaluability). The alternative clippy suggests (`const { assert!(…) }`) would move the failure to build time and lose the test-level diagnostic. | **LEGITIMATE — strength unchanged** |

**No harness fix weakened an assertion. No QA blocker from §6.**

---

## Independent Audit of dev report §8 (claimed pre-existing failures)

| Claim | My reproduction | Verdict |
|---|---|---|
| `mempool contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` is pre-existing | Built and ran a detached worktree at `b5f68bba` with **no INC-I-173 code present**. Identical failure, identical panic site `crates/mempool/src/contention_tests.rs:1108`, identical message. | **CONFIRMED pre-existing.** Recorded, not fixed. Recommend a separate incident. |
| `test_network::test_cluster_10x100` is environmental | Ran in isolation: `test result: ok. 1 passed` on **both** targets that link the module. Fails only under full-workspace parallelism (RocksDB fd exhaustion). | **CONFIRMED environmental.** |

Full workspace run: **145 `ok` targets, 4 `FAILED`** — the mempool test (1), `test_cluster_10x100`
reported by two targets (2), and one failure from my own temporary QA scaffolding (env-var gated,
since deleted). Matches §8's "2 distinct failures" exactly.

---

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Legacy validation below the gate | 840 verdicts, 2 independently built trees | **PASS** | 0 differences |
| Gate flip at the activation height | `AH-1` → `AH` → `AH+1` | **PASS** | flip occurs at exactly `AH`; `>=` confirmed |
| `AddMaintainer` becomes mineable above the gate | 0-in/0-out + well-formed `MaintainerChangeData` | **PASS** | `InsufficientFee` → `Ok`; this is the INC-I-173 fix, observed |
| Builder ↔ apply agreement | same tx, same height, both context shapes | **PASS** | verdict and error discriminant both equal |
| End-to-end mine + apply on live testnet (REQ-173-008) | — | **NOT RUN** | M2 scope; ISSUE-001 no longer blocks it |

---

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Differential dump with **empty** `extra_data` (my first instrument) | types reach the fee gate | all interesting types died at structural validation; both trees agreed **vacuously** | n/a — my own instrument defect, fixed with a positive control before any conclusion |
| 2 | Gate at exactly `AH` | above the gate (`>=`) | above the gate — verdict flips at `AH`, not `AH+1` | none |
| 3 | Gate `u64::MAX`, height `u64::MAX - 1` | stays legacy forever | stays legacy (`InsufficientFee`) | none |
| 4 | Gate `0`, height `0` | above the gate (devnet shape) | above the gate | none |
| 5 | `ValidationContext` built **without** the `with_*` builder, height 900,000 | fail-closed to legacy | legacy reject; field is `u64::MAX` | none |
| 6 | `AddMaintainer` + one non-zero output, above the gate | rejected | `InvalidMaintainerChange("… must have no outputs")` | none |
| 7 | `AddMaintainer` + one **zero-amount** output, above the gate | rejected | `InvalidTransaction("[ERRTX003] output 0 has zero amount")` | none |
| 8 | `AddMaintainer` + a `Bond` output, `lock_until = u64::MAX`, above the gate | rejected | rejected | none |
| 9 | `Registration` + one non-zero output, in the genesis window | balance check runs | `InsufficientFunds { inputs: 0, outputs: 1 }` | none |
| 10 | Output with `extra_data` only (amount 0) on every exempt type | rejected | rejected | none |
| 11 | `u64::MAX`-amount output on exempt types | rejected | `AmountExceedsSupply` | none |
| 12 | Junk bytes appended to a valid `MaintainerChangeData` payload | accepted above the gate (payload still parses) | accepted — **same as the clean payload**, so no new surface | low — noted for F5/M3, which bounds `extra_data` |
| 13 | Live testnet tip vs pinned `130_400` (iter 1) | comfortable lead | **136 blocks, ≈23 min** | **high — ISSUE-001, now CLOSED** |
| 14 | Live testnet tip vs re-pinned `133_000` (iter 2) | comfortable lead | **2,636 blocks, ≈7.32 h** | none |

Finding 12 is not introduced here — trailing-byte tolerance in `MaintainerChangeData` parsing
exists identically at `b5f68bba`; F5/REQ-173-014 (M3) owns bounding it. Recorded, not blocking.

---

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Apply site left unwired (F2 both-or-neither) | Yes — default-constructed context | Yes | n/a | Yes | falls back to legacy reject, a liveness bug not a fork; `qa_default_constructed_context_is_fail_closed_to_legacy_behaviour` |
| Builder site left unwired | Yes (same mechanism, inverse) | Yes | n/a | Yes | builder omits the tx; block still valid to everyone |
| Mixed fleet straddling the gate | **Untestable in this environment** | — | — | — | Needs a two-binary testnet fleet. The 7.32 h lead (iter 2) is what keeps this scenario off the table; exercise it in the M2 deploy rehearsal |
| New `TxType` added without classification | Yes — by inspection: `allows_empty_io` has no `_` arm | Yes, at **compile time** | n/a | Yes | best available failure mode |
| Gate value corrupted via env on mainnet | Yes | Yes | n/a | Yes | `env_loader.rs:437` returns the default when `is_mainnet`; `DOLI_INC_I_173_ACTIVATION_HEIGHT` is ignored |
| Genesis / fresh sync regression | Yes | Yes | n/a | Yes | REQ-173-002 probe passes on both branches |

---

## Security Validation

| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Coin creation via the exemption (C2 mint guard) | 100 cases: 5 exempt types × 5 output shapes × 4 above-gate heights, each asserting rejection | **PASS** | not one case escaped the balance check |
| Exemption via type alone | `is_zero_flow()` on every exempt type carrying an output | **PASS** | both conjuncts required; `qa_c2_is_zero_flow_requires_both_conjuncts` |
| Unauthenticated `Exit` becoming free | 0-in/0-out `ExitData`, 4 gate values × 7 heights | **PASS** | rejected everywhere; `allows_empty_io = false` (C1) |
| Forged `SlashProducer` becoming free | `SlashData` with `reporter_signature: Signature::default()`, 4 gate values × 7 heights | **PASS** | rejected everywhere. Note the payload proves the point: an all-zero signature is as acceptable to the validator as a real one, because `reporter_signature` has no verification reader — the reason F3 excludes it |
| Operator moving a mainnet consensus gate via `.env` | Read `env_loader.rs:437-448` | **PASS** | mainnet locked, matching the existing precedent |
| Oversized `extra_data` on maintainer txs | Appended junk to a valid payload | **Out of Scope** | pre-existing at `b5f68bba`, identical in both trees; owned by F5/REQ-173-014 in M3 |
| Fee bypass by a non-exempt type below the gate | 840-verdict differential | **PASS** | no verdict softened anywhere below the gate |

---

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `CLAUDE.md` ("If You Touch" → activation heights) | "Oracle + DeFi gates are `u64::MAX` (frozen pre-activation)" | mainnet `defi_activation_height = 0` (`crates/core/src/network_params/defaults.rs:164`, unchanged since before `b5f68bba`). Mainnet `oracle_activation_height` **is** `u64::MAX`, so only the DeFi half is wrong. | medium — pre-existing, already logged as Follow-up 4 in `specs/state-only-fee-gate-architecture.md:602`; it misled the test author (harness fix 6.2) |
| `specs/state-only-fee-gate-architecture.md` | Status "M1 IMPLEMENTED (F1+F2+F3)"; pinned devnet `0` / testnet `133_000` / mainnet `u64::MAX` (`:30`, `:554-562`, carrying the full two-entry re-pin history) | matches the code exactly | none |
| `specs/state-only-fee-gate-architecture.md:549,564` | cites the gate site as `utxo.rs:245` / `utxo.rs:222` | the gate now sits at `utxo.rs:239` | low — stale line cites in the *design* prose, written pre-implementation; pre-existing since iteration 1, not caused by the re-pin |
| `docs/redesigns/state-only-fee-gate-redesign-analysis.md` | traceability rows 506-515 | match the implemented symbols and file paths | none |
| `docs/DOCS.md`, `specs/SPECS.md` | one index row each | present and accurate | none |

No drift was introduced by this milestone.

---

## Blocking Issues

**None.** ISSUE-001 (iteration 1) is CLOSED — see "Iteration 2" below.

<details>
<summary>ISSUE-001, as raised in iteration 1 (resolved)</summary>

`crates/core/src/network_params/defaults.rs` pinned testnet `inc_i_173_activation_height: 130_400`,
justified in-code as 781 blocks ≈ 2.17 h measured at tip 129,619. By review time (2026-08-10
18:12 WEST) the live tip was **130,264** — **136 blocks ≈ 23 minutes** of lead — with nothing
committed, built into `~/testnet/bin/`, or deployed. The chain would have crossed the height with
a fleet that has no notion of the gate; every later rolling upgrade would then produce a mixed
fleet **already past** the activation height, where a new-binary producer builds a block carrying
a 0-in/0-out `AddMaintainer` that old-binary nodes reject — the exact fork the height exists to
prevent, and M2 (REQ-173-008) submits precisely those transactions. INC-I-054 additionally makes a
crossed height immutable, so the wrong value could never be corrected. Not a code defect.

</details>

---

## Non-Blocking Observations

- **[OBS-001]** `crates/core/src/transaction/types.rs` sits at **exactly 500 lines** — the module
  ceiling, zero headroom. The next edit must split it, and as a workspace-wide import that split is
  its own change.
- **[OBS-002]** `crates/mempool/src/contention_tests.rs:1108` fails at `b5f68bba`; needs its own
  incident so the suite can return to green — a permanently red test erodes the signal that
  protects consensus changes.
- **[OBS-003]** `test_network::test_cluster_10x100` fails only under full-workspace parallelism
  (RocksDB `Too many open files`), passes in isolation. `#[ignore]` + a serial target, or raise the
  harness fd limit.
- **[OBS-004]** `CLAUDE.md`'s "Oracle + DeFi gates are `u64::MAX`" is wrong for the DeFi half and
  actively misled the test author. Correct it at the source (see Specs/Docs Drift).
- **[OBS-005]** REQ-173-001's second criterion (one predicate shared with the structural
  allow-lists) is still open: `is_state_only()` (`transaction/core.rs:463`) keeps an independent
  9-type list. M1 correctly left it byte-identical; do not mark REQ-173-001 closed until F4/M3.
- **[OBS-006]** `MaintainerChangeData` parsing tolerates trailing junk in `extra_data`.
  Pre-existing, identical in both trees; F5/REQ-173-014 (M3) owns bounding it.
- **[OBS-007]** The testnet pin is a decaying value with no automated guard. A test asserting
  `pin > current_tip` is impossible offline, but M2's deploy checklist should require a fresh tip
  measurement immediately before commit. This defect recurred once already.

---

## M1/M3 Boundary Check — held

| Guard | Result |
|---|---|
| `is_state_only()` still exists, unmodified | **PASS** — function body extracted from both trees and compared: byte-identical, 9-type list intact |
| `validation/transaction.rs` L1/L2 (lines 39-88) character-identical | **PASS** — `diff` of the exact line range against `b5f68bba` is empty |
| `crates/updater/src/hardfork.rs` untouched | **PASS** — `git status` clean for that path |
| No F4-F7 work leaked in | **PASS** — the diff is 9 source files, all mapped to F1/F2/F3 |
| No new RPC method, no `extra_data` bound, no maintainer digest | **PASS** — absent from the diff |

---

## Modules Not Validated

- **REQ-173-008 end-to-end on the live testnet (M2 scope).** Not attempted — it requires deploying
  a new binary to the fleet, outside QA's authority here.
- **Mixed-fleet fork behaviour across the gate.** Untestable without a two-binary fleet; exercise
  it during the M2 deploy rehearsal.
- **Mainnet.** No contact made, by instruction. Mainnet stays `u64::MAX` — the feature is inert
  there until M4 pins a value against the live mainnet tip.

---

## Iteration 2 — re-validation (2026-08-10 18:27–18:29 WEST)

Scope as I set it in iteration 1: REQ-173-005's height literal, the documents that quote it, and
an independent check that the fix was genuinely surgical. Nothing is committed; still diffed
against `b5f68bba`.

### 1. REQ-173-005 re-verified — **PASS**

I measured the chain myself rather than accepting the developer's numbers, because this parameter
decays continuously and their measurement was already stale by construction.

| Quantity | My measurement | Method |
|---|---|---|
| Live testnet tip @ 18:27:34 | **130,354** | `getChainInfo` on `http://127.0.0.1:8500` |
| Live testnet tip @ 18:29:10 | **130,364** | same, re-issued at write time |
| Block rate | **exactly 10.00 s/block** | `getBlockByHeight` at h=129,354 (ts `1786372849`) and h=130,354 (ts `1786382849`) → 10,000 s / 1,000 blocks |
| Remaining lead vs `133_000` | **2,636 blocks ≈ 7.32 hours** | 133,000 − 130,364, at 10.00 s/block |
| Old pin `130_400` at this tip | **36 blocks ≈ 6 minutes** | confirms the iteration-1 blocker was real and near-terminal |

My independent rate sample lands on the developer's 10.00 s/block to the second, and my tip is
73 blocks past theirs — consistent with the ~12 min elapsed. 7.32 h comfortably covers the
remainder of M1 (review + security audit + commit) plus the M2 build, `codesign`, and 13-node
fleet restart. All RPC was READ-ONLY, `127.0.0.1` only; no SSH, no mainnet contact.

Per-network values, ownership machine-derived from the `Network::` match arms (`Mainnet`@18,
`Testnet`@294, `Devnet`@502) rather than line proximity:

| Network | Line | Value | Required | OK |
|---|---|---|---|---|
| Mainnet | 275 | `u64::MAX` | fail-closed until M4 | **YES** |
| Testnet | 480 | `133_000` | future, usable lead, `> 127_200` | **YES** |
| Devnet | 631 | `0` | always active | **YES** |

Ordering: `133_000 > 127_200`, the INC-I-172 testnet `maintainer_derivation_activation_height`
(`defaults.rs:450`, inside the same `Testnet` arm — a like-for-like comparison). **Holds.**

### 2. No other activation height moved — **PASS**

`git diff b5f68bba -- crates/core/src/network_params/defaults.rs` contains **zero deletion lines**
(full diff inspected, not just the `activation_height` grep); filtered to `activation_height:`
lines it yields exactly three `+` and zero `-`: `u64::MAX`, `133_000`, `0`. Nothing was committed
between iterations, so the re-pin is invisible here — the file is additions-only against the
baseline, the strongest possible form of "no existing height moved".

### 3. The fix was surgical — **PASS**

Two independent instruments, both agreeing. The re-pin ran at 18:17; only three files carry an
18:17 mtime — `network_params/defaults.rs` (18:17:13), `network_params/mod.rs` (18:17:15) and
`crates/core/tests/inc_i_173_activation_height.rs` (18:17:26). The six consensus-logic files
retain their iteration-1 mtimes of **16:56:33 – 16:58:10**, and each still produces the same diff
shape I validated then:

| File (diff vs `b5f68bba`) | mtime | diff sha256 (16) | +/− |
|---|---|---|---|
| `crates/core/src/validation/utxo.rs` | 16:57:57 | `4ada4f4f2062aab4` | +28/−7 |
| `crates/core/src/transaction/types.rs` | 16:56:33 | `21010a4ab51b0df1` | +35/−1 |
| `crates/core/src/transaction/core.rs` | 16:56:45 | `d8b2229f9f2af5d3` | +27/−1 |
| `crates/core/src/validation/types.rs` | 16:57:42 | `a47848a9423485b8` | +27/−1 |
| `bins/node/src/node/production/assembly.rs` | 16:58:03 | `a59df3ab8e0ecab4` | +8/−1 |
| `bins/node/src/node/apply_block/tx_processing.rs` | 16:58:10 | `530a7b85566a4a3c` | +8/−1 |

A grep of those six diffs for `133_000` / `130_400` / `127_200` returns **nothing** — the logic
files never learned the value, which is precisely what makes the re-pin incapable of altering
behaviour. I also re-ran the iteration-1 machine check on the gate itself: the legacy
`inputs.is_empty() && outputs.is_empty() && matches!(…Registration | DelegateBond | RevokeDelegation)`
expression occurs **exactly once** in both the baseline and the working tree and is
whitespace-normalised **token-identical** (INV-COMPAT-001 intact). **No logic delta. No new blocker.**

### 4. Every quoted copy updated consistently — **PASS**

Tree-wide sweep for `130_400` / `130400` (excluding `target/`, `.git/`) returns 11 hits, every one
accounted for and none of them a live value: `defaults.rs:461,467` (2 — the required re-pin-history
block, `u64::MAX → 130_400` then `130_400 → 133_000`), `specs/state-only-fee-gate-architecture.md:554-555`
(2 — the same history mirrored into the spec), and this report itself (7 — its own iteration-1
record). **Zero live `130_400` values remain.** The corresponding `133_000` sweep confirms every consumer
was updated: `defaults.rs:480` (the value), `network_params/mod.rs:631` (field doc, carrying tip
130,291 / 2,709 blocks / 7.53 h), `specs/…:30` (header) and `:554-562` (full two-entry history),
and the test at `crates/core/tests/inc_i_173_activation_height.rs:41,64,85,101,127`.
`bins/node/tests/inc_i_173_state_only_fee_gate.rs` correctly needed no change — it gates on a
synthetic `TEST_AH = 200_000` (`:103`), never on the real pin. I confirmed that by grep.

### 5. No test assertion weakened — **PASS**

`req_173_005_testnet_gate_is_pinned_near_future_and_is_not_a_no_op` still carries all four of its
checks, and the pin is still **exact**, not a range or an inequality:

- `assert_eq!(h, TESTNET_GATE)` with `const TESTNET_GATE: u64 = 133_000` (`:101`, `:127`)
- `assert_ne!(h, 0)` — retroactive-reinterpretation guard
- `assert_ne!(h, u64::MAX)` — "the fix must be reachable" guard
- `assert!(h > p.maintainer_derivation_activation_height)` — the INC-I-172 ordering guard, compared
  against the field rather than a literal, so it cannot silently pass if either moves

Only a constant and the strings quoting it changed. Nothing was deleted, `#[ignore]`d, loosened to
an inequality, or turned into a range.

### 6. Build gate re-run in full — **PASS**

| Step | Result |
|---|---|
| `cargo build --release` · `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` | PASS, all clean |
| `cargo test -p doli-core --lib` | **976 passed, 0 failed** |
| `--test inc_i_173_zero_flow_predicate` / `_fee_gate` / `_activation_height` | **13 / 18 / 12 passed, 0 failed** |
| `-p doli-node --test inc_i_173_state_only_fee_gate` | **12 passed, 0 failed** |

55/55 INC-I-173 tests green. The two pre-existing failures from iteration 1
(`mempool contention_tests::…inc_i_096_below_gate_rejects_remove_liquidity`, confirmed at
`b5f68bba`; and `test_network::test_cluster_10x100`, environmental) are **recorded, not fixed**,
per instruction — see OBS-002 and OBS-003.

### Iteration 2 residual risk

The pin decays. At 10.00 s/block the `133_000` gate is crossed **≈01:47 WEST on 2026-08-11**. If
commit + M2 testnet deploy has not completed by then, ISSUE-001 recurs identically and the height
must be re-pinned again — a property of the parameter, not a defect in the change.

---

## Final Verdict

All eight in-scope Must requirements pass, several on evidence stronger than the milestone
required: below-the-gate identity is demonstrated behaviourally — 840 verdicts against an
independently built `b5f68bba` worktree, zero differences — not asserted from a textual diff, and
the mint guard survived 100 constructed defeat attempts. The one iteration-1 blocker was a pinned
literal, not source logic; the re-pin to `133_000` was verified surgical by two independent
instruments, restores ≈7.32 h of lead against a tip and block rate I measured myself, preserves the
INC-I-172 ordering constraint, and weakened no assertion. Build gate and all 55 tests green.

M1 sign-off is **not** full closure of REQ-173-001 — its second criterion (one predicate shared
with the structural allow-lists) is open by design and belongs to F4/M3 (OBS-005). REQ-173-008 is
M2 scope and was not run. The pin decays: **re-measure the tip immediately before commit.**

**QA VERDICT: PASS**
