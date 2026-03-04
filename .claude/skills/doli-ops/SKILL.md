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

### 2.2 Git Pull / Deploy Summary

| Server | Git Repo | Remote | Pull Command |
|--------|----------|--------|-------------|
| Local (mac-001) | `~/repos/doli` | `git@github.com:e-weil/doli.git` (SSH) | `git pull` |
| omegacortex.ai | `~/repos/doli` | `git@github.com:e-weil/doli.git` (SSH) | `ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull"` |
| Node 4 (pro-KVM1) | `/opt/doli` | `https://github.com/e-weil/doli.git` | `ssh -p 50790 ilozada@72.60.70.166 "sudo -u isudoajl bash -c 'cd /opt/doli && git pull'"` |
| Node 5 (fpx) | `/opt/doli` | `git@github.com:e-weil/doli.git` | `ssh -p 50790 ilozada@72.60.115.209 "sudo -u isudoajl bash -c 'cd /opt/doli && git pull'"` |
| Partner (147.93.84.44) | **None** | N/A | Binary via SCP: `scp -P 50790 <binary> ilozada@147.93.84.44:~/doli-node` |

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
# 1. Build on omegacortex.ai
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && \
    source ~/.cargo/env && cargo build --release --package doli-node"

# 2. Copy binary to N3 (via SCP from omegacortex)
ssh ilozada@omegacortex.ai "scp -P 50790 ~/repos/doli/target/release/doli-node ilozada@147.93.84.44:~/doli-node"

# 2b. Pull & build on N4 and N5 (they have their own git repos)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo -u isudoajl bash -c \"cd /opt/doli && git pull && source ~/.cargo/env && cargo build --release --package doli-node\"'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo -u isudoajl bash -c \"cd /opt/doli && git pull && source ~/.cargo/env && cargo build --release --package doli-node\"'"

# 3. Stop ALL nodes via systemd
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node1 doli-mainnet-node2"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl stop doli-mainnet-node3'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl stop doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl stop doli-mainnet-node5'"

# 4. Start ALL nodes via systemd (N1 first = bootstrap)
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node1"
sleep 3
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node2"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-mainnet-node3'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl start doli-mainnet-node5'"

# 5. Verify all nodes are on same chain — see 3.5
```

### 3.4 Non-Consensus: Rolling Upgrade

Network, sync, RPC, logging, or UI changes can be deployed one node at a time.

```bash
# For each node: restart via systemd (binary already replaced by build)
# Example for node1:
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node1"
# Verify node1 is healthy before proceeding to node2 (see 3.5)
sleep 15
ssh ilozada@omegacortex.ai "sudo systemctl restart doli-mainnet-node2"
# Continue with N3, N4, N5...
```

### 3.5 Post-Deploy Verification

```bash
# 1. All services active
ssh ilozada@omegacortex.ai "sudo systemctl is-active doli-mainnet-node1 doli-mainnet-node2"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl is-active doli-mainnet-node3'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl is-active doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl is-active doli-mainnet-node5'"

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

### 3.6 Remote Build + Deploy (Single Command)

```bash
# Build + restart node1+node2 on omegacortex.ai
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && \
    source ~/.cargo/env && \
    cargo build --release --package doli-node && \
    sudo systemctl restart doli-mainnet-node1 && sleep 3 && \
    sudo systemctl restart doli-mainnet-node2"
```

---

## Section 4: Auto-Update System

### 4.1 How It Works

The auto-update system (`crates/updater/`) provides transparent updates with community veto:

1. **Release published** — signed by 3/5 maintainers (`sign_release_hash()`)
2. **Veto period begins** — 7 days mainnet, 60s devnet
3. **Producers vote** — can veto via `doli update vote`
4. **Threshold check** — if >= 40% weighted veto: REJECTED
5. **If approved** — 48h grace period, then enforcement (no-update-no-produce)

**Key constants:**
- Veto threshold: 40% (seniority-weighted)
- Required signatures: 3 of 5 maintainers
- Veto period: 7 days (mainnet), 60s (devnet)
- Grace period: 48 hours (mainnet), 30s (devnet)
- Check interval: 6 hours
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

```bash
# Apply approved update
doli-node --network mainnet update apply

# Rollback to backup
doli-node --network mainnet update rollback

# Manual upgrade from GitHub
doli-node upgrade --yes
```

### 4.6 Release Pipeline

**1. Build release binaries:**
```bash
./scripts/build_release.sh
# Outputs: target/release/ tarballs + CHECKSUMS.txt
```

**2. Sign release (each maintainer):**
```bash
doli-node release sign --key <MAINTAINER_KEY_PATH> --version 0.2.0 --hash <SHA256>
```

**3. Publish to GitHub:**
```bash
# Combine 3+ signatures into release.json and upload
./scripts/publish_release.sh
```

**4. Smoke test:**
```bash
./scripts/smoke_test_release.sh
```

---

## Section 5: doli-node Upgrade

### 5.1 Standard Version Upgrade

```bash
# 1. Build new version
cargo build --release --package doli-node

# 2. Stop node
sudo systemctl stop doli-testnet

# 3. Replace binary
cp target/release/doli-node /path/to/installed/doli-node

# 4. Start node
sudo systemctl start doli-testnet

# 5. Verify
sudo systemctl status doli-testnet
journalctl -u doli-testnet --since '1 min ago' | grep -i version
```

### 5.2 In-Place Upgrade from GitHub

```bash
# Downloads, verifies SHA256, replaces binary via exec() (same PID)
doli-node upgrade --yes
# Or specific version:
doli-node upgrade --version 0.2.0 --yes
```

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
| **atinoco** | `doli17f7pqlkfjweddk88ry6gtc23hvmptsqk2epxx7h6x9a8gvan3crsfl243e` | `d4b5451bf7...d9fd095e` | 19 | Height 495 |

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
