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

Out of scope here (tracked, not built now): verifier diagnosability (UNSIGNED vs REJECTED)
and the external "latest release verifies 3/3" monitor.
