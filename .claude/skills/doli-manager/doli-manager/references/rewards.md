# Rewards

## Table of Contents
- List Claimable Rewards
- Claim Specific Epoch
- Claim All
- Claim History
- Epoch Info

## List Claimable Rewards

```bash
doli rewards list
```

Shows each unclaimed epoch with: blocks present, presence rate, estimated reward.

## Claim Specific Epoch

```bash
doli rewards claim 42

# To specific recipient
doli rewards claim 42 --recipient doli1destination...
```

Epoch must be complete (past). Each claim is one transaction.

## Claim All

```bash
doli rewards claim-all

# To specific recipient
doli rewards claim-all --recipient doli1destination...
```

Creates one transaction per claimable epoch.

## Claim History

```bash
doli rewards history
doli rewards history --limit 50
```

Shows: epoch, amount, block height, tx hash.

## Epoch Info

```bash
doli rewards info
```

Shows: current height, current epoch, blocks per epoch (360 mainnet, 60 devnet), blocks remaining, block reward rate, progress bar.

## Reward Economics

- Rewards = 100% to producer (no split)
- Block reward halves every era (~4 years)
- Epoch = 360 blocks (mainnet) / 60 blocks (devnet)
- Rewards require presence tracking (heartbeat mechanism)
- Coinbase maturity: 100 blocks (mainnet), 10 blocks (devnet)
