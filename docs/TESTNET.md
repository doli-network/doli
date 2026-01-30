# DOLI Testnet

Official DOLI testnet for testing and development.

**Website**: [testnet.doli.network](https://testnet.doli.network)

---

## Testnet v2 (Fresh Genesis with Mainnet Parameters)

The testnet has been relaunched with parameters matching mainnet exactly.

| Status | Value |
|--------|-------|
| Genesis | January 29, 2026 22:00 UTC |
| Block Reward | **1 tDOLI** (matches mainnet) |
| Genesis Producers | 5 pre-registered |
| Slot Duration | 10 seconds |
| Epoch Length | 360 blocks (1 hour) |

**What's new in v2:**
- Block reward now 1 tDOLI (was 50, now matches mainnet exactly)
- 5 genesis producers pre-registered at genesis
- Epoch state persistence (rewards distribute correctly after restart)
- All consensus parameters match mainnet

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

### 2. Run as Producer

```bash
./target/release/doli-node --network testnet run --producer
```

Your node auto-connects to `testnet.doli.network` and starts producing blocks immediately.

### 3. Start Earning tDOLI

Once synced, you'll see:
```
Block produced at height X
```

You earn **50 tDOLI per block** you produce. No registration needed during bootstrap!

---

## Network Information

| Parameter | Value |
|-----------|-------|
| Network | Testnet |
| Address Prefix | `tdoli` |
| Slot Duration | 10 seconds |
| Block Reward | **1 tDOLI** |
| Epoch Length | 360 blocks (1 hour) |
| Genesis Producers | 5 |
| P2P Port | 40303 |
| RPC Port | 18545 |

---

## After Bootstrap (February 5+)

Once bootstrap ends, you'll need to register with bonds:

```bash
# Check your balance (should have tDOLI from producing during bootstrap)
./target/release/doli balance --rpc http://127.0.0.1:18545

# Register with 1 bond (1,000 tDOLI)
./target/release/doli producer register --bonds 1 --rpc http://127.0.0.1:18545

# Check status
./target/release/doli producer status --rpc http://127.0.0.1:18545
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

### 4. Run as Producer

```bash
./target/release/doli-node --network testnet run --producer
```

### 5. Run as Systemd Service (Recommended)

```bash
sudo tee /etc/systemd/system/doli-testnet.service > /dev/null << 'EOF'
[Unit]
Description=DOLI Testnet Producer
After=network.target

[Service]
Type=simple
User=YOUR_USER
ExecStart=/home/YOUR_USER/doli/target/release/doli-node --network testnet run --producer
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

## Seed Server

The testnet runs on `testnet.doli.network` (198.51.100.1).

### Maintainer Keys (Auto-Update System)

5 keys control protocol updates (3-of-5 threshold):

| # | Public Key |
|---|------------|
| 1 | `721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9` |
| 2 | `d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d` |
| 3 | `9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697` |
| 4 | `97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f` |
| 5 | `82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096` |

---

## Resources

- [CLI.md](./CLI.md) - CLI reference
- [RUNNING_A_NODE.md](./RUNNING_A_NODE.md) - Node guide
- [BECOMING_A_PRODUCER.md](./BECOMING_A_PRODUCER.md) - Producer guide
- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol spec

---

## Contact

- GitHub: [github.com/e-weil/doli](https://github.com/e-weil/doli)
