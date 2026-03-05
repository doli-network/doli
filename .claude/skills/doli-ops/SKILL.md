---
name: doli-ops
description: Operational runbook for DOLI infrastructure. MUST read before any deployment, node management, upgrade, or infrastructure task. Triggers on "deploy", "upgrade node", "restart node", "node status", "wipe node", "resync", "rollback", "release", "bond", "producer register", "ssh", "kill node", "stop node", "start node".
---

# DOLI Operations Runbook

**READ THIS BEFORE any infrastructure or deployment task.**

---

## Section 1: CLI Commands Reference

### 1.1 doli (Wallet CLI)

```
doli [OPTIONS] <COMMAND>
```

**Global options (BEFORE subcommand):**

| Flag | Default | Description |
|------|---------|-------------|
| `-w, --wallet <PATH>` | `~/.doli/wallet.json` | Wallet file path |
| `-r, --rpc <URL>` | `http://127.0.0.1:8545` | Node RPC endpoint |

**Commands:**

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `new` | Create a new wallet | `--name <NAME>` |
| `address` | Generate new address | `--label <LABEL>` |
| `addresses` | List all addresses | |
| `balance` | Show wallet balance | `--address <ADDR>` |
| `send <TO> <AMOUNT>` | Send coins | `--fee <FEE>` |
| `history` | Transaction history | `--limit <N>` (default 10) |
| `export <OUTPUT>` | Export wallet to file | |
| `import <INPUT>` | Import wallet from file | |
| `info` | Show wallet info | |
| `sign <MESSAGE>` | Sign a message | `--address <ADDR>` |
| `verify <MSG> <SIG> <PUBKEY>` | Verify signature | |
| `chain` | Show chain info | |

**Producer subcommands** (`doli producer <CMD>`):

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `register` | Register as producer | `-b, --bonds <N>` (default 1, range 1-10000) |
| `status` | Check producer status | `-p, --pubkey <KEY>` |
| `list` | List all producers | `--active` |
| `add-bond` | Add bonds to stake | `-c, --count <N>` (required, 1-10000) |
| `request-withdrawal` | Withdraw bonds (instant payout, FIFO) | `-c, --count <N>`, `-d, --destination <ADDR>` |
| `exit` | Exit producer set | `--force` (early exit with penalty) |
| `slash` | Submit equivocation evidence | |

**Rewards subcommands** (`doli rewards <CMD>`):

| Command | Description |
|---------|-------------|
| `list` | List claimable epochs |
| `claim` | Claim for specific epoch |
| `claim-all` | Claim all available |
| `history` | Show claim history |
| `info` | Current epoch info |

**Update governance** (`doli update <CMD>`):

| Command | Description |
|---------|-------------|
| `check` | Check for updates |
| `status` | Pending update status, veto progress |
| `vote` | Vote on pending update |
| `votes` | Show votes for a version |
| `apply` | Apply approved update |
| `rollback` | Rollback to previous version |

**Maintainer** (`doli maintainer <CMD>`):

| Command | Description |
|---------|-------------|
| `list` | List current maintainers |

**Examples:**
```bash
# Point CLI at testnet node
doli -r http://127.0.0.1:18545 chain

# Check balance on specific RPC
doli -r http://127.0.0.1:28545 balance

# Register producer with 5 bonds
doli -r http://127.0.0.1:8545 producer register -b 5

# Add 3 more bonds
doli -r http://127.0.0.1:8545 producer add-bond -c 3
```

### 1.2 doli-node

```
doli-node [OPTIONS] [COMMAND]
```

**CRITICAL: Global options go BEFORE the subcommand:**
```bash
# CORRECT:
doli-node --network testnet --data-dir /path run --producer

# WRONG:
doli-node run --network testnet --data-dir /path --producer
```

**Global options:**

| Flag | Default | Description |
|------|---------|-------------|
| `-n, --network <NET>` | `mainnet` | Network: mainnet, testnet, devnet |
| `-c, --config <PATH>` | `config.toml` | Config file |
| `-d, --data-dir <PATH>` | network default | Data directory override |
| `--log-level <LEVEL>` | `info` | Log level |

**Commands:**

| Command | Description |
|---------|-------------|
| `run` | Run the node (main command) |
| `init` | Initialize data directory |
| `status` | Show node status |
| `import` | Import blocks from file |
| `export` | Export blocks to file |
| `recover` | Rebuild chain state from block data |
| `upgrade` | Download and install latest release from GitHub |
| `devnet` | Local devnet management |
| `update` | Update governance commands |
| `maintainer` | Maintainer management |
| `release` | Release signing (maintainers only) |

**`run` subcommand flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--producer` | off | Enable block production |
| `--producer-key <PATH>` | | Producer key file |
| `--no-auto-update` | off | Disable auto-updates |
| `--update-notify-only` | off | Notify only, don't apply |
| `--no-auto-rollback` | off | Disable rollback on failures |
| `--p2p-port <PORT>` | network default | P2P listen port |
| `--rpc-port <PORT>` | network default | RPC listen port |
| `--metrics-port <PORT>` | 9090 | Metrics port |
| `--bootstrap <MULTIADDR>` | | Bootstrap node address |
| `--no-dht` | off | Disable DHT discovery |
| `--force-start` | off | Skip duplicate key check (DANGEROUS) |
| `--yes` | off | Skip confirmations |
| `--chainspec <PATH>` | | Custom chainspec JSON |
| `--external-address <MULTIADDR>` | | Public address to advertise (e.g., `/ip4/72.60.228.233/tcp/30303`) |
| `--relay-server` | off | Enable relay server for NAT'd peers |

**`recover` subcommand:**
```bash
# Rebuild UTXO/producer/chain state from block data after corruption
doli-node --network mainnet recover --yes
```

**`upgrade` subcommand:**
```bash
# Upgrade to latest release (downloads, verifies SHA256, replaces binary)
doli-node upgrade --yes
# Upgrade to specific version
doli-node upgrade --version 0.2.0 --yes
```

**`devnet` subcommands:**

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `devnet init` | Init local devnet | `--nodes <1-20>` (default 3) |
| `devnet start` | Start all devnet nodes | |
| `devnet stop` | Stop all devnet nodes | |
| `devnet status` | Show devnet status | |
| `devnet clean` | Remove devnet data | `--keep-keys` |
| `devnet add-producer` | Add producer to running devnet | `--count`, `--bonds`, `--fund-amount` |

### 1.3 Network Ports

| Network | ID | P2P | RPC | Metrics | Data Dir |
|---------|-----|------|------|---------|----------|
| Mainnet | 1 | 30303 | 8545 | 9090 | `~/.doli/mainnet/` |
| Testnet | 2 | 40303 | 18545 | 19090 | `~/.doli/testnet/` |
| Devnet | 99 | 50303 | 28545 | 29090 | `~/.doli/devnet/` |

### 1.4 RPC API Reference

> **CRITICAL: The RPC is POST-only JSON-RPC.** All endpoints go through `POST /` with a JSON body. GET requests return `405 Method Not Allowed`.

**Request format:**
```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"METHOD_NAME","params":{},"id":1}' \
  http://127.0.0.1:PORT/
```

**Available methods:**

| Method | Description | Key response fields |
|--------|-------------|-------------------|
| `getChainInfo` | Chain height, slot, genesis | `bestHeight`, `bestSlot`, `bestHash`, `genesisHash`, `network`, `version` |
| `getNodeInfo` | Node info | `version`, `peerCount`, `peerId`, `network`, `platform`, `arch` |
| `getNetworkInfo` | Network details | `peerCount` |
| `getBalance` | Address balance (use CLI instead — more reliable) | `confirmed`, `unconfirmed`, `immature`, `total` |
| `getBlockByHeight` | Block by height | `params: {"height": N}` |
| `getBlockByHash` | Block by hash | `params: {"hash": "..."}` |
| `getTransaction` | Transaction details | `params: {"hash": "..."}` |
| `sendTransaction` | Submit raw TX | |
| `getUtxos` | UTXOs for address | `params: {"address": "..."}` |
| `getMempoolInfo` | Mempool status | |
| `getProducer` | Producer info | `params: {"address": "..."}` |
| `getProducers` | All producers | |
| `getHistory` | TX history for address | `params: {"address": "..."}` |
| `getEpochInfo` | Current epoch details | |
| `getNetworkParams` | Network parameters | |
| `getBondDetails` | Per-bond vesting info | `params: {"address": "..."}` |
| `getUpdateStatus` | Auto-update status | |
| `getMaintainerSet` | Current maintainers | |

**Quick health check (all nodes):**
```bash
# Single node
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
  http://127.0.0.1:8545/ | jq -c '.result | {h: .bestHeight, s: .bestSlot}'

# All mainnet nodes on omegacortex
for port in 8545 8546 8547; do
  echo -n "port=$port: "
  curl -s --max-time 3 -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
    http://127.0.0.1:$port/ | jq -c '.result | {h: .bestHeight, s: .bestSlot}'
done
```

**NOTE**: `peerCount` from `getNodeInfo` may report 0 even when nodes are synced and producing. This is a known reporting issue — do NOT rely on it. Use `bestHeight` from `getChainInfo` to verify nodes are in sync.

---

## Section 2: Node Operations

### 2.1 Infrastructure

#### Server 1: omegacortex.ai (Node 1 + Node 2)

| Property | Value |
|----------|-------|
| SSH | `ssh ilozada@omegacortex.ai` |
| IP | 72.60.228.233 |
| Binary | `/home/ilozada/repos/doli/target/release/doli-node` |
| Chainspec | `/home/ilozada/.doli/mainnet/chainspec.json` |
| Managed by | **systemd** |

**Node 1:**

| Property | Value |
|----------|-------|
| Service | `doli-mainnet-node1` |
| Service file | `/etc/systemd/system/doli-mainnet-node1.service` |
| Data | `/home/ilozada/.doli/mainnet/node1/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_1.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/var/log/doli/node1.log` |

**Node 2:**

| Property | Value |
|----------|-------|
| Service | `doli-mainnet-node2` |
| Service file | `/etc/systemd/system/doli-mainnet-node2.service` |
| Data | `/home/ilozada/.doli/mainnet/node2/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_2.json` |
| P2P | 30304 |
| RPC | 8546 |
| Metrics | 9091 |
| Logs | `/var/log/doli/node2.log` |
| Bootstrap | Node 1 via `/ip4/127.0.0.1/tcp/30303` |

#### Server 2: 147.93.84.44 (Node 3 — partner)

| Property | Value |
|----------|-------|
| SSH | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@147.93.84.44` |
| IP | 147.93.84.44 |
| Specs | 1 CPU, 3.8GB RAM, Ubuntu 24.04 |
| Binary | `/home/ilozada/doli-node` |
| Service | `doli-mainnet-node3` |
| Service file | `/etc/systemd/system/doli-mainnet-node3.service` |
| Data | `/home/ilozada/.doli/mainnet/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_3.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/var/log/doli/node3.log` |
| Bootstrap | Node 1 via `/ip4/72.60.228.233/tcp/30303` |
| Git repo | **None** — standalone binary, updated via SCP from omegacortex.ai |
| Managed by | **systemd** |

#### Server 3: 72.60.70.166 (Node 4 — "pro-KVM1")

| Property | Value |
|----------|-------|
| SSH | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.70.166` |
| Hostname | pro-KVM1 |
| IP | 72.60.70.166 |
| User (node) | `isudoajl` |
| Binary | `/opt/doli/target/release/doli-node` |
| Service | `doli-mainnet-node4` |
| Service file | `/etc/systemd/system/doli-mainnet-node4.service` |
| Git repo | `/opt/doli` (owner: `isudoajl`, pull with `sudo -u isudoajl`) |
| Data | `/home/isudoajl/.doli/mainnet/` (no `data/` subdir) |
| Key | `/home/isudoajl/.doli/mainnet/producer.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/var/log/doli/node4.log` |
| Bootstrap | Node 1 via `/ip4/72.60.228.233/tcp/30303` |
| Managed by | **systemd** |

#### Server 4: 72.60.115.209 (Node 5 — "fpx")

| Property | Value |
|----------|-------|
| SSH | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.115.209` |
| Hostname | fpx |
| IP | 72.60.115.209 |
| User (node) | `isudoajl` |
| Binary | `/opt/doli/target/release/doli-node` |
| Service | `doli-mainnet-node5` |
| Service file | `/etc/systemd/system/doli-mainnet-node5.service` |
| Git repo | `/opt/doli` (owner: `isudoajl`, pull with `sudo -u isudoajl`) |
| Data | `/home/isudoajl/.doli/mainnet/` (no `data/` subdir) |
| Key | `/home/isudoajl/.doli/mainnet/producer.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/var/log/doli/node5.log` |
| Bootstrap | Node 1 via `/ip4/72.60.228.233/tcp/30303` |
| Managed by | **systemd** |

### 2.2 Build & Deploy Strategy

**Build once on omegacortex, distribute binaries everywhere.** Never build on remote nodes — they lack reliable Rust toolchains.

**Build:**
```bash
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && source ~/.cargo/env && \
    cargo build --release --package doli-node --package doli-cli"
```

**Source binaries:** `~/repos/doli/target/release/doli-node` and `~/repos/doli/target/release/doli`

**Binary paths per server:**

| Server | Binary location | Deploy method |
|--------|----------------|---------------|
| omegacortex (N1, N2, N6) | `~/repos/doli/target/release/doli-node` | Built in place |
| N3 (147.93.84.44) | `~/doli-node`, `~/doli` | SCP from omegacortex via `/tmp/` staging |
| N4 (72.60.70.166) | `/opt/doli/target/release/doli-node` | SCP from omegacortex via `/tmp/`, `sudo cp` |
| N5 (72.60.115.209) | `/opt/doli/target/release/doli-node` | SCP from omegacortex via `/tmp/`, `sudo cp` |

**MD5 verify every copy:**
```bash
# Source hashes (record these)
ssh ilozada@omegacortex.ai "md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli"

# Verify on each destination after copy
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@<IP> 'md5sum <binary_path>'"
```

**SCP commands:**
```bash
# N3 (no sudo needed, home dir — SCP from Mac, not omegacortex)
scp -P 50790 /tmp/doli-node /tmp/doli ilozada@147.93.84.44:/tmp/
ssh -p 50790 ilozada@147.93.84.44 'cp /tmp/doli-node ~/doli-node && cp /tmp/doli ~/doli && chmod +x ~/doli-node ~/doli'

# N4 (sudo needed for /opt/doli/)
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.70.166:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo cp /tmp/doli-node /tmp/doli /opt/doli/target/release/ && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"

# N5 (sudo needed for /opt/doli/)
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.115.209:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo cp /tmp/doli-node /tmp/doli /opt/doli/target/release/ && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"
```

**CRITICAL:** Use atomic rename (`cp new /path/binary.new && mv /path/binary.new /path/binary`) to avoid `Text file busy` errors. Direct `cp` over a running binary fails. See Section 3.7 for gotchas.

### 2.3 Starting Nodes

**ALL nodes are managed by systemd.** Never use `nohup` — it bypasses restart-on-failure, log rotation, and proper service lifecycle.

**Node 1 (omegacortex.ai):**
```bash
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node1"
```

**Node 2 (omegacortex.ai):**
```bash
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node2"
```

**Node 3 (partner — via jump host):**
```bash
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-mainnet-node3'"
```

**Node 4 (pro-KVM1 — via jump host):**
```bash
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-mainnet-node4'"
```

**Node 5 (fpx — via jump host):**
```bash
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl start doli-mainnet-node5'"
```

**Local devnet (for testing):**
```bash
cargo build --release
./target/release/doli-node devnet init --nodes 3
./target/release/doli-node devnet start
```

### 2.4 Stopping Nodes

**ALL nodes are managed by systemd.** `systemctl stop` sends SIGTERM for graceful shutdown.

```bash
# Node 1 (omegacortex.ai)
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node1"

# Node 2 (omegacortex.ai)
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node2"

# Node 3 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl stop doli-mainnet-node3'"

# Node 4 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl stop doli-mainnet-node4'"

# Node 5 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl stop doli-mainnet-node5'"

# Devnet (local only)
./target/release/doli-node devnet stop
```

**Restart a node:**
```bash
sudo systemctl restart doli-mainnet-nodeN
```

**NEVER use `kill -9` unless absolutely necessary** — it bypasses graceful shutdown and can create orphan blocks requiring shallow fork recovery.

### 2.5 Checking Node Status

**RPC health check:**
```bash
# Chain info (height, hash, slot)
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq .

# Network info (peer count, sync state)
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq .

# Producer list
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}' | jq .
```

**Service status (preferred):**
```bash
# omegacortex.ai (node1 + node2)
ssh ilozada@omegacortex.ai "sudo systemctl status doli-mainnet-node1 --no-pager | head -10"
ssh ilozada@omegacortex.ai "sudo systemctl status doli-mainnet-node2 --no-pager | head -10"

# node3 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl status doli-mainnet-node3 --no-pager | head -10'"

# node4 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl status doli-mainnet-node4 --no-pager | head -10'"

# node5 (via jump host)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl status doli-mainnet-node5 --no-pager | head -10'"
```

**Remote — quick chain status all 5 nodes:**
```bash
# N1+N2 on omegacortex
ssh ilozada@omegacortex.ai "for p in 8545 8546; do \
  echo \"N\$((p-8544)): \$(curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result | {h: .bestHeight, s: .bestSlot, hash: .bestHash[0:16]}')\"; done"

# N3
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'curl -s -X POST http://127.0.0.1:8545 \
    -H \"Content-Type: application/json\" \
    -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\"'"

# N4
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'curl -s -X POST http://127.0.0.1:8545 \
    -H \"Content-Type: application/json\" \
    -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\"'"

# N5
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'curl -s -X POST http://127.0.0.1:8545 \
    -H \"Content-Type: application/json\" \
    -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\"'"
```

**Tail logs:**
```bash
# Node 1
ssh ilozada@omegacortex.ai "tail -f /var/log/doli/node1.log"
# Node 2
ssh ilozada@omegacortex.ai "tail -f /var/log/doli/node2.log"
# Node 3 (via jump)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'tail -f /var/log/doli/node3.log'"
# Node 4 (via jump)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'tail -f /var/log/doli/node4.log'"
# Node 5 (via jump)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'tail -f /var/log/doli/node5.log'"
```

### 2.6 Checking Balances and Production

```bash
# Balance
doli -r http://127.0.0.1:PORT balance

# Producer status (uses wallet key)
doli -r http://127.0.0.1:PORT producer status

# Producer status (specific key)
doli -r http://127.0.0.1:PORT producer status -p <PUBKEY_HEX>

# All producers
doli -r http://127.0.0.1:PORT producer list

# Block at height
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":123},"id":1}' | jq .
```

### 2.7 Wipe and Resync

**Partial wipe (keep keys, resync chain):**
```bash
# Stop node first
sudo systemctl stop doli-mainnet-node1

# Remove chain data only (example for node1 on omegacortex.ai)
rm -rf ~/.doli/mainnet/node1/data/blocks/
rm -rf ~/.doli/mainnet/node1/data/signed_slots.db/
rm -f ~/.doli/mainnet/node1/data/chain_state.bin
rm -f ~/.doli/mainnet/node1/data/producers.bin
rm -f ~/.doli/mainnet/node1/data/utxo_set.bin

# Restart node — resyncs from peers (see Section 2.3 for exact command)
```

**State recovery (without wipe):**
```bash
# Rebuilds UTXO + producer + chain state from existing blocks
doli-node --network mainnet recover --yes
```

**Full wipe (devnet):**
```bash
./target/release/doli-node devnet clean
# Or keep keys:
./target/release/doli-node devnet clean --keep-keys
```

### 2.8 RocksDB LOCK File Cleanup

After a `kill -9`, RocksDB may leave stale LOCK files:
```bash
rm -f ~/.doli/<NETWORK>/data/node*/blocks/LOCK
rm -f ~/.doli/<NETWORK>/data/node*/signed_slots.db/LOCK
```

---

## Section 3: Deployment Procedure

### 3.1 Pre-Deploy Gates (ALL MUST PASS)

```bash
cargo build --release && \
cargo clippy -- -D warnings && \
cargo fmt --check && \
cargo test
```

Redirect verbose output:
```bash
cargo build --release > /tmp/build.log 2>&1 && \
    grep -iE "error|warn|fail" /tmp/build.log | head -20
```

### 3.2 Deployment Checklist

1. **Pre-flight**
   - [ ] All pre-deploy gates pass
   - [ ] Changes reviewed and approved by Ivan
   - [ ] Determine if consensus-critical (affects block validation, scheduling, VDF, economics)

2. **Build release binary**
   ```bash
   cargo build --release --package doli-node
   ```

3. **Deploy to node(s)** — see 3.3 or 3.4 below

4. **Post-deploy verification** — see 3.5

### 3.3 Consensus-Critical: Simultaneous Upgrade

Changes to block validation, scheduling, VDF, economics, or transaction processing **MUST** be deployed to all nodes simultaneously to prevent forks.

```bash
# 1. Build on omegacortex.ai (single build, distribute everywhere)
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && \
    source ~/.cargo/env && cargo build --release --package doli-node --package doli-cli"

# 2. MD5 source binaries (record these)
ssh ilozada@omegacortex.ai "md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli"

# 3. Stop ALL nodes (must stop before copying — "Text file busy" otherwise)
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node1 doli-mainnet-node2 doli-mainnet-node6"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo killall doli-node'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo killall doli-node'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo killall doli-node'"

# 4. SCP binaries to all remote servers via /tmp/ staging + verify MD5
# N3 (SCP from Mac, not omegacortex):
scp -P 50790 /tmp/doli-node /tmp/doli ilozada@147.93.84.44:/tmp/
ssh -p 50790 ilozada@147.93.84.44 'cp /tmp/doli-node ~/doli-node && cp /tmp/doli ~/doli && chmod +x ~/doli-node ~/doli && md5sum ~/doli-node ~/doli'
# N4:
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.70.166:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo cp /tmp/doli-node /tmp/doli /opt/doli/target/release/ && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli && md5sum /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"
# N5:
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.115.209:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo cp /tmp/doli-node /tmp/doli /opt/doli/target/release/ && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli && md5sum /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"

# 5. Start ALL nodes via systemd (N1 first = bootstrap)
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node1"
sleep 3
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node2"
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node6"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-mainnet-node3'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl start doli-mainnet-node5'"

# 6. Verify all nodes are on same chain — see 3.5
```

### 3.4 Non-Consensus: Rolling Upgrade

Network, sync, RPC, logging, or UI changes can be deployed one node at a time. **Preferred order: reverse (N6→N1)** — start with least critical, end with N1 (bootstrap).

If test nodes (NT) exist, deploy those first (NT18→NT1) as canaries.

**Strategy:** Build once on omegacortex → SCP binaries to all servers → rolling restart per node, verifying each advances before proceeding.

**Step-by-step (NT nodes first, then mainnet):**

```bash
# 0. Build + MD5 (see Section 2.2)
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && source ~/.cargo/env && \
    cargo build --release --package doli-node --package doli-cli"
ssh ilozada@omegacortex.ai "md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli"

# --- NT nodes (reverse: N5 → N4 → N3 → omegacortex) ---
# Each server: SCP binary → atomic rename install → restart NT services → verify

# NT14-18 (N5):
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.115.209:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo cp /tmp/doli-node /opt/doli/target/release/doli-node.new && sudo mv /opt/doli/target/release/doli-node.new /opt/doli/target/release/doli-node && sudo cp /tmp/doli /opt/doli/target/release/doli.new && sudo mv /opt/doli/target/release/doli.new /opt/doli/target/release/doli && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli && md5sum /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl restart doli-testnet-nt14 doli-testnet-nt15 doli-testnet-nt16 doli-testnet-nt17 doli-testnet-nt18'"
# Wait 15s, verify heights increasing

# NT9-13 (N4):
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli ilozada@72.60.70.166:/tmp/"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo cp /tmp/doli-node /opt/doli/target/release/doli-node.new && sudo mv /opt/doli/target/release/doli-node.new /opt/doli/target/release/doli-node && sudo cp /tmp/doli /opt/doli/target/release/doli.new && sudo mv /opt/doli/target/release/doli.new /opt/doli/target/release/doli && sudo chmod +x /opt/doli/target/release/doli-node /opt/doli/target/release/doli && md5sum /opt/doli/target/release/doli-node /opt/doli/target/release/doli'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl restart doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 doli-testnet-nt13'"
# Wait 15s, verify

# NT6-8 (N3 — direct SSH from Mac, not via omegacortex):
scp -P 50790 /tmp/doli-node /tmp/doli ilozada@147.93.84.44:/tmp/
ssh -p 50790 ilozada@147.93.84.44 'cp /tmp/doli-node ~/doli-node.new && mv ~/doli-node.new ~/doli-node && cp /tmp/doli ~/doli.new && mv ~/doli.new ~/doli && chmod +x ~/doli-node ~/doli && md5sum ~/doli-node ~/doli'
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl restart doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'
# Wait 15s, verify

# NT1-5 (omegacortex — binary already in place from build):
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5"
# Wait 15s, verify

# --- Mainnet nodes (reverse: N6 → N5 → N4 → N3 → N2 → N1) ---
# Binaries already copied above. Just restart via systemd + verify.
# WAIT for each node to sync and advance before proceeding to next.

# N6 (omegacortex, RPC 8547):
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node6"
# Wait 15s, verify:
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8547 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"

# N5 (72.60.115.209, RPC 8545) — binary already replaced when deploying NT14-18:
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl restart doli-mainnet-node5'"
# Wait 15s, verify:
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'curl -s -X POST http://127.0.0.1:8545 -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"'"

# N4 (72.60.70.166, RPC 8545):
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl restart doli-mainnet-node4'"
# Wait 15s, verify (same pattern as N5, IP 72.60.70.166)

# N3 (147.93.84.44, RPC 8545):
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl restart doli-mainnet-node3'"
# Wait 15s, verify (same pattern, IP 147.93.84.44)

# N2 (omegacortex, RPC 8546):
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node2"
# Wait 15s, verify:
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8546 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"

# N1 (omegacortex, RPC 8545) — LAST, it's the bootstrap node:
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node1"
# Wait 15s, verify:
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"
```

**RPC ports for verification:**

| Node | Server | RPC Port |
|------|--------|----------|
| N1 | omegacortex (direct) | 8545 |
| N2 | omegacortex (direct) | 8546 |
| N3 | 147.93.84.44 (via jump) | 8545 |
| N4 | 72.60.70.166 (via jump) | 8545 |
| N5 | 72.60.115.209 (via jump) | 8545 |
| N6 | omegacortex (direct) | 8547 |

### 3.5 Post-Deploy Verification

```bash
# 1. All mainnet services active
ssh ilozada@omegacortex.ai "sudo systemctl is-active doli-mainnet-node1 doli-mainnet-node2 doli-mainnet-node6"
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl is-active doli-mainnet-node3'
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl is-active doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl is-active doli-mainnet-node5'"

# 1b. All NT services active
ssh ilozada@omegacortex.ai "sudo systemctl is-active doli-testnet-nt{1..5}"
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl is-active doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl is-active doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 doli-testnet-nt13'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl is-active doli-testnet-nt14 doli-testnet-nt15 doli-testnet-nt16 doli-testnet-nt17 doli-testnet-nt18'"

# 2. Chain is advancing (run twice, 15s apart, height should increase)
ssh ilozada@omegacortex.ai "for p in 8545 8546; do \
  echo \"N\$((p-8544)): \$(curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result | {h: .bestHeight, s: .bestSlot, hash: .bestHash[0:16]}')\"; done"

# 3. No errors in recent logs
ssh ilozada@omegacortex.ai "tail -50 /var/log/doli/node1.log | grep -iE 'error|panic|fatal'"
ssh ilozada@omegacortex.ai "tail -50 /var/log/doli/node2.log | grep -iE 'error|panic|fatal'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'tail -50 /var/log/doli/node3.log | grep -iE \"error|panic|fatal\"'"

# 4. All nodes agree on chain tip
ssh ilozada@omegacortex.ai "for p in 8545 8546; do \
  curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result.bestHash[0:16]'; done"
```

### 3.6 Remote Build + Deploy (Single Command — omegacortex only)

```bash
# Build + restart node1+node2+node6 on omegacortex.ai (all share the same binary)
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && \
    source ~/.cargo/env && \
    cargo build --release --package doli-node --package doli-cli && \
    sudo systemctl restart doli-mainnet-node1 && sleep 3 && \
    sudo systemctl restart doli-mainnet-node2 && \
    sudo systemctl restart doli-mainnet-node6"
```

For N3/N4/N5, you MUST SCP binaries — see Section 2.2 and 3.4 for full procedure.

### 3.7 Deployment Gotchas (Lessons Learned)

1. **"Text file busy"** — Cannot `cp` over a running binary. Use atomic rename instead: `cp new /path/doli-node.new && mv /path/doli-node.new /path/doli-node`. The `doli upgrade` command does this automatically. Running processes keep the old inode.

2. **SCP to N3 `~/` may fail** — Use `/tmp/` as staging directory, then `cp` from `/tmp/` to final location.

3. **N4/N5 need `sudo` for `/opt/doli/`** — The binary path `/opt/doli/target/release/` is owned by `isudoajl`. Use `sudo cp` after SCP to `/tmp/`.

4. **NT nodes are individual systemd services** — Each NT has its own service (`doli-testnet-nt1` through `doli-testnet-nt18`). Use `sudo systemctl restart doli-testnet-nt<ID>` for per-node control. The old `manage.sh` bulk start/stop is deprecated.

5. **NT + mainnet on same server** — On N3/N4/N5 where both NT and mainnet nodes run, upgrade the binary once (atomic rename), then restart services individually. Never use `killall doli-node` — it kills everything.

6. **Restart order on shared servers** — Restart NT nodes first (canaries), verify they advance, then restart the mainnet node. This minimizes mainnet downtime.

7. **Never build on remote nodes** — N3 has no Rust toolchain. N4/N5 have stale `~/.cargo/env` paths that fail. Always build on omegacortex and distribute.

8. **MD5 verify every copy** — Always `md5sum` on source and destination. Binary corruption over SCP is rare but catastrophic on a blockchain node.

9. **SCP strips execute permissions** — `scp` copies files without the execute bit. Always `chmod +x` after copying binaries. Without this, systemd fails with `status=203/EXEC`. Every SCP command in this runbook includes `chmod +x` — if you improvise a copy, don't forget it.

### 3.8 `doli upgrade` — Standard Upgrade Procedure (v1.1.9+)

**This is the preferred upgrade method for all nodes.** It downloads the release tarball from GitHub, verifies SHA-256, installs both `doli` and `doli-node` binaries via atomic rename (safe even while nodes are running), and restarts the specified service.

#### 3.8.1 How `doli upgrade` works

1. Fetches release metadata from GitHub (`e-weil/doli`)
2. Downloads platform tarball (linux x86_64)
3. Verifies SHA-256 checksum from `CHECKSUMS.txt`
4. Checks maintainer signatures (informational — does not block manual upgrade)
5. Extracts and installs `doli` binary (to its own path via `current_exe()`)
6. Extracts and installs `doli-node` binary (auto-detected or `--doli-node-path`)
7. Restarts the specified service (`--service`) or auto-detects all doli services

**Atomic install**: Uses write-to-temp + `rename()`. Running processes keep the old binary via inode. No "text file busy" error. No need to stop nodes before installing.

#### 3.8.2 CLI Flags

```
doli upgrade [OPTIONS]

Options:
    --version <VERSION>                Target version (default: latest)
    --yes                              Skip confirmation prompt
    --doli-node-path <DOLI_NODE_PATH>  Custom path to doli-node binary
    --service <SERVICE>                Restart only this systemd service
```

**`--doli-node-path`**: Required on servers where `doli-node` is not in PATH or `/usr/local/bin/` or `/opt/doli/target/release/`. Without it, `find_doli_node_path()` checks: `which doli-node` → `/usr/local/bin/doli-node` → `/opt/doli/target/release/doli-node`.

**`--service`**: CRITICAL on multi-node servers. Without it, `restart_doli_service()` restarts ALL doli systemd services on the server (dangerous on omegacortex where N1+N2+N6 share one machine).

#### 3.8.3 Path & Flag Reference Per Server

| Server | doli binary | doli-node binary | `--doli-node-path` needed? | `--service` value | sudo needed? |
|--------|-------------|------------------|---------------------------|-------------------|--------------|
| omegacortex (N1) | `~/repos/doli/target/release/doli` | `~/repos/doli/target/release/doli-node` | **YES** | `doli-mainnet-node1` | No |
| omegacortex (N2) | same | same | **YES** | `doli-mainnet-node2` | No |
| omegacortex (N6) | same | same | **YES** | `doli-mainnet-node6` | No |
| N3 (147.93.84.44) | `~/doli` | `~/doli-node` | **YES** | `doli-mainnet-node3` | No |
| N4 (72.60.70.166) | `/opt/doli/target/release/doli` | `/opt/doli/target/release/doli-node` | No (in fallback) | `doli-mainnet-node4` | **YES** (`sudo`) |
| N5 (72.60.115.209) | `/opt/doli/target/release/doli` | `/opt/doli/target/release/doli-node` | No (in fallback) | `doli-mainnet-node5` | **YES** (`sudo`) |

**Exact `doli upgrade` commands per server:**
```bash
# N4 / N5 — auto-detects doli-node path, sudo needed for /opt/:
sudo /opt/doli/target/release/doli upgrade --version <VER> --yes --service doli-mainnet-node5

# N3 — doli-node at ~/doli-node (not in fallback chain):
~/doli upgrade --version <VER> --yes --doli-node-path ~/doli-node --service doli-mainnet-node3

# omegacortex — doli-node at ~/repos/... (not in fallback chain), one service at a time:
~/repos/doli/target/release/doli upgrade --version <VER> --yes \
  --doli-node-path ~/repos/doli/target/release/doli-node --service doli-mainnet-node6
# Then just restart N2 and N1 (binary already replaced):
sudo systemctl restart doli-mainnet-node2
sudo systemctl restart doli-mainnet-node1
```

**NT nodes**: Each NT node has its own systemd service (`doli-testnet-nt1` through `doli-testnet-nt18`). They share the same `doli-node` binary as the N-node on their server. Running `doli upgrade` on any server replaces the binary for all nodes on that server. After upgrade, restart each NT individually: `sudo systemctl restart doli-testnet-nt<ID>`.

#### NT Node Service Registry

| Server | NT Nodes | Services | Binary | RPC Ports |
|--------|----------|----------|--------|-----------|
| omegacortex | NT1-NT5 | `doli-testnet-nt1` ... `doli-testnet-nt5` | `~/repos/doli/target/release/doli-node` | 9001-9005 |
| N3 (147.93.84.44) | NT6-NT8 | `doli-testnet-nt6` ... `doli-testnet-nt8` | `~/doli-node` | 9001-9003 |
| N4 (72.60.70.166) | NT9-NT13 | `doli-testnet-nt9` ... `doli-testnet-nt13` | `/opt/doli/target/release/doli-node` | 9001-9005 |
| N5 (72.60.115.209) | NT14-NT18 | `doli-testnet-nt14` ... `doli-testnet-nt18` | `/opt/doli/target/release/doli-node` | 9001-9005 |

**Per-node control:**
```bash
sudo systemctl restart doli-testnet-nt18   # restart single NT
sudo systemctl stop doli-testnet-nt18      # stop single NT
sudo systemctl status doli-testnet-nt18    # check status
journalctl -u doli-testnet-nt18 -f         # tail logs
```

#### 3.8.4 Full Upgrade Sequence (Reverse Order: NT18→NT1, N6→N1)

**Prerequisites:**
- A GitHub Release must exist for the target version (CI builds tarballs on tag push)
- All nodes must have `doli` v1.1.9+ (the version with `--doli-node-path` and `--service` flags)

**Order**: NT nodes first (canaries), then mainnet nodes reverse. Within each server, upgrade the binary once, then restart services individually.

```bash
# ============================================================
# STEP 0: Verify current versions
# ============================================================
ssh ilozada@omegacortex.ai "~/repos/doli/target/release/doli --version"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 '~/doli --version'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 '/opt/doli/target/release/doli --version'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 '/opt/doli/target/release/doli --version'"

# ============================================================
# STEP 1: Upgrade N5 server (NT14-NT18 + N5)
# ============================================================
# 1a. Upgrade binaries on N5 (installs doli + doli-node, restarts N5 only)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo /opt/doli/target/release/doli upgrade --yes --service doli-mainnet-node5'"
# 1b. Wait 15s, verify N5 is advancing
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'curl -s -X POST http://127.0.0.1:8545 -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"'"
# 1c. Restart NT14-NT18 individually (binary already replaced by step 1a)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl restart doli-testnet-nt14 doli-testnet-nt15 doli-testnet-nt16 doli-testnet-nt17 doli-testnet-nt18'"
# 1d. Verify NTs advancing
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'for p in 9001 9002 9003 9004 9005; do h=\$(curl -s -X POST http://127.0.0.1:\$p -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | grep -o \"\\\"bestHeight\\\":[0-9]*\" | cut -d: -f2); echo \"port \$p: height=\$h\"; done'"

# ============================================================
# STEP 2: Upgrade N4 server (NT9-NT13 + N4)
# ============================================================
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo /opt/doli/target/release/doli upgrade --yes --service doli-mainnet-node4'"
# Wait 15s, verify N4
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'curl -s -X POST http://127.0.0.1:8545 -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"'"
# Restart NT9-NT13
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl restart doli-testnet-nt9 doli-testnet-nt10 doli-testnet-nt11 doli-testnet-nt12 doli-testnet-nt13'"
# Verify NTs advancing
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'for p in 9001 9002 9003 9004 9005; do h=\$(curl -s -X POST http://127.0.0.1:\$p -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | grep -o \"\\\"bestHeight\\\":[0-9]*\" | cut -d: -f2); echo \"port \$p: height=\$h\"; done'"

# ============================================================
# STEP 3: Upgrade N3 server (NT6-NT8 + N3)
# ============================================================
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 '~/doli upgrade --yes --doli-node-path ~/doli-node --service doli-mainnet-node3'"
# Wait 15s, verify N3
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'curl -s -X POST http://127.0.0.1:8545 -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"'"
# Restart NT6-NT8
ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl restart doli-testnet-nt6 doli-testnet-nt7 doli-testnet-nt8'
# Verify NTs advancing
ssh -p 50790 ilozada@147.93.84.44 'for p in 9001 9002 9003; do h=$(curl -s -X POST http://127.0.0.1:$p -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" | grep -o "\"bestHeight\":[0-9]*" | cut -d: -f2); echo "port $p: height=$h"; done'

# ============================================================
# STEP 4: Upgrade omegacortex (NT1-NT5 + N6 + N2 + N1)
# ============================================================
# 4a. Upgrade binaries (first run installs both doli + doli-node, restarts N6 only)
ssh ilozada@omegacortex.ai "~/repos/doli/target/release/doli upgrade --yes --doli-node-path ~/repos/doli/target/release/doli-node --service doli-mainnet-node6"
# Wait 15s, verify N6
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8547 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"

# 4b. Restart N2 (binary already replaced, just restart service)
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node2"
# Wait 15s, verify N2
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8546 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"

# 4c. Restart NT1-NT5 (binary already replaced)
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-testnet-nt1 doli-testnet-nt2 doli-testnet-nt3 doli-testnet-nt4 doli-testnet-nt5"
# Verify NTs advancing
ssh ilozada@omegacortex.ai 'for p in 9001 9002 9003 9004 9005; do h=$(curl -s -X POST http://127.0.0.1:$p -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" | grep -o "\"bestHeight\":[0-9]*" | cut -d: -f2); echo "port $p: height=$h"; done'

# 4d. Restart N1 — LAST (bootstrap node)
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node1"
# Wait 15s, verify N1
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, s: .bestSlot}'"

# ============================================================
# STEP 5: Final verification — all nodes on same version & chain
# ============================================================
ssh ilozada@omegacortex.ai "~/repos/doli/target/release/doli --version && ~/repos/doli/target/release/doli-node --version"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 '~/doli --version && ~/doli-node --version'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 '/opt/doli/target/release/doli --version && /opt/doli/target/release/doli-node --version'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 '/opt/doli/target/release/doli --version && /opt/doli/target/release/doli-node --version'"
# Chain tip agreement (all should show same height ±1):
ssh ilozada@omegacortex.ai "for p in 8545 8546 8547; do echo \"port \$p: \$(curl -s -X POST http://127.0.0.1:\$p -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | jq -c '.result | {h: .bestHeight, hash: .bestHash[0:16]}')\"; done"
```

#### 3.8.5 Upgrade Rules

1. **Always reverse order**: NT18→NT1, then N6→N5→N4→N3→N2→N1. NTs are canaries; N1 is last (bootstrap).
2. **One server at a time**: Upgrade + verify before moving to next server.
3. **`--service` is mandatory on omegacortex**: N1, N2, N6 share one machine. Without `--service`, all three restart simultaneously.
4. **`doli upgrade` only runs once per server**: It replaces both binaries. Subsequent N-nodes on the same server just need `sudo systemctl restart <service>`.
5. **NT restart is per-node via systemd**: After the binary is replaced, restart NTs individually: `sudo systemctl restart doli-testnet-nt<ID>`.
6. **If `doli upgrade` fails** (no GitHub Release, network error): Fall back to Section 3.4 (manual build + SCP).
7. **Consensus-critical changes**: Still require simultaneous deployment (Section 3.3). `doli upgrade` is for rolling upgrades only.

#### 3.8.6 `doli upgrade` vs Manual Build

| | `doli upgrade` (3.8) | Manual build + SCP (3.4) |
|---|---|---|
| **Source** | GitHub Releases tarball | `cargo build --release` on omegacortex |
| **Verification** | SHA-256 + maintainer sigs | MD5 after SCP |
| **Install method** | Atomic rename (no downtime) | Stop → cp → start |
| **When to use** | Rolling upgrades (non-consensus) | Consensus-critical changes, no GitHub Release, or `doli upgrade` fails |
| **Requires** | GitHub Release with platform tarball | Rust toolchain on omegacortex |

---

## Section 4: Auto-Update System

### 4.1 How It Works

The auto-update system (`crates/updater/`) provides fully automatic updates with community veto power:

1. **Release published** — signed by 3/5 maintainers (`sign_release_hash()`)
2. **Node detects** — `UpdateService` checks GitHub every 10 minutes, verifies 3/5 signatures
3. **Veto period begins** — 2 epochs mainnet (~2h), 60s devnet
4. **Producers vote** — can veto via `doli update vote` (weighted by bonds × seniority)
5. **Threshold check** — if >= 40% weighted veto: REJECTED, update discarded
6. **If approved** — **auto-apply**: download tarball from GitHub, verify SHA-256, atomic install, `exec()` restart (same PID, systemd/launchd unaware)
7. **If auto-apply fails** — grace period (2 min early network*), then enforcement (block production paused until updated)
8. **Watchdog** — if new binary crashes 3× within window, automatic rollback to backup

**Auto-apply flow (v1.1.10+):**
```
Veto period ends + < 40% veto
  → auto_apply_from_github(version)
    → fetch_github_release() → download tarball
    → verify_hash() → SHA-256 check
    → extract_binary_from_tarball() → extract doli-node
    → backup_current() → save .backup
    → install_binary() → atomic rename (write .new + rename)
    → restart_node() → exec() replaces process [NO RETURN]
  → New binary running, same PID, same args
```

**Disable auto-apply**: Use `--no-auto-update` flag on `doli-node run`, or set `notify_only: true` in update config. In notify-only mode, the node shows the banner but waits for manual `doli-node update apply`.

**Key constants:**
- Veto threshold: 40% (seniority-weighted)
- Required signatures: 3 of 5 maintainers
- Veto period: 5 min (mainnet/testnet early*), 60s (devnet)
- Grace period: 2 min (mainnet/testnet early*), 30s (devnet)
- Check interval: 10 min (mainnet/testnet), 10s (devnet)
- * Early-network values (v1.1.13+). Will extend as network grows.
- GitHub repo: `e-weil/doli`

**Vote weight formula:**
```
weight = bond_count x seniority_multiplier
seniority_multiplier = 1.0 + min(years, 4) x 0.75
```
Year 0: 1.0x, Year 1: 1.75x, Year 2: 2.5x, Year 3: 3.25x, Year 4+: 4.0x

### 4.2 Watchdog (Auto-Rollback)

After an update is applied, the watchdog monitors for crashes:
- State persisted at `{data_dir}/watchdog_state.json`
- If 3 crashes occur within the crash window → automatic rollback
- Clean shutdown clears the crash counter
- Crash window: 1 hour (mainnet), 120s (devnet)

### 4.3 Checking Update Status

```bash
# Via CLI
doli -r http://127.0.0.1:PORT update check
doli -r http://127.0.0.1:PORT update status

# Via node
doli-node --network mainnet update check
doli-node --network mainnet update status
```

### 4.4 Voting on an Update

```bash
# Approve
doli -r http://127.0.0.1:PORT update vote --version 1.0.1 --vote approve

# Veto
doli -r http://127.0.0.1:PORT update vote --version 1.0.1 --vote veto

# Check votes
doli -r http://127.0.0.1:PORT update votes
```

### 4.5 Applying / Rolling Back

**Automatic (default):** Once an update is approved (veto period passes without rejection), `doli-node` automatically downloads from GitHub, verifies, installs, and restarts via `exec()`. No operator action needed.

**Manual (if auto-apply fails or `notify_only` mode):**
```bash
# Apply approved update
doli-node --network mainnet update apply

# Rollback to backup
doli-node --network mainnet update rollback

# Manual upgrade from GitHub (bypasses veto system)
doli upgrade --yes --doli-node-path <PATH> --service <SERVICE>
```

**Safety nets:**
- If auto-apply fails (network error, disk full, etc.), node continues running old version and logs the error
- Grace period (48h) gives time for manual intervention
- After grace period, enforcement blocks production until updated
- Watchdog auto-rolls back if new binary crashes 3× within crash window

### 4.6 Release Pipeline

**1. Tag + push (triggers CI to build tarballs + CHECKSUMS.txt):**
```bash
# Bump version in Cargo.toml, commit, tag, push:
git tag v1.2.0 && git push origin main v1.2.0
# CI builds linux x86_64 tarball, creates GitHub Release with CHECKSUMS.txt
```

**2. Sign release with 3/5 maintainer keys:**

**Option A: Automated (requires `gh` CLI on signing machine):**
```bash
./scripts/sign-release.sh 1.2.0
```

**Option B: Split workflow (omegacortex signs, Mac uploads — PROVEN WORKFLOW):**

Omegacortex has the producer keys but no `gh` CLI. Mac has `gh` but no keys.
This is the standard procedure used for all releases.

```bash
# Step 1: Sign on omegacortex (where the keys live)
ssh ilozada@omegacortex.ai bash -s <<'REMOTE'
set -euo pipefail
DOLI=~/repos/doli/target/release/doli
KEY_DIR=~/.doli/mainnet/keys
VERSION="v1.2.0"   # ← change this
TMPDIR=$(mktemp -d)

# Download CHECKSUMS.txt from the GitHub Release
curl -sL "https://github.com/e-weil/doli/releases/download/${VERSION}/CHECKSUMS.txt" \
    -o "$TMPDIR/CHECKSUMS.txt"
CHECKSUMS_SHA256=$(sha256sum "$TMPDIR/CHECKSUMS.txt" | awk '{print $1}')
echo "CHECKSUMS.txt SHA-256: $CHECKSUMS_SHA256"

# Sign with 3 producer keys → extract JSON blocks
for i in 1 2 3; do
    KEY="$KEY_DIR/producer_${i}.json"
    "$DOLI" -w "$KEY" release sign --version "$VERSION" --key "$KEY" 2>/dev/null \
        | python3 -c "
import sys, json
raw = sys.stdin.read()
start = raw.index('{')
end = raw.index('}') + 1
print(json.dumps(json.loads(raw[start:end])))
" > "$TMPDIR/sig_${i}.json"
    echo "Signed with producer_${i}"
done

# Assemble SIGNATURES.json
python3 -c "
import json, glob
sigs = []
for f in sorted(glob.glob('$TMPDIR/sig_*.json')):
    sigs.append(json.load(open(f)))
out = {'version': '${VERSION#v}', 'checksums_sha256': '$CHECKSUMS_SHA256', 'signatures': sigs}
json.dump(out, open('$TMPDIR/SIGNATURES.json', 'w'), indent=2)
print(json.dumps(out, indent=2))
"
echo "File: $TMPDIR/SIGNATURES.json"
REMOTE

# Step 2: Copy SIGNATURES.json to Mac
scp ilozada@omegacortex.ai:/tmp/tmp.XXXXX/SIGNATURES.json /tmp/SIGNATURES.json
# ↑ Use the actual tmpdir path from step 1 output

# Step 3: Upload from Mac (where gh is authenticated)
gh release delete-asset v1.2.0 SIGNATURES.json --repo e-weil/doli --yes 2>/dev/null || true
gh release upload v1.2.0 /tmp/SIGNATURES.json --repo e-weil/doli

# Step 4: Verify (bypass CDN cache with gh, not curl)
gh release download v1.2.0 --repo e-weil/doli --pattern "SIGNATURES.json" --dir /tmp/verify --clobber
python3 -c "import json; d=json.load(open('/tmp/verify/SIGNATURES.json')); print(f'Sigs: {len(d[\"signatures\"])}')"
# Must show "Sigs: 3"
```

**IMPORTANT NOTES:**
- `doli release sign` outputs status text mixed with JSON — use the python extraction above
- The `--key` flag AND `-w` flag must BOTH point to the producer key file
- CI creates an empty SIGNATURES.json scaffold (0 signatures) — always delete it first
- CDN (curl) may serve cached empty file for minutes; use `gh release download` to verify
- Message signed: `"{version_bare}:{sha256(CHECKSUMS.txt)}"` (e.g., `"1.2.0:ac338d..."`)

**SIGNATURES.json format:**
```json
{
  "version": "1.2.0",
  "checksums_sha256": "<sha256 of CHECKSUMS.txt>",
  "signatures": [
    {"public_key": "<hex>", "signature": "<hex>"},
    {"public_key": "<hex>", "signature": "<hex>"},
    {"public_key": "<hex>", "signature": "<hex>"}
  ]
}
```

**3. Nodes auto-detect within 10 minutes:**
- `UpdateService` polls GitHub, finds new release
- Verifies 3/5 signatures against on-chain maintainer keys (from `MaintainerState`)
- Pre-bootstrap fallback: `BOOTSTRAP_MAINTAINER_KEYS` in `crates/updater/src/lib.rs`
- Starts veto period → if approved → auto-apply via `exec()`

**4. If CI is down — manual release:**
```bash
# Build on omegacortex:
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && source ~/.cargo/env && cargo build --release"
# Package tarball:
DIRNAME="doli-node-v1.2.0-x86_64-unknown-linux-gnu"
mkdir /tmp/$DIRNAME && cp target/release/doli-node target/release/doli /tmp/$DIRNAME/
cd /tmp && tar czf ${DIRNAME}.tar.gz $DIRNAME
shasum -a 256 ${DIRNAME}.tar.gz > CHECKSUMS.txt
# Create release + upload (from Mac):
gh release create v1.2.0 --title "v1.2.0" --generate-notes ${DIRNAME}.tar.gz CHECKSUMS.txt
# Then sign using Option B above
```

---

## Section 5: doli-node Upgrade

### 5.1 Standard Version Upgrade (Manual)

```bash
# 1. Build new version on omegacortex
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && source ~/.cargo/env && cargo build --release"

# 2. Atomic rename install (no need to stop the node)
# On omegacortex (binary built in place — nothing to copy)
# On N3/N4/N5: SCP + atomic rename:
scp binary ilozada@server:/tmp/doli-node
ssh ilozada@server 'sudo cp /tmp/doli-node /path/doli-node.new && sudo mv /path/doli-node.new /path/doli-node'

# 3. Restart service
sudo systemctl restart doli-mainnet-node<N>

# 4. Verify (height should increase within 20s)
curl -s -X POST http://127.0.0.1:PORT -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq '.result.bestHeight'
```

**Full procedure for all 24 nodes**: See Section 3.4 (rolling upgrade) and Section 3.8.4 (full sequence).

### 5.2 In-Place Upgrade from GitHub (Preferred Method — v1.1.9+)

Use `doli upgrade` (CLI) instead of `doli-node upgrade`. It installs **both** binaries and restarts the specified service.

```bash
# Standard (auto-detect doli-node path):
doli upgrade --yes --service doli-mainnet-node3

# Custom doli-node path (omegacortex, N3):
doli upgrade --yes --doli-node-path ~/repos/doli/target/release/doli-node --service doli-mainnet-node1

# With sudo (N4, N5 — /opt/doli/ permissions):
sudo doli upgrade --yes --service doli-mainnet-node4

# Specific version:
doli upgrade --version 1.2.0 --yes --service doli-mainnet-node3
```

**Full upgrade procedure with per-server flags**: See Section 3.8 for the complete step-by-step sequence covering all 24 nodes (NT18→NT1, N6→N1).

### 5.3 Handling Breaking Changes

If the new version changes on-disk format or consensus rules:

1. **Stop all nodes** (simultaneous for consensus changes)
2. **Backup data**: `cp -r ~/.doli/mainnet ~/.doli/mainnet.bak`
3. **Replace binaries** on all nodes
4. **If migration needed**: node logs will indicate; may need `recover`
5. **Start all nodes**
6. **Verify all on same chain**

### 5.4 Rollback Procedure

```bash
# 1. Stop node
sudo systemctl stop doli-testnet

# 2. Restore backup binary
cp /path/to/doli-node.backup /path/to/doli-node

# 3. If state is corrupted, recover from blocks
doli-node --network mainnet recover --yes

# 4. Restart
sudo systemctl start doli-testnet
```

**Automatic rollback** (via watchdog):
- If the updated node crashes 3 times within the crash window, the watchdog
  automatically restores the `.backup` binary and restarts.

---

## Section 6: Producer Bond Management

### 6.1 Register as Producer

**Prerequisites:**
- Wallet with sufficient balance (1 bond_unit = 10 DOLI mainnet, 1 DOLI devnet)
- Running node to submit tx to

```bash
# Register with 1 bond (minimum)
doli -r http://127.0.0.1:PORT producer register

# Register with 5 bonds
doli -r http://127.0.0.1:PORT producer register -b 5
```

### 6.2 Add Bonds (Bond Stacking)

Adding bonds increases scheduling weight for epoch-based producer selection.

```bash
# Add 3 more bonds
doli -r http://127.0.0.1:PORT producer add-bond -c 3
```

### 6.3 Check Bond Status

```bash
# Status of wallet's producer
doli -r http://127.0.0.1:PORT producer status

# Status of specific producer
doli -r http://127.0.0.1:PORT producer status -p <PUBKEY_HEX>

# All producers with bonds
doli -r http://127.0.0.1:PORT producer list
```

### 6.4 Withdrawal (Instant, FIFO — v1.0.23+)

**No delay.** Funds available immediately in the same block. Bonds removed at next epoch boundary.

**1-day vesting penalty (per-bond FIFO — oldest withdrawn first):**

| Quarter | Bond Age | Penalty |
|---------|----------|---------|
| Q1 | 0-6h | 75% burned |
| Q2 | 6-12h | 50% burned |
| Q3 | 12-18h | 25% burned |
| Q4+ | 18h+ | 0% |

Each bond's penalty is calculated individually based on its creation time. Penalty is burned.

```bash
# Withdraw 2 bonds (instant payout, FIFO order)
doli -r http://127.0.0.1:PORT producer request-withdrawal -c 2

# Withdraw to specific destination
doli -r http://127.0.0.1:PORT producer request-withdrawal -c 1 -d doli1recipient...

# Emergency exit (penalty applies per-bond)
doli -r http://127.0.0.1:PORT producer exit --force
```

**CLI shows interactive FIFO breakdown** before submitting:
- Bond inventory by vesting tier (Q1/Q2/Q3/vested)
- Per-tier penalty calculation for the requested count
- Total net amount, total burned, bonds remaining
- Confirmation prompt before submitting

**Double-withdrawal prevention**: Cannot submit two withdrawals in the same epoch. `withdrawal_pending_count` blocks duplicates until epoch boundary.

**Note**: `claim-withdrawal` (TxType 9) is no longer needed — withdrawal is instant. The command remains defined but unused.

### 6.5 Transaction Types Reference

| TxType | Value | Description |
|--------|-------|-------------|
| Transfer | 0 | Standard send |
| Register | 1 | Producer registration |
| Exit | 2 | Producer exit |
| ClaimBond | 4 | Claim unbonded stake |
| Slash | 5 | Slash equivocator |
| Coinbase | 6 | Block reward |
| AddBond | 7 | Add to existing bond |
| WithdrawalRequest | 8 | Request early withdrawal |
| WithdrawalClaim | 9 | Claim withdrawal |
| EpochReward | 10 | Epoch rewards |
| MaintainerAdd | 11 | Add maintainer |
| MaintainerRemove | 12 | Remove maintainer |

---

## Section 7: Common Issues & Fixes

### 7.1 Body Downloader Stall

**Symptoms:**
- Logs show `DownloadingBodies { pending: N, total: M }` for minutes
- Height stops advancing during sync
- `body_stall_retries` incrementing in logs

**Cause:** Peer disconnected mid-download, or responses timed out. Pending body data may be retained but no new requests go out.

**Fix:**
- The sync manager has built-in stall recovery (`body_stall_retries`). After a few retries with no progress, it resets to Idle and restarts sync.
- If stuck > 5 minutes: restart the node via `sudo systemctl restart doli-mainnet-nodeN`.

### 7.2 GSet Divergence

**Symptoms:**
- Logs show `GSet merge` / `merge_one` diagnostic messages
- Producer discovery shows different producer counts across nodes
- `[GSET]` log lines showing sequence mismatches

**Cause:** GSet sequence not persisted across restarts (fixed in d81a9c2), or ghost producers from crashed nodes.

**Fix:**
- Update to latest version (includes GSet sequence persistence + ghost purge)
- If persists: restart the node; GSet resyncs via gossip
- Nuclear option: wipe `producers.bin` and let node rebuild from blocks

### 7.3 Fork Detection and Resolution

**Symptoms:**
- `FORK DETECTION` in logs
- `BlockedChainMismatch`, `BlockedAheadOfPeers`, or `BlockedSyncFailures` production blocks
- Different `bestHash` across nodes at same height

**Shallow fork (1-2 blocks, e.g., orphan from kill):**
- Auto-resolved by `resolve_shallow_fork()`: detects 3+ consecutive empty header responses with small gap, rolls back 1 block, resyncs
- Logs: `Shallow fork detected ... rolling back 1 block`

**Deep fork (>10 empty header responses):**
- Auto-resolved by `force_resync_from_genesis()`: complete state reset, rebuilds from canonical chain
- Logs: `Deep fork detected: peers consistently reject our chain tip`

**Manual resolution if auto-recovery fails:**
```bash
# 1. Stop node
sudo systemctl stop doli-mainnet-nodeN

# 2. Wipe chain state (keeps blocks)
rm -f ~/.doli/<NETWORK>/data/*/chain_state.bin
rm -f ~/.doli/<NETWORK>/data/*/producers.bin
rm -f ~/.doli/<NETWORK>/data/*/utxo_set.bin

# 3. Recover
doli-node --network <NETWORK> recover --yes

# 4. If still wrong, full wipe
rm -rf ~/.doli/<NETWORK>/data/*/blocks/
# Restart — resyncs from peers
```

### 7.4 Node Stuck Syncing

**Symptoms:**
- Height not advancing
- Stuck in `DownloadingHeaders` or `Processing` state
- Repeated `Idle -> DownloadingHeaders -> Idle` cycles

**Diagnosis:**
```bash
# Check chain height
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq '.result.bestHeight'

# Check peers
curl -s -X POST http://127.0.0.1:PORT \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq '.result.peerCount'
```

**Fixes:**
1. **0 peers**: Check firewall (port 30303/40303/50303), add `--bootstrap` flag
2. **Peers but no progress**: Likely on a fork — node will auto-detect and resync
3. **Stuck Processing**: The cleanup() timer (5 min) will reset state automatically
4. **Persistent**: Restart node. If still stuck, `recover --yes` or full wipe+resync

### 7.5 RocksDB LOCK File (SIGTRAP on Start)

**Symptom:** Node crashes with `Trace/BPT trap: 5` immediately on start after a `kill -9`.

**Fix:**
```bash
rm -f ~/.doli/<NETWORK>/data/node*/blocks/LOCK
rm -f ~/.doli/<NETWORK>/data/node*/signed_slots.db/LOCK
```

### 7.6 Production Blocked (Various Reasons)

Check logs for `[CAN_PRODUCE]` lines. Common blocks:

| Block Type | Meaning | Action |
|------------|---------|--------|
| `BlockedSyncing` | Active sync in progress | Wait for sync to complete |
| `BlockedBehindPeers` | Behind network tip | Wait for sync |
| `BlockedAheadOfPeers` | Likely on a fork | Will auto-resync |
| `BlockedSyncFailures` | Chain tip rejected by peers | Shallow/deep fork recovery triggers |
| `BlockedChainMismatch` | Hash differs from peer at same height | Fork — auto-resync |
| `BlockedInsufficientPeers` | < 2 peers | Check connectivity |
| `BlockedNoGossipActivity` | No gossip blocks for 3 min | Check network isolation |
| `BlockedBootstrap` | Waiting for peer status | Normal during startup |
| `BlockedResync` | Grace period after resync | Wait for grace period |

### 7.7 Clock Drift

**Symptom:** Blocks rejected with `InvalidTimestamp`, or node consistently misses slots.

**Fix:**
```bash
# Check system clock
date -u

# Sync with NTP
sudo ntpdate pool.ntp.org
# Or ensure NTP service is running
sudo systemctl enable --now chronyd
```

Max allowed drift: 1s (slot), 200ms (block timestamp).

---

## Appendix: Key File Locations

| File | Purpose |
|------|---------|
| `~/.doli/<NET>/` | Network data root |
| `~/.doli/<NET>/.env` | Network config overrides |
| `~/.doli/<NET>/data/*/blocks/` | RocksDB block storage |
| `~/.doli/<NET>/data/*/chain_state.bin` | Serialized chain tip |
| `~/.doli/<NET>/data/*/producers.bin` | Producer registry |
| `~/.doli/<NET>/data/*/utxo_set.bin` | UTXO set |
| `~/.doli/<NET>/data/*/signed_slots.db/` | RocksDB equivocation tracking |
| `~/.doli/<NET>/data/*/watchdog_state.json` | Update watchdog state |
| `~/.doli/wallet.json` | Default wallet |

## Appendix: Useful Scripts

| Script | Purpose |
|--------|---------|
| `scripts/build_release.sh` | Build release binaries for all platforms |
| `scripts/smoke_test_release.sh` | Verify release artifacts |
| `scripts/publish_release.sh` | Publish signed release to GitHub |
| `scripts/deploy_producers.sh` | Deploy N producers to devnet |
| `scripts/launch_testnet.sh` | Launch local 2-node devnet |
| `scripts/update.sh` | Manual binary update from GitHub |
| `scripts/generate_chainspec.sh` | Generate chainspec from wallets |

---

## Section 8: Producer Key Registry

### 8.1 Our Producers (AUTHORITATIVE)

> **CRITICAL**: These are the ONLY valid producer keys. They match `BOOTSTRAP_MAINTAINER_KEYS` in `crates/updater/src/lib.rs`.

| Node | Key File | Address (`doli1...`) | Public Key (Ed25519) |
|------|----------|---------------------|----------------------|
| **N1** | `~/.doli/mainnet/keys/producer_1.json` | `doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef` | `202047256a...c3e6d3df` |
| **N2** | `~/.doli/mainnet/keys/producer_2.json` | `doli12uaj6e7nkl90ry9q2ze27la7w0cg23ny7zk5csyj7ffrlcttcansfzx4mz` | `effe88fefb...9926272b` |
| **N3** | `/home/ilozada/.doli/mainnet/keys/producer_3.json` | `doli109t8uyux22qqrx9ewzrpxww25scjt5cl49cunkn6m72me2txrgpsqd3rql` | `54323cefd0...25c48c2b` |
| **N4** | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` | `doli1eduw95x5c6erx4dpacpfm90dylhjvjjn43j3nwag3huym6d20sdqzcqyq6` | `a1596a36fd...e9beda1d` |
| **N5** | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` | `doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc` | `c5acb5b359...e3c03a9` |
| **N6** | `~/.doli/mainnet/keys/producer_6.json` | `doli1dy5scma8lrc5uyez7pyhpq7q7xeakyzyyc5xrrfyuusgvzkakh9swnrr0s` | `d13ae33891...4a1ec670` |
| **N8** | `~/.doli/mainnet/keys/producer_8.json` | `doli16qgdgxh7s7jn7au578yky8k6wakqdng4x82t6nu0h4dla9xjd43s30g6ma` | `3303a23595...77b4b88` |

**N1-N5**: Genesis producers and maintainers (governance 5/5).
**N6/N8**: NOT genesis producers, NOT maintainers.

Producer key files are wallet-compatible — use directly with `doli -w <key_file>`.

### 8.2 External Producers

| Name | Address (`doli1...`) | Public Key (Ed25519) | Bonds | Registered |
|------|---------------------|----------------------|-------|------------|
| **atinoco** | `doli17f7pqlkfjweddk88ry6gtc23hvmptsqk2epxx7h6x9a8gvan3crsfl243e` | `d4b5451bf7...d9fd095e` | 1 | Height 495 |
| **daniel** | `doli1p7s6hcacnm6t64nk670leeu9w3tvnkvwc688r9zlvh2f3573f6vs4cynzh` | — | 1 | Pending (Epoch 18) |

### 8.3 All-Node Balance Check

> **DO NOT use RPC `getBalance`** — returns 0. Use CLI instead.

```bash
ssh ilozada@omegacortex.ai "
  CLI=~/repos/doli/target/release/doli
  W=~/.doli/mainnet/keys/producer_1.json
  echo 'N1:' && \$CLI -w \$W balance --address doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef
  echo 'N2:' && \$CLI -w \$W balance --address doli12uaj6e7nkl90ry9q2ze27la7w0cg23ny7zk5csyj7ffrlcttcansfzx4mz
  echo 'N3:' && \$CLI -w \$W balance --address doli109t8uyux22qqrx9ewzrpxww25scjt5cl49cunkn6m72me2txrgpsqd3rql
  echo 'N4:' && \$CLI -w \$W balance --address doli1eduw95x5c6erx4dpacpfm90dylhjvjjn43j3nwag3huym6d20sdqzcqyq6
  echo 'N5:' && \$CLI -w \$W balance --address doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc
  echo 'N6:' && \$CLI -w \$W balance --address doli1dy5scma8lrc5uyez7pyhpq7q7xeakyzyyc5xrrfyuusgvzkakh9swnrr0s
  echo 'N8:' && \$CLI -w \$W balance --address doli16qgdgxh7s7jn7au578yky8k6wakqdng4x82t6nu0h4dla9xjd43s30g6ma
  echo 'atinoco:' && \$CLI -w \$W balance --address doli17f7pqlkfjweddk88ry6gtc23hvmptsqk2epxx7h6x9a8gvan3crsfl243e
"
```

---

## Section 9: Additional Nodes (N6, N8)

### 9.1 Node 6 (omegacortex)

| Property | Value |
|----------|-------|
| Service | `doli-mainnet-node6` |
| Service file | `/etc/systemd/system/doli-mainnet-node6.service` |
| Data | `/home/ilozada/.doli/mainnet/node6/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_6.json` |
| P2P | 30305 |
| RPC | 8547 |
| Metrics | 9092 |
| Logs | `/var/log/doli/node6.log` |
| Bootstrap | Node 1 via `/ip4/127.0.0.1/tcp/30303` |

Re-registered post-genesis at block 7812 with 10 bonds (100 DOLI). Not a maintainer. Shares host/binary with N1/N2.

### 9.2 Node 8 (macOS local)

| Property | Value |
|----------|-------|
| Service | `network.doli.mainnet.node8` (launchd) |
| Service file | `~/Library/LaunchAgents/network.doli.mainnet.node8.plist` |
| Binary | `/usr/local/bin/doli-node` |
| Data | `~/.doli/mainnet/node8/data` |
| Key | `~/.doli/mainnet/keys/producer_8.json` |
| P2P | 30305 |
| RPC | 8547 |
| Logs | `~/.doli/mainnet/node8.log` |

**KeepAlive: true** — must `launchctl unload` (not just `stop`) before wiping data.

```bash
# Manage N8 (macOS launchd)
launchctl list network.doli.mainnet.node8                    # status
launchctl stop network.doli.mainnet.node8                    # stop
launchctl start network.doli.mainnet.node8                   # start
launchctl unload ~/Library/LaunchAgents/network.doli.mainnet.node8.plist  # disable
launchctl load ~/Library/LaunchAgents/network.doli.mainnet.node8.plist    # enable
```

---

## Section 10: Chainspec, DNS & Snap Sync

### 10.1 Chainspec Rules (CONSENSUS-CRITICAL)

> **HARD LESSON (2026-02-22):** N4/N5 had no `chainspec.json` → different `genesis_timestamp` → slot diverged → chain fork.

1. Chainspec is **embedded in the binary** (`chainspec.mainnet.json` via `include_str!`)
2. On first start, if no `chainspec.json` in data dir, binary writes from embedded
3. Priority: `--chainspec /path` > `$DATA_DIR/chainspec.json` > embedded fallback
4. Producer nodes `exit(1)` without chainspec — code guard in `main.rs`
5. **NEVER** change `genesis.timestamp` or `consensus.slot_duration` — breaks consensus

### 10.2 DNS / Bootstrap

| Record | Resolves to | Purpose |
|--------|-------------|---------|
| `seed1.doli.network` | `72.60.228.233` | Default bootstrap (N1) |
| `seed2.doli.network` | `72.60.228.233` | Default bootstrap (N1) |

Hardcoded in `crates/core/src/network_params.rs`. Nodes without `--bootstrap` use these automatically.

### 10.3 Snap Sync

When >1000 blocks behind with 3+ peers, node uses snap sync (full state snapshot). Seconds instead of hours.

- Wire protocol: `GetStateRoot`/`StateRoot` + `GetStateSnapshot`/`StateSnapshot`
- State root: `H(H(chain_state) || H(utxo_set) || H(producer_set))` verified by 2+ peers
- Falls back to header-first sync if <3 peers or quorum fails
- Logs: `[SNAP_SYNC]` prefix

---

## Section 11: On-Chain Protocol Activation

Binaries with consensus changes ship new rules behind a **protocol version gate**. Rules stay dormant until activated on-chain by maintainers.

**Activation flow:**
1. Binary installed via auto-update (safe — new rules dormant)
2. Maintainers emit `ProtocolActivation` tx (3/5 multisig): `doli protocol activate --version 3 --key producer_N.json`
3. Grace period (2 epochs) — all nodes process the activation tx
4. At epoch boundary → ALL nodes switch simultaneously (deterministic, zero fork)

**Key pieces:**
- `TxType::ProtocolActivation = 15`
- `ChainState.active_protocol_version` (starts at 2)
- `consensus::is_protocol_active(v, cs)` gate function
- Same `validate_maintainer_tx()` as MaintainerAdd/Remove

| Change type | Activation | Example |
|-------------|-----------|---------|
| Non-consensus | Immediate on install | RPC, logging, sync fixes |
| Consensus-critical | On-chain ProtocolActivation | Scheduler, validation, economics, VDF |

**Auto-update env overrides** (all networks):
- `DOLI_VETO_PERIOD_SECS` — veto window (default: 7200s mainnet, 60s devnet)
- `DOLI_GRACE_PERIOD_SECS` — grace window (default: 3600s mainnet, 30s devnet)
- `DOLI_UPDATE_CHECK_INTERVAL_SECS` — poll interval (default: 21600s/6h mainnet, 10s devnet)

**Signing convention:** `message = "{version}:{sha256(CHECKSUMS.txt)}"`

**Updater key files:**

| File | Purpose |
|------|---------|
| `crates/updater/src/lib.rs` | Release, signatures, verification, constants |
| `crates/updater/src/download.rs` | `fetch_from_github()`, CHECKSUMS/SIGNATURES download |
| `crates/updater/src/vote.rs` | VoteTracker, seniority-weighted veto |
| `crates/updater/src/apply.rs` | Binary backup, install, rollback, extraction |
| `crates/updater/src/watchdog.rs` | Post-update crash detection, auto-rollback |
| `bins/node/src/updater.rs` | Node-side auto-update loop, enforcement |
| `.github/workflows/release.yml` | CI: build, package, CHECKSUMS.txt, SIGNATURES.json scaffold |

---

## Section 12: Temporary Test Nodes (NTx)

Sync-only nodes for network stress testing. All data lives in `~/doli-test/` per server — `rm -rf ~/doli-test` for full cleanup.

### 12.1 Distribution

| Server | Nodes | P2P Ports | RPC Ports | Binary |
|--------|-------|-----------|-----------|--------|
| omegacortex.ai | NT1-NT5 | 31001-31005 | 9001-9005 | `~/repos/doli/target/release/doli-node` |
| 147.93.84.44 (N3) | NT6-NT8 | 31001-31003 | 9001-9003 | `~/doli-node` |
| 72.60.70.166 (N4) | NT9-NT13 | 31001-31005 | 9001-9005 | `/opt/doli/target/release/doli-node` |
| 72.60.115.209 (N5) | NT14-NT18 | 31001-31005 | 9001-9005 | `/opt/doli/target/release/doli-node` |

**Total: 18 test nodes** (N3 has 3 due to limited RAM: 1 CPU, 3.8GB).

### 12.2 File Structure (per server)

```
~/doli-test/
├── .env              # DOLI_TEST_BINARY, DOLI_TEST_COUNT, DOLI_TEST_OFFSET
├── manage.sh         # start|stop|status|clean
├── keys/
│   └── ntX.json      # Wallet per node
├── nt1/
│   ├── data/         # Chain data
│   └── node.log      # Logs
├── nt2/
│   └── ...
└── ...
```

### 12.3 Management

Each server has a `manage.sh` script and a `doli-test-nodes` systemd service.

> **CRITICAL: `manage.sh status` is UNRELIABLE.** It may report "stopped" even when nodes are running via systemd. **Always verify via RPC** (see below). The systemd service `doli-test-nodes` is the authoritative process manager.

```bash
# Via systemd (PREFERRED — this is the authoritative service)
sudo systemctl start doli-test-nodes
sudo systemctl stop doli-test-nodes
sudo systemctl status doli-test-nodes

# Via script (must source .env first) — use for start/stop/clean, NOT for status
source ~/doli-test/.env && ~/doli-test/manage.sh start
source ~/doli-test/.env && ~/doli-test/manage.sh stop
source ~/doli-test/.env && ~/doli-test/manage.sh clean   # stops + wipes data, keeps keys/logs
```

**RELIABLE status check — always use RPC:**
```bash
# Check NT node status via JSON-RPC (the ONLY reliable method)
# NT RPC ports: 9001-9005 per server (matching node index within that server)
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
  http://127.0.0.1:9001/ | jq -c '.result | {h: .bestHeight, s: .bestSlot}'
```

**GOTCHA on omegacortex:** `source ~/doli-test/.env` sets `DOLI_TEST_BINARY` but does NOT export it. Over SSH, the variable is lost before `manage.sh` reads it. Fix:
```bash
# WRONG (binary not found):
ssh ilozada@omegacortex.ai "source ~/doli-test/.env && ~/doli-test/manage.sh start"

# CORRECT (explicit export):
ssh ilozada@omegacortex.ai "export DOLI_TEST_BINARY=/home/ilozada/repos/doli/target/release/doli-node && source ~/doli-test/.env && ~/doli-test/manage.sh start"
```

This only affects omegacortex. On N3/N4/N5, the `.env` is sourced in the same non-SSH shell context and works fine.

**Remote status check (via RPC — preferred over manage.sh):**
```bash
# omegacortex NT1-NT5 (ports 9001-9005)
ssh ilozada@omegacortex.ai 'for p in 9001 9002 9003 9004 9005; do echo -n "NT port=$p: "; curl -s --max-time 3 -X POST -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" http://127.0.0.1:$p/ | jq -c ".result | {h: .bestHeight, s: .bestSlot}"; done'

# N3 NT6-NT8 (ports 9001-9003)
ssh -p 50790 ilozada@147.93.84.44 'for p in 9001 9002 9003; do echo -n "NT port=$p: "; curl -s --max-time 3 -X POST -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}" http://127.0.0.1:$p/ | jq -c ".result | {h: .bestHeight, s: .bestSlot}"; done'

# N4 NT9-NT13 (ports 9001-9005) and N5 NT14-NT18 (ports 9001-9005) — via jump host
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'for p in 9001 9002 9003 9004 9005; do echo -n \"NT port=\$p: \"; curl -s --max-time 3 -X POST -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" http://127.0.0.1:\$p/ | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"; done'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'for p in 9001 9002 9003 9004 9005; do echo -n \"NT port=\$p: \"; curl -s --max-time 3 -X POST -H \"Content-Type: application/json\" -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\" http://127.0.0.1:\$p/ | jq -c \".result | {h: .bestHeight, s: .bestSlot}\"; done'"
```

### 12.4 Test Node Wallets

| Node | Address | Server |
|------|---------|--------|
| NT1 | `doli1aknspdkl05fvkar873jgwzsqec89750ahddrhz2dfqqwc4du4pqslpm40d` | omegacortex |
| NT2 | `doli1lnhatnrywewh2aj4pla9k6xf8f3k9vyhsdr4v5utedfhvs8y89gs3m68t9` | omegacortex |
| NT3 | `doli15rwdjcqlw3ue4yetrwzcafld6exmk47mclp9555rck8u36yn93mqwh3ky7` | omegacortex |
| NT4 | `doli1sjr56d0a0cpte7d7rqzwes6e8t6gqucgclu83h6nj7mev36wflcs6vgke4` | omegacortex |
| NT5 | `doli1nyufskea2099zrz9jllpka436gf635hdswkux3aqjqsyth5whz9svdr2ks` | omegacortex |
| NT6 | `doli1mrtqcl7wkyh09maxwd4qqf0q03ww2es5g2vnwpda6zg674wasr4qkh6eer` | N3 |
| NT7 | `doli1j659wwxkfry8pu70jq9lqccpdu4ndm8rec88dumw8qgarpyedg3sm3uxpp` | N3 |
| NT8 | `doli1mys9rqsdxke9jwvxp2qjgf5plunqjj8fg7mn06ae3n3datzyzq8qmttzk6` | N3 |
| NT9 | `doli1cjd2wr56v3r6rad2hjhr6xu0qkagnhx83k5hauez8078hqzpn8fs9yuads` | N4 |
| NT10 | `doli125qe88h4y9g98g5c0pgeucffzkhu5g9fccgg9h7eyj8cte30srusgquxlq` | N4 |
| NT11 | `doli13cgd9uzvah82x9jvd8w7rnzk9wwj4t3zz23t57864fdc2e8kgs4qjdajvu` | N4 |
| NT12 | `doli183snfefvkdpphynk6v3y0k36k629an0kc7r22vl664ad29q6lc6q49ehlz` | N4 |
| NT13 | `doli1fgekf27mlt5n2qakpeyd8hcmx3xtr3asdjcsvfn544kk5w78ehgqhr77xd` | N4 |
| NT14 | `doli1thrxjwqz5akuplj8x6xsh37dwpqkt70uy3jr82lsf9m7hz27u95snn0w68` | N5 |
| NT15 | `doli1zazurax04txfz9raka2vc4upr79xwannu6rhrdhxrhs2x8eez86qza0c6w` | N5 |
| NT16 | `doli1wxqsxjuxffzgyqhz7kg6jpty07hlpdaz4ujpdl73kahmrdn72e5qjrkkjj` | N5 |
| NT17 | `doli1gxsps883neu5qvp7aqu5lp4p2jc3fe7l480zqxtkq3t3735s8v2qn4r6ek` | N5 |
| NT18 | `doli1d40m5k7c90uk9d82hgk2p34adpd90exk2gzhf0faggh02k43xakqjf520k` | N5 |

> **NT wallet key files are v2 format** — they do NOT have a top-level `address` field. The address is nested inside `addresses[0].address` as a raw hex hash (NOT a pubkey_hash). To get the bech32m `doli1...` address, use `doli -w <key_file> info`. Do NOT try to parse the hex address from JSON — it won't work with `doli balance --address`.

**Getting NT bech32m addresses:**
```bash
# On omegacortex (NT1-NT5):
doli -w ~/doli-test/keys/nt1.json info | grep -o 'doli1[a-z0-9]*'

# On N3 (NT6-NT8):
~/doli -w ~/doli-test/keys/nt6.json info | grep -o 'doli1[a-z0-9]*'

# On N4 (NT9-NT13):
/opt/doli/target/release/doli -w ~/doli-test/keys/nt9.json info | grep -o 'doli1[a-z0-9]*'

# On N5 (NT14-NT18):
/opt/doli/target/release/doli -w ~/doli-test/keys/nt14.json info | grep -o 'doli1[a-z0-9]*'
```

**NT key file naming:** Keys are named `ntN.json` where N matches the NT number on that server (e.g., N3 has `nt6.json`, `nt7.json`, `nt8.json`). On N4, keys are `nt9.json` through `nt13.json`.

**Checking NT balance (from omegacortex):**
```bash
# Use the doli1 addresses from the table above
CLI=~/repos/doli/target/release/doli
W=~/.doli/mainnet/keys/producer_1.json
$CLI -w $W balance --address doli1aknspdkl05fvkar873jgwzsqec89750ahddrhz2dfqqwc4du4pqslpm40d
```

### 12.5 Full Cleanup

```bash
# Per server: stop nodes, remove service, delete all data
sudo systemctl stop doli-test-nodes
sudo rm /etc/systemd/system/doli-test-nodes.service
sudo systemctl daemon-reload
rm -rf ~/doli-test
```
