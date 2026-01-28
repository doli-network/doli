# BECOMING_A_PRODUCER.md - Block Producer Guide

This guide covers how to become a block producer in the DOLI network.

---

## 1. Overview

Block producers in DOLI:
- Create new blocks at their assigned slots
- Compute VDF proofs (~7 seconds per block)
- Earn block rewards (1 DOLI per block in Era 1)
- Require a bond (1,000 DOLI minimum)

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

| Requirement | Amount | Lock Period |
|-------------|--------|-------------|
| Minimum bond | 1,000 DOLI | 4 years |
| Maximum bond | 100,000 DOLI | 4 years |
| Bond unit | 1,000 DOLI | - |

### 2.3. Time Commitment

| Phase | Duration |
|-------|----------|
| Registration VDF | ~10 minutes (base) |
| Bond lock | 4 years |
| Unbonding period | 30 days |
| Withdrawal delay | 7 days |

---

## 3. Registration Process

### 3.1. Generate Producer Key

```bash
# Create a new producer keypair
./target/release/doli wallet new --name producer

# Display the public key
./target/release/doli wallet address

# Backup the key securely!
cp ~/.doli/wallet.json ~/secure-backup/producer-key.json
```

### 3.2. Fund the Producer Address

Transfer the bond amount plus fees to your producer address:

```bash
# Check balance
./target/release/doli wallet balance

# You need: bond amount + registration fee + operational buffer
# Example: 1,000 DOLI bond + 0.01 fee + 1 DOLI buffer = 1,001.01 DOLI
```

### 3.3. Register as Producer

```bash
# Register with minimum bond (1,000 DOLI)
./target/release/doli producer register

# Register with multiple bonds for more slots
./target/release/doli producer register --bonds 5  # 5,000 DOLI
```

**What happens during registration:**
1. Node computes registration VDF (~10 minutes base)
2. Registration transaction submitted to network
3. Bond locked for 4 years
4. Producer added to active set at next epoch

### 3.4. Start Producing

```bash
# Run node with producer mode enabled
./target/release/doli-node run \
    --producer \
    --producer-key ~/.doli/wallet.json
```

---

## 4. Understanding Selection

### 4.1. Deterministic Round-Robin

Selection is NOT a lottery. It's deterministic based on:
- Slot number
- Active producer set (frozen at epoch start)
- Bond counts

```
Example with 3 producers (10 total bonds):

Alice: 1 bond  → Produces 1 block per 10 slots
Bob:   5 bonds → Produces 5 blocks per 10 slots
Carol: 4 bonds → Produces 4 blocks per 10 slots

Selection pattern (repeating):
Slot 0: Alice
Slot 1: Bob
Slot 2: Bob
Slot 3: Bob
Slot 4: Bob
Slot 5: Bob
Slot 6: Carol
Slot 7: Carol
Slot 8: Carol
Slot 9: Carol
Slot 10: Alice (cycle repeats)
```

### 4.2. Calculating Your Slots

```
Your blocks per epoch = (your_bonds / total_bonds) × 360
Your blocks per day = (your_bonds / total_bonds) × 8,640
```

### 4.3. Fallback Mechanism

If the primary producer is offline:

| Time in slot | Eligible producers |
|--------------|-------------------|
| 0-3 seconds | Rank 0 only |
| 3-6 seconds | Rank 0 or 1 |
| 6-10 seconds | Rank 0, 1, or 2 |

Lower rank always wins if multiple valid blocks exist.

---

## 5. Rewards

### 5.1. Block Rewards

| Era | Years | Reward per Block |
|-----|-------|------------------|
| 1 | 0-4 | 1.0 DOLI |
| 2 | 4-8 | 0.5 DOLI |
| 3 | 8-12 | 0.25 DOLI |
| 4 | 12-16 | 0.125 DOLI |

### 5.2. ROI Calculation

**All producers earn identical ROI percentage:**

```
ROI per block = reward / bond_per_ticket
ROI per block = 1 DOLI / 1,000 DOLI = 0.1%

Blocks per year (with 1 bond, 1000 total bonds):
= 3,153,600 slots × (1/1000) = 3,153.6 blocks

Annual return = 3,153.6 × 1 DOLI = 3,153.6 DOLI
Annual ROI = 3,153.6 / 1,000 = 315.36%
```

**Note:** ROI decreases as more producers join and rewards halve.

### 5.3. Fee Income

Producers also earn transaction fees from included transactions.

---

## 6. Bond Management

### 6.1. Adding Bonds

Increase your stake (up to 100 bonds / 100,000 DOLI):

```bash
# Add 2 more bonds (2,000 DOLI)
./target/release/doli producer add-bond --amount 2000
```

### 6.2. Requesting Withdrawal

Start the withdrawal process (7-day delay):

```bash
# Request withdrawal of 1 bond (1,000 DOLI)
./target/release/doli producer request-withdrawal --bonds 1
```

### 6.3. Claiming Withdrawal

After 7-day delay:

```bash
./target/release/doli producer claim-withdrawal
```

### 6.4. Exiting

Full exit from producer set:

```bash
# Exit (starts 30-day unbonding)
./target/release/doli producer exit

# After 30 days, claim bond
./target/release/doli producer claim-bond
```

**Warning:** Early exit (before 4 years) incurs penalty:
```
penalty_pct = (time_remaining × 100) / 4_years
return = bond × (100 - penalty_pct) / 100
```

---

## 7. Producer Weight (Seniority)

Your influence in fork choice and governance increases with time:

| Years Active | Weight |
|--------------|--------|
| 0-1 | 1 |
| 1-2 | 2 |
| 2-3 | 3 |
| 3+ | 4 |

**Weight affects:**
- Fork choice (chains with higher weight win)
- Governance voting power
- Veto threshold calculations

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

Updates have a 7-day veto period:

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
| Minimum bond | 1,000 DOLI |
| Maximum bonds | 100 per producer |
| Block reward (Era 1) | 1 DOLI |
| Blocks per year (1 bond, 1000 total) | ~3,154 |
| Annual rewards (1 bond, 1000 total) | ~3,154 DOLI |
| Lock period | 4 years |
| Early exit penalty | Pro-rata based on time remaining |

---

*Last updated: January 2026*
