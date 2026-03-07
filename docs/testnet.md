# DOLI Testnet

Official DOLI testnet for testing and development.

**Website**: [testnet.doli.network](https://testnet.doli.network)

---

## Testnet v3 (March 2026 Relaunch)

The testnet has been relaunched with 12 genesis producers and parameters matching mainnet exactly.

| Status | Value |
|--------|-------|
| Genesis | March 7, 2026 07:40:52 UTC |
| Block Reward | **1 tDOLI** (matches mainnet) |
| Genesis Producers | 12 pre-registered (NT1-NT12) |
| Slot Duration | 10 seconds |
| Epoch Length | 360 blocks (1 hour) |
| Bootstrap DNS | `bootstrap1.testnet.doli.network:40303` / `bootstrap2.testnet.doli.network:40304` |

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
| Genesis Producers | 12 (NT1-NT12) |
| P2P Port | 40303 |
| RPC Port | 18545 |
| Bootstrap | `bootstrap1.testnet.doli.network:40303` |

---

## Becoming a Registered Producer

To earn block rewards, you need to register as a producer with bonds:

```bash
# Check your wallet balance
doli balance -w ~/.doli/testnet/producer.json --rpc http://127.0.0.1:18545

# Register with 1 bond (10 tDOLI)
doli producer register --bonds 1 -w ~/.doli/testnet/producer.json --rpc http://127.0.0.1:18545

# Check registration status
doli producer status -w ~/.doli/testnet/producer.json --rpc http://127.0.0.1:18545

# List all network producers
doli producer list --rpc http://127.0.0.1:18545
```

**Bond stacking** - Add more bonds to increase your selection probability:
```bash
doli producer add-bond --count 2 -w ~/.doli/testnet/producer.json --rpc http://127.0.0.1:18545
```

---

## CLI Commands

Set the RPC endpoint once:
```bash
export DOLI_RPC=http://127.0.0.1:18545
```

Then use:
```bash
doli balance                    # Check balance
doli send <address> <amount>    # Send tDOLI
doli chain                      # Chain info
doli producer status            # Producer status
doli producer list              # List all producers
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

## Seed / Bootstrap Servers

| DNS | IP | Port | Node |
|-----|-----|------|------|
| `bootstrap1.testnet.doli.network` | 72.60.228.233 | 40303 | NT1 (omegacortex) |
| `bootstrap2.testnet.doli.network` | 72.60.228.233 | 40304 | NT2 (omegacortex) |

Both are relay-enabled and embedded as defaults in the binary — no `--bootstrap` flag needed.

### Maintainer Keys (Auto-Update System)

5 keys control protocol updates (3-of-5 threshold).
Hardcoded in binary at `crates/updater/src/lib.rs` for security.

---

## External Producers

Community and partner nodes running on testnet.

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
