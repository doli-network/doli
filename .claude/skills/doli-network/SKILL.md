---
name: doli-network
description: Monitor and manage DOLI blockchain nodes. Use when checking chain status, producer rewards, node health, block info, or any RPC queries. Triggers on "check nodes", "chain status", "producer rewards", "network health", "testnet status", "devnet status".
---

# DOLI Network Operations

## RPC Endpoints by Network

| Network | RPC Port |
|---------|----------|
| Mainnet | 8545 |
| Testnet | 18545 |
| Devnet | 28545 |

## Essential RPC Commands

### Chain Status
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq '.result'
```

### Producer Rewards
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}' | jq '.result'
```

### Block by Height
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":HEIGHT},"id":1}' | jq '.result'
```

### Block by Hash
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getBlockByHash","params":{"hash":"HASH"},"id":1}' | jq '.result'
```

## Check Running Nodes
```bash
ps aux | grep "doli-node" | grep -v grep
```

## Multi-Node Status (testnet example with 5 nodes)
```bash
for port in 18545 18546 18547 18548 18549; do
  echo "=== RPC $port ==="
  curl -s -X POST http://127.0.0.1:$port -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -c '.result | {height: .bestHeight, slot: .bestSlot}'
done
```

## Key Response Fields

### getChainInfo
- `bestHeight`: Current block height
- `bestSlot`: Current slot number
- `network`: Network name (mainnet/testnet/devnet)
- `bestHash`: Latest block hash

### getProducers
- `publicKey`: Producer's public key
- `bondCount`: Number of bonds
- `bondAmount`: Total bonded amount (divide by 1e9 for DOLI)
- `blocksProduced`: Blocks produced count
- `pendingRewards`: Unclaimed rewards (divide by 1e9 for DOLI)
- `status`: active/inactive
