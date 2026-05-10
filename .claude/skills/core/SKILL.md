# core — DOLI Core Consensus & Types
<!-- @INDEX
ENTRY-POINTS: lines 19-44
DATA-FLOWS: lines 46-80
STRUCTS: lines 82-175
FUNCTIONS: lines 177-265
CONSTANTS: lines 267-360
ACTIVATION-HEIGHTS: lines 362-400
DEPENDENCIES: lines 402-415
CONSTRAINTS: lines 417-445
PATTERNS: lines 447-480
-->

## ENTRY-POINTS

Primary public API (all re-exported from `crates/core/src/lib.rs`):

**Block validation** — call these to process an incoming block:
- `validate_block(block, ctx) -> Result<(), ValidationError>` — full validation (VDF + context)
- `validate_block_with_mode(block, ctx, mode) -> Result<(), ValidationError>` — explicit mode
- `validate_header(header, ctx) -> Result<(), ValidationError>` — header-only

**Transaction validation:**
- `validate_transaction(tx, ctx) -> Result<(), ValidationError>` — structural
- `validate_transaction_with_utxos(tx, ctx, utxo_provider) -> Result<(), ValidationError>` — full with UTXO lookups

**Producer scheduling:**
- `DeterministicScheduler::new(producers)` — build scheduler from sorted producers
- `DeterministicScheduler::select_producer(slot, rank) -> Option<&PublicKey>` — primary selection
- `EpochState::derive_at_boundary(prev, input) -> EpochState` — ONE canonical epoch transition
- `EpochState::accumulate_block(input)` — per-block tracking update

**Genesis:**
- `generate_genesis_block(config: &GenesisConfig) -> Block` — `genesis.rs:189`
- `genesis_hash(network: Network) -> Hash` — `genesis.rs:302`
- `verify_genesis_block(block, network) -> Result<(), GenesisError>` — `genesis.rs:326`

**Network params (always prefer over raw constants):**
- `NetworkParams::load(network: Network) -> &'static NetworkParams` — cached singleton


## DATA-FLOWS

### Block production → apply
```
Producer: BlockBuilder::new(prev_hash, prev_slot, producer)
  .with_params(ConsensusParams)
  .with_presence_root(bitfield_commitment)   // BLAKE3(attestation_bitfield)
  .with_missed_producers(vec![...])          // on-chain liveness
  .add_transaction(coinbase_tx)              // first tx = coinbase to reward pool
  .build_with_vdf(timestamp) -> Block
```

### Slot selection (scheduler)
```
EpochState::derive_at_boundary(prev_epoch_state, input) -> EpochState
  → epoch_state.producer_list (attestation-filtered, epoch-frozen)
  → epoch_state.active_list (round-robin subset, post TIER_SYSTEM)

DeterministicScheduler::new(producers_with_bonds)
  → .select_producer(slot, rank=0) -> primary producer
  → .select_producer(slot, rank=1) -> fallback (activates after FALLBACK_TIMEOUT_MS=2000ms)
```

### Epoch reward at boundary
```
EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT → inputs=[sorted pool UTXO outpoints]
Transaction::new_epoch_reward_coinbase(pool_inputs, [(amount, pubkey_hash)], height, epoch)
  → TxType::EpochReward (type=10), distributes to ALL qualified producers
```

### Attestation bitfield encode/decode (CRITICAL invariant)
```
Encoder order: [epoch_state.producer_list | extra sorted by pubkey]
decode_attestation_bitfield(bitfield, producer_list, extra_sorted) -> attested indices
  → BOTH encode and decode MUST use same ordering or indices misalign → wrong rewards + fork
```

### Coinbase flow
```
Per-block: Transaction::new_coinbase(amount, reward_pool_pubkey_hash, height, slot)
  → goes to reward_pool_pubkey_hash (deterministic burn address, no private key)
  → extra_data = height(8 LE) || slot(4 LE)  [unique coinbase, post-UNIQUE_COINBASE_ACTIVATION]
At epoch boundary: EpochReward tx drains pool → distributes to producers bond-weighted
```

### ValidationContext construction (node layer fills this)
```
ValidationContext::new(params, network, current_time, current_height)
  .with_prev_block(slot, timestamp, hash)
  .with_epoch_producer_list(frozen_list)     // epoch-frozen, never changes mid-epoch
  .with_producers_weighted([(pk, weight)])   // for anti-grinding selection
  .with_fork_id(expected_hash, activation_height)
  .with_security_audit_activation_height(h)
```


## STRUCTS

### Block types (`block.rs`)
```rust
BlockHeader {
    version: u32,           // v2 = genesis_hash added
    prev_hash: Hash,
    merkle_root: Hash,
    presence_root: Hash,    // BLAKE3(attestation_bitfield) post-BITFIELD_BODY; Hash::ZERO legacy
    genesis_hash: Hash,     // chain identity; missing = Hash::ZERO (rejected by validation)
    timestamp: u64,
    slot: Slot,             // u32
    producer: PublicKey,
    vdf_output: VdfOutput,
    vdf_proof: VdfProof,
    missed_producers: Vec<PublicKey>,  // on-chain liveness exclusion
    data_root: Hash,        // BLAKE3(sorted blob_hashes) for large outputs
    fork_id: Hash,          // BLAKE3(genesis_hash || sorted active fork heights)
}

Block {
    header: BlockHeader,
    transactions: Vec<Transaction>,
    aggregate_bls_signature: Vec<u8>,   // 96 bytes, empty for pre-BLS
    attestation_bitfield: Vec<u8>,      // post-BITFIELD_BODY_ACTIVATION_HEIGHT
}

BlockBuilder { ... }  // builder pattern, block.rs:282
```

### Transaction types (`transaction/`)
```rust
Transaction {
    version: u32,
    tx_type: TxType,
    inputs: Vec<Input>,
    outputs: Vec<Output>,
    extra_data: Vec<u8>,
}

Input {
    prev_tx_hash: Hash,
    output_index: u32,
    signature: Signature,
    sighash_type: SighashType,           // All=0 (default), AnyoneCanPay=1
    committed_output_count: u32,         // AnyoneCanPay: 0=all outputs
    public_key: Option<crypto::PublicKey>, // mandatory post-sig_verification_height
}

Output {
    output_type: OutputType,
    amount: Amount,           // u64; native DOLI or token units (see is_native_amount())
    pubkey_hash: Hash,
    lock_until: BlockHeight,  // 0=normal, u64::MAX=bond (locked until withdrawal)
    extra_data: Vec<u8>,      // max BASE_EXTRA_DATA_SIZE=512KB, doubles per era
}
```

### Epoch & scheduler state (`epoch_state/mod.rs`)
```rust
EpochState {
    epoch: u64,
    bond_snapshot: HashMap<Hash, u64>,           // pubkey_hash → bond_count at boundary
    producer_list: Vec<PublicKey>,               // attestation-filtered, epoch-frozen, sorted
    active_list: Vec<PublicKey>,                 // round-robin subset (post TIER_SYSTEM)
    attested_sets: [HashSet<PublicKey>; 3],      // 3-epoch lookback
    attestation_accum: [HashMap<PublicKey, HashSet<u32>>; 3], // per-minute tracking
    blocks_produced: HashMap<PublicKey, u32>,
}

EpochDerivationInput {
    active_producers: Vec<PublicKey>,
    bond_counts: HashMap<Hash, u64>,
    blocks_per_epoch: u64,
    snap_attestation_skip_height: u64,
    height: u64, epoch: u64,
    registered_at: HashMap<PublicKey, u64>,
    ghost_exclusion_activation_height: u64,
}

BlockAccumulationInput {
    producer: PublicKey,
    slot: u32,
    has_attestation_data: bool,
    attested_indices: Vec<usize>,
}
```

### Consensus params (`consensus/params.rs`)
```rust
ConsensusParams {
    genesis_time: u64,
    slot_duration: u64,
    slots_per_epoch: u32,
    slots_per_reward_epoch: u32,
    blocks_per_era: BlockHeight,
    bootstrap_blocks: BlockHeight,
    initial_reward: Amount,
    initial_bond: Amount,
    base_block_size: usize,
    max_block_size_cap: usize,
    reward_mode: RewardMode,  // EpochPool (current)
    genesis_hash: crypto::Hash,
}
```

### Validation types (`validation/types.rs`)
```rust
ValidationContext {
    params: ConsensusParams,
    network: Network,
    current_time: u64,
    current_height: BlockHeight,
    prev_slot: u32, prev_timestamp: u64, prev_hash: Hash,
    active_producers: Vec<PublicKey>,           // legacy
    active_producers_weighted: Vec<(PublicKey, u64)>,
    epoch_producer_list: Vec<PublicKey>,        // epoch-frozen, scheduling denominator
    bootstrap_producers: Vec<PublicKey>,
    sig_verification_height: u64,              // default u64::MAX
    inc_i_026_scheduler_activation_height: u64,
    expected_fork_id: Hash,
    fork_id_activation_height: u64,
    encrypted_content_activation_height: u64,
    security_audit_activation_height: u64,
    ...
}

ValidationMode { Full, Light, Replay }
// Full = VDF verified; Light = VDF skipped (gap blocks post snap-sync); Replay = disaster recovery

UtxoInfo { output: Output, pubkey: Option<PublicKey>, spent: bool }
trait UtxoProvider { fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> }
```

### Scheduler (`scheduler.rs`)
```rust
DeterministicScheduler { producers: Vec<ScheduledProducer>, total_bonds: u64, ticket_boundaries: Vec<u64> }
ScheduledProducer { pubkey: PublicKey, bond_units: u32 }
SchedulerStats { producer_count, total_bonds, min_bonds, max_bonds, avg_bonds }
```

### NetworkParams (`network_params/mod.rs:47`)
Large struct — all network-tunable parameters. Key fields:
- `bond_unit: u64`, `slot_duration: u64`, `genesis_time: u64`
- `vdf_iterations: u64`, `blocks_per_reward_epoch: u64`, `vesting_quarter_slots: u64`
- All activation heights (see ACTIVATION-HEIGHTS section)
- `max_peers: usize`, mesh gossip params

### Attestation (`attestation.rs`)
```rust
Attestation { block_hash, slot, height, attester, attester_weight, signature, bls_signature }
RegionAggregate { block_hash, slot, region, attester_count, total_weight, signatures, attesters }
MinuteAttestationTracker  // tracks per-minute attestation (60 minutes/epoch)
```

### Conditions (`conditions/mod.rs`)
```rust
Condition {
    Signature(pubkey_hash),
    Multisig(threshold, keys),
    Hashlock(expected_hash),
    Timelock(min_height),        // height >= min_height
    TimelockExpiry(max_height),  // height < max_height
    And(Box, Box),
    Or(Box, Box),
    Threshold(n, conditions),
    AmountGuard(min_amount, output_index),    // guards_activation_height
    OutputTypeGuard(expected_type, idx),     // guards_activation_height
    RecipientGuard(expected_hash, idx),      // guards_activation_height
}
Witness { signatures: Vec<WitnessSignature>, preimages: Vec<Vec<u8>> }
EvalContext { height: BlockHeight, outputs: &[Output] }
```

### Finality (`finality.rs`)
```rust
FinalityCheckpoint { block_hash, height, slot, attestation_weight, total_weight }
FinalityTracker  // tracks pending blocks, emits checkpoint when 2/3+ weight
```


## FUNCTIONS

### Block hash computation (`block.rs:76-97`)
`BlockHeader::hash()` — BLAKE3 over: version, prev_hash, merkle_root, presence_root, genesis_hash, missed_producers (count + each pk), data_root, fork_id (if non-zero), timestamp, slot, producer, vdf_output.value
**NOTE**: fork_id only hashed if non-zero (backward compat). Timestamp/slot NOT in VDF input.

### VDF input (`block.rs:100-107`)
`BlockHeader::vdf_input() -> Hash` → `vdf::block_input(prev_hash, merkle_root, slot, producer)`

### Scheduler selection (`scheduler.rs:171-185`)
```
select_producer(slot, rank):
  offset = (total_bonds * rank) / MAX_FALLBACK_RANKS
  ticket = (slot + offset) % total_bonds
  binary search ticket_boundaries → producer
```
**CRITICAL**: selection.rs `select_producer_for_slot()` is DEPRECATED — use `DeterministicScheduler`.

### Slot timing (`consensus/selection.rs:94-111`)
```
eligible_rank_at_ms(offset_ms) -> Option<usize>:
  rank = offset_ms / FALLBACK_TIMEOUT_MS(2000)
  if rank < MAX_FALLBACK_RANKS(2): Some(rank) else None
```

### Withdrawal penalty (`consensus/constants.rs:371-378`)
```
withdrawal_penalty_rate_with_quarter(bond_age_slots, quarter_slots):
  quarters = bond_age_slots / quarter_slots
  0→75%, 1→50%, 2→25%, 3+→0%
```

### Max block size (`consensus/constants.rs:414-421`)
```
max_block_size(height): era = height / BLOCKS_PER_ERA; BASE_BLOCK_SIZE << era, capped at MAX_BLOCK_SIZE_CAP
max_extra_data_size(height): BASE_EXTRA_DATA_SIZE << era, capped at MAX_EXTRA_DATA_SIZE_CAP
```

### Epoch state (`epoch_state/mod.rs`)
- `EpochState::genesis()` → zeroed state for epoch 0
- `EpochState::accumulate_block(input)` → updates attested_sets[0], attestation_accum[0], blocks_produced
- `EpochState::derive_at_boundary(prev, input) -> EpochState` → 3-epoch lookback + 2/3 deadlock floor + tier system
- `epoch_state_hash(state) -> Hash` → for state root computation

### Attestation bitfield (`attestation.rs`)
- `encode_attestation_bitfield(attested_indices, total_producers) -> Vec<u8>`
- `decode_attestation_bitfield(bitfield, producer_list, extra) -> Vec<usize>` — order = [producer_list | extra sorted]
- `validate_attestation_bitfield(bitfield, block, ctx) -> Result<(), AttestationError>`
- `attestation_minute(slot: u32) -> u32` — `slot % ATTESTATION_MINUTES_PER_EPOCH(60)`

### Producer selection validation (`validation/producer.rs`)
- `validate_producer_eligibility(block, ctx) -> Result<(), ValidationError>`
- `bootstrap_fallback_order(producers, slot) -> Vec<PublicKey>` — pre-epoch scheduling

### Conditions (`conditions/eval.rs`)
- `evaluate(condition, witness, ctx) -> Result<(), ConditionError>` — deterministic, bounded

### Output constructors (`transaction/output.rs`)
- `Output::normal(amount, pubkey_hash)`
- `Output::bond(amount, pubkey_hash, lock_until, creation_slot)` — extra_data = creation_slot (4B LE)
- `Output::nft(...)`, `Output::fungible_asset(...)`, `Output::pool(...)`, `Output::collateral(...)`
- `Output::encrypted_content(...)`, `Output::encrypted_content_v1(...)`

### Genesis (`genesis.rs`)
- `generate_genesis_block(config) -> Block` — `genesis.rs:189`
- `genesis_hash(network) -> Hash` — `genesis.rs:302`


## CONSTANTS

All from `crates/core/src/consensus/constants.rs` unless noted.

**Protocol:**
- `INITIAL_PROTOCOL_VERSION: u32 = 1` — `constants.rs:7`
- `GENESIS_TIME: u64 = 1776837510` — `constants.rs:25` (must match chainspec.mainnet.json)
- `PROTOCOL_VERSION: u32 = 1` — `lib.rs:348`

**Time structure:**
- `SLOT_DURATION: u64 = 10` — seconds per slot
- `SLOTS_PER_EPOCH: u32 = 360` — 1 hour
- `SLOTS_PER_REWARD_EPOCH: u32 = 360`
- `BLOCKS_PER_REWARD_EPOCH: BlockHeight = 360`
- `SLOTS_PER_ERA: BlockHeight = 12_614_400` — ~4 years
- `BOOTSTRAP_BLOCKS: BlockHeight = 60_480` — ~1 week
- `YEAR_IN_SLOTS: Slot = 3_153_600`
- `VESTING_QUARTER_SLOTS: Slot = 3_153_600` — 1 year (mainnet)
- `VESTING_PERIOD_SLOTS: Slot = 12_614_400` — 4 years full vest

**Economics:**
- `INITIAL_REWARD: Amount = 100_000_000` — 1 DOLI per block
- `BLOCK_REWARD_POOL: Amount = 100_000_000`
- `EPOCH_REWARD_POOL: Amount = 36_000_000_000` — 360 DOLI/hour
- `TOTAL_SUPPLY: Amount = 2_522_880_000_000_000` — 25,228,800 DOLI
- `BOND_UNIT: Amount = 1_000_000_000` — 10 DOLI (mainnet)
- `MAX_BONDS_PER_PRODUCER: u32 = 3_000`
- `UNBONDING_PERIOD: BlockHeight = 60_480` — ~7 days
- `DELEGATION_UNBONDING_SLOTS: u64 = 60_480`
- `DELEGATE_REWARD_PCT: u32 = 10`
- `STAKER_REWARD_PCT: u32 = 90`

**Fees:**
- `BASE_FEE: Amount = 1` — minimum 1 satoshi
- `FEE_PER_BYTE: Amount = 1`
- `FEE_DIVISOR: Amount = 100` — effective rate = 0.01 sat/byte

**Block sizes:**
- `BASE_BLOCK_SIZE: usize = 2_000_000` — 2 MB era 0
- `MAX_BLOCK_SIZE_CAP: usize = 32_000_000` — 32 MB era 4+
- `BASE_EXTRA_DATA_SIZE: usize = 524_288` — 512 KB era 0
- `MAX_EXTRA_DATA_SIZE_CAP: usize = 8_388_608` — 8 MB era 4+

**Timing/windows:**
- `FALLBACK_TIMEOUT_MS: u64 = 2_000` — exclusive 2s fallback windows
- `MAX_FALLBACK_RANKS: usize = 2` — rank 0 (primary) + rank 1 (single fallback)
- `MAX_FUTURE_SLOTS: u64 = 1`
- `MAX_PAST_SLOTS: u64 = 192` — 32 minutes
- `MAX_DRIFT: u64 = 1` — 1 second clock drift tolerance
- `MAX_DRIFT_MS: u64 = 200`
- `BOOTSTRAP_GRACE_PERIOD_SECS: u64 = 15`
- `COINBASE_MATURITY: BlockHeight = 6`

**Presence/scoring:**
- `MIN_PRESENCE_RATE: u32 = 50` — 50% minimum
- `MIN_PRESENCE_SCORE: PresenceScore = 1`
- `MAX_PRESENCE_SCORE: PresenceScore = 10_000`
- `INITIAL_PRESENCE_SCORE: PresenceScore = 100`
- `SCORE_PRODUCE_BONUS: PresenceScore = 1`
- `SCORE_MISS_PENALTY: PresenceScore = 2`

**Tier system:**
- `ACTIVE_PRODUCERS_CAP: usize = 50` — max in round-robin
- `MIN_ATTESTATION_MINUTES: usize = 30` — out of 60 per epoch

**Inactivity leak:**
- `INACTIVITY_LEAK_START: u64 = 360` — 1 epoch
- `INACTIVITY_LEAK_RATE: u64 = 10` — 10% per epoch
- `INACTIVITY_LEAK_FLOOR: u64 = 1`
- `LIVENESS_WINDOW_MIN: u64 = 500`
- `REENTRY_INTERVAL: u32 = 50`

**Conditions constants (`conditions/mod.rs`):**
- `CONDITION_VERSION: u8 = 1`
- `MAX_CONDITION_OPS: usize` — DoS prevention
- `MAX_MULTISIG_KEYS: usize`
- `MAX_THRESHOLD_CONDITIONS: usize`
- `MAX_WITNESS_SIZE: usize`
- `HASHLOCK_DOMAIN: &[u8]` — domain for hash derivation

**Finality (`finality.rs`):**
- `FINALITY_THRESHOLD_PCT: u32 = 67`
- `FINALITY_TIMEOUT_SLOTS: u32 = 3`

**Attestation:**
- `ATTESTATION_MINUTES_PER_EPOCH: usize = 60`
- `ATTESTATION_QUALIFICATION_THRESHOLD: usize = 30` — minimum qualified minutes

**Types (`types.rs`):**
- `DECIMALS: u32 = 8`
- `UNITS_PER_COIN: Amount = 100_000_000`

**Registration (`consensus/registration.rs`):**
- `BASE_REGISTRATION_FEE`
- `MAX_REGISTRATION_FEE`
- `MAX_REGISTRATIONS_PER_BLOCK`

**Maintainer (`maintainer.rs`):**
- `INITIAL_MAINTAINER_COUNT`, `MAINTAINER_THRESHOLD`, `MAX_MAINTAINERS`, `MIN_MAINTAINERS`

**Ghost exclusion:**
- `GHOST_EXCLUSION_GRACE_EPOCHS: u64 = 3`
- `GHOST_EXCLUSION_ACTIVATION_HEIGHT: u64 = u64::MAX` — **DEPRECATED**, use NetworkParams


## ACTIVATION-HEIGHTS

All from `consensus/constants.rs`. Currently ALL = 0 (active from genesis on current chain).
Use `NetworkParams` fields instead of these constants for network-aware code.

| Constant | Value | What activates |
|----------|-------|----------------|
| `EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT` | 0 | EpochReward txs must have explicit pool UTXO inputs |
| `BITFIELD_BODY_ACTIVATION_HEIGHT` | 0 | Attestation bitfield moved from header to block body |
| `TIER_SYSTEM_ACTIVATION_HEIGHT` | 0 | Only top 50 producers enter round-robin |
| `TIER_PROMOTION_ACTIVATION_HEIGHT` | 0 | Active list by attestation_count, not seniority |
| `UNIQUE_COINBASE_ACTIVATION_HEIGHT` | 0 | Coinbase extra_data = height ++ slot (globally unique) |
| `SNAP_HEADER_ACTIVATION_HEIGHT` | 0 | Snap sync includes anchor header |
| `REWARDS_EPOCH_LIST_FIX_HEIGHT` | 0 | Reward decoding uses epoch_state.producer_list |
| `FULL_BITFIELD_DECODE_HEIGHT` | 0 | Decodes ALL bitfield indices (base + extra) |
| `ENCRYPTED_CONTENT_ACTIVATION_HEIGHT` | 0 | New NFT outputs rejected; EncryptedContent only |
| `EPOCH_STATE_REORG_ACTIVATION_HEIGHT` | 0 | execute_reorg restores epoch_state from undo data |
| `GHOST_EXCLUSION_ACTIVATION_HEIGHT` | u64::MAX | **DEPRECATED** — use `NetworkParams::ghost_exclusion_activation_height` |

NetworkParams (per-network, overrides above for active networks):
- `sig_verification_height` — mandatory Input.public_key
- `inc_i_026_scheduler_activation_height` — pure slot%len scheduling
- `fork_id_activation_height` — fork_id header enforcement
- `full_bitfield_decode_height` — mainnet: 14,000
- `rewards_epoch_list_fix_height` — mainnet: 13,320
- `encrypted_content_activation_height` — mainnet: 37,500
- `epoch_state_reorg_activation_height` — mainnet: 44,246
- `security_audit_activation_height` — mainnet: 27,547; testnet: 21,450
- `encrypted_content_v2_activation_height` — mainnet: 71,290; testnet: 20,690
- `ghost_exclusion_activation_height` — mainnet: u64::MAX; testnet: 10,830


## DEPENDENCIES

Internal crates used by `crates/core`:
- `crypto` — `Hash`, `PublicKey`, `PrivateKey`, `Signature`, BLS types, BLAKE3 hasher
- `vdf` — `VdfOutput`, `VdfProof`, `block_input()`
- `serde` + `bincode` — serialization (bincode for wire, serde for JSON RPC)
- `thiserror` — `ValidationError`, `AttestationError`, etc.
- `proptest` — property-based tests in `types.rs`

External consumers of `crates/core`:
- `bins/node` — uses all validation + epoch_state + scheduler
- `crates/storage` — uses Block, Transaction, Output, Hash types
- `crates/network` — uses Block, Transaction, ValidationError
- `crates/rpc` — uses block/tx types, ValidationContext
- `crates/mempool` — uses Transaction, validate_transaction


## CONSTRAINTS

**Immutable invariants — violating these forks the chain:**

1. **Bitfield encoder/decoder must use identical order**: `[epoch_state.producer_list | extra_sorted_by_pubkey]`. Any mismatch = wrong reward indices = permanent fork.

2. **CURRENT_PROTOCOL_VERSION must NOT be bumped** unless `EpochState` serialization format changes. Bumping triggers `delete_epoch_state()` on every node restart → non-deterministic rebuild → fork. Use `EPOCH_STATE_FORMAT_VERSION` for struct changes.

3. **Activation heights are IMMUTABLE** once crossed on mainnet. Never move an active height forward (higher). New features get their OWN height.

4. **EpochState::derive_at_boundary is the ONE canonical function** for epoch transitions. No alternative implementations. Node-local state (excluded_producers etc.) MUST NOT be used as scheduling input.

5. **Bond extra_data stamped by node**: CLI sends `creation_slot=0`, node stamps real slot at `apply_block()`. Never trust raw tx extra_data for bond creation_slot.

6. **Coinbase always goes to reward_pool_pubkey_hash** (deterministic address, no private key). Only epoch boundary EpochReward tx drains it.

7. **EncryptedContent is NOT conditioned**: `OutputType::is_conditioned()` returns false for EncryptedContent. Its extra_data uses `[ct_len | ciphertext | wrapped_key | nonce | content_hash]` layout, not condition-prefixed. (AUDIT-NFT-001 fix: incorrect conditioned check made EncryptedContent UTXOs unspendable.)

8. **HardForkSchedule NEVER used for rolling deploys**: `current_fork_id()` uses `u64::MAX`, making ALL schedule entries active immediately in fork_id. Use constant gates or NetworkParams activation heights instead.

9. **Producer mutations deferred to epoch boundary** (except epoch 0 and maintainer changes, which are immediate).

10. **Output::is_native_amount()** must be checked before summing amounts — Pool, LPShare, FungibleAsset, Collateral, ZKRollup store non-DOLI values in `amount`.


## PATTERNS

### How to check if a feature is active
```rust
// PREFERRED (network-aware):
let params = NetworkParams::load(network);
if current_height >= params.security_audit_activation_height { ... }

// For code without NetworkParams access:
use doli_core::consensus::is_protocol_active;
if is_protocol_active(required_version, state.active_protocol_version) { ... }
```

### How to build a ValidationContext
```rust
ValidationContext::new(params, network, unix_now, block_height)
    .with_prev_block(prev_slot, prev_ts, prev_hash)
    .with_epoch_producer_list(epoch_state.active_list.clone()) // epoch-frozen!
    .with_fork_id(expected_fork_id, params.fork_id_activation_height)
    .with_security_audit_activation_height(params.security_audit_activation_height)
    // ... other activation heights from NetworkParams
```

### How to compute an epoch boundary
```rust
let input = EpochDerivationInput {
    active_producers: producer_set.active_producers(height),
    bond_counts: utxo_set.bond_counts_at(height),
    blocks_per_epoch: params.blocks_per_reward_epoch,
    snap_attestation_skip_height: params.snap_attestation_skip_height,
    height, epoch,
    registered_at: producer_set.registered_at_map(),
    ghost_exclusion_activation_height: params.ghost_exclusion_activation_height,
};
let new_epoch_state = EpochState::derive_at_boundary(&prev_epoch_state, &input);
```

### Slot → epoch/era conversion
```rust
let epoch = slot / SLOTS_PER_EPOCH;       // slot is u32, SLOTS_PER_EPOCH=360
let era   = height / BLOCKS_PER_ERA;       // height is u64, BLOCKS_PER_ERA=12_614_400
let slot_in_epoch = slot % SLOTS_PER_EPOCH;
```

### Amount arithmetic — always use checked/saturating
```rust
let total = a.saturating_add(b);           // never plain `+` for amounts
let fee = amount.saturating_sub(output_sum);
```

### TxType discriminants (no gaps except 16, 23)
```
0=Transfer, 1=Registration, 2=Exit, 3=ClaimReward, 4=ClaimBond, 5=SlashProducer,
6=Coinbase, 7=AddBond, 8=RequestWithdrawal, 9=ClaimWithdrawal(tombstone),
10=EpochReward, 11=RemoveMaintainer, 12=AddMaintainer, 13=DelegateBond,
14=RevokeDelegation, 15=ProtocolActivation, 17=MintAsset, 18=BurnAsset,
19=CreatePool, 20=AddLiquidity, 21=RemoveLiquidity, 22=Swap,
24=CreateLoan, 25=RepayLoan, 26=LiquidateLoan, 27=LendingDeposit,
28=LendingWithdraw, 29=FractionalizeNft, 30=RedeemNft, 31=ZKSettle
NOTE: 16 and 23 are intentionally skipped (reserved/removed)
```

### OutputType discriminants
```
0=Normal, 1=Bond, 2=Multisig, 3=Hashlock, 4=HTLC, 5=Vesting,
6=NFT, 7=FungibleAsset, 8=BridgeHTLC, 9=Pool, 10=LPShare,
11=Collateral, 12=LendingDeposit, 13=ZKRollup, 14=EncryptedContent
Conditioned (uses condition-prefix in extra_data): 2,3,4,5,6,7,8
NOT conditioned (EncryptedContent=14): uses raw extra_data layout
```

### Network defaults (devnet overrides for fast testing)
```
Mainnet/Testnet: SLOT_DURATION=10s, SLOTS_PER_EPOCH=360, BOND_UNIT=10 DOLI
Devnet: slot_duration=1s, slots_per_epoch=60, bond_unit=1 DOLI, blocks_per_era=576
```

### Validation modes summary
- `Full` — VDF proof verified (gossip blocks, tip of sync)
- `Light` — VDF skipped (gap blocks after snap-sync, state root trusted by quorum)
- `Replay` — disaster recovery, also skips dedup + recovery gate + snap height guard
