# becoming_a_producer.md - Block Producer Guide

This guide covers how to become a block producer in the DOLI network.

---

## 1. Overview

Block producers in DOLI:
- Create new blocks at their assigned slots
- Compute VDF proofs (1,000 iterations per block)
- Earn block rewards (1 DOLI per block in Era 1)
- Require a bond (10 DOLI per bond unit on mainnet, 1 DOLI on testnet/devnet)

**Key differences from other systems:**
- **Deterministic selection** - You know exactly when your blocks will be produced
- **No pooling needed** - Equal ROI regardless of stake size
- **Time-based consensus** - VDF computation proves sequential time

---

## 2. Requirements

### 2.1. Hardware

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 8 GB | 16+ GB |
| Storage | 100 GB SSD | 500+ GB NVMe |
| Network | 50 Mbps | 100+ Mbps |
| Uptime | 95% | 99%+ |

**Critical:** VDF computation requires consistent CPU performance. Avoid:
- Shared hosting / VPS with noisy neighbors
- CPU throttling
- Power-saving modes

### 2.2. Economic

| Requirement | Amount | Vesting Period |
|-------------|--------|----------------|
| Bond unit | 10 DOLI (mainnet), 1 DOLI (testnet/devnet) | 4 years mainnet (4 × 1yr quarters), 1 day testnet (4 × 6h quarters) |
| Maximum bonds | 3,000 bonds (30,000 DOLI mainnet) | Same |

**Bond Stacking**: You can hold multiple bonds (up to 3,000), each with its own creation time and vesting schedule. Withdrawals follow FIFO (First-In-First-Out) order, withdrawing oldest bonds first.

### 2.3. Time Commitment

| Phase | Duration |
|-------|----------|
| Registration VDF | Near-instant (1,000 iterations on all networks) |
| Bond vesting | 4 years mainnet (0% penalty after 3 years), 1 day testnet (0% penalty after 18h) |
| Unbonding period | 60,480 blocks (~7 days) mainnet, 72 blocks (~12 min) testnet |
| Withdrawal delay | Same as unbonding period |

---

## 3. Registration Process

### 3.1. Complete Deployment Workflow

Deploying a new producer requires **4 steps**. Missing any step will result in a registered producer that cannot produce blocks.

| Step | Action | Result |
|------|--------|--------|
| 1. Create wallet | `doli wallet new` | Private key created |
| 2. Fund wallet | Send DOLI to new address | Balance for bond + fees |
| 3. Register | `doli producer register` | Public key on blockchain |
| 4. **Start node** | `doli-node run --producer --producer-key <wallet>` | **Blocks produced** |

**Common Mistake:** Completing steps 1-3 but not step 4. Registration only puts the public key on the blockchain. To actually produce blocks, you **must** run a node with the corresponding private key.

### 3.2. Generate Producer Key

```bash
# Create a new producer wallet/keypair
doli --wallet ~/.doli/keys/my_producer.json wallet new

# Display the public key (for funding)
doli --wallet ~/.doli/keys/my_producer.json wallet address

# Backup the key securely!
cp ~/.doli/keys/my_producer.json ~/secure-backup/
```

### 3.3. Fund the Producer Address

Transfer the bond amount plus fees to your producer address.

**⚠️ CRITICAL: Use "Pubkey Hash", NOT "Public Key"**

The `doli info` command shows three values. You MUST use the **Pubkey Hash (32-byte)** for sending:

```bash
doli --wallet ~/.doli/keys/my_producer.json info
# Output:
#   Address (20-byte):     cf98716522ee9e5c...              ❌ DON'T USE
#   Pubkey Hash (32-byte): cf98716522ee9e5c62f9...686eab84  ✅ USE THIS
#   Public Key:            cc9a1710b8bffb38...22d7cb51      ❌ DON'T USE
```

| Field | Use For |
|-------|---------|
| **Pubkey Hash (32-byte)** | **Sending coins, RPC queries** |
| Public Key | Verification only (different hash!) |

```bash
# Get the CORRECT address (Pubkey Hash)
NEW_ADDR=$(doli --wallet ~/.doli/keys/my_producer.json info | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')

# From an existing funded wallet, send to new producer
doli --wallet ~/.doli/keys/funded_wallet.json \
    --rpc http://127.0.0.1:8500 \
    send $NEW_ADDR 110

# Check balance on new wallet
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8500 \
    balance

# You need: bond amount + registration fee + operational buffer
# Example: 10 DOLI bond + 0.01 fee + 1 DOLI buffer = 11.01 DOLI
```

### 3.4. Register as Producer

```bash
# Register with minimum bond (10 DOLI = 1 bond)
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8500 \
    producer register

# Register with multiple bonds for more slots
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8500 \
    producer register --bonds 5  # 50 DOLI = 5 bonds
```

**What happens during registration:**
1. Node computes registration VDF (near-instant, 1,000 iterations)
2. Registration transaction submitted to network
3. Bond begins vesting (4 years mainnet, 1 day testnet)
4. Producer added to active set after ACTIVATION_DELAY (10 blocks)

### 3.5. Start Producing (CRITICAL)

**This step is required.** Without a running node, your registered producer cannot produce blocks.

```bash
# Run node with producer mode enabled
doli-node run \
    --producer \
    --producer-key ~/.doli/keys/my_producer.json \
    --p2p-port 30300 \
    --rpc-port 8500
```

For networks with existing nodes, add bootstrap:

```bash
# Connect to existing network
doli-node run \
    --network mainnet \
    --producer \
    --producer-key ~/.doli/keys/my_producer.json \
    --bootstrap /dns4/seed1.doli.network/tcp/30300
```

### 3.6. Verify Production

After starting the node, verify it's producing:

```bash
# Check producer status
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8500 \
    producer status

# Watch for production in logs
grep "Producing block" /var/log/doli-node.log

# Check balance is increasing (rewards)
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8500 \
    wallet balance
```

**Troubleshooting: Producer registered but not producing**

| Symptom | Cause | Solution |
|---------|-------|----------|
| Balance stuck at initial amount | No node running with this key | Start node with `--producer-key` |
| Node running but no blocks | Wrong key file | Verify `--producer-key` matches registered key |
| "Producing block" in logs but no rewards | Blocks orphaned | Check sync status, peer connectivity |

---

## 4. Understanding Selection

### 4.1. Ticket-Based Round-Robin

Selection is NOT a lottery. It's deterministic based on:
- Slot number
- Active producer set (sorted by pubkey)
- Bond counts (each bond = 1 ticket)

**How tickets work:**
- Producers are sorted by public key (deterministic ordering)
- Each producer gets consecutive tickets equal to their bond count
- Primary producer: `slot % total_tickets` determines which ticket is selected

```
Example with 2 producers (5 total bonds/tickets):

Alice: 3 bonds → tickets 0, 1, 2
Bob:   2 bonds → tickets 3, 4

Selection pattern (repeating every 5 slots):
Slot 0: 0 % 5 = 0 → ticket 0 (Alice)
Slot 1: 1 % 5 = 1 → ticket 1 (Alice)
Slot 2: 2 % 5 = 2 → ticket 2 (Alice)
Slot 3: 3 % 5 = 3 → ticket 3 (Bob)
Slot 4: 4 % 5 = 4 → ticket 4 (Bob)
Slot 5: 5 % 5 = 0 → ticket 0 (Alice) [cycle repeats]
```

### 4.2. Calculating Your Slots

```
Your blocks per epoch = (your_bonds / total_bonds) × blocks_per_epoch
  (mainnet: 360, testnet: 36, devnet: 4)
Your blocks per day = (your_bonds / total_bonds) × 8,640
```

### 4.3. Fallback Mechanism

If the primary producer is offline, fallback producers become eligible in exclusive 2-second windows:

| Time in slot | Eligible rank |
|--------------|---------------|
| 0-2 seconds | Rank 0 only (primary) |
| 2-4 seconds | Rank 1 (single fallback) |

2 ranks x 2,000ms = 4,000ms. Rank 1 only produces if rank 0's block hasn't arrived via gossip after 2 seconds.

**Fallback calculation:** The fallback producer at rank 1 is selected by offsetting the primary ticket:
`offset = total_bonds * rank / MAX_FALLBACK_RANKS`

Only one rank is eligible at any given time. Producers verify block existence before and after VDF computation to prevent duplicates.

---

## 5. Rewards

### 5.1. Block Rewards

| Era | Years | Reward per Block |
|-----|-------|------------------|
| 1 | 0-4 | 1.0 DOLI |
| 2 | 4-8 | 0.5 DOLI |
| 3 | 8-12 | 0.25 DOLI |
| 4 | 12-16 | 0.125 DOLI |

### 5.2. Distribution Mechanism

Rewards are distributed via a **pooled epoch model**:

| Aspect | Behavior |
|--------|----------|
| Per-block | Coinbase output goes to the **reward pool** (not directly to producer) |
| Epoch end | Pool is drained and distributed to qualified producers, weighted by bond count |
| Maturity | 6 blocks (~1 minute) before spendable |
| Empty slots | No reward generated for missed slots |

**Key characteristics:**

- **Pooled**: Each block's coinbase goes to a deterministic reward pool address
- **Epoch distribution**: At epoch boundary, pool is distributed bond-weighted to qualified producers
- **Deterministic**: Reward amount based on era (halves every ~4 years)
- **Maturity period**: Coinbase outputs require 6 confirmations before spending

**Example:**

```
Epoch of 360 blocks (mainnet):
- Each block: 1 DOLI coinbase to reward pool
- Epoch end: 360 DOLI distributed to qualified producers by bond weight
- Spendable after: 6 blocks (~1 minute)
```

### 5.3. ROI Calculation

**All producers earn identical ROI percentage per bond:**

```
ROI per block = reward / bond_unit
ROI per block = 1 DOLI / 10 DOLI = 10%

Blocks per year (with 1 bond, 1000 total bonds):
= 3,153,600 slots × (1/1000) = 3,153.6 blocks

Annual return = 3,153.6 × 1 DOLI = 3,153.6 DOLI
Annual ROI = 3,153.6 / 10 = 31,536%
```

**Note:** ROI decreases as more producers join and rewards halve. This example assumes 1000 total bonds across all producers.

### 5.4. Fee Income

Producers also earn transaction fees from included transactions.

---

## 6. Bond Management

### 6.1. Bond Stacking

You can hold multiple bonds (up to 3,000), each with its own vesting schedule:
- Each bond = 10 DOLI (mainnet) or 1 DOLI (testnet/devnet) = 1 ticket per cycle
- Maximum: 3,000 bonds = 30,000 DOLI (mainnet)
- Bonds vest individually based on their creation time

### 6.2. Adding Bonds

Increase your stake (up to 3,000 bonds / 30,000 DOLI on mainnet):

```bash
# Add 2 more bonds (20 DOLI)
./target/release/doli producer add-bond --count 2
```

### 6.3. Requesting Withdrawal

Withdrawal requires an **unbonding period** (60,480 blocks / ~7 days on mainnet, 72 blocks on testnet). Consumes Bond UTXOs in FIFO order (oldest first), applies vesting penalty.

```bash
# Request withdrawal of 1 bond (oldest Bond UTXO, subject to unbonding period)
./target/release/doli producer request-withdrawal --count 1
```

The penalty is the difference between bond value and payout — burned naturally via UTXO accounting. Bond count update takes effect at next epoch boundary.

### 6.4. Exiting

Full exit from producer set:

```bash
# Exit (withdraws all bonds instantly with vesting penalties)
./target/release/doli producer exit
```

### 6.6. Vesting Schedule and Early Withdrawal Penalties

**Each bond vests independently over 4 quarters. Quarter duration is network-specific:**

| Network | Quarter Duration | Full Vest |
|---------|-----------------|-----------|
| Mainnet | ~1 year (3,153,600 slots) | ~4 years |
| Testnet | ~6 hours (2,160 slots) | ~1 day |
| Devnet | ~10 min (60 slots) | ~40 min |

| Bond Age | Penalty on Withdrawal |
|----------|----------------------|
| Q1 (0-1 quarter) | 75% burned |
| Q2 (1-2 quarters) | 50% burned |
| Q3 (2-3 quarters) | 25% burned |
| Q4+ (3+ quarters) | 0% (fully vested) |

**Example:** If you have 3 bonds created at different times:
- Bond 1 (created 20 hours ago): 0% penalty, receive 10 DOLI
- Bond 2 (created 9 hours ago): 50% penalty, receive 5 DOLI
- Bond 3 (created 3 hours ago): 75% penalty, receive 2.5 DOLI

FIFO withdrawal means oldest bonds (typically with lower penalties) are withdrawn first.
Use `doli producer status` to see current vesting quarter and penalty % for your bonds.

---

## 7. Producer Weight (Seniority)

Producer weight is based on discrete yearly tiers and increases with time:

| Years Active | Weight |
|--------------|--------|
| 0-1 years | 1x |
| 1-2 years | 2x |
| 2-3 years | 3x |
| 3+ years | 4x (cap) |

**Weight affects fork choice only:**
- Chains with higher accumulated weight win
- Seniority rewards long-term commitment
- Does NOT affect block production frequency (that's bond count)

**Example fork choice:**
```
Chain A: blocks from producers with weights 1, 1, 2 = total weight 4
Chain B: blocks from producers with weights 2, 3 = total weight 5
→ Chain B wins (higher accumulated weight)
```

**Note:** Weight is also used for governance veto threshold calculations.

---

## 8. Slashing

### 8.1. Slashable Offense

Only one offense results in slashing:

| Offense | Penalty |
|---------|---------|
| Double production (same slot, different blocks) | 100% bond burned |

### 8.2. Non-Slashable Events

| Event | Consequence |
|-------|-------------|
| Missed slot | Lost reward opportunity |
| 50 consecutive missed slots | Removed from active set |
| Invalid block | Block rejected |

### 8.3. Prevention

**Never run two producers with the same key simultaneously.**

```bash
# Before starting a second node, ensure the first is stopped
sudo systemctl stop doli-node
# Wait for confirmation
sudo systemctl status doli-node
# Only then start on new machine
```

---

## 9. Monitoring

### 9.1. Check Producer Status

```bash
# Via CLI
./target/release/doli producer status

# Via RPC
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducer","params":["YOUR_PUBKEY"],"id":1}'
```

### 9.2. Key Metrics

| Metric | Target |
|--------|--------|
| Blocks produced | Match expected rate |
| Missed slots | < 1% |
| VDF computation time | < 8 seconds |
| Node uptime | > 99% |

### 9.3. Alerts

Set up alerts for:
- Node offline
- Missed blocks
- VDF computation slow
- Low peer count
- Chain not advancing

---

## 10. Operational Best Practices

### 10.1. Hardware

- Dedicated machine (no shared hosting)
- UPS for power stability
- RAID or redundant storage
- Multiple network paths if possible

### 10.2. Security

- Firewall: only P2P port open
- SSH: key-only authentication
- Regular security updates
- Separate producer key from operational funds

### 10.3. Redundancy

**Warning:** Never run two instances with the same key (causes slashing)

Safe redundancy patterns:
- Hot standby with automatic failover (mutex-protected)
- Cold standby with manual switchover
- Different bonds on different machines

### 10.4. Maintenance Windows

- Schedule updates during low-activity periods
- Monitor for a full epoch before/after changes
- Keep backup of working configuration

---

## 11. Troubleshooting

### 11.1. Not Producing Blocks

| Symptom | Possible Cause | Solution |
|---------|---------------|----------|
| No blocks | Not in active set | Wait for epoch boundary |
| No blocks | Node not synced | Check sync status |
| No blocks | VDF too slow | Check CPU performance |
| Missed slots | Network issues | Check connectivity |

### 11.2. VDF Performance Issues

```bash
# Check VDF computation time in logs
grep "VDF computed" /var/log/doli-node.log

# Should be < 8 seconds
# If > 8 seconds: CPU too slow or throttled
```

### 11.3. Recovery from Failure

1. Stop any running instances
2. Verify only one instance will run
3. Check chain sync status
4. Resume production

---

## 12. Governance Participation

### 12.1. Software Updates

Updates have a veto period (currently 5 minutes, early network; target 7 days):

```bash
# Check pending update
./target/release/doli-node update status

# Vote to veto (requires producer key)
./target/release/doli-node update vote --veto --key /path/to/key
```

### 12.2. Veto Threshold

- 40% of weighted votes required to reject
- Weight based on seniority (1-4)
- Dormant producers have reduced weight

---

## 13. Economics Summary

| Metric | Value |
|--------|-------|
| Bond unit | 10 DOLI (mainnet), 1 DOLI (testnet/devnet) |
| Maximum bonds | 3,000 per producer (30,000 DOLI mainnet) |
| Block reward (Era 1) | 1 DOLI |
| Blocks per year (1 bond, 1000 total) | ~3,154 |
| Annual rewards (1 bond, 1000 total) | ~3,154 DOLI |
| Full vesting period | 4 years mainnet (3,153,600 slots/quarter), 1 day testnet (2,160 slots/quarter) |
| Unbonding period | 60,480 blocks (~7 days mainnet), 72 blocks (testnet) |
| Coinbase maturity | 6 blocks |
| Early exit penalties | 75%/50%/25%/0% (Q1/Q2/Q3/Q4+; 1yr each mainnet, 6h each testnet) |

---

*Last updated: March 2026*
