# QA Report: INC-I-157 M1 — ORIGIN DE-PINNING

- **Incident**: INC-I-157
- **Milestone**: M1 (ORIGIN DE-PINNING)
- **Requirements**: REQ-I157-010 (origin pinned to a controlled namespace), REQ-I157-011 (no non-resolving hostname in the download fallback chain)
- **Branch**: `main` — working tree, nothing staged, nothing committed
- **Date**: 2026-08-08

---

## Summary

**PASS.**

Both requirements are met. The release origin is repointed from the unowned `e-weil`
namespace to the project-owned `doli-network` namespace across every shipped artifact
(Rust source, scripts, docker compose, specs, docs, Cargo.toml), and the NXDOMAIN
`FALLBACK_MIRROR` is fully removed — constant, both call sites, and the `lib.rs`
re-export. The highest-risk part of the change (the index→label mapping in
`download_binary`'s `urls_to_try` loop) was re-derived from first principles and is
**correct for both remaining entries with no off-by-one**. `fetch_latest_release()`
still returns `Ok(None)` on total failure — runtime-proven, not just read. No version
of any kind was bumped. `cargo check --workspace --all-targets` is clean and all 44
updater tests pass, including the 5 new `origin_pinning` tests.

Audit finding **NEW-4 is CLOSED**: `install.sh` and the updater now name the same
trust root.

Nothing blocking was found. Six non-blocking observations are recorded, of which two
matter: (a) `testnetlinux/explorer/*.html` was labelled a "runtime fixture" in the
exclusion list but is in fact **served by a systemd unit** and still links to the
unowned namespace; (b) the docker compose files now point at
`ghcr.io/doli-network/doli-node:latest`, an image that **has never been published** —
independently confirmed. Neither is a regression introduced by M1.

---

## System Entrypoint

This milestone touches a library crate and shipped metadata, not a runnable network
service, so validation was performed against the build system, the test suite, the
live GitHub/GHCR/DNS endpoints the code targets, and an out-of-repo harness binary.

| Purpose | Command |
|---|---|
| Compile gate (all consumers incl. tests) | `cargo check --workspace --all-targets` → **clean, 18.54s** |
| Test gate | `cargo test -p updater` → **36 lib + 3 apply_install_mode + 5 origin_pinning + 1 doctest, 0 failed** |
| Live origin reachability | `curl` against `api.github.com`, `github.com`, `ghcr.io` |
| Fallback host resolution | `dig` / `host` against `releases.doli.network` |
| Criterion-4 runtime proof | out-of-repo cargo harness depending on `crates/updater` by path (no repo edit) |

Note: the crate's cargo package name is `updater`, not `doli-updater`
(`cargo test -p doli-updater` errors with "did not match any packages").

---

## Traceability Matrix Status

| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-I157-010 | Must | Yes — `crates/updater/tests/origin_pinning.rs` P1/P2a/P2b/P3a/P3b + `test_no_unowned_namespace_literal_in_updater_source` | Yes | **Yes** | 3 constants pinned; source-scan test guards regression |
| REQ-I157-011 | Should | Yes — `test_no_nonresolving_fallback_mirror_in_updater_source` | Yes | **Yes** | Const + both call sites + re-export removed |

### Test-to-code binding verified

All 5 tests in `crates/updater/tests/origin_pinning.rs` execute and pass:

```
test test_github_repo_pinned_to_owned_namespace ... ok
test test_github_api_url_pinned_to_owned_namespace ... ok
test test_github_releases_url_pinned_to_owned_namespace ... ok
test test_no_unowned_namespace_literal_in_updater_source ... ok
test test_no_nonresolving_fallback_mirror_in_updater_source ... ok
```

The two source-scan tests are the durable guard: they assert on the *file contents* of
`crates/updater/src/**/*.rs`, so a future re-introduction of either literal fails CI
even if the constants themselves are renamed. The test file deliberately holds the
offending literals in its own constants (`UNOWNED_NAMESPACE`,
`NONRESOLVING_FALLBACK_HOST`) rather than importing `updater::FALLBACK_MIRROR` — which
is why it must scan `src/**` and not `tests/**`. That design is correct and is the only
reason `e-weil` / `releases.doli.network` legitimately survive in-repo.

### Gaps Found

- No test asserts that `download_binary`'s index→label mapping stays correct. This was
  the highest-risk element of the change and is currently guarded only by review.
  See OBS-005.
- No test asserts `fetch_latest_release` returns `Ok(None)` rather than `Err` on total
  source failure. Verified manually (runtime, below), but unguarded against regression.
  See OBS-006.

---

## Acceptance Criteria Results

### Criterion 1 — REQ-I157-010: no `e-weil` in any shipped artifact — **PASS**

Full-repo scan (`grep -rn "e-weil" .`, excluding `.git`/`target`). Every surviving
occurrence in the main working tree, classified:

| Location | Class | Operational? | Verdict |
|---|---|---|---|
| `crates/updater/tests/origin_pinning.rs:7,8,112` | Test negative-assertion literal | No — test-only | **Legitimate, required by the test design** |
| `docs/audits/security-audit-issue-174-2026-06-08.md:55` | Historical audit record (the NEW-4 row) | No | Legitimate — rewriting it would destroy the audit trail |
| `docs/bugfixes/inc-i-157-installer-integrity-analysis.md` (11 lines) | The investigation doc for this very incident | No | Legitimate — it documents the defect |
| `docs/legacy/implementation_distribution.md:175-176`, `docs/legacy/IMPLEMENTATION_PLAN_DISTRIBUTION.md:175-176` | Legacy plan checklists | No | Legitimate |
| `testnet/bin/{doli,doli-node}`, `testnetlinux/bin/{doli,doli-node}` | Git-tracked compiled binaries | **Partially** | See OBS-002 |
| `testnetlinux/explorer/index.html:583`, `network.html:219` | Labelled "runtime fixtures" in the exclusion list | **YES — served** | **See OBS-001 — the exclusion label is wrong** |
| `.claude/worktrees/**` | Separate git worktrees on other branches | No | Legitimate — not this branch |
| `.omega/memory.db` | Institutional memory (binary) | No | Legitimate |

**Zero occurrences** remain in the categories the requirement names. Positive
confirmation:

```
$ grep -rn "e-weil" docs/ specs/ scripts/ docker/ | grep -v docs/legacy/ | grep -v docs/audits/ | grep -v docs/bugfixes/
  NONE
```

Per-root confirmation that no Rust source outside the test file carries any hardcoded
GitHub host:

```
$ grep -rn "github\.com\|githubusercontent\|ghcr\.io" crates/ bins/ --include="*.rs"
crates/crypto/src/hash.rs:375        # BLAKE3 test-vector attribution comment — unrelated
crates/updater/tests/origin_pinning.rs:7,8   # test literals
crates/updater/src/constants.rs:135          # GITHUB_API_URL  (new namespace)
crates/updater/src/constants.rs:138          # GITHUB_RELEASES_URL (new namespace)
crates/updater/src/download.rs:378           # format!("https://api.github.com/repos/{}/...", crate::GITHUB_REPO)
```

`download.rs:378` builds from the constant, not a duplicate literal — so there is no
second, un-repointed URL construction path. This was an explicit exploratory target and
it comes back clean.

Live reachability, confirming the premise of the fix:

| URL | HTTP |
|---|---|
| `https://github.com/e-weil` | **404** (namespace genuinely unowned) |
| `https://api.github.com/users/e-weil` | **404** |
| `https://api.github.com/repos/doli-network/doli` | **200** |
| `https://api.github.com/repos/doli-network/doli/releases/latest` | **200** (tag `v6.24.1`) |
| `git remote -v` | `https://github.com/doli-network/doli.git` — matches the pinned constants |

### Criterion 2 — REQ-I157-011: no non-resolving hostname in the fallback chain — **PASS**

```
$ grep -rn "FALLBACK_MIRROR\|releases.doli.network" crates/ bins/ specs/ docs/
crates/updater/tests/origin_pinning.rs:14,23,26,48,68,102,116,267,269   # test doc comments + negative-assertion const
docs/legacy/bugs/REPORT_CONSENSUS.md:288                                # historical log excerpt
docs/bugfixes/inc-i-157-installer-integrity-analysis.md:71,177,343      # the investigation doc
```

Per-root breakdown (aggregate scans are not per-root facts):

| Root | `releases.doli.network` hits |
|---|---|
| `crates/` | 4 — all in `tests/origin_pinning.rs` (doc comments + `NONRESOLVING_FALLBACK_HOST`) |
| `bins/` | **0** |
| `scripts/` | **0** |
| `docker/` | **0** |

`specs/` and `docs/` hits are exclusively the two legitimately-historical documents
above. **No live code path references it.** Confirmed removed from all three sites:

- `crates/updater/src/constants.rs` — `pub const FALLBACK_MIRROR` deleted
- `crates/updater/src/download.rs` — deleted from the `use crate::{...}` import, from
  `download_binary`'s `urls_to_try.push(...)`, and from `fetch_latest_release`'s
  fallback block (whole 17-line block removed)
- `crates/updater/src/lib.rs:66` — removed from the `pub use constants::{...}` re-export

DNS confirmation: `host releases.doli.network` → **NXDOMAIN**. The removed name was
genuinely dangling.

### Criterion 3 — download loop index→label mapping — **PASS (highest-risk item, clean)**

`crates/updater/src/download.rs:29-57` after the change:

```rust
let mut urls_to_try = vec![url.clone()];        // index 0 = binary_url_template
urls_to_try.push(format!(                        // index 1 = GitHub Releases
    "{}/v{}/doli-node-{}",
    GITHUB_RELEASES_URL, release.version, platform
));

for (i, url) in urls_to_try.iter().enumerate() {
    let source = match i {
        0 => "primary",
        _ => "GitHub",
    };
```

**Derivation.** Before the change the vec had 3 elements and arms
`0 => "primary", 1 => "GitHub", _ => "fallback"`. The element removed was the one at
the **last** index (2). Removing a trailing element from an index-mapped vec cannot
renumber the surviving elements: index 0 stays 0, index 1 stays 1. The developer then
collapsed `1 => "GitHub", _ => "fallback"` into `_ => "GitHub"`. Since the only index
that can now reach the wildcard is 1, and 1 is exactly the GitHub entry, the mapping is
**correct for both remaining entries**.

**Counter-check (trying to disprove).** The off-by-one this refactor *could* have
produced is the mirror case: had the developer removed the *first* element instead,
index 0 would hold the GitHub URL while still being labelled `"primary"`. That bug is
**not** present — `urls_to_try` is still initialised from `release.binary_url_template`
(`vec![url.clone()]`), which is the primary. A second failure mode would be leaving a
stale `1 => "GitHub"` arm plus a `_ => "fallback"` arm, which would mislabel nothing
today but would silently mislabel any future third entry; that arm was correctly
deleted. A third would be leaving the `FALLBACK_MIRROR` import, which would fail
compilation — and the workspace check is clean, so it is gone.

**Blast-radius check.** `download_binary` has exactly one call site,
`crates/updater/src/apply.rs:127`, verified per-root (`bins/` → no match, with
`download_from_url` as positive control returning 3 hits in `bins/`, proving the scan
instrument works). The label is used only in `debug!`/`info!`/`warn!` output, so even a
mislabel would be a diagnostics defect, not a correctness one — but it is correct.

### Criterion 4 — `fetch_latest_release()` returns `Ok(None)` on total failure — **PASS (runtime-proven)**

Static reading confirms the trailing `Ok(None)` at `download.rs:146` is untouched, and
the custom-URL error branch at `download.rs:118-121` still returns `Ok(None)` rather
than propagating. The removed fallback block ended in a non-returning
`Err(e) => { warn!(...) }`, so deleting it does not change the function's terminal
path.

Proven at runtime with an out-of-repo harness linking `crates/updater` by path (no
repo files created or edited):

```
A custom_url-unreachable   => Ok(false)          # Ok(None): no Err, no panic
B github-live              => is_ok=true is_some=Ok(true)
   version=6.24.1 url_template=https://github.com/doli-network/doli/releases/download/v6.24.1/doli-node-{platform}
CRITERION-4: PASS (Ok(None) on failure, no panic, no Err)
```

Path A drives `custom_url` at an unresolvable host — returns `Ok(None)`. Path B
exercises the real GitHub path end-to-end and confirms the **new namespace resolves,
authenticates anonymously, and yields a parsed `Release`**. That is a positive control:
the `Ok(None)` in path A is meaningful precisely because path B shows the function *can*
return `Ok(Some(..))` under the same code.

### Criterion 5 — no version bumped anywhere — **PASS**

| Version | Value | Changed? |
|---|---|---|
| Workspace `Cargo.toml` `version` | `6.24.1` | **No** — `git diff Cargo.toml` shows only the `repository` line |
| `CURRENT_PROTOCOL_VERSION` (`crates/network/src/protocols/status.rs:49`) | `8` | **No** |
| `EPOCH_STATE_FORMAT_VERSION` (`status.rs:68`) | `1` | **No** |
| `MIN_PEER_PROTOCOL_VERSION` (`status.rs:83`) | `1` | **No** |

`git diff | grep "CURRENT_PROTOCOL_VERSION\|EPOCH_STATE_FORMAT_VERSION\|MIN_PEER_PROTOCOL_VERSION"`
returns nothing. The only `VERSION`-containing diff lines are inside a `docs/releases.md`
shell snippet (`VERSION=$(curl ...)`), which is a doc example, not a constant.

Correct call: this change alters no consensus rule and no block content, so no
activation height and no synchronized deploy are required. It is a compiled-in URL
change; old binaries keep using the GitHub rename-redirect, new ones use the correct
origin. No flag day.

### Criterion 6 — nothing staged or committed; unrelated work untouched — **PASS**

```
$ git diff --cached --stat
(empty)
$ git status --porcelain | grep -c "^ M"     -> 23
$ git status --porcelain | grep -c "^??"     -> 13
```

23 modified files, all **unstaged**. The 13 untracked items (the `docs/bugfixes/*`
analyses, `docs/reviews/*`, `docs/reports/`, `docs/announcements/`) are intact and
untouched.

One observation, not a defect of this change: repo `HEAD` moved from `e6d72577` (the
value in the task's opening snapshot) to `f2b66c19 Merge branch
'bugfix/inc-i-167-wallet-overwrite-guard'` during the session — concurrent work by
another actor. **None of the 23 INC-I-157 files are in that commit**; they remain
uncommitted in the working tree. Flagged so the developer is not surprised by a moved
base when committing.

---

## Adjacent-Breakage Checks

### Consumers of `GITHUB_RELEASES_URL` / `GITHUB_API_URL` / `GITHUB_REPO`

Complete consumer list (per-root scan over `crates/` and `bins/`):

| Site | Uses |
|---|---|
| `crates/updater/src/download.rs:35` | `GITHUB_RELEASES_URL` |
| `crates/updater/src/download.rs:187` | `GITHUB_API_URL` |
| `crates/updater/src/download.rs:300` | `GITHUB_RELEASES_URL` |
| `crates/updater/src/download.rs:379` | `GITHUB_REPO` |
| `crates/updater/src/download.rs:528, 551` | `GITHUB_RELEASES_URL` |
| `bins/node/src/updater/mod.rs:27` | re-exports `GITHUB_RELEASES_URL` |
| `bins/node/src/commands/misc.rs:57` | `updater::GITHUB_RELEASES_URL` |

Per-root note: `bins/cli/` contains **zero** direct references to these constants
(positive control: the same scan finds 16 `updater::` calls in `bins/cli/src/`, so the
instrument works). The CLI reaches the origin indirectly through
`updater::fetch_github_release()` and `updater::download_signatures_json()`, which
consume the constants internally — so it inherits the fix with no code change.

**Compilation**: `cargo check --workspace --all-targets` — clean. This covers
`bins/node`, `bins/cli`, `bins/gui`, all 11 crates, and both test targets.

### Constructed URLs — actually built and probed, not assumed

Strings produced from the new base `https://github.com/doli-network/doli/releases/download`
(no trailing slash) and `GITHUB_REPO = doli-network/doli`, for `version=6.24.1`,
`tag=v6.24.1`, `platform=linux-x64`:

| Site | Constructed URL | Live HTTP |
|---|---|---|
| `download.rs:33` (GitHub entry) | `https://github.com/doli-network/doli/releases/download/v6.24.1/doli-node-linux-x64` | **404** — see OBS-003 |
| `download.rs:298` (`binary_url_template`) | `https://github.com/doli-network/doli/releases/download/v6.24.1/doli-node-{platform}` | (same shape) |
| `download.rs:378` (tags API) | `https://api.github.com/repos/doli-network/doli/releases/tags/v6.24.1` | **200** |
| `download.rs:528` | `https://github.com/doli-network/doli/releases/download/v6.24.1/SIGNATURES.json` | **200** |
| `download.rs:551` | `https://github.com/doli-network/doli/releases/download/v6.24.1/CHECKSUMS.txt` | **200** |
| `misc.rs:57` (v-prefixed arg) | `https://github.com/doli-network/doli/releases/download/v6.24.1/CHECKSUMS.txt` | **200** |
| `misc.rs:57` (bare arg) | `https://github.com/doli-network/doli/releases/download/6.24.1/CHECKSUMS.txt` | **404** — see OBS-004 |

**No double slash and no missing segment** in any construction — verified
programmatically (`url[8..].contains("//")` → `false` for every constructed URL). The
base has no trailing slash and every `format!` supplies exactly one leading `/` per
segment.

### `scripts/publish_release.sh` and `scripts/sign-release.sh`

Both changed exactly one line: `REPO="e-weil/doli"` → `REPO="doli-network/doli"`.
Every downstream use is a quoted `"$REPO"` expansion, so the new value substitutes
cleanly:

`publish_release.sh` — `gh release view "$VERSION" --repo "$REPO"` (:78, :84),
`gh release download ... --repo "$REPO"` (:103), `gh release view ... --json body`
(:130), `--arg url_template "https://github.com/$REPO/releases/download/..."` (:157),
`gh release upload ... --repo "$REPO"` (:178).

`sign-release.sh` — `gh release view --repo "$REPO"` (:58, :67, :139), an echoed
help string (:61), `gh release download --repo "$REPO"` (:101),
`gh release delete-asset --repo "$REPO"` (:141), `gh release upload --repo "$REPO"`
(:144), and a final `https://github.com/$REPO/releases/tag/...` echo (:147).

All form valid `gh` invocations with `doli-network/doli`. **Neither script hardcodes
the old namespace anywhere else** — grep for `github.com`/`ghcr` in both returns only
the `$REPO`-derived line above.

`scripts/install.ps1` was also updated (`$Repo = "doli-network/doli"`), and its
`$GitHub`/`$Api` are derived from `$Repo`, so a single change propagates correctly.

### NEW-4 (two trust roots) — **CLOSED**

| Trust root | Value | Status |
|---|---|---|
| `scripts/install.sh:4` | `REPO="doli-network/doli"` | Unmodified — was already correct |
| `crates/updater/src/constants.rs:132` | `GITHUB_REPO = "doli-network/doli"` | **Now matches** |
| `scripts/install.ps1:7` | `$Repo = "doli-network/doli"` | Now matches |

The 2026-06-08 audit's NEW-4 finding ("`install.sh` uses `doli-network/doli` repo;
updater uses `e-weil/doli` — different trust roots", P3) is resolved: the installer and
the updater now converge on one namespace, and it is the one the project owns. The
audit row itself is correctly left in place as a historical record.

### Docker / GHCR — independently confirmed

MEASURED, reproduced independently of the claim provided:

```
ghcr.io/doli-network/doli-node   -> HTTP 403     (anonymous pull token + /v2/.../tags/list)
ghcr.io/e-weil/doli-node         -> HTTP 403
ghcr.io/astral-sh/uv             -> HTTP 200     (control — the probe works)
```

The control returning 200 is what makes the two 403s meaningful: an empty/denied result
against a known-good instrument indicates the repositories genuinely do not exist
publicly, rather than that the probe is broken.

CI confirmation: `.github/workflows/ci.yml:119-125` runs
`docker/build-push-action@v5` with `push: false` and `tags: doli-node:ci` — it builds
and discards. `.github/workflows/release.yml` contains **no container job at all**
(only `actions/upload-artifact` ×7 and `softprops/action-gh-release@v2` at :567, with
`permissions: contents: write` at :453-454). Neither workflow contains any namespace
literal, so **no workflow publishes to the old namespace** — release.yml targets
`github.repository` implicitly and therefore follows the repo automatically. No image
has ever been published under either namespace.

**User-facing impact** (OBS-003b): all three compose files
(`docker/docker-compose.yml:7`, `.devnet.yml:7`, `.testnet.yml:6`) plus
`docs/docker.md`, `docs/running_a_node.md` and `docs/releases.md` instruct operators to
`docker pull ghcr.io/doli-network/doli-node:latest` / `docker compose up`. That will
fail with a manifest-unknown/denied error. This is **pre-existing** — it failed
identically when pointed at `e-weil` — so M1 introduces no regression. But M1 does make
the instructions *look* freshly maintained while remaining non-functional, which is
arguably worse for a new operator than an obviously-stale namespace. Non-blocking;
should be tracked separately (either publish the image or remove the compose/docs
path).

---

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Node auto-update check | `service.rs:190 fetch_latest_release` → `download.rs:187 GITHUB_API_URL` → parse assets | **PASS** | Runtime-proven: returns `Ok(Some(v6.24.1))` from the new namespace |
| Node auto-update apply (live path) | `service.rs:9 auto_apply_from_github` → `apply.rs:415 fetch_github_release` → TOCTOU check `apply.rs:423-436` → `apply.rs:441 download_from_url(release_info.tarball_url)` | **PASS** | Uses the real `browser_download_url` from the asset list, not a synthesized name — unaffected by OBS-003 |
| `doli upgrade` (CLI) | `cmd_upgrade.rs:13 fetch_github_release` → `:60 download_from_url(tarball_url)` → `:66 verify_hash` → `:71 download_signatures_json` → `:82 verify_release_signatures` | **PASS** | All origin-derived URLs return 200 |
| `doli-node update apply` (manual) | `commands/update.rs:143 apply_update` → `apply.rs:127 download_binary` | **FAIL (pre-existing)** | Both `urls_to_try` entries 404 — see OBS-003 |
| Maintainer signing | `cmd_governance.rs:33 download_checksums_txt` → `download.rs:551` | **PASS** | 200; version normalized correctly by `download_checksums_txt` |
| `doli-node release sign` | `misc.rs:57` CHECKSUMS fetch | **Conditional** | 200 with `v`-prefixed arg, 404 with bare arg — see OBS-004 |
| Release publish | `publish_release.sh` / `sign-release.sh` `gh --repo doli-network/doli` | **PASS (static)** | Command forms valid; not executed (would mutate a live release) |
| Docker compose up | `docker compose -f docker/docker-compose.yml up` | **FAIL (pre-existing)** | Image does not exist in GHCR |

---

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Grep every Rust file for a GitHub URL built from a literal rather than the constants | Only the 3 constants | Only the constants + `download.rs:378` which interpolates `GITHUB_REPO`. Clean. | none |
| 2 | Inspect `.github/workflows/release.yml` + `ci.yml` for the old namespace or a GHCR push | Possibly a stale publish target | Zero namespace literals in either file; `release.yml` has no container job; `ci.yml` is `push: false` | none |
| 3 | Check `Cargo.lock` and all 14 sub-crate `Cargo.toml` for a stale repository URL | Possible per-crate drift | `Cargo.lock`: no `e-weil`. All 14 sub-crates use `repository.workspace = true`, so the single root change propagates. Clean. | none |
| 4 | Check `flake.nix` for a pinned source URL | Possible stale fetch | Only `nixpkgs`/`flake-utils`/`rust-overlay` inputs; the doli `src` fetch is commented out (:93). Clean. | none |
| 5 | Resolve the URL `download_binary` actually builds against the live release | 200 | **404** — asset `doli-node-linux-x64` does not exist; real assets are `doli-v6.24.1-<triple>.tar.gz` | medium (pre-existing) |
| 6 | Pass a bare (non-`v`) version to the `misc.rs:57` CHECKSUMS URL | 200 | **404** — this call site does not normalize the `v` prefix although `version_str` exists two lines above and `download_checksums_txt` does normalize | low (pre-existing) |
| 7 | Enumerate **every** `*.doli.network` hostname in shipped code and resolve each | Only `releases.` was dangling | **Three more NXDOMAIN names**: `rpc1`, `rpc2`, `testnet-rpc` at `crates/wallet/src/rpc_client.rs:255-258` | low (out of scope) |
| 8 | Verify `testnetlinux/explorer/*.html` is really a passive fixture | Passive | **It is served** by systemd unit `doli-explorer` — see OBS-001 | medium |
| 9 | Verify the exclusion `testnet*/bin/*` is inert | Inert | Git-tracked compiled binaries with the old origin baked in; refreshed only on rebuild+copy | low |
| 10 | Check the updater SKILL.md for residual `FALLBACK_MIRROR` after the doc pass | Fully updated | Line 568 and the function docs were updated; **line 32 still lists `FALLBACK_MIRROR`** as an exported constant | low |

---

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Custom update URL unreachable | **Yes** (harness, `.invalid` TLD) | Yes — `warn!("Failed to fetch from custom URL")` | N/A | **Yes** — `Ok(None)`, node keeps producing | Criterion 4 |
| All release sources fail | Partially (custom-URL arm) | Yes — `warn!("Could not fetch release info from any source")` | N/A | **Yes** — `Ok(None)`, no `Err`, no panic | GitHub arm not force-failed; would require DNS blackholing `api.github.com`, out of scope for a read-only run |
| Fallback mirror NXDOMAIN | **Yes** (`host` → NXDOMAIN) | N/A | N/A | **Yes** — the code path no longer exists, so the failure is now unreachable by construction | This is the fix |
| Download source 404 | **Yes** (`doli-node-linux-x64` → 404) | Yes — `warn!("Download failed from {}: {}", url, e)` per entry, then `Err(last_error)` | No | **Yes** — surfaces `DownloadFailed("HTTP 404")` to the caller rather than hanging or panicking | Loop correctly tries all entries before erroring |
| GitHub API returns 404 | Not triggered (live API returns 200) | Code path exists: `download.rs:189-191` returns `Ok(None)` on `StatusCode::NOT_FOUND` | N/A | Yes | Untriggerable without mutating the live repo |
| Enforcement timeout after failed download | Not triggered | Documented at `SKILL.md:567` — `binary_ready` stays false, production continues with warning, enforcement auto-expires at +30min | N/A | Yes | Requires a live node + approved release; out of scope |

---

## Security Validation

| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Update origin namespace hijack (the incident) | Resolved `github.com/e-weil` and `api.github.com/users/e-weil` | **PASS (fixed)** | Both 404 → namespace is unowned and re-registrable by anyone. Constants no longer point there; the trust root is now `doli-network`, which `git remote -v` confirms the project controls |
| Reliance on a rename-redirect as a security boundary | Read `constants.rs:118-138` | **PASS** | The new doc comment states the invariant explicitly and names the failure mode. Good — this is the durable part of the fix |
| Dangling DNS name in the download chain | `host releases.doli.network` → NXDOMAIN; grepped all 4 roots | **PASS (removed)** | An attacker who could register that name previously fed `binary_url_template`, which `download_binary` tries **first**. Removal eliminates a first-position hijack primitive |
| Two divergent trust roots (audit NEW-4) | Compared `install.sh:4`, `install.ps1:7`, `constants.rs:132` | **PASS (closed)** | All three now `doli-network/doli` |
| Second, un-repointed URL construction path | Per-root grep for hardcoded `github.com`/`ghcr.io` in all Rust source | **PASS** | Only the constants; `download.rs:378` interpolates `GITHUB_REPO` |
| CI publishing to the abandoned namespace | Read both workflow files for namespace literals and push targets | **PASS** | Zero namespace literals; `ci.yml push: false`; `release.yml` targets the current repo implicitly |
| Container registry namespace squat | Anonymous GHCR token probe, both namespaces + control | **PASS (no exposure)** | Neither image exists (403/403, control 200) → nothing to squat *on*, but also nothing published. See OBS-003b |
| TOCTOU on CHECKSUMS.txt | Read `apply.rs:417-437` | **PASS (unchanged)** | The AUDIT-UPDATE-002 defence is intact and untouched by this diff |
| Signature verification chain | Read `cmd_upgrade.rs:71-90`, confirmed `SIGNATURES.json` fetch returns 200 from the new origin | **PASS** | 3-of-5 maintainer verification path unaffected; the constants change only moves *where* the signed artifacts are fetched from |
| Other dangling hostnames in shipped code | Enumerated + resolved all `*.doli.network` | **Partial** | 3 more NXDOMAIN names in `rpc_client.rs` — but `doli.network` is project-owned, so a third party cannot create those subdomains. Availability defect, not a hijack primitive. See OBS-006 |
| Unowned-namespace link in a served page | Confirmed `doli-explorer.service` serves the HTML | **FAIL (non-blocking)** | See OBS-001 |

---

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `.claude/skills/updater/SKILL.md:32` | Lists `FALLBACK_MIRROR` among the constants exported by `crates/updater` | The constant no longer exists; `lib.rs:66` no longer re-exports it | low |
| `docker/docker-compose*.yml` (×3), `docs/docker.md`, `docs/running_a_node.md`, `docs/releases.md` | `docker pull ghcr.io/doli-network/doli-node:latest` works | Image has never been published (403; control 200); no workflow pushes it | medium (pre-existing) |
| `testnetlinux/explorer/index.html:583`, `network.html:219` | Link labelled as the project source repo | Points at `github.com/e-weil/doli` → 404 | medium |
| `crates/updater/src/download.rs:20-22` | "Tries sources in order: 1. Primary URL … 2. GitHub Releases (CDN)" | Accurate after the change | none — correctly updated |
| `.claude/skills/updater/SKILL.md:131,137,568` | Line numbers and fallback description | Accurate after the change (`download.rs:23`, `:104`, `constants.rs:132`) | none — correctly updated |

Positive note: the doc pass was thorough. `docs/architecture.md`, `docs/auto_update_system.md`,
`docs/buy_doli.md`, `docs/producer_node_quickstart.md`, `docs/testnet.md`,
`docs/troubleshooting.md`, `scripts/README.md`, `specs/engine-parts.md` and
`specs/gui-architecture.md` all now carry `doli-network/doli`, including the
`raw.githubusercontent.com` install one-liners and the GUI's `latest.json` URL.

---

## Blocking Issues (must fix before merge)

**None.** Both requirements are met, no Must criterion failed, no regression was
introduced, and no adjacent consumer broke.

---

## Non-Blocking Observations

- **[OBS-001] — `testnetlinux/explorer/*.html` is OPERATIONAL, not a "runtime fixture".**
  **Stating this loudly as instructed: this exclusion is mislabelled.**
  `testnetlinux/explorer/index.html:583` and `network.html:219` contain
  `<a href="https://github.com/e-weil/doli">`. These files are **served over HTTP by a
  systemd unit**, not sitting inert on disk:
  `testnetlinux/scripts/install-services.sh:130-149` writes a `doli-explorer.service`
  with `ExecStart=node ${explorer_dir}/server.js`; `server.js:99` creates the HTTP
  server, `:164` maps `/` → `/index.html`, `:197` listens (documented as port 8080 at
  `install-services.sh:10`).
  A testnet operator browsing the explorer sees a link to a namespace **nobody owns**.
  Today it 404s. If anyone re-registers `e-weil`, that link silently sends operators to
  attacker-controlled content presented as the official DOLI repository — a credible
  social-engineering path to trojaned install instructions.
  *Why it is still non-blocking*: it is a hyperlink, not a binary-fetch path; it is
  testnet-only (`testnetlinux/`, not the production explorer, which commit `48183c0`
  already fixed); and it is outside the artifact classes REQ-I157-010 enumerates.
  *Recommendation*: two-line fix in the same milestone — there is no reason to leave a
  live link to an unowned namespace in a served page while explicitly fixing exactly
  that class of defect elsewhere.

- **[OBS-002] — Git-tracked binaries carry the old origin compiled in.**
  `testnet/bin/{doli,doli-node}` and `testnetlinux/bin/{doli,doli-node}` are committed
  binaries whose embedded `GITHUB_*` constants still say `e-weil`. They self-heal on the
  normal workflow (`testnetlinux/scripts/testnet.sh:395` copies from
  `$DOLI_REPO/target/release/doli-node`), so a rebuild + deploy clears them. But a fresh
  clone that starts the local testnet without rebuilding runs stale-origin binaries.
  Low severity (local testnet only), but worth a rebuild-and-recommit, or removing
  binaries from version control entirely.

- **[OBS-003] — `download_binary()` 404s against the real release (PRE-EXISTING, not a
  regression).** `download.rs:33-36` synthesizes
  `.../download/v{version}/doli-node-{platform}` and `fetch_from_github()` at
  `download.rs:298-301` sets `binary_url_template` to the same shape — but the published
  assets are named `doli-v6.24.1-<target-triple>.tar.gz`. Measured:
  `.../v6.24.1/doli-node-linux-x64` → **404**, while
  `.../v6.24.1/doli-v6.24.1-x86_64-unknown-linux-gnu.tar.gz` → **200**. So **both**
  remaining `urls_to_try` entries fail whenever the `Release` came from
  `fetch_from_github()`, and `apply_update()` dies at step 1.
  Reachable from the operator command `doli-node update apply`
  (`bins/node/src/commands/update.rs:143`).
  **This is not caused by M1**: before the change all three entries were dead too
  (primary 404, GitHub 404, fallback NXDOMAIN), so removing the NXDOMAIN entry removed
  only a futile DNS lookup. The *live* auto-update path
  (`auto_apply_from_github` → `fetch_github_release` → real `browser_download_url`) is
  correct and unaffected. Recommend a separate incident.

- **[OBS-003b] — Compose files point at an unpublished image.** All three compose files
  and three docs instruct `docker pull ghcr.io/doli-network/doli-node:latest`. Confirmed
  independently: 403 for both old and new namespace, control `astral-sh/uv` 200,
  `ci.yml:123 push: false`, no container job in `release.yml`. An operator following
  `docs/docker.md` hits a hard failure on the first command. Pre-existing and unchanged
  in kind by M1 — but M1 refreshed the namespace without making the path work, so the
  instructions now look current while still being dead. Either publish the image from
  `release.yml` or remove the docker path from the docs.

- **[OBS-004] — `misc.rs:57` does not normalize the version prefix.**
  `bins/node/src/commands/misc.rs:57` builds
  `format!("{}/{}/CHECKSUMS.txt", updater::GITHUB_RELEASES_URL, version)` from the raw
  argument, while `version_str` (with `v` stripped) is computed two lines earlier and
  `updater::download_checksums_txt()` at `download.rs:544-549` *does* normalize.
  Measured: `v6.24.1` → 200, `6.24.1` → **404**. A maintainer running
  `doli-node release sign 6.24.1` gets an opaque fetch failure. Pre-existing; one-line
  fix (reuse the same `if version.starts_with('v')` normalization).

- **[OBS-005 / OBS-006] — Regression guards missing for the two riskiest properties.**
  (a) Nothing asserts the `download_binary` index→label mapping; a future third URL
  entry could reintroduce exactly the off-by-one this review had to rule out by hand.
  (b) Nothing asserts `fetch_latest_release` returns `Ok(None)` rather than `Err` on
  total failure — a property the node relies on to keep producing when the update
  channel is down. Both are cheap unit tests. Recommend adding them while the context is
  fresh.

- **[OBS-007] — Three more NXDOMAIN hostnames in shipped code (out of M1 scope).**
  `crates/wallet/src/rpc_client.rs:255-258` returns `rpc1.doli.network`,
  `rpc2.doli.network` (mainnet) and `testnet-rpc.doli.network` (testnet) from
  `default_endpoints()`; all three are NXDOMAIN (`seed1/2/3.doli.network`,
  `seeds.doli.network` and `testnet.doli.network` do resolve). Same *class* as
  REQ-I157-011 but a materially lower severity: `doli.network` is project-owned, so no
  third party can create those subdomains — this is a broken-default availability bug,
  not a hijack primitive. Different chain (wallet RPC, not the update download chain),
  so correctly out of scope here. Worth its own ticket.

- **[OBS-008] — `SKILL.md:32` residual drift.** The constants list still names
  `FALLBACK_MIRROR`. The rest of the file was updated correctly. This is the agent-facing
  index, so the stale entry will mislead future agents looking for the constant.

- **[OBS-009] — Base commit moved during the session.** `HEAD` advanced from `e6d72577`
  to `f2b66c19` (`Merge branch 'bugfix/inc-i-167-wallet-overwrite-guard'`) via concurrent
  work. The 23 INC-I-157 files are not in that commit and remain unstaged. Rebase/verify
  before committing.

---

## Modules Not Validated

- **Release publication end-to-end** — `publish_release.sh` and `sign-release.sh` were
  validated statically (command-form correctness with the new `$REPO`). They were not
  executed, because running them mutates a live GitHub release. Recommend a dry run
  against a throwaway tag before the next real release.
- **`fetch_latest_release` GitHub-arm total failure** — only the custom-URL arm was
  force-failed. Force-failing the GitHub arm requires DNS blackholing `api.github.com`,
  which is out of scope for a read-only validation run. The terminal `Ok(None)` is shared
  by both arms and was verified by reading, so residual risk is low.
- **Windows install path** (`scripts/install.ps1`) — change is a single variable and its
  derived URLs, verified by reading; not executed (no Windows host).

---

## Final Verdict

**PASS** — Both requirements (REQ-I157-010 Must, REQ-I157-011 Should) are met. The
origin is pinned to a namespace the project provably controls, the NXDOMAIN fallback is
fully removed from the constant, both call sites and the re-export, and the
highest-risk element of the change — the `urls_to_try` index→label mapping — is correct
with no off-by-one. `fetch_latest_release()` still returns `Ok(None)` on failure
(runtime-proven). No version was bumped, nothing is staged or committed, unrelated
concurrent work is untouched, the workspace compiles clean with all targets, and all 44
updater tests pass. Audit finding NEW-4 is closed. Approved for review.

Nine non-blocking observations are recorded above. Two warrant action in or near this
milestone: **OBS-001** (a served explorer page still links to the unowned namespace —
the "runtime fixture" exclusion label is wrong) and **OBS-005/006** (add the two cheap
regression tests for the properties this review had to verify by hand).
