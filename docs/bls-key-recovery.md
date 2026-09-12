# BLS Attestation Key Recovery for Producers

This page is for a producer operator whose node holds a BLS attestation key that the chain does
not hold. It lists how to detect that state, and every remedy, with the real commands.

Start here if you saw this line in the node log:

```
[ATTEST_EGRESS] own BLS half does not verify against the on-chain key — check the BLS key config
```

Registration, bonds, selection and the reward model are in
[docs/becoming_a_producer.md](becoming_a_producer.md). Section 11.4 of that page is the short
version of this one.

---

## 1. What goes wrong

Each producer commits one BLS public key on-chain at registration. The node signs the BLS half of
every attestation with the matching secret. The node also compares the two keys before it pools
its own half. A key that does not match is dropped, so your own attestation bit is never set.

A wallet restored from the 24-word phrase does **not** restore the registered BLS key. The phrase
derives a fresh BLS key. The node starts, syncs, and produces blocks with the fresh key. It earns
no attestation-derived reward, because the chain counts no attestation minutes for you.

The bond is safe. The registration is safe. Only the attestation half is broken.

---

## 2. Detect the mismatch

Run all three checks. Two agreeing checks are enough to act.

### 2.1. The log signal

```bash
grep "ATTEST_EGRESS" /path/to/node.log | tail -5
```

`own BLS half does not verify against the on-chain key` confirms the mismatch. The node writes
this line while it attests, so it repeats. The check is active at and above
`inc_i_208_own_attestation_activation_height`. Below that height the node writes nothing, and you
must use check 2.2.

### 2.2. Compare the wallet key with the chain key

This is the definitive check. It is the same comparison the GS-012 gauntlet scenario makes.

```bash
# 1. The key the wallet holds
doli -w /path/to/wallet.json info
#    field: BLS Public Key

# 2. The key the chain holds for the same producer
curl -s -X POST http://127.0.0.1:8500 -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getProducer","params":["<your-public-key>"]}'
#    field: result.blsPubkey
```

Both values are 96 hex characters (48 bytes). Equal values mean the keys match, and the cause of
your lost rewards is elsewhere. Different values mean you must repair the key.

You can read the same field from the wallet file directly:

```bash
python3 -c "import json;print(json.load(open('/path/to/wallet.json'))['addresses'][0]['bls_public_key'])"
```

### 2.3. Confirm the earnings effect

```bash
curl -s -X POST http://127.0.0.1:8500 -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAttestationStats","params":[]}'
```

Find your public key in `result.producers[]`. A mismatched producer shows `attestedMinutes: 0`
and `qualified: false`. Do not read `hasBls`: that field reports key registration, not signing.

---

## 3. Choose a remedy

| Your situation | Remedy | Section |
|----------------|--------|---------|
| You still hold the old BLS secret | Import it. No transaction. | 4 |
| The old BLS secret is lost or leaked | Rotate to a new key. One transaction. | 5 |
| You cannot operate the node any more | Delegate the bond. | 8 |
| Every option above is closed | Exit and register again. | 9 |

Try the remedies in that order. The list runs from cheapest to most expensive.

---

## 4. Remedy A — import the key you still hold

The old secret is a 64-hex-character string. Look for it in:

- an older `wallet.json`, field `addresses[0].bls_private_key`;
- a wallet file written by `doli export`;
- your own offline backup of that file.

```bash
# Back up the current wallet file FIRST.
cp /path/to/wallet.json /path/to/wallet.json.bak

# Install the old secret and check it against the chain in one step.
doli import-bls <64-hex-characters> --rpc http://127.0.0.1:8500

# Restart the node to load the imported key.
```

Rules the command enforces:

- It validates the secret before it writes. A rejected secret leaves `wallet.json` byte-identical.
- It refuses to replace an existing BLS key. Pass `--force` to accept the replacement.
- With `--rpc` it compares the derived public key against the chain before it saves.
- `--address <addr>` selects a non-primary wallet address. The default is the primary address.

Nothing here reaches the chain. No fee is paid. No epoch boundary is involved.

After the restart, repeat check 2.2. The two keys must now be equal. Your attestation minutes
resume in the epoch that follows.

**Back up the wallet file.** The 24-word phrase does not restore an imported key. That file is
the only copy.

---

## 5. Remedy B — rotate to a new key

`doli producer rotate-bls` publishes the BLS key **that the wallet currently holds**. It replaces
the key the chain holds for your producer.

### 5.1. Preconditions

1. The network must have activated rotation. `bls_key_rotation_activation_height` is 450_789
   on mainnet and 176_200 on testnet (both pinned 2026-09-12); devnet stays `u64::MAX`. Below that height the node refuses the transaction with
   `[ERRTX-ROT002]`. Ask the maintainers before you plan a rotation.
2. The wallet must hold the key you want to publish. If the wallet still holds the leaked key,
   replace it first with `doli import-bls <new-secret> --force`, or start from a wallet whose key
   is new.
3. The wallet must hold one spendable normal output that covers the fee.
4. No rotation may be queued already for this producer.

### 5.2. Run it

```bash
doli producer rotate-bls
# The command prints the old key, the new key and an irreversibility notice,
# then asks for confirmation. --yes accepts without the prompt.
```

### 5.3. What it costs

- **The fee.** At the shipped fee constants the fee is 3 base units for the 240-byte payload.
  The command spends the smallest normal, spendable output that is strictly larger than the fee,
  and returns the rest to you as one change output.
- **The boundary attestation.** The swap lands at the next epoch boundary. The attestation of the
  boundary block itself can be lost while the key changes.

### 5.4. What is irreversible

- The queued rotation cannot be cancelled.
- The only way back to the previous key is another rotation, with another fee.
- An attestation signed with a key the chain does not hold is never counted. Keep the new BLS
  secret in the wallet the node runs with, and back the file up.

### 5.5. Confirm success

**Step 1 — the queue accepted it.** The rotation appears as a pending update:

```bash
curl -s -X POST http://127.0.0.1:8500 -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getProducer","params":["<your-public-key>"]}'
```

`result.pendingUpdates[]` must carry an entry with `updateType: "rotate_bls_key"`, your new key in
`newBlsPubkey`, and the boundary height in `effectiveAtHeight`. `doli producer status` lists the
same entry as `? rotate_bls_key`, because its printer has no label for this kind yet.

**Step 2 — the boundary installed it.** After `effectiveAtHeight`, repeat check 2.2.
`result.blsPubkey` must now be the new key, and `pendingUpdates` must no longer carry the entry.

**Step 3 — the node counted it.** The node metric counts every rotation an epoch boundary
installs:

```bash
curl -s http://127.0.0.1:9000/metrics | grep doli_producer_bls_rotation_total
```

The counter increases by one for each installed rotation.

**Step 4 — the reward returns.** In the next full epoch, check 2.3 must show rising
`attestedMinutes` and `qualified: true`.

---

## 6. Refusals before anything is signed

The CLI checks the chain first, so a rotation that the node would skip never costs a fee.

| Message | Meaning | Do this |
|---------|---------|---------|
| `this wallet holds no BLS key, so there is nothing to rotate to` | The wallet has no BLS key. | Add one with `doli add-bls`, or import one. |
| `this key is not a registered producer` | The address is not an active or pending producer. | Register first. |
| `the wallet BLS key is already the key on chain` | Nothing would change. | No action. The keys already match. |
| `a BLS key rotation is already queued for this producer` | One rotation is queued. | Wait for the boundary. |
| `no single spendable output strictly covers the ... fee` | Every output is too small. | Send a small amount to the wallet, wait for maturity, retry. |
| `refusing to rotate the BLS key without confirmation` | No terminal and no `--yes`. | Re-run in a terminal, or pass `--yes`. |

---

## 7. Skips at apply time

The node applies a rotation at the epoch boundary. A skip does not invalidate the block, and the
fee is still spent. The node evaluates the conditions in this fixed order.

| Verdict | Plain meaning |
|---------|---------------|
| `NotProducer` | The sender is neither an active producer nor a queued registration. Nothing to rotate. |
| `SameKey` | The target key is the key the sender already holds. The rotation changes nothing. |
| `AlreadyPending` | This sender already has a rotation queued for this boundary. The second one is dropped. |
| `KeyInUse` | Another producer holds the target key, or another queued rotation reserved it. Two producers may not share one BLS key. |

If a rotation did not install, read this table first. The pre-flight checks of section 6 make the
first three verdicts hard to reach from the CLI.

---

## 8. Remedy C — delegate the bond

You keep the bond and stop operating the node. Another producer takes the production duty and the
attestation duty. See section 6 of [docs/becoming_a_producer.md](becoming_a_producer.md) for bond
management and delegation, and [docs/cli.md](cli.md) for the delegation commands.

This remedy does not repair the key. It moves the duty to a key that works.

---

## 9. Remedy D — exit and register again

This is the last option. It repairs the key by ending the producer and creating a new one.

Costs you cannot avoid:

- The registration date resets. Seniority weight starts again from zero.
- The vesting penalty applies to the bonds the exit returns. Read section 6.6 of
  [docs/becoming_a_producer.md](becoming_a_producer.md).
- You must fund and bond again at the current bond price.

Use it only when you hold no old key, rotation is not active on your network, and delegation does
not suit you.

---

## 10. Prevent a repeat

- Back up `wallet.json` itself, not only the 24-word phrase. The phrase does not restore an
  imported BLS key.
- Keep the backup off the producer host.
- After any wallet restore, run check 2.2 **before** you start the node.
- Never copy one wallet to two producer hosts. Two nodes signing with one producer key cause an
  equivocation slash.

---

## Related pages

| Page | Why |
|------|-----|
| [docs/becoming_a_producer.md](becoming_a_producer.md) | Registration, bonds, seniority, rewards, section 11.4 short recovery path |
| [docs/cli.md](cli.md) | Full command reference |
| [docs/rpc_reference.md](rpc_reference.md) | `getProducer`, `getProducers`, `getAttestationStats` |
| [docs/error-codes.md](error-codes.md) | `ERRTX-ROT*` refusal codes |
| [specs/protocol.md](../specs/protocol.md) | `RotateBlsKey` transaction format |
