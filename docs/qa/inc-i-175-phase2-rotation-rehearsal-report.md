# INC-I-175 Phase 2 — maintainer rotation rehearsal (LOCAL testnet)

Date: 2026-08-29 · Network: local testnet (`127.0.0.1`) · Binary: 6.25.0 + the M4 signer fix
Chain: genesis `f6cc888a…b7fc`, rotation applied h=56,838 → h=56,883

## Verdict

**GREEN.** The full 5-key rotation ran end to end. Ten transactions (Remove→Add ×5) all
mined and applied, the set never dropped below 4, and all 18 nodes converged on one
maintainer-set digest and one `(height, hash)`. Zero exposed keys remain on the testnet set.

Two defects were found on the way. Neither is in the chain rules; both are in the path an
operator actually walks.

## What was wrong before this run

### 1. The signer emitted the wrong bytes (fixed here)

`doli-node maintainer add|remove` signed via `MaintainerChangeData::signing_message`
(`crates/core/src/maintainer/data.rs:93`), which is `signing_message_legacy` — the frozen
pre-INC-I-176 message `add|remove:<target_hex>`. The apply path
(`bins/node/src/node/apply_block/governance.rs:97,157`) verifies
`signing_message_at`, which at or above `inc_i_176_auth_binding_activation_height` (#22) is the
domain-tagged, genesis-bound BLAKE3 digest. Both live networks are above #22, so every
signature the shipped tool produced was unverifiable.

Fix: `bins/node/src/commands/maintainer.rs::maintainer_auth_message` composes the message from
the SAME owner and the SAME two inputs the apply path reads — this chain's genesis hash and #22
— and both CLI arms call it. `--height` is now a required argument on `maintainer add` and
`maintainer remove`: it selects WHICH bytes are signed and nothing else. The command also prints
the full signer pubkey, the full signature, and the **preimage**, so the operator can read the
domain tag, genesis hash, action byte, target and expiry that went into the digest rather than
approving an opaque 32-byte value (the AUDIT-P0-011 lesson).

Tests: `bins/node/src/commands/maintainer.rs` unit module — bound-above-gate, legacy-below-gate
(the devnet case where #21=0 < #22=20 makes a governance tx mineable below the binding gate),
and cross-chain divergence. RED evidence: `docs/.workflow/inc-i-176-M4-test-red-evidence.txt`.

### 2. `submitMaintainerChange` did not broadcast — INC-I-195 (FIXED, verified)

`crates/rpc/src/methods/governance.rs:276-286` ends at `mempool.add_system_transaction` and
returns `{"status":"accepted"}`. It makes **zero** `broadcast_tx` calls, unlike the ordinary tx
path (`crates/rpc/src/methods/transaction.rs:225`). The transaction never leaves the node that
received the RPC, so a non-producer endpoint can never mine it.

Measured: the first removal, submitted to the seed (`--relay-server`, no `--producer`), returned
`accepted` and the set was unchanged 180 s (~36 blocks) later. The same rotation resubmitted to
producer n1 applied in 4–5 blocks per transaction, 10 of 10.

**Mainnet consequence:** a seed RPC is the natural endpoint an operator reaches for, and mainnet
seeds are relays. The rotation would report success and silently never apply.

Fix: the mempool write lock is scoped and `(self.broadcast_tx)(tx)` follows a successful admission,
mirroring `submit_transaction`. Nothing new had to be taught to the network — a maintainer change is
0-in/0-out, and `handle_new_transaction`
(`bins/node/src/node/validation_checks.rs:1253-1259`) already routes `is_zero_flow()` transactions to
the same `add_system_transaction` lane. Transport only: no consensus rule, no block content, no
activation height. Tests: `crates/rpc/src/methods/tests_inc_i195_broadcast.rs` (add relayed, remove
relayed, rejected submissions relay nothing); RED evidence
`docs/.workflow/inc-i-195-test-red-evidence.txt`.

**Verified on the testnet after a rolling deploy of all 18 nodes.** The same removal and addition,
submitted TO THE SEED that had silently swallowed them, applied at h=57,024 and h=57,025 — **one
block each**, against ~36 blocks of nothing before the fix. Relaying is also faster than the
producer workaround (1 block vs 4–5) because the transaction no longer waits for one specific
producer's slot. All 18 nodes then agreed: `digest=1c3dabadee2825ed n=5 last_change=57025` and
`(57026, 221ec865226a)`.

## Sequence as executed

| Round | Tx | Applied at | Set size | Signers |
|---|---|---|---|---|
| 1 | remove producer_1 | 56,838 | 4 | 3,4,5 |
| 1 | add producer_508 | 56,843 | 5 | 3,4,5 |
| 2 | remove producer_2 | 56,848 | 4 | 3,4,5 |
| 2 | add producer_509 | 56,853 | 5 | 3,4,5 |
| 3 | remove producer_3 | 56,858 | 4 | 4,5,**508** |
| 3 | add producer_510 | 56,863 | 5 | 4,5,**508** |
| 4 | remove producer_4 | 56,868 | 4 | 5,508,509 |
| 4 | add producer_511 | 56,873 | 5 | 5,508,509 |
| 5 | remove producer_5 | 56,878 | 4 | 508,509,510 |
| 5 | add producer_512 | 56,883 | 5 | 508,509,510 |

Control passed at round 3, exactly as the plan's F6 table predicts: from the third removal the
quorum can only be formed from fresh keys.

## Negative test — the retired keys are inert

A `remove` of a current maintainer (producer_508) signed by three rotated-out keys
(producer_1,2,3) was accepted by the RPC and **never applied**. 25 blocks later
`last_change_block` was still 56,883, the digest was unchanged, and the target was still a
maintainer. RPC acceptance is not authorization.

## Convergence

All 18 nodes (seed + n1–n17): `digest=1c3dabadee2825ed n=5 last_change=56883 source=on-chain`,
and one `(height, hash)` — `56886 44deac3be758`. No fork.

## Before the mainnet run

1. Deploy the M4 signer fix to whichever host produces the signatures. It is CLI-only — it
   changes no consensus code and needs no fleet restart or activation height.
2. Deploy the INC-I-195 relay fix to the node that will RECEIVE the submission. Peers do not need
   it to admit a relayed transaction, but the receiving node needs it to send one.
3. The five fresh mainnet keys are generated offline by the operator and never enter a repo
   (plan Phase 3). This rehearsal used spare testnet keys, which is the only difference.
4. One transaction at a time, confirm `getMaintainerSet` between each, never in parallel.
