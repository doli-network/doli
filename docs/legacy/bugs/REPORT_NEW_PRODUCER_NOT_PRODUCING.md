# Bug Report: New Producers Not Producing Blocks

**Date**: 2026-02-04
**Status**: Root Cause Identified
**Severity**: User Error (Configuration Mismatch)

## Symptoms

- 5 new producers registered at height 258 with 0.9999 DOLI balance
- These producers show "active" status but receive no block rewards
- 6 genesis producers continue producing all blocks

## Investigation Summary

| Check | Result |
|-------|--------|
| Producer registration | Correct - 11 producers in `producer_set` across all nodes |
| Chain sync | Correct - All nodes at same height/hash |
| Scheduler | Correct - Uses `active_producers_at_height()` with ACTIVATION_DELAY=10 |
| Production attempts | Node1 logs show "Producing block" messages |

## Root Cause

**Key file mismatch**: The running nodes use **different keys** than the newly registered producers.

Evidence from node1 logs:
```
Loading producer key from "/Users/isudoajl/.doli/devnet/keys/producer_1.json"
Producer key loaded: 5a3eec1f2b7ddbe3...  (hash: bc98e093b554545f)
```

But the RPC shows producer index 1 is `54d3ba2f6867ae34...` (registered at height 258).

The key `5a3eec1f2b7ddbe3` is actually a **genesis producer** (index 10), not the newly registered producer.

### Producer Key Mapping

| Key File | Actual Key | RPC Index | Registration |
|----------|-----------|-----------|--------------|
| producer_1.json | 5a3eec1f... | 10 | Genesis (h=0) |
| Expected | 54d3ba2f... | 1 | New (h=258) |

## Why This Happened

1. User registered 5 new producers on the blockchain (height 258)
2. User started nodes with **existing** key files (genesis producer keys)
3. The newly registered producers have **no nodes running with their private keys**
4. Genesis producers (weight 3 due to seniority) continue producing
5. New producers (weight 1) are selected but have no node to produce

## Scheduler Behavior

The scheduler correctly selects producers based on:
- Sorted pubkey order
- Bond-weighted tickets (each bond = 1 ticket)
- Seniority weight: 0-1yr=1, 1-2yr=2, 2-3yr=3, 3+yr=4

At height 316 with devnet's `blocks_per_year=144`:
- Genesis producers: 316/144 = 2.2 years = weight 3
- New producers: 58/144 = 0.4 years = weight 1

New producers should get ~22% of slots (5 tickets / ~23 total), but **no node produces** when they're selected.

## Solution

Two options:

1. **Start nodes with the correct keys**: Locate the private keys used to register the 5 new producers and start nodes with those keys.

2. **Re-register with existing keys**: If the original keys are lost, register the keys from `producer_*.json` files as new producers.

## Not a Bug

This is a configuration/operational error, not a code bug. The scheduler, registration, and production logic are working correctly.
