# rpc_reference.md - JSON-RPC API Documentation

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

**Note:** Currently only returns transactions in the mempool. Confirmed transaction
lookup requires a transaction index which is not yet implemented. For confirmed
transactions, use `getBlockByHash` or `getBlockByHeight` and search the transaction list.

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
| epochReward | Epoch reward distribution (deprecated) |
| claimEpochReward | Claim weighted presence rewards |
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
| spendable_only | boolean | Only return spendable UTXOs (default: false) |

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
    -d '{"jsonrpc":"2.0","method":"getUtxos","params":{"address":"0x...","spendable_only":true},"id":1}'
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
        "era": 0,
        "pendingWithdrawals": []
    }
]
```

**Note:** `pendingWithdrawals` is currently always empty (pending ProducerBonds integration).

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}'
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

### getHistory

Returns transaction history for an address.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| address | string | Address or pubkey hash (hex) |
| limit | integer | Maximum entries to return (default: 10) |

**Response:**
```json
[
    {
        "hash": "0x...",
        "type": "Transfer",
        "height": 12345,
        "status": "confirmed",
        "received": 100000000,
        "sent": 0,
        "fee": 1000
    }
]
```

**Note:** Fee calculation may be incomplete for some transaction types.

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getHistory","params":{"address":"0x...","limit":20},"id":1}'
```

---

### getNodeInfo

Returns information about the node.

**Parameters:** None

**Response:**
```json
{
    "version": "0.1.0",
    "network": "mainnet",
    "peerId": "12D3KooW...",
    "peerCount": 15,
    "platform": "linux",
    "arch": "x86_64"
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNodeInfo","params":{},"id":1}'
```

---

### getUpdateStatus

Returns the current auto-update status.

**Parameters:** None

**Response:**
```json
{
    "pendingUpdate": "0.2.0",
    "vetoPeriodActive": true,
    "vetoCount": 5,
    "vetoPercent": 12
}
```

Returns `null` for `pendingUpdate` if no update is pending.

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}'
```

---

### submitVote

Submit a veto vote for a pending update.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| version | string | Version to vote on |
| vote | integer | 0 = APPROVE, 1 = VETO |
| producer_id | string | Producer public key |
| timestamp | integer | Unix timestamp |
| signature | string | Signature over vote message |

**Response:**
```json
{
    "success": true
}
```

**Note:** Only active producers can submit votes.

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"submitVote","params":{"version":"0.2.0","vote":1,"producer_id":"0x...","timestamp":1706400000,"signature":"0x..."},"id":1}'
```

---

## 9. Rewards Methods

Methods for managing weighted presence rewards.

### getClaimableRewards

Returns unclaimed epoch rewards for a producer.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |

**Response:**
```json
{
    "epochs": [
        {
            "epoch": 5,
            "blocks_present": 358,
            "total_blocks": 360,
            "estimated_reward": 4750000000,
            "is_claimed": false
        }
    ],
    "total_claimable": 14175000000
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getClaimableRewards","params":{"producer_pubkey":"0x..."},"id":1}'
```

---

### getClaimHistory

Returns claim history for a producer.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| limit | number | Maximum entries to return (default: 10) |

**Response:**
```json
{
    "claims": [
        {
            "epoch": 4,
            "amount": 4600000000,
            "tx_hash": "0x...",
            "height": 1440,
            "timestamp": 1706500000
        }
    ],
    "total_claimed": 13625000000
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getClaimHistory","params":{"producer_pubkey":"0x...","limit":20},"id":1}'
```

---

### estimateEpochReward

Estimates reward for a specific epoch before claiming.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| epoch | number | Epoch number to estimate |

**Response:**
```json
{
    "epoch": 5,
    "blocks_present": 358,
    "total_blocks": 360,
    "total_producer_weight": 358000,
    "total_all_weights": 1790000,
    "block_reward": 100000000,
    "estimated_reward": 4750000000
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"estimateEpochReward","params":{"producer_pubkey":"0x...","epoch":5},"id":1}'
```

---

### buildClaimTx

Builds an unsigned claim transaction for signing.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| epoch | number | Epoch to claim |
| recipient | string | Optional recipient address (defaults to producer) |

**Response:**
```json
{
    "unsigned_tx": "0x...",
    "signing_message": "0x...",
    "epoch": 5,
    "amount": 4750000000,
    "recipient": "0x..."
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"buildClaimTx","params":{"producer_pubkey":"0x...","epoch":5},"id":1}'
```

---

### getEpochInfo

Returns current reward epoch information.

**Parameters:** None

**Response:**
```json
{
    "current_epoch": 8,
    "current_height": 2950,
    "blocks_per_epoch": 360,
    "epoch_start_height": 2880,
    "epoch_end_height": 3240,
    "epoch_progress": 70,
    "last_complete_epoch": 7
}
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getEpochInfo","params":{},"id":1}'
```

---

## 10. Error Codes

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
| -32007 | Epoch not complete | Epoch hasn't finished yet |
| -32008 | Already claimed | Epoch already claimed by producer |
| -32009 | No reward | Producer not present in epoch |

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

## 11. Units and Formatting

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

## 12. Rate Limiting

Default rate limits (configurable):

| Resource | Limit |
|----------|-------|
| Requests per second | 100 |
| Burst size | 200 |

Exceeded limits return HTTP 429 (Too Many Requests).

---

## 13. Security Considerations

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

## 14. WebSocket Support

WebSocket subscriptions are planned but not yet implemented.

Future subscription topics:
- `newBlocks` - New block notifications
- `pendingTransactions` - New mempool transactions
- `logs` - Event logs

---

*API version: 1.0*
