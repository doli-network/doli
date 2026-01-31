# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.


### Prerequisites: 
1. Check @specs/SPECS.md and @docs/DOCS.md for instructions on how to build and test the project.
2. Enter Nix Environment

**All commands must be run inside the Nix environment.** This ensures all dependencies and tools are available.

Always use nix to run all the stack tech using filter best practices to avoid context pollution.
```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## Build & Test Commands

## Command Output Filtering Best Practices
- **Reduce noise and pollution when running commands**: Use output redirection when running nix commands like > /tmp/output.log, and then use awk, sort, head to filter just what you need specially with nix commands when building and testing.
- **Case-insensitive filtering**: Use grep -i for keywords like "pass|fail|error|success|warning"
- **Deduplication**: Use sort -u or awk '!seen[$0]++' for order-preserving deduplication
- **Limit output**: Use head -n 20 or tail -n 10 to control output volume
- **Test filtering**: test_command | grep -i "pass\|fail\|error" | sort -u | head -20
- **Build command filtering**: build_command | grep -i "error\|warning\|failed\|success" | head -10
- **Common filter pattern**: command 2>&1 | grep -i "keyword1\|keyword2" | awk '!seen[$0]++' | head -15
- **Test summary extraction**: test_command | grep -E "^\+[0-9]+|-[0-9]+|passed|failed|Some tests" | tail -5

## Project Overview

DOLI is a Rust-based cryptocurrency where Verifiable Delay Functions (VDF) are the **primary consensus mechanism**—the first blockchain with this design. Time is the scarce resource, not energy or stake.

**Consensus**: Proof of Time (PoT) using hash-chain VDF with deterministic round-robin producer selection based on bond count.

## Build Commands

```bash
# Build all crates
cargo build
cargo build --release

# Run tests (all workspace crates)
cargo test

# Run tests for a specific crate
cargo test -p core
cargo test -p crypto
cargo test -p vdf

# Run a single test
cargo test -p core test_name

# Linting and formatting
cargo clippy
cargo fmt --check
cargo fmt

# Generate API documentation
cargo doc --workspace --no-deps --open
```

## Running Binaries

```bash
# Run node (package names differ from binary names)
cargo run -p doli-node -- run                      # mainnet
cargo run -p doli-node -- --network testnet run   # testnet
cargo run -p doli-node -- --network devnet run    # local dev

# Run CLI wallet
cargo run -p doli-cli -- wallet new
cargo run -p doli-cli -- wallet balance <address>
```

## Fuzz Testing

Fuzz tests are in a separate workspace at `testing/fuzz/` (not part of main workspace):

```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_block_deserialize
cargo +nightly fuzz run fuzz_tx_deserialize
cargo +nightly fuzz run fuzz_vdf_verify
```

## Test Scripts

- All test scripts located in `scripts/`
- Test script documentation maintained in `scripts/README.md`
- Before creating a new test script, check `scripts/README.md` for existing scripts
- When creating new scripts, update `scripts/README.md` and commit both together

```bash
# List available test scripts
ls scripts/*.sh

# Check script registry
cat scripts/README.md

# Run a test script
./scripts/launch_testnet.sh
./scripts/test_3node_proportional_rewards.sh
```
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

- **`crypto`**: BLAKE3 hashing, Ed25519 signatures, key derivation. Foundation layer with no internal dependencies.
- **`vdf`**: Hash-chain VDF with ~10M iterations for ~700ms heartbeat. Uses `rug` (GMP bindings) for performance.
- **`core`**: Types, validation rules, consensus parameters. The `tpop/` module provides telemetry (heartbeat tracking, presence metrics) - not used in consensus selection.
- **`storage`**: RocksDB-backed persistence for blocks, UTXO set, chain state, producer registry.
- **`network`**: libp2p-based P2P layer with gossipsub, Kademlia DHT, header/body sync, equivocation detection.
- **`mempool`**: Transaction pool with fee policies.
- **`rpc`**: JSON-RPC server (Axum) for wallet/explorer interaction.

### Consensus: How Proof of Time Works

1. **Slot-based time**: 10-second slots, wall-clock time determines when slots occur
2. **Deterministic selection**: `slot % total_bonds` selects producer (no lottery variance)
3. **Heartbeat VDF**: ~700ms proof of sequential presence per block (anti-grinding)
4. **Epoch Lookahead**: Leaders determined at epoch start, so grinding current block cannot influence future selection
5. **Weight-based fork choice**: Chain with highest accumulated producer weight wins (discrete yearly steps: 0-1yr=1, 1-2yr=2, 2-3yr=3, 3+yr=4)

### Time Structure

- Slot = 10 seconds (mainnet), 10s (testnet), 1s (devnet)
- Epoch = 360 slots (1 hour mainnet)
- Era = 12,614,400 slots (~4 years) - triggers reward halving

### Networks

| Network | ID | Slot | P2P Port | RPC Port | Address Prefix |
|---------|-----|------|----------|----------|----------------|
| Mainnet | 1   | 10s  | 30303    | 8545     | `doli`         |
| Testnet | 2   | 10s  | 40303    | 18545    | `tdoli`        |
| Devnet  | 99  | 1s   | 50303    | 28545    | `ddoli`        |

## Code Conventions

- Branch naming: `feature/`, `fix/`, `docs/`, `refactor/`
- Commits: Conventional Commits format (`feat(scope): description`)
- Max line length: 100 characters
- Unit tests in same file, integration tests in `testing/`
- Property-based testing (proptest) for cryptographic code

### Documentation Naming Convention

- **Lowercase**: All documentation files use lowercase with underscores
  - Examples: `protocol.md`, `running_a_node.md`, `security_model.md`
- **UPPERCASE**: Only master index files that represent folder contents
  - `README.md` - Repository/folder index (standard convention)
  - `CLAUDE.md` - Claude Code instructions
  - `DOCS.md` - Documentation index (`docs/`)
  - `SPECS.md` - Specifications index (`specs/`)
  - `SKILL.md` - Skill definition (`.claude/skills/*/`)
  - `WHITEPAPER.md` - Protocol whitepaper (root level master document)
- **Flat structure**: `docs/` and `specs/` use flat structure (no subdirectories)
  - Exception: `docs/legacy/` for archived historical documents

## Documentation Alignment Principle (MANDATORY)

**CRITICAL**: Documentation drift is a protocol liability. In a blockchain system, outdated or incorrect documentation leads to implementation errors, security vulnerabilities, and consensus failures. Documentation must be treated as a first-class artifact, not an afterthought.

### Truth Hierarchy

The following hierarchy defines the source of truth. Higher levels govern lower levels:

```
┌─────────────────────────────────────────┐
│  1. WHITEPAPER.md (Protocol Bible)      │  ← Defines WHAT the protocol IS
├─────────────────────────────────────────┤
│  2. specs/* (Technical Specifications)  │  ← Defines HOW it works technically
├─────────────────────────────────────────┤
│  3. docs/* (Operational Documentation)  │  ← Defines HOW to use it
├─────────────────────────────────────────┤
│  4. Code (Implementation)               │  ← Implements the above
└─────────────────────────────────────────┘
```

**Rules:**
1. **Code must implement WHITEPAPER** - If code differs from whitepaper, the code is wrong
2. **Specs must reflect code accurately** - If specs differ from code, specs must be updated
3. **Docs must describe reality** - If docs differ from behavior, docs must be updated
4. **Never document aspirations** - Only document what EXISTS, not what's planned

### Mandatory Alignment Checks

Before ANY commit that modifies behavior:

1. **Verify WHITEPAPER compliance** - Does the change align with whitepaper specifications?
2. **Check specs accuracy** - Do technical specs still accurately describe the system?
3. **Verify docs accuracy** - Do operational docs still accurately describe usage?
4. **Update or flag** - Either update documentation OR flag misalignment for review

### Zero Tolerance for Drift

The following are strictly prohibited:

- **Committing code that contradicts WHITEPAPER** without explicit approval to update whitepaper
- **Leaving specs outdated** after changing wire formats, consensus rules, or architecture
- **Leaving docs outdated** after changing CLI commands, RPC endpoints, or user-facing behavior
- **Documenting unimplemented features** - No "coming soon" or aspirational content
- **Assuming someone else will update docs** - If you change it, you document it

### When to Update Each Layer

| Change Type | WHITEPAPER | specs/* | docs/* |
|-------------|------------|---------|--------|
| Consensus rules | YES | YES | Maybe |
| Economic parameters | YES | YES | YES |
| Wire format/protocol | NO | YES | Maybe |
| Architecture/crates | NO | YES | NO |
| CLI commands | NO | NO | YES |
| RPC endpoints | NO | Maybe | YES |
| Node operation | NO | NO | YES |
| Internal refactor | NO | NO | NO |
| Bug fix (no behavior change) | NO | NO | NO |

### Enforcement

Use `/sync-docs` after any implementation to verify and update documentation alignment. This is not optional for changes that affect documented behavior.

---

## Workflow Rules

### Bug Fixing Protocol (MANDATORY)

**CRITICAL**: This is state-of-the-art blockchain technology. Quick fixes that mask symptoms without addressing root causes are strictly prohibited. Every bug fix must be treated as a potential architectural decision.

#### Phase 1: Root Cause Analysis (REQUIRED)

Before writing ANY fix:
1. **Identify the true root cause** - Never fix symptoms. Trace the bug to its origin in the codebase.
2. **Study the affected components** - Understand how the buggy code interacts with:
   - Consensus logic (PoT, VDF validation, producer selection)
   - State transitions (UTXO, bonds, rewards)
   - Network protocol (message propagation, sync)
   - Cryptographic guarantees (signatures, hashes)
3. **Assess architectural implications** - Document answers to:
   - Does this fix change any invariant or assumption in the protocol?
   - Could this fix introduce race conditions or state inconsistencies?
   - Does it affect any security properties defined in `specs/security_model.md`?
   - Will it impact consensus finality or fork choice?

#### Phase 2: Whitepaper Compliance (MANDATORY)

**You are strictly prohibited from implementing anything not specified in the WHITEPAPER.**

Before implementing:
1. Verify the fix aligns with `WHITEPAPER.md` specifications
2. If the bug reveals a gap between implementation and whitepaper, the whitepaper is the source of truth
3. If the whitepaper is silent on the matter, escalate to the user before proceeding
4. Never invent new mechanisms, parameters, or behaviors not documented in the whitepaper

#### Phase 3: Implementation

1. Implement the minimal fix that addresses the root cause
2. Write or update tests that would have caught this bug
3. Run full test suite: `cargo test`
4. Run clippy and format: `cargo clippy && cargo fmt`

#### Phase 4: User Validation (REQUIRED BEFORE COMMIT)

**You are strictly prohibited from committing without explicit user validation.**

After tests pass:
1. **STOP** - Do not commit automatically
2. Present to the user:
   - Root cause analysis summary
   - The fix applied and its rationale
   - Any architectural implications identified
   - Test results
3. **ASK**: "Have you validated that this bug is fixed?"
4. **WAIT** for explicit confirmation before proceeding

#### Phase 5: Documentation and Commit

Only after user confirms the fix is validated:
1. Update `specs/` and `docs/` **only if the fix affects documented behavior**
2. Do not add unnecessary documentation
3. Commit with descriptive message: `fix(scope): description of root cause and fix`

### After Passing Tests (New Features)

Once code changes pass all unit tests:
1. Update `specs/` and `docs/` if the changes affect documented behavior or specifications
2. Commit all changes together

### Milestone/Task Documentation Sync (MANDATORY)

**CRITICAL**: When implementation is divided into milestones or tasks, documentation alignment is MANDATORY after EACH milestone or task passes its tests—not just at the end.

#### The Rule

After ANY milestone or task successfully passes unit tests:

1. **STOP** - Do not proceed to the next milestone
2. **Review documentation impact** - Check if the completed work affects:
   - `WHITEPAPER.md` - Protocol-level behavior or specifications
   - `specs/*` - Technical specifications, architecture, wire formats
   - `docs/*` - User-facing behavior, CLI, RPC, operational guides
3. **Update immediately** - If documentation is affected, update it NOW
4. **Use `/sync-docs`** - Run the sync-docs skill to verify alignment
5. **Commit together** - Code changes AND documentation updates in the same commit

#### Why This Matters

```
❌ WRONG: Complete all milestones → Update docs at the end
✅ RIGHT: Complete milestone → Sync docs → Commit → Next milestone
```

**Rationale:**
- Documentation drift accumulates when deferred
- Each milestone may change assumptions that affect later work
- Other developers (or future Claude sessions) need accurate docs to continue
- Blockchain protocols require precise documentation—bugs kill trust

#### Enforcement Checklist

After each milestone/task passes tests, answer:

| Question | If YES |
|----------|--------|
| Does this change protocol behavior? | Update WHITEPAPER.md |
| Does this change internal architecture? | Update specs/architecture.md |
| Does this change consensus rules? | Update specs/protocol.md |
| Does this change CLI commands? | Update docs/cli.md |
| Does this change RPC endpoints? | Update docs/rpc_reference.md |
| Does this change node operation? | Update docs/running_a_node.md |

**If you cannot answer "no documentation needed" with certainty, run `/sync-docs`.**

### Handling Bug Reports (`REPORT_*.md` Files)

When fixing a bug documented in a `REPORT_*.md` file:
1. **Follow the Bug Fixing Protocol above** (all phases mandatory)
2. Update the report file with:
   - Root cause analysis
   - Description of the fix applied
   - Resolution date
3. Move the report to `docs/legacy/bugs/`
4. Update `specs/` and `docs/` if affected
5. Commit all changes together (only after user validation)
