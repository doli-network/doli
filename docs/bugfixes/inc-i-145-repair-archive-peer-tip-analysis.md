# INC-I-145 — repairArchiveFromPeer peer-tip parse always fails (bestHeight camelCase)

## Bug

`repair_archive_from_peer` (`crates/rpc/src/methods/guardian.rs:443-468`) fetches the
peer's chain tip via `getChainInfo` and parses it with:

```rust
body.pointer("/result/height")
    .or_else(|| body.pointer("/result/best_height"))
```

But `getChainInfo` (`crates/rpc/src/methods/network.rs:46-68`) serializes
`ChainInfoResponse` (`crates/rpc/src/types/chain.rs:50`), which carries
`#[serde(rename_all = "camelCase")]` — so the tip field is emitted as **`bestHeight`**.
Neither pointer can ever match. Every call returns:

```
{"code":-32603,"message":"Peer did not return chain height in getChainInfo"}
```

Confirmed on mainnet (binary v6.23.10 / ec6afc52) with a live peer returning
`{"result":{"bestHash":"...","bestHeight":110140,"bestSlot":...}}`. The method is 100%
non-functional since introduction (INC-I-055 replacement for the manual tar+scp relay).

## Architecture Context

- **Module**: `crates/rpc/src/methods/guardian.rs` — guardian/admin RPC methods. The
  affected function is a leaf handler on `RpcContext`; the code graph shows it only
  references `Value`/`RpcError`/`Result` and is reached solely via RPC dispatch
  (`RpcContext` method edge). No other code path consumes the parse result.
- **Data flow**: admin RPC call → SSRF validation (`validate_backfill_url`) → HTTP POST
  `getChainInfo` to peer → parse `/result/*` tip → background loop fetching
  `getBlockRaw` per height, BLAKE3-verified, written to archive dir.
- **Blast radius**: the fix changes only the JSON pointer fallback chain inside this
  one function. `getChainInfo` output is untouched. No consensus, no block content,
  no wire protocol — pure RPC-client-side parse. Rolling deploy safe; no activation
  height, no version bump.
- **Producer of the parsed JSON**: `ChainInfoResponse` in `crates/rpc/src/types/chain.rs`
  — `best_height: u64` under `rename_all = "camelCase"` → `bestHeight`.

## Root Cause

Field-name drift between the RPC response serializer (camelCase via serde rename) and
the hand-written JSON-pointer consumer (guessed `height` / `best_height`). Classic
encoder/decoder parity failure at the string level; never covered by a test because the
pointer chain was inline in an HTTP-coupled async method.

## Fix (SSF — one change)

Extract the pointer chain verbatim into a testable pure helper
`parse_peer_chain_tip(body: &Value) -> Option<u64>` and add `"/result/bestHeight"` as
the **first** pointer (it is the actual field), keeping `/result/height` and
`/result/best_height` as back-compat fallbacks per the user's constraint.

## Requirements

- **REQ-I145-001 (Must)**: `repair_archive_from_peer` parses the peer tip from a
  `getChainInfo`-shaped response `{"result":{"bestHeight":N,...}}`.
  AC: unit test feeds that shape and asserts parsed tip == N. FAIL before fix, PASS after.
- **REQ-I145-002 (Must)**: existing fallbacks `/result/height` and `/result/best_height`
  still parse. AC: unit tests assert both shapes parse.
- **REQ-I145-003 (Must)**: no change to `getChainInfo` output or any other behavior.
  AC: diff touches only `guardian.rs` parse path + tests.

## Impact analysis

Restores the automated cross-seed archive repair path (currently worked around by
manually relaying `.block`/`.blake3` files + `bridgeFromArchive`). No callers besides
RPC dispatch; no other consumer of the helper.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.95, measured — field name confirmed at chain.rs:49-50 serde rename_all=camelCase; live mainnet response shape confirms)
Reasoning: Deterministic, single-function, single-file parse bug with confirmed root cause; no cross-module interaction.
━━━━━━━━━━━━━━━━━━━━━━

Milestones: single milestone (1 file + test). No milestone split needed.
