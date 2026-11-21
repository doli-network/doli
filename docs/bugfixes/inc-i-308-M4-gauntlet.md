# INC-I-308 M4 — gauntlet rows GS-056, GS-057, GS-058, GS-061 on the END binary (run 732)

Date: 2026-11-21. Rows proposed by `docs/audits/security-audit-rpc-2026-11-21.md` (§ "GS-056").
GS-059 and GS-060 (same audit) are NOT registered in this milestone.

## Binary under test (END)

| Item | Value |
|------|-------|
| Build | `cargo build --release -p doli-node` in the `fix/rpc-admin-access` worktree, exit 0 |
| `doli-node --version` | `doli-node 6.46.2 (5cc7e23c)` |
| sha256 | `8d67993186c3759f60714dde325863fc101cfc3797fdb8e4deaf5e2c4942a204` |
| HEAD | `5cc7e23c7111f17d17f434ffb06544de40274911` (M1) |
| Uncommitted M2 source in the binary | `crates/crypto/src/address.rs`, `crates/rpc/src/methods/{backfill,guardian,pruning}.rs`, `crates/rpc/tests/it/main.rs` (`git diff -- crates/ \| shasum -a 256` = `56964f32…e636`); untracked `crates/rpc/src/methods/backfill_neighbor.rs` (`68181bb9…958f`), `guardian_repair_check.rs` (`429eeecb…b9`) |
| Frozen copy used by every run | `$TMPDIR/inc308-m4/doli-node-end` (same sha). Not copied to `~/testnet/bin`. |

Control binary (pre-INC-I-308): a copy of `~/testnet/bin/doli-node`, `doli-node 6.46.1 (7a8c3eb8)`,
sha256 `f7fdbc06…5754`. The copy ran as a scratch node only. The audit's own parent probe
(`ceacc42d`, `docs/audits/rpc-access-probe-evidence-2026-11-21.md`) is the other parent reference.

## Rows and registration

| Row | Runner | Kind | expect_fail |
|-----|--------|------|-------------|
| GS-056 rpc-admin-gate-by-source | `bash scripts/gauntlet-gs056.sh` | observational, fleet | 0 |
| GS-057 rpc-relay-does-not-launder-loopback | `bash scripts/gauntlet-gs057.sh` | 1 scratch node + 1 relay | 0 |
| GS-058 rpc-admin-gate-scratch-node | `bash scripts/gauntlet-gs058.sh` | 1 scratch node | 0 |
| GS-061 rpc-bind-and-token-census | `bash scripts/gauntlet-gs061.sh` | observational, no RPC | 0 |

Shared helpers: `scripts/gauntlet-rpc-admin-lib.sh`. Registered in `gauntlet_scenarios`
(`.omega/memory.db`, status `active`) and appended to `scripts/gauntlet-seed.sql`; `scripts/gauntlet.sh`
maps the `gs056-*|gs057-*|gs058-*|gs061-*` tokens to "standalone" (skip), as for GS-041; one bullet
in `scripts/README.md`. GS-057 is registered `expect_fail=0` (the fix is in; the audit's
`expect_fail=1` was for the pre-fix code).

Scratch node: `--network testnet`, data dir under `$TMPDIR`, RPC/P2P/metrics 8591/30391/9091
(GS-058) and 8592/30392/9092 + relay 8597 (GS-057), `--rpc-bind 127.0.0.1 --no-dht --no-discv5
--no-auto-update --no-snap-sync`, no `--bootstrap`, loopback bind shim built from
`scripts/gauntlet-stranded/loopback_bind.c`. Stopped by PID; data dir deleted, `node.log` kept.

## Commands

```bash
export GA_BIN=$TMPDIR/inc308-m4/doli-node-end
for m in gauntlet-gs056 gauntlet-gs057 gauntlet-gs058 gauntlet-gs061; do
  bash ../../../.claude/scripts/gauntlet-run.sh ../../../.omega/memory.db --run-id 732 --mechanism $m
done
# control: GA_BIN=$TMPDIR/inc308-m4/doli-node-control-6461 bash scripts/gauntlet-gs058.sh
```

## Results (END binary)

| Row | Result | Key lines |
|-----|--------|-----------|
| GS-056 | PASS (harness `gauntlet_runs` 511) | 7 nodes; every RPC on `127.0.0.1:850x`, `nonloopback_probe=none` (no connection on 192.168.1.103). INFO: `loopback_probe_info=-32603` on all 7 — the fleet runs pre-fix binaries, so `getUtxoDiff` with no token still runs from loopback there (parent behaviour, measured). |
| GS-057 | PASS (512) | `getChainInfo` via relay = ok; 14/14 `ADMIN_METHODS` via relay = `-32008`; node log `rejected=14 from_loopback=14`, sample `Admin method 'pauseProduction' from 127.0.0.1 rejected: invalid or missing bearer token` |
| GS-058 | PASS (513) | 14/14 `no_token=-32008`, 14/14 `wrong_token=-32008`, `guardian_unchanged=yes`, public `getChainInfo`/`getGuardianStatus` = ok, token file mode 600 len 64, not on argv; valid token: pause/resume, enter/exitRecoveryMode, createCheckpoint, getStateSnapshot, getStateRootDebug = ok; pruneBlocks/bridgeFromArchive/getUtxoDiff = -32603, backfillFromPeer/verifyChainIntegrity/repairArchiveFromPeer/forceReorgTo = -32602 (handler answers; none is -32008) |
| GS-061 | PASS (514) | 7 processes `rpc_bind=127.0.0.1` (seed: default) `listen=127.0.0.1:850x token_src=none token_on_argv=no`; 9 live plists, no literal token; installers: `scripts/install-services.sh` `0.0.0.0×3` (INFO: END node makes its own token file), `install.ps1`, `testnetlinux/.../install-services.sh`, `inc-i-300-repair-restart.sh` `127.0.0.1`; no literal token in any file |

Control (6.46.1, pre-INC-I-308) on GS-058: **VERDICT FAIL** — 0/14 refused with no token or a wrong
token (e.g. `pauseProduction no_token=ok`, `getUtxoDiff no_token=-32603`), `gs058-guardian-unchanged`
fails (the unauthenticated sweep paused the node and entered recovery mode), no `rpc-admin.token`.
The row discriminates parent vs END.

## Scratch processes

| Run | Scratch node PID | Relay PID | Stopped |
|-----|------------------|-----------|---------|
| GS-058 direct, END (first) | 6968 | — | gone=yes |
| GS-057 direct, END | 11649 | 11664 | gone=yes / gone=yes |
| GS-058 direct, END (after reorder edit) | 26893 | — | gone=yes |
| GS-058 control 6.46.1 | 27766 | — | gone=yes |
| GS-057, GS-058 via `gauntlet-run.sh` | not printed (harness keeps no output) | — | `pgrep -f 'doli-node.*inc308-m4'` empty after the runs |

## Fleet at the end (read-only)

PIDs unchanged from the start: 30089 (n1), 30430 (n2), 30541 (n3), 35431 (n4), 37039 (n5), 37085 (n6),
77975 (seed). `getChainInfo` on 8500-8506: all `bestHeight=498273`, hash `f98de0a1c227daf6…`.
No fleet node was stopped, restarted, reconfigured or written to.

## Not done

- GS-059 (multi-byte address crash) and GS-060 (hostile backfill peer) not registered.
- The control binary is 6.46.1 (`7a8c3eb8`), not the exact parent `ceacc42d`; no parent build was made.
