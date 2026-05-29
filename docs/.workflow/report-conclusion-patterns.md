# Report Conclusion — Pattern Matcher

Source: `docs/.workflow/design-patterns.md`

## Anti-pattern
**"Trust the Client's Computed Result" (TCCR)** — aka client-side authority / oracle-free declared state — across ALL 4 AMM tx types. The validator accepts attacker-declared new state (`new_reserve_a/b`, `new_total_lp`) instead of deriving it from transition inputs. 9 declared values trusted; only 1 (CreatePool reserve backing, INC-I-092 RC-B) fixed.

## Matched patterns (the fix)
- Account-based AMMs (Uniswap v2): reserves READ from storage, new state COMPUTED by contract, never declared by caller. UTXO analogue: derive new_reserve from the consumed Pool UTXO's old extra_data ± actual asset flows.
- Codebase already implements the correct "validator recomputes from measured flows" pattern at 5 sites incl. RC-B (`utxo.rs:857-918`), `validation_checks.rs:446-529`, `utxo.rs:210-262`, `pool.rs:68-79`, `pool.rs:111-121`. P1 = mechanically generalize RC-B to the other 3 AMM types.
- Multi-asset UTXO conservation (Cardano `Value = map<AssetId,u64>`, CKB cell model): per-asset value-conservation ledger → P2 dual-asset conservation, bounded to AMM types.
- Floor-division (H1): "round in protocol's favor with `<=`" — dust to pool.

## Proposals
- **P1** conf(0.65): generalize RC-B derive-from-flows to all 4 AMM types.
- **P2** conf(0.6, observed): dual-asset (DOLI + token_b + LP) conservation equation.
- **P3** conf(0.65): shared validation function (kills D2 by construction). `mempool/pool.rs:951` unconditional sum vs `utxo.rs:185` is_native filter confirmed.
- **P4 PARTIALLY KILLED:** validator calling `compute_swap()` directly is over-constraining for Swap; use conservation + k-invariant for Swap, `compute_remove_liquidity` for LP ops.

## ⚠️ CRITICAL — existing patch is NOT secure (P5, CONFIRMED)
The EXISTING INC-I-096 patch still has a live drain: `shares_burned = old_m.total_lp_shares - new_m.total_lp_shares` (`utxo.rs:796-808`) is computed from attacker-declared `new_total_lp`, NOT from consumed LPShare UTXOs. `utxo.rs:778-780` only checks LP shares decreased. Set `new_total_lp=0` → proportional cap inflates to full pool → drainable. **On devnet (gate=0) this drain is OPEN right now.**

## Disproved
- "Fee-change output breaks DOLI conservation": KILLED — `>=` inequality absorbs fee-change in Normal outputs.

## Gaps
- 25/5 bps protocol fee extraction path not located.
- `apply_block/tx_processing.rs` not examined.

## Cross-layer signals
- Silent-pass risk: `if let (Some(old_m), Some(new_m))` at `utxo.rs:723` silently passes on malformed AddLiquidity Pool metadata — use `ok_or_else` like `utxo.rs:611-613`.
