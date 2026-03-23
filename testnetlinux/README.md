# DOLI Local Testnet — Linux (systemd)

Operational environment for running a local DOLI testnet on Linux. Uses **systemd user services** — no sudo required.

**Code repo**: `~/repos/doli` (branch `DPFAESI`) — all builds happen here.
**This repo**: `~/repos/localdoli` — runtime data, keys, scripts, explorer only. No source code.

## Prerequisites

- `doli-node` binary built from `~/repos/doli`: `cd ~/repos/doli && cargo build --release -p doli-node`
- Producer keys in `testnetlinux/keys/producer_{1..12}.json` (included)
- Node.js (for the block explorer + swap bot)

## Quick Start

```bash
# 1. Build from doli repo (DPFAESI branch)
cd ~/repos/doli && cargo build --release -p doli-node -p doli-cli

# 2. Install systemd user services (seed + n1-n12 + explorer)
~/repos/localdoli/testnetlinux/scripts/install-services.sh

# 3. Start seed + N1-N5
~/repos/localdoli/testnetlinux/scripts/testnet.sh start seed
sleep 5
~/repos/localdoli/testnetlinux/scripts/testnet.sh start n1 n2 n3 n4 n5

# 4. Check status
~/repos/localdoli/testnetlinux/scripts/testnet.sh status

# 5. Open the explorer
xdg-open http://localhost:8080
```

## Service Management

```bash
scripts/testnet.sh start seed          # Start seed only
scripts/testnet.sh start n1 n5 n12     # Start specific producers
scripts/testnet.sh start all           # Start everything (seed + n1-n12 + explorer)
scripts/testnet.sh stop all            # Stop everything
scripts/testnet.sh restart all         # Restart everything
scripts/testnet.sh status              # Show status table
scripts/testnet.sh logs n1             # Tail n1 log
scripts/testnet.sh enable all          # Auto-start on boot
scripts/testnet.sh disable all         # Disable auto-start
```

## Port Layout

| Node | P2P | RPC | Metrics |
|------|-----|-----|---------|
| Seed | 30300 | 8500 | 9000 |
| N{i} | 30300+i | 8500+i | 9000+i |
| Explorer | - | 8080 (HTTP) | - |
| Swap bot | - | 3000 (HTTP) | - |

## Block Explorer

Node.js server that serves the explorer UI and proxies RPC calls to local nodes.

```bash
# Via systemd
scripts/testnet.sh start explorer

# Or manually
node testnetlinux/explorer/server.js
```

Pages:
- `http://localhost:8080` — Block explorer
- `http://localhost:8080/network.html` — Network status (auto-discovers running nodes)

## Directory Structure

```
testnetlinux/
├── bin/               # Copied binaries (optional, services point to ~/repos/doli/target/release/)
├── docs/              # Incident reports, implementation plans
├── doli-swap-bot/     # Swap bot (Node.js)
├── explorer/          # Block explorer (index.html, network.html, server.js)
├── genesis.md         # Chain reset procedure
├── keys/              # Producer key files
├── logs/              # Log files
├── n{1-12}/           # Node data directories
├── nodes{1-10}/       # Batch node data directories
├── scripts/
│   ├── install-services.sh   # Install systemd user services
│   ├── testnet.sh            # Manage services via systemctl --user
│   ├── auto-bond.sh          # Auto-bond script
│   └── auto-bond-others.sh   # Auto-bond for stress-test producers
└── seed/              # Seed node data
```

## Binary Source

All binaries are built from `~/repos/doli` (branch `DPFAESI`):
- `~/repos/doli/target/release/doli-node` — node binary
- `~/repos/doli/target/release/doli` — CLI binary

**Never build from localdoli** — this repo has no source code.

## Differences from macOS testnet/

| | macOS (`testnet/`) | Linux (`testnetlinux/`) |
|---|---|---|
| Init system | launchd (plists) | systemd (user services) |
| Config dir | `~/Library/LaunchAgents/` | `~/.config/systemd/user/` |
| Control | `launchctl load/start/stop` | `systemctl --user start/stop` |
| Auto-start | `RunAtLoad` in plist | `systemctl --user enable` |
| Linger | N/A | `loginctl enable-linger` |
| `stat` flag | `stat -f%z` | `stat -c%s` |
