# INC-I-308 M5 — evidence

## Symptom (measured)

> `backfillFromPeer` Phase 2 (crates/rpc/src/methods/backfill.rs ~lines 191-261) POSTs `verifyChainIntegrity` to the peer with NO Authorization header. Since M1, every admin method needs `Authorization: Bearer <token>`, so the peer refuses (-32008, logs "Admin method 'verifyChainIntegrity' from 127.0.0.1 rejected: invalid or missing bearer token"). The caller reads the missing `result.chainCommitment` as "no difference" and replies `{"started": false, "message": "No gaps detected — chain is complete."}` with no log — the divergence scan is skipped silently.

## RED

Command: `cargo test -p rpc --test inc_i_308_m5_backfill_peer_token` (work tree `fix/rpc-admin-access`, HEAD `1c57fdc8`, production code unchanged).

Result: `test result: FAILED. 0 passed; 3 failed`.

- `inc_i_308_m5_peer_token_sent_on_verify_chain_integrity` — FAILED at line 238:
  `verifyChainIntegrity must carry Authorization: Bearer <peer_token>; got [None], resp={"id":1,"jsonrpc":"2.0","result":{"message":"No gaps detected — chain is complete.","started":false}}`
  (serde ignores the unknown `peer_token`; the request goes out with no header.)
- `inc_i_308_m5_refused_divergence_check_is_reported` — FAILED at line 271:
  `refused peer must be reported in `divergence_check`, got silent reply: {"id":1,"jsonrpc":"2.0","result":{"message":"No gaps detected — chain is complete.","started":false}}`
- `inc_i_308_m5_local_token_never_sent_to_peer` (real `RpcServer`, admin_token L, HTTP call with `Bearer L`, peer_token P) — FAILED at line 326:
  `verifyChainIntegrity must carry the peer token: [("getBlockByHeight", None), ("verifyChainIntegrity", None)]`
  The fixture checks before it passed: the server accepted the admin call, the peer was contacted, and no request carried L.

## GREEN

Command: `cargo test -p rpc --test inc_i_308_m5_backfill_peer_token` → `test result: ok. 3 passed; 0 failed`.

- `inc_i_308_m5_peer_token_sent_on_verify_chain_integrity` — ok
- `inc_i_308_m5_refused_divergence_check_is_reported` — ok
- `inc_i_308_m5_local_token_never_sent_to_peer` — ok

Full crate: `cargo test -p rpc` → 158 passed, 0 failed, 1 ignored (lib 113, inc_i_173_m3 8, inc_i_236_m2_backfill 1, inc_i_281_ma_backfill 1, inc_i_308_m5 3, it 32 + 1 ignored).
`cargo clippy -p rpc --all-targets -- -D warnings`: clean. `cargo fmt --check`: clean.

Design: Phase 2 now goes through `backfill_peer_commitment::fetch(client, url, peer_token)`. It returns the commitment, or a skip reason when the peer is unreachable, replies non-JSON, returns an `error` (reason names `peer_token`), or sends no `chainCommitment`. A skip logs `warn!` and the no-gap reply carries `divergence_check: "skipped: <reason>"`; a compared commitment gives `"ok"`. The bearer header is built only from `params.peer_token`; the local admin token is not reachable from this code path. The gap/repair path is unchanged. backfill.rs went from 742 to 740 lines (net -2).

Files changed:
- `crates/rpc/src/types/chain.rs` — `BackfillParams.peer_token: Option<String>` (`#[serde(default)]`).
- `crates/rpc/src/methods/backfill_peer_commitment.rs` — new, 38 lines.
- `crates/rpc/src/methods/backfill.rs` — Phase 2 uses the helper; `divergence_check` in the no-gap reply.
- `docs/rpc_reference.md` — `peer_token` parameter, `divergence_check` in Behavior and in the no-gap response.
- `crates/rpc/tests/inc_i_308_m5_backfill_peer_token.rs` — comment-only: `INPUT PARTITIONS:` colon and a `covers:` line for the new module (pipeline-gate requirements); no assertion changed.

Scripts not changed: `scripts/gauntlet-stranded/gsn01.py:141` and `fastpath.py:249` backfill into stores that have holes, so Phase 2 (which runs only with no gaps) is never reached there; the pipeline gate also refuses edits to files the M5 test does not cover.
