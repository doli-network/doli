# Producer Operations

## Table of Contents
- Register as Producer
- Check Status
- List Producers
- Add Bonds (Bond Stacking)
- Request Withdrawal (Instant, FIFO)
- Exit Producer Set
- Submit Slashing Evidence
- Bond Economics
- Per-Bond Tracking & Vesting

## Register as Producer

```bash
# Register with 1 bond (10 DOLI on mainnet, 1 DOLI on devnet)
doli producer register -b 1

# Register with 5 bonds (50 DOLI)
doli producer register -b 5

# Devnet
doli -r http://127.0.0.1:28500 producer register -b 3
```

Requirements:
- Sufficient balance (bond_unit * count + fee)
- Registration includes VDF proof validation
- Each bond gets a creation timestamp (slot) for per-bond vesting tracking
- Bond unit is fixed at 10 DOLI (mainnet) across all eras — never changes

## Check Status

```bash
# Own status (uses wallet pubkey)
doli producer status

# Specific producer
doli producer status --pubkey <hex_pubkey>
```

Shows: address, status (active/unbonding/exited/slashed), bond count, bond amount, era, per-bond vesting tiers.

**Per-bond vesting display** (v1.0.23+):
```
Producer: doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef
Status: Active
Bonds: 19 (190.00 DOLI):
  Vested (0% penalty):   5 bonds
  Q3 (25% penalty):      4 bonds
  Q2 (50% penalty):      6 bonds
  Q1 (75% penalty):      4 bonds
```

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

Each bond = 1 bond_unit (10 DOLI mainnet, 1 DOLI devnet). Max 10,000 bonds per producer.

More bonds = higher selection weight in round-robin scheduling.

Each bond added gets its own creation timestamp. Bonds added at different times vest independently (FIFO tracking).

Applied at the next epoch boundary (deferred to prevent scheduler divergence).

## Request Withdrawal (Instant, FIFO)

```bash
# Withdraw 2 bonds to own wallet
doli producer request-withdrawal --count 2

# Withdraw to specific address (doli1... or hex)
doli producer request-withdrawal --count 1 --destination doli1recipient...
```

**Instant payout** (v1.0.23+): Funds are available immediately in the same block. No 7-day delay. No separate claim step. Bonds are removed from the producer set at the next epoch boundary.

**FIFO order**: Oldest bonds are withdrawn first. Each bond's penalty is calculated individually based on its creation time.

**Interactive FIFO breakdown** before confirmation:
```
Your bonds (19 total):
  5 bonds — vested (0% penalty) — created 20h+ ago
  4 bonds — Q3 (25% penalty) — created 14h ago
  6 bonds — Q2 (50% penalty) — created 9h ago
  4 bonds — Q1 (75% penalty) — created 2h ago

Withdrawing 7 bonds (FIFO — oldest first):
  5 x vested (0% penalty):  50.00 DOLI -> 50.00 DOLI (0 burned)
  2 x Q3 (25% penalty):     20.00 DOLI -> 15.00 DOLI (5.00 burned)
  ─────────────────────────────────────────────────
  Total:                     70.00 DOLI -> 65.00 DOLI (5.00 burned)

You receive: 65.00 DOLI
Penalty burned: 5.00 DOLI
Bonds remaining: 12

Proceed? [y/N]
```

**Double-withdrawal prevention**: Cannot submit a second withdrawal in the same epoch for the same producer. `withdrawal_pending_count` blocks duplicates until the epoch boundary applies the first withdrawal.

## Exit Producer Set

```bash
# Check penalty first
doli producer exit

# Force early exit (penalty applies)
doli producer exit --force
```

Exit removes all bonds. The vesting penalty applies per-bond (FIFO) based on each bond's individual age.

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
| Vesting Period | 1 day (8,640 slots) | Configurable (default 10 min) |
| Vesting Quarter | 6 hours (2,160 slots) | Configurable (default 60 slots) |
| Selection | `slot % total_bonds` | same |
| Fallback Window | 2s per rank, 5 ranks | same |
| Bond Unit Across Eras | Fixed 10 DOLI (never changes) | Fixed 1 DOLI |

### Selection Weight

Producer selection is deterministic bond-weighted round-robin:
- Each bond = 1 slot in the rotation
- More bonds = more frequent block production
- Selection formula: `slot_index % total_bonds -> assigned_producer`

## Per-Bond Tracking & Vesting

Each bond has an individual `StoredBondEntry` with `creation_slot` and `amount`. This enables:

1. **Per-bond vesting**: Bonds added at different times vest independently
2. **FIFO withdrawal**: Oldest bonds are withdrawn first, each with its own penalty
3. **Granular penalties**: A producer with a mix of old (vested) and new bonds can withdraw the vested ones penalty-free

### Vesting Schedule (1-day, quarter-based)

| Quarter | Bond Age | Penalty | You Receive |
|---------|----------|---------|-------------|
| Q1 | 0-6h (0-2,160 slots) | 75% burned | 25% |
| Q2 | 6-12h (2,160-4,320 slots) | 50% burned | 50% |
| Q3 | 12-18h (4,320-6,480 slots) | 25% burned | 75% |
| Q4+ | 18h+ (6,480+ slots) | 0% | 100% |

Penalty is burned (removed from supply). Constants: `VESTING_QUARTER_SLOTS=2,160`, `VESTING_PERIOD_SLOTS=8,640`.

### RPC: getBondDetails

Returns real per-bond data:
```json
{
  "bondCount": 19,
  "totalStaked": 19000000000,
  "summary": { "q1": 4, "q2": 6, "q3": 4, "vested": 5 },
  "bonds": [
    { "creationSlot": 1000, "amount": 1000000000, "ageSlots": 9000, "penaltyPct": 0, "vested": true },
    ...
  ],
  "withdrawalPendingCount": 0
}
```

### Storage: ProducerInfo fields

- `bond_entries: Vec<StoredBondEntry>` — per-bond creation timestamps + amounts
- `withdrawal_pending_count: u32` — bonds queued for withdrawal this epoch (prevents double-withdrawal)
- Migration: existing producers without `bond_entries` auto-populate from `bond_count` + `registered_at` on first load
