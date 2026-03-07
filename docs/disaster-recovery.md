# Disaster Recovery

How to back up and restore a DOLI node from the block archive.

## Overview

DOLI maintains a block archive — a flat-file copy of every finalized block, each with a BLAKE3 integrity checksum. If a node's database is corrupted or lost, you can restore the full chain from this archive in minutes.

The archive is hosted at `archive.doli.network` (omegacortex.ai) and can be copied to any location via rsync.

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
       │ rsync / rclone
       ▼
  Off-site backup (S3, remote server, etc.)
```

## DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `archive.doli.network` | A | `198.51.100.1` | Archive server (omegacortex) |
