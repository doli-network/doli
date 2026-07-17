# Code Review — disk-guardian M2 (installer logrotate drop-in)

Run: #458 · workflow_type: new-feature · Reviewer verdict: **APPROVED (0 blocking, 1 P3)**

## Scope Reviewed
- `bins/cli/src/cmd_service.rs` — `logrotate_dropin_path`/`logrotate_dropin_content`, install write, uninstall removal, inline `#[cfg(test)] mod tests`
- `bins/cli/tests/logrotate_dropin_test.rs` — source-wiring assertions
- `docs/troubleshooting.md` §1.7 (Disk full / ENOSPC)
- `docs/producer_node_quickstart.md` — log-rotation adoption note

## Completeness (REQ-DISK-201..204 + architecture §D2) — PASS
- **REQ-DISK-201**: `install_systemd` writes `/etc/logrotate.d/doli-{network}` with `maxsize 200M`, `daily`, `rotate 5`, `copytruncate`, `compress`, `delaycompress`, `missingok`, `notifempty` — byte-exact to §D2. Idempotent via `fs::write` overwrite. Byte-exact inline unit test present.
- **REQ-DISK-202**: ceiling `≈(rotate+1)×maxsize ≈ 1.2 GB` documented in troubleshooting.md.
- **REQ-DISK-203**: `cmd_uninstall` Linux branch removes the drop-in, `.exists()`-guarded (absent-file tolerated).
- **REQ-DISK-204**: troubleshooting §1.7 + copy-paste snippet + quickstart note present and correct.
- `cmd_logs` reader unaffected — `copytruncate` keeps the append path stable.

## Correctness — PASS
- `copytruncate` present and load-bearing (systemd holds the append fd; rename rotation would strand the writer on the rotated inode). Confirmed in generated content, inline comment, token test, and docs rationale.
- `fs::write`/`remove_file` use `?` — acceptable for a root-run installer CLI.
- Helpers referenced unconditionally by `install_systemd`/`cmd_uninstall` → no `dead_code` under `-D warnings`.

## Docs Accuracy — PASS
troubleshooting.md snippet is byte-identical to `logrotate_dropin_content("mainnet")` and the inline `EXPECTED_MAINNET` const. Quickstart note correct, cross-links §1.7.

## Scope / No Unintended Changes — PASS
Only the 4 M2 files carry in-scope changes. No node/consensus code, no version bump (`CURRENT_PROTOCOL_VERSION`/`EPOCH_STATE_FORMAT_VERSION`/`MIN_PEER_PROTOCOL_VERSION` untouched), no activation height. Committer stages ONLY the 4 M2 files; pre-existing working-tree drift (skill files, CLAUDE.md, INC-I-139 fmt, `docs/.workflow/*` scratch, `scripts/__pycache__/`) is excluded.

## Improvement Suggestions (non-blocking)
- **P3**: Add a drift-gate unit test asserting the troubleshooting.md snippet equals `logrotate_dropin_content("mainnet")` byte-for-byte (precedent: oracle §6 `m11_centralization_disclosure_byte_equal_to_spec`). Today docs/code parity is verified manually and can silently drift.

## Security Audit Verdict
Only variable is `network`, an operator-supplied CLI arg already validated upstream (`Network::from_str` → `mainnet|testnet|devnet`) and already used throughout install for `/var/log/doli/{network}.log` and unit paths. M2 introduces no new trust boundary: a static, non-parameterized-by-untrusted-input text file written to a root-owned path by an already-root `sudo doli service install`. No path-injection escalation beyond privileges the operator already holds.

**Security Audit Verdict: AUDIT-SKIP**
