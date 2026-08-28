# INC-I-170 — MAINNET producer retirement, live progress

> Host numbers below are positional only. The real SSH aliases and fleet layout live in
> the private ops repo and in `.claude/skills/mainnet/SKILL.md`, never in this public repo.

Procedure: `.claude/skills/producer-retirement/SKILL.md` (transaction-only path).
No consensus change, no activation height, no node binary deploy.

## Tooling (rebuild if the scratchpad is gone)

- Recovery CLI built from the **exact deployed mainnet commit `ca0b3093`** in a detached
  worktree, plus a 2-line patch (`withdrawal.rs` AND `exit.rs`: `payout_amount =
  bond_input_total + fee_change`). NEVER build it from the working branch — that branch is
  ~30 commits ahead and changes the wallet format to v3, which would strand fresh keys.
- Verify with `doli --version` → must print `6.24.1 (ca0b3093)`, matching
  the deployed `/mainnet/bin/doli --version` on a seed host.
- RPC access: `ssh -f -N -L 18500:127.0.0.1:8500 <seed-host>`. The tunnel dies between commands —
  open it in the SAME shell invocation as the command that uses it.
- Keys: `~/.ssh/doli/` (0700, files 0600). 1 cold wallet + 12 fresh producer keys.
  **The wallet FILE is the only backup of the BLS key** — at `ca0b3093`
  `BlsKeyPair::generate()` is random, NOT seed-derived. The 24 words are not enough.

## Baseline measured 2026-08-19 (h=242,087)

88 producers all active, total weight 17,625. Exposed = `producer_1..12` = 16,246 = 92.18%.
Honest = 76 producers = 1,379 = 7.82%. Mapping is exactly `nN -> producer_N.json`.

| node | server | weight | DOLI |
|---|---|---:|---:|
| n7,n8 | host 4 | 427 | 4,270 ea |
| n9,n10,n11,n12 | host 5 | 427 | 4,270 ea |
| n5 | host 2 | 2,260 | 22,600 |
| n6 | host 4 | 2,260 | 22,600 |
| n1,n2,n3 | host 1 | 2,291 | 22,910 ea |
| n4 | host 2 | 2,291 | 22,910 |

## Per-node sequence (self-funded re-key, one node at a time, human-gated)

1. `producer status` on the exposed key — record bond count. Guard: abort if it changed.
2. `request-withdrawal --count <all> --destination <cold bech32>` (patched CLI).
   The printout reports the HONEST penalized figure and is WRONG by design — verify on-chain.
3. Wait for the epoch boundary; confirm `status=exited`, weight 0, no fork.
4. `send` the residual reward coins from the exposed address to cold (sweep to 0.00000000).
5. `send` the recovered total from cold to the fresh `nN-new` address.
6. Stage key to the server (md5 check), then: stop service, back up old key, install new key,
   `chmod 600`, start service.
7. `producer register --bonds <same count>` from the fresh key. Activates at the next epoch
   boundary + 10 blocks.

## Status

| node | retired | swept | re-keyed | registered | active |
|---|---|---|---|---|---|
| **n12** | ✅ 431 bonds, 4,310.00 DOLI **full value, 0 burned** | ✅ 17.87439939 | ✅ | ✅ 431 bonds | ✅ **ACTIVE** (weight 433, self-bonded rewards) |
| n11, n10, n9, n8, n7, n6, n5, n4, n3, n2, n1 | — | — | — | — | — |

Transactions (n12): withdrawal `66bda952466afc1f6a63bc8d7438b5af69f53cae23c37c8b1af7b9df61d667ad`,
sweep `3ed0ceeff86dcc1fa51539a7e7b36c14e5077ffc02c05e314496c0267d155f62`,
funding `8ff4fc8da426e3ef0a544561cdf96a127f38cf25513816d083244486bd33d9d4`,
registration `928641e140c98cdfe0af7f0a7b8c7833efb494facda657f3cc321dde094982f5`.

## Findings

- **The 75% Q1 penalty is REAL on mainnet.** All 431 of n12's bonds were Q1. The honest CLI
  would have burned 3,232.50 DOLI. The patch returned full value. Testnet could not exercise
  this (testnet bonds were fully vested).
- **`producer exit` is a separate unpatched code path** (`exit.rs:168`). It would burn the
  penalty. Both files are patched now; never rely on only patching `withdrawal.rs`.
- **An exposed address keeps accruing reward coins after its bonds are gone.** Retirement is
  not complete until the final sweep takes it to 0.
- **The `attestation X/Y` figure in FINALITY log lines is the TRIGGER point, not a health
  margin.** Its minimum pins to ~67.0% across every producer-set total (17,771/17,731/
  17,699/17,375). Real participation is ~97%+ by weight (`ATTEST_MISS` shows 1-3 tiny
  producers). 923 finalizations, 0 failures. Do NOT read it as a thin margin.
- Honest weight cannot reach 67% during this procedure (needs ~270,000 DOLI of replacement).
  It is already 8% — the "pre-exit window" is the CURRENT STATE, not something we open.

## ⚠️ n11 STUCK — phantom producer (2026-08-19, requires decision)

**What happened.** n11 withdrawal `69d30f2a11c007b20cdc67608314cac069a7ae18b115633d89f52e8aa271f797`
requested **434** bonds. The producer-set "available" was **433** (one reward had just
auto-bonded as a Bond UTXO but the producer-set withdrawable count still read 433). Node log:

```
WARN WithdrawalRequest: not enough bonds (requested 434, available 433, delegated=0)
```

The producer-set deferred exit mutation was **rejected** by that WARN — but the **UTXO layer
still spent all 434 bond UTXOs and paid full value** (4,348.07989935 DOLI) to the cold wallet.

**Resulting divergence (uniform on all nodes — NOT a fork):**
- Bond/UTXO layer: `getBondDetails(b03fe629)` → `bondCount 0, bonds [], totalStaked 0`.
  `simulate-withdrawal --count 1` → "must be between 1 and 0" (no bond UTXOs to withdraw).
- Producer-set layer: `getProducers` → n11 `status active, selectionWeight 434, bondCount 434,
  pendingWithdrawals []`. **Still being scheduled and PRODUCING BLOCKS at epoch 680.**

**Money is safe** — 4,348 DOLI recovered to cold, 0 burned. The problem is n11's OLD exposed
identity is a phantom producer with 434 unbacked weight that no withdrawal can remove (every
removal path needs bond UTXOs, and there are none).

**Root-cause class:** RequestWithdrawal validates the UTXO spend and the producer-set mutation
INDEPENDENTLY; the count check is a non-fatal WARN, so `count > producer-set-available` spends
the bonds but skips the exit. Adjacent to INC-I-171 (withdrawal not consensus-enforced).

**Corrected guard for the remaining 10 nodes (n10..n1):** read the producer-set AVAILABLE
count with `simulate-withdrawal`, withdraw **at most** that — never the raw `getProducers`
`bondCount`. Safest: withdraw `available - 1`, then sweep the 1-bond residual the next epoch.

**Open decision:** how to remove n11's phantom weight (options under discussion — natural
ghost/weight-filter ejection via missed slots after re-key, vs a consensus-level fix). Do NOT
re-key or fund n11 until this is decided.

### n11 update (epoch 682, h=245,656) — phantom weight PERSISTS and grows

`getProducers(b03fe629)`: `selectionWeight=435`, `bondCount=1`, `bondAmount=10 DOLI`,
`pendingWithdrawals=[]`, `status=active`. `getBondDetails`: `bondCount=1`, `totalStaked=10 DOLI`.
Exposed address: 16.15954337 DOLI spendable (rewards earned while phantom-producing).

So across TWO epoch boundaries the withdrawal's weight decrease was never applied.
`selectionWeight` and `bondCount` have openly diverged (435 vs 1) and `selectionWeight` only
grew (434 → 435) as one reward auto-bonded. The dashboard column shows `bondCount`/`bondAmount`
(1 / 10 DOLI), NOT the scheduling weight (435). n11 keeps being scheduled and earning on the
exposed key. Recovery path still pending the investigation.

## Auto-bond cron frozen for exposed nodes (2026-08-20)

The hourly `/mainnet/scripts/auto-bond-nN.sh` (runs `producer add-bond` on each node's spendable
balance) is the DRIFT SOURCE behind the n11 phantom-weight failure: it creates a Bond UTXO
faster than the ProducerSet reflects it, so the UTXO bond count U exceeds ProducerSet-available
P, and a withdrawal of U strands U−P as phantom weight.

Commented the cron lines for **n1–n11** on hosts 1/2/4/5; **n12 left active** (re-keyed,
legitimate). Safe method: dump crontab to `~/crontab.bak.<ts>`, sed the dumped file, assert the
line count is unchanged, then `crontab <file>` — NEVER `crontab -l | sed | crontab -`.

Effect: bond counts on n1–n11 are now frozen (U = P), so the corrected guard `N ≤ P − W` and a
plain full-count withdrawal agree. Re-enable each node's cron (uncomment) after it is re-keyed,
or fleet-wide once the retirement is complete.

## BATCH 1 complete (2026-08-20) — n7, n8, n9, n10

Proved batching works. All four withdrawn in one epoch (guarded U==P==445, cron frozen), all
exited together at the epoch-693 boundary (h=249481, exited/0, no fork), residuals swept to
0, re-keyed on servers (n7,n8 host 4; n9,n10 host 5), funded 4451 each from cold, re-registered 445
bonds each. All four show Activating:4450, activate together at epoch 694 (h=249840). Auto-bond
crons re-enabled for n7-n10. Each recovered full value, 0 burned.

Cold wallet after funding: ~4348 DOLI (= n11's stuck recovery, earmarked; the four batch-1
portions were paid back into their fresh keys).

STATUS: n12 done; n7,n8,n9,n10 done (activating 694); n11 stuck (phantom, Exit recovery pending
testnet rehearsal); n1-n6 (heavies ~2290 each) remaining.

## BATCHES 2 & 3 complete — 11 of 12 exposed identities neutralized (2026-08-20)

Full audit at epoch 698 (h~251284): n1-n10 and n12 ALL show status=exited, weight 0, bonds 0.
Batch 2 (n4,n5,n6) replacements ACTIVE. Batch 3 (n1,n2,n3) withdrawn+exited+swept+re-keyed on
host 1+funded+re-registered 2386 bonds, activating epoch 699. All crons re-enabled n1-n10,n12.
Cold wallet ~5,100 DOLI (n11 earmark + spares). ZERO burned across all 11 nodes, NO fork ever.

ONLY REMAINING: n11 phantom producer (status=active, weight 444, bonds 10). Needs the
INC-I-180 Exit-tx recovery, verified on LOCAL testnet first. Do NOT improvise on mainnet.

## n11 RESOLVED — 12 of 12 exposed identities neutralized (2026-08-28, INC-I-180)

The last exposed identity is retired. n11's unbacked weight is cleared by ONE transaction, using
the R2 full-exit arm that shipped with the INC-I-180 withdrawal-holdings gate (mainnet
`withdrawal_holdings_gate_activation_height` = 317_861, crossed at the time of the write).

**Measured before the write** (h=318,874, deployed commit `9e27bd19` / 6.25.0):
P (`producerSetBondCount`) = 444, U (`getBondDetails.bondCount`) = 10 (all Q1, 75% tier),
`selectionWeight` = 444 (1.908% of 23,270 fleet weight), `pendingUpdates` = [],
`withdrawalPendingCount` = 0. n11's auto-bond cron on host 5 was confirmed still commented out, so
no bond could appear mid-flight.

**The transaction.** `575768a928cea20725b59140a089700ea5a753ca6936dbf2fa779a9d742429df`, mined at
h=318,877. Shape: **declared = 444 (the full ProducerSet allowance), inputs = all 10 owned Bond
UTXOs** + 1 normal fee UTXO; 11 inputs, 1 output. Built with a guard-free CLI compiled from the
DEPLOYED commit (`DOLI_I180_DECLARE_COUNT` decouples the declared count from `--count`), plus the
full-value payout patch in BOTH `withdrawal.rs` and `exit.rs`.

**Payout: 108.72251022 DOLI to the cold wallet, ZERO burned.** That is 100.00000000 of full bond
value (the honest CLI would have burned 75.00000000 across the ten Q1 bonds) plus the 8.72 fee-UTXO
change, minus the 1-satoshi fee. The CLI still PRINTED "You receive: 25.00000000 / Penalty burned:
75.00000000" — that printout is wrong by design and must never be used as verification.

**Exit confirmed at both layers.** RPC: `pendingUpdates: [{bondCount: 444, updateType:
"withdrawal"}]`. Node log on host 5: `Queued WithdrawalRequest (444 bonds) ... at height 318877
(deferred to epoch boundary)`, with ZERO "not enough bonds" warnings in the whole log.

**Transient (expected, ~84 blocks):** P=444, U=0, weight=444, still `active`. This does NOT reopen
the INC-I-182 vacuous-exit window: `withdrawal_pending_count += 444` is applied when the tx MINES
(`apply_block/tx_processing.rs`), so the allowance immediately becomes 444-444 = 0 and any further
withdrawal declaring >0 is rejected by R1.

**After the epoch-886 boundary (h=318,961):** `producerSetBondCount` 0, `bondCount` 0, `bondAmount`
0, `selectionWeight` 0, **`status` = exited**.

**No fork.** Block 318,962 hash `3f0dd5f3096e485206deabde237294da171bdbb8578957027dfe826ecea15154`
identical on all five structural hosts. Fleet-wide rescan: producers with P != U went from 5 to
**0** (the other four were U = P+1 auto-bond cron race on n9/n10/n12 and a peer, which the same
boundary flushed). Producers in the INC-I-182 vacuous shape (P>0, U==0): **0**.

**Sweep DONE.** The exposed address also held **1,620.47088232 DOLI spendable across 195 normal
UTXOs** — accumulated block reward the frozen auto-bond cron never bonded, and 16x larger than the
bonds this recovery returned. The INC-I-180 session prompt had described this as "small spare
coins"; the live `getBalance` did not agree. Swept in one transaction
`2626d746f917fd7c367d691200811d7c8fad9b3a287b34918d28038ad00c5550` (195 inputs, 1 output,
32,275 bytes, 1-satoshi fee), mined at h=318,982. `minimum_fee()` is `BASE_FEE = 1` and scales only
with output `extra_data`, NOT with input count — so a 195-input sweep still costs 1 satoshi and the
drain amount is simply `balance - 1`.

**Final state.** Exposed address `doli1ge79z234s825pummwlvj4cxf2ashzn8c5cv59cr76lkv705qanpsq7nt9u`
= **0.00000000 DOLI**. Cold wallet = 7,778.22072288 DOLI. n11 producer record: bondCount 0,
selectionWeight 0, status exited. Block 318,982 hash
`329e81907ba9756509a757aee783116eeaa4cbdbcf11802486fcad84db60d0f5` identical on all five structural hosts. No fork.

**INC-I-170 and INC-I-180 are both closed.** All 12 exposed identities are neutralized, ZERO DOLI
burned across the whole campaign, and the chain never forked.

**NEXT (decided 2026-08-28, not yet started):** re-key n11 to `~/.ssh/doli/n11-new.json` on host 5 and
re-register it. Re-enable `/mainnet/scripts/auto-bond-n11.sh` ONLY after the re-key lands.
