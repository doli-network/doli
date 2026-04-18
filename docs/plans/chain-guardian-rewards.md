# Chain Guardian Rewards — 2x bond weight for complete chain

**Status**: Planned (not scheduled)  
**Type**: Consensus change (activation height required)  
**Priority**: Feature — implement after network stability is confirmed  

## Problem

Nodes that maintain the complete blockchain history (every block from h=1) provide a critical service: they can serve any block to any peer for sync, verification, or audit. Nodes that snap-synced only have recent blocks and cannot serve historical data.

There is no incentive to keep the full chain. Snap sync is cheaper (less storage, faster startup), so rational operators delete historical blocks. As the chain grows, fewer nodes will have complete history, making full sync harder and reducing network resilience.

## Solution: Random Block Challenge

Each epoch, the protocol deterministically selects 3 random block heights. Producers must include the hashes of those blocks in their coinbase transaction. Validators verify the hashes against their own blocks. Producers who demonstrate chain completeness receive 2x bond weight in the epoch reward calculation.

## Mechanics

### Challenge Selection (deterministic)

```
challenge_height[i] = BLAKE3(epoch_boundary_hash || i) mod epoch_boundary_height
```

For i = 0, 1, 2. All nodes compute the same 3 heights. Heights are derived from the PREVIOUS epoch boundary hash, so the challenged blocks already exist when the epoch starts.

### Inclusion in Block

- Producer looks up the 3 blocks in its local block store
- Includes the 3 hashes in the coinbase TX `extra_data`
- Format: `[challenge_hash_0 || challenge_hash_1 || challenge_hash_2]` — 96 bytes fixed
- If the producer doesn't have a block: omit challenge data entirely (no penalty, just no 2x)

### Validation

- Validator computes the same 3 challenge heights
- Looks up blocks in its OWN store
- Compares hashes from coinbase with its own
- **Soft validation**: if the validator doesn't have the challenged blocks (snap synced), it accepts without verifying — does NOT reject the block
- **Hard validation**: if the validator has the blocks and the hashes don't match, the block is still valid but the producer doesn't qualify for 2x

### Accumulation per Epoch

- In `accumulate_block`: if the block has 3 correct challenge hashes, mark the producer as "chain_guardian" for this epoch
- A producer needs correct challenges in >= 90% of their produced blocks to qualify (same threshold as attestation)
- Tracked in `epoch_state.chain_guardian_producers: HashSet<PublicKey>`

### Reward Calculation

- In `calculate_epoch_rewards`: producers with chain_guardian status → `effective_bonds = bond_count × 2`
- Producers without → `effective_bonds = bond_count × 1` (normal, no penalty)
- The 2x is a reward multiplier, not a bond change — no UTXO mutation

## Why It Cannot Be Cheated

1. **Cannot copy from peer in real-time**: Must include hashes in EVERY block produced. At 10s/slot, would need to request 3 blocks from a peer every production cycle. Fragile, detectable, and the peer could be offline.

2. **Cannot pre-compute**: Challenge heights are determined by `epoch_boundary_hash`, unknown until the boundary block is applied. Would need to download the 3 specific blocks at epoch start — but if capable of that, maintaining the full chain is simpler.

3. **Cannot lie**: Validators with complete chains verify independently. Wrong hash = no 2x qualification.

4. **Cannot selectively store**: Challenge heights are random across the entire chain history. Storing only "likely challenge" blocks is equivalent to storing everything (uniform distribution).

## Files That Change

| File | Change |
|------|--------|
| `consensus/constants.rs` | `CHAIN_GUARDIAN_ACTIVATION_HEIGHT`, `CHAIN_CHALLENGE_COUNT = 3`, `CHAIN_GUARDIAN_MULTIPLIER = 2` |
| `epoch_state.rs` | New field: `chain_guardian_producers: HashSet<PublicKey>` |
| `production/assembly.rs` | Compute challenge heights, look up block hashes, include in coinbase extra_data |
| `apply_block/post_commit.rs` | Verify challenge hashes (soft), accumulate chain_guardian status |
| `rewards.rs` | If chain_guardian → effective_bonds × 2 |
| `validation_checks.rs` | Validate challenge hashes (soft — never reject block) |

## Files That Do NOT Change

- HardForkSchedule — constant gate only, rolling deploy safe
- Block header — no new fields (challenge goes in coinbase extra_data)

## Activation

- Constant `CHAIN_GUARDIAN_ACTIVATION_HEIGHT` — future epoch boundary
- Before activation: all producers receive 1x rewards (current behavior)
- After activation: chain guardians receive 2x, others receive 1x
- Consensus-breaking: changes epoch_state (new field) → state root changes

## Edge Cases

| Case | Behavior |
|------|----------|
| New node (no history) | 1x rewards until full chain downloaded |
| Snap-synced validator | Accepts challenge proofs without verifying (soft) |
| Producer omits challenge | Block valid, producer gets 1x rewards |
| Challenge height = 0 (genesis) | Valid — genesis block always exists |
| Corrupted block store | Wrong hash → no 2x qualification |
| Node downloads challenged blocks on-demand | Must do it every block produced — impractical at scale |

## Cost

- 96 bytes extra per coinbase TX
- 3 block hash lookups per block produced (O(1) in block store index)
- 3 hash comparisons per block validated
- 1 HashSet field in epoch_state (negligible memory)

## Economic Impact

Chain guardians earn 2x rewards proportional to their bonds. This creates a natural incentive gradient:

- Small producer (1 bond) with full chain earns same as a 2-bond producer without
- Large producer (100 bonds) with full chain earns same as a 200-bond producer without
- The incentive scales linearly — no threshold effects, no cliff

Producers who delete historical blocks lose 50% of their potential rewards. The cost of storage (a few GB for years of 10s blocks) is trivial compared to the reward loss.

## Future Considerations

- **Challenge count**: 3 is sufficient for probabilistic security. Could increase to 5 or 10 for stronger guarantees at minimal cost.
- **Multiplier**: 2x is a starting point. Could be adjusted via governance or hard fork.
- **Archiver nodes**: Non-producing nodes that serve historical blocks could be rewarded separately (requires a different mechanism — they don't produce blocks).
- **Pruning policy**: Once chain guardian rewards are active, the protocol could safely increase the snap sync anchor distance, knowing that guardians preserve the full history.
