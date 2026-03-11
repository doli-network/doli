---
name: doli-network
description: DOLI RPC reference, epoch params, block queries, state root debugging. NOT for balances, bonds, deploys, restarts — those are doli-ops. Triggers on "RPC method", "epoch info", "block query", "state root", "attestation", "getChainInfo", "getProducers", "devnet params".
---

# DOLI Network Operations

## Architecture (v3 — March 10, 2026)

3-server setup. Seeds on ai1+ai2+ai3. Producers on ai1+ai2 only. Odd nodes on ai1, even on ai2. ai3 = seeds only.

### Servers

| Server | IP | SSH | Role |
|--------|-----|-----|------|
| ai1 | 72.60.228.233 | `ssh ilozada@72.60.228.233` | Seeds + Producers (odd) |
| ai2 | 187.124.95.188 | `ssh ilozada@187.124.95.188` | Seeds + Producers (even) |
| ai3 | 187.124.148.93 | `ssh ai3` | Seeds only (no producers) |

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
| Devnet | 28500 | — |

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
| Seed3 | ai3 (187.124.148.93) | 30300 | 8500 | `doli-mainnet-seed` | `seed3.doli.network` |

### Producer Nodes

N1-N5 = producers + maintainers. N6-N12 = producers only. Currently running: N1-N6. N7-N12 ready but not started.

| Node | Server | P2P | RPC | Service | Key |
|------|--------|-----|-----|---------|-----|
| N1 | ai1 | 30301 | 8501 | `doli-mainnet-n1` | `/mainnet/n1/keys/producer.json` |
| N2 | ai2 | 30302 | 8502 | `doli-mainnet-n2` | `/mainnet/n2/keys/producer.json` |
| N3 | ai1 | 30303 | 8503 | `doli-mainnet-n3` | `/mainnet/n3/keys/producer.json` |
| N4 | ai2 | 30304 | 8504 | `doli-mainnet-n4` | `/mainnet/n4/keys/producer.json` |
| N5 | ai1 | 30305 | 8505 | `doli-mainnet-n5` | `/mainnet/n5/keys/producer.json` |
| N6 | ai2 | 30306 | 8506 | `doli-mainnet-n6` | `/mainnet/n6/keys/producer.json` |
| N7 | ai1 | 30307 | 8507 | `doli-mainnet-n7` | `/mainnet/n7/keys/producer.json` |
| N8 | ai2 | 30308 | 8508 | `doli-mainnet-n8` | `/mainnet/n8/keys/producer.json` |
| N9 | ai1 | 30309 | 8509 | `doli-mainnet-n9` | `/mainnet/n9/keys/producer.json` |
| N10 | ai2 | 30310 | 8510 | `doli-mainnet-n10` | `/mainnet/n10/keys/producer.json` |
| N11 | ai1 | 30311 | 8511 | `doli-mainnet-n11` | `/mainnet/n11/keys/producer.json` |
| N12 | ai2 | 30312 | 8512 | `doli-mainnet-n12` | `/mainnet/n12/keys/producer.json` |

Binary: `/mainnet/bin/doli-node`. Logs: `/var/log/doli/mainnet/nN.log`. Data: `/mainnet/n{N}/data/`.

Key backups (all 12): `/mainnet/keys/producer_{N}.json` on BOTH servers.

### Check All Mainnet Nodes
```bash
# From ai1 (seed + odd producers)
ssh ilozada@72.60.228.233 '
for entry in "8500:Seed" "8501:N1" "8503:N3" "8505:N5"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  v=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"version\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%-6s v=%s\n" "$name" "$h" "$v"
done'

# From ai2 (seed + even producers)
ssh ilozada@187.124.95.188 '
for entry in "8500:Seed" "8502:N2" "8504:N4" "8506:N6"; do
  port=${entry%%:*}; name=${entry##*:}
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}")
  h=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  v=$(echo $result | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"version\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%-6s v=%s\n" "$name" "$h" "$v"
done'

# From ai3 (seed only)
ssh ai3 '
for entry in "8500:Seed3"; do
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
| Seed3 | ai3 | 40300 | 18500 | `doli-testnet-seed` | `bootstrap3.testnet.doli.network` |

### Producer Nodes

NT1-NT5 = producers + maintainers. NT6-NT12 = producers only. Currently running: NT1-NT6. NT7-NT12 ready but not started.

| Node | Server | P2P | RPC | Service | Key |
|------|--------|-----|-----|---------|-----|
| NT1 | ai1 | 40301 | 18501 | `doli-testnet-nt1` | `/testnet/nt1/keys/producer.json` |
| NT2 | ai2 | 40302 | 18502 | `doli-testnet-nt2` | `/testnet/nt2/keys/producer.json` |
| NT3 | ai1 | 40303 | 18503 | `doli-testnet-nt3` | `/testnet/nt3/keys/producer.json` |
| NT4 | ai2 | 40304 | 18504 | `doli-testnet-nt4` | `/testnet/nt4/keys/producer.json` |
| NT5 | ai1 | 40305 | 18505 | `doli-testnet-nt5` | `/testnet/nt5/keys/producer.json` |
| NT6 | ai2 | 40306 | 18506 | `doli-testnet-nt6` | `/testnet/nt6/keys/producer.json` |
| NT7 | ai1 | 40307 | 18507 | `doli-testnet-nt7` | `/testnet/nt7/keys/producer.json` |
| NT8 | ai2 | 40308 | 18508 | `doli-testnet-nt8` | `/testnet/nt8/keys/producer.json` |
| NT9 | ai1 | 40309 | 18509 | `doli-testnet-nt9` | `/testnet/nt9/keys/producer.json` |
| NT10 | ai2 | 40310 | 18510 | `doli-testnet-nt10` | `/testnet/nt10/keys/producer.json` |
| NT11 | ai1 | 40311 | 18511 | `doli-testnet-nt11` | `/testnet/nt11/keys/producer.json` |
| NT12 | ai2 | 40312 | 18512 | `doli-testnet-nt12` | `/testnet/nt12/keys/producer.json` |

Binary: `/testnet/bin/doli-node`. Logs: `/var/log/doli/testnet/ntN.log`. Data: `/testnet/nt{N}/data/`.

Key backups (all 12): `/testnet/keys/nt{N}.json` on BOTH servers.

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

# From ai3 (seed only)
ssh ai3 '
for entry in "18500:Seed3"; do
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
- `bondAmount`: Total bonded amount (divide by 1e8 for DOLI)
- `status`: active/inactive
- `era`: Current era
- `pendingWithdrawals`: List of pending bond withdrawals

### verifyChainIntegrity
Full scan of every height from 1 to tip. Detects gaps anywhere in the chain.
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"verifyChainIntegrity","params":[],"id":1}' | jq '.result'
# Returns: { "complete": true/false, "tip": N, "scanned": N, "missing": ["45-67", ...], "missing_count": 0 }
```

### backfillFromPeer
Hot backfill missing blocks from a remote seed node (no restart needed).
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"backfillFromPeer","params":{"rpc_url":"http://SEED:PORT"},"id":1}'
```

### backfillStatus
Check progress of an active backfill.
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"backfillStatus","params":{},"id":1}' | jq '.result'
```

## State Root Debugging (Snap Sync Diagnosis)

When snap sync fails → nodes disagree on state root → quorum impossible.

### getStateRootDebug
Returns component hashes: `csHash` (ChainState), `utxoHash` (UtxoSet), `psHash` (ProducerSet), plus combined `stateRoot`.
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' | jq '.result'
```

### getUtxoDiff
Compares UTXO sets between two nodes. Shows per-UTXO differences including `extra_data`.
```bash
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getUtxoDiff","params":{"peer_url":"http://OTHER:PORT"},"id":1}' | jq '.result'
```

**Diagnosis flow**: Run `getStateRootDebug` on all nodes at same height. If `utxoHash` diverges → `getUtxoDiff` to find exact UTXOs. Root cause is usually Bond `extra_data` stamping in dual UTXO path. See `docs/architecture.md §9.1`.

---

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

## Critical Ops Notes

- **Node placement**: ai1 = ODD nodes only, ai2 = EVEN nodes only. NEVER create cross-server node dirs (slashing risk).
- **Data dir**: Services use `--data-dir <node>/data`. Runtime data (signed_slots.db, state_db, blocks, etc.) lives in `<node>/data/`, NOT `<node>/` top level.
- **Chain reset wipe**: Must wipe `<node>/data/` contents. If `signed_slots.db` survives, nodes hit SLASHING PROTECTION and refuse to produce.
- **Genesis timestamp**: 4 sources must be updated (consensus.rs, network_params.rs, chainspec.mainnet.json, chainspec.testnet.json). Chainspecs are embedded via `include_str!` — requires recompilation.
- **Compilation**: Done on ai2 (`cargo build --release`, no nix). Transfer to ai1 via ssh pipe. Verify md5 on all deployed binaries.
- **RPC binding**: Producers bind to 127.0.0.1 only. Seeds bind to 0.0.0.0. Must SSH to server to query producers.
- **Consensus-critical deploys**: NEVER use rolling restarts for changes that affect scheduling, validation, or rewards. MUST use simultaneous deployment: stop ALL → deploy ALL → start seeds → start producers. Rolling deploys cause irreconcilable forks (see `docs/legacy/bugs/REPORT_HA_FAILURE.md`). Full procedure in `docs/infrastructure.md` "Consensus-Critical Deployment".
