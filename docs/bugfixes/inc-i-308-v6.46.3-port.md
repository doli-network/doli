# INC-I-308 — port onto v6.46.3

v6.46.3 = v6.46.2 (`328ac1da`) + the INC-I-308 RPC admin access fix only. Run 737.
Plan: `docs/.workflow/run737-port-plan.md` (main checkout, not shipped).

## Source range

`4cf8be85..612876e4` on the fix branch (19 commits). 17 commits are cherry-picked in order with `-x`.

Skipped commits (unreleased fixture work only):

- `3120256f` (M15): GS-045 / GS-052 stranded-node fixtures.
- `612876e4` (M16): same fixtures.

## Dropped files (14)

These files belong to the unreleased stack. The fix only adds the admin token to their RPC calls.

- `scripts/gauntlet-inc-i-301-diskfull.sh`
- `scripts/gauntlet-inc-i-302-rewind.sh`
- `scripts/inc-i-300-repair-restart.sh`
- `scripts/gauntlet-stranded.sh`
- `scripts/gauntlet-stranded/{nodes,state_parity_watch,fastpath,gsn01,replay,storefill}.py`
- `scripts/test_gauntlet_stranded_replay_side.py`
- `scripts/test_gauntlet_stranded_storefill.py`
- `bins/node/examples/gs_store_fill.rs`, `bins/node/examples/gs_store_fill/tests.rs`

## Hand resolutions

- `crates/rpc/src/server.rs`, `server/auth_tests.rs`, `tests/it/audit_rpc_access.rs`, `tests/it/audit_rpc_access_fixture.rs`: whole file from `5cc7e23c`. `server/proxy_tests.rs` from `612876e4` (test-module split).
- `crates/rpc/tests/it/main.rs`: 2 `mod audit_rpc_access*` lines added by hand.
- `bins/node/tests/it/main.rs`: base file + `mod inc_i_308_m10_script_token_harness;` only.
- `docs/rpc_reference.md`, `scripts/README.md`, `scripts/gauntlet.sh`: base text + the INC-I-308 lines only. The `forceReorgTo` section keeps the v6.46.2 behavior text; only its access sentence changes (token from every address).

- `scripts/gauntlet-gs016.sh` (not in the fix diff): `_gs016_rpc` calls `forceReorgTo` and `verifyChainIntegrity` with no token at v6.46.2. On the fix branch its token path came from unreleased GRET Run 4b (`4472226e`, `bfa8e4aa`). Only that token body is ported (token from `DOLI_RPC_ADMIN_TOKEN`, sent on stdin). The `scripts/gauntlet.sh` GS-016 banner gets a comma so the caller census does not read display text as a call. Found by `scripts/tests/test-rpc-admin-token.sh` check h.

## Owner rulings

- **D-1**: KEEP `OPERATOR_TOKEN_METHODS` in `crates/rpc/src/server.rs`. It is a label read by tests only; `check_admin_auth` does not read it.
- **D-8**: BRING `scripts/gauntlet-stranded/loopback_bind.c` (from `612876e4`) so `gauntlet-rpc-admin-lib.sh` can build its P2P shim for GS-058 / GS-063. No other `scripts/gauntlet-stranded/` file ships.

## Not in this release

- `forceReorgTo` keeps the v6.46.2 behavior (no GRET Run 4b rules). Only its access changes.
- Not changed: `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, activation heights, `NetworkParams`, `HardForkSchedule`.

## Evidence limit

The evidence docs `docs/bugfixes/inc-i-308-*.md` were measured on `612876e4` (fix branch binary), not on v6.46.3. `inc-i-308-run735-*-analysis.md` also describes the GS-045 / GS-052 fixtures, which do not ship here.

## Send-back 1 — operator traps on structural hosts

Structural hosts set `DOLI_RPC_ADMIN_TOKEN` in the systemd unit and have no token file, so
`sudo doli -r http://127.0.0.1:<port> guardian halt` was refused `-32008` unless the operator
exported the token. Four fixes:

- **D1 node** (`crates/rpc/src/admin_token.rs`): an env or config token is also written to
  `<data_dir>/rpc-admin.token` (atomic, 0600) when the file is absent or differs; a matching
  file only gets its mode tightened. A write failure is logged without the token and does not
  stop the node. Removed behavior: "env token leaves no file on disk" is replaced by "the file
  holds exactly the env token, mode 0600".
- **D2 CLI** (`bins/cli/src/rpc_admin_locate.rs`): loopback lookup order is env token >
  `DOLI_DATA_DIR` > data dir of the process listening on the port (Linux `/proc/net/tcp{,6}` +
  `/proc/<pid>/fd`; macOS `lsof` + `ps`) > network default dir. When a listener on the port is
  visible, a candidate token file (any of the three dirs) is sent only if the listener's uid owns
  it; else no token is sent. Only with no visible listener are `DOLI_DATA_DIR` / the default dir
  used unchecked. The shell and Python helpers got the same uid check (they never fall back).
  The `-32008` hint names the file tried and `sudo`, or says the port's process is not owned by
  the token file's owner, or that the CLI could not identify the node on the port.
- **Run 737 Linux validation** (`docs/.workflow/run737-linux-validation.md`): F1 — a refused
  listener still got the default dir's token (root's `/var/lib/doli/mainnet` token reached an
  unprivileged fake listener); fixed by `choose_token_dir` (`bins/cli/src/rpc_admin_locate.rs`).
  F2 — the hint named the default file as the node's when no listener was visible; fixed.
  F3 — test 6 of `test-unit-token-tools.sh` drops to user `nobody` when run as root (else SKIP).
- **D3 installer** (`scripts/install-services.sh`): a re-run carries an existing unit's
  `DOLI_RPC_ADMIN_TOKEN` Environment line into the new unit; `validate` hides that line.
  Test: `scripts/tests/test-installer-unit-token.sh` (red on `8e065a3a`, `FAILS: 4`; green now).
- **Brought test (owner ruling)**: `crates/storage/tests/inv_deploy_003_rocksdb_memory_caps.rs`
  + its `[[test]]` entry, byte-equal from `e20c9ed1` (run 721), test-only: INV-DEPLOY-003 lists
  `install-services.sh` and the gate needs its linked test mutation-proved (KILLED) on this branch.
- **D4 snap** (`bins/cli/src/cmd_snap_wipe.rs`): `rpc-admin.token` is in the preserve list.
- **D5** `scripts/gauntlet-gs016.sh` `_gs016_rpc` uses `rpc_admin_header_for`.

Tests: `scripts/tests/test-unit-token-tools.sh <doli-node> <doli>` (scratch node, end to end),
`scripts/tests/test-rpc-admin-token.sh` k2-k4,
`admin_token::tests`, `rpc_admin_locate::tests`, `cmd_snap_wipe::tests`.
