# Requirements: INC-I-203 — over-cap AddBond admission gap

Analyst pass for `/omega-doctor --fix --incident=INC-I-203`. RUN_ID=543. Branch `main`.
Prior sessions worked on `fix/inc-i-202-m2-publish-gate` and never produced a diagnosis report;
every structural claim below was re-derived on `main` from the code graph plus targeted reads.

## Scope

- `crates/core/src/validation/tx_types.rs` — `check_addbond_cap` (the shared rule)
- `bins/node/src/node/validation_checks.rs` — `validate_block_economics` (the only enforcement today)
- `crates/mempool/src/{pool.rs, holdings.rs, withdrawal_holdings.rs}` — admission + revalidate
- `bins/node/src/node/production/{assembly.rs, withdrawal_holdings.rs, poison.rs}` — selection + containment
- `crates/rpc/src/methods/{transaction.rs, balance.rs}` — submit + the Spendable=0 symptom
- `bins/cli/src/cmd_producer/bonds.rs` — client-side construction
- `crates/core/src/network_params/defaults.rs` — the already-pinned activation height

Explicitly NOT in scope: `/mainnet/scripts/auto-bond-nX.sh` (outside this repo).

## Summary (plain language)

A producer can build and submit an "add bonds" transaction that pushes it over the 3000-bond limit.
The network already knows this transaction is illegal — the rule has been live on mainnet and testnet
since block 0 — but the rule is only checked at the very last moment, when a whole block is being
validated. So the transaction is accepted by the wallet, accepted by the node, spread to every other
node, and then packed into a block by whichever producer happens to be building next. That block is
thrown away. The producer loses its turn. Meanwhile the transaction sits in every mempool for up to
14 days, and because it is holding the producer's coins as inputs, the producer's spendable balance
reads zero the whole time.

Nothing forks and no money is lost — a containment guard added for a different incident catches it.
But a producer loses a build slot each time, and the operator's funds are frozen for two weeks.

The fix is to check the rule earlier, at the three places that already have exactly the data needed.

---

## A. Graph verification of the stored causal chain

Tooling: `graphify explain` / `graphify path` on `graphify-out/graph.json` (26 MB, built 2026-09-02 04:47),
`.claude/scripts/blast.py`. Constants are weak nodes in this graph (`blast.py` on
`MAX_BONDS_PER_PRODUCER` returns 0 dependents with a label-substring fallback warning), so the constant
itself was resolved by grep and every FUNCTION-level claim by the graph.

| # | Stored claim | Verdict | Current `file:line` |
|---|---|---|---|
| 1 | `validate_block_economics` is the ONLY enforcement point | **CONFIRMED** | Sole production caller of `check_addbond_cap` is `bins/node/src/node/validation_checks.rs:1212`. Graph: `check_addbond_cap` = `crates/core/src/validation/tx_types.rs:515`, degree 9, **all 6 inbound `calls` edges are test functions** in `bins/node/tests/addbond_cap_overflow.rs`. Grep corroborates: zero other non-test call sites. |
| 1b | Line discrepancy `1212` vs `L476` | **RESOLVED — both correct** | `validate_block_economics` opens at `bins/node/src/node/validation_checks.rs:476` (graph node); the `check_addbond_cap` call sits inside it at `:1212` (stored record). Not a contradiction. |
| 1c | Other `MAX_BONDS_PER_PRODUCER` readers exist | **CONFIRMED, none are AddBond enforcement** | `crates/core/src/consensus/constants.rs:390` (def, =3000); `consensus/bonds.rs:123`; `validation/registration.rs:135` (Registration, not AddBond); `storage/src/producer/info.rs:294,316` (`add_bonds` **clip** at epoch flush — the legacy silent-clip path); `wallet/src/tx_builder/{builder.rs:313,fees.rs:60}` (single-tx count only); `bins/cli/src/cmd_producer/delegation.rs:22` (DelegateBond). None consult the producer's *current* bond count. |
| 2 | Mempool `add_transaction` reaches no bond-cap check | **CONFIRMED (NO-EDGE)** | `crates/mempool/src/pool.rs:399`, degree 34. Its validation call is `validate_transaction()` at `:462`. `graphify path crates_mempool_src_pool_mempool_add_transaction -> check_addbond_cap` = **no directed path**. `validate_add_bond_data` (`crates/core/src/validation/tx_types.rs:~440-483`) carries the comment "*New total doesn't exceed MAX_BONDS_PER_PRODUCER … done at node level*" and performs no such check. |
| 3 | `select_for_block` is in `production/assembly.rs` | **REFUTED (file)** | `select_for_block` is `crates/mempool/src/pool.rs:1035`, a `Mempool` method (degree 6). `bins/node/src/node/production/assembly.rs:178` merely *calls* it. |
| 3b | Block selection reaches no bond-cap check | **CONFIRMED (NO-EDGE)** | `pool.rs:1035-1069` is pure CPFP fee-rate sort + size budget + ancestor closure. `graphify path select_for_block -> check_addbond_cap` = no directed path. The builder's only skip gate is `wd_parity.allow()` at `assembly.rs:323`, which returns `Ok(())` for every `tx_type != RequestWithdrawal` (`production/withdrawal_holdings.rs:147`). |
| 4 | RPC `send_transaction` validates then broadcasts unconditionally | **CONFIRMED** | `crates/rpc/src/methods/transaction.rs:165`. AddBond is not `is_zero_flow()` (it has inputs and outputs), so it takes the `add_transaction(tx, &utxo_set, current_height)` lane at `:218`. `(self.broadcast_tx)(tx)` at `:225` runs unconditionally on `Ok`. `graphify path send_transaction -> check_addbond_cap` = no directed path. |
| 5 | CLI prints success without checking the cap | **CONFIRMED, and worse than recorded** | `bins/cli/src/cmd_producer/bonds.rs:9` `handle_add_bond`. Checks: `1..=10000` on count (`:20`), registration membership (`:32-34`), balance (`:74`). It **already fetches `rpc.get_producers(false)` at `:32`** — the response carries `bond_count` — and never reads it. Prints `Bonds added successfully!` at `:129` on any RPC `Ok`. |
| 6 | Self-apply in `ValidationMode::Light` before broadcast + `[BLOCK_POISON]` rollback/purge | **CONFIRMED** | `bins/node/src/node/production/mod.rs:620` `apply_block(block.clone(), ValidationMode::Light)`; broadcast at `:657-658`. Failure arm calls `handle_failed_self_apply` in `bins/node/src/node/production/poison.rs`, which purges the block's TXs from the local mempool **first**, then requests `rollback_one_block(RollbackAuthority::ProductionSelfApply{..})`, emitting `POISON_CONTAINMENT{rolled_back|tip_kept}`. This is INC-I-204 M4.2, not the 2026-03-25 NFT guard the stored record credits (that guard is `remove_registration_txs` / the NFT purge helper at `crates/mempool/src/pool.rs:1076-1090`). |

### The finding the stored record missed

**The mempool CAN see the ProducerSet, and already does.**
`crates/mempool/src/holdings.rs` (shipped for INC-I-180 M2) defines:

- `ProducerHoldings { bond_count, pending_addbond, withdrawal_pending }` — `holdings.rs:19-24`
- `HoldingsSources { live: Option<Arc<tokio::sync::RwLock<ProducerSet>>>, snapshot: Option<HoldingsSnapshot> }` — `holdings.rs:64-71`
- `HoldingsSources::lookup()` — `holdings.rs:75-101`, live handle via non-blocking `try_read`, snapshot fallback
- `of_producer_set()` — `holdings.rs:105-114`, reads `info.bond_count` + `set.pending_addbond_count(pk)`

Wired into the mempool by `Mempool::share_producer_holdings` (`pool.rs:289`) and
`share_live_producers` (`pool.rs:305`), and stored as `producer_holdings: HoldingsSources` (`pool.rs:216`).

`ProducerHoldings.bond_count` and `.pending_addbond` are **exactly and only** the two terms
`check_addbond_cap` needs. The plumbing this ticket appeared to require already exists.

On the builder side the twin is `bins/node/src/node/production/withdrawal_holdings.rs::WithdrawalParity`,
which **already maintains `in_block_addbond: HashMap<PublicKey, u32>`** (`:24`, populated in `accept()`
at `:140-155` with the same "count Bond outputs" expression the gate uses at `validation_checks.rs:1208-1211`).

### The other finding the stored record missed

`addbond_cap_enforcement_activation_height` is **already `0`** on mainnet
(`crates/core/src/network_params/defaults.rs:160`) and testnet (`:449`), `u64::MAX` on devnet (`:711`).
The consensus rule has been live on both real networks since block 0 (INC-I-080 originally pinned mainnet
254_344, since collapsed to 0). **Nothing about the validity rule changes in this ticket.**

### Documentation drift flagged

`crates/network/src/gossip/staleness.rs:116-118` asserts "`BroadcastTransaction` is emitted only on
RPC submission … never re-publishing from the mempool". `bins/node/src/node/validation_checks.rs:1284`
re-broadcasts every gossip-received transaction that admission accepts. The *periodic* claim holds
(`periodic.rs:568` only calls `expire_old`), but "only on RPC submission" is wrong. Not load-bearing
for this fix; recorded so the next reader does not trust it.

---

## B. The open contradiction: testnet poisons, mainnet logged zero

### What the code rules OUT

Every mechanistic explanation offered as a candidate is **refuted** by the code:

| Candidate | Verdict | Evidence |
|---|---|---|
| Mempool selection order skips it | REFUTED | `pool.rs:1035-1069` sorts by `effective_fee_rate()` descending. An AddBond carries a real fee; there is no ordering rule that permanently excludes a tx. |
| Fee / priority filter | REFUTED | `MempoolPolicy::mainnet() == default()`, `min_fee_rate: 0` (`crates/mempool/src/policy.rs:26,38`). No fee floor to fall under. |
| Size / gas cap | REFUTED | `max_tx_size = 600 KB` (`policy.rs:31`). The mainnet toxic tx requests ~4 bonds → a few hundred bytes. |
| Per-tx retry backoff | REFUTED | No backoff exists anywhere in `select_for_block` or the assembly loop. |
| Poison blacklist | REFUTED | `poison.rs` `remove_transaction`s the block's TXs and keeps no denylist. A re-received tx would be re-admitted. |
| Age / expiry rule | REFUTED for the window | `max_age = 14 days` (`policy.rs:32`), applied by `expire_old()` (`pool.rs:1260`, called from `periodic.rs:568`). A 23 h window is far inside it. |

### What the code DOES explain — burnout

The poison cost is **one-shot per (producer, toxic tx) pair**, not recurring:

1. `poison.rs` purges the tx from the **local** mempool and nothing re-adds it.
2. There is **no periodic mempool rebroadcast** (`staleness.rs:116`; `periodic.rs` touches the mempool
   only for `expire_old`).
3. Re-receipt requires a peer to re-publish. `handle_new_transaction`
   (`validation_checks.rs:1235-1245`) returns early on `mempool.contains(&tx_hash)` **without**
   re-broadcasting, so nodes that still hold the tx never re-publish it. The flood is one wavefront.

So after injection, every producer that holds the tx poisons **once**, within roughly
(active producers × slot time) of injection, and then the event rate drops to zero permanently.
The testnet reproduction (h=77780) observed the wavefront live. A mainnet window opened ~23 h after
injection would legitimately show zero.

### Honest verdict: UNRESOLVED EVIDENCE GAP

The burnout mechanism is code-supported and sufficient, but it is **not proven** to be what happened
on mainnet, and a competing explanation is equally consistent: the mainnet toxic txs may never have
entered producer mempools in the first place (per the recorded symptom, `Spendable` reads 0 after the
first submission, so the wallet cannot fund a second AddBond — the hourly cron line in `auto-bond.log`
may be intent printed before the wallet fails).

Distinguishing them requires mainnet logs **from the injection window**, which were never captured;
only a later 23 h window was sampled, and only on 3 of the fleet's hosts. Additionally, per
`~/.claude/.../feedback_logs_in_files_not_journalctl.md`, `journalctl` on those hosts carries lifecycle
events only — if the "zero BLOCK_POISON" sweep read journalctl rather than the node log files, the
observation is void regardless.

**Declared gap.** No claim in this document depends on resolving it. Concretely:

> ⚠ Any statement of the form "every mainnet producer selects the toxic tx" is **UNPROVEN**.
> The fix below is justified by the testnet reproduction (5/5 producers poisoned, h=77780) and the
> 2026-09-02 re-confirmation (3 poison events at h=88055) — both on the **local testnet**. The
> incident entry's "n2/n4/n5" are testnet node names on 127.0.0.1, not mainnet hosts.

The mainnet-side claim that IS proven from code is different and stronger — see the Spendable=0
mechanism in the Impact Analysis.

### ⚠ Contradiction stop, resolved

The stored record's own two halves disagree: "*it propagates network-wide … becomes a latent trap*"
versus "*mainnet producers are apparently not selecting these TXs*". Resolution: propagation is
confirmed from code (RPC broadcasts, gossip forwards, admission accepts); **repeated** selection is
not, because purge is sticky. Both halves are compatible once burnout is understood. No further
contradiction remains.

---

## C. Architecture Context

### Module Boundaries

- **`bins/cli/src/cmd_producer/bonds.rs`** — builds and signs the AddBond. Depends on: `crates/wallet`
  (`TxBuilder::build_add_bond`), `rpc_client`. Depended by: `cmd_producer/dispatch.rs:16`. Holds a full
  producer list (`:32`) and a UTXO list (`:50`); consults neither for headroom.
- **`crates/rpc/src/methods/transaction.rs`** — `send_transaction`. Depends on: `Mempool`, `UtxoSet`,
  `broadcast_tx` closure. No producer state of its own; it delegates all verdicts to the mempool.
- **`crates/mempool`** — admission and residency. Depends on: `doli_core::validation`, `storage::UtxoSet`,
  and (INC-I-180 M2) `storage::ProducerSet` through `HoldingsSources`. Depended by: node, RPC.
- **`crates/network`** — gossip. No transaction semantics; identity-dedup only (`staleness.rs`).
- **`bins/node/src/node/production/`** — assembly + containment. Depends on: mempool selection,
  `ProducerSet`, `UtxoSet`. `assembly.rs` deliberately resolves producer holdings **before** entering
  the UTXO guard (`:183-195`) to avoid joining apply's `utxo→producers` order to rollback's
  `producers→utxo`.
- **`bins/node/src/node/validation_checks.rs`** — the consensus gate. Authoritative. 1614 lines.
- **`crates/core/src/validation/tx_types.rs`** — `check_addbond_cap`, the shared rule expression.
- **`crates/storage/src/producer/`** — `ProducerSet`, `ProducerInfo::add_bonds` (legacy clip path).

### Data Flow Through the Affected Area

```
CLI handle_add_bond (bonds.rs:9)
  └─ get_producers (:32, HAS bond_count, unused) ─ get_utxos (:50) ─ build_add_bond
     └─ RPC send_transaction (transaction.rs:165)
        ├─ Mempool::add_transaction (pool.rs:399)
        │    ├─ size / duplicate guards
        │    ├─ ValidationContext + validate_transaction (:462)  ← NO bond-cap arm
        │    └─ withdrawal_holdings_verdict (:573)               ← RequestWithdrawal ONLY
        └─ (self.broadcast_tx)(tx) (:225)  ── UNCONDITIONAL
           └─ gossip → peer handle_new_transaction (validation_checks.rs:1235)
              └─ Mempool::add_transaction  (same gaps) → re-broadcast (:1284)

producer slot → build_block_content (assembly.rs:9)
  ├─ Mempool::select_for_block (pool.rs:1035)   ← fee-rate + size only
  ├─ WithdrawalParity::load (production/withdrawal_holdings.rs:48)  ← RequestWithdrawal/Exit ONLY
  ├─ selection loop → wd_parity.allow (assembly.rs:323)             ← RequestWithdrawal ONLY
  └─ wd_parity.accept (:161)  ← ALREADY tallies in_block_addbond
     └─ apply_block(Light) (production/mod.rs:620)
        └─ validate_block_economics (validation_checks.rs:476)
           └─ check_addbond_cap (:1212)  ←── THE ONLY ENFORCEMENT
              └─ Err → handle_failed_self_apply (poison.rs) → purge + rollback attempt
                 (broadcast at mod.rs:657 never reached)
```

### Where "current" and "pending" bond counts live at each stage

| Stage | `bond_count` source | `pending_addbond` source | Reachable? |
|---|---|---|---|
| CLI | `rpc.get_producers()` response, `bonds.rs:32` | not fetched | **YES — already in hand** |
| RPC `send_transaction` | none directly | none | via mempool only |
| Mempool admission | `HoldingsSources::lookup` → `ProducerInfo::bond_count` | `ProducerSet::pending_addbond_count` (`set_core.rs:214`) | **YES — `holdings.rs`, already wired** |
| Mempool `revalidate` | same channel | same channel | **YES — `pool.rs:1276`** |
| Gossip | none | none | no (and must not) |
| Block assembly | `WithdrawalParity.holdings` via `of_producer_set` under the producer guard | same struct | **YES — `production/withdrawal_holdings.rs:48`** |
| Block validation | `producers.get_by_pubkey(pk).bond_count` | `pending_addbond_count` + in-block tally | YES (authoritative) |
| `apply_block` | mutation is **DEFERRED** to the epoch boundary (`PendingProducerUpdate::AddBond`) | — | — |

**The crux is settled: the mempool CAN see the ProducerSet.** The premise that it cannot — which would
have forced the fix into the builder only — is false on `main`.

### Architectural Constraints and Invariants

- **INV-CONSENSUS-002** (`incidents INC-I-080`): post-AH, no applied block may contain an AddBond where
  `bond_count + pending + in_block_prior + requested > MAX_BONDS_PER_PRODUCER`; rejection must precede
  any state mutation ("no orphan Bonds"). Pre-AH the clip path is preserved bit-identically.
  **This fix does not touch that rule.**
- **INV-PARAMS-001** / CLAUDE.md: `addbond_cap_enforcement_activation_height` is `0` on mainnet and
  testnet and has been crossed. It is **IMMUTABLE**. Do not touch it.
- **INV-DEPLOY-001 (INC-I-062)**: block-content changes need an activation height. Argued in §E.
- **Lock discipline (INC-I-180 M2 / S1)**: assembly must resolve producer holdings under the producer
  guard, drop it, then take the UTXO guard. `HoldingsSources::lookup` uses non-blocking `try_read` so
  admission can never deadlock against an `apply_block` writer (`holdings.rs:76-79`). Any new lookup
  must obey both.
- **Fail-open at admission (`holdings.rs:9-11`)**: `HoldingsLookup::Unavailable` must mean *skip the
  check*, never *reject*. Over-rejection at admission is censorship.
- **Empty-snapshot rule (`holdings.rs:85-95`)**: absence from an *empty* snapshot is `Unavailable`,
  not `Unregistered`, or `new_for_test`/`new_for_replay` nodes reject everything.
- **CLAUDE.md #0**: no genesis reset. Forward-only.

### Blast Radius (graph, `blast.py --hops 1` and `--hops 2`)

`blast.py` warns that Rust receiver-method calls are unresolved cross-file (graphify#2234), so counts
are lower bounds; each was cross-checked with grep, and the grep found no dependents the graph missed
beyond the known `self.method()` blind spot recorded in
`~/.claude/.../reference_graphify_rust_method_blind_spot.md`.

**Direct (functions a fix would modify):**

- `withdrawal_holdings_verdict` (`crates/mempool/src/pool.rs:314`) — hops 1: **3 dependents** —
  `pool.rs:399 add_transaction`, `pool.rs:1276 revalidate`, `pool.rs:161 Mempool` (method edge).
  hops 2: **33** — all of them `crates/mempool` test functions plus `bins/node/src/node/mod.rs:98 Node`
  and `crates/mempool/tests/inc_i_147_validation_parity.rs:271 mempool_verdict`.
- `build_block_content` (`bins/node/src/node/production/assembly.rs:9`) — hops 1: **1** (`Node` method
  edge). Grep: only `bins/node/tests/inc_i_081_incomplete_store_aborts_slot.rs` and the production
  slot path in `production/mod.rs`.
- `of_producer_set` (`crates/mempool/src/holdings.rs:105`) — hops 1: **2** —
  `crates/mempool/src/holdings.rs:75 lookup`, `bins/node/tests/it/inc_i_180_allowance_parity.rs:200`.
- `handle_add_bond` (`bins/cli/src/cmd_producer/bonds.rs:9`) — hops 1: **1** —
  `bins/cli/src/cmd_producer/dispatch.rs:16`.
- `check_addbond_cap` (`crates/core/src/validation/tx_types.rs:515`) — hops 1: **6**, all tests in
  `bins/node/tests/addbond_cap_overflow.rs`. Adding callers cannot break existing ones: the signature
  is unchanged.

**Indirect (consumers of the affected outputs):**

- Anything reading a produced block: `apply_block`, gossip peers, `crates/storage` block store,
  archiver, explorer. Affected only by *which mempool txs a block carries* — never by block validity.
- `crates/rpc/src/methods/balance.rs` — `getBalance` / `getUtxos` results change once toxic txs stop
  residing in the mempool (funds become spendable again). This is the intended effect.
- `crates/mempool/tests/it/inc_i_180_admission_parity.rs` and
  `bins/node/tests/it/inc_i_180_allowance_parity.rs` — parity suites over the same functions; must
  stay green.

### Brittleness Check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 0/5
Details:
  1. Cross-module blast radius (3+ unlinked modules)? NO — mempool and node/production
     already share the `mempool::holdings` channel; they are directly linked today.
  2. Invariant gaps? NO — INV-CONSENSUS-002 already states the exact rule; the shared
     expression already exists as `check_addbond_cap`.
  3. Data-flow reversal? NO — ProducerSet already flows INTO the mempool (pool.rs:305)
     and into assembly (assembly.rs:193). No new direction.
  4. Shared mutable state without an owner? NO — ProducerSet has one owner (Node) and is
     read through a non-blocking try_read plus a published snapshot.
  5. Contract absence? NO — `HoldingsLookup` / `ProducerHoldings` is an explicit contract
     with a parity test suite locking it to the gate.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## D. Impact Analysis

### Existing Code Affected

| File / module | How it is affected | Risk |
|---|---|---|
| `crates/mempool/src/withdrawal_holdings.rs` | New sibling module or a new arm; existing `check()` untouched | low |
| `crates/mempool/src/pool.rs` (1818 L) | `withdrawal_holdings_verdict` (:314) gains an AddBond branch; both call sites (:573, :1301) inherit it | medium — the file is the mempool's core and is already over budget |
| `bins/node/src/node/production/withdrawal_holdings.rs` (209 L) | `load()` must also resolve AddBond producers; `allow()` gains an AddBond arm | low |
| `bins/node/src/node/production/assembly.rs` (669 L) | No structural change; `wd_parity.allow()` at :323 starts returning `Err` for over-cap AddBonds | low |
| `bins/cli/src/cmd_producer/bonds.rs` (209 L) | Headroom guard before building | low |
| `bins/node/src/node/validation_checks.rs` (1614 L) | **UNCHANGED** — this is the point | none |
| `crates/core/src/validation/tx_types.rs` | **UNCHANGED** — `check_addbond_cap` gains callers only | none |
| `crates/core/src/network_params/` | **UNCHANGED** — no new height, no re-pin | none |

### What Breaks If This Changes

- **Producers whose AddBond is now rejected at admission.** Today they get `Bonds added successfully!`
  then silence for 14 days with frozen funds. After: an immediate `ADDBOND_CAP_EXCEEDED` RPC error and
  their funds are never touched. Strictly better. *Mitigation: none needed; the error names the cap.*
- **The INC-I-180 parity suites.** `withdrawal_holdings_verdict` gains a branch. If the AddBond arm is
  not strictly additive it could alter a withdrawal verdict. *Mitigation: the new arm must key on
  `tx_type == AddBond` and return `Ok(())` for every other type before touching anything.*
- **`WithdrawalParity::load`.** Extending the `named` match to include `TxType::AddBond` enlarges the
  `holdings` map, which the withdrawal allowance also reads. Loading MORE producers cannot change an
  existing withdrawal verdict (a producer either was already loaded or was absent, and absence
  produces `ECON_WITHDRAWAL_UNKNOWN_PRODUCER` only for the *named withdrawal* producer, which was
  always loaded). *Mitigation: an explicit test that a withdrawal verdict is unchanged when an
  unrelated AddBond is in the candidate set.*
- **Live toxic residents.** Once `revalidate` evicts them, `getUtxos`/`getBalance` for those producers
  change within one block. That is the desired unfreeze, but any operator dashboard that cached
  "pending AddBond" will see it vanish. *Mitigation: none; log the eviction at `warn!` as INC-I-180 does.*

### Regression Risk Areas

- `crates/mempool/tests/it/inc_i_180_admission_parity.rs` — the admission/gate parity contract.
- `bins/node/tests/it/inc_i_180_allowance_parity.rs` — `allowance_with` must remain the withdrawal
  expression; **the AddBond check must NOT use `allowance_with`** (see the parity note below).
- `bins/node/tests/addbond_cap_overflow.rs` — the six `check_addbond_cap` truth-table rows.
- `bins/node/tests/fork_recovery.rs` and the poison containment path — build-slot behaviour changes
  from "build, poison, rebuild" to "build clean once".

### ⚠ Parity note that will bite the developer

The gate's AddBond expression is `current + pending + in_block_prior + requested > MAX`
(`validation_checks.rs:1197-1216`). It does **NOT** subtract `withdrawal_pending`.
`ProducerHoldings::allowance_with()` (`holdings.rs:38-45`) is the *withdrawal* expression and DOES
subtract it. Using `allowance_with` for the AddBond check would be a silent parity break.
The AddBond check must read `h.bond_count.saturating_add(h.pending_addbond)` and pass those to
`check_addbond_cap` directly.

---

## E. Consensus-shape three-question checklist (CLAUDE.md / INV-12)

**Q1 — Can a user-submittable tx reach this path?**
**YES.** `AddBond` is submitted through `sendTransaction` (`crates/rpc/src/methods/transaction.rs:165`)
and through gossip (`validation_checks.rs:1235`).

**Q2 — Can a producer-action or attestation pattern reach it?**
**YES.** AddBond is a producer action; the toxic transaction in this incident was produced by a
producer's own hourly cron.

**Q3 — Is the new behavior bit-identical for ALL reachable inputs?**

Split, because the answer differs by dimension. Argued from the validity rule, not asserted:

*Block ACCEPTANCE (the consensus rule) — **bit-identical, YES.***
`validate_block_economics`, `check_addbond_cap`, `apply_block`, the ProducerSet mutation and the
state root are all untouched. For every block `B` and every height `h`, `accept(B, h)` before and
after this change is the same function. The filter therefore cannot make a currently-valid block
invalid or vice versa. Formally: `addbond_cap_enforcement_activation_height` is `0` on mainnet
(`defaults.rs:160`) and testnet (`:449`), so the predicate "contains an over-cap AddBond ⇒ invalid"
holds at **every** height on both networks. The transactions the filter drops are exactly the
transactions that **can never appear in any valid block at any height**. Dropping them removes no
valid block from the buildable set.

*Block CONTENT (what a producer packs) — **NOT bit-identical, NO.***
There is one input class where it genuinely differs: a producer at `bond_count = 2999` with two
AddBonds `A(+1)` and `B(+1)` resident. Each is individually within the cap; jointly they are not.
- **Today:** the builder packs both; `validate_block_economics`'s `in_block` tally catches the pair;
  the block is invalid; poison fires.
- **After:** the builder packs `A`, skips `B`; the block is **valid**.
This is a real behaviour change and it is called out as required. But note its direction: the old
behaviour produced a block **no node would ever accept**; the new one produces a block **every node
accepts**. No previously-accepted block is lost.

**Verdict on the checklist:**
`(Q1|Q2) = YES` and `Q3 = NO` **for block content only**, `Q3 = YES` for the acceptance function.
The checklist's activation-height trigger targets *consensus-visible computation* — the acceptance
function — which is unchanged. **No NEW activation height is required, and none may be added: the
existing `addbond_cap_enforcement_activation_height` is already `0` and IMMUTABLE.**

**REQ-BOND-005 makes this rigorous rather than a judgement call:** every new check site is gated on
the **existing** `addbond_cap_enforcement_activation_height`, so the node-local filter is active
exactly when — and only when — the consensus rule it mirrors is active. On devnet (`u64::MAX`,
`defaults.rs:711`) both stay off together. Zero new params, zero new pins, no mainnet AH decision
session, no re-pin of an immutable height.

### Second deploy question — does this change block CONTENT? (INC-I-062 / INV-8)

**YES, it changes what a producer packs — and NO, a synchronized deploy is NOT required.**

INV-DEPLOY-001 fires when block-content changes create *competing valid blocks during the
mixed-version window*. The mechanism it guards against is two nodes computing **different bytes for
the same consensus-visible field** (presence_root, coinbase shape, tx ordering, header fields) — a
field that other nodes validate against their own recomputation.

The field changed here is not of that class. "Which mempool transactions I include" is:
1. **Already non-deterministic across nodes** — every node has a different mempool, different arrival
   order, and `select_for_block` sorts by `effective_fee_rate()` over that local set. No node
   recomputes another node's selection.
2. **Not validated as a set** — there is no rule of the form "a block must contain all valid mempool
   transactions". Grep of `validate_block_economics` and `validate_block_for_apply` finds no such
   constraint.
3. **Produced by exactly one node per height** — a producer builds only in its own scheduled slot.
   Two competing valid blocks at one height require equivocation, not a version split. An old-binary
   producer and a new-binary producer never build the same height.

During a rolling deploy: an old-binary producer packs the pair and poisons (exactly today's
behaviour); a new-binary producer packs one and succeeds. Both outcomes are already reachable today
and both leave the chain consistent, because every node's **acceptance** function is identical.

**Deploy verdict: ROLLING DEPLOY IS SAFE. No synchronized stop-all/start-all. No `HardForkSchedule`
entry. No `CURRENT_PROTOCOL_VERSION` bump** (the EpochState serialization format does not change —
CLAUDE.md, INV-4).

*Recorded counter-precedent, deliberately not followed:* the structurally identical INC-I-180 builder
skip WAS gated behind a fresh `withdrawal_holdings_gate_activation_height` (mainnet 317_861,
`defaults.rs:161`). That gate is justified there because the withdrawal admission check *substitutes*
mempool-wide state for block-local state and can over-reject (`crates/mempool/src/withdrawal_holdings.rs:4-14`).
The AddBond check has no such substitution — see the strict-subset proof in §F — so it needs no new
height of its own. Reusing the existing AddBond height gives the same lockstep property at zero cost.

---

## F. SSF — the one recommendation

> **Extend the INC-I-180 holdings channel that already carries `bond_count` and `pending_addbond` into
> the mempool and the block builder with an AddBond arm that calls the already-shipped
> `check_addbond_cap`, gated on the already-pinned `addbond_cap_enforcement_activation_height`.**

It works because every part already exists and is already tested: the rule
(`crates/core/src/validation/tx_types.rs:515`), the data channel
(`crates/mempool/src/holdings.rs`), the builder's in-block tally
(`bins/node/src/node/production/withdrawal_holdings.rs:24`), the two call sites
(`crates/mempool/src/pool.rs:573,1301`), and the activation gate
(`crates/core/src/network_params/defaults.rs:160,449,711`). Nothing new is invented; a `RequestWithdrawal`-shaped
hole is filled with the AddBond shape beside it.

### Why this cannot censor (the strict-subset proof)

The gate rejects when `current + pending + in_block_prior + requested > MAX`.
The admission/builder check evaluates `current + pending + requested > MAX`.
Since `in_block_prior >= 0`, the admission total is **always ≤** the gate total for the same
`(current, pending)`. Admission therefore rejects a **strict subset** of what the gate rejects.
Unlike the withdrawal case — which substitutes mempool-wide state for block-local state and can
over-reject (`crates/mempool/src/withdrawal_holdings.rs:4-14`) — the AddBond check has **zero
over-rejection** relative to consensus at fixed state.

The one residual: `current`/`pending` can change between admission and inclusion. Headroom grows only
when a withdrawal or exit **flushes at an epoch boundary**. A producer whose AddBond was rejected then
re-submits. Cost: one re-submission, no fee burned, no UTXO consumed (rejection at admission never
enters the mempool). Bounded and self-announcing.

### What was considered and rejected

- **Fix the ops script only** — it is outside this repo, and it does not stop any other client, the
  RPC, or gossip from re-introducing the transaction. Leaves the root cause live ⇒ disqualified.
- **Rely on the M4.2 poison containment** — containment is not a fix; it converts a fork into a
  wasted slot and 14 days of frozen operator funds. Symptom management ⇒ disqualified.
- **Special-case eviction of `aadebd59`** — a one-off cleanup that does not prevent the next one
  ⇒ disqualified.
- **A new activation height for the filter** — argued unnecessary in §E and it would add an immutable
  consensus artefact for a node-local policy. Rejected on subtraction grounds.

---

## G. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|---------------------|
| REQ-BOND-001 | The block builder MUST skip an AddBond whose inclusion would make `validate_block_economics` reject the block | **Must** | - [ ] Producer at 2999, mempool holds AddBond(+2) → built block excludes it<br>- [ ] Chain advances; zero `[BLOCK_POISON]` with `ADDBOND_CAP_EXCEEDED`<br>- [ ] Skip logged at `warn!` with the tx hash, height and reason |
| REQ-BOND-002 | Mempool admission MUST reject an AddBond that exceeds the cap against current holdings | **Must** | - [ ] `add_transaction` returns `Err` for AddBond(+2) at 2999<br>- [ ] `sendTransaction` returns a structured RPC error carrying `ADDBOND_CAP_EXCEEDED`<br>- [ ] `broadcast_tx` is NOT invoked on rejection<br>- [ ] Rejected tx is absent from `mempool.iter()` |
| REQ-BOND-003 | `revalidate` MUST evict a resident AddBond that has become over-cap | **Must** | - [ ] Resident AddBond(+2); producer flushes to 2999 at the epoch boundary → evicted on the next `revalidate`<br>- [ ] Its inputs return to `getUtxos` / `Spendable` in the same block<br>- [ ] Eviction logged at `warn!` |
| REQ-BOND-004 | The admission/builder check MUST be expression-identical to the gate, minus `in_block` | **Must** | - [ ] Both evaluate `check_addbond_cap(bond_count, pending_addbond, requested, h, AH)`<br>- [ ] `requested` = count of `OutputType::Bond` outputs, matching `validation_checks.rs:1208-1211`<br>- [ ] `allowance_with()` is NOT used (no `withdrawal_pending` subtraction)<br>- [ ] Parity test: for 200 random `(current, pending, requested)` triples, admission rejects ⇒ the gate rejects |
| REQ-BOND-005 | Every new check site MUST be gated on the existing `addbond_cap_enforcement_activation_height` | **Must** | - [ ] `height < AH` → check is a no-op at all three sites<br>- [ ] Devnet (`u64::MAX`) unchanged: over-cap AddBond still admitted and still packed<br>- [ ] No new field in `NetworkParams`; `git diff crates/core/src/network_params/` is empty |
| REQ-BOND-006 | `HoldingsLookup::Unavailable` MUST fail OPEN at admission | **Must** | - [ ] No holdings source wired → AddBond admitted (no rejection)<br>- [ ] Empty snapshot → `Unavailable`, admitted<br>- [ ] `new_for_test` / `new_for_replay` nodes admit as today |
| REQ-BOND-007 | The CLI MUST refuse to build an AddBond exceeding remaining headroom | **Should** | - [ ] `doli producer add-bond --count 4` at 2999 exits non-zero before signing<br>- [ ] Message states current count, cap and remaining headroom<br>- [ ] No UTXO is consumed and no RPC submit occurs<br>- [ ] `--count 1` at 2999 still succeeds |
| REQ-BOND-008 | The lookup MUST NOT introduce a blocking lock or change lock ordering | **Should** | - [ ] Admission uses `try_read` only (no `.read().await` on `ProducerSet`)<br>- [ ] Builder resolves holdings before the UTXO guard is taken (`assembly.rs:183-195` order preserved)<br>- [ ] Existing INC-I-180 lock-order tests stay green |
| REQ-BOND-009 | Record the client-side ops defect as a separate ticket | **Should** | - [ ] `auto-bond-nX.sh` unclamped `int(spendable/BOND_UNIT)` recorded in memory.db as its own incident with `scope=ops-scripts, out-of-repo` |
| REQ-BOND-010 | Existing consensus behaviour MUST be untouched | **Must** | - [ ] `git diff` shows no change to `validation_checks.rs`, `tx_types.rs`, `network_params/`<br>- [ ] `bins/node/tests/addbond_cap_overflow.rs` green unmodified<br>- [ ] `cargo test -p doli-core --lib` green |
| REQ-BOND-011 | Mainnet-side proof of burnout vs never-admitted | **Won't** | N/A — deferred. Requires injection-window mainnet logs that were never captured (§B). Does not block the fix. |
| REQ-BOND-012 | A DelegateBond-cap admission filter | **Won't** | N/A — INC-I-078 DelegateBond *skips* and leaves the block valid (`tx_types.rs:511-513`); different failure mode, different ticket. |
| REQ-BOND-013 | Split `pool.rs` (1818 L) and `validation_checks.rs` (1614 L) to the 500-line budget | **Won't** | N/A — pre-existing Rule 19 debt; splitting them inside a consensus-adjacent bugfix multiplies review surface. Recorded below. |

### Detailed acceptance criteria for the Must set

**REQ-BOND-001 — builder skip**
- [ ] Given a mempool holding AddBond(P, +2) and `P.bond_count = 2999` at `h >= AH`, when
      `build_block_content` runs, then the returned block's transactions do not contain that tx.
- [ ] Given AddBond(P, +1) and AddBond(P, +1) both resident at `P.bond_count = 2999`, when the block
      is built, then exactly one is included and the block passes `validate_block_economics`.
- [ ] Given `P.bond_count = 2998` and AddBond(P, +2), when the block is built, then it IS included
      (boundary: `2998 + 0 + 2 = 3000`, not `> 3000`).
- [ ] Given `h < AH` (devnet), when the block is built, then the over-cap tx IS included — unchanged.
- [ ] Given an unrelated RequestWithdrawal in the same candidate set, its verdict is byte-identical
      to the pre-change verdict.

**REQ-BOND-002 — admission**
- [ ] Given holdings `Found{bond_count: 2999, pending_addbond: 0}`, when `add_transaction` is called
      with AddBond(+2) at `h >= AH`, then it returns `Err` and `mempool.len()` is unchanged.
- [ ] Given the same, when `sendTransaction` is called over RPC, then the response is a JSON-RPC error
      whose `data.error_code` is `ADDBOND_CAP_EXCEEDED` and the broadcast closure recorded zero calls.
- [ ] Given `pending_addbond: 1` and `bond_count: 2999`, AddBond(+1) is rejected (`2999+1+1 > 3000`).
- [ ] Given `bond_count: u32::MAX, pending: 1, requested: 1`, the arithmetic saturates and rejects
      without panic (mirrors `saturating_no_overflow_post_ah_rejected`).

**REQ-BOND-003 — eviction**
- [ ] Given a resident over-cap AddBond, when `revalidate` runs after an applied block at `h >= AH`,
      then the tx is removed and `mempool.is_outpoint_spent(input)` is false afterwards.
- [ ] Given a resident **within**-cap AddBond, `revalidate` does not evict it across 10 consecutive
      blocks.
- [ ] Given a resident RequestWithdrawal that the INC-I-180 rules keep, the AddBond arm does not
      evict it.

**REQ-BOND-004 — parity**: as tabled above.
**REQ-BOND-005 — gate**: as tabled above.
**REQ-BOND-006 — fail-open**: as tabled above.
**REQ-BOND-010 — no consensus drift**: as tabled above.

---

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-BOND-001 | Must | `inc_i_203_builder_gap.rs::over_cap_addbond_is_not_packed_at_or_above_activation_height` **[M1 GREEN]** | §C builder path | `WithdrawalParity::{addbond_active, allow_add_bond, load}` @ `bins/node/src/node/production/withdrawal_holdings.rs`; skip `warn!` @ `bins/node/src/node/production/assembly.rs:324` |
| REQ-BOND-002 | Must | `inc_i_203_addbond_cap.rs::addbond_cap_verdict_is_the_gate_expression_minus_in_block` leg (a) **[M1 GREEN]** | §C admission path | `addbond_cap_verdict` @ `crates/mempool/src/addbond_cap.rs` (shared verdict only; `pool.rs` admission wiring is M2/M3) |
| REQ-BOND-003 | Must | (M3 — eviction; not in M1) | §C revalidate | `crates/mempool/src/pool.rs:1276` |
| REQ-BOND-004 | Must | `inc_i_203_builder_gap.rs::{admission_expression_rejects_a_strict_subset_of_consensus, allowance_with_is_not_the_addbond_expression}`; `inc_i_203_addbond_cap.rs` leg (d) **[M1 GREEN]** | §D parity note | `addbond_cap_verdict` @ `crates/mempool/src/addbond_cap.rs` — CALLS `doli_core::validation::check_addbond_cap`; `allowance_with` is not reached |
| REQ-BOND-005 | Must | `inc_i_203_builder_gap.rs::below_activation_height_over_cap_addbond_is_still_packed`; `inc_i_203_addbond_cap.rs` leg (b) **[M1 GREEN]** | §E | `addbond_cap_verdict` height guard @ `crates/mempool/src/addbond_cap.rs`; `WithdrawalParity::addbond_active` @ `production/withdrawal_holdings.rs`, fed the EXISTING `addbond_cap_enforcement_activation_height` @ `production/assembly.rs:190` (no new param) |
| REQ-BOND-006 | Must | `inc_i_203_builder_gap.rs::unavailable_holdings_fails_open_and_packs` (legs A+B); `inc_i_203_addbond_cap.rs` leg (c) **[M1 GREEN]** | §C fail-open constraint | fail-open lives in `addbond_cap_verdict` @ `crates/mempool/src/addbond_cap.rs` (non-`Found` ⇒ `Ok`); a missing builder entry maps to `Unavailable` in `allow_add_bond` @ `production/withdrawal_holdings.rs` |
| REQ-BOND-007 | Should | (M4 — CLI; not in M1) | §C CLI row | `bins/cli/src/cmd_producer/bonds.rs` |
| REQ-BOND-008 | Should | `inc_i_203_builder_gap.rs::unavailable_holdings_fails_open_and_packs` leg A (admission `try_read`, hangs on a blocking read); builder lock-order half in M2 **[M1 GREEN]** | §C lock discipline | unchanged ordering @ `bins/node/src/node/production/assembly.rs:183-201` — producer guard resolved and dropped BEFORE the UTXO guard; no new lock, no nesting, no `.await` under a guard |
| REQ-BOND-009 | Should | n/a | — | memory.db |
| REQ-BOND-010 | Must | `inc_i_203_builder_gap.rs::admission_expression_rejects_a_strict_subset_of_consensus` + `bins/node/tests/addbond_cap_overflow.rs` unmodified **[M1 GREEN]** | §E | none (negative) — verified: `git diff --stat crates/core/src/network_params/` empty; no change to `crates/core/src/validation/`, `validation_checks.rs` or `pool.rs` (green-evidence COMMAND 5) |

---

## H. Milestones (local mode: test-writer → developer → commit per milestone)

**M1 — Reproduction (RED).** No fix code. Add failing tests proving the gap:
`crates/mempool/tests/it/inc_i_203_admission_gap.rs` (admission accepts an over-cap AddBond) and
`bins/node/tests/it/inc_i_203_builder_gap.rs` (the builder packs it). Both must FAIL. Rule 21 gate.
Touches: 2 new test files. Requirements: REQ-BOND-002, REQ-BOND-001.

**M2 — Shared check + builder skip.** New `crates/mempool/src/addbond_cap.rs` (~70 lines): one pure
`check(tx, HoldingsLookup, in_block_prior, height, AH) -> Result<(), String>` wrapping
`check_addbond_cap`. Extend `WithdrawalParity::load` to resolve AddBond producers and `::allow` with an
AddBond arm. `bins/node/tests/it/inc_i_203_builder_gap.rs` turns GREEN.
Touches: 1 new file, `production/withdrawal_holdings.rs` (+~25 L). Requirements: REQ-BOND-001, 004, 005, 008, 010.

**M3 — Admission + eviction.** Call the M2 function from `withdrawal_holdings_verdict`
(`pool.rs:314`), which feeds both `add_transaction` (:573) and `revalidate` (:1301).
`crates/mempool/tests/it/inc_i_203_admission_gap.rs` turns GREEN; add the eviction and fail-open rows.
Touches: `crates/mempool/src/pool.rs` (+~15 L). Requirements: REQ-BOND-002, 003, 006.

**M4 — CLI headroom guard.** Use the `get_producers` response already fetched at `bonds.rs:32`;
refuse before signing. Touches: `bins/cli/src/cmd_producer/bonds.rs` (+~12 L). Requirements: REQ-BOND-007.

**M5 — Close-out.** Register REQ-BOND-009 as a separate ops incident; extract an invariant
(candidate INV-BOND-001: *"a node-local filter mirroring a consensus rule must evaluate the gate's
expression minus block-local terms, so that it rejects a strict subset"*) with linked regression
tests; run the gauntlet per Rule 29; update `specs/SPECS.md`.

---

## ━━━ RESOURCE COST ━━━

Design target: 1000s of producers, 10 s slots.

**CPU**
- *Per inbound transaction (admission).* Non-AddBond types: one `tx_type` enum compare, ~1 ns —
  unmeasurable against the existing per-tx cost (signature verification per input dominates at
  ~50-100 µs/input). AddBond only: one `HoldingsSources::lookup`.
  - Live path: `try_read` (uncontended atomic) + `get_by_pubkey` = one BLAKE3 over 32 bytes plus one
    `HashMap` get ≈ **300 ns**, plus `pending_addbond_count` (`crates/storage/src/producer/set_core.rs:214`),
    a **linear scan over all `pending_updates`** = O(M). With M ≈ 1000-3000 queued updates in a busy
    epoch, ≈ **3-10 µs**. This term is **already paid** once per AddBond per block by
    `validate_block_economics`; admission adds at most one more evaluation per AddBond transaction.
  - Snapshot fallback (only when the live handle is contended): `guard.iter().find()` over
    `Vec<(PublicKey, ProducerHoldings)>` = O(P) 32-byte comparisons. At P = 3000, ≈ **15-30 µs**.
    ⚠ This is the only term that grows with the producer count. It is a pre-existing property of the
    INC-I-180 channel, not introduced here, but at 1000s of producers it should become a `HashMap`.
    Flagged for the architect; not blocking, because the fallback fires only under write contention.
  - `check_addbond_cap` itself: three saturating `u32` adds and one compare, **< 5 ns**.
- *Per block build.* `WithdrawalParity::load` gains one `of_producer_set` call per distinct AddBond
  producer in the candidate set — bounded by the select budget, realistically < 20 → **< 200 µs**,
  against a 6 s assembly deadline (`assembly.rs:197`, 60 % of a 10 s slot). **< 0.004 % of the budget.**
- *Per applied block (`revalidate`).* One lookup per resident AddBond. Bounded by
  `MempoolPolicy.max_count` = 5000 (mainnet, `policy.rs:26`); realistically < 50 AddBonds →
  **< 500 µs**, once per 10 s slot.
- **Net CPU:** strictly negative in the failure case — today an over-cap AddBond costs a full
  `apply_block` in `ValidationMode::Light` (full UTXO + producer mutation + state-root recompute) that
  is then rolled back. That is **milliseconds to tens of milliseconds**, three to four orders of
  magnitude more than the microseconds this check spends preventing it.

**Memory**
- Zero new steady-state allocation on the admission path — `HoldingsLookup::Found` is `Copy`
  (12 bytes, three `u32`).
- Builder: `WithdrawalParity.holdings` gains at most one entry per distinct AddBond producer in the
  candidate set. `HashMap<PublicKey(32) + ProducerHoldings(12)>` ≈ 80 B/entry with overhead →
  **< 2 KB per block build**, freed when the builder returns.
- New module `crates/mempool/src/addbond_cap.rs` ≈ 70 lines, no statics, no caches.
- **Net memory:** strictly negative in aggregate — every toxic AddBond kept out of the mempool saves
  its own `MempoolEntry` (tx bytes + ancestor/descendant index) on **every node in the network** for
  up to 14 days.

**I/O**
- **Zero.** No disk read, no disk write, no RocksDB access, no network call. All three check sites
  read in-memory structures the caller already holds. No new gossip messages, no new RPC round-trips.
- **Net I/O:** strictly negative — a prevented poison event avoids one `rollback_one_block()`, which
  writes an undo batch and re-reads block-store entries.

**Lock / contention**
- Admission: `HoldingsSources::lookup` uses `try_read` on the `ProducerSet` and **never blocks**
  (`crates/mempool/src/holdings.rs:76-79`). It cannot deadlock against an `apply_block` writer and
  cannot extend mempool write-lock hold time beyond a failed `try_read` (nanoseconds).
- Builder: reuses the holdings already resolved under the producer guard at `assembly.rs:183-195`,
  *before* the UTXO guard is taken. **No new lock is acquired and the existing one-lock-at-a-time
  ordering is unchanged** — the discipline that keeps assembly off apply's `utxo→producers` /
  rollback's `producers→utxo` cycle.
- `revalidate` already runs under the mempool write lock (`apply_block/mod.rs:386`,
  `block_handling.rs:1097`); the added work is a `try_read` per resident AddBond, adding
  **< 500 µs** to a lock already held for the full scan.
- **Net contention:** neutral to negative. Preventing a poison event removes a
  `rollback_one_block()` that takes the producer **write** lock and the UTXO **write** lock — by far
  the heaviest contention event on the path.

**━━━━━━━━━━━━━━━━━━━━━━**

---

## Module Size (Rule 19)

| File | Lines | Budget | Status |
|---|---|---|---|
| `crates/mempool/src/pool.rs` | 1818 | 500 | ⚠ **over (pre-existing)** — this fix adds ~15 lines |
| `bins/node/src/node/validation_checks.rs` | 1614 | 500 | ⚠ **over (pre-existing)** — untouched by this fix |
| `bins/node/src/node/production/assembly.rs` | 669 | 500 | ⚠ **over (pre-existing)** — untouched by this fix |
| `crates/mempool/src/contention_tests.rs` | 1119 | 800 (test) | ⚠ over (pre-existing) |
| `bins/node/src/node/production/withdrawal_holdings.rs` | 209 | 500 | OK — becomes ~235 |
| `bins/cli/src/cmd_producer/bonds.rs` | 209 | 500 | OK — becomes ~221 |
| `crates/mempool/src/addbond_cap.rs` (new) | 0 → ~70 | 500 | OK |

**Proposal:** the new logic goes in a **new** `crates/mempool/src/addbond_cap.rs` rather than inside
`pool.rs`, mirroring how `withdrawal_holdings.rs` and `holdings.rs` were split out for INC-I-180.
`pool.rs` gains only the ~15-line dispatch. This keeps the fix from worsening the largest offender.

Splitting `pool.rs` and `validation_checks.rs` down to 500 lines is **real, recorded debt** but is
deliberately NOT bundled here (REQ-BOND-013 / Won't): a consensus-adjacent bugfix must present a small
reviewable diff, and moving ~2500 lines of mempool and validation code would bury the ~110 lines that
matter. Recommend a dedicated `/omega-improve` pass afterwards.

---

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| 1 | `check_addbond_cap` has exactly one production caller on `main` | Only one place in the running code checks the bond limit | **Yes** — graph (6/6 inbound edges are tests) + grep |
| 2 | `addbond_cap_enforcement_activation_height == 0` on mainnet and testnet | The limit rule is already switched on everywhere that matters | **Yes** — `defaults.rs:160,449` |
| 3 | The mempool can read the ProducerSet | The mempool can see how many bonds a producer already has | **Yes** — `holdings.rs:64-101`, wired at `pool.rs:289,305` |
| 4 | AddBond is not `is_zero_flow()`, so it takes the fee lane | The transaction goes through the normal checked path, not the free system lane | **Yes** — AddBond carries inputs and Bond outputs |
| 5 | Mempool residency freezes the operator's spendable balance | The stuck transaction is why the wallet shows zero | **Yes** — `balance.rs:36-37,75` |
| 6 | `max_age = 14 days` is the only automatic release today | Without a fix the funds unfreeze only after two weeks | **Yes** — `policy.rs:32`, `pool.rs:1260` |
| 7 | No validation rule constrains *which* mempool txs a block must carry | Nobody checks that a producer included every possible transaction | **Yes** — grep of both validate paths |
| 8 | The mainnet zero-poison observation cannot be resolved from code | We cannot tell from the source why mainnet logged nothing | **Yes — declared as an evidence gap**, see §B |
| 9 | `pending_addbond_count` is O(M) over pending updates | One of the lookups gets slower as more producers queue changes | **Yes** — `set_core.rs:214-226`; flagged in RESOURCE COST |

## What I Do Not Understand (mandatory)

1. **Whether the mainnet toxic transactions ever entered producer mempools.** §B declares this an
   evidence gap. It is unresolvable from this repository.
2. **Why INC-I-180 chose a fresh activation height for a node-local builder skip.** I have argued in
   §E that the AddBond case does not need one (strict-subset proof) and that reusing the existing
   AddBond height is correct. If there is an unrecorded reason the project always pins a fresh height
   for builder-visible behaviour, that reason should override my argument. **This is the single
   decision the architect should re-examine.**
3. **`ProducerInfo::add_bonds` clip semantics pre-AH** (`crates/storage/src/producer/info.rs:294-320`).
   I did not read the full flush path. With AH = 0 on both live networks the clip path is dead there,
   but it is live on devnet and I have not traced whether a node-local filter at `h < AH` could shadow
   a clip that a replay expects. REQ-BOND-005 makes the filter a strict no-op below AH, which should
   make this moot — but "should" is not "verified".
4. **Whether `select_for_block`'s size budget could ever exclude the toxic tx on a busy mainnet.** I
   read the function and found no mechanism, but I did not measure real mainnet mempool depth.
5. **The exact `Spendable` arithmetic across change outputs.** I confirmed the *mechanism*
   (`balance.rs:36-37,75`) but did not verify that a failed AddBond's change output is or is not
   counted as pending, which affects how much of the operator's balance is frozen versus merely
   reclassified.

## Identified Risks

- **Parity break using `allowance_with`.** The withdrawal expression subtracts `withdrawal_pending`;
  the AddBond gate does not. *Mitigation:* REQ-BOND-004 plus an explicit randomised parity test.
- **Fail-closed regression.** If `HoldingsLookup::Unavailable` is treated as reject, replay and test
  nodes reject every AddBond. *Mitigation:* REQ-BOND-006 with dedicated rows.
- **Widening `WithdrawalParity::load` alters a withdrawal verdict.** *Mitigation:* an unchanged-verdict
  test in M2.
- **The mainnet fix does not free the already-stuck N3-N6 funds until the binary is deployed there.**
  Deploying to mainnet requires explicit user confirmation (CLAUDE.md, MEMORY #2/#4). Not in scope for
  this pipeline run.
- **`pending_addbond_count` O(M) at 1000s of producers.** Pre-existing, amplified by one extra call
  per AddBond. *Mitigation:* flagged for the architect; not a blocker.

## Specs Drift Detected

- `crates/network/src/gossip/staleness.rs:116-118` — claims transactions are re-published only on RPC
  submission; `bins/node/src/node/validation_checks.rs:1284` re-publishes on gossip receipt too.
- `crates/core/src/validation/tx_types.rs:477-481` — the comment "*These validations are done at node
  level: Producer is registered … Bond output amount matches bond_count * BOND_UNIT*" is still
  unqualified. The cap half was resolved by INC-I-080; the *registration* and *amount* halves were not
  re-verified in this pass and may be the same class of comment-only claim that INC-I-080 found.
  Recommend a follow-up check; not in scope here.
- The stored INC-I-203 record places `select_for_block` in `production/assembly.rs`. It is in
  `crates/mempool/src/pool.rs:1035`. Corrected in memory.db by this pass.

## Out of Scope (Won't)

- `/mainnet/scripts/auto-bond-nX.sh` headroom clamp — outside this repository (REQ-BOND-009 records it).
- Any mainnet deployment or manual mempool flush — requires explicit user confirmation.
- Proving the mainnet burnout hypothesis (REQ-BOND-011).
- DelegateBond cap admission (REQ-BOND-012).
- Splitting `pool.rs` / `validation_checks.rs` to the 500-line budget (REQ-BOND-013).
- Any change to `validate_block_economics`, `check_addbond_cap`, or `network_params/` — REQ-BOND-010
  makes their being unchanged a testable requirement.

---

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.88, code-graph + targeted reads on main; sole enforcement point proven by graph
  (6/6 inbound edges are tests) and corroborated by grep; three NO-EDGE path queries; activation
  height read from defaults.rs; holdings channel read end-to-end)
Reasoning: Probable cause identified and verified; brittleness 0/5 (LOCALIZED); zero prior fix
  attempts; the required data channel, the rule function, the builder tally and the activation gate
  all already exist, so the fix is additive wiring across two adjacent modules, not architecture.
━━━━━━━━━━━━━━━━━━━━━━
```

DEEP-trigger audit (all negative): probable cause identifiable → YES, identified. 3+ interacting
components → NO (mempool and node/production already share `mempool::holdings`; `crates/core` is
read-only here). Intermittent / timing-dependent → NO; deterministic on `(bond_count, requested)`
and reproduced on demand twice (h=77780, h=88055). Resumed incident WITH a previous FAILED fix →
NO, attempts = 0. Architectural issues → NO, brittleness 0/5.

The one open uncertainty (§B, the mainnet zero-poison window) is an **evidence** gap, not a
**diagnosis** gap: no requirement above depends on it, and it does not select the fix site.
