# INC-I-095 — pool remove selects LP UTXO from the wrong pool (MPTX007)

## Fundamentals Quick-Check
- Build: workspace builds (INC-I-092/093 shipped on this branch). N/A to re-run for a localized CLI bug.
- Bug is deterministic, reproducible from description, localized to LP-UTXO selection. Not capacity/resource.
- Occam ordering: not config/env/resource — it is a code-logic selection bug. Confirmed by reading the code.

## Symptom
`doli pool remove --pool <X> --shares N --yes` fails at mempool admission with
`[MPTX007] input 1 covenant condition not satisfied` whenever the wallet holds LP UTXOs
from more than one pool. No funds lost (mempool rejects before inputs are spent); UX is opaque.

## Root Cause (confirmed by reading code)
`cmd_pool_remove` (`bins/cli/src/cmd_pool.rs:1248-1258`) selects LP UTXOs to burn with:
```rust
for utxo in &all_utxos {
    if utxo.output_type == "lpShare" && lp_total < shares_to_burn { /* push as input */ }
}
```
It picks the **first** `lpShare` UTXO with sufficient amount, **without checking its embedded pool_id**.
When the wallet holds LP shares from multiple pools, input 1 can be an LP UTXO from a *different* pool
than `--pool`. The node's LPShare covenant requires the matching pool_id to be spent on input 0, so it
correctly rejects with MPTX007.

LP UTXOs encode their pool_id in `extra_data` as `[condition_bytes][1B version][32B pool_id]`,
extracted by `Output::lp_share_metadata()` (`crates/core/src/transaction/output.rs:944`).

## Architecture Context & Scope Correction
The incident hypothesis assumed the CLI could filter on `extra_data[0..32]`. **Code is SOT and shows it cannot**:
the `getUtxos` RPC response (`crates/rpc/src/types/chain.rs` → `UtxoResponse`) exposes neither `extra_data`
nor the LP pool_id. For an LPShare its `condition` is only `Signature(owner)` — no pool_id. The CLI `Utxo`
struct (`bins/cli/src/rpc_client.rs:66`) likewise has no pool_id.

Therefore the minimal correct fix needs a small **additive, read-only** RPC field surfacing the LP pool_id,
then a CLI-side filter. This expands scope to `crates/rpc/` (one struct field + populate in 2 branches).

**Three-question consensus checklist (INC-I-075):**
1. User-submittable tx triggers this? **No** — RPC response formatting + CLI tx construction (off-chain).
2. Producer/attestation pattern triggers it? **No.**
3. Bit-identical block behavior? **Yes** — node validation/production unchanged; only the client now selects
   correct-pool LP UTXOs. **No activation height. Safe for rolling deploy.** Additive JSON field; old clients ignore it.

## Fix (SSF — one behavioral change + the data plumbing to enable it)
1. `crates/rpc/src/types/chain.rs`: add `pool_id: Option<String>` to `UtxoResponse` (skip-if-none, like `nft`/`asset`).
2. `crates/rpc/src/methods/balance.rs`: populate it for `LPShare` via `output.lp_share_metadata()` in both the
   confirmed-UTXO branch and the mempool-pending branch.
3. `bins/cli/src/rpc_client.rs`: add `pub pool_id: Option<String>` (`#[serde(default)]`) to `Utxo`.
4. `bins/cli/src/cmd_pool.rs`: extract a pure helper `select_lp_share_utxos(&[Utxo], target_pool_id, shares_to_burn)`
   that filters `output_type == "lpShare" && pool_id == Some(target)`; wire it into `cmd_pool_remove`; emit a
   clear error when the wallet has LP shares but none for the target pool (tells user about stale cross-pool LP UTXOs).

## Impact Analysis
- `cmd_pool_remove` is the only consumer of LP-UTXO selection. AddLiquidity/CreatePool construct their own LP outputs
  and are unaffected.
- New RPC field is additive; no existing consumer breaks. `lp_share_metadata()` already unit-tested in core.
- Blast radius: CLI remove path + getUtxos response shape (additive). No consensus, no storage, no block content.

## Specs/docs drift
- `docs/rpc_reference.md` getUtxos response should document the new `poolId` field for lpShare.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, code-read + covenant semantics confirmed)
Reasoning: Deterministic, root cause located in one selection loop; fix is a pool_id filter + additive RPC field to supply it.
━━━━━━━━━━━━━━━━━━━━━━
