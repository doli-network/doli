# rpc_reference.md - JSON-RPC API Documentation

This document describes the DOLI node JSON-RPC API.

---

## Method Summary

| Category | Method | Status |
|----------|--------|--------|
| **Chain** | `getChainInfo` | Implemented |
| **Chain** | `getBlockByHash` | Implemented |
| **Chain** | `getBlockByHeight` | Implemented |
| **Chain** | `getChainStats` | Implemented |
| **Transaction** | `sendTransaction` | Implemented |
| **Transaction** | `getTransaction` | Implemented (mempool only) |
| **Balance** | `getBalance` | Implemented |
| **Balance** | `getUtxos` | Implemented |
| **Balance** | `getHistory` | Implemented |
| **Mempool** | `getMempoolInfo` | Implemented |
| **Mempool** | `getMempoolTransactions` | Implemented |
| **Network** | `getNetworkInfo` | Implemented |
| **Network** | `getNodeInfo` | Implemented |
| **Network** | `getNetworkParams` | Implemented |
| **Producer** | `getProducer` | Implemented |
| **Producer** | `getProducers` | Implemented |
| **Producer** | `getBondDetails` | Implemented |
| **Scheduling** | `getSlotSchedule` | Implemented |
| **Scheduling** | `getProducerSchedule` | Implemented |
| **Scheduling** | `getAttestationStats` | Implemented |
| **Governance** | `getUpdateStatus` | Implemented |
| **Governance** | `submitVote` | Implemented |
| **Governance** | `getMaintainerSet` | Implemented |
| **Governance** | `submitMaintainerChange` | Implemented |
| **Epoch** | `getEpochInfo` | Implemented |
| **Archive** | `getBlockRaw` | Implemented |
| **Archive** | `backfillFromPeer` | Implemented |
| **Archive** | `backfillStatus` | Implemented |
| **Archive** | `verifyChainIntegrity` | Implemented |
| **Debugging** | `getStateRootDebug` | Implemented |
| **Debugging** | `getUtxoDiff` | Implemented |
| **Snapshot** | `getStateSnapshot` | Implemented |
| **Network** | `getPeerInfo` | Implemented |
| **Pool** | `getPoolInfo` | Implemented |
| **Pool** | `getPoolList` | Implemented |
| **Pool** | `getPoolPrice` | Implemented |
| **Pool** | `getSwapQuote` | Implemented |
| **Lending** | `getLoanInfo` | Implemented |
| **Lending** | `getLoanList` | Implemented |

### Not Yet Implemented

The following methods are **NOT YET IMPLEMENTED** and will return "Method not found" errors:

| Method | Description |
|--------|-------------|
| `getBlockHeader` | Return header only (use `getBlockByHash` instead) |
| `getTransactionReceipt` | Transaction receipt with logs |
| `getUtxosByOutpoint` | Lookup specific UTXOs by outpoint |
| `validateAddress` | Validate address format |
| `estimateFee` | Estimate transaction fee |

---

## 1. Overview

| Property | Value |
|----------|-------|
| Protocol | JSON-RPC 2.0 |
| Transport | HTTP POST |
| Endpoint | `http://127.0.0.1:8500` (mainnet default) |
| Content-Type | `application/json` |

### Network Ports

| Network | RPC Port |
|---------|----------|
| Mainnet | 8500 |
| Testnet | 18500 |
| Devnet | 28500 |

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
curl -X POST http://127.0.0.1:8500 \
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
    "version": "1.1.11",
    "bestHash": "0x...",
    "bestHeight": 12345,
    "bestSlot": 45678,
    "genesisHash": "0x...",
    "rewardPoolBalance": 500000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| network | Network name (mainnet, testnet, devnet) |
| version | Node software version (e.g. "1.1.11") |
| bestHash | Best block hash |
| bestHeight | Best block height |
| bestSlot | Best block slot number |
| genesisHash | Genesis block hash |
| rewardPoolBalance | Reward pool balance in base units (sum of coinbase UTXOs held by pool) |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
| transfer (0) | Standard value transfer |
| registration (1) | Producer registration |
| exit (2) | Producer exit |
| claim_reward (3) | **DEPRECATED** - Claim epoch rewards |
| claim_bond (4) | Claim bond after unbonding |
| slash_producer (5) | Slash equivocating producer |
| coinbase (6) | Block reward (enum exists but coinbase uses Transfer) |
| add_bond (7) | Add bonds to producer |
| request_withdrawal (8) | Request bond withdrawal (instant with vesting penalty) |
| claim_withdrawal (9) | Reserved tombstone (wire compat) |
| epoch_reward (10) | Epoch reward distribution (active — pool drained bond-weighted at epoch boundary) |
| remove_maintainer (11) | Remove maintainer (3/5 multisig) |
| add_maintainer (12) | Add maintainer (3/5 multisig) |
| delegate_bond (13) | Delegate bond weight to another producer |
| revoke_delegation (14) | Revoke delegated bonds |
| protocol_activation (15) | On-chain consensus rule activation (3/5 multisig) |
| mint_asset (17) | Mint fungible asset (issuer-only) |
| burn_asset (18) | Burn fungible asset |
| create_pool (19) | Create AMM pool with initial liquidity |
| add_liquidity (20) | Add liquidity to AMM pool |
| remove_liquidity (21) | Remove liquidity from AMM pool |
| swap (22) | Swap assets through AMM pool |
| create_loan (24) | Create collateralized loan |
| repay_loan (25) | Repay loan and recover collateral |
| liquidate_loan (26) | Liquidate undercollateralized loan |
| lending_deposit (27) | Deposit DOLI into lending pool |
| lending_withdraw (28) | Withdraw DOLI + interest from lending pool |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
    "bonded": 50000000000,
    "total": 150105000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| confirmed | Spendable balance (mature, confirmed, minus mempool-spent) |
| unconfirmed | Pending credits from mempool (incoming change outputs) |
| immature | Coinbase/epoch rewards awaiting maturity (6 blocks) |
| bonded | Balance locked in Bond UTXOs (not spendable directly) |
| total | confirmed + unconfirmed + immature + bonded |

**Note:** Amounts are in base units (1 DOLI = 100,000,000 units)

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
    },
    {
        "txHash": "0x...",
        "outputIndex": 0,
        "amount": 0,
        "outputType": "nft",
        "lockUntil": 0,
        "height": 12500,
        "spendable": true,
        "nft": {
            "tokenId": "0x...",
            "contentHash": "0x...",
            "royalty": { "creator": "0x...", "bps": 500, "percent": "5.00" }
        }
    },
    {
        "txHash": "0x...",
        "outputIndex": 0,
        "amount": 100000000,
        "outputType": "normal",
        "lockUntil": 0,
        "height": 0,
        "spendable": true,
        "pending": true
    }
]
```

**Additional fields (present when applicable):**
| Field | Description |
|-------|-------------|
| pending | `true` if from a mempool transaction (not yet confirmed). Omitted when `false`. |
| condition | Decoded covenant condition object (only for conditioned output types) |
| nft | NFT metadata: `tokenId`, `contentHash`, optional `royalty` (only for NFT outputs) |
| asset | Fungible asset metadata: `assetId`, `totalSupply`, `ticker` (only for FungibleAsset outputs) |
| bridge | Bridge HTLC metadata: `targetChain`, `targetChainId`, `targetAddress`, optional `counterHash` (only for BridgeHTLC outputs) |

**Output Types:**
| Type | Description |
|------|-------------|
| normal | Standard spendable output |
| bond | Time-locked bond collateral |
| multisig | Multi-signature output |
| hashlock | Hash-locked output |
| htlc | Hash time-locked contract |
| vesting | Vesting schedule output |
| nft | Non-fungible token |
| fungibleAsset | Fungible token (issued via `issue-token`) |
| bridgeHtlc | Cross-chain bridge HTLC |
| pool | AMM pool state output |
| lpShare | Liquidity provider share |
| collateral | Lending collateral |
| lendingDeposit | Lending pool deposit |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}'
```

---

### getMempoolTransactions

Returns pending transactions from the mempool, sorted by fee rate (highest first).

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| limit | integer | Maximum transactions to return (default: 100, max: 500) |

**Response:**
```json
[
    {
        "hash": "0x...",
        "txType": "transfer",
        "size": 256,
        "fee": 10000,
        "feeRate": 39,
        "addedTime": 1706400000
    }
]
```

**Fields:**
| Field | Description |
|-------|-------------|
| hash | Transaction hash (hex) |
| txType | Transaction type (transfer, registration, etc.) |
| size | Transaction size in bytes |
| fee | Transaction fee in base units |
| feeRate | Fee per byte |
| addedTime | Unix timestamp when transaction entered the mempool |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolTransactions","params":{"limit":50},"id":1}'
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
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'
```

---

### getPeerInfo

Returns detailed information about all connected peers.

**Parameters:** None

**Response:**
```json
[
    {
        "peerId": "12D3KooW...",
        "address": "/ip4/1.2.3.4/tcp/30300",
        "bestHeight": 12345,
        "connectedSecs": 3600,
        "lastSeenSecs": 2,
        "latencyMs": 45
    }
]
```

**Fields:**
| Field | Description |
|-------|-------------|
| peerId | libp2p peer ID |
| address | Remote multiaddr |
| bestHeight | Best known height reported by this peer |
| connectedSecs | Connection duration in seconds |
| lastSeenSecs | Seconds since last message from this peer |
| latencyMs | Latency in milliseconds (null if unknown) |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getPeerInfo","params":{},"id":1}'
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
        "bondCount": 5,
        "status": "active",
        "era": 0
    }
]
```

**Note:** `bondCount` is derived from the UTXO set (count of Bond UTXOs for the producer's pubkey_hash). It reflects the current live count, not the epoch snapshot used for scheduling.

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
    "bondCount": 5,
    "status": "active",
    "era": 0
}
```

**Note:** `bondCount` is derived from the UTXO set. `RequestWithdrawal` (TxType 8)
processes instantly with FIFO vesting penalty (per-bond quarter-based).
`ClaimWithdrawal` (TxType 9) is reserved/unused (tombstone for wire compat).

**Status values:**
| Status | Description |
|--------|-------------|
| active | Producing blocks |
| exited | Completed exit |
| slashed | Slashed for misbehavior |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducer","params":{"public_key":"0x..."},"id":1}'
```

---

### getBondDetails

Returns per-bond vesting details derived from Bond UTXOs in the UTXO set.
Each Bond UTXO carries its `creation_slot` in `extra_data` (4 bytes LE).

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
    "vestingQuarterSlots": 2160,
    "vestingPeriodSlots": 8640,
    "bonds": [
        {
            "outpoint": "txhash:index",
            "creationSlot": 500,
            "ageSlots": 3000,
            "penaltyPct": 50,
            "vested": false,
            "quarter": "Q2"
        }
    ],
    "summary": {
        "q1": 0,
        "q2": 10,
        "q3": 0,
        "vested": 0
    }
}
```

**Data source:** Bond details are read directly from Bond UTXOs (output_type=1)
owned by the producer. `creationSlot` is decoded from the Bond UTXO's `extra_data`
field. No separate bond registry is consulted.

**Vesting quarters:**
| Quarter | Bond Age | Penalty |
|---------|----------|---------|
| Q1 | 0-6h | 75% |
| Q2 | 6-12h | 50% |
| Q3 | 12-18h | 25% |
| Q4+ | 18h+ | 0% |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNodeInfo","params":{},"id":1}'
```

---

### getNetworkParams

Returns the consensus and network parameters for the node's active network.

**Parameters:** None

**Response:**
```json
{
    "network": "mainnet",
    "bondUnit": 1000000000000,
    "slotDuration": 10,
    "slotsPerEpoch": 360,
    "blocksPerRewardEpoch": 360,
    "coinbaseMaturity": 100,
    "initialReward": 100000000,
    "genesisTime": 1706400000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| network | Network name (mainnet, testnet, devnet) |
| bondUnit | Bond size in base units (1 bond = this amount) |
| slotDuration | Slot duration in seconds |
| slotsPerEpoch | Number of slots per epoch |
| blocksPerRewardEpoch | Number of blocks per reward epoch |
| coinbaseMaturity | Blocks before coinbase outputs are spendable |
| initialReward | Initial block reward in base units |
| genesisTime | Genesis timestamp (unix) |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkParams","params":{},"id":1}'
```

---

### getChainStats

Returns aggregate chain statistics including supply, UTXO count, and staking info.

**Parameters:** None

**Response:**
```json
{
    "totalSupply": 123456789000000,
    "addressCount": 42,
    "utxoCount": 1500,
    "activeProducers": 5,
    "totalStaked": 50000000000000,
    "height": 12345,
    "rewardPoolBalance": 500000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| totalSupply | Total UTXO supply in base units |
| addressCount | Number of unique addresses with UTXOs |
| utxoCount | Total number of unspent outputs |
| activeProducers | Number of active producers |
| totalStaked | Total bonds staked in base units |
| height | Current chain height |
| rewardPoolBalance | Reward pool balance in base units (sum of coinbase UTXOs held by pool) |

**Note:** `totalSupply` is derived from summing all UTXO amounts. Divide by 1e8 for DOLI.

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainStats","params":{},"id":1}'
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
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
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
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"submitMaintainerChange","params":{"action":"add","target_pubkey":"0x...","signatures":[{"pubkey":"0x...","signature":"0x..."}]},"id":1}'
```

---

## 9. Rewards Methods

DOLI uses a **pooled epoch distribution** model. Every block's coinbase goes to a
deterministic reward pool address (no private key). At each epoch boundary, the pool
is drained and distributed bond-weighted to attestation-qualified producers via an
EpochReward transaction (TxType 10). **No manual claiming is needed.**

- Initial reward: 1 DOLI/block (to pool)
- Epoch distribution: every 360 blocks (mainnet), 36 blocks (testnet), 4 blocks (devnet)
- Reward maturity: 6 confirmations
- Halving interval: 12,614,400 blocks (~4 years)

### Deprecated Methods

The following RPC methods are **NOT IMPLEMENTED** and will return errors:

| Method | Status |
|--------|--------|
| `getClaimableRewards` | Not implemented - rewards distributed automatically at epoch boundary |
| `getClaimHistory` | Not implemented - no manual claiming |
| `estimateEpochReward` | Not implemented - use getEpochInfo instead |
| `buildClaimTx` | Not implemented - no claim transactions |

These methods were documented for a manual claim model. The active system
distributes rewards automatically via EpochReward (TxType 10) at each epoch
boundary, bond-weighted among attestation-qualified producers.

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
| blocksPerEpoch | Blocks per epoch (360 mainnet, 36 testnet, 4 devnet) |
| blocksRemaining | Blocks until current epoch ends |
| epochStartHeight | First block height of current epoch |
| epochEndHeight | Last block height of current epoch (exclusive) |
| blockReward | Current block reward in base units |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getEpochInfo","params":{},"id":1}'
```

---

## 10. Scheduling Methods

### getSlotSchedule

Returns the producer schedule for upcoming slots based on the current producer set and bond weights.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| fromSlot | integer | Starting slot (default: current slot) |
| count | integer | Number of slots to return (default: 20, max: 360) |

**Response:**
```json
{
    "slots": [
        {
            "slot": 45678,
            "producer": "0x...",
            "rank": 0
        },
        {
            "slot": 45679,
            "producer": "0x...",
            "rank": 0
        }
    ],
    "currentSlot": 45678,
    "epoch": 12,
    "slotsRemainingInEpoch": 282,
    "totalBonds": 50,
    "slotDuration": 10,
    "genesisTime": 1706400000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| slots | Array of slot assignments |
| slots[].slot | Slot number |
| slots[].producer | Assigned producer public key (hex) |
| slots[].rank | Rank (0 = primary producer) |
| currentSlot | Current chain slot |
| epoch | Current epoch number |
| slotsRemainingInEpoch | Slots left in the current epoch |
| totalBonds | Total bond count across all active producers |
| slotDuration | Slot duration in seconds |
| genesisTime | Genesis timestamp (unix) |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getSlotSchedule","params":{"fromSlot":45678,"count":10},"id":1}'
```

---

### getProducerSchedule

Returns schedule and performance information for a specific producer in the current epoch, including assigned slots, fill rate, and economics.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| publicKey | string | Producer public key (hex) |

**Response:**
```json
{
    "publicKey": "0x...",
    "currentSlot": 45678,
    "epoch": 12,
    "nextSlot": 45690,
    "secondsUntilNext": 120,
    "slotsThisEpoch": [45600, 45620, 45640, 45690, 45710],
    "assignedCount": 5,
    "producedCount": 3,
    "fillRate": 1.0,
    "bondCount": 10,
    "totalNetworkBonds": 50,
    "weeklyEarnings": 12096000000,
    "doublingWeeks": 82.67,
    "blockReward": 100000000
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| publicKey | Producer public key (hex) |
| currentSlot | Current chain slot |
| epoch | Current epoch number |
| nextSlot | Next slot where this producer is primary (null if none remaining in epoch) |
| secondsUntilNext | Seconds until the next assigned slot (null if none remaining) |
| slotsThisEpoch | Array of all slot numbers assigned to this producer in the current epoch |
| assignedCount | Total assigned slots this epoch |
| producedCount | Number of blocks actually produced this epoch |
| fillRate | Ratio of produced/assigned for past slots (0.0-1.0) |
| bondCount | Producer's effective bond count (minimum 1) |
| totalNetworkBonds | Total bonds across all active producers |
| weeklyEarnings | Estimated weekly earnings in base units |
| doublingWeeks | Estimated weeks until bond investment doubles from rewards |
| blockReward | Current block reward in base units |

**Note:** `fillRate` only considers past assigned slots (slots <= current slot). Future slots are excluded from the calculation. `doublingWeeks` is `Infinity` if weekly earnings are zero.

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducerSchedule","params":{"publicKey":"0x..."},"id":1}'
```

---

### getAttestationStats

Returns attestation statistics for the current epoch. Scans all blocks in the current epoch, decodes presence_root bitfields, and reports per-producer attestation minute counts. Used to determine which producers qualify for epoch rewards.

**Parameters:** None

**Response:**
```json
{
    "epoch": 12,
    "epochStart": 4320,
    "currentHeight": 4500,
    "blocksInEpoch": 181,
    "blocksWithAttestations": 170,
    "blocksWithBls": 165,
    "currentMinute": 30,
    "producers": [
        {
            "publicKey": "0x...",
            "attestedMinutes": 28,
            "totalMinutes": 31,
            "threshold": 20,
            "qualified": true,
            "hasBls": true
        }
    ]
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| epoch | Current epoch number |
| epochStart | First block height of the current epoch |
| currentHeight | Current chain height |
| blocksInEpoch | Number of blocks produced so far in this epoch |
| blocksWithAttestations | Blocks containing presence_root attestation data |
| blocksWithBls | Blocks containing aggregate BLS signatures |
| currentMinute | Current attestation minute within the epoch |
| producers | Per-producer attestation breakdown |
| producers[].publicKey | Producer public key (hex) |
| producers[].attestedMinutes | Number of distinct minutes this producer has attested |
| producers[].totalMinutes | Total minutes elapsed in the epoch |
| producers[].threshold | Minimum attested minutes required for reward qualification |
| producers[].qualified | Whether the producer meets the attestation threshold |
| producers[].hasBls | Whether the producer has a registered BLS key |

**Note:** Producers are sorted by public key bytes (same order as the attestation bitfield). A producer without a BLS key (`hasBls: false`) cannot sign attestations and will not qualify for epoch rewards.

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getAttestationStats","params":{},"id":1}'
```

---

## 11. Debugging Methods

These methods are intended for node operators diagnosing state divergence and snap sync failures. They expose internal state details that are not needed for normal operation.

### getStateRootDebug

Returns the per-component state root hashes. Compare these across nodes at the same height to identify which state component (ChainState, UTXO set, or ProducerSet) has diverged.

**Parameters:** None

**Response:**
```json
{
    "height": 12345,
    "bestHash": "0x...",
    "stateRoot": "0x...",
    "csHash": "0x...",
    "utxoHash": "0x...",
    "psHash": "0x...",
    "utxoCount": 1500,
    "producerCount": 5,
    "totalMinted": 0,
    "registrationSeq": 5
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| height | Current chain height |
| bestHash | Best block hash |
| stateRoot | Combined state root: `H(H(chain_state) \|\| H(utxo_set) \|\| H(producer_set))` |
| csHash | Hash of the canonical ChainState serialization |
| utxoHash | Hash of the canonical UTXO set serialization |
| psHash | Hash of the canonical ProducerSet serialization |
| utxoCount | Number of UTXOs in the set |
| producerCount | Number of active producers |
| totalMinted | Total minted (always 0 -- dead code, not used) |
| registrationSeq | Registration sequence counter from ChainState |

**Diagnosis workflow:**
1. Call `getStateRootDebug` on all nodes at the same height
2. Compare `stateRoot` -- if they match, state is consistent
3. If they differ, compare `csHash`, `utxoHash`, `psHash` to find which component diverges
4. If `utxoHash` differs, use `getUtxoDiff` to find the exact divergent UTXOs

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}'
```

---

### getUtxoDiff

Returns per-UTXO canonical hashes for diffing across nodes. Supports two modes: full dump (no params) and differential mode (with reference hashes from another node).

**Important:** Only works with the in-memory UTXO set. Returns an error for RocksDb-backed UTXO sets.

**Parameters (full dump mode):** None or `{}`

**Parameters (diff mode):**
| Name | Type | Description |
|------|------|-------------|
| referenceHashes | array of strings | Entry hashes from a reference node; only differing entries are returned |

**Response (full dump mode):**
```json
{
    "height": 12345,
    "count": 1500,
    "entries": [
        {
            "outpoint": "abcd1234...0000",
            "hash": "0x...",
            "detail": "amt=100000000 h=100 type=0 cb=1 er=0 lock=0 ed= pk=abcdef0123456789"
        }
    ]
}
```

**Response (diff mode):**
```json
{
    "height": 12345,
    "totalEntries": 1500,
    "diffCount": 2,
    "diffs": [
        {
            "outpoint": "abcd1234...0000",
            "hash": "0x...",
            "detail": "amt=100000000 h=100 type=1 cb=0 er=0 lock=4294967295 ed=e8030000 pk=abcdef0123456789"
        }
    ]
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| height | Chain height at time of query |
| count | Total UTXO entries (full dump mode) |
| totalEntries | Total UTXO entries (diff mode) |
| diffCount | Number of differing entries (diff mode) |
| entries / diffs | Array of UTXO entries |
| [].outpoint | Outpoint bytes (hex-encoded: tx_hash + output_index) |
| [].hash | Canonical hash of the UTXO entry |
| [].detail | Human-readable breakdown of UTXO fields |

**Detail format:** `amt=<amount> h=<height> type=<output_type> cb=<is_coinbase> er=<is_epoch_reward> lock=<lock_until> ed=<extra_data_hex> pk=<pubkey_hash_prefix>`

Where `type`: 0=Normal, 1=Bond. `cb`/`er`: 0=false, 1=true. `ed`: extra_data hex (Bond UTXOs store creation_slot as 4 bytes LE). `pk`: first 16 chars of pubkey_hash.

**Diagnosis workflow:**
1. On node A: `getUtxoDiff` with no params -- save all entry hashes
2. On node B: `getUtxoDiff` with `referenceHashes` set to node A's hashes
3. The response shows only entries that differ or are missing
4. Compare the `detail` fields to identify the exact divergence (e.g., different `extra_data` on Bond UTXOs)

**Example (full dump):**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getUtxoDiff","params":{},"id":1}'
```

**Example (diff mode):**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getUtxoDiff","params":{"referenceHashes":["0xabc...","0xdef..."]},"id":1}'
```

---

## 12. Error Codes

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

## 13. Units and Formatting

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

## 14. Rate Limiting

Default rate limits (configurable):

| Resource | Limit |
|----------|-------|
| Requests per second | 100 |
| Burst size | 200 |

Exceeded limits return HTTP 429 (Too Many Requests).

---

## 15. Security Considerations

### Binding Address

By default, RPC binds to `127.0.0.1` (localhost only).

**To enable external access (NOT recommended for production):**
```toml
[rpc]
listen_addr = "0.0.0.0:8500"
```

### Authentication

No built-in authentication. Use:
- Firewall rules
- Reverse proxy with auth (nginx, caddy)
- VPN for remote access

### CORS

Cross-origin requests disabled by default. Enable in config if needed for web applications.

---

## 16. WebSocket Support

WebSocket subscriptions are planned but not yet implemented.

Future subscription topics:
- `newBlocks` - New block notifications
- `pendingTransactions` - New mempool transactions
- `logs` - Event logs

---

## 17. Archive & Integrity Methods

### `getBlockRaw`

Retrieve a raw block by height (for archiving/backfill).

**Parameters:**
- `height` (integer) — Block height to retrieve

**Response:**
```json
{
  "hash": "abc123...",
  "height": 100,
  "raw": "<hex-encoded block bytes>"
}
```

### `backfillFromPeer`

Trigger hot backfill of missing blocks from a remote seed/archive node.

**Parameters:**
- `rpc_url` (string) — RPC URL of the source node (e.g., `"http://127.0.0.1:18500"`)

**Response:**
```json
{
  "status": "started",
  "gap_start": 1,
  "gap_end": 977,
  "source": "http://127.0.0.1:18500"
}
```

### `backfillStatus`

Check the progress of an active backfill operation.

**Parameters:** None

**Response:**
```json
{
  "active": true,
  "progress": 450,
  "total": 977,
  "source": "http://127.0.0.1:18500"
}
```

### `verifyChainIntegrity`

Full scan of every height from 1 to tip. Detects missing blocks (gaps) anywhere in the chain, not just at the start. Uses lightweight height-index lookups (no block deserialization), so ~10-30 seconds for 1M blocks on SSD.

**Parameters:** None

**Response (complete chain):**
```json
{
  "complete": true,
  "tip": 1223,
  "scanned": 1223,
  "missing": [],
  "missing_count": 0
}
```

**Response (gaps found):**
```json
{
  "complete": false,
  "tip": 1000000,
  "scanned": 1000000,
  "missing": ["45-67", "1234", "50000-50100"],
  "missing_count": 125
}
```

**Notes:**
- Missing heights are returned as compressed ranges (e.g., `"45-67"` means blocks 45 through 67 are missing)
- Single missing blocks are returned as individual strings (e.g., `"1234"`)
- `missing_count` is the total number of missing blocks across all ranges
- Runs in a background thread to avoid blocking the RPC event loop
- Added in v2.0.29

---

## 18. Snapshot Methods

### getStateSnapshot

Returns a full state snapshot (chain state, UTXO set, producer set) as hex-encoded bytes. Used for snap sync and state verification.

**Parameters:** None

**Response:**
```json
{
    "height": 12345,
    "blockHash": "0x...",
    "stateRoot": "0x...",
    "chainState": "hex...",
    "utxoSet": "hex...",
    "producerSet": "hex...",
    "totalBytes": 123456
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| height | Block height of the snapshot |
| blockHash | Block hash at snapshot height |
| stateRoot | Combined state root hash |
| chainState | Hex-encoded canonical ChainState bytes |
| utxoSet | Hex-encoded canonical UTXO set bytes |
| producerSet | Hex-encoded canonical ProducerSet bytes |
| totalBytes | Total size of all snapshot data in bytes |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateSnapshot","params":{},"id":1}'
```

---

## 19. Pool Methods (AMM)

### getPoolInfo

Returns detailed information about an AMM pool.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| poolId | string | Pool ID (hex) |

**Response:**
```json
{
    "poolId": "0x...",
    "assetA": "0x0000...0000",
    "assetB": "0x...",
    "reserveA": 1000000000,
    "reserveB": 5000000,
    "totalShares": 70710678,
    "feeBps": 30,
    "price": 0.005,
    "twapCumulativePrice": "12345678901234",
    "lastUpdateSlot": 45000,
    "creationSlot": 40000,
    "status": 0,
    "txHash": "0x...",
    "outputIndex": 0
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| poolId | Pool identifier (hex) |
| assetA | Asset A identifier (always DOLI = zero hash) |
| assetB | Asset B identifier (fungible token ID) |
| reserveA | DOLI reserve in base units |
| reserveB | Token reserve in raw token units |
| totalShares | Total LP shares outstanding |
| feeBps | Swap fee in basis points (30 = 0.3%) |
| price | Spot price (reserveB / reserveA) |
| twapCumulativePrice | Cumulative TWAP price (fixed-point) |
| lastUpdateSlot | Slot of last pool state update |
| creationSlot | Slot when pool was created |
| status | Pool status code |
| txHash | Transaction hash of the pool UTXO |
| outputIndex | Output index of the pool UTXO |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getPoolInfo","params":{"poolId":"0xabc..."},"id":1}'
```

---

### getPoolList

Returns all AMM pools (deduplicated by pool ID).

**Parameters:** None

**Response:**
```json
[
    {
        "poolId": "0x...",
        "assetB": "0x...",
        "reserveA": 1000000000,
        "reserveB": 5000000,
        "feeBps": 30,
        "price": 0.005
    }
]
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getPoolList","params":{},"id":1}'
```

---

### getPoolPrice

Returns the spot price for a pool, with optional TWAP computation.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| poolId | string | Pool ID (hex) |
| windowSlots | integer | (Optional) TWAP window in slots |

**Response:**
```json
{
    "spotPrice": 0.005,
    "twapPrice": 0.0048,
    "twapWindow": 360
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| spotPrice | Current spot price (reserveB / reserveA) |
| twapPrice | Time-weighted average price over the window (only if windowSlots provided) |
| twapWindow | Actual window used (capped to pool age) |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getPoolPrice","params":{"poolId":"0xabc...","windowSlots":360},"id":1}'
```

---

### getSwapQuote

Simulates a swap without creating a transaction. Returns expected output amount, price impact, and fee.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| poolId | string | Pool ID (hex) |
| amountIn | integer | Amount to swap (base units) |
| direction | string | Swap direction: `"a2b"` (DOLI to token) or `"b2a"` (token to DOLI) |

**Response:**
```json
{
    "amountOut": 4950,
    "priceImpact": 0.5,
    "fee": 30
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| amountOut | Expected output amount |
| priceImpact | Price impact as percentage (higher = worse) |
| fee | Fee deducted from input |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getSwapQuote","params":{"poolId":"0xabc...","amountIn":1000000,"direction":"a2b"},"id":1}'
```

---

## 20. Lending Methods

### getLoanInfo

Returns detailed information about a loan identified by its Collateral UTXO.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| txHash | string | Collateral UTXO transaction hash (hex) |
| outputIndex | integer | Collateral UTXO output index |

**Response:**
```json
{
    "outpoint": {
        "txHash": "0x...",
        "outputIndex": 0
    },
    "poolId": "0x...",
    "borrowerHash": "0x...",
    "collateralAmount": 100000000,
    "collateralAssetId": "0x...",
    "principal": 50000000,
    "interestRateBps": 500,
    "creationSlot": 40000,
    "liquidationRatioBps": 15000,
    "accruedInterest": 125000,
    "totalDebt": 50125000,
    "elapsedSlots": 1000,
    "ltvBps": 5012,
    "liquidatable": false
}
```

**Fields:**
| Field | Description |
|-------|-------------|
| outpoint | Collateral UTXO outpoint (txHash + outputIndex) |
| poolId | Lending pool ID (hex) |
| borrowerHash | Borrower's pubkey hash (hex) |
| collateralAmount | Collateral amount in base units |
| collateralAssetId | Collateral asset ID (hex) |
| principal | Original borrowed amount |
| interestRateBps | Annual interest rate in basis points (500 = 5%) |
| creationSlot | Slot when the loan was created |
| liquidationRatioBps | LTV ratio at which liquidation is allowed (15000 = 150%) |
| accruedInterest | Interest accrued since creation |
| totalDebt | principal + accruedInterest |
| elapsedSlots | Slots elapsed since loan creation |
| ltvBps | Current loan-to-value ratio in basis points |
| liquidatable | Whether the loan can be liquidated at current LTV |

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getLoanInfo","params":{"txHash":"0xabc...","outputIndex":0},"id":1}'
```

---

### getLoanList

Returns all active loans (Collateral UTXOs), optionally filtered by borrower.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| borrower | string | (Optional) Borrower pubkey hash (hex) to filter by |

**Response:**
```json
[
    {
        "outpoint": {
            "txHash": "0x...",
            "outputIndex": 0
        },
        "borrowerHash": "0x...",
        "collateralAmount": 100000000,
        "principal": 50000000,
        "totalDebt": 50125000,
        "interestRateBps": 500,
        "liquidatable": false
    }
]
```

**Example:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getLoanList","params":{},"id":1}'

# Filter by borrower
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getLoanList","params":{"borrower":"0xabc..."},"id":1}'
```

---

*API version: 1.3 (39 methods)*

*Last updated: March 2026*
