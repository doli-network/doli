# DOLI Infrastructure

## Architecture Overview (v9 — March 23, 2026)

Mainnet producers distributed across ai1, ai2, ai3, ai4, ai5. Seeds on ai1, ai2, ai3. ai2 is the build server. Named producers (SANTIAGO, IVAN) on ai3.

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
| ai3 | Seeds + Producers (SANTIAGO, IVAN) + testnet seed |
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
| `seeds.doli.network` | A | <ai3-ip> (ai3) | Mainnet P2P seed (aggregate) |
| `seeds.testnet.doli.network` | A | <ai3-ip> (ai3) | Testnet P2P seed (aggregate) |
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
| Seed3 | ai3 | 30300 | 8500 | 9000 | `doli-mainnet-seed` | `seeds.doli.network` |

All run with `--relay-server --rpc-bind 0.0.0.0 --archive-to /mainnet/seed/blocks`.

### Producer Nodes (N1-N12 + Named Producers)

Producers distributed across 5 servers. N1-N5 = maintainers + producers. N6-N12 = producers only. Named producers (SANTIAGO, IVAN) on ai3.

| Nodes | Server | P2P | RPC | Metrics | Service |
|-------|--------|-----|-----|---------|---------|
| N1-N3 | ai1 | 30301-30303 | 8501-8503 | 9001-9003 | `doli-mainnet-n{1,2,3}` |
| N4-N5 | ai2 | 30304-30305 | 8504-8505 | 9004-9005 | `doli-mainnet-n{4,5}` |
| N6-N8 | ai4 | 30306-30308 | 8506-8508 | 9006-9008 | `doli-mainnet-n{6,7,8}` |
| N9-N12 | ai5 | 30309-30312 | 8509-8512 | 9009-9012 | `doli-mainnet-n{9,10,11,12}` |
| SANTIAGO | ai3 | 30313 | 8513 | 9013 | `doli-mainnet-santiago` |
| IVAN | ai3 | 30314 | 8514 | 9014 | `doli-mainnet-ivan` |

All bootstrap from `--bootstrap /dns4/seed1.doli.network/tcp/30300 --bootstrap /dns4/seed2.doli.network/tcp/30300`.

Keys: `/mainnet/n{N}/keys/producer.json`. Data: `/mainnet/n{N}/data/`.

Named producers on ai3:
- SANTIAGO: Keys `/mainnet/santiago/keys/wallet.json`, Data `/mainnet/santiago/data/`
- IVAN: Keys `/mainnet/ivan/keys/wallet.json`, Data `/mainnet/ivan/data/`

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
| ai3 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` (seeds + SANTIAGO, IVAN) |
| ai4 | `/mainnet/bin/doli-node` |
| ai5 | `/mainnet/bin/doli-node`, `/testnet/bin/doli-node` |

**Upgrade procedure — Atomic Deploy (non-consensus changes only — UI, RPC, logging, etc.):**

> **CRITICAL**: NEVER stop services before binaries are pre-copied and MD5-verified on the target server.
> The old stop-copy-start procedure is DEPRECATED — it caused 5-minute downtime per server.
> This procedure achieves ~2 seconds disruption per server via atomic `mv` swap.

```bash
# ── Phase 1: Build on ai2 ──────────────────────────────────────────────
ssh $USER@<ai2-ip>
source ~/.cargo/env && cd ~/repos/doli
git fetch origin && git reset --hard origin/main
cargo build --release -p doli-node -p doli-cli

# Record source MD5
md5sum target/release/doli-node target/release/doli | tee /tmp/source_md5.txt

# ── Phase 2: Pre-copy as .new to ALL servers in parallel ────────────────
# ai2 (local — build server)
cp target/release/doli-node /mainnet/bin/doli-node.new
cp target/release/doli /mainnet/bin/doli.new

# ai1, ai3, ai4, ai5 (parallel scp)
scp target/release/doli-node $USER@<ai1-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai1-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai3-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai3-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai4-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai4-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai5-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai5-ip>:/mainnet/bin/doli.new &
wait

# ── Phase 3: Verify MD5 on ALL servers ──────────────────────────────────
cat /tmp/source_md5.txt
ssh $USER@<ai1-ip> 'md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new'
ssh $USER@<ai3-ip> 'md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new'
ssh $USER@<ai4-ip> 'md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new'
ssh $USER@<ai5-ip> 'md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new'
# ALL must match source_md5.txt — STOP if any mismatch

# ── Phase 4: chmod +x on ALL servers ───────────────────────────────────
chmod +x /mainnet/bin/doli-node.new /mainnet/bin/doli.new  # ai2 local
ssh $USER@<ai1-ip> 'chmod +x /mainnet/bin/doli-node.new /mainnet/bin/doli.new' &
ssh $USER@<ai3-ip> 'chmod +x /mainnet/bin/doli-node.new /mainnet/bin/doli.new' &
ssh $USER@<ai4-ip> 'chmod +x /mainnet/bin/doli-node.new /mainnet/bin/doli.new' &
ssh $USER@<ai5-ip> 'chmod +x /mainnet/bin/doli-node.new /mainnet/bin/doli.new' &
wait

# ── Phase 5: Atomic swap — ai2 FIRST (canary) ──────────────────────────
# ai2: Seed2 + N4 + N5
cd /mainnet/bin && \
  sudo systemctl stop doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5 && \
  mv doli-node doli-node.old && mv doli-node.new doli-node && \
  mv doli doli.old && mv doli.new doli && \
  sudo systemctl start doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5 && \
  echo "ai2 done"

# Verify ai2 — check version and height on seed2
curl -s -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
# Confirm version is correct and height is advancing before proceeding

# ── Phase 6: Atomic swap — ai1/ai3/ai4/ai5 in parallel ─────────────────
# ai1: Seed1 + N1 + N2 + N3
ssh $USER@<ai1-ip> 'cd /mainnet/bin && \
  sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3 && \
  mv doli-node doli-node.old && mv doli-node.new doli-node && \
  mv doli doli.old && mv doli.new doli && \
  sudo systemctl start doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3 && \
  echo "ai1 done"' &

# ai3: Seed3 + SANTIAGO + IVAN
ssh $USER@<ai3-ip> 'cd /mainnet/bin && \
  sudo systemctl stop doli-mainnet-seed doli-mainnet-santiago doli-mainnet-ivan && \
  mv doli-node doli-node.old && mv doli-node.new doli-node && \
  mv doli doli.old && mv doli.new doli && \
  sudo systemctl start doli-mainnet-seed doli-mainnet-santiago doli-mainnet-ivan && \
  echo "ai3 done"' &

# ai4: N6 + N7 + N8
ssh $USER@<ai4-ip> 'cd /mainnet/bin && \
  sudo systemctl stop doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8 && \
  mv doli-node doli-node.old && mv doli-node.new doli-node && \
  mv doli doli.old && mv doli.new doli && \
  sudo systemctl start doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8 && \
  echo "ai4 done"' &

# ai5: N9 + N10 + N11 + N12
ssh $USER@<ai5-ip> 'cd /mainnet/bin && \
  sudo systemctl stop doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12 && \
  mv doli-node doli-node.old && mv doli-node.new doli-node && \
  mv doli doli.old && mv doli.new doli && \
  sudo systemctl start doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12 && \
  echo "ai5 done"' &

wait

# ── Phase 7: Verify all seeds show correct version and synced height ────
for ip in <ai1-ip> <ai2-ip> <ai3-ip>; do
  echo "--- $ip ---"
  curl -s --max-time 3 -X POST http://$ip:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
  echo ""
done
# All seeds should show same version and heights within +/-1
```

**Key rules:**
- NEVER stop services before `.new` binaries are on disk and MD5-verified
- NEVER separate `stop` and `start` into different commands -- one line, ~2 seconds disruption
- ai2 deploys first (canary), verify version + height, then ai1/ai3/ai4/ai5 in parallel
- The `mv` is atomic on the same filesystem -- instant swap, no "Text file busy"
- Keep `.old` binaries for instant rollback if needed: `mv doli-node doli-node.bad && mv doli-node.old doli-node`

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
| ai3 | `seed.log`, `santiago.log`, `ivan.log` |
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
| Genesis | March 29, 2026 01:52:25 UTC (timestamp 1774749145, v96) |
| Genesis Producers | 0 (all register on-chain post-genesis) |
| Block Reward | 1 tDOLI |
| Slot Duration | 10 seconds |
| Epoch Length | 36 blocks (~6 minutes) |
| Bond Unit | 1 tDOLI |
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

Use the **Atomic Deploy** procedure above. Order:

1. **ai2 first (canary)**: Seed2 + N4 + N5 -- verify version and height before proceeding
2. **ai1/ai3/ai4/ai5 in parallel**: after ai2 is confirmed healthy

Per-server service groups for the atomic stop-swap-start command:

```
ai1: doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3
ai2: doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5
ai3: doli-mainnet-seed doli-mainnet-santiago doli-mainnet-ivan
ai4: doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8
ai5: doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12
```

> **NOTE**: `systemctl restart` does NOT work for binary upgrades -- the binary is
> "text file busy" during restart. Use the atomic `stop → mv → start` pattern instead.

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

#### Phase 1: Build & Pre-Copy

```bash
# 1. Build on ai2 (build server)
ssh $USER@<ai2-ip>
source ~/.cargo/env && cd ~/repos/doli
git fetch origin && git reset --hard origin/main
cargo build --release -p doli-node -p doli-cli

# 2. Record source MD5
md5sum target/release/doli-node target/release/doli | tee /tmp/source_md5.txt

# 3. Pre-copy as .new to ALL 5 servers in parallel
# ai2 (local)
cp target/release/doli-node /mainnet/bin/doli-node.new
cp target/release/doli /mainnet/bin/doli.new

# ai1, ai3, ai4, ai5 (parallel)
scp target/release/doli-node $USER@<ai1-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai1-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai3-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai3-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai4-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai4-ip>:/mainnet/bin/doli.new &
scp target/release/doli-node $USER@<ai5-ip>:/mainnet/bin/doli-node.new &
scp target/release/doli      $USER@<ai5-ip>:/mainnet/bin/doli.new &
wait

# 4. Verify MD5 on ALL servers
cat /tmp/source_md5.txt
for ip in <ai1-ip> <ai3-ip> <ai4-ip> <ai5-ip>; do
  echo "--- $ip ---"
  ssh $USER@$ip 'md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new'
done
md5sum /mainnet/bin/doli-node.new /mainnet/bin/doli.new  # ai2 local
# ALL must match — STOP if any mismatch

# 5. chmod +x on ALL servers
chmod +x /mainnet/bin/*.new  # ai2
for ip in <ai1-ip> <ai3-ip> <ai4-ip> <ai5-ip>; do
  ssh $USER@$ip 'chmod +x /mainnet/bin/*.new' &
done
wait
```

#### Phase 2: Stop ALL Nodes Simultaneously (Atomic Swap)

For consensus-critical changes, ALL nodes must be stopped before ANY are restarted.
Use the atomic stop-swap-start pattern, but execute the stop on ALL 5 servers simultaneously.

```bash
# Stop ALL nodes on ALL 5 servers (run in parallel terminals or with &)
ssh $USER@<ai1-ip> 'sudo systemctl stop doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3' &
ssh $USER@<ai2-ip> 'sudo systemctl stop doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5' &
ssh $USER@<ai3-ip> 'sudo systemctl stop doli-mainnet-seed doli-mainnet-santiago doli-mainnet-ivan' &
ssh $USER@<ai4-ip> 'sudo systemctl stop doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8' &
ssh $USER@<ai5-ip> 'sudo systemctl stop doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12' &
wait

# Verify ALL stopped
for ip in <ai1-ip> <ai2-ip> <ai3-ip> <ai4-ip> <ai5-ip>; do
  echo "--- $ip ---"
  ssh $USER@$ip 'pgrep -la doli-node || echo "all stopped"'
done

# Atomic mv on ALL servers (parallel)
ssh $USER@<ai1-ip> 'cd /mainnet/bin && mv doli-node doli-node.old && mv doli-node.new doli-node && mv doli doli.old && mv doli.new doli' &
ssh $USER@<ai2-ip> 'cd /mainnet/bin && mv doli-node doli-node.old && mv doli-node.new doli-node && mv doli doli.old && mv doli.new doli' &
ssh $USER@<ai3-ip> 'cd /mainnet/bin && mv doli-node doli-node.old && mv doli-node.new doli-node && mv doli doli.old && mv doli.new doli' &
ssh $USER@<ai4-ip> 'cd /mainnet/bin && mv doli-node doli-node.old && mv doli-node.new doli-node && mv doli doli.old && mv doli.new doli' &
ssh $USER@<ai5-ip> 'cd /mainnet/bin && mv doli-node doli-node.old && mv doli-node.new doli-node && mv doli doli.old && mv doli.new doli' &
wait
```

#### Phase 3: Wipe Data (If Genesis Changed)

Only needed if `genesis_hash` changed (timestamp, message, network_id, or slot_duration modified).

```bash
# ai1 — wipe mainnet node data
ssh $USER@<ai1-ip> '
  for N in seed n1 n2 n3; do
    sudo rm -rf /mainnet/$N/data/* && echo "wiped /mainnet/$N/data"
  done
  sudo rm -rf /mainnet/seed/blocks/*'

# ai2 — wipe mainnet node data
ssh $USER@<ai2-ip> '
  for N in seed n4 n5; do
    sudo rm -rf /mainnet/$N/data/* && echo "wiped /mainnet/$N/data"
  done
  sudo rm -rf /mainnet/seed/blocks/*'

# ai3 — wipe seed + named producer data
ssh $USER@<ai3-ip> '
  sudo rm -rf /mainnet/seed/data/* && echo "wiped ai3 seed data"
  sudo rm -rf /mainnet/seed/blocks/*
  sudo rm -rf /mainnet/santiago/data/* /mainnet/ivan/data/* && echo "wiped ai3 producer data"'

# ai4 — wipe mainnet node data
ssh $USER@<ai4-ip> '
  for N in n6 n7 n8; do
    sudo rm -rf /mainnet/$N/data/* && echo "wiped /mainnet/$N/data"
  done'

# ai5 — wipe mainnet node data
ssh $USER@<ai5-ip> '
  for N in n9 n10 n11 n12; do
    sudo rm -rf /mainnet/$N/data/* && echo "wiped /mainnet/$N/data"
  done'
```

#### Phase 4: Start Nodes (Ordered)

Start seeds first (all three servers), wait for them to peer, then start producers.

```bash
# Step 1: Start ALL seeds on ai1, ai2, ai3
ssh $USER@<ai1-ip> 'sudo systemctl start doli-mainnet-seed' &
ssh $USER@<ai2-ip> 'sudo systemctl start doli-mainnet-seed' &
ssh $USER@<ai3-ip> 'sudo systemctl start doli-mainnet-seed' &
wait

# Wait 10 seconds for seeds to initialize and peer with each other
sleep 10

# Step 2: Start ALL producers on ALL servers (parallel)
ssh $USER@<ai1-ip> 'sudo systemctl start doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3' &
ssh $USER@<ai2-ip> 'sudo systemctl start doli-mainnet-n4 doli-mainnet-n5' &
ssh $USER@<ai3-ip> 'sudo systemctl start doli-mainnet-santiago doli-mainnet-ivan' &
ssh $USER@<ai4-ip> 'sudo systemctl start doli-mainnet-n6 doli-mainnet-n7 doli-mainnet-n8' &
ssh $USER@<ai5-ip> 'sudo systemctl start doli-mainnet-n9 doli-mainnet-n10 doli-mainnet-n11 doli-mainnet-n12' &
wait
```

#### Phase 5: Verify Consensus

Wait ~30 seconds, then confirm all nodes are on the same chain.

```bash
# Check all seeds (ai1, ai2, ai3)
for ip in <ai1-ip> <ai2-ip> <ai3-ip>; do
  echo "--- Seed @ $ip ---"
  curl -s --max-time 3 -X POST http://$ip:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
  echo ""
done

# Check all producers per server
# ai1: N1-N3
ssh $USER@<ai1-ip> '
for entry in "8501:N1" "8502:N2" "8503:N3"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-10s %s\n" "$name" "$h"
done'

# ai2: N4-N5
ssh $USER@<ai2-ip> '
for entry in "8504:N4" "8505:N5"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-10s %s\n" "$name" "$h"
done'

# ai3: SANTIAGO + IVAN
ssh $USER@<ai3-ip> '
for entry in "8513:SANTIAGO" "8514:IVAN"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-10s %s\n" "$name" "$h"
done'

# ai4: N6-N8
ssh $USER@<ai4-ip> '
for entry in "8506:N6" "8507:N7" "8508:N8"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-10s %s\n" "$name" "$h"
done'

# ai5: N9-N12
ssh $USER@<ai5-ip> '
for entry in "8509:N9" "8510:N10" "8511:N11" "8512:N12"; do
  port=${entry%%:*}; name=${entry##*:}
  h=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" \
    | grep -oP "\"bestHeight\":\d+" 2>/dev/null || echo "unreachable")
  printf "%-10s %s\n" "$name" "$h"
done'

# All heights should be within +/-1 of each other
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
