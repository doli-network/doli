# Bugfix Analysis: Inconsistent BOND_UNIT Constant in Storage Crate

## Investigation Summary

**Bug**: Storage crate defines `BOND_UNIT = 10,000,000,000` (100 DOLI) while canonical core crate defines `BOND_UNIT = 1,000,000,000` (10 DOLI). Comments in 3 files contain wrong values. Specs drift in architecture.md.

**Status**: Dormant — production code uses `new_with_bonds()` which uses `CORE_BOND_UNIT`. But a trap for future developers.

**NOT consensus-critical. Does NOT require chain reset.**

## Scope

| Domain | Files Affected |
|--------|---------------|
| Storage constants | `crates/storage/src/producer/constants.rs` |
| Producer info | `crates/storage/src/producer/info.rs` |
| Core constants (comment) | `crates/core/src/consensus/constants.rs` |
| Core economics (comments) | `crates/core/src/network/economics.rs` |
| Spec drift | `specs/architecture.md` |

## Requirements

| ID | Requirement | Priority |
|----|------------|----------|
| REQ-BOND-001 | Fix storage BOND_UNIT to match core (1B = 10 DOLI) | Must |
| REQ-BOND-002 | Fix wrong comments in storage constants.rs | Must |
| REQ-BOND-003 | Fix wrong comments in economics.rs | Must |
| REQ-BOND-004 | Fix stale comment in consensus/constants.rs | Should |
| REQ-BOND-005 | Fix specs drift in architecture.md | Must |
| REQ-BOND-006 | Update storage producer tests for correct constant | Should |
| REQ-BOND-007 | All existing tests pass after changes | Must |

## Implementation Approach

**Option A (Recommended): Remove storage BOND_UNIT, use core constant directly**

1. In `constants.rs`: Remove local `BOND_UNIT = 10,000,000,000`, re-export from core
2. In `info.rs`: Unify fallbacks to use `CORE_BOND_UNIT` (since both names now resolve to same value)
3. Fix all stale comments (3 files) and spec drift (1 file)
4. Update tests to use correct bond amounts
