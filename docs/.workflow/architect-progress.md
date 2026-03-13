# Architect Progress: DOLI GUI Desktop Application

## Status: COMPLETE

## Files Created/Modified

### Created
- `specs/gui-architecture.md` -- Full architecture specification (7 milestones, 5 modules, security model, failure modes, performance budgets, data flows, traceability)

### Modified
- `specs/gui-desktop-requirements.md` -- Updated traceability matrix with Architecture Section column (all 62 requirements mapped)
- `specs/SPECS.md` -- Added gui-architecture.md to index
- `docs/DOCS.md` -- Updated architecture.md description to note GUI section
- `docs/architecture.md` -- Added Section 11: GUI Desktop Application

## Architecture Summary

### Key Design Decisions
1. **New `crates/wallet/` crate** -- shared between CLI and GUI, depends on `crypto` only (NOT doli-core/vdf), eliminates GMP requirement for GUI
2. **Transaction builder in wallet crate** -- duplicates ~200 lines of serialization but avoids entire vdf->rug->GMP dependency chain
3. **VDF feature flag in doli-core** -- `default = ["vdf"]`, additive. GUI uses wallet crate directly; flag enables future tools
4. **Svelte 5 frontend** -- smallest bundle size (~5KB compiled), compiled reactivity, official Tauri template
5. **Separate CI jobs** -- GUI build failure does not block CLI/node releases. Windows GUI build needs NO GMP/MSYS2.
6. **Config isolation** -- GUI uses `~/.doli-gui/config.json`, wallet files are shared via user-chosen paths

### Milestones (7)
- M1: Core Infrastructure (wallet crate + VDF feature flag)
- M2: Tauri App Shell + Wallet
- M3: Transactions + Balance
- M4: Producer + Rewards
- M5: NFT + Bridge + Governance
- M6: CI/CD + Packaging
- M7: Could-Priority Features

### Module Count: 5
1. `crates/wallet/` -- Shared wallet library
2. `bins/gui/src-tauri/` -- Tauri Rust backend
3. `bins/gui/src/` -- Svelte frontend
4. VDF feature flag (crates/core modification)
5. CI/CD pipeline (release.yml extension)
