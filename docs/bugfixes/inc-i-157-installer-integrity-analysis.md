# INC-I-157 — Installer & Auto-Update Binary Integrity Analysis

**Incident**: INC-I-157 (open, severity high, domain supply-chain / release-distribution)
**Run**: 499 (`/omega-doctor`) — Analyst pass
**Date**: 2026-08-07/08
**Mode**: ANALYSIS ONLY. No fix, no commit, no deploy. Read-only on all repo files except this document.
**Scope**: `scripts/install.sh`, `bins/node/postinst.sh`, `crates/updater/`, `bins/cli/src/cmd_upgrade.rs`, `bins/node/src/updater/`, `.github/workflows/release.yml`, and the cross-repo boundary into `doli-network/explorer`.

---

## 0. Anchor Detection & Contradiction Log (MANDATORY)

No `docs/.workflow/skeptic-analysis.md` exists, so the fallback anchor procedure applies.

**FIRST READ**: "The served installer is a stale fork that no pipeline can reach, so the repo fix at `857746b6` is structurally stranded." **Setting aside.**

**SECOND (contradicting) INTERPRETATION**: "A pipeline DOES reach the served endpoint; the fix is not stranded, it is being actively reverted." I chose the second, on this evidence: `.github/workflows/release.yml:423-445` defines a `deploy-install-scripts` job that scps `scripts/install.sh` to the exact served docroot, and `gh run view 29909361661` shows that job concluded **success** on 2026-07-22 for tag v6.24.0. The first interpretation is factually false.

### ⚠ CONTRADICTION (with my own task brief)

> The task brief states: *"Commit `857746b6` added download integrity verification to `scripts/install.sh` — a file no production path reads."*

**This is DISPROVEN.** `.github/workflows/release.yml:423-445` reads it and publishes it to the production docroot on every `v*` tag, and that job has succeeded as recently as 2026-07-22. The correct statement is: *a production path reads it, and a second, more frequent, destructive publisher reverts it.* REQ-I157-006 is answered on the corrected premise. I am flagging this rather than silently adopting the brief's framing.

### ⚠ USER CLAIM DISPROVEN (re-verified from scratch this session)

> *"republished 2026-08-05 by an unidentified publisher"*

**FALSE, plainly.** The publisher is the project's own `doli-network/explorer` repository and its `Deploy to ai2` workflow. Independently re-measured at 2026-08-07 23:24:15 GMT:

| Artifact | sha256 | bytes | lines |
|---|---|---|---|
| `curl https://doli.network/install.sh` | `1580298301e114ccd723b02dba2ceb119bb114d5b97f15765644c267a91184bb` | 7473 | 197 |
| `../explorer/doli.network/install.sh` (git-tracked) | `1580298301e114cc…` (**identical**) | 7473 | 197 |
| repo `scripts/install.sh` | `44c042827b5a5096a…` | 9114 | 232 |

`cmp served explorer/doli.network/install.sh` → **IDENTICAL** (exit 0). Response headers: `server: nginx/1.24.0 (Ubuntu)`, `etag: "6a7597e7-1d31"`, `last-modified: Fri, 07 Aug 2026 08:31:35 GMT`. The `last-modified` moved from 2026-08-05 14:16:48 GMT (prior session) to 2026-08-07 08:31:35 GMT while sha256 stayed byte-identical → **mtime touch from a redeploy, not a content change**. There is no evidence of intrusion. This is artifact drift inside our own infrastructure.

---

## 1. Architecture Context

### 1.1 Graph coverage disclosure

Per `.claude/protocols/graph-briefing.md`, the code graph was auto-refreshed (`graphify update .` → `graphify-out/graph.json`) and queried **first**:

```
python3 .claude/scripts/blast.py graphify-out/graph.json install_binary --hops 2
  bins/node/src/updater/service.rs  L457  calls  .auto_apply()
  crates/updater/src/apply.rs       L411  calls  auto_apply_from_github()
  crates/updater/src/apply.rs        L75  calls  apply_update()

python3 .claude/scripts/blast.py graphify-out/graph.json verify_release_signatures_with_keys --hops 2
  bins/node/src/updater/service.rs  L182  calls  .check_for_updates()
  bins/node/src/updater/service.rs   L34  method UpdateService
  crates/updater/src/verification.rs L48  calls  verify_release_signatures()
```

**Graph blind spots cross-referenced by grep (must be stated, per protocol §3):**
- The graph did **not** report `bins/cli/src/cmd_upgrade.rs:110` and `:129` as dependents of `install_binary`, although both call `updater::install_binary`. This is the known Rust method/cross-crate blind spot. Verified by `grep -rn --include='*.rs' "install_binary"`.
- **Shell scripts (`scripts/install.sh`, `bins/node/postinst.sh`), GitHub Actions YAML, nginx config, and the second repository (`doli-network/explorer`) are entirely outside the graph.** The most load-bearing edge in this whole incident — CI job → docroot file → nginx → `sudo sh` on a producer — is **not representable in the code graph at all**. That boundary was mapped with `git log`, `gh run view --json jobs`, `gh run view --log-failed`, and direct file reads. This is an architectural observation in its own right: the release-distribution pipeline has no machine-checkable dependency representation anywhere in the project.

### 1.2 Module boundaries

| Module | Responsibility | Depends on | Depended by |
|---|---|---|---|
| `.github/workflows/release.yml` (jobs `release`, `deploy-install-scripts`) | Build 7 artifacts, generate `CHECKSUMS.txt`, generate `SIGNATURES.json`, publish GitHub Release, scp installers to docroot | GitHub-hosted runners, `DEPLOY_SSH_*` secrets, SSH reachability of the web host | Every install and every upgrade on every host |
| `scripts/install.sh` (232 lines, canonical) | One-liner bootstrap installer **with** checksum verification | GitHub API, GitHub Releases, `CHECKSUMS.txt` | `deploy-install-scripts` job only |
| `../explorer/doli.network/install.sh` (197 lines, vendored fork) | The bytes actually served at `https://doli.network/install.sh` | GitHub API, GitHub Releases | **Every operator running the documented one-liner** |
| `../explorer/.github/workflows/deploy.yml` | Deploy explorer via `git reset --hard origin/main` in `/var/www/explorer-repo` | explorer `main`, `DEPLOY_*` secrets | The served docroot — *including files it does not own* |
| `crates/updater/src/download.rs` | Fetch release metadata, fetch `CHECKSUMS.txt`, compute `sha256(CHECKSUMS.txt)`, parse per-platform hash, download artifacts | `GITHUB_API_URL`, `GITHUB_RELEASES_URL`, `FALLBACK_MIRROR` | `apply.rs`, node update service, `cmd_upgrade.rs` |
| `crates/updater/src/verification.rs` | Ed25519 3-of-5 maintainer signature check over `"{version}:{sha256(CHECKSUMS.txt)}"` | `constants.rs` bootstrap keys or on-chain keys | node update service (**blocking**), `cmd_upgrade.rs` (**non-blocking**) |
| `crates/updater/src/apply.rs` | Stage, `sudo rm -f` + `sudo cp`, mode postcondition read-back | sudoers whitelist installed by `install.sh`/`postinst.sh` | node update service, `cmd_upgrade.rs` |
| `bins/node/postinst.sh` | .deb/.rpm post-install: user, dirs, polkit, sudoers, `/usr/local/bin` symlinks | dpkg/rpm | deb/rpm host class |

### 1.3 Artifact flow: `git tag` → producer host

```
git tag v6.24.1
   └─> release.yml: 7 build jobs  ──> artifacts/*.tar.gz|.deb|.rpm|.pkg|.zip
        └─> job `release`
             ├─ release.yml:470-473   sha256sum * > CHECKSUMS.txt
             ├─ release.yml:475-486   SIGNATURES.json  {"signatures": []}   ← EMPTY, ALWAYS
             └─ release.yml:566-574   softprops/action-gh-release  → github.com/doli-network/doli/releases
        └─> job `deploy-install-scripts` (needs: [release])
             └─ release.yml:443-445   scp scripts/install.sh → /var/www/explorer-repo/doli.network/
                                       [SUCCESS on v6.24.0 2026-07-22 | FAILURE on v6.24.1 2026-08-05]
                                                    ▲
                                                    │  SAME PATH, TWO WRITERS
                                                    ▼
   push to explorer main (41 pushes 2026-07-22..2026-08-08)
        └─> explorer/.github/workflows/deploy.yml:18
             └─ "cd /var/www/explorer-repo && git fetch origin main && git reset --hard origin/main"
                                       ← DESTRUCTIVE: reverts install.sh to the vendored 197-line fork

   nginx/1.24.0 serves /var/www/explorer-repo/doli.network/install.sh at https://doli.network/install.sh
        └─> operator runs:  curl -sSfL https://doli.network/install.sh | sudo sh
             └─> [no checksum]  tarball/deb/rpm/pkg → /usr/local/bin or dpkg/rpm database
```

Parallel path (auto-update / manual upgrade), which does **not** touch the docroot at all:

```
doli-node UpdateService (6h interval)          |   doli upgrade (interactive/cron)
 fetch_latest_release()                        |    fetch_github_release()
 → GITHUB_API_URL = e-weil/doli (301 → us)     |    → same constant
 → CHECKSUMS.txt, SIGNATURES.json              |    → CHECKSUMS.txt
 → verify_release_signatures_with_keys()       |    → verify_hash(tarball, expected)   [BLOCKING]
     [BLOCKING — returns early on failure]     |    → verify_release_signatures()      [ADVISORY ONLY]
 → veto period 5min, 40% threshold             |    → install_binary → sudo rm -f + sudo cp
 → auto_apply_from_github(v, signed_sha)       |    → restart service
 → TOCTOU re-check of sha256(CHECKSUMS.txt)    |
 → verify_hash(tarball, per-platform hash)     |
 → install_binary → sudo rm -f + sudo cp       |
```

### 1.4 Architectural constraints & invariants

| Constraint | Why it exists | What breaks if violated |
|---|---|---|
| `/var/www/explorer-repo` is a **git working tree** that a deploy resets hard | It is the explorer's deployment mechanism | Any file placed there by a non-git writer is silently destroyed on the next explorer push |
| Sudoers whitelist grants exactly 4 verbs (`rm -f` ×2, `cp` ×2) on fixed paths | Minimize privileged surface (`install.sh:192-199`, `postinst.sh:42-48`) | Adding a privileged verb in code fails closed on every already-deployed host — a code-only fix cannot widen it |
| `INSTALLED_BINARY_MODE = 0o755` and the other-execute bit must survive `sudo cp` | INC-I-153 root cause | `status=203/EXEC` on a producer running as `User=doli` |
| Maintainer signatures cover `"{version}:{sha256(CHECKSUMS.txt)}"` — one hash anchors the whole chain | `apply.rs:392-397`, AUDIT-UPDATE-002 | Without the signed anchor the TOCTOU re-check at `apply.rs:423-436` compares an unsigned value against itself |
| Two binary locations exist by design: `/usr/bin` (canonical + sudoers targets) and `/usr/local/bin` (postinst symlinks, served-installer tarball target) | Historical | If a *real file* lands at `/usr/local/bin/doli-node`, the postinst `-f` guard never corrects it and PATH order decides which version runs |

### 1.5 Blast radius

**Direct** — every host that has ever run the documented one-liner or `doli upgrade`; every future host that runs it. Files: `scripts/install.sh`, `../explorer/doli.network/install.sh`, `.github/workflows/release.yml`, `../explorer/.github/workflows/deploy.yml`, `crates/updater/src/constants.rs`, `bins/cli/src/cmd_upgrade.rs`, `bins/node/postinst.sh`.

**Indirect** — the consensus layer. A chosen binary on a mainnet producer is not a host compromise, it is a *consensus* compromise: it can forge attestations, equivocate, or silently alter `apply_block()` state transitions. The blast radius of this supply-chain surface is the entire chain, not the individual host.

**Cross-repo** — remediation of the served bytes crosses into `doli-network/explorer`, a separate repository with its own CI, its own secrets, and its own commit/push authority. No change in this repository alone can fix the served endpoint while `git reset --hard` remains the explorer's deploy verb.

### 1.6 Brittleness Check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
Details:
  [1] Cross-module blast radius — YES. The fix spans two repositories, two CI workflows,
      one nginx docroot, the Rust updater crate, and the packaging postinst. None of these
      share a direct dependency edge.
  [2] Invariant gaps — YES. No module anywhere enforces "the bytes served at
      https://doli.network/install.sh equal the bytes at scripts/install.sh@<latest tag>".
      Nothing measures it, nothing alerts on it, and it was only found by manual curl.
  [3] Data flow reversal — NO. Artifact flow is uniformly forward (tag → CI → docroot → host).
  [4] Shared mutable state — YES. /var/www/explorer-repo/doli.network/install.sh is written by
      two independent pipelines with NO owner, NO lock, and NO last-writer-wins policy.
      One of the two writers (git reset --hard) destroys the other's write unconditionally.
  [5] Contract absence — YES. There is no interface, manifest, or declared ownership between
      the doli release pipeline and the explorer deploy pipeline. The coupling is an
      undocumented filesystem path string appearing in two repos that never reference
      each other.
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 2. Requirements

Requirement IDs use `REQ-I157-NNN`. These are **findings-as-requirements** for an analysis-only pass: each states what must be established/true, its MoSCoW priority for the remediation that follows, and verifiable acceptance criteria.

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-I157-001 | The served installer MUST verify a cryptographic hash of the artifact before installing it | Must | - [x] Served copy audited line-by-line: **zero** hash/signature checks (confirmed gap)<br>- [ ] Post-fix: served bytes contain a `CHECKSUMS.txt` fetch + `sha256sum` compare that fails closed |
| REQ-I157-002 | The complete trust anchor for both install paths MUST be enumerated and every inert control identified | Must | - [x] Table in §4 lists every control as EXISTS / INERT / ABSENT with `file:line`<br>- [x] `SIGNATURES.json` emptiness verified live against a real release |
| REQ-I157-003 | Attacker entry points MUST be enumerated, ranked, and separated into reachable-today vs theoretical | Must | - [x] §5 table: 9 entry points, each with required control, cost tier, reachability verdict |
| REQ-I157-004 | The affected host population MUST be stated as measured or explicitly UNVERIFIED | Must | - [x] §6 states exactly what is measurable from the repo (nothing) and the exact query that would measure it<br>- [x] The "~30 producers" figure is labelled UNVERIFIED |
| REQ-I157-005 | The jorge causal claim MUST be tested link-by-link against INC-I-153's confirmed root cause | Must | - [x] §7 renders a verdict of same/distinct/compounding with evidence for each link |
| REQ-I157-006 | The structural reason the divergence persists MUST be identified, with the implication for any future installer security fix | Must | - [x] §8 identifies the destructive second writer with CI job-level evidence<br>- [x] Implication stated for all future fixes |
| REQ-I157-007 | Exactly one publisher MUST own `/var/www/explorer-repo/doli.network/install.sh` | Must | - [ ] `git log` on the owning repo shows a single writer<br>- [ ] Two consecutive deploys of the *other* pipeline leave the served sha256 unchanged |
| REQ-I157-008 | `doli upgrade` MUST fail closed on insufficient maintainer signatures once signatures actually exist | Should | - [ ] `cmd_upgrade.rs` returns `Err` (not `println!`) on `InsufficientSignatures`<br>- [ ] Gated so it does not brick upgrades while `signatures: []` ships |
| REQ-I157-009 | The release pipeline MUST produce non-empty maintainer signatures, or the 3-of-5 scheme MUST be removed rather than simulated | Should | - [ ] `SIGNATURES.json` for a new tag contains ≥3 signatures verifying against `bootstrap_maintainer_keys(Mainnet)`<br>- [ ] `verify_release_signatures` returns `Ok` for that release |
| REQ-I157-010 | The updater's release origin MUST be the repository the project actually controls | Should | - [ ] `constants.rs` `GITHUB_REPO`/`GITHUB_API_URL`/`GITHUB_RELEASES_URL` name `doli-network/doli`<br>- [ ] No production fetch relies on a GitHub rename redirect |
| REQ-I157-011 | The download fallback chain MUST NOT contain a non-resolving hostname | Should | - [ ] `FALLBACK_MIRROR` either resolves and serves signed content, or is removed |
| REQ-I157-012 | The `deploy-install-scripts` job failing MUST be visible, not silent | Should | - [ ] A failed installer deploy blocks the release or raises an alert (it failed on v6.24.1 and nothing surfaced) |
| REQ-I157-013 | `postinst.sh` MUST NOT let a stale regular file at `/usr/local/bin/doli-node` shadow the packaged binary forever | Should | - [ ] Guard replaces the path unconditionally unless it already IS the intended symlink |
| REQ-I157-014 | Intel Macs MUST NOT be served an `aarch64` binary as if it were native | Could | - [ ] `Darwin-x86_64` maps to a real x86_64 artifact or errors explicitly |
| REQ-I157-015 | Pipe-to-shell (`curl \| sudo sh`) replaced with a download-verify-execute flow | Won't (this iteration) | N/A — deferred. Documented in §11 |

### Detailed acceptance criteria

**REQ-I157-001 — Confirm/refute the checksum gap** *(see §3 for full evidence)*
- [x] Given the served 197-line installer, when it downloads `doli-${VERSION}-${TARGET}.{tar.gz,deb,rpm,pkg}` at `install.sh:117`, then it proceeds directly to `case "$METHOD"` at `:120` and installs — **no hash is computed, no `CHECKSUMS.txt` is fetched, no signature is checked**.
- [x] Given the canonical 232-line installer, when it downloads at `scripts/install.sh:85`, then it fetches `CHECKSUMS.txt` (`:90-93`), extracts the expected hash (`:95-96`), computes the actual hash (`:98-104`), and calls `err` on mismatch (`:106-108`) — fail-closed at four distinct points.
- [x] Edge case: the canonical installer also fails closed when `sha256sum`/`shasum` are both absent (`:103`), rather than skipping verification.

**REQ-I157-007 — Single publisher**
- [ ] Given the doli release CI has just scp'd `scripts/install.sh` to the docroot, when the explorer pushes to `main` and its deploy runs, then `curl https://doli.network/install.sh | sha256sum` still equals `sha256sum scripts/install.sh` at the released tag.
- [ ] Given a fresh clone of the owning repo, when `git log -- <installer path>` is run, then exactly one pipeline appears as the source of truth.

---

## 3. REQ-I157-001 — Confirm/refute the checksum gap

**VERDICT: CONFIRMED. The served installer performs zero binary-integrity verification.**

### 3.1 Served copy (`../explorer/doli.network/install.sh`, 197 lines) — integrity checks: **NONE**

Complete enumeration of every check the served copy performs:

| Line | Check | Integrity-relevant? |
|---|---|---|
| `:22-32` | OS/arch supported | No |
| `:56` | `curl -sSfL "$API"` succeeded | Transport only |
| `:59` | `tag_name` non-empty | No |
| `:77-87` | Installed version ≥ latest (semver compare) | No |
| `:117` | `$FETCH_OUT ... \|\| err "Download failed"` | HTTP status only |
| `:120-142` | `case "$METHOD"` → `installer -pkg` / `dpkg -i` / `rpm -i` / `tar -xzf` + `install -m 755` | **None** |

The critical sequence, verbatim (`../explorer/doli.network/install.sh:116-140`):

```sh
info "Downloading ${FILE}..."
$FETCH_OUT "${TMPDIR}/${FILE}" "$URL" || err "Download failed. Check ${GITHUB}/releases/tag/${VERSION}"

# ── Install ────────────────────────────────────────────────────
case "$METHOD" in
    pkg)   sudo installer -pkg "${TMPDIR}/${FILE}" -target / ;;
    deb)   sudo dpkg -i "${TMPDIR}/${FILE}" ;;
    rpm)   sudo rpm -i "${TMPDIR}/${FILE}" ;;
    tarball)
        tar -xzf "${TMPDIR}/${FILE}" -C "$TMPDIR"
        ...
        sudo install -m 755 "${DIR}/doli-node" /usr/local/bin/doli-node
```

Download at `:117`, privileged install at `:123`/`:127`/`:131`/`:139`. **Nothing between them.** There is no `sha256sum`, no `shasum`, no `gpg`, no `cosign`, and no reference to `CHECKSUMS.txt` anywhere in the 197 lines (`grep -c 'CHECKSUMS\|sha256\|shasum\|gpg' → 0`).

### 3.2 Canonical copy (`scripts/install.sh`, 232 lines) — integrity checks: **4, fail-closed**

`scripts/install.sh:87-109`:

```sh
# ISSUE-174 #3: verify SHA-256 of tarball against CHECKSUMS.txt from the same release.
CHECKSUMS_URL="${GITHUB}/releases/download/${VERSION}/CHECKSUMS.txt"
info "Verifying integrity..."
$FETCH_OUT "${TMPDIR}/CHECKSUMS.txt" "$CHECKSUMS_URL" \
    || err "Could not download CHECKSUMS.txt from ${CHECKSUMS_URL}. Refusing to install unverified binary."

EXPECTED_HASH=$(grep " ${FILE}$" "${TMPDIR}/CHECKSUMS.txt" | awk '{print $1}' | head -1)
[ -z "$EXPECTED_HASH" ] && err "CHECKSUMS.txt does not contain an entry for ${FILE}. Refusing to install."

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_HASH=$(sha256sum "${TMPDIR}/${FILE}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_HASH=$(shasum -a 256 "${TMPDIR}/${FILE}" | awk '{print $1}')
else
    err "sha256sum/shasum not found — cannot verify integrity. Install coreutils or perl."
fi

if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
    err "Checksum mismatch for ${FILE}: expected ${EXPECTED_HASH}, got ${ACTUAL_HASH}"
fi
```

Four independent fail-closed exits: missing `CHECKSUMS.txt` (`:93`), missing entry (`:96`), missing hasher (`:103`), mismatch (`:107`).

### 3.3 Conceptual diff — what the served copy is MISSING

| Behavior | Canonical (232L) | Served (197L) | Security delta |
|---|---|---|---|
| SHA-256 verification of the downloaded artifact | Yes, 4 fail-closed gates (`:87-109`) | **No** | **The entire gap** |
| Install target (tarball path) | `/usr/bin` (`:116-117`) | `/usr/local/bin` (`:139-140`) | Two competing locations; feeds the `postinst.sh:52` shadowing defect |
| Install format | Tarball only, always (`:74-77`) | pkg / deb / rpm / tarball (`:92-108`) | `rpm -i` (never `-U`) makes rpm hosts un-upgradable |
| `/etc/sudoers.d/doli-update` (auto-update grants) | Yes (`:192-199`) — targets `/usr/bin/*` | **No** | Hosts installed by the served copy have **no sudoers grants** → `install_binary_sudo()` `sudo cp` fails |
| polkit rule for `doli` group service control | Yes (`:170-181`) | **No** | Group members cannot manage units |
| Agent skills install to `~/.doli/skills` | Yes (`:122-137`) | **No** | Feature absent on installed hosts |
| Directory creation | `install -d -o doli -g doli -m 2770` (`:164-167`) | `mkdir` + `chown` + `chmod 2770` + `chown -R` (`:167-175`) | Functionally similar |
| Version-skip source | `doli-node --version` (`:64-72`) | `dpkg-query` → `rpm -q` → `doli-node --version` (`:67-73`) | Served copy trusts the package DB, which the `doli`/`doli-node` name schism breaks |
| Intel Mac (`Darwin-x86_64`) | Maps to `aarch64-apple-darwin` (`:35`) | Maps to `aarch64-apple-darwin` (`:36`, comment "runs via Rosetta 2") | **Both wrong** — not a divergence |

**Net**: the served copy is missing the security control *and* the privilege plumbing the auto-updater depends on. This is not merely "an older version" — the two files have materially different post-conditions on the host.

---

## 4. REQ-I157-002 — Complete trust anchor map

Legend: **EXISTS** = present and effective; **INERT** = present in code but produces no security effect in the current deployment; **ABSENT** = not implemented.

### Path (a) — one-liner: `curl -sSfL https://doli.network/install.sh | sudo sh`

| # | Control | State | Evidence |
|---|---|---|---|
| a1 | TLS/HTTPS to `doli.network` (`curl -sSfL`, fails on cert error) | **EXISTS** | Live: `HTTP/2 200`, `server: nginx/1.24.0 (Ubuntu)` |
| a2 | Authenticity of the *script* itself | **ABSENT** | No signature, no pinned hash, no `Content-Digest`. Whatever nginx serves is executed as root. |
| a3 | TLS to `api.github.com` and `github.com` | **EXISTS** | `install.sh:47-48` uses `curl -sSfL` (fail on error, follow redirects) |
| a4 | HTTP status check on the artifact download | **EXISTS** (weak) | `install.sh:117` — proves a 200, proves nothing about content |
| a5 | **SHA-256 of the downloaded artifact** | **ABSENT** | §3.1 — the gap |
| a6 | Maintainer signature on the artifact | **ABSENT** | No reference to `SIGNATURES.json` in the served copy |
| a7 | `.deb`/`.rpm`/`.pkg` package signature verification | **ABSENT** | `dpkg -i` / `rpm -i` / `installer -pkg` on an unsigned local file perform no origin verification |
| a8 | Privilege boundary | **ABSENT by construction** | The pipe executes as root before any of the above could matter |
| **Net trust anchor for path (a)** | **A single TLS session to a web server we control, and nothing else.** Whoever writes bytes to that docroot has root on every host that runs the one-liner afterwards. | | |

### Path (b) — node auto-update (`UpdateService`, 6-hour poll)

| # | Control | State | Evidence |
|---|---|---|---|
| b1 | TLS to GitHub API/Releases (reqwest default) | **EXISTS** | `download.rs:73-77, 208-213` |
| b2 | Release origin pinning | **INERT/DRIFTED** | `constants.rs:120-126` pins `e-weil/doli`; live check returns `301 → /repositories/1141544443` (= `doli-network/doli`). Works only because the GitHub rename redirect is alive. |
| b3 | Network targeting (`metadata.json`) | **EXISTS** | `download.rs:294-319, 177-195` |
| b4 | **3-of-5 Ed25519 maintainer signature check (blocking)** | **INERT** | `service.rs:220-226` calls `verify_release_signatures_with_keys` and `return`s on error. `constants.rs:29` `REQUIRED_SIGNATURES = 3`. Live `SIGNATURES.json` for v6.24.1: `"signatures": []`. → `valid_count = 0 < 3` → `InsufficientSignatures` → **early return, always**. |
| b5 | Veto period + 40% threshold governance | **UNREACHABLE** | `service.rs:236-258` is downstream of b4 and never executes |
| b6 | TOCTOU re-verification of `sha256(CHECKSUMS.txt)` against the signed value | **UNREACHABLE, and inert if reached** | `apply.rs:423-436`. The "signed" value it compares against is `checksums_sha256` from an unsigned file. With `signatures: []` this compares GitHub's value to GitHub's value. |
| b7 | Per-platform tarball SHA-256 vs `CHECKSUMS.txt` | **UNREACHABLE** | `apply.rs:446` |
| b8 | Staging dir `/var/lib/doli` 2770 + `O_NOFOLLOW` (ISSUE-174 #7) | **EXISTS** | `apply.rs:189, 260-283` |
| b9 | Sudoers whitelist of exactly 4 verbs | **EXISTS on canonical-installed hosts, ABSENT on served-installer hosts** | `scripts/install.sh:192-199`, `postinst.sh:42-48` vs. served copy has no sudoers block |
| b10 | Installed-mode postcondition read-back (INC-I-153 fix) | **EXISTS** | `apply.rs:321-374` |
| **Net for path (b)** | **The auto-update path is DEAD, fail-closed, at b4.** No mainnet host can be auto-updated by the node service today. This is the one place where the empty signature list is *protective*. It also means the "6h auto-update" the operator believes is running has never applied anything. | | |

### Path (b′) — `doli upgrade` (manual and cron)

| # | Control | State | Evidence |
|---|---|---|---|
| c1 | TLS to GitHub | **EXISTS** | `download.rs:391-414` |
| c2 | Release origin | **INERT/DRIFTED** — same `e-weil/doli` redirect | `constants.rs:123`, `download.rs:404-409` |
| c3 | **Tarball SHA-256 vs `CHECKSUMS.txt` (blocking)** | **EXISTS** | `cmd_upgrade.rs:64-68` → `verify_hash` returns `Err` → `anyhow!` propagates. **This is the only real binary-integrity control on any path that actually runs today.** |
| c4 | Independent anchor for `CHECKSUMS.txt` itself | **ABSENT** | `CHECKSUMS.txt` and the tarball come from the same GitHub release. Whoever can rewrite one can rewrite the other. c3 detects transport corruption and CDN tampering, **not** a compromised release. |
| c5 | 3-of-5 maintainer signature check | **INERT AND NON-BLOCKING** | `cmd_upgrade.rs:70` comment: *"Check maintainer signatures (informational — never blocks manual upgrade)"*. `:86-94` prints `Warning:` and continues. Also hardcodes `Network::Mainnet` at `:82` regardless of the host's actual network. |
| c6 | Install-path detection | **EXISTS (heuristic)** | `cmd_upgrade.rs:178-228` — `pgrep` → `which` → 4 hardcoded paths |
| c7 | Mode postcondition | **EXISTS** | via `install_binary` → `apply.rs` |
| **Net for path (b′)** | **`CHECKSUMS.txt` from the same release as the artifact.** This is trust-on-first-use in GitHub. It defends the wire; it does not defend against a compromised release, a compromised CI token, or a hijacked `e-weil` namespace. | | |

### Summary verdict for REQ-I157-002

> **Every integrity control that would resist a determined adversary is either INERT (the 3-of-5 signature scheme, because CI publishes an empty list) or ABSENT (the served installer's checksum). The only control actually operating in production is a self-referential checksum inside `doli upgrade` — `CHECKSUMS.txt` from the same GitHub release as the artifact it validates.**

---

## 5. REQ-I157-003 — Attacker model, ranked

"Reachable today" means: exploitable with the current deployed state, no additional preconditions beyond the named control.

| Rank | Entry point | Attacker must control | Cost | Reachable today | Effect |
|---|---|---|---|---|---|
| **1** | **nginx docroot `/var/www/explorer-repo/doli.network/install.sh`** (origin host, ai2) | Write access to one directory on the web host — via the host itself, the explorer `DEPLOY_SSH_KEY`, or a push to explorer `main` | **LOW.** A single GitHub repo write on the *explorer* repo is sufficient: `deploy.yml:18` auto-deploys every push to `main` with no review gate. | **YES** | Arbitrary root code execution on every host that subsequently runs the one-liner. No checksum, no signature, no review would catch it. |
| **2** | **GitHub release artifact / CI token on `doli-network/doli`** | `GITHUB_TOKEN` in the release job, or repo write to push a tag, or an `actions/download-artifact` supply-chain injection | **LOW-MEDIUM.** `release.yml:566-574` grants `contents: write`. Anyone able to push a `v*` tag publishes binaries. | **YES** | Poisons `CHECKSUMS.txt` **and** the artifact together → defeats c3 and a5 simultaneously. Signature check would catch it — but it is inert (`signatures: []`). |
| **3** | **`e-weil/doli` namespace reoccupation** | Create a repo at the old path, or acquire the `e-weil` account | **LOW-MEDIUM.** The updater's release origin is a *rename redirect*, not a pin. Redirects are not a security boundary; they lapse the moment the old path is reoccupied. | **CONDITIONAL — precondition not currently met** (301 still resolves to us), but the precondition is outside our control | Every `doli upgrade` and every auto-update download URL points to attacker content. `CHECKSUMS.txt` comes from the *same* attacker release → c3 passes. |
| **4** | **Explorer repo write access** | Push to `doli-network/explorer` `main` | **LOW** (same as #1 but via git rather than the host) | **YES** | Identical to #1 — the deploy is fully automatic. Listed separately because the *audit trail* differs: #4 leaves a git commit, #1 does not. |
| **5** | **PATH shadowing via the `postinst.sh` `-f` guard** | Ability to write one file to `/usr/local/bin/doli-node` once (local user with sudo, an earlier one-liner run, or an old install) | **MEDIUM** (requires prior host access) — but **persistence is free**: `postinst.sh:52` `[ ! -f /usr/local/bin/doli-node ]` is TRUE for a stale regular file, so **no future package install ever corrects it** | **YES — measured on jorge**: a 6.23.8 binary from 2026-06-21 was still shadowing 6.24.1 on 2026-08-07 | Indefinite version pinning / stale-binary persistence that survives every upgrade. As a *persistence* primitive this is arguably worse than the initial-compromise vectors. |
| **6** | **DNS hijack of `doli.network`** | Registrar or authoritative DNS | **MEDIUM-HIGH** | Theoretical | Serve any installer. Note `releases.doli.network` is currently **NXDOMAIN** while `constants.rs:129` still lists it as a download fallback — a dangling name in the trust chain. |
| **7** | **TLS/CA compromise or mis-issuance for `doli.network`** | A trusted CA, or a network position + a CA that will mis-issue | **HIGH** | Theoretical | MITM the one-liner. No cert pinning anywhere (`curl -sSfL` uses the system store). |
| **8** | **Sudoers grants abuse** | Already be the `doli` user, or win a race on `/var/lib/doli/update.bin` | **HIGH.** ISSUE-174 #7 closed the `/tmp` TOCTOU: staging is now `/var/lib/doli` 2770 `doli:doli` + `O_NOFOLLOW` (`apply.rs:189, 260-283`). Grants are 4 fixed verbs on fixed paths. | Low residual | Root, but requires already being the trusted service account. **This is the best-hardened surface in the whole chain.** |
| **9** | **Package repository** | N/A | **N/A** | **NO — does not exist** | There is no apt/yum repo. `.deb`/`.rpm` are downloaded as loose files from GitHub Releases and installed with `dpkg -i` / `rpm -i`, so no repo signing exists to be attacked *or* to protect. |

### Ranked top-3, restated

1. **The nginx docroot / explorer push path** — one repo write, fully automated deploy, root execution on every subsequent install, zero verification anywhere in the chain. Lowest cost, highest impact, reachable today.
2. **The GitHub release artifact + CI token** — poisons artifact and checksum together, defeating the only live control. The 3-of-5 signature scheme was designed to stop exactly this and is inert.
3. **The `e-weil/doli` rename redirect** — the updater's release origin is not pinned to a namespace we control; it depends on a GitHub redirect continuing to exist.

Note on ordering: #1 and #2 are close. #1 ranks first because it requires *no* cryptographic material, defeats *every* control (there are none), and its blast radius includes hosts that would otherwise be protected by the canonical installer.

---

## 6. REQ-I157-004 — Blast radius / population

**Measurable from this repository: nothing about host counts.** I grepped and read the repo; there is no fleet inventory, no producer registry snapshot, and no install telemetry. The install path is `curl | sudo sh` with no callback, so **there is no mechanism anywhere in this codebase that records how many hosts installed by which method.**

| Population | Value | Basis |
|---|---|---|
| Mainnet external producers on the one-liner path | **UNVERIFIED** | The "~30" figure is asserted in the user's framing and in prior notes; it is not measured anywhere in this repo and I did not query mainnet (constraint: no SSH, no mainnet contact) |
| Hosts with a stale file at `/usr/local/bin/doli-node` | **UNVERIFIED — n≥1** | Only jorge is measured (6.23.8 shadowing 6.24.1, 2026-08-07). `postinst.sh:52` means the condition is silent, so the true count cannot be inferred |
| Hosts on the deb/rpm path vs tarball path | **UNVERIFIED** | Determined per-host by `command -v dpkg` / `command -v rpm` at install time (`served install.sh:99-104`); not recorded |
| Structural fleet (N1-N12 + 3 seeds) | Not on the one-liner path — deployed by operator scripts | Project convention (`feedback_structural_vs_external_fleet`); **not re-verified this session** |
| Hosts auto-updating via the node `UpdateService` | **ZERO** | Derived, not assumed: `service.rs:222` returns early on `InsufficientSignatures`, and live `SIGNATURES.json` is `{"signatures": []}` for v6.24.1. No release can pass b4. |

**What would measure it (NOT run here):** query a mainnet seed's RPC for the registered producer set (`getProducers`), then correlate to install method — which is itself not recorded, so even that would only bound the population, not partition it. **An accurate answer requires a fleet inventory that does not currently exist.** That gap is itself a finding: the project cannot answer "how many hosts would a poisoned installer reach" from any data it holds.

**Lower bound that IS certain:** every host installed or upgraded via the documented one-liner since the served fork took over — i.e. at minimum, every external producer onboarded through `https://doli.network` — plus every future one.

---

## 7. REQ-I157-005 — Testing the jorge causal claim

**Claim under test:** *"That republish is what broke jorge's cron updates and forced the manual upgrade in the first place."*

**VERDICT: (c) COMPOUNDING PRECONDITION — with the first link of the claimed chain FALSE.** The installer divergence is a **distinct defect** from INC-I-153's root cause, but it is a *necessary contributing condition* for the specific failure mode that manifested on jorge. It is not the trigger, and no "republish" event occurred.

Link-by-link:

| Link | Claim | Verdict | Evidence |
|---|---|---|---|
| L1 | "A republish happened on 2026-08-05" | **FALSE** | The served sha256 is unchanged across 2026-08-05 14:16:48 and 2026-08-07 08:31:35 (`1580298301e1…` both times). `git reset --hard` rewrites the file with identical content, moving mtime only. No content changed. There was no republish — there was a *revert*, and there have been many. |
| L2 | "By an unidentified publisher" | **FALSE** | Byte-identical to `../explorer/doli.network/install.sh`, tracked in `doli-network/explorer`, deployed by `explorer/.github/workflows/deploy.yml`. Our own infrastructure. |
| L3 | "It broke jorge's cron updates" | **NOT THE CAUSE — but a precondition** | INC-I-153's confirmed root cause is `install_binary_sudo()` mode inheritance: `5a9414cf` (2026-05-08) replaced a post-copy `sudo chmod 755` with a pre-copy mode on the staged file, leaving the installed mode an unverified inheritance; `857746b6` (2026-06-09) then tightened the staged mode to `0o750`, so the installed binary lost the other-execute bit and the node — running `User=doli` against a `root:root` file — hit `status=203/EXEC`. That is a **Rust defect in `crates/updater/src/apply.rs`**, entirely independent of which installer wrote the host. The current code documents this at `apply.rs:218-225`. |
| L4 | "…and forced the manual upgrade" | **PARTIALLY — different mechanism** | Two independent reasons a cron/auto-update could not self-heal jorge: (i) the node `UpdateService` is dead at `service.rs:222` because `SIGNATURES.json` is empty — so it was never going to update anything, republish or not; (ii) `rpm -i` (never `-U`) plus the `doli` vs `doli-node` package-name schism makes the one-liner a no-op on rpm hosts. Neither is caused by an installer republish. |

### Where the divergence genuinely compounds

The served installer **never writes `/etc/sudoers.d/doli-update`** (§3.3). The canonical installer does (`scripts/install.sh:192-199`), and `postinst.sh:42-48` does for deb/rpm. `install_binary_sudo()` depends on exactly those four whitelisted verbs (`apply.rs:292-299`). Therefore:

- A host installed by the **served** copy has no sudoers grants → `sudo cp` prompts for a password under a non-interactive cron → fails.
- The served copy also installs to `/usr/local/bin` while the sudoers grants (where they exist) target `/usr/bin` → the whitelist does not match the install path.
- And `postinst.sh:52`'s `-f` guard permanently freezes whichever binary landed at `/usr/local/bin` first.

So the divergence **shapes and perpetuates** the failure surface INC-I-153 detonated on, and it is why the recovery required hands-on work. It is **not** the defect that broke execution — that was `apply.rs` mode inheritance, and it would have broken jorge regardless of which installer had provisioned the host.

**Do not merge these two incidents.** INC-I-153 is a Rust postcondition bug, fixed at `52927912` (committed, not pushed, not released). INC-I-157 is a release-distribution ownership bug, unfixed. They intersect on one host; they are different defects with different fixes in different layers.

---

## 8. REQ-I157-006 — Why the fork persists (structural cause)

**The task brief's premise is wrong, and the true cause is worse.**

`scripts/install.sh` **is** read by a production path: `.github/workflows/release.yml:423-445`, job `deploy-install-scripts`, which scps it to `/var/www/explorer-repo/doli.network/` on every `v*` tag. That job was added in `eeb765df` (2026-05-12) and it **works**:

```
gh run view 29909361661 --json jobs      # v6.24.0, 2026-07-22
  ...
  Create Release            success
  Deploy Install Scripts    success      ← the canonical installer WAS published
```

So on 2026-07-22 the served endpoint carried the integrity-verifying 232-line installer. It does not today. Why:

### The destructive second writer

`../explorer/.github/workflows/deploy.yml:18`:

```yaml
ssh ... "cd /var/www/explorer-repo && git fetch origin main && git reset --hard origin/main"
```

`/var/www/explorer-repo` is a **git working tree**, and `doli.network/install.sh` is a **tracked file in that tree**. `git reset --hard origin/main` unconditionally restores every tracked file to its committed content — including files that a completely different pipeline wrote there minutes earlier. The explorer deploy therefore **reverts** the doli release CI's scp, every time, with no error, no warning, and no diff anyone sees.

### Write frequency asymmetry — the reason the fork always wins

| Publisher | Verb | Trigger | Frequency 2026-07-22 → 2026-08-08 |
|---|---|---|---|
| doli release CI | `scp` (additive) | `v*` tag AND all 7 builds green AND SSH reachable | **1 success** (v6.24.0), **1 failure** (v6.24.1) |
| explorer deploy | `git reset --hard` (destructive) | **every push to explorer `main`** | **41 commits** on `origin/main` |

Roughly 41 destructive writes against 1 successful additive write. The canonical installer's presence on the endpoint is a transient state that lasts until the next explorer push. Observed mtimes (2026-08-05 14:16:48, 2026-08-07 08:31:35) with unchanged sha256 are exactly this: reverts, not republishes.

### The second, independent failure

`gh run view 31037845111 --log-failed` (v6.24.1, 2026-08-05):

```
Deploy Install Scripts | ssh: connect to host *** port ***: Connection timed out
Deploy Install Scripts | scp: Connection closed
Deploy Install Scripts | ##[error]Process completed with exit code 255.
```

The installer deploy **failed silently** on the most recent release. `deploy-install-scripts` runs *after* `release`, so the GitHub Release published successfully and nobody was told the installer had not been updated. Even without the `reset --hard` problem, this job is a single unmonitored SSH hop with no retry and no alert.

### Implication for ANY future security fix to the installer

> **A security fix committed to `scripts/install.sh` in this repository has a bounded, decaying, and unmonitored lifetime on the production endpoint. It reaches production only if a `v*` tag is cut AND all seven build jobs pass AND the web host's SSH port is reachable from a GitHub runner — and it survives only until the next push to `doli-network/explorer` `main`, which is currently ~2.4 pushes per day.**

Concretely: `857746b6` fixed the checksum gap on 2026-06-09. It reached production on 2026-07-22 (v6.24.0). It was gone again within days. It is gone now. **Committing a fix here is not shipping a fix.** Any remediation that does not resolve *who owns that path* will regress on the next explorer push, and nobody will notice, because nothing measures it. That is the finding, and it generalizes to `install.ps1` (same scp, same revert) and to every future installer change.

---

## 9. Impact Analysis

### 9.1 Existing code affected (if REQ-I157-007 is implemented)

| File/module | How affected | Risk |
|---|---|---|
| `../explorer/doli.network/install.sh` | Deleted from the explorer repo (ceases to be a tracked file) | **Medium** — cross-repo change; if `deploy-install-scripts` is broken at that moment, `git reset --hard` will *remove* the file and the endpoint 404s until the next successful release deploy. Ordering matters. |
| `.github/workflows/release.yml:423-445` | Becomes the sole publisher; its failure becomes a release blocker | **Medium** — it failed on v6.24.1 (SSH timeout). Must be fixed/monitored before it becomes the only writer. |
| `scripts/install.sh` | Becomes the served artifact; its `/usr/bin` target and sudoers block start reaching hosts that previously got `/usr/local/bin` and no sudoers | **Medium** — behavior change on new installs. Hosts with a stale `/usr/local/bin/doli-node` will now have a *newer* `/usr/bin` binary shadowed by an *older* `/usr/local/bin` one (`postinst.sh:52`). REQ-I157-013 must land with or before this. |
| `bins/node/postinst.sh:52,55` | Unchanged by REQ-007, but its defect becomes more visible | **Medium** |
| `crates/updater/` | Untouched by REQ-007 | Low |

### 9.2 What breaks if this changes

| Module/behavior | What happens | Mitigation |
|---|---|---|
| Hosts installed with the served copy (`/usr/local/bin`, no sudoers) | Continue to run; a future canonical install writes `/usr/bin` and adds sudoers, leaving two binaries | Fix `postinst.sh` guard (REQ-013); document the manual cleanup |
| rpm hosts | Still un-upgradable (`rpm -i` never `-U`, name schism) — the canonical installer sidesteps this by using tarball-only | Improvement, not a regression |
| Intel Macs | Still receive an aarch64 build in **both** copies (`:35` / `:36`) | Unchanged; REQ-014 (Could) |
| macOS `.pkg` users | The canonical installer is tarball-only, so `.pkg` installs stop happening via the one-liner | Behavior change — must be an explicit decision, not a side effect |
| The `doli` group / polkit setup | Canonical installer adds a polkit rule the served one does not | Additive |

### 9.3 Regression risk areas

- **Endpoint availability** — the highest risk in this whole remediation is a window where `https://doli.network/install.sh` 404s. Deleting the explorer's copy before proving the release-CI publisher works creates exactly that window.
- **`deploy-install-scripts` reliability** — one un-retried SSH hop, already observed failing. Making it the sole publisher without fixing it converts a silent staleness bug into a loud outage bug.
- **Two-binary hosts** — changing the install target from `/usr/local/bin` to `/usr/bin` on hosts that already have `/usr/local/bin/doli-node` interacts directly with the `postinst.sh:52` shadowing defect. The version an operator sees becomes PATH-dependent (already measured on jorge).
- **Cross-repo commit authority** — the fix requires a commit and push in `doli-network/explorer`, which the user must perform. A partial landing (one repo only) leaves the system in a worse state than today.

---

## 10. Regression Archeology (MANDATORY)

### 10.1 `scripts/install.sh` (this repo)

```
857746b6  2026-06-09  security: fix issue #174 — close 5 P1s (admin-RPC, SSRF, install integrity, sudo TOCTOU)
7e0e69bf  2026-05-15  fix(install): skip reinstall when version already current (INC-I-076)
c6d38713  2026-05-12  feat: package agent skills in releases and install to ~/.doli/skills/
5a9414cf  2026-05-08  fix(updater): fix "Text file busy" on self-upgrade + release v6.21.12
3a6ffa4d  2026-05-08  fix(cli): fix installation UX — no-sudo init, service pre-flight, better errors
8d779210  2026-04-06  fix(install): use tarball always, correct repo URL and sudoers paths
a0bb970d  2026-04-04  fix(updater): sudo fallback for auto-update on root-owned binary paths
ede960bd  2026-03-21  feat: zero-config producer UX — doli init, doli service, standard paths
```

### 10.2 `../explorer/doli.network/install.sh` (explorer repo)

```
7d9cb96   2026-04-12  feat: skip install if already at latest version     ← LAST CONTENT CHANGE
48183c0   2026-04-01  fix: install.sh repo URL from e-weil/doli to doli-network/doli
2b09897   2026-04-01  feat: sync with production, add From/To transaction display
```

### 10.3 Commits that created and sustained the divergence — each ruled in or out

| Commit | Date | Repo | Role | Verdict |
|---|---|---|---|---|
| `2b09897` | 2026-04-01 | explorer | Vendored a copy of the installer into the explorer repo | **CREATED the fork.** Two independent copies of the same artifact begin here. Ruled IN. |
| `48183c0` | 2026-04-01 | explorer | Fixed the repo URL `e-weil/doli` → `doli-network/doli` in the explorer copy **only** | **DIVERGENCE MARKER.** The same class of fix was applied to the explorer copy but `crates/updater/src/constants.rs:120-126` still says `e-weil/doli` today — proving the two copies were already being maintained independently. Ruled IN as evidence, not as cause. |
| `7d9cb96` | 2026-04-12 | explorer | Last content change to the served copy | **FROZE the served copy.** Everything after this date exists only in the doli repo. Ruled IN. |
| `8d779210` | 2026-04-06 | doli | "use tarball always, correct repo URL and sudoers paths" | **WIDENED the fork** — moved the canonical copy to tarball-only and `/usr/bin` while the explorer copy stayed multi-format and `/usr/local/bin`. Ruled IN. |
| `a0bb970d`, `5a9414cf` | 2026-04/05 | doli | Updater sudo fallback and Text-file-busy fix | Made the canonical installer's sudoers block load-bearing for auto-update; the served copy never got it. Ruled IN as a compounding factor. `5a9414cf` is separately the INC-I-153 **enabler** (mode inheritance). |
| `eeb765df` | 2026-05-12 | doli | **Added the `deploy-install-scripts` CI job** | **CREATED THE TWO-WRITER RACE.** Before this, the explorer was the sole (if stale) publisher. After it, two pipelines write the same path and one of them is `git reset --hard`. Ruled IN — this is the structural cause. |
| `c6d38713` | 2026-05-12 | doli | Skills packaging in the canonical installer | Widened the fork further. Ruled IN (minor). |
| `7e0e69bf` | 2026-05-15 | doli | Version-skip logic (INC-I-076) — the doli-repo equivalent of explorer's `7d9cb96` | Both copies solved the same problem separately, one month apart, with different implementations (`doli-node --version` vs `dpkg-query`/`rpm -q`). Ruled IN as proof of parallel maintenance. |
| `857746b6` | 2026-06-09 | doli | **Added the checksum verification** (ISSUE-174 #3) | **The fix that cannot stay deployed.** Reached production once (v6.24.0, 2026-07-22) and was reverted by an explorer deploy. Also the INC-I-153 **detonator** (staged mode `0o750`). Ruled IN. |
| explorer `main` × 41 | 2026-07-22 → 2026-08-08 | explorer | Each triggers `deploy.yml:18` `git reset --hard` | **SUSTAINS the divergence.** Ruled IN, collectively. |
| `52927912` | 2026-08 | doli | INC-I-153 fix (`apply.rs` mode postcondition) | Committed, **not pushed, not released** → `deploy-install-scripts` has not run for it. Unrelated to the installer divergence. Ruled OUT as a cause of INC-I-157. |

**"Pre-existing defect" is NOT the conclusion.** The divergence has a specific creation commit (`2b09897`), a specific freeze commit (`7d9cb96`), a specific structural-conflict commit (`eeb765df`), and a specific sustaining mechanism (41 `git reset --hard` deploys). Every commit in range was reviewed and assigned a role.

---

## 11. What I Don't Understand (MANDATORY — intellectual honesty)

Gaps in my understanding, stated before the proposal, because gaps here become gaps in requirements:

1. **The nginx server block for `doli.network`.** I inferred that `/var/www/explorer-repo/doli.network/` is the docroot from (a) the doli CI scp target and (b) byte-identity of served content with the explorer's tracked file. I have **not read the nginx config** (it is on a remote host; no SSH permitted). If there is a separate docroot with a symlink or an `alias`, my mechanism is still consistent but the file path may not be literal.
2. **Whether `deploy-install-scripts` and `explorer/deploy.yml` target the same host.** Both use `DEPLOY_*` secrets I cannot read. The strong circumstantial evidence is the identical `/var/www/explorer-repo` path string in both repos. Not directly confirmed.
3. **GitHub's exact policy on reoccupying `e-weil/doli`.** I confirmed the 301 is live. I did **not** verify under what conditions GitHub retires that redirect. My ranking of entry point #3 as "conditional" reflects this; treating it as a hard pin would be wrong either way, since a redirect is not a security control.
4. **Why `deploy-install-scripts` timed out on v6.24.1.** Could be a firewall change, a host restart, a runner-IP block, or a rotated key. Unknown without host access.
5. **The real host population.** §6 — genuinely unmeasurable from here.
6. **Whether any mainnet producer is currently running a binary obtained from a compromised source.** Nothing in this analysis is evidence of compromise. Absence of verification is not evidence of exploitation, and I am not claiming one.
7. **Whether `.pkg`/`.deb` artifacts carry any platform-level signature.** I read the workflow's packaging steps only via grep for signing keywords (none found). I did not read the full 574-line workflow, so an incidental `codesign`/`productsign` step could exist outside the lines I read.

---

## 12. SSF Proposal (Rule 18)

### The simplest fix that resolves the root cause

> **Delete `doli.network/install.sh` from the explorer repository, so the doli release CI's `deploy-install-scripts` job is the only writer of that path.**

This works because the divergence is not a content problem, it is an ownership problem: `git reset --hard origin/main` can only revert a file that git *tracks*, so once the explorer stops tracking it, the only remaining writer is the pipeline that publishes the checksum-verifying `scripts/install.sh` — and the served bytes become, permanently, the reviewed bytes from this repo.

That single change closes REQ-I157-001 (checksum gap), REQ-I157-006 (persistence), REQ-I157-007 (single publisher), and neutralizes attacker entry point #1's *drift* dimension — without writing a line of new verification logic, because the verification logic already exists and is already correct at `scripts/install.sh:87-109`.

### What it does NOT cover (stated separately, per protocol — not a menu)

- It does **not** fix `deploy-install-scripts` failing silently (it timed out on v6.24.1). Making the CI the sole publisher without fixing this converts silent staleness into an endpoint outage. **This must be sequenced first**: prove one green `deploy-install-scripts` run, verify the served sha256 equals `sha256sum scripts/install.sh`, and only then remove the explorer's copy.
- It does not make the 3-of-5 signature scheme real (`signatures: []`, REQ-009), nor make `doli upgrade` fail closed on signatures (REQ-008).
- It does not repoint the updater off `e-weil/doli` (REQ-010) or remove the NXDOMAIN fallback mirror (REQ-011).
- It does not fix the `postinst.sh:52` `-f` shadowing guard (REQ-013) — and that defect becomes *more* visible once the install target changes to `/usr/bin`.
- It does not address `curl | sudo sh` as a pattern (REQ-015, explicitly Won't this iteration).
- It requires a commit and push in `doli-network/explorer`, which is outside this repository and outside my authority.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one file deleted from a git repo; no runtime code path changes)
  Memory:   0 (observed — no process gains or loses allocations)
  IO:       0 (observed — nginx serves one file either way; git reset --hard touches one fewer path)
  Network:  +1 HTTP GET per install (observed — scripts/install.sh:92 fetches CHECKSUMS.txt, ~1KB, once per install; this is the restored security control, not new overhead)
  Disk:     0 (observed — 7473 bytes removed from one git tree)
  Latency:  +~200ms per install (inferred — one extra TLS round-trip for CHECKSUMS.txt, one-time at install, never on a hot path)
Inevitability: AVOIDABLE
Cheaper alternative: Copy the canonical installer's checksum block into the explorer's vendored file, leaving both copies in place.
Why this proposal anyway: The cheaper alternative preserves the two-writer race, so the two copies resume drifting the moment either is edited — which is precisely the failure this incident documents (857746b6 shipped the same control on 2026-06-09 and it is not on the endpoint today). Single ownership is the only variant where a future security fix to the installer stays deployed. The measured cost of the fix is one 1KB fetch per install.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 13. Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-I157-001 | Must | (test-writer) | §1.3, §3 | `scripts/install.sh:87-109` |
| REQ-I157-002 | Must | (test-writer) | §4 | `crates/updater/src/{verification,download,apply}.rs`, `bins/cli/src/cmd_upgrade.rs` |
| REQ-I157-003 | Must | (test-writer) | §5 | cross-cutting |
| REQ-I157-004 | Must | N/A (measurement) | §6 | none — gap |
| REQ-I157-005 | Must | (test-writer) | §7 | `crates/updater/src/apply.rs` |
| REQ-I157-006 | Must | (test-writer) | §8 | `.github/workflows/release.yml:423-445`, `../explorer/.github/workflows/deploy.yml:18` |
| REQ-I157-007 | Must | (test-writer) | §8, §12 | `../explorer/doli.network/install.sh` (deletion) |
| REQ-I157-008 | Should | (test-writer) | §4 (c5) | `bins/cli/src/cmd_upgrade.rs:70-103` |
| REQ-I157-009 | Should | (test-writer) | §4 (b4) | `.github/workflows/release.yml:475-486`, `scripts/sign-release.sh` |
| REQ-I157-010 | Should | (test-writer) | §4 (b2) | `crates/updater/src/constants.rs:120-126` |
| REQ-I157-011 | Should | (test-writer) | §4 | `crates/updater/src/constants.rs:129` |
| REQ-I157-012 | Should | (test-writer) | §8 | `.github/workflows/release.yml:423-445` |
| REQ-I157-013 | Should | (test-writer) | §5 (#5) | `bins/node/postinst.sh:52,55` |
| REQ-I157-014 | Could | (test-writer) | §3.3 | both installers, `Darwin-x86_64` case |
| REQ-I157-015 | Won't | N/A | §4 (a8) | deferred |

---

## 14. Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| 1 | `/var/www/explorer-repo/doli.network/install.sh` is the file nginx serves at `https://doli.network/install.sh` | The web server hands out the file that both pipelines write to | **Strong inference** — byte-identity of served content with the explorer's tracked copy + both repos naming the same path. nginx config not read (no SSH). |
| 2 | Both `DEPLOY_SSH_HOST` (doli) and `DEPLOY_HOST` (explorer) are the same machine | The two pipelines fight over one server | Inferred from the shared `/var/www/explorer-repo` path. Secrets not readable. |
| 3 | `git reset --hard origin/main` reverts a tracked file overwritten by an external process | Git restores every tracked file to its committed state, wiping outside edits | **Certain** — git semantics |
| 4 | No mainnet host is currently running a maliciously substituted binary | This is a missing-control finding, not a breach finding | **Not verified** — and I am not claiming it either way |
| 5 | The `~30 external producers` figure | How many machines this affects | **UNVERIFIED** — asserted, never measured |
| 6 | The node `UpdateService` has applied zero updates since `signatures: []` shipped | The advertised 6-hour auto-update has not been updating anything | **Derived from code + live artifact**, not from host logs |

---

## 15. Identified Risks

| Risk | Mitigation |
|---|---|
| Deleting the explorer's copy before `deploy-install-scripts` is proven green → `https://doli.network/install.sh` returns 404 and onboarding breaks | Sequence: fix/verify the CI job → cut a tag → confirm served sha256 == `sha256sum scripts/install.sh` → only then delete the explorer copy |
| Switching the served installer changes the install target `/usr/local/bin` → `/usr/bin` on new installs, interacting with the `postinst.sh:52` shadowing defect | Land REQ-I157-013 with or before REQ-I157-007; publish an operator note on removing stale `/usr/local/bin/doli-node` |
| The canonical installer is tarball-only; `.pkg`/`.deb`/`.rpm` one-liner installs stop | Make this an explicit decision, not a side effect |
| Cross-repo change requires the user to commit and push in `doli-network/explorer` | Flag explicitly; a partial landing is worse than the status quo |
| Making signatures real (REQ-009) without gating could brick `doli upgrade` for every host if key management is wrong | Ship signature *production* before signature *enforcement*; verify a real 3-of-5 `SIGNATURES.json` end-to-end on testnet first |
| Repointing the updater off `e-weil/doli` (REQ-010) changes a URL that already-deployed binaries have compiled in | Old binaries keep using the redirect; new ones use the correct origin. No flag day. Not a consensus change. |

---

## 16. Out of Scope (Won't — this iteration)

- **Replacing `curl \| sudo sh`** with download-verify-execute or a signed package repository (REQ-I157-015). A real fix, but it is a distribution redesign, not an incident remediation, and it would not have prevented this incident — the canonical installer already verifies.
- **Any fix, commit, deploy, or push.** The user's explicit ask was analyze → confirm → propose.
- **Any mainnet or SSH contact.** Constraint honored: no host was contacted. All live evidence is unauthenticated HTTPS to `doli.network` and `api.github.com`, plus `gh` API reads of our own CI.
- **Re-litigating INC-I-153.** Its root cause is confirmed and its fix is committed at `52927912`. §7 establishes the two incidents are distinct.

---

```
━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.93, measured — live curl + cmp byte-identity, gh run job-level CI results, git log across both repos, and file:line reads of both installers, the updater crate, and both CI workflows; the 0.07 shortfall is the unread nginx config and the unmeasurable host population)
Reasoning: DEEP triggers met — 3+ interacting components (two repos, two CI pipelines, one shared docroot, the Rust updater, packaging postinst) AND architectural issues detected (BRITTLENESS 4/5: an unowned shared-mutable path with no contract between two publishers, one of which is destructive).
━━━━━━━━━━━━━━━━━━━━━━
```
