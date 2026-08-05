---
name: producer-bootstrap
description: "Post-genesis producer bootstrap — fund all structural nodes (N1-N12) from a source wallet, distribute DOLI equitably across the fleet, and register/bond each as a block producer with BALANCED selection weight. Use after a genesis reset or when standing up the producer set from scratch. Covers send/register/add-bond CLI, bond-unit math, equitable-distribution routing, epoch-deferred activation, and the weight-balance safety rule. Triggers on: 'fresh genesis', 'genesis reset', 'fund the producers', 'register N6-N12', 'bond all producers', 'distribute DOLI equally', 'bootstrap the fleet', 'stand up producers', 'selection weight', 'add-bond', 'producer register'."
version: 1.0.0
user_invocable: true
---

# DOLI Producer Bootstrap (post-genesis)

The runbook for taking a freshly-reset chain (or a chain where the structural
nodes hold DOLI but aren't yet producers) to a **balanced 12-producer set**.

Three phases: **(1) Fund → (2) Distribute equitably → (3) Register + bond.**
All steps are ordinary transactions on the live chain — **no binary deploy, no
synchronized restart**. The only consensus effect (producer-set change) is
**deferred to the next epoch boundary** by design.

> First run: 2026-07-16. This skill is the distilled runbook from that session.

---

## Fleet map — server / port / wallet (memorize this)

Each structural node's producer identity **and** spendable wallet is the single
file `producer.json` (holds Ed25519 + BLS keys). The CLI is `/mainnet/bin/doli`.

| Node | Server (SSH) | RPC port | Wallet / producer key | Data dir |
|------|--------------|----------|-----------------------|----------|
| N1 | ai1 | 8501 | `/mainnet/n1/keys/producer.json` | `/mainnet/n1/data` |
| N2 | ai1 | 8502 | `/mainnet/n2/keys/producer.json` | `/mainnet/n2/data` |
| N3 | ai1 | 8503 | `/mainnet/n3/keys/producer.json` | `/mainnet/n3/data` |
| N4 | ai2 | 8504 | `/mainnet/n4/keys/producer.json` | `/mainnet/n4/data` |
| N5 | ai2 | 8505 | `/mainnet/n5/keys/producer.json` | `/mainnet/n5/data` |
| N6 | ai4 | 8506 | `/mainnet/n6/keys/producer.json` | `/mainnet/n6/data` |
| N7 | ai4 | 8507 | `/mainnet/n7/keys/producer.json` | `/mainnet/n7/data` |
| N8 | ai4 | 8508 | `/mainnet/n8/keys/producer.json` | `/mainnet/n8/data` |
| N9 | ai5 | 8509 | `/mainnet/n9/keys/producer.json` | `/mainnet/n9/data` |
| N10 | ai5 | 8510 | `/mainnet/n10/keys/producer.json` | `/mainnet/n10/data` |
| N11 | ai5 | 8511 | `/mainnet/n11/keys/producer.json` | `/mainnet/n11/data` |
| N12 | ai5 | 8512 | `/mainnet/n12/keys/producer.json` | `/mainnet/n12/data` |

- Port rule: **RPC = 8500 + node number**. Seed RPC = 8500.
- Commands run on-server via `ssh <alias>` + `sudo` (files are `<user>:<group>`).
- Always pass `-r http://127.0.0.1:<port>` explicitly. Balance can auto-detect but be explicit.
- **Faucet wallets live on ai3** (separate from producers): hot faucet
  `/mainnet/faucet/keys/wallet.json` + reserve `/mainnet/faucet-vault/keys/wallet.json`.
  Fund the vault the same way you fund a node (see Phase 1).

### Identity cross-check (do this once)
The on-server `address` in `producer.json` is the **20-byte prefix** of the
32-byte structural hash hardcoded in `crates/core/src/consensus/constants.rs`
(`STRUCTURAL_PUBKEY_HASHES_HEX`, one per N1-N12). Before bootstrapping, confirm
each node's key matches its slot:

```bash
# on the node's server
sudo /mainnet/bin/doli -w /mainnet/n6/keys/producer.json addresses   # doli1... form
# hex prefix must equal the first 40 hex chars of STRUCTURAL_PUBKEY_HASHES_HEX[N-1]
```
A mismatch means the wrong key is deployed to that slot — STOP.

---

## Key constants (from `crates/core/src/consensus/constants.rs`)

| Constant | Value | Meaning |
|----------|-------|---------|
| `BOND_UNIT` | `1_000_000_000` = **10 DOLI** | 1 bond = 10 DOLI. Bonds are whole units only. |
| `MAX_BONDS_PER_PRODUCER` | **3000** | Hard cap = 30,000 DOLI bonded. (CLI accepts 1-10000 but consensus caps at 3000.) |
| `INITIAL_BOND` | 1 bond (10 DOLI) | Default `register` stake. |
| selection weight | = **bondCount** | Block production share is proportional to bond count. |
| activation | **epoch boundary** + `ACTIVATION_DELAY` (10 blocks) | Register/add-bond do not take effect immediately. |
| withdrawal | `request-withdrawal` | FIFO, 7-day delay + vesting penalty. Bonds are NOT instantly liquid. |

**Max whole bonds from a balance** = `floor(spendable_DOLI / 10)`, and leave a few
DOLI for the tx fee (fee is ~1e-8 DOLI, trivial, but the odd sub-10 remainder
can't be bonded). Example: 5046 DOLI → **504 bonds** (5040 DOLI), 6 DOLI remains.

---

## Phase 1 — Fund the fleet from a source wallet

Pick the wallet that holds the initial supply (post-genesis this is usually a
genesis-funded node, e.g. N1). Send with `send <TO> <AMOUNT> --yes`:

```bash
ssh <ai1> "sudo /mainnet/bin/doli -w /mainnet/n1/keys/producer.json \
  -r http://127.0.0.1:8501 \
  send <doli1-recipient> <amount> --yes"
```

- `<TO>` accepts a `doli1...` bech32m address (preferred) or hex. Get a node's
  address with `doli -w <wallet> addresses` (plural — **never** `doli address`
  singular, which creates a NEW address).
- `send` **does** take `--yes` to skip the confirm prompt.
- Fee is auto (~0.00000001 DOLI). Output prints `TX Hash:`.

Confirm receipt: the recipient shows `Pending: <amt>` then `Spendable` after the
next block (~10s):
```bash
ssh <ai3> "sudo /mainnet/bin/doli -w /mainnet/faucet-vault/keys/wallet.json balance"
```

---

## Phase 2 — Equitable distribution across N1-N12

Goal: every node ends with roughly the same spendable balance, drawing only from
the nodes that currently hold funds.

### Math
1. `POOL = Σ spendable(source nodes)`.
2. `share = POOL / 12` (round to whole DOLI; a few DOLI drift is fine).
3. Each source **keeps** `share` and **sends its excess** (`current − share`) out
   to the zero-balance nodes until every node holds ~`share`.

### Routing (minimal transactions)
For S sources feeding K sinks, a fully-balanced flow needs **S + K − 1**
transactions (a transportation-problem minimum). Give each sink one whole `share`
from a single source where possible; only fragment the leftover.

Worked example from the first run (POOL = 60,549 across N1-N5; N6-N12 = 0;
share ≈ 5046, rounded whole DOLI, 11 transactions):

| From | → To | DOLI |  | From | → To | DOLI |
|------|------|------|--|------|------|------|
| N2 | N6 | 5046 || N3 | N11 | 1427 |
| N3 | N7 | 5046 || N4 | N11 | 3236 |
| N4 | N8 | 5046 || N5 | N11 | 383 |
| N5 | N9 | 5046 || N5 | N12 | 2854 |
| N2 | N10 | 3236 || N1 | N12 | 2192 |
| N3 | N10 | 1810 ||  |  |  |

Run sends **sequentially per source wallet** (the mempool marks spent UTXOs so
back-to-back sends don't double-spend). Batch per server with `&&`.

### Gotcha: producers earn while you work
Active producers mint **72-DOLI coinbase reward UTXOs** as they build blocks. A
source node can end a few DOLI above `share` because it produced a block mid-run.
This is expected — "equitable" tolerates it. Don't chase exact equality.

---

## Phase 3 — Register + bond, with BALANCED weight

### ⚠️ The one rule that matters: keep bond counts EQUAL
`selectionWeight == bondCount`, and block production is proportional to weight.
If you bond N6-N12 to 504 while N1-N5 stay at 1, then **N6-N12 produce ~99.86% of
blocks and N1-N5 stop producing** — a self-inflicted centralization on mainnet.

> **Always bond every producer to the same count.** New nodes `register --bonds K`;
> already-registered nodes `add-bond --count (K − existing_bonds)`.

### Commands (note the DIFFERENT flags, and neither takes `--yes`)
```bash
# NEW producer (not yet registered): --bonds
echo y | sudo /mainnet/bin/doli -w /mainnet/n6/keys/producer.json \
  -r http://127.0.0.1:8506 producer register --bonds 504

# EXISTING producer (bond stacking): --count
echo y | sudo /mainnet/bin/doli -w /mainnet/n1/keys/producer.json \
  -r http://127.0.0.1:8501 producer add-bond --count 503
```
- `register` uses `--bonds`; `add-bond` uses `--count`. Easy to mix up.
- **Neither accepts `--yes`** (only `send` does). Pipe `echo y` for non-interactive ssh.
- `register` reads the BLS pubkey + PoP from the wallet automatically — no BLS flags.
- Bond count must be whole; `register --bonds K` stakes `K × 10` DOLI.

### Balanced-bootstrap recipe (all 12 → K bonds)
Let `K = floor(min_spendable / 10)` across the fleet (use the smallest so all can
match). First run used **K = 504**:
- N6-N12 (fresh): `producer register --bonds 504`  → 5040 DOLI each
- N1-N5 (had 1 bond each): `producer add-bond --count 503`  → 504 total each

Batch per server:
```bash
ssh <ai4> 'for i in 6 7 8; do p=$((8500+i)); \
  echo y | sudo /mainnet/bin/doli -w /mainnet/n$i/keys/producer.json \
  -r http://127.0.0.1:$p producer register --bonds 504 2>&1 | grep -iE "TX Hash|error"; done'
```

---

## Verification

**Immediately after** (tx applied, weight NOT yet active):
```bash
sudo /mainnet/bin/doli -w /mainnet/n1/keys/producer.json -r http://127.0.0.1:8501 balance
#   Bonded: 5040.00000000 DOLI  (producer bond)   ← bond UTXOs created at apply
sudo /mainnet/bin/doli -w /mainnet/n6/keys/producer.json -r http://127.0.0.1:8506 producer status
#   Status: Not registered / "pending activation at the next epoch boundary"  ← EXPECTED
```
`producer status` reads "Not registered" until the epoch boundary — that is normal
for a just-submitted registration, not an error.

**After the next epoch boundary** (add-bond output prints the ETA, e.g.
"~51 minutes, Epoch 187, block 67320") confirm all 12 are active with equal weight:
```bash
ssh <ai4> 'curl -s -X POST http://127.0.0.1:8506 -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getProducers\",\"params\":[]}"' \
  | python3 -m json.tool | grep -E "addressHash|bondCount|selectionWeight|status"
```
Expect 12 producers, `bondCount == K`, `selectionWeight == K`, `status == active`.

---

## Learnings / pitfalls (from the first run)

1. **`send` takes `--yes`; `register`/`add-bond` do NOT** — they error on `--yes`.
   Pipe `echo y` instead. (Nothing broadcasts on the arg error, so it's safe to retry.)
2. **`register --bonds` vs `add-bond --count`** — different flag names for the count.
3. **Bonds are 10-DOLI units** — you can never bond the sub-10 remainder; leave it
   as spendable (also covers the fee).
4. **Weight = bondCount** — unequal bonds ⇒ unequal (potentially ~100%) production.
   The default `register` (1 bond) is the balanced baseline; scale all nodes together.
5. **Epoch-deferred** — registrations/add-bonds activate at the next epoch boundary
   (+10-block activation delay). Don't expect `getProducers` to show them until then.
6. **Producers earn 72-DOLI rewards mid-operation** — balances drift up slightly; fine.
7. **Bonds lock** — undoing needs `request-withdrawal` (FIFO, 7-day delay + vesting
   penalty). Bond only what you intend to commit.
8. **This is NOT a deploy** — no binary change, no synchronized restart, no activation
   height. It is the normal transaction path; the only consensus change (producer set)
   is epoch-gated automatically. (Per CLAUDE.md deploy checklist: Q1 consensus RULES?
   No — no code change. Q2 block CONTENT? No. So no AH / no synchronized restart.)
9. **Mainnet value transfers are irreversible** — show the plan (from/to/amount table)
   and get explicit approval before broadcasting. One seed/producer at a time if unsure.
10. **Identity is `producer.json`** — same file is wallet AND producer key; it carries
    the BLS keys `register` needs. Back it up; losing it loses the producer.

## References
- Fleet deploy & server layout: `.claude/skills/mainnet/SKILL.md`
- Recovery / fork / checkpoint: `.claude/skills/guardian/SKILL.md`, `.claude/skills/mainnet/RECOVERY.md`
- Bond/producer consensus internals: `.claude/skills/core/SKILL.md`, `crates/core/src/consensus/constants.rs`
- Delegation (weight without self-bond): `.claude/skills/delegation/SKILL.md`
