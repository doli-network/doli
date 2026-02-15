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
| `request-withdrawal` | Start withdrawal delay | `-c, --count <N>`, `-d, --destination <ADDR>` |
| `claim-withdrawal` | Claim after delay | `-i, --index <N>` (default 0) |
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
| Started with | `nohup` (NOT systemd) |

**Node 1:**

| Property | Value |
|----------|-------|
| Data | `/home/ilozada/.doli/mainnet/node1/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_1.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/tmp/node1.log` |

**Node 2:**

| Property | Value |
|----------|-------|
| Data | `/home/ilozada/.doli/mainnet/node2/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_2.json` |
| P2P | 30304 |
| RPC | 8546 |
| Metrics | 9091 |
| Logs | `/tmp/node2.log` |
| Bootstrap | Node 1 via `/ip4/127.0.0.1/tcp/30303` |

#### Server 2: 147.93.84.44 (Node 3 — partner)

| Property | Value |
|----------|-------|
| SSH | `ssh -p 50790 ilozada@147.93.84.44` |
| IP | 147.93.84.44 |
| Specs | 1 CPU, 3.8GB RAM, Ubuntu 24.04 |
| Binary | `/home/ilozada/doli-node` |
| Data | `/home/ilozada/.doli/mainnet/data` |
| Key | `/home/ilozada/.doli/mainnet/keys/producer_3.json` |
| P2P | 30303 |
| RPC | 8545 |
| Metrics | 9090 |
| Logs | `/tmp/node3.log` |
| Bootstrap | Node 1 via `/ip4/72.60.228.233/tcp/30303` |

### 2.2 Starting Nodes

**Node 1 (omegacortex.ai):**
```bash
ssh ilozada@omegacortex.ai
nohup /home/ilozada/repos/doli/target/release/doli-node \
    --data-dir /home/ilozada/.doli/mainnet/node1/data run \
    --producer \
    --producer-key /home/ilozada/.doli/mainnet/keys/producer_1.json \
    --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
    --no-auto-update --yes --force-start \
    > /tmp/node1.log 2>&1 &
```

**Node 2 (omegacortex.ai — same server, different ports):**
```bash
nohup /home/ilozada/repos/doli/target/release/doli-node \
    --data-dir /home/ilozada/.doli/mainnet/node2/data run \
    --producer \
    --producer-key /home/ilozada/.doli/mainnet/keys/producer_2.json \
    --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
    --no-auto-update --yes --force-start \
    --p2p-port 30304 --rpc-port 8546 --metrics-port 9091 \
    --bootstrap /ip4/127.0.0.1/tcp/30303 \
    > /tmp/node2.log 2>&1 &
```

**Node 3 (partner server):**
```bash
ssh -p 50790 ilozada@147.93.84.44
nohup /home/ilozada/doli-node \
    --data-dir /home/ilozada/.doli/mainnet/data run \
    --producer \
    --producer-key /home/ilozada/.doli/mainnet/keys/producer_3.json \
    --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
    --no-auto-update --yes --force-start \
    --bootstrap /ip4/72.60.228.233/tcp/30303 \
    > /tmp/node3.log 2>&1 &
```

**Local devnet (for testing):**
```bash
cargo build --release
./target/release/doli-node devnet init --nodes 3
./target/release/doli-node devnet start
```

### 2.3 Stopping Nodes

**Graceful shutdown (preferred):**
```bash
# Sends SIGTERM — node finishes current block, saves state, exits
kill <pid>

# Find PIDs on omegacortex.ai
ssh ilozada@omegacortex.ai "pgrep -la doli-node"

# Find PID on node3
ssh -p 50790 ilozada@147.93.84.44 "pgrep -la doli-node"

# Devnet
./target/release/doli-node devnet stop
```

**NEVER use `kill -9` unless absolutely necessary** — it bypasses graceful shutdown and can create orphan blocks requiring shallow fork recovery.

### 2.4 Checking Node Status

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

**Process check:**
```bash
# omegacortex.ai (node1 + node2)
ssh ilozada@omegacortex.ai "pgrep -la doli-node"

# node3
ssh -p 50790 ilozada@147.93.84.44 "pgrep -la doli-node"
```

**Remote — quick status all 3 nodes:**
```bash
# Node 1 chain info
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'"

# Node 2 chain info
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8546 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'"

# Node 3 chain info
ssh -p 50790 ilozada@147.93.84.44 "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'"
```

**Tail logs:**
```bash
# Node 1
ssh ilozada@omegacortex.ai "tail -f /tmp/node1.log"
# Node 2
ssh ilozada@omegacortex.ai "tail -f /tmp/node2.log"
# Node 3
ssh -p 50790 ilozada@147.93.84.44 "tail -f /tmp/node3.log"
```

### 2.5 Checking Balances and Production

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

### 2.6 Wipe and Resync

**Partial wipe (keep keys, resync chain):**
```bash
# Stop node first
sudo systemctl stop doli-testnet

# Remove chain data only (example for node1 on omegacortex.ai)
rm -rf ~/.doli/mainnet/node1/data/blocks/
rm -rf ~/.doli/mainnet/node1/data/signed_slots.db/
rm -f ~/.doli/mainnet/node1/data/chain_state.bin
rm -f ~/.doli/mainnet/node1/data/producers.bin
rm -f ~/.doli/mainnet/node1/data/utxo_set.bin

# Restart node — resyncs from peers (see Section 2.2 for exact command)
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

### 2.7 RocksDB LOCK File Cleanup

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

# 2. Copy binary to node3
scp -P 50790 ilozada@omegacortex.ai:~/repos/doli/target/release/doli-node \
    ilozada@147.93.84.44:~/doli-node

# 3. Stop ALL nodes (kill gracefully)
ssh ilozada@omegacortex.ai "pkill -f doli-node"
ssh -p 50790 ilozada@147.93.84.44 "pkill -f doli-node"

# 4. Wait for graceful shutdown (check processes are gone)
sleep 5
ssh ilozada@omegacortex.ai "pgrep -la doli-node || echo 'stopped'"
ssh -p 50790 ilozada@147.93.84.44 "pgrep -la doli-node || echo 'stopped'"

# 5. Start ALL nodes (see Section 2.2 for exact commands)
# Start node1 first (bootstrap), then node2, then node3

# 6. Verify all nodes are on same chain — see 3.5
```

### 3.4 Non-Consensus: Rolling Upgrade

Network, sync, RPC, logging, or UI changes can be deployed one node at a time.

```bash
# For each node: stop, replace binary, restart
# Example for node1:
ssh ilozada@omegacortex.ai "kill \$(pgrep -f 'data-dir.*node1')"
sleep 3
# Restart node1 (see Section 2.2 for exact command)
# Verify node1 is healthy before proceeding to node2
```

### 3.5 Post-Deploy Verification

```bash
# 1. All nodes running
ssh ilozada@omegacortex.ai "pgrep -la doli-node"
ssh -p 50790 ilozada@147.93.84.44 "pgrep -la doli-node"

# 2. Chain is advancing (run twice, 15s apart, height should increase)
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'"

# 3. Peers connected (each node should see 2 peers)
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getNetworkInfo\",\"params\":{},\"id\":1}'"

# 4. No errors in recent logs
ssh ilozada@omegacortex.ai "tail -50 /tmp/node1.log | grep -iE 'error|panic|fatal'"
ssh ilozada@omegacortex.ai "tail -50 /tmp/node2.log | grep -iE 'error|panic|fatal'"
ssh -p 50790 ilozada@147.93.84.44 "tail -50 /tmp/node3.log | grep -iE 'error|panic|fatal'"

# 5. All nodes agree on chain tip (compare bestHash across nodes)
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | grep bestHash"
ssh ilozada@omegacortex.ai "curl -s -X POST http://127.0.0.1:8546 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | grep bestHash"
ssh -p 50790 ilozada@147.93.84.44 "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' | grep bestHash"
```

### 3.6 Remote Build + Deploy (Single Command)

```bash
# Build + restart node1+node2 on omegacortex.ai
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && \
    source ~/.cargo/env && \
    cargo build --release --package doli-node && \
    pkill -f doli-node && sleep 3"
# Then restart nodes per Section 2.2
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

### 6.4 Withdrawal

**Withdrawal has a 7-day delay (mainnet) / 10min (devnet) + vesting penalty:**

| Bond Age | Penalty |
|----------|---------|
| < 1 year | 75% burned |
| 1-2 years | 50% burned |
| 2-3 years | 25% burned |
| 3+ years | 0% |

```bash
# Request withdrawal (starts timer)
doli -r http://127.0.0.1:PORT producer request-withdrawal -c 2

# After delay, claim
doli -r http://127.0.0.1:PORT producer claim-withdrawal

# Emergency exit (penalty applies)
doli -r http://127.0.0.1:PORT producer exit --force
```

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
- If stuck > 5 minutes: restart the node. Graceful `kill <pid>` is safe.

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
kill <pid>

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
