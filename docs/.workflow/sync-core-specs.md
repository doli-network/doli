# Core Specs Drift -- 2026-03-29 (Verification Pass)

This is an adversarial verification pass against the three core spec files.
All findings are confirmed by reading the actual source code.

## specs/protocol.md

### P1: DelegateBond transaction (section 3.18) - WRONG inputs/outputs
- **Spec says**: `inputs: [{...}]` and `outputs: [{...}]` with "Must spend from delegator's address" and "Change output"
- **Code says**: DelegateBond is state-only (`is_state_only()` returns true). `new_delegate_bond()` creates tx with `inputs: Vec::new()`, `outputs: Vec::new()`.
- **Fix**: Change to empty inputs/outputs, mark as state-only operation. **FIXED**

### P2: RevokeDelegation transaction (section 3.19) - WRONG inputs/outputs
- **Spec says**: `inputs: [{...}]` and `outputs: [{...}]`
- **Code says**: RevokeDelegation is state-only. `new_revoke_delegation()` creates tx with `inputs: Vec::new()`, `outputs: Vec::new()`.
- **Fix**: Change to empty inputs/outputs, mark as state-only operation. **FIXED**

### P3: WithdrawalRequest extra_data (section 3.13) - INCOMPLETE
- **Spec says**: `extra_data: { producer_pubkey: 32 bytes }`
- **Code says**: `WithdrawalRequestData { producer_pubkey: PublicKey, bond_count: u32, destination: Hash }` -- extra_data includes all 3 fields.
- **Fix**: Add `bond_count` and `destination` to the spec's extra_data definition. **FIXED**

## specs/architecture.md

### A1: Storage Layer section missing cf_undo
- **Spec says** (Storage Layer section, line ~183): Lists only 5 CFs (cf_utxo, cf_utxo_by_pubkey, cf_producers, cf_exit_history, cf_meta)
- **Code says**: StateDb opens 6 CFs including cf_undo (used for rollback undo data)
- **Dependencies section** (line 972) correctly lists 6 CFs including cf_undo.
- **Fix**: Add cf_undo to the Storage Layer section. **FIXED**

## specs/security_model.md

### S1: RegistrationData struct missing bls fields (section 6.2.1)
- **Spec says**: 7-field struct (public_key through bond_count)
- **Code says**: Struct has 9 fields including `bls_pubkey: Vec<u8>` and `bls_pop: Vec<u8>` (both `#[serde(default)]` optional)
- **Fix**: Add bls_pubkey and bls_pop fields to the spec. **FIXED**

## Verified Correct (No Drift)

### specs/protocol.md
- Transaction types 0-28: all type IDs, names, and gaps (16, 23) match code exactly
- Output types 0-12: all match code exactly (13 total)
- Input struct: all 5 fields match (prev_tx_hash, output_index, signature, sighash_type, committed_output_count)
- SighashType enum: All=0, AnyoneCanPay=1 -- correct
- Block header fields: all 9 fields match code (including genesis_hash)
- Block body: aggregate_bls_signature field present -- correct
- Block hash formula: includes presence_root and genesis_hash, excludes vdf_proof -- correct
- VDF input: HASH("DOLI_VDF_BLOCK_V1" || prev_hash || merkle_root || slot || producer) -- correct
- All consensus constants: GENESIS_TIME, SLOT_DURATION, SLOTS_PER_EPOCH, SLOTS_PER_ERA, BOOTSTRAP_BLOCKS -- all correct
- INITIAL_REWARD = 100,000,000 -- correct
- BOND_UNIT = 1,000,000,000 (10 DOLI) -- correct
- MAX_BONDS_PER_PRODUCER = 3,000 -- correct
- UNBONDING_PERIOD = 60,480 -- correct
- COINBASE_MATURITY = 6 -- correct
- BASE_BLOCK_SIZE = 2,000,000, MAX_BLOCK_SIZE_CAP = 32,000,000 -- correct
- TOTAL_SUPPLY = 2,522,880,000,000,000 -- correct
- EXCLUSION_SLOTS = 60,480 -- correct
- MAX_FAILURES = 50 -- correct
- BASE_FEE = 1, FEE_PER_BYTE = 1 -- correct
- FALLBACK_TIMEOUT_MS = 2,000, MAX_FALLBACK_RANKS = 2 -- correct
- MAX_DRIFT = 1, MAX_DRIFT_MS = 200 -- correct
- Vesting schedule (4-year, year-based quarters) -- correct
- Testnet vesting_quarter_slots = 2,160 -- correct
- Network parameters table (8.2): all values match NetworkParams::defaults() -- correct
- Bootstrap nodes: all 3 mainnet and 3 testnet entries match -- correct
- Registration fee: BASE_REGISTRATION_FEE = 100,000, MAX_FEE_MULTIPLIER_X100 = 1000 -- correct
- T_REGISTER_BASE = 1,000, T_REGISTER_CAP = 5,000,000 -- correct
- T_BLOCK = 800,000 -- correct
- VDF iterations all networks = 1,000 (devnet = 1) -- correct
- RegistrationData struct (all 9 fields) -- correct
- Epoch reward distribution algorithm -- correct
- Producer selection (slot % total_tickets) -- correct

### specs/architecture.md
- apply_block/ sub-modules: mod, state_update, tx_processing, governance, genesis_completion, post_commit (6) -- correct
- production/ sub-modules: mod, scheduling, assembly, gates (4) -- correct
- Crate dependency flow -- correct
- Workspace members in Cargo.toml -- matches (crates/*, bins/*, testing/benchmarks, testing/integration)

### specs/security_model.md
- T_BLOCK = 800,000 -- correct
- T_REGISTER_BASE = 1,000 -- correct
- T_REGISTER_CAP = 5,000,000 -- correct
- FALLBACK_TIMEOUT_MS = 2,000 -- correct
- MAX_FALLBACK_RANKS = 2 -- correct
- BOND_UNIT = 10 DOLI, MAX_BONDS = 3,000 -- correct
- Selection: slot % total_tickets (independent of prev_hash) -- correct
- Weight-based fork choice -- correct
- Slashing: double production only, 100% burn -- correct
- VDF input construction -- correct
- Domain tags -- correct
- Peer scoring thresholds -- correct (not re-verified against code this pass, verified 2026-03-29)

## Summary

| Spec File | Drift Items Found | Fixed |
|-----------|-------------------|-------|
| specs/protocol.md | 3 (P1, P2, P3) | 3 |
| specs/architecture.md | 1 (A1) | 1 |
| specs/security_model.md | 1 (S1) | 1 |
| **Total** | **5** | **5** |
