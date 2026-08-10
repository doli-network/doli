# Redesign Analysis — INC-I-173: state-only txs cannot be mined (fee/balance gate)

> **Status: PROPOSAL / SCOPING ONLY.** No code, no chain state, and no configuration was modified.
> All testnet interaction was read-only (RPC GET-style calls on 127.0.0.1 + log grep).
> Analyst: OMEGA Analyst. Date: 2026-08-10. Branch: `bugfix/inc-i-172-maintainer-trust-root`.

---

## 1. Problem statement

A transaction whose type is permitted by the *structural* validator to carry zero inputs and zero
outputs is nevertheless rejected by the *UTXO* validator for `InsufficientFee`, because the UTXO
validator carries its own hand-maintained 3-entry allow-list that is narrower than every other
definition of "state-only" in the codebase.

Consequence: **6 of the 24 live transaction types can never be included in a block.** Among them are
the two governance types that INC-I-172 exists to make usable (`AddMaintainer` / `RemoveMaintainer`),
the on-chain protocol-upgrade type (`ProtocolActivation`), and the equivocation-punishment type
(`SlashProducer`). The mempool admits them, the network relays them (for most), and `apply_block`
has fully implemented handlers for them — but the block builder skips them every slot and a
validating node would reject any block that contained one.

Live evidence (read-only, this session, local testnet n1 RPC 8501):

```
{"result":{"maxCount":10000,"maxSize":10485760,"minFeeRate":0,"totalSize":1337,"txCount":3}}

WARN doli_node::node::production::assembly: Skipping mempool tx 810de622… — UTXO validation failed:
  insufficient fee: got 0, minimum 1 (base 1 + 0 bytes * 1/byte)
```

The three stuck txs are the INC-I-172 governance functional test (remove / duplicate-signer remove /
re-add). They have been re-skipped on every block since 13:48.

---

## 2. Re-verified root cause

### 2.1 The two divergent definitions — CONFIRMED

**Broad definition — `crates/core/src/transaction/core.rs:463` `Transaction::is_state_only()`** —
9 types:

```rust
TxType::Exit | ClaimReward | ClaimBond | SlashProducer | DelegateBond
    | RevokeDelegation | AddMaintainer | RemoveMaintainer | PriceAttestation
```

**Narrow definition — `crates/core/src/validation/utxo.rs:222-227`** — 3 types, inside
`validate_transaction_with_utxos`:

```rust
let is_state_only_tx = tx.inputs.is_empty()
    && tx.outputs.is_empty()
    && matches!(tx.tx_type,
        TxType::Registration | TxType::DelegateBond | TxType::RevokeDelegation);
if !is_state_only_tx { /* balance check :250, fee check :270-283 */ }
```

`AddMaintainer` / `RemoveMaintainer` are 0-input/0-output (`core.rs:748-754`, `core.rs:772-778` —
`inputs: Vec::new(), outputs: Vec::new()`), are in the broad set, are NOT in the narrow set, and
therefore reach the fee check at `utxo.rs:270-283`, where `actual_fee = 0 - 0 = 0` and
`min_fee = BASE_FEE = 1` (`core.rs:691-696`; a 0-output tx has zero per-byte component) →
`InsufficientFee`. **Prior diagnosis confirmed exactly as written.**

### 2.2 Corrections and additions to the prior diagnosis

The prior diagnosis is correct but **incomplete in four material ways**. Each of these changes the
shape of the fix.

**⚠ CORRECTION 1 — the naive SSF candidate is unsafe: it would break genesis Registration.**
The handoff proposes replacing the `matches!` with `tx.is_state_only()`. `Transaction::is_state_only()`
does **NOT** include `TxType::Registration`. Genesis registrations are built 0-input/0-output at
`bins/node/src/node/production/assembly.rs:137-143` and are exempted today *only* because
`Registration` sits in the narrow list. A straight swap to `is_state_only()` would make every
genesis-era Registration tx fail the fee check — breaking genesis block production on a fresh chain
and breaking full (non-`Replay`) re-validation of genesis-era blocks during a reorg. The replacement
predicate must be a strict **superset** of today's narrow list, not `is_state_only()` alone.

**⚠ CORRECTION 2 — the naive SSF candidate would not fix `ProtocolActivation` either.**
`ProtocolActivation` (TxType 15) is 0-input/0-output (`core.rs:858-866`) and is in **neither** list.
It is reachable from the shipped CLI (`bins/cli/src/cmd_governance.rs:215` → `sendTransaction`).
Because `is_state_only()` returns false for it, RPC `sendTransaction` routes it to
`add_transaction` (`crates/rpc/src/methods/transaction.rs:203-214`), where the mempool fee floor at
`crates/mempool/src/pool.rs:572-577` rejects it with `FeeTooLow` — **it never even reaches the
mempool**, and consequently is never gossiped. The on-chain protocol-activation mechanism is dead at
a strictly earlier stage than the maintainer txs. Widening only `utxo.rs` leaves it dead.

**⚠ CORRECTION 3 — `ClaimReward` / `ClaimBond` fail on the BALANCE check, not the fee check, and are
a live conservation hazard for the fix.** Both are constructed 0-input with **one non-zero output**
(`core.rs:281-287`, `core.rs:307-313`). They hit `total_input (0) < total_output (amount)` at
`utxo.rs:250-255` → `InsufficientFunds`. The safety guard that stops this from being a coin-minting
hole today is the `tx.inputs.is_empty() && tx.outputs.is_empty()` **conjunct**, not the type list.
Any redesign MUST preserve that conjunct. Dropping it while widening the type list would fee- and
balance-exempt a 0-input tx that mints an arbitrary output → unbounded inflation. This is the
INV-VALIDATION conservation risk the handoff asked about, and it is real.

**⚠ CORRECTION 4 — `SlashProducer` is reachable and is the highest-severity casualty.** The handoff
lists it as a hypothesis. Verified: the node itself builds and submits it on equivocation detection —
`bins/node/src/node/rewards.rs:474-502` calls `mempool.add_system_transaction(slash_tx, …)` and then
`network.broadcast_transaction(slash_tx)`. It is 0-in/0-out (`core.rs:336-342`) and not in the narrow
list, so it is admitted, gossiped fleet-wide, and then skipped by every builder forever.
**Equivocation slashing is currently unenforceable on-chain.** This is a security property the
whitepaper and `specs/security_model.md` assume works; it should be raised as its own incident.

### 2.3 The architectural root cause

`utxo.rs:222` is not the disease; it is the second symptom of one disease. There are **five**
independently hand-maintained lists that must agree about "which tx types may carry no UTXO flow",
and nothing in the type system, the tests, or the review process binds them together:

| # | Location | Entries | Consulted by |
|---|----------|---------|--------------|
| L1 | `validation/transaction.rs:39-63` — empty-inputs allow-list | 16 predicates | structural validation (all paths) |
| L2 | `validation/transaction.rs:67-88` — empty-outputs allow-list | 15 predicates | structural validation (all paths) |
| L3 | `transaction/core.rs:463` `is_state_only()` | 9 types | mempool routing, gossip relay, RPC submit |
| L4 | `validation/utxo.rs:222-227` | 3 types | **consensus** fee/balance exemption |
| L5 | `validation/utxo.rs:261-269` `fee_exempt` | 6 DeFi types | consensus fee exemption |

L1/L2 say "these types may have no flow". L3 says "these types skip mempool fee accounting". L4 says
"these types skip consensus fee accounting". The three sets should be the same set. They are not, and
they have never been the same set. Every new 0-flow tx type added since v6.21.7 (`AddMaintainer`,
`RemoveMaintainer`, `ProtocolActivation`, `PriceAttestation`) was added to L1/L2 (so it validates
structurally) and to L3 (so it is admitted) but not to L4 (so it can never be mined). The defect is
**structural drift between redundant enumerations**, and it recurs on every new type.

The `is_state_only()` doc contract is itself wrong and reinforces the drift: it says
*"State-only txs (Exit, RequestWithdrawal, etc.) … have no UTXO inputs by design"* — but
`RequestWithdrawal` is not in the list and does consume Bond inputs (`core.rs:416`), and
`ClaimReward`/`ClaimBond` are in the list yet do produce outputs. The name promises a property the
function does not compute.

---

## 3. Capability inventory (PRIOR-KNOWLEDGE-GATE)

**All 24 live `TxType` variants** (`crates/core/src/transaction/types.rs:7-140`; discriminants 24-30
are permanently tombstoned and `from_u32` returns `None` for them):

`Transfer(0)`, `Registration(1)`, `Exit(2)`, `ClaimReward(3)`, `ClaimBond(4)`, `SlashProducer(5)`,
`Coinbase(6)`, `AddBond(7)`, `RequestWithdrawal(8)`, `ClaimWithdrawal(9, tombstone-in-enum)`,
`EpochReward(10)`, `RemoveMaintainer(11)`, `AddMaintainer(12)`, `DelegateBond(13)`,
`RevokeDelegation(14)`, `ProtocolActivation(15)`, `PriceAttestation(16)`, `MintAsset(17)`,
`BurnAsset(18)`, `CreatePool(19)`, `AddLiquidity(20)`, `RemoveLiquidity(21)`, `Swap(22)`,
`ZKSettle(31)` — **24 variants**, matching the CLAUDE.md code map.

Types that bypass `validate_transaction_with_utxos` entirely (early return, `utxo.rs:37-87`):
`Coinbase` (always), `EpochReward` (own conservation path). These are unaffected by this defect.

---

## 4. Blast-radius table — every 0-flow candidate

Wire shape is read from the constructor, not assumed. "Narrow" = in `utxo.rs:222` list.
"Broad" = in `is_state_only()`.

| # | TxType | Constructor (file:line) | inputs | outputs | Narrow | Broad | Consensus verdict today | Reachable today? |
|---|--------|------------------------|--------|---------|--------|-------|-------------------------|------------------|
| 1 | `Registration` (genesis form) | `assembly.rs:137-143` | 0 | 0 | ✅ | ❌ | **VALID** (exempt) | Yes — every genesis block. **Load-bearing.** |
| 2 | `Registration` (normal form) | `core.rs:207-249` | ≥1 | ≥1 Bond | n/a | ❌ | VALID (pays fee) | Yes — CLI register |
| 3 | `DelegateBond` | `core.rs:831-839` | 0 | 0 | ✅ | ✅ | **VALID** (exempt) | Yes — `cli/cmd_producer/delegation.rs:125` |
| 4 | `RevokeDelegation` | `core.rs:842-850` | 0 | 0 | ✅ | ✅ | **VALID** (exempt) | Yes — `cli/cmd_producer/delegation.rs:215` |
| 5 | `AddMaintainer` | `core.rs:766-779` | 0 | 0 | ❌ | ✅ | **`InsufficientFee`** | Yes — RPC `submitMaintainerChange` (`governance.rs:241`) — **INC-I-173** |
| 6 | `RemoveMaintainer` | `core.rs:737-755` | 0 | 0 | ❌ | ✅ | **`InsufficientFee`** | Yes — RPC `submitMaintainerChange` (`governance.rs:243`) — **INC-I-173** |
| 7 | `SlashProducer` | `core.rs:333-343` | 0 | 0 | ❌ | ✅ | **`InsufficientFee`** | **Yes — node-generated on equivocation** (`rewards.rs:474-502`). Silently dead. |
| 8 | `ProtocolActivation` | `core.rs:858-866` | 0 | 0 | ❌ | ❌ | **`InsufficientFee`** — but blocked EARLIER at mempool `FeeTooLow` (`pool.rs:575`) and never relayed | Yes — CLI `cmd_governance.rs:215` |
| 9 | `Exit` | `core.rs:252-263` | 0 | 0 | ❌ | ✅ | **`InsufficientFee`** | Only hand-crafted raw tx. **No shipped tool builds it** — `cli producer exit` builds a `RequestWithdrawal` instead (`cmd_producer/exit.rs:169`). |
| 10 | `ClaimReward` | `core.rs:277-288` | 0 | **1 (amount>0)** | ❌ | ✅ | **`InsufficientFunds`** (0 < amount) | Only hand-crafted. Superseded by `EpochReward` ("No manual claim needed", `types.rs:32`). **Conservation trap for the fix.** |
| 11 | `ClaimBond` | `core.rs:303-314` | 0 | **1 (amount>0)** | ❌ | ✅ | **`InsufficientFunds`** | Only hand-crafted. **Conservation trap for the fix.** |
| 12 | `PriceAttestation` | `core.rs:898-906` | 0 | 0 | ❌ | ✅ | Would be `InsufficientFee` — never reached | **No** — `oracle_activation_height = u64::MAX` on mainnet/testnet/devnet (`network_params/defaults.rs:195, 405, 570`); structurally rejected pre-activation. |

**Un-mineable and reachable today: `AddMaintainer`, `RemoveMaintainer`, `SlashProducer`,
`ProtocolActivation` (4).** Un-mineable and reachable only by a hand-crafted raw transaction:
`Exit`, `ClaimReward`, `ClaimBond` (3). Un-mineable but frozen behind an activation height:
`PriceAttestation` (1) — it becomes a live defect the moment `oracle_activation_height` is pinned,
so the fix must land **before** any oracle activation decision.

**Verified working (the tell):** `DelegateBond`/`RevokeDelegation` are the only 0-flow governance
types in the narrow list, and they are the only ones with a working end-to-end CLI path. This is
consistent with them being the two types INC-I-057 fixed. (I could not find a positive on-chain
sample on the current testnet — no `getDelegations` RPC exists and the log window has rotated past
any historical delegation; recorded as an open question, not a blocker, since the code path is
unambiguous.)

---

## 5. Validation topology map

### 5.1 `validate_transaction_with_utxos` — production call sites

Grep across the workspace (`crates`, `bins`), excluding `target/` and test files:

| Call site | Role | Failure effect |
|-----------|------|----------------|
| `bins/node/src/node/production/assembly.rs:235` | **block builder**, per mempool tx | `continue` → tx skipped, stays in mempool, retried next slot forever |
| `bins/node/src/node/apply_block/tx_processing.rs:99` | **consensus / apply_block**, per block tx | `ValidationMode::Full` → `return Err` → **the whole block is rejected**; `ValidationMode::Replay` → warn and continue (INC-I-064) |

Everything else is test code. (The code graph shows only test callers because graphify is blind to
Rust `self.method()` receivers — this table is grep-derived, per the known blind spot.)

**Decisive consequence:** because `apply_block` runs the same validator in `Full` mode, a block
containing a 0-fee `AddMaintainer` **is rejected by every validating node today**. Therefore this is
not merely a builder-liveness gap — the current behavior is a **consensus rule**, and changing it is
a **consensus-rule relaxation** that requires a forward-only activation height. This directly
answers the handoff's question and settles the INV-12 classification.

**Builder/apply parity (INV-PROD-003) is intact.** Both sides call the identical function with an
identically-constructed `ValidationContext` (`assembly.rs:180-223` vs `tx_processing.rs:61-98` — same
`with_*` chain, same params source). There is no parity split to repair; the defect is that both
sides are consistently wrong. **Any fix must be applied inside the shared validator (or in a shared
predicate it calls), never at one call site**, or it would *create* the parity split that
INV-VALIDATION-001 / INV-PROD-003 exist to prevent.

### 5.2 `is_state_only()` — production call sites

| Call site | Role | Behavior for a type NOT in the broad set |
|-----------|------|------------------------------------------|
| `crates/rpc/src/methods/transaction.rs:203` | RPC `sendTransaction` routing | falls to `add_transaction` → UTXO+fee check → `FeeTooLow` for 0-flow txs |
| `bins/node/src/node/validation_checks.rs:915-928` | **gossip-received tx handler** `handle_new_transaction` | same → rejected → **not added, and therefore not re-broadcast** (`validation_checks.rs:945-947`) |
| `crates/mempool/src/pool.rs:735` | comment only (documents why `Registration` is excluded) | — |

`crates/rpc/src/methods/governance.rs:257` bypasses the routing decision entirely and calls
`add_system_transaction` unconditionally for maintainer changes — which is exactly why those txs are
*accepted* by the RPC and then silently starve.

`add_system_transaction` (`pool.rs:706-764`) runs `validate_transaction` (structural only) and skips
all UTXO/fee accounting; it inserts with `fee_rate = 0` (`pool.rs:779-780`), so such txs are the
lowest-priority build candidates but are still offered to the builder every slot.

### 5.3 Relay blocking (matches the INC-I-057 symptom)

INC-I-057's recorded symptoms included "txs not gossiped to other nodes". The mechanism is
`validation_checks.rs:915` → `add_transaction` rejection → the `Err` arm never reaches
`network.broadcast_transaction`. That mechanism is **still live today for `ProtocolActivation`**
(not in the broad set). It is not live for the maintainer types (they are in the broad set).

### 5.4 Other hand-maintained tx-type lists at drift risk

Searched `crates/core/src/validation/` for `matches!` over `tx.tx_type` and for `!tx.is_*()` chains:

- `validation/utxo.rs:93-97` `is_amm_pool_tx` (3 types) — pairs with `utxo.rs:178-182` `amm_pool_input_exempt` (same 3 types, duplicated).
- `validation/utxo.rs:261-269` `fee_exempt` (6 DeFi types) — mirrored **by hand** in
  `mempool/src/pool.rs:567-583` (`amm_gated`), with a comment admitting the mirror is manual.
- `validation/transaction.rs:39-63` and `:67-88` — the L1/L2 structural allow-lists (16 and 15
  predicates), maintained by appending `&& !tx.is_<new_type>()`.
- `validation/transaction.rs:101-111` AMM activation gate (4 types).

All five have the same failure mode as L4: adding a tx type requires remembering an unmarked list.

---

## 6. INC-I-057 precedent — how the same bug was fixed last time

`git log -L 210,235:crates/core/src/validation/utxo.rs` and `git show 34691e2a`:

- **Commit `34691e2a`, 2026-05-07, "chore: release v6.21.7 — delegation fixes (INC-I-057, INC-I-061)"**.
- The diff to `utxo.rs` renamed `is_genesis_registration` (1 type) to `is_state_only_tx` and widened
  the `matches!` from `{Registration}` to `{Registration, DelegateBond, RevokeDelegation}` —
  i.e. **it enumerated exactly the two types that were stuck at that moment**, and did not reach for
  `Transaction::is_state_only()`, which already existed and already listed 7+ types.
- The same commit patched `bins/node/src/node/validation_checks.rs` with a *second* hand-maintained
  list: `matches!(tx.tx_type, TxType::DelegateBond | TxType::RevokeDelegation)`. That one was later
  unified to `tx.is_state_only()` (present form at `validation_checks.rs:915`) — the consensus-side
  list never was.
- **The change shipped with NO activation height and no gate.** It was an unconditional consensus
  relaxation released as `v6.21.7`. There is no `inc_i_057` symbol anywhere in the tree.

**Reading of the precedent:** it establishes that this exact widening has been done before, but it
does **not** establish a safe precedent to copy. Shipping an unconditional consensus relaxation is
precisely what the post-INC-I-054 / INC-I-075 discipline in CLAUDE.md forbids ("(1|2) YES + (3) NO →
activation height REQUIRED"). v6.21.7 predates that discipline hardening. The INC-I-173 fix must
**not** reuse the v6.21.7 deployment pattern; it must be gated. Whether v6.21.7's unconditional
relaxation ever caused a mixed-fleet divergence is an open question (see §10).

**Pre-existing vs INC-I-172 regression — CONFIRMED pre-existing.**
`crates/core/src/validation/utxo.rs` last touched `c0cbcc06` (2026-05-29, INC-I-096 M2);
`bins/node/src/node/production/assembly.rs` last touched `98356071` (2026-08-06, a comment-only
docs commit). Neither is `b5f68bba` (INC-I-172). INC-I-172 only exposed the defect by exercising the
governance write path for the first time.

---

## 7. Architecture context

### Module boundaries
- `crates/core/src/transaction` — wire shapes + `is_state_only()`. Depends on: `crypto`.
  Depended by: everything.
- `crates/core/src/validation` — structural (`transaction.rs`) + UTXO/consensus (`utxo.rs`).
  Depends on: `transaction`. Depended by: `mempool`, `bins/node`, `crates/rpc` (indirectly).
- `crates/mempool` — admission policy; owns its own fee floor, mirroring consensus by hand.
- `crates/rpc` — submission routing (`transaction.rs`), governance submission (`governance.rs`).
- `bins/node` — builder (`production/assembly.rs`), consensus apply (`apply_block/`), gossip
  handler (`validation_checks.rs`), slash generation (`rewards.rs`).

### Data flow through the affected area
```
CLI/RPC ─► sendTransaction ─┬─(is_state_only)─► add_system_transaction ─► structural only ─► POOL
                            └─(else)──────────► add_transaction ─► UTXO+fee ─► POOL or FeeTooLow
gossip  ─► handle_new_transaction ─► same fork ─► POOL ─► rebroadcast (only on Ok)
POOL ─► assembly.rs:235 validate_transaction_with_utxos ─► [FEE GATE] ─► block
block ─► tx_processing.rs:99 validate_transaction_with_utxos ─► [SAME FEE GATE] ─► apply or REJECT BLOCK
apply ─► apply_block/governance.rs:30/65/101 (maintainer + protocol activation handlers — currently unreachable)
```

### Architectural constraints and invariants
- **INV-VALIDATION-001 / INV-PROD-003 (builder ↔ apply parity)** — the builder and `apply_block` must
  accept exactly the same tx set. Preserved today. Any fix must live in the shared validator.
- **Conservation** — native DOLI output must never exceed native input. Enforced *only* by
  `utxo.rs:250` for non-exempt txs and by the `outputs.is_empty()` conjunct for exempt ones.
- **INV-PARAMS-001 / INC-I-054** — a crossed activation height is immutable; new features get their
  own new height, never a reused or bundled one.
- **CLAUDE.md #0** — forward-only activation, never a genesis reset.
- **Genesis Registration exemption is load-bearing** (see Correction 1).

### Blast radius
- **Direct:** `crates/core/src/validation/utxo.rs`, `crates/core/src/transaction/core.rs`
  (`is_state_only` and/or a new predicate), `crates/core/src/network_params/` (new AH field for all
  3 networks).
- **Indirect (behavior changes if the predicate is shared):** `crates/mempool/src/pool.rs`
  (admission routing/fee floor), `crates/rpc/src/methods/transaction.rs` (submit routing),
  `bins/node/src/node/validation_checks.rs` (gossip routing + relay),
  `bins/node/src/node/production/assembly.rs` (builder inclusion),
  `bins/node/src/node/apply_block/tx_processing.rs` + `apply_block/governance.rs` (the handlers that
  become reachable for the first time — they have never executed on any network).
- **Newly-live code:** `apply_block/governance.rs:30-160` has never run in production. It becomes
  reachable the moment this is fixed. It must be treated as unproven code, not as regression-safe.

### Brittleness check
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 3/5
Details:
  [1] Cross-module blast radius — the correct behavior is enumerated independently in 5 places
      across crates/core/validation, crates/core/transaction, crates/mempool, crates/rpc and
      bins/node, none of which depend on a shared predicate.
  [2] Invariant gaps — the invariant "types allowed 0-in/0-out (L1∩L2) == types exempt from
      fee/balance (L4) == types routed via the system path (L3)" is enforced by no module, no
      type, and no test.
  [5] Contract absence — is_state_only()'s doc contract is factually wrong (cites
      RequestWithdrawal, which is not in the list and has inputs; includes ClaimReward/ClaimBond,
      which have outputs). The five lists interact through implicit convention only.
  Not detected: [3] data-flow reversal, [4] shared mutable state.
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```
This is an architectural drift defect, not a localized code bug. A patch that only widens
`utxo.rs:222` to include the two maintainer types would re-create the exact dead-end that INC-I-057's
patch created, and INC-I-173 would recur at the next 0-flow tx type.

---

## 8. Consensus classification (INV-12, MANDATORY)

**Q1 — can a user-submittable tx reach this path?**
**YES.** `AddMaintainer`/`RemoveMaintainer` via RPC `submitMaintainerChange`
(`governance.rs:241/243`); `ProtocolActivation` via CLI (`cmd_governance.rs:215`);
`Exit`/`ClaimReward`/`ClaimBond` via a hand-crafted raw `sendTransaction`.

**Q2 — can a producer-action or attestation pattern reach it?**
**YES.** `SlashProducer` is generated by the node itself on equivocation detection
(`rewards.rs:474-502`) and broadcast fleet-wide. `PriceAttestation` would reach it if
`oracle_activation_height` is ever pinned.

**Q3 — is the new behavior bit-identical for ALL reachable inputs?**
**NO.** Transactions that `validate_transaction_with_utxos` rejects today would be accepted after the
change — flipping both builder inclusion and, critically, the `apply_block` accept/reject verdict for
any block containing one.

**(Q1 | Q2) YES + Q3 NO → ACTIVATION HEIGHT REQUIRED.** A new, dedicated, never-before-used
activation-height field in `crates/core/src/network_params/` (its own name; not reused, not bundled
onto `maintainer_derivation_activation_height` or `security_audit_activation_height`), set to a
future height on mainnet and testnet.

**Deploy question 2 — does this change block CONTENT?**
Below the activation height: **no** — the validator is bit-identical, so a rolling restart is safe.
At and above it: **yes** — new tx types can appear in blocks. The gate converts a synchronized-deploy
requirement into a fleet-upgrade-deadline requirement: every node (including the ~30 external
auto-update producers) must be running the new binary before the chosen height. The height must be
chosen with the external-producer upgrade window in mind, not just the structural fleet.

**No genesis reset is required.** The change activates forward-only at a future height and does not
alter the state root of any existing block.

**No version bumps.** `CURRENT_PROTOCOL_VERSION` and `EPOCH_STATE_FORMAT_VERSION` must NOT be
touched: the `EpochState` serialization format is unchanged and the peer handshake is unaffected.

---

## 9. Requirements (MoSCoW)

### Summary (plain language)
Some transaction types are built to move no coins at all — governance votes, slashing evidence,
delegation. The rule that says "every transaction must pay a fee" is applied to them anyway, so they
can never get into a block. The list of exceptions was written by hand and was never kept up to date.
We want to replace the hand-written list with one shared definition, turn it on at a future block
height so nobody's chain splits, and make sure no transaction that actually moves coins can sneak
into the exception.

### User stories
- As a maintainer, I want a `submitMaintainerChange` transaction to be mined and applied, so that the
  update trust root can actually be rotated (the goal of INC-I-172).
- As a node operator, I want an equivocation slash transaction I generate to be mined, so that
  double-signing is punished on-chain as the security model claims.
- As a protocol maintainer, I want a `ProtocolActivation` transaction to be admitted, relayed and
  mined, so that on-chain protocol activation is an available mechanism.
- As a core developer, I want adding a new zero-flow transaction type to require editing exactly one
  list, so that this defect class cannot recur a third time.

### Requirements

| ID | Requirement | Priority | Acceptance criteria |
|----|-------------|----------|---------------------|
| REQ-173-001 | The fee/balance exemption in `validate_transaction_with_utxos` must be derived from a single shared predicate, not a locally hand-maintained `matches!`. | Must | - [ ] `utxo.rs` contains no literal tx-type list for the state-only exemption<br>- [ ] the predicate lives in `crates/core` and is the same symbol consulted by the structural allow-lists |
| REQ-173-002 | The new predicate must be a strict superset of today's narrow list, including `Registration`. | Must | - [ ] a 0-in/0-out `Registration` is still exempt<br>- [ ] a test builds the exact genesis registration tx from `assembly.rs:137` and asserts it validates |
| REQ-173-003 | Behavior below the activation height must be bit-identical to today for every tx type. | Must | - [ ] for each of the 24 types, a test asserts the accept/reject verdict at `height = AH - 1` equals the pre-change verdict<br>- [ ] `Exit`, `ClaimReward`, `ClaimBond`, `SlashProducer`, `AddMaintainer`, `RemoveMaintainer`, `ProtocolActivation`, `PriceAttestation` all still fail below the gate |
| REQ-173-004 | The exemption must remain conditioned on `inputs.is_empty() && outputs.is_empty()`. | Must | - [ ] a `ClaimReward` with 0 inputs and a 1 000 000 DOLI output is REJECTED at every height, above and below the gate<br>- [ ] a `ClaimBond` with 0 inputs and a non-zero output is REJECTED at every height<br>- [ ] a fuzz/property test asserts no tx with a non-empty output set is ever fee/balance exempt |
| REQ-173-005 | The change must be gated by a NEW, dedicated forward-only activation height in `network_params`, set to a future height on mainnet and testnet. | Must | - [ ] a new named field exists for all 3 networks<br>- [ ] no existing activation height is moved or reused<br>- [ ] the mainnet value is strictly greater than the mainnet tip at release<br>- [ ] the height is threaded through `ValidationContext` with a `with_*` builder, and BOTH `assembly.rs` and `tx_processing.rs` set it |
| REQ-173-006 | Builder/apply parity must be preserved (INV-PROD-003). | Must | - [ ] the gate is evaluated inside `validate_transaction_with_utxos`, not at any call site<br>- [ ] a test drives the same tx + same height through both the builder context and the apply context and asserts identical verdicts |
| REQ-173-007 | No version bumps; no genesis reset. | Must | - [ ] `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION` unchanged in the diff<br>- [ ] genesis hash unchanged; chainspec untouched |
| REQ-173-008 | Above the activation height, a `RemoveMaintainer` then `AddMaintainer` must be mined AND applied on the local testnet, with the `[MAINTAINER]` apply log line observed and `getMaintainerSet` reflecting the change across a block boundary. | Must | - [ ] end-to-end run: tx accepted → included in block N → `[MAINTAINER]` log at N → set changed at N+1 → set still changed at N+5 (no auto-reset)<br>- [ ] this closes the INC-I-172 verification gap |
| REQ-173-009 | `ProtocolActivation` must be admitted, relayed and mined above the gate. | Should | - [ ] `is_state_only()` (or the new shared predicate used for routing) includes `ProtocolActivation`<br>- [ ] a gossip-relay test asserts the tx is re-broadcast after admission |
| REQ-173-010 | `SlashProducer` generated by `rewards.rs` must be mineable above the gate. | Should | - [ ] an equivocation-driven integration test produces a slash tx that lands in a block<br>- [ ] raised and tracked as its own incident (security-model gap, not just liveness) |
| REQ-173-011 | The five redundant lists must be reduced so that adding a new zero-flow tx type requires editing ONE place, with a compile-time or test-time failure if any other site drifts. | Should | - [ ] L1/L2 structural allow-lists and L4 derive from the same source<br>- [ ] a test enumerates all 24 `TxType` variants and asserts the L1∩L2 set equals the exemption set — new types fail the build/test until classified<br>- [ ] the fix must NOT re-create the enumeration dead-end (anti-overengineering Rule 18: non-foreclosure) |
| REQ-173-012 | `is_state_only()`'s doc contract must be corrected to describe what it computes. | Should | - [ ] the doc no longer cites `RequestWithdrawal`<br>- [ ] the doc states explicitly whether outputs are permitted, and reconciles `ClaimReward`/`ClaimBond` |
| REQ-173-013 | `PriceAttestation` must be covered by the same exemption so that pinning `oracle_activation_height` later does not resurrect this defect. | Should | - [ ] a test asserts a `PriceAttestation` is exempt above BOTH gates<br>- [ ] no change to `oracle_activation_height` itself (stays `u64::MAX`) |
| REQ-173-014 | Spam/DoS analysis for newly-mineable 0-fee txs. | Should | - [ ] documented answer: what stops an attacker filling blocks with 0-fee `AddMaintainer` txs carrying invalid signatures?<br>- [ ] confirm structural validation rejects them before the builder spends a slot, or add a cost |
| REQ-173-015 | Fix the CLI `doli-node maintainer add` signature truncation (16 hex chars) so a maintainer change can be produced without an external signer. | Could | - [ ] CLI emits a full 128-hex-char Ed25519 signature |
| REQ-173-016 | Purge or expire the 3 stuck maintainer txs from the testnet mempool. | Won't | N/A — harmless, they expire; and they are useful live evidence until the fix lands |
| REQ-173-017 | INC-I-171 (vesting penalty unenforced) and INC-I-170 (key exposure). | Won't | N/A — independent defects, separately tracked |
| REQ-173-018 | Removing `ClaimReward`/`ClaimBond` as tx types, or redesigning the fee schedule. | Won't | N/A — deferred; out of scope for a fee-gate correction |

---

## 10. Simplest recommendation (SSF)

**The simplest fix that addresses the root cause:** make the `utxo.rs` fee/balance exemption consult
the *same* predicate that the structural validator already uses to decide which types may be
0-input/0-output, keeping the `inputs.is_empty() && outputs.is_empty()` conjunct, behind a new
forward-only activation height.

This works because a transaction type that the protocol already permits to carry zero inputs and zero
outputs is by construction a transaction with no UTXO flow to charge a fee against — so the two
questions "may this type have no flow?" and "is this type exempt from flow accounting?" are the same
question, and answering them from one place removes the drift that produced both INC-I-057 and
INC-I-173. It is strictly safer than swapping in `is_state_only()` (which drops `Registration` and
misses `ProtocolActivation`), and the retained conjunct keeps `ClaimReward`/`ClaimBond` unable to
mint.

Design and failure-mode analysis, RESOURCE COST, and the specific activation height belong to the
architect. **Stopping at the user gate — no code, no height pinned.**

---

## 11. What I do not understand (mandatory)

1. **Whether v6.21.7's unconditional relaxation ever split a fleet.** I confirmed the commit shipped
   with no gate; I did not verify how it was deployed (synchronized vs rolling) or whether a
   divergence followed. If it was deployed synchronized, that is a precedent for a deploy pattern —
   but the external auto-update producers make that infeasible today.
2. **The mainnet tip height**, and therefore what a safe future activation height is. I did not query
   mainnet (read-only local testnet only, per constraint). Needed before any height is proposed.
3. **Whether `Exit` is dead by design or by accident.** No shipped tool builds it, yet
   `apply_block` presumably still handles it. If it is dead by design it should be tombstoned;
   if it is a regression, it is a separate incident. I did not trace the apply-side `Exit` handler.
4. **The spam surface of newly-mineable 0-fee txs** (REQ-173-014). I confirmed structural validation
   runs first, but I did not measure its cost per tx, so I cannot say whether an attacker could
   exhaust the 60%-of-slot build budget with cheap invalid governance txs.
5. **Whether `ClaimReward`/`ClaimBond` are still handled by `apply_block`.** If they are not, they
   are dead types whose presence in `is_state_only()` is pure hazard and they should be tombstoned
   rather than carried through the redesign.
6. **Positive on-chain evidence that `DelegateBond` has been mined.** The code path is unambiguous
   and no other explanation fits the narrow list, but I could not produce a mined sample (no
   `getDelegations` RPC; log window rotated).

---

## 12. Open questions for the user (not blocking this analysis)

1. Should `SlashProducer` being unenforceable be raised as its own incident (security-model gap),
   separate from INC-I-173?
2. Should `ProtocolActivation` be brought into scope of this fix (it is broken at an earlier stage —
   mempool admission — and needs the routing predicate widened too), or split into its own incident?
3. Confirm the target activation height policy: one new height covering all zero-flow types at once,
   or per-type heights? (One height is simpler; per-type foreclosure risk is low.)
4. Is `Exit` intended to remain a live transaction type?
5. Testnet-first is assumed. Confirm that the testnet activation height should be set to a near-term
   height so REQ-173-008 can be exercised in this cycle.

---

## 13. Traceability matrix

Test IDs filled in by the test-writer for **M1 (F1+F2+F3) only**. M2/M3/M4
requirements are marked `(M2)` / `(M3)` / `(M4)` and are deliberately untested at
this milestone. Full test plan: `docs/.workflow/inc-i-173-M1-test-plan.md`.
RED evidence: `docs/.workflow/inc-i-173-M1-test-red-evidence.txt`.

Test file keys:
- **ZFP** = `crates/core/tests/inc_i_173_zero_flow_predicate.rs`
- **FG**  = `crates/core/tests/inc_i_173_fee_gate.rs`
- **AH**  = `crates/core/tests/inc_i_173_activation_height.rs`
- **ND**  = `bins/node/tests/inc_i_173_state_only_fee_gate.rs`

| Requirement ID | Priority | Test IDs | Architecture section | Implementation module |
|---------------|----------|----------|---------------------|----------------------|
| REQ-173-001 | Must | ZFP: `exempt_set_is_exactly_the_five_authorized_types`, `exempt_set_is_a_strict_superset_of_the_frozen_legacy_three`, `maintainer_governance_types_are_exempt`, `zero_flow_reduces_to_allows_empty_io_when_shape_is_zero_in_zero_out`, `no_transaction_with_inputs_is_ever_zero_flow`, `allows_empty_io_is_a_pure_total_const_function`, `all_tx_types_lists_twenty_four_distinct_variants` / FG: `req_173_001_maintainer_txs_are_accepted_at_and_above_the_gate`, `req_173_001_maintainer_txs_are_rejected_with_insufficient_fee_below_the_gate`, `req_173_001_the_verdict_flips_at_exactly_the_activation_height`, `req_173_001_a_context_that_never_sets_the_height_stays_below_the_gate_forever` | F1 | IMPLEMENTED M1: `TxType::allows_empty_io` @ `crates/core/src/transaction/types.rs`; `Transaction::is_zero_flow` @ `crates/core/src/transaction/core.rs`; AH-gated twin binding `is_state_only_tx` @ `crates/core/src/validation/utxo.rs` |
| REQ-173-002 | Must | FG: `req_173_002_genesis_registration_validates_below_the_gate`, `req_173_002_genesis_registration_validates_above_the_gate`, `req_173_002_genesis_registration_validates_on_devnet_where_the_gate_is_zero`, `req_173_002_delegation_types_validate_on_both_branches` | F3 (C3) | IMPLEMENTED M1: `TxType::allows_empty_io` (`Registration` arm = true) @ `crates/core/src/transaction/types.rs` |
| REQ-173-003 | Must | FG: `req_173_003_all_24_types_keep_their_pre_change_verdict_below_the_gate`, `req_173_003_fee_gate_rejections_keep_their_error_variant_below_the_gate`, `req_173_003_only_the_two_maintainer_types_change_verdict_above_the_gate` / ND (C8): `req_173_003_c8_the_declared_validation_mode_is_full_not_replay`, `req_173_003_c8_apply_only_tolerates_utxo_failures_in_replay_mode` | F1 + C8 | IMPLEMENTED M1: below-gate `else` branch retained character-identical @ `crates/core/src/validation/utxo.rs` |
| REQ-173-003b | Must | ZFP: `exit_is_not_exempt_because_its_apply_handler_authenticates_nobody`, `slash_producer_is_not_exempt_because_its_evidence_is_forgeable_for_free`, `exit_and_slash_share_the_wire_shape_of_the_exempt_types_yet_differ` / FG: `req_173_003b_exit_is_rejected_at_every_height`, `req_173_003b_slash_producer_is_rejected_at_every_height`, `req_173_003b_negatives_are_not_vacuous_the_exempt_types_pass_at_the_same_heights` | F3 (C1) | IMPLEMENTED M1: `TxType::allows_empty_io` (`Exit` / `SlashProducer` arms = false, cited) @ `crates/core/src/transaction/types.rs` |
| REQ-173-004 | Must | ZFP: `no_transaction_with_outputs_is_ever_zero_flow`, `claim_reward_and_claim_bond_are_never_exempt`, `fixture_tx_with_outputs_builds_real_outputs` / FG: `req_173_004_claim_reward_with_a_large_output_is_rejected_at_every_height`, `req_173_004_claim_bond_with_a_non_zero_output_is_rejected_at_every_height`, `req_173_004_no_exempt_type_escapes_the_balance_check_once_it_has_an_output`, `req_173_004_a_zero_amount_output_still_disqualifies_the_exemption` | F1 (C2) | IMPLEMENTED M1: `Transaction::is_zero_flow` mint-guard conjunct @ `crates/core/src/transaction/core.rs` |
| REQ-173-005 | Must | AH: `req_173_005_devnet_gate_is_zero`, `req_173_005_testnet_gate_is_pinned_near_future_and_is_not_a_no_op`, `req_173_005_mainnet_gate_is_not_pinned_in_m1`, `req_173_005_the_gate_is_dedicated_and_not_bundled_onto_an_existing_height`, `req_173_005_no_existing_activation_height_was_moved`, `req_173_005_validation_context_defaults_the_gate_to_u64_max`, `req_173_005_the_builder_sets_the_gate`, `req_173_005_the_builder_is_a_plain_assignment_last_write_wins`, `req_173_005_the_builder_touches_only_its_own_field` | F2 | IMPLEMENTED M1: `NetworkParams::inc_i_173_activation_height` @ `crates/core/src/network_params/{mod,defaults,env_loader}.rs`; `ValidationContext::inc_i_173_activation_height` + `with_inc_i_173_activation_height` @ `crates/core/src/validation/types.rs` |
| REQ-173-006 | Must | ND: `req_173_006_builder_and_apply_contexts_agree_on_every_verdict`, `req_173_006_both_shapes_flip_at_the_gate_so_parity_is_not_vacuous`, `req_173_006_forgetting_one_site_is_observably_a_fork`, `req_173_006_assembly_sets_the_inc_i_173_activation_height`, `req_173_006_tx_processing_sets_the_inc_i_173_activation_height`, `req_173_006_neither_call_site_hardcodes_the_height`, `req_173_006_neither_call_site_evaluates_the_gate_itself` | F2 (C4) | IMPLEMENTED M1 (BOTH sites): `.with_inc_i_173_activation_height(...)` @ `bins/node/src/node/production/assembly.rs` AND `bins/node/src/node/apply_block/tx_processing.rs` |
| REQ-173-007 | Must | AH: `req_173_007_mainnet_genesis_hash_is_unchanged`, `req_173_007_the_three_networks_keep_distinct_genesis_identities`, `req_173_007_consensus_params_genesis_hash_still_matches_the_chainspec` / ND: `req_173_007_no_protocol_version_was_bumped`, `req_173_007_no_hardfork_schedule_entry_was_added`, `req_173_007_adding_the_gate_did_not_disturb_the_genesis_identity` | Consensus Classification | IMPLEMENTED M1: none (guard only) — verified no version constant, genesis hash or `HardForkSchedule` entry changed |
| REQ-173-008 | Must | (M2 — e2e testnet above the gate; NOT covered by M1) | Migration Path step 3 | (M2) |
| REQ-173-009 | Should | (Option A, NOT taken in M1; FG `req_173_003_only_the_two_maintainer_types_change_verdict_above_the_gate` PINS `ProtocolActivation` as still rejected, so taking Option A later requires deliberately editing that test) | Option A | (deferred) |
| REQ-173-010 | Should | (M3) | F4 | (M3) |
| REQ-173-011 | Should | (M3 — F7 cross-list test; ZFP `all_tx_types_lists_twenty_four_distinct_variants` is the partial M1 down-payment) | F7 | (M3) |
| REQ-173-012 | Should | (M3 — retired by F4 deletion) | F4 | (M3) |
| REQ-173-013 | Should | (Option B, NOT taken in M1; FG pins `PriceAttestation` as still rejected) | Option B | (deferred) |
| REQ-173-014 | Should | (M3) | F5 | (M3) |
| REQ-173-015 | Could | (M3+) | Option E | (deferred) |

---

## 14. Specs drift detected

- `crates/core/src/transaction/core.rs:456-462` — `is_state_only()` doc contract is factually wrong
  (cites `RequestWithdrawal`, which is not in the list and consumes Bond inputs; claims "no UTXO
  inputs by design" while `ClaimReward`/`ClaimBond` produce outputs). Not fixed here — flagged as
  REQ-173-012 because it is inside the change surface.
- `crates/rpc/src/methods/transaction.rs:193-195` — same incorrect "Exit, RequestWithdrawal, etc."
  phrasing in the routing comment.
- `crates/core/src/validation/utxo.rs:219-221` — the comment enumerates only Registration and
  delegation, which is accurate for the code but describes a policy that contradicts
  `is_state_only()`'s stated intent.
- `specs/security_model.md` / whitepaper claims about equivocation slashing should be checked against
  the finding that `SlashProducer` has never been mineable. Not read in this scoping pass.

---

## 15. Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|------------------------|----------------|-----------|
| 1 | `validate_transaction_with_utxos` has exactly two production call sites (builder + apply_block). | Only two places in the node run this check. | Yes — grep, §5.1 |
| 2 | `apply_block` in `Full` mode rejects the entire block on a tx validation error. | A block with one bad tx is thrown away whole. | Yes — `tx_processing.rs:117-123` |
| 3 | Genesis `Registration` txs are 0-in/0-out and depend on the current exemption. | The very first blocks of a chain need this exception. | Yes — `assembly.rs:137-143` |
| 4 | `oracle_activation_height` is `u64::MAX` on all three networks. | The oracle is switched off everywhere. | Yes — `defaults.rs:195, 405, 570` |
| 5 | The 3 stuck testnet txs are the INC-I-172 governance test txs. | The stuck transactions are ours, from the last session. | Assumed — hashes match the handoff's described test, not independently decoded |
| 6 | No mainnet state was read or changed during this analysis. | I touched nothing outside my own machine. | Yes — only 127.0.0.1 RPC and local files |
