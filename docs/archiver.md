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
Producer nodes (N1-N6)
       │
       │ gossip (blocks + attestations)
       ▼
Archiver / Seed nodes (ai1 + ai2)
       │
       ├── P2P seed (DNS: seed1/seed2.doli.network)
       ├── Public RPC → doli.network/explorer.html
       ├── Archive flat files → disaster recovery
       └── getBlockRaw RPC → remote backfill source
```

Both archiver nodes run on ai1 and ai2 behind DNS round-robin, providing full redundancy.

### Node Inventory

#### Mainnet

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 (72.60.228.233) | 30300 | 8500 | `doli-mainnet-seed` | `seed1.doli.network` |
| Seed2 | ai2 (187.124.95.188) | 30300 | 8500 | `doli-mainnet-seed` | `seed2.doli.network` |

Archive directory: `/mainnet/seed/blocks/`

#### Testnet

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap1.testnet.doli.network` |
| Seed2 | ai2 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap2.testnet.doli.network` |

Archive directory: `/testnet/seed/blocks/`

### DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `seed1.doli.network` | A | 72.60.228.233 | Mainnet P2P seed + RPC |
| `seed2.doli.network` | A | 187.124.95.188 | Mainnet P2P seed + RPC |
| `archive.doli.network` | A | 187.124.95.188 | Mainnet archive RPC (legacy alias) |
| `bootstrap1.testnet.doli.network` | A | 72.60.228.233 | Testnet P2P seed + RPC |
| `bootstrap2.testnet.doli.network` | A | 187.124.95.188 | Testnet P2P seed + RPC |
| `archive.testnet.doli.network` | A | 72.60.228.233 | Testnet archive RPC (legacy alias) |

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
    --bootstrap /dns4/seed2.doli.network/tcp/30300
```

Key flags:
- `--relay-server`: Enables relay functionality for NAT-traversal
- `--rpc-bind 0.0.0.0`: Binds RPC to all interfaces (public access for explorer)
- `--archive-to <path>`: Enables block archiving to flat files
- No `--producer` or `--producer-key`: Non-producing, sync-only

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
  "genesis_hash": "d34b16931c4d..."
}
```

### Write Safety

- **Atomic writes**: Each file is written to a `.tmp` path first, then renamed — crash-safe
- **Non-blocking**: Uses `mpsc::channel` with `try_send` — never stalls block production or sync
- **Catch-up on startup**: Reads missing blocks from local BlockStore up to chain tip
- **Idempotent**: Skips blocks that are already archived (file existence check)

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
rsync -av ilozada@seed2.doli.network:/mainnet/seed/blocks/ /path/to/local/archive/

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

The node syncs normally and streams all applied blocks to the archive directory. On startup it catches up any blocks missed while offline.

For a dedicated archiver that also serves as a seed:

```bash
doli-node --network mainnet run \
    --relay-server \
    --rpc-bind 0.0.0.0 \
    --archive-to /path/to/archive \
    --bootstrap /dns4/seed1.doli.network/tcp/30300 \
    --bootstrap /dns4/seed2.doli.network/tcp/30300
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
| `crates/storage/src/archiver.rs` | BlockArchiver, catch-up, restore/backfill from files |
| `crates/rpc/src/methods.rs` | `getBlockRaw`, `backfillFromPeer`, `backfillStatus`, `verifyChainIntegrity` RPC handlers |
| `bins/node/src/main.rs` | CLI flags (`--archive-to`, `restore --from/--from-rpc/--backfill`), `restore_from_rpc()` |
