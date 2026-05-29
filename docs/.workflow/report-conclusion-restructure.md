# Report Conclusion — Restructurer

Source: `docs/.workflow/design-restructure.md`

## Structural defects
- **SD-1 (Compute/Verify Split):** The 7 canonical AMM math functions in `crates/core/src/pool.rs` are called by CLI (`cmd_pool.rs:603`) and RPC (`pool.rs:191`) ONLY — zero calls from any validation/enforcement site. `validation/utxo.rs` (~250 AMM lines) and `mempool/pool.rs` (~120 lines) re-implement weaker ad-hoc checks. conf(0.65, observed).
- **SD-2 (Duplicate Conservation):** native DOLI conservation copy-pasted across mempool and consensus with historically divergent semantics (D2). INC-I-096 patch deepens this by copy-pasting a 15-line block into both sites instead of extracting shared code.
- **SD-3 (Asset-Blind Accounting):** token_b (FungibleAsset) has no conservation framework. NEW gaps beyond the 6 defects: AddLiquidity has ZERO token_b input binding in any site (incl. INC-I-096 patch); Swap B→A has ZERO token_b input binding.

## Proposals
- **P1** conf(0.65, observed): extract shared `AmmTransitionVerifier` / `verify_amm_transition` to new `crates/core/src/validation/amm.rs` that CALLS pool.rs math. Both mempool and consensus delegate. Kills SD-1+SD-2 structurally. Dependency-clean: `storage::UtxoSet` already implements `core::validation::UtxoProvider` (`storage/src/utxo/set.rs:404`) — no circular dep.
- **P2** conf(0.55): `AmmFlowSummary` struct for per-asset flow accounting.
- **P3** conf(0.60): shared lookup-agnostic input-classification accumulator (resolves mempool unconfirmed-parent edge case).
- **P4** conf(0.60): validator calls pool.rs math to re-verify declared new pool state (kills D3/D4 by construction).
- **P5 KILLED** by C6 (new inc_i_096_activation_height mandatory; cannot consolidate heights).

## Before/After
```
BEFORE: mempool/pool.rs → reimplements conservation
        validation/utxo.rs → reimplements conservation + binding
        core/pool.rs → unused by validation/mempool
AFTER:  mempool/pool.rs → core::validation::amm → core::pool.rs
        validation/utxo.rs → core::validation::amm → core::pool.rs
```

## Cross-layer signals
- SD-1 mirrors the known `calculate_epoch_rewards` / `calculate_expected_epoch_rewards` disconnection (CLAUDE.md) — recurring codebase pattern.
- P1 alone is the minimum structurally sound fix; P2/P3/P4 are refinements.
