# INC-I-308 M3 evidence — operator tools send the admin token; installers print safe RPC access (run 732)

Branch `fix/rpc-admin-access`, parent 5cc7e23c (M1). Contract: `docs/bugfixes/inc-i-308-rpc-admin-access-analysis.md`.
Consensus shape: (1) no (2) no (3) n/a — tools and installers only. No activation height, no version bump.

## Outcome metric
- Metric: files under scripts/, testnetlinux/, bins/cli/src that name an admin method and carry no token mechanism
  (probe: grep for the 14 names, minus files that reference Authorization / rpc-admin-token / rpc_admin_token / admin_token).
- before = 48, after = 35. The remaining 35 are covered through a shared, patched function (tn.rpc, nodes.rpcp,
  gs024-lib, devnet-lib, RpcClient for cmd_chain/cmd_guardian), are unit-test fixtures, or name a method only as a
  dict key / registry text — each is an EXCEPTION with a reason in `scripts/tests/test-rpc-admin-token.sh` (census check h)
  or is proven by the CLI wire tests below. The census (check h) is the authoritative gate and passes.

## Red / green
| Test | Red on parent (4cf8be85 + M1) | Green |
|---|---|---|
| `bins/cli/src/rpc_admin_auth_tests.rs` (admin_methods_match_rpc_server_list, public_method_is_not_admin, token_resolves_from_env_first, token_resolves_from_file_when_env_absent_or_empty, token_none_when_neither, unauthorized_hint_names_env_and_file) | compile FAIL: `cannot find function resolve_admin_token` etc. | pass |
| `bins/cli/src/rpc_client_auth_tests.rs` (admin_call_carries_bearer_header, public_call_carries_no_auth_header, admin_unauthorized_reply_prints_hint, typed_admin_call_carries_bearer_header) | compile FAIL: `no method named with_admin_token` | pass |
| `scripts/tests/test-rpc-admin-token.sh` | FAIL a (helper missing), FAIL h (23 of 24 admin callers lack a token path) | exit 0 |
| `scripts/tests/test-installer-rpc-bind.sh` | FAIL c (install-services.sh guidance, ssh -L), d (install.sh, cmd_service.rs), e (relay listen + authorization forward) | exit 0 |

`cargo test -p doli-cli`: exit 0, 488 passed, 0 failed. `cargo clippy -p doli-cli --all-targets -- -D warnings`: exit 0.
`rustfmt --check` on the changed CLI files: exit 0. `bash -n` on every patched .sh: clean.

## CLI (R-5)
# INC-I-308 M3 — CLI half evidence (run 732)

## R-5 — CLI admin callers

| Caller (file:line) | Admin method(s) | Token source | Proving test |
|---|---|---|---|
| bins/cli/src/cmd_guardian.rs:103 (`guardian halt`) | pauseProduction | RpcClient::post (rpc_client.rs:484): explicit, else env, else `<data dir>/rpc-admin.token` if endpoint is loopback | rpc_client_auth_tests::admin_call_carries_bearer_header |
| bins/cli/src/cmd_guardian.rs:132 (`guardian resume`) | resumeProduction | same (call_raw) | admin_call_carries_bearer_header (same path) |
| bins/cli/src/cmd_guardian.rs:163 (`guardian checkpoint`) | createCheckpoint | same (call_raw) | admin_unauthorized_reply_prints_hint (header sent + -32008 hint) |
| bins/cli/src/cmd_chain.rs:38 -> rpc_client.rs verify_chain_integrity (`chain-verify`) | verifyChainIntegrity | same (typed call -> call_inspect -> post) | typed_admin_call_carries_bearer_header |
| bins/cli/src/cmd_snap.rs:423 query_state_root (`snap`) | getStateRootDebug | env DOLI_RPC_ADMIN_TOKEN only (cmd_snap.rs:477) | no unit test (raw reqwest to remote seeds) |
| bins/cli/src/cmd_snap.rs:453 download_snapshot (`snap`) | getStateSnapshot | env DOLI_RPC_ADMIN_TOKEN only | no unit test |
| any public method via RpcClient | none | never sends Authorization | public_call_carries_no_auth_header |

Classification + resolution: rpc_admin_auth::tests (6 tests: list parity with crates/rpc/src/server.rs, public-not-admin, env-first, file trim LF/CRLF, none-when-blank, one-line hint).

## Decisions

- D-snap: `doli snap` sends a bearer to remote seeds ONLY from env DOLI_RPC_ADMIN_TOKEN; the local token file is this node's credential and is never sent to a remote seed. A -32008 reply bails with a one-line hint (cmd_snap.rs:484).
- D-loopback (added beyond the brief): RpcClient reads the local token FILE only when the endpoint is loopback (127.0.0.1 / localhost / [::1]); a non-loopback `--rpc` gets only the env token. Same leak argument as D-snap; ssh -L tunnels are loopback so they still work.
- `with_admin_token` has no production caller (test seam; production uses lazy env/file resolution); marked `#[allow(dead_code)]` like `call_raw`. Wiring-debt row needed (docs/.workflow is outside this writer's scope).

## Service install guidance

- bins/cli/src/service_rpc_guidance.rs (15 lines) prints token path, CLI auto-read, `ssh -L` tunnel, never 0.0.0.0.
- Called from cmd_service.rs:405 (systemd) and :508 (launchd); cmd_service.rs net +3 lines.
- scripts/tests/test-installer-rpc-bind.sh: `PASS d cmd_service.rs prints rpc-admin.token guidance`; `PASS a` still passes.

## Green

- `cargo test -p doli-cli`: exit=0, 14 test-result lines, 488 passed, 0 failed (/tmp/m3-cli-green.txt).
- `cargo clippy -p doli-cli --all-targets -- -D warnings`: exit=0.
- `cargo fmt -p doli-cli -- --check`: exit=0 (rustfmt reflowed one line in rpc_admin_auth_tests.rs; no assertion change).

## Scripts, installers, relay (R-5, R-3, P1-006 relay half)
# INC-I-308 run 732 M3 — scripts/installers evidence (developer, scripts half)

Green: test-rpc-admin-token.sh exit=0 (/tmp/m3-script-green-1.txt, census 13 callers + 11 verified exceptions);
test-installer-rpc-bind.sh exit=0 (/tmp/m3-script-green-2.txt, incl. d cmd_service.rs from the CLI developer).

## Helpers (new)
- scripts/lib/rpc-admin-token.sh: rpc_admin_token, rpc_admin_curl_args (test contract); rpc_admin_port_data_dir (lsof+ps --data-dir);
  rpc_admin_header_for <method> <url|host:port> (header only for the 14 admin methods; used as `curl -H @<(...)`, token off argv);
  rpc_admin_ssh <host> <service> <port> <json> (token piped on stdin or read on the remote host from the unit's --data-dir; curl -H @-).
- scripts/lib/rpc_admin_token.py: admin_token, auth_headers (test contract), ADMIN_METHODS, data_dir_for_port, headers_for(method, port, data_dir).
- Verified live: echo server got `Bearer zzzz` for pauseProduction, None for getChainInfo; fake-ssh sim got the token on curl stdin (file and env paths).

## R-5 patched callers (17 files)
| File | Admin methods | Token source | Proof |
|---|---|---|---|
| emergency-halt.sh | pauseProduction (local + mainnet ssh) | local: header_for (env / port data dir); mainnet: rpc_admin_ssh (remote unit data dir, sudo -n cat) | census h, bash -n, ssh sim |
| emergency-resume.sh | resumeProduction | header_for | census h, bash -n |
| seed-backup.sh | createCheckpoint | header_for | census h, bash -n |
| test_guardian.sh | createCheckpoint, pause/resumeProduction | header_for(ENDPOINT) | census h, bash -n |
| gauntlet-gs024-lib.sh | verifyChainIntegrity (gs024_rpc/irpc) | header_for | bash -n, test_gauntlet_gs024.sh exit 0 |
| gauntlet-gs026.sh | pause/resumeProduction | header_for (method parsed from body) | census h, bash -n, gs026 size 516 ≤ 517 |
| gauntlet-devnet-lib.sh | call()/rpc() for gs027/028/029 | header_for, lazy-sourced inside rpc() (lib must stay functions-only) | bash -n, test_gauntlet_gs027/028/029/devnet_checks exit 0, bash smoke |
| gauntlet-inc-i-301-diskfull.sh | getStateRootDebug (embedded gs054.py) | helper copied to RUN_DIR, headers_for(port) | census h, bash -n, py_compile (extracted) |
| gauntlet-inc-i-302-rewind.sh | getStateRootDebug | header_for | census h, bash -n |
| inc-i-235-drill.sh | enter/exitRecoveryMode, pause/resumeProduction | header_for(port) | census h, bash -n |
| inc-i-300-repair-restart.sh | getStateRootDebug (heredoc python) | RPC_ADMIN_LIB env -> headers_for(port) | census h, bash -n, py_compile (extracted) |
| gauntlet-collect.py | getStateRootDebug | headers_for(port) | census h, py_compile |
| gauntlet-gs033/tn.py | shared tn.rpc | headers_for(data_dir=NODES[i]["datadir"]) | py_compile, test_inject_order/test_restore_nowipe exit 0 |
| gauntlet-stranded/nodes.py | shared nodes.rpcp | headers_for(port) | census h, py_compile, stranded unit tests exit 0 |
| gauntlet-stranded/state_parity_watch.py | getStateRootDebug, createCheckpoint | headers_for(port) | census h, py_compile |
| install-services.sh, install.sh | (guidance only, R-3) | — | installer test c/d |

## Exceptions added (verified by reading; all route through a patched shared function)
fork-lab/gs035.py (tn.rpc :329), gauntlet-gs033/probes.py (tn.rpc :180), gauntlet-stranded/{bank (:54), fastpath (:249,:291),
gsn01 (:141,:369 aligned), gsn02_strand (:53,:54)}.py (nodes.rpcp / tn.rpc); fork.py and strand.py (method name is only a result dict
key; the call is nodes.integrity -> nodes.rpcp); gauntlet-gs016-runner/replica.py (token=True -> lib.rpc sends Authorization lib.py:35;
scratch node started with DOLI_RPC_ADMIN_TOKEN=TOKEN :123, honored by admin_token.rs:40); gauntlet-gs024.sh (gs024-lib);
gauntlet-gs029-probes.sh (devnet-lib call/rpc).

## R-3 installers
| Installer | Bind | Token written? | Guidance printed | Why |
|---|---|---|---|---|
| scripts/install-services.sh (fleet) | 0.0.0.0 kept | no | rpc-admin.token + ssh -L 8500:127.0.0.1:8500 + Bearer | explorer reads seed/producer RPC remotely |
| scripts/install.sh | 127.0.0.1 (check b) | no | rpc-admin.token in data dir + ssh -L + Bearer | end-user installer |
| install.ps1, bins/node/postinst(.sh), docker-entrypoint.sh, install-local-services.sh, install-stress-services.sh, testnetlinux/scripts/install-services.sh | 127.0.0.1 / no 0.0.0.0 (check b) | no (b, f) | not required | unchanged |

## Relay (P1-006)
testnetlinux/explorer/server.js: HOST = env EXPLORER_HOST || "127.0.0.1"; server.listen(PORT, HOST); WebSocket upgrade path drops
the `authorization` raw header; JSON-RPC proxy already sends a fixed header set. node --check OK.

## Explorer sync (ACTION)
scripts/install.sh is synced from the explorer repo `../explorer/doli.network/install.sh` — NOT edited here; the explorer copy must get
the same 3 echo lines (after "doli chain") or the next sync reverts them.

## Not patched / open
testnetlinux/explorer/network.html:471 calls verifyChainIntegrity from the browser through the relay (no token, outside scripts/**):
it will get -32008 now; needs a product decision (drop the panel, or use a public method).
testnetlinux/scripts/**: no admin-method calls found.
Count: 15 caller scripts patched + 2 installers + relay + README = 19 files; 2 helper files new.

## Callers OUTSIDE this repo (or outside M3 scope) that will need the token after M1
- Explorer integrity panel: `../explorer/explorer.doli.network/js/network-integrity.js:61,78` — browser `verifyChainIntegrity`
  through `/api/mainnet/*` nginx proxies; gets -32008 now. Needs a server-side reader with the token or removal of the panel.
- Kit explorer page: `testnetlinux/explorer/network.html:471` calls `verifyChainIntegrity` through the relay; the relay
  strips `authorization`, so this panel gets -32008 (decision needed; not patched).
- Node-to-node backfill Phase 2: `crates/rpc/src/methods/backfill.rs` calls ADMIN `verifyChainIntegrity` on the peer URL
  with no header (M2 owns that file).
- `doli snap` against public seeds: admin `getStateRootDebug`/`getStateSnapshot`; works only with an operator-supplied
  `DOLI_RPC_ADMIN_TOKEN` (the local token file is never sent to a remote host).
- Explorer repo copy `../explorer/doli.network/install.sh` must be synced with `scripts/install.sh` (guidance lines added here).
- Skills / docs with copy-paste admin curls and no header (`.claude/skills/guardian/reference/procedures.md`, `rpc/SKILL.md`,
  `mainnet/RECOVERY.md`, `doli-network/SKILL.md`, `docs/archiver.md`) — outside M3 file ownership.

## Open items
- `RpcClient::with_admin_token` is used by tests only (production resolves lazily); `rpc_client.rs` was already over
  the 500-line budget (1009 → 1074).
- `cmd_snap` env-only token path has no unit test.

## M6a — verifier fix round (run 732)

Findings from `docs/verifications/verify-4cf8be8531e6-a602869fc1bc.md`:
1. P1 — `is_loopback` was a substring test, so the CLI attached the local `rpc-admin.token` for non-loopback hosts.
2. P1 — `network.html` integrity panel swallowed the `-32008` admin refusal and showed nothing.

RED (pre-fix code):
- `cargo test -p doli-cli --bins m6a_` → `must not be loopback: http://localhost@attacker.example:8500`, 0 passed / 1 failed.
- `bash scripts/tests/test-installer-rpc-bind.sh` section g → `FAIL g ... handles -32008`, `FAIL g ... shows admin token message`.

Fix:
- `bins/cli/src/rpc_client.rs` `is_loopback`: parse with `reqwest::Url`; reject userinfo; host must be a loopback IP (127.0.0.0/8, ::1) or exactly `localhost`.
- `testnetlinux/explorer/network.html`: `rpc()` keeps the JSON-RPC error code; on `-32008` the panel shows "Integrity check needs the node admin token (admin method). Run `doli chain-verify` on the node host."; other errors show a fixed message with the numeric code only. No token or auth header in the page or relay.

GREEN:
- `cargo test -p doli-cli --bins m6a_` → 1 passed.
- `cargo test -p doli-cli` → 489 passed, 0 failed.
- `cargo clippy -p doli-cli --all-targets -- -D warnings` → exit 0; `cargo fmt --check` → exit 0.
- `bash scripts/tests/test-installer-rpc-bind.sh` → all pass (section g: 3 PASS).

Outcome probe (real binary, mock RPC listener on 127.0.0.1, token file in `DOLI_DATA_DIR`, endpoint `http://localhost@127.0.0.1:PORT`, `doli chain-verify`): `token_leak_userinfo_url` before (a602869f) / after — see the M6a commit body.

## M7 — loopback-only token file + repair genesis abort (run 732)

Fixes:
- `scripts/lib/rpc-admin-token.sh` `rpc_admin_header_for`: reads the local token file only when the URL host is exactly loopback (`127.a.b.c`, `localhost`, `::1`, `[::1]`) and the authority has no userinfo (`@`). Other hosts get only `DOLI_RPC_ADMIN_TOKEN`, else no header. `scripts/lib/rpc_admin_token.py` adds `headers_for_host(method, host, port)` with the same rule; `headers_for` is unchanged because its callers pass a local port or data dir only.
- `crates/rpc/src/methods/guardian.rs` `repairArchiveFromPeer`: the genesis check runs on the decoded block BEFORE `repair_check::admissible`, so a foreign-genesis peer aborts with `wrong genesis_hash — peer is on a different chain` instead of a count in `errors`.

Tests:
- `scripts/tests/test-rpc-admin-token.sh` section `i` (REQ-RPC-ADMIN-LOOPBACK).
- `audit_p1_rpc_007_repair_archive_reports_different_chain_for_foreign_genesis` in `crates/rpc/tests/it/audit_rpc_effects.rs`.

RED (pre-fix):
- section `i`: 4 FAIL (remote `203.0.113.9`, `10.0.0.5`, `localhost@203.0.113.9`, `127.0.0.1.evil.example` with env unset got the file token).
- `cargo test -p rpc audit_p1_rpc_007_repair_archive_reports_different_chain_for_foreign_genesis` → `want different-chain error: ... "errors":3 ... "status":"ok"`, 0 passed / 1 failed.

GREEN: see the results below.
- `bash scripts/tests/test-rpc-admin-token.sh` → ALL PASS (section `i`: 11 PASS).
- `cargo test -p rpc` → 165 passed, 0 failed, 1 ignored (incl. `audit_p1_rpc_007_repair_archive_reports_different_chain_for_foreign_genesis`).
- `cargo clippy -p rpc --all-targets -- -D warnings` → exit 0; `cargo fmt --check` → exit 0; `bash -n scripts/lib/rpc-admin-token.sh` and `python3 -m py_compile scripts/lib/rpc_admin_token.py` → exit 0.

## M8 — bare-port callers and the repairArchiveFromPeer anchor note

- RED on HEAD 2ae61693: `bash scripts/tests/test-rpc-admin-token.sh` → `FAIL i loopback 8545 → file token — got: ` (1 check failed). A bare port parsed as host `8545`, not loopback, so no file token was sent.
- Fix: `scripts/lib/rpc-admin-token.sh` `rpc_admin_header_for` treats an all-digits argument as the local node on 127.0.0.1. A URL `http://8545` stays remote (new remote case in section i passes before and after).
- GREEN: `bash -n scripts/lib/rpc-admin-token.sh` ok; `bash scripts/tests/test-rpc-admin-token.sh` ALL PASS; `bash scripts/tests/test-installer-rpc-bind.sh` ALL PASS.
- Doc: `docs/rpc_reference.md` repairArchiveFromPeer note no longer says "Run again to extend the range"; it states that only the height next to the lowest stored block can be anchored, admitted blocks go to the archive only, and other heights need `backfillFromPeer` into the store first.
- P3 (CLI env token < 32 chars) NOT done: existing tests in `bins/cli/src/rpc_admin_auth_tests.rs:101-107` use short env tokens; the change needs a test rewrite and a CLI build, over the 3-minute limit. Open.
