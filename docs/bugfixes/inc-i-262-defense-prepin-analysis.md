# INC-I-262 — Defense pre-pin gaps: analysis

## Scope

Five defects that stand between the slashing, unbonding and producer-authorization
rules and their activation on a public network. All five rules are behind gates
that are not pinned on mainnet or testnet (`u64::MAX`), and are live on devnet
(height 0).

| Finding | Area | Modules |
|---|---|---|
| F1 (R-1) | Producer transaction builders outside the CLI | `bins/gui/src/commands/producer.rs`, `crates/wallet/src/tx_builder/builder.rs` |
| F2 (R-2), AUDIT-P2-603-001 | CLI exit with no Bond UTXO | `bins/cli/src/unbonding_tx.rs`, `bins/cli/src/cmd_producer/exit.rs` |
| F3 (R-3), SEC-CRYPTO-R-001 | Slash evidence tenure check | `crates/core/src/validation/slash_window.rs` and its four callers, `bins/node/src/node/slash_redeliver.rs` |
| F4 (R-4), REV-B3-002, REV-B3-016 | Slash target facts built below the gate | `bins/node/src/node/validation_checks/mod.rs`, `bins/node/src/node/production/assembly.rs`, `crates/mempool/src/slash_admission.rs` |
| F5 (R-5) | Offline producer-set tools | `bins/node/src/cli.rs`, `bins/node/src/main.rs`, `bins/node/src/operations/producer_set_check.rs`, `bins/node/src/node/init.rs` |

Line numbers are at HEAD `8facb556`.

## Summary (plain language)

1. The desktop wallet cannot register, add bonds or withdraw once producer
   signatures are required. Registration only lacks the signature. Add-bond and
   withdrawal are already malformed today, so a signature alone does not fix them.
2. A producer that exits with no bonds left loses access to more money than it
   must. The tool picks the first coins the node lists, locks the whole remainder
   for the unbonding period, and tells the user "You receive: 0".
3. A valid slash can be refused when the chain has skipped slots. The check
   compares a slot count with a block count. The node then deletes its own stored
   slash, so the offender is never punished.
4. Every block validation and every block build looks up producers for the
   slashing rules, also at heights where the slashing rules do not apply.
5. The offline producer-set check cannot replay a real test-network data
   directory: it uses genesis time 0. Its flags are also easy to misuse. No tool
   lists exited producers that still hold bonds.

## Memory briefing (constraints)

| Invariant | Constrains |
|---|---|
| INV-VALIDATION-001 (one rule, equal context on mempool and builder) | REQ-DEF-003, REQ-DEF-004 |
| INV-BOND-002 (an unmineable tx is refused where first seen) | REQ-DEF-001, REQ-DEF-003 |
| INV-SLASH-001 (evidence must verify at the effective height) | REQ-DEF-003 |
| INV-SLASH-004 (offence inside current registration, window in slots, same predicate on four paths) | REQ-DEF-003 |
| INV-SLASH-005 (a created slash stays deliverable until its burn is canonical) | REQ-DEF-003 |
| INV-SLASH-006, INV-SLASH-007 (registrant refusal; settled accused refused before owner lookup) | REQ-DEF-004 |
| INV-APPLY-BLOCK-002, INV-APPLY-BLOCK-003 (failed apply leaves no residue) | REQ-DEF-005 (the check replays through `apply_block`) |
| INV-UNBOND-001 (one Unbonding output at the gate) | REQ-DEF-002 |
| INV-VEST-001 (bond age from the node-stamped slot) | REQ-DEF-003 (candidate slot source, see F3) |

Active decisions read: decision 173 (producer-auth binding rule), decision 185
(evidence window), decision 165 (slashing rules). No failed approaches are
recorded for this scope.

## Architecture Context

### Module boundaries

- `crates/core/src/transaction/producer_auth.rs`: the producer-auth digest and
  signer. It does no I/O. Used by the CLI only. Validation in
  `crates/core/src/validation/producer_auth.rs` uses the digest.
- `bins/cli/src/producer_tx.rs`: pure builders for Registration and AddBond, and
  `finalize_producer_tx`, which signs the inputs and then the producer auth. Used
  by `cmd_producer/{register,bonds,withdrawal,exit}.rs`. This is a library module
  of the `doli_cli` crate.
- `crates/wallet`: a hand-written transaction builder and serializer. It depends
  on `crypto` but not on `doli-core` (see `crates/wallet/Cargo.toml`).
- `bins/gui`: Tauri commands. It depends on `wallet` and on `doli-core`. The
  Registration command builds a `doli_core::Transaction` directly. AddBond,
  RequestWithdrawal and Exit use the wallet crate builder.
- `crates/core/src/validation/slash_state.rs`, `slash_window.rs`,
  `slash_registration.rs`: the slash predicates. They read a precomputed
  `ctx.slash_targets` map and never read storage.
- `crates/mempool/src/slash_admission.rs`: builds the `slash_targets` facts
  (`slash_targets`, `target_state`) from `ProducerSet` and `UtxoSet`. It does the
  mempool admission and revalidation arms.
- `bins/node/src/node/validation_checks/mod.rs` (`validate_block_for_apply`):
  builds the block-validation context for gossip, sync, reorg, apply and replay.
- `bins/node/src/node/production/assembly.rs`: builds the builder context.
- `bins/node/src/node/slash_redeliver.rs` + `slash_reported.rs`: the per-node
  ledger of slashes that this node created.
- `bins/node/src/operations/producer_set_check.rs`: the offline check. It opens
  the store read-only, replays through `Node::new_for_replay`, and compares.

### Data flows through the affected area

- Producer tx (GUI/wallet/CLI) → `sendTransaction` RPC → mempool admission
  (`check_producer_auth` at next height) → builder → `validate_block_for_apply`
  → apply. The rejection text is `[ERRTX-PAUTH001]`.
- Slash evidence (gossip or local detector) → mempool `admission_ctx` →
  builder `slash_targets` + `validate_slash_target` → block validation
  `validate_block_slashes` → apply. The local ledger re-offers through
  `admit_own_slash` on every periodic tick.
- Stopped data directory → `StateDb::open_read_only` + `BlockStore::open_read_only`
  → replay into a scratch directory → compare producer sets → exit code.

### Architectural constraints and invariants

- The slash predicate must give one verdict on every node, including
  snap-synced nodes. `specs/security_model.md` (Tenure caveat) excludes a
  floor-header lookup because a snap-synced node may not hold that header. Any
  slot source for the tenure floor must come from state that every node holds
  identically. Today that is the UTXO set, the producer set or a constant.
- `ProducerInfo` bincode bytes are in the state root. A new stored field changes
  `state_root` at every height. That needs a chain reset, so it is not allowed.
- At and above the unbonding gate, a RequestWithdrawal must pay exactly one
  Unbonding output (`crates/core/src/validation/unbonding.rs:160`). So the exit
  cannot return change in a second output.
- Below the producer-auth gate, the signed form (payload + 64 bytes) is admitted,
  selected and applied (`bins/node/tests/it/inc_i_182_m1_below_gate_signed.rs`).
  The CLI depends on this: it always signs.

### Blast radius (graph first)

The code graph was built and queried with `blast.py --hops 2`:
- `validate_slash_window`: 14 dependents, all through `validate_slash_target`
  and `validate_block_slashes`, plus core window tests.
- `slash_targets`: 11 dependents in the graph. The graph misses the two
  cross-crate node callers. Grep confirms them at
  `validation_checks/mod.rs:231` and `production/assembly.rs:217`.
- `sign_producer_auth`: 47 dependents, all on the CLI side (four handlers + tests).
- `build_withdrawal_tx`: 26 dependents (CLI exit, withdrawal, tests).
- `new_for_replay`: the graph shows a lower bound (method blind spot). Grep finds
  two callers: `operations/producer_set_check.rs:189` and
  `operations/chain.rs:344` (`recover_chain_state`). A fix to the replay
  genesis time also changes `doli-node recover`.

Indirect impact: none of the five fixes changes a stored byte. F3 changes a
consensus verdict, but only behind the unpinned slashing gate.

### Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 1/5
Details: (2) invariant gap — for F3, no node-independent state holds the slot of
the tenure floor block. Signals 1, 3, 4 and 5 do not apply: every fix stays
inside one predicate, one builder or one tool.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Verified causes

### F1 — GUI and wallet producer builders never sign (R-1)

- The signer: `crates/core/src/transaction/producer_auth.rs:10` (domain tag),
  `:17-24` `producer_auth_digest`, `:28-32` `sign_producer_auth`. It appends 64
  bytes to `extra_data`.
- The rule: `crates/core/src/validation/producer_auth.rs:28-57`
  `check_producer_auth`. The gate is compared at `:37` against
  `inc_i_257_rule_height()`, which is the next height in the mempool. Refusal
  `[ERRTX-PAUTH001]` at `:23-25`. The payload must be canonical (`:60-74`).
- The gate: `crates/core/src/network_params/defaults.rs:286` (mainnet `u64::MAX`),
  `:559` (testnet `u64::MAX`), `:834` (devnet 0). The devnet env override is at
  `env_loader.rs:601-607`.
- The CLI: `bins/cli/src/producer_tx.rs:79-86` `finalize_producer_tx` signs the
  inputs and then always calls `sign_producer_auth`. The genesis hash comes from
  `getChainInfo.genesis_hash` (`:89-92`). Callers: `register.rs:194-195`,
  `bonds.rs:145-146`, `withdrawal.rs:154,212`, `exit.rs:174,197`.
- GUI Registration: `bins/gui/src/commands/producer.rs:53` `register_producer`.
  It builds the tx (`:205-211`), signs the inputs only (`:228-232`) and submits
  (`:235-238`). It never calls `sign_producer_auth`. It already fetches the chain
  info (`:98`), and the wallet `ChainInfo` has `genesis_hash`
  (`crates/wallet/src/types.rs:56`). **The cause is confirmed.**
- GUI AddBond: `producer.rs:256` `add_bonds` → `crates/wallet/src/tx_builder/builder.rs:304`
  `build_add_bond`. GUI RequestWithdrawal: `producer.rs:308` → `builder.rs:371`
  `build_request_withdrawal`. Both are signed by `builder.rs:138` `sign_and_build`.
  Neither adds a producer-auth tail.

⚠ CONTRADICTION 1: the stated cause says the CLI "reads the node's gate heights
via RPC (AUDIT-P2-548) and signs". The code shows otherwise:
- No RPC returns the producer-auth gate. `getNetworkParams`
  (`crates/rpc/src/methods/network.rs:119`) returns no activation height.
- The CLI never checks the producer-auth gate. It signs on every network and at
  every height.
- AUDIT-P2-548 only made the unbonding builder use `NetworkParams::load`
  (`bins/cli/src/unbonding_tx.rs:33-35`), which is a local source.
So the helper the GUI/wallet can reuse is `doli_core::transaction::sign_producer_auth`
(or `finalize_producer_tx`). There is no gate-reading step to reuse.

⚠ CONTRADICTION 2: the wallet AddBond and RequestWithdrawal builders are wrong
below the gate as well, not only unsigned:
- `build_add_bond` never sets `extra_data` (no `AddBondData`). The node refuses
  this with "invalid add bond data" at `crates/core/src/validation/tx_types.rs:282-283`.
- `build_request_withdrawal` writes 4 or 36 bytes (`builder.rs:382-388`), not the
  68-byte `WithdrawalRequestData`. It adds a dummy input (`:392`) and a zero-amount
  output (`:393`). It is refused at `tx_types.rs:401` and `:406-408`. At the
  unbonding gate it also lacks the Unbonding output.
- A producer-auth tail over these payloads still fails `canonical_producer`.
The fix must build the canonical payloads. The wallet crate has no `doli-core`
dependency, so it cannot reuse the core helper without a second implementation.
The GUI already depends on `doli-core`.

Consequence for the REQ-DEF-001 acceptance criterion "builder output below the
gate is unchanged": this can hold only for GUI Registration. It holds there only
if the GUI signs conditionally on the gate, which the CLI does not do. The wallet
AddBond and RequestWithdrawal bytes must change, because their current form is
invalid. See Assumption A1.

Simplest fix that addresses the root cause: build the three GUI producer txs
with the CLI's pure builders and `finalize_producer_tx`, which are already tested
and node-accepted, moved to a crate the GUI can depend on. It works because there
is then one implementation of payload, signing order and digest.

### F2 — zero-Bond exit locks the whole remainder (R-2, AUDIT-P2-603-001)

- Coin gathering: `bins/cli/src/cmd_producer/exit.rs:150-162`. It keeps the
  spendable Normal UTXOs in RPC order, with no sort.
- Coin selection: `bins/cli/src/unbonding_tx.rs:50-60` `take_fee_inputs`. It
  appends coins in the given order until the total reaches the target. For a
  zero-Bond exit the target is `fee + 1` (`:85-87`).
- Output construction: `unbonding_tx.rs:94` sets `gross = total_net + paid_in`.
  `:102-115` makes one Unbonding output of `gross - fee`. It is locked to
  `best_height + unbonding_period + UNBONDING_LOCK_MARGIN` (`:43-47`, margin 10
  at `:10`).
- Summary printer: `exit.rs:98-101` prints `breakdown.total_net`, which is 0 with
  no bonds. It does this before coin selection. `:187-193` prints the lock height
  but not the locked amount. **The cause is confirmed.**
- Constraint: a change output is impossible at the gate (one Unbonding output,
  `crates/core/src/validation/unbonding.rs:160`). The zero-value payout is refused,
  so at least 1 unit must be locked. "Lock only what the exit needs" therefore
  means choosing one coin: the exact `fee + 1` coin if present, otherwise the
  smallest coin above the fee.

### F3 — tenure compares blocks with slots (R-3, SEC-CRYPTO-R-001)

- The predicate: `crates/core/src/validation/slash_window.rs:31-32` computes
  `age` in **slots** (`prev_slot + 1 - offence slot`). `:36-39` computes `tenure`
  in **blocks** (`current_height - max(registered_at, inc_i_255 gate)`). `:42`
  refuses when `tenure == 0 || age > tenure`. With missed slots after the floor,
  `age` grows faster than `tenure`, so an offence inside the registration is
  refused as "predates". **The cause is confirmed.**
- The floor source: `registered_at` is a block height
  (`crates/storage/src/producer/types.rs:75-76`). It is copied into
  `SlashTargetState` by `crates/mempool/src/slash_admission.rs:61-74`
  (`target_state`). `ProducerInfo` stores no registration slot.
- The axis per path (all four judge at `prev_slot + 1` and use the height being
  judged, so they agree with each other):
  - Mempool admission and revalidation: context from `crates/mempool/src/pool.rs:375-376`
    (tip height, `with_prev_slot(current_slot)`). Height + 1 at
    `slash_admission.rs:117` (admission) and `:303-304` (revalidation).
    Predicate calls at `:122`, `:131`, `:340`.
  - Builder: `bins/node/src/node/production/assembly.rs:217-222` (facts),
    `:297-298` (`with_prev_slot(prev_slot)`), `:325` (predicate).
  - Block validation for gossip/sync/reorg/apply/replay:
    `bins/node/src/node/validation_checks/mod.rs:208` `validate_block_for_apply`.
    It builds facts at `:231-236`, sets `with_prev_block(state.best_slot, …)` at
    `:338` and `with_slash_targets` at `:410`. It reaches
    `crates/core/src/validation/block.rs:159` → `slash_state.rs:178`
    `validate_block_slashes` → `:225` → `slash_window.rs`. Apply calls it at
    `bins/node/src/node/apply_block/mod.rs:114` for every mode.
  - Redelivery ledger: `bins/node/src/node/slash_redeliver.rs:63-118`. It judges
    at `best_height + 1` (`:64`) through `admit_own_slash` (`:29-59`, tip height
    and slot at `:34-37`). Any refusal other than Busy/Full/AlreadyExists
    (`:19-24`) calls `forget` (`:112-115`). So the false "predates" refusal
    **permanently deletes** the node's own slash, which breaks INV-SLASH-005.
- This behaviour is a documented trade-off, not an oversight.
  `specs/security_model.md:331` (Tenure caveat) calls the rule "sound, not
  complete". It rejects the floor-header lookup because a snap-synced node may
  not hold that header. The fix therefore needs a floor slot that every node holds
  in consensus state. One existing candidate: Bond UTXOs carry a node-stamped
  `creation_slot` (`bins/node/src/node/apply_block/tx_processing.rs:399`,
  INV-VEST-001). The design choice belongs to the architect.
- Gate: `inc_i_255_slashing_activation_height` is `u64::MAX` on mainnet and
  testnet, so the rule can change under the same gate. No activation height
  moves and no genesis reset is needed.

### F4 — slash target facts are built below the gate (R-4, REV-B3-002, REV-B3-016)

- Block validation: `bins/node/src/node/validation_checks/mod.rs:226-236` calls
  `mempool::slash_admission::slash_targets` for **every** block, with no gate
  check first.
- Builder: `bins/node/src/node/production/assembly.rs:217-222` does the same for
  every build, over all `mempool_txs`.
- The work: `crates/mempool/src/slash_admission.rs:25-58`. For each SlashProducer
  it does one `get_by_pubkey`, a pending-queue scan and one owner UTXO lookup
  (`get_by_pubkey_hash`, `:40-46`). For each Registration/AddBond it decodes the
  payload (`slash_registrant`, `crates/core/src/validation/slash_registration.rs:13-19`)
  and does one `target_state` lookup (`:53-56`). It also takes the producer-set
  and UTXO read locks.
- Below the gate the result is unused: `validate_block_slashes` gates on
  `slash_rule_active` (`slash_state.rs:182`), and so do `validate_slash_target`
  (`:72`) and `validate_not_slashed_registrant` (`slash_registration.rs:27`).
  **The cause is confirmed.**
- The mempool arms already gate first: `registrant_refusal` (`slash_admission.rs:138-146`),
  the slash arm via `slash_fee_exempt` (`:121-122`), and `slash_rule_evictions`
  (`:305-306`). Minor asymmetry: the eviction gate checks only
  `inc_i_255_slashing_activation_height`, while `slash_rule_active` also requires
  the block-signature rule (`crates/core/src/validation/slash_evidence.rs:31-33`).
  The verdict is the same, but lookups can run in that band.
- Existing below-gate parity tests: `bins/node/tests/it/inc_i_255_m1_below_gate.rs`,
  `inc_i_182_m1_below_gate_golden.rs`, `inc_i_257_m1_below_gate_golden.rs`. Lookup
  budget probes: `crates/mempool/tests/it/b3_m2_slash_scan_bound.rs`,
  `b3_m3_replayed_slash_no_scan.rs`, `inc_i_257_m2_slash_targets.rs`. Eviction:
  `crates/mempool/tests/it/inc_i_256_m1_slash_eviction.rs`. New node-level, Full-mode
  and eviction below-gate tests belong next to these.

### F5 — check-producer-set on a real data directory; no census (R-5)

(a) Flags.
⚠ CONTRADICTION 3: `check-producer-set` has **no** own `--network` flag. It is a
unit variant (`bins/node/src/cli.rs:254`). The real problems are these:
- The global `--network` defaults to `mainnet` (`cli.rs:23-25`, `global = true`).
  So a run without `--network` on a test-network directory replays with mainnet
  parameters.
- `--data-dir` (`cli.rs:31-33`) is **not** `global`. So
  `check-producer-set --data-dir …` placed after the subcommand is refused by the
  argument parser. It only works before the subcommand.
- The subcommand that does shadow the global flag is `init`, with its own
  `--network` default `mainnet` (`cli.rs:182-184`).
- The dispatch is at `bins/node/src/main.rs:258-263`. The data dir resolves at
  `:65-70`. This was verified by reading the parser attributes. The binary was
  not run.

(b) Slot derivation at height 1.
- `Node::new_for_replay` (`bins/node/src/node/init.rs:1421`) builds
  `ChainSpec::testnet()` but uses only its genesis hash (`:1427`). It takes
  `ConsensusParams::for_network(network)` (`:1428`), whose testnet
  `genesis_time` is `0` (`crates/core/src/consensus/params.rs:72`). It passes
  `genesis_time_override: Some(params.genesis_time)` (`init.rs:1522`).
- A running node calls `params.apply_chainspec(spec)` (`init.rs:174`, which sets
  `genesis_time` from the chainspec at `params.rs:143-152`). The embedded testnet
  chainspec timestamp is `1777037598` (`chainspec.testnet.json:6`).
- Arithmetic check with a 10 s slot. The failing block's timestamp T satisfies
  T/10 ≈ 178,947,464, so T ≈ 1,789,474,640. Then (T − 1,777,037,598)/10 ≈
  1,243,704. This equals the header's slot 1,243,705 up to rounding of T. So the
  replay derives the slot from genesis time 0, and the live node uses the
  chainspec. **The cause is confirmed.**
- Mainnet is not affected: `ConsensusParams::mainnet()` uses `GENESIS_TIME`
  (`crates/core/src/consensus/constants.rs:26`), and that equals the chainspec
  value. `recover_chain_state` (`bins/node/src/operations/chain.rs:231`, replay
  at `:344`) shares the same constructor and very likely has the same testnet
  defect. This is not checked at runtime.

(c) Census.
- No command lists Exited producers that still hold Bond UTXOs. The loading path
  to share is in `producer_set_check.rs:116-144`: lock probe, scratch dir,
  `StateDb::open_read_only` (`crates/storage/src/state_db/open.rs:257`),
  `get_chain_state` (`queries.rs:409`) and `load_producer_set` (`queries.rs:656`).
- The UTXO read can use `get_utxos_by_pubkey` per bond owner (`queries.rs:75`).
  The owner hash comes from `doli_core::validation::bond_owner`. This avoids
  loading the whole set with `iter_utxos`.

## Regression check (baseline `b4cff010`, released v6.44.2; HEAD `8facb556`)

| Finding | Commits in range on the suspect paths | Introduced or pre-existing |
|---|---|---|
| F1 | GUI/wallet files: **none**. Core/CLI auth: `5a75a0db` (producer-auth rule + CLI signer) | The GUI/wallet code is older than the range. `5a75a0db` created the gap: it added the rule and signed only the CLI. The malformed wallet AddBond/RequestWithdrawal payloads are **older than the range** and were already invalid on v6.44.2. |
| F2 | `f06b5efd` (Unbonding builder), `5a75a0db` (signing), `0229c143` (zero-Bond exit), `21133bf4` (`NetworkParams::load` gate source) | **Introduced by `0229c143`**: the `fee + 1` target and folding `paid_in` into the locked payout. The "You receive" printer is older (unchanged in range); the new branch reused it. `f06b5efd` and `21133bf4` did not cause it. |
| F3 | `0e9eab52`, `4348ca12`, `13be7da2`, `61b027f9`, `f737cc2d`, `f06b5efd`, `16f4e2f0`, `ac186dbd`, `5a75a0db`, `c5a85716`, `af6190ec`, `6977585c`, `04d01092`, `27e86090`, `29633d57` | **Introduced by `af6190ec`** (block tenure, recorded as a caveat in the spec). `6977585c` reworded it to the current `tenure == 0 \|\| age > tenure`. The permanent drop comes from `c5a85716`/`04d01092` (ledger `forget` on a final refusal) meeting this check. The others touch the same files but not the window. |
| F4 | `0e9eab52`, `4348ca12`, `13be7da2`, `61b027f9`, `f737cc2d`, `f06b5efd`, `16f4e2f0`, `5a75a0db`, `c5a85716`, `af6190ec`, `27e86090`, `29633d57`, `8facb556` | **Introduced by `4348ca12`** (unconditional `slash_targets` in the builder/validation). `16f4e2f0` added the owner UTXO lookup. `27e86090`/`29633d57` added the per-registrant lookup. `8facb556` narrowed only the mempool replay arm. |
| F5 | `11dda3ec`, `13be7da2` (main/init, unrelated), `27e86090` (adds `check-producer-set`), `8facb556` (scratch sweep) | **Introduced by `27e86090`**: it is the first tool to use `new_for_replay` on testnet. The genesis-time-0 replay constructor is **older than the range** (`init.rs:1522` is unchanged in range). The flag layout of `cli.rs` is older as well. |

Conclusion: none of the five defects is on released v6.44.2 as a live defect,
except the old wallet payload bug in F1. F1–F5 come from the unreleased gated
work. No gate is pinned, so no released network is exposed.

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-DEF-001 | GUI and wallet producer txs (Registration, AddBond, RequestWithdrawal) carry the canonical payload and the producer-auth signature from the single core helper | Must | - [ ] At the gate, each current GUI/wallet tx is refused with `[ERRTX-PAUTH001]` before the fix and accepted after<br>- [ ] Below the gate the builder output stays byte-equal (see A1 for the scope of this criterion) |
| REQ-DEF-002 | The CLI zero-Bond exit locks only what the exit needs and prints the true locked and returned amounts | Must | - [ ] Tests over at least two coin sets show the locked amount and the summary before and after<br>- [ ] The exit picks the exact `fee + 1` coin, or else the smallest coin above the fee, and prints the true locked and returned amounts |
| REQ-DEF-003 | The slash tenure check compares the offence with the registration on one time axis, and the redelivery ledger never drops a slash because of a missed-slot artefact | Must | - [ ] After a reorg with missed slots, a valid re-offered slash is refused and dropped before the fix, and admitted and mined once after<br>- [ ] Parity on mempool, builder, block validation/apply/replay and the ledger<br>- [ ] Behind the slashing gate |
| REQ-DEF-004 | Block validation and the builder build no slash target facts below the effective slashing height | Must | - [ ] A probe shows key lookups per block below the gate fall from N to 0, with verdicts unchanged<br>- [ ] New below-gate parity tests at node level, in Full mode and for eviction pass |
| REQ-DEF-005 | Offline producer-set tools work on a real network data dir: (a) the check honours the global `--network`/`--data-dir` and replays with the network's chainspec genesis time; (b) a read-only census lists Exited producers holding Bond UTXOs, machine-readable, on the same loading code | Must | - [ ] (a) On a stopped copy of a test-network data dir the check exits 1 before the fix, and prints EQUAL or DIFFERS with a summary line and exits 0 or 2 after; the global flags apply<br>- [ ] (b) A devnet test with a seeded Exited producer holding a Bond UTXO lists it; a clean chain lists none<br>- [ ] A hash of the data copy is equal before and after each tool run |

## Acceptance criteria (detailed)

### REQ-DEF-001
- [ ] Given a devnet node at the producer-auth gate, when the GUI Registration,
  AddBond and RequestWithdrawal txs (current code) are submitted, each is refused
  with `[ERRTX-PAUTH001]` (Registration). AddBond and RequestWithdrawal are refused
  earlier by the structural rule (`tx_types.rs:282`/`:406`). The test records which
  refusal fires.
- [ ] Given the fixed builders, the same three txs are admitted, selected by the
  builder and applied. For RequestWithdrawal this is at and above the unbonding
  gate too, with one Unbonding output to the bond owner.
- [ ] The signature comes from `doli_core::transaction::sign_producer_auth`. No
  second digest implementation exists (grep proof: one definition of
  `PRODUCER_AUTH_DOMAIN`).
- [ ] Below the gate (decision D1 settles A1): the builders sign always, like the
  CLI. For GUI Registration the payload, inputs and outputs stay byte-equal to the
  pre-fix builder; only the 64-byte signature tail is appended to `extra_data`.
  A node below the gate admits the signed form (test). The CLI's unconditional
  signing is unchanged.
- SEC: the genesis hash is taken from the node's `getChainInfo` and parsed with
  the same strict hex parser as the CLI (`chain_genesis_hash`). A malformed value
  aborts the build. The signing key never leaves the wallet process.

### REQ-DEF-002
- [ ] Coin set A = {fee+1, 5·unit, 100·unit} in RPC order {100·unit, 5·unit, fee+1}.
  Before the fix, 100·unit − fee is locked. After, exactly 1 unit is locked and
  only the `fee + 1` coin is spent.
- [ ] Coin set B = {3·unit, 40·unit} (no exact coin). After the fix, the 3·unit
  coin is spent and 3·unit − fee is locked.
- [ ] No spendable coin above the fee: the build fails with the existing
  insufficient-balance error, and nothing is submitted.
- [ ] The summary prints the locked amount, the lock height and the amount that
  stays spendable. "You receive: 0" is never printed when funds are locked.
- [ ] With Bond inputs, below the gate, and on the withdrawal command: output is
  byte-equal to before (golden test next to `bins/cli/tests/it/inc_i_182_m2_cli_exit.rs`).

### REQ-DEF-003
- [ ] Given a devnet chain at the slashing gate with missed slots after the
  accused's floor, and a valid offence after the floor: when the including block
  is rolled back and the ledger re-offers, the slash is refused ("predates") and
  forgotten before the fix. After the fix it is admitted and mined exactly once.
- [ ] Evidence for an offence before the floor is still refused on all four
  paths. Soundness is kept: never a wrongful burn. Decision D3 (v3): the
  registration floor is the smallest stamped slot of the Bond UTXOs owned by
  the accused's bond owner (consensus UTXO state, read in the existing owner
  lookup), applied only while the accused holds no locked collateral; the gate
  bound keeps today's shape against the gate only; the `tenure == 0` refusal
  and the slot-based expiry window stay as they are.
- [ ] Pin-order unit test: the slashing gate is at or above the unbonding gate
  on every network's defaults.
- [ ] The verdict does not depend on whether the node is snap-synced. No floor
  block header is read.
- [ ] The expiry check (`age >= unbonding_period` slots) is unchanged.
- [ ] Below the effective slashing height, every path is bit-identical to HEAD.
- [ ] The ledger forgets a slash only on a refusal that can never become valid.
- SEC: the evidence remains untrusted input; the signature and genesis checks run
  before any state read (INV-SLASH-001, INV-SLASH-007).

### REQ-DEF-004
- [ ] Counting probe: below the effective slashing height, `slash_targets` lookups
  in `validate_block_for_apply` and the builder are 0 per block. Today they are N
  (one per Registration/AddBond/SlashProducer).
- [ ] At and above the gate the facts, and so every verdict, are byte-equal to HEAD.
- [ ] New below-gate parity tests: node-level apply, Full-mode validation, and
  mempool eviction (including the band between the slashing and signature gates).

### REQ-DEF-005
- [ ] (a) A stopped copy of a test-network data dir: before the fix, exit 1 at
  height 1 with "invalid slot derivation". After, EQUAL or DIFFERS, one summary
  line, exit 0 or 2.
- [ ] (a) `--network` and `--data-dir` work before or after the subcommand.
- [ ] (a) The replay uses the network's chainspec genesis time on mainnet,
  testnet and devnet. `recover` keeps working on the same constructor.
- [ ] (b) The census prints one machine-readable record per Exited producer that
  holds at least one Bond UTXO (key, status, bond count, amount) and a summary.
  On a clean chain it prints no records.
- [ ] (b) The census refuses a locked (running) store, like the check.
- [ ] Both tools: a hash of the data copy is equal before and after.
- SEC: the data-dir path is operator input. The store is opened read-only and the
  scratch dir lives under the temp dir with the existing sweep. No path is built
  from on-disk content.

## Trust Boundaries

| Entry point | External data | Consumer |
|---|---|---|
| `getChainInfo` → GUI/wallet | `genesis_hash` (node-supplied) | `producer_auth_digest` (REQ-DEF-001) |
| `getUtxos` → CLI exit | coin list and amounts (node-supplied) | coin selection (REQ-DEF-002) |
| Gossip/RPC SlashProducer | two block headers | `validate_slash` → window (REQ-DEF-003) |
| Operator CLI | `--data-dir` path | read-only store open (REQ-DEF-005) |

## Impact analysis

- `bins/gui/src/commands/producer.rs`: three commands change. Risk medium: there
  are no GUI tests today (`bins/gui` has no `tests/`).
- `crates/wallet/src/tx_builder/builder.rs`: AddBond/RequestWithdrawal builders
  change or are retired. Risk medium (`crates/wallet/tests/tx_builder.rs`,
  `src/tx_builder/tests.rs` assert the current shape).
- `bins/cli/src/unbonding_tx.rs`, `cmd_producer/exit.rs`: risk low. Shared with
  `withdrawal.rs`, which must stay byte-equal.
- `crates/core/src/validation/slash_window.rs` + fact builder: consensus verdict
  behind the gate. Risk high if the slot source differs across nodes.
- `validation_checks/mod.rs`, `production/assembly.rs`: hot path. The change
  removes work and does not add any.
- `init.rs` `new_for_replay`: also used by `recover`.
- Hardware: F4 removes per-block lock and lookup work. The census reads per-owner
  UTXO rows, which is bounded by the number of Exited producers. Neither tool runs
  on a live node.

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies | Status |
|----|------|-----------------|----------------------|--------------|--------|
| M1 | Producer tools sign and exit correctly | `bins/gui/src/commands/producer.rs`, `crates/wallet/src/tx_builder/` (`builder.rs`, `tests.rs`), `bins/cli/src/producer_tx.rs`, `bins/cli/src/unbonding_tx.rs`, `bins/cli/src/cmd_producer/exit.rs`, `crates/core/src/transaction/producer_auth.rs` (read-only reuse) | REQ-DEF-001, REQ-DEF-002 | none | PENDING |
| M2 | Slash time axis, gated lookup, chain-state tools | `crates/core/src/validation/slash_window.rs`, `crates/core/src/validation/slash_state.rs`, `crates/mempool/src/slash_admission.rs`, `bins/node/src/node/validation_checks/mod.rs`, `bins/node/src/node/production/assembly.rs`, `bins/node/src/node/slash_redeliver.rs`, `bins/node/src/cli.rs`, `bins/node/src/main.rs`, `bins/node/src/operations/producer_set_check.rs` (+ a census module beside it), `bins/node/src/node/init.rs` (`new_for_replay`) | REQ-DEF-003, REQ-DEF-004, REQ-DEF-005 | M1 | PENDING |

## Traceability matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-DEF-001 | Must | `inc_i_262_m1_req_def_001_*` (cli it: wiring, gui_registration; node it: current_forms, reference_forms, fixed_forms, wallet_sign) | (architect) | `crates/producer-tx/src/producer_tx.rs` (`build_signed_registration`), `crates/wallet/src/tx_builder/builder.rs` (`build_add_bond`, `build_request_withdrawal`), `crates/wallet/src/tx_builder/mod.rs` (`ProducerSigner`, `WithdrawalRequest`), `bins/gui/src/commands/producer.rs`; wallet transfer per-input signing OPEN (blocked by `serialization_compat::test_m1_transfer_serialization_matches_core`) |
| REQ-DEF-002 | Must | `inc_i_262_m1_req_def_002_*` (cli it: exit_coins, exit_summary, wiring) + `inc_i_182_m2_req_pauth_004_zero_bond_exit_built_and_valid_above_both_gates` | (architect) | `crates/producer-tx/src/unbonding_tx.rs` (`build_withdrawal_tx` one-coin choice, `exit_summary_lines`), `bins/cli/src/cmd_producer/exit.rs` |
| REQ-DEF-003 | Must | `inc_i_262_m2_slot_floor_*`, `inc_i_262_m2_pin_order_*` (core it); `inc_i_262_m2_bond_slot_facts_*` (mempool it); `inc_i_262_m2_reorg_missed_slots::*` (node it) | D3 v3, D3-pin | `crates/core/src/validation/slash_window.rs` (`validate_slash_window`), `crates/core/src/validation/slash_state.rs` (`SlashTargetState::oldest_bond_slot`), `crates/mempool/src/slash_admission.rs` (`slash_targets`) |
| REQ-DEF-004 | Must | `inc_i_262_m2_gated_slash_facts::*` (node it) | D4 | `bins/node/src/node/validation_checks/slash_facts.rs` (`Node::slash_facts`), called from `validation_checks/mod.rs` (`validate_block_for_apply`) and `production/assembly.rs` (builder) |
| REQ-DEF-005 | Must | `inc_i_262_m2_replay_tools::*`, `inc_i_262_m2_replay_epoch_reward::*` (node it) | D5 | `bins/node/src/cli.rs` (global `--data-dir`, `ExitedBondCensus`), `bins/node/src/chainspec_select.rs` (`network_chainspec`, `genesis_producer_set`, shared with `run.rs`), `bins/node/src/node/replay_chainspec.rs` (`Node::new_for_replay`), `bins/node/src/node/init.rs` (`new_for_replay_with_chainspec`), `bins/node/src/operations/producer_set_check.rs` (`open_stopped`), `bins/node/src/operations/exited_bond_census.rs`; `bins/node/src/node/validation_checks/mod.rs` (Replay skips the EpochReward input arms) |

## Existing tests to extend

- CLI producer-auth and exit: `bins/cli/tests/it/inc_i_182_m1_producer_txs.rs`,
  `inc_i_182_m2_cli_exit.rs`, `ldp_m1_cli_gate_source.rs`.
- Node producer-auth: `bins/node/tests/it/inc_i_182_m1_*.rs` (incl.
  `below_gate_signed`, `r1_scope`, `m25_builder_parity`).
- Slashing window/tenure/ledger: `crates/core/tests/it/inc_i_256_m2_window.rs`,
  `bins/node/tests/it/inc_i_256_m2_{tenure,window}.rs`,
  `inc_i_256_m1_slash_redeliver.rs`, `b3_m1_slash_reorg.rs`, `inc_i_255_m*`.
- Live scenarios: `scripts/gauntlet-gs027.sh`, `gauntlet-gs028.sh`,
  `gauntlet-gs029.sh` (+ `gauntlet-gs029-probes.sh`) and their
  `scripts/test_gauntlet_gs02{7,8,9}.sh`.
- check-producer-set: `bins/node/tests/it/b3_m2_residue_check.rs`,
  `b3_m3_residue_check_cleanup.rs`.

## Specs drift detected

- `specs/security_model.md:331` (Tenure caveat) states the missed-punishment
  outcome as accepted. With the ledger drop, the outcome is a lost slash, not
  only a missed one. The caveat must be updated with REQ-DEF-003.
- Brief-level drift: the claim that the CLI reads the producer-auth gate over
  RPC does not match the code (Contradiction 1).

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| A1 | The byte-equality criterion of REQ-DEF-001 applies to GUI Registration only, and only if the GUI signs conditionally. The recommended path signs always, like the CLI (the signed form is admitted below the gate). The criterion then becomes "below the gate the payload, inputs and outputs are byte-equal and the signed form is admitted". | The wallet's old add-bond and withdrawal bytes are invalid today, so they must change. We must choose between "sign only at the gate" and "always sign, like the CLI". | No |
| A2 | All external inputs in the trust-boundary table are untrusted. | Data from the node or from disk is checked before use. | No |
| A3 | The F3 floor slot comes from consensus state every node holds; no stored-byte change. | The fix for the time comparison must not need a chain reset. | No |
| A4 | The F5(b) failing block timestamp is about 1,789,474,640 (derived, not read). | The arithmetic fits the genesis-time-0 cause. | No |

## Design decisions (orchestrator, before the milestone loop)

| # | Decision | Rationale | Rejected alternative |
|---|---|---|---|
| D1 | GUI and wallet producer builders sign always, with the one core helper, exactly like the CLI. The CLI's producer builders become a library that the CLI and the GUI share; the wallet-crate AddBond and RequestWithdrawal functions must produce valid, signed transactions (delegating to that library or fixed in place, whichever changes least). | One code path for every tool; the signed form is admitted below the gate; no gate lookup and no second signing-message implementation. | Sign only at the gate: needs a gate source the GUI does not have, leaves a window where a transaction built below the gate is refused above it, and still cannot keep the wallet's malformed bytes. |
| D2 | REQ-DEF-001 byte-equality applies to GUI Registration: payload, inputs and outputs unchanged, signature tail appended. | The wallet AddBond and RequestWithdrawal bytes are invalid today, so they must change. | Byte-equality for all three builders (impossible). |
| D3 (v3; v1 withdrawn: `BlockHeader` has no height field; v2 withdrawn: registration `bond_entries` store the registration height, only AddBond entries store a slot) | The registration floor is a slot every node holds: `SlashTargetState` gains `oldest_bond_slot: Option<u32>` = the smallest `Output::bond_creation_slot()` over the Bond UTXOs owned by the accused's bond owner, read inside the existing bounded owner lookup of `slash_targets` (the same pass that finds locked Unbonding outputs; no extra lookup). Every Bond UTXO is stamped with its block's slot at apply on both UTXO backends, so for the registration bonds this is exactly the registration block's slot. `None` when the lookup was past the cap or the owner holds no Bond UTXO; with no locked collateral, `None` refuses (nothing burnable, or unknown past the cap — unreachable in a valid block, which holds at most `MAX_SLASH_TXS_PER_BLOCK` slashes). At and above the slashing gate the predicate is: (1) expiry window unchanged (`age >= unbonding_period` in slots); (2) `tenure == 0` refusal unchanged (`current_height == max(registered_at, gate)`); (3) gate bound keeps today's sound shape but against the gate only: refuse when `age > current_height − gate`; (4) registration floor in slots: when the accused holds no locked collateral and `oldest_bond_slot` is `Some(s)`, refuse when the offence slot `< s`. While locked collateral exists (an Unbonding output still locked at the judged height) the registration floor is waived: that collateral is slashable by rule 7, and draining `bond_entries` through RequestWithdrawal must not move the floor past the offence. No entries (legacy or none left) = no registration floor. | Every input is in consensus state or in the evidence: no block header below the tip is read, so the verdict is the same on a snap-synced node. The mixed slots-versus-blocks comparison stays only for the gate, where it is sound and stops binding `unbonding_period` blocks after activation. The waiver closes the spec's own residual (re-registration while old collateral is still locked). | v1 heights from the evidence (no such field). A floor from the oldest Bond UTXO or entry without the waiver: an equivocator adds a bond, withdraws the older ones, and the floor passes the offence. A stored registration slot: changes stored bytes (chain reset). Reading the floor block header: node-dependent (spec forbids). |
| D3-pin | Pin-order constraint: `inc_i_255_slashing_activation_height >= inc_i_257_unbonding_activation_height` on every network, asserted by a unit test over the network defaults. | Below the unbonding gate a RequestWithdrawal pays instantly, so no locked collateral exists and the waiver cannot protect the floor. | None. |
| D4 | REQ-DEF-004 gates the `slash_targets` call itself in block validation and the builder; the facts are built only at and above the effective slashing height. | Verdicts are already bit-identical for all inputs; this removes work only. | Lazy facts inside the predicate (more code, same effect). |
| D5 | REQ-DEF-005: `--data-dir` becomes a global flag like `--network`; the replay constructor applies the network chainspec as the running node does; the census is a sibling read-only subcommand that prints JSON lines and a summary, on the shared loader. | Smallest change that makes both tools work on a real data dir. | A per-subcommand `--network` default (hides the global flag). |

## What I don't understand

- Whether the wallet `sign_and_build` input-signing message equals
  `Transaction::signing_message_for_input`. It hashes its own byte layout. I did
  not verify this. If it differs, the wallet AddBond/RequestWithdrawal inputs fail
  as well.
- Why devnet replay passes the existing residue tests while devnet
  `ConsensusParams` also has `genesis_time: 0`. Possibly the devnet tests
  produce blocks whose slot fits genesis time 0, or the chainspec env applies.
- Which node-independent slot value the architect will choose for the F3 floor,
  and how it treats an accused whose registration Bonds were partly withdrawn.

## Identified risks

- F3 slot source differs across nodes → fork above the gate. Mitigation:
  parity tests on all paths, including a snap-synced fixture.
- GUI rewrite without GUI tests. Mitigation: test the pure builder in a crate
  with tests and keep only RPC glue in the Tauri command.

## Out of scope (Won't)

- Pinning any activation height.
- GUI Exit (`producer.rs:394`): Exit is not a producer-auth type.
- Changing the CLI's unconditional signing.

## Consensus-shape checklist

- REQ-DEF-001/002/005: client or offline tools only. No consensus change.
- REQ-DEF-003: (1) yes, a user can submit a slash; (2) yes; (3) not bit-identical
  above the gate. The existing unpinned `inc_i_255` gate covers it, so no new
  height and no genesis reset are needed.
- REQ-DEF-004: verdicts are bit-identical for all inputs. No gate is needed.

━━━ RESOURCE COST ━━━
CPU: lower per block (F4). F3 adds no storage read beyond what the architect picks.
Memory: census holds the producer set plus per-owner UTXO rows (offline).
IO: read-only tools. No live-node write path is added.
Locks: F4 removes two read-lock acquisitions per block below the gate.
━━━━━━━━━━━━━━━━━━━━━

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.75, basis=verified — every cause read at HEAD with file:line; F5(a) from parser attributes, F5(b) by arithmetic, binary not run)
Reasoning: All five defects have deterministic, localized causes inside one
predicate, builder or tool. Brittleness is LOCALIZED (1/5). No code reading
contradicts the existence of a defect. Three details differ from the brief: the
CLI does not read the gate over RPC and signs always; the wallet AddBond and
RequestWithdrawal payloads are malformed even below the gate; check-producer-set
has no own `--network` flag (`--data-dir` is not global and `--network`
defaults to mainnet). These details widen the M1 scope and A1 needs a user
decision. F3 has one open design decision (a node-independent floor slot,
constrained by `specs/security_model.md:331`). Settle it before the M2
developer step.
━━━━━━━━━━━━━━━━━━━━━━
