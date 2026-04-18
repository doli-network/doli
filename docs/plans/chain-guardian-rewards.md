# Chain Guardian Rewards — 2x bond weight for complete chain

**Status**: Scheduled  
**Version**: v6.18.0  
**Activation**: h=40,000 (constant gate, no HardForkSchedule)  
**Type**: Consensus change  
**Deploy**: Rolling per-node, all nodes must update before h=40,000  

## Problem

Nodes that maintain the complete blockchain history (every block from h=1) provide a critical service: they can serve any block to any peer for sync, verification, or audit. Nodes that snap-synced only have recent blocks and cannot serve historical data.

There is no incentive to keep the full chain. Snap sync is cheaper (less storage, faster startup), so rational operators delete historical blocks. As the chain grows, fewer nodes will have complete history, making full sync harder and reducing network resilience.

## Solution: Random Block Challenge (opt-in)

Each epoch, the protocol deterministically selects 3 random block heights. Producers running with `--full-chain` flag include the hashes of those blocks in their coinbase transaction. Validators verify the hashes against their own blocks. Producers who demonstrate chain completeness receive 2x bond weight in the epoch reward calculation.

The `--full-chain` flag is opt-in. Producers without the flag produce normal blocks and receive 1x rewards. No penalty for not participating.

## Mechanics

### Flag: `--full-chain`

- CLI flag on `doli-node run --full-chain`
- Default: off (no challenge proofs, no overhead)
- When on: producer computes and includes challenge proofs in every block
- Can be enabled/disabled at any time without restart (future: runtime toggle via RPC)

### Challenge Selection (deterministic)

```
challenge_height[i] = BLAKE3(prev_epoch_boundary_hash || i) mod prev_epoch_boundary_height
```

For i = 0, 1, 2. All nodes compute the same 3 heights. Heights are derived from the PREVIOUS epoch boundary hash, so the challenged blocks already exist when the epoch starts.

### Inclusion in Block

- Producer looks up the 3 blocks in its local block store
- Includes the 3 hashes in the coinbase TX `extra_data`
- Format: `[challenge_hash_0 || challenge_hash_1 || challenge_hash_2]` — 96 bytes fixed
- If `--full-chain` not set or producer doesn't have a block: omit challenge data (no penalty, 1x rewards)

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

2. **Cannot pre-compute**: Challenge heights are determined by `prev_epoch_boundary_hash`, unknown until the boundary block is applied. Would need to download the 3 specific blocks at epoch start — but if capable of that, maintaining the full chain is simpler.

3. **Cannot lie**: Validators with complete chains verify independently. Wrong hash = no 2x qualification.

4. **Cannot selectively store**: Challenge heights are random across the entire chain history. Storing only "likely challenge" blocks is equivalent to storing everything (uniform distribution).

## Implementation Plan

### Constants (`crates/core/src/consensus/constants.rs`)

```rust
pub const CHAIN_GUARDIAN_ACTIVATION_HEIGHT: u64 = 40_000;
pub const CHAIN_CHALLENGE_COUNT: usize = 3;
pub const CHAIN_GUARDIAN_MULTIPLIER: u64 = 2;
```

### CLI flag (`bins/node/src/cli.rs`)

```
--full-chain    Enable chain guardian mode: include block challenge proofs for 2x rewards
```

Stored in Node config, passed to block production.

### EpochState (`crates/core/src/epoch_state.rs`)

- New field: `chain_guardian_producers: HashSet<PublicKey>`
- Included in `hash()` for state root (deterministic)
- Included in `serialize()`/`deserialize()` for persistence
- Reset at epoch boundary in `derive_at_boundary()`

### Block Production (`bins/node/src/node/production/assembly.rs`)

If `--full-chain` enabled AND `height >= CHAIN_GUARDIAN_ACTIVATION_HEIGHT`:
1. Get previous epoch boundary hash and height
2. Compute 3 challenge heights: `BLAKE3(boundary_hash || i) mod boundary_height`
3. Look up 3 blocks in block store
4. Append 96 bytes (3 × 32-byte hashes) to coinbase `extra_data`

### Apply Block (`bins/node/src/node/apply_block/post_commit.rs`)

If `height >= CHAIN_GUARDIAN_ACTIVATION_HEIGHT`:
1. Compute 3 challenge heights (same formula)
2. Extract 96 bytes from coinbase `extra_data` (if present)
3. If validator has the challenged blocks: compare hashes
4. If all 3 match: `epoch_state.chain_guardian_producers.insert(producer)`
5. If validator doesn't have blocks (snap sync): accept proof without verifying (soft)
6. If no challenge data in coinbase: no insertion (producer gets 1x)

### Rewards (`bins/node/src/node/rewards.rs`)

If `epoch_start >= CHAIN_GUARDIAN_ACTIVATION_HEIGHT`:
- For each qualified producer: check if in `chain_guardian_producers`
- If yes: `effective_bonds = bond_count × CHAIN_GUARDIAN_MULTIPLIER`
- If no: `effective_bonds = bond_count`

### Files Summary

| File | Change |
|------|--------|
| `consensus/constants.rs` | 3 new constants |
| `cli.rs` | `--full-chain` flag |
| `node/mod.rs` | Store flag in Node config |
| `epoch_state.rs` | New field + hash + serialize + reset |
| `production/assembly.rs` | Challenge computation + coinbase inclusion |
| `apply_block/post_commit.rs` | Challenge verification + accumulation |
| `rewards.rs` | 2x multiplier for chain guardians |

### Files That Do NOT Change

- **HardForkSchedule** — no entry. Constant gate only. Rolling deploy safe. fork_id unchanged.
- **Block header** — no new fields. Challenge goes in coinbase extra_data.

## Activation & Deploy

1. **Build** v6.18.0 on ai2 with all changes
2. **Rolling deploy** per-node to ai1-ai5 (per-node binary layout)
3. **Deploy** to ai7-ai11 (atomic per server)
4. **External nodes**: `doli upgrade` before h=40,000
5. **Enable** `--full-chain` on nodes with complete chain (ai1-ai5 producers)
6. **h=40,000**: activation — state root changes, 2x rewards begin
7. Nodes without the update will diverge at h=40,000 (different state root from new epoch_state field)

## Edge Cases

| Case | Behavior |
|------|----------|
| New node (no history) | 1x rewards until full chain downloaded + `--full-chain` enabled |
| Snap-synced validator | Accepts challenge proofs without verifying (soft) |
| Producer without `--full-chain` | Block valid, 1x rewards, no challenge data |
| Producer with `--full-chain` but missing blocks | Cannot produce valid proofs → 1x rewards |
| Challenge height = 0 (genesis) | Valid — genesis block always exists |
| Corrupted block store | Wrong hash → no 2x qualification |
| Node enables `--full-chain` mid-epoch | Starts including proofs, qualifies if >= 90% of epoch blocks have proofs |
| Node downloads challenged blocks on-demand | Must do it every block produced — impractical at scale |

## Cost

- 96 bytes extra per coinbase TX (only for `--full-chain` nodes)
- 3 block hash lookups per block produced (O(1) in block store index)
- 3 hash comparisons per block validated
- 1 HashSet field in epoch_state (negligible memory)
- Zero overhead for nodes without `--full-chain`

## Economic Impact

Chain guardians earn 2x rewards proportional to their bonds. This creates a natural incentive gradient:

- Small producer (1 bond) with full chain earns same as a 2-bond producer without
- Large producer (100 bonds) with full chain earns same as a 200-bond producer without
- The incentive scales linearly — no threshold effects, no cliff

Producers who delete historical blocks lose 50% of their potential rewards. The cost of storage (a few GB for years of 10s blocks) is trivial compared to the reward loss.

## Future Considerations

- **Challenge count**: 3 is sufficient for probabilistic security. Could increase to 5 or 10 for stronger guarantees at minimal cost.
- **Multiplier**: 2x is a starting point. Could be adjusted via constant change + activation height.
- **Archiver nodes**: Non-producing nodes that serve historical blocks could be rewarded separately (requires a different mechanism — they don't produce blocks).
- **Pruning policy**: Once chain guardian rewards are active, the protocol could safely increase the snap sync anchor distance, knowing that guardians preserve the full history.
- **Explorer display**: Show chain guardian status per producer in attestation stats (badge or indicator).
