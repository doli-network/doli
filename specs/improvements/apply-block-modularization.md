# Requirements: apply_block.rs Modularization

> Split `bins/node/src/node/apply_block.rs` (1,022 lines) into sub-modules.
> Pattern: follows `production/` — orchestrator in `mod.rs`, domain helpers in sub-modules.

## Scope

**Target**: `apply_block.rs` -> `apply_block/` directory with 6 files
**Gate**: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`
**Risk**: EXTREME — consensus-critical state transition function

## Proposed Split

| Module | Content |
|--------|---------|
| `mod.rs` | Orchestrator: pre-check, setup, tx loop dispatch, persistence |
| `tx_processing.rs` | Per-tx UTXO ops, 7 epoch-deferred producer mutations, unbonding |
| `governance.rs` | MaintainerAdd, MaintainerRemove, ProtocolActivation |
| `state_update.rs` | Known producers, chain state, state root, finality, epoch deferred |
| `genesis_completion.rs` | Genesis phase end: derive producers, create bonds from pool |
| `post_commit.rs` | Tier, epoch snapshot, attestation, archive, websocket |

## Key Constraints

- `apply_block()` signature unchanged: `pub(super) async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()>`
- All helpers are `pub(super)` on `impl Node`
- Write guards passed as parameters (never re-acquired in helpers)
- Single `batch.commit()` in `mod.rs` — no helper calls commit
- Lock acquisition order preserved exactly
