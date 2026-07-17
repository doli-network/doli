# QA Report: disk-guardian M2 — Bound log growth (REQ-DISK-201..204)

## Scope Validated
Installer + generated text + docs ONLY (per milestone scope):
- `bins/cli/src/cmd_service.rs` — `logrotate_dropin_path`, `logrotate_dropin_content`, install write, uninstall remove.
- `bins/cli/tests/logrotate_dropin_test.rs` — source-wiring assertions.
- `docs/troubleshooting.md` §1.7 Disk full / ENOSPC.
- `docs/producer_node_quickstart.md` adoption note.

## Summary
**PASS.** All four M2 acceptance criteria (REQ-DISK-201 Must, REQ-DISK-202 Must, REQ-DISK-203 Should, REQ-DISK-204 Should) are met. The generated logrotate drop-in is byte-exact to architecture §D2, install uses an unconditional overwrite (idempotent), uninstall removes the drop-in behind an `exists()` guard, `cmd_logs` is untouched (pure-addition diff), and the troubleshooting snippet matches the generated content byte-for-byte. Full `doli-cli` suite passes; no consensus/node code, version bumps, or activation heights touched.

## System Entrypoint
Static + unit validation for a non-consensus installer/docs change: `cargo test -p doli-cli` (read-only). The real install writes to `/etc/logrotate.d` and require root, so wiring is asserted via `include_str!` source search — matching the repo convention. No node runtime required.

## Traceability Matrix Status
| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-DISK-201 | Must | Yes | Yes | Yes | Byte-exact unit test + install-wiring test |
| REQ-DISK-202 | Must | Docs/ops | N/A (docs) | Yes | ~1.2G ceiling documented (troubleshooting + arch §D2) |
| REQ-DISK-203 | Should | Yes | Yes | Yes | Uninstall-wiring test asserts `logrotate_dropin_path` + `remove_file` |
| REQ-DISK-204 | Should | N/A (docs) | N/A | Yes | Troubleshooting §1.7 + quickstart note present |

No gaps. `logrotate -d` dry-run acceptance (REQ-DISK-202 second bullet) is an ops/gauntlet step, not code-testable here — deferred to ops validation per arch §D2 step 3.

## Acceptance Criteria Results

### Must
#### REQ-DISK-201: Installer writes size-capped logrotate drop-in
- [x] Content byte-exact to arch §D2: `/var/log/doli/{network}.log { maxsize 200M / daily / rotate 5 / copytruncate / compress / delaycompress / missingok / notifempty }` with trailing newline — PASS (`req_disk_201_dropin_content_mainnet_is_byte_exact`)
- [x] `copytruncate` present + reasoning honored (systemd holds append fd; rename rotation would be bypassed) — PASS (documented in code comment lines 233-238 and enforced by directive-presence test)
- [x] Path `/etc/logrotate.d/doli-{network}` — PASS (`logrotate_dropin_path`, `req_disk_201_dropin_content_and_path_are_network_scoped`)
- [x] Re-install idempotent (overwrites) — PASS: `install_systemd` uses `std::fs::write(&dropin_path, ...)` (unconditional truncating write, NOT create-new)
- [x] Unit test asserts generated content byte-exactly — PASS

#### REQ-DISK-202: Bounded ceiling documented
- [x] `(rotate+1)×maxsize ≈ 1.2 GB` + inter-rotation burst-day residual stated in `docs/troubleshooting.md` (lines 283-285) and arch §D2 — PASS

### Should
#### REQ-DISK-203: Uninstall removes drop-in
- [x] `cmd_uninstall` linux branch removes drop-in via `remove_file`, absent-file tolerated by `Path::new(&dropin).exists()` guard (lines 570-575) — PASS

#### REQ-DISK-204: Adoption docs
- [x] Troubleshooting §1.7 has disk-full/ENOSPC section: reclaim steps (`df`, `du`, `truncate -s 0`, `rm *.gz`), 1.2G cap, copy-paste snippet — PASS
- [x] Snippet (troubleshooting.md lines 289-298) is byte-identical to `logrotate_dropin_content("mainnet")` — PASS (manual diff)
- [x] `docs/producer_node_quickstart.md` adoption note present (lines 180-185) — PASS

## End-to-End Flow Results
| Flow | Steps | Result | Notes |
|---|---|---|---|
| install → drop-in written | fs::write to `/etc/logrotate.d/doli-{network}` | PASS (wiring) | Byte-exact content; overwrite semantics |
| re-install → overwrite | second fs::write | PASS | Unconditional write = idempotent |
| uninstall → drop-in removed | exists()-guarded remove_file | PASS (wiring) | Absent-file tolerated |
| `doli service logs` reads path | reads `/var/log/doli/{network}.log` | PASS | Unchanged; copytruncate keeps path/inode-name stable |

## Exploratory Testing Findings
None material. Network substitution verified for both `mainnet` and `testnet` partitions (first-line path + drop-in path both scoped correctly).

## Path Stability / cmd_logs
`git diff bins/cli/src/cmd_service.rs` = **122 insertions, 0 deletions** → `cmd_logs` was NOT altered. It still reads `/var/log/doli/{network}.log` (line 765). `copytruncate` truncates in place, preserving the path and inode name, so the reader is unaffected. CONFIRMED.

## Constraint Check
- No version bump (no `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION`) — CONFIRMED.
- No activation height added — CONFIRMED (installer/docs only, non-consensus).
- No node/consensus code touched — CONFIRMED.

## Regression
`cargo test -p doli-cli` → all pass (0 failed). Lib: 192 passed; new `logrotate_dropin_test.rs`: 3 passed; new in-module `cmd_service::tests`: 3 byte-exact/directive tests passed; all other integration suites green.

`git diff --name-only` (milestone-relevant): `bins/cli/src/cmd_service.rs`, `docs/troubleshooting.md`, `docs/producer_node_quickstart.md`; plus untracked new `bins/cli/tests/logrotate_dropin_test.rs`. All other listed changes (`.claude/skills/*`, `CLAUDE.md`, `crates/network/src/sync/manager/tests_inc_i139.rs`, `docs/.workflow/milestone-progress.md`) are pre-existing unrelated working-tree drift, NOT part of this milestone.

## Specs/Docs Drift
None. Generated content, spec REQ-DISK-201, arch §D2 block, and the troubleshooting copy-paste snippet are mutually byte-consistent.

## Blocking Issues
None.

## Non-Blocking Observations
- **OBS-001**: `logrotate -d` dry-run acceptance (REQ-DISK-202 bullet 2) and forced-rotation-truncates-in-place are ops/gauntlet steps not exercisable in this environment. Recommend an ops pass on the local testnet before fleet adoption (arch §D2 step 3).

## Final Verdict
**PASS** — All Must (REQ-DISK-201, 202) and Should (REQ-DISK-203, 204) acceptance criteria met. No blocking issues. Approved for review.
