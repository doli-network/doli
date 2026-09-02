━━━ FINDINGS — 12 total (P0:1 P1:2 P2:4 P3:5) ━━━

  [F1] P0 conf(0.99, measured) — scripts/gauntlet-seed.sql:1-92 + scripts/gauntlet.sh:658 — GS-015 has no row in `gauntlet_scenarios` and no INSERT in the version-controlled seed, so `_gs015_assert` is never dispatched, README:917 and gauntlet.sh:52-53 claim it "runs in the DEFAULT gate", and INC-I-202 stays trace-gate-blocked — evidence: `sqlite3 .omega/memory.db "SELECT scenario_id FROM gauntlet_scenarios"` → GS-001..GS-014, no GS-015; `grep -oE "GS-[0-9]{3}" scripts/gauntlet-seed.sql | sort -u` → GS-001..GS-009 only; `SELECT COUNT(*) FROM gauntlet_scenarios WHERE incident_ids LIKE '%INC-I-202%'` → 0, the exact query trace-gate.sh:201 uses for SCENARIO_COUNT
  [F2] P1 conf(0.95, measured) — scripts/gauntlet-gs015.sh:38-53 — preflight omits `jq`; with jq unavailable the monitor's `IS_DRAFT` fallback (monitor:56) yields `true` and GS-015 FAILs a healthy release with "release is still a DRAFT" — evidence: sandbox probe A returned `rc=1 FAIL=[...UNHEALTHY v6.26.3: release is still a DRAFT...]` vs control `rc=0 INFO=[...HEALTHY v6.26.3...]`
  [F3] P1 conf(0.93, measured) — scripts/gauntlet-gs015.sh:26-30 — `_gs015_doli_resolvable` accepts any non-empty `DOLI_CLI` without an `-x` test, so a stale/typo path FAILs with "signatures are missing or sub-threshold" — the literal INC-I-202 symptom — instead of SKIPping — evidence: probe D with `DOLI_CLI=/nonexistent/path/doli` returned `rc=1 FAIL=[...'doli release verify' failed — signatures are missing or sub-threshold...]`
  [F4] P2 conf(0.93, measured) — scripts/gauntlet-gs015.sh:38-53 — preflight omits `git` and any tag-presence check; a tarball/tagless checkout FAILs with "no v* tag found" rather than SKIPping — evidence: probes C and E returned `rc=1 FAIL=[...UNHEALTHY: no v* tag found in <dir> — nothing to monitor.]`
  [F5] P2 conf(0.90, observed) — scripts/gauntlet.sh:642-646 vs 652-655,675 — `SKIP_REASONS` is printed to stdout only; the PASS branch appends nothing to `FAILURES_JSON`, so the durable `gauntlet_runs` row a fully-skipped GS-015 produces is byte-identical to a genuinely-green one, and `gauntlet-gate.sh` reads that row
  [F6] P2 conf(0.92, observed) — scripts/test_gauntlet_gs015.sh:9-10,33,295-300 — the read-only guarantee (O5) is declared "must be empty in every partition" but asserted in exactly one (S1); it is a stub-log artifact with no counterpart in `gauntlet-gs015.sh`, which records nothing
  [F7] P2 conf(0.88, observed) — scripts/gauntlet-gs015.sh:76 — the `draft: true` regex is file-global, not anchored to the `Create GitHub Release` step, so a `draft: true` under any other job/step in release.yml satisfies the gate after the real one at release.yml:592 is reverted
  [F8] P2 conf(0.95, observed) — scripts/README.md:896,917 — the GS-015 dependency text lists only `gh` + `doli` and asserts "SKIPs (never fails) without them"; the adjacent monitor-release-signed.sh row (README:845) correctly lists `jq` and `git`, which F2/F4 show are hard FAIL triggers
  [F9] P3 conf(0.90, observed) — scripts/test_gauntlet_gs015.sh:214-248 — `run_assert` runs under `set +e` with neither `-u` nor `pipefail`, so the runner's actual shell options (gauntlet.sh:67 `set -uo pipefail`) are never exercised, and no test covers gauntlet.sh dispatch or scenario registration — the blind spot that let F1 pass at 21/21
  [F10] P3 conf(0.99, observed) — scripts/test_gauntlet_gs015.sh:3,369 — the contract cites "gauntlet.sh:629" for the rc-0/rc-2 aggregation; post-diff that line is `FJ_FIRST=0` inside the waiver branch, the aggregation is at gauntlet.sh:639

  [F11] P3 conf(0.95, measured) — scripts/gauntlet-gs015.sh:31 — `[ -x "$DOLI_CLI" ]` is true for a DIRECTORY, so `DOLI_CLI=/some/dir` still reaches the monitor and FAILs with "signatures are missing or sub-threshold" — the residual of F3, found during fix-verification — evidence: probe F3b returned `rc=1 FAIL=[...'doli release verify' failed — signatures are missing or sub-threshold...]` while F3's own case (nonexistent path) correctly returned rc 2
  [F12] P3 conf(0.95, measured) — scripts/gauntlet-gs015.sh:89-97 — `_gs015_release_step_block` concatenates ALL softprops step blocks, so the draft check is "ANY release-creation step drafts" where the correct predicate is "EVERY" — the residual of F7 — evidence: a fixture with a drafting nightly softprops step and `draft: false` on the real Create Release step returned rc 0

  Round 1 fix-verification: F1 F2 F3 F4 F7 F8 F10 VERIFIED (measured); F5 F6 F9 deferred by agreement; F11 F12 are new non-blocking residuals.
  Speculative: 0
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-202 GS-015 gauntlet scenario (pre-commit)

Branch `fix/inc-i-202-gs015-release-signature`. Shell-script only; no Rust, no consensus code touched.

## Scope Reviewed

| File | State | Reviewed |
|---|---|---|
| `scripts/gauntlet-gs015.sh` | new, 92 lines | full |
| `scripts/test_gauntlet_gs015.sh` | new, 488 lines | full |
| `scripts/gauntlet.sh` | modified +13 | diff + aggregation block (lines 60-80, 570-690) |
| `scripts/README.md` | modified +1/-1 | diff + surrounding table (lines 838-920) |
| `docs/bugfixes/inc-i-202-analysis.md` | modified +14 | diff |

Read for corroboration, not reviewed as changes: `scripts/monitor-release-signed.sh`, `scripts/gauntlet-seed.sql`, `.github/workflows/release.yml:575-600`, `.omega/gauntlet.conf`, `.claude/hooks/gauntlet-gate.sh`, `bins/cli/src/cmd_upgrade.rs:66-350`, `crates/storage/src/maintainer.rs:120-125`.

Explicitly out of scope and not read as changes: `CLAUDE.md`, `specs/*`, `docs/DOCS.md`, all other untracked files.

## Summary

**❌ Requires changes.** The read-only guarantee (focus 1) is sound and I confirmed it down to the Rust layer. The rest does not hold: the scenario is **inert** — no `gauntlet_scenarios` row exists and the version-controlled seed does not create one, so `_gs015_assert` is never reached by the runner, while two committed documents state it is part of the default run. Separately, the preflight covers 2 of the 4 runtime dependencies of the script it delegates to, and I measured three distinct environment conditions that produce a **false FAIL whose text names the exact INC-I-202 symptom** — the precise failure the preflight comment (gauntlet-gs015.sh:34-37) says it exists to prevent.

## Focus-Area Verdicts

### 1. Read-only guarantee — HOLDS (positive finding)

I enumerated every externally-observable call reachable from `_gs015_assert`:

| Call | Site | Mutating? |
|---|---|---|
| `command -v gh` / `command -v doli` | gs015:29,38 | no |
| `gh auth status` | gs015:42 | no |
| `gh release view "$TAG" --repo --json isDraft` | monitor:49 | no |
| `git -C "$REPO_DIR" tag --list 'v*' --sort=-v:refname` | monitor:41 | no |
| `jq -r '.isDraft'` | monitor:56 | no |
| `"$DOLI" release verify --version "$TAG"` | monitor:62 | no — see below |
| `grep -Eq` on `$GS015_WORKFLOW` | gs015:76 | no |

`doli release verify` resolves to `cmd_release_verify` (`bins/cli/src/cmd_upgrade.rs:307-350`): it calls `resolve_upgrade_trust_root` → `storage::MaintainerState::load` which is a plain `std::fs::read` (`crates/storage/src/maintainer.rs:120-125`, no RocksDB open, no `save`), then two HTTPS GETs (`fetch_github_release`, `download_signatures_json`) held in memory. No filesystem write, no chain write, no node RPC, no `gh` mutation verb. **There is no path from GS-015 to a release, the chain, a node, or repo state.**

Caveat on "enforces vs records" (F6): the O5 log is a *test-harness* construct. The stubs at test:107-146 append to `$GH_LOG`/`$DOLI_LOG`; `gauntlet-gs015.sh` records nothing and there is no wrapper, allowlist or audit trail in production. The property is guaranteed by code shape and one test partition, not by an enforcement mechanism. That is acceptable for a 92-line library, but the contract header's claim (test:9-10) that O5 "must be empty in every partition" is stronger than what is asserted (MATRIX at test:33 assigns O5 to S6 only, and S6 reuses the S1 sandbox at test:295 — the FAIL partitions S2/S3, where an "auto-remediate" regression would land, are never checked).

### 2. SKIP vs PASS semantics — PARTIALLY HOLDS

Confirmed correct: `SKIP_REASONS` is the only skip signal, rc 2 is returned on every preflight refusal (gs015:40,44,48,52,71), and gauntlet.sh **does** surface it — line 645 prints a yellow `skip:` line inside the PASS branch. Both `assert()` (gauntlet.sh:597) and the aggregation (`{ rc = 0 || rc = 2 }` at 639) treat 0 and 2 alike, exactly as the contract states.

What does not hold (F5): the surfacing is **stdout-only**. The PASS branch (642-646) appends nothing to `FAILURES_JSON`; only failures (655) and waivers (630) are persisted. The result row written at 675 therefore records `status='pass'` with an empty failures array whether GS-015 checked the release or skipped both assertions. `.omega/gauntlet.conf` arms `gauntlet-gate.sh` to require "a fresh gauntlet_runs pass row" — it reads the row, not the terminal. A permanently-skipping GS-015 satisfies that gate forever, with no durable trace. This is a pre-existing runner property (GS-001 already uses rc 2), but GS-015 materially widens it: skipping is the *default* state on any host without an authenticated `gh`, and it silences the whole release assertion rather than one narrow sub-case.

### 3. Integration into gauntlet.sh — CLEAN (mechanically), INERT (functionally)

Mechanically correct and consistent with GS-009/010/014:
- `ROOT` is defined at gauntlet.sh:69, before the `GS015_LIB` assignment at 78 — no ordering bug.
- `[ -f "$GS015_LIB" ] && . "$GS015_LIB"` matches the existing guard pattern; the added `# shellcheck source=/dev/null` is a (harmless, if inconsistent) improvement over the GS-009/010/014 lines.
- Token dispatch (592-593) is placed before the `*)` unknown-token fallback at 594 — a mis-ordering would have swallowed it.
- **No symbol collisions.** `_GS015_ROOT`, `GS015_{REPO_DIR,MONITOR,WORKFLOW}`, `_gs015_{doli_resolvable,release_check,workflow_check,assert}` each appear only in `gauntlet-gs015.sh` (plus the one dispatch reference to `_gs015_assert`). No existing GS-001..GS-014 behavior is touched.
- **`set -uo pipefail` interaction is safe** and I verified it by execution, not inspection. Note gauntlet.sh:67 is `set -uo pipefail`, **not** `set -euo pipefail`. That matters: `msg="$(printf ... | grep 'HEALTHY' | tail -1)"` (gs015:56) returns non-zero under `pipefail` when the monitor emits no HEALTHY line, and would abort the whole runner under `-e`. It does not, because `-e` is absent. All three globals are initialized at gauntlet.sh:634 before `assert` is called, and every source-time expansion in gs015:20-23 uses `${VAR:-default}`, so `-u` is satisfied. My probes ran the library under an explicit `set -uo pipefail` and behaved correctly.

Functionally inert — see F1 below.

### 4. Preflight / monitor resolution-order parity — CLAIM IS ACCURATE

Header comment gs015:25 claims parity with `monitor-release-signed.sh:28-38`. Verified byte-for-byte in semantics:

| Step | monitor:28-38 | gs015:26-30 |
|---|---|---|
| 1 | `DOLI="${DOLI_CLI:-}"`, used if non-empty | `[ -n "${DOLI_CLI:-}" ] && return 0` |
| 2 | `[[ -x "./target/release/doli" ]]` | `[ -x "./target/release/doli" ]` |
| 3 | `command -v doli` | `command -v doli` |

Both resolve `./target/release/doli` relative to CWD, and `_gs015_release_check` invokes the monitor without changing directory (gs015:54 overrides only `REPO_DIR`), so the two resolutions cannot diverge. **The claim is true.** The shared hole is F3: neither side `-x`-tests a set-but-invalid `DOLI_CLI`, so parity is preserved into a false FAIL.

### 5. README / docs accuracy — TWO DRIFTS

F1 (the "part of the DEFAULT run" / "runs in the DEFAULT gate" claim, README:917 and gauntlet.sh:52-53) and F8 (the dependency list). The `docs/bugfixes/inc-i-202-analysis.md` addendum is internally accurate: the traceability row, the 21/21 GREEN evidence and the `inj_tag()` justification (every unlisted scenario falls through to `*) obs`, gauntlet.sh:609/611) all check out. Its `Implementation Module` column names three files and omits the registration edge — a symptom of the same gap as F1, not a separate error.

## Findings

### P0

#### AUDIT-P0-001: GS-015 is registered nowhere and therefore never runs
- **Location:** `scripts/gauntlet-seed.sql:1-92` (missing INSERT); dispatch at `scripts/gauntlet.sh:592-593`; consumer at `scripts/gauntlet.sh:658`
- **Category:** broken-logic / false-assurance
- **Description:** The runner sources its scenario list from the DB: `SELECT scenario_id,name,assertions FROM gauntlet_scenarios WHERE status='active'` (gauntlet.sh:658). `assert()` is only ever called with tokens taken from that `assertions` column. There is no GS-015 row:
  ```
  $ sqlite3 .omega/memory.db "SELECT scenario_id FROM gauntlet_scenarios ORDER BY scenario_id;"
  GS-001 … GS-010, GS-011(archived), GS-012, GS-013, GS-014      # no GS-015
  $ grep -oE "GS-[0-9]{3}" scripts/gauntlet-seed.sql | sort -u
  GS-001 … GS-009                                                 # no GS-015 (nor 010/012/013/014)
  ```
  `.omega/` is gitignored (`.omega/gauntlet.conf` says so explicitly), and `scripts/gauntlet-seed.sql:3` declares itself "Source of truth for `gauntlet_scenarios`" — the file gauntlet.sh:178 tells the operator to apply. Nothing in the intended commit set creates the row, on this machine or any other. I found no auto-registration path (`grep -n "gauntlet_scenarios" scripts/gauntlet.sh scripts/gauntlet-seed.sql .claude/hooks/gauntlet-gate.sh` → reads and tamper-guards only, no INSERT outside the seed).

  A repo-wide sweep confirms it: the only `INSERT INTO gauntlet_scenarios` statements outside `scripts/gauntlet-seed.sql` are the guidance template printed by `.claude/hooks/trace-gate.sh:284` and the tamper-guards in `.claude/hooks/gauntlet-gate.sh`. Nothing auto-registers.

  **Second, independent confirmation — the commit does not achieve its stated purpose.** `.claude/hooks/trace-gate.sh:281-291` gates incident close on `SCENARIO_COUNT`, computed at `trace-gate.sh:201` as `SELECT COUNT(*) FROM gauntlet_scenarios WHERE status='active' AND incident_ids LIKE '%$INC_ID%'`. Run for this incident:
  ```
  $ sqlite3 .omega/memory.db "SELECT COUNT(*) FROM gauntlet_scenarios WHERE incident_ids LIKE '%INC-I-202%';"
  0
  ```
  No active scenario references INC-I-202 (GS-009 carries INC-I-175 and INC-I-196, not 202). The trace gate's own instruction is two-part and ordered — INSERT the row first, then "If the scenario is NEW, also teach `scripts/gauntlet.sh` to execute it" (trace-gate.sh:291). This commit performs step 2 and omits step 1, so INC-I-202 remains trace-gate-blocked after it lands — the exact condition the scenario was written to clear.
- **Impact:** The scenario never executes. Two committed documents assert the opposite — `scripts/README.md:917` ("part of the DEFAULT run") and `scripts/gauntlet.sh:52-53` ("runs in the DEFAULT gate, is NOT opt-in"). INC-I-202 would be closed against a release-signing guard that has never evaluated a single assertion, and every future gauntlet run would keep reporting `N/N` with GS-015 absent from the count entirely — not even a yellow skip line. This is the false-assurance shape the scenario itself was written to detect.
- **Suggested Fix:** Add an `INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status) VALUES ('GS-015','release-published-and-signed', …, 'INC-I-202', 'gs015-newest-release-published-and-signed,gs015-workflow-drafts-releases', …, 'active');` to `scripts/gauntlet-seed.sql`, apply it to `.omega/memory.db`, and re-run `bash scripts/gauntlet.sh` to prove GS-015 appears in the scenario list. While there, backfill the four other rows the seed has drifted from (GS-010, GS-012, GS-013, GS-014) or state in the seed header that it is no longer the source of truth — one or the other, not the current ambiguity. The row's `incident_ids` MUST contain `INC-I-202` or the trace gate stays shut regardless of registration. Populate `runner` as well: `gauntlet_scenarios` has gained `runner TEXT, expect_fail INTEGER`, and `.claude/scripts/gauntlet-run.sh:81` counts `runner IS NULL` rows as UNRUNNABLE — only GS-013 sets it today, so a new row that omits it is born already degraded on that second execution path. (That script is untracked and outside this review's scope; noted only so the new row is not stale on arrival.)
- **Test Strategy:** Add an S11 partition to `scripts/test_gauntlet_gs015.sh` that asserts registration, not behavior: `sqlite3 "$DB" "SELECT assertions FROM gauntlet_scenarios WHERE scenario_id='GS-015' AND status='active';"` returns both tokens, and every returned token routes to a non-`*)` arm of `assert()`. Prove RED by running it before the seed row is added.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (inferred — one extra SQLite row read per gauntlet run)
  Memory:   0 (inferred)
  IO:       0 (inferred)
  Network:  N-A (inferred — the network cost is GS-015 executing at all, already accounted in F2's block)
  Disk:     0 (inferred — one row, < 1 KB)
  Latency:  0 (inferred — registration itself adds no wall time)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS — the runner has no other scenario source.
Why this proposal anyway: without the row the entire commit is dead weight.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### P1

#### AUDIT-P1-001: absent `jq` turns a healthy published release into a FAIL that says "DRAFT"
- **Location:** `scripts/gauntlet-gs015.sh:38-53` (preflight) against `scripts/monitor-release-signed.sh:56-60`
- **Category:** bug / false-positive
- **Description:** The preflight checks `gh` on PATH, `gh auth status`, monitor readability and `doli` resolvability. It does not check `jq`, which the monitor needs at line 56. Monitor:56 is written fail-closed — `IS_DRAFT="$(jq -r '.isDraft' <<<"$DRAFT_JSON" 2>/dev/null)" || IS_DRAFT="true"` — so a missing `jq` (exit 127, stderr swallowed) is indistinguishable from a genuine draft, and line 57-59 emits `UNHEALTHY <tag>: release is still a DRAFT`. GS-015 renders that as a FAIL. Measured in a sandbox with `gh`/`doli` stubs and a real tagged git repo:
  ```
  === A: jq unavailable, gh+doli fine, release NOT draft ===
  rc=1
  FAIL=[; gs015-newest-release-published-and-signed: UNHEALTHY v6.26.3: release is still a DRAFT
        — unreachable by nodes and doli upgrade. Run scripts/publish-release.sh v6.26.3 to promote it.]
  === B: control, jq present ===
  rc=0
  INFO=[; gs015-newest-release-published-and-signed: HEALTHY v6.26.3: published and verified …]
  ```
  This is invisible on the author's machine only because macOS 15 ships `/usr/bin/jq` (`ls -l /usr/bin/jq` → present) and the test sandbox PATH is `$BIN_DIR:/usr/bin:/bin` (test:220). On a Linux runner or an older macOS, S1 would go red.
- **Impact:** A false FAIL on a healthy release, with diagnostic text that points the operator directly at the INC-I-202 root cause. Per the script's own comment (gs015:36-37), "One false FAIL is how a scenario earns a standing waiver and stops guarding anything" — this defect is that comment's own scenario.
- **Suggested Fix:** Add `if ! command -v jq >/dev/null 2>&1; then SKIP_REASONS=…; return 2; fi` to the preflight, alongside the `gh` check. Optionally harden `monitor-release-signed.sh:56` to distinguish "jq failed" from "isDraft is true", but the preflight is the fix that belongs in this commit.
- **Test Strategy:** New partition S11 in `test_gauntlet_gs015.sh`: `new_sandbox` with a `jq` shim on `$BIN_DIR` that `exit 127`s (or a PATH excluding jq), assert `RC -eq 2` and `R_SKIP` names `jq`, `R_FAIL` empty. It fails today with `RC=1`.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (measured — one `command -v jq` builtin lookup per GS-015 run)
  Memory:   0 (measured)
  IO:       0 (inferred — PATH lookup, no file read)
  Network:  -O(1) (inferred — on jq-less hosts it removes one `gh release view` round trip)
  Disk:     0 (inferred)
  Latency:  +<1ms (inferred — one builtin, once per gauntlet run)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS — the check must precede the monitor invocation to prevent the false FAIL.
Why this proposal anyway: it removes a network call on the failing path and costs one builtin on the passing one.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P1-002: a set-but-unresolvable `DOLI_CLI` FAILs with "signatures are missing or sub-threshold"
- **Location:** `scripts/gauntlet-gs015.sh:26-30`
- **Category:** bug / false-positive
- **Description:** `_gs015_doli_resolvable` returns 0 for any non-empty `DOLI_CLI` without testing executability. The monitor then reaches `if ! "$DOLI" release verify …` (monitor:62), the shell returns 127, and the monitor's own error branch prints its signature-specific diagnosis. Measured:
  ```
  === D: DOLI_CLI=/nonexistent/path/doli ===
  rc=1
  FAIL=[; gs015-newest-release-published-and-signed: UNHEALTHY v6.26.3: 'doli release verify' failed
        — signatures are missing or sub-threshold. Run scripts/sign-release.sh v6.26.3 to re-sign.]
  ```
- **Impact:** A stale `DOLI_CLI` in an operator's shell profile produces a red gauntlet whose message is verbatim the INC-I-202 symptom ("signatures are missing or sub-threshold"). An operator would go re-sign an already-correctly-signed release. Note the parity claim at gs015:25 is *satisfied* here — the monitor has the same hole — so parity alone does not make this correct.
- **Suggested Fix:** Change the first arm to `[ -n "${DOLI_CLI:-}" ] && { [ -x "$DOLI_CLI" ] && return 0; SKIP_REASONS="$SKIP_REASONS; $t: DOLI_CLI is set to '$DOLI_CLI' which is not executable"; return 2; }` — or, less invasively, `command -v "${DOLI_CLI}" >/dev/null 2>&1`. The header comment at gs015:25 must then be amended: the preflight is *stricter* than monitor:28-38, not identical.
- **Test Strategy:** New partition S12: `new_sandbox`, `export DOLI_CLI="$CASE_DIR/nope"`, assert `RC -eq 2` and `R_SKIP` names `DOLI_CLI`. Fails today with `RC=1` (measured above).

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (measured — one stat(2) via `test -x`)
  Memory:   0 (measured)
  IO:       +1 stat (measured — a single stat syscall, once per gauntlet run)
  Network:  -O(1) (inferred — skips the monitor's gh call on the failing path)
  Disk:     0 (inferred)
  Latency:  +<1ms (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS — resolvability cannot be decided without a stat.
Why this proposal anyway: one stat buys the difference between "your env is broken" and "go re-sign a good release".
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### P2

#### AUDIT-P2-001: a tagless or non-git `GS015_REPO_DIR` FAILs instead of skipping
- **Location:** `scripts/gauntlet-gs015.sh:38-53`; monitor at `scripts/monitor-release-signed.sh:41-45`
- **Category:** edge-case / false-positive
- **Description:** Neither `git` presence nor tag presence is preflighted. Measured:
  ```
  === C: repo has no v* tags (shallow clone / CI checkout without tags) ===   rc=1  FAIL=[… no v* tag found in …]
  === E: GS015_REPO_DIR is not a git repo at all ===                           rc=1  FAIL=[… no v* tag found in …]
  ```
  `actions/checkout` with the default `fetch-depth: 1` fetches no tags, so any CI invocation of the gauntlet hits case C.
- **Impact:** Same false-FAIL-to-waiver dynamic as P1-001, on a different trigger.
- **Suggested Fix:** Extend the preflight with `command -v git` and `[ -n "$(git -C "$GS015_REPO_DIR" tag --list 'v*' 2>/dev/null)" ]`, SKIPping with a reason that names the directory. "No tags here" is an environment fact, not a release defect; "tags exist but the newest has no release" remains a correct FAIL.
- **Test Strategy:** Partition S13: `build_git_repo "$REPO_DIR"` with no tag arguments, assert `RC -eq 2` and `R_SKIP` names the repo dir. Fails today with `RC=1` (measured above).

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (inferred — one `git tag --list` on a local repo, already run by the monitor moments later)
  Memory:   0 (inferred)
  IO:       +O(refs) (inferred — one packed-refs read, same as the monitor's existing call)
  Network:  -O(1) (inferred — skips a gh round trip when there is nothing to check)
  Disk:     0 (inferred)
  Latency:  +~5ms (inferred — one local git invocation)
Inevitability: AVOIDABLE
Cheaper alternative: let the monitor stay authoritative and instead pattern-match its "no v* tag found" output back to rc 2 in `_gs015_release_check` — zero extra process, but couples GS-015 to the monitor's message text.
Why this proposal anyway: an explicit preflight is readable and keeps the skip/fail decision in one place; the duplicated `git tag` is ~5ms once per gauntlet run.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P2-002: a fully-skipped GS-015 is durably indistinguishable from a green one
- **Location:** `scripts/gauntlet.sh:642-646` (PASS branch, no persistence) vs `652-655` (FAIL branch) and `675` (row write)
- **Category:** tech-debt / false-assurance
- **Description:** `SKIP_REASONS` reaches only stdout (line 645). `FAILURES_JSON` accumulates failures (655) and waivers (630); the PASS branch contributes nothing. The row written at 675 therefore carries `status='pass'`, `scenarios_passed = scenarios_run`, `failures='[…]'` with no skip record. `.omega/gauntlet.conf` states the gate requires "a fresh gauntlet_runs pass row" — the DB row, not the terminal transcript.
- **Impact:** On any host without an authenticated `gh` (the common CI and non-maintainer case), GS-015 skips both assertions, counts as passed, and leaves no durable evidence that the release was never checked. Over time the gauntlet reports a growing pass count that includes a guard that has never fired. This is a pre-existing runner property, but GS-015 is the first scenario for which skipping is the default rather than an edge case, so it converts a latent design gap into a standing one.
- **Suggested Fix:** In the PASS branch, when `SKIP_REASONS` is non-empty, append `{"scenario":"$sid","skipped":true,"reason":<json>}` to `FAILURES_JSON` (mirroring the waiver shape at 630) so the skip is queryable from `gauntlet_runs`. Do not change the pass/fail arithmetic — a skip must stay non-blocking; it must stop being invisible.
- **Test Strategy:** Run the runner with `gh` removed from PATH, then assert `sqlite3 .omega/memory.db "SELECT failures FROM gauntlet_runs ORDER BY id DESC LIMIT 1;"` contains `"scenario":"GS-015","skipped":true`. NOT_TESTABLE from `test_gauntlet_gs015.sh` as written — it tests the library in isolation and never invokes `gauntlet.sh`.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (inferred — one python3 json.dumps per skipped scenario, at most once per scenario)
  Memory:   +O(len(skip reasons)) (inferred — a few hundred bytes)
  IO:       0 (inferred)
  Network:  0 (inferred)
  Disk:     +O(bytes) (inferred — the gauntlet_runs.failures TEXT grows by the skip reasons, < 1 KB/run)
  Latency:  +~20ms (inferred — one extra python3 spawn per skipped scenario, matching the existing waiver path)
Inevitability: AVOIDABLE
Cheaper alternative: append the skip text to the existing failures string without a python3 spawn, accepting weaker JSON escaping.
Why this proposal anyway: reusing the waiver path's exact escaping avoids a second, differently-broken quoting rule in the same field; ~20ms on a 50-90s run.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P2-003: the read-only contract (O5) is asserted in one partition, not "every partition"
- **Location:** `scripts/test_gauntlet_gs015.sh:9-10` (claim), `:33` (MATRIX), `:295-300` (the single assertion)
- **Category:** code-quality / test-gap
- **Description:** The contract header says the gh/doli log "must be empty in every partition"; the MATRIX assigns O5 to S6 alone, and S6 re-reads the S1 sandbox's logs without a fresh `new_sandbox`. S2 (draft), S3 (sub-threshold) and S7/S8 (workflow) never check for mutation. The implementation itself records nothing — `mutated()` (test:250-253) only sees what the stubs write.
- **Impact:** The FAIL partitions are exactly where a future "while we're here, promote the draft" regression would be added, and they are unguarded. Additionally `mutated()`'s pattern `release (edit|upload|delete|create)` misses `gh release delete-asset`, `gh api -X POST|PATCH|DELETE`, and `gh workflow run`.
- **Suggested Fix:** Hoist the `mutated` assertion into `run_assert` (or a `assert_read_only "$label"` helper) called after every partition, and broaden the pattern to `(release (edit|upload|delete|delete-asset|create)|api .*-X *(POST|PATCH|PUT|DELETE)|workflow run)`.
- **Test Strategy:** Self-testing — after the change, temporarily add `gh release edit x` to `_gs015_workflow_check` and confirm S7/S8 go red; revert.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (inferred — one extra grep per test partition, test-time only)
  Memory:   0 (inferred)
  IO:       +O(log size) (inferred — two small file reads per partition, test-time only)
  Network:  0 (inferred)
  Disk:     0 (inferred)
  Latency:  +~50ms total (inferred — 9 extra greps in a suite that already runs in seconds)
Inevitability: INEVITABLE
Cheaper alternative: NONE-NEEDED — cost is test-time only and does not touch production.
Why this proposal anyway: the header already promises this coverage; the fix aligns the assertion with the promise.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P2-004: the `draft: true` grep is file-global, not anchored to the Create Release step
- **Location:** `scripts/gauntlet-gs015.sh:76`
- **Category:** edge-case
- **Description:** `grep -Eq '^[[:space:]]*draft:[[:space:]]*true[[:space:]]*(#.*)?$'` correctly rejects a commented-out `# draft: true` and correctly ignores the `prerelease:` neighbour — I verified `grep -n -E '^[[:space:]]*draft:' .github/workflows/release.yml` returns exactly one hit, line 592, matching the header comment. But the match is not tied to the `Create GitHub Release` step (release.yml:585-596). A future workflow that adds a second release-creating job (nightly, RC, mirror) with `draft: true` would let a revert of line 592 pass unnoticed.
- **Impact:** Today: none, single occurrence verified. Latent: the one gate keeping unsigned CI artifacts unreachable could be reverted while GS-015 stays green.
- **Suggested Fix:** Either assert the count (`[ "$(grep -cE '^[[:space:]]*draft:' "$GS015_WORKFLOW")" = 1 ]` plus the true-check, FAILing on any additional `draft:` key as "unreviewed second release path"), or scope the match to the `softprops/action-gh-release` block with `awk`. The count assertion is the smaller change and fails loudly on the exact structural drift that would defeat the current check.
- **Test Strategy:** Partition S14: `write_workflow` variant emitting two jobs — one with `draft: false` at the `softprops` step and one with `draft: true` in an unrelated job. Assert `RC -eq 1`. Passes vacuously today.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (inferred — one extra grep over a ~600-line YAML file)
  Memory:   0 (inferred)
  IO:       +1 file read (inferred — the file is already read once; a second pass adds one ~25 KB read)
  Network:  0 (inferred)
  Disk:     0 (inferred)
  Latency:  +<5ms (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: a single `grep -c` whose result feeds both the count check and the true check, avoiding the second pass.
Why this proposal anyway: at ~25 KB and once per gauntlet run, a second pass is not worth the added shell branching; take the cheaper alternative only if it stays readable.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### P3

#### AUDIT-P3-001: README dependency text is incomplete and contradicts measured behavior
- **Location:** `scripts/README.md:896` and `:917`
- **Description:** Both lines name only `gh` and `doli` and assert GS-015 "SKIPs (never fails) without them". The adjacent `monitor-release-signed.sh` row (README:845) correctly lists `gh` CLI (authenticated), `jq`, `doli` CLI, `git` — GS-015 delegates to that script and therefore inherits all four, but preflights two.
- **Suggested Fix:** After P1-001/P2-001 land, restate as: "Optional: an authenticated `gh`, `jq`, `git`, and the `doli` CLI — GS-015 needs all four and SKIPs (never fails) without any of them." Do not update the text before the code, or the docs will assert a property the code does not have.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (inferred)   Memory:  0 (inferred)   IO:  0 (inferred)
  Network:  0 (inferred)   Disk:    0 (inferred)   Latency: 0 (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-NEEDED — documentation-only edit, no runtime effect.
Why this proposal anyway: the current text states a guarantee the code does not provide.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P3-002: tests do not reproduce the runner's shell options or its dispatch path
- **Location:** `scripts/test_gauntlet_gs015.sh:214-248`
- **Description:** `run_assert` executes under `set +e` with neither `-u` nor `-o pipefail`, while the real caller runs `set -uo pipefail` (gauntlet.sh:67). No test sources `gauntlet.sh` or calls `assert`, so neither the dispatch arm (gauntlet.sh:592-593) nor scenario registration is covered — which is precisely why P0-001 survived a 21/21 green run. I verified by direct execution that the library behaves correctly under `set -uo pipefail`, so this is a fidelity gap and not a live defect.
- **Suggested Fix:** Add `set -uo pipefail` inside the `run_assert` subshell (after `set +e`), and add the registration/dispatch partition described in P0-001's Test Strategy.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (inferred — shell option flags, test-time only)
  Memory:   0 (inferred)   IO:  +1 sqlite3 query (inferred, test-time)
  Network:  0 (inferred)   Disk: 0 (inferred)   Latency: +~10ms (inferred, test-time)
Inevitability: INEVITABLE
Cheaper alternative: NONE-NEEDED — test-time only, no production path.
Why this proposal anyway: the current suite cannot fail on the defect that matters most (P0-001).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### AUDIT-P3-003: stale line citation for the aggregation point
- **Location:** `scripts/test_gauntlet_gs015.sh:3` and `:369`
- **Description:** Both cite "gauntlet.sh:629" as where rc 0 and rc 2 are treated alike. Post-diff, line 629 is `FJ_FIRST=0` inside the waiver branch; the aggregation is `{ [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0` at gauntlet.sh:639. The +13-line diff this commit adds to gauntlet.sh is itself what moved it.
- **Suggested Fix:** Update both citations to `gauntlet.sh:639`, or cite the construct by name rather than by line number so the reference survives the next edit.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (inferred)   Memory: 0 (inferred)   IO: 0 (inferred)
  Network:  0 (inferred)   Disk:   0 (inferred)   Latency: 0 (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-NEEDED — comment-only edit.
Why this proposal anyway: a line citation that points at the wrong construct misleads the next reader of the contract.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Specs/Docs Drift

- `scripts/README.md:917` — "part of the DEFAULT run" is false as committed (P0-001).
- `scripts/gauntlet.sh:52-53` — "runs in the DEFAULT gate, is NOT opt-in" is false as committed (P0-001).
- `scripts/README.md:896,917` — dependency list omits `jq` and `git` and overstates the SKIP guarantee (P3-001).
- `scripts/gauntlet-seed.sql:3` — declares itself "Source of truth for `gauntlet_scenarios`" but is five scenarios behind the live DB (GS-010, GS-012, GS-013, GS-014 missing, plus GS-015). Pre-existing; P0-001 makes it load-bearing.
- `docs/bugfixes/inc-i-202-analysis.md:95-107` — accurate as written; its `Implementation Module` column omits the registration edge, mirroring P0-001.
- `specs/SPECS.md` / `docs/DOCS.md` — out of scope per the review brief; not checked.

## System Impact

GS-015 is not a protection mechanism under `.claude/protocols/system-impact.md`: it constrains no system dynamic, has no threshold, no trigger on live traffic, and no action on the running system. It is an observational assertion. No `protection_mechanisms` row is required and none is missing. I queried `v_protection_surface` (14 active mechanisms, all consensus/production-path) and found no trigger surface GS-015 could share — it never touches the chain, a node, or the mempool. No feedback loop, no starvation path, no scale-sensitive constant.

## Injection Pattern Scan

Scanned the two new shell files for shell/command injection. Findings: none blocking. Every external invocation uses quoted expansions (`bash "$GS015_MONITOR"`, `grep -Eq '…' "$GS015_WORKFLOW"`); there is no `eval`, no `sh -c`, no unquoted expansion into a command position, and no SQL. The one trust note: `GS015_MONITOR` is an env-overridable path executed via `bash`, so an operator who exports it controls what runs — this is the same trust model as `DOLI_CLI` in `monitor-release-signed.sh` and every other `GS0**_LIB` in the runner, and the env is the operator's own. Not a finding.

## Modules Not Reviewed

None within scope. `scripts/monitor-release-signed.sh` and `bins/cli/src/cmd_upgrade.rs` were read as dependencies of the read-only proof, not audited as changes (they are already committed and out of this commit set).

## Final Verdict

**REQUEST-CHANGES.** Blocking: **AUDIT-P0-001** (GS-015 is registered nowhere and never runs, while README:917 and gauntlet.sh:52-53 state it does — and with no `incident_ids` row, INC-I-202 stays trace-gate-blocked, so the commit does not accomplish what it was written for), **AUDIT-P1-001** (missing `jq` → false FAIL "release is still a DRAFT"), **AUDIT-P1-002** (unresolvable `DOLI_CLI` → false FAIL "signatures are missing or sub-threshold"). AUDIT-P2-001 should ship with the P1 fixes — it is the same preflight, the same root cause, and the same measured false-FAIL class.

The read-only design is correct and I verified it to the Rust layer; the integration into `gauntlet.sh` is mechanically clean; the preflight/monitor parity claim is accurate. Once the scenario is registered and the preflight covers all four of the monitor's dependencies, this is a good guard.

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: enforcement & deploy surface — the change adds gate logic to the gauntlet runner, the mechanism that decides what future commits are blocked on, and its subject is the release-signing trust path (INC-I-202, maintainer trust root, `doli release verify`). A guard that is documented as active while being inert (P0-001) is a false-assurance defect on the release-security surface, which is the class this rule exists to catch. Additionally: external data (GitHub release API, `.github/workflows/release.yml`) and subprocess construction from env-controlled paths.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

# Fix Verification — Round 1

Scope: `scripts/gauntlet-gs015.sh`, `scripts/test_gauntlet_gs015.sh`, `scripts/gauntlet-seed.sql`, `scripts/README.md`. F5/F6/F9 deferred by agreement; the seed's pre-existing GS-010..GS-014 drift is out of scope. Neither is re-opened below.

`bash scripts/test_gauntlet_gs015.sh` re-run by me: **35 passed, 0 failed, exit 0**. `shellcheck -S warning` clean on both new shell files. `scripts/gauntlet.sh` still shows exactly `13 insertions(+)` — the integration was not disturbed. Sizes: `gauntlet-gs015.sh` 133 lines, `test_gauntlet_gs015.sh` 719 lines — both inside budget.

I did not rely on the suite alone. Every verdict below is backed by a probe I ran against the fixed library outside the test harness, under `set -uo pipefail`, with real `git` repos and stub `gh`/`doli` on PATH.

## Per-finding verdicts

| ID | Verdict | Evidence |
|---|---|---|
| F1 | **VERIFIED** | Seed row added (`gauntlet-seed.sql:91-97`): both tokens, `runner='gauntlet.sh'`, `status='active'`, `incident_ids=json('["INC-I-202"]')`, GS-009-style upsert. Applied to a throwaway DB with real `sqlite3`: round-trips correctly, and applying it **twice** is idempotent (no constraint error). `SELECT COUNT(*) … WHERE status='active' AND incident_ids LIKE '%INC-I-202%'` → **1**, the trace-gate predicate from `trace-gate.sh:201`. S11 is a genuine registration test — it applies the real seed via real `sqlite3`, reads the `assertions` column back, splits it, and dispatches every token through `_gs015_assert`, failing if any lands on the unknown-token arm. That tests seed↔dispatch agreement, which is the actual invariant. |
| F2 | **VERIFIED** | `jq` added to the `for tool in jq git` preflight (gs015:56-61). Probe with a PATH mirror of `/bin`+`/usr/bin` minus `jq`: `rc=2 SKIP=[… jq not on PATH — release state unreadable here, not a release defect]`. Was `rc=1 FAIL=[… release is still a DRAFT …]` before the fix. |
| F3 | **VERIFIED** (residual F11) | `[ -x "$DOLI_CLI" ]` now required (gs015:31), with a distinct reason string via `_GS015_DOLI_WHY`. Probe `DOLI_CLI=/nonexistent/doli`: `rc=2 SKIP=[… DOLI_CLI is set to '/nonexistent/doli', which is not executable …]`. Was `rc=1 FAIL=[… signatures are missing or sub-threshold …]`. |
| F4 | **VERIFIED** | `git` added to the tool preflight, plus a tag-presence gate (gs015:72-75). Probes: git absent → `rc=2 SKIP=[… git not on PATH …]`; tagless repo → `rc=2 SKIP=[… no v* tag in <dir> (tagless or shallow checkout) …]`; non-git directory → same `rc=2`. All three were `rc=1` before. |
| F7 | **VERIFIED** (residual F12) | Draft check now reads inside the release-creation step block via `_gs015_release_step_block` (awk) + a here-string grep, which is `pipefail`-safe. Verified against the **real** `.github/workflows/release.yml`: the extractor returns exactly the 12-line `Create GitHub Release` step (585-596), stopping before `- name: Draft release reminder`, with `draft: true` inside → `rc=0`. Stray `draft: true` in an unrelated non-softprops job → `rc=1`. No softprops step at all → `rc=1` with "the draft gate cannot be read where it acts" — correctly a FAIL (repo fact), not a SKIP. |
| F8 | **VERIFIED** | README:896 now lists `gh`, `jq`, `git`, `doli` ("needs all four"); README:917 additionally names the `v*` tag requirement, the seed registration, and that the draft gate is read inside the release step. Both statements now match measured behavior. |
| F10 | **VERIFIED** | Both citations are `gauntlet.sh:639` (test:3, test:382). The two other line references added by the fix (`gauntlet.sh:658`, test:490 and test:544) are also correct — that is the `SELECT … FROM gauntlet_scenarios` line. |

## No-regression check (the coordinator's masking concern)

The risk in adding five SKIP branches is that a genuine release defect gets reclassified as SKIP. Probed directly — all preserved:

```
draft release          rc=1  FAIL=[… UNHEALTHY v6.26.3: release is still a DRAFT …]
no release for tag     rc=1  FAIL=[… UNHEALTHY v6.26.3: no GitHub release found …]
healthy published      rc=0  INFO=[… HEALTHY v6.26.3: published and verified …]
```

The preflight ordering is sound by construction: every SKIP branch tests a **host** fact (gh, gh auth, jq, git, monitor readable, doli executable) except the tag gate, which is a repo fact that cannot mask a FAIL-class fact — zero `v*` tags means there is no newest tag, so "no release for the newest tag", "it is a draft" and "it fails verify" are all unreachable. The developer drew the same line in `_gs015_workflow_check`: a missing workflow file SKIPs (host fact), a workflow with no release-creation step FAILs (repo fact). That distinction is correct and was made deliberately.

## New findings introduced by the fixes

Both are narrow residuals of the findings they close, both non-blocking.

#### AUDIT-P3-004 (F11): `-x` accepts a directory
- **Location:** `scripts/gauntlet-gs015.sh:31`
- **Description:** `[ -x "$DOLI_CLI" ]` is true for any directory with the execute bit, so `DOLI_CLI=/path/to/dir` passes the preflight, exits 126 at `"$DOLI" release verify`, and reproduces the exact message F3 was filed against. Measured: `rc=1 FAIL=[… 'doli release verify' failed — signatures are missing or sub-threshold …]`.
- **Impact:** Narrower than F3 — a stale *file* path is now handled; only a directory-valued `DOLI_CLI` slips through.
- **Suggested Fix:** `[ -x "$DOLI_CLI" ] && [ ! -d "$DOLI_CLI" ]`, or `command -v "$DOLI_CLI" >/dev/null 2>&1`.
- **Test Strategy:** Extend S13 with `export DOLI_CLI="$CASE_DIR"`; assert `RC -eq 2`.

#### AUDIT-P3-005 (F12): the draft predicate is ANY, not EVERY
- **Location:** `scripts/gauntlet-gs015.sh:89-97`
- **Description:** `_gs015_release_step_block` `printf`s every matching step block into one string, so the subsequent grep passes if *any* softprops step drafts. The correct predicate is that *every* release-creation step drafts — one that publishes non-draft is a hole regardless of what the others do. Measured with a two-softprops fixture (nightly step `draft: true`, real Create Release step `draft: false`): `rc=0`, i.e. green on a reverted gate.
- **Impact:** Requires someone to add a second `softprops/action-gh-release` step, so it is materially narrower than the original F7 (which any stray `draft:` key satisfied) and the added step is itself a reviewable event. Latent, not current — the real file has exactly one such step, which I verified.
- **Suggested Fix:** Emit a separator between blocks and require every block to match, or assert the softprops step count is exactly 1 and FAIL on more than one as "second, unreviewed release path".
- **Test Strategy:** The `two_softprops.yml` fixture above; assert `RC -eq 1`. Passes vacuously today.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (measured — F11 adds one `test -d`; F12 adds one awk counter, no extra pass)
  Memory:   0 (measured)
  IO:       0 (measured — both operate on data already read)
  Network:  0 (measured)
  Disk:     0 (measured)
  Latency:  +<1ms (measured — once per gauntlet run)
Inevitability: AVOIDABLE
Cheaper alternative: leave both as recorded residuals — neither is reachable in the current repo state.
Why this proposal anyway: each is a one-line change closing the last reachable path of a finding already agreed as real; deferring is also defensible.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Operational precondition (not a code finding)

`scripts/gauntlet-seed.sql` is the version-controlled fix and is correct. The live `.omega/memory.db` on this machine still has **no** GS-015 row (`SELECT … WHERE scenario_id='GS-015'` → empty), because `.omega/` is gitignored and per-machine. Until the seed is applied, this host's gauntlet still will not run GS-015 and INC-I-202's trace gate still reads 0:

```
sqlite3 .omega/memory.db < scripts/gauntlet-seed.sql
```

I did not run it — seeding the scenario table is not a reviewer action. Flagging it because F1's close depends on it and the code change alone does not accomplish it.

## Fix-Verification Verdict

**APPROVE.** All four blocking findings (F1, F2, F3, F4) and all three riders (F7, F8, F10) are verified against the code by measurement, not by test report alone. No regression in the FAIL or PASS classes. Two new P3 residuals (F11, F12) are recorded and are explicitly non-blocking — neither is reachable in the current repo state. F5/F6/F9 remain open by agreement.
