# Proposal: P2P Historical Block Backfill via archive.doli.network

> **Status: IMPLEMENTED** (commit `7ee401c`). Backfill runs automatically on every node.
> Implementation in `bins/node/src/node.rs` — `detect_backfill_gap()`, `maybe_backfill_block()`, `handle_backfill_response()`.

## Problem

Nodes that joined the network via snap sync are missing historical blocks (heights 1 to ~475). Currently 8 of 25 nodes have this gap. The only recovery mechanism is file-based (`backfill_from_archive`), which requires SSH access to the archive server and manual file transfer.

## Observation

Everything needed already exists:

| Component | Status |
|-----------|--------|
| `GetBlockByHeight` P2P request | Implemented |
| Handler that serves blocks by height from BlockStore | Implemented (`node.rs:4161`) |
| Archiver node with all blocks since height 1 | Running (`archive.doli.network`) |
| Archiver as P2P peer (libp2p, gossipsub) | Already connected |
| Backfill detection (first available block > 1) | Not implemented |
| Backfill request loop | Not implemented |

## Proposal

Add a background **backfill task** to the sync layer that fills historical gaps using existing P2P primitives. No new protocol messages, no new endpoints, no new infrastructure.

### Design

```
Node starts
  |
  v
Detect first available block in BlockStore (e.g. height 470)
  |
  v
If first_block > 1:
  Start background backfill task
  |
  v
For each missing height (1, 2, 3, ... 469):
  Send GetBlockByHeight to a connected peer
  Store response in BlockStore (put_block_canonical)
  |
  v
Done — node has complete history
```

### Key decisions

1. **Background, non-blocking**: Backfill runs as a low-priority background task. It never interferes with forward sync, block production, or validation. Production and tip-sync always take priority.

2. **Uses existing protocol**: `GetBlockByHeight` request + `SyncResponse::Block` response. No new message types.

3. **Any peer can serve**: Not limited to the archiver. Any peer that has the blocks will respond. The archiver is just the guaranteed source.

4. **Rate-limited**: Request 1 block at a time or in small batches (e.g. 10) to avoid overwhelming peers. Historical backfill is not urgent — it can take minutes.

5. **Resumable**: Tracks progress via BlockStore. If the node restarts, it picks up where it left off (skips blocks already stored).

6. **No state rebuild**: Backfill only inserts blocks into BlockStore for historical record. It does NOT replay them through consensus or modify UTXO/producer state. State is already correct from snap sync.

### archive.doli.network as bootstrap

Add `archive.doli.network` to the bootstrap node list (or as a well-known peer). This ensures every node can discover at least one peer with complete history.

```
# Bootstrap multiaddr
/dns4/archive.doli.network/tcp/30306/p2p/<peer_id>
```

Any node that connects to the network will automatically have access to the archiver as a peer, and the backfill task will use it (along with any other peers that have the blocks) to fill gaps.

### External nodes

A new operator joining the network:
1. Starts `doli-node` with default bootstrap list (which includes `archive.doli.network`)
2. Node snap-syncs to tip (seconds)
3. Background backfill kicks in, requests historical blocks from archiver peer
4. Full history available without any manual steps, SSH access, or file transfers

No DNS registration needed by the operator. No rsync. No special commands. It just works.

## Implementation scope

| Change | File | Effort |
|--------|------|--------|
| Detect first available block on startup | `node.rs` (`detect_backfill_gap()`) | Done |
| Background backfill loop (request missing heights) | `node.rs` (`maybe_backfill_block()`) | Done |
| Handle backfill responses | `node.rs` (`handle_backfill_response()`) | Done |
| Rate limiting (100ms between requests) | `node.rs` | Done |

**Not needed:**
- No new P2P messages
- No new RPC endpoints
- No HTTP/REST API
- No new DNS records
- No firewall changes
- No changes to the archiver node

## Comparison

| Approach | Requires SSH? | Requires file transfer? | Automatic? | Decentralized? |
|----------|:---:|:---:|:---:|:---:|
| File-based `backfill_from_archive` | Yes | Yes | No | No |
| HTTP endpoint on archiver | No | No | No | No (single server) |
| **P2P backfill (this proposal)** | **No** | **No** | **Yes** | **Yes** |

## Precedent

- **Bitcoin**: Nodes request historical blocks from peers via `getdata`. "Assumevalid" skips signature verification for old blocks but still downloads them.
- **Ethereum**: Portal Network serves historical data. Full nodes serve blocks to peers on request.
- **DOLI (this proposal)**: Same pattern. `GetBlockByHeight` is our `getdata`. The archiver is our guaranteed seed.
