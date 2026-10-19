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
- `--archive-to` and `--relay-server` each imply `--no-snap-sync`. The node never snap-syncs: it closes every gap with full blocks (header-first) and keeps every block. Without an explicit `--no-snap-sync`, the startup log shows `Snap sync disabled: archive/seed role (--archive-to/--relay-server) syncs full blocks`. A new node at height 0 therefore replays the whole chain, which is slow on a long chain.

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

### 5.3. Hot Backfill: Fill and Verify a Height Hole

Use this runbook when a running node answers `Block not found` for a height range (for example, a snap sync skipped the range). The node keeps running, and producing, during steps 1-5.

Two other nodes take part:
- **Source**: a node that has the full range. `backfillFromPeer` copies blocks from it.
- **Verifier**: a different node that holds the range from its own header-first sync, where it applied every block in the range. Never use the source as the verifier, and never use a node that received the range by a backfill, a `restore` or a snap sync. Steps 4 and 5 prove that the repaired node agrees with the verifier's copy. They do not re-validate either copy.

Run the commands on the repaired node's host. `backfillFromPeer` and `verifyChainIntegrity` are admin methods; `getBlockRaw` is not. Send admin requests to `127.0.0.1` only: a loopback request needs no token. Run the verifier's commands on the verifier's host over ssh, also against its own `127.0.0.1`. Never send `Authorization: Bearer <admin-token>` over plain `http://` to an address that is not loopback. The RPC server has no TLS, so anyone on the path can read the token and replay it against every admin method. The source needs no token: backfill reads it with `getBlockRaw`. On mainnet, `backfillFromPeer` refuses a URL whose host is a private, loopback or link-local IP literal, so give the source as a DNS name. Set these once (the verifier host needs `curl` and `jq`):

```bash
PORT=8500                                     # RPC port of the node to repair (this host)
NODE=http://127.0.0.1:$PORT
SOURCE=http://<source-hostname>:8500          # has the full range; no token is sent to it
VERIFIER_HOST=<verifier-host>                 # ssh name of a third node's host, NOT the source
VERIFIER_PORT=8500                            # RPC port of the verifier on its host
ARCHIVE_DIR=/path/to/archive                  # the node's --archive-to directory
# One JSON-RPC call to the node on 127.0.0.1:<port> of the host where it runs.
rpc() { curl -s -X POST "http://127.0.0.1:$1" -H "Content-Type: application/json" -d "$2"; }
```

Several nodes can share one host, each on its own port. Before step 1, prove that the three URLs reach three different nodes. `getNodeInfo` is not an admin method:

```bash
ID='{"jsonrpc":"2.0","method":"getNodeInfo","params":{},"id":1}'
rpc "$PORT" "$ID" | jq -r .result.peerId
{ declare -f rpc; printf 'rpc %q %q\n' "$VERIFIER_PORT" "$ID"; } | ssh "$VERIFIER_HOST" bash | jq -r .result.peerId
curl -s -X POST "$SOURCE" -H "Content-Type: application/json" -d "$ID" | jq -r .result.peerId
```

Continue only when the command prints three different peer IDs.

**Trust root first (INC-I-248).** A backfill also fills the hole that the start-time trust-root check looks for (block D present, block D+1 missing). If the node was ever snap-synced, start it once on a release with the INC-I-248 fix before step 2, and before any `restore`. That start resets a maintainer set that the node did not derive from replayed history. Then compare the release trust root with the verifier's, before step 2 and again after step 3. `getMaintainerSet` is not an admin method:

```bash
GM='{"jsonrpc":"2.0","method":"getMaintainerSet","params":{},"id":1}'
rpc "$PORT" "$GM" | jq -r '.result | "\(.source) \(.member_count) \(.maintainer_set_digest)"'
{ declare -f rpc; printf 'rpc %q %q\n' "$VERIFIER_PORT" "$GM"; } | ssh "$VERIFIER_HOST" bash | jq -r '.result | "\(.source) \(.member_count) \(.maintainer_set_digest)"'
```

Pass: both lines show `on-chain` and the same `maintainer_set_digest`. A `member_count` of 0 on the repaired node is normal for a few minutes after that start: wait for the `[MAINTAINER] quorum verdict` line in its log. If the digests still differ after a backfill, see [troubleshooting.md §5.4](./troubleshooting.md), "After a backfill".

1. **Find the hole** on the repaired node:
   ```bash
   curl -s -X POST "$NODE" -H "Content-Type: application/json" \
       -d '{"jsonrpc":"2.0","method":"verifyChainIntegrity","params":[],"id":1}'
   # → "missing": ["100001-103284"], "missingCount": 3284
   FROM=100001; TO=103284                     # the hole, for steps 4-6
   ```
   This proves which heights have no block in the node's block store. If `missing` lists more than one range, do steps 4-6 for each range.
2. **Start the backfill.** Give the source's RPC URL, never a peer ID. The node scans every height from 1 to its tip and fetches each missing block. There is no range parameter.
   ```bash
   curl -s -X POST "$NODE" -H "Content-Type: application/json" \
       -d "{\"jsonrpc\":\"2.0\",\"method\":\"backfillFromPeer\",\"params\":{\"rpc_url\":\"$SOURCE\"},\"id\":1}"
   # → {"started": true, "gaps": "100001-103284", "total": 3284}
   ```
   The call fails with `Tip divergence detected` when the local tip hash differs from the source's block at the same height. Backfill cannot repair diverged state. Set `"skip_divergence_check": true` only after a genesis reset, where the tips differ by design. Backfill writes the block store (RocksDB) only. It does not apply state and never writes the archive directory.
3. **Wait for the end.** Poll until `running` is `false` and `error` is `null`:
   ```bash
   curl -s -X POST "$NODE" -H "Content-Type: application/json" \
       -d '{"jsonrpc":"2.0","method":"backfillStatus","params":{},"id":1}'
   # → {"running": false, "imported": 3284, "total": 3284, "pct": 100, "error": null}
   ```
   If the source does not have a height, backfill skips it with a log warning. `error` stays `null` and `imported` stays below `total`. Step 4 shows these heights. Run step 2 again with another source.

   This proves only that the copy finished. The BLAKE3 checksum that backfill checks comes from the source together with the block, so it detects accidental corruption only. It does not prove that the block is authentic, and backfill skips it when the source sends no checksum. Backfill does not validate the blocks: it checks no signature, VDF or merkle root.
4. **Compare the block hashes with the verifier.** Pick one height `H` at or above `TO` and at or below both tips. Run `verifyChainIntegrity [H]` with the SAME `H` on the repaired node and on the verifier:
   ```bash
   H=$TO
   Q="{\"jsonrpc\":\"2.0\",\"method\":\"verifyChainIntegrity\",\"params\":[$H],\"id\":1}"
   rpc "$PORT" "$Q"; echo
   { declare -f rpc; printf 'rpc %q %q\n' "$VERIFIER_PORT" "$Q"; } | ssh "$VERIFIER_HOST" bash; echo
   ```
   Pass: both return `"missingCount": 0` and the same `chainCommitment`. The commitment is a BLAKE3 chain over the block header hashes 1..H, so equal values prove that every header hash in 1..H matches the verifier's. They do not prove the block bodies or the VDF proof: a header hash commits to the transaction merkle root, but nothing on the backfill path checks a body against its header, and the header hash does not cover the VDF proof. Always give an explicit `H`: the no-parameter form can return a persisted commitment for another height.
5. **Compare the block bytes with the verifier**, for every height in the hole:
   ```bash
   # Prints "<height> <blake3>" per height of the node on port $1; "none" when it has no block.
   hashes() { for h in $(seq "$2" "$3"); do
       curl -s -X POST "http://127.0.0.1:$1" -H "Content-Type: application/json" \
           -d "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockRaw\",\"params\":{\"height\":$h},\"id\":1}" \
           | jq -r --arg h "$h" '"\($h) \(.result.blake3 // "none")"'
   done; }
   hashes "$PORT" "$FROM" "$TO" > node.hashes
   { declare -f hashes; echo "hashes $VERIFIER_PORT $FROM $TO"; } | ssh "$VERIFIER_HOST" bash > verifier.hashes
   n=$((TO - FROM + 1))
   a=$(grep -c ' [0-9a-f]\{64\}$' node.hashes); b=$(grep -c ' [0-9a-f]\{64\}$' verifier.hashes)
   echo "expected=$n node=$a verifier=$b"
   [ "$n" -gt 0 ] && [ "$a" = "$n" ] && [ "$b" = "$n" ] && diff node.hashes verifier.hashes && echo "block bytes match"
   ```
   Pass: the last line is `block bytes match`. It prints only when both lists hold one valid `blake3` for every height in `FROM..TO` and `diff` finds no difference. A missing height, a failed call, an unreachable verifier or an empty range can never print it. Each node computes `blake3` over its whole stored block: the header with its VDF proof, and the body (transactions, attestation bitfield, BLS aggregate). Equal values prove that every block in `FROM..TO` is byte-identical to the verifier's copy. This step does not prove that the verifier's copy is valid. That is why the verifier must hold the range from its own header-first sync.
6. **Refill the archive directory** (archive nodes only). Backfill never writes the archive directory. The archiver copies missing `.block` files from the block store above the manifest `verified_height`, which stays below the hole: the running archiver does it at the next finalized block, and a restart does it in one background catch-up while RPC answers. `verified_height` stays below a height that is in neither the archive directory nor the block store. After step 5, check the range:
   ```bash
   for h in $(seq "$FROM" "$TO"); do
       f=$(printf '%s/%010d.block' "$ARCHIVE_DIR" "$h"); [ -f "$f" ] || echo "missing $h"
   done
   ```
   No output means that every file exists. This step checks presence only, not file contents.

Restart an archive node only with a binary where `--archive-to` and `--relay-server` imply `--no-snap-sync` (§2). An older binary can snap-sync a far-behind archive node again and open a new hole. Only one backfill runs at a time.

### 5.4. Comparison of Recovery Methods

| Method | Restart? | Source | Chain Verification |
|--------|:--------:|--------|:------------------:|
| `restore --from /path` | Yes (offline) | Local archive files | BLAKE3 + genesis_hash |
| `restore --from-rpc <URL>` | Yes (offline) | Any seed/archiver RPC | BLAKE3 + genesis_hash |
| `backfillFromPeer` RPC | **No** (hot) | Any node's RPC | BLAKE3 (accidental corruption only); then header hashes (`verifyChainIntegrity [H]`) and block bytes (`getBlockRaw` blake3) compared with a second node (§5.3) |

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

Starts a background task to fill block-store gaps from a remote node's RPC. Params: `rpc_url` (required) and `skip_divergence_check` (optional, default `false`). It writes the block store only, never the archive directory. Procedure: §5.3.

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

Full scan of every height from 1 to tip (or `[up_to_height, from_height]`). Detects missing blocks (gaps) anywhere in the chain and returns a BLAKE3 `chainCommitment` over the block hashes. To compare two nodes, pass the same explicit `up_to_height` to both. Full reference: [rpc_reference.md](./rpc_reference.md).

**Request:**
```json
{"jsonrpc": "2.0", "method": "verifyChainIntegrity", "params": [], "id": 1}
```

**Response (complete):**
```json
{"result": {"complete": true, "tip": 1223, "scanned": 1223, "fromHeight": 1, "missing": [], "missingCount": 0, "chainCommitment": "abc123..."}}
```

**Response (gaps found):**
```json
{"result": {"complete": false, "tip": 1000000, "scanned": 1000000, "fromHeight": 1, "missing": ["45-67", "1234"], "missingCount": 24, "chainCommitment": null}}
```

Missing heights are compressed into ranges. Performance: ~10-30s for 1M blocks on SSD. Added in v2.0.29.

---

## 7. Running Your Own Archiver

Any node can archive blocks by adding `--archive-to`:

```bash
doli-node --network mainnet run --archive-to /path/to/archive
```

The node syncs header-first (never snap, see §2) and streams finalized blocks to the archive directory. At each start, one background catch-up checks the heights above the manifest `verified_height` and fills missing blocks from the local BlockStore, so the archive stays complete after a resync. RPC answers while it runs.

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
