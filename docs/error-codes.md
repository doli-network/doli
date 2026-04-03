# DOLI Error Code Registry

Error codes for programmatic error parsing by agents and automated tooling.

## Conventions

- Error codes are `UPPER_SNAKE_CASE` in square brackets: `[DOMAIN_CODE]`
- Codes appear as the first token in `anyhow::bail!` messages
- Structured `thiserror` variants use typed fields instead of codes
- RPC errors use JSON-RPC numeric codes (standard + custom)

---

## ECON -- Block Economics Validation

Used in `bins/node/src/node/validation_checks.rs::validate_block_economics()`.

| Code | Description | Severity |
|------|-------------|----------|
| `ECON_PRODUCER` | Block producer not in active set or GSet | Reject block |
| `ECON_COINBASE_MISSING` | Block has no transactions (missing coinbase) | Reject block |
| `ECON_COINBASE_INVALID` | First transaction is not a valid coinbase | Reject block |
| `ECON_COINBASE_AMOUNT` | Coinbase amount does not match expected block reward | Reject block |
| `ECON_COINBASE_RECIPIENT` | Coinbase recipient is not the reward pool address | Reject block (theft attempt) |
| `ECON_EPOCH_NOT_BOUNDARY` | EpochReward transaction at non-epoch-boundary height | Reject block |
| `ECON_EPOCH_ZERO` | EpochReward at epoch 0 (genesis pool used for bonds) | Reject block |
| `ECON_EPOCH_DUPLICATE` | More than one EpochReward TX in a single block | Reject block |
| `ECON_EPOCH_EXTRA_DATA` | EpochReward extra_data field is too short (<16 bytes) | Reject block |
| `ECON_EPOCH_HEIGHT` | EpochReward embedded height does not match block height | Reject block |
| `ECON_EPOCH_NUMBER` | EpochReward embedded epoch does not match completed epoch | Reject block |
| `ECON_EPOCH_OVERFLOW` | EpochReward total distributed exceeds pool balance | Reject block (inflation) |
| `ECON_EPOCH_DISTRIBUTION` | EpochReward output amounts/recipients don't match expected | Reject block |
| `ECON_EPOCH_NO_INPUTS` | Post-activation EpochReward missing explicit pool inputs | Reject block |
| `ECON_EPOCH_INPUTS_MISMATCH` | EpochReward pool inputs don't match expected outpoints | Reject block |
| `ECON_EPOCH_PRE_INPUTS` | Pre-activation EpochReward should not have inputs | Reject block |
| `ECON_EPOCH_MISSING` | Epoch boundary block is missing required EpochReward TX | Reject block |

## FORK -- Fork Recovery

Used in `bins/node/src/node/fork_recovery.rs`.

| Code | Description | Severity |
|------|-------------|----------|
| `FORK_INVALID_PRODUCER` | Cached fork block has invalid producer after scheduler rebuild | Reject chain |
| `FORK_CHAIN_INCOMPLETE` | Cannot build complete chain from cached blocks | Fall back to sync |

## RPC -- JSON-RPC Error Codes

Used in `crates/rpc/src/error.rs`. Standard JSON-RPC codes plus custom application codes.

| Code | Constant | Description |
|------|----------|-------------|
| `-32700` | `PARSE_ERROR` | Invalid JSON received |
| `-32600` | `INVALID_REQUEST` | JSON is not a valid request object |
| `-32601` | `METHOD_NOT_FOUND` | Requested method does not exist |
| `-32602` | `INVALID_PARAMS` | Invalid method parameters |
| `-32603` | `INTERNAL_ERROR` | Internal server error |
| `-32000` | `BLOCK_NOT_FOUND` | Block not found (may include `data.searched_by`, `data.hash`, `data.height`) |
| `-32001` | `TX_NOT_FOUND` | Transaction not found (may include `data.searched_by`, `data.hash`) |
| `-32002` | `INVALID_TX` | Transaction validation failed |
| `-32003` | `TX_ALREADY_KNOWN` | Transaction already in mempool |
| `-32004` | `MEMPOOL_FULL` | Mempool is full |
| `-32005` | `UTXO_NOT_FOUND` | UTXO not found |
| `-32006` | `PRODUCER_NOT_FOUND` | Producer not found |
| `-32007` | `POOL_NOT_FOUND` | Pool not found |
| `-32008` | `UNAUTHORIZED` | Admin token required |

## ERRTX -- InvalidTransaction Errors

Used in `ValidationError::InvalidTransaction(String)` across `crates/core/src/validation/` and `bins/node/src/node/validation_checks.rs`. These are structural transaction validation failures.

### Structural (transaction.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX001` | Transaction must have inputs | `tx_type` |
| `ERRTX002` | Transaction must have outputs | `tx_type` |

### Output Validation (transaction.rs::validate_outputs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX003` | Output has zero amount | `output_index`, `output_type` |
| `ERRTX004` | Output extra_data exceeds max size | `output_index`, `actual_size`, `max_size`, `height` |
| `ERRTX005` | Normal output has non-zero lock_until | `output_index`, `lock_until` |
| `ERRTX006` | Normal output has non-empty extra_data | `output_index`, `byte_count` |
| `ERRTX007` | Bond output has wrong extra_data size (expected 4) | `output_index`, `actual_size` |
| `ERRTX008` | Conditioned output rejected (covenants not activated) | `output_index`, `output_type`, `activation_height`, `current_height` |
| `ERRTX009` | Conditioned output has empty extra_data | `output_index`, `output_type` |
| `ERRTX010` | Conditioned output has invalid condition | `output_index`, `output_type`, `decode_error` |
| `ERRTX011` | NFT output rejected (covenants not activated) | `output_index`, `activation_height`, `current_height` |
| `ERRTX012` | NFT output has empty extra_data | `output_index` |
| `ERRTX013` | NFT output has invalid condition | `output_index`, `decode_error` |
| `ERRTX014` | NFT output has invalid or missing metadata | `output_index`, `byte_count` |
| `ERRTX015` | Duplicate NFT token_id in same transaction | `output_index` |
| `ERRTX016` | FungibleAsset output rejected (covenants not activated) | `output_index`, `activation_height`, `current_height` |
| `ERRTX017` | FungibleAsset output has empty extra_data | `output_index` |
| `ERRTX018` | FungibleAsset output has invalid condition | `output_index`, `decode_error` |
| `ERRTX019` | FungibleAsset output has invalid or missing asset metadata | `output_index`, `byte_count` |
| `ERRTX020` | BridgeHTLC output rejected (covenants not activated) | `output_index`, `activation_height`, `current_height` |
| `ERRTX021` | BridgeHTLC output has empty extra_data | `output_index` |
| `ERRTX022` | BridgeHTLC output has invalid condition | `output_index`, `decode_error` |
| `ERRTX023` | BridgeHTLC output has invalid or missing bridge metadata | `output_index`, `byte_count` |
| `ERRTX024` | Pool output has invalid extra_data size | `output_index`, `actual_size`, `min_size` |
| `ERRTX025` | Pool output has invalid or undecodable metadata | `output_index`, `byte_count` |
| `ERRTX026` | LPShare output has invalid extra_data size | `output_index`, `actual_size`, `min_size` |
| `ERRTX027` | LPShare output has invalid metadata | `output_index`, `byte_count` |
| `ERRTX028` | Collateral output has invalid extra_data size | `output_index`, `actual_size`, `min_size` |
| `ERRTX029` | Collateral output has invalid or undecodable metadata | `output_index`, `byte_count` |
| `ERRTX030` | LendingDeposit output has invalid extra_data size | `output_index`, `actual_size`, `min_size` |
| `ERRTX031` | LendingDeposit output has invalid or undecodable metadata | `output_index`, `byte_count` |
| `ERRTX032` | Output has zero pubkey_hash | `output_index`, `output_type`, `amount` |

### UTXO Validation (utxo.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX033` | EpochReward input is not a pool UTXO | `input_index`, `pubkey_hash` |
| `ERRTX034` | Input has committed_output_count but sighash is not AnyoneCanPay | `input_index`, `committed_output_count`, `sighash_type` |
| `ERRTX035` | Input committed_output_count exceeds output count | `input_index`, `committed_output_count`, `output_count` |
| `ERRTX036` | Input is a fractionalized NFT (can only be spent by RedeemNft) | `input_index`, `tx_type` |
| `ERRTX037` | NFT input requires royalty payment to creator | `input_index`, `required_royalty`, `actual_royalty`, `sale_price`, `royalty_bps` |
| `ERRTX038` | Input references output with invalid condition | `input_index`, `decode_error` |
| `ERRTX039` | Input condition exceeds ops limit | `input_index`, `ops_count`, `max_ops` |
| `ERRTX040` | Input has invalid witness data | `input_index`, `decode_error` |

### Exit Transaction (tx_types.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX041` | Exit transaction must have no inputs | -- |
| `ERRTX042` | Exit transaction must have no outputs | -- |
| `ERRTX043` | Missing exit data in extra_data | -- |
| `ERRTX044` | Invalid exit data (deserialization failed) | `decode_error` |

### Lending (lending.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX045` | CreateLoan requires at least one input | -- |
| `ERRTX046` | CreateLoan requires at least 2 outputs | `output_count` |
| `ERRTX047` | CreateLoan first output must be Collateral type | `actual_type` |
| `ERRTX048` | CreateLoan invalid collateral metadata in output 0 | `byte_count` |
| `ERRTX049` | CreateLoan principal must be positive | -- |
| `ERRTX050` | CreateLoan collateral amount must be positive | -- |
| `ERRTX051` | CreateLoan interest rate exceeds maximum | `rate_bps`, `max_bps` |
| `ERRTX052` | CreateLoan liquidation ratio below minimum | `ratio_bps`, `min_bps` |
| `ERRTX053` | CreateLoan second output must be Normal type | `actual_type` |
| `ERRTX054` | CreateLoan output[1] amount must equal principal | `actual_amount`, `principal` |
| `ERRTX055` | CreateLoan change output has wrong type | `output_index`, `actual_type` |
| `ERRTX056` | RepayLoan requires at least 2 inputs | `input_count` |
| `ERRTX057` | RepayLoan requires at least 1 output | -- |
| `ERRTX058` | LiquidateLoan requires at least 1 input | -- |
| `ERRTX059` | LiquidateLoan requires at least 1 output | -- |
| `ERRTX060` | LendingDeposit requires at least one input | -- |
| `ERRTX061` | LendingDeposit requires at least one output | -- |
| `ERRTX062` | LendingDeposit first output must be LendingDeposit type | `actual_type` |
| `ERRTX063` | LendingDeposit invalid metadata in output 0 | `byte_count` |
| `ERRTX064` | LendingDeposit deposit amount must be positive | -- |
| `ERRTX065` | LendingDeposit change output has wrong type | `output_index`, `actual_type` |
| `ERRTX066` | LendingWithdraw requires at least 1 input | -- |
| `ERRTX067` | LendingWithdraw requires at least 1 output | -- |

### Block Validation (validation_checks.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `ERRTX068` | missed_producers exceeds max entries per block | `count`, `max` |
| `ERRTX069` | missed_producers contains key not in epoch producer list | `pubkey_prefix`, `list_size` |
| `ERRTX070` | missed_producers would exceed total exclusion cap | `total_after`, `max_total`, `currently_excluded` |

## MPTX -- Mempool Transaction Validation

Used in `MempoolError::InvalidTransaction(String)` in `crates/mempool/src/pool.rs`. Pre-block mempool admission checks.

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `MPTX001` | Input missing public_key | `input_index`, `sig_height` |
| `MPTX002` | Input pubkey hash mismatch | `input_index`, `expected_hash`, `actual_hash` |
| `MPTX003` | Input invalid signature | `input_index` |
| `MPTX004` | Input has invalid condition | `input_index`, `decode_error` |
| `MPTX005` | Input condition exceeds ops limit | `input_index`, `ops_count`, `max_ops` |
| `MPTX006` | Input has invalid witness | `input_index`, `decode_error` |
| `MPTX007` | Input covenant condition not satisfied | `input_index` |
| `MPTX008` | Insufficient funds (input < output) | `input_total`, `output_total`, `deficit` |
| `MPTX009` | Output not spendable (locked/immature) | `spendable_height`, `current_height`, `maturity` |

## STOR -- Storage Layer Errors

Used in `StorageError` variants (`NotFound`, `AlreadyExists`, `Serialization`) across `crates/storage/src/`.

### Producer Set (producer/)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `STOR001` | Producer not found for exit request | `pubkey` |
| `STOR002` | Producer already in unbonding | `pubkey` |
| `STOR003` | Producer already exited | `pubkey` |
| `STOR004` | Producer was slashed | `pubkey` |
| `STOR005` | Producer not found for cancel_exit | `pubkey` |
| `STOR006` | Cannot cancel exit -- producer already active | `pubkey` |
| `STOR007` | Cannot cancel exit -- producer already exited | `pubkey` |
| `STOR008` | Cannot cancel exit -- producer was slashed | `pubkey` |
| `STOR009` | Producer not found for slash | `pubkey` |
| `STOR010` | Producer not found for renew | `pubkey` |
| `STOR011` | Producer already registered | `pubkey`, `status` |
| `STOR012` | Producer already registered for network | `pubkey`, `status` |
| `STOR013` | Genesis producer already registered | `pubkey` |

### UTXO Store (utxo_rocks.rs, state_db/, utxo/in_memory.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `STOR014` | UTXO not found in RocksDB store | `tx_hash`, `output_index` |
| `STOR015` | UTXO deserialize failed in RocksDB store | `tx_hash`, `output_index`, `error` |
| `STOR016` | UTXO not found in state_db | `tx_hash`, `output_index` |
| `STOR017` | UTXO deserialize failed in state_db | `tx_hash`, `output_index`, `error` |
| `STOR018` | UTXO not found in batch (same-block spend) | `tx_hash`, `output_index` |
| `STOR019` | UTXO deserialize failed in batch | `tx_hash`, `output_index`, `error` |
| `STOR026` | UTXO not found in memory store | `tx_hash`, `output_index` |

### Block Store (block_store/)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `STOR020` | Header missing during chain walk | `hash` |
| `STOR021` | Header missing during reindex | `hash` |
| `STOR022` | Invalid height length in DB | `actual_bytes` |
| `STOR023` | Invalid hash length in DB (height index) | `actual_bytes` |
| `STOR024` | Invalid tx_index height length | `actual_bytes` |
| `STOR025` | Invalid slot hash length | `actual_bytes` |

### Canonical UTXO Serialization (utxo/set.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `STOR027` | Canonical bytes too short for header | `actual_bytes` |
| `STOR028` | Canonical bytes truncated at outpoint | `pos`, `len` |
| `STOR029` | Invalid outpoint in canonical bytes | `pos` |
| `STOR030` | Canonical bytes truncated at entry | `pos`, `len` |
| `STOR031` | Canonical bytes truncated at extra_data | `pos`, `entry_size`, `len` |
| `STOR032` | Invalid UTXO entry in canonical bytes | `pos`, `entry_size` |

### Snapshot (snapshot.rs)

| Code | Description | Context Variables |
|------|-------------|-------------------|
| `STOR033` | ChainState deserialization failed | `byte_count`, `error` |
| `STOR034` | ProducerSet deserialization failed | `byte_count`, `error` |
| `STOR035` | UtxoSet deserialization failed | `byte_count`, `error` |
