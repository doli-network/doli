# core — DOLI Core Consensus & Types
<!-- @INDEX
ENTRY-POINTS: lines 15-53
OPERATIONS: lines 55-69
DATA-FLOWS: lines 71-162
STRUCTS: lines 164-372
FUNCTIONS: lines 374-451
CONSTANTS: lines 453-582
ACTIVATION-HEIGHTS: lines 584-628
DEPENDENCIES: lines 630-646
CONSTRAINTS: lines 648-681
PATTERNS: lines 683-788
-->

## ENTRY-POINTS

Crate: `doli-core` (`crates/core/`), ~109 `.rs` files, 24 top-level modules (see `lib.rs:115-138`):
`attestation, block, chainspec, conditions, config_validation, consensus, discovery, epoch_state,
finality, genesis, heartbeat, maintainer, network, network_params, nft, oracle, pool, presence,
rewards, scheduler, tpop, transaction, types, validation`.

Primary public API (all re-exported from `crates/core/src/lib.rs`):

**Block validation** — call these to process an incoming block:
- `validate_block(block, ctx) -> Result<(), ValidationError>` — full validation (VDF + context), `validation/block.rs`
- `validate_block_with_mode(block, ctx, mode) -> Result<(), ValidationError>` — explicit mode
- `validate_header(header, ctx) -> Result<(), ValidationError>` — header-only

**Transaction validation:**
- `validate_transaction(tx, ctx) -> Result<(), ValidationError>` — structural, `validation/transaction.rs`
- `validate_transaction_with_utxos(tx, ctx, utxo_provider) -> Result<(), ValidationError>` — full with UTXO lookups, `validation/utxo.rs`
- `verify_amm_conservation(...) -> AmmConservationResult` — pool-aware conservation (INC-I-096), `validation/amm.rs`

**Producer scheduling:**
- `DeterministicScheduler::new(producers)` — `scheduler.rs:106`
- `DeterministicScheduler::select_producer(slot, rank) -> Option<&PublicKey>` — `scheduler.rs:171`
- `EpochState::derive_at_boundary(prev, input) -> EpochState` — `epoch_state/mod.rs:238` (ONE canonical epoch transition)
- `EpochState::accumulate_block(input)` — `epoch_state/mod.rs:205`
- `compute_live_producer_list(...) -> Vec<PublicKey>` — `epoch_state/mod.rs:32` (shared by scheduler + rewards.rs)

**Genesis:**
- `generate_genesis_block(config: &GenesisConfig) -> Block` — `genesis.rs:189`
- `genesis_hash(network: Network) -> Hash` — `genesis.rs:302`
- `verify_genesis_block(block, network) -> Result<(), GenesisError>` — `genesis.rs:326`

**Network params (always prefer over raw constants):**
- `NetworkParams::load(network: Network) -> &'static NetworkParams` — `network_params/mod.rs:518`, cached singleton, runs `validate_amm_conservation_ordering()` debug-assert (INV-DEPLOY-002)

**Oracle (Phase 2.1, frozen pre-activation):**
- `bond_weighted_median(attestations, bond_snapshot) -> Option<(u64, u16)>` — `oracle/mod.rs:107`
- `compute_structural_share_bps(...) -> Option<u16>` — `oracle/mod.rs:443` (sunset metric, M8)
- `OracleSunsetState::transition(share_bps, epoch) -> OracleHealthState` — `oracle/mod.rs:347`


## OPERATIONS

Consensus-visible procedures — how a producer builds/validates a block, how epoch rewards drain,
how activation gates are checked.

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Build a block for a slot | 1. Look up scheduled producer via `DeterministicScheduler::select_producer(slot, rank)` 2. `BlockBuilder::new(prev_hash, prev_slot, producer)` 3. `.with_params(params).with_presence_root(presence_commitment).with_missed_producers(gap_list)` 4. select+add mempool txs within `max_block_size(height)` / builder policy budget 5. prepend coinbase tx 6. `.build_with_vdf(timestamp)` | `BlockBuilder::new`, `.with_presence_root`, `.build_with_vdf` (`block.rs:282+`) | prev block info, mempool txs, missed-producer gap, VDF iterations from `NetworkParams` | `Block` with valid header hash, VDF proof, merkle_root matching transactions |
| Validate an incoming block | 1. Build `ValidationContext` with epoch-frozen producer list + all activation heights 2. `validate_block_with_mode(block, ctx, mode)` — `Full` (VDF checked) or `Light` (gap blocks post-snap) | `ValidationContext::new(...).with_epoch_producer_list(...).with_fork_id(...)`, `validate_block_with_mode` | `Block`, `ConsensusParams`, `NetworkParams` activation heights, epoch-frozen producer list | `Ok(())` or typed `ValidationError` |
| Compute the next epoch boundary | 1. Gather `EpochDerivationInput` (active_producers, bond_counts, registered_at, ghost/prune activation heights) 2. `EpochState::derive_at_boundary(&prev, &input)` | `EpochState::derive_at_boundary` (`epoch_state/mod.rs:238`) | prev `EpochState`, bond snapshot from UTXO set at boundary height, `NetworkParams::ghost_exclusion_activation_height` / `epoch_prune_activation_height` | New `EpochState` — identical on every node given identical inputs (type-level guarantee) |
| Distribute epoch rewards | 1. At `height % blocks_per_reward_epoch == 0`, decode all block bitfields using `epoch_state.producer_list` ordering 2. `Transaction::new_epoch_reward_coinbase(pool_inputs, distributions, height, epoch)` | encode/decode_attestation_bitfield (`attestation.rs:266+`), reward calc in `bins/node` (out of domain) | pool UTXO outpoints (`EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT` gate), qualified producer list | `EpochReward` tx (TxType=10) draining the pool, bond-weighted |
| Submit a Phase 2.1 price attestation (frozen — `oracle_activation_height = u64::MAX` on all networks) | 1. Attester builds `PriceAttestationData` (144B) with `pair_id = phase_2_1_known_pair_id()` 2. Submit as `TxType::PriceAttestation` (16), empty inputs/outputs 3. Node rejects pre-activation with `[ERRTX-ORACLE001]` | `phase_2_1_known_pair_id()` (`oracle/mod.rs:202`), `TxType::PriceAttestation` | bonded producer status, one attestation per `(epoch, pair_id)` | Pre-activation: always rejected. Post-activation: aggregated at epoch boundary into `OraclePrice` UTXO via `bond_weighted_median` |
| Swap/AddLiquidity/RemoveLiquidity/CreatePool (`amm_activation_height = 0` on all networks post fresh-genesis reset) | 1. Build tx with `TxType::{CreatePool,AddLiquidity,RemoveLiquidity,Swap}` 2. Validator checks `current_height >= amm_activation_height` 3. Pool-input auth exempted from signature (RC-A, `inc_i_092_activation_height`) 4. Pool-aware conservation (RC-B/INC-I-096) | `pool::compute_swap`, `compute_lp_shares`, `compute_remove_liquidity` (`pool.rs`), `validation/amm.rs::verify_amm_conservation` | reserves in Pool UTXO `extra_data`, `MINIMUM_LIQUIDITY=1000` locked on CreatePool | Pool UTXO reserves updated per x·y=k invariant, LP shares minted/burned |
| Check whether a feature is active at a given height | 1. `NetworkParams::load(network)` 2. compare `current_height` against the specific `*_activation_height` field (NEVER a bare `consensus::` constant for network-aware code) | `NetworkParams::load`, field access | `Network`, `current_height` | Correct gate decision without cross-network leakage |


## DATA-FLOWS

### Block production → apply
```
Producer: BlockBuilder::new(prev_hash, prev_slot, producer)
  .with_params(ConsensusParams)
  .with_presence_root(presence_commitment)   // commitment HASH, never decoded into indices
  .with_missed_producers(vec![...])          // on-chain liveness
  .add_transaction(coinbase_tx)              // first tx = coinbase to reward pool
  .build_with_vdf(timestamp) -> Block
```

`presence_root` is a commitment HASH, not a bit array, and is never decoded into producer
indices. Its preimage depends on `inc_i_178_attestation_bls_activation_height` — `u64::MAX` on
mainnet, testnet AND devnet today, so **no network is pinned**:
- below the height: `BLAKE3(attestation_bitfield)`, and zero attesters keeps the `Hash::ZERO`
  sentinel — byte-identical to the 6.26.x binary
- at/after: `BLAKE3( u32le(len bits) ‖ bits ‖ u32le(len agg) ‖ agg )` over the body bit array and
  the aggregate BLS signature (`attestation/commitment.rs::presence_commitment`); zero attesters
  yields the canonical empty commitment — a REAL hash, NOT `Hash::ZERO`

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

### Phase 2.1 oracle aggregation (M6, orchestrator lives in bins/node, pure fns here)
```
Per epoch: collect all valid PriceAttestation txs for closing epoch
  → dedupe_latest_per_attester(contributions) -> Vec<AttestationContribution>  [oracle/mod.rs:171]
  → bond_weighted_median(attestations, bond_snapshot) -> Option<(median_cents, count)>  [oracle/mod.rs:107]
  → orchestrator writes/consumes OraclePrice UTXO at oracle_price_outpoint(pair_id)  [oracle/mod.rs:508]
Sunset check (M8, every boundary):
  compute_structural_share_bps(bond_snapshot[prev epoch], registered_at, height, blocks_per_epoch, STRUCTURAL_PUBKEY_HASHES)
  → OracleSunsetState::transition(share_bps, epoch) -> Healthy | Warning | HaltRecoverable | HaltPermanent
  → share < 5500 bps (55%) HALTs new attestations; share < 5500 for >=4 epochs -> HaltPermanent (binary upgrade only)
```

### AMM pool flow (pure math in pool.rs, gates in NetworkParams)
```
CreatePool: reserve_a/reserve_b funded by net DOLI inputs (RC-B, inc_i_092_activation_height)
  → MINIMUM_LIQUIDITY=1000 permanently locked, never materialized as LPShare UTXO
Swap: pool::compute_swap(reserve_a, reserve_b, dx, fee_bps) -> (dy, new_a, new_b)
  → Pool UTXO input EXEMPT from signature (authorized by x*y=k invariant, RC-A)
  → validation/amm.rs::verify_amm_conservation accounts for Pool extra_data reserve deltas (INC-I-096)
RemoveLiquidity: pool::compute_remove_liquidity(shares, reserve_a, reserve_b, total_shares)
  → proportional withdrawal bound to LP shares burned (INC-I-096)
```

### ValidationContext construction (node layer fills this)
```
ValidationContext::new(params, network, current_time, current_height)
  .with_prev_block(slot, timestamp, hash)
  .with_epoch_producer_list(frozen_list)     // epoch-frozen, never changes mid-epoch
  .with_producers_weighted([(pk, weight)])   // for anti-grinding selection
  .with_fork_id(expected_hash, activation_height)
  .with_security_audit_activation_height(h)
  .with_amm_activation_height(h)              // AMM Foundations M1
  .with_oracle_activation_height(h)           // Phase 2.1 oracle (frozen: u64::MAX)
  .with_oracle_sunset_triggered(bool)         // M8 sunset flag, node-maintained
  .with_inc_i_092_activation_height(h)        // Pool-input auth + CreatePool funding
  .with_inc_i_096_activation_height(h)        // Pool-aware conservation
```


## STRUCTS

### Block types (`block.rs`)
```rust
BlockHeader {
    version: u32,           // v2 = genesis_hash added
    prev_hash: Hash,
    merkle_root: Hash,
    presence_root: Hash,    // commitment hash; preimage is AH-dependent (see DATA-FLOWS)
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
    attestation_bitfield: Vec<u8>,      // attestation bits; accepted width per `attestation/width.rs`
}

BlockBuilder { ... }  // builder pattern, block.rs:282
```

### Transaction types (`transaction/` — split into core.rs, data.rs, output.rs, types.rs, legacy.rs)
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

### Oracle types (`oracle/mod.rs`)
```rust
AttestationContribution { signer_hash: Hash, price_cents: u64 }
OracleHealthState { Healthy, Warning, HaltRecoverable, HaltPermanent }
OracleSunsetState { warning_since_epoch: Option<u64>, halt_since_epoch: Option<u64>, halt_permanent: bool }
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
    epoch_prune_activation_height: u64,          // NEW (INC-I-116)
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

### Validation types (`validation/types.rs`) — much larger than pre-DeFi era
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
    encrypted_content_v2_activation_height: u64,
    security_audit_activation_height: u64,
    defi_activation_height: u64,                // NEW — INC-I-088 Phase 0 (tombstoned types, retained)
    amm_activation_height: u64,                 // NEW — AMM Foundations M1
    oracle_activation_height: u64,              // NEW — Phase 2.1 oracle
    oracle_sunset_triggered: bool,              // NEW — M8 sunset flag, node-maintained
    inc_i_092_activation_height: u64,           // NEW — pool-input auth + CreatePool funding
    inc_i_096_activation_height: u64,           // NEW — pool-aware conservation
    ...
}

ValidationMode { Full, Light, Replay }
// Full = VDF verified; Light = VDF skipped (gap blocks post snap-sync); Replay = disaster recovery

UtxoInfo { output: Output, pubkey: Option<PublicKey>, spent: bool }
trait UtxoProvider { fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> }
```
Validation module now spans 14 files: `amm.rs, block.rs, error.rs, errors_oracle.rs, parallel.rs,
pool.rs, producer.rs, registration.rs, rewards_legacy.rs (dead code), transaction.rs, tx_types.rs,
types.rs, utxo.rs, zk.rs` (`validation/mod.rs:1-66`).

### Scheduler (`scheduler.rs`)
```rust
DeterministicScheduler { producers: Vec<ScheduledProducer>, total_bonds: u64, ticket_boundaries: Vec<u64> }
ScheduledProducer { pubkey: PublicKey, bond_units: u32 }
SchedulerStats { producer_count, total_bonds, min_bonds, max_bonds, avg_bonds }
```

### NetworkParams (`network_params/mod.rs:50`)
Large struct (~65 fields) — all network-tunable parameters, split across `mod.rs` (struct + docs),
`defaults.rs` (per-network hardcoded values), `env_loader.rs`, `chainspec_loader.rs`. Key fields:
- `bond_unit: u64`, `slot_duration: u64`, `genesis_time: u64`
- `vdf_iterations: u64`, `blocks_per_reward_epoch: u64`, `vesting_quarter_slots: u64`
- All activation heights (see ACTIVATION-HEIGHTS section)
- `max_peers: usize`, mesh gossip params (`mesh_n`, `mesh_n_low`, `mesh_n_high`, `gossip_lazy`)
- `received_delegation_cap: u64` — max total delegated bonds per producer (INC-I-078)

### Attestation (`attestation.rs`)
```rust
Attestation { block_hash, slot, height, attester, attester_weight, signature, bls_signature }
RegionAggregate { block_hash, slot, region, attester_count, total_weight, signatures, attesters }
MinuteAttestationTracker  // tracks per-minute attestation (60 minutes/epoch)
```

### AMM pool math (`pool.rs`) — pure integer arithmetic, no floating point
```rust
compute_swap(reserve_a, reserve_b, dx, fee_bps) -> Option<(dy, new_a, new_b)>       // pool.rs:13
compute_initial_lp_shares(amount_a, amount_b) -> Amount                            // pool.rs:41, isqrt
compute_lp_shares(amount_a, amount_b, reserve_a, reserve_b, total) -> Option<Amount> // pool.rs:47
compute_remove_liquidity(shares, reserve_a, reserve_b, total) -> Option<(da, db)>   // pool.rs:68
compute_twap_price(...), update_twap(...), verify_invariant(...)
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

### Scheduler selection (`scheduler.rs:171-199`)
```
select_producer(slot, rank):
  offset = (total_bonds * rank) / MAX_FALLBACK_RANKS
  ticket = (slot + offset) % total_bonds
  binary search ticket_boundaries → producer
```
**CRITICAL**: `consensus::selection::select_producer_for_slot()` is DEPRECATED — use `DeterministicScheduler`.

### Slot timing (`consensus/selection.rs`)
```
eligible_rank_at_ms(offset_ms) -> Option<usize>:
  rank = offset_ms / FALLBACK_TIMEOUT_MS(2000)
  if rank < MAX_FALLBACK_RANKS(2): Some(rank) else None
```

### Withdrawal penalty (`consensus/constants.rs:436-444`)
```
withdrawal_penalty_rate_with_quarter(bond_age_slots, quarter_slots):
  quarters = bond_age_slots / quarter_slots
  0→75%, 1→50%, 2→25%, 3+→0%
```

### Max block size (`consensus/constants.rs:509-516`)
```
max_block_size(height): era = height / BLOCKS_PER_ERA; BASE_BLOCK_SIZE << era, capped at MAX_BLOCK_SIZE_CAP
```
Builder-policy budgets (NOT consensus rules, `consensus/constants.rs:487-495`): pre-`large_block_activation_height`
use `LEGACY_BLOCK_SELECT_BUDGET=1_000_000`; post-activation `LARGE_BLOCK_SELECT_BUDGET=1_900_000` (~300 TPS, INC-I-091).

### Epoch state (`epoch_state/mod.rs`)
- `compute_live_producer_list(...) -> Vec<PublicKey>` — `epoch_state/mod.rs:32`, shared attestation-filter + floor logic
- `EpochState::genesis()` → `epoch_state/mod.rs:189`, zeroed state for epoch 0
- `EpochState::accumulate_block(input)` → `epoch_state/mod.rs:205`
- `EpochState::derive_at_boundary(prev, input) -> EpochState` → `epoch_state/mod.rs:238` — 3-epoch lookback + floor (proportional pre-INC-I-116, absolute `MIN_PRODUCERS_FLOOR=3` post) + tier system
- `epoch_state_hash(state) -> Hash` → for state root computation

### Attestation bitfield (`attestation.rs`)
- `attestation_minute(slot: u32) -> u32` — `attestation.rs:257`, `slot / SLOTS_PER_ATTESTATION_MINUTE(6)`
- `encode_attestation_bitfield(attested_indices) -> Hash` — `attestation.rs:266`, 256-producer cap (legacy header format)
- `decode_attestation_bitfield(bitfield, producer_list, extra) -> Vec<usize>` — order = [producer_list | extra sorted]
- `attestation_minutes_per_epoch(blocks_per_epoch) -> u32` — `attestation.rs:240`, `blocks_per_epoch/6`
- `attestation_qualification_threshold(blocks_per_epoch) -> u32` — `attestation.rs:247`, `90% of minutes` (mainnet: 54 of 60 — NOT the static 30 used by the *tier-promotion* system, which is a separate `MIN_ATTESTATION_MINUTES=30` const)

### Oracle (`oracle/mod.rs`)
- `bond_weighted_median(attestations, bond_snapshot) -> Option<(u64, u16)>` — `oracle/mod.rs:107`, 50%-crossing median, lower-median tie-break
- `dedupe_latest_per_attester(contributions) -> Vec<AttestationContribution>` — `oracle/mod.rs:171`, BTreeMap-ordered (deterministic)
- `compute_structural_share_bps(...) -> Option<u16>` — `oracle/mod.rs:443`, 1-epoch-lagged anti-dilution metric
- `oracle_price_outpoint(pair_id) -> (Hash, u32)` — `oracle/mod.rs:508`, deterministic synthetic outpoint for the singleton UTXO

### Producer selection validation (`validation/producer.rs`)
- `validate_producer_eligibility(block, ctx) -> Result<(), ValidationError>`
- `bootstrap_fallback_order(producers, slot) -> Vec<PublicKey>` — pre-epoch scheduling
- `bootstrap_schedule_with_liveness(...)` — liveness-aware bootstrap variant

### Conditions (`conditions/eval.rs`)
- `evaluate(condition, witness, ctx) -> Result<(), ConditionError>` — deterministic, bounded

### Output constructors (`transaction/output.rs`)
- `Output::normal(amount, pubkey_hash)`
- `Output::bond(amount, pubkey_hash, lock_until, creation_slot)` — extra_data = creation_slot (4B LE)
- `Output::nft(...)`, `Output::fungible_asset(...)`, `Output::pool(...)`
- `Output::encrypted_content(...)`, `Output::encrypted_content_v1(...)`
- `Output::oracle_price_address(pair_id) -> Hash` — deterministic system address `BLAKE3("ORACLE_PRICE" || pair_id)`

### Genesis (`genesis.rs`)
- `generate_genesis_block(config) -> Block` — `genesis.rs:189`
- `genesis_hash(network) -> Hash` — `genesis.rs:302`


## CONSTANTS

All from `crates/core/src/consensus/constants.rs` unless noted. Module split:
`bonds.rs, constants.rs, exit.rs, params.rs, producer_state.rs, registration.rs, reward_epoch.rs,
selection.rs, stress.rs, vdf.rs` (`consensus/mod.rs:62-75`).

**Protocol:**
- `INITIAL_PROTOCOL_VERSION: u32 = 1` — `constants.rs:7`
- `GENESIS_TIME: u64 = 1_783_532_348` — `constants.rs:26` — **CHANGED 2026-07-08**: fresh mainnet genesis
  reset (network loss recovery). Previously `1_776_837_510`. Must match `chainspec.mainnet.json`.
- `PROTOCOL_VERSION: u32 = 1` — `lib.rs:330`

**Time structure:**
- `SLOT_DURATION: u64 = 10` — seconds per slot
- `SLOTS_PER_EPOCH: u32 = 360` — 1 hour
- `SLOTS_PER_REWARD_EPOCH: u32 = 360`
- `BLOCKS_PER_REWARD_EPOCH: BlockHeight = 360`
- `SLOTS_PER_ERA: BlockHeight = 12_614_400` — ~4 years (alias `BLOCKS_PER_ERA`, `HALVING_INTERVAL`)
- `BOOTSTRAP_BLOCKS: BlockHeight = 60_480` — ~1 week
- `YEAR_IN_SLOTS: Slot = 3_153_600`
- `VESTING_QUARTER_SLOTS: Slot = 3_153_600` — 1 year (mainnet)
- `VESTING_PERIOD_SLOTS: Slot = 12_614_400` — 4 years full vest
- `UNDO_KEEP_DEPTH: u64 = 100` — undo-record retention depth (`constants.rs:259`); truncate_chain advertises this

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

**AMM Foundations economics (`constants.rs:337-368`, locked BEFORE `amm_activation_height` ever crosses):**
- `MINIMUM_LIQUIDITY: u64 = 1000` — permanently locked LP shares on first CreatePool deposit (D1, anti first-deposit-inflation)

**Fees:**
- `BASE_FEE: Amount = 1` — minimum 1 satoshi
- `FEE_PER_BYTE: Amount = 1`
- `FEE_DIVISOR: Amount = 100` — effective rate = 0.01 sat/byte

**Block sizes:**
- `BASE_BLOCK_SIZE: usize = 2_000_000` — 2 MB era 0 (validation cap, unchanged)
- `MAX_BLOCK_SIZE_CAP: usize = 32_000_000` — 32 MB era 4+
- `BASE_EXTRA_DATA_SIZE: usize = 524_288` — 512 KB era 0
- `MAX_EXTRA_DATA_SIZE_CAP: usize = 8_388_608` — 8 MB era 4+
- `GOSSIP_ENVELOPE_MARGIN: usize = 64 * 1024` — gossipsub framing overhead margin, `constants.rs:473`

**Builder-policy block budgets (INC-I-091, NOT consensus — `constants.rs:487-495`):**
- `LEGACY_BLOCK_SELECT_BUDGET: usize = 1_000_000` — pre-`large_block_activation_height`
- `LEGACY_BLOCK_USER_DATA_BUDGET: usize = 1_048_576`
- `LARGE_BLOCK_SELECT_BUDGET: usize = 1_900_000` — post-activation, ~300 TPS
- `LARGE_BLOCK_USER_DATA_BUDGET: usize = 1_900_000`

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
- `MIN_ATTESTATION_MINUTES: usize = 30` — tier-PROMOTION threshold (out of 60/epoch). **DISTINCT** from the
  attestation-QUALIFICATION threshold below (`attestation.rs`) — do not conflate the two systems.

**Attestation (`attestation.rs`) — CORRECTED from prior skill version:**
- `ATTESTATION_MINUTES_PER_EPOCH: u32 = 60` — mainnet default constant
- `ATTESTATION_QUALIFICATION_THRESHOLD: u32 = 54` — **90% of 60**, mainnet default constant.
  Per-network: `attestation_qualification_threshold(blocks_per_epoch)` computes `90% of (blocks_per_epoch/6)`
  dynamically (testnet with 36 blocks/epoch → threshold 5 of 6 minutes). This is NOT the old
  `usize = 30` value from the pre-DeFi skill snapshot — that number belonged to the separate
  tier-promotion `MIN_ATTESTATION_MINUTES`.

**Ghost exclusion / epoch prune / floor:**
- `GHOST_EXCLUSION_GRACE_EPOCHS: u64 = 3`
- `GHOST_EXCLUSION_ACTIVATION_HEIGHT: u64 = u64::MAX` — **DEPRECATED**, use `NetworkParams`
- `MIN_PRODUCERS_FLOOR: usize = 3` — INC-I-116 absolute floor (post `epoch_prune_activation_height`), `constants.rs:155`

**Inactivity leak:**
- `INACTIVITY_LEAK_START: u64 = 360` — 1 epoch
- `INACTIVITY_LEAK_RATE: u64 = 10` — 10% per epoch
- `INACTIVITY_LEAK_FLOOR: u64 = 1`
- `LIVENESS_WINDOW_MIN: u64 = 500`
- `REENTRY_INTERVAL: u32 = 50`

**Phase 2.1 Oracle (`oracle/mod.rs`):**
- `SUNSET_THRESHOLD_BPS: u16 = 5500` — 55.00%, below this new attestations HALT
- `SUNSET_WARNING_BPS: u16 = 6000` — 60.00%, below this but ≥5500 → WARNING (no halt)
- `ORACLE_RECOVERY_EPOCHS: u64 = 4` — consecutive halted epochs before HaltPermanent (binary upgrade only)
- `PHASE_2_1_PAIR_STRING: &[u8] = b"DOLI/USD"` — single allowlisted pair (Phase 2.1)
- `STRUCTURAL_PUBKEY_HASHES_HEX: [&str; 12]` — N1-N12 mainnet structural producer pubkey-hashes (`consensus/constants.rs:721-734`)

**Finality (`finality.rs`):**
- `FINALITY_THRESHOLD_PCT: u32 = 67`
- `FINALITY_TIMEOUT_SLOTS: u32 = 3`

**Types (`types.rs`):**
- `DECIMALS: u32 = 8`
- `UNITS_PER_COIN: Amount = 100_000_000`

**Registration (`consensus/registration.rs`):**
- `BASE_REGISTRATION_FEE`, `MAX_REGISTRATION_FEE`, `MAX_REGISTRATIONS_PER_BLOCK`

**Maintainer (`maintainer/` — directory module since INC-I-172 M2: `mod`/`set`/`data`/`derivation`):**
- `INITIAL_MAINTAINER_COUNT`, `MAINTAINER_THRESHOLD`, `MAX_MAINTAINERS`, `MIN_MAINTAINERS`
- Gated on `NetworkParams::maintainer_derivation_activation_height` (mainnet `172_000`,
  testnet `127_200`, devnet `0`): distinct-signer k-of-n via `verify_multisig_at`, one-shot
  genesis seed, canonical order `(registered_at, pubkey_bytes)`, `ProtocolActivation`
  fail-close. Below the gate the pre-M2 entry-counting behavior is reproduced verbatim.
- `MaintainerSet::is_authorizable` (empty / zero-threshold refusal) is **UNGATED** — applies
  at every height.
- `derive_maintainer_set` is replay-complete but has **ZERO production callers**; the node
  seeds via `derive_canonical_maintainer_set` over the live `ProducerSet`
  (`bins/node/src/node/periodic.rs`). Do not claim a node replays governance history.


## ACTIVATION-HEIGHTS

**MAJOR DRIFT FROM PRIOR SKILL**: mainnet underwent a fresh genesis reset (2026-07-08, network loss
recovery — commits `61218e90`, `db05c2c5`). ALL mainnet legacy activation heights are now `0`
(active from genesis) EXCEPT the two below marked FROZEN. Old absolute-height values (13,320 /
14,000 / 27,547 / 37,500 / 44,246 / 71,290 / 197,800 / 254,344 etc.) from the pre-reset chain are
**STALE** — do not cite them for the current chain.

All hard-fork constants live in `consensus/constants.rs`; per-network values are `NetworkParams`
fields (`network_params/mod.rs`, defaults in `network_params/defaults.rs`). ALWAYS prefer
`NetworkParams` over the raw `consensus::` constant for network-aware code.

| NetworkParams field | Mainnet (fresh genesis) | Testnet | Devnet | What activates |
|---|---|---|---|---|
| `sig_verification_height` | 0 | 0 | 0 | Input.public_key mandatory |
| `inc_i_026_scheduler_activation_height` | 0 | 0 | 0 | pure slot%len scheduling |
| `fork_id_activation_height` | 0 | 0 | 0 | fork_id header enforcement |
| `full_bitfield_decode_height` | 0 | 0 | 0 | full [base+extra] bitfield decode |
| `rewards_epoch_list_fix_height` | 0 | 0 | 0 | epoch reward decode uses producer_list |
| `encrypted_content_activation_height` | 0 | 0 | 0 | NFT plaintext rejected, EncryptedContent only |
| `encrypted_content_v2_activation_height` | 0 | 0 | 0 | MIME + royalties metadata |
| `epoch_state_reorg_activation_height` | 0 | 0 | 0 | reorg restores epoch_state from undo |
| `security_audit_activation_height` | 0 | 0 | 0 | all 2026-04-24 audit fixes bundled |
| `ghost_exclusion_activation_height` | 0 | 0 | 0 | ghost producers excluded from floor |
| `epoch_prune_activation_height` | 0 | 0 | 0 | INC-I-116 absolute `MIN_PRODUCERS_FLOOR=3` |
| `inc_i_068_weight_filter_activation_height` | 0 | 0 | 0 | selection_weight==0 producers filtered out |
| `received_delegation_cap` / `_activation_height` | 3000 / 0 | 3000 / 0 | u64::MAX / u64::MAX | INC-I-078 delegation concentration cap |
| `delegation_auth_activation_height` | 0 | 0 | u64::MAX | DelegateBond/RevokeDelegation Ed25519 auth |
| `addbond_cap_enforcement_activation_height` | 0 | 0 | u64::MAX | AddBond over-cap REJECTED (not silently clipped) |
| `defi_activation_height` | **0** (operator directive; 7 tx types tombstoned, unreachable) | u64::MAX | u64::MAX | non-AMM DeFi gate (dead — no reachable path) |
| `amm_activation_height` | **0** | **0** | 0 | CreatePool/AddLiquidity/RemoveLiquidity/Swap valid |
| `inc_i_092_activation_height` | **0** | **0** | 0 | Pool-input sig exemption (RC-A) + CreatePool funding (RC-B) |
| `inc_i_096_activation_height` | **0** | **0** | 0 | Pool-aware value conservation |
| `large_block_activation_height` | **0** | **0** | 0 | ~2MB builder budget, ~300 TPS (builder policy, not consensus) |
| `oracle_activation_height` | **u64::MAX — FROZEN** | **u64::MAX — FROZEN** | u64::MAX | Phase 2.1 PriceAttestation (TxType=16) — code shipped M1-M11 but gate NEVER pinned; separate decision-session required (HC-6/INC-I-075) |

**IMMUTABILITY (INC-I-054)**: once any height above is crossed AND honored by a deployed binary on
mainnet, it is IMMUTABLE — never move forward. The fresh genesis reset does not violate this: the
prior mainnet chain (with its own crossed heights) was abandoned entirely, not retroactively edited.

**INV-DEPLOY-002** (`network_params/mod.rs:538-573`): `inc_i_096_activation_height` MUST be
`<= amm_activation_height` on every network except grandfathered `Testnet` (historical exception,
naive conservation rejected — never drained — pre-fix). Enforced by `debug_assert!` inside
`NetworkParams::load()`.


## DEPENDENCIES

Internal crates used by `crates/core`:
- `crypto` — `Hash`, `PublicKey`, `PrivateKey`, `Signature`, BLS types, BLAKE3 hasher, `hash_with_domain`
- `vdf` — `VdfOutput`, `VdfProof`, `block_input()`
- `serde` + `bincode` — serialization (bincode for wire, serde for JSON RPC)
- `thiserror` — `ValidationError`, `AttestationError`, etc.
- `proptest` — property-based tests in `types.rs`

External consumers of `crates/core`:
- `bins/node` — uses all validation + epoch_state + scheduler + oracle epoch-boundary orchestrator (`apply_block/oracle.rs`, out of this domain)
- `crates/storage` — uses Block, Transaction, Output, Hash types, UtxoSet canonical serialization (incl. OutputType=15 OraclePrice)
- `crates/network` — uses Block, Transaction, ValidationError
- `crates/rpc` — uses block/tx types, ValidationContext; `crates/rpc/src/methods/oracle.rs` + `oracle_status.rs` (M9-M11) read oracle types from this crate
- `crates/mempool` — uses Transaction, validate_transaction, mirrors AMM/oracle activation-height gates for pre-inclusion admission
- `crates/updater` — reads `CURRENT_PROTOCOL_VERSION`-adjacent constants for HardForkSchedule (constant gates preferred over schedule entries per CLAUDE.md)


## CONSTRAINTS

**Immutable invariants — violating these forks the chain:**

1. **Bitfield encoder/decoder must use identical order**: `[epoch_state.producer_list | extra_sorted_by_pubkey]`. Any mismatch = wrong reward indices = permanent fork.

2. **CURRENT_PROTOCOL_VERSION must NOT be bumped** unless `EpochState` serialization format changes. Bumping triggers `delete_epoch_state()` on every node restart → non-deterministic rebuild → fork. Use `EPOCH_STATE_FORMAT_VERSION` for struct changes.

3. **Activation heights are IMMUTABLE** once crossed AND honored by a deployed binary on mainnet. Never move an activated height forward (higher). New features get their OWN height — see the AMM/oracle/inc_i_092/inc_i_096 quartet, each independently gated (HC-6 / INC-I-075 NEVER-bundle rule).

4. **EpochState::derive_at_boundary is the ONE canonical function** for epoch transitions; `compute_live_producer_list` is its shared floor/filter logic (also used by `rewards.rs`). No alternative implementations. Node-local state (excluded_producers etc.) MUST NOT be used as scheduling input.

5. **Bond extra_data stamped by node**: CLI sends `creation_slot=0`, node stamps real slot at `apply_block()`. Never trust raw tx extra_data for bond creation_slot.

6. **Coinbase always goes to reward_pool_pubkey_hash** (deterministic address, no private key). Only epoch boundary EpochReward tx drains it.

7. **EncryptedContent is NOT conditioned**: `OutputType::is_conditioned()` returns false for EncryptedContent. Its extra_data uses `[ct_len | ciphertext | wrapped_key | nonce | content_hash]` layout, not condition-prefixed. (AUDIT-NFT-001 fix.)

8. **HardForkSchedule NEVER used for rolling deploys**: `current_fork_id()` uses `u64::MAX`, making ALL schedule entries active immediately in fork_id. Use constant gates or NetworkParams activation heights instead.

9. **Producer mutations deferred to epoch boundary** (except epoch 0 and maintainer changes, which are immediate).

10. **Output::is_native_amount()** must be checked before summing amounts — Pool, LPShare, FungibleAsset, ZKRollup store non-DOLI (or zero) values in `amount`. NOTE: `Collateral` and `LendingDeposit` (old discriminants 11,12) are TOMBSTONED — no longer valid OutputType variants; `is_native_amount()` no longer references them.

11. **TxType/OutputType discriminant gaps are PERMANENT tombstones, never reuse**: TxType 23 (reserved, never assigned), 24-28 (native lending, B.1), 29-30 (NFT fractionalization, B.2). OutputType 11-12 (Collateral, LendingDeposit, B.1). `from_u32`/`from_u8` return `None` for these — reusing a tombstoned discriminant would let old un-upgraded nodes misinterpret new tx/output semantics as the old (removed) subsystem.

12. **Phase 2.1 Oracle NEVER constraint**: `oracle_activation_height` MUST remain independent of `defi_activation_height` and `amm_activation_height` — never bundle, never reuse (HC-6). Same rule applies symmetrically among `amm_activation_height`, `inc_i_092_activation_height`, `inc_i_096_activation_height` — each is its own commit-able gate even when co-pinned to the same numeric height.

13. **INV-DEPLOY-002**: `inc_i_096_activation_height <= amm_activation_height` on every network except the grandfathered `Testnet` historical exception. Enforced by `debug_assert!` in `NetworkParams::load()` — violating this on a NEW network config would let AMM DOLI-outflow txs run under pre-fix (drainable) conservation.

14. **Pool UTXO signature exemption is height-gated, not universal**: below `inc_i_092_activation_height`, a Pool input still takes the legacy signature path and correctly FAILS (`PubkeyHashMismatch`) — this is intentional so a mixed pre/post-activation fleet does not fork. The exemption only applies at/after the gate.

15. **AMM `MINIMUM_LIQUIDITY=1000` must be set before ANY Pool UTXO is ever created on ANY network** — changing it after `amm_activation_height` crosses retroactively invalidates or under-secures existing pools.


## PATTERNS

### How to check if a feature is active
```rust
// PREFERRED (network-aware):
let params = NetworkParams::load(network);
if current_height >= params.security_audit_activation_height { ... }
if current_height >= params.amm_activation_height { ... }
if current_height >= params.oracle_activation_height { ... }   // frozen: always false today

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
    .with_amm_activation_height(params.amm_activation_height)
    .with_oracle_activation_height(params.oracle_activation_height)
    .with_oracle_sunset_triggered(node_tracked_sunset_bool)
    .with_inc_i_092_activation_height(params.inc_i_092_activation_height)
    .with_inc_i_096_activation_height(params.inc_i_096_activation_height)
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
    epoch_prune_activation_height: params.epoch_prune_activation_height,
};
let new_epoch_state = EpochState::derive_at_boundary(&prev_epoch_state, &input);
```

### How to run the Phase 2.1 oracle epoch-boundary aggregation (orchestrator lives in bins/node)
```rust
let contributions = dedupe_latest_per_attester(&raw_attestations_for_pair);
if let Some((median_cents, count)) = bond_weighted_median(&contributions, &bond_snapshot) {
    // write/consume OraclePrice UTXO at oracle_price_outpoint(&pair_id)
}
let share_bps = compute_structural_share_bps(&prev_epoch_bond_snapshot, &registered_at, height, blocks_per_epoch, &structural_hashes);
let health = sunset_state.transition(share_bps, epoch); // Healthy | Warning | HaltRecoverable | HaltPermanent
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

### TxType discriminants (24 constructible; gaps are PERMANENT tombstones — see CONSTRAINTS #11)
```
0=Transfer, 1=Registration, 2=Exit, 3=ClaimReward, 4=ClaimBond, 5=SlashProducer,
6=Coinbase, 7=AddBond, 8=RequestWithdrawal, 9=ClaimWithdrawal(tombstone-but-decodable),
10=EpochReward, 11=RemoveMaintainer, 12=AddMaintainer, 13=DelegateBond,
14=RevokeDelegation, 15=ProtocolActivation, 16=PriceAttestation (Phase 2.1 oracle, frozen),
17=MintAsset, 18=BurnAsset, 19=CreatePool, 20=AddLiquidity, 21=RemoveLiquidity, 22=Swap,
23=(reserved, never assigned), 24-28=TOMBSTONED (native lending, B.1),
29-30=TOMBSTONED (NFT fractionalization, B.2), 31=ZKSettle
```

### OutputType discriminants (14 constructible; 11-12 PERMANENTLY tombstoned — B.1 lending removal)
```
0=Normal, 1=Bond, 2=Multisig, 3=Hashlock, 4=HTLC, 5=Vesting,
6=NFT, 7=FungibleAsset, 8=BridgeHTLC, 9=Pool, 10=LPShare,
11-12=TOMBSTONED (was Collateral, LendingDeposit — B.1 native lending removal),
13=ZKRollup, 14=EncryptedContent, 15=OraclePrice (Phase 2.1, system-only, amount always 0)
Conditioned (uses condition-prefix in extra_data): 2,3,4,5,6,7,8,10(LPShare)
NOT conditioned: EncryptedContent(14), OraclePrice(15) — system/signature paths only
```

### Network defaults (devnet overrides for fast testing)
```
Mainnet/Testnet: SLOT_DURATION=10s, SLOTS_PER_EPOCH=360, BOND_UNIT=10 DOLI (testnet: 1 DOLI)
Devnet: slot_duration=10s (same), blocks_per_year=144 (~24min), blocks_per_reward_epoch=4 (~40s), bond_unit=1 DOLI
```

### Validation modes summary
- `Full` — VDF proof verified (gossip blocks, tip of sync)
- `Light` — VDF skipped (gap blocks after snap-sync, state root trusted by quorum)
- `Replay` — disaster recovery, also skips dedup + recovery gate + snap height guard

### AMM integer math pattern (no floating point, ever)
```rust
// x*y=k with basis-point fee, all u128 intermediates:
let dx_eff = (dx as u128) * (10_000 - fee_bps as u128) / 10_000;
let dy = ((reserve_b as u128) * dx_eff / ((reserve_a as u128) + dx_eff)) as Amount;
```
