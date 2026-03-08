---
name: doli-network
description: Monitor and manage DOLI blockchain nodes. Use when checking chain status, producer rewards, node health, block info, or any RPC queries. Triggers on "check nodes", "chain status", "producer rewards", "network health", "testnet status", "devnet status".
---

# DOLI Network Operations

## Architecture (v2 — March 8, 2026)

2-server HA setup. Seeds (archive+relay) separated from producers. Odd nodes on ai1, even on ai2.

### Servers

| Server | IP | SSH |
|--------|-----|-----|
| ai1 | 72.60.228.233 | `ssh ilozada@72.60.228.233` |
| ai2 | 187.124.95.188 | `ssh ilozada@187.124.95.188` |

### Port Formula

```
Mainnet:  P2P = 30300 + N    RPC = 8500 + N    Metrics = 9000 + N
Testnet:  P2P = 40300 + N    RPC = 18500 + N   Metrics = 19000 + N
Seeds:    suffix 00 → P2P 30300/40300, RPC 8500/18500, Metrics 9000/19000
```

## RPC Endpoints by Network

| Network | Seed RPC | Producer RPC range |
|---------|----------|-------------------|
| Mainnet | 8500 (seed) | 8501-8506 (producers) |
| Testnet | 18500 (seed) | 18501-18506 (producers) |
| Devnet | 28545 | — |

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

## Mainnet Node Inventory

### Seed Nodes (Archive + Relay)

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 (72.60.228.233) | 30300 | 8500 | `doli-mainnet-seed` | `seed1.doli.network` |
| Seed2 | ai2 (187.124.95.188) | 30300 | 8500 | `doli-mainnet-seed` | `seed2.doli.network` |

### Producer Nodes

| Node | Server | P2P | RPC | Service | Key |
|------|--------|-----|-----|---------|-----|
| N1 | ai1 | 30301 | 8501 | `doli-mainnet-n1` | `/mainnet/n1/keys/producer.json` |
| N2 | ai2 | 30302 | 8502 | `doli-mainnet-n2` | `/mainnet/n2/keys/producer.json` |
| N3 | ai1 | 30303 | 8503 | `doli-mainnet-n3` | `/mainnet/n3/keys/producer.json` |
| N6 | ai2 | 30306 | 8506 | `doli-mainnet-n6` | `/mainnet/n6/keys/producer.json` |

Binary: `/mainnet/bin/doli-node`. Logs: `/var/log/doli/mainnet/nN.log`.

### Check All Mainnet Nodes
```bash
# From ai1 (seed, N1, N3)
ssh ilozada@72.60.228.233 '
for entry in "8500:Seed" "8501:N1" "8503:N3"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  v=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"version\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%-6s v=%s\n" "$name" "$h" "$v"
done'

# From ai2 (seed, N2, N6)
ssh ilozada@187.124.95.188 '
for entry in "8500:Seed" "8502:N2" "8506:N6"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  v=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"version\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%-6s v=%s\n" "$name" "$h" "$v"
done'
```

## Testnet Node Inventory

### Seed Nodes (Archive + Relay)

| Node | Server | P2P | RPC | Service | DNS |
|------|--------|-----|-----|---------|-----|
| Seed1 | ai1 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap1.testnet.doli.network` |
| Seed2 | ai2 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap2.testnet.doli.network` |

### Producer Nodes

| Node | Server | P2P | RPC | Service | Key |
|------|--------|-----|-----|---------|-----|
| NT1 | ai1 | 40301 | 18501 | `doli-testnet-nt1` | `/testnet/nt1/keys/producer.json` |
| NT2 | ai2 | 40302 | 18502 | `doli-testnet-nt2` | `/testnet/nt2/keys/producer.json` |
| NT3 | ai1 | 40303 | 18503 | `doli-testnet-nt3` | `/testnet/nt3/keys/producer.json` |
| NT4 | ai2 | 40304 | 18504 | `doli-testnet-nt4` | `/testnet/nt4/keys/producer.json` |
| NT5 | ai1 | 40305 | 18505 | `doli-testnet-nt5` | `/testnet/nt5/keys/producer.json` |
| NT6 | ai2 | 40306 | 18506 | `doli-testnet-nt6` | `/testnet/nt6/keys/producer.json` |

Binary: `/testnet/bin/doli-node`. Logs: `/var/log/doli/testnet/ntN.log`.

### Check All Testnet Nodes
```bash
# From ai1 (seed, NT1, NT3, NT5)
ssh ilozada@72.60.228.233 '
for entry in "18500:Seed" "18501:NT1" "18503:NT3" "18505:NT5"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%s\n" "$name" "$h"
done'

# From ai2 (seed, NT2, NT4, NT6)
ssh ilozada@187.124.95.188 '
for entry in "18500:Seed" "18502:NT2" "18504:NT4" "18506:NT6"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%s\n" "$name" "$h"
done'
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
grep -c "Block.*produced" /var/log/doli/mainnet/n1.log

# Show block production details
grep "Block.*produced" /var/log/doli/mainnet/n1.log | tail -10
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

## Epoch Parameters by Network

| Network | Slots/Epoch | Slot Duration | Epoch Duration |
|---------|-------------|---------------|----------------|
| Mainnet | 360 | 10s | 1 hour |
| Testnet | 360 | 10s | 1 hour |
| Devnet | 30 | 1s | 30 seconds |
