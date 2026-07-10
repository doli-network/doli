<!--
OUTPUT CONTRACT: N/A — specs index file (not a test file)
INPUT PARTITIONS: N/A — specs index file (not a test file)
-->

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
| [state-of-the-art-architecture.md](./state-of-the-art-architecture.md) | State-of-the-art redesign proposal — ZKSettle activation path, DeFi TX gating, Condition SDK, competitive positioning vs. Move/Sui/Bitcoin. 5-evaluator convergence synthesis with SSF candidate, 4-phase migration plan, and 8 hard constraints from incident history. |
| [defi-subsystem-architecture.md](./defi-subsystem-architecture.md) | DeFi subsystem redesign — AMM-First (SSF Tier 2): 5-evaluator convergence picks (a) constant-product AMM with 25/5 fee split (W2 — 25 bps LP + 5 bps producer pool), (a) condition-based collateral via existing covenant guards, (b) per-primitive activation heights, NO oracle in Phase 1. Ships 4 AMM TX types + composed bilateral escrow-loan (W3 naming); defers native lending (Phase 2) and NFT fractionalization (Phase 3) to separate redesign cycles. 6 design decisions resolved, 5 defects addressed, 19 invariants. |
| [tokenomics.md](./tokenomics.md) | DOLI tokenomics SKELETON — supply mechanics, value inflows (incl. W2 AMM protocol fee), value outflows, bond-vs-LP capital allocation game (Cosmos/Osmosis problem), fee-switch policy (immutability over governance), Era-by-era security budget model, 7 open decisions (T1-T7). MUST-DO before lowering `amm_activation_height`. Produced from `docs/.workflow/defi-economic-review-2026-05-24-defi-subsystem.md`. |
| [defi-foundations-economics.md](./defi-foundations-economics.md) | DeFi foundations economics — 5-evaluator convergence synthesis. **APPROVED 2026-05-25: SSF + Option A** = 4 items (~135-325 LOC, 0 new activation heights, 0 new TX types, 0 new governance surfaces): P8 LPShare `is_conditioned()` fix, P5 pre-sim mempool contention signal, P7 escrow-loan CLI template, P3 MaxDelta + ReserveRatio guards. **Pre-activation decisions:** D1 `MINIMUM_LIQUIDITY=1000` (LOCKED), D2 `pool_id` includes `fee_bps` (LOCKED, IRREVERSIBLE once `amm_activation_height` crosses), D3 AC-2 split into AC-2a (intra-block 0 bps structural) + AC-2b (cross-slot residual scaling with swap-size-to-pool-depth, honest disclosure) **(ACCEPTED 2026-05-29)**, D4 AC-6 reframed as monitoring metric `R = total_active_bonds / max_pool_TVL` via `getDefiHealthMetric` RPC + Prometheus gauge **(ACCEPTED 2026-05-29)**. **Dropped permanently:** restitution slash (4/5 converged). **Deferred (Phase 2+):** oracle, intent+solver, batch settlement. |
| [oracle-structural-anchored-economics.md](./oracle-structural-anchored-economics.md) | Oracle Phase 2.1 economics — 5-evaluator convergence synthesis. Structural-anchored model: bond-weighted median aggregation, equivocation-only slash, per-epoch cadence, zero attestation reward, HALT sunset at 55% structural share. TxType 16 (PriceAttestation) + OutputType 15 (OraclePrice UTXO). **IMPLEMENTED 2026-05-25** behind `oracle_activation_height = u64::MAX` on all networks: M1 d80f127f (NetworkParams field), ME1 214a2e39 (error code taxonomy), M2 13e1ccd3 (`STRUCTURAL_PUBKEY_HASHES_HEX`), M3 19960adb (TxType 16 payload), M4 a82da836 (validation rules 1/2/3/6), M5 3d379ba6 (OutputType 15 + snap-sync), M6 62e13291 (bond-weighted median aggregator), M7 756dfaca (equivocation slash evidence), M8 ab59f278 (sunset HALT), M9 9fc8f1d1 (`getOraclePrice` RPC), M10 ee8520c2 (`getOracleAttestations` RPC), M11 2d28c4bf (`getOracleStatus` RPC). Phase 2.1 oracle is end-to-end complete, frozen behind `oracle_activation_height = u64::MAX`; pinning a real activation height is a separate decision session. |
| [event-subscriptions.md](./event-subscriptions.md) | Event subscriptions Phase 2.2 (PROPOSAL-ONLY, pending User Gate) — 5-evaluator convergence synthesis. Extend existing /ws with topic-filtered push (4 topics: blocks, utxo, epoch, consensus), in-memory ring buffer (100 blocks), lag-disconnect at 64, no auth, no activation height. Admin-gate pre-existing NewTx. Generic UTXO surface for oracle events. ~350 LOC. Ships in binary upgrade. |
| [defi-l1-foundations-architecture.md](./defi-l1-foundations-architecture.md) | DeFi L1 foundations architecture (2026-05-26, PROPOSAL-ONLY, pending User Gate) -- 5-evaluator convergence synthesis. KEEP: AMM (4 tx types), Oracle (Phase 2.1), ZKSettle (L2 settlement), Conditions (5 guards + 6 templates), FungibleAsset, BridgeHTLC. TOMBSTONE: native lending (5 tx types + 2 output types, 4/5 convergence), NFT-frac (2 tx types, 3/5 convergence). 3 mandatory pre-activation fixes: MintAsset issuer auth, compute_swap overflow, oracle sunset gradient. SSF alternative presented (radical minimum: conditions + ZKSettle only). |
| [amm-value-conservation-architecture.md](./amm-value-conservation-architecture.md) | AMM value-conservation architecture (INC-I-096, PROPOSAL-ONLY, 2026-05-29) -- 5-evaluator convergence synthesis. Single shared `verify_amm_conservation()` function with 3 per-asset balance equations (DOLI, token_b, LP supply) + k-invariant, replacing ~467 lines of fragmented conservation across 3 sites with ~130 lines in 1 new module (`validation/amm.rs`). Kills TCCR anti-pattern (Trust the Client's Computed Result) across all 4 AMM tx types. 9 hard filters from Failure Analyst, 12 invariants, 4 milestones. New findings: FM-S11 (asset_id counterfeiting), P5 (existing patch still drainable). Gated by `inc_i_096_activation_height`. |
| [rocksdb-configuration-architecture.md](./rocksdb-configuration-architecture.md) | RocksDB configuration architecture (INC-I-104) — first-principles per-instance, per-CF configuration for 3 RocksDB instances (block_store, state_db, utxo_store). Column families with workload-derived memtable budgets, WAL caps, bloom filters, L0 trigger protection. 93% reduction in theoretical memtable ceiling. |
| [epoch-liveness-prune-architecture.md](./epoch-liveness-prune-architecture.md) | Epoch-boundary liveness prune architecture (INC-I-116, PROPOSAL-ONLY, 2026-06-18) -- 5-evaluator convergence synthesis. Replace proportional 2/3 floor (mod.rs:209) with absolute MIN_PRODUCERS_FLOOR=3, gated by epoch_prune_activation_height. Extract shared compute_live_producer_list() to eliminate 170-line inline dupe in rewards.rs. 14 failure filters, 4 milestones, 3 user options (floor formula, grace period, ghost exclusion). |
| [sync-snap-admission-architecture.md](./sync-snap-admission-architecture.md) | SnapSync admission architecture (INC-I-139, PROPOSAL-ONLY, 2026-07-09) -- 5-evaluator convergence synthesis. Kill the 4-incident recurrence class (INC-I-005/033/138/139) by SUBTRACTION: delete the bare-gap Route A (decision.rs:168) + A1 redirect (dispatch.rs:96-117), narrow the Gate-1 floor for forward-large-gap reasons, single-owner evidence counter -- request_genesis_resync becomes the sole gap-based admission authority (5/5 convergence, rolling-safe Phase 1). Phase 2: FORK_GUARD wedge-escape behind new fork_choice_weight_tiebreak_activation_height (Q2=YES/Q3=NO). 4 code-verified contradiction resolutions, extended INV-SYNC-011 draft, 8 regression-test classes. |

## Future Interface Specifications

| File | Description |
|------|-------------|
| [l2-settlement.md](./l2-settlement.md) | L2 settlement interface — minimal L1 surface for ZK-rollup settlement via `ZKRollup` output type and `ZKSettle` transaction type. Verifier-as-pure-function API, permissionless verifying-key-in-UTXO design, activation via existing `ProtocolActivation` hard-fork mechanism. Zero changes to the consensus engine. |

## Requirements Specifications

| File | Description |
|------|-------------|
| [single-proposer-requirements.md](./single-proposer-requirements.md) | Single-Proposer-Per-Slot requirements - migration from multi-rank fallback to single proposer, attestation fork choice |
| [gui-desktop-requirements.md](./gui-desktop-requirements.md) | GUI Desktop Application requirements - Tauri 2.x cross-platform wallet with full CLI feature parity |
| [sdk-templates-requirements.md](./sdk-templates-requirements.md) | Covenant Guard CLI Parity + Template SDK requirements (workflow #356). Part A: 4 parser arms (threshold, amount_guard, output_type_guard, recipient_guard), multi-output spend upgrade, mainnet CLI warning. Part B (gated): 5 named template patterns (vault, escrow, htlc-payment, subscription, agent-allowance). Devnet/testnet only; no consensus changes. |
| [sdk-templates-architecture.md](./sdk-templates-architecture.md) | Covenant Guard CLI Parity + Template SDK architecture (workflow #356). Module structure (parser placement, mainnet warning, multi-output spend, test architecture), failure mode mitigations (S1-S8), 8 milestones (A1-A5 + re-gate + B1-B2). CLI-only; zero consensus changes. |

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
├── SPECS.md                              # <- You are here (specifications index)
├── protocol.md                           # Full protocol specification
├── architecture.md                       # Comprehensive architecture
├── security_model.md                     # Complete security model
├── state-of-the-art-architecture.md      # State-of-the-art redesign proposal
├── rocksdb-configuration-architecture.md # RocksDB configuration (INC-I-104)
├── single-proposer-architecture.md       # Single-proposer migration architecture
├── single-proposer-requirements.md       # Single-proposer migration requirements
├── gui-architecture.md                   # GUI Desktop Application architecture
├── gui-desktop-requirements.md           # GUI Desktop Application requirements
├── sdk-templates-requirements.md         # Guard CLI parity + template SDK requirements (#356)
├── sdk-templates-architecture.md         # Guard CLI parity + template SDK architecture (#356)
├── l2-settlement.md                      # L2 settlement interface (ZKSettle / ZKRollup)
├── improvements/
│   ├── apply-block-modularization.md     # Apply-block modularization
│   ├── cli-modularization.md             # CLI modularization
│   ├── consensus-modularization.md       # Consensus modularization
│   ├── modularization-improvement.md     # General modularization
│   └── scaling-100k-producers.md         # Scaling to 100K producers
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
