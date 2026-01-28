---
name: doli-just
description: This skill should be used when the user asks to "build", "test", "run tests", "lint", "format", "run node", "start node", "check wallet", "run just", "just command", mentions justfile, cargo build, cargo test, or wants to execute DOLI blockchain development commands.
version: 1.0.0
---

# DOLI Justfile Commands

This skill provides access to the DOLI blockchain project's justfile commands. All commands run inside the Nix environment automatically.

## Quick Reference

To list all available commands:
```bash
just
```

## Build Commands

| Command | Description |
|---------|-------------|
| `just build` | Build all crates (debug) |
| `just build-release` | Build all crates (release) |
| `just build-crate <crate>` | Build specific crate |
| `just check` | Check all crates without building |
| `just clean` | Clean build artifacts |

## Testing Commands

| Command | Description |
|---------|-------------|
| `just test` | Run all tests |
| `just test-crate <crate>` | Run tests for specific crate |
| `just test-single <crate> <test_name>` | Run a single test by name |
| `just test-core` | Run doli-core tests |
| `just test-crypto` | Run crypto crate tests |
| `just test-vdf` | Run VDF crate tests |
| `just test-storage` | Run storage tests |
| `just test-network` | Run network tests |
| `just test-mempool` | Run mempool tests |
| `just test-rpc` | Run RPC tests |
| `just test-integration` | Run integration tests |
| `just test-e2e` | Run end-to-end tests |

## Linting & Formatting

| Command | Description |
|---------|-------------|
| `just lint` | Run clippy lints |
| `just lint-strict` | Run clippy with strict warnings (deny all) |
| `just fmt-check` | Check code formatting |
| `just fmt` | Format code |
| `just qa` | Run all quality checks (lint + format + test) |

## Node Operations

| Command | Description |
|---------|-------------|
| `just node-mainnet` | Run node on mainnet |
| `just node-testnet` | Run node on testnet |
| `just node-devnet` | Run node on devnet (local development) |
| `just node-config <path>` | Run node with custom config |
| `just node-release` | Run node in release mode (mainnet) |

## CLI Wallet

| Command | Description |
|---------|-------------|
| `just wallet-new` | Create new wallet |
| `just wallet-balance <address>` | Check wallet balance |
| `just cli <args>` | Run arbitrary CLI command |

## Development Helpers

| Command | Description |
|---------|-------------|
| `just watch` | Watch for changes and rebuild |
| `just watch-test` | Watch for changes and run tests |
| `just deps` | Show crate dependency tree |
| `just workspace` | Show workspace members |
| `just update` | Update dependencies |
| `just audit` | Audit dependencies for security vulnerabilities |

## Documentation

| Command | Description |
|---------|-------------|
| `just doc` | Generate API documentation |
| `just doc-open` | Generate and open API documentation |

## Quick Recipes

| Command | Description |
|---------|-------------|
| `just dev` | Full dev cycle: format, lint, test |
| `just quick` | Quick check: just verify it compiles |
| `just ci` | CI pipeline simulation |

## Network Info

| Command | Description |
|---------|-------------|
| `just networks` | Display network configuration table |
| `just arch` | Display architecture diagram |
| `just time-info` | Display time structure info |

## Fuzz Testing

Fuzz tests require nightly Rust:

| Command | Description |
|---------|-------------|
| `just fuzz-block` | Run block deserialization fuzzer |
| `just fuzz-tx` | Run transaction deserialization fuzzer |
| `just fuzz-vdf` | Run VDF verification fuzzer |
| `just fuzz-list` | List available fuzz targets |

## Benchmarks

| Command | Description |
|---------|-------------|
| `just bench` | Run benchmarks |

## Release

| Command | Description |
|---------|-------------|
| `just release-build` | Build release binaries |
| `just release-artifacts` | Create release artifacts in dist/ |

## Network Scripts

| Command | Description |
|---------|-------------|
| `just launch-testnet` | Launch testnet using script |
| `just stress-test` | Run stress test (600 nodes simulation) |

## Common Workflows

### Starting Development
```bash
just nix-shell    # Enter Nix environment interactively
just quick        # Verify everything compiles
just dev          # Full dev cycle
```

### Before Committing
```bash
just qa           # Run all quality checks
```

### Running a Local Node
```bash
just node-devnet  # Start local development node
```

### Testing a Specific Feature
```bash
just test-crate <crate_name>
just test-single <crate> <test_name>
```
