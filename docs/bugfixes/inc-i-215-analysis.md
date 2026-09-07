# INC-I-215 — Analysis: auto-update is dead on every `doli service install` node

Analyst pass for `/omega-doctor` RUN_ID=549. Root cause is **already confirmed with live
evidence** (incident_entries #2731) and the fix design is **user-approved** (#2732). This
document does not re-diagnose. It establishes the code facts the test-writer and developer
will build on, the architecture context, requirements with acceptance criteria, and the
milestone split.

Branch: `bugfix/inc-i-215-staged-upgrade` (worktree, from `origin/main` @ `017b8d35`).

---

## 1. Bottom line (SSF)

**The node stages a verified release into its own writable data directory and a root
systemd `.path` unit swaps the binary — because the one thing the sandbox forbids is the
node writing its own executable, and nothing else about the update pipeline is broken.**

This works because every security decision (poll → 3-of-5 maintainer signature verify →
veto → approve → checksum bind) already happens inside the node and stays exactly where it
is; only the final `write(2)` moves to a process that is allowed to perform it.

### ⚠ One refinement to the approved design — established from code, not opinion

The approved design says the node stages "the verified binary + the signed manifest".
**It must stage the TARBALL, not the extracted binary.** The maintainer signature chain is:

```
SIGNATURES.json  →  sha256(CHECKSUMS.txt)  →  per-platform TARBALL hash  →  the .tar.gz
```

(`crates/updater/src/install_gate.rs:16-21`, L4 at `:135-138`.) **No signed value covers the
extracted `doli-node` ELF.** If the node stages the extracted binary, the CLI's "re-hash the
staged binary against the manifest checksum" step has no signed operand to compare against —
it would have to trust the node's own extraction, which is precisely the trust the staging
handoff exists to avoid, and INV-REL-001 would hold only up to the node and not through to
the install. Staging the tarball keeps the chain unbroken end to end and lets
`--from-staged` run the identical L1–L4 gate offline.

Everything else in the approved design is unchanged.

---

## 2. Architecture Context

### 2.1 Module boundaries

| Module | Responsibility | Depends on | Depended on by |
|---|---|---|---|
| `crates/updater` | Release fetch, signature/trust-root gate, download, checksum bind, install, restart | `crates/core` (Network), `crates/crypto`, `crates/storage` (indirect via callers) | `bins/node`, `bins/cli` |
| `crates/updater/src/install_gate.rs` | The **only** artifact-binding gate (L1–L4). Deliberately single-implementation so the two operator paths cannot drift (`:12-14`) | `download.rs`, `trust_root.rs`, `verification.rs` | `bins/cli/src/cmd_upgrade.rs`, `bins/node/src/commands/misc.rs`, `bins/cli/src/cmd_release_verify.rs` |
| `crates/updater/src/apply.rs` | Install mechanics: `current_binary_path`, backup, `install_binary`, sudo fallback, `auto_apply_from_github`, `restart_node` | `download.rs`, `verification.rs` | `bins/node/src/updater/service.rs`, `bins/node/src/commands/misc.rs`, `bins/cli/src/cmd_upgrade.rs` |
| `bins/node/src/updater/` | In-process poll/veto/approve state machine + `pending_update.json` persistence | `crates/updater` | node event loop, RPC |
| `bins/cli/src/cmd_upgrade.rs` | Operator install path (root), same gate | `crates/updater`, `crates/storage` | `main.rs` |
| `bins/cli/src/cmd_service.rs` | systemd/launchd unit rendering + install/uninstall | none (shells out to `systemctl`) | `main.rs` |

Dependency direction is strictly `bins/* → crates/updater`. The two binaries never call each
other; they meet only on the host filesystem. **The staging handoff is a new meeting point
and it is the first one that is a security contract**, which is why it must reuse
`install_gate` rather than grow a second verifier.

### 2.2 Data flow of the update path today

```
check_for_updates (service_checks.rs:76)
  → fetch_latest_release
  → is_newer_version?                                  (:98)
  → already pending for this version? → return          (:104-112)   ← the idempotency latch
  → verify_release_with_trust_root                      (:118)
  → PendingUpdate{first_notified_at=now}.save(data_dir) (:142-151)   → pending_update.json

check_veto_status (service.rs:178)  [every 60 s]
  → veto deadline reached & producer count known
  → Approved  → p.approved=true, enforcement=Some{..}, save → auto_apply   (:253-301)
  → Rejected  → drop pending, remove pending_update.json                   (:303-317)
  → ActivateEnforcement → enforcement.active=true, save → auto_apply       (:318-357)

auto_apply (service.rs:372)                                    ← THE BRANCH POINT
  → clone pending release                               (:382-389)
  → verify_release_with_trust_root  (INC-I-172 F7(a))   (:396-413)  ← MUST STAY IN THIS BODY
  → auto_apply_from_github(version, signed_checksums_sha256)      (:416)
       ├ fetch_github_release                     apply.rs:443
       ├ CHECKSUMS.txt integrity vs signed hash   apply.rs:451-465   (TOCTOU close)
       ├ download_from_url(tarball)               apply.rs:469
       ├ verify_hash(tarball, per-platform hash)  apply.rs:474
       ├ extract_binary_from_tarball              apply.rs:478
       ├ backup_current                           apply.rs:481
       ├ install_binary(&binary, current_exe)     apply.rs:485      ← EROFS HERE
       ├ install doli CLI beside it (best effort) apply.rs:490-515
       └ install skills (best effort)             apply.rs:518-522
  → Ok  → remove pending_update.json → restart_node() (exec, never returns)  (:417-428)
  → Err → log "Auto-apply failed … Will retry on next check cycle"
          AND CLEAR pending + delete pending_update.json                     (:429-446)
```

The last arm is why the observed host loops: clearing `pending` deletes the idempotency
latch at `service_checks.rs:107`, so the next check re-detects the same release, restarts a
fresh veto window, re-approves and re-downloads — forever, at whatever
`check_interval_secs` the unit sets (600 s on `doli-server-rafael`; the compiled default is
`CHECK_INTERVAL = 6 h`, `constants.rs:149`).

### 2.3 Architectural constraints and invariants (must survive the fix)

| Invariant | Where enforced | What breaks if violated |
|---|---|---|
| **INV-REL-001** — every installable artifact carries a threshold-satisfying manifest | `install_gate.rs` L1–L4 | A staged path that skips or re-implements any link is a fail-open installer wearing a fail-closed mask |
| **INV-REL-002** — verification uses a trust root whose freshness is visible | `TrustRoot::resolve` + provenance printing (`cmd_upgrade.rs:195-202`) | Operator cannot tell whether a stale local snapshot authorised the install (INC-I-206 risk) |
| **ONE gate implementation** | `install_gate.rs:12-14` (explicit design note) | The two operator paths drift; that drift *was* INC-I-172 F1/F2 |
| **`auto_apply` body must contain the trust-root re-verify** | asserted structurally by `bins/node/tests/inc_i_172_service_timing_test.rs:318-325` (`method_body("async fn auto_apply")`) | Moving the re-verify into a helper module **breaks a live regression test** and re-opens INC-I-172 F7(a) |
| **`install_binary`'s two branches install the same mode** | `apply.rs:186-194` vs `:342-402`, asserted by `crates/updater/tests/apply_install_mode.rs` | INC-I-153: `status=203/EXEC`, bricked producer |
| **Never delete a good binary on a failed install** | `apply.rs:320-336` (rm-then-cp is the exception, guarded by postcondition read-back) | A refused staged install that unlinks the target bricks the node |
| **Verification precedes install, structurally** | `bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs:72`, `bins/node/tests/inc_i_172_upgrade_cmd_verify_blocks_test.rs:100` (`INSTALL_LANDMARK = "install_binary"`) | A new install path with the verify *after* the write |
| **Trust root never degrades to compiled keys on error** | `TrustRoot::resolve` (`trust_root.rs:119-175`) | INC-I-175 leaked-key exposure returns |
| No consensus/block-content surface | — | n/a for this fix (see §4) |

### 2.4 Blast radius (graph + grep confirmation)

The orchestrator's graph query returned 3 dependents of `install_binary`. Grep confirms the
graph was a **lower bound** — it missed one production caller:

| Symbol | Callers (grep-confirmed) |
|---|---|
| `updater::install_binary` | `crates/updater/src/apply.rs:485` (node binary), `:502` (CLI binary); `bins/cli/src/cmd_upgrade.rs:235` (doli), `:254` (doli-node); **`bins/node/src/commands/misc.rs:268`** (`doli-node upgrade` — *missed by the graph*); re-exported `crates/updater/src/lib.rs:50`, `bins/node/src/updater/mod.rs:28` |
| `UpdateService::auto_apply` | `bins/node/src/updater/service.rs:299` (Approved), `:354` (ActivateEnforcement) — **only two**, both inside `check_veto_status` |
| `auto_apply_from_github` | `bins/node/src/updater/service.rs:416` (only production caller); `apply.rs:165` (from `apply_update`); `lib.rs:49` |
| `apply_update` | no production caller found outside `apply.rs`/tests — the manual-apply path |
| `install_systemd` (unit render) | `bins/cli/src/cmd_service.rs:279` only; asserted structurally by `bins/cli/tests/logrotate_dropin_test.rs` |
| `Commands::Upgrade` dispatch | `bins/cli/src/main.rs:215-232` only |
| `verify_release_manifest` | `install_gate.rs:128` (via `verify_release_artifact`), `bins/cli/src/cmd_release_verify.rs:25`, `bins/cli/src/cmd_upgrade.rs:376` |

**Direct impact:** `crates/updater` (new staging + a bytes-in-hand gate entry point),
`bins/node/src/updater` (one branch in `auto_apply` + startup preflight),
`bins/cli` (new `--from-staged` module, helper-unit rendering, 2 one-line wirings).

**Indirect impact:** every host installed by `doli service install` (behaviour change: auto-
update starts working); `bins/node/src/commands/misc.rs` and `bins/cli/src/cmd_upgrade.rs`
are **unchanged in behaviour** but share `install_gate`, so a refactor there must keep their
call sites compiling and their structural regression tests green.

**Not touched:** `crates/core`, `bins/node/src/node/**`, consensus, block content, activation
heights, RPC.

### 2.5 Implicit assumptions the current code makes (all now falsified on our own hosts)

1. *"The process that decides to update is the process that may perform the update."*
   `auto_apply_from_github` installs to `current_exe()` (`apply.rs:484`) with no capability
   check.
2. *"A permission failure is `EACCES`, so the sudo fallback covers it."* `install_binary`
   only falls back on `ErrorKind::PermissionDenied` (`apply.rs:200`). Under
   `ProtectSystem=full` the kernel returns **`EROFS` (os error 30)**, which takes the
   `Err(e) => Err(e.into())` arm at `:205` — no fallback is even attempted. And
   `NoNewPrivileges=true` would have killed `sudo` anyway.
3. *"A failed install is transient, so clearing state and retrying is progress."*
   (`service.rs:437-446`) With a structural cause the retry is an infinite silent loop.
4. *"The operator reads the node's log."* The only evidence of the dead updater is one
   `error!` line in `/var/log/doli/{network}.log`; `journalctl` shows nothing
   (`StandardOutput=append:`, `cmd_service.rs:368`). No metric, no RPC field, no exit code.
5. *"`doli service install` and the auto-updater are independent features."* The same CLI
   that writes `ProtectSystem=full` also ships the updater that the sandbox forbids. The two
   have **no contract** — assumption (5) is the actual root cause and the reason this is a
   class bug, not a host bug.

### 2.6 Brittleness Check (bugfix workflow)

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details:
  ✗ Cross-module blast radius — 3 modules, but bins/node and bins/cli BOTH depend
    directly on crates/updater; the change travels along existing edges, adds no new
    module-to-module dependency.
  ✓ Invariant gaps — "the install target must be writable by the installing process"
    is enforced by NOTHING. It is discovered at write(2) time and only logged.
  ✗ Data flow reversal — flow direction is unchanged (release → node → binary); the fix
    inserts a handoff, it does not reverse anything.
  ✗ Shared mutable state without an owner — the staging dir introduced BY the fix has a
    single writer (node) and a single consumer (root helper), made atomic by temp+rename.
    The BUG does not involve shared mutable state.
  ✓ Contract absence — `doli service install` (writes the sandbox) and the in-process
    updater (needs to escape it) are produced by the same codebase and have no explicit
    interface. This is the root cause restated.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

Both signals point at the *same* missing contract, and the approved design creates exactly
that contract. No architectural redesign is implied.

---

## 3. Code facts (a–j)

### (a) The auto-apply call chain

- Entry: `UpdateService::auto_apply(&self, version, maintainer_keys_fn)` —
  `bins/node/src/updater/service.rs:372`. Private method; only callers `:299` and `:354`.
- Clones `PendingUpdate.release` (`:382-389`), then re-verifies against the **current** root:
  `updater::verify_release_with_trust_root(&staged_release, &trust_root)` at `:397`.
  **This call must remain textually inside `auto_apply`'s body** —
  `bins/node/tests/inc_i_172_service_timing_test.rs:318` slices the method body and asserts it.
- Then `auto_apply_from_github(version, &signed_checksums_sha256)` at `:416`, where
  `signed_checksums_sha256 = staged_release.binary_sha256` (`:414`) — despite the field name,
  this is **sha256(CHECKSUMS.txt)**, not a binary hash (`apply.rs:157-164`).
- `pub async fn auto_apply_from_github(version: &str, signed_checksums_sha256: &str) -> Result<()>`
  — `crates/updater/src/apply.rs:439`. Internals: `fetch_github_release` `:443`;
  CHECKSUMS integrity `:451-465`; `download_from_url` `:469`;
  `verify_hash(&tarball, &release_info.expected_hash)` `:474`;
  `extract_binary_from_tarball` `:478`; `backup_current` `:481`;
  `current_binary_path()` `:484`; `install_binary(&binary, &target)` `:485`;
  optional CLI install `:490-515`; skills `:518-522`.
- Manifest type: `updater::SignaturesFile { version, checksums_sha256, signatures: Vec<MaintainerSignature{public_key, signature}> }` —
  `crates/updater/src/types.rs:63-72`. **serde JSON**, `Serialize + Deserialize`, written and
  read as `SIGNATURES.json` (`bins/cli/src/cmd_release_verify.rs:15-19`).
- `data_dir` in this path: `UpdateService.data_dir: PathBuf` — `service.rs:48`, set in
  `UpdateService::new` `:55-108`, reachable as `self.data_dir` inside `auto_apply`.
  **`crates/updater` has no access to it** — `auto_apply_from_github` is a free function
  with no data-dir parameter. The staging target must be passed down from `service.rs`.
- Log lines: `error!("Auto-apply failed for v{}: {}. Node continues with v{}. Will retry on next check cycle.", …)` —
  `service.rs:430-436`. Success line `:418`. `"Update service started (check interval: {}h)"` — `:132-135`.

### (b) How `doli release verify` verifies a manifest

- CLI entry `cmd_release_verify(version, dir, data_dir, network, trust_root)` —
  `bins/cli/src/cmd_upgrade.rs:307`; dispatched at `bins/cli/src/main.rs:236-246`.
- Offline (dir) path: `doli_cli::cmd_release_verify::verify_manifest_dir(&d, &version, &root)` —
  `bins/cli/src/cmd_release_verify.rs:10-31`. Reads `<dir>/SIGNATURES.json` (serde_json) and
  `<dir>/CHECKSUMS.txt` (raw bytes), delegates everything to
  `updater::verify_release_manifest`. **This is the exact function `--from-staged` reuses.**
- `pub fn verify_release_manifest(version: &str, checksums_body: &[u8], signatures: &SignaturesFile, root: &TrustRoot) -> Result<usize>` —
  `crates/updater/src/install_gate.rs:57`. L1 version bind `:64-75`; L2 `sha256(CHECKSUMS.txt)` bind `:78-94`;
  L3 distinct-signer threshold via `verify_release_with_trust_root` `:99-108`. Returns the
  **distinct signer count**.
- Threshold source: **not** the `REQUIRED_SIGNATURES = 3` constant (`constants.rs:40`). It is
  `TrustRoot::threshold()`, taken verbatim from `MaintainerState.set.threshold`
  (`trust_root.rs:146-152`), which is `MaintainerSet::calculate_threshold(n)` —
  `crates/core/src/maintainer/set.rs:106-116` → **5 members ⇒ 3**.
- Trust root entry point: `resolve_upgrade_trust_root(data_dir, network)` —
  `bins/cli/src/cmd_upgrade.rs:66-79`: `storage::MaintainerState::load(data_dir)` (a **local
  file**, `maintainer_state.bin`, no RPC) → `updater::TrustRoot::resolve(keys, threshold,
  last_derived_height, network)` (`trust_root.rs:119`). Load failure is **fatal** and never
  degrades to bootstrap; the advice text is `trust_root_load_advice` `:16-49`.
- ⚠ **INC-I-206 risk carried into `--from-staged`**: the local snapshot can be stale.
  `resolve` is fail-closed (empty set + height > 0 ⇒ unusable), and `cmd_release_verify`
  already prints `last_derived_height` for staleness (`:320-323, :339-347`). `--from-staged`
  **must print the same provenance + height line** so a stale root is visible, not silent.

### (c) `install_binary` / `install_binary_sudo`

- `pub async fn install_binary(binary: &[u8], target: &Path) -> Result<()>` — `apply.rs:180`.
  Writes `target.with_extension("new")` **in the target's directory** (`:181`), chmods it to
  `INSTALLED_BINARY_MODE = 0o755` (`:188-194`, const at `:227`), then `fs::rename` onto the
  target (`:196`). Fallback **only** on `ErrorKind::PermissionDenied` (`:200-204`); every
  other error (including `EROFS`) returns at `:205`.
- `async fn install_binary_sudo(binary, target)` — `apply.rs:258` (**private**). Stages at
  `/var/lib/doli/update.bin` (`STAGED_BINARY_PATH`, `:217`) with `O_NOFOLLOW` + mode 0o755
  (`:288-311`), then `sudo rm -f target` (`:320-322`) and `sudo cp staged target`
  (`:324-336`), then **reads the installed mode back** and fails unless `mode & 0o001 != 0`
  (`:349-402`, INC-I-153 postcondition). Cannot run under `NoNewPrivileges=true`.
- "Target" means **an explicit path parameter**. `current_exe()` is resolved by the *caller*:
  `current_binary_path()` (`apply.rs:16-24`, strips a trailing `" (deleted)"`) is called at
  `apply.rs:484` and `bins/node/src/commands/misc.rs:267`. `cmd_upgrade.rs` instead passes
  `std::env::current_exe()` for the CLI (`:233`) and `find_doli_node_path()` for the node
  (`:251`). **`--from-staged` therefore reuses `install_binary` unchanged with an explicit
  target — no new install mechanics, no INC-I-153 re-litigation.**
- Backup: `backup_current()` (`apply.rs:34-50`) copies `current_exe → *.backup`. It backs up
  **the running process's own binary**, so on the CLI path it would back up `doli`, not
  `doli-node`. `cmd_upgrade.rs` does **not** call it; it backs the CLI up ad hoc at
  `apply.rs:496-500` only for the node-driven path. `--from-staged` must therefore do its own
  explicit `target → target.backup` copy before installing (see REQ-215-006).

### (d) How `doli upgrade` restarts and resolves paths

- Restart: `restart_specific_service(&svc)` or `restart_doli_service(installed_node_path)` —
  `bins/cli/src/cmd_upgrade.rs:290-294` → `bins/cli/src/upgrade_restart.rs:76` / `:91`.
- Both run `systemd_restart_plan(unit)` (`bins/cli/src/upgrade_systemd_plan.rs:14-33`):
  best-effort `sudo systemctl reset-failed <unit>` then required `sudo systemctl restart <unit>`.
  **The `reset-failed` step is the INC-I-188 start-limit-lock escape** (`StartLimitBurst=5`,
  `cmd_service.rs:358-359`) and is covered by `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs`.
- Network → unit name: there is **no** network→unit mapping in `doli upgrade`. It either
  takes `--service <name>` verbatim, or discovers units by listing
  `systemctl list-unit-files --type=service` and filtering on `"doli"` + an `ExecStart` that
  contains the installed binary path (`upgrade_restart.rs:122-195`). The naming convention
  lives only in `cmd_service.rs:51-53`: `resolve_service_name(network, name) = name.unwrap_or("doli-{network}")`.
- Node binary path: `find_doli_node_path()` — `upgrade_restart.rs:23-73`
  (pgrep → `which doli-node` → `/mainnet/bin`, `/testnet/bin`, `/usr/local/bin`, `/opt/doli/target/release`).
- **Yes, `doli upgrade` installs BOTH**: `doli` to `std::env::current_exe()` (`cmd_upgrade.rs:231-244`)
  and `doli-node` to the resolved path (`:248-275`), plus skills (`:278-287`).

### (e) `doli service install`

- `cmd_install` → `install_systemd(network, name, data_dir, producer_key, p2p_port, rpc_port)` —
  `bins/cli/src/cmd_service.rs:268-285`, `:330`. Gated by `check_sudo()` (`:63-81`, EUID must be 0).
- Unit rendered as one `format!` raw string — `:353-384`. Sandbox lines:
  `NoNewPrivileges=true` `:370`, `ProtectSystem=full` `:371`,
  `ReadWritePaths={data_dir} /var/log/doli` `:372`.
- Writes: `std::fs::write(&unit_path, &unit)` `:406` where
  `unit_path = /etc/systemd/system/{service_name}.service` `:339`; logrotate drop-in `:410-412`.
  Then `systemctl daemon-reload` `:415`, `systemctl enable {service}` `:418`,
  `systemctl start {service}` `:421` — all via `run_cmd` (`:834`).
- Known at render time: `network` (param), `service_name = resolve_service_name(network, name)`
  (`:338`), `actual_data_dir = data_dir.unwrap_or("/var/lib/doli/{network}")` (`:341-342`),
  `doli_node_bin = which_doli_node()` (`:345`, `:166-193`), `(run_user, run_group) = detect_service_user()`
  (`:348`, `:195-220`). **The `doli` CLI path is NOT resolved today** — a `which_doli_cli()`
  sibling of `which_doli_node()` is needed, defaulting to `std::env::current_exe()`
  (this process *is* `doli`, running as root).
- **Existing template test to model:** `bins/cli/tests/logrotate_dropin_test.rs`. `doli-cli`
  is bin-only (`lib.rs` exposes just 3 pure modules), so integration tests assert the unit
  wiring **structurally over `include_str!("../src/cmd_service.rs")`** with an `fn_body()`
  slicer (`:25-39`). Pure content/path helpers (`logrotate_dropin_content` / `_path`,
  `:241-262`) are the testable seam. The helper-unit work must follow this exact pattern.

### (f) The `Upgrade` clap definition and dispatch

- `Commands::Upgrade { version, yes, doli_node_path, service, data_dir }` —
  `bins/cli/src/commands.rs:222-243` (file is **1397 lines**, already over budget).
- Dispatch: `bins/cli/src/main.rs:215-232` — parses `cli.network` strictly, then calls
  `cmd_upgrade::cmd_upgrade(version, yes, doli_node_path, service, data_dir, network)`
  (file is **591 lines**, already over budget).
- Adding `--from-staged <DIR>` needs **+4 lines** in `commands.rs` and **+4 lines** in
  `main.rs` (an early `if let Some(dir) = from_staged { … return }` routing to the new
  module). No logic in either file.
- ⚠ `bins/cli/tests/it/inc_i_214_release_sign_verify.rs:352` (`upgrade_must_not_accept_the_trust_root_override`)
  asserts the shape of `doli upgrade`'s flag surface — the new flag must not disturb it.

### (g) Startup preflight — where and how

- Anchor: `info!("Update service started (check interval: {}h)", …)` —
  `bins/node/src/updater/service.rs:132-135`, inside `UpdateService::run` after the
  `config.enabled` guard (`:127-130`).
- In scope there: `self.config`, `self.data_dir` (`:48`), `self.network`
  (`doli_core::network::Network`, `:44`). Install target via `updater::current_binary_path()`.
- **Writability under the sandbox is directory-scoped** (see (i)), so the probe is on
  `target.parent()`.
- **Helper detection from inside the sandbox:** `ProtectSystem=full` mounts `/usr`, `/boot`,
  `/efi` **and `/etc`** read-only — they remain **readable**. So
  `/etc/systemd/system/*.path` can be listed and read by the `doli` user.
  Recommended (name-independent, handles `--name` custom services and multi-node hosts):
  scan `/etc/systemd/system/` for any `*.path` whose contents mention **this node's**
  `{data_dir}/updates/ready`. Fallback if the scan fails: existence of
  `/etc/systemd/system/doli-{network}-upgrade.path`.

### (h) Existing test infrastructure

- Test maintainer keys: `crates/updater/src/test_keys.rs` — `TEST_MAINTAINER_KEYS` (5, lazy,
  `:42-50`), `test_maintainer_pubkeys()` `:53`, `sign_with_test_key(i, msg)` `:63`,
  `create_test_release_signatures(version, sha)` (signs with the first **3**) `:74`,
  `should_use_test_keys()` = env `DOLI_TEST_KEYS=1` `:91-95`.
- CLI integration tests: **one consolidated binary**, `bins/cli/tests/it/main.rs` (module
  aggregator, enforced by `.claude/hooks/test-binary-gate.sh`). New tests are **modules
  there**, not new `tests/*.rs` files.
- Harness: plain `std::process::Command::new(env!("CARGO_BIN_EXE_doli"))` — **no `assert_cmd`**.
  Model: `bins/cli/tests/it/inc_i_214_release_sign_verify.rs:53-60` (`fn doli(args) -> Output`,
  always `--network devnet`).
- **How a test supplies a trust root:** *not* via `DOLI_TEST_KEYS`. The tests write a real
  `maintainer_state.bin` into a `tempfile::tempdir()` and pass it with `--data-dir`:
  `write_on_chain_root(data_dir, &[kp…], height)` (`:110-121`) builds a
  `doli_core::maintainer::MaintainerSet` with `calculate_threshold(n)` and
  `storage::MaintainerState{ set, last_derived_height, .. }.save(data_dir)`. Manifests are
  built with `updater::sign_release_hash(&kp, version, &sha)` and written as
  `SIGNATURES.json` + `CHECKSUMS.txt` (`write_manifest_dir`, `:90-108`).
  **5 members ⇒ threshold 3, so a 2-signature manifest is genuinely sub-threshold** —
  exactly the INV-REL-001 refusal case, testable with zero new infrastructure.
- Structural (source-text) tests are an established convention for private fns:
  `bins/cli/tests/logrotate_dropin_test.rs`, `bins/node/tests/inc_i_172_service_timing_test.rs`,
  `crates/updater/tests/apply_install_mode.rs`, `bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs`.

### (i) How "writable target" must be probed

`install_binary` writes `target.with_extension("new")` **into the target's own directory**
and then renames onto the target (`apply.rs:181-196`). Therefore:

- **Directory writability is what decides the outcome**, not the mode of the target file.
  A `root:root 0755` `/usr/bin/doli-node` is irrelevant; `/usr/bin` being a read-only mount
  is what returns `EROFS`.
- The probe must be **`target.parent()` + create/remove a uniquely-named temp file**
  (e.g. `.doli-writable-probe-{pid}`), mirroring the real write, and must clean up on every
  path. A `metadata().permissions().readonly()` check is **wrong** — it reports the file's
  mode bits and says nothing about a read-only mount or the sandbox.
- The probe must treat **both** `EROFS` and `EACCES` (and any error) as "not writable" —
  the current code's fatal mistake was special-casing `PermissionDenied` alone
  (`apply.rs:200-205`).

### (j) Existing state machine and the idempotency hazard

- `PendingUpdate { release, vote_tracker, first_notified_at, approved, enforcement }` —
  `bins/node/src/updater/mod.rs:61`; `load` `:93`, `save` `:126`, `remove` `:133`, file
  `{data_dir}/pending_update.json` (`:94`). `first_notified_at` has **no `serde(default)`**
  by design (AUDIT-P2-013, `:68`).
- The **idempotency latch** is `service_checks.rs:104-112`: if a pending update exists for
  the same version, `check_for_updates` returns before fetching anything.
  **Clearing `pending` on failure (`service.rs:437-446`) destroys that latch — that is the
  10-minute loop.**
- Startup drain already exists: `UpdateService::new` deletes `pending_update.json` when
  `enforcement.version_meets_requirement(current_version())` (`service.rs:64-79`). So once
  the root helper installs the binary and restarts the service, the pending state cleans
  itself up. **No new drain mechanism is needed.**
- Residual re-stage window: after a **restart** with `approved=true, enforcement.active=false`,
  the `ActivateEnforcement` transition (`service.rs:318-357`) fires once and calls
  `auto_apply` again → a second download + re-stage of the same version.
  **Required behaviour:** `auto_apply` must check for a `ready` marker naming **this exact
  version** *before* any download and return early with one INFO line. That is the
  idempotency requirement (REQ-215-005) and it also makes the staging path cheap on repeat.
- On successful **staging** the code must **keep** `pending` (do not clear, do not
  `restart_node()`): keeping it re-arms the latch at `service_checks.rs:107` and lets version
  enforcement do its normal job if the operator's helper never runs.

---

## 4. Impact analysis and drift flags

### 4.1 Deploy classification

- **Consensus RULES changed?** No. Nothing in `crates/core`, `bins/node/src/node/**`,
  scheduler, bitfield, coinbase, or validation is touched. **No activation height.**
- **Block CONTENT changed?** No. **No synchronized deploy.** Rolling-safe.
- `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `HardForkSchedule`: untouched.
- Behaviour change on upgrade: a node that installs this build starts *staging* instead of
  failing. Hosts whose target **is** writable keep the current in-process install path
  byte-for-byte.

### 4.2 What breaks if this changes — and mitigation

| Area | Risk | Mitigation |
|---|---|---|
| `auto_apply` body (`service.rs:372-448`) | Moving the trust-root re-verify out of the body **fails** `inc_i_172_service_timing_test.rs:318` | Keep `verify_release_with_trust_root` textually in `auto_apply`; put only the *staging* call in a child module |
| `apply.rs` refactor (extracting fetch+verify) | `crates/updater/tests/apply_install_mode.rs` slices `async fn install_binary_sudo` by name; `inc_i_172_install_gate_binding.rs` looks for `install_binary` after a verify | Do not rename or move `install_binary` / `install_binary_sudo`; extract only the *fetch/verify prefix* of `auto_apply_from_github` |
| `install_gate.rs` signature change | `verify_release_artifact(&GithubReleaseInfo, …)` has 2 production callers (`cmd_upgrade.rs:209`, `bins/node/src/commands/misc.rs`) | **Add** `verify_release_artifact_bytes(version, checksums_body, tarball, sf, root)` and make the existing fn a thin delegate — callers unchanged |
| `install_systemd` (`cmd_service.rs:330`) | `logrotate_dropin_test.rs` slices this fn body | Additive only; new helpers referenced from the same body |
| Root helper unit | A `.path` unit that triggers on a *partially written* file would install a truncated tarball | Node writes `tarball.tmp`/`ready.tmp` and **renames**; the unit watches only `ready` |
| Root helper unit | A compromised node could drop a crafted `ready` + tarball | `--from-staged` runs the **full L1–L4 gate against the on-chain trust root** before installing. This is the whole point of not chowning the binary |
| Staging dir on disk | Unbounded growth if the helper never runs | Single version-named dir, overwritten; `--from-staged` removes it after a successful install |
| `doli service install` on existing hosts | Re-running `install` restarts the node (`cmd_service.rs:421`) | Documented; root `doli upgrade` also refreshes the units without a service reinstall |
| INC-I-206 | A stale local `maintainer_state.bin` authorises against an old key set | `--from-staged` prints provenance + `last_derived_height`, same as `release verify` |

### 4.3 Regression-risk areas

`crates/updater/tests/apply_install_mode.rs` (INC-I-153 mode postcondition) ·
`crates/updater/tests/inc_i_172_apply_update_gate.rs` ·
`crates/updater/tests/inc_i_172_install_gate_binding.rs` ·
`bins/node/tests/inc_i_172_service_timing_test.rs` ·
`bins/node/tests/inc_i_172_upgrade_cmd_verify_blocks_test.rs` ·
`bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs` ·
`bins/cli/tests/logrotate_dropin_test.rs` ·
`bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs` ·
`bins/cli/tests/it/inc_i_214_release_sign_verify.rs`.
**Run all of these before every milestone commit.**

### 4.4 Specs / docs drift to fix (milestone M5)

| File | What must change |
|---|---|
| `docs/cli.md` | New `doli upgrade --from-staged <DIR>`; note that `doli service install` now also installs `doli-{network}-upgrade.{path,service}` |
| `docs/troubleshooting.md` | New entry: *"Auto-apply failed … Read-only file system (os error 30)"* → what it means, the new WARN line, and the fix (`sudo doli service install`, or `sudo doli upgrade --from-staged`) |
| `docs/releases.md` | Fleet update flow now has a staged variant; how a release reaches a sandboxed node |
| `docs/producer_node_quickstart.md` | Auto-update now works out of the box on `doli service install` hosts; helper units are part of the install |
| `docs/architecture.md` | Update-path section: the node/root privilege split and the staging contract |
| `.claude/skills/auto-update/SKILL.md` | Staged path, marker semantics, idempotency, the new WARN |
| `.claude/skills/updater/SKILL.md` | New module map (`staging.rs`, `--from-staged`), the "staged artifact is the TARBALL" rule |
| `.claude/skills/release/SKILL.md` | How a signed release reaches a sandboxed node |
| `.claude/skills/SKILLS-INDEX.md` | New keywords: `from-staged`, `doli-upgrade.path`, EROFS |
| `specs/security_model.md` | Staging dir as a trust boundary: node = untrusted writer, root CLI = verifying consumer; the 3-of-5 threshold holds end to end |
| `specs/SPECS.md` | Index entry if a new spec section is added |
| `CLAUDE.md` (**If You Touch**) | One line: touching the updater install path → check target writability + the staged handoff |

**No drift found between code and specs during this pass** — the gap is that the staged path
does not exist yet anywhere.

---

## 5. Requirements

Trust-boundary note: the staging directory is a **new trust boundary** (a `doli`-writable
path consumed by a root process). Per Global Rule 17 the security requirements below are
**Must** and non-negotiable.

| ID | Requirement | Priority |
|---|---|---|
| REQ-215-001 | Probe install-target writability before attempting an in-process install | Must |
| REQ-215-002 | Stage the verified **tarball** + `CHECKSUMS.txt` + `SIGNATURES.json` + atomic `ready` marker into `{data_dir}/updates/` when the target is not writable | Must |
| REQ-215-003 | Preserve today's behaviour exactly when the target **is** writable | Must |
| REQ-215-004 | Do not clear `pending`/`pending_update.json` after a successful staging | Must |
| REQ-215-005 | Idempotency: a `ready` marker for the same version short-circuits before any download | Must |
| REQ-215-006 | `doli upgrade --from-staged <DIR>`: re-verify L1–L4 against the on-chain trust root, back up, install, restart, clean up | Must |
| REQ-215-007 | `--from-staged` refuses a tampered tarball (hash mismatch) and installs nothing | Must |
| REQ-215-008 | `--from-staged` refuses a sub-threshold manifest and installs nothing | Must |
| REQ-215-009 | `--from-staged` refuses a missing/incomplete staging dir and installs nothing | Must |
| REQ-215-010 | `--from-staged` refuses a version that is not newer than the installed one | Should |
| REQ-215-011 | `doli service install` installs and enables the root `.path` + `.service` helper units | Must |
| REQ-215-012 | Root `doli upgrade` refreshes the helper units on existing hosts | Should |
| REQ-215-013 | `doli service uninstall` removes the helper units | Should |
| REQ-215-014 | Startup preflight: one WARN when the target is not writable and no helper is present | Must |
| REQ-215-015 | Exactly one INFO line per staging event, naming version, path and next step | Should |
| REQ-215-016 | Module budget: all new code in new modules; no file crosses 500 (800 test) lines | Must |
| REQ-215-017 | One verification implementation — no second copy of the L1–L4 chain | Must |
| REQ-215-018 | Docs/specs/skills aligned before close | Must |
| REQ-215-019 | Consensus/block content untouched; rolling-safe | Must |
| REQ-215-020 | Chown/relax-sandbox/root-timer alternatives stay rejected | Won't |

### Acceptance criteria (detailed)

**REQ-215-001 (Must)** — writability probe
- [ ] Given a target directory on a read-only mount, when probed, then the result is *not
      writable* — the probe must not rely on `PermissionDenied` alone (`EROFS` must also count).
- [ ] Given a writable target directory, when probed, then *writable*, and any probe artifact
      is removed (directory listing unchanged before/after).
- [ ] The probe targets `target.parent()`, matching `install_binary`'s temp+rename
      (`apply.rs:181-196`); a probe of the target file's mode bits alone is a failing implementation.
- [ ] The probe never panics and never propagates an error; an unknown outcome is treated as
      *not writable* (fail toward staging, which is safe).

**REQ-215-002 (Must)** — staging
- [ ] Given an approved release and a non-writable target, when auto-apply runs, then
      `{data_dir}/updates/` contains `SIGNATURES.json`, `CHECKSUMS.txt`, the platform
      `.tar.gz`, and `ready`.
- [ ] `ready` names the version and is created by **temp + rename** (never `create` + later
      `write`), so a `.path` unit can never observe a partially written marker.
- [ ] The tarball and both manifest files are fully written and fsynced **before** `ready` is
      renamed into place.
- [ ] The staged tarball is byte-identical to the bytes that passed
      `verify_hash(&tarball, per-platform-hash)`; the staged `CHECKSUMS.txt` hashes to
      `SIGNATURES.json.checksums_sha256`.
- [ ] The **extracted binary is not** what is staged (a staged bare ELF is a failing implementation).
- [ ] Everything is written inside `{data_dir}`, i.e. inside `ReadWritePaths`; nothing is
      written to `/tmp`, `/var/lib/doli/update.bin`, or any path outside the sandbox.
- [ ] `restart_node()` is **not** called on the staging path.

**REQ-215-003 (Must)** — no regression on writable hosts
- [ ] Given a writable target, when auto-apply runs, then the install → `pending_update.json`
      removal → `restart_node()` sequence is unchanged and nothing is written to
      `{data_dir}/updates/`.
- [ ] `verify_release_with_trust_root` still appears textually inside `auto_apply`'s body
      (`inc_i_172_service_timing_test.rs:318` stays green).
- [ ] `install_binary`'s two branches and their shared `0o755` mode are unchanged
      (`apply_install_mode.rs` stays green).

**REQ-215-004 (Must)** — pending retained
- [ ] Given a successful staging, then `pending_update.json` still exists and still names
      that version.
- [ ] Consequently the next `check_for_updates` returns at the "already pending" latch
      (`service_checks.rs:107`) without any network fetch.

**REQ-215-005 (Must)** — idempotency
- [ ] Given `{data_dir}/updates/ready` for version X, when auto-apply for X runs again, then
      **no** GitHub fetch and **no** download occur and exactly one INFO line is emitted.
- [ ] Given a `ready` marker for version X and an approved release Y > X, then the stale
      staging dir is replaced by Y's (a node must never be stuck on an older staged version).
- [ ] The check happens **before** `fetch_github_release`, not after.

**REQ-215-006 (Must)** — `doli upgrade --from-staged <DIR>`
- [ ] Reads `<DIR>/SIGNATURES.json` + `<DIR>/CHECKSUMS.txt` and calls the **same**
      `updater::verify_release_manifest` that `doli release verify --dir` calls (L1–L3).
- [ ] Verifies the staged tarball against the per-platform hash parsed from the just-verified
      `CHECKSUMS.txt` (L4) — not against any value carried in the staging dir's own metadata.
- [ ] Resolves the trust root exactly as `doli upgrade` does
      (`resolve_upgrade_trust_root` → `MaintainerState::load` → `TrustRoot::resolve`), and
      **prints provenance, key count, threshold and `last_derived_height`** (INC-I-206 visibility).
- [ ] Copies `target → target.backup` before installing, then installs via
      `updater::install_binary` (no new install mechanics).
- [ ] Installs `doli-node`, and `doli` when present, using the same target resolution as
      `doli upgrade` (`find_doli_node_path`, `--doli-node-path`).
- [ ] Restarts through `systemd_restart_plan` so `reset-failed` still precedes `restart`
      (INC-I-188).
- [ ] Removes the staging directory **only after** a successful install + restart.
- [ ] Gates on **target-directory writability** (`updater::target_dir_is_writable`), NOT on
      EUID/root: EROFS and EACCES are the same refusal and a root check would pass on a
      read-only mount. The refusal names the target and says
      `Try: sudo doli upgrade --from-staged <DIR> [--yes]`.
      *(CORRECTED 2026-09-07 against `bins/cli/src/cmd_upgrade_staged.rs:154-163`; the
      original "Requires root (EUID 0)" wording never matched the shipped code.)*

**REQ-215-007 (Must)** — tampered artifact refused
- [ ] Given a staging dir whose tarball has one flipped byte, when `--from-staged` runs, then
      it exits non-zero, the message names a hash mismatch, **the existing binary is
      untouched** (same mtime and bytes), no `.backup` overwrite has destroyed it, and the
      staging dir is **not** deleted (evidence is preserved).

**REQ-215-008 (Must)** — sub-threshold manifest refused
- [ ] Given a 5-member on-chain root (threshold 3) and a manifest signed by 2 of them, when
      `--from-staged` runs, then it exits non-zero naming insufficient signatures and installs
      nothing.
- [ ] Given a manifest whose `version` differs from the release being installed (L1 replay),
      then it refuses.
- [ ] Given a `CHECKSUMS.txt` that does not hash to `SIGNATURES.json.checksums_sha256` (L2),
      then it refuses.

**REQ-215-009 (Must)** — incomplete staging refused
- [ ] Missing `ready`, missing `SIGNATURES.json`, missing `CHECKSUMS.txt`, missing tarball, or
      a non-existent `<DIR>` each produce a non-zero exit naming the missing file, and
      install nothing.
- [ ] A `ready` marker whose version does not match the staged manifest's version is refused.

**REQ-215-010 (Should)** — downgrade refused
- [ ] Given a staged version that is not newer than `updater::current_version()`, then
      `--from-staged` refuses by default and says so. (Rationale: a stale staging dir left by
      a crashed helper must not silently reinstall an old build.)

**REQ-215-011 (Must)** — helper units installed
- [ ] After `sudo doli service install --network N`, `/etc/systemd/system/` contains a
      `.path` unit with `PathExists={data_dir}/updates/ready` and `Unit=` naming the helper
      `.service`, and a `.service` unit with `Type=oneshot`, no `User=` (root), and
      `ExecStart=<doli cli path> --network {network} upgrade --from-staged {data_dir}/updates
      --data-dir {data_dir} --service {service_name} --yes`.
      *(CORRECTED 2026-09-07 against `bins/cli/src/cmd_service_helper_units.rs:65-87`:
      `--network` is a GLOBAL clap flag on `Cli` and must PRECEDE the subcommand. The
      original flag order does not parse.)*
- [ ] Unit names are derived from the resolved service name so a `--name`-customised or
      multi-node host does not collide (e.g. `{service_name}-upgrade.{path,service}`).
- [ ] The `.path` unit is enabled (`systemctl enable`) and started; the `.service` is **not**
      enabled (it is triggered, not booted).
- [ ] Content and path come from **pure helper functions** in a new module, mirroring
      `logrotate_dropin_content` / `logrotate_dropin_path` — testable via `include_str!` with
      no root and no systemd (`logrotate_dropin_test.rs` pattern).
- [ ] Re-running `service install` is idempotent (overwrite, no duplicate units).
- [ ] The helper `.service` does **not** inherit the node's sandbox directives.

**REQ-215-012 (Should)** — refresh on existing hosts
- [ ] When `doli upgrade` runs as root and the helper units are absent or differ from the
      rendered content, it writes them, runs `daemon-reload` and enables the `.path` unit,
      printing one line about it.
- [ ] When not root, it prints nothing about units and does not fail.
- [ ] The added call site in `cmd_upgrade.rs` is ≤ 3 lines (see REQ-215-016).

**REQ-215-013 (Should)** — uninstall
- [ ] `doli service uninstall` disables and removes both helper units and tolerates their absence.
- [ ] Structurally asserted in the `logrotate_dropin_test.rs` style (`cmd_uninstall` body
      references the helper path fn and `remove_file`).

**REQ-215-014 (Must)** — startup preflight WARN
- [ ] Given a non-writable target and no helper unit, when the update service starts, then
      **exactly one** WARN is logged naming (1) the target path, (2) the reason
      ("not writable by this process"), (3) the fix (`sudo doli service install`), and (4) a
      fixed greppable token (e.g. `UPDATE_TARGET_NOT_WRITABLE`) for monitoring.
- [ ] Given a non-writable target **and** a helper present, then INFO (not WARN), stating the
      node will stage and the helper will install.
- [ ] Given a writable target, then no extra line at all.
- [ ] The WARN is emitted once per process start, never on a timer.
- [ ] Helper presence is detected by reading `/etc/systemd/system/` (readable under
      `ProtectSystem=full`); a failed scan degrades to "no helper" and never panics.

**REQ-215-015 (Should)** — one staging log line
- [ ] A successful staging emits exactly one INFO naming the version, the staging directory,
      and that a privileged helper must complete the install. No per-tick repetition.

**REQ-215-016 (Must)** — module budget (Rule 19)
- [ ] After the change: `crates/updater/src/apply.rs` ≤ 500 (it is **699 today** — it must
      **shrink**, by extracting the fetch/verify prefix, not grow);
      `bins/node/src/updater/service.rs` ≤ 500 (**478 today** — at most ~15 new lines);
      `bins/cli/src/cmd_service.rs` ≤ 500 (**918 today** — must not grow; new content in a
      new module); `bins/cli/src/cmd_upgrade.rs` ≤ 500 (**507 today** — must **shrink**).
- [ ] Every new file ≤ 500 lines (≤ 800 for tests).
- [ ] `commands.rs` (1397) and `main.rs` (591) are pre-existing over-budget files: the change
      adds **≤ 4 lines each** and **no logic**.
- [ ] Recommended headroom moves (mechanical, zero behaviour change, already the project's
      own convention — cf. `cmd_release_verify.rs` → `cmd_release_verify_tests.rs`):
      move `cmd_upgrade.rs:399-507` (`mod inc_i_199_trust_root_advice_tests`) to
      `bins/cli/src/cmd_upgrade_tests.rs` via `#[path]` → cmd_upgrade.rs ≈ 399 lines.

**REQ-215-017 (Must)** — one verifier (INV-REL-001 / INV-REL-002)
- [ ] `--from-staged` calls `updater::verify_release_manifest` (or a delegate of it) — no
      re-implementation of version bind, checksum bind, signature counting or platform-hash parsing.
- [ ] The per-platform tarball hash comes from `download.rs::platform_tarball_hash`
      (`pub(crate)` today) exposed through a new **bytes-in-hand** entry point in
      `install_gate.rs`; the existing `verify_release_artifact(&GithubReleaseInfo, …)`
      becomes a thin delegate so its two callers are untouched.
- [ ] The 3-of-5 threshold is read from `TrustRoot::threshold()`, never from the
      `REQUIRED_SIGNATURES` constant.

**REQ-215-018 (Must)** — documentation
- [ ] Every file in §4.4 updated; `docs/DOCS.md` / `specs/SPECS.md` indices consistent.

**REQ-215-019 (Must)** — deploy safety
- [ ] `git diff --stat` shows **zero** changes under `crates/core/` and `bins/node/src/node/`.
- [ ] The commit message answers both deploy questions: consensus rules NO, block content NO;
      rolling-safe, no activation height.

**REQ-215-020 (Won't)** — explicitly out of scope
- Relaxing `ProtectSystem` / `NoNewPrivileges`; chowning the binary to `doli`; a root timer
  that runs `doli upgrade` (bypasses veto/approval); `CAP_DAC_OVERRIDE`; a persistent root
  daemon; touching `install.sh` / `postinst.sh` sudoers; auto-update on macOS/launchd hosts.

### Trust boundaries introduced

| # | Boundary | Data crossing | Consuming operation | Control |
|---|---|---|---|---|
| TB-1 | `{data_dir}/updates/` — written by `doli`, read by root | tarball, `CHECKSUMS.txt`, `SIGNATURES.json`, `ready` | root install of an executable | Full L1–L4 gate against the on-chain trust root **inside the root process** (REQ-215-006/007/008) |
| TB-2 | `ready` marker → systemd `.path` activation | file existence | starts a root oneshot | Marker created by rename only; the unit passes a **fixed** `ExecStart` (no marker content is ever interpolated into a command line) |
| TB-3 | `<DIR>` argument of `--from-staged` | operator-supplied path | file reads + install | Path is used only for reads; no shell interpolation; a directory that fails any gate installs nothing |

**Explicit non-control:** the staging dir's *contents* are untrusted. A compromised node can
write anything there; the design's whole value is that root re-verifies rather than trusting.
The `ExecStart` must therefore never derive any argument from staged file *contents*.

---

## 6. Milestones

Each is independently committable and ≤ 4 modules. Strict TDD: the RED tests land first and
must fail before any source edit.

### M1 — updater staging (crates/updater)
Modules: `crates/updater/src/staging.rs` (new) · `crates/updater/src/fetch_verified.rs` (new,
lifted out of `apply.rs` so apply.rs **shrinks** under 500) · `crates/updater/src/install_gate.rs`
(+`verify_release_artifact_bytes`) · `crates/updater/src/lib.rs` (exports).
Requirements: REQ-215-001, -002, -005, -016, -017.
RED tests (`crates/updater/tests/inc_i_215_staging.rs`):
- `probe_reports_not_writable_for_a_read_only_directory` (and for EACCES) [REQ-215-001]
- `probe_leaves_no_artifact_in_a_writable_directory` [REQ-215-001]
- `stage_writes_tarball_manifest_checksums_and_ready` [REQ-215-002]
- `ready_marker_is_created_by_rename_not_in_place_write` (structural + fs) [REQ-215-002]
- `staged_artifact_is_the_tarball_not_the_extracted_binary` [REQ-215-002]
- `staged_ready_for_same_version_reports_already_staged` [REQ-215-005]
- `verify_release_artifact_bytes_matches_verify_release_artifact_on_the_same_inputs` [REQ-215-017]
- `apply_rs_is_within_the_module_budget` [REQ-215-016]
GREEN-lock: `apply_install_mode.rs`, `inc_i_172_install_gate_binding.rs`,
`inc_i_172_apply_update_gate.rs` must stay green.

### M2 — node auto_apply branch + startup preflight (bins/node)
Modules (SHIPPED): `bins/node/src/updater/staged_apply.rs` (new, 66 lines, sibling module) ·
`bins/node/src/updater/preflight.rs` (new, 73 lines) · `bins/node/src/updater/service.rs` (496 lines,
+18) · `bins/node/examples/inc_i_215_preflight_probe.rs` (outcome probe).
Requirements: REQ-215-003, -004, -005, -014, -015, -016.
RED tests (`bins/node/tests/inc_i_215_staged_apply.rs`, structural in the
`inc_i_172_service_timing_test.rs` style + unit tests on the pure preflight helpers):
- `auto_apply_probes_writability_before_installing` [REQ-215-001/003]
- `auto_apply_still_contains_the_trust_root_reverification` (GREEN-lock, INC-I-172 F7(a))
- `auto_apply_does_not_clear_pending_on_the_staging_path` [REQ-215-004]
- `auto_apply_checks_the_ready_marker_before_fetch` [REQ-215-005]
- `preflight_warns_once_naming_target_reason_and_fix_when_no_helper` [REQ-215-014]
- `preflight_is_info_not_warn_when_a_helper_watches_this_staging_path` [REQ-215-014]
- `preflight_is_silent_when_the_target_is_writable` [REQ-215-014]
- `staging_success_emits_exactly_one_info_line` [REQ-215-015]
- `helper_scan_tolerates_missing_or_unreadable_units_dir` [REQ-215-014]
- `service_rs_is_within_the_module_budget` [REQ-215-016]

### M3 — CLI `doli upgrade --from-staged` (bins/cli)
Modules: `bins/cli/src/cmd_upgrade_staged.rs` (new) · `bins/cli/src/commands.rs` (+4) ·
`bins/cli/src/main.rs` (+4) · `bins/cli/src/cmd_upgrade_tests.rs` (moved test module, headroom).
Requirements: REQ-215-006 … -010, -016, -017.
RED tests (`bins/cli/tests/it/inc_i_215_from_staged.rs`, registered in `tests/it/main.rs`;
harness = `CARGO_BIN_EXE_doli` + `tempfile` + `write_on_chain_root` per
`inc_i_214_release_sign_verify.rs`):
- `from_staged_installs_and_reports_the_distinct_signer_count` [REQ-215-006]
- `from_staged_prints_trust_root_provenance_and_last_derived_height` [REQ-215-006, INC-I-206]
- `from_staged_refuses_a_tampered_tarball_and_leaves_the_binary_untouched` [REQ-215-007, INV-REL-001]
- `from_staged_refuses_a_two_signature_manifest_under_a_five_member_root` [REQ-215-008, INV-REL-001]
- `from_staged_refuses_a_manifest_for_a_different_version` [REQ-215-008, L1]
- `from_staged_refuses_when_ready_or_any_manifest_file_is_missing` [REQ-215-009]
- `from_staged_refuses_a_version_not_newer_than_the_installed_one` [REQ-215-010]
- GREEN-lock: `upgrade_must_not_accept_the_trust_root_override` stays green.

### M4 — helper units in `service install` + root `upgrade` refresh (bins/cli)
Modules: `bins/cli/src/cmd_service_helper_units.rs` (new) · `bins/cli/src/cmd_service.rs`
(≤ ~8 lines in `install_systemd` + `cmd_uninstall`) · `bins/cli/src/cmd_upgrade.rs` (≤ 3 lines).
Requirements: REQ-215-011, -012, -013, -016.
RED tests (`bins/cli/tests/inc_i_215_helper_units_test.rs`, `logrotate_dropin_test.rs` pattern):
- `helper_unit_content_and_path_helpers_are_defined` [REQ-215-011]
- `path_unit_watches_the_ready_marker_and_names_the_service_unit` [REQ-215-011]
- `service_unit_is_a_root_oneshot_running_upgrade_from_staged_with_the_network` [REQ-215-011]
- `service_unit_carries_no_sandbox_directives_from_the_node_unit` [REQ-215-011]
- `unit_names_derive_from_the_resolved_service_name_so_multi_node_hosts_do_not_collide` [REQ-215-011]
- `install_systemd_writes_and_enables_the_path_unit` [REQ-215-011]
- `cmd_uninstall_removes_both_helper_units` [REQ-215-013]
- `cmd_upgrade_refreshes_the_helper_units_when_root` [REQ-215-012]
- `cmd_service_rs_and_cmd_upgrade_rs_are_within_the_module_budget` [REQ-215-016]

### M5 — docs / specs / skills alignment
Modules: the §4.4 list. Requirements: REQ-215-018, -019.
RED test: none (documentation). Gate = `/sync-docs` + a manual read-through, plus a
`git diff --stat` check that `crates/core/` and `bins/node/src/node/` are untouched [REQ-215-019].

**Ordering:** M1 → M2 → M3 → M4 → M5. M3 depends on M1's `verify_release_artifact_bytes`;
M4's units reference M3's flag; M2 and M3 are independent of each other after M1.

---

## 7. What I do not understand (stated before the verdict)

1. **Whether any deployed host has a writable target** and therefore keeps the current path.
   I read the code, not the fleet. If *every* systemd host is sandboxed, REQ-215-003's branch
   is dead code in production (still correct to keep — dev/`/mainnet/bin` layouts exist).
2. **The exact `check_interval_secs` configured on the affected hosts.** The compiled default
   is 6 h; the observed retry cadence was 10 min, so the unit sets it. Does not change the fix.
3. **Whether `systemd` `.path` units are available on every deployed distro/version.** I did
   not survey the fleet's systemd versions. `PathExists=` is ancient and universally present,
   but the *fallback for a host without the helper* is REQ-215-014's WARN — which is why that
   requirement is Must and not Should.
4. **Whether the helper should also handle the `doli` CLI binary** when `doli` itself lives on
   a read-only mount. `--from-staged` runs *as* `doli`; replacing a running `doli` works via
   temp+rename (`apply.rs:181-196`), but I have not verified this against a
   `/usr/bin`-read-only host — flagged for the developer.
5. **The skills-install side effect.** `auto_apply_from_github` also installs agent skills to
   `$HOME/.doli/skills` (`apply.rs:518`). Under the sandbox `$HOME` for `User=doli` may be
   unwritable too; it is best-effort and non-fatal today, so I did not make it a requirement.

---

## 8. Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.85, basis=code-read + confirmed live evidence + user-approved design)
Reasoning:
  1. The root cause is CONFIRMED with live evidence from the failing host
     (incident_entries #2731: EROFS at 15:53:13Z and 16:03:13Z, unit directives read
     off disk, /usr/bin/doli-node mtime and owner captured). No diagnostic work remains.
  2. There are ZERO previous failed fix attempts on this incident, so the evidence-pivot
     trigger does not fire.
  3. Brittleness check is 2/5 -> LOCALIZED. Both signals (invariant gap, contract absence)
     restate the same missing contract, and the approved design creates exactly that
     contract. No architectural redesign is implied.
  4. The design is USER-APPROVED with the alternatives explicitly rejected (#2732), so
     there is no design space left for an architect to explore.
  5. The one genuinely open design question -- what artifact gets staged -- is RESOLVED
     in this document from code (install_gate.rs L1-L4 binds the TARBALL; nothing signed
     covers the extracted ELF), together with module placement, the reuse points, the
     idempotency rule and the unit shapes. That IS the architecture; an architect stage
     would re-derive it.
  6. The multi-module surface (3 crates, 5 milestones) is the FIX's surface, not
     diagnostic uncertainty. DEEP buys diagnosis or redesign; this incident needs neither.
ESCALATE TO DEEP IF: the developer finds that staging the tarball cannot satisfy
  REQ-215-006 (i.e. the L1-L4 chain cannot be re-run offline from the staged files), or
  that keeping `verify_release_with_trust_root` inside auto_apply's body is incompatible
  with the staging branch. Either would mean the trust contract needs redesign, not
  implementation.
━━━━━━━━━━━━━━━━━━━━━
```

---

## 9. Resource cost

```
━━━ RESOURCE COST ━━━
Scope: per node, per APPROVED release (not per poll). Steady state is unchanged.

CPU
  Node: +1 sha256 over the tarball is NOT added -- verify_hash already runs today
        (apply.rs:474). Staging adds only file writes. The writability probe is one
        create + one unlink per auto-apply attempt (microseconds).
        Startup preflight: one probe + one readdir of /etc/systemd/system, once per
        process start. Effectively zero.
  Host: the root helper runs ONE oneshot per release: sha256 over the tarball + the
        3-of-5 Ed25519 signature verifications + gunzip/untar. Seconds, once, and it
        replaces work that already happens today inside the node.
  Verdict: no change to the 10-second slot budget. Nothing on the block path.

Memory
  Node: unchanged. The tarball is already fully in RAM today (download_from_url returns
        Vec<u8>, apply.rs:469); staging writes those same bytes out instead of extracting
        them. No new peak allocation.
  Host: the oneshot's RSS is bounded by the tarball size, in a separate short-lived
        process, not in the node's address space.

IO / disk
  +1 write of one release tarball (tens of MB) into {data_dir}/updates/, plus two small
  manifest files and a zero-length marker. Written ONCE per approved release, deleted by
  the helper after a successful install. This is genuinely NEW IO -- today the node never
  touches the disk on this path.
  Steady-state footprint: 0 bytes (drained). Worst case (helper never runs): ONE tarball,
  overwritten by the next version, never accumulating -- bounded, not unbounded.
  Contention: the write is a single sequential append in the data dir, not in RocksDB's
  path. It does not share a WAL or a column family with the state DB.

Locks
  None added. The staging call sits in `auto_apply`, which already holds no state lock
  across the download; `self.pending` is read (:382-385) and written (:407-411) exactly as
  today. No lock is held across any filesystem or network call. Zero contention with
  apply_block or the producer path.

Host cost of the helper .path unit
  A systemd .path unit is an inotify watch on one directory -- one file descriptor and a
  few KB of kernel state, no polling, no timer, no resident process. It is the cheapest
  activation primitive systemd offers.
  Two extra unit files on disk (~1 KB total). One extra daemon-reload at install time.

Risk-weighted view
  The cost is one tarball-sized disk write per release, on a node whose auto-update is
  currently 100% dead. The alternative being paid today is manual `sudo doli upgrade` on
  every host before every activation height.
━━━━━━━━━━━━━━━━━━━━━
```

---

## 10. Traceability matrix

| Requirement | Priority | Milestone | Test IDs | Architecture § | Implementation module |
|---|---|---|---|---|---|
| REQ-215-001 | Must | M1, M2 | (test-writer) | §3(i) | `crates/updater/src/staging.rs` |
| REQ-215-002 | Must | M1 | (test-writer) | §1, §3(b) | `crates/updater/src/staging.rs` |
| REQ-215-003 | Must | M2 | `auto_apply_probes_writability_before_installing` | §2.2 | `install_target_is_writable` @ `bins/node/src/updater/staged_apply.rs` + branch @ `service.rs::auto_apply` |
| REQ-215-004 | Must | M2 | `auto_apply_does_not_clear_pending_on_the_staging_path` | §3(j) | `service.rs::auto_apply` staging arm @ `bins/node/src/updater/service.rs` |
| REQ-215-005 | Must | M1, M2 | `auto_apply_checks_the_ready_marker_before_fetch` | §3(j) | `already_staged` + `log_already_staged` @ `bins/node/src/updater/staged_apply.rs` |
| REQ-215-006 | Must | M3 | (test-writer) | §3(b),(c),(d) | `bins/cli/src/cmd_upgrade_staged.rs` |
| REQ-215-007 | Must | M3 | (test-writer) | §5 TB-1 | `cmd_upgrade_staged.rs` |
| REQ-215-008 | Must | M3 | (test-writer) | §5 TB-1 | `cmd_upgrade_staged.rs` |
| REQ-215-009 | Must | M3 | (test-writer) | §5 TB-1 | `cmd_upgrade_staged.rs` |
| REQ-215-010 | Should | M3 | (test-writer) | §3(j) | `cmd_upgrade_staged.rs` |
| REQ-215-011 | Must | M4 | (test-writer) | §3(e) | `bins/cli/src/cmd_service_helper_units.rs` |
| REQ-215-012 | Should | M4 | (test-writer) | §3(d) | `cmd_service_helper_units.rs` |
| REQ-215-013 | Should | M4 | (test-writer) | §3(e) | `cmd_service.rs::cmd_uninstall` |
| REQ-215-014 | Must | M2 | `preflight_warns_once_...`, `preflight_is_info_not_warn_...`, `preflight_is_silent_...`, `helper_scan_tolerates_...` | §3(g) | `helper_unit_watches` + `preflight_verdict` + `report_install_target` @ `bins/node/src/updater/preflight.rs` |
| REQ-215-015 | Should | M1, M2 | `staging_success_emits_exactly_one_info_line` | §2.5(4) | `stage_for_privileged_install` @ `bins/node/src/updater/staged_apply.rs` |
| REQ-215-016 | Must | M1–M4 | (test-writer) | §5 | all |
| REQ-215-017 | Must | M1, M3 | (test-writer) | §2.3 | `install_gate.rs` |
| REQ-215-018 | Must | M5 | n/a | §4.4 | docs/specs/skills |
| REQ-215-019 | Must | M1–M5 | n/a | §4.1 | commit gate |
| REQ-215-020 | Won't | — | n/a | §5 | — |

## 11. Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| 1 | `ProtectSystem=full` leaves `/etc` readable, so a `doli`-user process can stat and read unit files | The node can check whether the root helper is installed | No — systemd doc knowledge; **developer must verify on a real host** |
| 2 | `{data_dir}` is inside `ReadWritePaths`, so `{data_dir}/updates/` is writable by the node | The node can write where it already writes `pending_update.json` | Yes — `cmd_service.rs:372` + live evidence #2731 |
| 3 | A 5-member on-chain maintainer set yields threshold 3 | "3 of 5 signatures" holds in tests | Yes — `crates/core/src/maintainer/set.rs:113` |
| 4 | `install_binary`'s temp+rename can replace a running binary | The helper can swap `doli-node` while it runs | Yes — `apply.rs:13-23` documents the `(deleted)` case |
| 5 | The `.path` unit fires on `PathExists=` for a file created by rename | The marker reliably triggers the helper | No — **M4 must verify on a real host**; the rename discipline (REQ-215-002) is the safeguard |
| 6 | No deployed host relies on the sudo fallback path today | Removing/keeping it changes nothing in production | No — the fallback stays untouched, so this assumption is not load-bearing |
