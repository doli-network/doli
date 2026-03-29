# protocol.md - Wire Protocol Specification

This document specifies the DOLI network protocol, message formats, and P2P communication standards.

---

## 1. Overview

DOLI uses libp2p as its networking foundation with multiple sub-protocols:

| Protocol | Path | Purpose |
|----------|------|---------|
| Status | `/doli/status/1.0.0` | Peer handshake and chain state |
| Sync | `/doli/sync/1.0.0` | Block synchronization |
| TxFetch | `/doli/txfetch/1.0.0` | Announce-request transaction fetching |
| Kademlia | `/doli/kad/1.0.0` | Peer discovery |
| GossipSub | 8 topics (see Section 6) | Message propagation |

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
    GetStateSnapshot {
        block_hash: [u8; 32],  // Recent finalized block hash
    },
    GetStateRoot {
        block_hash: [u8; 32],  // Block hash to compute state root for
    },
    GetHeadersByHeight {
        start_height: u64,     // Returns headers from height+1 onward
        max_count: u32,
    },
}
```

### 5.2. Response Types

```rust
enum SyncResponse {
    Headers(Vec<BlockHeader>),
    Bodies(Vec<Block>),
    Block(Option<Block>),
    StateSnapshot {
        block_hash: [u8; 32],
        block_height: u64,
        chain_state: Vec<u8>,   // Serialized ChainState (bincode)
        utxo_set: Vec<u8>,      // Serialized UtxoSet (bincode)
        producer_set: Vec<u8>,  // Serialized ProducerSet (bincode)
        state_root: [u8; 32],   // Merkle root: H(H(chain) || H(utxo) || H(producer))
    },
    StateRoot {
        block_hash: [u8; 32],
        block_height: u64,
        state_root: [u8; 32],
    },
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
| `/doli/producers/1` | Producer announcements (G-Set CRDT) | 64 KB |
| `/doli/votes/1` | Governance veto votes | 64 KB |
| `/doli/heartbeats/1` | Presence heartbeats (weighted rewards) | 64 KB |
| `/doli/headers/1` | Lightweight block headers (all tiers) | 64 KB |
| `/doli/t1/blocks/1` | Tier 1 blocks (validators only) | 1 MB |
| `/doli/attestations/1` | Attestation aggregates (Tier 1+2 finality) | 64 KB |

### 6.2. GossipSub Parameters

```
heartbeat_interval:    1 second
mesh_n:                12 peers (target)        # mainnet/devnet
mesh_n_low:            8 peers (minimum)        # mainnet/devnet
mesh_n_high:           24 peers (maximum)       # mainnet/devnet
mesh_outbound_min:     4 peers (dynamic: mesh_n/3, capped at mesh_n/2)
gossip_lazy:           12 peers                 # mainnet/devnet
gossip_factor:         0.50                 # INC-I-015: raised for faster non-mesh delivery
history_length:        5 messages
history_gossip:        3 messages
duplicate_cache_time:  60 seconds
```

**Note:** Testnet overrides mesh parameters for eager push to all connected peers
(mesh_n=25, mesh_n_low=20, mesh_n_high=50, gossip_lazy=25).

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
    vdf_output: VdfOutput,     // VDF computation result (hash-chain output)
    vdf_proof: VdfProof,       // VDF proof bytes (hash-chain, NOT Wesolowski in production)
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
    extra_data: Vec<u8>,       // Type-specific data (bincode-serialized)
}

enum TxType {
    Transfer = 0,
    Registration = 1,
    Exit = 2,
    ClaimReward = 3,
    ClaimBond = 4,
    SlashProducer = 5,
    Coinbase = 6,
    AddBond = 7,
    RequestWithdrawal = 8,
    ClaimWithdrawal = 9,      // Reserved tombstone — DO NOT REUSE
    EpochReward = 10,
    RemoveMaintainer = 11,
    AddMaintainer = 12,
    DelegateBond = 13,
    RevokeDelegation = 14,
    ProtocolActivation = 15,
    // 16 unused
    MintAsset = 17,
    BurnAsset = 18,
    CreatePool = 19,
    AddLiquidity = 20,
    RemoveLiquidity = 21,
    Swap = 22,
    // 23 unused
    CreateLoan = 24,
    RepayLoan = 25,
    LiquidateLoan = 26,
    LendingDeposit = 27,
    LendingWithdraw = 28,
}

struct Input {
    prev_tx_hash: [u8; 32],
    output_index: u32,
    signature: [u8; 64],
    sighash_type: SighashType,         // All(0) or AnyoneCanPay(1)
    committed_output_count: u32,       // 0 = all outputs, N > 0 = first N only
}

struct Output {
    output_type: OutputType,   // See OutputType enum below
    amount: u64,               // In base units
    pubkey_hash: [u8; 32],     // Recipient address (BLAKE3-256 hash of public key)
    lock_until: u64,           // Lock height (0 = unlocked)
}

enum OutputType {
    Normal = 0,          // Normal spendable output
    Bond = 1,            // Bond output (time-locked, protocol-governed withdrawal)
    Multisig = 2,        // Threshold-of-N signatures (also used for escrow)
    Hashlock = 3,        // Requires preimage reveal
    HTLC = 4,            // Hashlock + timelock OR expiry refund
    Vesting = 5,         // Signature + timelock
    NFT = 6,             // Non-fungible token with metadata + covenant conditions
    FungibleAsset = 7,   // Custom fungible token
    BridgeHTLC = 8,      // Cross-chain atomic swap with target chain metadata
    Pool = 9,            // AMM liquidity pool
    LPShare = 10,        // LP share token
    Collateral = 11,     // Lending collateral
    LendingDeposit = 12, // Lending pool deposit
}
```

---

## 9. VDF Format

DOLI uses an iterated BLAKE3 hash-chain VDF for block production (NOT the Wesolowski class group VDF, which exists in the codebase but is only used for telemetry presence).

```rust
struct VdfOutput {
    value: Vec<u8>,            // Final hash-chain output (32 bytes, BLAKE3)
}

struct VdfProof {
    pi: Vec<u8>,               // Empty for hash-chain VDF (verification = recomputation)
}
```

**VDF Parameters:**
- Consensus constant `T_BLOCK`: 800,000 iterations (~55ms on reference hardware)
- **Network default** `vdf_iterations`: 1,000 (mainnet/testnet via NetworkParams); devnet uses 1 iteration for fast development. Bond is the real Sybil protection; VDF provides anti-grinding at minimal cost
- Registration VDF: 1,000 iterations (all networks, essentially instant)
- VDF input: `BLAKE3(prefix || prev_hash || tx_root || slot || producer_key)`
- Verification: recompute the hash chain (linear time, same as computation)

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
| Invalid block received | -100 |
| Invalid transaction received | -20 |
| Timeout on request | -5 per timeout (max -50) |
| Spam detected | -50 |
| Duplicate message | -5 |
| Malformed message | -30 |

Score range: -1000 to +1000. Peers below disconnect threshold are dropped; peers below ban threshold are banned.

---

## 12. Rate Limiting

Protection against DoS attacks:

| Resource | Limit |
|----------|-------|
| Blocks per peer per minute | 10 |
| Transactions per peer per second | 50 |
| Requests per peer per second | 20 |
| Bandwidth per peer per second | 1 MB |

---

## 13. Network Events

Events emitted by the network layer:

```rust
enum NetworkEvent {
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    NewBlock(Block, PeerId),
    NewHeader(BlockHeader),
    NewTransaction(Transaction),
    StatusRequest { peer_id, request, channel },
    SyncRequest { peer_id, request, channel },
    SyncResponse { peer_id, response },
    PeerStatus { peer_id, status },
    NetworkMismatch { peer_id, our_network_id, their_network_id },
    GenesisMismatch { peer_id },
    ProducersAnnounced(Vec<PublicKey>),
    ProducerAnnouncementsReceived(Vec<ProducerAnnouncement>),
    ProducerDigestReceived { peer_id, digest },
    NewVote(Vec<u8>),
    NewHeartbeat(Vec<u8>),
    NewAttestation(Vec<u8>),
    TxAnnouncement { peer_id, hashes },
    TxFetchRequest { peer_id, hashes, channel },
    TxFetchResponse { peer_id, transactions },
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
