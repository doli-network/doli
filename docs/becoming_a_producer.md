# becoming_a_producer.md - Block Producer Guide

This guide covers how to become a block producer in the DOLI network.

---

## 1. Overview

Block producers in DOLI:
- Create new blocks at their assigned slots
- Compute VDF proofs (~55ms per block)
- Earn block rewards (1 DOLI per block in Era 1)
- Require a bond (10 DOLI per bond unit)

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
| Bond unit | 10 DOLI | 1 day (4 × 6h quarters) |
| Maximum bonds | 10,000 bonds (100,000 DOLI) | 1 day |

**Bond Stacking**: You can hold multiple bonds (up to 10,000), each with its own creation time and vesting schedule. Withdrawals follow FIFO (First-In-First-Out) order, withdrawing oldest bonds first.

### 2.3. Time Commitment

| Phase | Duration |
|-------|----------|
| Registration VDF | ~10 minutes (base) |
| Bond vesting | 1 day (0% penalty after 18h) |
| Unbonding period | 7 days |
| Withdrawal delay | 7 days |

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
    --rpc http://127.0.0.1:8545 \
    send $NEW_ADDR 110

# Check balance on new wallet
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8545 \
    balance

# You need: bond amount + registration fee + operational buffer
# Example: 10 DOLI bond + 0.01 fee + 1 DOLI buffer = 11.01 DOLI
```

### 3.4. Register as Producer

```bash
# Register with minimum bond (10 DOLI = 1 bond)
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register

# Register with multiple bonds for more slots
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8545 \
    producer register --bonds 5  # 50 DOLI = 5 bonds
```

**What happens during registration:**
1. Node computes registration VDF (~10 minutes base)
2. Registration transaction submitted to network
3. Bond begins 1-day vesting (fully vested after 18h)
4. Producer added to active set after ACTIVATION_DELAY (10 blocks)

### 3.5. Start Producing (CRITICAL)

**This step is required.** Without a running node, your registered producer cannot produce blocks.

```bash
# Run node with producer mode enabled
doli-node run \
    --producer \
    --producer-key ~/.doli/keys/my_producer.json \
    --p2p-port 30303 \
    --rpc-port 8545
```

For networks with existing nodes, add bootstrap:

```bash
# Connect to existing network
doli-node run \
    --network mainnet \
    --producer \
    --producer-key ~/.doli/keys/my_producer.json \
    --bootstrap /dns4/seed1.doli.network/tcp/30303
```

### 3.6. Verify Production

After starting the node, verify it's producing:

```bash
# Check producer status
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8545 \
    producer status

# Watch for production in logs
grep "Producing block" /var/log/doli-node.log

# Check balance is increasing (rewards)
doli --wallet ~/.doli/keys/my_producer.json \
    --rpc http://127.0.0.1:8545 \
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
Your blocks per epoch = (your_bonds / total_bonds) × 360
Your blocks per day = (your_bonds / total_bonds) × 8,640
```

### 4.3. Fallback Mechanism

If the primary producer is offline, fallback producers become eligible in exclusive 2-second windows:

| Time in slot | Eligible rank |
|--------------|---------------|
| 0-2 seconds | Rank 0 only (primary) |
| 2-4 seconds | Rank 1 (first fallback) |
| 4-6 seconds | Rank 2 (second fallback) |
| 6-8 seconds | Rank 3 (third fallback) |
| 8-10 seconds | Rank 4 (fourth fallback) |

5 ranks x 2,000ms = 10,000ms — fills the entire slot.

**Fallback calculation:** Fallback producers at ranks 1-4 are selected by offsetting the primary ticket:
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

Rewards are distributed via **coinbase transactions** directly to the block producer:

| Aspect | Behavior |
|--------|----------|
| Timing | Immediate, included in each block |
| Delivery | Coinbase transaction to producer's address |
| Maturity | 100 blocks (~17 minutes) before spendable |
| Empty slots | No reward generated for missed slots |

**Key characteristics:**

- **Simple**: Each block contains a coinbase output to its producer
- **Deterministic**: Reward amount based on era (halves every ~4 years)
- **No claiming required**: Rewards appear directly in your UTXO set
- **Maturity period**: Coinbase outputs require 100 confirmations before spending

**Example:**

```
Block produced by Alice at height 1000:
- Coinbase reward: 1 DOLI to Alice
- Spendable after: height 1100 (100 block maturity)
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

You can hold multiple bonds (up to 10,000), each with its own vesting schedule:
- Each bond = 10 DOLI = 1 ticket per cycle
- Maximum: 10,000 bonds = 100,000 DOLI
- Bonds vest individually based on their creation time

### 6.2. Adding Bonds

Increase your stake (up to 10,000 bonds / 100,000 DOLI):

```bash
# Add 2 more bonds (20 DOLI)
./target/release/doli producer add-bond --count 2
```

### 6.3. Requesting Withdrawal

Start the withdrawal process (7-day delay). **Withdrawals follow FIFO order** - oldest bonds are withdrawn first:

```bash
# Request withdrawal of 1 bond (10 DOLI, oldest bond)
./target/release/doli producer request-withdrawal --count 1
```

### 6.4. Claiming Withdrawal

After 7-day delay (60,480 slots):

```bash
./target/release/doli producer claim-withdrawal
```

### 6.5. Exiting

Full exit from producer set:

```bash
# Exit (starts 7-day unbonding for all bonds)
./target/release/doli producer exit

# After 7 days, claim bond
./target/release/doli producer claim-bond
```

### 6.6. Vesting Schedule and Early Withdrawal Penalties

**Each bond vests independently over 1 day (4 quarters of 6 hours):**

| Bond Age | Penalty on Withdrawal |
|----------|----------------------|
| Q1 (0-6h) | 75% burned |
| Q2 (6-12h) | 50% burned |
| Q3 (12-18h) | 25% burned |
| Q4+ (18h+) | 0% (fully vested) |

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
curl -X POST http://127.0.0.1:8545 \
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
| Bond unit | 10 DOLI |
| Maximum bonds | 10,000 per producer (100,000 DOLI) |
| Block reward (Era 1) | 1 DOLI |
| Blocks per year (1 bond, 1000 total) | ~3,154 |
| Annual rewards (1 bond, 1000 total) | ~3,154 DOLI |
| Full vesting period | 1 day (8,640 slots) |
| Withdrawal delay | 7 days (60,480 slots) |
| Coinbase maturity | 100 blocks |
| Early exit penalties | 75%/50%/25%/0% (Q1/Q2/Q3/Q4+, 6h each) |

---

*Last updated: February 2026*
