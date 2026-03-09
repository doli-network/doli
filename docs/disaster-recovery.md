# Disaster Recovery

How to back up and restore a DOLI node from the block archive.

> **See also:** [archiver.md](./archiver.md) for the full archiver architecture, seed/relay role, block explorer integration, and RPC methods.

## Overview

DOLI maintains a block archive — a flat-file copy of every applied block, each with a BLAKE3 integrity checksum. If a node's database is corrupted or lost, you can restore the full chain from this archive in minutes.

The archive is served by the seed/archiver nodes (`seed1.doli.network`, `seed2.doli.network`). It can be copied via rsync or fetched directly over RPC.

**For nodes that joined via snap sync**, historical blocks can be filled from the archive using `restore --backfill`. See [Backfill only](#backfill-only-fill-snap-sync-gaps) below.

**No SSH access to the archive server?** Use `restore --from-rpc` to download blocks directly from any seed node's RPC endpoint. See [Restore from RPC](#restore-from-rpc) below.

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

## Archiver Architecture & Hot Backfill

For detailed archiver architecture, service configuration, RPC methods (`getBlockRaw`, `backfillFromPeer`, `backfillStatus`), hot backfill, and DNS infrastructure, see **[archiver.md](./archiver.md)**.
