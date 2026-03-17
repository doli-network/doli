# DOLI Infrastructure

## Architecture Overview (v8 — March 17, 2026)

Mainnet producers distributed across ai1, ai2, ai4, ai5. Seeds on ai1, ai2, ai3. ai2 is the build server.

### Design Principles

- **Load distribution**: Mainnet producers spread across 4 servers for resilience
- **Dedicated build server**: ai2 compiles release binaries, distributes to all servers
- **Deterministic ports**: Node N → P2P 3030N, RPC 850N, Metrics 900N. Seeds use port suffix 00.
- **Central key backup**: All keys duplicated in `/mainnet/keys/` and `/testnet/keys/` on their respective servers
- **Unified paths**: All nodes under `/mainnet/` and `/testnet/` from root, owned by `ilozada:doliadmin`

## Servers

| Server | Role |
|--------|------|
| ai1 | Mainnet seed + N1-N3, Testnet seed + NT1-NT5 |
| ai2 | Mainnet seed + N4-N5, Testnet seed + build + explorer |
| ai3 | Seeds only (mainnet + testnet) |
| ai4 | Mainnet N6-N8 |
| ai5 | Mainnet N9-N12, Testnet NT6-NT12 |

> IPs, SSH credentials, and ports are in the private `doli-ops` repo. See `~/.doli/servers.env`.

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
/mainnet/                      # ai2 only
├── bin/
│   ├── doli-node              # shared binary, all mainnet nodes use this
│   └── doli                   # CLI binary
├── keys/                      # CENTRAL key backup (all producer keys)
│   ├── producer_1.json
│   ├── producer_2.json
│   ├── producer_3.json
│   └── producer_6.json
├── seed/
│   ├── data/                  # seed chain data
│   └── blocks/                # archived blocks
├── n1/
│   ├── data/
│   └── keys/
│       └── producer.json      # symlink or copy of producer_1.json
├── n2/ ... n6/
│   └── (same structure)
└── (n4, n5 dirs reserved for future expansion)

/testnet/                      # ai1 only
├── bin/
│   ├── doli-node
│   └── doli
├── keys/                      # CENTRAL key backup
│   ├── nt1.json ... nt6.json
├── seed/
│   ├── data/
│   └── blocks/
├── nt1/ ... nt6/
│   ├── data/
│   └── keys/
│       └── producer.json

/var/log/doli/
├── mainnet/                   # ai2 only
│   ├── seed.log
│   ├── n1.log ... n6.log
└── testnet/                   # ai1 only
    ├── seed.log
    ├── nt1.log ... nt6.log
```

## DNS

| Record | Type | Value | Purpose |
|--------|------|-------|---------|
| `doli.network` | A | <ai2-ip> (ai2) | Website |
| `www.doli.network` | CNAME | → doli.network | Website alias |
| `seed1.doli.network` | A | <ai2-ip> (ai2) | Mainnet P2P seed |
| `seed2.doli.network` | A | <ai3-ip> (ai3) | Mainnet P2P seed |
| `archive.doli.network` | A | <ai2-ip> (ai2) | Mainnet archive RPC |
| `testnet.doli.network` | A | <ai1-ip> (ai1) | Testnet web |
| `bootstrap1.testnet.doli.network` | A | <ai1-ip> (ai1) | Testnet P2P seed |
| `bootstrap2.testnet.doli.network` | A | <ai3-ip> (ai3) | Testnet P2P seed |
| `archive.testnet.doli.network` | A | <ai1-ip> (ai1) | Testnet archive RPC |

DNS managed at Hostinger (ns1.dns-parking.com / ns2.dns-parking.com).

## SSL Certificates

Managed by certbot with auto-renewal on ai2:

| Domain | Server | Expiry |
|--------|--------|--------|
| `doli.network` | ai2 | 2026-06-06 |
| `www.doli.network` | ai2 | 2026-06-06 |
| `testnet.doli.network` | ai1 | 2026-06-06 |

## Mainnet

### Seed / Archiver Nodes (Archive + Relay + Public RPC)

Non-producing, sync-only, publicly accessible. These are the network entry points, block archive sources, and RPC backends for the block explorer (`doli.network/explorer.html`). See **[archiver.md](./archiver.md)** for full details.

| Node | Server | P2P | RPC | Metrics | Service | DNS |
|------|--------|-----|-----|---------|---------|-----|
| Seed1 | ai1 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seed1.doli.network` |
| Seed2 | ai2 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seed2.doli.network` |
| Seed3 | ai3 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seed3.doli.network` |

All run with `--relay-server --rpc-bind 0.0.0.0 --archive-to /mainnet/seed/blocks`.

### Producer Nodes (N1-N12)

Producers distributed across 4 servers. N1-N5 = maintainers + producers. N6-N12 = producers only.

| Nodes | Server | P2P | RPC | Metrics | Service |
|-------|--------|-----|-----|---------|---------|
| N1-N3 | ai1 | 30301-30303 | 8501-8503 | 9001-9003 | `doli-mainnet-n{1,2,3}` |
| N4-N5 | ai2 | 30304-30305 | 8504-8505 | 9004-9005 | `doli-mainnet-n{4,5}` |
| N6-N8 | ai4 | 30306-30308 | 8506-8508 | 9006-9008 | `doli-mainnet-n{6,7,8}` |
| N9-N12 | ai5 | 30309-30312 | 8509-8512 | 9009-9012 | `doli-mainnet-n{9,10,11,12}` |

All bootstrap from `--bootstrap /dns4/seed1.doli.network/tcp/30300 --bootstrap /dns4/seed2.doli.network/tcp/30300`.

Keys: `/mainnet/n{N}/keys/producer.json`. Data: `/mainnet/n{N}/data/`.

### Port Formula

```
Mainnet:  P2P = 30300 + N    RPC = 8500 + N    Metrics = 9000 + N
Testnet:  P2P = 40300 + N    RPC = 18500 + N   Metrics = 19000 + N
Seeds:    suffix = 00 (i.e., 30300, 8500, 9000 / 40300, 18500, 19000)
```

### Binaries

| Server | Binary Paths |
|--------|-------------|
| ai1 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` |
| ai2 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` (build server) |
| ai3 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` (seeds only) |
| ai4 | `/mainnet/bin/doli-node` |
| ai5 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` |

**Upgrade procedure (non-consensus changes only — UI, RPC, logging, etc.):**

> **IMPORTANT**: You cannot overwrite a binary while any process is using it ("Text file busy").
> You must stop ALL nodes using that binary path before copying, then restart them.
> This includes standby nodes (N7-N12, NT7-NT12) and cross-server services
> (mainnet seed on ai1, testnet seed on ai2).

```bash
# 1. Build on ai2 (exclude GUI — glib-2.0 not available on server)
ssh $USER@<ai2-ip>
source ~/.cargo/env && cd ~/repos/doli
git fetch origin && git reset --hard origin/main   # safe after force pushes
cargo build --release -p doli-node -p doli-cli

# 2. Distribute to ai1 and ai3 BEFORE stopping anything
scp target/release/doli-node $USER@<ai1-ip>:/tmp/doli-node-new
scp -P $AI3_SSH_PORTtarget/release/doli-node $USER@<ai3-ip>:/tmp/doli-node-new

# 3. Stop ALL nodes on ai2 (active + standby + cross-network seeds)
sudo systemctl stop doli-mainnet-seed doli-mainnet-n{1..12} doli-testnet-seed 2>/dev/null
pgrep -la doli-node || echo "ai2: all stopped"

# 4. Copy binary on ai2
sudo cp target/release/doli-node /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node

# 5. Restart ai2 (seed first, then producers)
sudo systemctl start doli-mainnet-seed && sleep 3
sudo systemctl start doli-mainnet-n{1..6}
sudo systemctl start doli-mainnet-n{7..12} 2>/dev/null  # standby nodes

# 6. Deploy ai1 — stop all, copy, restart
ssh $USER@<ai1-ip> '
  sudo systemctl stop doli-testnet-seed doli-testnet-nt{1..12} doli-mainnet-seed 2>/dev/null
  pgrep -la doli-node || echo "ai1: all stopped"
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo systemctl start doli-mainnet-seed doli-testnet-seed && sleep 3
  sudo systemctl start doli-testnet-nt{1..12} 2>/dev/null'

# 7. Deploy ai3 — seeds only
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> '
  sudo systemctl stop doli-mainnet-seed doli-testnet-seed
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo systemctl start doli-mainnet-seed doli-testnet-seed'
```

> **WARNING**: For consensus-critical changes, do NOT use rolling restarts. See [Consensus-Critical Deployment](#consensus-critical-deployment) below.

### Mainnet Bootstrap

All producers use DNS-based bootstrap:
```
--bootstrap /dns4/seed1.doli.network/tcp/30300
--bootstrap /dns4/seed2.doli.network/tcp/30300
```

### Log Paths

Logs are on each node's respective server at `/var/log/doli/mainnet/n{N}.log`. Seeds at `/var/log/doli/mainnet/seed.log`.

| Server | Logs |
|--------|------|
| ai1 | `seed.log`, `n1.log`, `n2.log`, `n3.log` |
| ai2 | `seed.log`, `n4.log`, `n5.log` |
| ai3 | `seed.log` |
| ai4 | `n6.log`, `n7.log`, `n8.log` |
| ai5 | `n9.log`, `n10.log`, `n11.log`, `n12.log` |

## Testnet

### Seed / Archiver Nodes (Archive + Relay + Public RPC)

| Node | Server | P2P | RPC | Metrics | Service | DNS |
|------|--------|-----|-----|---------|---------|-----|
| Seed1 | ai1 | 40300 | 18500 | 19000 | `doli-testnet-seed` | `bootstrap1.testnet.doli.network` |
| Seed2 | ai3 | 40300 | 18500 | 19000 | `doli-testnet-seed` | `bootstrap2.testnet.doli.network` |

### Producer Nodes (NT1-NT6)

All testnet producers run on **ai1**.

| Node | Server | P2P | RPC | Metrics | Service | Key |
|------|--------|-----|-----|---------|---------|-----|
| NT1 | ai1 | 40301 | 18501 | 19001 | `doli-testnet-nt1` | `/testnet/nt1/keys/producer.json` |
| NT2 | ai1 | 40302 | 18502 | 19002 | `doli-testnet-nt2` | `/testnet/nt2/keys/producer.json` |
| NT3 | ai1 | 40303 | 18503 | 19003 | `doli-testnet-nt3` | `/testnet/nt3/keys/producer.json` |
| NT4 | ai1 | 40304 | 18504 | 19004 | `doli-testnet-nt4` | `/testnet/nt4/keys/producer.json` |
| NT5 | ai1 | 40305 | 18505 | 19005 | `doli-testnet-nt5` | `/testnet/nt5/keys/producer.json` |
| NT6 | ai1 | 40306 | 18506 | 19006 | `doli-testnet-nt6` | `/testnet/nt6/keys/producer.json` |

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

All testnet logs on **ai1**:

| Node | Path |
|------|------|
| Seed | `/var/log/doli/testnet/seed.log` |
| NT1 | `/var/log/doli/testnet/nt1.log` |
| NT2 | `/var/log/doli/testnet/nt2.log` |
| NT3 | `/var/log/doli/testnet/nt3.log` |
| NT4 | `/var/log/doli/testnet/nt4.log` |
| NT5 | `/var/log/doli/testnet/nt5.log` |
| NT6 | `/var/log/doli/testnet/nt6.log` |

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
# Status (mainnet — run on ai2)
sudo systemctl status doli-mainnet-n1

# Status (testnet — run on ai1)
sudo systemctl status doli-testnet-nt1

# Restart single node
sudo systemctl restart doli-mainnet-n1

# View logs
tail -f /var/log/doli/mainnet/n1.log

# Enable on boot
sudo systemctl enable doli-mainnet-n1

# Stop all mainnet on ai2
sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3 doli-mainnet-n6
```

### Upgrade Order (Non-Consensus Changes)

**Rolling restarts** — safe for: UI, RPC, logging, metrics, non-consensus bug fixes.

Seeds first, then producers. Testnet first, then mainnet. Include ALL nodes (active + standby).

> **NOTE**: `systemctl restart` does NOT work for binary upgrades — the binary is still
> "text file busy" during restart. You must stop ALL nodes sharing the same binary path,
> copy the new binary, then start them. See the upgrade procedure above.

```
ai1: mainnet-seed + testnet-seed → NT1-NT12
ai2: mainnet-seed + testnet-seed → N1-N12
ai3: mainnet-seed + testnet-seed
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
ssh $USER@<ai2-ip>
source ~/.cargo/env && cd ~/repos/doli
git fetch origin && git reset --hard origin/main   # safe after force pushes
cargo build --release -p doli-node -p doli-cli      # exclude GUI (no glib-2.0 on server)

# 2. Record source MD5
md5sum target/release/doli-node | tee /tmp/source_md5.txt

# 3. Deploy locally on ai2 (mainnet)
sudo cp target/release/doli-node /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
sudo cp target/release/doli /mainnet/bin/doli && sudo chmod 755 /mainnet/bin/doli

# 4. Distribute to ai1 (testnet)
scp target/release/doli-node $USER@<ai1-ip>:/tmp/doli-node-new
scp target/release/doli $USER@<ai1-ip>:/tmp/doli-new
ssh $USER@<ai1-ip> '
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-new /testnet/bin/doli && sudo chmod 755 /testnet/bin/doli'

# 5. Distribute to ai3 (seeds — via Mac relay if no direct connectivity)
# Option A: direct from ai2
scp -P $AI3_SSH_PORTtarget/release/doli-node $USER@<ai3-ip>:/tmp/doli-node-new
scp -P $AI3_SSH_PORTtarget/release/doli $USER@<ai3-ip>:/tmp/doli-new
# Option B: via Mac if ai2→ai3 has no route
#   scp from ai2 to Mac, then scp -P $AI3_SSH_PORTfrom Mac to ai3
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> '
  sudo cp /tmp/doli-node-new /mainnet/bin/doli-node && sudo chmod 755 /mainnet/bin/doli-node
  sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node
  sudo cp /tmp/doli-new /mainnet/bin/doli && sudo chmod 755 /mainnet/bin/doli
  sudo cp /tmp/doli-new /testnet/bin/doli && sudo chmod 755 /testnet/bin/doli'

# 6. Verify checksums match on ALL THREE servers
cat /tmp/source_md5.txt
ssh $USER@<ai1-ip> 'md5sum /testnet/bin/doli-node /testnet/bin/doli'
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> 'md5sum /mainnet/bin/doli-node /testnet/bin/doli-node'
# All must be identical
```

#### Phase 2: Stop ALL Nodes (All Servers Simultaneously)

```bash
# Terminal 1 (ai2 — mainnet + standby + testnet seed):
ssh $USER@<ai2-ip> 'sudo systemctl stop \
  doli-mainnet-seed doli-mainnet-n{1..12} doli-testnet-seed 2>/dev/null'

# Terminal 2 (ai1 — testnet + standby + mainnet seed):
ssh $USER@<ai1-ip> 'sudo systemctl stop \
  doli-testnet-seed doli-testnet-nt{1..12} doli-mainnet-seed 2>/dev/null'

# Terminal 3 (ai3 — seeds only):
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> 'sudo systemctl stop doli-mainnet-seed doli-testnet-seed'
```

**WAIT until ALL nodes are confirmed stopped on ALL THREE servers before proceeding.**
Use `pgrep -la doli-node` — `systemctl stop` may miss nodes with old-gen service names.

```bash
# Verify no nodes running
ssh $USER@<ai1-ip> 'pgrep -la doli-node || echo "ai1: all stopped"'
ssh $USER@<ai2-ip> 'pgrep -la doli-node || echo "ai2: all stopped"'
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> 'pgrep -la doli-node || echo "ai3: all stopped"'
```

#### Phase 3: Wipe Data (If Genesis Changed)

Only needed if `genesis_hash` changed (timestamp, message, network_id, or slot_duration modified).

```bash
# ai2 — wipe mainnet data (ALL nodes including standby)
ssh $USER@<ai2-ip> '
  for N in seed n1 n2 n3 n4 n5 n6 n7 n8 n9 n10 n11 n12; do
    sudo rm -rf /mainnet/$N/data/* && echo "wiped /mainnet/$N/data"
  done
  sudo rm -rf /mainnet/seed/blocks/*'

# ai1 — wipe testnet data (ALL nodes including standby)
ssh $USER@<ai1-ip> '
  for N in seed nt1 nt2 nt3 nt4 nt5 nt6 nt7 nt8 nt9 nt10 nt11 nt12; do
    sudo rm -rf /testnet/$N/data/* && echo "wiped /testnet/$N/data"
  done
  sudo rm -rf /testnet/seed/blocks/*'

# ai3 — wipe seed data
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> '
  sudo rm -rf /mainnet/seed/data/* /testnet/seed/data/* && echo "wiped ai3 seed data"
  sudo rm -rf /mainnet/seed/blocks/* /testnet/seed/blocks/*'
```

#### Phase 4: Start Nodes (Ordered)

Start seeds first (all three servers), wait for them to peer, then start producers.

```bash
# Step 1: Start ALL seeds on ALL THREE servers
ssh $USER@<ai2-ip> 'sudo systemctl start doli-mainnet-seed doli-testnet-seed 2>/dev/null'
ssh $USER@<ai1-ip> 'sudo systemctl start doli-testnet-seed doli-mainnet-seed 2>/dev/null'
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> 'sudo systemctl start doli-mainnet-seed doli-testnet-seed'

# Wait 10 seconds for seeds to initialize and peer with each other
sleep 10

# Step 2: Start ai2 mainnet producers (active + standby)
ssh $USER@<ai2-ip> 'sudo systemctl start doli-mainnet-n{1..12} 2>/dev/null'

# Step 3: Start ai1 testnet producers (active + standby)
ssh $USER@<ai1-ip> 'sudo systemctl start doli-testnet-nt{1..12} 2>/dev/null'
```

#### Phase 5: Verify Consensus

Wait ~30 seconds, then confirm all nodes are on the same chain.

```bash
# ai2 mainnet (all N1-N12 + seed)
ssh $USER@<ai2-ip> '
for entry in "8500:Seed" "8501:N1" "8502:N2" "8503:N3" "8504:N4" "8505:N5" "8506:N6" \
             "8507:N7" "8508:N8" "8509:N9" "8510:N10" "8511:N11" "8512:N12"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-6s %s\n" "$name" "$h"
done'

# ai1 testnet (all NT1-NT12 + seed)
ssh $USER@<ai1-ip> '
for entry in "18500:Seed" "18501:NT1" "18502:NT2" "18503:NT3" "18504:NT4" "18505:NT5" "18506:NT6" \
             "18507:NT7" "18508:NT8" "18509:NT9" "18510:NT10" "18511:NT11" "18512:NT12"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-6s %s\n" "$name" "$h"
done'

# ai3 seeds
ssh -p $AI3_SSH_PORT$USER@<ai3-ip> '
for entry in "8500:Seed-M" "18500:Seed-T"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "?")
  printf "%-6s %s\n" "$name" "$h"
done'

# All heights should be within ±1 of each other within each network
```

### Quick Decision Checklist

Before deploying, answer these:

- [ ] Does this change affect `slot % total_bonds` calculation? → **Simultaneous**
- [ ] Does this change how blocks are validated? → **Simultaneous**
- [ ] Does this change genesis_hash inputs? → **Simultaneous + wipe**
- [ ] Does this change reward/penalty calculations? → **Simultaneous**
- [ ] Is it RPC-only, logging, or CLI? → Rolling is safe

---

## Legacy Architecture (pre March 13, 2026)

> This section documents old setups for reference when investigating historical issues.

### v2 Layout (March 8 - March 13, 2026)

Mixed mainnet/testnet on both ai1 and ai2:
- ai1: Seed1, N1, N3, Seed1-T, NT1, NT3, NT5
- ai2: Seed2, N2, N6, Seed2-T, NT2, NT4, NT6

### v1 Layout (pre March 8, 2026)

5-server setup with inconsistent paths:

| Server | IP | Role |
|--------|-----|------|
| omegacortex (ai1) | <ai1-ip> | N1 (seed+relay+producer), N2, N6, Archiver, NT1-NT5, Archive-T |
| omegacortex (ai2) | <ai2-ip> | Non-producing mirrors of ai1 + Web + Swap Bot |
| N3 | <n3-ip> | N3, NT6-NT8 |
| N4 | <n4-ip> | N4, N8-N12 |
| N5 | <n5-ip> | N5, N7, NT9-NT12 |

Problems: N1 was producer AND seed, DNS round-robin to same server, 5 servers to manage, inconsistent paths, N4/N5 keys swapped.
