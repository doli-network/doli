# Block Archiver & Seed Infrastructure

The DOLI block archiver has evolved from a simple disaster recovery tool into a core infrastructure component. Archiver nodes serve as **seed nodes** (DNS-registered network entry points), **block explorers** (powering `doli.network/explorer.html`), and **disaster recovery** sources — all in one process.

---

## 1. Architecture

Archiver nodes are full sync-only nodes that run with `--archive-to` and `--relay-server`. They do not produce blocks but serve three critical roles:

| Role | Description |
|------|-------------|
| **Seed / Relay** | DNS-registered P2P entry points for new nodes joining the network |
| **Block Archive** | Stream every applied block to flat files with BLAKE3 checksums |
| **RPC Gateway** | Public RPC bound to `0.0.0.0` — serves the block explorer and external queries |

### Deployment

```
Producer nodes
       │
       │ gossip (blocks + attestations)
       ▼
Archiver / Seed nodes
       │
       ├── P2P seed (DNS: seed1/seed2.doli.network)
       ├── Public RPC → doli.network/explorer.html
       ├── Archive flat files → disaster recovery
       └── getBlockRaw RPC → remote backfill source
```

Several archiver nodes run on separate servers behind DNS round-robin, providing redundancy.

### Data Flow: Archive vs Block Store

Each seed maintains **two independent copies** of block data that serve different purposes:

```
apply_block()                       --archive-to flag
     │                                    │
     ▼                                    ▼
Block Store (RocksDB)               Archive (flat files)
/testnet/seed/data/blocks/          /testnet/seed/blocks/
     │                                    │
     │ ◄── getBlockRaw RPC               │ ◄── restore --from --backfill
     │ ◄── backfillFromPeer (source)      │     (imports into block store)
     │                                    │
     ▼                                    │
backfillFromPeer (target) ◄───────────────┘
                            (indirect: archive → seed block store → RPC → producer block store)
```

After snap sync or chain reset, the block store has gaps but the archive is complete (it has been streaming every block since genesis). To fix gaps across the network:

1. **Seeds:** `restore --from <archive_dir> --backfill --yes` (requires stopping the seed — exclusive DB lock)
2. **Producers:** `backfillFromPeer` RPC pointing to a seed with a complete block store (hot, no restart)

See [disaster-recovery.md](./disaster-recovery.md) for the full runbook.

### Node Inventory

#### Mainnet

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 (<ai1-ip>) | 30300 | 8500 | `doli-mainnet-seed` | `seed1.doli.network` |
| Seed2 | ai2 (<ai2-ip>) | 30300 | 8500 | `doli-mainnet-seed` | `seed2.doli.network` |
| Seed3 | ai3 (<ai3-ip>) | 30300 | 8500 | `doli-mainnet-seed` | `seeds.doli.network` |

Archive directory: `/mainnet/seed/blocks/`

#### Testnet

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap1.testnet.doli.network` |
| Seed2 | ai2 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap2.testnet.doli.network` |
| Seed3 | ai3 | 40300 | 18500 | `doli-testnet-seed` | `seeds.testnet.doli.network` |

Archive directory: `/testnet/seed/blocks/`

### DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `seed1.doli.network` | A | <ai1-ip> | Mainnet P2P seed + RPC |
| `seed2.doli.network` | A | <ai2-ip> | Mainnet P2P seed + RPC |
| `seeds.doli.network` | A | <ai3-ip> | Mainnet P2P seed round-robin + RPC |
| `archive.doli.network` | A | <ai2-ip> | Mainnet archive RPC (legacy alias) |
| `bootstrap1.testnet.doli.network` | A | <ai1-ip> | Testnet P2P seed + RPC |
| `bootstrap2.testnet.doli.network` | A | <ai2-ip> | Testnet P2P seed + RPC |
| `seeds.testnet.doli.network` | A | <ai3-ip> | Testnet P2P seed round-robin + RPC |
| `archive.testnet.doli.network` | A | <ai1-ip> | Testnet archive RPC (legacy alias) |

---

## 2. Service Configuration

Archiver/seed nodes run with these flags:

```bash
doli-node --network mainnet run \
    --relay-server \
    --rpc-bind 0.0.0.0 \
    --p2p-port 30300 \
    --rpc-port 8500 \
    --archive-to /mainnet/seed/blocks \
    --bootstrap /dns4/seed1.doli.network/tcp/30300 \
    --bootstrap /dns4/seed2.doli.network/tcp/30300 \
    --bootstrap /dns4/seed3.doli.network/tcp/30300
```

Key flags:
- `--relay-server`: Enables relay functionality for NAT-traversal
- `--rpc-bind 0.0.0.0`: Binds RPC to all interfaces (public access for explorer)
- `--archive-to <path>`: Enables block archiving to flat files
- No `--producer` or `--producer-key`: Non-producing, sync-only

The fleet watchdog (`scripts/doli-watchdog.sh`) does not restart a unit that became active less than `DOLI_WATCHDOG_COLD_START_GRACE` seconds ago (default 600). See [troubleshooting.md §1.11](./troubleshooting.md#111-archive-node-restarts-in-a-loop-after-a-host-reboot-inc-i-249).

---

## 3. Block Explorer

The block explorer at `https://doli.network/explorer.html` queries the archiver's public RPC to display chain data. The RPC methods it uses:

| Method | Purpose |
|--------|---------|
| `getChainInfo` | Current height, slot, version, genesis hash |
| `getBlockByHeight` | Block details (header, transactions, producer) |
| `getProducers` | Active producer list with bond counts |
| `getBalance` | Address balance lookup |
| `getTransaction` | Transaction details by hash |

The explorer connects to the seed RPC endpoints (`seed1.doli.network:8500` / `seed2.doli.network:8500`) which are publicly accessible because archivers run with `--rpc-bind 0.0.0.0`.

---

## 4. Archive Format

Each applied block is streamed to flat files with atomic writes:

```
/mainnet/seed/blocks/
  0000000001.block      # Bincode-serialized Block struct
  0000000001.blake3     # BLAKE3 checksum of the .block file
  0000000002.block
  0000000002.blake3
  ...
  manifest.json         # Latest archived state
```

**manifest.json:**
```json
{
  "latest_height": 12345,
  "latest_hash": "a1b2c3d4e5f6...",
  "genesis_hash": "d34b16931c4d...",
  "verified_height": 12340
}
```

- `latest_height` is a high-water mark (import reads `1..=latest_height`, skipping missing files).
- A `manifest.json` larger than 64 KiB is never read: every reader (catch-up, fence, import, RPC) treats it as absent, so the next catch-up runs one full check and rewrites it.
- `verified_height` (H) is the highest height such that every `.block` + `.blake3` pair in `1..=H` was read and matched the node genesis, or was written by the archiver from the node's own chain. It is written in the same atomic manifest write as `genesis_hash`. Manifests from older binaries have no `verified_height`.

### Write Safety

- **Atomic, durable writes**: The archiver writes each file to `<name>.tmp-archiver`, calls `sync_all`, then renames it. A manifest save covers a height only after its pair is durable. (`repairArchiveFromPeer` uses `.tmp`, so the two writers never share a tmp file.)
- **Non-blocking**: The node hands each finalized block to the archive thread with `try_send` on a 256-slot channel, so archiving never stalls block production or sync. A full channel drops the block; the next archived block fills the missing heights from the BlockStore (`archive_finalized`).
- **O(1) start**: At start the node reads only `manifest.json` (genesis fence: a foreign `genesis_hash` renames the directory to `<dir>.stale-<unix-ts>` and recreates it empty; when the rename fails, the fence removes the contents of the directory in place, `manifest.json` last; when that removal fails too, the archiver does not start for that run and the node logs `Archiver not started: genesis fence of <dir> failed: ...`), opens the channel and continues. RPC does not wait for any archive IO.
- **One background catch-up per start**: One detached OS thread (`doli-archiver`) deletes a fenced stale directory, runs one `catch_up_bounded` up to the tip at start, then runs the live archive loop. It is the only `manifest.json` writer. No periodic task runs a catch-up.
- **Bounded catch-up**: `catch_up(.., Some(genesis))` trusts H only when the manifest parses, `genesis_hash` equals the node genesis, `verified_height <= latest_height`, and the spot-check at H passes. The spot-check is metadata-only: both files of the pair at H exist and are not empty; it never reads file H. Then it reads only `H+1..=tip`, so a bounded catch-up reads at most `tip − H` block files: an existing pair is read and its genesis checked (a foreign file is rewritten from the BlockStore); a missing pair is written from the BlockStore. H advances only over contiguous verified or self-written heights; it stops at a height missing from both the archive and the BlockStore. A file that does not deserialize (INC-I-119 legacy) is intact when its `.blake3` sidecar matches and its header decodes: with the node genesis, H crosses it and the file is kept (never rewritten); with another genesis, it is rewritten like a foreign file. Any other undeserializable file stops H below it and is kept. An untrusted or missing H costs one full check from height 1, with the manifest saved at least every 1000 heights (resumable).
- **First start on an older archive**: A manifest written by an older binary has no `verified_height`, so the first start of a new binary runs one full check from height 1 in the background. RPC stays up, but the check reads every archive file: on a slow disk the IO load is high until it finishes. Roll archive nodes one at a time.

### Integrity

- Every block has a `.blake3` BLAKE3 checksum sidecar
- `manifest.json` includes `genesis_hash` to prevent cross-chain confusion
- Manual verification: `b3sum 0000000042.block` vs `cat 0000000042.blake3`

---

## 5. Disaster Recovery

### 5.1. Restore from Local Archive

Copy the archive from the seed node, then import:

```bash
# Copy archive via rsync
rsync -av $USER@seed2.doli.network:/mainnet/seed/blocks/ /path/to/local/archive/

# Full restore — imports all blocks + rebuilds state
doli-node --network mainnet restore --from /path/to/local/archive --yes

# Backfill only — fills snap sync gaps, no state rebuild
doli-node --network mainnet restore --from /path/to/local/archive --backfill --yes
```

### 5.2. Restore from RPC (No SSH Required)

Download blocks directly from any archiver's RPC:

```bash
# Full restore via RPC
doli-node --network mainnet restore --from-rpc http://seed2.doli.network:8500 --yes

# Backfill via RPC
doli-node --network mainnet restore --from-rpc http://seed2.doli.network:8500 --backfill --yes
```

How it works:
1. Queries `getChainInfo` for the archiver's best height
2. Fetches block 1 via `getBlockRaw` to validate genesis hash
3. Downloads each block as base64 bincode via `getBlockRaw`
4. Verifies BLAKE3 checksum on every block
5. In backfill mode, skips blocks already in local BlockStore

### 5.3. Hot Backfill (Live — No Restart)

For nodes already running and synced to tip but missing historical blocks (snap sync gaps), `backfillFromPeer` fills the gap while the node keeps producing:

```bash
# Start backfill
curl -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillFromPeer","params":{"rpc_url":"http://seed2.doli.network:8500"},"id":1}'

# Monitor progress
curl -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillStatus","params":{},"id":1}'
```

**Response:**
```json
{"result": {"running": true, "imported": 3994, "total": 6291, "pct": 63, "error": null}}
```

Security guarantees:
- **Chain-linking**: Every block's `prev_hash` must match the previous block's hash, from genesis
- **Anchor verification**: Last backfilled block must connect to the lowest existing block
- **BLAKE3**: Checksum verified on every block
- **Single instance**: Only one backfill can run at a time (atomic guard)

### 5.4. Comparison of Recovery Methods

| Method | Restart? | Source | Chain Verification |
|--------|:--------:|--------|:------------------:|
| `restore --from /path` | Yes (offline) | Local archive files | BLAKE3 + genesis_hash |
| `restore --from-rpc <URL>` | Yes (offline) | Any seed/archiver RPC | BLAKE3 + genesis_hash |
| `backfillFromPeer` RPC | **No** (hot) | Any node's RPC | BLAKE3 + chain-linking + anchor |

---

## 6. RPC Methods

### getBlockRaw

Returns the raw bincode-serialized block with BLAKE3 checksum. Used by `restore --from-rpc` and `backfillFromPeer`.

**Reads from:** Block store (RocksDB) only. Does **not** read from archive flat files. If a block exists in the archive but not in the block store, `getBlockRaw` returns an error. To serve archive-only blocks, first run `restore --from <archive> --backfill` on the seed.

**Request:**
```json
{"jsonrpc": "2.0", "method": "getBlockRaw", "params": {"height": 42}, "id": 1}
```

**Response:**
```json
{
  "result": {
    "block": "<base64-encoded bincode>",
    "blake3": "a1b2c3d4...",
    "height": 42
  }
}
```

Unlike `getBlockByHeight` (JSON — lossy for signatures/VDF proofs), `getBlockRaw` returns exact bincode bytes.

**Legacy compatibility:** Blocks stored from older node versions are deserialized via `deserialize_block_compat()` (tries current format, then LegacyBlockV2, then LegacyBlock) and re-serialized in current format before returning. Callers always receive current-format bincode.

### backfillFromPeer

Starts a background task to fill snap sync gaps from a remote node's RPC.

**Request:**
```json
{"jsonrpc": "2.0", "method": "backfillFromPeer", "params": {"rpc_url": "http://seed2.doli.network:8500"}, "id": 1}
```

**Response:**
```json
{"result": {"started": true, "gaps": "1-6291", "total": 6291}}
```

### backfillStatus

Returns current backfill progress.

**Request:**
```json
{"jsonrpc": "2.0", "method": "backfillStatus", "params": {}, "id": 1}
```

**Response:**
```json
{"result": {"running": true, "imported": 3994, "total": 6291, "pct": 63, "error": null}}
```

### verifyChainIntegrity

Full scan of every height from 1 to tip. Detects missing blocks (gaps) anywhere in the chain — not just at the start. Uses lightweight height-index lookups (no block deserialization).

**Request:**
```json
{"jsonrpc": "2.0", "method": "verifyChainIntegrity", "params": [], "id": 1}
```

**Response (complete):**
```json
{"result": {"complete": true, "tip": 1223, "scanned": 1223, "missing": [], "missing_count": 0}}
```

**Response (gaps found):**
```json
{"result": {"complete": false, "tip": 1000000, "scanned": 1000000, "missing": ["45-67", "1234"], "missing_count": 24}}
```

Missing heights are compressed into ranges. Performance: ~10-30s for 1M blocks on SSD. Added in v2.0.29.

---

## 7. Running Your Own Archiver

Any node can archive blocks by adding `--archive-to`:

```bash
doli-node --network mainnet run --archive-to /path/to/archive
```

The node syncs normally and streams finalized blocks to the archive directory. At each start, one background catch-up checks the heights above the manifest `verified_height` and fills missing blocks from the local BlockStore, so the archive stays complete after a resync. RPC answers while it runs.

For a dedicated archiver that also serves as a seed:

```bash
doli-node --network mainnet run \
    --relay-server \
    --rpc-bind 0.0.0.0 \
    --archive-to /path/to/archive \
    --bootstrap /dns4/seed1.doli.network/tcp/30300 \
    --bootstrap /dns4/seed2.doli.network/tcp/30300 \
    --bootstrap /dns4/seed3.doli.network/tcp/30300
```

---

## 8. Off-Site Backup

The archive is plain files — use any backup tool:

```bash
# rsync to a remote server
rsync -av /mainnet/seed/blocks/ backup-server:/backups/doli-archive/

# Upload to S3-compatible storage
rclone sync /mainnet/seed/blocks/ s3:doli-backups/archive/

# Create a tarball
tar czf doli-archive-$(date +%Y%m%d).tar.gz /mainnet/seed/blocks/
```

---

## 9. Code Reference

| File | Purpose |
|------|---------|
| `crates/storage/src/archiver/` | BlockArchiver, `catch_up()` / `catch_up_bounded()` (`catch_up.rs`), `manifest.json` keys (`manifest.rs`), durable writes (`write.rs`), restore/backfill from files (`import.rs`), O(1) stale-archive fence (`fence.rs`) |
| `bins/node/src/node/archive_thread.rs` | `start_archive_thread()`, called from `bins/node/src/run.rs`: O(1) genesis fence on the caller (a fence error returns `Err` and starts nothing), then one detached thread runs stale-dir delete → one `catch_up_bounded()` → live loop (`archive_finalized()`) |
| `bins/node/src/node/periodic.rs` | `flush_finalized_to_archive()`: sends finalized blocks to the archive channel (no catch-up) |
| `crates/rpc/src/methods/` | `getBlockRaw`, `backfillFromPeer`, `backfillStatus`, `verifyChainIntegrity` RPC handlers (dispatch in `dispatch.rs`, backfill in `backfill.rs`) |
| `bins/node/src/main.rs` | CLI flags (`--archive-to`, `restore --from/--from-rpc/--backfill`); delegates to `operations` module for `restore_from_rpc()`, `backfill_from_archive()`, `restore_from_archive()` |
