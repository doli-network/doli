# Task Corrections (English Translation)

## crates/core/src/types.rs
- **BlockHeader** → Ensure `vdf_output` field is mandatory (not optional in mainnet)
- **BlockHeader** → If `vdf_proof` exists, mark it as `None`/unused in v1

## crates/core/src/consensus.rs
- **GENESIS_TIMESTAMP** → 1769904000
- **SLOT_DURATION** → 10
- **SLOTS_PER_EPOCH** → 360
- **SLOTS_PER_ERA** → 12,614,400
- **SLOTS_PER_YEAR** → 3,153,600 (add)
- **UNBONDING_PERIOD** → 259,200
- **INACTIVITY_THRESHOLD** → 50
- **COINBASE_MATURITY** → 100 (add)
- **T_BLOCK** → 10,000,000
- **INITIAL_BLOCK_REWARD** → 100,000,000
- **HALVING_INTERVAL** → 12,614,400
- **TOTAL_SUPPLY** → 25,228,800 * 100,000,000 (add)
- **MAX_DRIFT** → Define clear window in seconds consistent with slot=10s
- **MAX_FUTURE_SLOTS** → Adjust according to DRIFT
- **Fallback windows** → 3,000, 6,000, 10,000 ms
- Remove `select_producers_for_slot()` and `select_producers_for_slot_weighted()`
- Add `select_producer_for_slot()` with consecutive tickets
- **block_reward()** → Return 0 if `total_minted >= TOTAL_SUPPLY`
- **construct_vdf_input()** → Add function: `HASH(prefix || prev_hash || tx_root || slot || producer_key)`
- **Registration smoothing** → Define fixed α (e.g., 0.2) and deterministic EWMA formula
- **R_TARGET** → Define target registrations per epoch

## crates/core/src/validation.rs
- **validate_producer_eligibility()** → Use `select_producer_for_slot()` without `prev_hash`
- **Update imports** → Remove old functions, import new one
- **validate_vdf()** → Use fixed `T_BLOCK`, reject if `vdf_output` missing/zeroed in mainnet
- **validate_block_header_time()** → Verify:
  - `timestamp <= network_time + DRIFT`
  - `slot = floor((timestamp - GENESIS) / 10)`
  - `slot > prev_block.slot`
  - Reject if timestamp outside window
- **validate_coinbase()** → Add check: `total_minted + reward <= TOTAL_SUPPLY`
- Add **validate_block_with_store()** → New API that receives `BlockStore`
- Add **validate_slash_evidence_full()** → Verify with signed headers (don't depend on local cache)
- **validate_registration()** → Remove global chain
- **validate_registration()** → Check state: pubkey not duplicated
- **validate_registration()** → Use deterministic smoothing with EWMA
- **is_rank_eligible_at_offset()** → Windows 0-3s/3-6s/6-10s
- Remove any use of presence score in validity conditions
- Remove any use of "presence threshold" in selection

## crates/core/src/genesis.rs
- **GENESIS_MESSAGE** → "Time is the only fair currency. 01/Feb/2026"
- **GENESIS_REWARD** → 100,000,000

## crates/core/src/types.rs
- **Update comments** → Match with consensus.rs
- **BlockHeader.vdf_output** → Mandatory in mainnet

## crates/core/src/state.rs or crates/core/src/ledger.rs
- **total_minted** → Track total minted
- **apply_coinbase()** → Enforce `total_minted + reward <= TOTAL_SUPPLY`
- **registrations_count_epoch[E]** → Store per epoch
- **smoothed_demand_epoch[E]** → Store deterministic EWMA per epoch

## crates/core/src/tpop/presence.rs
- **Completely isolate** → No exported function should affect validity or selection
- **Mark as telemetry** → Metrics only, not consensus

## crates/vdf/src/lib.rs
- **T_BLOCK** → 10,000,000
- **T_REGISTER_BASE** → 600,000,000
- **T_REGISTER_CAP** → 86,400,000,000

## crates/storage/src/producer.rs
- **seniority_weight()** → Use `SLOTS_PER_YEAR` for years
- **Weights** → 0-1=1, 1-2=2, 2-3=3, 3+=4
- Add storage for `registrations_count_epoch[E]`
- Add storage for `smoothed_demand_epoch[E]`

## crates/network/src/sync/reorg.rs
- **Fork choice** → Use updated `seniority_weight()`

## crates/network/src/sync/equivocation.rs
- **Slash evidence** → Include complete signed headers in the evidence (don't depend on local store)
- **Determinism** → All data needed to verify must be in the evidence itself

## bins/node/src/node.rs
- Remove `deterministic_producer_selection()`
- Use `select_producer_for_slot()` from core
- **VDF production** → Compute `SHA256^T_BLOCK(input)` and set `header.vdf_output`
- **Slashing** → Call `validate_block_with_store()` before accepting
- **Units bug** → Change `1_000_000_000` to `UNITS_PER_COIN`
- Remove any use of presence score in production/selection

## specs/protocol.md
- **Deprecate or update** → Match with whitepaper

## Tests
- `constants_match_whitepaper`
- `selection_independent_of_prev_hash`
- `selection_uses_consecutive_tickets`
- `vdf_required_mainnet`
- `vdf_output_mandatory` → Block without `vdf_output` rejected
- `forged_slash_rejected`
- `real_equivocation_slashed`
- `slash_deterministic` → Same result regardless of local cache
- `supply_converges`
- `supply_never_exceeds_cap` → `total_minted` never > `TOTAL_SUPPLY`
- `fallback_windows`
- `seniority_uses_years`
- `timestamp_drift_enforced` → Block with timestamp outside window rejected
- `registration_smoothing_deterministic` → Same `T_registration` given same state
- `presence_not_in_consensus` → Changing presence score doesn't change validity