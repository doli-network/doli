# INC-I-092 Analysis — DeFi spend-path failures + pool inflation

## Architecture Context

DOLI is a UTXO chain. Spend authorization for an input is decided in TWO places that MUST agree:
- **Mempool admission**: `crates/mempool/src/pool.rs:277-355` (error codes MPTX001-MPTX007)
- **Full-block consensus**: `crates/core/src/validation/utxo.rs::verify_input_conditions` (utxo.rs:841) called per-input at utxo.rs:154

Both branch on `OutputType::is_conditioned()` (`crates/core/src/transaction/types.rs:311`):
- **Conditioned** (Multisig, Hashlock, HTLC, Vesting, NFT, FungibleAsset, BridgeHTLC, LPShare) → decode condition prefix from `extra_data`, decode witness from `tx.extra_data` (SegWit-style), `conditions::evaluate(condition, witness)`.
- **Non-conditioned** (Normal, Bond, **Pool**, ...) → `verify_input_signature`: requires a pubkey whose `hash_with_domain(ADDRESS_DOMAIN, pk) == output.pubkey_hash`.

Existing authorization carve-outs that bypass signature: **ZKRollup** (utxo.rs:153 — ZK proof is the signature) and **EpochReward pool inputs** (utxo.rs:42-70 — protocol-spent). AMM invariant enforcement for swaps ALREADY EXISTS in `validate_transaction_with_utxos` (utxo.rs:524-633: `new_k >= old_k`, token conservation, pool_id/asset_b/fee/LP-supply preservation) + the math in `crates/core/src/pool.rs` (`compute_swap_output`, `verify_invariant`).

## Corrected Root-Cause Split (the incident's single-covenant theory is PARTLY WRONG)

### RC-A — AMM pool UTXO is permanently unspendable (FINDING-002, the mainnet blocker)
- **Not a covenant bug.** `Output::pool()` (output.rs:806-836) sets `output_type=Pool` (NOT in `is_conditioned()`) and `pubkey_hash = pool_id`.
- A `Swap`/`AddLiquidity`/`RemoveLiquidity` spends the pool UTXO (input 0). Both authorization gates take the **signature path** and demand a pubkey hashing to `pool_id`. But `pool_id = BLAKE3(POOL_ID_DOMAIN ‖ fee_bps ‖ sort(a,b))` is NOT a pubkey hash → `PubkeyHashMismatch` / **[MPTX002]**. No key can ever satisfy it.
- **Intended model**: pool input authorized by the AMM invariant (already enforced), exactly like ZKRollup/EpochReward. **The pool-input signature exemption was never implemented.**
- **Fix location**: add a Pool-input authorization carve-out in BOTH utxo.rs (consensus) and mempool/pool.rs (admission), mirroring the ZKRollup pattern. Localized; CODE-FIXABLE. Invariant safety already covered by utxo.rs:524-633.
- Confidence: conf(0.9, code-read; needs FAIL→PASS test to reach fix-confidence).

### RC-B — pool_create accepts unfunded reserves → u64::MAX inflation (FINDING-001/-005/-006, P0-1)
- `Output::pool()` sets `amount = 0` — DOLI reserve_a lives in `extra_data`, NOT `Output.amount`. The DOLI-conservation check `total_input >= total_output` (utxo.rs:191-198) therefore compares native input against ~0 native output and **passes regardless of declared reserve_a**. So `reserve_a = u64::MAX` is accepted while only fee/change DOLI is actually consumed → 184B phantom DOLI.
- `validate_create_pool` (validation/pool.rs:39-44) only checks `reserve_a > 0`, never that the caller funded `reserve_a`.
- Duplicate-pool: rejected in apply_block (tx_processing.rs:128) but the deposit is still consumed → silent burn (no UTXO-context rejection in validation before spend).
- Zero-amount LPShare at MIN_LIQUIDITY boundary: structural validate passes; node rejects later.
- **Fix location**: enforce `declared reserve_a (+ reserve_b for token side) backed by net inputs` in CreatePool UTXO-context validation (utxo.rs), and reject duplicate pool_id at validation time. CODE-FIXABLE.
- Confidence: conf(0.85, code-read).

### RC-C — channel/bridge covenant condition↔witness tree mismatch (FINDING-003/-004, [MPTX007])
- These ARE conditioned (HTLC, BridgeHTLC). `evaluate(condition, witness)` returns false → **[MPTX007]**. This is where the original "witness vs condition" theory genuinely applies.
- Early signal: `cmd_bridge` lock builds `And(Multisig, htlc_signed_refund)` (cmd_bridge.rs:164-166) for one flow, but `cmd_bridge_claim` attaches only `branch(left)+preimage` (cmd_bridge.rs:380) — a witness tree that may not match the locked condition tree. Channel close path analogous.
- **Fix location**: align CLI witness builders (cmd_channel.rs/cmd_bridge.rs) with the condition templates (conditions/templates.rs) and evaluator (conditions/eval.rs). Needs per-subsystem tracing. CODE-FIXABLE but the most intricate of the three.
- Confidence: conf(0.55, code-read — needs deeper tracing of each lock/spend pair).

## Impact / Blast Radius
- All three touch **consensus validation** for height-gated AMM/covenant tx types.
- `amm_activation_height`: mainnet = effectively disabled (u64::MAX-style); **testnet = 20099, current h≈22280 → AMM is LIVE on testnet.** Changing AMM validation on testnet is a consensus-shape change requiring coordinated deploy or a new activation height (local net has ~30 external producers — no synchronized stop-all).
- HTLC/BridgeHTLC covenants (RC-C) are NOT amm-gated — they are general covenant spends, so a fix there affects all covenant HTLC users. Needs an activation-height gate if it changes acceptance of already-locked UTXOs.
- Mainnet AMM activation (`amm_activation_height` → real value) is BLOCKED until RC-A and RC-B are fixed + verified e2e.

## Specs/docs drift flagged
- `specs/defi-foundations-economics.md` / CLAUDE.md describe "amm_activation_height (AMM Foundations M1, shipped)" — but the **spend authorization path for pools was never wired**, so M1 is functionally incomplete for swaps. Doc claims more than code delivers.

## TRIAGE VERDICT
```
━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.88, multi-component consensus bug with 3 distinct root causes)
Reasoning: Spans 3 subsystems (AMM/channel/bridge) across mempool + consensus validation; consensus-critical; original unifying theory was wrong. RC-A and RC-B already root-caused by direct code reading; RC-C needs per-subsystem covenant tracing. Architecture is CODE-FIXABLE (invariant enforcement already present; fixes are localized auth/validation carve-outs + CLI witness alignment).
━━━━━━━━━━━━━━━━━━━━━━
```

## Recommended fix order (each TDD: failing test FIRST)
1. RC-A (AMM pool-input auth exemption) — unblocks all pool spends; smallest, highest-leverage.
2. RC-B (pool_create reserve funding + duplicate + zero-LPShare) — closes inflation.
3. RC-C (channel/bridge covenant witness alignment) — most intricate; per-subsystem.

---

## Open Findings Inventory (resume 2026-05-29, RUN_ID=382)

P0 consensus blockers (RC-A/RC-B) shipped in 92eff255; RC-C → INC-I-093 (0c39b031). The 11 remaining findings are ALL CLI-side (no consensus rule / block-content change → no activation height required). Node validation unchanged.

⚠️ P2-008..013 detailed descriptions were never persisted (entry 874). The set below is RECONSTRUCTED by white-box audit and CONFIRMED with the user (2026-05-29).

| ID | Severity | File:line | Defect | Fix | Test seam |
|----|----------|-----------|--------|-----|-----------|
| P0-005 | P0 | cmd_pool.rs:271 | `lp_shares < MINIMUM_LIQUIDITY` guard uses `<`; at exact boundary creator_lp_shares=0 → zero-amount LP output → node rejects, fee burned | `<` → `<=` | unit (pure boundary math) |
| P1-006 | P1 | cmd_pool.rs:264 (create) | No pool-existence pre-check; duplicate pool_id rejected by node in apply_block, inputs consumed → silent burn | getPoolInfo pre-check; bail if pool exists | integration |
| P1-007 | P1 | cmd_channel.rs:70 (open) | No `remote_hash != local_hash` guard → self-channel buildable | bail if peer == self | unit (extractable) |
| P2-008 | P2 | cmd_pool_add:857-868 | Missing pool → `unwrap_or(0)` reserves → builds doomed tx | bail if pool fields absent | integration |
| P2-009 | P2 | cmd_pool_remove:1117-1128 | Same unwrap_or(0) pool-existence gap | bail if pool fields absent | integration |
| P2-010 | P2 | cmd_pool_add:894-901,990 | `new_shares==0` (tiny deposit) → zero-amount LP output → fee burned | bail if new_shares==0 | unit |
| P2-011 | P2 | cmd_pool_create:238-264 | asset_b non-existence only surfaces at UTXO-selection as confusing "insufficient token" | clearer pre-check/error | integration |
| P2-012 | P2 | cmd_channel Pay:228-279 | Mutates local store without verifying funding confirmed on-chain | warn/guard on unconfirmed funding | integration |
| P2-013 | P2 | cmd_nft/mint.rs:34 | amount 0 silently clamped to 1 via `max(1,..)` instead of erroring | bail on 0 (explicit) | unit (extractable) |
| P3-014 | P3 | cmd_nft/list.rs:24 | `nft list` filters outputType=='nft'; mint emits EncryptedContent → invisible | include EncryptedContent UTXOs in list (DECISION: show both) | integration |
| P3-015 | P3 | commands.rs PoolCommands::Info | `pool info <id>` positional vs `--pool` elsewhere | accept positional OR --pool (DECISION: accept both) | unit (clap parse) |

User decisions (2026-05-29): P3-014=show both in list; P3-015=accept both forms; P2 set=fix all 6 reconstructed.

### Triage (resume)
All 11 are FAST-path, localized CLI edits across cmd_pool.rs, cmd_channel.rs, cmd_nft/{mint,list}.rs, commands.rs. No DEEP investigation; root causes are read-confirmed. Deterministic seams get unit tests (P0-005, P1-007, P2-010, P2-013, P3-015); RPC-dependent guards verified via scripts/test_defi_e2e.sh on live testnet (h≈24467, past activation).

---

## Verification Verdict (2026-05-29, RUN_ID=382)

White-box re-verification of the reconstructed P2 set found **5 of 6 were already-guarded non-bugs** — NOT fixed (no fabricated changes):
- **P2-008/009** (add/remove missing pool): `getPoolInfo` returns RPC error -32007; `call_raw` (rpc_client.rs:437) maps any `error` field to `Err`, propagated by `?` before `unwrap_or(0)`. Clean error already.
- **P2-010** (zero LP on add): `compute_lp_shares` (pool.rs:68-70) returns `None` on shares==0; `cmd_pool_add` `.ok_or_else()?` already errors.
- **P2-012** (pay unconfirmed): `ChannelState::is_active` (types.rs:68) is true only for `Active`; `Open` sets `FundingBroadcast`; `Pay` bails on `!is_active`.
- **P2-013** (nft amount 0): default `--amount="0"` (commands.rs); 0→1 dust clamp is intentional (NFT value defaults 0, protocol needs non-zero UTXO). Erroring would break the default mint.

### Fixes applied (6 real defects)
| ID | Fix | Verification |
|----|-----|--------------|
| P0-005 | cmd_pool.rs `creator_lp_shares_on_create`: `<` → `<=` | unit FAIL→PASS (`p0_005_create_rejects_zero_creator_shares`) |
| P1-006 | cmd_pool_create: getPoolInfo duplicate pre-check | live testnet: re-create existing pool bails before broadcast |
| P1-007 | cmd_channel `ensure_distinct_channel_parties` guard | unit (`p1_007_rejects_self_channel`) |
| P2-011 | cmd_pool_create: distinct "holds 0 of asset" error | build + read |
| P3-014 | nft list includes EncryptedContent section | live testnet: 9 encrypted items now listed |
| P3-015 | `pool info` accepts positional OR `--pool` | unit FAIL→PASS (`p3_015_...`) + live both forms |

Gates: `cargo build --release -p doli-cli` ✓, `clippy --all-targets -D warnings` ✓, `fmt --check` ✓, `cargo test -p doli-cli` 202+3 pass ✓. All changes CLI-side — no consensus rule / block-content change → no activation height. Node binary unchanged.
