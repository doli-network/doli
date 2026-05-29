# INC-I-097 — Channel state never transitions FundingBroadcast → Active

**Severity:** High · **Path:** FAST · **Scope:** `crates/channels/src`, `bins/cli/src/cmd_channel.rs`
**Status:** Resolved · **Branch:** defi/foundations

## Symptom

`doli channel pay` fails with **"Channel … is not active (state: FundingBroadcast)"** for every channel in the wallet — including channels whose funding tx confirmed thousands of blocks ago (h≈22,226) and whose funding multisig UTXOs are present and spendable on-chain. Surfaced 2026-05-29 when E2E Phase 12 first exercised channel pay.

## Root cause (confirmed against code)

**Type (a): no executable code path advances `FundingBroadcast` → `Active`.**

All building blocks already existed:
- `state_machine.rs:15` already permits the `FundingBroadcast → Active` transition.
- `ChannelRecord::transition()` (`channel.rs:59`) applies validated transitions.
- `channels::rpc::RpcClient::get_transaction_status()` returns `{ block_height, confirmations }`.
- `monitor.rs::check_channel()` already produces a `FundingConfirmed` event for `FundingBroadcast` channels.

The single missing piece: **no caller ever queries the funding tx's confirmation count and applies the transition.** `channel open` (`cmd_channel.rs:211`) sets `state = FundingBroadcast` and persists, then nothing observes the chain. The `monitor.rs` machinery is never run by any loop, and the CLI never invokes it. `channel pay`'s `is_active()` guard (`cmd_channel.rs:262`) therefore blocks forever. The funding tx hash was already stored (`funding_outpoint.tx_hash`), so no schema change was needed.

This was misread by prior analyses as "monitor.rs is a stub that never emits events" — in fact `check_channel` does emit `FundingConfirmed`; the gap is purely orchestration.

## Fix (SSF)

Lazy, on-demand activation at the start of the `pay` path — no background daemon.

1. **`ChannelRecord::try_activate(confirmations, required) -> bool`** (`channel.rs`) — pure, I/O-free. Records the observed confirmation count while in `FundingBroadcast`; transitions to `Active` when `confirmations >= required.max(1)` (the `.max(1)` guards against a misconfigured `required == 0` activating an unconfirmed channel). Returns `true` iff the state changed. Unit-testable without a node.

2. **CLI `Pay` lazy refresh** (`cmd_channel.rs`) — before the `is_active()` guard, if the channel is `FundingBroadcast`, query `get_transaction_status(funding_tx_hash)`, call `try_activate(confs, config.funding_confirmations)`, and persist on change. On RPC error: print a warning and leave state untouched (the existing guard then correctly blocks). Already-`Active` channels skip the refresh (no redundant RPC).

The `is_active()` guard is unchanged — payments on genuinely unconfirmed channels are still rejected.

## Why this is not a consensus / deploy hazard

- `doli-node`'s `Cargo.toml` does **not** depend on `channels`; the crate is client-side (CLI) only. Channel state lives in `channels.json` next to the wallet; the node sees only standard multisig UTXOs.
- Deploy-safety Q1 (consensus RULES changed?): **NO**. Q2 (block CONTENT changed?): **NO**.
- INC-I-075 three-question checklist: touches none of active_producers / scheduler / bond snapshot / attestation bitfield / coinbase. **No activation height required.**
- Deploy = ship the new `doli` CLI binary. No synchronized stop-all, no node redeploy.

## Tests

`crates/channels/tests/inc_i_097_funding_activation.rs` — 7 cases over the `{O1 return, O2 state, O3 funding_confirmations} × {P1 non-FundingBroadcast, P2 below threshold, P3 ≥ threshold}` matrix plus the defensive `required==0` zero-conf guard. FAILED before the fix (method absent), PASSES after. Full `channels` suite: 108 lib + 7 new = green. clippy/fmt clean.

## Requirements

| ID | Priority | Status |
|----|----------|--------|
| REQ-CHAN-001 lazy state refresh on confirmation | Must | Done |
| REQ-CHAN-002 graceful RPC-unreachable degradation | Must | Done (warn, leave state) |
| REQ-CHAN-003 `is_active()` guard not weakened | Must | Done (unchanged) |
| REQ-CHAN-006 funding tx hash stored for later query | Must | Already satisfied (`funding_outpoint`) |
| REQ-CHAN-004 `list`/`info` show refreshed state | Should | Deferred — `list`/`info` are offline-capable; adding a node round-trip expands scope. `pay` (the blocker) is fixed. |

## Verification

E2E `scripts/test_defi_e2e.sh` Phase 12 (channel pay) is the end-to-end check on a live testnet.
