━━━ FINDINGS — 7 total (DECISION:7) ━━━

  [F1] DECISION conf(0.88, converged) — crates/core/src/validation/utxo.rs:222 + transaction/types.rs — replace the hand-maintained 3-type `matches!` with an exhaustive `TxType::allows_empty_io()` (no `_` arm) wrapped by `Transaction::is_zero_flow()`, AH-gated inside the shared validator
  [F2] DECISION conf(0.90, converged) — crates/core/src/network_params/ + validation/types.rs — new dedicated forward-only `inc_i_173_activation_height`, threaded via `.with_*` to BOTH assembly.rs:186 and tx_processing.rs:61; gate lives inside `validate_transaction_with_utxos` (INV-PROD-003 by construction)
  [F3] DECISION conf(0.85, converged) — transaction/types.rs allows_empty_io() arms — exempt set = {Registration, DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer}; `Exit` and `SlashProducer` classified `false` (unauthenticated) with cited reasons + negative tests
  [F4] DECISION conf(0.70, converged) — transaction/core.rs:463 + rpc/methods/transaction.rs:203 + validation_checks.rs:915 — delete `is_state_only()`; route admission/relay on `is_zero_flow()` (non-consensus; deploy separately from the AH)
  [F5] DECISION conf(0.65, converged) — crates/core/src/validation/tx_types.rs:739 — bound the newly-exempt maintainer types' `extra_data.len()` and `MaintainerChangeData.signatures.len()` under the same AH (FM-4/FM-11)
  [F6] DECISION conf(0.62, observed) — rpc/methods/governance.rs + apply_block/governance.rs — publish a chain-derived maintainer-set digest via RPC so node-local trust-root divergence (reorg/snap-sync) is observable (FM-6)
  [F7] DECISION conf(0.68, converged) — crates/core/src/transaction/tests + validation/transaction.rs:39-88 — bind L1/L2 to the exempt authority with a cross-list total test over all 24 `TxType` variants (test, NOT a refactor of L1/L2)

  Speculative: 5 (report-only options — ProtocolActivation mineability, PriceAttestation future-proofing, ValidationContext::new AH-derivation, mempool dual-path collapse, anti-replay chain-binding)
  VERDICT: adopt the radical minimum (F1–F3) as the definite fix; F4–F7 are recommended safe companions; SlashProducer + Exit are EXCLUDED and routed to their own incidents. Activation-height VALUE is not pinned here (mainnet tip unknown) — mechanism + testnet-near value only.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# State-Only Fee-Gate Architecture (INC-I-173)

> **Status: M1 LANDED (F1+F2+F3, commit `32e0a650`). M3a IMPLEMENTED IN THE WORKING TREE,
> PENDING COMMIT (F4+F5+F6+F7 only) — review iteration 1 applied 2026-08-11.
> Options A–E remain PROPOSAL. Option E was built in the full M3 and then WITHDRAWN before
> commit — see its section below.**
>
> One label for one body of work: the six-item milestone is called **M3** and was SUPERSEDED;
> what is in the tree is **M3a**, its four-item reduction. Where this spec says "M3" without
> the suffix it means the superseded six-item milestone.
> This spec synthesizes five independent design evaluations (subtraction, restructure, patterns,
> failures, radical) plus the analyst's redesign analysis. Author: design-synthesizer. Date: 2026-08-10.
> Branch: `bugfix/inc-i-173-state-only-fee-gate`. Incident: INC-I-173 (open).
> Reasoning trace: `docs/.workflow/architecture-reasoning.md`.
>
> **M1 LANDED 2026-08-10** as commit `32e0a650` on branch
> `bugfix/inc-i-173-state-only-fee-gate` (base `b5f68bba`).
> Shipped: `TxType::allows_empty_io()` (`transaction/types.rs`, 24-arm exhaustive match, no `_` arm),
> `Transaction::is_zero_flow()` (`transaction/core.rs`), `inc_i_173_activation_height` on
> `NetworkParams` (`network_params/{mod,defaults,env_loader}.rs`, mainnet env-locked) and on
> `ValidationContext` (`validation/types.rs`, default `u64::MAX`), the AH-gated twin at
> `validation/utxo.rs` with the legacy branch character-identical, and `.with_*` at BOTH
> `production/assembly.rs` and `apply_block/tx_processing.rs`.
> **Pinned activation heights: devnet `0`, testnet `133_000`, mainnet `u64::MAX` (fail-closed; the
> real mainnet value is decided at M4 per the Activation Plan below).**
> Evidence: `docs/.workflow/inc-i-173-M1-dev-green-evidence.txt` (55/55 M1 tests, clippy/fmt clean).
> NOT shipped in M1: F4, F5, F6, F7 (M3) and every Option A–E.
>
> **M3a IMPLEMENTED IN THE WORKING TREE 2026-08-11, PENDING COMMIT** on the same branch
> (base `32e0a650`, which is `HEAD` — nothing of M3a is committed yet). The six-item M3 was
> SUPERSEDED: its rotation journal and its Option E anti-replay were reverted to `32e0a650`
> before review. In the tree:
> * **F5** — `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES = 1024`,
>   `MAX_MAINTAINER_CHANGE_SIGNATURES = MAX_MAINTAINERS` (5),
>   `MAX_MAINTAINER_CHANGE_REASON_BYTES = 256` in `crates/core/src/maintainer/mod.rs`;
>   `validate_maintainer_change_data(tx, ctx)` enforces them at and above the EXISTING
>   `inc_i_173_activation_height`, with the SIZE cap evaluated BEFORE `from_bytes` so
>   bincode never sees an attacker-sized buffer. Below the gate the four historical
>   checks are inert (retroactive vacuity — no block below the gate can carry either
>   type, which is the INC-I-173 bug itself).
> * **F6** — `maintainer_set_digest(set, genesis_hash)` leaf fn
>   (`crates/core/src/maintainer/digest.rs`); published by `getMaintainerSet` on BOTH
>   the on-chain and derived branches (with `genesis_hash` on all three branches, and
>   NO digest on the `none` branch), and logged by the apply path on the fixed grep
>   anchor `MAINTAINER_SET_DIGEST=`. The preimage is
>   `BLAKE3_256("DOLI-MAINTAINER-SET-V1" || genesis_hash || threshold_le_u64 ||
>   members sorted ASCENDING)` — exactly the terms `verify_multisig` consults.
>   **`last_updated` is EXCLUDED** (review iteration 1 / F2): it is node-local, outside
>   the state root, and was measured divergent across 13 testnet nodes at an identical
>   tip holding the same members and threshold, so binding it made the digest report a
>   mismatch for an aligned fleet. It is still published as `last_change_block`.
> * **F4** — `Transaction::is_state_only()` DELETED; both production callers
>   (`rpc/methods/transaction.rs`, `node/validation_checks.rs`) route on `is_zero_flow()`.
>   `.with_inc_i_173_activation_height(...)` wired into the four remaining
>   `ValidationContext` sites (AUDIT-P3-003).
> * **F7** — cross-list total test over all 24 `TxType` variants, probing L1/L2
>   BEHAVIOURALLY. L1/L2 are character-identical.
> * **Option E — built, then WITHDRAWN before commit.** Its binding term was node-local and
>   divergent; a correct one is consensus-visible and owes its own activation height. Not in M3a.
> Evidence: `docs/.workflow/inc-i-173-M3-implementation.md` (the superseded six-item M3) and
> `docs/.workflow/inc-i-173-M3a-implementation.md` (the reduction + review iteration 1).
> Review: `docs/reviews/inc-i-173-M3a-reduction-review.md` — code APPROVED.

## Problem Statement

A transaction whose type the *structural* validator permits to carry zero inputs and zero outputs is
nevertheless rejected by the *UTXO/consensus* validator (`crates/core/src/validation/utxo.rs:222-227`)
for `InsufficientFee`, because that validator carries its own hand-maintained 3-type allow-list
(`Registration | DelegateBond | RevokeDelegation`) that is narrower than every other "state-only"
definition in the codebase. `AddMaintainer` / `RemoveMaintainer` are 0-in/0-out, are admitted to the
mempool and relayed, and have fully implemented apply handlers — but the block builder skips them every
slot (`assembly.rs:235`) and any node would reject a block containing one (`tx_processing.rs:99`, `Full`
mode). The result: the governance transactions INC-I-172 exists to make usable can never be mined.

Root cause is architectural, not local: the knowledge "which tx types carry no UTXO flow" is restated in
**six** independently hand-maintained enumerations (L1 `validation/transaction.rs:39-63`, L2 `:67-88`,
L3 `is_state_only()` `transaction/core.rs:463`, L4 `utxo.rs:222`, L5 `fee_exempt` `utxo.rs:261`, L6
`mempool/pool.rs:567` hand mirror) with nothing binding them. The drift has fired twice — INC-I-057
(`34691e2a`, symptom-patched by enumerating the then-stuck types) and now INC-I-173. A third
enumerate-the-current-victims patch would recur on the next zero-flow type.

The incident fix is the **mineability of maintainer governance transactions**. Adjacent findings
(ProtocolActivation resurrection, SlashProducer/Exit authorization, ClaimReward/ClaimBond hazard,
mempool DoS pricing) are folded in ONLY where the incident fix requires them; the rest are
explicitly-labeled follow-ups.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 u64 compare and 1 exhaustive-match dispatch per non-coinbase tx, replacing an existing 3-arm matches! (observed)
  Memory:   +8 bytes per ValidationContext for one u64 field, ~2 live contexts per block (observed)
  IO:       0 (observed)
  Network:  +small above the gate only, maintainer txs become gossiped block content, 0 below (inferred)
  Disk:     +small above the gate only, maintainer txs persist in blocks at ~300-500 B each, 0 below (inferred)
  Latency:  -small the builder stops re-validating and re-skipping the 3 wedged txs every slot (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: INV-12 answers Q1=YES Q2=YES Q3=NO, so a forward-only activation height is mandatory; the only cheaper path, an ungated 2-token widening of the matches!, re-creates INC-I-057 and forks the ~30 external auto-update producers who cannot be stopped for a synchronized restart.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Replace L1/L2/L4 with one 2-bit `flow_shape()`; delete `is_state_only()` | conf(0.68, measured) | Fix is net-negative in code; the correct predicate already exists — delete four lists, don't add a sixth |
| Restructurer | boundaries | Give the decision ONE owner (exhaustive `match` on `TxType`); derive exemption from **authorization**, exclude `Exit` | conf(0.70, measured) | The fee gate is an accidental authorization boundary; `Exit` has no auth anywhere; nine (not five) drifted lists |
| Pattern Matcher | patterns | Instantiate the existing "INC-gated validator relaxation" idiom verbatim as `inc_i_173_activation_height` | conf(0.70, measured) | DOLI already has a 5×-used gating idiom AND a wildcard-free exhaustive match — reuse+subtract, don't invent |
| Failure Analyst | failures | Exempt ONLY types whose apply handler authenticates the actor | conf(0.68, measured) | The defect is load-bearing: `utxo.rs:222` is the last gate before free, keyless `SlashProducer`/`Exit` — 10 KILL filters |
| Radical Simplifier | minimal | ONE exhaustive `allows_empty_io()` + `is_zero_flow()`, AH-gated at `utxo.rs:222`, `is_state_only()` deleted, `Exit`=false | conf(0.68, measured) | Shape does not imply authorization; the exempt set must be curated, not derived from L1∩L2 |

## Convergence Matrix

```
                                              Sub  Rest  Patt  Fail  Rad   Score
Replace L4 w/ ONE exhaustive TxType match      Y    Y     Y     Y*   Y     5/5  DEFINITE
AH-gate at utxo.rs (new dedicated height)      Y    (Y)   Y!    Y    Y     5/5  DEFINITE (INEVITABLE)
Keep 0-in/0-out conjunct (mint guard)          Y    Y     Y     Y    Y     5/5  DEFINITE
Exclude Exit from exempt set                   -    Y     -     Y!   Y     3/5  DEFINITE (KILL filter C1)
Exclude SlashProducer from exempt set          -    N     -     Y!   N     1/5→ DEFINITE (KILL filter C1 overrides)
Delete is_state_only() (route on shape)        Y    (Y)   Y     -    Y     3/5  RECOMMENDED (non-consensus)
Bound maintainer extra_data/signatures         -    -     -     Y    -     1/5  RECOMMENDED (KILL filter C5)
Observe maintainer-state divergence (digest)   -    -     -     Y    -     1/5  RECOMMENDED (KILL filter C6)
Bind L1/L2 to authority via TEST (not refactor)-    Y     Y     -    Y     3/5  RECOMMENDED
Derive AH inside ValidationContext::new        Y    Y     -     -    -     2/5  OPTION
Collapse mempool dual admission path           Y    Y     -     -    -     2/5  OPTION
Make ProtocolActivation mineable               (Y)  -     Y     ~    Y     3/5  OPTION (adjacent scope)
Anti-replay chain-binding of governance auth   -    -     -     Y    -     1/5  OPTION (follow-up)

Y = proposed;  (Y) = compatible/endorsed;  Y! = proposed as INEVITABLE / KILL filter;
Y* = Failure Analyst frames it as an explicit allow-list via exhaustive match;  ~ = conditional;  N = opposed.
```

**Independence check (core convergence, 5/5):** each evaluator reached "one exhaustive `TxType` match,
AH-gated inside the shared validator" from a *different* evidence base — Subtractionist from caller-count
subtraction, Restructurer from dependency-direction/coupling, Pattern Matcher from analogical idiom
matching, Failure Analyst from adversarial authorization analysis, Radical from first-principles minimum.
This is true convergence, not shared-evidence agreement — confidence boost applies. See reasoning trace
§Deletion Convergence for the per-evaluator evidence sources.

**Contradiction resolved (SlashProducer):** Radical and Restructurer classify `SlashProducer` as
authorized (VDF-verified evidence, "self-authorizing"); the Failure Analyst proves it is NOT — the VDF
input is the producer's *public* key over a *public* hash chain (no secret required) and
`reporter_signature` has **zero verification readers** in `crates/` or `bins/` (grep-confirmed), so an
attacker forges equivocation evidence for free (FM-1, CRITICAL). The Failure Analyst's evidence quality
wins (it traced the actual VDF input and grep'd the readers). Resolution: `SlashProducer` is EXCLUDED
from the exempt set and routed to its own incident. This is the one place the synthesized proposal is
*narrower* than the radical minimum — a strengthening in the safe direction, expressed as a different arm
value in the same exhaustive match.

**Contradiction resolved (params source):** the analyst (§5.1/§7) claims builder and apply use "the same
params source"; the Restructurer claims they do not. Verified directly against code: `assembly.rs:187`
passes `self.params.clone()` (chainspec-override aware), `tx_processing.rs:62` passes
`ConsensusParams::for_network(self.config.network)` (NOT chainspec aware) — the Restructurer is correct.
BUT the ten `.with_*_activation_height(...)` calls are byte-identical between the two sites, and the
INC-I-173 gate reads a **`ValidationContext` field** (`ctx.current_height >= ctx.inc_i_173_activation_height`),
not a `ConsensusParams` field; the fee comparison reads `BASE_FEE`/`FEE_PER_BYTE`, which are consensus
constants. Therefore the divergence is **inert for this fix's gate**. The operative consequence: the new
height must be threaded via `.with_inc_i_173_activation_height(...)` at BOTH `assembly.rs:186` and
`tx_processing.rs:61` (see F2). The general INV-PROD-003 weakness is flagged as a follow-up.

## Definite Changes (High Convergence)

### [F1] ARCHITECTURAL: Replace the `utxo.rs:222` narrow `matches!` with one exhaustive `TxType::allows_empty_io()`, wrapped by `Transaction::is_zero_flow()`, evaluated inside the shared validator behind the activation height

Convergence: Subtractionist P1, Restructurer P1, Pattern Matcher P2, Failure Analyst P1, Radical P1 (5/5, independent).
Evidence: six drifted enumerations (`validation/transaction.rs:39-63`, `:67-88`, `transaction/core.rs:463`,
`utxo.rs:222-227`, `utxo.rs:261-269`, `mempool/pool.rs:567`); the wildcard-free 24-arm exhaustive match
already load-bearing at `validation/transaction.rs:125`; the twin gate idiom already in the SAME function
at `utxo.rs:245` (`ctx.current_height >= ctx.inc_i_096_activation_height`, INV-COMPAT-001 below-gate branch
verified at `utxo.rs:240-244`); `TxType` is not `#[non_exhaustive]` (`from_u32` closes the set at 24
variants) so a `match` with no `_` arm is a compile error on any new variant.
Confidence: conf(0.88, converged).

What changes architecturally: the classification decision gains ONE owner — a `const fn
TxType::allows_empty_io(self) -> bool`, an exhaustive `match` with **no `_` arm**, in the leaf crate
everything already depends on (`crates/core/src/transaction/types.rs`). `Transaction::is_zero_flow()` =
`inputs.is_empty() && outputs.is_empty() && tx_type.allows_empty_io()` makes the 0-in/0-out conjunct
non-bypassable by construction. At `utxo.rs:222`: above the gate → `tx.is_zero_flow()`; below the gate →
the literal 3-type `matches!` retained **character-identical** (frozen consensus history). This eliminates
the drift seam by construction: adding a tx type cannot compile until it is classified. It replaces L4;
it does NOT refactor L1/L2 (see F7) or L5 (a different question — non-native asset flow).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: append TxType::AddMaintainer | TxType::RemoveMaintainer to the existing matches! at utxo.rs:226 and stop (the 34691e2a / INC-I-057 shape, ~2 tokens)
Why this proposal anyway: that 2-token edit is byte-for-byte what shipped as v6.21.7 and produced INC-I-173; a matches! cannot fail the build for the next unclassified 0-flow type, an exhaustive match can
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F2] ARCHITECTURAL: Add a new dedicated forward-only `inc_i_173_activation_height`, threaded through `ValidationContext` to BOTH consensus call sites; the gate is evaluated inside `validate_transaction_with_utxos`

Convergence: Pattern Matcher P1 (INEVITABLE), Radical P1, Subtractionist P1, Failure Analyst P5, Restructurer (endorsed, out-of-lane on value). Analyst §8. (5/5 on mechanism.)
Evidence: INV-12 classification (Q1 YES user-submittable; Q2 YES `SlashProducer`/`PriceAttestation`
producer-reachable; Q3 NO — verdict flips for maintainer types) → activation height REQUIRED. The
existing 5×-instantiated idiom: `NetworkParams` field → `ValidationContext` field defaulting to `u64::MAX`
(fail-closed) → `with_*` builder → in-validator gate → `bins/node/tests/inc_i_NNN_*.rs`. Freshest
forward-only precedent: `maintainer_derivation_activation_height` = mainnet 172_000 / testnet 127_200 /
devnet 0 (`defaults.rs:264,439,587`, committed `b5f68bba`). Both consensus callers verified:
`assembly.rs:235` (builder) and `tx_processing.rs:99` (apply) are the only two production callers of
`validate_transaction_with_utxos`.
Confidence: conf(0.90, converged).

What changes architecturally: a new consensus rule gate that is forward-only and immutable-once-crossed
(INV-PARAMS-001). Because the gate is inside the shared validator, builder/apply parity (INV-PROD-003)
holds *by construction* — both callers get the identical verdict. The height MUST be set via
`.with_inc_i_173_activation_height(...)` at BOTH `assembly.rs:186` and `tx_processing.rs:61`: if the
apply site is forgotten, its field stays `u64::MAX`, apply keeps the legacy branch and REJECTS a
maintainer-bearing block the builder (which has the height set) produced → the producing node forks
itself. If the builder site is forgotten, only liveness is lost (no fork). Both-or-neither is the
constraint; F7's test and Option C (derive-in-`new`) each de-risk it. The mainnet VALUE is deliberately
NOT pinned here (see Activation Plan).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 u64 compare per tx at utxo.rs:222, on a path already running >=1 BLAKE3 hash and >=1 signature verify per input (observed)
  Memory:   +8 bytes per ValidationContext for one u64 field, ~2 live contexts per block (observed)
  IO:       0 (observed)
  Network:  +small above the gate, maintainer txs become gossiped block content, rare and operator-initiated, 0 below (inferred)
  Disk:     +small above the gate, maintainer txs persist in blocks at ~300-500 B each, 0 below (inferred)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: INV-12 is (Q1|Q2) YES + Q3 NO, so an activation height is mandatory to avoid forking the ~30 external auto-update producers who cannot be stopped for a synchronized restart
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F3] ARCHITECTURAL: Curate the exempt set by AUTHORIZATION — `{Registration, DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer}`; classify `Exit` and `SlashProducer` `false` with cited reasons and negative tests

Convergence: Restructurer P2 (exclude `Exit`), Radical P4 (`Exit`=false), Failure Analyst C1/P1 (exclude `Exit` AND `SlashProducer`). Exit exclusion 3/5; SlashProducer exclusion is a KILL filter overriding Radical/Restructurer inclusion.
Evidence: `ExitData` is `{ public_key }` with no signature (`transaction/data.rs:55-58`);
`validate_exit_data` performs no crypto check (`validation/tx_types.rs:11-42`); the apply handler
force-withdraws all bonds of the named pubkey (`tx_processing.rs:256-290`) — an anonymous forced-exit
primitive (FM-3). `SlashProducer.reporter_signature` has zero verification readers (grep in `crates/` +
`bins/`); the VDF is a public hash chain over the producer's *public* key (`validation/producer.rs:12-51`),
so evidence is forgeable for ~800ms of hashing and zero DOLI (FM-1). Both types are 0-in/0-out and in
`is_state_only()` + L1∩L2, so every *shape-derived* predicate would import them. The authorized members
each cite an auth path: `Registration` (VDF, genesis form, `assembly.rs:137-143`), `DelegateBond` /
`RevokeDelegation` (Ed25519 at apply, height-gated since INC-I-078, `tx_processing.rs:437-444`),
`AddMaintainer` / `RemoveMaintainer` (3-of-5 multisig at apply, `governance.rs:36-93`).
Confidence: conf(0.85, converged).

What changes architecturally: the exemption becomes an *authorization* property, not a *shape* property.
This closes the category error the five-list design embodies ("may this wire shape exist?" conflated with
"is this actor authorized?"). The exempt set is a strict superset of today's 3 (REQ-173-002) and excludes
by name the two types whose apply handlers accept an actor identity without verifying it. `Exit` and
`SlashProducer` remain un-mineable — exactly today's behavior — and each is routed to its own incident
where the missing authorization is designed under its own activation height before any exemption.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: NONE-NEEDED
Why this proposal anyway: this is a zero-runtime-cost set-membership decision that removes an unauthenticated forced-exit primitive and a free keyless bond-destruction primitive from an exemption that both shape-derived candidate predicates would have granted
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

### [F4] ARCHITECTURAL: Delete `Transaction::is_state_only()` (L3); route RPC-submit and gossip admission on `is_zero_flow()`

Convergence: Subtractionist P2, Pattern Matcher P5, Radical P2 (3/5); Restructurer P3 compatible ("thin wrapper or deleted").
Evidence: exactly two production callers (`rpc/methods/transaction.rs:203`, `validation_checks.rs:915`;
`pool.rs:735` is a comment). Its doc contract is false on three counts (cites `RequestWithdrawal`, which
has inputs `core.rs:416`; includes `ClaimReward`/`ClaimBond`, which have outputs `core.rs:277-314`).
`ClaimReward`/`ClaimBond` currently route to `add_system_transaction` (no signature/registration check)
at `fee_rate=0` and are re-broadcast unconditionally — a free-relay amplification path that can evict
legitimate 0-fee governance txs via `evict_lowest_fee` (Subtractionist cross-perspective, FM-10).
Confidence: conf(0.70, converged).

What changes architecturally: routing asks the transaction its actual shape instead of consulting a
9-entry list with a false name. `ClaimReward`/`ClaimBond` (they have outputs) stop being system-routed →
the free-relay eviction path closes.

**CORRECTED 2026-08-11 (M3 QA iteration 1, OBS-2) — "stop being system-routed" UNDERSTATES it.**
The phrase reads as "they now pay a fee". They do not. Both types are structurally required to have
**0 inputs and exactly 1 positive output** (`validate_claim_data` / `validate_claim_bond_data`,
`crates/core/src/validation/tx_types.rs:52-140`), so in the normal lane `total_input (0) < total_output`
always holds and the mempool rejects them **unconditionally** — measured, not inferred:
`[MPTX008] insufficient funds: input=0 < output=…`. There is no fee that makes them admissible.

This has **no live impact and is not a regression**, because both types are dead in the tree:
`Transaction::new_claim_reward` / `new_claim_bond` have zero non-test callers, no CLI command and no RPC
method constructs either, and `bins/node/src/node/apply_block/` has **no handler for either type** — so
they were never mineable in the first place. Recorded here so that whoever revives them knows the normal
lane will not accept them as-shaped and that reviving them needs an apply handler plus a shape decision,
not merely a fee.

NOTE: this is **node-local mempool/routing policy, not consensus** —
it takes effect fleet-wide on restart and MUST NOT be folded into the F2 activation height (constraint
C4-pattern). REQ-173-012 (fix the doc) is retired by deletion.

#### F4 routing deltas — the COMPLETE list (corrected 2026-08-11, M3 review iteration 1)

Derived from the two set definitions, not from the earlier reports. OLD lane test = `tx_type ∈
is_state_only` (9 arms, shape ignored). NEW lane test = `0-in ∧ 0-out ∧ tx_type ∈ allows_empty_io`
(`crates/core/src/transaction/types.rs:184-188`).

| Type | Delta | Notes |
|---|---|---|
| `Exit`, `SlashProducer` | LOSE the 0-fee system lane | Silent limbo → loud, type-specific mempool rejection. Improvement. |
| `ClaimReward`, `ClaimBond` | LOSE the system lane | Rejected **unconditionally** by the normal lane (`[MPTX008]`), not "now pay a fee" — see the OBS-2 correction above. Zero production constructors, no `apply_block` handler: never mineable. |
| `PriceAttestation` | LOSES the system lane | No live impact: `oracle_activation_height = u64::MAX` on every network. |
| `DelegateBond`, `RevokeDelegation` | **none** | INERT: `validate_delegate_bond_data` (`crates/core/src/validation/tx_types.rs:849-859`) rejects any input or output, so a valid one is always 0-in/0-out and never changes lane. |
| **`Registration`, 0-in/0-out** | **GAINS the system lane** | **REV-173-M3-004 / F4, added this iteration.** The only delta moving toward MORE free relay. `is_state_only` excluded it; `allows_empty_io` includes it. Reachable ONLY inside the genesis window — `validation/registration.rs:37-63` takes the no-inputs branch under `is_in_genesis`, and post-genesis `:67-71` rejects with "registration must have inputs for bond". Mainnet and testnet are far past theirs. On a fresh chain the mandatory VDF proof bounds amplification. Correct on both branches; no behaviour change recommended. The stale comment in `crates/mempool/src/pool.rs` that denied this — citing the deleted `is_state_only` — was corrected in the same iteration. |
| `RequestWithdrawal` | **none** | **REV-173-M3-007 / F7, removed this iteration.** Previously listed as a delta; it was **never in `is_state_only`**, so it was already on the normal lane at base. Its QA probe rejection (`InvalidWithdrawalRequest`) is real, but it is type-specific validation, not an M3 routing change. |

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -small removes structural validation and hashing for unauthenticated ClaimReward/ClaimBond floods that pass add_system_transaction today (observed)
  Memory:   -small those txs no longer occupy MempoolEntry slots (observed)
  IO:       0 (observed)
  Network:  -moderate removes the unauthenticated re-broadcast amplification path at validation_checks.rs:945-947 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: keep is_state_only() and only remove ClaimReward | ClaimBond from its matches! (2 lines), or rewrite its doc comment per REQ-173-012
Why this proposal anyway: a corrected comment does not retire a name whose contract is false; the wrong phrasing has already been copied into rpc/methods/transaction.rs:193-195, and deletion removes the ambiguity that produced the analyst's vetoed naive swap
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F5] ARCHITECTURAL: Bound the newly-exempt maintainer types' consensus footprint — cap `tx.extra_data.len()` and `MaintainerChangeData.signatures.len()` inside the structural validator, under the same activation height

Convergence: Failure Analyst P2 (KILL filter C5); addresses analyst REQ-173-014 and the Subtractionist/Restructurer/Pattern "spam unmeasured" gap (KILL-filter cost bound on the incident's own newly-mineable types).
Evidence: only `ZKSettle` bounds tx-level `extra_data` today (`tx_types.rs:1062`); `minimum_fee()` prices
output `extra_data` only (`core.rs:691-696`). `MaintainerChangeData.signatures` is an unbounded `Vec` and
`reason` an unbounded `Option<String>` (`maintainer/data.rs:10-17`), unbounded in
`validate_maintainer_change_data` (`tx_types.rs:739-778`). `count_distinct_signers` is O(members ×
signatures) with an Ed25519 verify per matching entry (`maintainer/set.rs:130-149`) → FM-4 (free permanent
storage) + FM-11 (quadratic verify) once these become mineable.
Confidence: conf(0.65, converged).

What changes architecturally: the newly-opened exempt lane gets a consensus-level admission cost bound,
so making maintainer changes mineable does not simultaneously ship a free-permanent-storage + quadratic-
verify DoS. Rides the same AH as F2 (the bound is retroactively vacuous — no block with these types can
exist today, F-P2 kill test). Applies ONLY to the newly-exempted types (not to types with mined history).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -large removes up to ~6200 Ed25519 verifications per hostile maintainer tx per node per replay (measured)
  Memory:   -small bounded Vec deserialization, previously attacker-sized (observed)
  IO:       0 (observed)
  Network:  -small bounded gossip payload for the exempt lane (inferred)
  Disk:     -small bounds permanent chain growth from the free lane to a constant per tx (inferred)
  Latency:  -small removes a multi-hundred-ms stall inside the maintainer_state write lock (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: rely on the mempool max_tx_size = 600 KB policy at policy.rs:29, which is free but is policy not consensus, so a malicious producer building its own block ignores it
Why this proposal anyway: it is the only bound that binds a block producer, and it strictly reduces every resource dimension relative to shipping F1-F3 alone
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F6] ARCHITECTURAL: Make maintainer-state divergence observable — publish a chain-derived maintainer-set digest via RPC (minimum-viable option "c")

Convergence: Failure Analyst P3 option (c) (KILL filter C6); flagged by Restructurer C8 / Radical constraint 6 as an unresolved consequence of the incident fix.
Evidence: `MaintainerState` is node-local, file-backed, and deliberately NOT in the state root (INC-I-172
fork-safety design; `trust_root_wiring.rs:44`, `governance.rs:46,82`). `rollback.rs` has **zero**
`maintainer` references, so a reorged-out maintainer change is not undone; snap-sync never replays
governance handlers; the apply handler is non-fatal `warn!` (`governance.rs:57,89`) — three silent paths
to a divergent trust root once maintainer txs are mineable (FM-6).
Confidence: conf(0.62, observed).

What changes architecturally: this surfaces the tension between INC-I-172 (keep the trust root out of the
state root for fork safety) and INC-I-173 (let chain data mutate it). Option (c) — expose a chain-derived
digest of the maintainer set via `getMaintainerSet` so operators can compare nodes — is the minimum
obligation and adds no consensus surface and no new dependency edge from `crates/core/validation` toward
node-local state (constraint C-coupling honored). Reorg-aware undo (option a) and deterministic
re-derivation (option b) are heavier and deferred (they touch `rollback.rs`, a high-blast-radius file).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: change nothing and rely on the INC-I-172 note that the set is node-local by design
Why this proposal anyway: that note keeps the set fork-safe only while nothing on-chain can mutate it; F1-F3 break that premise, so without at least option (c) the fleet can disagree about which binaries are trusted while agreeing on every block
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F7] ARCHITECTURAL: Bind L1/L2 to the exempt authority with a cross-list total test over all 24 `TxType` variants — a test, NOT a refactor of L1/L2

Convergence: Radical P3, Restructurer P1 (reduced scope), Pattern Matcher P4 (total-coverage test) (3/5).
Evidence: L1 (16 terms) ≠ L2 (15 terms); `L1 \ L2 = {Coinbase, ClaimReward, ClaimBond}`
(`validation/transaction.rs:40,42,43` present, absent `:67-88`) — L1/L2 answer input-shape and
output-shape separately and must stay distinct. Refactoring the two 16-/15-term negated chains touches
structural validation on every tx on every path (including historical re-validation) — a non-bit-identical
change there is a consensus change on its own (Pattern P2 risk, Subtractionist P1 risk). The safe binding
is a one-way implication test: `allows_empty_io(t) ⇒ t ∈ L1 ∧ t ∈ L2` for all 24 variants.
Confidence: conf(0.68, converged).

What changes architecturally: the invariant "every exempt type is structurally permitted to be 0-in/0-out"
becomes machine-checked without perturbing the L1/L2 consensus expressions. The exhaustive match (F1)
catches an *unclassified* new type at compile time; this test catches a type classified `true` while
L1/L2 still reject its shape (which would silently produce an inert fix). L1/L2 are left character-identical.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: rely on the exhaustive match alone (F1), which already fails the build for an unclassified new variant
Why this proposal anyway: the exhaustive match cannot catch a type classified true while L1/L2 still reject its shape, which combination silently produces an inert fix; the cross-list test is the only cheap guard for it
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

Each option is below the definite/recommended bar for THIS incident. They are genuine choices, not
automated decisions. Low-evidence items are tagged.

**Option A — Also make `ProtocolActivation` mineable (classify `true` + adopt F4 routing).**
Evidence: `ProtocolActivation` is 0-in/0-out (`core.rs:858-866`), soft-multisig-authorized at apply
(`governance.rs:101-151`, same class as maintainer types), but dead at an *earlier* stage — mempool
`FeeTooLow` (`pool.rs:575`) because `is_state_only()` excludes it. It needs BOTH the exhaustive-match
`true` arm AND the F4 routing change. Analyst REQ-173-009 (Should). Complexity: +0 consensus (rides F2's
AH), depends on F4. Failure-mode filter: FM-5 replay applies with greatest force (it can schedule a
consensus change) — pairs with Option E. conf(0.60, observed). Scope note: adjacent, not incident-required.

**Option B — Also classify `PriceAttestation` `true` (future-proof REQ-173-013).**
Evidence: `PriceAttestation` is 0-in/0-out and HARD-authorized (payload signature + attester ∈
`active_producers`, rejecting — `tx_types.rs:869`), but unreachable today (`oracle_activation_height =
u64::MAX` on all networks). Classifying it `true` now prevents INC-I-173 from re-firing for
`PriceAttestation` when oracle is later activated. Zero marginal risk (unreachable). conf(0.60, observed).
Scope note: `oracle_activation_height` is out of scope (HC-6, Won't) — this only pre-classifies.

**Option C — Derive activation heights inside `ValidationContext::new` (delete the hand-copied `.with_*` chains).**
Evidence: Subtractionist P6 (conf 0.63), Restructurer P4 (conf 0.65) (2/5). Five sites hand-thread the
`.with_*_activation_height` chain; `validation_checks.rs:103` already drifted (omits
`.with_sig_verification_height`). Deriving from the process-wide `NetworkParams` `OnceLock` makes a missed
site unrepresentable — directly de-risking the F2 both-sites constraint. Cost: a mechanical refactor of
two consensus-critical files (`assembly.rs`, `tx_processing.rs`); must NOT touch the `ConsensusParams`
source divergence (see params contradiction). conf(0.65, converged). Scope note: bigger blast radius; the
F7 parity test covers the immediate risk at lower cost.

**Option D — Collapse the mempool dual admission path (delete `add_system_transaction`).** *[low-evidence]*
Evidence: Subtractionist P5 (conf 0.52), Restructurer P3 (conf 0.60) (2/5). `add_system_transaction`
(`pool.rs:706-789`) duplicates the head of `add_transaction` and lets the caller choose to skip fee +
signature + spend-tracking. Blocked by an undocumented mempool/utxo lock-order question (Subtractionist
Unknown #3). conf(0.55, observed). Scope note: non-consensus cleanup, own change.

**Option E — Bind governance authorizations to chain + state (anti-replay). BUILT, THEN WITHDRAWN.
Still a PROPOSAL; not shipped.**
Originally filed *[low-evidence]* from Failure Analyst P4 (conf 0.55); promoted to a DECISION by the
M1 security audit, which raised the same defect independently as **AUDIT-P1-004** (3/5 converged,
conf 0.85) and **AUDIT-P2-002**.

The defect is real and stands. The current `signing_message` is `"add:<hex>"` / `"remove:<hex>"` with
no nonce, height, chain id, expiry or set version — a permanent BEARER TOKEN. It also collides
byte-for-byte with the release-signing family, so one `release sign` invocation with attacker-chosen
arguments mints a maintainer-seat authorization (AUDIT-P0-011). **None of that is fixed by M3a.**

**Why the built version was withdrawn.** The shipped-then-reverted format bound the authorization to
`MaintainerSet::last_updated` — a NODE-LOCAL value outside the state root (INC-I-172). The M3
security audit measured it divergent across the live testnet fleet at an identical tip: RPC 8512
reported `last_change_block = 88289` while twelve peers reported `1`. A binding that differs per host
makes one authorization valid on some nodes and silently skipped on others, which is a worse failure
than the replay it was meant to stop — it is INC-I-173's own silent-limbo class, relocated.

**What a correct version needs.** The binding term must be CONSENSUS-VISIBLE, so every node computes
the same value at the same height. That makes it a consensus change, and therefore it owes its OWN
activation height — it cannot ride `inc_i_173_activation_height`, which is already committed and
already crossed on testnet. It also cannot be justified by the retroactive-vacuity argument used for
F5, because that argument only covers predicates that are inert below the gate.

**Disposition.** Tracked in its own incident. `reason`-in-the-signed-message (AUDIT-P2-002) and the
release-signing domain collision (AUDIT-P0-011) travel with it: both are properties of the same
message format, and splitting them would ship a second partial format.

## Constraints (from Failure Analyst — KILL filters, plus Restructurer/Pattern/Radical filters)

Any chosen path MUST satisfy these. C1–C4 are hard kills; a proposal that violates one is dead.

- **C1 (actor authentication).** The exempt set MUST NOT include a type whose apply handler accepts an
  actor identity without verifying a signature / binding proof. Excludes `SlashProducer` and `Exit` today.
  Honored by F3. Overturn only by shipping their auth under their own heights (follow-up incidents).
- **C2 (the conjunct).** `inputs.is_empty() && outputs.is_empty()` must remain a conjunct — it is what
  stops `ClaimReward`/`ClaimBond` (0-in, 1-value-output) from riding a widened type list into a mint.
  Honored by F1 (`is_zero_flow`).
- **C3 (genesis).** The predicate must be a strict superset of `{Registration, DelegateBond,
  RevokeDelegation}`; `is_state_only()` omits `Registration` and would break genesis + fresh sync.
  Honored by F3.
- **C4 (one place, block's height).** The gate must be evaluated inside `validate_transaction_with_utxos`
  from the block's height in `ValidationContext`, never `chain_state.best_height`, never at a call site.
  Honored by F1/F2.
- **C5 (cost bound).** Any newly-exempt type needs a consensus bound on `extra_data.len()` and attacker-
  sized `Vec`s. Addressed by F5.
- **C6 (state divergence).** A newly-exempt type mutating out-of-state-root state (`MaintainerState`) must
  declare reorg / snap-sync behavior. Addressed by F6 (minimum option c) for OBSERVABILITY only — the
  digest makes divergence visible, it does not repair it. The reorg gap itself is INC-I-174.
- **C7 (replay).** 0-input txs have no replay protection and authorizations have no expiry. **NOT addressed
  by M3a — this is the flagged open risk, now realised.** Option E was the intended answer and was
  withdrawn (see its section); the bearer-token `signing_message` is unchanged on this branch. M3a's
  scope is the DoS bound (F5) and observability (F6), neither of which is a replay defence.
- **C8 (test-mode Full).** Below-the-gate bit-identity tests (REQ-173-003) MUST run in `ValidationMode::Full`
  — `Replay` silently swallows validation errors (INC-I-064). Folded into the test plan.
- **C9 (sequencing).** Pinning the height is necessary, not sufficient: no newly-exempt tx may sit in any
  mempool when the height passes; purge the 3 stuck testnet txs before the testnet height. Folded into
  Migration Path.
- **C10 (no-new-parallelism).** Do not exempt any type feeding the unbounded thread-per-tx VDF pre-pass
  (`validation/block.rs:145-180`) without first bounding it. Satisfied by excluding `SlashProducer`;
  binding for the SlashProducer follow-up incident.
- **C-coupling (no upward dependency).** No new dependency edge from `crates/core/validation` toward
  node-local maintainer state. Honored — F6 keeps the digest at the RPC/apply layer.

## Architecture Maps

### Current Architecture

```
CLI/RPC ─► sendTransaction ─┬─(is_state_only? L3)─► add_system_transaction ─► structural only ─► POOL(fee_rate=0)
                            └─(else)──────────────► add_transaction ─► UTXO+fee ─► POOL or FeeTooLow
gossip  ─► handle_new_transaction ─► same fork (L3) ─► POOL ─► rebroadcast (only on Ok)
POOL ─► assembly.rs:235  validate_transaction_with_utxos ─► [L4 fee gate: 3 types] ─► block   (maintainer txs SKIPPED)
block ─► tx_processing.rs:99 validate_transaction_with_utxos ─► [SAME L4 gate] ─► apply or REJECT WHOLE BLOCK
apply ─► apply_block/governance.rs (maintainer/protocol handlers — never executed on any network)

Six drifted enumerations of "carries no UTXO flow": L1, L2, L3, L4, L5, L6. Nothing binds them.
New TxType compiles cleanly while being wrong in up to six places.  (INC-I-057, then INC-I-173.)
```

### Proposed Architecture (Definite + Recommended)

```
                                TxType::allows_empty_io()   ── exhaustive 24-arm match, NO `_` arm
                                          │                    (authorization-curated true-set)
                        Transaction::is_zero_flow()  = inputs.is_empty() && outputs.is_empty() && allows_empty_io()
                                          │
        ┌─────────────────────────────────┼──────────────────────────────────┐
   utxo.rs:222 (consensus, AH-gated)   rpc/gossip routing (F4)         L1/L2 unchanged, bound by TEST (F7)
   above: is_zero_flow()               on is_zero_flow()               L5 unchanged (different question)
   below: FROZEN literal 3-type matches!  (consensus history, character-identical, INV-COMPAT-001)

   inc_i_173_activation_height:  NetworkParams ─► ValidationContext (default u64::MAX) ─► with_* ─► set at BOTH
                                 assembly.rs:186 AND tx_processing.rs:61   (parity by construction)
   Exempt set (above gate): Registration, DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer
   Excluded (false): Exit, SlashProducer  ──► own incidents.   ClaimReward/ClaimBond: false (have outputs).
   New TxType ─► BUILD FAILS until classified.   F5 bounds maintainer footprint.   F6 exposes trust-root digest.
```

## Migration Path

Testnet-first. The change is rolling-restart safe below the gate and a fleet-upgrade deadline at the gate.

> ### ⚠ STATUS 2026-08-11 — THE TESTNET GATE IS ALREADY CROSSED (REV-173-M3-001 / F1)
>
> **Measured: live testnet `bestHeight` = 134,159 on v6.24.1, agreed across RPC `8500`/`8501`/`8502`,
> against `inc_i_173_activation_height = 133_000` (`crates/core/src/network_params/defaults.rs:480`).**
> Crossed by ~1,159 blocks and still climbing at ~10 s/block. Mainnet is `u64::MAX` and devnet is `0`
> — **both unaffected; this is testnet-only.** The sentence above ("rolling-restart safe below the
> gate") is still true as written, but testnet is no longer below the gate. Concretely:
>
> - **The staged-activation safety property is GONE on testnet until M2 re-pins above the then-current
>   tip.** On testnet the M1/M3 consensus wiring becomes active the moment the binary lands, not at a
>   future scheduled height.
> - **Deploying to testnet therefore requires a SYNCHRONIZED stop-all/start-all, not a rolling
>   restart** (INV-8 / INC-I-062): a new-binary producer could immediately mine a 0-in/0-out
>   `AddMaintainer` that old-binary nodes reject, forking the fleet mid-restart.
> - **History is NOT invalidated.** Above the gate the new predicate is strictly MORE permissive — the
>   same three types plus `AddMaintainer` / `RemoveMaintainer` — so no block valid under the old rules
>   becomes invalid, and the running v6.24.1 binary has no knowledge of this height at all.
> - **M2 MUST re-pin the testnet height above the tip at deploy time and MUST re-verify the tip
>   immediately before pinning** — the same discipline that produced the `130_400 → 133_000` re-pin,
>   for the same reason (a 2.17 h lead decayed to ~20 min before deploy). Re-pinning is M2's decision;
>   no activation-height VALUE was changed when this note was written.

1. **M1 — land the consensus core (F1+F2+F3) + tests, testnet-value AH.** Ship binaries to the local
   testnet with `inc_i_173_activation_height` set to a NEAR-FUTURE testnet height (idiom: devnet=0,
   testnet-near, mainnet-far — copy `maintainer_derivation_activation_height` shape). Below the gate,
   behavior is bit-identical (rolling-restart safe).
2. **BRIDGE: purge the 3 stuck testnet governance txs before the testnet height crosses.** These are the
   INC-I-172 functional-test txs (remove / duplicate-signer remove / re-add). Per C9/FM-9 they must NOT be
   auto-mined at the boundary in a builder-nondeterministic order. This is a transitional operational step
   (mempool purge or let `max_age=14d` expire them, verified) — it is removed from concern once the
   maintainer flow is re-exercised cleanly above the gate. Do NOT auto-resubmit them.
3. **M2 — exercise REQ-173-008 above the testnet gate.** A `RemoveMaintainer` then `AddMaintainer` is
   mined AND applied; `[MAINTAINER]` apply log observed; `getMaintainerSet` reflects the change across a
   block boundary and holds at N+5. This closes the INC-I-172 verification gap.
4. **M3 — land routing/footprint/observability (F4+F5+F6).** F4 is non-consensus (fleet-wide on restart,
   separate deploy decision from the AH). F5 rides the AH. F6 is additive RPC.
5. **M4 — mainnet rollout (sequencing, C9/FM-7).** (a) Re-verify the mainnet tip; pin
   `inc_i_173_activation_height` strictly above `mainnet tip + external auto-update window` (~8,680 blocks
   ≈ 24.1h lead is the documented convention); add a re-pin-history comment (C6-pattern). (b) Ship
   binaries to ALL nodes incl. the ~30 external auto-update producers. (c) VERIFY fleet version coverage
   (one RPC version sweep). (d) ONLY THEN submit the first mainnet governance tx. No genesis reset, no
   version bumps, no HardForkSchedule entry.

The 3 stuck txs are the only pre-existing state to reconcile; there is no on-chain migration and no state
root change below the gate.

## Complexity Comparison

| Metric | Current | Radical Minimum (F1–F3) | Proposed (F1–F7) |
|--------|---------|--------------------------|-------------------|
| Enumerations answering "carries no UTXO flow?" | 6 (L1,L2,L3,L4,L5,L6) | 1 authority + 1 frozen legacy branch (+L1,L2,L5 unchanged) | same as radical (L3 deleted → 5 live) |
| Compile-time drift protection | none | full (exhaustive `match`, no `_`) | full + cross-list test (F7) |
| New modules | — | 0 | 0 |
| New abstractions (traits/enums/config tables) | — | 0 | 0 |
| New methods on `TxType`/`Transaction` | 1 (`is_state_only`) | +2 (`allows_empty_io`, `is_zero_flow`), −1 (`is_state_only` deleted) | same |
| `ValidationContext`/`NetworkParams` fields | 20 heights | +1 (`inc_i_173_activation_height`) | +1 |
| Activation heights added | — | 1 | 1 |
| Non-test LOC (core) | — | ≈ +95 / −20, 6 files (45 = measured AH-plumbing floor from `9efad2cb`) | ≈ +140 / −30 (F4–F7 add routing/bound/digest/test) |
| Defects closed by construction | — | INC-I-173 (maintainer mineability) + free-relay eviction path | + footprint DoS + observability of trust-root divergence |
| Types routed to own incident | — | `SlashProducer`, `Exit` | same |

## Milestones

The fix touches ≥4 modules (`crates/core/transaction`, `crates/core/validation`, `crates/core/network_params`,
`bins/node/{production,apply_block,validation_checks}`, `crates/rpc`, `crates/mempool`), so milestones are
defined:

- **M1 (consensus core, DEFINITE):** F1 + F2 + F3 + REQ-173-002/003/004/006 tests. Gate: `cargo build &&
  clippy -D warnings && fmt --check && cargo test -p doli-core`. Testnet-near AH. Rolling-restart safe
  below the gate.
- **M2 (end-to-end verification):** REQ-173-008 above the testnet gate (closes the INC-I-172 gap). Depends
  on M1 + the BRIDGE purge.
- **M3 (hardening + hygiene, RECOMMENDED):** F4 (routing, non-consensus) + F5 (footprint bound, rides AH) +
  F6 (digest RPC) + F7 (cross-list test).
- **M4 (mainnet rollout):** sequencing C9/FM-7; mainnet AH value decided at release after re-checking tip.

## Consensus Classification (INV-12, MANDATORY)

- **Q1 — user-submittable tx reaches this path? YES.** `AddMaintainer`/`RemoveMaintainer` via RPC
  `submitMaintainerChange` (`governance.rs:241/243`); (`ProtocolActivation` via CLI if Option A taken).
- **Q2 — producer-action / attestation pattern reaches it? YES.** `SlashProducer` is node-generated on
  equivocation (`rewards.rs:474-502`); `PriceAttestation` would reach it if oracle is ever activated.
  (Both are classified `false`/excluded here, but the classification path is producer-reachable.)
- **Q3 — bit-identical for all reachable inputs? NO.** Above the gate, a block containing a 0-fee
  `AddMaintainer` flips from REJECT to ACCEPT at `tx_processing.rs:99`.
- **(Q1 | Q2) YES + Q3 NO → ACTIVATION HEIGHT REQUIRED.** Verified against the proposal's actual affected
  set (maintainer types); a new, dedicated, never-before-used field `inc_i_173_activation_height` in
  `crates/core/src/network_params/` — not reused, not bundled onto `maintainer_derivation_activation_height`
  (INV-PARAMS-001 / INC-I-054).

**Deploy question 1 (consensus RULE change?) — YES →** new dedicated forward-only activation height.
**Deploy question 2 (block CONTENT change?) — YES, above the gate →** the height converts a synchronized-
deploy requirement into a fleet-upgrade deadline. Below the gate: bit-identical, rolling-restart safe.
The height must clear the ~30 external auto-update producers' window. **No version bumps**
(`CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION` untouched — the
`EpochState` format is unchanged and the peer handshake is unaffected). **No genesis reset. No
HardForkSchedule entry** (`current_fork_id(u64::MAX)` would make it active immediately).

### Activation Plan (mechanism + testnet value; NO mainnet value invented)

Follow the established idiom (Pattern Matcher, `utxo.rs:245` twin already in the same function):
1. `inc_i_173_activation_height: u64` field on `NetworkParams` (`network_params/mod.rs`).
2. Per-network literal in `network_params/defaults.rs` with a mandatory re-pin-history comment
   (date, tip at pin time, lead-time in blocks and hours): **devnet = 0**, **testnet = a near-future
   testnet height** (so REQ-173-008 runs this cycle), **mainnet = decided at release**.
   *M1 PINNED (2026-08-10):* devnet `0`; testnet `133_000` — re-pin history `u64::MAX → 130_400`
   then `130_400 → 133_000` (QA ISSUE-001: the testnet kept producing during M1, so the initial
   `2.17 h` lead decayed to ~120 blocks ≈ 20 min before the change was ever deployed; a height
   crossed by an un-upgraded fleet nullifies the mixed-fleet purpose of the gate and freezes a
   wrong value permanently, INC-I-054). **`130_291` below is the tip AT RE-PIN TIME (2026-08-10) — a
   historical record, NOT the current tip. As of 2026-08-11 the live testnet tip is `134_159` and the
   gate is CROSSED; see the STATUS box under "Migration Path".** Live testnet tip at re-pin time
   `130_291`, measured rate
   `10.00 s/block` (1000-block sample, heights `129_286 → 130_286`, timestamps
   `1786372169 → 1786382169`), lead `2_709 blocks ≈ 7.53 h` — enough to cover the remainder of M1
   (review + security audit + commit) plus the M2 testnet deploy — and strictly above the
   INC-I-172 testnet derivation gate `127_200`; mainnet `u64::MAX` (fail-closed, M4 decides).
3. `ValidationContext` field defaulting to `u64::MAX` (fail-closed) + `with_inc_i_173_activation_height()`
   builder (`validation/types.rs`).
4. Gate inside `validate_transaction_with_utxos` (`utxo.rs:222`); set the height via `.with_*` at BOTH
   `assembly.rs:186` and `tx_processing.rs:61`.
5. Integration test `bins/node/tests/inc_i_173_*.rs`.

**Mainnet value decision criteria (decided at release, NOT here):** re-check the live mainnet tip; pin
`inc_i_173_activation_height` strictly greater than `tip + external-auto-update-window`. Do NOT read
`amm`/`inc_i_092`/`inc_i_096` mainnet literals as the pinning precedent — they are literal `0` on Mainnet
in apparent contradiction with their own re-pin comments (Pattern Matcher / Subtractionist HIGH signal,
unresolved — see Follow-ups). Read `maintainer_derivation_activation_height` (172_000 / 127_200 / 0) as
the precedent.

## Test Plan Skeleton (REQ-173-* → planned tests)

| REQ | Priority | Planned test (mode) |
|-----|----------|---------------------|
| REQ-173-001 | Must | `utxo.rs` contains no literal state-only list above the gate; the exemption derives from `is_zero_flow()` |
| REQ-173-002 | Must | build the exact genesis `Registration` tx (`assembly.rs:137`), assert it validates above AND below the gate |
| REQ-173-003 | Must (C8) | for all 24 `TxType` variants, assert accept/reject at `height = AH-1` equals pre-change verdict — **`ValidationMode::Full`** |
| REQ-173-004 | Must | `ClaimReward`/`ClaimBond` with a non-zero output REJECTED at every height; property test: no tx with non-empty outputs is ever exempt |
| REQ-173-005 | Must | new field on all 3 networks; no crossed height moved; height threaded to `ValidationContext`; set at BOTH `assembly.rs` and `tx_processing.rs` |
| REQ-173-006 | Must | drive the same tx + height through builder-context and apply-context; assert identical verdicts (parity) |
| REQ-173-007 | Must | diff asserts no version bump; genesis hash unchanged |
| REQ-173-008 | Must (M2) | e2e testnet: `RemoveMaintainer`→`AddMaintainer` mined + applied; `[MAINTAINER]` log; `getMaintainerSet` holds N→N+5 |
| REQ-173-003b | Must (F3) | negative tests: `Exit` and `SlashProducer` REJECTED at every height (above and below) |
| REQ-173-011 | Should (F1/F7) | exhaustive `match` fails build on a synthetic new variant; cross-list test asserts `allows_empty_io(t) ⇒ t∈L1∧t∈L2` for all 24 |
| REQ-173-014 | Should (F5) | maintainer tx with oversized `extra_data`/`signatures` REJECTED above the gate |

## Follow-up Recommendations (NOT changes in this spec)

1. **NEW INCIDENT — `SlashProducer` never mineable AND unauthenticated (FM-1/FM-2).** `reporter_signature`
   has zero verification readers; the VDF is publicly computable; the block pre-pass spawns unbounded
   threads-per-VDF (`block.rs:145-180`). Needs `reporter_signature` verification + a thread-pool bound
   (C10) + its own activation height BEFORE any exemption. Do not "make it mineable" as a liveness fix.
2. **NEW INCIDENT — `Exit` unauthenticated forced-exit primitive (FM-3).** `ExitData` has no auth. Needs an
   ownership-proof mechanism under its own height, OR tombstone it (`ClaimWithdrawal` pattern) if no shipped
   tool builds it (verify chain history first).
3. **NEW INCIDENT(s) — two live pre-existing DoS/censorship vectors independent of INC-I-173:** (a) ~800ms
   synchronous VDF hashing per gossiped forged `SlashProducer` while holding the mempool write lock
   (`validation_checks.rs:913-928`); (b) `evict_lowest_fee` preferentially evicts `fee_rate=0` entries —
   the node's own governance/slash txs — cheap censorship of the system-tx lane.
4. **HIGH — possible live consensus drift:** `amm`/`inc_i_092`/`inc_i_096` activation heights are literal
   `0` on Mainnet (`defaults.rs:187,229,243`) contradicting their re-pin comments (`h=375_640`) and
   CLAUDE.md ("Oracle + DeFi gates are `u64::MAX`"). Needs a correctness owner to query live mainnet.
5. **Latent same-class drift:** L5 `fee_exempt` (`utxo.rs:261`) is hand-mirrored in `pool.rs:567`
   (`amm_gated`) with a comment admitting the manual mirror — INC-I-173's defect class, one module over.
6. **Cleanup:** `defi_activation_height` is dead end-to-end (only its own setter reads it; Subtractionist
   P4, conf 0.70) — rolling-restart-safe deletion, own small change.
7. **INV-PROD-003 weakness:** builder/apply use different `ConsensusParams` sources
   (`self.params.clone()` vs `ConsensusParams::for_network`). Inert for this gate; owner of INV-PROD-003
   should reconcile.
8. **Doc drift:** `is_state_only()` (`core.rs:457-462`) and `rpc/methods/transaction.rs:193-195` falsely
   claim state-only txs are "spam-protected by requiring a registered producer bond" — no such check
   exists. Retired by F4 if taken.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:             5/5
Deletion convergence items:       3 (core match+AH+conjunct, 5/5; is_state_only delete 3/5; Exit exclude 3/5)
Restructuring convergence:        2 (one-owner exhaustive match; authorization-based exempt set)
Addition options presented:       5 (ProtocolActivation, PriceAttestation, ValidationContext::new, mempool collapse, anti-replay)
Failure modes identified:         12 (FM-1..FM-12) + 10 KILL filters (C1..C10)
Failure modes applied as filters: 10/10 KILL filters applied to every definite/recommended change
Radical floor gap:               current 6 enumerations -> radical minimum 1 authority + 1 frozen branch -> proposed = radical + 4 safe companions
Contradictions found:             2 (SlashProducer auth; builder/apply params source)
Contradictions resolved:          2/2 (SlashProducer excluded on evidence quality; params divergence verified inert for this gate)
Evidence independence verified:   YES (5 distinct evidence bases for the core convergence)
SSF: radical minimum == final proposal? YES (F1-F3 ARE the radical minimum; SlashProducer moved to false in the safe direction; F4-F7 are appendix-grade companions)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
