# Bugfix Analysis: Wallet-Core Serialization Drift Safety Net

## Investigation Summary

The wallet crate reimplements transaction types, serialization, and constants independently from doli-core (architectural constraint: no runtime dependency). Current wire format is correct, but coverage is thin — only 2/7 constructible tx types have byte-identical serialization tests.

**Critical finding**: Wallet TxType enum discriminants (repr(u8)) do NOT match core's (repr(u32)). A `to_core_type_id()` mapping function compensates, but is the single point of failure.

## Scope

| Domain | Files Affected |
|--------|---------------|
| Wallet types | `crates/wallet/src/tx_builder/types.rs` |
| Wallet tests | `crates/wallet/tests/serialization_compat.rs` |
| Wallet tests | `crates/wallet/src/tx_builder/tests.rs` |

## Requirements

| ID | Requirement | Priority |
|----|------------|----------|
| REQ-WSER-001 | Expand serialization compat tests to all constructible tx types | Must |
| REQ-WSER-002 | Add compile-time variant count assertion for TxType drift | Must |
| REQ-WSER-003 | Add compile-time constant equality assertions (all 8 constants) | Must |
| REQ-WSER-004 | Fix misleading test_tx_type_values test | Should |
| REQ-WSER-005 | Add exhaustive to_core_type_id mapping test against core enum | Must |
| REQ-WSER-006 | Add wallet TxType variants for MintAsset and BurnAsset | Could |
| REQ-WSER-007 | Add fee_multiplier_x100 output parity test | Should |
| REQ-WSER-008 | Document to_core_type_id mapping as critical bridge | Should |

## Key Findings

1. Wallet `Coinbase=3` vs core `Coinbase=6`. The `to_core_type_id()` mapping compensates but is fragile.
2. Core has `ProtocolActivation=15`, `MintAsset=17`, `BurnAsset=18` — wallet has none.
3. Only Transfer and AddBond have byte-identical serialization tests.
4. All 8 duplicated constants currently match core — no drift detected.
5. Fee calculation logic is correctly duplicated — no drift detected.

## Implementation: All test-only changes with zero runtime risk.
