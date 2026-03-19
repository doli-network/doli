# protocol.md - Wire Protocol Specification

This document specifies the DOLI network protocol, message formats, and P2P communication standards.

---

## 1. Overview

DOLI uses libp2p as its networking foundation with multiple sub-protocols:

| Protocol | Path | Purpose |
|----------|------|---------|
| Status | `/doli/status/1.0.0` | Peer handshake and chain state |
| Sync | `/doli/sync/1.0.0` | Block synchronization |
| Kademlia | `/doli/kad/1.0.0` | Peer discovery |
| GossipSub | `/doli/blocks/1`, `/doli/txs/1` | Message propagation |

---

## 2. Transport Layer

### 2.1. Connection Stack

```
TCP (with TCP_NODELAY)
    │
    ▼
Noise Protocol (Ed25519 encryption)
    │
    ▼
Yamux (stream multiplexing)
```

- **TCP**: Tokio-based async TCP with nodelay optimization
- **Noise**: Ed25519 keypair-based authenticated encryption
- **Yamux**: Multiplexes concurrent streams over single connection

### 2.2. Network Ports

| Network | P2P Port | RPC Port |
|---------|----------|----------|
| Mainnet | 30300 | 8500 |
| Testnet | 40300 | 18500 |
| Devnet | 50300 | 28500 |

---

## 3. Message Serialization

All messages use **bincode** binary encoding with length-prefix framing:

```
┌──────────────────┬────────────────────────┐
│ Length (4 bytes) │ Payload (bincode)      │
│ Little-endian    │ Variable length        │
└──────────────────┴────────────────────────┘
```

Maximum message sizes:
- Status messages: 64 KB
- Sync messages: 16 MB
- GossipSub messages: 1 MB

---

## 4. Status Protocol

Request-response protocol for peer handshake (`/doli/status/1.0.0`).

### 4.1. StatusRequest

```rust
struct StatusRequest {
    version: u32,              // Protocol version
    network_id: u32,           // 1=mainnet, 2=testnet, 99=devnet
    genesis_hash: [u8; 32],    // Genesis block hash
    producer_pubkey: Option<[u8; 32]>, // If node is a producer
}
```

### 4.2. StatusResponse

```rust
struct StatusResponse {
    version: u32,
    network_id: u32,
    genesis_hash: [u8; 32],
    best_height: u64,          // Current chain height
    best_hash: [u8; 32],       // Best block hash
    best_slot: u32,            // Best block slot
    producer_pubkey: Option<[u8; 32]>,
}
```

### 4.3. Handshake Flow

1. Initiator sends `StatusRequest`
2. Responder validates `network_id` and `genesis_hash`
3. Responder sends `StatusResponse`
4. Connection established or rejected

**Rejection conditions:**
- Network ID mismatch
- Genesis hash mismatch
- Protocol version incompatible

---

## 5. Sync Protocol

Request-response protocol for chain synchronization (`/doli/sync/1.0.0`).

### 5.1. Request Types

```rust
enum SyncRequest {
    GetHeaders {
        start_hash: [u8; 32],  // Start from this hash (zero = genesis)
        max_count: u32,        // Maximum headers (default: 2000)
    },
    GetBodies {
        hashes: Vec<[u8; 32]>, // Block hashes to fetch
    },
    GetBlockByHeight {
        height: u64,
    },
    GetBlockByHash {
        hash: [u8; 32],
    },
}
```

### 5.2. Response Types

```rust
enum SyncResponse {
    Headers(Vec<BlockHeader>),
    Bodies(Vec<Block>),
    Block(Option<Block>),
    Error(String),
}
```

### 5.3. Sync Flow

```
┌─────────────┐                    ┌─────────────┐
│   Syncer    │                    │    Peer     │
└──────┬──────┘                    └──────┬──────┘
       │                                  │
       │ GetHeaders(start, 2000)          │
       │─────────────────────────────────>│
       │                                  │
       │ Headers([h1, h2, ..., h2000])    │
       │<─────────────────────────────────│
       │                                  │
       │ GetBodies([h1..h128])            │
       │─────────────────────────────────>│
       │                                  │
       │ Bodies([b1, b2, ..., b128])      │
       │<─────────────────────────────────│
       │                                  │
       │ (repeat for remaining bodies)    │
       │                                  │
```

**Sync parameters:**
- Max headers per request: 2000
- Max bodies per request: 128
- Max concurrent body requests: 8
- Request timeout: 30 seconds

---

## 6. GossipSub Topics

Pub-sub message propagation using GossipSub protocol.

### 6.1. Topics

| Topic | Content | Max Size |
|-------|---------|----------|
| `/doli/blocks/1` | New blocks | 1 MB |
| `/doli/txs/1` | New transactions | 1 MB |
| `/doli/producers/1` | Producer announcements | 64 KB |

### 6.2. GossipSub Parameters

```
heartbeat_interval:    1 second
mesh_n:                12 peers (target)
mesh_n_low:            8 peers (minimum)
mesh_n_high:           24 peers (maximum)
mesh_outbound_min:     4 peers
gossip_lazy:           12 peers
gossip_factor:         0.25
history_length:        5 messages
history_gossip:        3 messages
duplicate_cache_time:  60 seconds
```

### 6.3. Message Format

GossipSub messages are signed with the node's Ed25519 key:

```rust
struct GossipMessage {
    data: Vec<u8>,             // Serialized block or transaction
    signature: [u8; 64],       // Ed25519 signature
    sequence_number: u64,      // For ordering
    topic: String,             // Topic identifier
}
```

---

## 7. Block Header Format

```rust
struct BlockHeader {
    version: u32,              // Currently 2
    prev_hash: [u8; 32],       // Previous block hash
    merkle_root: [u8; 32],     // Transaction merkle root
    presence_root: [u8; 32],   // Presence commitment hash (ZERO in deterministic model)
    genesis_hash: [u8; 32],    // Chain identity fingerprint (v2+)
    timestamp: u64,            // Unix timestamp (seconds)
    slot: u32,                 // Slot number
    producer: [u8; 32],        // Producer public key
    vdf_output: VdfOutput,     // VDF computation result
    vdf_proof: VdfProof,       // Wesolowski proof
}
```

**Header size:** ~404 bytes (varies with VDF proof size)

**genesis_hash**: `BLAKE3(genesis_time || network_id || slot_duration || message)`. Ensures
blocks from nodes with different genesis parameters are rejected immediately.

---

## 8. Transaction Format

```rust
struct Transaction {
    version: u32,
    tx_type: TxType,           // See below
    inputs: Vec<Input>,
    outputs: Vec<Output>,
    data: Option<TxData>,      // Type-specific data
}

enum TxType {
    Transfer = 0,
    Registration = 1,
    Exit = 2,
    Coinbase = 3,
    ClaimReward = 4,
    ClaimBond = 5,
    AddBond = 6,
    RequestWithdrawal = 7,
    ClaimWithdrawal = 8,
    EpochReward = 9,
    SlashProducer = 10,
}

struct Input {
    prev_tx_hash: [u8; 32],
    output_index: u32,
    signature: [u8; 64],
}

struct Output {
    output_type: OutputType,   // Normal or Bond
    amount: u64,               // In base units
    pubkey_hash: [u8; 20],     // Recipient address
    lock_until: u64,           // Lock height (0 = unlocked)
}
```

---

## 9. VDF Format

```rust
struct VdfOutput {
    y: Vec<u8>,                // Class group element (variable size)
}

struct VdfProof {
    pi: Vec<u8>,               // Wesolowski proof (variable size)
}
```

**VDF Parameters:**
- Block VDF: 800K iterations (~55ms)
- Registration VDF base: 600M iterations (~10 minutes)
- Discriminant bits: 2048

---

## 10. Kademlia DHT

Peer discovery using Kademlia (`/doli/kad/1.0.0`).

**Parameters:**
- Replication factor: 20
- Query timeout: 60 seconds
- Storage: In-memory (peer addresses only)
- Mode: Server (responds to routing queries)

**Bootstrap:**
1. Node connects to bootstrap nodes
2. Performs Kademlia FIND_NODE for own ID
3. Populates routing table with discovered peers
4. Periodic refresh maintains connectivity

---

## 11. Peer Scoring

Peers are scored based on behavior:

| Behavior | Score Impact |
|----------|--------------|
| Valid block received | +10 |
| Valid transaction received | +1 |
| Invalid block received | -50 |
| Invalid transaction received | -10 |
| Timeout on request | -5 |
| Protocol violation | -100 |

Peers with score below threshold are disconnected.

---

## 12. Rate Limiting

Protection against DoS attacks:

| Resource | Limit |
|----------|-------|
| Blocks per peer per minute | 100 |
| Transactions per peer per minute | 1000 |
| Sync requests per peer per minute | 60 |
| Status requests per peer per minute | 10 |

---

## 13. Network Events

Events emitted by the network layer:

```rust
enum NetworkEvent {
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    NewBlock(Block),
    NewTransaction(Transaction),
    StatusRequest { peer_id, request, channel },
    SyncRequest { peer_id, request, channel },
    SyncResponse { peer_id, response },
    PeerStatus { peer_id, status },
    NetworkMismatch { peer_id, our_network_id, their_network_id },
    GenesisMismatch { peer_id },
}
```

---

## 14. Magic Bytes and Identifiers

| Identifier | Value |
|------------|-------|
| Protocol ID | `/doli/1.0.0` |
| Network ID (Mainnet) | 1 |
| Network ID (Testnet) | 2 |
| Network ID (Devnet) | 99 |
| User Agent | `doli-node/{version}` |

---

*Protocol version: 1.0.0*
