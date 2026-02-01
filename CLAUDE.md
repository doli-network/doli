# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Prerequisites

1. Check `specs/SPECS.md` and `docs/DOCS.md` for instructions on how to build and test the project.
2. **All commands must be run inside the Nix environment:**

```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## Command Output Filtering

Reduce noise when running commands. Redirect output and filter for relevant information:

```bash
# General pattern
command 2>&1 | grep -i "keyword1\|keyword2" | awk '!seen[$0]++' | head -15

# Build commands
build_command > /tmp/output.log 2>&1 && grep -i "error\|warning\|failed\|success" /tmp/output.log | head -10

# Test commands
test_command 2>&1 | grep -i "pass\|fail\|error" | sort -u | head -20

# Test summary
test_command 2>&1 | grep -E "^\+[0-9]+|-[0-9]+|passed|failed|Some tests" | tail -5
```

## Build, Test & Run Commands

### Building

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo clippy                   # Linting
cargo fmt --check              # Format check
cargo fmt                      # Auto-format
cargo doc --workspace --no-deps --open  # Generate docs
```

### Testing

```bash
cargo test                     # All workspace tests
cargo test -p core             # Single crate
cargo test -p core test_name   # Single test
```

### Fuzz Testing

Fuzz tests are in a **separate workspace** at `testing/fuzz/`:

```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_block_deserialize
cargo +nightly fuzz run fuzz_tx_deserialize
cargo +nightly fuzz run fuzz_vdf_verify
```

### Test Scripts

Scripts are in `scripts/`. Check `scripts/README.md` before creating new ones.

```bash
ls scripts/*.sh                          # List scripts
./scripts/launch_testnet.sh              # Example
./scripts/test_3node_proportional_rewards.sh
```

### Running Binaries

```bash
# Node
cargo run -p doli-node -- run                      # mainnet
cargo run -p doli-node -- --network testnet run   # testnet
cargo run -p doli-node -- --network devnet run    # devnet

# CLI wallet
cargo run -p doli-cli -- wallet new
cargo run -p doli-cli -- wallet balance <address>
```

## Project Overview

DOLI is a Rust-based cryptocurrency where Verifiable Delay Functions (VDF) are the **primary consensus mechanism**—the first blockchain with this design. Time is the scarce resource, not energy or stake.

**Consensus**: Proof of Time (PoT) using hash-chain VDF with deterministic round-robin producer selection based on bond count.

## Architecture

### Crate Dependency Flow

```
bins/node (doli-node)          bins/cli (doli-cli)
    │                              │
    ├─→ network ─┐                 │
    ├─→ rpc ─────┤                 │
    ├─→ mempool ─┤                 │
    ├─→ storage ─┤                 │
    ├─→ updater ─┤                 │
    │            ▼                 │
    └─────────→ core ←─────────────┘
                 │
                 ▼
         ┌───────┴───────┐
         ▼               ▼
      crypto            vdf
```

### Key Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `crypto` | BLAKE3 hashing, Ed25519 signatures, key derivation. No internal dependencies. |
| `vdf` | Hash-chain VDF (~10M iterations, ~700ms heartbeat). Uses `rug` (GMP bindings). |
| `core` | Types, validation, consensus parameters. `tpop/` provides telemetry (not used in consensus). |
| `storage` | RocksDB persistence for blocks, UTXO, chain state, producer registry. |
| `network` | libp2p P2P layer: gossipsub, Kademlia DHT, sync, equivocation detection. |
| `mempool` | Transaction pool with fee policies. |
| `rpc` | JSON-RPC server (Axum) for wallet/explorer interaction. |

### Consensus: Proof of Time

1. **Slot-based time**: Wall-clock determines slots
2. **Deterministic selection**: `slot % total_bonds` selects producer
3. **Heartbeat VDF**: ~700ms proof of sequential presence (anti-grinding)
4. **Epoch Lookahead**: Leaders fixed at epoch start
5. **Weight-based fork choice**: Accumulated producer weight (discrete yearly steps: 0-1yr=1, 1-2yr=2, 2-3yr=3, 3+yr=4)

### Time Structure

- **Slot**: 10s (mainnet/testnet), 1s (devnet)
- **Epoch**: 360 slots (1 hour mainnet)
- **Era**: 12,614,400 slots (~4 years) - triggers reward halving

### Networks

| Network | ID | Slot | P2P Port | RPC Port | Prefix |
|---------|----|------|----------|----------|--------|
| Mainnet | 1  | 10s  | 30303    | 8545     | `doli` |
| Testnet | 2  | 10s  | 40303    | 18545    | `tdoli`|
| Devnet  | 99 | 1s   | 50303    | 28545    | `ddoli`|

## Code Conventions

- **Branches**: `feature/`, `fix/`, `docs/`, `refactor/`
- **Commits**: Conventional Commits (`feat(scope): description`, `fix(scope): description`)
- **Git Author**: All commits must use `--author="E. Weil <weil@doli.network>"`
- **Line length**: 100 characters max
- **Tests**: Unit tests in same file, integration tests in `testing/`
- **Crypto code**: Use property-based testing (proptest)

### File Naming

| Type | Convention | Examples |
|------|------------|----------|
| Documentation | lowercase with underscores | `protocol.md`, `running_a_node.md` |
| Master indexes | UPPERCASE | `README.md`, `CLAUDE.md`, `DOCS.md`, `SPECS.md`, `WHITEPAPER.md` |
| Flat structure | No subdirectories in `docs/`, `specs/` | Exception: `docs/legacy/` |

## Documentation Alignment (MANDATORY)

Documentation drift is a protocol liability.

### Truth Hierarchy

```
1. WHITEPAPER.md    ← Defines WHAT the protocol IS (source of truth)
2. specs/*          ← Defines HOW it works technically
3. docs/*           ← Defines HOW to use it
4. Code             ← Implements the above
```

**Rules:**
- Code must implement WHITEPAPER—if they differ, code is wrong
- Specs must reflect code—if they differ, update specs
- Docs must describe reality—never document aspirations

## Workflow Rules

### Pre-Commit Gate (BLOCKING)

**BEFORE EVERY COMMIT, you MUST complete this checklist. No exceptions.**

```
┌─────────────────────────────────────────────────────────────────┐
│  PRE-COMMIT DOCUMENTATION GATE                                  │
│  Cannot proceed to `git commit` until ALL boxes are checked     │
├─────────────────────────────────────────────────────────────────┤
│  [ ] 1. Run `/sync-docs` OR manually verify:                    │
│         - specs/protocol.md reflects any protocol changes       │
│         - docs/*.md reflects any user-facing changes            │
│         - docs/rpc_reference.md reflects any RPC changes        │
│         - docs/cli.md reflects any CLI changes                  │
│                                                                 │
│  [ ] 2. If implementation adds/changes behavior:                │
│         - specs/ updated with technical details                 │
│         - docs/ updated with usage instructions                 │
│                                                                 │
│  [ ] 3. If implementation adds/changes tests:                   │
│         - scripts/README.md updated (if test script added)      │
│         - Test coverage documented in relevant docs             │
│                                                                 │
│  [ ] 4. State which docs were checked/updated in commit message │
└─────────────────────────────────────────────────────────────────┘
```

**If you skip this gate, you are violating CLAUDE.md.**

### Milestone Workflow

When working on implementation milestones (e.g., from IMPLEMENTATION_CLAIM_REWARD.md):

1. **Implement** - Write the code/tests for the milestone
2. **Verify** - Run `cargo build`, `cargo clippy`, `cargo test` (all must pass)
3. **Document** - Complete Pre-Commit Gate checklist above
4. **Commit** - Using conventions (conventional commits, `--author`, HEREDOC format)
5. **Push** - Push to remote
6. **Stop** - Report completion, wait for next milestone

A milestone is not complete until pushed. Do not stop between steps.

### For All Changes

After tests pass:
1. **EXECUTE Pre-Commit Gate above** (this is not optional)
2. Update relevant documentation immediately
3. Commit code and docs together

### Bug Fixing Protocol (MANDATORY)

**Use `/fix-bug` for all bug fixes.** Quick fixes that mask symptoms are prohibited.

Key constraints:
- You are prohibited from implementing anything not specified in WHITEPAPER.md
- If code differs from whitepaper, whitepaper is truth
- Never commit without explicit user validation
