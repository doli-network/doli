# Configuration Verification Report

**Date**: 2026-02-05
**Scope**: Verification of environment variable extraction and the 3-level configuration hierarchy.

## Executive Summary

We have successfully verified that the DOLI codebase adheres to the strict 3-level configuration hierarchy defined in `architecture.md`. The `crates/core/src/network_params.rs` module correctly serves as the central configuration manager, loading parameters from environment variables (with precedence rules) and exposing them to the rest of the application.

Legacy constants in `crates/core/src/consensus.rs` remain as the "DNA" (Level 1) but are properly deprecated for direct usage, with consumers routed to `NetworkParams` (Level 2).

## Verification Findings

### 1. Environment Variable Loading
- **Mechanism**: `bins/node/src/main.rs` calls `network_params::load_env_for_network` during startup, loading `.env` files into the process environment.
- **Verification**: Confirmed `NetworkParams::load_from_env` prioritizes environment variables (`std::env::var`) over defaults for non-Mainnet networks.
- **Test Coverage**: Added `test_env_override` to `crates/core/src/network_params.rs` which explicitly verifies:
    - Setting a dummy `DOLI_SLOT_DURATION` overrides the default on Devnet.
    - Mainnet correctly IGNORES the override (security feature).

### 2. Module Integration (Level 3 Consumers)
The following modules were audited and confirmed to consume `NetworkParams` instead of hardcoded constants:

| Module | Usage | Verification |
|--------|-------|--------------|
| `crates/core/src/network.rs` | `default_p2p_port` | Uses `self.params().p2p_port` |
| `crates/storage/src/producer.rs` | `blocks_per_year` | Uses `NetworkParams::load(network).blocks_per_year` |
| `bins/node/src/config.rs` | Node Initialization | Uses `network.default_p2p_port()` (routes to NetworkParams) |
| `crates/rpc/src/server.rs` | RPC Context | Initialized with `NetworkParams` values |
| `crates/core/src/tpop/presence.rs` | Telemetry | Uses `NetworkParams::load(network).grace_period_secs` |

### 3. Legacy Constants
- `crates/core/src/consensus.rs` defines `YEAR_IN_SLOTS`.
- **Finding**: Some internal methods in `consensus.rs` (like `is_vested`) still use `YEAR_IN_SLOTS`.
- **Status**: Acceptable for now as `consensus.rs` represents rules. For full Devnet acceleration, these specific internal methods may need refactoring in the future, but they do not block the primary configuration flow.

### 4. `.env` Fallback and Chainspec Defaults (2026-02-06)

Two bugs were found and fixed:

**Bug 1 — `.env` fallback**: `load_env_for_network()` only checked `{data_dir}/.env`. When `--data-dir` pointed to a subdirectory (e.g., `~/.doli/devnet/data/node5`), the `.env` at `~/.doli/devnet/.env` was never found. Fix: added fallback to `get_default_data_dir(network_name)/.env`.

**Bug 2 — Chainspec phantom**: The `NetworkParams` OnceLock was triggered by `NodeConfig::for_network()` before the chainspec was loaded. The chainspec's `ConsensusSpec` fields were stored in JSON but never applied. Fix: added `apply_chainspec_defaults()` that sets env vars from chainspec before any code triggers the OnceLock.

**Updated priority hierarchy**: Parent ENV > `.env` file > Chainspec > `consensus.rs` defaults

**New test coverage**:
- `test_load_env_fallback_to_network_root`
- `test_apply_chainspec_defaults_sets_vars`
- `test_apply_chainspec_defaults_no_override`
- `test_apply_chainspec_defaults_mainnet_skipped`
- `test_apply_chainspec_defaults_malformed_file`

## Conclusion

The configuration refactoring is complete and verified. `NetworkParams` acts as the single source of truth for configurable parameters, enabling safe environment variable overrides for Devnet while protecting Mainnet constants. Chainspec consensus parameters are now properly applied as the lowest-priority defaults before the OnceLock triggers.
