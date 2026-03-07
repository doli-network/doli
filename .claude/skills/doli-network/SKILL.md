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

## Mainnet Node Inventory

When asked for "node status" or "all nodes", check ALL of these:

### Producer Nodes (omegacortex — 72.60.228.233)

| Node | RPC Port | Service |
|------|----------|---------|
| N1 | 8545 | `doli-mainnet-node1` |
| N2 | 8546 | `doli-mainnet-node2` |
| N6 | 8547 | `doli-mainnet-node6` |

### Archive Node (omegacortex — 72.60.228.233)

| Node | RPC Port | Service | DNS |
|------|----------|---------|-----|
| Archiver | 8548 | `doli-mainnet-archiver` | `archive.doli.network` |

### Remote Producer Nodes (via omegacortex jump host)

| Node | Server | RPC Port | SSH |
|------|--------|----------|-----|
| N3 | 147.93.84.44 | 8545 | `ssh -p 50790 ilozada@147.93.84.44` |
| N4 | 72.60.70.166 | 8545 | `ssh -p 50790 ilozada@72.60.70.166` |
| N5 | 72.60.115.209 | 8545 | `ssh -p 50790 ilozada@72.60.115.209` |

### Testnet Nodes (NT1-NT18)

| Server | Nodes | RPC Ports |
|--------|-------|-----------|
| omegacortex | NT1-NT5 | 9001-9005 |
| N3 (147.93.84.44) | NT6-NT8 | 9001-9003 |
| N4 (72.60.70.166) | NT9-NT13 | 9001-9005 |
| N5 (72.60.115.209) | NT14-NT18 | 9001-9005 |

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
- `status`: active/inactive
- `era`: Current era
- `pendingWithdrawals`: List of pending bond withdrawals

## Checking Blocks Produced

### From Node Logs
```bash
# Count blocks produced per node
grep -c "Block.*produced" /path/to/node.log

# Show block production details
grep "Block.*produced" /path/to/node.log | tail -10
```

### From RPC - Scan Chain for Producer
```bash
# Count blocks by a specific producer (by pubkey prefix)
PRODUCER_PREFIX="7655d10a"  # First 8 chars of pubkey
for h in $(seq 1 100); do
  BLOCK=$(curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":{\"height\":$h},\"id\":1}")
  PRODUCER=$(echo $BLOCK | jq -r '.result.producer // ""')
  if [[ "$PRODUCER" == "$PRODUCER_PREFIX"* ]]; then
    echo "Height $h: Producer $PRODUCER"
  fi
done
```

## Checking Epoch Rewards

### From Node Logs
```bash
# Show epoch reward distribution
grep "Epoch.*rewards:" /path/to/node.log

# Show all epoch reward inclusions
grep "Including.*epoch reward" /path/to/node.log
```

### Find Epoch Boundary Blocks
```bash
# Devnet: 30 slots/epoch, Testnet: 360 slots/epoch
SLOTS_PER_EPOCH=30  # For devnet

for h in $(seq 1 200); do
  BLOCK=$(curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":{\"height\":$h},\"id\":1}")
  SLOT=$(echo $BLOCK | jq -r '.result.slot // 0')
  TX_COUNT=$(echo $BLOCK | jq -r '.result.txCount // 0')
  EPOCH=$((SLOT / SLOTS_PER_EPOCH))

  # First block of new epoch has reward transactions
  if [ "$TX_COUNT" -gt 0 ]; then
    echo "Height $h (Slot $SLOT, Epoch $EPOCH): $TX_COUNT transactions"
  fi
done
```

### Multi-Node Block Production Summary
```bash
# For 5-node testnet logs
for i in 1 2 3 4 5; do
  COUNT=$(grep -c "Block.*produced" /path/to/logs/node$i.log 2>/dev/null || echo "0")
  echo "Node $i: $COUNT blocks produced"
done
```

## Epoch Parameters by Network

| Network | Slots/Epoch | Slot Duration | Epoch Duration |
|---------|-------------|---------------|----------------|
| Mainnet | 360 | 10s | 1 hour |
| Testnet | 360 | 10s | 1 hour |
| Devnet | 30 | 1s | 30 seconds |
