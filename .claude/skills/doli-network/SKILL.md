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

### Remote Producer Nodes (direct SSH from Mac — NOT via omegacortex)

| Node | Server | RPC Port | SSH |
|------|--------|----------|-----|
| N3 | 147.93.84.44 | 8545 (localhost) | `ssh -p 50790 ilozada@147.93.84.44` |
| N4 | 72.60.115.209 | 8545 (localhost) | `ssh -p 50790 ilozada@72.60.115.209` |
| N5 | 72.60.70.166 | 8545 (localhost) | `ssh -p 50790 ilozada@72.60.70.166` |

N3/N4/N5 RPC is localhost-only. Must SSH in first, then curl 127.0.0.1.

### Later Producer Nodes N7-N12 (not yet registered, syncing)

| Node | Server | P2P Port | RPC Port | Metrics | Service | Key Path | Data Dir | Log |
|------|--------|----------|----------|---------|---------|----------|----------|-----|
| N7 | N5 (72.60.70.166) | 30304 | 8546 | 9091 | `doli-mainnet-node7` | `~/.doli/mainnet/keys/n7.json` | `~/.doli/mainnet/n7/data` | `/var/log/doli/node7.log` |
| N8 | N4 (72.60.115.209) | 30304 | 8546 | 9091 | `doli-mainnet-node8` | `~/.doli/mainnet/keys/n8.json` | `~/.doli/mainnet/n8/data` | `/var/log/doli/node8.log` |
| N9 | N4 (72.60.115.209) | 30305 | 8547 | 9092 | `doli-mainnet-node9` | `~/.doli/mainnet/keys/n9.json` | `~/.doli/mainnet/n9/data` | `/var/log/doli/node9.log` |
| N10 | N4 (72.60.115.209) | 30306 | 8548 | 9093 | `doli-mainnet-node10` | `~/.doli/mainnet/keys/n10.json` | `~/.doli/mainnet/n10/data` | `/var/log/doli/node10.log` |
| N11 | N4 (72.60.115.209) | 30307 | 8549 | 9094 | `doli-mainnet-node11` | `~/.doli/mainnet/keys/n11.json` | `~/.doli/mainnet/n11/data` | `/var/log/doli/node11.log` |
| N12 | N4 (72.60.115.209) | 30308 | 8550 | 9095 | `doli-mainnet-node12` | `~/.doli/mainnet/keys/n12.json` | `~/.doli/mainnet/n12/data` | `/var/log/doli/node12.log` |

All N7-N12 use binary `/opt/doli/target/release/doli-node`, owner `ilozada`, `--force-start --yes`.
N7 bootstraps from omegacortex + N4. N8-N12 bootstrap from omegacortex + N3.

#### Check N7-N12 Status
```bash
# N7 (on N5)
ssh -p 50790 -o ConnectTimeout=5 ilozada@72.60.70.166 \
  'curl -s -X POST http://127.0.0.1:8546 -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}"'

# N8-N12 (on N4)
ssh -p 50790 -o ConnectTimeout=5 ilozada@72.60.115.209 '
for p in 8546 8547 8548 8549 8550; do
  echo "PORT $p:"
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$p \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}"
  echo
done'
```

### Testnet Nodes (NT1-NT12)

#### NT1-NT5 (omegacortex — 72.60.228.233)

| Node | P2P | RPC | Metrics | Service | Key | Binary |
|------|-----|-----|---------|---------|-----|--------|
| NT1 | 40303 | 18545 | 19090 | `doli-testnet-nt1` | `~/doli-test/keys/nt1.json` | `~/repos/doli/target/release/doli-node` |
| NT2 | 40304 | 18546 | 19091 | `doli-testnet-nt2` | `~/doli-test/keys/nt2.json` | same |
| NT3 | 40305 | 18547 | 19092 | `doli-testnet-nt3` | `~/doli-test/keys/nt3.json` | same |
| NT4 | 40306 | 18548 | 19093 | `doli-testnet-nt4` | `~/doli-test/keys/nt4.json` | same |
| NT5 | 40307 | 18549 | 19094 | `doli-testnet-nt5` | `~/doli-test/keys/nt5.json` | same |

NT1+NT2 have `--relay-server --rpc-bind 0.0.0.0`. Logs: `~/doli-test/ntN/node.log`.

#### NT6-NT8 (N3 — 147.93.84.44, SSH port 50790)

| Node | P2P | RPC | Metrics | Service | Key | Binary |
|------|-----|-----|---------|---------|-----|--------|
| NT6 | 40303 | 18545 | 19090 | `doli-testnet-nt6` | `~/doli-test/keys/nt6.json` | `~/doli-node` |
| NT7 | 40304 | 18546 | 19091 | `doli-testnet-nt7` | `~/doli-test/keys/nt7.json` | same |
| NT8 | 40305 | 18547 | 19092 | `doli-testnet-nt8` | `~/doli-test/keys/nt8.json` | same |

Logs: `~/doli-test/ntN/node.log`. Bootstrap: omegacortex:40303.

#### NT9-NT12 (N5 — 72.60.70.166, SSH port 50790)

| Node | P2P | RPC | Metrics | Service | Key | Binary |
|------|-----|-----|---------|---------|-----|--------|
| NT9 | 40303 | 18545 | 19090 | `doli-testnet-nt9` | `~/doli-test/keys/nt9.json` | `/opt/doli/target/release/doli-node` |
| NT10 | 40304 | 18546 | 19091 | `doli-testnet-nt10` | `~/doli-test/keys/nt10.json` | same |
| NT11 | 40305 | 18547 | 19092 | `doli-testnet-nt11` | `~/doli-test/keys/nt11.json` | same |
| NT12 | 40306 | 18548 | 19093 | `doli-testnet-nt12` | `~/doli-test/keys/nt12.json` | same |

Logs: `~/doli-test/ntN/node.log`. Bootstrap: omegacortex:40303.

Testnet RPC is localhost-only on all hosts. N4 has no testnet nodes.

#### Check All Testnet Nodes
```bash
# NT1-NT5 (omegacortex)
ssh -o ConnectTimeout=5 ilozada@72.60.228.233 '
for port in 18545 18546 18547 18548 18549; do
  echo "PORT $port:"
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}"
  echo
done'

# NT6-NT8 (N3)
ssh -p 50790 -o ConnectTimeout=5 ilozada@147.93.84.44 '
for port in 18545 18546 18547; do
  echo "PORT $port:"
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}"
  echo
done'

# NT9-NT12 (N5)
ssh -p 50790 -o ConnectTimeout=5 ilozada@72.60.70.166 '
for port in 18545 18546 18547 18548; do
  echo "PORT $port:"
  curl -s --connect-timeout 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}"
  echo
done'
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
