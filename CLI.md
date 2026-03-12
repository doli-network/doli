# CLI Reference

## Programmable Outputs

DOLI supports NFTs, fungible tokens, and cross-chain bridge HTLCs as native UTXO output types. No VM, no gas — just declarative spending conditions verified at consensus.

### NFT Commands

**`mint`** — Mint a new NFT (UniqueAsset output).

```
doli --wallet <wallet.json> mint <content> [--condition <cond>] [--amount <doli>]
```

- `content`: Content hash or URI (IPFS CID, HTTP URL, or hex hash).
- `--condition`: Spending condition (default: `signature(<minter_pubkey_hash>)`). Supports `signature(hash)`, `hashlock(hash)`, `timelock(slot)`, `multisig(k,hash1,hash2,...)`, boolean combinators.
- `--amount`: DOLI value to attach (default: 0, pure NFT).
- Token ID: `BLAKE3("DOLI_NFT" || creator_pubkey_hash || nonce)` — globally unique.

**`nft-transfer`** — Transfer an NFT to a new owner.

```
doli --wallet <wallet.json> nft-transfer <txhash:index> <recipient_address> [--witness <witness>]
```

- `utxo`: The UTXO containing the NFT, format `txhash:output_index`.
- `to`: Recipient address (e.g. `doli1...`).
- `--witness`: Witness to satisfy the spending condition (default: `none()`, auto-signs with wallet key if condition is `signature(owner)`).

**`nft-info`** — Display NFT metadata from a UTXO.

```
doli --wallet <wallet.json> nft-info <txhash:index>
```

Shows token_id, content hash/URI, creator, condition, and attached DOLI value.

### Fungible Token Commands

**`issue-token`** — Issue a new fungible token (meme coin, stablecoin, etc.).

```
doli --wallet <wallet.json> issue-token <ticker> --supply <amount> [--condition <cond>]
```

- `ticker`: Token symbol, max 16 characters (e.g. `DOGEOLI`).
- `--supply`: Total supply, fixed at issuance. Denominated in token units.
- `--condition`: Spending condition (default: `signature(<issuer_pubkey_hash>)`).
- Asset ID: `BLAKE3("DOLI_ASSET" || genesis_tx_hash || output_index)` — unique by construction.
- Anchored with 1 satoshi dust (protocol requires non-zero DOLI on all outputs).

**`token-info`** — Display fungible token metadata from a UTXO.

```
doli --wallet <wallet.json> token-info <txhash:index>
```

Shows asset_id, ticker, supply, issuer, and condition.

### Cross-Chain Bridge Commands

**`bridge-lock`** — Lock DOLI in a bridge HTLC for cross-chain atomic swap.

```
doli --wallet <wallet.json> bridge-lock <amount> --hash <hash> --lock <height> --expiry <height> --chain <chain> --to <address>
```

- `amount`: DOLI to lock.
- `--hash`: BLAKE3 hashlock hash (64 hex chars). Computed as `BLAKE3(LE32(13) || "DOLI_HASHLOCK" || preimage)`.
- `--lock`: Block height after which claim is available.
- `--expiry`: Block height after which refund is available.
- `--chain`: Target chain — `bitcoin`, `ethereum`, `monero`, `litecoin`, `cardano`.
- `--to`: Recipient address on the target chain.

**`bridge-claim`** — Claim a bridge HTLC by revealing the preimage (receiver side).

```
doli --wallet <wallet.json> bridge-claim <txhash:index> --preimage <preimage>
```

- `utxo`: The UTXO containing the bridge HTLC.
- `--preimage`: 64 hex chars that hash to the locked hash.

**`bridge-refund`** — Refund a bridge HTLC after expiry (sender side).

```
doli --wallet <wallet.json> bridge-refund <txhash:index>
```

Only valid after the expiry height has passed.

### Gotchas

- `--wallet` goes BEFORE the subcommand: `doli --wallet f.json mint ...`
- UTXO selection automatically filters to `output_type == "normal" && spendable` — bond and conditioned outputs are never consumed as fee inputs.
- NFT transfers with default `none()` witness auto-sign with the wallet key when the condition is `signature(owner)`.
- Token issuance uses 1 satoshi dust anchor — the token UTXO will show `amount: 1` in raw RPC.

---

# CLI Issues

## 2026-02-04 - `devnet clean` doesn't kill manually started nodes

- **Type**: Constraint
- **Command**: `doli-node devnet clean`
- **Observed**: Only kills nodes tracked in `devnet.toml` (nodes 0 to `node_count-1`). Manually started nodes (e.g., additional producers started with `doli-node run --producer`) remain running and hold ports.
- **Expected**: Should kill ALL doli-node processes running on devnet, or at minimum scan the entire `pids/` directory for all PID files (not just 0..node_count).
- **Priority**: Medium
- **Status**: Resolved
- **Root Cause**: `bins/node/src/devnet.rs` - loop iterated `0..config.node_count`, missing any nodes added after init.
- **Fix**: Added `scan_and_kill_all_pids()` helper that scans `pids/` directory for all `node*.pid` files. Updated both `stop` and `clean` functions to use this helper.

## 2026-02-28 - Header-first sync fails for genesis blocks on mature chains

- **Type**: Bug
- **Command**: N/A (internal sync path)
- **Observed**: When a new node joins a chain with >192 slots (~32 min old), header-first sync rejects genesis blocks because their slot numbers (e.g., slot 10) are more than `MAX_PAST_SLOTS` (192) behind the current slot. Sync stalls until snap sync takes over as fallback.
- **Expected**: Header-first sync should apply historical blocks without the `MAX_PAST_SLOTS` check. `ValidationMode::Light` already exists and skips this check (line 714-776 in `node.rs`), but `apply_block()` hardcodes `ValidationMode::Full` (line 2833).
- **Priority**: Low (snap sync covers this as fallback; only matters if snap sync fails due to <3 peers)
- **Status**: Open
- **Root Cause**: `bins/node/src/node.rs` line 2833 — `validate_block_for_apply()` called with `ValidationMode::Full` for all blocks, including synced historical blocks. The fix is to pass `ValidationMode` as a parameter to `apply_block()` and use `Light` for sync blocks in `run_periodic_tasks()` (line 5206).
- **Workaround**: Snap sync activates automatically when gap >1000 blocks with 3+ peers, bypassing header-first sync entirely.

## 2026-03-10 - `producer add-bond` accepts non-registered producers, burns funds

- **Type**: Bug
- **Command**: `doli producer add-bond -c 1`
- **Observed**: CLI sends AddBond TX without checking if the pubkey is a registered producer. Node includes the TX in a block, consumes inputs, creates a Bond UTXO — but at epoch boundary `apply_pending_updates()` silently discards the update because the producer doesn't exist. Result: 10 DOLI per bond locked in an orphan Bond UTXO with `lock_until=12,615,975` (~4 years), irrecoverable.
- **Expected**: CLI should reject with "not registered as a producer". Node should skip queuing AddBond for non-existent producers.
- **Priority**: High
- **Status**: Resolved
- **Root Cause**: Three layers missing producer existence check: (1) CLI had no validation, (2) mempool/validation only does structural checks, (3) `apply_block()` queued the update without verifying the producer exists — only `apply_pending_updates()` checked, but by then the inputs were already consumed.
- **Fix**: Added producer existence guard in CLI (`get_producers` RPC check before submit), in `apply_block()` AddBond processing (skip + warn if producer not registered or pending), and in `rebuild_producer_set_from_blocks()` (same guard for replay path).
- **Impact**: 60 DOLI burned across N7-N12 mainnet (10 DOLI each in orphan Bond UTXOs). Funds are irrecoverable by design.
