# DOLI JSON-RPC API Reference

The DOLI node exposes a JSON-RPC 2.0 API for querying blockchain state and submitting transactions.

## Endpoint

Default: `http://localhost:8545/`

## Request Format

```json
{
    "jsonrpc": "2.0",
    "method": "<method_name>",
    "params": { ... },
    "id": 1
}
```

## Response Format

Success:
```json
{
    "jsonrpc": "2.0",
    "result": { ... },
    "id": 1
}
```

Error:
```json
{
    "jsonrpc": "2.0",
    "error": {
        "code": -32600,
        "message": "Invalid Request"
    },
    "id": 1
}
```

**Note:** All JSON responses use camelCase field names (e.g., `bestHash`, `txCount`, `pubkeyHash`).

---

## Address Format

DOLI uses a 32-byte **pubkey_hash** for all RPC queries. This is the BLAKE3 hash of the public key:

```
pubkey_hash = BLAKE3(public_key_bytes)
```

The CLI wallet shows both formats:
- **Address (20-byte)**: Truncated hash for display (e.g., `doli1abc123...`)
- **Pubkey Hash (32-byte)**: Full 64-character hex string used for RPC queries

**Example:**
```
Public Key:      a9e3b8cc65373b24ea427114eea2d9333dde0876cb42863ccbadb4e9ee3b3c40
Pubkey Hash:     e76442ea0445e1abcb9782c83e77834b47dd586cfec76c2dc4566f5d21d71873
```

Use the **Pubkey Hash** (64 hex characters) for all `getBalance`, `getUtxos`, and `send` operations.

---

## Chain Methods

### getChainInfo

Get current chain state information.

**Parameters:** None

**Response:**
```json
{
    "network": "mainnet",
    "bestHash": "abc123...",
    "bestHeight": 12345,
    "bestSlot": 67890,
    "genesisHash": "def456..."
}
```

**Example:**
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

---

### getBlockByHash

Get a block by its hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| hash | string | Block hash (hex) |

**Response:**
```json
{
    "hash": "abc123...",
    "height": 12345,
    "slot": 67890,
    "prevHash": "def456...",
    "merkleRoot": "789abc...",
    "timestamp": 1700000000,
    "producer": "pubkey_hex",
    "txCount": 5,
    "transactions": ["tx_hash_1", "tx_hash_2", ...]
}
```

---

### getBlockByHeight

Get a block by its height.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| height | number | Block height |

**Response:** Same as `getBlockByHash`

---

## Transaction Methods

### getTransaction

Get a transaction by hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| hash | string | Transaction hash (hex) |

**Response:**
```json
{
    "hash": "tx_hash...",
    "version": 1,
    "txType": "transfer",
    "inputs": [
        {
            "prevTxHash": "prev_tx...",
            "outputIndex": 0,
            "signature": "sig_hex..."
        }
    ],
    "outputs": [
        {
            "outputType": "normal",
            "amount": 100000000,
            "pubkeyHash": "recipient...",
            "lockUntil": 0
        }
    ],
    "fee": 1000,
    "blockHash": "abc...",
    "confirmations": 6
}
```

---

### sendTransaction

Submit a signed transaction to the network.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| tx | string | Hex-encoded signed transaction |

**Response:** Transaction hash (string)

**Errors:**
| Code | Message | Description |
|------|---------|-------------|
| -32001 | Tx already known | Transaction exists in mempool |
| -32002 | Invalid transaction | Transaction validation failed |
| -32003 | Mempool full | Mempool at capacity |

---

## Balance Methods

### getBalance

Get balance for a pubkey hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| address | string | Pubkey hash (64-character hex, BLAKE3 hash of public key) |

**Response:**
```json
{
    "confirmed": 100000000,
    "unconfirmed": 5000000,
    "total": 105000000
}
```

Amounts are in base units (1 DOLI = 100,000,000 units).

---

### getUtxos

Get unspent transaction outputs for a pubkey hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| address | string | Pubkey hash (64-character hex, BLAKE3 hash of public key) |
| spendable_only | boolean | Only return spendable UTXOs (default: false) |

**Response:**
```json
[
    {
        "txHash": "abc...",
        "outputIndex": 0,
        "amount": 50000000,
        "outputType": "normal",
        "lockUntil": 0,
        "height": 1000,
        "spendable": true
    }
]
```

---

## Mempool Methods

### getMempoolInfo

Get mempool statistics.

**Parameters:** None

**Response:**
```json
{
    "txCount": 150,
    "totalSize": 45000,
    "minFeeRate": 1,
    "maxSize": 10485760,
    "maxCount": 5000
}
```

---

### getMempoolTransactions

Get transactions in mempool.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| limit | number | Maximum number to return |

**Response:** Array of transaction hashes

---

## Network Methods

### getNetworkInfo

Get network status.

**Parameters:** None

**Response:**
```json
{
    "peerId": "12D3Koo...",
    "peerCount": 8,
    "syncing": false,
    "syncProgress": null
}
```

---

### getPeers

Get connected peers.

**Parameters:** None

**Response:**
```json
[
    {
        "peerId": "12D3Koo...",
        "address": "/ip4/1.2.3.4/tcp/30303",
        "connectedSince": 1700000000,
        "bestHeight": 12345
    }
]
```

---

## Producer Methods

### getProducerSet

Get active producer set.

**Parameters:** None

**Response:**
```json
{
    "activeCount": 100,
    "totalBond": 100000000000000,
    "producers": [
        {
            "publicKey": "abc...",
            "bond": 100000000000,
            "blocksProduced": 42
        }
    ]
}
```

---

### getProducer

Get specific producer information.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| public_key | string | Producer public key (hex) |

**Response:**
```json
{
    "publicKey": "abc...",
    "registrationHeight": 1000,
    "bondAmount": 100000000000,
    "status": "active",
    "blocksProduced": 42,
    "era": 1
}
```

---

## Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid Request | Not a valid JSON-RPC request |
| -32601 | Method not found | Method doesn't exist |
| -32602 | Invalid params | Invalid method parameters |
| -32603 | Internal error | Internal server error |
| -32001 | Tx already known | Transaction in mempool |
| -32002 | Invalid transaction | Transaction validation failed |
| -32003 | Mempool full | Mempool capacity reached |
| -32004 | Block not found | Requested block not found |
| -32005 | Tx not found | Requested transaction not found |
| -32006 | Producer not found | Producer not registered |

---

## Rate Limits

Default limits per IP:
- 100 requests per second
- 10 MB bandwidth per second

Configure in node settings if needed.
