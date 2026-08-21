# INC-I-180 — Analysis: n11 keeps active-set weight after all Bond UTXOs were spent

**Mode:** `/omega-doctor --investigate` (Step 1, Analyst). Diagnosis only. No fix, no state change.
**Run:** 524 · **Incident:** INC-I-180 · **Date:** 2026-08-19

---

## Deployed commit and code worktree (downstream agents: reuse this)

| Item | Value |
|---|---|
| Fleet binary | `doli-node 6.24.1 (ca0b3093)` (queried from a mainnet seed host; see the `mainnet` skill for access) |
| **Deployed commit** | **`ca0b30937178200abc5e013f7d6c4e9b6e464d98`** (2026-08-05, `chore(docs,skills): skills refresh + workflow archive rotation + AH registry entry`) |
| **Read-only worktree** | **`/private/tmp/claude-501/-Users-isudoajl-ownCloud-Projects-doli-network-doli/2b6d7830-24df-48c7-ba2a-ddc42e1bee7c/scratchpad/doli-deployed`** |

**Every `file:line` below is at `ca0b3093` in that worktree.** The local branch
`bugfix/inc-i-173-state-only-fee-gate` is ahead of the fleet and must not be cited.

**Dependency-graph note:** the orchestrator's `blast.py` run on `WithdrawalRequest` returned only
weak substring matches, and graphify is known blind to Rust `self.method()` dispatch
(auto-memory `reference_graphify_rust_method_blind_spot`). Blast radius below was therefore
built by grep + read in the deployed worktree. This is the accepted fallback, and it is stated
so downstream agents do not treat the graph result as authoritative.

---

## Scope

`crates/core/src/validation/` (tx validation) · `bins/node/src/node/apply_block/`
(UTXO + producer effects) · `crates/storage/src/producer/` (ProducerSet, deferred updates) ·
`crates/rpc/src/methods/producer.rs` (getProducer/getProducers/getBondDetails) ·
`bins/cli/src/cmd_producer/withdrawal.rs` · `bins/node/src/node/rewards.rs` (epoch snapshot,
rebuild) · **plus one non-repo component: the hourly `auto-bond-nN.sh` cron on the mainnet hosts.**

---

## Summary (plain language)

A withdrawal transaction is checked in two different places that count bonds from two different
sources, and nothing forces the two to agree.

- The **CLI** asks `getBondDetails`, which counts **Bond UTXOs**.
- The **node**, when the block lands, checks the **ProducerSet** record instead.

An hourly cron on the mainnet servers (`/mainnet/scripts/auto-bond-nN.sh`) bonds each node's
rewards every hour. A new bond appears in the UTXO set **immediately**, but the ProducerSet
record only learns about it at the **next epoch boundary** (up to one hour later). During that
window the UTXO count is one higher than the ProducerSet count.

n11's withdrawal was submitted inside that window. It asked for 434 bonds; the ProducerSet said
433 were available. The node **spent all 434 Bond UTXOs anyway** and then **silently skipped**
the producer-set half of the operation — it only writes a `WARN` line. n11 therefore lost all
its collateral but kept its producer weight. The control node n12 submitted its withdrawal
outside that window, the two counts agreed, and it exited correctly.

---

## Reproduced evidence (all claims independently re-verified)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `getProducers` shows n11 active, weight unchanged, `pendingWithdrawals=[]` | **PARTLY REFUTED** | `getProducer(b03fe629…)` → `status=active`, `selectionWeight=434`, `pendingWithdrawals=[]`, **but `bondCount=1`, `bondAmount=1000000000`**, `pendingUpdates=[{add_bond,1}]`. `bondCount` is *not* 434. |
| 2 | `getBondDetails` → bonds `[]`, totalStaked 0 | **REFUTED (now)** | `bondCount=1`, `totalStaked=1000000000`, one bond `creationSlot=245796`. It *was* 0 right after the withdrawal; the hourly cron has since bonded a new one. |
| 3 | Withdrawal tx on-chain, inputs are Bond UTXOs, payout to cold address | **CONFIRMED** | `getTransaction(69d30f2a…)` → h=244708, `txType=request_withdrawal`, **435 inputs** (434 Bond + 1 fee), 1 normal output `434807989935` (4348.07989935 DOLI) to `doli1v90kgpq…`. |
| 4 | `WARN … not enough bonds (requested N, available M)`, N=M+1 | **CONFIRMED, exact** | `/var/log/doli/mainnet/n11.log:31815` — `requested 434, available 433, delegated=0` at 19:51:51.999, immediately after `[BLOCK] Applied h=244708`. |
| 5 | n11 still producing after the withdrawal | **CONFIRMED** | n11 is `active` with `selectionWeight=434` at tip h=245399; it is still scheduled and still attesting. |
| 6 | Control shows `Queued WithdrawalRequest (K bonds) … deferred to epoch boundary`; later `exited` | **CONFIRMED** | n11.log: `Queued WithdrawalRequest (431 bonds) for producer f6e4b891… at height 243407`. `getProducer(e89c6d48…)` → `status=exited`, `bondCount=0`, `selectionWeight=0`. |
| 7 | Bond count drifting upward before the withdrawal | **CONFIRMED, mechanism identified** | `crontab -l` on **ai5**: `30 * * * * /mainnet/scripts/auto-bond-n{9,10,11,12}.sh` ("Mainnet auto-bond (layout v8)", re-enabled 2026-07-31). n11's AddBonds in the log: h=**243149, 243503, 243863, 244583, 245303** — one per hour ≈ one per 360-block epoch. |

`crypto_hash(n11_pubkey) = blake3(b03fe629…) = 5777bc12200ef44b1d4f0688e444562608308a94b33d770e97e24de6cecf40cf`
(algorithm confirmed at `crates/crypto/src/hash.rs:320`), which is how the AddBond log lines were
attributed to n11.

### The decisive timing difference — n11 vs control

Epoch length 360. Deferred updates flush at the boundary (`state_update.rs:181-208`; log confirms
flushes at h=**243360, 244440, 244800**).

| | last AddBond | last boundary before tx | withdrawal tx | AddBond already flushed? | result |
|---|---|---|---|---|---|
| **n12 (control)** | h=243149 | h=**243360** | h=243407 | **YES** (243149 < 243360) | UTXO count == ProducerSet count (431) → **queued** → exited at h=243720 |
| **n11** | h=**244583** | h=244440 | h=244708 | **NO** (244440 < 244583 < 244708) | UTXO count = ProducerSet count **+1** (434 vs 433) → **silently skipped** |

This is not an assumption. Both AddBond heights, both boundary flushes and both withdrawal
heights are in `/var/log/doli/mainnet/n11.log`.

---

## Architecture Context

### Module boundaries

| Module | Responsibility | Depends on | Depended on by |
|---|---|---|---|
| `bins/cli/src/cmd_producer/withdrawal.rs` | builds the withdrawal tx, **chooses `bond_count`** | RPC `getBondDetails`, `getUtxos`, `getNetworkParams` | operator |
| `crates/rpc/src/methods/producer.rs` | `getProducer(s)` / `getBondDetails` | UtxoSet **and** ProducerSet | CLI, operator, explorer |
| `crates/core/src/validation/tx_types.rs` | **structural-only** validation of `RequestWithdrawal` | tx bytes only | block validation |
| `bins/node/src/node/apply_block/mod.rs` | per-tx apply loop, ordering, undo log | UtxoSet, ProducerSet, state_db | all nodes |
| `…/apply_block/tx_processing.rs` | UTXO mutation **then** producer effects | both stores | apply loop |
| `crates/storage/src/producer/` | `ProducerInfo.bond_count`, pending-update queue, `apply_withdrawal` | — | scheduler, rewards, finality, state root |
| `bins/node/src/node/rewards.rs` | epoch reward split, `bond_snapshot`, producer-set rebuild | ProducerSet | consensus economics |
| **ops:** `/mainnet/scripts/auto-bond-nN.sh` (cron, hourly) | submits `AddBond` for each node | CLI + RPC | **not in the repo — invisible to every code-level review** |

### Data flows through the affected area

There are **two independent bond ledgers** and no reconciliation between them:

```
AddBond tx  ──► process_transaction_utxos()   ──► Bond UTXO created NOW      (UtxoSet)
            └─► process_transaction_producer_effects() ──► PendingProducerUpdate::AddBond
                                                          ──► ProducerInfo.bond_count += n
                                                              ONLY at next epoch boundary
```

Readers pick different ledgers:

| Reader | Source of "bond count" | Citation |
|---|---|---|
| `getBondDetails.bondCount` / `.bonds` / `.totalStaked` | **UTXO set** ("source of truth" per the comment) | `crates/rpc/src/methods/producer.rs:301-310` |
| `getProducer(s).bondCount` | **UTXO set when > 0, else ProducerSet** | `producer.rs:75-84`, `:184-190` |
| `getProducer(s).selectionWeight` | **ProducerSet** `info.bond_count` | `producer.rs:117`, `:215` → `crates/storage/src/producer/info.rs:390-407` |
| node apply-time withdrawal gate | **ProducerSet** `info.bond_count` | `bins/node/src/node/apply_block/tx_processing.rs:375-378` |
| scheduling / active set | **ProducerSet** (`selection_weight_at > 0`) | `crates/storage/src/producer/set_core.rs:365` |
| epoch reward weights (`bond_snapshot`) | **ProducerSet** `selection_weight_at` | `bins/node/src/node/apply_block/post_commit.rs:209-221` |
| CLI `available` | **UTXO count − ProducerSet pending** (mixes both!) | `bins/cli/src/cmd_producer/withdrawal.rs:41-49` |

`bins/cli/…/withdrawal.rs:43` is the mixed read:
`let available = details.bond_count - details.withdrawal_pending_count;` — the minuend comes from
the UTXO ledger (`producer.rs:306`) and the subtrahend from the ProducerSet ledger
(`producer.rs:375`).

### Architectural constraints and invariants

- **INV (implicit, unenforced):** a producer's ProducerSet `bond_count` should equal its Bond-UTXO
  count. Nothing in the codebase asserts or restores this. It is violated by design for a whole
  epoch after every AddBond.
- **INV-DEFER:** producer mutations are epoch-deferred; maintainer changes are immediate.
  **VERIFIED in code** — `tx_processing.rs:203-530` queues Register/Exit/Slash/AddBond/
  RequestWithdrawal/Delegate/Revoke; `state_update.rs:183-208` flushes at the boundary;
  `apply_block/governance.rs` applies maintainer changes inline.
- **INV-ORDER:** `process_transaction_utxos` runs **before** `process_transaction_producer_effects`
  for every tx (`apply_block/mod.rs:197-217`). The producer-effects function returns `()` — it
  **structurally cannot** reject a transaction or undo a UTXO spend. The code says so explicitly
  at `tx_processing.rs:347-351`.
- **INV-DETERMINISM:** the skip is a pure function of (block bytes, ProducerSet), both identical on
  every node ⇒ every node reaches the same wrong state. See fork-risk read below.
- **Precedent the codebase already set and then did not generalise:** INC-I-080 added
  `ProducerSet::pending_addbond_count()` (`set_core.rs:205-220`) precisely because *in-flight
  epoch-deferred AddBonds* make the ProducerSet count stale. It is consumed by the AddBond cap
  check only. The withdrawal gate 130 lines away never learned the lesson.

### Blast radius (grep/read-derived; graphify unusable here)

**Direct** — `tx_processing.rs:369-430`, `producer/set_core.rs:154-173`, `producer/info.rs:482-521`
(`apply_withdrawal`, incl. auto-exit at `bond_count == 0`), `rpc/methods/producer.rs:46-140` and
`:276-379`, `cli/cmd_producer/withdrawal.rs:41-49` (and the same pattern in `exit.rs`).

**Indirect (consumers of ProducerSet weight)** — scheduler eligibility
(`set_core.rs:344-366`), epoch `bond_snapshot` → **reward distribution**
(`post_commit.rs:209-221` → `rewards.rs:304-353`), attestation/finality denominator, the
**state root** (ProducerSet is one of the three states), snap-sync payload, and the
`rebuild_producer_set_from_blocks` replay path (`rewards.rs:1105`, withdrawal arm at
`rewards.rs:1236-1259`).

**Non-code** — the hourly `auto-bond-nN.sh` cron on ai1–ai5 arms this window for **every remaining
exposed node n1–n10**, and is **still running for n11 today** (fresh Bond UTXO at slot 245796 and
a pending `add_bond` right now).

### Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
Details:
  1. Cross-module blast radius  — YES (CLI + RPC + apply path + producer store; a correct fix
     spans at least three of them, and they share no direct dependency)
  2. Invariant gaps             — YES (no module owns or enforces "ProducerSet bond_count ==
     Bond-UTXO count"; it is knowingly violated for a full epoch after every AddBond)
  3. Data flow reversal         — NO
  4. Shared mutable state       — YES (bond accounting duplicated across UtxoSet and ProducerSet
     with no single owner and no reconciliation)
  5. Contract absence           — YES (RequestWithdrawal.extra_data.bond_count is bound by no
     consensus rule to either ledger; "available" is an implicit convention re-derived
     differently in the CLI, in the RPC, and in the node)
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Code trace (all at `ca0b3093`)

**(a) What the withdrawal does to the UTXO set.** `apply_block/mod.rs:197-206` calls
`process_transaction_utxos` first, inside the per-tx loop. All 434 Bond UTXOs are spent and the
single Normal payout output is created. Lock bypass for Bond inputs is explicit at
`crates/core/src/validation/utxo.rs:152-153`.

**(b) What it does to producer-set accounting.** `apply_block/mod.rs:209-217` then calls
`process_transaction_producer_effects` → `tx_processing.rs:369-430`:

```rust
let remaining = info.bond_count.saturating_sub(info.withdrawal_pending_count);   // :375-377
let available = remaining.saturating_sub(info.delegated_bonds);                  // :378
if data.bond_count > available { … warn!("… not enough bonds …") }               // :379-398
if data.bond_count <= remaining {                                                // :399
    producer.withdrawal_pending_count += data.bond_count;                        // :408
    producers.queue_update(PendingProducerUpdate::RequestWithdrawal { … });      // :414-418
}
```

The two `if`s are **independent**. When `bond_count > remaining` the second block is skipped, so
`withdrawal_pending_count` is never raised and **no `PendingProducerUpdate::RequestWithdrawal` is
ever queued**. There is no `else`, no error, no return.

**(c) Atomic or independent — independent.** They are sequential, in one loop, over one
`BlockBatch`, but the producer-effects pass returns `()`. `tx_processing.rs:347-351` states the
design reason verbatim: a late reject "would leave the in-memory set divergent from disk".
Consensus validation never bridges them: `validation/transaction.rs:152-154` →
`validation/tx_types.rs:548-600` is **structural only** (inputs non-empty, exactly one Normal
output, amount > 0, valid `WithdrawalRequestData`, non-zero destination). Its own doc comment,
`tx_types.rs:547`, says: *"Bond UTXO ownership, producer bond holdings, and FIFO calculation done
at node level."* At node level the check is a `warn!`.

**(d) What the "not enough bonds" branch does — nothing but log.** It does not abort the tx, does
not roll back the UTXO spend, does not mark the producer, does not queue anything. The block is
applied normally (`[APPLY_END] status=applied` at n11.log:31824). The producer keeps its weight.

**(e) The control path.** `tx_processing.rs:419-424` emits
`Queued WithdrawalRequest (K bonds) … deferred to epoch boundary` only from inside the
`bond_count <= remaining` block. At the next boundary, `state_update.rs:183-208` calls
`ProducerSet::apply_pending_updates_with_cap` (`set_core.rs:106`), whose
`RequestWithdrawal` arm (`set_core.rs:154-173`) calls `ProducerInfo::apply_withdrawal`
(`info.rs:482-521`): it drains the oldest `bond_entries`, decrements `bond_count`/`bond_amount`/
`withdrawal_pending_count`, and — `info.rs:517-520` — **auto-exits when `bond_count` reaches 0**
(`status = Exited`), after which `selection_weight_at` returns 0 (`info.rs:391`, `:404-406`).
That is exactly what n12 did and n11 never entered.

**Consequence for n11.** At the h=244800 boundary the only pending update for n11 was the
AddBond(+1) from h=244583. It applied: `bond_count 433 → 434`. Nothing decremented it. Today
`selectionWeight = 434` with **zero backing collateral at withdrawal time** — matching
`info.bond_count = 434` exactly, which independently confirms the reconstruction.

---

## Candidate hypotheses

**H1 — Epoch-deferred AddBond opens a one-epoch window in which the UTXO ledger leads the
ProducerSet ledger; the CLI sizes the withdrawal from the UTXO ledger and the node gates it on the
ProducerSet ledger; the mismatch branch is log-only. `conf(0.95, evidence)` — ACCEPTED.**
Confirms: exact numbers (434/433); n11 AddBond at h=244583 strictly between boundary h=244440 and
tx h=244708; n12 AddBond at h=243149 strictly before boundary h=243360; CLI mixed read at
`withdrawal.rs:43`; UTXO-derived `getBondDetails` at `producer.rs:301-310`; ProducerSet-derived
gate at `tx_processing.rs:375-378`; skip-without-queue at `:399`. Would have killed it: an AddBond
for n11 *before* h=244440, or a `getBondDetails` sourced from the ProducerSet, or an `else` branch
that rejected the tx.

**H2 — CLI bond-count selection is independently wrong (off-by-one, or counts a non-bond UTXO).
`conf(0.05, evidence)` — REJECTED as primary, retained as contributing design defect.**
`withdrawal.rs:41-49` has no ±1 arithmetic error; it faithfully reports the UTXO ledger. The
defect is that `available` mixes two ledgers, not that it miscounts one. Kill test: the same CLI
produced the *correct* count for n12 twenty minutes after a boundary.

**H3 — Delegation involvement (`delegated_bonds` shrinking `available`). `conf(0.01, evidence)` —
KILLED.** The log line itself prints `delegated=0`, and `getProducer` shows
`delegatedBonds: 0`, `receivedDelegations` absent. The `delegated_bonds > 0` auto-revoke branch
(`tx_processing.rs:380-391`) was not taken.

**H4 — Mempool/ordering: a second withdrawal or an AddBond landed in the same block and consumed
the allowance. `conf(0.03, evidence)` — KILLED for this incident.** Block h=244708 carries
`txs=2`; the only producer-affecting tx is the withdrawal, and no other n11 update appears in the
log for that height. **Not killed as a general vector:** two withdrawals for the same producer in
one epoch, or an AddBond and a withdrawal in the same block, hit the same gate with the same
log-only outcome. Downstream should test it.

**H5 — Epoch timing alone (tx landed on a boundary block). `conf(0.02, evidence)` — KILLED.**
h=244708 is not a boundary (244440 / 244800 are). Timing matters only through H1's window.

**H6 — Operational/non-repo cause: the hourly `auto-bond-nN.sh` cron is what arms the window.
`conf(0.9, evidence)` — ACCEPTED as the enabling condition (not the fault itself).**
`crontab -l` on ai5 shows `30 * * * *` auto-bond for n9–n12; n11's AddBond cadence in the log is
hourly. This is the hypothesis "outside the producer-set layer" that actually matters: **the same
trap is armed on n1–n10, and the cron is still bonding into the compromised n11 identity right
now.** Kill test: disable the cron on a node and confirm the divergence window closes.

**H7 — Reward auto-bonding inside the node (as the SKILL and the prior notes assert).
`conf(0.02, evidence)` — KILLED.** No `TxType::AddBond` is constructed anywhere in `bins/node`;
the only builder is `crates/wallet/src/tx_builder/builder.rs:329` (CLI). Epoch rewards are plain
Normal outputs (`rewards.rs:348-438`; no `OutputType::Bond` in that path). The "rewards auto-bond"
belief is true operationally (cron) and false architecturally — see Drift.

**H8 — Divergence between nodes with different history (snap-synced / rebuilt vs continuous).
`conf(0.25, assumed)` — OPEN, hand to the fork-risk investigator.** `rewards.rs:1236-1259`
mirrors the live gate (`data.bond_count <= available` → queue, else skip), which is the right
shape. Unverified: whether the rebuild loop flushes pending updates at boundaries in the same
order as `state_update.rs:183`, and whether a snap-sync recipient receives a serialized
ProducerSet (safe) or rebuilds it (risk). This is the one hypothesis I could not close.

---

## Impact analysis

**State the anomaly touches.**

- **Scheduling** — n11 is in `active_producers` because `selection_weight_at > 0`
  (`set_core.rs:365`). It is still being scheduled and still produces blocks with 434 unbacked weight.
- **Reward distribution** — `bond_snapshot` is built from `selection_weight_at`
  (`post_commit.rs:212`), i.e. the ProducerSet ledger. **n11 is still paid, bond-weighted, on
  collateral it no longer holds** (~434 / ~17,7xx ≈ 2.4% of every epoch pool). Those rewards land
  at an address whose private key is **public** (INC-I-170 / INC-I-175). Value is leaking into a
  compromised identity every epoch, and the hourly cron then bonds it back in.
- **Finality / attestation** — n11's 434 weight sits in the denominator of the 67% threshold while
  being economically unbacked; it also dilutes the honest fraction that the INC-I-170 retirement
  procedure is trying to raise.
- **State root** — ProducerSet is one of the three states in the root. The divergent value is
  *consistent* across nodes today, so the root agrees.
- **Snap sync** — a new node must reproduce `bond_count = 434` for a producer with zero Bond UTXOs.
  See H8.

**Initial fork-risk read: NO fork today; residual risk is history-dependent. `conf(0.8, evidence)`.**
Both the UTXO spend and the producer-effects skip are pure functions of the block bytes and the
ProducerSet, which every node holds identically; the `warn!` has no state effect. All nodes are at
one tip and hash (`getChainInfo` → h=245399, `bestHash 6ac5db6a…`). The residual risk is H8. Final
proof is for the follow-up investigator, not this analysis.

**What breaks if this is "fixed" carelessly.** Any change to the withdrawal gate changes which
producers are in `active_producers` for some height ⇒ CLAUDE.md three-question checklist answers
(1) YES, (2) YES, (3) NO ⇒ **activation height required** (INV-CONSENSUS-001, INV-PARAMS-001).
Any change that alters block content or the producer set at a height ⇒ **synchronized deploy**
question must also be answered. Both belong to the architect, not here.

---

## Specs / docs drift detected

1. **`bins/node/src/node/rewards.rs:304-308`** — comment claims *"the epoch_bond_snapshot is
   computed from the UTXO set at the epoch boundary"*. It is not: `post_commit.rs:209-221` builds
   it from `selection_weight_at()`, i.e. the ProducerSet. This comment is exactly the belief that
   would have made someone think n11 stops earning rewards. **It does not.**
2. **`bins/node/src/node/rewards.rs:519-523`** — doc comment for the rebuild lists the tx types it
   handles and **omits `RequestWithdrawal`**, which is in fact handled at `rewards.rs:1236`.
3. **`.claude/skills/producer-retirement/SKILL.md`** (gotchas + step 1 guard) — states *"rewards
   auto-bond"* as if it were node behaviour. It is an **operator cron**
   (`/mainnet/scripts/auto-bond-nN.sh`, `30 * * * *`). The skill's guard ("abort if the count
   changed") is also insufficient: it compares a count against itself over time instead of
   comparing the **two ledgers** against each other, which is the failure that actually occurred.
4. **`.claude/skills/producer-retirement/SKILL.md`** — *"Withdrawals serialize per producer. The
   CLI computes `available = bondCount − withdrawalPendingCount`"* is repeated as correct
   guidance. It is the defect: those two numbers come from different ledgers.
5. **`docs/bugfixes/inc-i-170-mainnet-retirement-progress.md:75-108`** — the "phantom producer"
   entry states `bondCount 434` in `getProducers`; live RPC now returns `bondCount 1` (UTXO-derived)
   with `selectionWeight 434`. The note also asserts "one reward had just auto-bonded", which is
   directionally right but attributes it to the node rather than the cron.

---

## Requirements for remediation (no fix design here)

| ID | Requirement | Priority | Acceptance criteria |
|---|---|---|---|
| REQ-I180-001 | A `RequestWithdrawal` whose `bond_count` exceeds the producer-set allowance MUST NOT be able to spend Bond UTXOs. The UTXO effect and the producer-set effect must succeed or fail together. | **Must** | - [ ] Given a producer with an unflushed in-flight AddBond, when a withdrawal for (utxo_count) bonds is submitted, then either the tx is rejected before any UTXO is spent, or the producer-set update is applied for the full amount.<br>- [ ] No code path exists in which a Bond UTXO is spent and no `PendingProducerUpdate::RequestWithdrawal` is queued.<br>- [ ] Regression test replays the exact h=244583 AddBond → h=244708 withdrawal → h=244800 boundary sequence and asserts `status=exited, selection_weight=0`. |
| REQ-I180-002 | The producer-set withdrawal gate MUST account for in-flight epoch-deferred AddBonds, exactly as the AddBond cap check already does via `pending_addbond_count()`. | **Must** | - [ ] `remaining` includes queued-but-unflushed AddBond bonds for that producer.<br>- [ ] Unit test: bond_count=433 + pending AddBond(1) ⇒ a 434-bond withdrawal is accepted, not skipped. |
| REQ-I180-003 | Any consensus-visible change from 001/002 MUST be gated by a new `NetworkParams` activation height and MUST answer both deploy questions in the commit message. | **Must** | - [ ] New AH field in `crates/core/src/network_params/`, mainnet value above current tip; no existing AH reused or moved.<br>- [ ] Pre-activation behaviour bit-identical to `ca0b3093`.<br>- [ ] Three-question checklist answered in the commit body. |
| REQ-I180-004 | The "not enough bonds" condition MUST be observable as an error, not only a `WARN`. | **Must** | - [ ] The condition increments a metric and/or produces a distinct alertable event.<br>- [ ] An operator can detect the state "producer with selection_weight > 0 and zero Bond UTXOs" from RPC alone. |
| REQ-I180-005 | The CLI MUST derive `available` from a single, self-consistent source and MUST refuse to submit when the two ledgers disagree. | **Must** | - [ ] `withdrawal.rs` and `exit.rs` compare UTXO-derived and producer-set-derived counts and abort with an explicit message on mismatch.<br>- [ ] `request-withdrawal` gains a pre-submit confirmation showing both numbers. |
| REQ-I180-006 | RPC MUST expose the producer-set bond count as its own field, distinct from the UTXO-derived `bondCount`. | **Should** | - [ ] `getProducer(s)` returns both, unambiguously named.<br>- [ ] `docs/rpc_reference.md` documents which ledger each field reads. |
| REQ-I180-007 | An operational guard MUST prevent a withdrawal from being submitted inside the post-AddBond, pre-boundary window on the remaining exposed nodes n1–n10. | **Must** | - [ ] Written pre-withdrawal check, verified against code, that reads both ledgers and the pending-update queue.<br>- [ ] The `auto-bond-nN.sh` cron state is an explicit, recorded input to the check. |
| REQ-I180-008 | A safe path MUST exist to remove n11's 434 unbacked weight without loss of value and without a fork. | **Must** | - [ ] Ranked options with blast radius and both deploy answers each.<br>- [ ] Selected option leaves all nodes at one height and one hash. |
| REQ-I180-009 | Determinism across snap-synced / rebuilt nodes MUST be proven for the current n11 state (H8). | **Must** | - [ ] Evidence that a node rebuilding from block history reproduces `bond_count = 434` for n11.<br>- [ ] State-root equality demonstrated between a continuous node and a freshly synced one. |
| REQ-I180-010 | Drift items 1–5 above MUST be corrected in code comments, the skill, and the progress notes. | **Should** | - [ ] Each of the five is fixed and cites the code. |
| REQ-I180-011 | Reward accrual to unbacked weight (n11 being paid on collateral it does not hold) MUST be assessed against INC-I-177. | **Should** | - [ ] Written assessment of whether these are one defect class. |
| REQ-I180-012 | Retroactive correction of past epochs' reward distribution. | **Won't** | N/A — deferred; would require rewriting consensus history. |

### Traceability — M1 test coverage (written 2026-08-20, TDD RED)

Tests exist and fail BEFORE the fix. `R` = red today (reproduces the defect), `G` = green
today and must stay green (replay-safety / accept-boundary lock), `C` = compile-red (names a
symbol the developer must add).

| Req | Test IDs | State |
|---|---|---|
| REQ-I180-001 | `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`::`req_i180_001_post_ah_over_allowance_block_is_rejected`, `…_two_withdrawals_in_one_block_are_summed`, `…_unknown_producer_is_rejected`, `…_u32_max_saturates_and_rejects`, `…_validation_and_apply_never_disagree` | R |
| REQ-I180-001 | same file::`req_i180_001_post_ah_exact_fit_is_accepted_and_lands`, `…_zero_bond_withdrawal_is_accepted` | G |
| REQ-I180-001 (F4 flush order) | `crates/storage/tests/it/inc_i_180_withdrawal_holdings.rs`::`f4_fifo_flush_addbond_then_full_withdrawal_exits_at_zero`, `f4_fifo_order_is_the_discriminating_row`, `legacy_producer_without_bond_entries_still_exits`, `withdrawal_of_zero_bonds_is_a_no_op` | G |
| REQ-I180-002 | `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`::`req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable` | R |
| REQ-I180-002 | `crates/storage/tests/it/inc_i_180_withdrawal_holdings.rs`::`pending_addbond_count_is_zero_with_no_queue`, `…_is_scoped_to_the_pubkey`, `…_counts_outpoints_not_updates` | G |
| REQ-I180-003 | `crates/core/tests/it/inc_i_180_activation_height.rs`::`req_i180_003_mainnet_gate_is_frozen_and_not_pinned_in_m1`, `…_testnet_gate_is_pinned_near_future`, `…_devnet_gate_leaves_a_pre_activation_band`, `…_the_gate_is_dedicated_and_not_bundled`, `…_no_existing_activation_height_was_moved` | C |
| REQ-I180-003 | `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`::`req_i180_003_pre_ah_n11_replay_preserves_the_silent_skip`, `…_pre_ah_over_allowance_keeps_the_legacy_verdict`, `…_pre_ah_two_withdrawals_in_one_block_keep_legacy` | G |
| REQ-I180-004 … 012 | not in M1 scope | — |

**QA-1 round (written 2026-08-20, TDD RED before the fix; QA run 525 ISSUE-001/002/003):**

| Req | Test IDs | State |
|---|---|---|
| REQ-I180-001 (ISSUE-001, `Exit` charges the allowance) | `bins/node/tests/it/inc_i_180_gate_bindings.rs`::`req_i180_001_post_ah_exit_plus_withdrawal_is_rejected`, `…_two_exits_charge_the_allowance_twice` | R |
| REQ-I180-001 (ISSUE-001 accept side / liveness) | same file::`req_i180_001_post_ah_double_charge_accept_boundary`, `…_exit_only_block_stays_admitted` | G |
| REQ-I180-001 (ISSUE-002, declared count bound to Bond inputs) | same file::`req_i180_001_post_ah_under_declared_bond_count_is_rejected`, `…_over_declared_bond_count_is_rejected`, `…_non_bond_inputs_are_not_bonds`, `…_unseeded_inputs_are_not_bonds` | R |
| REQ-I180-001 (ISSUE-002 accept side) | same file::`req_i180_001_post_ah_declared_count_matching_inputs_lands` | G |
| REQ-I180-001 (ISSUE-003, rebuild⇄live parity) | `bins/node/tests/it/inc_i_180_rebuild_parity.rs`::`req_i180_001_rebuild_matches_live_with_addbond_in_flight` | R |
| REQ-I180-003 (ISSUE-003 invariance) | same file::`req_i180_003_rebuild_keeps_the_legacy_skip_with_nothing_in_flight` | G |
| REQ-I180-003 (pre-AH bit-identity for both new admission rules) | `bins/node/tests/it/inc_i_180_gate_bindings.rs`::`req_i180_003_pre_ah_exit_plus_withdrawal_keeps_legacy`, `…_pre_ah_under_declared_keeps_legacy` | G |
| REQ-I180-001 (ISSUE-001 storage substrate) | `crates/storage/tests/it/inc_i_180_withdrawal_holdings.rs`::`charging_withdrawal_pending_does_not_move_bond_count_before_the_flush`, `accumulated_exit_charges_drain_in_order_without_underflow` | G |

### Traceability — M1 implementation (2026-08-20, all rows GREEN)

| Req | Implementation Module @ file path |
|---|---|
| REQ-I180-001 | withdrawal-holdings gate @ `bins/node/src/node/validation_checks.rs` (`validate_block_economics`, pre-mutation, all modes; error codes `ECON_WITHDRAWAL_OVER_HOLDINGS` / `ECON_WITHDRAWAL_UNKNOWN_PRODUCER`) |
| REQ-I180-001 | apply-layer parity @ `bins/node/src/node/apply_block/tx_processing.rs` (`process_transaction_producer_effects`, `RequestWithdrawal` arm) |
| REQ-I180-001 (F4) | no change — FIFO flush order already correct @ `crates/storage/src/producer/set_core.rs` (`apply_pending_updates_with_cap`), now locked by tests |
| REQ-I180-002 | `+ pending_addbond_count(pk)` allowance term @ `validation_checks.rs` and `tx_processing.rs`; primitive unchanged @ `crates/storage/src/producer/set_core.rs:214` |
| REQ-I180-003 | `withdrawal_holdings_gate_activation_height` @ `crates/core/src/network_params/mod.rs` (field), `defaults.rs` (mainnet `u64::MAX` / testnet `230_000` / devnet `20`), `env_loader.rs` (`DOLI_WITHDRAWAL_HOLDINGS_GATE_ACTIVATION_HEIGHT`, mainnet-locked) |
| REQ-I180-001 (ISSUE-001) | `Exit` arm of the gate's `match` charges `info.bond_count` to `in_block_withdrawn` @ `bins/node/src/node/validation_checks.rs` — mirrors apply's per-Exit `+=` against an unchanged `bond_count`, double charge included |
| REQ-I180-001 (ISSUE-002) | declared `bond_count` bound to the count of inputs resolving to `OutputType::Bond` in the pre-block UTXO view @ `bins/node/src/node/validation_checks.rs`; new code `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` |
| REQ-I180-001 (ISSUE-003) | **FIXED in the QA-1 round** — height-gated `+ pending_addbond_count()` term in the `RequestWithdrawal` arm of `rebuild_producer_set_from_blocks` @ `bins/node/src/node/rewards.rs`. Closes `FIND-I180-M1-001`. The two ADMISSION rules above are deliberately not mirrored there (the replay reads already-canonical blocks). |
| REQ-I180-001 (ISSUE-006) | **FIXED in the QA-2 round** — the Bond-input count is bound to the NAMED producer: an input counts only if its pre-block UTXO is `OutputType::Bond` AND `pubkey_hash == hash_with_domain(ADDRESS_DOMAIN, wd.producer_pubkey)` @ `bins/node/src/node/validation_checks.rs`. Foreign bonds count zero, so `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` fires. Same activation height; no new error code. |
| REQ-I180-001 (AUDIT-P1-001) | **FIXED in security-audit round 1** — R2 is now an EXCLUSIVITY rule: `declared == owned == all`, where `all` counts every input whose pre-block UTXO is `OutputType::Bond` and `owned` the subset at `hash_with_domain(ADDRESS_DOMAIN, wd.producer_pubkey)` @ `bins/node/src/node/validation_checks.rs`. Both counts come from ONE pass over the lookups the gate already performed, so the utxo-read-then-drop / producer-read lock order is unchanged. Same activation height; same `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH` code, message extended with the total. Closes the two-key rider shape (declare B's true count, let A's Bond UTXOs ride along). |
| REQ-I180-003 (ISSUE-005) | **FIXED in the QA-2 round** — the whole INC-I-180 gate block is evaluated BEFORE the EpochReward section of `validate_block_economics` @ `bins/node/src/node/validation_checks.rs`, so the Full-mode `[INC_I_081_MISSING_CHECK_SKIP]` early return can no longer skip it. INC-I-080's AddBond cap stays BELOW that return, byte-for-byte unchanged (`git diff HEAD` on the file shows zero deletions): its AH is `0` on mainnet and testnet, so arming it in a case where it never ran would change canonical verdicts. |

**QA-2 round tests (written 2026-08-20, TDD RED before the fix; QA run 525 ISSUE-005/006):**

| Req | Test IDs | State |
|---|---|---|
| REQ-I180-001 (ISSUE-006, owner binding) | `bins/node/tests/it/inc_i_180_gate_bindings.rs`::`req_i180_001_post_ah_cross_owner_bond_inputs_are_rejected`, `…_mixed_owner_bond_inputs_are_rejected` | R |
| REQ-I180-001 (ISSUE-006 accept side) | same file::`req_i180_001_post_ah_same_owner_bond_inputs_are_accepted` | G |
| REQ-I180-003 (ISSUE-006 pre-AH bit-identity) | same file::`req_i180_003_pre_ah_cross_owner_bond_inputs_keep_legacy` | G |
| REQ-I180-001 (ISSUE-005, mode-independent verdict) | `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`::`req_i180_001_epoch_boundary_verdict_is_mode_independent` | R |
| REQ-I180-001 (ISSUE-005 liveness + scoping) | same file::`req_i180_001_epoch_boundary_legal_withdrawal_stays_admitted`, `…_epoch_boundary_missing_epochreward_skip_intact` | G |
| REQ-I180-003 (INC-I-080 reachability unchanged) | same file::`inc_i_080_addbond_cap_stays_below_the_epoch_reward_return` | R (positional guard) |
| REQ-I180-001 (QA OBS-R2-001 ports) | `bins/node/tests/it/inc_i_180_gate_bindings.rs`::`req_i180_001_post_ah_withdrawal_then_exit_is_admitted_in_parity`, `…_in_block_addbond_extends_the_allowance` | G |

**Security-audit round 1 tests (written 2026-08-20, TDD RED before the fix; AUDIT-P1-001):**

| Req | Test IDs | State |
|---|---|---|
| REQ-I180-001 (AUDIT-P1-001, exclusivity) | `bins/node/tests/it/inc_i_180_gate_bindings.rs`::`req_i180_001_post_ah_foreign_bond_riders_are_rejected` | R |
| REQ-I180-003 (AUDIT-P1-001 pre-AH bit-identity) | same file::`req_i180_003_pre_ah_foreign_bond_riders_keep_legacy` | G |

---

## What I do not understand (stated before conclusions are used)

1. **Who owns `/mainnet/scripts/auto-bond-nN.sh` and what it does with n11's exposed wallet.** I
   confirmed the cron exists and its cadence matches the AddBonds; I did not read the script. It is
   still bonding into a compromised identity.
2. **Whether snap sync ships a serialized ProducerSet or rebuilds it** (H8). This is the only
   remaining path to a real fork and I did not close it.
3. **Whether `withdrawal_pending_count` can be left non-zero forever** by a partially-skipped
   withdrawal, permanently shrinking a producer's future allowance. The n11 numbers are consistent
   with `pending = 0` throughout, so I could not exercise it.
4. **Whether the same skip exists on the `Exit` path** (`tx_processing.rs:255-291`). I read the
   withdrawal arm closely and only skimmed Exit.
5. **Why n11's `bondCount` reads 1 rather than 0** beyond "the cron bonded one more" — I did not
   trace the funding source of that specific AddBond.

## Contradictions found and resolved

⚠ **CONTRADICTION:** the prior notes and the retirement skill both state that *rewards auto-bond*
(implying node behaviour), yet no `TxType::AddBond` is constructed anywhere in `bins/node` and
epoch rewards are Normal outputs. **Resolved:** the auto-bonding is real but is an **operator cron**
outside the repository. Both statements are true of the system and false of the code. Recorded as
Drift #3.

⚠ **CONTRADICTION:** the prompt's claim 1 says `bondCount` is unchanged at 434; live RPC returns
`bondCount = 1`. **Resolved:** `getProducer.bondCount` is UTXO-derived when non-zero
(`producer.rs:81-84`); the 434 the prior session saw was the ProducerSet value surfacing through
`selectionWeight`. The observation was right about the symptom and wrong about the field.

---

```
━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.93, evidence)
Reasoning: mainnet, cross-layer state divergence between the UTXO and ProducerSet ledgers spanning 5+ interacting components (CLI, RPC, apply path, producer store, epoch boundary) plus a non-repo cron, brittleness 4/5, unbacked consensus weight still earning rewards, and the same window armed on 10 remaining exposed nodes.
━━━━━━━━━━━━━━━━━━━━━━
```
