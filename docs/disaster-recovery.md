# Disaster Recovery

How to back up and restore a DOLI node from the block archive.

## Overview

DOLI maintains a block archive — a flat-file copy of every finalized block, each with a BLAKE3 integrity checksum. If a node's database is corrupted or lost, you can restore the full chain from this archive in minutes.

The archive is hosted at `archive.doli.network` (omegacortex.ai). It can be copied to any location via rsync.

**For nodes that joined via snap sync**, historical blocks can be filled from the archive using `restore --backfill`. See [Backfill only](#backfill-only-fill-snap-sync-gaps) below.

**No SSH access to the archive server?** Use `restore --from-rpc` to download blocks directly from any archiver's RPC endpoint. See [Restore from RPC](#restore-from-rpc) below.

## Archive Format

```
archive/
  0000000001.block      # Bincode-serialized block
  0000000001.blake3     # BLAKE3 checksum of the .block file
  0000000002.block
  0000000002.blake3
  ...
  manifest.json         # Tracks latest height, hash, and genesis_hash
```

**manifest.json example:**
```json
{
  "latest_height": 1057,
  "latest_hash": "03d91f6ccb18...",
  "genesis_hash": "d34b16931c4d..."
}
```

Only finalized blocks (67%+ attestation weight) are archived — blocks that the protocol has declared irreversible.

## Getting the Archive

### Option A: Copy from the archive server

```bash
# From omegacortex (same machine)
cp -r ~/.doli/mainnet/archive/ /path/to/local/archive/

# From another machine via rsync
rsync -av ilozada@archive.doli.network:~/.doli/mainnet/archive/ /path/to/local/archive/
```

### Option B: Run your own archiver

Any node can archive blocks by adding `--archive-to`:

```bash
doli-node --network mainnet run --archive-to /path/to/archive
```

The node syncs normally and streams finalized blocks to the archive directory. On startup it catches up any blocks it missed while offline.

## Restoring from Archive

### Quick restore

```bash
doli-node --network mainnet restore --from /path/to/archive --yes
```

This command:
1. Reads `manifest.json` to determine block count
2. Validates `genesis_hash` matches the target network
3. Imports each block, verifying its BLAKE3 checksum
4. Automatically rebuilds UTXO set, producer registry, and chain state

### Step by step

If you prefer manual control:

```bash
# 1. Import blocks (verifies checksums + genesis_hash)
doli-node --network mainnet restore --from /path/to/archive --yes

# 2. Start the node — it will sync remaining blocks from peers
doli-node --network mainnet run
```

### Restore to a custom data directory

```bash
doli-node --network mainnet --data-dir /new/data/dir restore --from /path/to/archive --yes
```

### Backfill only (fill snap sync gaps)

Nodes that joined via snap sync have correct state but are missing historical blocks (e.g., heights 1 to ~475). Use `--backfill` to import only the missing blocks without rebuilding state:

```bash
doli-node --network mainnet restore --from /path/to/archive --backfill --yes
```

This command:
1. Scans the archive directory for block files
2. Skips blocks that already exist in the BlockStore
3. Imports only missing blocks, verifying BLAKE3 checksums
4. Does **not** rebuild UTXO set, producer registry, or chain state (they're already correct from snap sync)

Typical output: `Backfill complete: imported 469 blocks (skipped 588 existing)`

## Restore from RPC

If you don't have SSH or filesystem access to the archive, you can restore directly from any archiver node's RPC endpoint using `--from-rpc`:

### Backfill via RPC (most common)

```bash
doli-node --network mainnet restore --from-rpc http://archive.doli.network:8548 --backfill --yes
```

### Full restore via RPC

```bash
doli-node --network mainnet restore --from-rpc http://archive.doli.network:8548 --yes
```

After a full restore, rebuild state: `doli-node recover --yes`

### How it works

1. Queries `getChainInfo` to determine the archiver's best height
2. Fetches block 1 via `getBlockRaw` to validate genesis hash (prevents cross-chain imports)
3. Downloads each block as base64-encoded bincode via `getBlockRaw`
4. Verifies BLAKE3 checksum on every block before storing
5. In backfill mode, skips blocks that already exist in the local BlockStore

### RPC method: `getBlockRaw`

Returns the raw bincode-serialized block with its BLAKE3 checksum.

**Request:**
```json
{"jsonrpc": "2.0", "method": "getBlockRaw", "params": {"height": 42}, "id": 1}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "block": "<base64-encoded bincode>",
    "blake3": "a1b2c3d4...",
    "height": 42
  },
  "id": 1
}
```

Unlike `getBlockByHeight` (which returns JSON — lossy for signatures and VDF proofs), `getBlockRaw` returns the exact bincode bytes that can be deserialized back into a `Block` struct.

### Comparison

| Method | Requires SSH? | Requires file transfer? | RPC only? |
|--------|:---:|:---:|:---:|
| `--from /path` (file-based) | Yes | Yes (rsync/scp) | No |
| `--from-rpc <URL>` | **No** | **No** | **Yes** |

## Verifying Archive Integrity

You can manually verify any block's checksum:

```bash
# Compute BLAKE3 hash of a block file
b3sum 0000000042.block

# Compare with the stored checksum
cat 0000000042.blake3
```

The `manifest.json` `genesis_hash` field lets you confirm the archive belongs to the correct network before importing.

## Off-Site Backup

The archive is plain files — use any backup tool:

```bash
# rsync to a remote server
rsync -av /path/to/archive/ backup-server:/backups/doli-archive/

# Upload to S3-compatible storage
rclone sync /path/to/archive/ s3:doli-backups/archive/

# Create a tarball
tar czf doli-archive-$(date +%Y%m%d).tar.gz /path/to/archive/
```

Schedule periodic rsync for continuous off-site backup.

## How the Archiver Works

The block archiver runs as part of a `doli-node` process with the `--archive-to` flag:

1. **Catch-up on startup**: reads missing blocks from the local BlockStore up to chain tip
2. **Real-time streaming**: as new blocks are applied, they're buffered in memory
3. **Finality gate**: buffered blocks are only written to disk when the FinalityTracker confirms them as irreversible (67%+ attestation weight from active producers)
4. **Atomic writes**: each file is written to a `.tmp` path first, then renamed — safe against crashes
5. **BLAKE3 checksums**: a `.blake3` sidecar is written alongside each block for integrity verification

The archiver never slows down block production — it uses non-blocking channel sends (`try_send`) and runs on a separate async task.

## Architecture

```
Producer nodes (N1-N6)
       │
       │ gossip (blocks + attestations)
       ▼
  Archive node (sync-only, --archive-to)
       │
       │ finality gate (67%+ attestations)
       ▼
  ~/.doli/mainnet/archive/
       │
       ├── rsync / rclone → Off-site backup (S3, remote server, etc.)
       ├── restore --backfill → Operator fills snap sync gaps from local archive copy
       └── restore --from-rpc → Operator fills gaps via RPC (no SSH needed)
```

## Hot Backfill (Live RPC — No Restart Required)

For nodes that are **already running** and synced to tip but missing historical blocks (snap sync gap), the `backfillFromPeer` RPC endpoint fills the gap **without stopping the node**. The node continues producing and syncing normally while backfill runs in the background.

### Usage

**Start backfill:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillFromPeer","params":{"rpc_url":"http://archive.doli.network:8548"},"id":1}'
```

**Response:**
```json
{
  "result": {
    "started": true,
    "gaps": "1-6291",
    "total": 6291
  }
}
```

**Monitor progress:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillStatus","params":{},"id":1}'
```

**Response:**
```json
{
  "result": {
    "running": true,
    "imported": 3994,
    "total": 6291,
    "pct": 63,
    "error": null
  }
}
```

### How it works

1. Detects the gap via binary search for the lowest existing block
2. Spawns a background tokio task (non-blocking — production continues)
3. Fetches each missing block via `getBlockRaw` from the source peer's RPC
4. **Chain-linking verification**: every block's `prev_hash` must match the previous block's hash
5. **Anchor verification**: the last backfilled block must connect to the lowest existing block
6. BLAKE3 checksum verified on every block
7. Only one backfill can run at a time (atomic guard)

### Comparison of backfill methods

| Method | Requires restart? | Source | Chain verification |
|--------|:-:|--------|:--:|
| `restore --from /path --backfill` | **Yes** (offline) | Local archive files | BLAKE3 only |
| `restore --from-rpc <URL> --backfill` | **Yes** (offline) | Any archiver RPC | BLAKE3 only |
| **`backfillFromPeer` RPC** | **No** (hot) | Any node's RPC | BLAKE3 + chain-linking + anchor |

### Security

- **Chain-linking**: Each block's `parent_hash` is verified against the previous block, starting from `genesis_hash` for block 1. A malicious peer cannot inject fabricated blocks — the chain must link continuously from genesis to the anchor.
- **Anchor verification**: After backfill completes, the hash of the last backfilled block is verified against the `prev_hash` of the lowest existing block. This ensures the backfilled chain connects to the node's existing validated chain.
- **RPC access**: Only accessible from localhost on most nodes (N3/N4/N5). External RPC nodes (N1/N2) are protected by the chain-linking verification.

## DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `archive.doli.network` | A | `72.60.228.233` | Archive server (omegacortex) |
