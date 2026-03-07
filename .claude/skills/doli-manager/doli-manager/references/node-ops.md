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

## Mainnet Infrastructure (6 Nodes)

| Node | Host | SSH | Ports (P2P/RPC/Metrics) |
|------|------|----|------------------------|
| N1 | 72.60.228.233 | `ssh ilozada@72.60.228.233` | 30303/8545/9090 |
| N2 | 72.60.228.233 | same host | 30304/8546/9091 |
| N3 | 147.93.84.44 | `ssh -p 50790 ilozada@147.93.84.44` (direct from Mac) | 30303/8545/9090 |
| N4 | 72.60.115.209 | `ssh -p 50790 ilozada@72.60.115.209` (direct from Mac) | 30303/8545/9090 |
| N5 | 72.60.70.166 | `ssh -p 50790 ilozada@72.60.70.166` (direct from Mac) | 30303/8545/9090 |
| N6 | 72.60.228.233 | same host | 30305/8547/9092 |

N1/N2 RPC is externally accessible (--rpc-bind 0.0.0.0). N3/N4/N5/N6 RPC is localhost-only — must SSH in first.

### Binaries

| Node | doli-node | doli (CLI) | Rust? |
|------|-----------|------------|-------|
| N1/N2/N6 | `~/repos/doli/target/release/doli-node` | `~/repos/doli/target/release/doli` | Yes |
| N3 | `/home/ilozada/doli-node` | `/home/ilozada/doli` | No (SCP from Mac) |
| N4/N5 | `/opt/doli/target/release/doli-node` | `/usr/local/bin/doli` | No (SCP from Mac) |

### Wallet Keys

| Node | Key File | Process User |
|------|----------|--------------|
| N1 | `~/.doli/mainnet/keys/producer_1.json` | ilozada |
| N2 | `~/.doli/mainnet/keys/producer_2.json` | ilozada |
| N3 | `~/.doli/mainnet/keys/producer_3.json` | ilozada |
| N4 | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` | isudoajl |
| N5 | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` | isudoajl |
| N6 | `~/.doli/mainnet/keys/producer_6.json` | ilozada |

> **⚠️ N4/N5 keys are swapped**: N4 runs `producer_5.json`, N5 runs `producer_4.json`. This is intentional — do NOT "fix" it.

### Connectivity

- N1/N2/N6: on omegacortex, accessed directly
- N3/N4/N5: SSH direct from Mac as `ilozada` on port 50790. **omegacortex CANNOT reach these nodes!**
- N4/N5 SSH aliases: `fpx` (N4), `kv1-fpx` (N5) — but these use user `isudoajl` with passphrase key, prefer explicit `ssh -p 50790 ilozada@<IP>`

## Deployment Procedure

### Step 1: Build on omegacortex

```bash
ssh ilozada@72.60.228.233 "cd ~/repos/doli && git pull && cargo build --release"
```

### Step 2: Deploy to N3/N4/N5

Deploy binaries from Mac directly (omegacortex cannot reach these nodes):

```bash
# First, get binary from omegacortex to Mac
scp ilozada@72.60.228.233:~/repos/doli/target/release/doli-node /tmp/doli-node

# Deploy to N3
scp -P 50790 /tmp/doli-node ilozada@147.93.84.44:~/doli-node

# Deploy to N4
scp -P 50790 /tmp/doli-node ilozada@72.60.115.209:/tmp/
ssh -p 50790 ilozada@72.60.115.209 'sudo cp /tmp/doli-node /opt/doli/target/release/doli-node && sudo chmod +x /opt/doli/target/release/doli-node'

# Deploy to N5
scp -P 50790 /tmp/doli-node ilozada@72.60.70.166:/tmp/
ssh -p 50790 ilozada@72.60.70.166 'sudo cp /tmp/doli-node /opt/doli/target/release/doli-node && sudo chmod +x /opt/doli/target/release/doli-node'
```

### Step 3: Stop nodes

```bash
# N1/N2/N6 (omegacortex - use systemd, NOT kill)
ssh ilozada@72.60.228.233 "sudo systemctl stop doli-mainnet-node1"

# N4/N5 (direct from Mac)
ssh -p 50790 ilozada@72.60.115.209 'sudo kill $(pgrep doli-node) 2>/dev/null; echo done'  # N4
ssh -p 50790 ilozada@72.60.70.166 'sudo kill $(pgrep doli-node) 2>/dev/null; echo done'   # N5
```

### Step 4: Start nodes

Start N1 first (bootstrap), then N2, then N3/N4/N5. See CLAUDE.md for exact commands per node.

### Step 5: Verify

```bash
# All same height?
ssh ilozada@72.60.228.233 "for p in 8545 8546 8547; do \
  echo \"N\$((p-8544)): \$(curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result | {h: .bestHeight, s: .bestSlot, hash: .bestHash[0:16]}')\"; done"
```

Run twice 15s apart to confirm height advancing.

## Wipe & Resync

When a node is forked or corrupted:

```bash
# 1. Stop the node
# 2. Delete state files (NOT keys or chainspec)
rm -f chain_state.bin producers.bin utxo.bin
rm -rf blocks/ signed_slots.db/

# 3. Restart - node will resync from peers
```

**N4/N5 paths**: `/home/isudoajl/.doli/mainnet/` (no `data/` subdir, process user `isudoajl`)
**N1/N2/N6 paths**: `~/.doli/mainnet/nodeN/data/` (on omegacortex)
**N3 paths**: `~/.doli/mainnet/data/`

### Balance Check Commands

**IMPORTANT**: The CLI on omegacortex MUST use `~/repos/doli/target/release/doli` (current build).
The old binary at `~/doli/target/release/doli` is from Jan 26 and does NOT support bech32m — it will silently return 0.
The CLI needs `-r http://127.0.0.1:<RPC_PORT>` to connect (default RPC endpoint `seed1.doli.network` does not resolve).

```bash
# N1 (omegacortex — CLI needs -r flag)
ssh ilozada@72.60.228.233 "~/repos/doli/target/release/doli -w ~/.doli/mainnet/keys/producer_1.json -r http://127.0.0.1:8545 balance"

# N2 (omegacortex)
ssh ilozada@72.60.228.233 "~/repos/doli/target/release/doli -w ~/.doli/mainnet/keys/producer_2.json -r http://127.0.0.1:8545 balance"

# N6 (omegacortex)
ssh ilozada@72.60.228.233 "~/repos/doli/target/release/doli -w ~/.doli/mainnet/keys/producer_6.json -r http://127.0.0.1:8545 balance"

# N3 (producer_3 key also on omegacortex)
ssh ilozada@72.60.228.233 "~/repos/doli/target/release/doli -w ~/.doli/mainnet/keys/producer_3.json -r http://127.0.0.1:8545 balance"

# N4/N5: keys not on omegacortex — use RPC with bech32m addresses instead:
# N4 (producer_5): doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc
# N5 (producer_4): doli1eduw95x5c6erx4dpacpfm90dylhjvjjn43j3nwag3huym6d20sdqzcqyq6
curl -s -X POST http://72.60.228.233:8545 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc"},"id":1}'
```

**CRITICAL**: `getBalance` MUST use bech32m addresses (`doli1...`), NOT raw public keys.
Raw 64-char public keys return 0 because UTXOs are keyed by `BLAKE3("DOLI_ADDR_V1" || pubkey)`.

#### All bech32m addresses (mainnet genesis producers)
```
N1 (producer_1): doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef
N2 (producer_2): doli12uaj6e7nkl90ry9q2ze27la7w0cg23ny7zk5csyj7ffrlcttcansfzx4mz
N3 (producer_3): doli109t8uyux22qqrx9ewzrpxww25scjt5cl49cunkn6m72me2txrgpsqd3rql
N4 (producer_5): doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc
N5 (producer_4): doli1eduw95x5c6erx4dpacpfm90dylhjvjjn43j3nwag3huym6d20sdqzcqyq6
N6 (producer_6): doli1dy5scma8lrc5uyez7pyhpq7q7xeakyzyyc5xrrfyuusgvzkakh9swnrr0s
```

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

## Archiver Node

A non-producing archive node runs on omegacortex as `doli-mainnet-archiver`:

| Service | Ports (P2P/RPC/Metrics) | Archive Dir |
|---------|------------------------|-------------|
| `doli-mainnet-archiver` | 30306/8548/9093 | `~/.doli/mainnet/archive/` |

- **DNS**: `archive.doli.network` → 72.60.228.233
- **Bootstrap peer**: included in mainnet bootstrap list (P2P port 30306)

Archives finality-gated blocks (only blocks confirmed by 67% attestation threshold). Each block stored as `{height:010}.block` with `.blake3` checksum sidecar. `manifest.json` tracks latest archived height.

```bash
# Check archiver status
sudo systemctl status doli-mainnet-archiver

# Check latest archived block
cat ~/.doli/mainnet/archive/manifest.json

# Restore from archive (disaster recovery)
doli-node restore --archive-dir ~/.doli/mainnet/archive/ --data-dir ~/.doli/mainnet/nodeN/data/
# Then: doli-node recover --yes --data-dir ~/.doli/mainnet/nodeN/data/
```

## Snap Sync

Nodes >1000 blocks behind with 3+ peers use snap sync: download full state snapshot instead of replaying blocks. Takes seconds vs hours. Logs prefixed `[SNAP_SYNC]`.
