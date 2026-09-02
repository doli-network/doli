# INC-I-202 — Release signing: producer gap (Part 2 milestone plan)

Root cause (see `docs/.workflow/diagnosis-report.md`): the release pipeline enforces
signatures but never produces them. CI writes `"signatures": []` and publishes the release
as public `Latest` immediately; the only producer is an unenforced manual script whose key
paths are pre-rotation (INC-I-175) and whose result parsing is broken (INC-I-202 runtime
evidence, DEFECT 2).

Part 1 (done, not code): v6.26.2 signed by hand, 3/3 verified, manifest published.

TRIAGE VERDICT
Path: FAST (root cause already confirmed by the DEEP investigation; the remaining work is
three localized, independently testable changes)
Confidence: conf(0.9, measured - runtime evidence in docs/.workflow/runtime-evidence.md)
Reasoning: each milestone touches one file family, with a deterministic pass/fail test.

## M1 - Repair the signing producer (`scripts/sign-release.sh`)
REQ-202-001 (Must): default key paths resolve to the ROTATED maintainer wallets, not the
  leaked `producer_{1,2,3}.json` names. `KEY_DIR` stays overridable.
REQ-202-002 (Must): the collected signature survives a CLI that writes a status preamble to
  stdout. Extract the JSON object rather than assuming stdout is pure JSON.
REQ-202-003 (Should): fail loudly if a collected block is not valid JSON, naming the key.
Acceptance: with a stub `doli` that prints `Fetching ...` then the JSON, the script produces
a manifest with 3 entries and exits 0. With the current code, the same input fails.

## M2 - Close the publication gap (`.github/workflows/release.yml`)
REQ-202-004 (Must): a release is not promoted to public/`Latest` while its SIGNATURES.json
  has fewer than 3 valid signatures against the mainnet trust root. Private keys stay OUT
  of CI - the gate only verifies.
REQ-202-005 (Must): the verification logic is a reusable script, callable locally.
Acceptance: the verify script exits non-zero on a 0-entry manifest and zero on the real
v6.26.2 manifest (3 entries).

## M3 - Make the step unskippable in the runbook
REQ-202-006 (Must): `docs/releases.md` and `.claude/skills/release/SKILL.md` carry the
signing step as a BLOCKING item with the rotated key locations and the verify command.
Acceptance: both files name the signing + verification step; the skill file matches `sign`.

## M2.5 - Make a forgotten signature impossible to miss
REQ-202-007 (Must): the draft release states, in its own notes and in the CI step summary,
  that it is a DRAFT reachable by nobody until `scripts/publish-release.sh <version>` runs.
  `publish-release.sh` STRIPS that banner on promotion - a published release carrying an
  "unsigned" banner is a new defect, not a leftover.
REQ-202-008 (Must): a standing monitor answers one predicate - the newest version tag has a
  PUBLISHED release whose SIGNATURES.json verifies at or above threshold - reusing
  `doli release verify --version`, never reimplementing crypto. Exit 0 when it holds; one
  actionable line naming the tag and the fix command when it does not.
Acceptance: with stubbed `gh`/`doli` and a temporary tagged git repo, the monitor exits 0 on
published+verified, non-zero naming the tag on draft and on sub-threshold; the promotion path
hands GitHub notes that contain neither banner marker but still contain the changelog.

Out of scope here (tracked, not built now): verifier diagnosability (UNSIGNED vs REJECTED).
The "latest release verifies 3/3" monitor moved INTO scope as M2.5/REQ-202-008.

## M2 traceability matrix (filled by the developer, run 540)

| Req | Test | Implementation Module |
|-----|------|-----------------------|
| REQ-202-004 | `scripts/test_publish_release.sh` S1-S5 (14 assertions); `cmd_release_verify_tests.rs` P1/P3/P4/P5/P6/P7 | `Create GitHub Release: draft: true` @ `.github/workflows/release.yml:584`; `publish-release.sh` @ `scripts/publish-release.sh` |
| REQ-202-005 | `updater::install_gate::tests::verify_release_manifest_refuses_a_zero_entry_manifest`; `cmd_release_verify_tests.rs` P2/P8/P9 | `verify_release_manifest` @ `crates/updater/src/install_gate.rs`; `verify_manifest_dir` @ `bins/cli/src/cmd_release_verify.rs`; `cmd_release_verify` @ `bins/cli/src/cmd_upgrade.rs` |

Note: the M2 acceptance text above names v6.26.2. The tests and fixtures are anchored to
v6.26.3 (both tags now carry a hand-signed 3-entry manifest); the criterion is unchanged in
substance.

## M3 traceability matrix (filled by the developer, run 540)

| Req | Test | Implementation Module |
|-----|------|-----------------------|
| REQ-202-006 | `scripts/test_release_docs.sh` S1-S7 (19 assertions) | `BLOCKING: Post-Tag Release Sequence (INC-I-202)` + `Rules` 8-10 @ `.claude/skills/release/SKILL.md`; steps 3-6 + `Release Checklist` + `Downloading Releases` note @ `docs/releases.md`; `sign-release.sh` / `publish-release.sh` / `monitor-release-signed.sh` entries + legacy demotion @ `scripts/README.md`; `| release |` manifest row + 6 keyword rows @ `.claude/skills/SKILLS-INDEX.md` |

Green evidence (run 540): `test_release_docs.sh` 19 PASS / 0 FAIL / exit 0 (was 10/9/exit 1).
Regression guards held: `test_sign_release.sh` 11/0, `test_publish_release.sh` 25/0,
`test_monitor_release_signed.sh` 11/0, `cargo test -p updater --lib` 32/0,
`cargo test -p doli-cli --lib` 9/0.

## M2.5 traceability matrix (test writer, run 540 — RED, no implementation yet)

| Req | Test | Implementation Module (not yet written) |
|-----|------|-------------------------------------------|
| REQ-202-007 | `scripts/test_publish_release.sh` S6-S9 (11 assertions: S6 4, S7 3, S8 2, S9 2) | banner block @ `.github/workflows/release.yml` (new); notes-strip step @ `scripts/publish-release.sh` (new) |
| REQ-202-008 | `scripts/test_monitor_release_signed.sh` S1-S5 (11 assertions, ALL FAIL — script absent) | `scripts/monitor-release-signed.sh` (new, does not exist yet) |

RED evidence (run 540): `test_monitor_release_signed.sh` 0 PASS / 11 FAIL / exit 1 (script
absent, every assertion guarded by `script_ran`). `test_publish_release.sh` 19 PASS / 6 FAIL /
exit 1 — the original 14 S1-S5 assertions stay green; of the 11 new M2.5 assertions, S8 (2/2)
and S9 (2/2) fail as mandated (no banner/summary code exists yet), S6's banner-stripping
checks (2 of 4: notes-file content assertions) fail because `publish-release.sh` never calls
`gh release view --json body` or passes `--notes-file` today, and S6's exit-code/promotion
checks plus all 3 of S7's checks pass because they assert properties the M2 gate already
guarantees (verified promotion happens; a failed verify already blocks any `gh release edit`)
— S7 is a regression guard against a future banner-stripping bug jumping ahead of the
verification gate, not a fact this milestone changes.

## GS-015 traceability matrix (developer)

| Req | Test | Implementation Module |
|-----|------|-----------------------|
| REQ-202-GS015 | `scripts/test_gauntlet_gs015.sh` S1-S10 (21 assertions) | `_gs015_assert` / `_gs015_release_check` / `_gs015_workflow_check` @ `scripts/gauntlet-gs015.sh`; source + `assert()` dispatch @ `scripts/gauntlet.sh`; scenario docs @ `scripts/README.md` |

GREEN evidence: `test_gauntlet_gs015.sh` 21 PASS / 0 FAIL / exit 0 (was 0/21, exit 1).
Baselines held: `test_sign_release.sh` 11/11, `test_publish_release.sh` 25/25,
`test_monitor_release_signed.sh` 11/11, `test_release_docs.sh` 19/19 — all exit 0.
The scenario delegates the newest-tag predicate to `scripts/monitor-release-signed.sh`
(REQ-202-008) rather than re-implementing it, behind a `gh`/`doli` preflight that SKIPs
so an offline host never reads as a release defect. `inj_tag()` needs no entry: every
branch already falls through to `*) obs`, which is correct for an observational scenario.
