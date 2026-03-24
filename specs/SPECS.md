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
| [syncmanager-architecture.md](./syncmanager-architecture.md) | Sync Manager redesign - subtraction-first restructuring of 55-field SyncManager into 5 typed sub-structs, fork recovery coordination, production gate simplification |
| [sync-recovery-architecture.md](./sync-recovery-architecture.md) | Sync Recovery redesign - recovery gate method (request_genesis_resync), state transition validation, monotonic progress floor extension. Addresses INC-I-005 cascade feedback loops F1-F5 |
| [sync-redesign-architecture.md](./sync-redesign-architecture.md) | **Sync Manager v3 redesign** — subtraction-first: 8 sync states -> 3, 5 recovery paths -> 3 levels, 11 production gate layers -> 3 checks. Addresses 13 incidents of cascading sync/fork bugs. Supersedes syncmanager-architecture and sync-recovery-architecture. |
| [single-proposer-architecture.md](./single-proposer-architecture.md) | Single-Proposer-Per-Slot migration architecture - protocol v2 gating, attestation fork choice, emergency fallback, 3-phase implementation plan |
| [gui-architecture.md](./gui-architecture.md) | GUI Desktop Application architecture - Tauri 2.x app structure, shared wallet crate, VDF feature flag, CI/CD pipeline, security model |

## Requirements Specifications

| File | Description |
|------|-------------|
| [single-proposer-requirements.md](./single-proposer-requirements.md) | Single-Proposer-Per-Slot requirements - migration from multi-rank fallback to single proposer, attestation fork choice |
| [gui-desktop-requirements.md](./gui-desktop-requirements.md) | GUI Desktop Application requirements - Tauri 2.x cross-platform wallet with full CLI feature parity |

---

## Quick Navigation

```
specs/
├── SPECS.md                          # <- You are here (specifications index)
├── protocol.md                       # Full protocol specification
├── architecture.md                   # Comprehensive architecture
├── security_model.md                 # Complete security model
├── syncmanager-architecture.md        # Sync Manager redesign architecture
├── sync-recovery-architecture.md     # Sync Recovery redesign (INC-I-005 cascade fix)
├── sync-redesign-architecture.md     # Sync Manager v3 — subtraction-first redesign
├── single-proposer-architecture.md   # Single-proposer migration architecture
├── single-proposer-requirements.md   # Single-proposer migration requirements
├── gui-architecture.md               # GUI Desktop Application architecture
└── gui-desktop-requirements.md       # GUI Desktop Application requirements
```

---

## See Also

For user-facing documentation, operational guides, and implementation references, see [docs/DOCS.md](/docs/DOCS.md).

Specific guides:
- [running_a_node.md](/docs/running_a_node.md) - General Node Guide
- [testnet.md](/docs/testnet.md) - Testnet Guide
- [devnet.md](/docs/devnet.md) - Devnet & Bootstrap Guide

**Note:** The `docs/` directory contains user-facing guides and operational documentation derived from these specifications. When implementing protocol features, refer to the specs in this directory. When operating nodes or using the CLI, refer to docs/.
