# INC-I-180 — LOCAL testnet reproduction & fix verification

**Date:** 2026-08-24 · **Network:** LOCAL testnet (`~/testnet/`, 127.0.0.1) · **Branch:** `bugfix/inc-i-180-withdrawal-holdings-gate` @ `8744711f`
**Scope:** local only. No push, no mainnet, no genesis reset.

## Environment

| Item | Value |
|---|---|
| Fleet | seed + n1..n12 (13 processes), synchronized start 16:49:52–16:50:08 |
| Node binary | `~/testnet/bin/doli-node` — contains `ECON_WITHDRAWAL_OVER_HOLDINGS` (pre-fix backup does not) |
| `withdrawal_holdings_gate_activation_height` (testnet) | **15_087** (uncommitted re-pin; was 230_000) |
| Tip at test start | 15_128 → gate **ACTIVE** (no genesis reset needed) |
| Mainnet AH | `u64::MAX` — untouched |
| `bond_unit` (testnet) | 100_000_000 base units = **1 DOLI** (`defaults.rs:350`) |
| Epoch length | 36 blocks · `ACTIVATION_DELAY` = 10 |

Test producer A: pubkey `a1359c40…4b22`, wallet `~/testnet/keys/i180_test_a.json`, funded 200 DOLI from `producer_1`.

## Reproduction of the n11 shape (U > P window)

| Step | Height | Action | Result |
|---|---|---|---|
| 1 | 15_272 | `register --bonds 2` | queued, epoch-deferred |
| 2 | 15_301 | epoch 425 boundary | **active**: `bondCount=2, producerSetBondCount=2, selectionWeight=2` |
| 3 | ~15_303 | `add-bond --count 1` | Bond UTXO created immediately; ProducerSet update deferred |
| 4 | 15_304 | read ledgers | **U=3, P=2** — `bondCount:3` vs `producerSetBondCount:2`, `pendingUpdates:[{bondCount:1}]` |

Step 4 is the exact mainnet n11 shape (there: P=433, U=434).

## Assertion results

### A3 — M3 CLI guard (HEAD `doli`) — **PASS**
`doli producer request-withdrawal --count 3` at U=3/P=2:
```
Error: Ledger mismatch: wallet shows 3 Bond UTXO(s) but the ProducerSet allowance is 2.
The node would reject or misapply this withdrawal. Aborting — retry after the next
epoch boundary flushes pending bond changes.
```
The client refuses to emit the n11-shape tx at all. To reach the consensus gate the tx had
to be driven by a guard-free CLI built at `448dca75` (M2 — 0 occurrences of "Ledger mismatch";
HEAD has 2).

### A2 — over-allowance REJECTED — **PASS (live, mempool)**
Harness build of the M2 CLI with a `DOLI_I180_DECLARE_COUNT` override declares `bond_count=4`
while spending the 3 real Bond UTXOs. Node response at h=15_307:
```
RPC -32002 (INVALID_TRANSACTION): [ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at
height=15307 producer=854eacbc… requests 4 bonds but allowance is 3
(held=2, pending_addbond=1, withdrawal_pending=0, in_mempool_withdrawn=0)
stage: mempool
```
This confirms the allowance formula live: `allowance = held(P=2) + pending_addbond(1) = 3`.

**Anti-theft property verified** — immediately after the rejection the producer was untouched:
`bondCount=3, totalStaked=300000000, withdrawalPendingCount=0, selectionWeight=2, status=active`.
No Bond UTXO was spent.

Block-validation rejection (as opposed to mempool admission) is covered by the Rust suite —
it is not reachable from a live testnet because M2's mempool guard rejects the tx before any
producer can assemble it into a block.

### A1 — legitimate full withdrawal ACCEPTED — **PASS**
`request-withdrawal --count 3` (= allowance 3, crediting the pending AddBond) was accepted and
mined. Transient window observed at h=15_310…15_323 — **`utxoBonds=0, producerSetBondCount=2,
selectionWeight=2, status=active`** — this is precisely the n11 shape, which the OLD code made
permanent. Resolution at the epoch boundary is recorded below.

### A4 — RPC ledger visibility — **PASS**
`getProducer` / `getProducers` expose both ledgers as distinct fields:
`bondCount` (UTXO-derived, un-masked) and `producerSetBondCount` (ProducerSet-derived).
At h=15_304 they read 3 and 2; during the transient they read 0 and 2. A producer holding
weight with zero Bond UTXOs is therefore detectable from RPC alone — it was not before.

### A1 (resolution) — clean exit at the epoch boundary — **PASS**
Pending queue before the flush (order matters, and it is queue-insertion order —
`set_core.rs:106-175`): `[add_bond{1}, withdrawal{3}]`.

At h=15_336 (epoch 426 boundary): AddBond flushed first (P 2→3), then the withdrawal
(3−3=0) → auto-exit at zero (INC-I-056):
```
bondAmount:0  bondCount:0  producerSetBondCount:0  selectionWeight:0  status:"exited"
```
`pendingUpdates` cleared. **No unbacked weight.** Under the pre-fix code this same tx
sequence leaves `selectionWeight=2` with zero Bond UTXOs, permanently.

Confirmed identically on nodes 8500 / 8501 / 8506 / 8512.

### A5 — no fork — **PASS**
All 13 nodes agreed on a single height/hash at every checkpoint:
15_140 `cd4ec6be…` → 15_315 `8a1b3ed5…` → 15_337 `3423aebe…` (13/13 each time).
`grep -c "unbacked producer weight will result"` = **0 in every node log**.

## Test suites (`cargo test --release … inc_i_180`)

| Package | Result |
|---|---|
| `doli-node` | **69 passed / 0 failed** — includes 11 pre-AH legacy-behaviour controls |
| `storage` | 9 passed / 0 failed |
| `doli-cli` | 13 passed / 0 failed |
| `mempool` | 6 passed / 0 failed |
| `rpc` | 3 passed / 0 failed |
| `doli-core` | 2 passed / **3 failed** — see below |

### Negative control (pre-AH legacy behaviour)
The gate is a pure `if height >= AH` (`validation_checks.rs:621`), so below the AH the old
path runs bit-identically. 11 tests pin that, e.g.
`req_i180_003_pre_ah_under_declared_keeps_legacy` (`gate_bindings.rs:590`) declares 1 bond
while spending 434 and asserts the legacy verdict `(bond_count 433, Active, weight 433)` —
the ledger half of "bonds spent, weight kept". **Caveat:** no INC-I-180 test executes the real
UTXO spend; the harness calls `process_transaction_producer_effects` against a throwaway
`UtxoSet` and never `apply_block`. The UTXO-destruction half is documented as inference "O3",
not executed.

## ⚠️ Blocker for committing the working tree

Three `doli-core` tests fail — **not from the fix logic**, but from the uncommitted testnet
AH re-pin that made this reproduction possible:

| Test | Expected | Actual |
|---|---|---|
| `req_i180_003_testnet_gate_is_pinned_near_future` | 230_000 | 15_087 |
| `req_i180_003_no_existing_activation_height_was_moved` (testnet `maintainer_derivation`) | 127_200 | 15_087 |
| `req_i180_003_the_gate_is_dedicated_and_not_bundled` | #23 ≠ #22 | both 15_087 |

These are the AH-discipline guards doing their job: the working tree collapsed **four** distinct
testnet gates (#172 derivation, #173 fee, #176 auth-binding, #180 holdings) onto the single
height 15_087. All **mainnet** assertions still pass — mainnet gate frozen at `u64::MAX`,
`maintainer_derivation` still 172_000. Nothing mainnet-facing moved.

Before any commit: either revert the testnet re-pin, or update these three tests to the new
pinned values with an explicit rationale.

## Verdict

**INC-I-180 is solved on the LOCAL testnet.** The n11 shape was reproduced exactly (U=3 > P=2
with one unflushed AddBond), and on the fixed binary it can no longer produce unbacked weight:
the legitimate full withdrawal is accepted and exits cleanly to weight 0, while an
over-allowance withdrawal is rejected before any Bond UTXO is spent. The fleet never forked.

**Not covered here:** block-validation-stage rejection (unreachable live — the mempool guard
rejects first; covered by the Rust suite), and real on-disk UTXO destruction in the pre-AH
control (inferred, not executed).

---

# Addendum — closing the two evidence gaps (live)

The first pass left two things unproven: block-validation-stage rejection, and real on-disk
UTXO destruction. Both were closed by one live experiment.

## Method — isolating the single variable

A worktree of HEAD `8744711f` was built with the working tree's `defaults.rs` copied in verbatim
and **only** `withdrawal_holdings_gate_activation_height` (testnet) changed to `u64::MAX`. Gates
#172 / #173 / #176 stayed at 15_087, identical to the fleet — so the ONLY behavioural difference
between this node and the other 12 is INC-I-180. (Using the older `ca0b3093` backup binary would
have confounded the test: it predates the re-pin and would also disagree on gates #172/#173/#176.)

Note on why an M1-only build cannot do this: the producer applies its own block through
`apply_block` (`production/mod.rs:620` → `validate_block_economics`, `apply_block/mod.rs:113`),
so an M1 node rejects its **own** block as `[BLOCK_POISON]` and never gossips it. Only a node
with the gate fully off will emit the bad block.

The binary was installed as `doli-node-n1` (n1 is the one node with a dedicated binary path, so
the shared `doli-node` used by seed+n2..n12 was never touched). Stop/start via
`scripts/testnet.sh`, MD5-verified restore point taken first.

Test producer B: `c70a72d4…823d`, registered with 2 bonds, active at h=15_733 with U=P=2,
allowance=2. Over-declare tx (`bond_count=3` declared, 2 Bond UTXOs spent) submitted **only** to
n1's RPC on :8501. The gated nodes reject that shape at mempool; n1 accepted and retained it.

## Gap 1 — block-validation rejection, ON THE WIRE — **CLOSED**

n1 built block h=15_745 containing the tx. Every other node rejected it at
`doli_node::node::block_handling` — the **block-validation** stage, not mempool:
```
WARN doli_node::node::block_handling: [BLOCK] REJECT slot=1055605 h=15745 producer=20204725
error=[ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at height=15745 producer=5a24203e…
requests 3 bonds but allowance is 2 (held=2, pending_addbond=0, in_block_addbond=0, …)
```
Rejection counts in the logs: n2=2, n3=2, n4=2, n5=1, n6=2, n7=2, n8=2. `producer=20204725` is
n1's key (`202047256a8072a8`). The 12 fixed nodes stayed on one hash and kept advancing.

## Gap 2 — real on-disk UTXO destruction — **CLOSED**

n1, with the gate off, applied its own block and logged the defect firing for real:
```
ERROR doli_node::node::apply_block::tx_processing: WithdrawalRequest: not enough bonds
(requested 3, available 2, delegated=0) — unbacked producer weight will result
```
Same producer, two nodes, same height — the differential:

| | `bondCount` (UTXO) | `producerSetBondCount` | `selectionWeight` | status |
|---|---|---|---|---|
| **n1 — gate OFF** | **0** | 2 | **2** | active |
| **n2 — gate ON** | 2 | 2 | 2 | active |

n1 shows **unbacked weight with the collateral destroyed on disk** — the genuine n11 defect on a
real chain, executed through the real `apply_block`/`process_transaction_utxos` path. This is no
longer the inferred "O3" observable. On every fixed node the same tx never reached a block and
not one Bond UTXO was spent.

## Fork containment

n1 diverged to its own tip (h=15_745 `f1180c5e…`) while the fleet continued on
`e0c1989e…` → `1e2b5cf1…`. 12/12 nodes stayed mutually consistent throughout — the bad block
never entered the canonical chain. n1 was then stopped, the HEAD binary restored (MD5 verified
against the pre-swap capture), and restarted.

### Recovery — n1 self-healed

n1 raised `[STUCK_FORK]` and the finality-guarded rollback coordinator reverted its own block:
```
INFO doli_node::node::rollback: [FORK] ROLLBACK_DONE h=15744 hash=e746e094… cumulative_depth=1
INFO doli_node::node::periodic:  [SYNC_STATE] gap=0 phase="Synchronized" rollback_depth=0
```
Elapsed ~2 minutes from restart, no manual backfill or wipe needed. Final state:

- **13/13 nodes** at h=15_762, hash `14f302067bb98db7` — uniform.
- Producer B on n1 and n2 alike: `bondCount=2, producerSetBondCount=2, selectionWeight=2, active`
  — the unbacked-weight state was rolled back with the block that created it.

Worth noting for the incident record: a node that produces a block the fleet rejects does **not**
recover instantly. It sat wedged for ~2 min emitting `[STUCK_FORK]` while
`suppressing rollback cascade` held the rollback back. That is the finality guard behaving
conservatively by design, not a defect — but it is the observed recovery latency.

## Revised gap status

| Gap | First pass | Now |
|---|---|---|
| Block-validation-stage rejection | inferred (Rust suite only) | **proven live on the wire** — `[BLOCK] REJECT … [ECON_WITHDRAWAL_OVER_HOLDINGS]` on 7+ nodes |
| Real on-disk UTXO destruction | inferred ("O3") | **proven live** — `bondCount:0` + `selectionWeight:2` on the gate-off node, real `apply_block` path |

## Permanent regression test

`bins/node/tests/it/inc_i_180_apply_block_utxo_destruction.rs` (new, 3 tests) drives the real
`Node::apply_block` and reads the real `UtxoSet` back, so the destruction half is now an
executed assertion in CI rather than an inference:

| Test | Proves |
|---|---|
| `req_i180_003_pre_ah_apply_block_destroys_bonds_and_keeps_weight` | below the gate: apply Ok, `count_bonds → 0`, every seeded outpoint gone, `bond_count`/`weight` still 2, nothing queued — the defect |
| `req_i180_003_post_ah_apply_block_rejects_and_spends_nothing` | at/above the gate: Err carrying `ECON_WITHDRAWAL_OVER_HOLDINGS`, all 3 Bond UTXOs still live — the anti-theft property |
| `req_i180_003_fixture_actually_seeds_bond_utxos` | guards against a seeder that stops emitting Bond-typed outputs, under which the other two would pass vacuously |

Three things the existing suite's constants and harness could not supply, all documented in the
file header:

1. **UTXO backend.** The gate reads `node.utxo_set` while the spend path reads the `state_db`
   batch. A node from `new_for_test` has an in-memory set the spend path cannot see, so a seeded
   bond yields `OutputNotFound`, not a spend. The fixture repoints `utxo_set` at
   `UtxoSet::from_state_db` first, so all three views agree.
2. **`POST_AH = 1_000_007` is unusable here** — past the emission decay, so
   `block_reward(height)` is 0 and the coinbase trips `[ERRTX003] output 0 has zero amount`
   inside `validate_block_for_apply`, before the gate is consulted. Local `APPLY_POST_AH = 25`.
3. **`PRE_AH = 5` is unusable here** — `update_height_index` walks back via `prev_hash`
   (`block_store/writes.rs:162`) and dies on `[STOR020] header 000…0 missing during chain walk`
   with an empty store. The loop only terminates at `height == 0` (:149), so
   `APPLY_PRE_AH = 1`. Both keep the devnet gate (20) on the correct side.

Result: `3 passed; 0 failed`.
