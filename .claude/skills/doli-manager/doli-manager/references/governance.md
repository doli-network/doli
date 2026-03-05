# Governance

## Table of Contents
- Update System
- Veto Voting
- Maintainer Management
- Release Signing

## Update System

### Check for Updates

```bash
doli update check

# Or via node CLI
doli-node update check
```

### View Pending Update Status

```bash
doli update status
```

Shows: version, veto period status, veto count, veto percentage.

### Vote on Update

```bash
# Approve
doli update vote --version 1.0.1 --approve

# Veto
doli update vote --version 1.0.1 --veto
```

### View Votes

```bash
doli update votes --version 1.0.1
```

### Auto-Update Flow

1. Maintainers sign release (3/5 multisig)
2. Veto period starts (5 min early network*, target 7 days)
3. Producers can veto (40% stake threshold blocks)
4. If not vetoed: grace period (2 min early network*), then auto-apply
5. Watchdog monitors: 3 crashes in window triggers rollback

### Vote Weight Formula

```
weight = bond_count * seniority_multiplier
seniority = 1.0 + min(years_active, 4) * 0.75
```

| Year | Multiplier |
|------|-----------|
| 0 | 1.0x |
| 1 | 1.75x |
| 2 | 2.5x |
| 3 | 3.25x |
| 4+ | 4.0x |

## Maintainer Management

### List Maintainers

```bash
doli maintainer list

# Via node CLI
doli-node maintainer list
```

First 5 registered producers automatically become maintainers.

### Add/Remove Maintainer (Node CLI)

```bash
# Propose adding
doli-node maintainer add --target <pubkey_hex> --key <maintainer_key_path>

# Propose removing
doli-node maintainer remove --target <pubkey_hex> --key <maintainer_key_path> --reason "inactive"

# Sign proposal
doli-node maintainer sign --proposal_id <ID> --key <maintainer_key_path>

# Verify if pubkey is maintainer
doli-node maintainer verify --pubkey <pubkey_hex>
```

Requires 3/5 signatures (MAINTAINER_THRESHOLD).

## Release Signing (Maintainers Only)

```bash
doli-node release sign --key ~/.doli/mainnet/keys/producer.json --version 0.3.1
```

Hash is auto-computed from the binary. 3 of 5 maintainers must sign for release to be valid.

## Key Constants

| Parameter | Value |
|-----------|-------|
| Maintainer count | First 5 producers |
| Signature threshold | 3 of 5 |
| Veto threshold | 40% weighted stake |
| Veto period | 5 min (early network*) |
| Grace period | 2 min (early network*) |
| Check interval | 10 min |
| Crash window for rollback | 3 crashes |
