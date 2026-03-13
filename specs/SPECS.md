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
| [gui-architecture.md](./gui-architecture.md) | GUI Desktop Application architecture - Tauri 2.x app structure, shared wallet crate, VDF feature flag, CI/CD pipeline, security model |

## Requirements Specifications

| File | Description |
|------|-------------|
| [gui-desktop-requirements.md](./gui-desktop-requirements.md) | GUI Desktop Application requirements - Tauri 2.x cross-platform wallet with full CLI feature parity |

---

## Quick Navigation

```
specs/
├── SPECS.md                      # ← You are here (specifications index)
├── protocol.md                   # Full protocol specification
├── architecture.md               # Comprehensive architecture
├── security_model.md             # Complete security model
├── gui-architecture.md           # GUI Desktop Application architecture
└── gui-desktop-requirements.md   # GUI Desktop Application requirements
```

---

## See Also

For user-facing documentation, operational guides, and implementation references, see [docs/DOCS.md](/docs/DOCS.md).

Specific guides:
- [running_a_node.md](/docs/running_a_node.md) - General Node Guide
- [testnet.md](/docs/testnet.md) - Testnet Guide
- [devnet.md](/docs/devnet.md) - Devnet & Bootstrap Guide

**Note:** The `docs/` directory contains user-facing guides and operational documentation derived from these specifications. When implementing protocol features, refer to the specs in this directory. When operating nodes or using the CLI, refer to docs/.
