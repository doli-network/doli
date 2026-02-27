# Producer Node Quickstart - DOLI Mainnet

Step-by-step guide to set up a producer node on DOLI mainnet.

---

## Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 8 GB | 16+ GB |
| Storage | 100 GB SSD | 500+ GB NVMe |
| Network | 50 Mbps | 100+ Mbps |
| Bond | 10 DOLI (1 bond) | - |

---

## Step 1: Clone and build

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# System dependencies (Ubuntu/Debian)
sudo apt install build-essential pkg-config libssl-dev libgmp-dev librocksdb-dev

# Clone and build
git clone https://github.com/e-weil/doli.git
cd doli
cargo build --release
```

> **WARNING: `--release` is mandatory.** Debug builds (`cargo build` without `--release`) produce a binary that is ~10x slower for VDF computation, causing block production timeouts, sync failures, and fork divergence. Debug binaries are also ~2x larger (~17MB vs ~8MB). If your binary is larger than 10MB, you have a debug build — rebuild with `--release`.

Binaries are in `target/release/`:
- `doli-node` — full node
- `doli` — wallet CLI

---

## Step 2: Create producer wallet

```bash
# IMPORTANT: -w is a global flag, it goes BEFORE the subcommand
./target/release/doli -w ~/.doli/mainnet/producer.json new
```

> **Note:** `-w` (wallet path) is a global CLI flag. It always goes before the subcommand (`new`, `info`, `balance`, etc.), never after.

Verify it was created:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json info
```

The output shows three values:
- **Address (20-byte)** — DO NOT use for sending
- **Pubkey Hash (32-byte)** — USE THIS to receive funds
- **Public Key** — verification only

Back up the wallet file:

```bash
cp ~/.doli/mainnet/producer.json ~/backup/
```

---

## Step 3: Fund the wallet

Get the correct address (32-byte Pubkey Hash):

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json info
```

From a funded wallet, send to the new producer's Pubkey Hash:

```bash
./target/release/doli -w ~/.doli/mainnet/funded_wallet.json \
    --rpc http://127.0.0.1:8545 \
    send <PRODUCER_PUBKEY_HASH> 15
```

You need: bond (10 DOLI) + registration fee + operational margin.

Verify balance:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    balance
```

---

## Step 4: Register as producer

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register
```

This starts the registration VDF (~10 minutes) and submits the registration transaction with 1 bond (10 DOLI).

To register with more bonds:

```bash
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register --bonds 5
```

---

## Step 5: Start the producer node

```bash
./target/release/doli-node run \
    --producer \
    --producer-key ~/.doli/mainnet/producer.json \
    --no-auto-update \
    --p2p-port 30303 \
    --rpc-port 8545
```

With a bootstrap node (to join an existing network):

```bash
./target/release/doli-node run \
    --producer \
    --producer-key ~/.doli/mainnet/producer.json \
    --no-auto-update \
    --bootstrap /ip4/<BOOTSTRAP_IP>/tcp/30303
```

> **Note:** `--no-auto-update` is recommended during early mainnet while the update system uses bootstrap keys. Once maintainer keys are derived on-chain, this flag can be removed.

### systemd service (production)

```bash
mkdir -p ~/.config/systemd/user/
```

Create `~/.config/systemd/user/doli-producer.service`:

```ini
[Unit]
Description=DOLI Producer Node
After=network.target

[Service]
Type=simple
ExecStart=%h/repos/doli/target/release/doli-node run \
    --producer \
    --producer-key %h/.doli/mainnet/producer.json \
    --no-auto-update \
    --p2p-port 30303 \
    --rpc-port 8545
Restart=always
RestartSec=10
LimitNOFILE=65536

[Install]
WantedBy=default.target
```

Enable and start:

```bash
systemctl --user daemon-reload
systemctl --user enable doli-producer
systemctl --user start doli-producer
systemctl --user status doli-producer
```

---

## Step 6: Verify production

```bash
# Producer status
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    producer status

# Balance (should increase with rewards)
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    balance

# Chain info
./target/release/doli -w ~/.doli/mainnet/producer.json \
    --rpc http://127.0.0.1:8545 \
    chain
```

---

## Firewall

Open only the P2P port:

```bash
sudo ufw allow 30303/tcp
```

DO NOT expose the RPC port (8545) to the internet.

---

## Backup

| File | Priority | Note |
|------|----------|------|
| `~/.doli/mainnet/producer.json` | Critical | Producer key — losing it = losing the bond |
| `~/.doli/mainnet/node.key` | High | Node identity — without it the PeerId changes |
| `~/.doli/mainnet/db/` | Low | Can be resynced |

---

## Warnings

- **NEVER** run two nodes with the same producer key simultaneously. This causes slashing (100% of bond burned).
- **NEVER** share the `producer.json` file — it contains the private key.
- Coinbase rewards have a 100-block maturity period before they can be spent.
- The bond is locked for 4 years. Early withdrawal incurs penalties: 75% (year 0-1), 50% (year 1-2), 25% (year 2-3), 0% (year 3+).

---

*Last updated: February 2026*
