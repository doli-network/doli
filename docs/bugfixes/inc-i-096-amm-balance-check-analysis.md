# INC-I-096 — AMM value-conservation check rejects valid RemoveLiquidity — Analysis

**Incident:** INC-I-096  **Run:** 384  **Branch:** defi/foundations  **HEAD:** 4e5b9685
**Scope:** crates/mempool/src/pool.rs, bins/node/src/node/apply_block, crates/core/src/validation/{utxo.rs,pool.rs}, crates/core/src/network_params

## Symptom (reproduced on testnet h=25,334 v=6.23.0)
- `doli pool remove --shares 95238` → `RPC -32002 (MPTX008): insufficient funds: input=720095238 < output=819999906 (deficit=99904668)`.
- `doli pool remove --shares 100` → mempool-accepted ("Liquidity removed successfully") but `blockHeight=None` forever (consensus rejects at apply, or fee/dust slips mempool then fails block validation).

## Architecture Context (verified by reading code, not grep)

The native DOLI value-conservation check exists at **TWO** sites that must agree or the network forks:

1. **Mempool** — `crates/mempool/src/pool.rs:383`
   `if total_input < total_output { MPTX008 }`. `calculate_inputs` (pool.rs:874) sums only `utxo.output.amount`. The Pool UTXO has `output_type=Pool`, `amount=0` (reserves live in `extra_data`), so its DOLI reserve contributes **0** to `total_input`.

2. **Consensus** — `crates/core/src/validation/utxo.rs:210-217`
   `if total_input < total_output { InsufficientFunds }`. Input loop at line 185 adds to `total_input` **only when `utxo.output.output_type.is_native_amount()`** — Pool is not native, so again contributes 0. The comment at lines 196-199 deliberately keeps AMM types subject to this balance check ("native output can NEVER exceed native input — prevents coin creation"); INC-I-092 exempted them only from the **fee** check (lines 223-231).

`tx.total_output()` (transaction/core.rs:701) sums native `Output.amount`, which **includes** the `doli_out` Normal output the user receives from pool reserves. Result: for RemoveLiquidity (and B→A Swap) `total_output > total_input` legitimately, because the released reserve DOLI is real value that exists only in the Pool UTXO's `extra_data`, never as a counted input.

### ROOT CAUSE (conf 0.9, code-traced)
The native conservation equation omits the Pool UTXO's reserve release. The released DOLI (`old_reserve_a − new_reserve_a`) is value that legitimately funds `doli_out` but is invisible to both balance checks. Both sites reject valid AMM reserve-unlocking transactions.

## ⚠️ SECURITY FINDING — the incident's prescribed fix is UNSAFE (conf 0.85, code-traced)

The incident prescribes "mirror INC-I-092 RC-A": add a blanket **value-flow exemption** for AMM Pool-input tx types, claiming "the constant-product invariant in validate_remove_liquidity already authorizes the unlocked value." **This claim is false.** I verified:

- `validation/pool.rs` is **structural only** (its own doc, line 3-4): checks input/output counts and types. No reserve/proportionality math.
- `apply_block/*` does **no** AMM invariant binding (only a duplicate-pool-id guard, tx_processing.rs:131). The pool.rs comment "checks happen in apply_block" is stale.
- `validation/utxo.rs` RemoveLiquidity validation (696-735) checks ONLY: `pool_id` preserved, reserves **decreased-or-equal**, `total_lp_shares` **decreased**. It does **NOT** bind the user's `doli_out`/`tokens_out` to the reserve deltas, nor enforce proportional withdrawal (`shares_burned/total ⇒ reserve delta`). The proportional math in `crates/core/src/pool.rs:77-78` is a **builder helper, never called by consensus**.

**Consequence:** Today the buggy native balance check is the ONLY thing bounding `doli_out` (you can't output more DOLI than your external inputs). A **blanket exemption removes that bound**, enabling an attacker to submit a RemoveLiquidity that decreases `reserve_a` by X and pays themselves `doli_out=X` while burning only **1 LP share** — draining the pool / stealing from other LPs. The buggy check currently *accidentally prevents* this. **Fixing liveness naively unmasks an LP-theft / value-extraction vector.**

(Token side: `tokens_out` is FungibleAsset, not in the native sum, so it is *already* unbound for RemoveLiquidity today — a latent issue independent of this fix, but the same proportional binding closes it.)

## Correct fix shape (to be confirmed by deep investigation)
For AMM tx with Pool input 0 (Swap/AddLiquidity/RemoveLiquidity), gated by a **NEW `inc_i_096_activation_height`** (do not reuse `inc_i_092` — immutable per CLAUDE.md; mainnet=u64::MAX pinned with amm_activation_height; testnet=future height > 25,334 for rolling deploy; devnet=0):

1. **Pool-aware native conservation** (both mempool + consensus): replace the naive check with
   `total_input + old_pool.reserve_a ≥ total_output + new_pool.reserve_a`
   (difference = fee). Preserves no-coin-creation: output DOLI bounded by external inputs + actual pool reserve.
2. **Proportional-withdrawal binding** (consensus): for RemoveLiquidity, bind `doli_out == old_reserve_a − new_reserve_a` and `tokens_out == old_reserve_b − new_reserve_b`, and the deltas proportional to `shares_burned/total_lp_shares`. Without this, (1) alone still permits LP theft.
3. Tighten Swap B→A exact-output binding (utxo.rs:632-644 only bounds `≤ reserve_a`, not exact).

## Impact analysis
- Consensus-visible: flips reject→accept for valid AMM txs AND reject→reject(stronger) for theft txs. INC-I-075 checklist: Q1=YES, Q3=NO → **activation height REQUIRED**.
- Both mempool and consensus must change together and gate on the same height, or mixed-fleet fork.
- ~30 external testnet producers → no synchronized stop; height gate + lead time mandatory.
- Mainnet AMM is disabled (`amm_activation_height=u64::MAX`) → no live mainnet exposure today; the gate must remain u64::MAX on mainnet, pinned with amm_activation_height.

## Drift flagged
- `crates/core/src/validation/pool.rs:4` comment "Invariant (x*y=k) and reserve checks happen in apply_block" is **stale** — they happen in `validation/utxo.rs`. Update during fix.

━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.9, code-traced root cause; 0.85, security finding)
Reasoning: Liveness root cause is clear, BUT the reported/prescribed fix is unsafe — it unmasks an LP-theft vector because consensus never binds AMM outputs to reserve deltas. Fix spans 3 interacting components (mempool, consensus conservation, AMM invariant validation) across 2 crates, is consensus+money critical, and the correct fix is materially larger than the incident assumed. Independent verification + security chain warranted (Rule 17).
━━━━━━━━━━━━━━━━━━━━━━
