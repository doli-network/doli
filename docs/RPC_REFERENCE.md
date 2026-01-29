# RPC_REFERENCE.md - JSON-RPC API Documentation

This document describes the DOLI node JSON-RPC API.

---

## 1. Overview

| Property | Value |
|----------|-------|
| Protocol | JSON-RPC 2.0 |
| Transport | HTTP POST |
| Endpoint | `http://127.0.0.1:8545` (mainnet default) |
| Content-Type | `application/json` |

### Network Ports

| Network | RPC Port |
|---------|----------|
| Mainnet | 8545 |
| Testnet | 18545 |
| Devnet | 28545 |

---

## 2. Request Format

```json
{
    "jsonrpc": "2.0",
    "method": "methodName",
    "params": { ... },
    "id": 1
}
```

### Example with curl

```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

---

## 3. Chain Methods

### getChainInfo

Returns current chain state information.

**Parameters:** None

**Response:**
```json
{
    "network": "mainnet",
    "bestHash": "0x...",
    "bestHeight": 12345,
    "bestSlot": 45678,
    "genesisHash": "0x..."
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

---

### getBlockByHash

Returns block by its hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| hash | string | Block hash (hex, 0x-prefixed) |

**Response:**
```json
{
    "hash": "0x...",
    "prevHash": "0x...",
    "height": 12345,
    "slot": 45678,
    "timestamp": 1706400000,
    "producer": "0x...",
    "merkleRoot": "0x...",
    "txCount": 5,
    "transactions": ["0x...", "0x..."],
    "size": 1234
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHash","params":{"hash":"0xabc..."},"id":1}'
```

---

### getBlockByHeight

Returns block by its height.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| height | number | Block height |

**Response:** Same as `getBlockByHash`

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":12345},"id":1}'
```

---

## 4. Transaction Methods

### getTransaction

Returns transaction by its hash.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| hash | string | Transaction hash (hex) |

**Response:**
```json
{
    "hash": "0x...",
    "version": 1,
    "txType": "transfer",
    "inputs": [
        {
            "prevTxHash": "0x...",
            "outputIndex": 0,
            "signature": "0x..."
        }
    ],
    "outputs": [
        {
            "outputType": "normal",
            "amount": 100000000,
            "pubkeyHash": "0x...",
            "lockUntil": 0
        }
    ],
    "size": 256,
    "fee": 10000,
    "blockHash": "0x...",
    "confirmations": 6
}
```

**Transaction Types:**
| Type | Description |
|------|-------------|
| transfer | Standard value transfer |
| registration | Producer registration |
| exit | Producer exit |
| coinbase | Block reward |
| claimReward | Claim epoch rewards |
| claimBond | Claim bond after unbonding |
| addBond | Add bonds to producer |
| requestWithdrawal | Request bond withdrawal |
| claimWithdrawal | Claim after withdrawal delay |
| epochReward | Epoch reward distribution |
| slashProducer | Slash equivocating producer |

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getTransaction","params":{"hash":"0xabc..."},"id":1}'
```

---

### sendTransaction

Submits a signed transaction to the network.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| tx | string | Serialized transaction (hex) |

**Response:**
```json
"0x..." // Transaction hash
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"sendTransaction","params":{"tx":"0x..."},"id":1}'
```

---

## 5. Balance Methods

### getBalance

Returns balance for an address.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| address | string | Address or pubkey hash (hex) |

**Response:**
```json
{
    "confirmed": 100000000000,
    "unconfirmed": 5000000,
    "total": 100005000000
}
```

**Note:** Amounts are in base units (1 DOLI = 100,000,000 units)

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"0x..."},"id":1}'
```

---

### getUtxos

Returns unspent transaction outputs for an address.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| address | string | Address or pubkey hash (hex) |
| spendableOnly | boolean | Only return spendable UTXOs (default: false) |

**Response:**
```json
[
    {
        "txHash": "0x...",
        "outputIndex": 0,
        "amount": 100000000,
        "outputType": "normal",
        "lockUntil": 0,
        "height": 12345,
        "spendable": true
    },
    {
        "txHash": "0x...",
        "outputIndex": 1,
        "amount": 1000000000000,
        "outputType": "bond",
        "lockUntil": 15000000,
        "height": 12000,
        "spendable": false
    }
]
```

**Output Types:**
| Type | Description |
|------|-------------|
| normal | Standard spendable output |
| bond | Time-locked bond collateral |

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getUtxos","params":{"address":"0x...","spendableOnly":true},"id":1}'
```

---

## 6. Mempool Methods

### getMempoolInfo

Returns mempool statistics.

**Parameters:** None

**Response:**
```json
{
    "txCount": 42,
    "totalSize": 12345,
    "minFeeRate": 1000,
    "maxSize": 5242880,
    "maxCount": 5000
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}'
```

---

## 7. Network Methods

### getNetworkInfo

Returns network status information.

**Parameters:** None

**Response:**
```json
{
    "peerId": "12D3KooW...",
    "peerCount": 25,
    "syncing": false,
    "syncProgress": 100.0
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'
```

---

## 8. Producer Methods

### getProducerSet

Returns the active producer set.

**Parameters:** None

**Response:**
```json
{
    "epoch": 1234,
    "totalBonds": 50000,
    "producers": [
        {
            "pubkey": "0x...",
            "bondCount": 5,
            "weight": 3,
            "activeSince": 100000
        }
    ]
}
```

---

### getProducer

Returns information about a specific producer.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| public_key | string | Producer public key (hex) |

**Response:**
```json
{
    "publicKey": "0x...",
    "registrationHeight": 100000,
    "bondAmount": 5000000000000,
    "bondCount": 5,
    "status": "active",
    "blocksProduced": 1234,
    "pendingRewards": 500000000,
    "era": 0,
    "pendingWithdrawals": [
        {
            "bondCount": 2,
            "requestSlot": 45000,
            "netAmount": 1800000000000,
            "claimable": true
        }
    ]
}
```

**Status values:**
| Status | Description |
|--------|-------------|
| active | Producing blocks |
| unbonding | Exit requested, in unbonding period |
| exited | Completed exit |
| slashed | Slashed for misbehavior |

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducer","params":{"public_key":"0x..."},"id":1}'
```

---

### getProducers

Returns all producers in the network.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| active_only | boolean | Only return active producers (default: false) |

**Response:**
```json
[
    {
        "publicKey": "0x...",
        "registrationHeight": 100000,
        "bondAmount": 5000000000000,
        "bondCount": 5,
        "status": "active",
        "blocksProduced": 1234,
        "pendingRewards": 500000000,
        "era": 0,
        "pendingWithdrawals": []
    }
]
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}'
```

---

## 9. Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid request | Missing required fields |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Invalid parameters |
| -32603 | Internal error | Server error |
| -32000 | Block not found | Requested block doesn't exist |
| -32001 | Transaction not found | Requested tx doesn't exist |
| -32002 | Invalid transaction | Transaction validation failed |
| -32003 | Already in mempool | Transaction already submitted |
| -32004 | Mempool full | Mempool at capacity |
| -32005 | UTXO not found | Referenced UTXO doesn't exist |
| -32006 | Producer not found | Producer not in registry |

**Error Response Format:**
```json
{
    "jsonrpc": "2.0",
    "error": {
        "code": -32000,
        "message": "Block not found"
    },
    "id": 1
}
```

---

## 10. Units and Formatting

### Amount Units

| Unit | Base Units | Example |
|------|------------|---------|
| 1 DOLI | 100,000,000 | `100000000` |
| 0.1 DOLI | 10,000,000 | `10000000` |
| 0.00000001 DOLI | 1 | `1` |

### Hex Encoding

All binary data is hex-encoded with `0x` prefix:
- Hashes: 32 bytes → 66 characters (`0x` + 64 hex chars)
- Public keys: 32 bytes → 66 characters
- Addresses: 20 bytes → 42 characters
- Signatures: 64 bytes → 130 characters

---

## 11. Rate Limiting

Default rate limits (configurable):

| Resource | Limit |
|----------|-------|
| Requests per second | 100 |
| Burst size | 200 |

Exceeded limits return HTTP 429 (Too Many Requests).

---

## 12. Security Considerations

### Binding Address

By default, RPC binds to `127.0.0.1` (localhost only).

**To enable external access (NOT recommended for production):**
```toml
[rpc]
listen_addr = "0.0.0.0:8545"
```

### Authentication

No built-in authentication. Use:
- Firewall rules
- Reverse proxy with auth (nginx, caddy)
- VPN for remote access

### CORS

Cross-origin requests disabled by default. Enable in config if needed for web applications.

---

## 13. WebSocket Support

WebSocket subscriptions are planned but not yet implemented.

Future subscription topics:
- `newBlocks` - New block notifications
- `pendingTransactions` - New mempool transactions
- `logs` - Event logs

---

*API version: 1.0*
