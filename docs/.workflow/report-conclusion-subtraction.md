# Report Conclusion — Subtractionist

Source: `docs/.workflow/design-subtraction.md`

## Top finding
15 AMM checks across 4 sites (~691 lines). ~231 mempool lines are redundant reimplementations of consensus logic with divergent/weaker semantics (root of D1/D2). ~258 consensus lines are ad-hoc structural checks that trust declared pool state instead of recomputing. Correct pool math exists in `crates/core/src/pool.rs` but is never called by the validator ("builder computes, validator trusts" anti-pattern → D3, D4, unbound AddLiquidity).

## Proposals (ordered by subtraction purity)
- **P5** conf(0.7): delete `verify_invariant` dead export (called only in pool.rs tests).
- **P3** conf(0.65, observed): remove mempool pool-aware conservation for AMM txs entirely (-73 lines, 0 added). Block assembly runs full consensus validation before inclusion (`assembly.rs:235`), so mempool conservation is a perf filter, not a security gate. Eliminates D1+D2 by removing one side of the parity equation. Smallest viable subtraction.
- **P1** conf(0.6): replace mempool reimplemented validation with `MempoolUtxoProvider` delegating to consensus (-~200 net). Eliminates D2 by construction.
- **P2** conf(0.55, inferred): replace ~258 lines of ad-hoc trust-declared-state checks with recompute-and-verify using pool.rs math (-158 net). Structurally eliminates declared-vs-actual trust → D3/D4.
- **P4** conf(0.6): add FungibleAsset input binding (+35 lines) to close D4/H2. Pure addition.

## Disproved
- "Removing mempool conservation = unbounded DoS": disproved (mempool count/size limits + one-pool-tx-per-block contention; consensus authoritative).
- "Full mempool delegation trivial": partially disproved — mempool still needs fee computation for non-AMM eviction priority.

## Gaps
- 25/5 bps protocol fee mechanics not traced — critical for recompute model (P2): if 5 bps is a separate DOLI output vs reserve deduction, the conservation equation changes.
- Existing INC-I-096 patch not compared for conflicts.

## Cross-layer signals
- Confirms D2 is structural copy-paste (`mempool/pool.rs:951` unconditional sum vs `utxo.rs:185` is_native filter).
- "Builder computes validator trusts" may exist in other tx types (NFT royalties, BridgeHTLC).
