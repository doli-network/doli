# INC-I-308 M1 — evidence (run 732): admin gate + token provisioning

Parent: `4cf8be85`. Contract: `docs/bugfixes/inc-i-308-rpc-admin-access-analysis.md` (main tree).

## Outcome metric
- metric: `ADMIN_CALLS_ANSWERED_WITHOUT_TOKEN` — state-changing admin RPC calls the HTTP gate answers for a caller with no valid bearer (audit C1 + C2 callers)
- probe: `.claude/scripts/inc-i-308-m1-probe.sh <tag>` (main tree)
- before: 72 (C1 36 + C2 36) · after: 0

## RED — parent gate, stub `admin_token` (19 failed)
`cargo test -p rpc --lib -- server:: admin_token` → 14 passed, 16 failed:
```
FAIL admin_token::tests::{contract_constants_and_usable_token_boundary, config_token_used_when_env_absent_and_short_config_ignored,
  fresh_dir_creates_owner_only_64_hex_token_then_reuses_it, usable_env_token_wins_and_writes_no_file,
  env_token_with_surrounding_whitespace_is_used_trimmed, no_temp_file_is_left_after_creation, missing_data_dir_is_created,
  empty_or_short_env_token_is_not_used, existing_unusable_file_is_replaced_by_a_fresh_token, unwritable_location_is_an_error,
  existing_valid_file_is_reused_and_its_mode_tightened, two_fresh_dirs_get_different_tokens}
  | admin_token.rs:33:5: not yet implemented: INC-I-308 M1
FAIL server::auth_tests::r2_no_token_configured_refuses_every_admin_method_everywhere
  | auth_tests.rs:272:5: admin_token None: pauseProduction from 127.0.0.1 with no header: admitted
FAIL server::auth_tests::r2_private_client_behind_a_trusted_proxy_needs_the_token
  | auth_tests.rs:342:9: pauseProduction: proxied 10.0.0.5 with no bearer was admitted
FAIL server::auth_tests::r2_every_admin_method_needs_the_token_from_every_address
  | auth_tests.rs:252:5: R-2: 130 of 294 cells wrong
FAIL server::auth_tests::r2_empty_or_short_configured_token_never_admits
  | auth_tests.rs:296:5: config "": pauseProduction from 127.0.0.1 with no header: admitted
```
`cargo test -p rpc --test it audit_rpc_access -- --test-threads=1` → 4 passed, 3 failed, 1 ignored (C4):
```
FAIL audit_rpc_access_c1_admin_methods_answer_loopback_without_token | C1: 36 state-changing admin calls were answered
FAIL audit_rpc_access_c2_admin_methods_answer_private_network_without_token | C2: 36 state-changing admin calls were answered
FAIL audit_rpc_access_c5_forged_forwarding_header_passes_as_trusted_address | C5: 18 state-changing admin calls were answered
```
Passed on parent already: `r2_public_methods_need_no_token_from_any_address`, the 4 `force_reorg_*` tests, C3/C4/C5 controls, table guard.

## GREEN
- `cargo test -p rpc` → lib 113/0/0; it 32 passed, 0 failed, 1 ignored (C4); other targets 8 + 1 + 1 + 0 pass.
- `cargo test -p doli-node --lib` → 211 passed, 0 failed.
- `cargo clippy -p rpc -p doli-node --all-targets -- -D warnings` → clean. `cargo fmt --check -p rpc -p doli-node` → clean.

## Changed pre-existing tests
| Test / constant | Change | Why |
|---|---|---|
| `auth_tests::every_other_admin_method_keeps_todays_rules` | replaced by `r2_every_admin_method_needs_the_token_from_every_address` | it pinned the loopback / RFC 1918 bypass this fix removes |
| `auth_tests::TOKEN` | 21 chars → 64 lowercase hex | the gate now treats a configured token < 32 chars as not configured; `force_reorg_*` assertions unchanged |
| `audit_rpc_access_fixture::TOKEN` | 21 chars → 64 lowercase hex | same reason; keeps the C3/C5 controls meaningful (one-line edit outside the M1 list) |
| audit C1, C2, C5 | `#[ignore]` removed | red-first reproductions of P1-001/-002/-003/-006, P2-001 |
| audit C4 | stays ignored, reason reworded | AUDIT-P3-RPC-001 is out of scope |

No other test in `crates/rpc` or `bins/node` reaches the HTTP gate: `inc_i_204_m41_force_reorg_rpc.rs` calls `RpcContext::handle_request`, which does not pass through `check_admin_auth`.

## Token file contract as built (`crates/rpc/src/admin_token.rs`)
Env `DOLI_RPC_ADMIN_TOKEN` if ≥ 32 chars after trim → else config `rpc.admin_token` if ≥ 32 → else `<data_dir>/rpc-admin.token`:
reused when its trimmed content is ≥ 32 chars (mode tightened to 0600 if group/other bits set); otherwise 32 OsRng bytes → 64 lowercase
hex + `\n`, written to a 0600 `create_new` temp file in `data_dir`, fsynced, renamed over the target. Short/empty env or config → `error!`
log, ignored. `startup.rs` passes `&self.config.data_dir`, logs the source (and the path for file sources) at info, never the token;
on write error it logs `error!` and runs with `admin_token = None` (every admin call refused).

## Does block / tx validation call this file?
| File | Called by block or tx validation? |
|---|---|
| `crates/rpc/src/server.rs` | no — HTTP handler only |
| `crates/rpc/src/admin_token.rs` | no — called once from `start_rpc` |
| `crates/rpc/src/lib.rs` | no — module line |
| `crates/rpc/Cargo.toml`, `Cargo.lock` | no — adds `rand` 0.8 (already in the workspace) to `rpc` |
| `bins/node/src/node/startup.rs` (`start_rpc`) | no — RPC startup only |
| tests, `docs/rpc_reference.md`, `specs/security_model.md` | no |

Consensus-shape answers: (1) no (2) no (3) n/a — RPC access only. No activation height, no version bump.

## Open
- `bins/node/src/node/startup.rs` was 679 lines before this change (697 now), above the 500-line budget; a split is outside M1.
