━━━ FINDINGS — 13 total (Critical:1 Major:7 Minor:5) ━━━

  [F1] CRITICAL conf(0.93, observed) — bins/cli/src/cmd_upgrade.rs:104-121 + bins/node/src/commands/misc.rs:203-221 — the two new operator install gates verify a SIGNATURES.json whose version and checksum hash are taken from the file itself and never bound to the artifact being installed; a replayed genuine SIGNATURES.json authorises an arbitrary tarball
  [F2] MAJOR conf(0.90, observed) — bins/node/src/commands/update.rs:143-150 → crates/updater/src/apply.rs:78-147 — `doli-node update apply` is a fifth install path with zero signature verification; F7(a) re-verification landed only in `auto_apply`, so revocation cannot reach it
  [F3] MAJOR conf(0.88, observed) — bins/node/src/commands/misc.rs:215 + bins/node/src/commands/update.rs:391 — the node's own upgrade/verify commands resolve `TrustRoot::bootstrap` on a host that holds the on-chain set on disk, leaving the leaked compiled constants authoritative on every producer host through one command
  [F4] MAJOR conf(0.85, observed) — crates/storage/src/maintainer.rs:205-221 — `save` is a non-atomic `fs::write` while F5 made a failed load FATAL; a torn write bricks node startup, and the migration performs exactly this write on every node during the rolling deploy
  [F5] MAJOR conf(0.95, observed) — specs/protocol.md:1829-1845 — the authoritative protocol spec still specifies the deleted fail-open algorithm verbatim, plus the entry-count (not distinct-signer) counter and a hardcoded `>= 3`; file untouched by the diff
  [F6] MAJOR conf(0.95, observed) — .claude/skills/updater/SKILL.md:471 — an agent-facing file the diff DID update still says "Bootstrap keys are static fallback; on-chain keys take precedence once synced", contradicting lines 49, 156 and 450 of the same file
  [F7] MAJOR conf(0.90, observed) — crates/updater/src/constants.rs + crates/updater/src/lib.rs — both files carry M1 AND INC-I-157 changes, so "stage only M1 paths" cannot produce a commit that compiles
  [F8] MAJOR conf(0.95, measured) — .omega/memory.db:protection_mechanisms — M1 adds two protection mechanisms (fail-closed release trust root, fatal-on-undecodable maintainer state) and registers neither; their interaction with the auto-updater restart path is unrecorded
  [F9] MINOR conf(0.90, observed) — bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs:65 (+3 siblings) — the four `include_str!` regression tests assert source-text wiring order, not the security property; none of them can fail for F1
  [F10] MINOR conf(0.85, observed) — crates/updater/src/verification.rs:99 — exact-string public-key comparison rejects an uppercase-hex SIGNATURES.json entry with a misleading `InsufficientSignatures`
  [F11] MINOR conf(0.90, measured) — crates/storage/src/maintainer.rs:27-28 — the magic-aliasing justification cites "~5.7 billion" members; the actual threshold is 1,414,745,412 (0x54534D44)
  [F12] MINOR conf(0.90, observed) — crates/updater/src/constants.rs:126-132 + crates/updater/src/params.rs:20-60 — `get_maintainer_keys` survives as a dead second reader of the compiled keys, and three `UpdateParams` seniority fields are now inert config
  [F13] MINOR conf(0.85, observed) — crates/storage/src/maintainer.rs:109-111 → bins/node/src/updater/trust_root_wiring.rs:74-83 — deleting or wiping `maintainer_state.bin` silently returns the host to the Bootstrap root, i.e. to the leaked constants, with no warning and no monitoring signal

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-172 M1 — maintainer trust root (Layer 1, node-local)

Run 508 · branch `bugfix/inc-i-172-maintainer-trust-root` · reviewer pass 1
Contract: `docs/.workflow/inc-i-172-M1-api-contract.md` (§1-§10, including runner
decisions §8 G1-G5, correction §9, override §10).
Design: `specs/maintainer-trust-root-architecture.md` F1/F3/F5/F6/F7/F8.

---

## 1. Scope reviewed, and what was excluded

Reviewed (M1 payload):

| Area | Files |
|---|---|
| Trust-root type + verification | `crates/updater/src/trust_root.rs`, `verification.rs`, `types.rs`, `apply.rs`, `enforcement.rs`, `params.rs`, `vote.rs` |
| Composition root | `bins/node/src/run.rs`, `bins/node/src/updater/{mod.rs,trust_root_wiring.rs,service.rs,service_checks.rs}`, `bins/node/src/node/startup.rs` |
| Install paths | `bins/cli/src/{main.rs,cmd_upgrade.rs,upgrade_restart.rs}`, `bins/node/src/{main.rs,commands/misc.rs,commands/update.rs,commands/maintainer.rs}` |
| Storage format | `crates/storage/src/maintainer.rs`, `crates/storage/src/lib.rs` |
| Tests | `crates/storage/tests/maintainer_state_versioned_test.rs`, `crates/updater/tests/trust_root_fail_closed.rs`, `bins/node/tests/inc_i_172_*.rs`, `bins/cli/tests/inc_i_172_*.rs` |
| Docs/specs | `docs/{DOCS.md,architecture.md,attack_analysis.md,auto_update_system.md,cli.md}`, `specs/{SPECS.md,delegation-architecture.md,maintainer-trust-root-architecture.md}`, `.claude/skills/{SKILLS-INDEX.md,auto-update,updater}/SKILL.md` |

**Excluded — INC-I-157 (release-origin de-pinning), a different incident:**
`crates/updater/src/download.rs`, `crates/updater/tests/origin_pinning.rs`, `Cargo.toml`
(the `repository` URL only — the workspace version is unchanged at `6.24.1`),
`docker/docker-compose{,.devnet,.testnet}.yml`, `scripts/{README.md,install.ps1,publish_release.sh,sign-release.sh}`,
`docs/{buy_doli.md,producer_node_quickstart.md,troubleshooting.md,testnet.md,docker.md,releases.md,running_a_node.md}`,
`specs/{engine-parts.md,gui-architecture.md}`, `testnetlinux/explorer/{index,network}.html`,
`docs/qa/inc-i-157-M1-qa-report.md`, `docs/reviews/inc-i-157-M1-origin-depinning-review.md`.
Verified INC-I-157-only by diff inspection: `git diff docs/releases.md docs/running_a_node.md`
filtered of `e-weil`/`doli-network` lines yields the empty set.

**Excluded — unrelated untracked work:** `docs/bugfixes/inc-i-15{3,4,7}-*`, `inc-i-16{2,7}-*`,
`inc-i-170-*`, `family-ram-growth-*`, `mainnet-fresh-sync-wedge-*`, `docs/announcements/`,
`docs/reports/`, `docs/reviews/inc-i-153-*`, `.claude/skills/producer-retirement/`.

**Cannot be excluded by path — see [F7]:** `crates/updater/src/constants.rs`,
`crates/updater/src/lib.rs`, `docs/architecture.md`, `.claude/skills/updater/SKILL.md`
each contain BOTH incidents' changes.

## 2. Gate results (run by the reviewer, this working tree)

```
cmd:$ cargo build --release
   → exit 0

cmd:$ cargo clippy --workspace --all-targets -- -D warnings
   Finished `dev` profile [optimized + debuginfo] target(s) in 0.39s
   → exit 0   (no warnings; this is also the positive control that the F8
                deletions have no surviving caller anywhere, tests included)

cmd:$ cargo fmt --check
   → exit 0 (no output)

cmd:$ cargo test -p storage -p updater -p doli-cli -p doli-core
   → exit 0; 44 suites, every line "test result: ok"; 0 failed
     (largest: 972 passed, 251 passed, 195 passed)

cmd:$ cargo test -p doli-node -- --skip test_cluster_10x100
   → exit 0; final suite "test result: ok. 12 passed; 0 failed; 11 ignored;
     0 measured; 2 filtered out; finished in 171.19s"; 0 failed across all suites
```

The whole gate is green. Every finding below is a defect the green gate does not see.

## 3. Question 1 — is F1's root cause structurally true?

**Partly. The deletion is real and correct where it was applied; it does not reach two
of the five install paths.**

`verification.rs` no longer names `bootstrap_maintainer_keys` (verification.rs:1-204;
the only remaining mention is prose at :62). The empty-slice sentinel is gone with
`verify_release_signatures_with_keys`. `TrustRoot::bootstrap` (`trust_root.rs:54-63`) is
now the sole reader of the compiled array, and only two constructors exist.

Complete enumeration of every path that still reaches the compiled constants
(`grep -rn "bootstrap_maintainer_keys\|TrustRoot::bootstrap\|verify_release_signatures("
bins crates --include='*.rs'`, plus `bins/node/src/cli.rs:33`):

| # | Path | Reaches compiled keys via | Verdict |
|---|---|---|---|
| 1 | `trust_root_wiring.rs:74-83` — node, `members.is_empty() && last_derived_height == 0` | `TrustRoot::bootstrap` | **Legitimate.** Genuinely unbootstrapped node; REQ-172-005 mandates it. But see [F13] — a wiped data dir is indistinguishable from a fresh one. |
| 2 | `bins/cli/src/cmd_upgrade.rs:114` — `doli upgrade` | shim → `TrustRoot::bootstrap` | **Legitimate by design** (the CLI has no chain state) — but the gate itself is vacuous, [F1]. |
| 3 | `bins/node/src/commands/misc.rs:215` — `doli-node upgrade` | shim → `TrustRoot::bootstrap` | **Surviving fail-open, [F3].** This binary runs on a host that HAS `maintainer_state.bin`; `--data-dir` is in scope at `bins/node/src/cli.rs:33` and `resolve_trust_root` is already `pub`. |
| 4 | `bins/node/src/commands/update.rs:391` — `doli-node update verify` | shim → `TrustRoot::bootstrap` | **Contract-blessed (§2) but same objection as #3**, and now that it returns `Err` an operator can script it and be told a revoked-key release verifies. |
| 5 | `bins/node/src/commands/maintainer.rs:97` | `bootstrap_maintainer_keys` | Display only. Legitimate; wording corrected. |
| 6 | `crates/updater/src/constants.rs:126-132` `get_maintainer_keys` | `bootstrap_maintainer_keys` | **Dead** — zero non-test callers. Deferred to M2 per §10; recorded as [F12]. |
| 7 | `node/mod.rs:76`, `node/startup.rs:9,16` `is_using_placeholder_keys` | `bootstrap_maintainer_keys` | Legitimate — placeholder detection, no authorization. |

So: **the deletion is structurally sound for the auto-update path** (`service_checks.rs:77-90`
and `service.rs:361-377` both take the resolved root and both fail closed), and it is
**incomplete for the operator paths**, which are exactly where §10 said M1 must be complete.

## 4. Question 2 — does the fail-closed property hold?

**Yes for the property as stated; the attacks that get through go around it, not through it.**

- `TrustRoot::is_usable()` (`trust_root.rs:96-98`) requires `threshold >= 1 && keys.len() >= threshold`.
  The `threshold >= 1` half is load-bearing and correctly justified in the doc comment:
  `calculate_threshold(0) == 0`, so `valid >= threshold` would be vacuously true on a defaulted
  set. I tried to construct an empty-or-sub-threshold root that verifies anyway and could not:
  `verify_release_with_trust_root` returns `TrustRootUnavailable` before touching signatures
  (`verification.rs:76-91`).
- **`try_read()` contention is fail-CLOSED, not fail-open.** `trust_root_wiring.rs:106-119`
  returns `TrustRoot::on_chain(Vec::new(), REQUIRED_SIGNATURES)` on `Err(_)` — an unusable
  OnChain root, not `Bootstrap`. This is the correct choice and the comment says why. The
  contending writers are `node/periodic.rs:66` (`maybe_bootstrap_maintainer_set`) and
  `node/apply_block/governance.rs:22,52`; both hold the write lock briefly, and a missed check
  simply retries on the next 6 h tick. The `getUpdateStatus` mirror
  (`node/startup.rs:473-476`) degrades to `trust_root: null`, which is display-only.
- **A set that shrinks while an update is pending is handled.** `auto_apply`
  (`service.rs:355-377`) re-resolves the root and re-verifies the staged release immediately
  before `auto_apply_from_github`, and on failure clears the in-memory pending update AND
  removes `pending_update.json`. That is a correct implementation of F7(a) — for that path.
  It does **not** cover `doli-node update apply`: see **[F2]**.
- The distinct-signer counter (`verification.rs:96-111`) is the covenant k-of-n shape:
  outer loop over `root.keys()`, inner over `release.signatures`, `break` after the first
  valid entry. Three entries from one key count as one. Correct.
- `published_at` is fully removed from every deadline (`enforcement.rs:26-56`,
  `params.rs:91-113`, `service.rs:199,240`, `updater/mod.rs:113-131`, `apply.rs:87-88`).
  G1 is satisfied in full, including the display-side `days_remaining`/`hours_remaining`.

The two ways an attacker still wins are [F1] (the gate does not bind the artifact) and
[F3] (the gate consults the wrong root). Neither is a hole in `is_usable()`.

## 5. Question 3 — the storage format change

**Encoder/decoder parity: correct and pinned.** `save` writes `MAGIC || u32 LE version ||
bincode(MaintainerStateBody)` (`maintainer.rs:205-221`); `decode_current` reads
`data[4..8]` as the version and `data[HEADER_LEN..]` as the body (`:127-147`). The body
struct is deliberately identical to the legacy schema, and `test_body_encoding_is_identical_to_the_legacy_layout`
(`:304-332`) asserts the post-header bytes equal a fresh legacy encoding — that is a real
parity test, not a round-trip tautology.

**The migration cannot lose or alter a set.** `migrate_legacy` (`:156-191`) decodes with
the same schema, so `members` / `threshold` / `last_derived_height` pass through
`from_body` untouched. `crates/storage/tests/maintainer_state_versioned_test.rs:396-455`
migrates the real 232-byte file from a live node and asserts field equality.

Edge cases, all traced:

| Input | Behaviour | Verdict |
|---|---|---|
| missing file | `Ok(default())` (`:109-111`) | correct; but see [F13] |
| zero-length file | `len < HEADER_LEN` → `migrate_legacy` → bincode error → `Err(Serialization)` → FATAL | correct as a *decision*, dangerous as a *reachable state* — [F4]. Covered by test `:521-530`. |
| exactly `HEADER_LEN` bytes, magic present | `decode_current` on an empty body slice → `Err(Serialization)` | correct |
| exactly `HEADER_LEN` bytes, no magic | `migrate_legacy` → 8 bytes cannot satisfy `MaintainerSet` → `Err` | correct |
| legacy file that starts with the magic by coincidence | impossible: requires `set.members.len() == 0x54534D44` | correct; the cited magnitude is wrong, [F11] |
| legacy file with exactly 1 member (aliases `VERSION = 1`) | no magic ⇒ LEGACY branch, migrated | correct; regression test at `:334-394` |
| read-only data dir during migration | `save` fails → `warn!` → `Ok(state)`, node continues | correct, §9 compliant. **Not covered by a test** — minor gap. |

**INV-4 / INC-I-054 compliance: clean.** `git diff -U0 | grep '^\+.*activation_height'` returns
only a line inside the `specs/SPECS.md` index prose — no code activation height. No
`CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION` or `MIN_PEER_PROTOCOL_VERSION`
change anywhere in the diff. `MAINTAINER_STATE_VERSION` is a separate, node-local constant
and the comment at `maintainer.rs:44-47` states so.

**Genuinely node-local: confirmed.** `grep -rn "MaintainerState" bins crates --include='*.rs'`
yields consumers in `bins/node/src/updater/trust_root_wiring.rs`, `bins/node/src/node/mod.rs`,
`crates/rpc/src/methods/{governance.rs,context.rs}` and `crates/storage/src/lib.rs` only.
It is not gossiped, not hashed, not part of any state root, and `MaintainerStateBody` is a
private struct with a single writer. The added `version` field lives on the outer struct and
is never serialized into the body.

## 6. Question 4 — scope discipline

**Nothing went too far.**

- `git status --porcelain crates/core` → empty. `crates/core/src/maintainer.rs`
  (governance multisig, `calculate_threshold`, `derive_maintainer_set`) is untouched; the
  `// M2:` deferral comment sits at `trust_root_wiring.rs:67-72` as §8 G5 required.
- `git status --porcelain WHITEPAPER.md WHITEPAPER_ES.md` → empty. Correct; §7 forbade it.
- No activation height. No consensus-visible computation touched.
- No `Cargo.toml` version bump. The single `Cargo.toml` hunk changes `repository` and is
  INC-I-157.

**The reverse check is also clean.** `cargo clippy --workspace --all-targets -D warnings`
exits 0, which is a workspace-wide positive control that the F8 deletions
(`with_weights`, `set_weights`, `should_reject_weighted`, `veto_weight`, `approval_weight`,
`veto_percent_weighted`, `calculate_vote_weight`, `seniority_multiplier`,
`is_eligible_to_vote`) left no live or test caller. The `producer_weights` field removal
from `VoteTracker` is safe across restarts: `PendingUpdate` persists as JSON and serde_json
ignores unknown fields by default, so an existing `pending_update.json` still loads.

## 7. Findings

### [F1] CRITICAL — the new operator gates verify a signature that is not bound to the artifact

- **Location:** `bins/cli/src/cmd_upgrade.rs:104-121`; `bins/node/src/commands/misc.rs:203-221`
- **Evidence:**
  ```rust
  // cmd_upgrade.rs:104-107 (misc.rs:203-206 is byte-equivalent)
  let sig_release = updater::Release {
      version: sf.version.clone(),
      binary_sha256: sf.checksums_sha256.clone(),
      ...
  ```
  `sf` is the downloaded `SIGNATURES.json`. The signed message is
  `"{sf.version}:{sf.checksums_sha256}"` — **both operands come from the same
  attacker-supplied file.** `GithubReleaseInfo` already carries the authoritative pair:
  `version` (`crates/updater/src/download.rs:331`) and `checksums_sha256`
  (`download.rs:336-341`, documented as "the value that maintainer signatures cover").
  Neither is compared.
  Positive control that the correct shape exists in this codebase: the auto-update path
  DISCARDS `sf.version` / `sf.checksums_sha256` and binds `Release.version` to the GitHub
  tag and `binary_sha256` to the locally recomputed `sha256(CHECKSUMS.txt)`
  (`download.rs:200-204`, `:230-235`, `:295-297`), then re-compares at install time
  (`crates/updater/src/apply.rs:427-437`).
- **Impact:** an adversary who controls the release origin — the exact adversary
  INC-I-157 and INC-I-172 exist for — serves (a) a malicious tarball, (b) a `CHECKSUMS.txt`
  matching it, (c) a **verbatim copy of any past genuine `SIGNATURES.json`**.
  `verify_hash(tarball, release.expected_hash)` passes because both come from (b).
  `verify_release_signatures` passes because (c) carries real maintainer signatures over a
  real historical `version:hash` pair. Install proceeds. The gate that §10 called "the
  fix … already written and tested" adds **zero** integrity over the checksum it was added
  to backstop, on both operator paths, on every network. This makes REQ-172-006's stated
  property ("the operator-facing path's root is explicit") false in exactly the A5 way §10
  point 3 forbids.
- **Suggested fix:** immediately after `let sf = ...`, before constructing `sig_release`:
  reject unless `sf.version.trim_start_matches('v') == release.version.trim_start_matches('v')`
  **and** `sf.checksums_sha256.eq_ignore_ascii_case(&release.checksums_sha256)`. Put the check
  in one shared helper in `crates/updater` and call it from both sites so they cannot drift.
- **Test strategy:** unit-test the helper with (i) matching pair → `Ok`, (ii) `sf.version`
  from a different release → `Err`, (iii) `sf.checksums_sha256` not equal to the fetched
  `CHECKSUMS.txt` hash → `Err`. Add an integration test that fixture-serves a mismatched
  `SIGNATURES.json` and asserts no `install_binary` call occurs.
- **Confidence:** `conf(0.93, observed)`
- **Severity:** Critical

### [F2] MAJOR — `doli-node update apply` installs with no signature verification at all

- **Location:** `bins/node/src/commands/update.rs:143-150` → `crates/updater/src/apply.rs:78-147`
- **Evidence:**
  ```
  cmd:$ grep -n "verify_release\|TrustRoot" crates/updater/src/apply.rs
  NO_SIGNATURE_VERIFICATION_IN_apply.rs        (no matches; grep exit 1)
  cmd:$ grep -c "verify_release" bins/node/src/updater/service.rs
  2                                            (positive control — the same grep matches
                                                where the control does exist)
  ```
  `apply_update` performs exactly two checks — veto period (`apply.rs:87-105`) and
  `approved` (`:108-123`) — then `download_binary` → `verify_hash` → `backup_current` →
  `install_binary`. The caller passes `approved_or_forced = pending.approved || force`
  (`update.rs:141`), so `--force` also removes the approval check.
- **Impact:** contract §6(a) requires re-verification against the CURRENT trust root
  "immediately before install/apply". It landed only in `UpdateService::auto_apply`. A
  pending update staged before a key rotation, then applied manually, installs under the
  revoked signers. Revocation that cannot reach the manual apply path is not revocation —
  which is F7(a)'s own stated rationale.
  Secondary, latent: `verify_hash(&binary, &release.binary_sha256)` (`apply.rs:134`)
  compares the downloaded **binary** against `sha256(CHECKSUMS.txt)` for any
  GitHub-sourced `Release`. Those are different objects, so this path is probably already
  non-functional against GitHub releases — which limits current exposure but is itself a
  bug and hides the missing verification from anyone testing by hand.
- **Suggested fix:** thread the same `TrustRoot` port into `handle_update_command`'s
  `Apply` arm (it already has `network` and the data dir) and call
  `verify_release_with_trust_root(&pending.release, &root)?` before `apply_update`; or move
  the check inside `apply_update` behind a required `&TrustRoot` parameter so no caller can
  omit it. Fix `verify_hash`'s operand to the per-platform tarball hash while there.
- **Test strategy:** stage a `pending_update.json`, rotate the on-chain set so the staged
  signers are no longer trusted, run the `Apply` arm, assert `Err` and that
  `install_binary` was not reached.
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Major

### [F3] MAJOR — the node's own upgrade command consults the compiled keys, not the on-chain set on its own disk

- **Location:** `bins/node/src/commands/misc.rs:215`; `bins/node/src/commands/update.rs:391`
- **Evidence:** both call `updater::verify_release_signatures(&…, network)`, which is the
  shim that resolves `TrustRoot::bootstrap(network)` (`crates/updater/src/verification.rs:53-55`),
  which reads the compiled array (`crates/updater/src/trust_root.rs:56`). The node binary
  has `pub data_dir: Option<PathBuf>` at `bins/node/src/cli.rs:33`, and
  `crate::updater::{load_maintainer_state, resolve_trust_root}` are already public and
  re-exported (`bins/node/src/updater/mod.rs:37`, `trust_root_wiring.rs:42,58`).
- **Impact:** M1's headline claim is that the leaked compiled constants are no longer
  authoritative. On every producer host that runs `doli-node upgrade`, they still are —
  through the one command §10 added to M1 precisely because operators will reach for it
  when the other paths start refusing. The same applies to `doli-node update verify`,
  which the contract blessed in §2 but which now returns `Err`, making it scriptable
  (`doli-node update verify vX && doli upgrade`) and therefore load-bearing.
- **Suggested fix:** in `handle_upgrade_command`, resolve
  `resolve_trust_root(&load_maintainer_state(&data_dir)?, network)` and call
  `verify_release_with_trust_root`; fall back to `TrustRoot::bootstrap` only when no data
  dir is present or the file is missing, and print which provenance was used. Same for the
  `verify` arm of `handle_update_command`.
- **Test strategy:** write a `maintainer_state.bin` whose set does NOT contain the compiled
  keys, run the upgrade command against a release signed by the compiled keys, assert it
  refuses. Today it accepts.
- **Confidence:** `conf(0.88, observed)`
- **Severity:** Major

### [F4] MAJOR — non-atomic `save` plus fatal `load` can brick a node on the upgrade it ships in

- **Location:** `crates/storage/src/maintainer.rs:205-221`; fatal path at
  `bins/node/src/updater/trust_root_wiring.rs:42-55` and `bins/node/src/run.rs:453`
- **Evidence:** `std::fs::write(&path, out)` (`maintainer.rs:219`) is create + **truncate** +
  `write_all`. A crash or power loss between truncate and write leaves a zero-byte file.
  The project's own test asserts that state must be fatal:
  `crates/storage/tests/maintainer_state_versioned_test.rs:526-530` —
  *"load() ACCEPTED a zero-byte maintainer_state.bin. A torn write that truncates the file
  to 0 must not be indistinguishable from a fresh node."* And `migrate_legacy` calls
  `state.save(data_dir)` (`maintainer.rs:181`), so **every node in the fleet performs this
  non-atomic write exactly once — on its first boot after this upgrade**, i.e. inside the
  rolling-deploy window, on a restart the auto-updater itself triggers. This codebase
  already knows the right pattern: `install_binary` does temp-write + atomic rename
  (`crates/updater/src/apply.rs:149-157`).
- **Impact:** low probability per node, but the outcome is "node refuses to start" with a
  manual-only recovery, across ~30 external auto-updating producers who will not be
  watching. This is the INC-I-153 failure class that §9 was written to prevent, arriving
  through a different door.
- **Suggested fix:** in `save`, write to `maintainer_state.bin.tmp`, `File::sync_all()`,
  then `std::fs::rename` over the target. Three lines, no API change.
- **Test strategy:** unit test asserting that a `save` leaves no `.tmp` behind and that the
  target file is either fully absent or fully valid; plus a test that a pre-existing valid
  file survives a `save` that fails mid-way (simulate by making the tmp path unwritable).
- **Confidence:** `conf(0.85, observed)`
- **Severity:** Major

### [F5] MAJOR — `specs/protocol.md` §10.2 still specifies the deleted fail-open algorithm

- **Location:** `specs/protocol.md:1829-1845`
- **Evidence:** the file is untouched (`git status --porcelain specs/protocol.md` → empty)
  and still reads:
  > "If on-chain state is unavailable (pre-sync, CLI), bootstrap keys hardcoded in
  > `BOOTSTRAP_MAINTAINER_KEYS` are used as **fallback**."
  ```
  if on_chain_keys is non-empty:
      allowed_keys = on_chain_keys
  else:
      allowed_keys = BOOTSTRAP_MAINTAINER_KEYS
  valid_sigs = count(verify(message, sig, key) for sig in signatures where key in allowed_keys)
  release_valid = valid_sigs >= 3
  ```
  Three separate falsehoods against the code: (1) the `else` branch is the deleted F1
  fallback; (2) `count(... for sig in signatures ...)` is the **entry** counter that F3
  replaced with a distinct-signer counter (`crates/updater/src/verification.rs:96-111`);
  (3) `>= 3` is not `root.threshold()`.
- **Impact:** this is the authoritative protocol spec and it is the exact drift class F8
  exists to eliminate. A future reader implementing to spec re-introduces the defect.
- **Suggested fix:** rewrite §10.2 to the `TrustRoot` resolution table from
  `bins/node/src/updater/trust_root_wiring.rs:12-16`, the distinct-signer loop, and
  `root.threshold()`; state explicitly that an empty or sub-threshold on-chain root fails
  closed.
- **Test strategy:** NOT_TESTABLE (documentation). Optionally add `specs/protocol.md` to
  whatever doc-drift check covers the other files.
- **Confidence:** `conf(0.95, observed)`
- **Severity:** Major

### [F6] MAJOR — `.claude/skills/updater/SKILL.md` contradicts itself in a file the diff updated

- **Location:** `.claude/skills/updater/SKILL.md:471` vs `:49`, `:156`, `:450`
- **Evidence:** `:471` — *"Bootstrap keys are static fallback; on-chain keys (first 5
  registered producers) take precedence once synced"*. `:450` — *"Verification FAILS
  CLOSED: an empty or sub-threshold on-chain trust root refuses, it never falls back to
  the compiled bootstrap keys"*. `:156` — *"no fallback to compiled keys"*. The file was
  modified by this diff (124 changed lines) and the stale line survived.
- **Impact:** this is agent-facing executable instruction text with a grep-first index
  (`SKILLS-INDEX.md`). An agent grepping "fallback" lands on `:471` and gets the deleted
  behaviour as fact — the same mechanism that kept the compiled keys authoritative in the
  first place.
- **Secondary, same file:** `:49` and `:156` state the return type is `Ok(())`; the code
  returns `Ok(usize)` (the distinct-signer count) — `crates/updater/src/verification.rs:53,75`.
- **Suggested fix:** delete/rewrite `:471` to match `:450`; correct the two `Ok(())`
  signatures to `Ok(distinct_signers: usize)`.
- **Test strategy:** NOT_TESTABLE (documentation).
- **Confidence:** `conf(0.95, observed)`
- **Severity:** Major

### [F7] MAJOR — the M1 commit cannot be staged by path without breaking the build

- **Location:** `crates/updater/src/constants.rs`, `crates/updater/src/lib.rs`
- **Evidence:** `constants.rs` carries the M1 G3/G4 comment corrections (`:12-18`, `:36-40`,
  `:44-52`, `:70-77`) **and** the INC-I-157 origin re-point plus the deletion of
  `pub const FALLBACK_MIRROR` (`:137-155`). `lib.rs` removes `FALLBACK_MIRROR` from the
  `pub use constants::{…}` list (`lib.rs:67-72`), while `crates/updater/src/download.rs`
  at HEAD still does `use crate::{…, UpdateError, FALLBACK_MIRROR, GITHUB_RELEASES_URL};`.
  Staging `lib.rs` (M1) without `download.rs` + `constants.rs` (INC-I-157) therefore leaves
  `crate::FALLBACK_MIRROR` unresolvable — **the M1 commit does not compile.** Staging
  `constants.rs` instead silently pulls the INC-I-157 origin change into the M1 commit.
- **Impact:** §10 OBS-004's plan ("stage ONLY M1-relevant paths") is not achievable at file
  granularity. A non-compiling commit on `main` in an auto-update codebase is a bisect
  hazard at minimum.
- **Suggested fix:** commit INC-I-157 first as its own commit (it is already reviewed and
  QA'd), then commit M1 on top. If that is not acceptable, stage
  `constants.rs` + `lib.rs` + `download.rs` + `tests/origin_pinning.rs` together and say in
  the M1 commit message that it carries the INC-I-157 origin change.
- **Test strategy:** `git stash` the unstaged remainder and run `cargo check -p updater`
  against the staged tree before committing.
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Major

### [F8] MAJOR — two new protection mechanisms, neither registered

- **Location:** `.omega/memory.db` → `protection_mechanisms` / `v_protection_surface`
- **Evidence:**
  ```
  cmd:$ test -f .omega/gauntlet.conf && echo GAUNTLET_CONF_PRESENT
  GAUNTLET_CONF_PRESENT
  cmd:$ sqlite3 .omega/memory.db "SELECT mechanism_id,name FROM protection_mechanisms
        WHERE name LIKE '%trust%' OR name LIKE '%maintainer%' OR name LIKE '%updat%';"
  (empty)
  ```
  Positive control: the same table returns PM-001 … PM-025 for other predicates, so the
  instrument works and the blank is a true zero.
- **Impact:** M1 adds (a) a fail-closed release-verification trust root and (b) a
  fatal-on-undecodable persisted security file. Both are protection mechanisms under the
  system-impact protocol, and `.omega/gauntlet.conf` is present, so registration is
  mandatory. The unrecorded interaction that matters: **(b) fires on node start, and the
  auto-updater is what restarts the node** — the same coupling that produced INC-I-153.
  [F4] is the concrete instance of that interaction, and it is exactly what the registry
  exists to surface before shipping rather than after.
- **Suggested fix:** register both with trigger condition, action, scale assumptions and
  `interacts-with` (each other, plus the auto-update restart path). Record the [F4]
  torn-write interaction explicitly. Run the gauntlet before commit and cite the run.
- **Test strategy:** NOT_TESTABLE (process artefact), but the gauntlet run is the evidence.
- **Confidence:** `conf(0.95, measured)`
- **Severity:** Major

### [F9] MINOR — the F6/§10 regression tests assert wiring order, not the security property

- **Location:** `bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs:65`;
  `bins/node/tests/inc_i_172_upgrade_cmd_verify_blocks_test.rs:89`;
  `bins/node/tests/inc_i_172_update_cmd_verify_blocks_test.rs:60`;
  `bins/node/tests/inc_i_172_service_timing_test.rs:97`
- **Evidence:** all four use `const SRC: &str = include_str!("../src/…")` and assert on
  source text (that the verification call and its blocking `?`/`return` appear before the
  `install_binary` call). None of them evaluates a signature against an artifact, so none
  can fail for [F1] — the verification call IS present and IS before the install; it is
  simply verifying the wrong thing. QA's PASS was earned against tests that are structurally
  incapable of detecting the defect.
- **Suggested fix:** keep the `include_str!` tests as cheap wiring guards, but add at least
  one behavioural test per operator path that feeds a mismatched `SIGNATURES.json` through
  the real verification helper and asserts refusal.
- **Test strategy:** see [F1] test strategy — the same test closes this.
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Minor

### [F10] MINOR — case-sensitive public-key comparison

- **Location:** `crates/updater/src/verification.rs:99`
- **Evidence:** `if &sig.public_key != expected_key { continue; }` is an exact `String`
  comparison. Root keys are lowercase (`PublicKey::to_hex()`, used at
  `bins/node/src/updater/trust_root_wiring.rs:60`, and the lowercase compiled arrays at
  `crates/updater/src/constants.rs:56-86`), while `sig.public_key` is free-form JSON text.
  An uppercase-hex entry is silently a non-match. Fail-closed, so not exploitable — but the
  operator sees `InsufficientSignatures` and cannot distinguish "wrong key" from "wrong
  case". The codebase already prefers the tolerant form elsewhere:
  `crates/updater/src/apply.rs:429` uses `eq_ignore_ascii_case`.
- **Suggested fix:** `sig.public_key.eq_ignore_ascii_case(expected_key)`.
- **Test strategy:** sign a release, uppercase the `public_key` field in the fixture,
  assert it still verifies.
- **Confidence:** `conf(0.85, observed)`
- **Severity:** Minor

### [F11] MINOR — the magic-aliasing justification cites the wrong magnitude

- **Location:** `crates/storage/src/maintainer.rs:27-28`
- **Evidence:** the comment says a legacy file could only begin with `DMST` if
  `set.members` had a length of "~5.7 billion". `DMST` = `0x44 0x4D 0x53 0x54`, which as the
  low half of bincode's little-endian `u64` length prefix is `0x54534D44` =
  **1,414,745,412** (≈1.41 billion). The security conclusion is unaffected — no reachable
  member count is anywhere near either number — but a load-bearing comment carries a wrong
  number, and code-is-SoT discipline applies to the justification too.
- **Suggested fix:** replace "~5.7 billion" with "~1.41 billion (0x54534D44)".
- **Test strategy:** NOT_TESTABLE (comment).
- **Confidence:** `conf(0.90, measured)`
- **Severity:** Minor

### [F12] MINOR — dead readers of the compiled keys and inert config survive

- **Location:** `crates/updater/src/constants.rs:126-132`; `crates/updater/src/lib.rs:67-72`;
  `crates/updater/src/params.rs` (`seniority_step_blocks`, `seniority_maturity_blocks`,
  `min_voting_age_blocks`)
- **Evidence:** `grep -rn "get_maintainer_keys" bins crates --include='*.rs'` returns only
  its definition and its `lib.rs` re-export — zero call sites. It is a ready-made second
  reader of the compiled keys (and the only remaining path to `test_maintainer_pubkeys`).
  §10 explicitly defers it to M2, so this is **recorded, not a blocker**. Separately, the
  three `UpdateParams` seniority fields are now inert after F8 deleted their only consumers;
  `.claude/skills/updater/SKILL.md:77` already says so, but the fields remain in the
  serialized struct.
- **Suggested fix:** M2 — delete `get_maintainer_keys` with the `crates/core` work; decide
  then whether the inert `UpdateParams` fields go too (they are serialized, so removing them
  is a config-format change).
- **Test strategy:** NOT_TESTABLE at M1 (deferred by contract).
- **Confidence:** `conf(0.90, observed)`
- **Severity:** Minor

### [F13] MINOR — a wiped or deleted `maintainer_state.bin` silently re-arms the leaked constants

- **Location:** `crates/storage/src/maintainer.rs:109-111` →
  `bins/node/src/updater/trust_root_wiring.rs:74-83`
- **Evidence:** a missing file returns `Ok(Self::default())`, i.e. `members = []` and
  `last_derived_height = 0`, which `resolve_trust_root` maps to `TrustRoot::bootstrap` at
  `debug!` level. This is contract-mandated (REQ-172-005) and correct for a genuinely fresh
  node — but it is **indistinguishable from a wiped one**, and this project's runbooks wipe
  data dirs routinely (`scripts/chain-reset.sh`, the cascade-recovery "full-wipe + snap"
  procedure). Until `maybe_bootstrap_maintainer_set` re-derives
  (`bins/node/src/node/periodic.rs:35-80`), the host verifies binaries against the exposed
  compiled keys, and the only trace is a `debug!` line.
- **Suggested fix:** no design change (the branch is required). Raise the Bootstrap-root
  selection from `debug!` to `warn!` naming the condition, surface it through the
  `trust_root` object already added to `getUpdateStatus`
  (`bins/node/src/node/startup.rs:477-483` — `provenance` is there but nothing alerts on
  it), and register a monitoring signal so "a mainnet producer is running on the Bootstrap
  root" is visible in Grafana rather than in a debug log.
- **Test strategy:** assert `resolve_trust_root(&MaintainerState::default(), Mainnet)`
  yields `Bootstrap`, and that `getUpdateStatus` reports `provenance: "Bootstrap"` — pin
  the observable so a future change cannot silence it.
- **Confidence:** `conf(0.85, observed)`
- **Severity:** Minor

## 8. Speculative findings (low-confidence, not actionable)

### [S1] `try_read()` contention denies updates to a legitimately-Bootstrap node

`bins/node/src/updater/trust_root_wiring.rs:106-119` returns an unusable **OnChain** root on
lock contention, even when the node's true resolution would be `Bootstrap`. A genuinely
fresh node that keeps losing the race against `maybe_bootstrap_maintainer_set` /
`apply_block::governance` write locks would never auto-update, and the error message would
name a root it does not have. The write locks are brief and the check retries every 6 h, so
I could not establish that this is materially harmful — but the returned value is a lie about
provenance, and `TrustRootProvenance::Unavailable` would be the honest third state.
`conf(0.50, inferred)`

## 9. Specs/docs drift summary

| File | Status |
|---|---|
| `specs/protocol.md:1829-1845` | **FALSE — [F5].** Untouched by the diff; still specifies the deleted fallback, the entry-count counter and `>= 3`. |
| `.claude/skills/updater/SKILL.md:471` | **FALSE — [F6].** Contradicts `:450` in the same file. |
| `.claude/skills/updater/SKILL.md:49,156` | Signature drift: `Ok(())` vs the code's `Ok(usize)`. |
| `docs/auto_update_system.md:246-248` | Accurate — the re-verification claim is correctly scoped to `UpdateService::auto_apply`. It does not cover `doli-node update apply`, and given [F2] that scoping is doing real work; leave it scoped. |
| `docs/auto_update_system.md:819` | Structurally accurate, semantically overstated while [F1] stands. |
| `docs/cli.md:1370`, `docs/architecture.md:407-408`, `crates/updater/src/constants.rs:12-18,36-40` | Correct — the "7-day seniority-weighted veto" claims are now accurate (configured period, head-count threshold). G4 satisfied. |
| `docs/architecture.md:405-408`, `.claude/skills/updater/SKILL.md` | Mixed INC-I-157 / INC-I-172 content — see [F7]. |
| `WHITEPAPER.md` | Correctly untouched. Its §15 governance text remains the out-of-scope cross-repo item the contract flagged for user decision. |
| `docs/DOCS.md`, `specs/SPECS.md` | Indexes updated for the new architecture spec. |

## 10. Residual risk on a ROLLING deploy (no synchronized stop)

1. **[F4] is the deploy-day risk.** Every node rewrites `maintainer_state.bin`
   non-atomically on its first boot after the upgrade, and a torn write is now fatal.
   Recommend fixing before deploy; it is three lines.
2. **[F1] means the operator escape hatch is not actually protected.** If the auto-update
   path refuses (correctly) during the rollout, operators will use `doli upgrade` /
   `doli-node upgrade`, which today print a reassuring "Verified: N distinct maintainer
   signature(s)" for a release whose signatures prove nothing about the artifact.
3. **Mixed-version fleet behaviour is benign.** M1 changes no block content, no consensus
   rule, no wire format and no gossiped structure; old and new binaries interoperate. The
   `first_notified_at` change is node-local, so a mixed fleet simply has per-node veto
   windows — which is what the change intends.
4. **`pending_update.json` compatibility is fine.** JSON + serde default behaviour ignores
   the removed `producer_weights` field.
5. **Recommend, before commit:** register the two protection mechanisms ([F8]), run the
   gauntlet, and land INC-I-157 as its own commit first ([F7]).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — the F1/F3 fixes add two string comparisons and one file read per operator-initiated upgrade, a human-triggered path that runs at most a few times per release; the F4 fix replaces one write() with write()+fsync()+rename() on a path executed at most once per maintainer-set change)
  Memory:   0 (observed — no new long-lived allocation; the F3 fix loads a 232-byte file into a short-lived buffer)
  IO:       +2 syscalls per maintainer_state save (observed — fsync + rename added by the F4 fix, on a write that occurs on migration and on maintainer-set changes only, not on any block path)
  Network:  0 (observed — the F1 fix compares fields already downloaded; no additional request)
  Disk:     0 (observed — same bytes written; the F4 fix writes them to a temp path first, transiently doubling a 232-byte file)
  Latency:  0 (observed — no change to block production, validation, gossip, sync or RPC; the affected paths are node startup and operator-initiated upgrade)
Inevitability: AVOIDABLE
Cheaper alternative: leave the operator gates as they are and rely on the auto-update path alone, which already binds the signature to the artifact correctly
Why this proposal anyway: the cheaper path ships a control that does not exist — M1 would claim REQ-172-006's "explicit trust root on the operator-facing path" while both operator commands accept a replayed signature over an unrelated release, which is the exact A5 anti-pattern F8 deletes dead machinery to prevent; the measurable advantage is that a compromised release origin can no longer install an arbitrary binary through the two commands operators reach for first
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## 11. Verdicts

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: signature-verification trust root (`crates/updater/src/{trust_root,verification}.rs`); four binary-install authorization paths (`bins/cli/src/cmd_upgrade.rs`, `bins/node/src/commands/{misc,update}.rs`, `bins/node/src/updater/service.rs`) plus a fifth found unguarded (`crates/updater/src/apply.rs`); external data ingestion (`SIGNATURES.json`, `CHECKSUMS.txt`, GitHub release metadata parsed from an attacker-reachable origin); a persisted security-relevant file format with a migration decoder (`crates/storage/src/maintainer.rs`); enforcement/deploy surface (the code that decides which binaries every node in the fleet will install)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Justification: this milestone rewrites the trust root for binary installation across an
auto-updating fleet, adds a new on-disk format with a migration decoder that must parse
attacker-influenceable-at-rest bytes, and converts three advisory checks into install
gates. Every one of the six standard signal rows is hit, and the seventh — enforcement and
deploy surface — is hit most directly of all: the artifact under review IS the mechanism
that authorises what every node runs. A defect here is not a bug in a feature, it is a
bug in the thing that decides which code exists. The review already found one CRITICAL
(F1) and two MAJOR (F2, F3) authorization defects that a green build, green clippy and a
PASS QA report all missed, which is itself evidence that the single-reviewer pass is not
sufficient coverage for this change. The full 5-auditor sweep must run after the findings
are addressed.

**REVIEW VERDICT: CHANGES REQUIRED**

Blocking: [F1] (Critical), [F2], [F3], [F4], [F7], [F8].
Non-blocking but should land with the fix: [F5], [F6], [F9], [F10], [F11], [F13].
Deferred by contract, correctly: [F12].

The engineering that IS here is good — the `TrustRoot` type, the fail-closed
`is_usable()` reasoning, the contention path, the `published_at` removal in full
(including the display side), the lossless migration with a real byte-parity test, and
the F8 deletions verified clean by a workspace-wide `--all-targets` clippy. The milestone
fails on completeness, not on craft: it closed the auto-update door correctly and left
two operator doors open while printing a message that says all four are shut.
