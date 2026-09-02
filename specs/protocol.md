<!-- OUTPUT CONTRACT: N/A — protocol specification document, not a test -->
<!-- INPUT PARTITIONS: N/A — protocol specification document -->

# DOLI Protocol Specification

This document provides the technical specification for implementing a DOLI-compatible node.

## Table of Contents

1. [Encoding Rules](#1-encoding-rules)
2. [Cryptographic Primitives](#2-cryptographic-primitives)
3. [Transactions](#3-transactions)
4. [Blocks](#4-blocks)
5. [Consensus Rules](#5-consensus-rules)
6. [Producer Registration](#6-producer-registration)
7. [Network Protocol](#7-network-protocol)
8. [Networks](#8-networks)
9. [Test Vectors](#9-test-vectors)
10. [Auto-Update System](#10-auto-update-system)

---

## 1. Encoding Rules

### 1.1 Integers

All integers are encoded in **little-endian** format.

| Type   | Size    | Usage                           |
|--------|---------|----------------------------------|
| uint32 | 4 bytes | slot, epoch, index, version, type |
| uint64 | 8 bytes | amount, timestamp                |

**Example**: Slot 1000 = `0xE8030000`

### 1.2 Byte Strings

Byte strings are concatenated directly without length prefixes or separators in hash preimages.

### 1.3 Literals

ASCII literals are encoded without NUL terminator:

| Literal | Bytes | Usage |
|---------|-------|-------|
| "DOLI_VDF_BLOCK_V1" | 17 bytes | Block VDF preimage |
| "DOLI_VDF_REGISTER_V1" | 20 bytes | Registration VDF preimage |
| "SEED" | `0x53 0x45 0x45 0x44` (4 bytes) | Selection seed |

### 1.4 Addresses

```
address = HASH(public_key)[0:20]   // First 20 bytes of hash
```

### 1.5 Serialization

Transactions and blocks are serialized using **bincode** format with the following configuration:
- Little-endian byte order
- Fixed-int encoding (no varint compression)
- No length limits

This matches the Rust `bincode` crate with `standard()` configuration.

---

## 2. Cryptographic Primitives

### 2.1 Hash Function

```
HASH(x) = BLAKE3-256(x)
```

Output: 32 bytes

### 2.2 Signatures

Algorithm: **Ed25519**

| Component   | Size     |
|-------------|----------|
| Private key | 32 bytes |
| Public key  | 32 bytes |
| Signature   | 64 bytes |

Signing message for transactions:
```
message = HASH(tx_without_signatures)
```

### 2.3 Verifiable Delay Function (Hash-Chain VDF)

Construction: **Iterated BLAKE3 hash chain**

DOLI uses a hash-chain VDF with dynamic calibration to maintain consistent timing across all networks:

| Parameter       | Consensus Constant | Network Default |
|-----------------|-------------------|-----------------|
| T_BLOCK         | 800,000           | 1,000 (all networks) |
| Target time     | ~55ms (at 800K)   | <1ms (at 1,000) |
| Output          | 32 bytes          | 32 bytes |
| Verification    | Recompute         | Recompute |

**Note:** Network defaults override VDF iterations to 1,000 (minimal). Bond staking is the primary Sybil defense, not VDF timing. The consensus constant T_BLOCK = 800,000 exists for potential future tightening via protocol upgrade.

```
VDF_compute(input, iterations) -> output
VDF_verify(input, output, iterations) -> bool  // Recomputes the chain
```

**Dynamic Calibration:**
A calibration module exists in the codebase (`tpop/calibration.rs`) but is not currently used for block production. Block iterations are set per network via `NetworkParams` (currently 1,000 for all networks). Calibration bounds: min 100,000, max 100,000,000 iterations.

**Note**: Grinding prevention comes from Epoch Lookahead (deterministic leader selection), not VDF timing. Bond staking is the primary Sybil defense.

---

## 3. Transactions

### 3.1 Transaction Structure

```
transaction = {
    version:    uint32,          // Currently 1
    type:       uint32,          // 0 = transfer, 1 = registration, 2 = exit,
                                 // 3 = claim_reward (DEPRECATED), 4 = claim_bond,
                                 // 5 = slash_producer, 6 = coinbase, 7 = add_bond,
                                 // 8 = request_withdrawal, 9 = claim_withdrawal (tombstone),
                                 // 10 = epoch_reward,
                                 // 11 = remove_maintainer, 12 = add_maintainer,
                                 // 13 = delegate_bond, 14 = revoke_delegation,
                                 // 15 = protocol_activation, (16 reserved),
                                 // 17 = mint_asset, 18 = burn_asset,
                                 // 19 = create_pool, 20 = add_liquidity,
                                 // 21 = remove_liquidity, 22 = swap, (23 reserved),
                                 // 24 = create_loan, 25 = repay_loan,
                                 // 26 = liquidate_loan, 27 = lending_deposit,
                                 // 28 = lending_withdraw
    inputs:     input[],
    outputs:    output[],
    extra_data: bytes            // Type-specific data
}
```

### 3.2 Input Structure

```
input = {
    prev_tx_hash:          32 bytes,     // Hash of previous transaction
    output_index:          uint32,       // Index of output being spent
    signature:             64 bytes,     // Ed25519 signature
    sighash_type:          uint8,        // 0 = All (default), 1 = AnyoneCanPay
    committed_output_count: uint32       // 0 = all outputs (default); N > 0 = first N outputs only (AnyoneCanPay)
}
```

Both `sighash_type` and `committed_output_count` default to 0 for backwards compatibility
with pre-v3.7.1 transactions. `AnyoneCanPay` allows partial signing (PSBT-style) — the
signer commits only to their own input + the first `committed_output_count` outputs,
allowing other parties to add inputs (and optionally outputs) after signing.

### 3.3 Output Structure

```
output = {
    output_type:   uint8,        // See output types below
    amount:        uint64,       // Amount in base units
    pubkey_hash:   32 bytes,     // HASH(public_key)
    lock_until:    uint64,       // 0 for normal, u64::MAX for bonds
    extra_data:    bytes         // Type-specific (see below)
}
```

**Output Types (16 total):**

| ID | Type | Purpose | extra_data |
|----|------|---------|------------|
| 0 | Normal | Standard spendable output | Empty |
| 1 | Bond | Time-locked bond UTXO | 4 bytes LE `creation_slot` |
| 2 | Multisig | Threshold-of-N signatures | Covenant conditions |
| 3 | Hashlock | Preimage reveal | Covenant conditions |
| 4 | HTLC | Hashlock + timelock | Covenant conditions |
| 5 | Vesting | Signature + timelock | Covenant conditions |
| 6 | NFT | Non-fungible token with metadata | Covenant conditions |
| 7 | FungibleAsset | User-issued token | Covenant conditions + asset_id |
| 8 | BridgeHTLC | Cross-chain atomic swap | Covenant conditions + chain metadata |
| 9 | Pool | AMM pool (reserves + TWAP state) | Pool metadata (116 bytes) |
| 10 | LPShare | Liquidity provider share (transferable) | Pool ID |
| 11 | Collateral | Lending collateral (locked loan collateral) | Loan metadata |
| 12 | LendingDeposit | Lending pool deposit receipt | Deposit metadata |
| 13 | ZKRollup | L2 committed state (verifying_key + state_root) | `ZkRollupData` (per `specs/l2-settlement.md`) |
| 14 | EncryptedContent | Privacy-first NFT replacement | `[ciphertext_len(4 LE) | ciphertext | wrapped_key(80) | nonce(12) | content_hash(32)]` (≥128 bytes) |
| 15 | OraclePrice | Phase 2.1 oracle aggregated price (system-only UTXO) | 50 bytes: `[price_cents(8 LE) | last_update_height(8 LE) | contributor_count(2 LE) | pair_id(32)]` |

Output types 2-8 use the programmable conditions system (`crates/core/src/conditions/`)
with composable predicates (Signature, Multisig, Hashlock, Timelock, And, Or, Threshold).
Output types 9-12 are DeFi primitives for the AMM pool and lending systems.
OutputType 13 (ZKRollup) is the L2 settlement commitment — see `specs/l2-settlement.md`.
OutputType 14 (EncryptedContent) is the privacy-first NFT replacement.
OutputType 15 (OraclePrice) is the Phase 2.1 oracle's per-pair singleton — created
and consumed exclusively by `apply_block` at epoch boundaries (system-spent only;
user transactions cannot mint one — validation hard-rejects with `[ERRTX-ORACLE004]`).
See `specs/oracle-structural-anchored-economics.md` §1.2.

**Bond UTXO extra_data format:**
```
extra_data = creation_slot as u32 (4 bytes, little-endian)
```

Bond UTXOs are self-descriptive: the UTXO set is the single source of truth for
bond tracking. `creation_slot` in extra_data enables per-bond vesting penalty
calculation without a separate registry. Normal outputs have empty extra_data.

### 3.4 Transaction Hash

```
tx_hash = HASH(version || type || inputs_without_sigs || outputs || extra_data)
```

The signature field is replaced with zeros for hashing.

### 3.5 Transaction Validation

A transaction is valid if:

1. **Format**: All fields are properly encoded
2. **Inputs exist**: Each input references an unspent output
3. **Signatures valid**: Each signature matches the referenced output's pubkey
4. **Amounts balance**: `sum(inputs) >= sum(outputs)`
5. **Positive amounts**: All output amounts > 0
6. **No double-spend**: No output is spent twice within the same tx
7. **Sufficient fee**: `sum(inputs) - sum(outputs) >= min_fee`

Minimum fee:
```
min_fee = BASE_FEE + sum(output.extra_data.len()) * FEE_PER_BYTE
```
Where `BASE_FEE = 1` and `FEE_PER_BYTE = 1`. This prices on-chain storage proportionally:
- Transfer (0 extra_data bytes): 1 sat
- Bond (4 bytes): 5 sats
- NFT (300 bytes): 301 sats

### 3.6 Coinbase Transaction

Every block contains a coinbase transaction as the first transaction. The coinbase
sends the block reward to the **reward pool address** (`reward_pool_pubkey_hash()`),
NOT directly to the producer. Rewards are then distributed from the pool at epoch
boundaries via EpochReward transactions (see section 3.11).

```
coinbase_tx = {
    version: 1,
    type: 0,                     // TxType::Transfer (coinbase uses Transfer with no inputs)
    inputs: [],                  // Empty
    outputs: [{
        output_type: 0,
        amount: block_reward + total_fees,
        pubkey_hash: reward_pool_pubkey_hash,  // Pool address, NOT producer
        lock_until: 0
    }],
    extra_data: block_height as uint64
}
```

Coinbase outputs require 6 confirmations before spending (COINBASE_MATURITY).

### 3.7 Exit Transaction

```
exit_tx = {
    version: 1,
    type: 2,
    inputs: [],                  // Must be empty
    outputs: [],                 // Must be empty
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

Initiates the 7-day unbonding period. Producer is removed from active set. Exit transactions must have no inputs or outputs - they simply identify the producer exiting. The bond is released after the unbonding period via ClaimBond transaction.

### 3.8 Claim Reward Transaction (DEPRECATED)

> **Note**: This transaction type is deprecated per whitepaper compliance.
> Block rewards are now distributed via EpochReward transactions at epoch boundaries (section 3.11).

```
claim_reward_tx = {
    version: 1,
    type: 3,
    inputs: [],
    outputs: [{
        output_type: 0,
        amount: accumulated_rewards,
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

### 3.9 Claim Bond Transaction

```
claim_bond_tx = {
    version: 1,
    type: 4,
    inputs: [],
    outputs: [{
        output_type: 0,
        amount: bond_to_return,
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

Only valid after unbonding period is complete.

### 3.10 Slash Producer Transaction

Slashing is reserved ONLY for double production (creating two different blocks for the same slot). Invalid blocks are simply rejected by the network - no slashing.

```
slash_tx = {
    version: 1,
    type: 5,
    inputs: [],
    outputs: [],                 // Bond is burned
    extra_data: {
        producer_pubkey: 32 bytes,
        evidence: evidence_data,
        reporter_signature: 64 bytes
    }
}

evidence_data = SlashingEvidence::DoubleProduction {
    block_header_1: BlockHeader,  // Full header with VDF proof
    block_header_2: BlockHeader,  // Full header with VDF proof (must differ)
    // Both headers must have same producer + same slot + different hashes
    // Full headers required so validators can verify VDF proofs
}
```

Burns 100% of producer's bond. This is the only slashable offense because it's the only one that cannot happen by accident.

### 3.11 Epoch Reward Transaction

Epoch rewards are the **primary reward mechanism**. At each epoch boundary,
rewards are automatically distributed to attestation-qualified producers, weighted by
bond count (from epoch bond snapshot). This is NOT deprecated — it is the active reward path.

Epoch rewards are distributed at epoch boundaries using a fully deterministic model.
All calculations derive from on-chain data (attestation bitfields + UTXO bond snapshot).

```
epoch_reward_tx = {
    version: 1,
    type: 10,
    inputs: [],                  // Pool UTXOs consumed by consensus engine
    outputs: [{
        output_type: 0,
        amount: proportional_share,  // (pool * producer_bonds) / qualifying_bonds
        pubkey_hash: producer_or_delegator_hash,
        lock_until: 0
    }, ...],                     // One output per qualifier (+ delegator outputs)
    extra_data: {
        height: uint64,          // Block height (8 bytes LE)
        epoch: uint64            // Epoch number being rewarded (8 bytes LE)
    }
}
```

**Epoch Boundary Detection:**

Rewards are triggered when `current_epoch > last_rewarded_epoch`:

```python
def should_include_epoch_rewards(current_slot):
    current_epoch = current_slot // slots_per_reward_epoch
    last_rewarded = scan_chain_for_last_epoch_reward()  # From BlockStore
    if current_epoch > last_rewarded:
        return last_rewarded + 1  # Epoch to reward
    return None

def scan_chain_for_last_epoch_reward():
    # Scan backwards from tip, find most recent EpochReward tx
    # Extract epoch number from EpochRewardData.epoch field
    # Return 0 if no rewards ever distributed
```

**Pool Calculation:**

The reward pool is the sum of accumulated coinbase UTXOs at the reward pool address,
plus the current block's coinbase (not yet in UTXO set during production):

```
pool = sum(coinbase UTXOs at reward_pool_address) + block_reward(epoch_end_height)
```

**Distribution Algorithm (Bond-Weighted with Attestation Qualification):**

```python
def calculate_epoch_rewards(epoch, current_height):
    epoch_start = epoch * blocks_per_reward_epoch
    epoch_end = (epoch + 1) * blocks_per_reward_epoch

    # Get active producers at epoch start, sorted by pubkey
    sorted_producers = get_active_producers_sorted(epoch_start)

    # Scan blocks in epoch, decode presence_root attestation bitfields
    attested_minutes = {}  # producer_index -> set of attested minutes
    for h in range(epoch_start, epoch_end):
        block = block_store.get(h)
        if block and block.presence_root != ZERO:
            minute = attestation_minute(block.slot)
            indices = decode_attestation_bitfield(block.presence_root, len(sorted_producers))
            for idx in indices:
                attested_minutes.setdefault(idx, set()).add(minute)

    # Qualify with never-burn fallback tiers
    threshold = attestation_qualification_threshold(blocks_per_reward_epoch)  # 90%
    if epoch == 0:
        qualified = sorted_producers  # All qualify in genesis epoch
    else:
        tier1 = [p for i, p in enumerate(sorted_producers)
                 if len(attested_minutes.get(i, set())) >= threshold]
        if tier1:
            qualified = tier1
        else:
            # Tier 2: 80% of median attendance, floor of 1
            # Tier 3: if all have 0, pool accumulates
            ...

    # Bond-weighted distribution from epoch bond snapshot
    pool = sum(coinbase UTXOs at pool) + block_reward(epoch_end)
    qualifying_bonds = sum(epoch_bond_snapshot[p] for p in qualified)
    for producer in qualified:
        reward = pool * bonds[producer] / qualifying_bonds  # u128 intermediate

        if producer has no delegations:
            outputs.append((reward, producer_pubkey_hash))
        else:
            # Split between producer's own bonds and delegated bonds
            own_bonds = count_bonds(producer)
            delegated = sum(delegation_counts)
            total = own_bonds + delegated

            own_share = reward * own_bonds / total
            delegated_share = reward - own_share
            delegate_fee = delegated_share * DELEGATE_REWARD_PCT / 100  # 10%
            staker_pool = delegated_share - delegate_fee

            # Distribute staker_pool to individual delegators by bond count
            for delegator in delegations:
                delegator_reward = staker_pool * delegator_bonds / delegated
            # Last delegator gets remainder (no dust)

            outputs.append((own_share + delegate_fee, producer_pubkey_hash))
            outputs.append((delegator_reward, delegator_pubkey_hash))  # per delegator

    # Integer division remainder goes to first qualifier
```

**Catch-Up Mechanism:**

If multiple epochs pass without blocks (e.g., network downtime), rewards catch up
one epoch at a time:

```
Slot 1080 (epoch 3), last_rewarded = 0:

Block N+0: current_epoch=3 > last_rewarded=0 → distribute epoch 1
           (after apply: last_rewarded = 1)

Block N+1: current_epoch=3 > last_rewarded=1 → distribute epoch 2
           (after apply: last_rewarded = 2)

Block N+2: current_epoch=3 > last_rewarded=2 → distribute epoch 3
           (after apply: last_rewarded = 3)

Block N+3: current_epoch=3 == last_rewarded=3 → normal block, no rewards
```

**Validation Rules (Exact Match Required):**

Validators recalculate expected rewards from BlockStore and require exact match:

| Condition | Rule |
|-----------|------|
| At epoch boundary | Block MUST contain correct EpochReward txs |
| At non-boundary | Block MUST NOT contain EpochReward txs |
| Amounts | Each reward MUST match `(pool × producer_bonds) / qualifying_bonds` exactly |
| Recipients | Only attestation-qualified producers (and their delegators) |
| Ordering | EpochReward txs sorted by producer pubkey |
| Epoch number | Must match `last_rewarded + 1` |

**Guarantees:**

- **Deterministic**: Any node calculates identical rewards from the same BlockStore
- **Restart-safe**: SyncManager initializes from stored ChainState on restart (resumes from tip, not genesis). `apply_block()` rejects duplicate blocks already in BlockStore.
- **Sync-safe**: Nodes syncing from peers validate all historical rewards
- **Fork-safe**: Each chain fork recalculates rewards from its own blocks
- **Maturity**: Epoch reward outputs require 6 confirmations (COINBASE_MATURITY)

### 3.12 AddBond Transaction

Allows a producer to add additional bonds. Each bond creates a Bond UTXO with
`creation_slot` in extra_data.

```
add_bond_tx = {
    version: 1,
    type: 7,
    inputs: [...],               // Funds to become bonds
    outputs: [{                  // One Bond UTXO per bond unit
        output_type: 1,          // Bond
        amount: BOND_UNIT,       // 10 DOLI per bond
        pubkey_hash: producer_pubkey_hash,
        lock_until: u64::MAX,    // Locked until withdrawal
        extra_data: creation_slot as u32 LE  // 4 bytes
    }, ...],
    extra_data: {
        producer_pubkey: 32 bytes,
        bond_count: uint32       // Number of bonds to add (must be positive)
    }
}
```

**Validation rules:**
- Producer must be registered
- `bond_count` must be > 0
- Input amount must equal `bond_count × BOND_UNIT` (plus fees)
- Each output must be type Bond with correct amount, lock_until=u64::MAX, and 4-byte extra_data
- **INC-I-080**: total bonds after addition must not exceed `MAX_BONDS_PER_PRODUCER`
  (3,000), enforced height-gated at `addbond_cap_enforcement_activation_height`.
  - **Pre-activation** (`height < AH`): NOT enforced at validation. Historical
    behavior is preserved — `ProducerInfo::add_bonds` silently clips the excess
    at epoch flush and the surplus Bond UTXOs are orphaned. Kept for replay
    safety on pre-activation blocks.
  - **Post-activation** (`height >= AH`): a block carrying an AddBond where
    `bond_count + in-flight pending AddBonds (incl. earlier in the same block) +
    requested > MAX_BONDS_PER_PRODUCER` is **rejected** at block validation
    (`validation::check_addbond_cap` → `ValidationError::AddBondCapExceeded`,
    error code `ADDBOND_CAP_EXCEEDED`), before any state mutation, so no Bond
    UTXOs are created. Unlike the INC-I-078 DelegateBond cap (silent skip-in-
    block — DelegateBond has no outputs to orphan), AddBond must reject the
    block because its Bond outputs are real UTXOs. Mainnet AH = `0`, testnet
    `0`, devnet `u64::MAX` (`crates/core/src/network_params/defaults.rs:160,449,711`
    — code is SoT). Mainnet was re-pinned from `254_344` to `0` by the
    2026-07-08 fresh-genesis reset (`61218e90`, "all AH→0"), so the cap has been
    enforced from block 0 of the current mainnet chain and the pre-activation
    clip path is unreachable there. No `CURRENT_PROTOCOL_VERSION` bump
    (EpochState unchanged); no `HardForkSchedule` entry (pure validation rule);
    rolling-deploy safe.
  - **INC-I-203 — node-local builder filter (not consensus).** The block builder
    also evaluates `check_addbond_cap` during selection
    (`bins/node/src/node/production/withdrawal_holdings.rs`, via
    `mempool::addbond_cap::addbond_cap_verdict`) and SKIPS an AddBond that would
    make the assembled block fail the gate above, logging the skip at `warn!`.
    This is builder policy, not a consensus rule. The builder carries the same
    block-local `in_block` tally the gate does, so its expression is the gate's
    expression term for term and it cannot drop a transaction that would have
    sat in the valid block it is assembling. (Mempool admission, which has no
    block context, drops the `in_block` term and therefore rejects a strict
    subset of what the gate rejects.)
    It is gated on the same `addbond_cap_enforcement_activation_height`, and
    when no holdings source can answer it fails OPEN (packs, as before).

### 3.13 WithdrawalRequest Transaction

Instant bond withdrawal. Consumes Bond UTXOs (FIFO — oldest first), creates a
Normal output with the net amount after vesting penalties. Penalty is the
difference between input (bond) and output (payout) — burned naturally via
UTXO accounting. **No delay. Funds available in the same block.**

```
withdrawal_request_tx = {
    version: 1,
    type: 8,
    inputs: [{                   // Bond UTXOs to consume (FIFO order)
        prev_tx_hash: ...,       // References a Bond UTXO
        output_index: ...,
        signature: ...
    }, ...],
    outputs: [{
        output_type: 0,          // Normal output (payout)
        amount: net_amount,      // Bond value minus vesting penalty
        pubkey_hash: destination,
        lock_until: 0,
        extra_data: []           // Empty for normal outputs
    }],
    extra_data: {
        producer_pubkey: 32 bytes,   // Producer's public key
        bond_count: uint32,          // Number of bonds to withdraw
        destination: 32 bytes        // Destination address (pubkey_hash)
    }
}
```

**Validation rules:**
- Producer must be registered
- All inputs must reference Bond UTXOs (output_type == 1)
- Inputs must be owned by the producer (pubkey_hash matches)
- Validation bypasses `is_spendable_at()` for Bond UTXOs (lock_until=u64::MAX)
- Vesting penalty per bond: derived from `creation_slot` in the Bond UTXO's extra_data
- `net_amount = sum(bond_amounts) - sum(penalties)`
- Penalty = `sum(inputs) - sum(outputs)` (burned, not redistributed)
- Bond count update takes effect at next epoch boundary (PendingProducerUpdate)
- **INC-I-180**: the requested `bond_count` must not exceed the producer's bond
  holdings, enforced height-gated at `withdrawal_holdings_gate_activation_height`.
  A conforming client derives the allowance from the ProducerSet
  (`selectionWeight − Σ receivedDelegations + delegatedBonds − withdrawal_pending_count`),
  never from the UTXO-derived bond count, and refuses to emit when the two ledgers
  disagree (the DOLI CLI does this as of INC-I-180 M3; RPC exposes the ProducerSet
  count as `producerSetBondCount` distinct from the UTXO-derived `bondCount`).
  - **Pre-activation** (`height < AH`): NOT enforced. Historical behavior is
    preserved — `process_transaction_producer_effects` silently skips the
    deferred `PendingProducerUpdate::RequestWithdrawal` on shortfall, *after*
    `process_transaction_utxos` already spent every Bond UTXO input. The
    producer keeps selection weight with no bonds behind it (mainnet n11:
    434 unbacked weight units). Kept for replay safety.
  - **Post-activation** (`height >= AH`): a block carrying a RequestWithdrawal
    is **rejected** at block validation (`Node::validate_block_economics`),
    before any state mutation, so no Bond UTXO is ever spent, when any of:
    - `bond_count > allowance` — `ECON_WITHDRAWAL_OVER_HOLDINGS`. The allowance
      is `bond_count + pending AddBonds + AddBonds earlier in the same block
      − withdrawal_pending_count − bonds charged by Exits and RequestWithdrawals
      earlier in the same block`, saturating. An `Exit` charges the producer's
      whole `bond_count` because the apply layer bumps `withdrawal_pending_count`
      for it immediately; the apply layer re-reads an unchanged `bond_count` per
      `Exit`, so two Exits for one producer charge it twice and the allowance
      reproduces that.
    - the producer is not in the ProducerSet — `ECON_WITHDRAWAL_UNKNOWN_PRODUCER`.
    - any input references a transaction at a **lower index in the same block** —
      `ECON_WITHDRAWAL_SAME_BLOCK_INPUT`. Block validation resolves inputs
      against the **pre-block** UTXO view, which every node computes identically
      at this height and which `validate_block_economics` holds no write batch
      over. An outpoint created earlier in the same block is therefore invisible
      to the Bond counters below while `process_transaction_utxos` spends it
      anyway, so it is refused rather than counted. This is what makes the
      pre-block view **exhaustive** for withdrawal inputs, and the exclusivity
      rule below complete rather than partial (AUDIT-P1-006).
    - the transaction fails `owned == all`, where `all` is the number of inputs
      that resolve, in the pre-block UTXO set, to a `Bond` output and `owned` is
      the subset of those whose `pubkey_hash` equals
      `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey)` — the address at which
      Registration, AddBond and genesis all create Bond outputs —
      `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`. This **exclusivity** rule runs
      before the shape split below and applies to every shape. Without the
      **owner** binding a transaction signed by producer `A`, spending `A`'s own
      Bond UTXOs, may name producer `B`: `A` loses the UTXOs, `B` loses the
      weight, and `A` keeps unbacked selection weight. Without the
      **exclusivity** half an actor holding both keys declares `B`'s true count
      and adds `A`'s Bond UTXOs as riders — `process_transaction_utxos` spends
      all of them, and only `B`'s ledger moves (AUDIT-P1-001). The Bond lock is
      bypassed for this transaction type, so ownership is not otherwise checked.
    - the transaction fails the rule for its **shape**. A request declaring the
      producer's whole allowance (`bond_count == allowance && bond_count > 0`)
      is a **full exit**: `apply_withdrawal` drives `bond_count` to 0 and the
      auto-exit fires whatever the declared number was, so the ledger can never
      outlive its bonds and the obligation moves to the UTXO side — the
      transaction must spend EVERY Bond UTXO the producer owns in the pre-block
      view (`owned == utxo.get_bond_entries(addr)`), else
      `ECON_WITHDRAWAL_INCOMPLETE_DRAIN`. Any other request is a **partial** and
      keeps the strict `bond_count == owned` binding, else
      `ECON_WITHDRAWAL_BOND_COUNT_MISMATCH`: the allowance bounds the declared
      count from above only, so an under-declared partial would destroy every
      Bond input while removing one bond of weight. The full-exit branch is the
      permanent in-band remedy for a producer whose ledger already disagrees with
      its Bond UTXOs — it can always zero the ledger and retire (AUDIT-P1-002).

    The apply-layer enqueue accepts exactly the same allowance, and the
    reorg/rollback replay (`rebuild_producer_set_from_blocks`) mirrors the
    in-flight AddBond term under the same height gate, so live apply and replay
    queue the same updates. **INC-I-180 M2 (AUDIT-P1-004)**: the replay also
    mirrors the live auto-revoke — a full exit blocked by an active delegation
    queues `PendingProducerUpdate::RevokeDelegation` before the withdrawal
    (INC-I-058). That branch is not itself height-gated in live apply, only the
    in-flight term inside its allowance is, so the mirror inherits the same
    height-dependence and needs no gate of its own. Without it a reorg through
    such a block leaves `received_delegations` un-revoked, and that field is
    inside `serialize_canonical()` — the rebuilt and live nodes would then
    disagree on the producer-set contribution to the state root. The admission
    rules (the `Exit` charge, the same-block-input refusal, the exclusivity rule
    and the shape split) are NOT mirrored in the replay: it reads blocks that
    are already canonical.

    **Mode split (INC-I-180 M2 / S3).** The gate is evaluated **before** the
    EpochReward section of `validate_block_economics`, so no early return can
    make it mode-dependent by accident. Its verdict is identical in **Full and
    Light**, the two admission modes gossip-received blocks reach.
    `ValidationMode::Replay` — reached only by the operator `recover`/reindex
    tool — carves out the rules whose every term is read from the pre-block
    UTXO view, which a replay legitimately sees degraded (INC-I-064): the
    unknown-producer rule, the exclusivity rule and both shape rules `warn!`
    with `[REPLAY_SKIP]` and skip that transaction instead of failing the
    block. `ECON_WITHDRAWAL_OVER_HOLDINGS` (reads the ProducerSet allowance
    only) and `ECON_WITHDRAWAL_SAME_BLOCK_INPUT` (reads the block's own earlier
    transaction hashes, which a replay has in full) stay **strict in all three
    modes**. The skipped transaction still charges the allowance, or the
    strict allowance rule would drift for a later withdrawal by the same
    producer in the same block. Without this carve-out the reindex aborts on
    the first already-canonical block whose Bond inputs are not yet resolvable.
    The EpochReward section returns `Ok(())` early in Full mode
    (`INC_I_081_MISSING_CHECK_SKIP`) whenever the local store cannot prove an
    EpochReward was owed — the normal state of a freshly snap-synced node — and
    a rule placed after it would be enforced in Light/Replay only. The INC-I-080
    AddBond cap remains **after** that early return, unchanged: it is enforced
    from height 0 on mainnet and testnet, so running it in a case where it never
    ran would change the verdict of already-canonical blocks.
    Mainnet AH = `u64::MAX` (frozen — pinning is a separate decision session);
    testnet `230_000`; devnet `20`. No `CURRENT_PROTOCOL_VERSION` bump
    (EpochState unchanged); no `HardForkSchedule` entry; rolling-deploy safe.

  - **Admission and selection parity (INC-I-180 M2, INV-VALIDATION-001 /
    INV-PROD-003).** A consensus rule with no mempool and no builder
    counterpart turns an ordinary user mistake into free block poison: the
    transaction never confirms, so no fee is paid and no input is spent, yet
    every producer that selects it burns a block build. Both layers therefore
    apply the same rules, height-gated on the same AH and strict no-ops below
    it. Neither is consensus: skipping at selection yields a valid subset
    block, and refusing admission keeps a transaction out of one node's
    mempool only.
    - **Block builder** (`bins/node/src/node/production/withdrawal_holdings.rs`)
      applies the whole table — including the same-block-input refusal and the
      in-block `AddBond`/`Exit`/`RequestWithdrawal` accounting carried across
      the selection loop — and **skips** an offending transaction, exactly like
      the NFT/Pool unique-id checks. The builder and the mempool compute the
      allowance through `ProducerHoldings::allowance_with`
      (`crates/mempool/src/holdings.rs`). The gate does NOT call it: it holds
      the reference expression inline in `validate_block_economics`, because
      routing consensus validation through the mempool crate would invert the
      layering. Those two transcriptions are locked by the two
      `inc_i180_m2_the_gate_allowance_equals_the_shared_function*` rows, eight
      allowance shapes in total, which require the allowance the gate REPORTS
      to equal `allowance_with` on the terms the same message echoes.
      The lock is load-bearing: the terms saturate, so a second order silently
      raises one layer's allowance above the other's when
      `withdrawal_pending > bond_count + pending_addbond` AND the block carries
      an earlier same-producer `AddBond` — the deficit alone is not enough,
      because with `in_block_addbond = 0` both orders clamp to the same 0.
      **The lock covers R1 only.** Every row declares `allowance + 1` and so
      bails at R1 by construction; the unknown-producer, exclusivity, shape and
      same-block-input rules are transcribed once per layer with no equivalent
      term-exact lock, and unifying them is the routed residual
      (`FIND-I180-M2-TRANSCRIPTION-001`). The builder's refusal is
      never a build failure, never an abort, and it never evicts from the
      mempool. It resolves producer holdings under the
      producer guard and releases it before taking the UTXO guard, so only one
      of the two is ever held: `apply_block` takes utxo→producers while
      `rollback` takes producers→utxo, and holding both here would join those
      orders. The builder never constructs `[partial(P), full-exit(P)]` in one
      block — that pair is unsatisfiable at any input set, because the owned
      Bond UTXO count is memoised over the pre-block view while the allowance
      shrinks as the block is walked.
    - **Mempool** (`crates/mempool/src/withdrawal_holdings.rs`) applies the
      subset decidable from a single transaction against current state: the
      unknown-producer, allowance, exclusivity and shape rules. The
      same-block-input rule is not checkable there and is deliberately absent.
      Admission does not evaluate the block's in-block terms; it **substitutes
      mempool-wide state** for them. `in_block_addbond` is zero, and
      `in_block_withdrawn` is replaced by `in_mempool_withdrawn` — the bonds
      every same-producer `RequestWithdrawal` this mempool holds already claims
      (`pool.rs`, `count_residents = true`). The general rule follows: admission
      can OVER-reject only when the substitute RAISES the block's allowance
      (`in_block_addbond → 0`) or EXCEEDS the block's debit
      (`in_mempool_withdrawn > in_block_withdrawn`, since a block need not carry
      the residents). It can also UNDER-reject — the absent R4, and any block
      debit larger than this mempool's residents — which the builder and the
      gate still catch; only the over-rejection direction is a liveness cost.
      This is a rule, not an
      enumeration: `[AddBond(P, +n), RequestWithdrawal(P, d)]` with `d` above
      the flushed allowance is one instance, and a resident
      `RequestWithdrawal(P, d1)` that drops the admission allowance far enough
      to push a second `RequestWithdrawal(P, d2)` out of R2's partial branch and
      into its full-exit branch is another — with no `AddBond` anywhere.
      Every such over-rejection is bounded until the resident confirms **or
      expires** (14-day mempool age), and costs no fee, no input and no block;
      then the operator resubmits. The resident charge is deliberate — it keeps
      this mempool from ever offering a builder the `[partial(P), full-exit(P)]`
      pair (SEC-FIXVERIFY2-001). `revalidate` re-evaluates and
      **evicts** a held withdrawal the ledger has moved out from under, which
      input-existence alone can never shed. Holdings are resolved from the
      node's live `ProducerSet` (non-blocking `try_read`), falling back to a
      published snapshot while that handle is contended, and the rules do not
      run at all when neither is wired — or when the snapshot is wired but
      EMPTY, which is no answer rather than "not a producer". That snapshot is
      refreshed once per
      APPLIED block and is **not** refreshed by rollback, reorg, fork recovery
      or snapshot install, so after a rewind of depth N it is up to N blocks
      stale until the next block is applied. Both staleness directions are
      non-safety: the live handle is tried first, and a stale answer that
      over-rejects costs one resubmission while one that under-rejects is still
      caught by the builder and by the gate.

**Note:** TxType 9 (ClaimWithdrawal) is reserved but unused — withdrawal is instant.

**Vesting Schedule (Early Withdrawal Penalties):**

Bond vesting is network-differentiated:

**Mainnet** — 4-year vesting (1-year quarters):

| Bond Age | Penalty | Net Return |
|----------|---------|------------|
| Y1 (0-1 year) | 75% burned | 25% returned |
| Y2 (1-2 years) | 50% burned | 50% returned |
| Y3 (2-3 years) | 25% burned | 75% returned |
| Y4+ (3+ years) | 0% (fully vested) | 100% returned |

`VESTING_QUARTER_SLOTS = 3,153,600` (1 year), `VESTING_PERIOD_SLOTS = 12,614,400` (4 years).

**Testnet** — 1-day vesting (6h quarters):

| Bond Age | Penalty | Net Return |
|----------|---------|------------|
| Q1 (0-6 hours) | 75% burned | 25% returned |
| Q2 (6-12 hours) | 50% burned | 50% returned |
| Q3 (12-18 hours) | 25% burned | 75% returned |
| Q4+ (18+ hours) | 0% (fully vested) | 100% returned |

Testnet `vesting_quarter_slots = 2,160` via `NetworkParams`. Devnet configurable via `DOLI_VESTING_QUARTER_SLOTS`.

Penalty calculation uses FIFO order — oldest Bond UTXOs are consumed first,
ensuring bonds that have vested longer incur lower penalties. The `creation_slot`
stored in each Bond UTXO's `extra_data` (4 bytes LE) determines the bond's age
and therefore its penalty tier.

### 3.15 RemoveMaintainer Transaction

Removes a maintainer from the auto-update system. Requires 3/5 multisig from OTHER maintainers
(the target cannot sign their own removal).

```
remove_maintainer_tx = {
    version: 1,
    type: 11,
    inputs: [],                  // Must be empty (state-only operation)
    outputs: [],                 // Must be empty
    extra_data: {
        target: 32 bytes,        // Public key of maintainer to remove
        reason: string,          // Optional reason for removal
        signatures: signature[]  // Signatures from at least 3 maintainers
    }
}
```

**Validation rules:**
- Target must be a current maintainer
- At least 3 valid signatures from OTHER current maintainers (target cannot sign)
- Cannot reduce maintainer count below MIN_MAINTAINERS (3)
- Slashed producers are automatically removed from maintainer set

### 3.16 AddMaintainer Transaction

Adds a new maintainer to the auto-update system. Requires 3/5 multisig from current maintainers.

```
add_maintainer_tx = {
    version: 1,
    type: 12,
    inputs: [],                  // Must be empty (state-only operation)
    outputs: [],                 // Must be empty
    extra_data: {
        target: 32 bytes,        // Public key of producer to add as maintainer
        reason: string,          // Optional reason
        signatures: signature[]  // Signatures from at least 3 maintainers
    }
}
```

**Validation rules:**
- Target must be a registered producer
- Target must not already be a maintainer
- At least 3 valid signatures from current maintainers
- Cannot exceed MAX_MAINTAINERS (5)

**Maintainer Bootstrap:**
- The first 5 registered producers automatically become the initial maintainer set
- Bootstrap occurs once at the epoch boundary where the 5th producer becomes available
- The maintainer set is persisted to `maintainer_state.bin` in the node's data directory
- After bootstrap, changes require 3/5 multisig via AddMaintainer/RemoveMaintainer transactions
- MaintainerAdd/RemoveMaintainer transactions are applied immediately (not deferred to epoch boundary)
- Pre-bootstrap: nodes fall back to `BOOTSTRAP_MAINTAINER_KEYS` (hardcoded in binary) for release signature verification

### 3.17 ProtocolActivation Transaction

Activates new consensus rules at a future epoch boundary. Requires 3/5 multisig from current maintainers (first 5 registered producers). This is the on-chain mechanism for coordinated consensus upgrades — all nodes switch simultaneously at the target epoch.

```
protocol_activation_tx = {
    version: 1,
    type: 15,                    // TxType::ProtocolActivation
    inputs: [],                  // Must be empty (state-only operation)
    outputs: [],                 // Must be empty
    extra_data: {
        protocol_version: uint32,   // Version to activate (must be > current)
        activation_epoch: uint64,   // Epoch at which activation occurs (must be future)
        description: string,        // Human-readable description of changes
        signatures: signature[]     // Signatures from at least 3 maintainers
    }
}
```

**Signing message format:** `"activate:{version}:{epoch}"` (UTF-8 bytes)

**Validation rules (structural):**
- No inputs, no outputs
- Valid ProtocolActivationData in extra_data
- `protocol_version > 0`
- `activation_epoch > 0`
- At least 1 signature present

**Validation rules (node-level, requires state):**
- 3/5 valid maintainer signatures (first 5 registered producers)
- `protocol_version > active_protocol_version`
- `activation_epoch > current_epoch`

**Activation lifecycle:**
1. Tx included in block → `pending_protocol_activation = Some((version, epoch))`
2. At epoch boundary where `current_epoch >= activation_epoch` → `active_protocol_version = version`
3. Code gated by `is_protocol_active(version, state)` now executes

**Gate function:**
```rust
pub fn is_protocol_active(required: u32, active: u32) -> bool {
    active >= required
}
```

### 3.18 DelegateBond Transaction

Delegates bond weight to a Tier 1/2 validator. The delegate receives the staker's weight for reward distribution (not for slot selection — see §5.3 hotfix). Rewards are split: delegate keeps 10% (`DELEGATE_REWARD_PCT`), stakers receive 90% (`STAKER_REWARD_PCT`).

```
delegate_bond_tx = {
    version: 1,
    type: 13,                    // TxType::DelegateBond
    inputs: [],                  // Must be empty (state-only operation)
    outputs: [],                 // Must be empty
    extra_data: {                // Two wire forms (INC-I-078 M2, F3-compat):
        // Legacy form (68 B, pre `delegation_auth_activation_height`):
        delegator_pubkey: 32 bytes,
        delegate_pubkey:  32 bytes,
        bond_count:       uint32_le,
        // Authenticated form (132 B, post-activation; legacy fields PLUS):
        signature:        64 bytes,  // Ed25519 by delegator over signing_message
    }
}
```

**Signing message (post-activation):** `BLAKE3("DELEGATE_BOND" || delegate_pubkey || bond_count_le)`. The delegator's pubkey is NOT in the commit — it is the verification key, including it would be redundant.

**Validation rules:**
- Delegator must be a registered producer with sufficient bonds
- Delegate must be a registered, active Tier 1/2 validator
- Bond count must be > 0 and <= delegator's available (undelegated) bonds
- State-only: no UTXO inputs required (spam-protected by bond requirement)
- **INC-I-078 M1**: at and after `received_delegation_cap_activation_height`, reject (silent skip at apply) if `delegate.received_delegations.sum() + bond_count > received_delegation_cap`. Grandfathered: existing over-cap delegates are not forced to shed.
- **INC-I-078 M2**: at and after `delegation_auth_activation_height`, the `signature` field MUST verify against `delegator_pubkey` over the signing message above. Default (all-zero) signatures fail-closed. Pre-activation: signature ignored; both wire forms accepted.

### 3.19 RevokeDelegation Transaction

Revokes a previous delegation. Subject to `DELEGATION_UNBONDING_SLOTS` delay (~7 days / 60,480 slots).

```
revoke_delegation_tx = {
    version: 1,
    type: 14,                    // TxType::RevokeDelegation
    inputs: [],                  // Must be empty (state-only operation)
    outputs: [],                 // Must be empty
    extra_data: {                // Two wire forms (INC-I-078 M2, F3-compat):
        // Legacy form (64 B, pre `delegation_auth_activation_height`):
        delegator_pubkey: 32 bytes,
        delegate_pubkey:  32 bytes,
        // Authenticated form (128 B, post-activation; legacy fields PLUS):
        signature:        64 bytes,  // Ed25519 by delegator over signing_message
    }
}
```

**Signing message (post-activation):** `BLAKE3("REVOKE_DELEGATION" || delegate_pubkey)`. Same authentication scheme as DelegateBond (constraint C7 — RevokeDelegation has the same zero-input forgery vector that DelegateBond closed).

State-only transaction: no UTXO inputs required (spam-protected by bond requirement).

### 3.20 MintAsset Transaction

Mints new units of a fungible asset. Issuer-only — requires matching `asset_id`.

```
mint_asset_tx = {
    version: 1,
    type: 17,                    // TxType::MintAsset
    inputs: [{...}],             // Must spend from issuer's address
    outputs: [{
        output_type: 7,          // OutputType::FungibleAsset
        amount: uint64,
        pubkey_hash: 32 bytes,   // Recipient
        lock_until: 0,
        extra_data: {
            asset_id: 32 bytes   // Asset identifier
        }
    }],
    extra_data: {}
}
```

### 3.21 BurnAsset Transaction

Burns units of a fungible asset. Holder burns own tokens, provably destroyed.

```
burn_asset_tx = {
    version: 1,
    type: 18,                    // TxType::BurnAsset
    inputs: [{...}],             // Must spend FungibleAsset UTXOs
    outputs: [{...}],            // Optional change output (remaining asset balance)
    extra_data: {}
}
```

### 3.21 Phase 2.1 Oracle Transactions

#### PriceAttestation (type 16)

Submitted by a bonded producer (the "attester") containing a price observation
for an asset pair, scoped to a single epoch. At the epoch-boundary block,
`apply_block` aggregates all valid attestations into the per-pair OraclePrice
UTXO (OutputType=15) using bond-weighted median. Gated by
`oracle_activation_height` in `NetworkParams` (default `u64::MAX` on all
networks until a future binary flips it).

```
{
    version: 1,
    type: 16,                    // TxType::PriceAttestation
    inputs: [],                  // ALWAYS empty (purely informational)
    outputs: [],                 // ALWAYS empty (no UTXO mutation)
    extra_data: PriceAttestationData (144 bytes)
}
```

`extra_data` layout (144 bytes, fixed, no legacy form):

```
offset   0  signer_pubkey   [u8; 32]   Ed25519 verifying key
offset  32  price_cents     u64 LE      attested price in USD cents
offset  40  pair_id         [u8; 32]    BLAKE3("ORACLE_PAIR" || pair_string)
offset  72  epoch_number    u64 LE      epoch in which attestation is valid
offset  80  signature       [u8; 64]    Ed25519 sig over signing_message()
```

`signing_message() = BLAKE3(pair_id || price_cents.to_le_bytes() || epoch_number.to_le_bytes())`
— no domain prefix (spec §1.1 verbatim).

**Validation rules** (`validate_transaction` PriceAttestation arm):
1. `current_height ≥ oracle_activation_height` (`[ERRTX-ORACLE001]`)
2. M8 sunset NOT triggered (`[ERRTX-ORACLE003]`)
3. `signer_pubkey ∈ ctx.active_producers`
4. `epoch_number == reward_epoch::from_height(current_height)`
5. Signature verifies over `signing_message()`
6. Structural: no inputs, no outputs, 144-byte extra_data

Rule 4 (pool-liquidity check) and the consensus-strict at-most-one-per-attester
rule are deferred to the M6 aggregator (`bins/node/src/node/apply_block/oracle.rs`).
The aggregator dedups latest-per-attester defensively before computing the
median.

See `specs/oracle-structural-anchored-economics.md` §1.1.

### 3.22 AMM Pool Transactions

The following transaction types support the on-chain AMM (Automated Market Maker) system:

**CreatePool (type 19):** Creates a new AMM pool with initial liquidity. Creates a Pool output (type 9) with reserves and TWAP state, plus LPShare outputs (type 10) for the creator.

**AddLiquidity (type 20):** Adds liquidity to an existing pool. Consumes the existing Pool UTXO, creates an updated Pool UTXO with increased reserves, and mints new LPShare outputs.

**RemoveLiquidity (type 21):** Burns LPShare outputs to remove liquidity. Consumes LPShare UTXOs and the Pool UTXO, creates updated Pool UTXO with reduced reserves, and returns assets to the provider.

**Swap (type 22):** Swaps assets through a pool. Consumes the Pool UTXO and input assets, creates updated Pool UTXO with modified reserves and the swapped output. Fee: 0.3% (30 basis points) by default.

Pool IDs are deterministic: `BLAKE3("DOLI_POOL" || asset_a_id || asset_b_id)`. Pool metadata (116 bytes) includes: version, pool_id, asset_b_id, reserve_a, reserve_b, total_lp, cumulative_price (TWAP), last_slot, fee_bps, creation_slot, status.

### 3.23 Lending Transactions

The following transaction types support the on-chain lending system:

**CreateLoan (type 24):** Creates a collateralized loan. Locks collateral in a Collateral output (type 11). The borrower receives the loan amount from the lending pool.

**RepayLoan (type 25):** Repays a loan and recovers collateral. Consumes the Collateral UTXO and returns it to the borrower after loan repayment (principal + interest).

**LiquidateLoan (type 26):** Liquidates an undercollateralized loan. When collateral value falls below the liquidation threshold, anyone can liquidate the position.

**LendingDeposit (type 27):** Deposits DOLI into the lending pool. Creates a LendingDeposit output (type 12) representing the depositor's share of the pool.

**LendingWithdraw (type 28):** Withdraws DOLI plus accrued interest from the lending pool. Consumes LendingDeposit UTXOs.

---

## 4. Blocks

### 4.1 Block Header

```
block_header = {
    version:       uint32,       // Currently 2
    prev_hash:     32 bytes,     // Hash of previous block header
    merkle_root:   32 bytes,     // Merkle root of transactions
    presence_root: 32 bytes,     // Presence commitment hash (ZERO in deterministic model)
    genesis_hash:  32 bytes,     // Chain identity: BLAKE3(genesis_time || network_id || slot_duration || message)
    timestamp:     uint64,       // Unix timestamp (seconds)
    slot:          uint32,       // Derived from timestamp
    producer:      32 bytes,     // Producer's public key
    vdf_output:    bytes,        // VDF computation result (~256 bytes)
    vdf_proof:     bytes         // VDF proof (~256 bytes)
}
```

**genesis_hash** is a cryptographic fingerprint of the chain's genesis parameters. It ensures
that blocks from nodes with different genesis configurations (even 1 second difference in
timestamp) are rejected immediately. Computed as:

```
genesis_hash = BLAKE3(
    genesis_timestamp (8 bytes LE) ||
    network_id (4 bytes LE) ||
    slot_duration (8 bytes LE) ||
    genesis_message (variable bytes)
)
```

### 4.2 Block Body

```
block = {
    header:                  BlockHeader,
    transactions:            transaction[],
    aggregate_bls_signature: bytes       // BLS aggregate sig over attestation bitfield (empty for pre-BLS blocks)
}
```

The `aggregate_bls_signature` field stores the aggregated BLS signatures of producers whose bits are set in the attestation bitfield (stored in `header.presence_root` for v2+ blocks). Empty for pre-BLS blocks (backward compatibility). Stored in body (not header) to keep header hash stable.

**Note:** In the deterministic scheduler model, presence commitments are not used for consensus. The `presence_root` field in the header is retained for backward compatibility and is `Hash::ZERO` in the deterministic model. For v2+ blocks, it may contain an attestation commitment (Merkle root of RegionAggregates).

### 4.3 Block Hash

```
block_hash = HASH(
    version (4B LE) ||
    prev_hash (32B) ||
    merkle_root (32B) ||
    presence_root (32B) ||
    genesis_hash (32B) ||
    timestamp (8B LE) ||
    slot (4B LE) ||
    producer (32B) ||
    vdf_output.value (variable)
)
```

Note: The `vdf_proof` is NOT included in the block hash. The presence_root and genesis_hash are always included (presence_root is Hash::ZERO in deterministic scheduler model).

### 4.4 Merkle Root

Binary Merkle tree using BLAKE3:

```
merkle_root = merkle_tree([tx_hash for tx in transactions])

def merkle_tree(hashes):
    if len(hashes) == 0:
        return HASH("")
    if len(hashes) == 1:
        return hashes[0]
    if len(hashes) % 2 == 1:
        hashes.append(hashes[-1])  // Duplicate last
    next_level = []
    for i in range(0, len(hashes), 2):
        next_level.append(HASH(hashes[i] || hashes[i+1]))
    return merkle_tree(next_level)
```

### 4.5 VDF Preimage

```
vdf_input = HASH("DOLI_VDF_BLOCK_V1" || prev_hash || merkle_root || slot || producer)

// Breakdown:
// "DOLI_VDF_BLOCK_V1" = 17 bytes (domain separator)
// prev_hash    = 32 bytes
// merkle_root  = 32 bytes
// slot         = 4 bytes (uint32 LE)
// producer     = 32 bytes
// Total: 117 bytes before hashing
```

---

## 5. Consensus Rules

### 5.1 Time Constants

```
GENESIS_TIME = 1774540572         // Mainnet genesis (must match chainspec)
SLOT_DURATION = 10                // seconds (all networks)
SLOTS_PER_EPOCH = 360             // 1 hour (360 × 10s)
SLOTS_PER_ERA = 12_614_400        // ~4 years
BOOTSTRAP_BLOCKS = 60_480         // ~1 week
```

### 5.2 Slot Derivation

```
slot = floor((timestamp - GENESIS_TIME) / SLOT_DURATION)
```

The slot is NOT a free field; it must be derived from the timestamp.

### 5.3 Block Validity

A block B is valid if ALL conditions hold.

**Implementation Reference:** See `crates/core/src/validation/` (modularized):
- `block.rs` -- `validate_header()`, `validate_block()` -- header and full block validation
- `transaction.rs` -- `validate_transaction()` -- transaction validation
- `producer.rs` -- `validate_producer_eligibility()` -- producer checks
- `registration.rs` -- registration validation
- `utxo.rs` -- UTXO validation

```
0. CHAIN IDENTITY (checked FIRST):
   B.genesis_hash == local_genesis_hash
   Rejects blocks from nodes with different genesis parameters.

1. FORMAT:
   B.version == 2
   B.prev_hash references a known valid block

2. TIMING:
   B.timestamp > prev_block.timestamp
   B.timestamp <= local_time + MAX_DRIFT (1 second)
   B.timestamp >= slot_start + (SLOT_DURATION - NETWORK_MARGIN)
   B.timestamp <= slot_start + SLOT_DURATION + DRIFT

3. SLOT:
   B.slot == floor((B.timestamp - GENESIS_TIME) / SLOT_DURATION)
   B.slot > prev_block.slot

4. PRODUCER (if height >= BOOTSTRAP_BLOCKS):
   B.producer == selected_producer(B.slot, active_producer_set)
   // INC-I-078 hotfix: actual production scheduler is unweighted
   // round-robin — slot % active_producer_count — at
   // bins/node/src/node/production/scheduling.rs:446. The
   // bond-weighted "ticket" rotation described in 5.4 below is the
   // LEGACY/REFERENCE model and is no longer used for slot selection.
   // Bonds influence reward weight, attestation, and governance, NOT
   // slot selection. Verified against the codebase 2026-05-17.
   B.producer is in active_producer_set

5. VDF:
   vdf_input = HASH("DOLI_VDF_BLOCK_V1" || prev_hash || B.merkle_root || B.slot || B.producer)
   VDF_verify(vdf_input, B.vdf_output, B.vdf_proof, T_BLOCK) == true

6. TRANSACTIONS:
   B.merkle_root == merkle_tree([tx.hash for tx in B.transactions])
   All transactions are valid
   First transaction is valid coinbase
   No double-spends within block (check_internal_double_spend in validation/utxo.rs)
```

### 5.4 Producer Selection (Deterministic Round-Robin)

> ⚠️ **Doc/code drift (INC-I-078 hotfix, 2026-05-17)**: this section described
> the legacy bond-weighted "ticket" rotation that was the original reference
> model. The **actual production scheduler** is the unweighted round-robin
> `slot % active_producer_count` implemented in
> `bins/node/src/node/production/scheduling.rs:446`. Bonds influence reward
> weight, attestation, and governance — they do **not** influence slot
> selection. The pseudocode below is kept for historical context (it
> documents the original Selection design); the rule that nodes actually
> enforce is captured in §5.3 step 4.

DOLI uses **deterministic round-robin rotation**, NOT probabilistic lottery.

**Epoch Bond Snapshot:** At each epoch boundary, the scheduler scans all Bond UTXOs
in the UTXO set and builds a `HashMap<PublicKey, u32>` (bond count per producer).
This snapshot is frozen for the entire epoch — mid-epoch AddBond/Withdrawal
transactions do NOT affect the current epoch's schedule. Changes take effect at
the next epoch boundary via `PendingProducerUpdate`.

```python
def epoch_bond_snapshot(utxo_set):
    """Scan Bond UTXOs once per epoch (~10ms at 250K producers)."""
    snapshot = {}
    for utxo in utxo_set.all_bond_utxos():
        pubkey = utxo.pubkey_hash
        snapshot[pubkey] = snapshot.get(pubkey, 0) + 1
    return snapshot

def selected_producer(slot, active_producers, bond_snapshot):
    """
    Deterministic rotation based on bond count (tickets).

    Example with Alice:1, Bob:5, Carol:4 bonds (total 10):
      Tickets: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
      Slot 0 → Alice, Slot 1-5 → Bob, Slot 6-9 → Carol

    Bob ALWAYS produces exactly 5 of every 10 blocks. No variance, no luck.
    """
    # Sort by pubkey for deterministic ordering
    sorted_producers = sorted(active_producers, key=lambda p: p.pubkey)

    # Calculate total tickets from epoch snapshot
    total_tickets = sum(bond_snapshot.get(p.pubkey, 0) for p in sorted_producers)

    # Deterministic selection: slot mod total_tickets
    ticket_index = slot % total_tickets

    # Find ticket owner
    cumulative = 0
    for producer in sorted_producers:
        cumulative += bond_snapshot.get(producer.pubkey, 0)
        if ticket_index < cumulative:
            return producer.pubkey
```

**Key properties:**
- NOT probabilistic: Each producer gets EXACTLY their proportion of slots
- Deterministic: All nodes compute the same result for any slot
- Equitable ROI: 10 bonds = 10x absolute return, same % ROI as 1 bond

### 5.4.1 Bond Stacking

Producers can stake multiple bonds (1-3,000) to increase their block production share:

| Parameter | Value | Notes |
|-----------|-------|-------|
| BOND_UNIT | 10 DOLI | 1 bond = 10 DOLI (1,000,000,000 base units) |
| MIN_BONDS | 1 | Minimum to register |
| MAX_BONDS | 3,000 | Anti-whale cap (30,000 DOLI max) |

**FIFO Withdrawal:** When withdrawing bonds, the oldest bonds are withdrawn first.
This ensures fair vesting calculation - bonds that have vested longer incur lower penalties.

### 5.4.2 Sequential Fallback Windows

When the primary producer misses their slot, a single fallback producer takes over in an exclusive 2-second sequential window. Only ONE rank is eligible at any given time:

| Window (ms) | Eligible Rank | Description |
|-------------|---------------|-------------|
| 0 -- 1,999 | 0 | Primary producer only |
| 2,000 -- 3,999 | 1 | Single fallback (only if rank 0's block not seen via gossip) |

2 ranks x 2,000ms = 4,000ms. The remaining 6,000ms of the slot is empty if both ranks miss. With IP colocation fix, gossip propagates blocks in <100ms, so rank 1 will always see rank 0's block in time, eliminating competing blocks.

**Constants:**
- `FALLBACK_TIMEOUT_MS = 2,000` -- duration of each exclusive window
- `MAX_FALLBACK_RANKS = 2` -- total number of ranked producers per slot (primary + single fallback)
- `MAX_FALLBACK_PRODUCERS = 2` -- maximum producers per slot
- `MAX_DRIFT_MS = 200` -- maximum clock drift tolerance (ms)

**History:** Previously set to 5 ranks (filling the entire 10s slot), reduced to 2 to eliminate fork fragmentation observed at 22+ nodes. With reliable gossip, a single fallback is sufficient to cover offline primaries without risking competing blocks.

**Fallback producer selection:** Each rank gets an evenly-distributed offset in the ticket space:
```
offset(rank) = total_tickets * rank / MAX_FALLBACK_RANKS
ticket(slot, rank) = (slot + offset(rank)) % total_tickets
```

This ensures fallback producers are spread across the producer set, not consecutive.

**Example distribution:**
```
Producer  Bonds   Tickets   Blocks/100   ROI/Bond
────────────────────────────────────────────────
Alice       1        1          1         1.0
Bob         5        5          5         1.0
Carol       4        4          4         1.0
Total      10       10         10         1.0 (equal)
```

**ROI Calculation:**
- Alice: 1 bond → 1 block/cycle → ROI = 1/1 = 1.0
- Bob: 5 bonds → 5 blocks/cycle → ROI = 5/5 = 1.0
- Carol: 4 bonds → 4 blocks/cycle → ROI = 4/4 = 1.0

All producers earn the **same percentage return** on their investment.

### 5.5 Chain Selection (Weight-Based Fork Choice)

DOLI uses a weight-based fork choice rule. The chain with the highest accumulated producer weight wins:

```python
def should_reorg(current_chain, new_chain):
    current_weight = accumulated_weight(current_chain.tip)
    new_weight = accumulated_weight(new_chain.tip)
    return new_weight > current_weight

def accumulated_weight(block):
    if block.is_genesis():
        return 0
    return accumulated_weight(block.parent) + block.producer.effective_weight
```

**Weight calculation (seniority only, discrete yearly steps):**
- Year 1: weight = 1
- Year 2: weight = 2
- Year 3: weight = 3
- Year 4+: weight = 4 (maximum)

**Important distinction:**
- Weight is based on **seniority only** (years active)
- Bond count affects **slot allocation** (more bonds = more slots per cycle)
- Bond count does NOT affect weight

**No activity penalty:**
- Producers who miss slots simply miss rewards
- No slashing or weight reduction for inactivity
- Only slashable offense: double production (equivocation)

This prevents Sybil attacks where an attacker creates many low-weight blocks.

### 5.6 Emission Schedule

```
def block_reward(height):
    era = height // SLOTS_PER_ERA
    return INITIAL_REWARD >> era   // Right shift = halving

INITIAL_REWARD = 100_000_000      // 1.0 DOLI (8 decimals)
```

| Era | Block Reward (base units) | Block Reward (DOLI) |
|-----|---------------------------|---------------------|
| 0   | 100,000,000               | 1.0                 |
| 1   | 50,000,000                | 0.5                 |
| 2   | 25,000,000                | 0.25                |
| 3   | 12,500,000                | 0.125               |
| ... | ...                       | ...                 |

---

## 6. Producer Registration

### 6.1 Registration Transaction

```
registration_tx = {
    version: 1,
    type: 1,
    inputs: [...],               // To pay fee
    outputs: [
        {
            output_type: 1,      // BOND
            amount: bond_amount(height),
            pubkey_hash: HASH(producer_pubkey),
            lock_until: height + LOCK_DURATION
        },
        ...                      // Change outputs
    ],
    extra_data: {
        public_key: 32 bytes,
        epoch: uint32,
        vdf_output: bytes,
        vdf_proof: bytes,
        prev_registration_hash: 32 bytes,  // Chain to previous registration
        sequence_number: uint64,           // Monotonic counter
        bond_count: uint32,                // Initial bond count at registration
        bls_pubkey: 48 bytes,              // BLS12-381 public key for aggregate attestations (optional, default empty)
        bls_pop: 96 bytes                  // BLS proof-of-possession over bls_pubkey (optional, default empty)
    }
}
```

### 6.2 Bond Amount

```
def bond_amount(bond_count):
    return bond_count * BOND_UNIT

BOND_UNIT = 1_000_000_000        // 10 DOLI per bond
MAX_BONDS = 3_000                // Maximum bonds per producer (30,000 DOLI max)
LOCK_DURATION = VESTING_PERIOD_SLOTS  // Mainnet: 4 years (4 × 1-year quarters); Testnet: 1 day (4 × 6h quarters)
```

**Bond tracking:** Registration creates Bond UTXOs (one per bond unit) with
`creation_slot` in extra_data. The bond count for scheduling is derived from
the UTXO set at each epoch boundary (epoch bond snapshot), not stored in
ProducerInfo. See Section 5.4 for the epoch snapshot mechanism.

### 6.3 Registration VDF

```
reg_input = HASH("DOLI_VDF_REGISTER_V1" || public_key || epoch)

// Breakdown:
// "DOLI_VDF_REGISTER_V1" = 20 bytes (domain separator)
// public_key   = 32 bytes
// epoch        = 4 bytes (uint32 LE)
// Total: 56 bytes before hashing

vdf_output = VDF(reg_input, T_REGISTER(epoch))
```

### 6.4 Registration Difficulty

Registration VDF is **fixed** at 1,000 iterations (minimal computation) for all networks.
Bond is the primary Sybil defense. The VDF is a lightweight barrier, not the primary defense.

```
T_REGISTER_BASE = 1,000       // Minimal (bond provides real Sybil protection)
T_REGISTER_CAP  = 5,000,000   // Consensus constant cap (not used by network defaults)
R_TARGET = 10                  // Target registrations per epoch (fee calculation)
vdf_register_iterations = 1,000  // All networks via NetworkParams
```

**Note:** The consensus constant `T_REGISTER_CAP` is 5,000,000 but all network defaults override `vdf_register_iterations` to 1,000. The bond requirement (10 DOLI per bond on mainnet, 1 DOLI on testnet/devnet) provides economic Sybil protection at scale.

### 6.5 Registration Validity

A registration is valid if:

1. VDF proof verifies with `T_REGISTER(declared_epoch)`
2. Declared epoch is current or previous
3. Public key is not already registered — TWO separate checks, both required:
   a. not in the ACTIVE producer set (`producer already registered`)
   b. not in `ProducerSet::pending_updates`, i.e. a registration that is mined but
      not yet flushed at the epoch boundary (`producer already has a pending
      registration`). Producer mutations are epoch-deferred, so a producer stays
      invisible to (a) for up to a full epoch after its registration mines.
   Both checks are enforced at block validation AND at mempool admission. Admission
   additionally treats a still-unmined registration held in the local mempool as
   pending (node-local policy; it does not affect block validity).
4. Bond output has correct amount and lock duration
5. Fee is sufficient
6. `bond_count` is in range [1, MAX_BONDS] (consensus-critical for producer selection)
7. Registration chain: `prev_registration_hash` and `sequence_number` are valid

### 6.6 Producer Activation

Producer becomes eligible for scheduling after `ACTIVATION_DELAY` (10 blocks, ~100 seconds) from registration height. Genesis producers (registered_at == 0) are exempt from the delay. This block-count-based delay ensures all nodes have received and confirmed the registration block before the producer enters the scheduling pool.

### 6.7 Inactivity Rule

```
if producer.consecutive_misses >= MAX_FAILURES:
    producer.status = INACTIVE
    // Bond remains locked
    // Must re-register with new VDF to reactivate

MAX_FAILURES = 50
```

---

## 7. Network Protocol

DOLI uses **libp2p** for all P2P networking. This provides a robust, modular networking stack with built-in support for NAT traversal, peer discovery, and encrypted connections.

### 7.1 Transport Layer

- **Protocol**: libp2p with Noise encryption and Yamux multiplexing
- **Discovery**: Kademlia DHT (`/doli/kad/1.0.0`) for peer discovery, 60s bootstrap interval
- **Gossip**: GossipSub for block and transaction propagation
- **Sync**: Request-response protocol for block synchronization
- **Identify**: Peer address exchange for DHT population

#### Connection Limits (Two-Tier Model)

Peer management is split into two layers:

| Layer | Limit | Purpose |
|-------|-------|---------|
| **Transport** (libp2p ConnectionLimits) | `max_peers * 1.5` in/out | Hard ceiling on TCP connections. Set higher than application limit to allow headroom for new peer evaluation. |
| **Application** (peers HashMap + gossipsub scoring) | `max_peers` | When full, evicts the peer with the lowest gossipsub score. Producers naturally retain slots (high P2 first-message-delivery score). |

Without transport headroom, `ConnectionLimits` rejects connections at TCP level before the application-layer scoring logic can evaluate them — the eviction code becomes dead code.

| Network | max_peers | Transport limit | Override |
|---------|-----------|-----------------|----------|
| Mainnet | 50 | 75 | `DOLI_MAX_PEERS` |
| Testnet | 25 | ~37 | `DOLI_MAX_PEERS` |
| Devnet | 150 | 225 | `DOLI_MAX_PEERS` |

Per-peer limit: 2 established connections (handles simultaneous-dial race and DCUtR hole-punching).

#### Peer Discovery Flow

```
1. Node starts, dials bootstrap nodes (explicit)
2. Bootstrap responds → Identify exchanges addresses
3. Addresses added to Kademlia DHT routing table
4. Every 60s: kademlia.bootstrap() refreshes routing table
5. New peers discovered → dialed directly (not through bootstrap)
6. Peer cache persisted to disk for fast restart recovery
```

Bootnodes are introduction points, not permanent hubs. A node needs one successful bootstrap connection to discover the rest of the network via DHT.

### 7.2 GossipSub Topics

| Topic | Content | Purpose |
|-------|---------|---------|
| `/doli/blocks/1` | Block headers + bodies | New block announcements |
| `/doli/txs/1` | Transactions | Transaction propagation |
| `/doli/producers/1` | Producer announcements | Bootstrap protocol |
| `/doli/votes/1` | Update votes | Governance veto system |
| `/doli/heartbeats/1` | Presence heartbeats | Weighted rewards |
| `/doli/t1/blocks/1` | Block propagation | Tier 1 dense mesh (validators only) |
| `/doli/headers/1` | Lightweight headers | All tiers (Tier 3 header-only validation) |
| `/doli/attestations/1` | Attestations | Tier 1 + Tier 2 (finality gadget) |

Topics do NOT include `network_id`. Network isolation is achieved via genesis hash check during status exchange (see section 8.3).

**Message validation (INC-I-114):** GossipSub is configured with `validate_messages=true`. Every received message is held un-forwarded until the application calls `report_message_validation_result()` with a verdict:

- **Block-body topics** (`/doli/blocks/1`, `/doli/t1/blocks/1`, `/doli/r{N}/blocks/1`): The application deserializes the message as a `Block` and applies staleness classification:
  - `Accept` — block slot is within `STALE_BLOCK_SLOT_THRESHOLD` (6) slots of the current wall-clock slot. Forwarded to mesh peers.
  - `Ignore` — block is stale (slot < wall_clock_slot - 6). Dropped without peer-score penalty. Prevents the dedup-cache-expiry amplification storm where old blocks were re-forwarded as fresh.
  - `Reject` — bytes cannot be deserialized as a `Block`. P4 peer-score penalty applied to sender.
- **Producer announcements** (`/doli/producers/1`): `Accept`/`Ignore` via `classify_producer_gossip()` (GSet with a bounded-age freshness bound); never `Reject`.
- **Stateful re-forward topics** (`/doli/attestations/1`, `/doli/heartbeats/1`, `/doli/headers/1`, `/doli/votes/1`, `/doli/txs/1`): as of **INC-I-142** these are NO LONGER `Accept`-by-default. Every subscribed topic is routed through the unified `classify_gossip()` gate (`crates/network/src/gossip/staleness.rs`), which replaced the fleet-wide re-forward storm on un-gated topics (daily/periodic CPU + symmetric network spike + `Unexpected delivery trace` log flood):
  - **Exhaustive-enum dispatch (no Accept-by-default).** `GossipTopic::from_topic_str()` maps a subscribed topic string to a `GossipTopic` variant, and `classify_gossip()` matches it with NO wildcard arm — adding a topic forces a classifier decision at compile time. An unrecognized or dynamic topic string returns `None` and is handled by local policy, never silently `Accept`ed through a classifier.
  - **PRIMARY identity dedup (storm-closer).** Each classifier computes an identity key `blake3(topic_discriminant || raw_message_bytes)` and consults a shared, bounded `SeenCache` (`SEEN_CACHE_TTL_SECS = 180s`, capacity `16_384` entries, drop-oldest). A key already present within TTL → `Ignore`; otherwise it is recorded on the first `Accept`. This — not libp2p's 60s `duplicate_cache_time` — is what closes the 60–120s re-delivery amplification window, independent of the libp2p dedup cache.
  - **SECONDARY semantic age filter (generous).** After dedup, a message that is fully no-longer-actionable is `Ignore`d: attestations, heartbeats, and headers by slot age (`ATTEST_STALE_SLOTS = 12`, `HEARTBEAT_STALE_SLOTS = 6`, `STALE_BLOCK_SLOT_THRESHOLD = 6`, each vs. the wall-clock slot), votes by timestamp age (`VOTE_MAX_AGE_SECS = 7 days`). Transactions carry no embedded age, so batch-level identity dedup is the whole gate. This filter is deliberately wide — its only job is to bound the cache and drop truly-ancient messages, not to close the storm.
  - **Verdict domain (Accept | Ignore, never Reject; fail-open).** These five topics return ONLY `Accept` or `Ignore` — NEVER `Reject`. Every classifier fails OPEN (`Accept`) on decode failure or clock-unavailable (`genesis_time == 0`), so a serialization/clock drift can never `Reject` an honest peer and trigger the P4 → mesh-expulsion cascade (INV-NETWORK-002). Headers specifically decode via `BlockHeader::deserialize`, never `Block::deserialize` (P0-001).
  - **Security note (INV-NETWORK-004).** Dedup keys hash the FULL raw message bytes, never unauthenticated semantic sub-fields (e.g. `(attester, block_hash)`, `(producer, slot)`, or a `txid` that excludes the signature). A forged message copying a victim's semantic fields but flipping any byte gets a DIFFERENT key, so an attacker cannot pre-seed the cache to suppress a genuine message before it propagates (INC-I-142 SEC-LOGIC-001/002; also resolves the BLS/non-BLS collision SEC-CONSENSUS-003).

### 7.3 Request-Response Protocols

| Protocol | Request | Response |
|----------|---------|----------|
| `/doli/status/1.0.0` | Status request (version, network_id, genesis_hash, producer_pubkey?) | Status response (version, network_id, genesis_hash, best_height, best_hash, best_slot, producer_pubkey?) |
| `/doli/sync/1.0.0` | GetHeaders, GetBodies, GetBlockByHeight, GetBlockByHash, GetStateSnapshot, GetStateRoot, GetHeadersByHeight | Headers, bodies, blocks, state snapshots, state root hashes |
| `/doli/txfetch/1.0.0` | Transaction hashes (max 50) | Full transactions from mempool |

### 7.4 Connection Flow

```
Initiator                     Responder
    |                             |
    |--- Noise handshake -------->|
    |<-- Noise handshake ---------|
    |--- Status exchange -------->|
    |<-- Status exchange ---------|
    |--- (sync if needed) ------->|
    |<-- (gossip subscribed) -----|
    |                             |
```

### 7.5 Block Propagation

New blocks are propagated via GossipSub with application-level validation:

```
Producer                      Network                     Receiving Node
    |                            |                             |
    |-- publish to /blocks ----->|                             |
    |                            |-- deliver (held) ---------> |
    |                            |                    classify_block_gossip()
    |                            |                    Accept/Ignore/Reject
    |                            | <-- report_result --------- |
    |                            | (forward if Accept)         |
    |                            |                             |
```

The `validate_messages=true` setting ensures that gossipsub does not auto-forward messages. Without this, after the duplicate cache expires (60s), a re-gossiped copy of a stale block would pass the dedup check and be auto-forwarded to all mesh peers, causing a fleet-wide amplification storm (INC-I-114).

The staleness threshold of 6 slots (60s at SLOT_DURATION=10s) tolerates clock skew and propagation delay while filtering blocks from the INC-I-114 storm pattern (3-9 hours old). When `genesis_time` is unset (0), staleness filtering is disabled (fail-open) to prevent silent gossip death from misconfiguration.

**Load-shedding event queue (INC-I-114 Part B, M1).** After staleness filtering, accepted blocks are enqueued to the node event channel via a non-blocking `try_send`. If the bounded channel is full (consumer behind), the block is dropped and counted by `GossipShedMetrics` rather than suspending the swarm task. This prevents libp2p's internal message buffer from growing unboundedly during a gossip flood. Only block-topic sends use this load-shedding path; non-block event sends (transactions, headers, votes, heartbeats, attestations) remain on their backpressure-aware `.send().await` path. Dropped blocks are recoverable via the sync protocol (GetBlocks request-response).

**Memory watchdog (INC-I-114 Part B, M2).** A periodic sampler (5s interval) checks process RSS and trips a shared `AtomicBool` flag when memory crosses a configurable soft threshold (`DOLI_MEMORY_WATCHDOG_BYTES`, default 0 = disabled). When tripped, the gossip block handler sheds ALL accepted blocks after `report_message_validation_result()` but before enqueue, preventing OOM under sustained memory pressure. The watchdog fails open: on non-Linux platforms or when the sampler is unavailable, the flag stays false (never sheds). Recovery is automatic — when RSS drops below the threshold, the flag clears and normal gossip processing resumes. Implementation: `crates/network/src/watchdog.rs`.

**Construction-time hardening invariant (INV-NETWORK-002, INC-I-114 Part B, M3).** The gossipsub `Config` is validated at construction time by `assert_gossip_hardening_invariant()`. If `flood_publish=true` or `duplicate_cache_time <= 30s` (the `AGGRESSIVE_DEDUP_THRESHOLD`), then both `validate_messages=true` AND a bounded event queue must be present, or the node fails to start with a descriptive error citing INV-NETWORK-002. This compile-time gate prevents re-introduction of the aggressive-propagation-without-validation configuration that caused 5 fleet-wide incidents (INC-I-009, INC-I-014, INC-I-118, INC-I-120, INC-I-114). Implementation: `crates/network/src/gossip/config.rs`.

**Outbound sync-request governor (INV-SYNC-009, INC-I-120, Layer 1).** The gossip path (above) had flood control but the adjacent *sync* request/response path had inbound serving caps and ZERO outbound rate governance. A natural fork therefore self-amplified into a ~40 req/s busy-retry loop (3.5M req/node/day) → fleet-wide resource collapse. Every outbound sync request funnels through the single `NetworkCommand::RequestSync` chokepoint, which now applies a per-peer + global token-bucket governor (`max_sync_requests_per_second`, default 10/peer, 60/s global; reuses `crates/network/src/rate_limit.rs`). When a bucket is empty the request is **dropped** (not queued) — the sync state machine re-derives it on the next tick at a governed rate. Only retry-storm classes are governed (`GetHeaders`, `GetBodies`, `GetBlockByHeight`); recovery/canonical-critical classes (`GetStateSnapshot`, `GetStateRoot`, `GetHeadersByHeight`, `GetBlockByHash` orphan-chase, `DirectAttestation`) bypass the governor entirely via `SyncRequest::is_rate_governed` (guardrail G1, from INC-I-049 — never throttle canonical-block delivery or fork-recovery). Complementary fix: a `"busy"` sync response now applies cooperative backoff (blacklist peer + go Idle) instead of immediately re-issuing `start_sync()`, which was the self-amplification trigger.

**Sustained stuck-fork recovery (INV-FORK-001, INC-I-120, Layer 2).** A node genuinely forked on a small gap never satisfies the RecoveryCoordinator's `recently_synced()` precondition for `ShallowRollback` (its tip is stale by definition), so it previously looped `HeaderFirstSync` forever against peers that don't recognize its tip. `cleanup.rs` now re-raises a guarded stuck-fork signal (guardrail G3: ≥300s no apply AND small gap AND ≥3 consecutive empty-header replies = local tip ∉ peer-majority chain), and `periodic.rs` wires the consumed signal to the coordinator as `RecoveryEvidence::StuckFork`, which escalates to a finality-guarded `ShallowRollback` (honors INV-SYNC-001/004/008: never roll back below finality). This completes the unfinished half of INC-I-090 (detector built, action never wired).

**Finality-wedged sibling fetch + snap-anchor integrity (INC-I-143).** Two gaps in the recovery layer above are closed. (1) *SiblingFetch (D4):* when a `StuckFork` coincides with `finality == local_tip`, the depth-1 rollback target falls below finality and the INV-SYNC-008 guard correctly refuses it — the coordinator previously returned `None` and re-refused every tick (454-refusal livelock). It now emits `RecoveryAction::SiblingFetch { height: local_tip }`, which sends `GetBlockByHeight{local_tip}` to up to 3 top peers (the recovery-critical class, ungoverned) to pull the competing sibling onto the wedged node; the sibling is re-evaluated by node-local fork choice via `plan_reorg`. Bounded to `SIBLING_FETCH_MAX(3)` consecutive attempts (any other concrete action resets the budget), then falls through to standard escalation. The finality guard's strict `<` is UNCHANGED. (2) *Snap-anchor admission (D1/D2):* `handle_snap_snapshot` now refuses a snapshot whose served `response_root` ≠ the quorum-agreed root, AND refuses an anchor whose `(block_hash, block_height)` pair is not corroborated by a STATUS quorum of connected peers — deriving the install height from quorum evidence rather than a single peer's uncorroborated current tip. Either refusal increments `snap.integrity_refusals` and falls back to an alternate peer (then header-first). Pre-fix, an accepted root mismatch and a single-peer height claim spliced a forked anchor at a −1 offset with a 45-block hole (candidate `INV-SYNC-011` extension).


---

## 8. Networks

DOLI defines three networks with distinct parameters. A single binary connects to any network via the `--network` flag.

**Development workflow:**
```
Devnet (local development) → Testnet (public testing) → Mainnet (production)
```

- **Mainnet**: Production network with real economic value
- **Testnet**: Public test network for integration testing before mainnet
- **Devnet**: Local development network with fast blocks and minimal requirements

### 8.1 Network Identifiers

| Network | ID | Address Prefix | P2P Port | RPC Port |
|---------|-----|----------------|----------|----------|
| Mainnet | 1   | `doli`  | 30300 | 8500 |
| Testnet | 2   | `tdoli` | 40300 | 18500 |
| Devnet  | 99  | `ddoli` | 50300 | 28500 |

### 8.2 Network Parameters

| Parameter | Mainnet | Testnet | Devnet | Configurable |
|-----------|---------|---------|--------|--------------|
| Genesis Time | 1774540572 (2026-03-24) | 1774749145 (testnet v96) | Dynamic | Devnet only |
| Slot Duration | 10s | 10s | 10s | Devnet only |
| Max Peers | 50 | 25 | 150 | All (`DOLI_MAX_PEERS`) |
| Transport Limit | 75 (1.5x) | ~37 (1.5x) | 225 (1.5x) | Derived from max_peers |
| P2P Port | 30300 | 40300 | 50300 | All |
| RPC Port | 8500 | 18500 | 28500 | All |
| Metrics Port | 9000 | 19000 | 29000 | All |
| Bond Unit | 10 DOLI | 1 DOLI | 1 DOLI | Devnet only |
| Initial Reward | 1 DOLI | 1 DOLI | 20 DOLI | Devnet only |
| VDF Iterations (block) | 1,000 | 1,000 | 1 | Devnet only |
| Heartbeat VDF | 1,000 | 1,000 | 1,000 | Devnet only |
| Blocks/Year | 3,153,600 | 3,153,600 | 144 | Devnet only |
| Reward Epoch | 360 blocks | 36 blocks | 4 blocks | Devnet only |
| Bootstrap Blocks | 60,480 | 60,480 | 60 | Devnet only |
| Unbonding Period | 60,480 blocks (~7d) | 72 blocks (~2 epochs) | 60 blocks | Devnet only |
| Vesting Quarter | 3,153,600 slots (1yr) | 2,160 slots (6h) | 60 slots (10min) | Devnet only |
| Veto Period | 5 min | 5 min | 60s | All |
| Fallback Ranks | 2 | 2 | 2 | All |
| DeFi Activation Height | `u64::MAX` | `u64::MAX` | `u64::MAX` | Non-mainnet (`DOLI_DEFI_ACTIVATION_HEIGHT`) |
| Data Directory | `~/.doli/mainnet/` | `~/.doli/testnet/` | `~/.doli/devnet/` | - |
| Config File | `.env` in data dir | `.env` in data dir | `.env` in data dir | - |

**DeFi activation gate (INC-I-088 Phase 0)**: The 11 DeFi transaction types
(`CreatePool=19`, `AddLiquidity=20`, `RemoveLiquidity=21`, `Swap=22`,
`CreateLoan=24`, `RepayLoan=25`, `LiquidateLoan=26`, `LendingDeposit=27`,
`LendingWithdraw=28`, `FractionalizeNft=29`, `RedeemNft=30`) are rejected
when `current_height < defi_activation_height` with stable error code
`DEFI_NOT_ACTIVATED`. Mainnet default is `u64::MAX` (always disabled) until
the DeFi subsystem is audited and un-gated. Mempool, block-assembly, and
block-apply paths all enforce identically. Pairs with the
`OutputType::Collateral` hard-freeze in `verify_input_conditions`
(`[ERRTX-DEFI001]`) to also block any spend of pre-existing Collateral
UTXOs.

**VDF note:** Block VDF iterations are set to 1,000 for all production networks (minimal computation). The bond requirement is the primary Sybil defense. The consensus constant `T_BLOCK = 800,000` exists but network defaults override it to 1,000 via `NetworkParams`. The consensus constant `T_REGISTER_CAP = 5,000,000` exists but is not currently applied by network defaults (reserved for future tightening).

### 8.2.1 Environment Configuration

Network parameters can be customized via `.env` files in the data directory:

```bash
# Location: ~/.doli/{network}/.env
# Example for devnet: ~/.doli/devnet/.env

DOLI_P2P_PORT=51303
DOLI_RPC_PORT=29545
DOLI_MAX_PEERS=100              # Application-layer peer limit (transport = 1.5×)
DOLI_BLOCKS_PER_REWARD_EPOCH=2
```

**.env file lookup**: The node searches for `.env` in two locations:
1. `{data_dir}/.env` — The directory specified by `--data-dir` (or the network default)
2. `~/.doli/{network}/.env` — Fallback to the network root directory

This fallback ensures that nodes started with custom `--data-dir` paths (e.g., `--data-dir ~/.doli/devnet/data/node5`) still pick up the shared network `.env` file.

**Mainnet Security**: The following parameters are **locked for mainnet** and cannot be overridden:
- `DOLI_SLOT_DURATION`, `DOLI_GENESIS_TIME`
- `DOLI_BOND_UNIT`, `DOLI_INITIAL_REWARD`
- `DOLI_VDF_ITERATIONS`, `DOLI_HEARTBEAT_VDF_ITERATIONS`
- `DOLI_BLOCKS_PER_YEAR`, `DOLI_BLOCKS_PER_REWARD_EPOCH`
- `DOLI_COINBASE_MATURITY`, `DOLI_UNBONDING_PERIOD`

Attempting to override locked parameters on mainnet logs a warning and uses hardcoded values.

**Precedence** (highest to lowest):
1. **Embedded binary** (mainnet ONLY — chainspec is compiled in, disk/CLI overrides disabled)
2. **CLI flags** (e.g., `--p2p-port`)
3. **Chainspec direct injection** (`--chainspec` or `{data_dir}/chainspec.json`) — testnet/devnet only
4. **Parent process environment variables**
5. **`.env` file variables**
6. **Network defaults** (hardcoded in `consensus.rs`)

**SECURITY (mainnet)**: The mainnet chainspec is always loaded from the embedded binary via
`include_str!`. The `--chainspec` flag and disk `chainspec.json` files are ignored. This
prevents genesis-time-hijack attacks where a tampered chainspec on disk could cause slot
schedule divergence and chain forks.

Chainspec parameters are applied in two phases:
- **Phase 1 (env defaults)**: Before the OnceLock initializes, chainspec fields are set as lowest-priority env var defaults (backward compatibility).
- **Phase 2 (direct injection)**: After `ConsensusParams::for_network()`, `apply_chainspec()` overwrites params directly from the chainspec. This is authoritative — it guarantees chainspec values are used regardless of OnceLock state.

The following chainspec fields are applied directly:

| Chainspec Field | ConsensusParams Field |
|----------------|----------------------|
| `consensus.slot_duration` | `slot_duration` |
| `consensus.bond_amount` | `initial_bond` |
| `consensus.slots_per_epoch` | `slots_per_reward_epoch` |
| `genesis.initial_reward` | `initial_reward` |
| `genesis.timestamp` (non-zero) | `genesis_time` |

Mainnet chainspecs are skipped entirely (defense-in-depth).

### 8.3 Network Isolation

Networks are isolated at multiple levels:

1. **Genesis hash**: Validated during status exchange — peers with mismatched genesis are rejected
2. **Network ID**: Exchanged during peer status exchange — must match local network
3. **Address prefix**: Prevents cross-network address confusion
4. **Ports**: Different default ports allow running multiple networks simultaneously

Note: GossipSub topic names do NOT include network ID. Network isolation relies on the genesis hash and network ID checks during the status handshake protocol (`/doli/status/1.0.0`).

### 8.4 Peer Validation

During connection handshake, nodes exchange status messages:

```
status_request = {
    version:         uint32,
    network_id:      uint32,          // Must match local network
    genesis_hash:    32 bytes,        // Must match local genesis
    producer_pubkey: 32 bytes | null  // Optional: producer pubkey for bootstrap discovery
}
```

Peers with mismatched `network_id` or `genesis_hash` are immediately disconnected.

**Protocol version enforcement:**

The `version` field carries the node's protocol version (`CURRENT_PROTOCOL_VERSION`, currently `2`). Each node defines a `MIN_PEER_PROTOCOL_VERSION` (currently `1`). If a peer's version is below this minimum, the connection is rejected with a `VersionMismatch` event and the peer is disconnected. This allows the network to partition old nodes after a breaking upgrade by bumping `MIN_PEER_PROTOCOL_VERSION`.

Protocol version history:
| Version | Description |
|---------|-------------|
| 1 | Original (version field present but never checked) |
| 2 | Version enforcement in status handshake |

### 8.5 Bootstrap Nodes

| Network | Bootstrap Nodes |
|---------|-----------------|
| Mainnet | `/dns4/seed1.doli.network/tcp/30300`<br>`/dns4/seed2.doli.network/tcp/30300`<br>`/dns4/seeds.doli.network/tcp/30300` |
| Testnet | `/dns4/bootstrap1.testnet.doli.network/tcp/40300`<br>`/dns4/bootstrap2.testnet.doli.network/tcp/40300`<br>`/dns4/seeds.testnet.doli.network/tcp/40300` |
| Devnet  | None (local development) |

---

## 9. Test Vectors

### 9.1 SEED Hash (Slot 0)

```
Input:
  literal     = "SEED" = 0x53 0x45 0x45 0x44
  prev_hash   = 0x00 * 32
  slot        = 0 = 0x00000000

Concatenation (40 bytes):
  53454544
  0000000000000000000000000000000000000000000000000000000000000000
  00000000

Result:
  f3b4b63bfa289f7b4b2f11f08cfc26bd38ccdbdd9dae33ef9b77c1fc3b96ebb2
```

### 9.2 SEED Hash (Slot 1)

```
Input:
  literal     = "SEED"
  prev_hash   = 0x00 * 32
  slot        = 1 = 0x01000000 (little-endian)

Result:
  ac1d2a15e55cc413c69036ba29cd08066a560a5bf152ac89a35089eae1fd6bbe
```

### 9.3 SEED Hash (Non-zero prev_hash)

```
Input:
  literal     = "SEED"
  prev_hash   = 0x01 followed by 31 zeros
  slot        = 0

Result:
  1cf7ca92b30ec36c921c1f0f899bb6304b9bb9606ef986ed23afe3baa6b265d1
```

### 9.4 REG Hash

```
Input:
  literal     = "DOLI_VDF_REGISTER_V1" (20 bytes)
  public_key  = 0x00 * 32
  epoch       = 0 = 0x00000000

Concatenation (56 bytes):
  444F4C495F5644465F52454749535445525F5631  (DOLI_VDF_REGISTER_V1)
  0000000000000000000000000000000000000000000000000000000000000000
  00000000

Result:
  [compute with BLAKE3-256]
```

### 9.5 BLK Hash

```
Input:
  literal     = "DOLI_VDF_BLOCK_V1" (17 bytes)
  prev_hash   = 0x00 * 32
  merkle_root = 0x00 * 32
  slot        = 0 = 0x00000000
  producer    = 0x00 * 32

Concatenation (117 bytes):
  444F4C495F5644465F424C4F434B5F5631  (DOLI_VDF_BLOCK_V1)
  0000...0000  (32 bytes prev_hash)
  0000...0000  (32 bytes merkle_root)
  00000000     (4 bytes slot)
  0000...0000  (32 bytes producer)

Result:
  [compute with BLAKE3-256]
```

---

## Parameters Summary

| Parameter          | Value                    |
|--------------------|--------------------------|
| BLOCK_VERSION      | 2                        |
| GENESIS_TIME       | Set at chain launch      |
| SLOT_DURATION      | 10 (all networks)      |
| SLOTS_PER_EPOCH    | 360                      |
| SLOTS_PER_ERA      | 12,614,400               |
| BOOTSTRAP_BLOCKS   | 60,480                   |
| DRIFT              | 1                        |
| NETWORK_MARGIN     | 1                        |
| T_BLOCK (consensus const) | 800,000            |
| VDF_ITERATIONS (network default) | 1,000 (bond is primary Sybil defense) |
| VDF_ITERATIONS_MIN | 100,000 (calibration bound)  |
| VDF_ITERATIONS_MAX | 100,000,000 (calibration bound) |
| VDF_TARGET_TIME_MS | 55                       |
| T_REGISTER_BASE    | 1,000 (minimal)          |
| T_REGISTER_CAP     | 5,000,000 (consensus constant, not used by defaults) |
| R_TARGET           | 10                       |
| INITIAL_REWARD     | 100,000,000 (1 DOLI)     |
| BOND_UNIT          | 1,000,000,000 (10 DOLI)   |
| MAX_BONDS_PER_PRODUCER | 3,000                |
| WITHDRAWAL_DELAY_SLOTS | 60,480 (~7 days)     |
| YEAR_IN_SLOTS      | 3,153,600                  |
| VESTING_QUARTER_SLOTS | 3,153,600 (mainnet=1yr, testnet=2,160=6h) |
| VESTING_PERIOD_SLOTS | 12,614,400 (mainnet=4yr, testnet=8,640=1d) |
| COMMITMENT_PERIOD  | 12,614,400 (= VESTING_PERIOD_SLOTS) |
| UNBONDING_PERIOD   | 60,480 (~7 days)         |
| ACTIVATION_DELAY   | 10 (blocks, ~100s)       |
| MAX_FAILURES       | 50                       |
| REWARD_MATURITY    | 6                        |
| BASE_BLOCK_SIZE    | 2,000,000                |
| MAX_BLOCK_SIZE_CAP | 32,000,000               |
| BLOCK_SIZE_GROWTH  | ×2 per era               |
| EXCLUSION_PERIOD   | 60,480 (~7 days)         |
| TOTAL_SUPPLY       | 2,522,880,000,000,000    |

| VETO_PERIOD        | 300 (5 min early*)       |
| GRACE_PERIOD       | 120 (2 min early*)       |
| VETO_THRESHOLD     | 40%                      |
| REQUIRED_SIGS      | 3 of 5                   |
| MIN_MAINTAINERS    | 3                        |
| MAX_MAINTAINERS    | 5                        |

**Vesting Penalties (mainnet: 4yr/1yr quarters, testnet: 1d/6h quarters):**

| Mainnet | Testnet | Penalty Rate |
|---------|---------|-------------|
| Y1 (0-1yr) | Q1 (0-6h) | 75%  |
| Y2 (1-2yr) | Q2 (6-12h) | 50% |
| Y3 (2-3yr) | Q3 (12-18h) | 25% |
| Y4+ (3yr+) | Q4+ (18h+) | 0%  |

---

## 10. Auto-Update System

### 10.1 Release Structure

```
release = {
    version: string,             // Semantic version
    binary_sha256: string,       // SHA-256 hash of binary (hex)
    binary_url_template: string, // URL with {platform} placeholder
    changelog: string,
    published_at: uint64,        // Unix timestamp
    signatures: signature[],
    target_networks: string[]    // From metadata.json; empty = all networks
}

signature = {
    public_key: string,          // Maintainer public key (hex)
    signature: string            // Signature over "version:binary_sha256"
}

metadata = {                     // metadata.json (optional release asset)
    version: string,
    networks: string[],          // ["mainnet"], ["testnet"], ["mainnet","testnet"]
    min_protocol_version: uint32 // Optional
}
```

**Network targeting:** Each GitHub Release may include a `metadata.json` asset specifying
which networks the release targets. If present, nodes filter releases by their `--network`.
If absent, the release targets all networks (backward compatibility).

### 10.2 Verification

Verification runs against a **resolved trust root** — keys, the threshold they must
meet, and where they came from — never against a bare key list. There is **no fallback
to the compiled bootstrap keys** (INC-I-172 F1): an empty or sub-threshold on-chain root
**fails closed** and refuses every release.

**Trust-root resolution** (`bins/node/src/updater/trust_root_wiring.rs`, the only place
in the node that makes this decision):

| on-chain `members` | `last_derived_height` | resolved root |
|---|---|---|
| non-empty | any | `OnChain(members, set.threshold)` — authoritative |
| empty | `0` | `Bootstrap(BOOTSTRAP_MAINTAINER_KEYS, REQUIRED_SIGNATURES)` — this node has never established an on-chain set (fresh install) |
| empty | `> 0` | `OnChain([], threshold)` — the set existed and was emptied. **Unusable; refuses.** This is the attack case, not a fresh node. |

A root that cannot be read at all (lock contention, unreadable
`maintainer_state.bin`) resolves to an unusable `OnChain` root or is fatal at startup.
"I could not check" is never "it is fine".

```
// Verification, given a resolved root
if root.threshold < 1 OR count(root.keys) < root.threshold:
    REFUSE (TrustRootUnavailable)      // never falls back to BOOTSTRAP_MAINTAINER_KEYS

message = version + ":" + binary_sha256      // binary_sha256 = sha256(CHECKSUMS.txt)

// DISTINCT-SIGNER count: outer loop over the ROOT's keys, inner loop over the
// release's signature entries, break on the first valid entry for that key.
// N signature entries produced by ONE key therefore count as 1.
valid_signers = 0
for key in root.keys:
    for sig in signatures:
        if sig.public_key == key (ASCII case-insensitive) AND verify(message, sig, key):
            valid_signers += 1
            break

release_valid = valid_signers >= root.threshold   // NOT a hardcoded 3
```

`root.threshold` is `MaintainerSet.threshold` for an `OnChain` root and
`REQUIRED_SIGNATURES` for a `Bootstrap` one.

**Artifact binding.** A valid signature proves the maintainers signed *something*; it
does not say *what* is being installed. Every install path additionally binds the
signature to the artifact before writing anything (INC-I-172 F1,
`crates/updater/src/install_gate.rs`). All four links are checked and any break blocks:

```
L1  SIGNATURES.json .version          == the release TAG being installed (modulo "v")
L2  SIGNATURES.json .checksums_sha256 == sha256(the CHECKSUMS.txt actually fetched)
L3  distinct valid signers            >= root.threshold        (the loop above)
L4  sha256(tarball)                   == the per-platform hash parsed from THAT CHECKSUMS.txt
```

Without L1/L2 the check is circular — both message operands would come from the same
file that carries the signatures, so a verbatim copy of any past genuine
`SIGNATURES.json` would authorise an arbitrary binary.

**Where each root applies:**

| Path | Root |
|---|---|
| node auto-update (`UpdateService`) | resolved per check; re-verified again immediately before install |
| `doli-node upgrade` / `update verify` / `update apply` | resolved from this host's `maintainer_state.bin` |
| `doli upgrade` (CLI) | `Bootstrap` — the CLI is not the node host and has no chain state |

### 10.3 Veto Voting

```
vote_message = {
    version: string,
    vote: uint8,                 // 0 = APPROVE, 1 = VETO
    producer_id: string,
    signature: bytes
}
```

Only active producers can vote. Votes propagate via gossip.

### 10.4 Veto Calculation

Veto is a **head count**. There is no seniority multiplier and no bond weighting: the
weighted machinery described here previously (`calculate_vote_weight`,
`seniority_multiplier`, `VoteTracker::*_weighted`) had no non-test callers and was
deleted in INC-I-172 F8. `crates/updater/src/verification.rs::calculate_veto_result`:

```
veto_percent = (veto_count * 100) / total_producers   // 0 when total_producers == 0

if veto_percent >= 40:
    update REJECTED
else:
    update APPROVED after the veto period
```

The veto period is the **configured** `UpdateConfig::veto_period_secs` (default
`VETO_PERIOD`), not a literal "7 days", and it is measured from
`PendingUpdate::first_notified_at` — the node-local moment this node first observed the
release. It is never measured from `Release::published_at`, which is attacker-supplied
and unsigned; a forged value would collapse the window to zero (INC-I-172 F7(b)).
Because the reference point is node-local, a mixed fleet simply has per-node windows.

### 10.5 Production Gating

Two independent mechanisms prevent outdated producers from creating blocks:

**Auto-update enforcement** (runtime): After a signed release is approved and the grace period expires, `is_production_allowed()` blocks production. Depends on the update service discovering the release from GitHub.

**Hard fork schedule** (compile-time): `HardForkSchedule::default_schedule()` contains a list of `(activation_height, min_version)` pairs baked into the binary. At each production tick, if `current_height >= activation_height` and the binary version is below `min_version`, production is blocked. This is deterministic — no external service dependency.

```
hardfork_entry = {
    activation_height: uint64,
    min_version: string,          // Semver (e.g., "5.0.0")
    consensus_changes: string[]   // Human-readable description
}

// Check on every production tick:
for fork in schedule:
    if current_height >= fork.activation_height
       AND binary_version < fork.min_version:
        BLOCK PRODUCTION
```

### 10.6 Network-Layer Version Enforcement

The status handshake (Section 8.4) enforces protocol version compatibility at connection time. Peers below `MIN_PEER_PROTOCOL_VERSION` are disconnected and receive an `IncompatibleVersion` peer scoring penalty (-200, instant disconnect threshold).

This provides three layers of version protection:
1. **Network layer**: Incompatible peers cannot connect (status handshake)
2. **Production layer**: Outdated producers cannot create blocks (hard fork schedule + auto-update)
3. **Validation layer**: Structurally incompatible blocks are rejected (block version check)

---

*For architecture overview, see [architecture.md](architecture.md)*
