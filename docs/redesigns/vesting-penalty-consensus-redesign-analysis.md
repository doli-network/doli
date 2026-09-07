# Redesign Analysis — Consensus Enforcement of the Bond-Withdrawal Vesting Penalty (INC-I-171)

Analyst output for `/omega-redesign` (proposal-only). **This document scopes the problem. It does not choose the design.**
Companion evidence: `docs/.workflow/constraint-table-vesting.md`.
Date 2026-09-05 · Branch `feature/inc-i-178-attestation-bls` · Code read at that commit.

---

## SSF — the simplest one-sentence solution that resolves the root cause

> **Bound the `RequestWithdrawal` payout at consensus: reject any block where a `RequestWithdrawal`'s single output exceeds `Σ over the Bond inputs it actually spends of amount − (amount × penalty_pct(age)/100)` plus the total value of its non-Bond inputs — computed from the pre-block UTXO view, in `validate_block_economics` beside the existing INC-I-180 holdings gate, behind one new activation height.**

Why this works: it deletes the free parameter. Today the payout is a client-chosen number bounded only by conservation; this replaces the upper bound with the protocol's own number. Computing over the **inputs the tx actually spends** (rather than "the first N entries of the producer's bond list") makes the sum order-independent, so it is deterministic across the in-memory and RocksDB UTXO backends, and it never rejects a tx the honest CLI builds. Non-FIFO selection is not an attack: picking younger bonds only makes the withdrawer pay a larger penalty.

---

## Anchor detection (deterministic-reasoning protocol)

No `docs/.workflow/skeptic-analysis.md` exists, so the internal procedure applies.

**FIRST READ** (the incident's own framing): *"one predicate is missing — add `total_output <= calculate_withdrawal(count, current_slot).net` and gate it."* Set aside.

**CONTRADICTING SECOND READ**: *the withdrawal path has no protocol-level definition of its payout at all.* Evidence found in code, not assumed: (a) `unbonding_period` is declared (`network_params/mod.rs:87`), defaulted per network (`defaults.rs:37,365,650`), env-guarded and unit-tested, yet has **zero consensus consumers** — its only accessor `NetworkTiming::unbonding_period()` (`network/timing.rs:135`) has exactly one caller, a test (`network/tests.rs:216`); (b) `TxType::ClaimWithdrawal` is hard-rejected — `validation/transaction.rs:155-158` returns `InvalidClaimWithdrawal("ClaimWithdrawal is not supported")`; (c) the payout is `Output::normal(...)`, and `transaction/output.rs:139` sets `lock_until: 0`.

**Chosen: the second.** The penalty gap is one symptom of a wider one — *every* economic term of `RequestWithdrawal` (payout, unbonding lock, two-step claim) is client-defined. The SSF above remains the recommended *mechanism*: it is the narrowest change that closes the root cause for the payout term. The unbonding term is scoped below as explicitly-not-now, so the redesign does not foreclose it.

---

## Whitepaper — design intent, quoted verbatim

`WHITEPAPER.md:604-615`:

> Early withdrawal incurs a tiered FIFO vesting penalty based on individual bond age:
> `<1 yr → 75% penalty / 25% returned` · `1-2 yr → 50%/50%` · `2-3 yr → 25%/75%` · `3+ yr → 0%/100%`
>
> A producer with bonds of mixed ages can withdraw selectively. The oldest bonds (lowest penalty) are withdrawn first. This rewards long-term commitment while allowing flexible exit.
>
> **All penalties are burned permanently, removing coins from circulation.**

`WHITEPAPER.md:600` (the unbonding claim, also unenforced):

> **Withdrawal uses a two-step process with a 7-day unbonding period** (60,480 blocks). A producer submits a `RequestWithdrawal` transaction, which begins the unbonding countdown. After 60,480 blocks (~7 days), the producer submits a `ClaimWithdrawal` transaction to receive the funds.

`WHITEPAPER.md:1021`: "Their bonds vest on the same schedule … and face the same withdrawal penalties."

**Specs agree with the whitepaper and with each other**: `specs/tokenomics.md:51,66` — "Penalty destination: `PenaltyDestination::Burn`. Penalty amount disappears from supply." and lists the schedule as **LIVE**. That "LIVE" label is spec drift: the schedule is computed, not enforced (see Drift, below).

---

## Scope

**In scope**: the `RequestWithdrawal` economic terms — payout bound, penalty derivation, penalty destination — across validation, apply, rebuild, mempool admission, block building, rollback, CLI, RPC, snap sync, and the activation-height/deploy shape.
**Out of scope for this analysis**: `Exit`, `SlashProducer`, delegation unbonding, AMM/DeFi paths, and the INC-I-170 mainnet retirement operation (a hard dependency — see Open Question 6).

---

## Capability inventory (PRIOR-KNOWLEDGE-GATE — built before any "cannot" claim)

| Inventory | Count | Notes |
|---|---|---|
| `TxType` variants (`crates/core/src/transaction/types.rs`) | **24** | Withdrawal-relevant: `Exit=2`, `ClaimBond=4`, `AddBond=7`, `RequestWithdrawal=8`, `ClaimWithdrawal=9`, `DelegateBond=13`, `RevokeDelegation=14`. `ClaimWithdrawal` exists as a discriminant but is rejected at validation. |
| `OutputType` variants | **14** | `Normal=0, Bond=1, Multisig=2, Hashlock=3, HTLC=4, Vesting=5, NFT=6, FungibleAsset=7, BridgeHTLC=8, Pool=9, LPShare=10, ZKRollup=13, EncryptedContent=14, OraclePrice=15`. **`Vesting=5` (`Signature(owner) AND Timelock(unlock_height)`, `WHITEPAPER.md:171`) already exists** — the chain is not missing a primitive for a time-locked payout. |
| Covenant/condition types | the 14 `OutputType`s + `lock_until: u64` + per-output `extra_data`. `Output::normal` hardcodes `lock_until: 0` (`output.rs:139`); nothing prevents a non-zero value. |
| Activation-height fields in `NetworkParams` | **27** (`grep -c "activation_height: u64" crates/core/src/network_params/mod.rs`) | Nearest precedents: `withdrawal_holdings_gate_activation_height` (`mod.rs:374`), `addbond_cap_enforcement_activation_height` (`:369`), `inc_i_204_fork_choice…` (`:810`), `inc_i_178_attestation_bls…` (`:826`), `inc_i_208_own_attestation…` (`:838`). |
| Penalty implementations already in the tree | **5** | `withdrawal_penalty_rate_with_quarter` (`consensus/constants.rs:430`, the one shared primitive); `calculate_exit_with_quarter` (`consensus/exit.rs:84`); `calculate_withdrawal_from_bonds` (`producer/info.rs:21`); `ProducerInfo::calculate_withdrawal_with_quarter` (`info.rs:450`); `compute_fifo_breakdown` (CLI, `common.rs:23`). Plus the RPC advisory at `rpc/methods/producer.rs:314`. |
| Validation functions on the `RequestWithdrawal` path | **4** | stateless `validate_withdrawal_request_data` (`validation/tx_types.rs:560`); UTXO/conservation (`validation/utxo.rs:80,271`); block-level `validate_block_economics` (`validation_checks.rs:481`, gate `:625`); mempool `withdrawal_holdings_verdict` (`mempool/pool.rs:314`). |

**Claim now licensed by the inventory**: the system already has every primitive needed (a shared penalty function, a per-bond `creation_slot`, a pre-mutation block-validation hook that reads the UTXO owner index, a 27-slot AH mechanism, and a `lock_until` field). Nothing new must be invented; what is missing is a *rule*.

---

## Current architecture map (file:line)

### Transaction shape
- `Transaction::new_request_withdrawal` — `crates/core/src/transaction/core.rs:416-433`. Inputs = Bond UTXOs (+ optional fee UTXOs); `outputs: vec![Output::normal(net_amount, destination)]`; `extra_data` = `WithdrawalRequestData{producer_pubkey, bond_count, destination}` (`transaction/data.rs:449`). Doc-comment `core.rs:414`: *"Penalty is implicitly burned (sum(bond inputs) − net_amount = burned)"* — the burn is an **omission**, never an output.
- Bond `extra_data` carries the 4-byte LE creation slot (`WHITEPAPER.md:110`), read via `Output::bond_creation_slot()`.

### Validation (three layers, none checks the amount)
1. Stateless `validate_withdrawal_request_data` — `validation/tx_types.rs:560-618`: ≥1 input, exactly 1 output, `Normal` type, `amount > 0`, parseable `extra_data`, `bond_count > 0`, `destination != ZERO`, `output.pubkey_hash == destination`. Its closing comment (`:614-617`) says **"Output amount <= FIFO net calculation"** is "done at node level" — it is not.
2. UTXO/conservation `validation/utxo.rs:271` — `total_input < total_output` → `InsufficientFunds`. `utxo.rs:153` grants `RequestWithdrawal`/`Exit` a `lock_bypass` to spend `lock_until=MAX` Bond UTXOs.
3. `validate_block_economics` — `bins/node/src/node/validation_checks.rs:481`; INC-I-180 gate at `:625` enforces **counts only** (`[ECON_WITHDRAWAL_*]` at `:767,784,818,853,878,890`). **No amount predicate anywhere.**

### Apply, rebuild, rollback
- Apply — `apply_block/tx_processing.rs:376`: reads `bond_count`, `withdrawal_pending_count`, `delegated_bonds`, adds `pending_addbond_count` post-AH, bumps `withdrawal_pending_count`, queues `PendingProducerUpdate`. **Deferred to the epoch boundary.** No amount is read; no burn recorded.
- Rebuild mirror — `bins/node/src/node/rewards.rs:1368` mirrors the enqueue condition (INV-EPOCH-003).
- Rollback — `bins/node/src/node/rollback.rs` has **no** withdrawal-specific logic; undo is generic UTXO + ProducerSet restore, so a **validation-only** rule adds nothing there. Caveat INC-I-159: `unbonding_index` is `#[serde(skip)]` and is emptied on every undo-based ProducerSet restore — a design storing per-withdrawal state would land in that broken path.

### Mempool + block builder (parity sites)
- `crates/mempool/src/pool.rs:314-345` `withdrawal_holdings_verdict` (same AH gate) + `resident_withdrawn`; core logic `crates/mempool/src/withdrawal_holdings.rs:88`. Builder twin: `bins/node/src/node/production/withdrawal_holdings.rs:68,96,141`.
- **A new rule must land at all three sites** (INC-I-203) or the tx is admitted, gossiped, packed, and then costs a producer its block.

### CLI (where the protocol rule currently lives)
- `bins/cli/src/cmd_producer/withdrawal.rs:198` — `let payout_amount = total_net + fee_change;`, `total_net` from `compute_fifo_breakdown` (`common.rs:23`) over the RPC's per-bond `penalty_pct`. Inputs = `select_bond_inputs_by_count` (`bins/cli/src/producer_ledger.rs:30`) = first `count` of the RPC's FIFO-sorted list. Flat fee 1 base unit (`withdrawal.rs:185`).

### RPC
- `crates/rpc/src/methods/producer.rs:302-340` `getBondDetails` — bonds from `utxo_set.get_bond_entries()`, `penalty_pct` via `withdrawal_penalty_rate_with_quarter` (`:314`). Advisory only. Also `:76`.

### Parameters
- `withdrawal_penalty_rate_with_quarter(age, quarter)` — `consensus/constants.rs:430-437`: `quarters = age/quarter; 0→75, 1→50, 2→25, _→0`. Integer division; `penalty = amount*pct/100` truncating.
- `vesting_quarter_slots` — `network_params/mod.rs:151`; **mainnet 3_153_600** (`defaults.rs:75` ← `constants.rs:392`), **testnet 2_160** (`defaults.rs:403`), **devnet 60** (`defaults.rs:688`); mainnet env-LOCKED (`env_loader.rs:213-217`).
- Destination already modelled: `PenaltyDestination::Burn` is `#[default]`, `RewardPool` is `#[deprecated]` (`consensus/exit.rs:13-20`).

---

## Tangles found

**T1 — The protocol rule lives in the client.** The only enforcement of the whitepaper penalty is `bins/cli/src/cmd_producer/withdrawal.rs:198`. The boundary is inverted: a consensus rule sits downstream of the layer that should own it.

**T2 — Five implementations of one rule; the two canonical-looking ones are dead.** `ProducerInfo::calculate_withdrawal*` and `calculate_withdrawal_from_bonds` (`storage/producer/info.rs:21,443,450`) have **zero non-test callers** (grep-confirmed; consistent with `blast.py`, which found only `ProducerInfo`'s own impl). The *live* computation is duplicated in the CLI and the RPC. The rule with a name is unused; the rule in use has no name.

**T3 — The burn is an omission, not an act, and nothing records it.** **Verified it does not leak to the block producer**: the coinbase is `base_reward + extra_fees`, where `extra_fees` is a per-**byte** charge over `output.extra_data` lengths (`validation_checks.rs:557-573`), never `total_input − total_output` (`:583`). So omitted value is genuinely destroyed — the "burn by omission pays the producer" hazard does **not** apply. It is, however, unobservable and unauditable.

**T4 — A latent non-determinism in the obvious fix.** `get_bond_entries` collects from a `HashMap<Outpoint, UtxoEntry>` (`utxo/in_memory.rs:23,243,319`) then `sort_by_key(creation_slot)` — a *stable* sort over a *nondeterministic* order; the RocksDB twin (`state_db/queries.rs:195-210`) sorts the same way over a different base order. For bonds sharing a `creation_slot` but differing in `amount` — reachable, since INC-I-184 shows Bond amounts are unconstrained at creation — "the first N entries" is **not** a deterministic set. Any rule shaped as `calculate_withdrawal(count, current_slot)` (the incident's own proposal) inherits this and **can fork the network**. Computing over the *spent inputs* avoids it: summation is order-independent.

**T5 — Ledger FIFO and UTXO FIFO can diverge.** `ProducerInfo::apply_withdrawal` removes the **oldest** `count` `bond_entries` at the epoch boundary, while the tx may spend *any* `count` Bond UTXOs it owns (INC-I-180 R2 binds the count, not the identity). Counts stay consistent (INV-BOND-001 holds), but surviving `bond_entries[].creation_slot` values can drift from the surviving Bond UTXOs.

**T6 — Two more whitepaper terms are unenforced on the same path.** `unbonding_period` has zero consensus consumers; `ClaimWithdrawal` is rejected as "not supported"; the payout is `lock_until: 0`. The whitepaper's 7-day coin lock does not exist — only the *weight* removal is deferred to the epoch boundary.

---

## Architecture Context

### Module boundaries, data flow
See "Current architecture map" above for the per-module file:line detail. Dependency direction: `doli-core::validation` (no `ProducerSet` handle — decision 75) ← `doli-storage::{producer,utxo}` ← `bins/node::validation_checks` (the only pre-mutation hook seeing BOTH UtxoSet and ProducerSet in Full/Light/Replay) ← `doli-mempool` + `bins/node::production` (admission twins). Flow:
`CLI (getBondDetails → compute_fifo_breakdown → payout) → gossip → mempool admission (counts only) → block builder (counts only) → validate_block_economics (counts only) → apply_block (queue PendingProducerUpdate) → epoch flush (apply_withdrawal, remove oldest N) → ProducerSet/UtxoSet → state root → snap-sync snapshot.` **The amount travels the whole path unexamined.**

### Constraints & invariants
Statements in `docs/.workflow/constraint-table-vesting.md` §4. Binding: **INV-BOND-001** · **INV-CONSENSUS-002** (post-AH rejection pre-mutation) · **INV-PARAMS-001 / INC-I-054** (mainnet AHs immutable; own AH, never bundled) · **INV-EPOCH-003** (apply ≡ rebuild) · **INV-COMPAT-001** (below the gate, character-identical; ~30 external auto-update producers on mixed versions) · **INV-DEPLOY-001** · plus determinism (T4).

### Blast radius
- **Direct** (change together): `bins/node/src/node/validation_checks.rs`, `crates/mempool/src/withdrawal_holdings.rs`, `bins/node/src/node/production/withdrawal_holdings.rs`, `crates/core/src/network_params/{mod,defaults,env_loader}.rs`, plus wherever the single shared penalty primitive lands.
- **Indirect** (must agree, need not change): `bins/cli/src/cmd_producer/{withdrawal,common}.rs` (else honest users get rejected), `crates/rpc/src/methods/producer.rs` (advisory numbers must match), `apply_block/tx_processing.rs` + `rewards.rs:1368` (only if the apply side changes — it need not).
- **Not affected**: `rollback.rs` / undo path (no withdrawal-specific state); snapshot + state-root serialization (a validation-only rule writes nothing).
- **Graph cross-check**: `blast.py calculate_withdrawal --hops 2` → 1 dependent (`ProducerInfo`, `info.rs:47`). Grep confirms and extends: the *dead* function has no consumers, but the *live* rule has 3 independent re-implementations the graph cannot see because they are copies, not calls. **The graph's silence is a true negative about calls and a false negative about the rule.**

### Brittleness check
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
  YES Contract absence — no explicit interface defines the RequestWithdrawal
      payout; CLI/RPC/node agree by convention only. That IS the bug.
  YES Shared mutable state — the bond ledger has two owners (UtxoSet,
      ProducerSet) reconciled only at the epoch flush (T5, INV-BOND-001).
  NO  Cross-module blast radius — 3 sites, but an established parity triad
      with a working precedent (INC-I-180 / INC-I-203).
  NO  Invariant gaps — the invariant is computable from the pre-block UTXO view.
  NO  Data flow reversal — the rule reads the direction data already flows.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```
The *payout* problem is localized. T6 (unbonding/ClaimWithdrawal) is a separate, larger problem — do not let it expand this redesign.

---

## Impact analysis

| Area | Effect | Risk |
|---|---|---|
| `validation_checks.rs` gate block | New predicate inside/next to the existing AH block | **medium** — 1619-line file, near the 500-line module budget concern; the gate block is already dense |
| Mempool + builder twins | Must mirror or admission diverges from validation | **medium** — INC-I-203 is the cautionary precedent |
| Honest CLI withdrawals | Must remain valid pre- and post-AH | **high** if the bound is `==` instead of `<=` (fee-change term, truncation) |
| Historical blocks | None below the AH; the block is skipped whole (INC-I-180 pattern) | **low** |
| State roots | Validation-only ⇒ no state write ⇒ no root change | **low** |
| Snap-synced nodes | Need only the pre-block UTXO view; Bond `extra_data` (creation slot) is inside `UtxoEntry` and inside `serialize_canonical` | **low** — but **must be verified**, not assumed |
| INC-I-170 mainnet retirement | The operation currently **depends** on the penalty staying unenforced | **high** — sequencing hazard, see OQ-6 |
| Mixed fleet (~30 external auto-update producers) | Pre-AH path byte-identical | **medium** — INV-COMPAT-001 |

### Regression risk areas
- Fee change folded into the payout output (`withdrawal.rs:198`) — an exact-equality rule rejects every honest tx that has change.
- Integer truncation: `amount*pct/100` truncates; CLI and node must truncate **per bond**, not on the aggregate.
- Bonds decoding to `creation_slot = 0` (`bond_creation_slot().unwrap_or(0)`) read as maximally aged ⇒ **fully vested, zero penalty**. Confirm intent before the gate makes it consensus (OQ-7).
- `current_slot` source: RPC uses `chain_state.best_slot`; a consensus rule must use the **block's own** slot/height from the validation context (INC-I-173 C4 precedent), never a node-local read.

---

## Requirements

| ID | Requirement | Priority | Acceptance criteria (summary) |
|---|---|---|---|
| REQ-VEST-001 | A `RequestWithdrawal` payout must be bounded at consensus by the penalized net of the Bond inputs it spends plus its non-Bond input value | **Must** | Full-value withdrawal of Q1 bonds is REJECTED post-AH; honest CLI tx ACCEPTED |
| REQ-VEST-002 | The penalty rule must have exactly ONE implementation, reused by node validation, mempool, builder, CLI and RPC | **Must** | Grep shows one definition; CLI/RPC call it rather than re-implement |
| REQ-VEST-003 | The rule must be gated by its OWN new `NetworkParams` activation height, frozen (`u64::MAX`) on every network at ship | **Must** | Below AH the code path is byte-identical to the current binary |
| REQ-VEST-004 | The rule must be deterministic: integer-only, order-independent, backend-independent | **Must** | Same verdict for in-memory and RocksDB backends with equal-slot / unequal-amount bonds |
| REQ-VEST-005 | Rejection must be pre-mutation, in all validation modes (Full/Light/Replay) | **Must** | No ProducerSet or UtxoSet write occurs for a rejected block |
| REQ-VEST-006 | The tier must derive from `NetworkParams.vesting_quarter_slots`, never the mainnet constant | **Must** | Testnet (2_160) and devnet (60) produce testnet/devnet tiers |
| REQ-VEST-007 | Age must be computed from the **block's** slot/height in the validation context | **Must** | Two nodes validating the same block at different wall-clock times agree |
| REQ-VEST-008 | Mempool admission and the block builder must reach the same verdict as block validation | **Should** | A tx rejected by the gate is never admitted, gossiped or packed |
| REQ-VEST-009 | The honest CLI must remain valid before AND after activation without a CLI upgrade being mandatory for correctness | **Should** | Replay of historical honest withdrawals passes; a current-release CLI tx passes post-AH |
| REQ-VEST-010 | Delete or wire the dead penalty implementations (`info.rs:21,443,450`) | **Should** | Wiring-census clean; no orphan `pub fn` |
| REQ-VEST-011 | Make the burn observable (metric/log/RPC field for penalized value destroyed) | **Could** | Auditors can total burned value without replaying blocks |
| REQ-VEST-012 | Bind the ledger-side FIFO to the UTXO-side selection (T5) | **Could** | `bond_entries` creation slots match surviving Bond UTXOs after a non-FIFO withdrawal |
| REQ-VEST-013 | Enforce the 7-day unbonding lock and/or implement `ClaimWithdrawal` (T6) | **Won't** | Deferred — its own incident, its own AH. Must not be bundled here. |
| REQ-VEST-014 | Change the penalty destination away from burn (treasury/reward pool) | **Won't** | Whitepaper says burn; a change is a tokenomics decision, not this redesign. |
| REQ-VEST-015 | Fix signer→producer binding (INC-I-182) or bond amount/owner constraints (INC-I-184) | **Won't** | Separate open incidents with their own AHs. |

### Detailed acceptance criteria (Must)

**REQ-VEST-001** — Given a producer with `n` Q1-tier (75%) bonds, when it submits a `RequestWithdrawal` whose single output equals the full Bond input total, then post-AH the block is rejected with a distinct `[ECON_WITHDRAWAL_*]` code and **no state is mutated**; given the same producer using the released CLI, the tx is accepted both pre- and post-AH; given a fully vested (Q4+) producer, full-value withdrawal is accepted at every height; given an output *below* the bound (over-burn), the tx is accepted.

**REQ-VEST-003** — Given `height = AH − 1`, validation of every historical block is identical to the pre-change binary (full replay reproduces every state root). Given all three network defaults, the shipped value is `u64::MAX`; pinning is a separate decision-session (decision #93).

**REQ-VEST-004** — Given a producer holding two Bond UTXOs with the **same** `creation_slot` and **different** `amount`s, the verdict is identical for `UtxoSet::InMemory` and `UtxoSet::RocksDb` and stable across runs (HashMap seed variation); given any permutation of the tx's inputs, the bound is identical.

**REQ-VEST-006** — Given `vesting_quarter_slots = 2_160` (testnet), a bond aged 2_200 slots yields 50%, not 75%; the `VESTING_QUARTER_SLOTS` constant is not referenced on the consensus path.

**REQ-VEST-005 / 007** — No ProducerSet or UtxoSet write occurs for a rejected block in any validation mode; the same block validated at two different node-local slots yields the same verdict.

---

## Acceptance criteria for the redesign itself

**Behaviour preserved**
1. Honest CLI withdrawals valid before AND after activation (REQ-VEST-009).
2. `ClaimWithdrawal` semantics unchanged (it stays rejected).
3. Every state root at `height < AH` byte-identical; **no genesis reset** (CLAUDE.md #0).
4. A snap-synced node reaches the same verdict from the snapshot alone. **Verification owed**: the rule may read only the UTXO set (Bond `extra_data` → creation slot, amount, owner) plus `NetworkParams`; it must NOT depend on `pending_updates` (INC-I-181: those sit outside state-root coverage) or on block history a snap-synced node lacks.

**Structural properties**
5. Exactly ONE consensus implementation of the penalty; CLI and RPC consume it (REQ-VEST-002).
6. Integer-only, order-independent, unit-testable without a node (REQ-VEST-004).
7. Pre-mutation, all validation modes, three-site parity (REQ-VEST-005, -008).

**Non-foreclosure (Rule 18 redesign variant)**
8. The structure must not re-create the escaped dead end — no rule that is *computed* somewhere and *enforced* nowhere. Every new penalty-adjacent function ships with a production caller or a wiring-debt row.
9. Evolution paths that must stay reachable **without being built now**: (a) per-bond partial withdrawals; (b) delegation withdrawals under the same tier; (c) a change of penalty destination; (d) the 7-day unbonding lock via the existing `lock_until` / `OutputType::Vesting` primitives. Keeping the rule a pure function of (spent Bond inputs, `vesting_quarter_slots`, block slot) leaves all four open.

**Deploy shape — both mandatory questions answered**
- **Consensus RULES change? YES.** Three-question checklist (INV-12): Q1 a user-submittable `RequestWithdrawal` reaches this path = **YES**; Q2 producer-action reachable = **YES**; Q3 bit-identical for all reachable inputs = **NO** (full-value withdrawals flip valid→invalid). ⇒ **Its own activation height is REQUIRED.** Never bundle onto `withdrawal_holdings_gate_activation_height`, already crossed on mainnet at 301_020 — moving it would violate INV-PARAMS-001 / INC-I-054.
- **Block CONTENT change? NO** — no bitfield, coinbase, tx-ordering, `presence_root` or header change; a rejected block is simply never built. ⇒ INV-DEPLOY-001's synchronized-deploy requirement does **not** apply. **A rolling deploy is safe**, because the gate is a pure `height >= AH` check: below the pinned height all binaries behave identically, so old and new coexist; the fleet must simply be fully upgraded *before* the pinned height. Ship frozen (`u64::MAX`), soak, then pin above the measured live tip with ≥24 h lead (decision #81).

---

## What I do not understand (mandatory honesty section)

1. **Whether INC-I-170's mainnet retirement is complete.** The INC-I-171 record states the fix is deferred *because* that operation relies on the penalty staying unenforced. I did not verify the current state of INC-I-170. If exposed producers still need patched-CLI full-value exits, pinning this AH strands them. **This is a sequencing prerequisite, not a design question.**
2. **Whether genesis/legacy Bond UTXOs actually carry a creation slot.** `bond_creation_slot().unwrap_or(0)` makes a missing slot read as "maximally aged / fully vested". I did not enumerate mainnet Bond UTXOs to see how many decode to 0.
3. **Whether the RocksDB and in-memory `get_bond_entries` really can disagree on live data.** I proved the *code* permits it (stable sort over differing base orders); I did not construct the divergence. It matters only for a rule shaped as "first N entries" — which I am recommending against.
4. **Whether `serialize_canonical` preserves Bond `extra_data`.** The doc comment claims bit-identity between backends for the whole `UtxoEntry`, and `deserialize_canonical` reconstructs entries, but I did not read the entry codec byte-by-byte.
5. **The exact fee semantics the community expects** — whether the 1-base-unit flat fee should be carved from the gross or the net (OQ-4).

---

## Open questions (user unavailable — defaults stated so evaluators can proceed)

| # | Question | **Default assumption** (with basis) |
|---|---|---|
| OQ-1 | Where does the burned value go? | **Destroyed (burn by omission), unchanged.** Verified it does not reach the producer (T3). Basis: `WHITEPAPER.md:615`, `specs/tokenomics.md:51,72`, `PenaltyDestination::Burn`. |
| OQ-2 | `total_output <= net` or `== net`? | **`<=` (over-burning allowed).** Equality rejects honest txs — fee change is folded into the single output (`withdrawal.rs:198`) — and is brittle to truncation. Over-burning destroys only the withdrawer's own coins. |
| OQ-3 | Does the penalty apply to delegation withdrawals? | **No — out of scope.** `DelegateBond`/`RevokeDelegation` are zero-flow state-only txs (INV-VALIDATION-008) with their own `DELEGATION_UNBONDING_SLOTS` (`constants.rs:619`); the whitepaper vests *bonds*, not delegations. Left reachable as evolution path (b). |
| OQ-4 | Is the fee carved from the net or the gross? | **Neither — the fee comes from separate non-Bond inputs** and its change is folded into the payout, so the bound must be `penalized_net(Bond inputs) + Σ(non-Bond inputs)`. Basis: `withdrawal.rs:172-198`. |
| OQ-5 | Testnet vs mainnet activation policy? | **Ship all three networks frozen at `u64::MAX`** (devnet too); pin testnet first after a soak; pin mainnet in a separate decision-session above the measured live tip with ≥24 h lead. Basis: decisions #93, #81; INV-PARAMS-001. |
| OQ-6 | Does INC-I-170 still require unenforced penalties? | **Assume YES until an operator confirms otherwise** — "INC-I-170 retirement complete" is a hard precondition for pinning any mainnet height. Implementation may proceed frozen. Basis: INC-I-171 record, 2026-08-09 update. |
| OQ-7 | Should `creation_slot == 0` mean "fully vested"? | **Yes, preserve current behaviour** (`unwrap_or(0)`), so the gate never *increases* the penalty on an existing bond relative to what `getBondDetails` already told its owner. Basis: `in_memory.rs:325`, `queries.rs:204`. |

---

## Specs drift detected

| File | Drift |
|---|---|
| `specs/tokenomics.md:51,66` | Lists the vesting penalty as **LIVE** with `PenaltyDestination::Burn`. It is computed and client-applied, never consensus-enforced. Should read **CLIENT-SIDE ONLY / UNENFORCED (INC-I-171)** until this redesign ships. |
| `WHITEPAPER.md:600` | States a two-step withdrawal with a 7-day unbonding period and a `ClaimWithdrawal` tx. `ClaimWithdrawal` is rejected as "not supported" (`validation/transaction.rs:158`), `unbonding_period` has zero consensus consumers, and the payout is `lock_until: 0`. **Second, independent drift — needs its own incident.** |
| `crates/core/src/validation/tx_types.rs:614-617` | Comment claims "Output amount <= FIFO net calculation" is "done at node level". It is not. |
| `specs/SPECS.md` | No entry for bond withdrawal / vesting. Add one when the redesign lands. |

---

## Traceability stub

| Requirement | Priority | Test IDs | Architecture section | Implementation module |
|---|---|---|---|---|
| REQ-VEST-001 … 007 | Must | (test-writer) | (architect) | (developer) |
| REQ-VEST-008 | Should | `bins/node/tests/inc_i_171_m5_vesting_build.rs` (9), `crates/mempool/tests/inc_i_171_m5_vesting_admission.rs` (8) | `specs/vesting-penalty-consensus-architecture.md` M5 | `vesting_bound_verdict` @ `crates/mempool/src/vesting_bound.rs`; `Mempool::withdrawal_verdict` @ `crates/mempool/src/pool.rs`; `WithdrawalParity::allow_vesting` @ `bins/node/src/node/production/withdrawal_holdings.rs` |
| REQ-VEST-009, 010 | Should | (test-writer) | (architect) | (developer) |
| REQ-VEST-011 | Could | `crates/mempool/tests/inc_i_171_m5_vesting_admission.rs` (B6-B8), `bins/node/tests/inc_i_171_m5_vesting_build.rs` (A3) | `specs/vesting-penalty-consensus-architecture.md` M5 | `Mempool::slot_watermark` @ `crates/mempool/src/pool.rs` — non-regressing, and the vesting bound's evaluation input at admission (never the call's slot) |
| REQ-VEST-012 | Could | (test-writer) | (architect) | (developer) |
| REQ-VEST-013 … 015 | Won't | N/A (deferred) | N/A | N/A |
