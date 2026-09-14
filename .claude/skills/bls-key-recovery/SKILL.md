---
name: bls-key-recovery
description: Operator procedure for a producer whose node BLS attestation key does not match the key registered on-chain, and for moving a pre-v3 (random-key) wallet to the phrase-derived key before anything is lost. Covers detection, the four remedies (import-bls, producer rotate-bls, delegate, exit), the proactive migration, the timing rule at the epoch boundary, verification probes and refusals. Triggers on "[ATTEST_EGRESS]", "own BLS half does not verify", "unverifiable BLS half", "restored wallet wrong BLS key", "0 attested minutes", "rotate-bls", "import-bls", "BLS key rotation", "phrase does not restore the producer key", "proactive migration", "ERRTX-ROT002", "INC-I-217", "INC-I-162".
user_invocable: true
---

# BLS Key Recovery and Rotation — operator procedure

Full reference page: `docs/bls-key-recovery.md` (sections 1–11). This skill is the runbook: what to
check, which command to run, and what to look at afterwards. Code is the source of truth
(`bins/cli/src/cmd_producer/rotate.rs`, `bins/cli/src/cmd_wallet_bls.rs`,
`crates/core/src/validation/rotate_bls.rs`, `crates/storage/src/producer/rotation.rs`).

## 0. Facts you need before anything else

| Fact | Value |
|------|-------|
| Wallet version 3 (created or restored with a binary from 2026-08-08 on) | BOTH keys derive from the 24-word phrase; the phrase is a complete backup |
| Wallet version 1 or 2 | the BLS key is RANDOM; the phrase restores the address and funds, NOT the producer identity; the wallet file is the only copy |
| What the chain verifies | every attestation against `ProducerInfo.bls_pubkey` = the key given at registration (or the last rotation) |
| `bls_key_rotation_activation_height` | mainnet 450_789, testnet 176_200, devnet `u64::MAX` (devnet/testnet env override `DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT`; mainnet locked) |
| A rotation takes effect | at the NEXT EPOCH BOUNDARY after it is mined; the CLI prints `Takes effect: height N` |
| Cost of a rotation | one ordinary transaction fee (1 input from the producer's own address, 1 change output); irreversible except by another rotation |
| Where a queued rotation shows | `getProducer` / `getProducers` → `pendingUpdates: [{ "updateType": "rotate_bls_key", "newBlsPubkey": "<hex>", "effectiveAtHeight": N }]` |

Never print, paste or log a BLS secret. `doli export` is the only deliberate path that writes one.

## 1. Detect: is this a key mismatch?

Symptoms (all three appear together):
- Producer node log, every block: `[ATTEST_EGRESS] own BLS half does not verify against the on-chain key — check the BLS key config`.
- Seed / peer logs: `[ATTEST_INGEST] unverifiable BLS half from <attester pubkey prefix>`.
- `getAttestationStats` (RPC) shows the producer with `attestedMinutes: 0`; it never qualifies and earns nothing.

Confirm with two reads:

```
doli -n <network> -w <wallet.json> info          # "Backup:" section; note the BLS Public Key
curl -s <rpc> -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getProducer","params":["<producer ed25519 pubkey hex>"]}'
```

`info` BLS Public Key ≠ `getProducer → blsPubkey` ⇒ mismatch. `info` also tells you the wallet
version; version 1/2 prints "the BLS producer key is RANDOM" and (from 6.30.x with the hint)
the three migration steps of §4.

Not a key mismatch if: `info` key == chain key (then look at peers, clock, or the INC-I-208 own
attestation gate instead), or `getProducer` says `exited` / no record.

## 2. Choose the remedy

| You still hold… | Do |
|---|---|
| the old wallet file or the old BLS secret hex | §3 Remedy A `import-bls` — no on-chain change, no fee |
| nothing but the 24 words (or the key is gone) | §3 Remedy B `producer rotate-bls` — on-chain, after the activation height |
| a healthy pre-v3 producer (key still matches) and want the phrase to become a full backup | §4 proactive migration (restore + rotate + switch) |
| neither the key nor the will to rotate | delegate the bond to another producer, or exit and register again (vesting penalty) — `docs/bls-key-recovery.md` §8–9 |

## 3. Remedies

### Remedy A — import the key you still hold (client-side only)

```
doli -n <network> -w <wallet.json> import-bls <SECRET_HEX_64> [--force] [--rpc <url>] [--address <addr>]
```
- `--force` replaces an existing key (DANGEROUS: no phrase restores what it replaces).
- `--rpc` compares the key with the chain before writing; `--address` picks the registration to compare (default: primary).
- After import the wallet's version marker drops below 3: the phrase no longer restores this key. Back up the FILE.
- Restart the node once so it loads the key. Verify with §5.

### Remedy B — rotate the on-chain key to the key in this wallet

Preconditions: the network is at or past `bls_key_rotation_activation_height`; the wallet holds the
BLS key you want on-chain (a version-3 wallet, or one you just imported into); the producer's
address has at least one spendable UTXO for the fee; no rotation is already queued.

```
doli -n <network> -w <wallet.json> -r <rpc> producer rotate-bls --yes
```
The command prints the current and new key, the fee, the change output, the consent text and
`Takes effect: height N (next epoch boundary)`. Without `--yes` it prompts.

Refusals you can meet:
- `Error: the wallet BLS key is already the key on chain` — nothing to rotate (§1 was wrong).
- a rotation is already queued for this producer — wait for its boundary.
- `RPC error -32002 (INVALID_TRANSACTION): [ROTATE_BLS_NOT_ACTIVATED] [ERRTX-ROT002] bls key rotation not activated: current_height=… activation_height=…` — below the activation height; nothing was queued; retry after it.
- other `[ERRTX-ROT0xx]` codes: `docs/error-codes.md`.

Until height N the chain still expects the OLD key. If the node already runs the new key, its
attestations fail until N; that is expected. Verify with §5 after N.

## 4. Proactive migration — pre-v3 wallet, key still works

Goal: make the 24 words a complete backup before the file is ever lost. Rehearsed end to end on
the testnet on 2026-09-14; the steps below are exactly what ran.

1. Keep the node running on the current wallet file (old random key A). Do not stop it.
2. Restore the phrase into a NEW file (the command refuses to overwrite an existing path; it reads the phrase from stdin, nothing on the command line):
   ```
   doli -n <network> -w <new-wallet.json> restore -n <name>
   doli -n <network> -w <new-wallet.json> info      # must say: version 3, "The phrase is a COMPLETE backup"
   ```
   Same address, same Ed25519 key, a NEW phrase-derived BLS key B.
3. At or past the activation height, rotate with the NEW file (the fee is paid from the same address):
   ```
   doli -n <network> -w <new-wallet.json> -r <rpc> producer rotate-bls --yes     # note "Takes effect: height N"
   ```
4. At height N, or within a block after it, point the node at the new file (the wallet / `--producer-key` path it starts with) and restart it ONCE. Do not switch earlier: before N the chain expects key A. Switching within one block of N loses at most two attestations (the boundary block's own, and the last one before the restart).
5. Verify with §5. Keep the old file for the record; it is no longer needed for attestation.

## 5. Verify (after the boundary)

```
getProducer  → blsPubkey == the wallet's BLS Public Key, pendingUpdates empty
getAttestationStats (seed) → attestedMinutes rising for this producer within the epoch after N
node log     → no new "[ATTEST_EGRESS] own BLS half does not verify"; "[BLS_ROTATE] applied" once at the boundary
seed log     → no new "[ATTEST_INGEST] unverifiable BLS half from <this producer>"
metrics      → doli_producer_bls_rotation_total incremented on nodes that were running at the boundary
```
Metric and `[BLS_ROTATE] applied` are on every node, not only the producer's.

## 6. Gotchas seen in rehearsal

- `doli restore` has no phrase flag; pipe the 24 words on stdin, numbering stripped.
- `producer exit` has no `--yes`; it takes `--force` for an early exit (75 % penalty band) and asks for confirmation on stdin.
- No RPC method exposes the activation height; the CLI surfaces the node's `[ERRTX-ROT002]` refusal verbatim.
- After a full fleet restart, start producers a few seconds after the seed; a node that dials the seed before its listener is up may sit at one peer (INC-I-221, fixed by the bootstrap re-dial in 6.30.x builds after the fix).
- The `[BLS_ROTATE] applied` line prints internal hash identifiers and the queue height, not the key prefixes and the boundary height (INC-I-220).
- Log level: nodes started without `--log-level info` do not print the "BLS key loaded" line; judge by behaviour (§5).

## 7. For agents reading code

- Wire: `TxType::RotateBlsKey = 32` (bincode ordinal 24), 240-byte payload `producer(32) ‖ new_bls_pubkey(48) ‖ bls_pop(96) ‖ signature(64)`; preimage domain `DOLI-ROTATE-BLS-V1`, PoP domain `DOLI-ROTATE-POP-V1` (`crates/core/src/transaction/rotate_bls.rs`, `crates/crypto/src/bls_rotation.rs`).
- One stateless predicate for mempool, builder and block validation: `crates/core/src/validation/rotate_bls.rs`; relay wrapper `crates/mempool/src/rotation_filter.rs`.
- Apply: epoch-deferred `PendingProducerUpdate::RotateBlsKey`, verdict order NotProducer / SameKey / AlreadyPending / KeyInUse, unconditional flush at the boundary in both apply paths (`crates/storage/src/producer/rotation.rs`, `bins/node/src/node/rewards.rs` rebuild arm).
- CLI builder: `bins/cli/src/rotate_tx.rs` (pure, golden-pinned against the node validator); command: `bins/cli/src/cmd_producer/rotate.rs`; import: `bins/cli/src/cmd_wallet_bls.rs`; info hint: `bins/cli/src/bls_migration_hint.rs`.
- Related: `specs/protocol.md` "RotateBlsKey (type 32)", `specs/bls-key-rotation-architecture.md`, `docs/error-codes.md` (`ERRTX-ROT*`), incidents INC-I-162 (wallet derivation), INC-I-217 (rotation), INC-I-220 (log format).
