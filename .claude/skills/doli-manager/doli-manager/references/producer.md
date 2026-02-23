# Producer Operations

## Table of Contents
- Register as Producer
- Check Status
- List Producers
- Add Bonds (Bond Stacking)
- Request Withdrawal
- Claim Withdrawal
- Exit Producer Set
- Submit Slashing Evidence
- Bond Economics

## Register as Producer

```bash
# Register with 1 bond (10 DOLI on mainnet, 1 DOLI on devnet)
doli producer register -b 1

# Register with 5 bonds (50 DOLI)
doli producer register -b 5

# Devnet
doli -r http://127.0.0.1:28545 producer register -b 3
```

Requirements:
- Sufficient balance (bond_unit * count + fee)
- Registration includes VDF proof validation
- Bond is locked for one era (~4 years mainnet, ~10 min devnet)

## Check Status

```bash
# Own status (uses wallet pubkey)
doli producer status

# Specific producer
doli producer status --pubkey <hex_pubkey>
```

Shows: address, status (active/unbonding/exited/slashed), bond count, bond amount, era, pending withdrawals.

## List Producers

```bash
# All producers
doli producer list

# Active only
doli producer list --active
```

Shows: status, address (doli1...), bonds, era.

## Add Bonds (Bond Stacking)

```bash
# Add 3 more bonds
doli producer add-bond --count 3
```

Each bond = 1 bond_unit (10 DOLI mainnet, 1 DOLI devnet). Max 10000 bonds per producer.

More bonds = higher selection weight in round-robin scheduling.

## Request Withdrawal

```bash
# Withdraw 2 bonds to own wallet
doli producer request-withdrawal --count 2

# Withdraw to specific address (doli1... or hex)
doli producer request-withdrawal --count 1 --destination doli1recipient...
```

Starts a 7-day delay (mainnet) / 10 min (devnet). Bonds remain locked during delay.

## Claim Withdrawal

```bash
# Claim first pending withdrawal
doli producer claim-withdrawal

# Claim specific withdrawal by index
doli producer claim-withdrawal --index 1
```

Only works after delay period. Check `producer status` for pending withdrawals.

## Exit Producer Set

```bash
# Check penalty first
doli producer exit

# Force early exit (penalty applies)
doli producer exit --force
```

### Early Withdrawal Penalty (Bond Vesting)

| Time Active | Penalty | You Receive |
|-------------|---------|-------------|
| < 1 year | 75% burned | 25% |
| 1-2 years | 50% burned | 50% |
| 2-3 years | 25% burned | 75% |
| 3+ years | 0% | 100% |

## Submit Slashing Evidence

```bash
doli producer slash --block1 <hash1> --block2 <hash2>
```

Requirements:
- Both blocks must be for the SAME slot
- Both blocks from the SAME producer
- Blocks must be DIFFERENT (different hashes)

Penalty: 100% bond burn, immediate exclusion.

## Bond Economics

| Parameter | Mainnet | Devnet |
|-----------|---------|--------|
| Bond Unit | 10 DOLI | 1 DOLI |
| Max Bonds | 10,000 | 10,000 |
| Unbond Delay | 7 days | 10 min |
| Selection | `slot % total_bonds` | same |
| Fallback Window | 2s per rank, 5 ranks | same |

### Selection Weight

Producer selection is deterministic bond-weighted round-robin:
- Each bond = 1 slot in the rotation
- More bonds = more frequent block production
- Selection formula: `slot_index % total_bonds → assigned_producer`
