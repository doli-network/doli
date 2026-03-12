# DOLI Infrastructure

## Architecture Overview (v2 — March 8, 2026)

High-availability setup across 2 servers with dedicated seed/archive nodes separated from producers. If one server goes down, the other maintains the chain with half the producers.

### Design Principles

- **Separation of concerns**: Seeds (public entry points) are separate from producers (block creation)
- **Alternating distribution**: Odd producers on ai1, even on ai2 — maximizes HA for sequential slot scheduling
- **Deterministic ports**: Node N → P2P 3030N, RPC 850N, Metrics 900N. Seeds use port suffix 00.
- **Central key backup**: All keys duplicated in `/mainnet/keys/` and `/testnet/keys/` on both servers
- **Unified paths**: All nodes under `/mainnet/` and `/testnet/` from root, owned by `ilozada:doliadmin`

## Servers

| Server | IP | Alias | Role | SSH |
|--------|-----|-------|------|-----|
| omegacortex (ai1) | 72.60.228.233 | ai1 | Seed1, N1, N3, Seed1-T, NT1, NT3, NT5 | `ssh ilozada@72.60.228.233` |
| omegacortex (ai2) | 187.124.95.188 | ai2 | Seed2, N2, N6, Seed2-T, NT2, NT4, NT6 | `ssh ilozada@187.124.95.188` |

### Users & Permissions

| Item | Value |
|------|-------|
| Group | `doliadmin` (GID 2000) |
| Members | `ilozada`, `isudoajl` |
| Ownership | `ilozada:doliadmin` (all under `/mainnet`, `/testnet`) |
| Directory perms | 2775 (setgid) |
| Key perms | 640 |
| Binary perms | 755 |

Both users can operate any node via the shared `doliadmin` group.

## Directory Structure

```
/mainnet/
├── bin/
│   ├── doli-node          # shared binary, all nodes use this
│   └── doli               # CLI binary
├── keys/                  # CENTRAL key backup (all producer keys)
│   ├── producer_1.json
│   ├── producer_2.json
│   ├── producer_3.json
│   └── producer_6.json
├── seed/
│   ├── data/              # seed chain data
│   └── blocks/            # archived blocks
├── n1/
│   ├── data/
│   └── keys/
│       └── producer.json  # symlink or copy of producer_1.json
├── n2/ ... n6/
│   └── (same structure)
└── (n4, n5 dirs reserved for future expansion)

/testnet/
├── bin/
│   ├── doli-node
│   └── doli
├── keys/                  # CENTRAL key backup
│   ├── nt1.json ... nt6.json
├── seed/
│   ├── data/
│   └── blocks/
├── nt1/ ... nt6/
│   ├── data/
│   └── keys/
│       └── producer.json

/var/log/doli/
├── mainnet/
│   ├── seed.log
│   ├── n1.log ... n6.log
└── testnet/
    ├── seed.log
    ├── nt1.log ... nt6.log
```

## DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `doli.network` | A | 72.60.228.233 + 187.124.95.188 | Website (round-robin) |
| `www.doli.network` | CNAME | → doli.network | Website alias |
| `seed1.doli.network` | A | 72.60.228.233 (ai1) | Mainnet P2P seed |
| `seed2.doli.network` | A | 187.124.95.188 (ai2) | Mainnet P2P seed |
| `archive.doli.network` | A | 187.124.95.188 (ai2) | Mainnet archive RPC |
| `testnet.doli.network` | A | 187.124.95.188 (ai2) | Testnet web |
| `bootstrap1.testnet.doli.network` | A | 72.60.228.233 (ai1) | Testnet P2P seed |
| `bootstrap2.testnet.doli.network` | A | 187.124.95.188 (ai2) | Testnet P2P seed |
| `archive.testnet.doli.network` | A | 72.60.228.233 (ai1) | Testnet archive RPC |

DNS managed at Hostinger (ns1.dns-parking.com / ns2.dns-parking.com).

## SSL Certificates

Managed by certbot with auto-renewal on ai2:

| Domain | Server | Expiry |
|--------|--------|--------|
| `doli.network` | ai2 | 2026-06-06 |
| `www.doli.network` | ai2 | 2026-06-06 |
| `testnet.doli.network` | ai2 | 2026-06-06 |

ai1 also has its own SSL certificates for doli.network.

## Mainnet

### Seed / Archiver Nodes (Archive + Relay + Public RPC)

Non-producing, sync-only, publicly accessible. These are the network entry points, block archive sources, and RPC backends for the block explorer (`doli.network/explorer.html`). See **[archiver.md](./archiver.md)** for full details.

| Node | Server | P2P | RPC | Metrics | Service | DNS |
|------|--------|-----|-----|---------|---------|-----|
| Seed1 | ai1 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seed1.doli.network` |
| Seed2 | ai2 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seed2.doli.network` |

Both run with `--relay-server --rpc-bind 0.0.0.0 --archive-to /mainnet/seed/blocks`.

### Producer Nodes (N1-N6)

| Node | Server | P2P | RPC | Metrics | Service | Key |
|------|--------|-----|-----|---------|---------|-----|
| N1 | ai1 | 30301 | 8501 | 9001 | `doli-mainnet-n1` | `/mainnet/n1/keys/producer.json` |
| N2 | ai2 | 30302 | 8502 | 9002 | `doli-mainnet-n2` | `/mainnet/n2/keys/producer.json` |
| N3 | ai1 | 30300 | 8503 | 9003 | `doli-mainnet-n3` | `/mainnet/n3/keys/producer.json` |
| N6 | ai2 | 30300 | 8506 | 9006 | `doli-mainnet-n6` | `/mainnet/n6/keys/producer.json` |

All bootstrap from `--bootstrap /dns4/seed1.doli.network/tcp/30300 --bootstrap /dns4/seed2.doli.network/tcp/30300`.

**On-chain status (as of March 8, 2026):**

| Node | PubKey (prefix) | On-chain | Bonds |
|------|-----------------|----------|-------|
| N1 | `202047...` | active | 1 |
| N2 | `effe88...` | active | 1 |
| N3 | `54323c...` | NOT bonded | 0 |
| N6 | `d13ae3...` | active | 1 |

N3 is running but not yet registered on-chain. N4, N5 directories reserved for future expansion.

### Port Formula

```
Mainnet:  P2P = 30300 + N    RPC = 8500 + N    Metrics = 9000 + N
Testnet:  P2P = 40300 + N    RPC = 18500 + N   Metrics = 19000 + N
Seeds:    suffix = 00 (i.e., 30300, 8500, 9000 / 40300, 18500, 19000)
```

### Binaries

Both servers use the same layout:

| Binary | Path |
|--------|------|
| Mainnet doli-node | `/mainnet/bin/doli-node` |
| Mainnet doli CLI | `/mainnet/bin/doli` |
| Testnet doli-node | `/testnet/bin/doli-node` |
| Testnet doli CLI | `/testnet/bin/doli` |

**Upgrade procedure (non-consensus changes only — UI, RPC, logging, etc.):**
```bash
# 1. Copy new binary
sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node

# 2. Restart nodes one at a time (seeds first, then producers)
sudo systemctl restart doli-mainnet-seed
sleep 10
sudo systemctl restart doli-mainnet-n1
# ... etc
```

> **WARNING**: For consensus-critical changes, do NOT use rolling restarts. See [Consensus-Critical Deployment](#consensus-critical-deployment) below.

### Mainnet Bootstrap

All producers use DNS-based bootstrap:
```
--bootstrap /dns4/seed1.doli.network/tcp/30300
--bootstrap /dns4/seed2.doli.network/tcp/30300
```

### Log Paths

| Node | Path | Server |
|------|------|--------|
| Seed | `/var/log/doli/mainnet/seed.log` | ai1 / ai2 |
| N1 | `/var/log/doli/mainnet/n1.log` | ai1 |
| N2 | `/var/log/doli/mainnet/n2.log` | ai2 |
| N3 | `/var/log/doli/mainnet/n3.log` | ai1 |
| N6 | `/var/log/doli/mainnet/n6.log` | ai2 |

## Testnet

### Seed / Archiver Nodes (Archive + Relay + Public RPC)

| Node | Server | P2P | RPC | Metrics | Service | DNS |
|------|--------|-----|-----|---------|---------|-----|
| Seed1 | ai1 | 40300 | 18500 | 19000 | `doli-testnet-seed` | `bootstrap1.testnet.doli.network` |
| Seed2 | ai2 | 40300 | 18500 | 19000 | `doli-testnet-seed` | `bootstrap2.testnet.doli.network` |

### Producer Nodes (NT1-NT6)

| Node | Server | P2P | RPC | Metrics | Service | Key |
|------|--------|-----|-----|---------|---------|-----|
| NT1 | ai1 | 40301 | 18501 | 19001 | `doli-testnet-nt1` | `/testnet/nt1/keys/producer.json` |
| NT2 | ai2 | 40302 | 18502 | 19002 | `doli-testnet-nt2` | `/testnet/nt2/keys/producer.json` |
| NT3 | ai1 | 40300 | 18503 | 19003 | `doli-testnet-nt3` | `/testnet/nt3/keys/producer.json` |
| NT4 | ai2 | 40304 | 18504 | 19004 | `doli-testnet-nt4` | `/testnet/nt4/keys/producer.json` |
| NT5 | ai1 | 40305 | 18505 | 19005 | `doli-testnet-nt5` | `/testnet/nt5/keys/producer.json` |
| NT6 | ai2 | 40306 | 18506 | 19006 | `doli-testnet-nt6` | `/testnet/nt6/keys/producer.json` |

All bootstrap from `--bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40300 --bootstrap /dns4/bootstrap2.testnet.doli.network/tcp/40300`.

### Testnet Parameters

| Parameter | Value |
|-----------|-------|
| Genesis | March 8, 2026 08:54:00 UTC |
| Genesis Producers | 6 (NT1-NT6) |
| Block Reward | 1 tDOLI |
| Slot Duration | 10 seconds |
| Epoch Length | 360 blocks (1 hour) |
| Bond Unit | 10 tDOLI |
| Vesting | 1 day (6h quarters: 75/50/25/0%) |

### Log Paths

| Node | Path | Server |
|------|------|--------|
| Seed | `/var/log/doli/testnet/seed.log` | ai1 / ai2 |
| NT1 | `/var/log/doli/testnet/nt1.log` | ai1 |
| NT2 | `/var/log/doli/testnet/nt2.log` | ai2 |
| NT3 | `/var/log/doli/testnet/nt3.log` | ai1 |
| NT4 | `/var/log/doli/testnet/nt4.log` | ai2 |
| NT5 | `/var/log/doli/testnet/nt5.log` | ai1 |
| NT6 | `/var/log/doli/testnet/nt6.log` | ai2 |

## External Producers

### Mainnet

| Operator | Address |
|----------|---------|
| atinoco | `doli17f7pqlkfjweddk88ry6gtc23hvmptsqk2epxx7h6x9a8gvan3crsfl243e` |
| antonio | `doli1nc3erj8tqew5yz09s60ang7n77p3ftjh7e9m370w3v5c95aaj38qvv98wl` |
| daniel | `doli1p7s6hcacnm6t64nk670leeu9w3tvnkvwc688r9zlvh2f3573f6vs4cynzh` |

### Testnet

| Operator | Address |
|----------|---------|
| atinoco | `tdoli17axj5cjstmwqs8a4zg6xxy5qjwnd7j7dnggyrhy3gya37x7ckrhsefjvfy` |

## Systemd Services

### Naming Convention

```
doli-{network}-{role}.service
```

Examples: `doli-mainnet-seed`, `doli-mainnet-n1`, `doli-testnet-seed`, `doli-testnet-nt3`.

### Common Operations

```bash
# Status
sudo systemctl status doli-mainnet-n1

# Restart single node
sudo systemctl restart doli-mainnet-n1

# View logs
tail -f /var/log/doli/mainnet/n1.log

# Enable on boot
sudo systemctl enable doli-mainnet-n1

# Stop all mainnet on a server
sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n3
```

### Upgrade Order (Non-Consensus Changes)

**Rolling restarts** — safe for: UI, RPC, logging, metrics, non-consensus bug fixes.

Seeds first, then producers. Testnet first, then mainnet.

```
Testnet: seed-ai1 → seed-ai2 → NT6 → NT5 → NT4 → NT3 → NT2 → NT1
Mainnet: seed-ai1 → seed-ai2 → N6 → N3 → N2 → N1
```

> For consensus-critical changes, see [Consensus-Critical Deployment](#consensus-critical-deployment) below.

## Consensus-Critical Deployment

**MANDATORY procedure for ANY change that affects block production, validation, or scheduling.**

### What Is Consensus-Critical?

A change is consensus-critical if different binary versions would produce or validate blocks differently. If the answer to "would running old and new binaries simultaneously cause a fork?" is YES or MAYBE, use this procedure.

| Category | Examples | Consensus-Critical? |
|----------|----------|:---:|
| **Scheduling** | `count_bonds()`, `select_producer_for_slot()`, bond weights, sort order | **YES** |
| **Validation** | Block validation rules, timestamp checks, VDF params | **YES** |
| **Genesis** | Genesis timestamp, genesis message, network_id, slot_duration | **YES** (new genesis_hash) |
| **Economics** | Reward calculation, halving schedule, bond_unit, vesting | **YES** |
| **Transaction** | New TxType, changed validation for existing types | **YES** |
| **RPC/Display** | RPC response format, logging, metrics, explorer | No |
| **Networking** | Gossip optimization, peer scoring, NAT traversal | Usually no |
| **CLI** | New subcommands, output formatting, wallet features | No |

**Rule of thumb**: If it touches `scheduler.rs`, `consensus.rs`, `validation.rs`, or how `apply_block()` processes transactions — it is consensus-critical.

### Procedure: Simultaneous Deployment

**NEVER use rolling restarts for consensus-critical changes.** A rolling deployment creates a window where nodes run incompatible binaries, causing an irreconcilable fork. See `docs/legacy/bugs/REPORT_HA_FAILURE.md` for the incident analysis.

#### Phase 1: Build & Distribute

```bash
# 1. Build on ai2 (build server)
ssh ilozada@187.124.95.188
cd ~/repos/doli && git pull && cargo build --release

# 2. Copy binary to deployment staging on BOTH servers
# ai2 (local)
sudo cp target/release/doli-node /tmp/doli-node-new
sudo cp target/release/doli /tmp/doli-new

# ai1 (remote)
ssh ilozada@72.60.228.233 'cat > /tmp/doli-node-new' < target/release/doli-node
ssh ilozada@72.60.228.233 'cat > /tmp/doli-new' < target/release/doli

# ai3 (remote — no SSH key to ai1/ai2, transfer via Mac or ai2)
ssh ilozada@187.124.95.188 'cat target/release/doli-node' | ssh ai3 'cat > /tmp/doli-node-new'
ssh ilozada@187.124.95.188 'cat target/release/doli' | ssh ai3 'cat > /tmp/doli-new'

# 3. Verify checksums match on ALL THREE servers
md5sum /tmp/doli-node-new
ssh ilozada@72.60.228.233 'md5sum /tmp/doli-node-new'
ssh ai3 'md5sum /tmp/doli-node-new'
# All three must be identical
```

#### Phase 2: Stop ALL Nodes (All Servers Simultaneously)

```bash
# Open three terminals — one per server. Execute SIMULTANEOUSLY.

# Terminal 1 (ai1):
ssh ilozada@72.60.228.233 'sudo systemctl stop \
  doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n3 \
  doli-testnet-seed doli-testnet-nt1 doli-testnet-nt3 doli-testnet-nt5'

# Terminal 2 (ai2):
ssh ilozada@187.124.95.188 'sudo systemctl stop \
  doli-mainnet-seed doli-mainnet-n2 doli-mainnet-n4 doli-mainnet-n6 \
  doli-testnet-seed doli-testnet-nt2 doli-testnet-nt4 doli-testnet-nt6'

# Terminal 3 (ai3 — seeds only):
ssh ai3 'sudo systemctl stop doli-mainnet-seed doli-testnet-seed'
```

**WAIT until ALL nodes are confirmed stopped on ALL THREE servers before proceeding.**

```bash
# Verify no nodes running
ssh ilozada@72.60.228.233 'pgrep -la doli-node || echo "ai1: all stopped"'
ssh ilozada@187.124.95.188 'pgrep -la doli-node || echo "ai2: all stopped"'
ssh ai3 'pgrep -la doli-node || echo "ai3: all stopped"'
```

#### Phase 3: Deploy Binary (Both Servers)

```bash
# ai1
ssh ilozada@72.60.228.233 '
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-new /mainnet/bin/doli && sudo chmod 755 /mainnet/bin/doli
  sudo cp /tmp/doli-new /testnet/bin/doli && sudo chmod 755 /testnet/bin/doli'

# ai2
ssh ilozada@187.124.95.188 '
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-new /mainnet/bin/doli && sudo chmod 755 /mainnet/bin/doli
  sudo cp /tmp/doli-new /testnet/bin/doli && sudo chmod 755 /testnet/bin/doli'

# ai3
ssh ai3 '
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-new /mainnet/bin/doli && sudo chmod 755 /mainnet/bin/doli
  sudo cp /tmp/doli-new /testnet/bin/doli && sudo chmod 755 /testnet/bin/doli'
```

#### Phase 4: Wipe Data (If Genesis Changed)

Only needed if `genesis_hash` changed (timestamp, message, network_id, or slot_duration modified).

```bash
# ai1 — wipe ALL node data dirs
ssh ilozada@72.60.228.233 '
  for d in /mainnet/seed/data /mainnet/n1/data /mainnet/n3/data \
           /testnet/seed/data /testnet/nt1/data /testnet/nt3/data /testnet/nt5/data; do
    sudo rm -rf $d/* && echo "wiped $d"
  done
  sudo rm -rf /mainnet/seed/blocks/*'

# ai2 — wipe ALL node data dirs
ssh ilozada@187.124.95.188 '
  for d in /mainnet/seed/data /mainnet/n2/data /mainnet/n4/data /mainnet/n6/data \
           /testnet/seed/data /testnet/nt2/data /testnet/nt4/data /testnet/nt6/data; do
    sudo rm -rf $d/* && echo "wiped $d"
  done
  sudo rm -rf /mainnet/seed/blocks/*'

# ai3 — wipe seed data dirs
ssh ai3 '
  sudo rm -rf ~/.doli/mainnet/* ~/.doli/testnet/* && echo "wiped ai3 seed data"
  sudo rm -rf /mainnet/seed/blocks/* /testnet/seed/blocks/*'
```

#### Phase 5: Start Nodes (Ordered)

Start seeds first (all three servers), wait for them to peer, then start producers.

```bash
# Step 1: Start seeds on ALL THREE servers
ssh ilozada@72.60.228.233 'sudo systemctl start doli-mainnet-seed doli-testnet-seed'
ssh ilozada@187.124.95.188 'sudo systemctl start doli-mainnet-seed doli-testnet-seed'
ssh ai3 'sudo systemctl start doli-mainnet-seed doli-testnet-seed'

# Wait 10 seconds for seeds to initialize and peer with each other
sleep 10

# Step 2: Start ai1 producers
ssh ilozada@72.60.228.233 'sudo systemctl start \
  doli-mainnet-n1 doli-mainnet-n3 \
  doli-testnet-nt1 doli-testnet-nt3 doli-testnet-nt5'

# Step 3: Start ai2 producers (immediately after ai1 — no need to wait)
ssh ilozada@187.124.95.188 'sudo systemctl start \
  doli-mainnet-n2 doli-mainnet-n4 doli-mainnet-n6 \
  doli-testnet-nt2 doli-testnet-nt4 doli-testnet-nt6'
```

#### Phase 6: Verify Consensus

Wait ~30 seconds, then confirm all nodes are on the same chain.

```bash
# ai1 mainnet
ssh ilozada@72.60.228.233 '
for entry in "8500:Seed" "8501:N1" "8503:N3"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%s\n" "$name" "$h"
done'

# ai2 mainnet
ssh ilozada@187.124.95.188 '
for entry in "8500:Seed" "8502:N2" "8504:N4" "8506:N6"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%s\n" "$name" "$h"
done'

# ai3 seed
ssh ai3 '
for entry in "8500:Seed3"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)[\"result\"][\"bestHeight\"])" 2>/dev/null || echo "?")
  printf "%-6s h=%s\n" "$name" "$h"
done'

# All heights should be within ±1 of each other across ALL THREE servers
# If heights diverge by >2, STOP and investigate — do NOT let the fork grow
# Also check explorer.doli.network/network.html — Genesis should show "unified"
```

### Quick Decision Checklist

Before deploying, answer these:

- [ ] Does this change affect `slot % total_bonds` calculation? → **Simultaneous**
- [ ] Does this change how blocks are validated? → **Simultaneous**
- [ ] Does this change genesis_hash inputs? → **Simultaneous + wipe**
- [ ] Does this change reward/penalty calculations? → **Simultaneous**
- [ ] Is it RPC-only, logging, or CLI? → Rolling is safe

---

## Legacy Architecture (pre March 8, 2026)

> This section documents the old setup for reference when investigating historical issues.

### Old Server Layout

| Server | IP | Role |
|--------|-----|------|
| omegacortex (ai1) | 72.60.228.233 | N1 (seed+relay+producer), N2, N6, Archiver, NT1-NT5, Archive-T |
| omegacortex (ai2) | 187.124.95.188 | Non-producing mirrors of ai1 + Web + Swap Bot |
| N3 | 147.93.84.44 (SSH port 50790) | N3, NT6-NT8 |
| N4 | 72.60.115.209 (SSH port 50790) | N4, N8-N12 |
| N5 | 72.60.70.166 (SSH port 50790) | N5, N7, NT9-NT12 |

### Old Problems

- N1 was producer AND seed+relay simultaneously (single point of failure for network entry)
- Seeds and producers shared the same process — if seed got DDoSed, production stopped
- Inconsistent ports (N6 was on :30305 instead of :30300)
- DNS round-robin pointed both seed1 and seed2 to the same server (zero real redundancy)
- 5 servers to manage, inconsistent paths (`~/doli-node`, `/opt/doli/target/release/`, etc.)
- Keys scattered across multiple paths and servers
- N4/N5 keys swapped (producer_5 on N4, producer_4 on N5)

### Old Port Mapping

| Node | Server | P2P | RPC | Service |
|------|--------|-----|-----|---------|
| N1 | ai1 | 30300 | 8500 | `doli-mainnet-node1` |
| N2 | ai1 | 30304 | 8546 | `doli-mainnet-node2` |
| N6 | ai1 | 30305 | 8547 | `doli-mainnet-node6` |
| N3 | N3 VPS | 30300 | 8500 | `doli-mainnet-node3` |
| N4 | N4 VPS | 30300 | 8500 | `doli-mainnet-node4` |
| N5 | N5 VPS | 30300 | 8500 | `doli-mainnet-node5` |
| Archiver | ai1 | 30300 | 8548 | `doli-mainnet-archiver` |

### Old Binary Paths

| Server | Mainnet | Testnet |
|--------|---------|---------|
| ai1 | `~/repos/doli/target/release/doli-node` | `/opt/doli/testnet/doli-node` |
| ai2 | `/opt/doli/target/release/doli-node` | `/opt/doli/testnet/doli-node` |
| N3 | `~/doli-node` (service) + `/opt/doli/target/release/doli-node` | `/opt/doli/testnet/doli-node` |
| N4 | `/opt/doli/target/release/doli-node` | — |
| N5 | `/opt/doli/target/release/doli-node` | `/opt/doli/testnet/doli-node` |

### Old Key Paths

| Network | Server | Path |
|---------|--------|------|
| Mainnet | ai1 | `~/.doli/mainnet/keys/producer_{1,2,6}.json` |
| Mainnet | N3 | `~/.doli/mainnet/keys/producer_3.json` |
| Mainnet | N4 | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` (swapped!) |
| Mainnet | N5 | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` (swapped!) |
| Testnet | ai1 | `~/doli-test/keys/nt{1-5}.json` |
| Testnet | N3 | `~/doli-test/keys/nt{6-8}.json` |
| Testnet | N5 | `~/doli-test/keys/nt{9-12}.json` |

### Migration Notes (March 8, 2026)

- Chain was wiped and restarted at 08:54:00 UTC with new genesis
- All nodes consolidated from 5 servers to 2 (ai1 + ai2)
- N3/N4/N5 VPS nodes decommissioned (processes stopped, data wiped, services disabled)
- Seeds separated from producers into dedicated archive+relay nodes
- Deterministic port numbering adopted
- Central key backup established at `/mainnet/keys/` and `/testnet/keys/`
