# BLS Attestation Key Recovery for Producers

> Stub created by the Architect (run 552, 2026-09-11). Full text lands in milestone M11.
> Design: `specs/bls-key-rotation-architecture.md`.

If your node's BLS key does not match the key the chain holds for your producer, your attestations
are rejected and you earn no rewards. There are two remedies:

1. **You still have the old key** (an old `wallet.json`, a backup, a `doli export` dump):
   `doli wallet import-bls <secret-hex>` puts it back into the wallet. No transaction is needed.
2. **The old key is lost or leaked**: `doli producer rotate-bls` publishes the wallet's current BLS
   key on-chain. It spends one small UTXO (the fee), takes effect at the next epoch boundary, and
   is only available once the network has pinned `bls_key_rotation_activation_height`.
