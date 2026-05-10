---
name = "delegation"
description = "DOLI bond delegation system — delegate bond weight, revoke delegation, check status, reward splitting, lifecycle rules. Covers CLI commands, RPC fields, constants, and epoch-deferred processing."
trigger = "delegation|delegate bond|revoke delegation|delegation status|delegated_to|delegated_bonds|received_delegations|selection_weight|DELEGATE_REWARD_PCT|STAKER_REWARD_PCT|DELEGATION_UNBONDING_SLOTS|DelegateBondData|RevokeDelegationData"
---

# DOLI Bond Delegation

> Source of truth: code. CLI: `bins/cli/src/cmd_producer/delegation.rs`. Core types: `crates/core/src/transaction/data.rs`. Storage: `crates/storage/src/producer/`.

## Overview

Producers can delegate their bond weight to another producer (the "delegatee"). This increases the delegatee's effective selection weight for block production scheduling. Rewards are split: delegatee keeps 10%, delegator receives 90%.

- One active delegation per producer (must revoke before re-delegating)
- Epoch-deferred: delegation/revocation queued as PendingProducerUpdate, applied at next epoch boundary
- Revocation has an unbonding delay (DELEGATION_UNBONDING_SLOTS = 60,480 slots, ~7 days)
- Delegation state is cleaned up on exit, slash, or unbonding completion

## CLI Commands

### Delegate Bond Weight

```
doli producer delegate <DELEGATEE_PUBKEY> --bonds <N>
```

| Flag | Required | Description |
|------|----------|-------------|
| `<DELEGATEE_PUBKEY>` | yes | Delegatee's public key (64-char hex) |
| `--bonds <N>` / `-b <N>` | yes | Number of bonds to delegate (1-100) |

**Validations (CLI-side):**
- Bond count 1-100
- Cannot self-delegate
- Caller must be registered and active
- No existing active delegation (must revoke first)
- Sufficient available bonds (bond_count - delegated_bonds >= N)
- Delegatee must exist and be active

**Example:**
```bash
# Find a producer's pubkey
doli producer list --format json | jq '.[].publicKey'

# Delegate 5 bonds
doli producer delegate abc123...def456 --bonds 5
```

**Output:**
```
Delegate Bond Weight
------------------------------------------------------------

Delegating 5 bond(s) to:
  Delegatee: doli1qw3e...
  Value:     50 DOLI
  Reward split: delegatee keeps 10%, you receive 90%

Submitting delegation transaction...
Delegation submitted successfully!
TX Hash: a1b2c3...
Delegation will take effect at the next epoch boundary.
Estimated activation: ~45 minutes (Epoch 124, block 44640).
```

### Revoke Delegation

```
doli producer revoke-delegation
```

No arguments — revokes the caller's single active delegation.

**Validations (CLI-side):**
- Caller must be registered
- Must have an active delegation (delegated_to is Some)

**Example:**
```bash
doli producer revoke-delegation
```

**Output:**
```
Revoke Delegation
------------------------------------------------------------

Revoking delegation of 5 bond(s) from:
  Delegatee: doli1qw3e...

Note: Unbonding delay applies after revocation.

Submitting revocation transaction...
Revocation submitted successfully!
TX Hash: d4e5f6...
Revocation will take effect at the next epoch boundary.
```

### Delegation Status

```
doli producer delegation-status [--address <PUBKEY>]
```

| Flag | Required | Description |
|------|----------|-------------|
| `--address <PUBKEY>` / `-a` | no | Public key to check (default: wallet key) |

**Example:**
```bash
# Own delegation status
doli producer delegation-status

# Check another producer
doli producer delegation-status --address abc123...def456
```

**Output:**
```
Delegation Status
------------------------------------------------------------

Producer:         doli1abc...
Status:           active
Bond Count:       10
Selection Weight: 7 (effective)

Delegated To:     doli1qw3e...
Delegated Bonds:  3
Available Bonds:  7

Received Delegations: 5 total from 2 delegator(s)
  Delegator Hash                                   Bonds
  ------------------------------------------------------------
  a1b2c3d4e5f6...12345678                          3
  f9e8d7c6b5a4...87654321                          2
```

## RPC Fields (getProducer / getProducers)

The delegation state is returned in the ProducerResponse:

```json
{
  "publicKey": "abc...",
  "bondCount": 10,
  "delegatedTo": "def...",
  "delegatedBonds": 3,
  "receivedDelegations": [
    { "delegatorHash": "a1b2...", "bondCount": 3 },
    { "delegatorHash": "f9e8...", "bondCount": 2 }
  ],
  "selectionWeight": 12,
  "pendingUpdates": [
    { "updateType": "delegate_bond", "bondCount": 5 }
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `delegatedTo` | `string?` | Hex pubkey of delegatee (null if none) |
| `delegatedBonds` | `u32` | Bonds delegated away |
| `receivedDelegations` | `array` | List of {delegatorHash, bondCount} |
| `selectionWeight` | `u64` | Effective weight = own - delegated + received |

## Constants

| Constant | Value | Location |
|----------|-------|----------|
| `DELEGATE_REWARD_PCT` | 10 | `crates/core/src/consensus/constants.rs:524` |
| `STAKER_REWARD_PCT` | 90 | `crates/core/src/consensus/constants.rs:527` |
| `DELEGATION_UNBONDING_SLOTS` | 60,480 (~7 days) | `crates/core/src/consensus/constants.rs:530` |

## Transaction Types

| Type | ID | Inputs | Outputs | Extra Data |
|------|----|--------|---------|------------|
| DelegateBond | 13 | none | none | DelegateBondData (delegator, delegate, bond_count) |
| RevokeDelegation | 14 | none | none | RevokeDelegationData (delegator, delegate) |

Both are state-only transactions: no inputs, no outputs, no signing required.

## Selection Weight Formula

```
selection_weight = (bond_count - delegated_bonds) + sum(received_delegations[*].bond_count)
```

Weight is conserved across the network: delegation moves weight, it does not create it.

## Lifecycle Rules

1. **Delegate:** Queued as `PendingProducerUpdate::DelegateBond`, applied at epoch boundary
2. **Revoke:** Queued as `PendingProducerUpdate::RevokeDelegation`, applied at epoch boundary
3. **Exit/Slash cleanup:** `cleanup_all_delegations()` removes both incoming and outgoing delegations
4. **Unbonding completion:** Delegations cleaned when delegatee's unbonding completes
5. **Same-epoch delegate+revoke:** Net zero effect (both processed in FIFO order)
6. **Re-registration after exit:** Starts clean — no stale delegation state

## Code Map

| What | Where |
|------|-------|
| CLI handlers | `bins/cli/src/cmd_producer/delegation.rs` |
| CLI command defs | `bins/cli/src/commands.rs` (ProducerCommands::Delegate, RevokeDelegation, DelegationStatus) |
| CLI dispatch | `bins/cli/src/cmd_producer/dispatch.rs` |
| Transaction data types | `crates/core/src/transaction/data.rs` (DelegateBondData, RevokeDelegationData) |
| Transaction constructors | `crates/core/src/transaction/core.rs` (new_delegate_bond, new_revoke_delegation) |
| Structural validation | `crates/core/src/validation/tx_types.rs` (validate_delegate_bond_data, validate_revoke_delegation_data) |
| Storage ProducerInfo fields | `crates/storage/src/producer/types.rs` (delegated_to, delegated_bonds, received_delegations) |
| Storage delegation ops | `crates/storage/src/producer/set_lifecycle.rs` (delegate_bonds, revoke_delegation, cleanup_all_delegations) |
| selection_weight_at() | `crates/storage/src/producer/info.rs:390` |
| RPC ProducerResponse | `crates/rpc/src/types/producer.rs` |
| RPC population | `crates/rpc/src/methods/producer.rs` |
| Delegation tests (27) | `crates/storage/src/producer/tests_delegation.rs` |
| Constants | `crates/core/src/consensus/constants.rs` |

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| "already has an active delegation" | Can only delegate to one producer | `doli producer revoke-delegation` first |
| "insufficient available bonds" | bond_count - delegated_bonds < requested | Reduce --bonds or revoke existing delegation |
| "delegatee is not a registered producer" | Pubkey not in producer set | Verify pubkey with `doli producer list --format json` |
| Delegation not visible after submit | Epoch-deferred processing | Wait for next epoch boundary |
| Weight unchanged after delegation | Same epoch — not applied yet | Check `pendingUpdates` in producer status |
