# rpc_reference.md - JSON-RPC API Documentation

This document describes the DOLI node JSON-RPC API.

---

## Method Summary

| Category | Method | Status |
|----------|--------|--------|
| **Chain** | `getChainInfo` | Implemented |
| **Chain** | `getBlockByHash` | Implemented |
| **Chain** | `getBlockByHeight` | Implemented |
| **Transaction** | `sendTransaction` | Implemented |
| **Transaction** | `getTransaction` | Implemented (mempool only) |
| **Balance** | `getBalance` | Implemented |
| **Balance** | `getUtxos` | Implemented |
| **Balance** | `getHistory` | Implemented |
| **Mempool** | `getMempoolInfo` | Implemented |
| **Network** | `getNetworkInfo` | Implemented |
| **Network** | `getNodeInfo` | Implemented |
| **Producer** | `getProducer` | Implemented |
| **Producer** | `getProducers` | Implemented |
| **Governance** | `getUpdateStatus` | Implemented |
| **Governance** | `submitVote` | Implemented |
| **Governance** | `getMaintainerSet` | Implemented |
| **Governance** | `submitMaintainerChange` | Implemented |
| **Epoch** | `getEpochInfo` | Implemented |
| **Producer** | `getBondDetails` | Implemented |
| **Network** | `getNetworkParams` | Implemented |

### Not Yet Implemented

The following methods are **NOT YET IMPLEMENTED** and will return "Method not found" errors:

| Method | Description |
|--------|-------------|
| `getBlockHeader` | Return header only (use `getBlockByHash` instead) |
| `getTransactionReceipt` | Transaction receipt with logs |
| `getUtxosByOutpoint` | Lookup specific UTXOs by outpoint |
| `getSchedule` | Producer schedule for upcoming slots |
| `getPeerInfo` | Detailed peer list (use `getNetworkInfo` instead) |
| `getRawMempool` | List all mempool transaction hashes |
| `validateAddress` | Validate address format |
| `estimateFee` | Estimate transaction fee |

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
| claim_reward | **DEPRECATED** - Claim epoch rewards |
| claim_bond | Claim bond after unbonding |
| add_bond | Add bonds to producer |
| request_withdrawal | Request bond withdrawal |
| claim_withdrawal | Claim after withdrawal delay |
| epoch_reward | **DEPRECATED** - Epoch reward distribution |
| slash_producer | Slash equivocating producer |
| add_maintainer | Add maintainer (3/5 multisig) |
| remove_maintainer | Remove maintainer (3/5 multisig) |

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
    "immature": 100000000,
    "total": 100105000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| confirmed | Spendable balance (mature, confirmed) |
| unconfirmed | Pending credits from mempool |
| immature | Coinbase/epoch rewards awaiting maturity (100 blocks) |
| total | confirmed + unconfirmed + immature |

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

### getBondDetails

Returns bond vesting details for a producer, including penalty percentages and maturation info.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| public_key | string | Producer public key (hex) |

**Response:**
```json
{
    "publicKey": "0x...",
    "bondCount": 10,
    "totalStaked": 10000000000,
    "registrationSlot": 5000,
    "ageSlots": 3000,
    "penaltyPct": 50,
    "vested": false,
    "maturationSlot": 13640,
    "vestingQuarterSlots": 2160,
    "vestingPeriodSlots": 8640,
    "summary": {
        "q1": 0,
        "q2": 10,
        "q3": 0,
        "vested": 0
    },
    "pendingWithdrawals": []
}
```

**Vesting quarters:**
| Quarter | Bond Age | Penalty |
|---------|----------|---------|
| Q1 | 0-6h | 75% |
| Q2 | 6-12h | 50% |
| Q3 | 12-18h | 25% |
| Q4+ | 18h+ | 0% |

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBondDetails","params":{"public_key":"0x..."},"id":1}'
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
        "txType": "transfer",
        "blockHash": "0x...",
        "height": 12345,
        "timestamp": 1706400000,
        "amountReceived": 100000000,
        "amountSent": 0,
        "fee": 0,
        "confirmations": 6
    }
]
```

**Fields:**
| Field | Description |
|-------|-------------|
| hash | Transaction hash |
| txType | Transaction type (transfer, coinbase, etc.) |
| blockHash | Block containing this transaction |
| height | Block height |
| timestamp | Block timestamp |
| amountReceived | Amount received by this address |
| amountSent | Amount sent from this address |
| fee | Transaction fee (may be 0 if not calculable) |
| confirmations | Number of confirmations |

**Note:** Fee calculation may be incomplete for some transaction types. History scans up to 1000 recent blocks.

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
    "pending_update": null,
    "veto_period_active": false,
    "veto_count": 0,
    "veto_percent": 0,
    "message": "Update status tracking not yet integrated with RPC"
}
```

**Note:** This is currently a placeholder implementation. Full update status tracking is not yet integrated with the RPC layer. Returns `null` for `pending_update` if no update is pending.

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
| vote | object | Vote message object (see below) |

**Vote Message Object:**
| Field | Type | Description |
|-------|------|-------------|
| version | string | Version to vote on |
| vote | string | "approve" or "veto" |
| producerId | string | Producer public key (hex) |
| timestamp | integer | Unix timestamp |
| signature | string | Signature over "version:vote:timestamp" (hex) |

**Response:**
```json
{
    "status": "submitted",
    "message": "Vote submitted and broadcast to network"
}
```

**Note:** Only active producers can submit votes.

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"submitVote","params":{"vote":{"version":"0.2.0","vote":"veto","producerId":"0x...","timestamp":1706400000,"signature":"0x..."}},"id":1}'
```

---

### getMaintainerSet

Returns the current maintainer set. Since v1.1.15, reads from the persisted `MaintainerState` (bootstrapped from the first 5 registered producers, then governed via on-chain `MaintainerAdd`/`MaintainerRemove` transactions). Falls back to ad-hoc derivation if `MaintainerState` is not yet available.

**Parameters:** None

**Response:**
```json
{
    "maintainers": [
        {
            "pubkey": "0x...",
            "registered_at_block": 100,
            "is_active_producer": true
        }
    ],
    "threshold": 3,
    "member_count": 5,
    "max_maintainers": 5,
    "min_maintainers": 3,
    "initial_maintainer_count": 5,
    "last_change_block": 500,
    "source": "on-chain"
}
```

**Source values:**
| Value | Description |
|-------|-------------|
| `on-chain` | Read from persisted `MaintainerState` (bootstrapped or governed) |
| `derived` | Fallback: ad-hoc derivation from producer registry (pre-v1.1.15 behavior) |
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMaintainerSet","params":{},"id":1}'
```

---

### submitMaintainerChange

Submit a maintainer add or remove transaction (requires 3/5 multisig).

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| action | string | "add" or "remove" |
| target_pubkey | string | Public key to add/remove (hex) |
| signatures | array | Array of signature entries |
| reason | string | (Optional) Reason for removal |

**Signature Entry:**
```json
{
    "pubkey": "0x...",
    "signature": "0x..."
}
```

**Response:**
```json
{
    "status": "accepted",
    "tx_hash": "0x...",
    "message": "Maintainer add transaction submitted"
}
```

**Note:** Requires at least 3 valid signatures from current maintainers.

**Example:**
```bash
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"submitMaintainerChange","params":{"action":"add","target_pubkey":"0x...","signatures":[{"pubkey":"0x...","signature":"0x..."}]},"id":1}'
```

---

## 9. Rewards Methods

Block rewards in DOLI work like Bitcoin: producers receive rewards automatically
when they produce a block via the coinbase transaction. **No claiming is needed.**

Per WHITEPAPER.md Section 9.1:
- Initial reward: 1 DOLI/block
- Reward maturity: 100 confirmations (Section 9.2)
- Halving interval: 12,614,400 blocks (~4 years)

### Deprecated Methods

The following RPC methods are **NOT IMPLEMENTED** and will return errors:

| Method | Status |
|--------|--------|
| `getClaimableRewards` | Not implemented - rewards are automatic |
| `getClaimHistory` | Not implemented - no claiming |
| `estimateEpochReward` | Not implemented - rewards are automatic |
| `buildClaimTx` | Not implemented - no claim transactions |

These methods were documented for a weighted presence reward system that was
deprecated in favor of the simpler Bitcoin-like coinbase model where 100% of
block rewards go directly to producers.

---

### getEpochInfo

Returns current reward epoch information.

**Parameters:** None

**Response:**
```json
{
    "currentHeight": 2950,
    "currentEpoch": 8,
    "lastCompleteEpoch": 7,
    "blocksPerEpoch": 360,
    "blocksRemaining": 290,
    "epochStartHeight": 2880,
    "epochEndHeight": 3240,
    "blockReward": 100000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| currentHeight | Current blockchain height |
| currentEpoch | Current reward epoch number |
| lastCompleteEpoch | Most recently completed epoch (null if epoch 0) |
| blocksPerEpoch | Blocks per epoch (360 mainnet/testnet, 60 devnet) |
| blocksRemaining | Blocks until current epoch ends |
| epochStartHeight | First block height of current epoch |
| epochEndHeight | Last block height of current epoch (exclusive) |
| blockReward | Current block reward in base units |

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
