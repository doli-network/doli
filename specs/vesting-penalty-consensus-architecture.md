━━━ FINDINGS — 9 total (DECISION:9) ━━━

  [F1] DECISION conf(0.85, converged) — bins/node/src/node/validation_checks.rs:639-688 — bound the RequestWithdrawal payout at consensus inside the existing INC-I-180 input pass, over the spent inputs, under its own activation height
  [F2] DECISION conf(0.90, converged) — crates/storage/src/producer/info.rs:21-45,443-476 — delete calculate_withdrawal_from_bonds + ProducerInfo::calculate_withdrawal{,_with_quarter}
  [F3] DECISION conf(0.85, converged) — crates/mempool/src/withdrawal_holdings.rs:120-135 — collapse the three bond_input_split copies into one storage resolver
  [F4] DECISION conf(0.85, converged) — crates/core/src/validation/tx_types.rs:613-617 — delete the false "done at node level" comment, add a tripwire test
  [F5] DECISION conf(0.78, converged) — crates/core/src/validation/vesting.rs (new) — pure penalty predicate in doli-core; UTXO resolver in storage
  [F6] DECISION conf(0.72, converged) — crates/mempool/src/pool.rs:424-428 — three-site parity: thread current_slot into mempool admission
  [F7] DECISION conf(0.65, inferred) — crates/core/src/network_params/mod.rs — paired inc_i_171_vesting_penalty_disable_height, frozen u64::MAX
  [F8] DECISION conf(0.70, converged) — crates/wallet/src/tx_builder/fees.rs:82-100 — delete the wallet's mainnet-hardcoded penalty ladder
  [F9] DECISION conf(0.65, converged) — crates/core/src/consensus/exit.rs — delete calculate_exit{,_with_quarter}/ExitTerms/PenaltyDestination (keep calculate_slash)

  Speculative: 6 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Vesting Penalty Consensus Enforcement — Architecture (INC-I-171)

Design-synthesizer output, `/omega-redesign` proposal-only, 2026-09-05. Inputs: `docs/.workflow/design-*.md` (5 evaluators), `docs/redesigns/vesting-penalty-consensus-redesign-analysis.md`. Trace with parity math, unit arithmetic and the spot-check log: `docs/.workflow/architecture-reasoning.md`.

**Status:** APPROVED at the user gate on 2026-09-06 as the locked design for INC-I-171 (proposal-only run; implementation pending a `--fix` run). Open decisions 1-5 keep their stated defaults. Activation-height pinning is a separate decision session gated by the Preconditions section.

## Summary

**SSF:** reject any block in which a `RequestWithdrawal` output exceeds `sum penalized_bond_net(spent Bond inputs) + sum(non-Bond inputs)`, computed from the node-stamped Bond `extra_data` slot in the pre-block UTXO view, inside the INC-I-180 input pass the node already runs, behind `inc_i_171_vesting_penalty_activation_height` (frozen `u64::MAX` everywhere).

The Radical minimum (same arithmetic in core `validate_transaction_with_utxos`) scores conf(0.45) after filtering vs conf(0.80) here — no SSF override. Measured: the core site runs after `producer_set.write()` at apply (`apply_block/mod.rs:198` then `:202`), is Replay-muted (`tx_processing.rs:119-123`), and the mempool never calls it (`pool.rs:487`). Its claim that the node site re-resolves under a second guard is false: the INC-I-180 pass resolves every input under one guard (`validation_checks.rs:640-684`).

**Age-source verdict:** the Failure Analyst's F1 ("every Bond has `creation_slot = 0`; nothing stamps at apply") is refuted by source. All three UTXO write paths stamp Bond `extra_data` with the block slot — `state_db/batch.rs:149-150`, `utxo/in_memory.rs:96-97`, `utxo/set.rs:182` — since `252947a6` (2026-03-11), tested in `crates/storage/tests/twap_equivalence_test.rs:44-45`. The live `creationSlot: 0` is a genesis bond (all 6 bonded testnet producers have `registrationHeight: 0`); mainnet genesis 2026-07-22 postdates the fix; constraint-table proof #2 (n19, 100 Q1 bonds at tip ~925k slots) proves a post-genesis bond carries a real slot. The tier stays **slot-based**, referenced to `block.header.slot`.

## Problem Statement

`WHITEPAPER.md:604-615` defines a tiered FIFO vesting penalty by bond age (`<1 yr 75% / 1-2 yr 50% / 2-3 yr 25% / 3+ yr 0%`, burned). The only code that decides a payout is the CLI (`bins/cli/src/cmd_producer/withdrawal.rs:198`); the node checks shape and counts only (`tx_types.rs:560-618`; INC-I-180 gate `validation_checks.rs:625`); `tx_types.rs:614` claims a node-level bound that does not exist. A patched client withdraws Q1 bonds at 100% (proofs #1/#2). The payout is bounded only by conservation (`validation/utxo.rs:271`).

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(k) integer ops per RequestWithdrawal input (1 u128 mul, 1 div, 2 adds) in the loop already running at validation_checks.rs:670; 0 for blocks without a withdrawal (observed shape, inferred count)
  Memory:   +16 B x k transient per withdrawal tx (k <= MAX_BONDS_PER_PRODUCER) + 16 B in NetworkParams (measured)
  IO:       0 — reuses the utxo.get at validation_checks.rs:671 (measured)
  Network:  0 — validation-only; slightly negative once mempool stops gossiping invalid withdrawals (observed)
  Disk:     0 — nothing persisted, state root unchanged (observed)
  Latency:  +<1 us per withdrawal-bearing block; guard hold time unchanged (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS — a payout bound needs the spent inputs once; this reads them zero extra times. Mempool/builder-only variants enforce nothing (that is INC-I-171).
Why this proposal anyway: the rule must bind where blocks are accepted; the cheapest such site already holds every operand.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

Subtractionist conf(0.66, measured) delete 6 dead impls, bound rides in the INC-I-180 scan; Restructurer conf(0.68, measured) pure fn in core, enforce in node; Pattern Matcher conf(0.70, measured) INC-I-203 shape inside the INC-I-180 pass; Failure Analyst conf(0.66, measured) block age-from-`extra_data`, paired disable AH (F1 refuted below, other filters stand); Radical Simplifier conf(0.65, observed) one pure fn + `<=` in the core UTXO loop.

## Convergence Matrix

Formed from `conclusion-*.md` before any full report; verified against full reports and source; independence of evidence checked per row (trace section 1).

| Finding | Sub | Res | Pat | Fail | Rad | N | Conf |
|---|---|---|---|---|---|---|---|
| Bound over SPENT inputs, never `get_bond_entries` first-N | Y | Y | Y | Y | Y | 5/5 | 0.95 |
| Check site = node `validate_block_economics` INC-I-180 pass | Y | Y | Y | Y | N | 4/5 | 0.85 |
| Own AH, never nested in `withdrawal_holdings_gate` | Y | Y | Y | Y | Y | 5/5 | 0.90 |
| Delete info.rs calculators | Y | Y | Y | Y | Y | 5/5 | 0.90 |
| Collapse `bond_input_split` x3 | Y | Y | Y | Y | - | 4/5 | 0.85 |
| Delete lying comment + tripwire | Y | - | Y | Y | - | 3/5 | 0.85 |
| Pure fn home = `doli-core` | N | Y | Y | - | Y | 3/5 | 0.78 |
| `<=`; per-bond penalty-first truncation | Y | Y | Y | Y | Y | 5/5 | 0.90 |
| u128 saturating, zero guard, AH as parameter | - | Y | Y | Y | Y | 4/5 | 0.85 |
| Three-site parity | Y | Y | Y | Y | Should | 4/5 | 0.72 |
| Delete wallet ladder | Y | - | - | - | Y | 2/5 | 0.70 |
| Delete `exit.rs` penalty fns | Y | - | - | - | Y | 2/5 | 0.65 |
| Paired DISABLE height | - | - | - | Y | - | 1/5 | 0.65 |
| Age source dead -> use height | - | N | - | Y | - | contradiction | REJECTED |
| Delete `bond_creation_slot()` + payload | - | - | - | Y | - | 1/5 | REJECTED |
| `bonds.rs`, `simulateWithdrawal`, bare `withdrawal_penalty_rate`, `unbonding_period()` accessor | Y | - | - | - | - | 1/5 | options |

Vetoes honored: `ClaimWithdrawal = 9` tombstone (`transaction/types.rs:26`); `consensus::UNBONDING_PERIOD` (live at `tx_processing.rs:583`).

## Definite Changes (High Convergence)

- ARCHITECTURAL: Enforce the payout bound at consensus inside the INC-I-180 input pass, over the transaction's own spent inputs, under its own activation height.
    Convergence: Subtractionist P2, Restructurer, Pattern Matcher P2, Failure Analyst P1; Radical on rule shape.
    Evidence: `validation_checks.rs:639-688` resolves every input under one guard and discards amount/extra_data (`:671-683`); `apply_block/mod.rs:113` runs before `:198`; R4 (`:812-826`).
    Confidence: conf(0.85, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +O(k) integer ops per RequestWithdrawal input in the existing loop (observed)
      Memory:   +16 B x k transient per withdrawal tx (measured)
      IO:       0 — reuses validation_checks.rs:671 lookup (measured)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  +<1 us per withdrawal-bearing block (inferred)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: the only pre-mutation, all-mode hook that already holds every operand.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete `calculate_withdrawal_from_bonds` (`producer/info.rs:21-45`), `ProducerInfo::calculate_withdrawal{,_with_quarter}` (`:443-476`), re-export `producer/mod.rs:54`. Lands FIRST (M1).
    Convergence: all five, independent evidence.
    Evidence: zero non-test callers (`blast.py`: 1 dependent = `ProducerInfo`); `:444` hardcodes the mainnet quarter; signature carries a storage type and iterates the ledger.
    Confidence: conf(0.90, converged)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: `#[allow(dead_code)]` or wiring-debt rows
    Why this proposal anyway: six debt rows cost more than six deletions and keep the wrong rule reachable.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Collapse the three `bond_input_split` loops (`validation_checks.rs:669-684`, `mempool/withdrawal_holdings.rs:120-135`, `production/withdrawal_holdings.rs:245-260`) into one storage resolver returning counts, spent-bond `(amount, creation_slot)` list and non-Bond total.
    Convergence: Subtractionist P3, Restructurer BR-3, Pattern Matcher AP-4, Failure Analyst.
    Evidence: all three call `utxo.get` per input and discard the values; hazard named at `mempool/src/addbond_cap.rs:4-5`.
    Confidence: conf(0.85, converged)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 net (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: add the bound to all three copies
    Why this proposal anyway: three copies of a consensus predicate is the failure INC-I-203 paid for.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete the false comment at `tx_types.rs:613-617`; add a `req_vest_*` tripwire test naming the enforcing function.
    Convergence: Subtractionist, Pattern Matcher P3, Failure Analyst.
    Evidence: the same idiom at `:477-481` was false for years; `:486-492` confesses INC-I-080.
    Confidence: conf(0.85, converged)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: delete the comment, no test
    Why this proposal anyway: deletion alone lets the next reader re-add the claim.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: Two-layer home — pure math in `crates/core/src/validation/vesting.rs`; UTXO resolution in `crates/storage/src/producer/withdrawal_inputs.rs`.
    Convergence: Restructurer, Pattern Matcher, Radical (math in core); Subtractionist (resolver where inputs live). Different functions, both satisfied.
    Evidence: Cargo graph — `core -> {crypto, vdf}`; `storage -> core`; `mempool -> core, storage`; `rpc`/`cli`/node -> both. Core reaches every consumer with zero new edges; the resolver needs `&UtxoSet`. Precedent `verify_amm_conservation` (`utxo.rs:270`, `pool.rs:650`).
    Confidence: conf(0.78, converged)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 vs inline (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: inline at the node site (INC-I-180 shape)
    Why this proposal anyway: the inline shape produced three divergent copies (AP-4) at the same runtime cost.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Three-site parity — mempool and builder call the same resolver + predicate; mempool gains `current_slot: u32` (`pool.rs:424-428`); builder passes the `current_slot` in scope (`assembly.rs:15`).
    Convergence: Restructurer BR-4, Pattern Matcher, Failure Analyst F11/F13, Subtractionist P3.
    Evidence (verified 2026-09-05, `docs/.workflow/design-verification.md`): the tier is monotone non-decreasing in age (`constants.rs:430-437`) and the builder's `current_slot` is the slot written to `block.header.slot` (`assembly.rs:377`, `block.rs:385`), so the BUILDER is the exact safety gate. The mempool today has NO slot: `add_transaction(&mut self, tx, utxo_set, current_height)` is height-only (`pool.rs:424-428`, `withdrawal_holdings.rs:26-32`), `ValidationContext` gets `current_time = 0` (`pool.rs:460`). `chain_state.best_slot` is NOT a safe admission slot: a reorg sets `state.best_slot = common_ancestor_slot` (`block_handling.rs:803, 875`) and `revalidate` EVICTS on any `Err` (`pool.rs:1345-1352`), so an honest withdrawal built for a higher slot would be dropped after a reorg. Design amendment (R1): the mempool evaluates the bound with a non-regressing slot watermark `max(best_slot seen so far, wall-clock slot)`, and the vesting error is excluded from reorg eviction (re-check only, never evict). Admission is liveness-only: an over-admitted tx is skipped by the builder, never included.
    Confidence: conf(0.80, measured) — every premise read at the cited lines; the watermark plumbing is new code and untested until M5.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +O(k) per admitted RequestWithdrawal at mempool and builder (observed)
      Memory:   +4 B per add_transaction frame (measured)
      IO:       0 (observed)
      Network:  slightly negative — invalid withdrawals no longer gossiped (inferred)
      Disk:     0 (observed)
      Latency:  +~1 us per admitted withdrawal (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: builder + block gate only
    Why this proposal anyway: an unmirrored mempool admits, gossips and packs a tx that costs an honest producer its block (INC-I-203).
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Paired `inc_i_171_vesting_penalty_disable_height: u64`, frozen `u64::MAX`; gate = `enable <= height && height < disable`, inside the pure predicate.
    Convergence: Failure Analyst P2; adopted (Open decision 2).
    Evidence: INV-PARAMS-001 / INC-I-054 make a crossed AH immutable; a binary revert partitions ~30 auto-update producers. No `NetworkParams` disable/sunset height exists (grep 0).
    Confidence: conf(0.65, inferred)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      +1 u64 comparison per RequestWithdrawal post-AH (measured)
      Memory:   +8 B in NetworkParams (measured)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: rely on a binary revert
    Why this proposal anyway: the only in-protocol undo for a rule that is permanent once mainnet crosses it.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete the wallet ladder — `vesting_penalty_pct`, `calculate_withdrawal_net` (`wallet/tx_builder/fees.rs:82-100`), `wallet::VESTING_QUARTER_SLOTS`, `wallet::UNBONDING_PERIOD` (`wallet/types.rs:309,318`), re-exports `wallet/lib.rs:28`.
    Convergence: Subtractionist, Radical.
    Evidence: hardcodes the mainnet ladder in a public API (REQ-VEST-006); `bins/gui` never calls it.
    Confidence: conf(0.70, converged) — out-of-tree consumers unverified.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep, mark `#[deprecated]`
    Why this proposal anyway: a public API computing the wrong tier on testnet/devnet is a live REQ-VEST-006 violation.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete `calculate_exit{,_with_quarter}`, `ExitTerms`, `PenaltyDestination` (`consensus/exit.rs`, ~147 L); keep `calculate_slash`.
    Convergence: Subtractionist, Radical.
    Evidence: zero non-test callers; ages from `registered_at` (height) — the wrong rule.
    Confidence: conf(0.65, converged)

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave in place
    Why this proposal anyway: a wrong, dead implementation of the same rule is the AP-7 mechanism.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Design (presented alone)

**Pure layer — `crates/core/src/validation/vesting.rs`** (~50 L + tests; no storage type in any signature).

```rust
/// One bond. Byte-identical to CLI common.rs:35-36: penalty = amount*pct/100 (u128), net = amount - penalty.
pub fn penalized_bond_net(amount: u64, creation_slot: u32, block_slot: u32, quarter_slots: u32) -> u64;

/// Gate INSIDE (INC-I-203 template, tx_types.rs:523): Ok(()) when height < activation || height >= disable.
/// Err(VestingQuarterInvalid) when quarter_slots == 0 or > u32::MAX — never a panic (constants.rs:431 is unguarded).
/// Err(WithdrawalPayoutExceedsVestedNet{payout, bound, bond_inputs, non_bond_value}) when payout > bound.
pub fn check_withdrawal_payout_bound(
    payout: u64, spent_bonds: &[(u64, u32)], non_bond_input_total: u64,
    block_slot: u32, quarter_slots: u64, height: u64, activation_height: u64, disable_height: u64,
) -> Result<(), ValidationError>;
```

Tier: `age = block_slot.saturating_sub(creation_slot)`; `quarters = age / quarter_slots`; `0->75, 1->50, 2->25, >=3->0` via the existing `withdrawal_penalty_rate_with_quarter` (`constants.rs:430-437`, the single ladder). `creation_slot = 0` (genesis bonds) ages from genesis (OQ-7 re-scoped). Golden `101 @ 75% => penalty 75, net 26`. Error code `ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET` (`error.rs:466-519`), bailed byte-identical at all sites; inherits `Infraction::InvalidBlock` -100.

**Resolver layer — `crates/storage/src/producer/withdrawal_inputs.rs`:**

```rust
pub struct WithdrawalInputs { pub owned_bonds: u32, pub all_bonds: u32, pub spent_bonds: Vec<(u64, Option<u32>)>, pub non_bond_total: u64, pub malformed_bond_inputs: Vec<(usize, usize)> }
pub fn resolve_withdrawal_inputs(tx: &Transaction, utxo: &UtxoSet, owner: &Hash) -> WithdrawalInputs; // one utxo.get per input, math-free
```

The resolver runs below the activation height too, so it is infallible: a Bond `extra_data` that is not 4 bytes decodes to `None` and is recorded in `malformed_bond_inputs`, and only the gated M4 check raises `WithdrawalBondExtraDataMalformed`.

**Rule:** `payout <= sum penalized_bond_net(spent Bond inputs) + sum(non-Bond inputs)` (OQ-2). Honest CLI slack >= 1 base unit at every tier (trace section 8). Over-burn accepted. A reorg re-mining across a quarter boundary into an earlier slot can reject deterministically — tx returns to mempool, never a fork.

**Age source:** node-stamped Bond `extra_data` (4-byte LE slot, all three write paths), inside the canonical encoding (`utxo/types.rs:57-77`), snap-sync safe; reference `block.header.slot`. Forbidden: `chain_state`, `SystemTime`, `ctx.current_time`, `ctx.prev_slot` (reads 0 at apply), `StoredBondEntry.creation_slot` (unit-confused, F3). A quarter: testnet 2,160 slots = 6 h = about 223 blocks (9.67 slots/block); mainnet 3,153,600 slots = 365 d = about 3.15 M blocks (about 1.06 slots/block, inferred); devnet 60. Slot age is the whitepaper's "1 yr"; a height tier would stretch a testnet year ~9.7x. Mainnet genesis bonds are Q1 until about 2027-07-22.

**Activation heights** (`network_params/mod.rs` after `:838`): `inc_i_171_vesting_penalty_activation_height`, `inc_i_171_vesting_penalty_disable_height`; `u64::MAX` on mainnet, testnet, devnet (decision #93); `env_loader.rs` mainnet-locked (shape `:484-489`); doc-comment: Q1 YES / Q2 YES / Q3 NO => AH; CONTENT NO => rolling. Tests: INV-PARAMS-002 triad.

**Admission-site parity**

| Site | Slot | Resolver / predicate | Modes |
|---|---|---|---|
| Block validation `validate_block_economics` (`:639-688`) | `block.header.slot` | resolver replaces inline `:669-684`; scan hoisted above both gates (`height >= min(both AHs)`); predicate under own gate, after R4 | Full/Light strict, pre-mutation; Replay warn-only as R2/R3 (`:831-842`) |
| Mempool `withdrawal_holdings::check` (`:28`) | non-regressing watermark `max(best_slot seen, wall-clock slot)` — the mempool is height-only today (`pool.rs:424-428`); `best_slot` alone regresses on reorg (`block_handling.rs:803,875`) | same fn, same `[CODE]`; vesting `Err` never evicts in `revalidate` (`pool.rs:1345-1352`) | liveness-only; builder is the safety gate |
| Builder `WithdrawalParity::allow` (`:92`) | `current_slot` (`assembly.rs:15`) | same | exact |
| Core `validate_transaction_with_utxos` (`utxo.rs:138-205`) | - | NOT used: post-mutation, Replay-muted, mempool-bypassed | - |

## Failure-Mode Filters Applied

| Filter | Verdict | How the design passes |
|---|---|---|
| F1 age input absent | REFUTED | stamping at three write paths; proof #2; PC-2 census kept |
| F2 age attacker-writable | REFUTED (UTXO view) | stamp overwrites tx bytes; same-block window closed by R4 |
| F3 ledger unit confusion | NEUTRAL | never reads `StoredBondEntry`; new incident |
| F4 restart migration | PASS | reads `UtxoEntry` only |
| F5 storage order | PASS by construction | fold over `tx.inputs`; `get_bond_entries` only as `.len()` |
| F6 rounding | PASS | penalty-first per-bond truncation; `<=`; golden 101@75 => 26 |
| F8 div-by-zero / env override | PASS given PC-3 | `quarter == 0 => Err`; lock param on all networks |
| F9 slot mode-bounded | PASS with note | Full allows +1 slot (`MAX_FUTURE_SLOTS = 1`), producer must be scheduled; Light/Replay see only Full-accepted blocks |
| F10 wall-clock leakage | PASS | header slot, pre-block view, params only (INV-VEST-002) |
| F11/F13 site parity | PASS | one resolver + one predicate, three sites |
| F15 persistence trap | PASS | validation-only (INV-VEST-009) |
| Radical ctx hard-zero | N/A | no `ValidationContext` change; INV-VEST-011 |
| Replay vs REQ-VEST-005 | RESOLVED | follow R2/R3: Replay re-processes only Full-accepted blocks, so a rejection there protects nothing and can wedge a reindex (INC-I-064) |

## Invariants to Register (`consensus/producer-withdrawal`)

| ID | Statement | Test intent |
|---|---|---|
| INV-VEST-001 | Age from the node-stamped Bond `extra_data` slot; never tx bytes | Bond built with `creation_slot = 999_999` applies with the block slot |
| INV-VEST-002 | Pure fn of (spent Bond inputs, non-Bond value, params, `block.header.slot`) | same block, two wall-clocks/tips => same verdict |
| INV-VEST-003 | Backend- and permutation-invariant | equal-slot/unequal-amount bonds, 100 seeds, both backends |
| INV-VEST-004 | Per-bond `penalty = amount*pct/100; net = amount - penalty`, summed after | golden 101@75 => 26; node bound >= CLI payout |
| INV-VEST-005 | Honest current CLI accepted at every tier at `AH-1` and `AH` | four tiers, both heights |
| INV-VEST-006 | `vesting_quarter_slots` non-zero, non-env-overridable everywhere | `0 => Err`; env refused; testnet/devnet own tiers |
| INV-VEST-007 | Below AH byte-identical to the prior binary | replay to `AH-1` reproduces state roots |
| INV-VEST-008 | Mempool, builder, validation: one verdict | INC-I-203-style 3-site harness |
| INV-VEST-009 | Nothing persisted; rollback exact | apply + roll back => parent root |
| INV-VEST-010 | Bond stamping (three paths: `state_db/batch.rs:149-150`, `utxo/in_memory.rs:96-97`, `utxo/set.rs:179-182`) consensus-load-bearing, byte-identical for every admissible input | NEW test that applies one Bond-creating block through the production RocksDB apply path AND the rebuild path (`set.rs` via `rollback.rs:303`) and asserts identical `extra_data`. The existing `twap_equivalence_test.rs:65-73` calls no production function (INC-I-213) and does NOT satisfy this row |
| INV-VEST-011 | Admission sites never evaluate at slot 0; the mempool slot is a non-regressing watermark; a vesting `Err` never evicts a tx in reorg `revalidate` | mempool + builder assert `current_slot > 0`; reorg test: tx valid at slot S stays in the pool after `best_slot` drops to the common ancestor |
| INV-VEST-012 | `[ERRTX007]` (`validation/transaction.rs:418-424`) rejects Bond `extra_data.len() != 4`, ungated, in Full, Light and Replay (`block.rs:237`, `:314`); it is load-bearing for INV-VEST-010 and INV-VEST-001 | tripwire: a Register/AddBond tx with `extra_data.len() in {0,3,5,8}` is rejected in all three modes at every height |
| INV-VEST-013 | The bound FAILS CLOSED on a malformed Bond: a spent Bond input whose `extra_data` does not decode to a u32 slot makes the withdrawal invalid. Never reuse `bond_creation_slot().unwrap_or(0)` (`in_memory.rs:325`, `queries.rs:204`), which fails OPEN (slot 0 = fully vested). Snap-sync ingress (`fork_recovery.rs:319`) bypasses ERRTX007, so malformed Bonds can exist in a snap-synced UTXO view | unit: `spent_bonds` with a 3-byte `extra_data` → `Err`, not `bound = amount` |

## Migration Path (deletions first; tests-first)

| M | Commit | Content | Tests first |
|---|---|---|---|
| M1 | `refactor: delete dead vesting-penalty implementations` | [F2] [F8] [F9] [F4]; optional single-evaluator items (decision 3) | builds; tests green; `req_vest_ap3_tripwire` |
| M2 | `feat(core): vesting-penalty predicate + activation heights frozen u64::MAX` | `vesting.rs`; error variant; enable/disable fields; env locks; zero guard | golden vectors; permutation; u128; `quarter=0 => Err`; INV-PARAMS-002 triad |
| M3 | `refactor(storage): one withdrawal-input resolver replaces three bond_input_split copies` | `withdrawal_inputs.rs`; three call sites; counts unchanged | bit-identical counts; INV-VEST-003/007 |
| M4 | `feat(node): enforce vesting payout bound under inc_i_171 gate` | hoist scan; predicate inside/after R4; Replay warn; INC-I-180 arm to a sibling module (file is 1,619 L) | reject full-value Q1 post-AH; honest CLI four tiers pre/post; over-burn; root unchanged on reject; `AH-1` identical |
| M5 | `feat(mempool,production): vesting bound at admission and build; thread a non-regressing slot watermark` | mempool gains a slot watermark (`max(best_slot seen, wall-clock slot)`; today `add_transaction` is height-only, `pool.rs:424-428`); vesting `Err` excluded from `revalidate` eviction (`pool.rs:1345-1352`); builder passes `current_slot` (`assembly.rs:377`); INV-VEST-011/013 | INV-VEST-008 harness; slot-0 rejection; reorg-keeps-honest-tx test; malformed-Bond fail-closed test |
| M6 | `feat(observability): shadow would-reject counter below AH` (optional) | `doli_vesting_would_reject_total`; verify WRITTEN (INC-I-187) | metric appears after one synthetic tx |
| M7 | `docs(specs): vesting penalty enforced from AH` + `refactor(rpc,cli): consume penalized_bond_net` (Should) | `tokenomics.md:51,66`; `protocol.md`; `security_model.md`; `docs/architecture.md` | `/sync-docs`; CLI golden unchanged |

No `BRIDGE:` entries: below the AH every path is byte-identical. Pinning is a separate decision-session gated by the Preconditions.

## Deploy Shape

- **Consensus RULES change? YES** (Q1 yes, Q2 yes, Q3 no) => own AH + paired disable; never bundled onto `withdrawal_holdings_gate_activation_height` (317_861 (mainnet; `defaults.rs:161`), crossed).
- **Block CONTENT change? NO** => rolling deploy safe while frozen; fleet fully upgraded before any pinned height.

## Preconditions for Pinning (gates, not decisions)

**Testnet:** T1 M1-M5 merged; INV-VEST-007 replay to tip. T2 the live testnet has ONLY genesis bonds (all 0%) — rehearse AddBond -> RequestWithdrawal at Q1..Q4 (quarter = 6 h), honest and patched CLIs. T3 zero honest would-rejects across >= 1 quarter boundary and >= 1 reorg. T4 `DOLI_VESTING_QUARTER_SLOTS` lock on all 18 nodes. T5 pin >= 24 h above the tip.
**Mainnet:** PC-1 INC-I-170 n11 retirement complete — its ~4,348 DOLI exit would face 75% (genesis bonds are Q1 until about 2027-07-22). PC-2 bond census via `getBondDetails` on every producer + slot/height ratio. PC-3 `vesting_quarter_slots` env-locked everywhere + zero guard shipped. PC-4 no bundling onto 317_861 (mainnet; `defaults.rs:161`). PC-5 all ~30 auto-update producers on the release; one seed at a time; >= 24 h lead. PC-6 testnet T1-T5 complete.
**Incidents opened from this run:** (a) **INC-I-212** — `StoredBondEntry.creation_slot` mixes HEIGHT (`producer/info.rs:74,430`) and SLOT (`tx_processing.rs:363`); not on the consensus path of this design. (b) **INC-I-213** — `twap_equivalence_test.rs:65-73` transcribes the deleted `utxo_rocks.rs` logic and calls no production function, so it cannot fail on a stamping regression; replace it per INV-VEST-010. **Refuted (verification 2026-09-05):** the suspected `utxo/set.rs:182` length hazard. The in-place write is guarded by `extra_data.len() >= 4` (`set.rs:179-182`, no panic), and `[ERRTX007]` (`validation/transaction.rs:418-424`) rejects any Bond `extra_data.len() != 4` ungated in all modes, so the three write paths are byte-identical for every input a validated block can carry. The only ingress that bypasses ERRTX007 is snap sync (`fork_recovery.rs:319`), covered by INV-VEST-013 (fail closed).

## Complexity Comparison

| Metric | Current | Analyst SSF | Radical (A) | Recommended |
|---|---|---|---|---|
| Consensus enforcement points | 0 | 1 | 1 (post-mutation) | 1 (pre-mutation) |
| Admission-site parity | - | 3, hand-copied | 2 of 3 | 3 of 3, one resolver |
| Ladder applications | 7 / 9 families | 8 | 1 | 1 (+ RPC/CLI consumers after M7) |
| Files (crates) touched | - | 7 (4) | 9 + 5 deletion (2) | ~10 (4) + 5 deletion-only |
| New modules / pub fns / types | - | 0 / 1 / 0 | 1 / 2 / 0 | 2 / 3 / 1 struct + 1 error |
| New params / ctx fields | - | 1 / 0 | 1 / 3 | 2 / 0 |
| Extra UTXO lookups . locks | - | +N . +1 | 0 . 0 | 0 . 0 |
| New tests | - | ~10 | ~12 | ~16 |
| Lines | - | ~+157 | ~+88 / -60 | ~+220 / -600 (net about -380) |

## Requirements Traceability

| REQ | Pri | Design element | M |
|---|---|---|---|
| VEST-001 payout bounded | Must | predicate in `validate_block_economics` | M2, M4 |
| VEST-002 one implementation | Must | single ladder; deletions; RPC/CLI consume | M1, M2, M7 |
| VEST-003 own AH frozen | Must | enable + disable `u64::MAX` x3 | M2 |
| VEST-004 deterministic | Must | fold over `tx.inputs`; u128; INV-VEST-003 | M2, M3 |
| VEST-005 pre-mutation | Must (Replay warn) | `mod.rs:113` before `:198` | M4 |
| VEST-006 tier from params | Must | `quarter_slots` from params; wallet ladder deleted | M1, M2 |
| VEST-007 block's own slot | Must | `block.header.slot`; INV-VEST-002 | M4 |
| VEST-008 3-site parity | Should | INV-VEST-008/011 | M3, M5 |
| VEST-009 honest CLI valid | Should | `<=`, penalty-first; INV-VEST-005 | M2, M4 |
| VEST-010 delete dead impls | Should | [F2] [F8] [F9] | M1 |
| VEST-011 burn observable | Could | shadow counter | M6 |
| VEST-012 ledger FIFO | Could | not built; incident (a) | - |
| VEST-013/014/015 | Won't | untouched | - |

## Open Decisions for the User

1. **Replay mode:** warn-only (R2/R3 precedent, default; amends REQ-VEST-005) or strict (can wedge a reindex).
2. **Paired disable height [F7]:** adopt (default) or strike.
3. **Single-evaluator deletions in M1:** `consensus/bonds.rs` (312 L), `simulateWithdrawal` path, bare `withdrawal_penalty_rate`, `unbonding_period()` accessor — now or defer.
4. **M6 shadow counter:** include or defer.
5. **M7 strict REQ-VEST-002** (RPC/CLI call `penalized_bond_net`): now or follow-up.

## Verification Addendum (2026-09-05, `docs/.workflow/design-verification.md`)

The user rejected the first gate presentation because two code-verifiable residuals were still open. A targeted read-only verification pass closed them:

| Residual | Verdict | Evidence | Effect on the design |
|---|---|---|---|
| R1 mempool slot source / monotonicity | OPEN → resolved as a DESIGN AMENDMENT (liveness, not consensus) | mempool is height-only (`pool.rs:424-428`); `best_slot` regresses on reorg (`block_handling.rs:803,875`); `revalidate` evicts on `Err` (`pool.rs:1345-1352`); tier monotone (`constants.rs:430-437`); builder slot = header slot (`assembly.rs:377`, `block.rs:385`) | mempool uses a non-regressing slot watermark; vesting `Err` never evicts on reorg; builder is the exact safety gate (INV-VEST-011, M5) |
| R2 `set.rs:182` write shape | CLOSED | `batch.rs:149-150` and `in_memory.rs:96-97` REPLACE the vec with u32 LE; `set.rs:179-182` writes `[..4]` in place behind `len() >= 4`; `[ERRTX007]` `transaction.rs:418-424` rejects `len != 4` ungated in Full/Light/Replay; production uses RocksDB (`init.rs:317`), rebuild uses `set.rs` (`rollback.rs:303`, `block_handling.rs:930`) | byte-identical for every admissible input; INV-VEST-012 registers ERRTX007 as load-bearing; INC-I-213 opened for the fake tripwire |
| New: fail-open decode default | ADOPTED as filter | `bond_creation_slot().unwrap_or(0)` (`in_memory.rs:325`, `queries.rs:204`) returns "fully vested" on a malformed Bond; snap-sync ingress `fork_recovery.rs:319` bypasses ERRTX007 | INV-VEST-013: the bound fails closed on a malformed spent Bond |

What remains open, and why it cannot be closed in a proposal-only run: (1) no test has executed, because no code exists yet — M2 golden vectors (FAIL→PASS), the INV-VEST-008 three-site harness and the four-tier testnet rehearsal are the evidence that raises confidence further; (2) the mainnet Bond census (PC-2) needs a live RPC sweep of every producer and is a pinning precondition, not a design input; (3) the CLI payout arithmetic was compared with the bound by reading, not by executing (golden `101@75% => 26`).

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     3 (3+/5)
Restructuring convergence:      4 (spent-inputs fold 5/5; node site 4/5; own AH 5/5; core home 3/5)
Addition options presented:     5
Failure modes identified:       13
Failure modes applied as filters: 13/13
Radical floor gap:              0 -> radical 1 site (core, 2/3 parity) -> proposed 1 site (node, 3/3 parity, +1 module, +2 params)
Contradictions found:           4 (age source; check site; fn home; Replay vs REQ-VEST-005)
Contradictions resolved:        4/4 (by source, not vote)
Evidence independence verified: YES
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

VERDICT conf(0.85, measured + verified) — site, rule shape, age source, deletions, stamping equivalence (R2) and the mempool premise (R1) were all read from source at the cited lines and cross-checked on live testnet; no code-verifiable residual remains open. The gap to 1.0 is exactly: no test has executed (proposal-only run — closed by M2 golden vectors FAIL→PASS, the INV-VEST-008 harness and the four-tier testnet rehearsal), the mainnet Bond census (PC-2, live RPC sweep before any pin), and CLI-vs-bound arithmetic verified by reading only.
