# DOLI Testnet

Official DOLI testnet for testing and development.

**Website**: [testnet.doli.network](https://testnet.doli.network)

---

## Testnet v3 (March 2026 Relaunch)

The testnet has been relaunched with 6 genesis producers and parameters matching mainnet exactly. Additional producers (NT7-NT12) register on-chain after genesis.

| Status | Value |
|--------|-------|
| Genesis | March 7, 2026 07:40:52 UTC |
| Block Reward | **1 tDOLI** (matches mainnet) |
| Genesis Producers | 6 pre-registered (NT1-NT6) |
| Slot Duration | 10 seconds |
| Epoch Length | 360 blocks (1 hour) |
| Bootstrap DNS | `bootstrap1.testnet.doli.network:40300` / `bootstrap2.testnet.doli.network:40300` |

**To join:**
- Run with `--producer` flag to participate in block production
- Producers are selected in round-robin based on bond count

---

## Quick Start (3 Steps)

### 1. Build DOLI

```bash
git clone https://github.com/e-weil/doli.git
cd doli
nix develop
cargo build --release
```

### 2. Create a Producer Wallet

```bash
# Create wallet for your producer
doli new -w ~/.doli/testnet/producer.json

# View your public key
doli info -w ~/.doli/testnet/producer.json
```

### 3. Run as Producer

```bash
# Run node with your producer wallet
doli-node --network testnet run --producer --producer-key ~/.doli/testnet/producer.json
```

Your node auto-connects via `bootstrap1.testnet.doli.network` and starts syncing immediately.

Once synced, you'll see:
```
Block produced at height X
```

You earn **1 tDOLI per block** you produce (matches mainnet).

---

## Network Information

| Parameter | Value |
|-----------|-------|
| Network | Testnet |
| Address Prefix | `tdoli` |
| Slot Duration | 10 seconds |
| Block Reward | **1 tDOLI** |
| Epoch Length | 360 blocks (1 hour) |
| Genesis Producers | 6 (NT1-NT6), NT7-NT12 register on-chain |
| P2P Port | 40303 |
| RPC Port | 18545 |
| Bootstrap | `bootstrap1.testnet.doli.network:40300` |

---

## Becoming a Registered Producer

To earn block rewards, you need to register as a producer with bonds:

```bash
# Check your wallet balance
doli -n testnet -w ~/.doli/testnet/producer.json balance

# Register with 1 bond (10 tDOLI)
doli -n testnet -w ~/.doli/testnet/producer.json producer register --bonds 1

# Check registration status
doli -n testnet -w ~/.doli/testnet/producer.json producer status

# List all network producers
doli -n testnet -w ~/.doli/testnet/producer.json producer list
```

**Bond stacking** - Add more bonds to increase your selection probability:
```bash
doli -n testnet -w ~/.doli/testnet/producer.json producer add-bond --count 2
```

---

## CLI Commands

Use `-n testnet` to auto-detect RPC and address prefix (`tdoli1`):
```bash
doli -n testnet -w <wallet> balance                    # Check balance
doli -n testnet -w <wallet> send <address> <amount>    # Send tDOLI
doli -n testnet -w <wallet> chain                      # Chain info
doli -n testnet -w <wallet> producer status             # Producer status
doli -n testnet -w <wallet> producer list               # List all producers
```

Or use explicit RPC:
```bash
doli -n testnet -r http://198.51.100.1:18545 -w <wallet> balance
```

---

## Server Setup (Complete Guide)

### 1. Requirements

- Ubuntu 22.04+ or similar Linux
- 2+ CPU cores, 4 GB RAM, 50 GB SSD
- Port 40303 open

### 2. Install & Build

```bash
# Install Nix
curl -L https://nixos.org/nix/install | sh -s -- --daemon
exec $SHELL

# Build DOLI
git clone https://github.com/e-weil/doli.git
cd doli
nix develop --command cargo build --release
```

### 3. Open Firewall

```bash
sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'
sudo ufw enable
```

### 4. Create Producer Wallet

```bash
# Create wallet
./target/release/doli new -w ~/.doli/testnet/producer.json

# View wallet info (shows public key)
./target/release/doli info -w ~/.doli/testnet/producer.json
```

### 5. Run as Producer

```bash
./target/release/doli-node --network testnet run --producer --producer-key ~/.doli/testnet/producer.json
```

### 6. Run as Systemd Service (Recommended)

```bash
sudo tee /etc/systemd/system/doli-testnet.service > /dev/null << 'EOF'
[Unit]
Description=DOLI Testnet Producer
After=network.target

[Service]
Type=simple
User=YOUR_USER
ExecStart=/home/YOUR_USER/doli/target/release/doli-node --network testnet run --producer --producer-key /home/YOUR_USER/.doli/testnet/producer.json
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

sudo sed -i "s/YOUR_USER/$USER/g" /etc/systemd/system/doli-testnet.service
sudo systemctl daemon-reload
sudo systemctl enable doli-testnet
sudo systemctl start doli-testnet

# View logs
journalctl -u doli-testnet -f
```

---

## Chainspec Configuration (Network Operators Only)

**Note:** This section is for network operators launching a new network. Regular producers joining an existing network do NOT need chainspec - just use the CLI commands above.

Chainspec files define genesis producers (pre-registered at block 0) for new network launches. This follows industry standards (Ethereum, Cosmos, Polkadot).

### When You Need Chainspec

- Launching a new testnet/mainnet from scratch
- Running the seed nodes that bootstrap a network
- NOT needed for joining an existing network

### Generating Chainspec

```bash
# 1. Create wallets for genesis producers
for i in 1 2 3 4 5; do
    doli new -w ~/.doli/genesis/producer_$i.json
done

# 2. Generate chainspec (automatically extracts pubkeys)
./scripts/generate_chainspec.sh testnet ~/.doli/genesis testnet.json

# 3. Start genesis node with chainspec
doli-node --network testnet --chainspec testnet.json run \
    --producer --producer-key ~/.doli/genesis/producer_1.json
```

See [genesis.md](./genesis.md) for complete network launch procedures.

---

## Troubleshooting

### Node won't sync
```bash
nc -zv testnet.doli.network 40303  # Test connectivity
sudo ufw status                     # Check firewall
```

### Not producing blocks
1. Ensure `--producer` flag is set
2. Wait for sync to complete
3. Wait 15 seconds for producer discovery

### Check node status
```bash
journalctl -u doli-testnet | grep -i "height\|produced"
```

---

## Binary Segregation

Testnet and mainnet use **separate binaries** at standardized paths on both servers.

| Network | Binary Path | Servers |
|---------|-------------|---------|
| Mainnet | `/mainnet/bin/doli-node` | ai1, ai2 |
| Testnet | `/testnet/bin/doli-node` | ai1, ai2 |

**Upgrade testnet (both servers):**
```bash
# ai1:
ssh ilozada@198.51.100.1 "sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node && \
  sudo systemctl restart doli-testnet-seed && sleep 5 && \
  sudo systemctl restart doli-testnet-nt1 doli-testnet-nt3 doli-testnet-nt5"

# ai2:
ssh ilozada@198.51.100.2 "sudo cp /tmp/doli-node-new /testnet/bin/doli-node && sudo chmod 755 /testnet/bin/doli-node && \
  sudo systemctl restart doli-testnet-seed && sleep 5 && \
  sudo systemctl restart doli-testnet-nt2 doli-testnet-nt4 doli-testnet-nt6"
```

---

## Seed / Bootstrap Servers

Dedicated archive+relay seed nodes, separated from producers for security and HA.

| DNS | IP | Port | Server | Role |
|-----|-----|------|--------|------|
| `bootstrap1.testnet.doli.network` | 198.51.100.1 | 40300 | ai1 | Seed + Archive + Relay |
| `bootstrap2.testnet.doli.network` | 198.51.100.2 | 40300 | ai2 | Seed + Archive + Relay |

Both run as `doli-testnet-seed` systemd service with `--relay-server --archive-to /testnet/seed/blocks`.

### DNS Records

| DNS | IP | Purpose |
|-----|-----|---------|
| `seed1.doli.network` | 198.51.100.1 (ai1) | Mainnet P2P seed |
| `seed2.doli.network` | 198.51.100.2 (ai2) | Mainnet P2P seed |
| `bootstrap1.testnet.doli.network` | 198.51.100.1 (ai1) | Testnet P2P seed |
| `bootstrap2.testnet.doli.network` | 198.51.100.2 (ai2) | Testnet P2P seed |
| `testnet.doli.network` | 198.51.100.2 | Testnet web |
| `archive.doli.network` | 198.51.100.2 | Archive RPC |

### Maintainer Keys (Auto-Update System)

Each network has its own set of 5 maintainer keys (3-of-5 threshold for release signing).
Hardcoded in binary at `crates/updater/src/lib.rs` for security.

- **Mainnet**: N1-N5 are producers AND maintainers. N6-N12 are producers only.
- **Testnet**: NT1-NT5 are producers AND maintainers. NT6-NT12 are producers only.

---

## External Producers

Community and partner nodes.

### Mainnet

| Operator | Host | Wallet Address | Joined |
|----------|------|----------------|--------|
| atinoco | doli02 | `doli17f7pqlkfjweddk88ry6gtc23hvmptsqk2epxx7h6x9a8gvan3crsfl243e` | 2026-03-07 |
| antonio | — | `doli1nc3erj8tqew5yz09s60ang7n77p3ftjh7e9m370w3v5c95aaj38qvv98wl` | 2026-03-07 |
| daniel | — | `doli1p7s6hcacnm6t64nk670leeu9w3tvnkvwc688r9zlvh2f3573f6vs4cynzh` | 2026-03-07 |


### Testnet

| Operator | Host | Wallet Address | Joined |
|----------|------|----------------|--------|
| atinoco | doli02 | `tdoli17axj5cjstmwqs8a4zg6xxy5qjwnd7j7dnggyrhy3gya37x7ckrhsefjvfy` | 2026-03-07 |

---

## Resources

- [cli.md](./cli.md) - CLI reference
- [running_a_node.md](./running_a_node.md) - Node guide
- [becoming_a_producer.md](./becoming_a_producer.md) - Producer guide
- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol spec

---

## Contact

- GitHub: [github.com/e-weil/doli](https://github.com/e-weil/doli)
