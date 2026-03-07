# Disaster Recovery

How to back up and restore a DOLI node from the block archive.

## Overview

DOLI maintains a block archive — a flat-file copy of every finalized block, each with a BLAKE3 integrity checksum. If a node's database is corrupted or lost, you can restore the full chain from this archive in minutes.

The archive is hosted at `archive.doli.network` (omegacortex.ai). It can be copied to any location via rsync.

**For nodes that joined via snap sync**, historical blocks are filled automatically via P2P backfill — no manual intervention needed. See [Automatic P2P Backfill](#automatic-p2p-backfill) below.

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

## Automatic P2P Backfill

Nodes that joined the network via snap sync are missing historical blocks (heights before their snap sync point). The P2P backfill system fills these gaps automatically — no manual intervention needed.

**How it works:**
1. On startup, the node scans heights 1–1000 to detect the first available block
2. If blocks before that height are missing, a background backfill task starts
3. The task requests missing blocks one at a time via `GetBlockByHeight` from connected peers
4. Blocks are stored in the BlockStore for historical record (no state rebuild)
5. Rate-limited to 100ms between requests to avoid overwhelming peers

**Key properties:**
- **Background, non-blocking**: Never interferes with forward sync, block production, or validation
- **Uses existing protocol**: `GetBlockByHeight` — no new P2P messages
- **Any peer can serve**: Not limited to the archiver; any peer with the blocks will respond
- **Resumable**: Progress tracked via BlockStore; restarts pick up where they left off
- **No state rebuild**: Blocks inserted for historical record only; state is correct from snap sync

**Logs:**
```
[BACKFILL] Gap detected: first block at height 470, missing 1..469
[BACKFILL] Stored block at height 1 from peer 12D3KooW...
[BACKFILL] Stored block at height 2 from peer 12D3KooW...
...
[BACKFILL] Complete — filled 469 missing blocks (1..469)
```

No configuration needed. P2P backfill runs automatically whenever a gap is detected.

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
       └── P2P backfill → Any node can request historical blocks from peers
```

## DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `archive.doli.network` | A | `198.51.100.1` | Archive server (omegacortex) |
