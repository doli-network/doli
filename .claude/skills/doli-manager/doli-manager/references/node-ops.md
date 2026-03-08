# Node Operations & Deployment

## Table of Contents
- Running a Node
- Node Configuration
- Mainnet Infrastructure
- Deployment Procedure
- Wipe & Resync
- Upgrade via GitHub
- Rolling vs Full Restart

## Running a Node

### Non-Producer (Sync Only)

```bash
# Mainnet
./target/release/doli-node run --yes

# Testnet
./target/release/doli-node --network testnet run --yes

# Custom data dir
./target/release/doli-node --data-dir /data/doli run --yes
```

### Producer Node

```bash
./target/release/doli-node --data-dir ~/.doli/mainnet/data run \
  --producer --producer-key ~/.doli/mainnet/keys/producer.json \
  --chainspec ~/.doli/mainnet/chainspec.json \
  --no-auto-update --yes --force-start
```

### Production (systemd)

All production nodes are managed by systemd. **NEVER use nohup.**

```bash
sudo systemctl start doli-mainnet-nodeN
sudo systemctl stop doli-mainnet-nodeN
sudo systemctl restart doli-mainnet-nodeN
sudo systemctl status doli-mainnet-nodeN
```

## Node Configuration

### doli-node Global Flags (BEFORE subcommand)

| Flag | Default | Description |
|------|---------|-------------|
| `--network <NET>` | mainnet | mainnet, testnet, devnet |
| `--data-dir <PATH>` | network default | Data directory |
| `--log-level <LVL>` | info | trace, debug, info, warn, error |

### `run` Subcommand Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--producer` | off | Enable block production |
| `--producer-key <PATH>` | - | Key file for producing |
| `--p2p-port <PORT>` | network default | P2P listen port |
| `--rpc-port <PORT>` | network default | RPC listen port |
| `--metrics-port <PORT>` | 9090 | Prometheus metrics |
| `--bootstrap <MULTIADDR>` | DNS seeds | Custom bootstrap |
| `--chainspec <PATH>` | embedded | Chainspec file |
| `--relay-server` | off | Enable relay mode |
| `--force-start` | off | Skip duplicate key check |
| `--no-auto-update` | off | Disable auto-updates |
| `--yes` | off | Skip confirmations |

## Infrastructure (v2 — March 8, 2026)

2-server HA setup. Odd nodes on ai1, even on ai2. Seeds separated from producers.

| Server | IP | SSH | Hosts |
|--------|-----|-----|-------|
| ai1 | 72.60.228.233 | `ssh ilozada@72.60.228.233` | ODD nodes (N1,N3,N5,N7,N9,N11 + NT1,NT3,NT5,NT7,NT9,NT11) + seeds |
| ai2 | 187.124.95.188 | `ssh ilozada@187.124.95.188` | EVEN nodes (N2,N4,N6,N8,N10,N12 + NT2,NT4,NT6,NT8,NT10,NT12) + seeds |

**Port formula**: Mainnet P2P=30300+N, RPC=8500+N, Metrics=9000+N. Testnet P2P=40300+N, RPC=18500+N, Metrics=19000+N. Seeds use +0.

**RPC**: Producers bind 127.0.0.1. Seeds bind 0.0.0.0.

### Binaries (shared, not per-node)

| Network | doli-node | doli (CLI) |
|---------|-----------|------------|
| Mainnet | `/mainnet/bin/doli-node` | `/mainnet/bin/doli` |
| Testnet | `/testnet/bin/doli-node` | `/testnet/bin/doli` |

### Paths

| Item | Path |
|------|------|
| Node dir | `/mainnet/n{N}/` or `/testnet/nt{N}/` |
| Data dir | `/mainnet/n{N}/data/` or `/testnet/nt{N}/data/` |
| Keys | `/mainnet/n{N}/keys/producer.json` or `/testnet/nt{N}/keys/producer.json` |
| Key backups | `/mainnet/keys/producer_{N}.json` or `/testnet/keys/nt{N}.json` (all 12 on BOTH servers) |
| Logs | `/var/log/doli/mainnet/n{N}.log` or `/var/log/doli/testnet/nt{N}.log` |

### CRITICAL: Node Placement

- ai1 must NEVER have even node dirs. ai2 must NEVER have odd node dirs.
- Creating a node dir on the wrong server risks double-spending and slashing.
- Key backups in `/mainnet/keys/` and `/testnet/keys/` are safe (just files, no services).

### Producers & Maintainers

- N1-N5: mainnet producers + maintainers (3-of-5 governance)
- N6-N12: mainnet producers only
- NT1-NT5: testnet producers + maintainers
- NT6-NT12: testnet producers only
- All dirs owned by `ilozada:doliadmin` (GID 2000), permissions `2770`

## Deployment Procedure

### Step 1: Compile on ai2

```bash
ssh ilozada@187.124.95.188 'cd ~/repos/doli && git pull && export PATH="$HOME/.cargo/bin:$PATH" && cargo build --release'
```

ai2 does NOT have nix — use cargo directly.

### Step 2: Record md5

```bash
ssh ilozada@187.124.95.188 'md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli'
```

### Step 3: Deploy on ai2

```bash
ssh ilozada@187.124.95.188 'sudo cp ~/repos/doli/target/release/doli-node /mainnet/bin/doli-node && \
  sudo cp ~/repos/doli/target/release/doli /mainnet/bin/doli && \
  sudo cp ~/repos/doli/target/release/doli-node /testnet/bin/doli-node && \
  sudo cp ~/repos/doli/target/release/doli /testnet/bin/doli'
```

### Step 4: Transfer to ai1 and deploy

```bash
# Transfer via ssh pipe
ssh ilozada@187.124.95.188 'cat ~/repos/doli/target/release/doli-node' | \
  ssh ilozada@72.60.228.233 'cat > /tmp/doli-node && chmod +x /tmp/doli-node'
ssh ilozada@187.124.95.188 'cat ~/repos/doli/target/release/doli' | \
  ssh ilozada@72.60.228.233 'cat > /tmp/doli && chmod +x /tmp/doli'

# Deploy on ai1
ssh ilozada@72.60.228.233 'sudo cp /tmp/doli-node /mainnet/bin/doli-node && \
  sudo cp /tmp/doli /mainnet/bin/doli && \
  sudo cp /tmp/doli-node /testnet/bin/doli-node && \
  sudo cp /tmp/doli /testnet/bin/doli'
```

### Step 5: Verify md5

```bash
ssh ilozada@72.60.228.233 'md5sum /mainnet/bin/doli-node /testnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli'
ssh ilozada@187.124.95.188 'md5sum /mainnet/bin/doli-node /testnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli'
```

All 4 must match the build output from Step 2.

### Step 6: Restart services

Seeds first, then producers. Use systemd only.

```bash
# Seeds
ssh ilozada@72.60.228.233 'sudo systemctl restart doli-mainnet-seed doli-testnet-seed'
ssh ilozada@187.124.95.188 'sudo systemctl restart doli-mainnet-seed doli-testnet-seed'

# Producers
ssh ilozada@72.60.228.233 'sudo systemctl restart doli-mainnet-n{1,3,5} doli-testnet-nt{1,3,5}'
ssh ilozada@187.124.95.188 'sudo systemctl restart doli-mainnet-n{2,4,6} doli-testnet-nt{2,4,6}'
```

### Step 7: Verify

```bash
# Check all nodes via getChainInfo (JSON-RPC POST to http://127.0.0.1:<port>/)
# Run twice 15s apart to confirm height advancing
```

## Wipe & Resync

When a node is forked or corrupted, or for a full chain reset:

**CRITICAL**: Services use `--data-dir <node>/data`. Runtime data lives in `<node>/data/`, NOT `<node>/` top level.

```bash
# 1. Stop the node
sudo systemctl stop doli-mainnet-n{N}

# 2. Wipe data/ subdirectory (PRIMARY — this is where signed_slots.db lives)
find /mainnet/n{N}/data -mindepth 1 -delete

# 3. Also clean top-level stale files from older layouts
rm -f /mainnet/n{N}/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin}
rm -rf /mainnet/n{N}/{blocks,signed_slots.db,utxo_rocks,state_db}

# 4. Restart - node will resync from peers
sudo systemctl start doli-mainnet-n{N}
```

**WARNING**: If `signed_slots.db` inside `data/` is not wiped during a genesis reset, nodes will hit SLASHING PROTECTION and refuse to produce blocks. This is the #1 chain reset failure mode.

## Upgrade via GitHub

```bash
doli-node upgrade --yes                    # latest
doli-node upgrade --version 0.3.0 --yes   # specific version
```

## Rolling vs Full Restart

| Change Type | Strategy |
|-------------|----------|
| Consensus-critical (validation, scheduling, VDF, economics) | Stop ALL, deploy, start all |
| Non-consensus (sync, networking, RPC, logging) | Rolling: one at a time, verify health |

## Seed / Archive Nodes

Seeds double as archive + relay nodes. One on each server for redundancy.

| Network | Server | Service | P2P | RPC | Data |
|---------|--------|---------|-----|-----|------|
| Mainnet | ai1 | `doli-mainnet-seed` | 30300 | 8500 | `/mainnet/seed/data/` |
| Mainnet | ai2 | `doli-mainnet-seed` | 30300 | 8500 | `/mainnet/seed/data/` |
| Testnet | ai1 | `doli-testnet-seed` | 40300 | 18500 | `/testnet/seed/data/` |
| Testnet | ai2 | `doli-testnet-seed` | 40300 | 18500 | `/testnet/seed/data/` |

DNS: `seed1.doli.network` + `seed2.doli.network` (round-robin both IPs). `archive.doli.network` (round-robin).

## Snap Sync

Nodes >1000 blocks behind with 3+ peers use snap sync: download full state snapshot instead of replaying blocks. Takes seconds vs hours. Logs prefixed `[SNAP_SYNC]`.
