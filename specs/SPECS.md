# SPECS - Technical Specifications Index

Master index for all DOLI protocol specifications.

---

## Core Specifications

| File | Description |
|------|-------------|
| [WHITEPAPER.md](/WHITEPAPER.md) | Complete protocol whitepaper - VDF-based blockchain with Proof of Time (PoT) consensus |
| [protocol.md](./protocol.md) | Full protocol specification - encoding, cryptographic primitives, consensus rules, test vectors |
| [architecture.md](./architecture.md) | Comprehensive system architecture - all crate responsibilities and component interactions |
| [security_model.md](./security_model.md) | Complete security model - threat analysis, attack vectors, cryptographic guarantees |

## Architecture Specifications

| File | Description |
|------|-------------|
| [single-proposer-architecture.md](./single-proposer-architecture.md) | Single-Proposer-Per-Slot migration architecture - protocol v2 gating, attestation fork choice, emergency fallback, 3-phase implementation plan |
| [gui-architecture.md](./gui-architecture.md) | GUI Desktop Application architecture - Tauri 2.x app structure, shared wallet crate, VDF feature flag, CI/CD pipeline, security model |

## Requirements Specifications

| File | Description |
|------|-------------|
| [single-proposer-requirements.md](./single-proposer-requirements.md) | Single-Proposer-Per-Slot requirements - migration from multi-rank fallback to single proposer, attestation fork choice |
| [gui-desktop-requirements.md](./gui-desktop-requirements.md) | GUI Desktop Application requirements - Tauri 2.x cross-platform wallet with full CLI feature parity |

## Improvement Specifications

| File | Description |
|------|-------------|
| [improvements/apply-block-modularization.md](./improvements/apply-block-modularization.md) | Apply-block modularization analysis and plan |
| [improvements/cli-modularization.md](./improvements/cli-modularization.md) | CLI modularization analysis and plan |
| [improvements/consensus-modularization.md](./improvements/consensus-modularization.md) | Consensus module modularization analysis and plan |
| [improvements/modularization-improvement.md](./improvements/modularization-improvement.md) | General modularization improvement specification |
| [improvements/scaling-100k-producers.md](./improvements/scaling-100k-producers.md) | Scaling to 100K producers analysis and plan |

## Bugfix Analysis

| File | Description |
|------|-------------|
| [bugfixes/production-gate-deadlock-analysis.md](./bugfixes/production-gate-deadlock-analysis.md) | Production gate deadlock root cause analysis |
| [bugfixes/reward-validation-analysis.md](./bugfixes/reward-validation-analysis.md) | Reward validation gap analysis and fixes |

---

## Quick Navigation

```
specs/
├── SPECS.md                          # <- You are here (specifications index)
├── protocol.md                       # Full protocol specification
├── architecture.md                   # Comprehensive architecture
├── security_model.md                 # Complete security model
├── single-proposer-architecture.md   # Single-proposer migration architecture
├── single-proposer-requirements.md   # Single-proposer migration requirements
├── gui-architecture.md               # GUI Desktop Application architecture
├── gui-desktop-requirements.md       # GUI Desktop Application requirements
├── improvements/
│   ├── apply-block-modularization.md # Apply-block modularization
│   ├── cli-modularization.md         # CLI modularization
│   ├── consensus-modularization.md   # Consensus modularization
│   ├── modularization-improvement.md # General modularization
│   └── scaling-100k-producers.md     # Scaling to 100K producers
└── bugfixes/
    ├── production-gate-deadlock-analysis.md  # Production gate deadlock
    └── reward-validation-analysis.md         # Reward validation gaps
```

---

## See Also

For user-facing documentation, operational guides, and implementation references, see [docs/DOCS.md](/docs/DOCS.md).

Specific guides:
- [running_a_node.md](/docs/running_a_node.md) - General Node Guide
- [testnet.md](/docs/testnet.md) - Testnet Guide
- [devnet.md](/docs/devnet.md) - Devnet & Bootstrap Guide

**Note:** The `docs/` directory contains user-facing guides and operational documentation derived from these specifications. When implementing protocol features, refer to the specs in this directory. When operating nodes or using the CLI, refer to docs/.
