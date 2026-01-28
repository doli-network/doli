# Becoming a DOLI Block Producer

This guide explains how to register as a block producer on the DOLI network.

## Overview

Block producers create new blocks and earn rewards. To become a producer, you must:

1. Complete a VDF registration proof (~10 minutes)
2. Lock a bond (collateral)
3. Run a reliable node

## Requirements

### Hardware

DOLI's Proof of Time uses two mechanisms:
1. **Epoch Lookahead**: Leaders determined at epoch start (prevents grinding)
2. **Heartbeat VDF**: ~700ms proof of continuous presence

**Minimum Requirements:**
- CPU: Any modern CPU (2015+)
- RAM: 4GB minimum (8GB recommended)
- Storage: 50GB+ SSD recommended
- Network: Stable connection with good uptime

**VDF Timing:**

| Network | Slot  | VDF Time | Purpose                    |
|---------|-------|----------|----------------------------|
| All     | 60/10/5s | ~700ms | Heartbeat proof of presence|

**Note**: The VDF is a lightweight heartbeat (~700ms), not a bottleneck. Grinding prevention comes from Epoch Lookahead (deterministic leader selection based on bond distribution, not prev_hash).

### Financial (Mainnet)

| Era | Bond Required | Block Reward |
|-----|---------------|--------------|
| 1 (Years 0-4) | 1,000 DOLI | 5.0 DOLI |
| 2 (Years 4-8) | 700 DOLI | 2.5 DOLI |
| 3 (Years 8-12) | 490 DOLI | 1.25 DOLI |
| 4+ | Decreases 30% per era | Decreases 50% per era |

The bond is locked for a 4-year commitment period. Early exit incurs a proportional penalty (recycled to reward pool). Normal exit after 4 years requires a 30-day unbonding period.

### Bond Stacking

Producers can stake multiple bonds (1-100) to increase their block production share:

| Parameter | Value | Notes |
|-----------|-------|-------|
| Minimum | 1 bond | 1,000 DOLI (Era 1) |
| Maximum | 100 bonds | 100,000 DOLI (anti-whale cap) |
| Selection | Deterministic | NOT lottery/probabilistic |

**How It Works:**

DOLI uses **deterministic round-robin rotation**, not probabilistic lottery:

```
Alice: 1 bond  → produces 1 block every 10 slots
Bob:   5 bonds → produces 5 blocks every 10 slots
Carol: 4 bonds → produces 4 blocks every 10 slots
```

**Key Difference from PoS:**
- PoS Lottery: Bob *might* produce 10 blocks or 0 (high variance)
- DOLI Round-Robin: Bob *always* produces exactly 5 of every 10 blocks

**Equitable ROI:**

| Producer | Investment | Blocks/Cycle | Reward/Cycle | ROI % |
|----------|------------|--------------|--------------|-------|
| Alice | 1,000 DOLI | 1 | 5 DOLI | 0.5% |
| Bob | 5,000 DOLI | 5 | 25 DOLI | 0.5% |
| Carol | 4,000 DOLI | 4 | 20 DOLI | 0.5% |

All producers earn the **same percentage return**. More bonds = more absolute return, same ROI %.

### Network-Specific Parameters

For testing on testnet or devnet, requirements are lower:

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| Initial Bond | 1,000 DOLI | 10 DOLI | 1 DOLI |
| Block Reward | 5 DOLI | 50 DOLI | 500 DOLI |
| Slot Duration | 60s | 10s | 5s |
| VDF Time | ~700ms | ~700ms | ~700ms |
| Veto Period | 7 days | 1 day | 1 min |

All networks use the same ~700ms heartbeat VDF. Leader selection is deterministic (Epoch Lookahead), not dependent on VDF timing.

## Step 1: Set Up a Node

First, run a full node and wait for it to sync:

```bash
# Build
cargo build --release

# Run on mainnet (default)
./doli-node run

# Or run on testnet for testing
./doli-node --network testnet run

# Or run on devnet for local development
./doli-node --network devnet run

# Wait for sync to complete
# Check with (adjust port for your network):
# Mainnet: curl http://localhost:8545 -d '{"jsonrpc":"2.0","method":"getNetworkInfo","id":1}'
# Testnet: curl http://localhost:18545 -d '{"jsonrpc":"2.0","method":"getNetworkInfo","id":1}'
# Devnet:  curl http://localhost:28545 -d '{"jsonrpc":"2.0","method":"getNetworkInfo","id":1}'
```

**Tip**: Start with devnet for local testing, then testnet, then mainnet.

## Step 2: Create a Wallet

```bash
# Create a new wallet
./doli wallet new --name producer

# Show your address
./doli wallet addresses
```

## Step 3: Fund Your Wallet

You need:
- Bond amount (e.g., 1,000 DOLI for Era 1)
- Transaction fees

Transfer DOLI to your wallet address.

## Step 4: Generate Registration VDF

The registration VDF proves you've invested time to create an identity. The time varies by network:

| Network | Registration VDF | Block VDF | Bond Required |
|---------|------------------|-----------|---------------|
| Mainnet | ~10 min          | ~700ms    | 1,000 DOLI    |
| Testnet | ~30 sec          | ~700ms    | 10 DOLI       |
| Devnet  | ~5 sec           | ~700ms    | 1 DOLI        |

All networks use the same ~700ms heartbeat VDF for block production.

**Devnet Bootstrap Mode:** On devnet, if no producers are registered, any node with `--producer` flag can produce blocks without registration. See the "Bootstrap Mode" section below.

```bash
# Start VDF computation (mainnet)
./doli producer register

# For testnet
./doli --network testnet producer register

# This will:
# 1. Generate a VDF proof
# 2. Create a registration transaction
# 3. Lock your bond
# 4. Broadcast to the network
```

## Step 5: Start Producing Blocks

Once registered, configure your node for production:

```bash
# Export your producer key
./doli wallet export --producer-key /path/to/producer.key

# Run node with producer enabled (mainnet)
./doli-node run --producer --producer-key /path/to/producer.key

# For testnet
./doli-node --network testnet run --producer --producer-key /path/to/producer.key
```

## Monitoring Your Status

```bash
# Check producer status (mainnet)
./doli producer status

# For testnet
./doli --network testnet producer status

# Example output:
# Network:       testnet
# Public Key:    abc123...
# Status:        active
# Registered at: block 12345
# Bond Amount:   10.00000000 DOLI
# Blocks Made:   42
# Current Era:   1
```

## Understanding Slot Assignment

Slot duration varies by network: 60 seconds on mainnet/testnet, 5 seconds on devnet.

Each slot is assigned to producers deterministically (times shown for mainnet/testnet):

| Time in Slot | Eligible Producers |
|--------------|-------------------|
| 0-30s | Primary producer only |
| 30-45s | Primary or secondary |
| 45-60s | Primary, secondary, or tertiary |

**Devnet timing** (5-second slots): 0-2.5s primary, 2.5-3.75s secondary, 3.75-5s tertiary.

If the primary producer is offline, fallback producers can create the block.

**Note:** This applies to registered producers. For devnet bootstrap mode (no registered producers), see the "Bootstrap Mode" section below.

## Bootstrap Mode (Devnet Only)

On devnet, when there are no registered producers, the network operates in **bootstrap mode**. This allows rapid development without going through the registration process.

### Enabling Bootstrap Production

```bash
# No registration needed on devnet with bootstrap mode
./doli-node --network devnet run \
    --producer \
    --producer-key ~/.doli/devnet/wallet.json
```

### Leader Election in Bootstrap Mode

Bootstrap mode uses a deterministic scoring algorithm to prevent multiple producers from racing:

1. **Slot Seed**: Each slot has a deterministic seed based on the slot number only (not the previous block hash, to ensure all nodes compute the same election)

2. **Producer Score**: Each producer's public key is hashed with the slot seed to get a score (0-255)

3. **Time Offset**: The score maps to a time offset within the slot:
   ```
   Score × 80% / 255 = % of slot before producing
   ```
   - Score 0 → Produce immediately
   - Score 128 → Wait ~40% of slot
   - Score 255 → Wait 80% of slot

4. **First Valid Wins**: The first valid block to arrive wins; other producers skip their attempt

### Sync-Before-Produce

New nodes use **sync-before-produce** logic (not time-based delays):

| Condition | Action |
|-----------|--------|
| No peers connected | Produce immediately (you're the seed node) |
| Peers connected, chain behind | Wait for sync to complete |
| Peers connected, chain synced | Produce (within 2 slots of best peer) |

**This design scales globally:**
- No arbitrary time delays
- Thousands of nodes can start simultaneously
- Each node naturally syncs before competing to produce
- Seed nodes bootstrap the network immediately

### Multi-Node Development Setup

When running multiple producer nodes locally:

1. **Start sequentially**: Start the seed node first, wait 5+ seconds, then start others
2. **Use unique ports**: Each node needs separate P2P, RPC, and metrics ports
3. **Connect via bootstrap**: Non-seed nodes should use `--bootstrap` to connect

```bash
# Node 1: Seed (starts producing after warmup)
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-1 \
    --p2p-port 50301 --rpc-port 28541 --metrics-port 9091 \
    --producer --producer-key ~/.doli/devnet-1/wallet.json

# Wait 5 seconds, then start Node 2
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-2 \
    --p2p-port 50302 --rpc-port 28542 --metrics-port 9092 \
    --producer --producer-key ~/.doli/devnet-2/wallet.json \
    --bootstrap /ip4/127.0.0.1/tcp/50301
```

### Transitioning from Bootstrap to Registered

Once you're ready to test the full registration flow:

1. Run nodes without `--producer` flag (observer mode)
2. Use `./doli --network devnet producer register` to register
3. Once registered, enable `--producer` flag
4. Bootstrap mode automatically deactivates when registered producers exist

## Rewards

### Epoch-Based Distribution

DOLI uses a **Pool-First Epoch Reward Distribution** system for fair reward allocation:

1. **During the epoch**: Block rewards accumulate in a pool (no immediate coinbase)
2. **At epoch boundary**: Pool is divided equally among all participating producers
3. **Deterministic remainder**: First producer (sorted by pubkey) receives any dust

This ensures all active producers receive equal rewards regardless of how many blocks they produced during the epoch.

### Reward Sources

Producers receive:
- **Epoch rewards** (block reward × slots in epoch ÷ active producers)
- **Transaction fees** from included transactions

### Maturity

All rewards (epoch rewards and coinbase) require **100 confirmations** before spending. This prevents short-term attacks and ensures network stability.

## Slashing

Your bond can ONLY be slashed for double production (creating two different blocks for the same slot). This is the only unambiguously intentional offense.

| Behavior | Consequence |
|----------|-------------|
| Invalid block | Rejected by network. You lose the slot. Bond intact. |
| Repeated inactivity | Removed from producer set. Bond intact. |
| Double production | 100% of bond burned. |

**Why this design?**

Invalid blocks can happen due to software bugs or configuration errors. The natural penalty (losing the slot and its reward) is sufficient. Following Bitcoin's philosophy: the network rejects bad blocks, you wasted your time, end of story.

Double production requires signing two different blocks for the same slot. This cannot happen by accident. It's deliberate fraud and deserves maximum punishment.

## Exiting

To stop being a producer:

```bash
./doli producer withdraw
```

### Normal Exit (after 4-year commitment)

This initiates a 30-day unbonding period:
1. You are immediately removed from the producer set
2. Slashing still applies during unbonding (for past infractions)
3. After 30 days, your full bond is released
4. Run `./doli producer claim-bond` to retrieve your bond

### Early Exit (before 4-year commitment)

If you exit before completing your 4-year commitment:
1. Penalty is proportional to time remaining
2. Penalty goes to the reward pool (not burned)
3. You receive: `bond * (time_completed / commitment_period)`

| Time Completed | Penalty | Bond Returned |
|----------------|---------|---------------|
| 1 year (25%)   | 75%     | 250 DOLI      |
| 2 years (50%)  | 50%     | 500 DOLI      |
| 3 years (75%)  | 25%     | 750 DOLI      |
| 4 years (100%) | 0%      | 1,000 DOLI    |

## Software Updates

As a producer, you have veto power over software updates.

### Auto-Update System

The node includes automatic updates with producer veto:

- All updates require 3 of 5 maintainer signatures
- Veto period varies by network:
  - Mainnet: 7 days
  - Testnet: 1 day
  - Devnet: 1 minute
- 33% of producers can veto any update

### Voting on Updates

When a new update is available:

```bash
# View pending update
./doli update status

# Vote to approve (default if no action)
./doli update vote approve

# Vote to veto
./doli update vote veto
```

### Configuration

```bash
# Enable auto-updates (default)
./doli-node run --auto-update=true

# Disable auto-updates (notification only)
./doli-node run --auto-update=false
```

## Best Practices

### Uptime

- Run on reliable infrastructure (VPS, dedicated server)
- Use process managers (systemd, Docker)
- Set up monitoring and alerts

### Security

- Keep producer key encrypted and backed up
- Use firewall rules
- Regular security updates

### Redundancy

- Consider running a hot standby node
- Keep backups of your producer key

## Troubleshooting

### Not getting assigned slots

1. Verify registration is confirmed
2. Check that node is synced
3. Confirm producer key is loaded

### Missing blocks

1. Check network connectivity
2. Verify VDF computation completes in time
3. Review node logs for errors

### Falling behind

If you consistently produce blocks late (after fallbacks):
1. Check the VDF calibration logs - iterations should auto-adjust to maintain ~700ms timing
2. Verify no other processes are consuming CPU during VDF computation
3. The dynamic calibration system should handle most hardware differences automatically
