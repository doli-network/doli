# INC-I-099 — CLI auto-fee underestimates for medium-size covenant tx

## Symptom
`doli template escrow --send ...` prints "Transaction hash: ..." then fails:
`RPC error -32002 (FEE_TOO_LOW): fee too low: 1 < 2`.
Vault template (smaller condition) does not hit it. Threshold is at/above escrow output size.

## Architecture Context
- Template handlers (`bins/cli/src/cmd_template/*`) build a covenant `Condition`, serialize it to a
  `--condition` string, and delegate to `cmd_wallet::cmd_send` via `vault::send_with_condition`.
- `cmd_send` is the single shared send path for both `doli send --condition` and all template `--send` calls.
- The node's mempool admission requires `fee >= tx.minimum_fee()` where
  `minimum_fee = BASE_FEE + (sum(output.extra_data.len()) * FEE_PER_BYTE) / FEE_DIVISOR`
  (`crates/core/src/transaction/core.rs:690`; constants BASE_FEE=1, FEE_PER_BYTE=1, FEE_DIVISOR=100).
- Covenant outputs encode the condition into `output.extra_data`. Escrow's condition ≥100 bytes →
  `minimum_fee = 2`. Plain transfers (0 extra_data) → `minimum_fee = 1`.

## Root Cause
`cmd_send` (`bins/cli/src/cmd_wallet.rs:403,417`) uses a **flat auto fee of `1`** when `--fee` is omitted:
`let fee_units = explicit_fee.unwrap_or(1);`. This ignores the size-scaled node minimum. For any
covenant/large-extra_data output where `minimum_fee() > 1`, the CLI builds a tx with fee=1 and the node
rejects it. The NFT command path already fixed this (`cmd_nft/buy.rs:242` uses `tx.minimum_fee()`); the
generic send path was never updated.

## Fix (SSF)
When `--fee` is omitted, derive the auto fee from the transaction's own outputs using the **same**
`Transaction::minimum_fee()` the node enforces, instead of the flat `1`. The fee depends only on output
`extra_data` lengths (recipient condition); the change output is a normal output with empty extra_data, so
the fee is independent of UTXO selection and change amount — it can be computed from the recipient output
before selection. An explicit `--fee` still overrides (preserves the high-fee warning path and user control).

Blast radius: `cmd_send` only. Fixes template `--send` AND direct `doli send --condition`. Plain transfers
unchanged (auto fee stays 1). No consensus/protocol change — CLI-side fee selection only.

## Scope Note
Incident scope listed `bins/cli/src/cmd_template`, but the defect is in the shared `cmd_send`
(`bins/cli/src/cmd_wallet.rs`) that templates delegate to. Fixing it there is the root-cause fix.

## Acceptance Criteria
- REQ-099-001 (Must): auto fee for a covenant output with extra_data ≥100 bytes equals the node's
  `minimum_fee()` (≥2), not flat 1.
- REQ-099-002 (Must): auto fee for a plain transfer (0 extra_data) stays 1 (no regression).
- REQ-099-003 (Should): explicit `--fee` still overrides the auto value.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, code-traced)
Reasoning: Deterministic, localized to one default-fee line in cmd_send; formula and a fixed precedent (NFT path) both confirmed in code.
━━━━━━━━━━━━━━━━━━━━━━
