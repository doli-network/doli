━━━ FINDINGS — 6 total (Critical:1 Major:3 Minor:2) ━━━

  [F1] CRITICAL conf(0.92, observed) — bins/cli/src/cmd_upgrade.rs:70-101 — installed base still trusts the unowned origin compiled in; signatures never block `doli upgrade`; no mitigation assigned (residual, NOT a defect in this diff)
  [F2] MAJOR conf(0.98, measured) — .claude/skills/updater/SKILL.md:32,95,133,135,139,141,143,145,147,491 — SKILL refresh was partial: 1 deleted symbol still listed as an export, 9 `download.rs:` offsets now stale by 8 or 26 lines
  [F3] MAJOR conf(0.97, measured) — crates/updater/src/constants.rs:130 + crates/updater/tests/origin_pinning.rs:4 — both cite `docs/bugfixes/inc-i-157-installer-integrity-analysis.md`, which is UNTRACKED (`git status` → `??`); the shipped comment's only explanatory pointer dangles
  [F4] MAJOR conf(0.95, observed) — crates/updater/tests/origin_pinning.rs:128-130 — guard scoped to `crates/updater/src/` only; the 5 other shipped origin definitions this same milestone repointed carry no regression guard — the exact drift shape that already recurred once
  [F5] MINOR conf(0.90, observed) — crates/updater/src/download.rs:30-36 vs :298-301 — after the mirror removal `urls_to_try` holds two byte-identical URLs on the standard `v{version}` tag; label pair implies two origins where one exists (NOT an off-by-one — the match-arm collapse is correct)
  [F6] MINOR conf(0.85, observed) — docs/legacy/implementation_distribution.md:175-176, docs/legacy/IMPLEMENTATION_PLAN_DISTRIBUTION.md:175-176 — copy-pasteable `docker pull ghcr.io/e-weil/doli-node` instructions survive in a shipped directory

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-157 M1 — ORIGIN DE-PINNING

**Reviewer pass:** 1
**Base:** `f2b66c19` (HEAD moved during the session — QA OBS-009 stands, rebase before commit)
**Scope:** full uncommitted working tree + untracked `crates/updater/tests/origin_pinning.rs`.
Unrelated concurrent-session artifacts excluded per instruction.

---

## Scope Reviewed

| Area | Files |
|---|---|
| Source | `crates/updater/src/{constants,download,lib}.rs` |
| Test | `crates/updater/tests/origin_pinning.rs` (new, 291 lines) |
| Packaging | `Cargo.toml`, `docker/docker-compose{,.devnet,.testnet}.yml` |
| Scripts | `scripts/{install.ps1,publish_release.sh,sign-release.sh,README.md}` |
| Docs/specs | `docs/{architecture,auto_update_system,buy_doli,docker,producer_node_quickstart,releases,running_a_node,testnet,troubleshooting}.md`, `specs/{engine-parts,gui-architecture}.md`, `.claude/skills/updater/SKILL.md` |
| Served assets | `testnetlinux/explorer/{index,network}.html` |
| Cross-checked, not modified | `scripts/install.sh` (already `doli-network/doli` since `48183c0`), `.github/workflows/*` (no origin literal), `Cargo.lock`, 14 sub-crate manifests (`repository.workspace = true`) |

---

## Summary

**⚠️ Approved with required in-diff fixes.**

The change is correct, minimal, and does what it claims. All 4 gates pass; the reproduction test went red→green; QA's PASS is sound and I did not overturn it. The two things that must land *inside this commit* are F2 and F3 — both trivial, both introduced by this change. F4 belongs to this milestone or an immediate follow-up. F1 is a real, live, Critical-severity exposure that this diff **cannot** fix and that currently has **no owner**; it must be filed before the incident is closed.

I found no injection pattern, no unsafe block, no unwrap regression, no error-handling regression, and no contradiction between the stated fix and the code.

---

## Answers to the Seven Judgement Questions

### 1. Root cause vs patch — **it is the root-cause fix. Do not deepen it.**

The defect statement is "the constant names a namespace the project does not own." The constants **are** the root of trust; there is no layer beneath them to fix. Verified by enumeration — `GITHUB_REPO` / `GITHUB_API_URL` / `GITHUB_RELEASES_URL` are consumed at exactly eight sites, all of which are HTTP URL construction and nothing else:

```
crates/updater/src/download.rs:35,187,300,379,383,528,551
bins/node/src/commands/misc.rs:57
```

There is no derived value, no cache, no persisted copy. Repointing the constant repoints the whole trust root.

**Making the origin configurable would be a regression, not a de-pinning.** An env-var- or config-settable update origin is a *new* hijack primitive: it converts "compromise GitHub" into "compromise any file the node reads at startup." I checked whether that hole already exists — it does not: `UpdateConfig::custom_url` defaults to `None` (`crates/updater/src/types.rs:108`) and the node hardcodes `custom_url: None` at `bins/node/src/main.rs:199`. The pin is therefore *effective*, with no config bypass. Introducing one would undo that.

**Build-time verification** (asserting in `build.rs`/CI that `GITHUB_API_URL` resolves 200 without a 301) is the only genuinely deeper mechanism, and it is the right shape for the *drift* half of the problem — a redirect-dependent origin is exactly what a "no-301" assertion catches. I do not recommend it in `build.rs` (it makes the build network-dependent and breaks offline/nix builds). I recommend it as a CI job. That is F4's territory, not a redesign.

**No `[ARCHITECTURE]` escalation.** The design is right; the constant was wrong.

**Residual for already-deployed binaries → see F1.** This is the one place where "M1's job or a later milestone's" has a non-obvious answer, and my answer is: *neither* — it is not a code milestone at all, it is an operations action, and it is more urgent than any code in this diff.

### 2. The FALLBACK_MIRROR removal — **correct, and it removed more risk than it looks.**

**Availability: no scenario is worse.** The host is NXDOMAIN (QA measured via `dig`/`host`; I confirmed nothing in-repo ever served it — the only surviving mention is a 2024-era log line in `docs/legacy/bugs/REPORT_CONSENSUS.md:288` showing it *already failing in production*). Every request to it could only ever terminate in a DNS failure. Removal is a strict availability *improvement*: it deletes one doomed DNS resolution + one doomed HTTP attempt from the worst-case `fetch_latest_release` path, and one more from `download_binary`.

The "but GitHub could go down" argument does not apply, because the constant never pointed at infrastructure the project operated. If mirror redundancy is genuinely wanted later, it is a new capability with its own trust design (independently signed metadata, not a bare hostname), not a resurrection of this constant.

**The security half is understated in the current docs.** `FALLBACK_MIRROR` sat in *two* positions, and the second was the dangerous one:

- `download_binary` — last position (index 2). Weak: a binary fetched there still faced `verify_hash`.
- `fetch_latest_release` — the **metadata** source. A `Release` deserialized from `{FALLBACK_MIRROR}/latest.json` carries an attacker-chosen `binary_url_template` **and** an attacker-chosen `binary_sha256`, and `download_binary` tries `binary_url_template` at index 0. That is a self-consistent, hash-check-passing substitution primitive from a name anyone could have registered. Removing it is the highest-value line in this diff after the constants themselves.

**Off-by-one / mislabel: none.** I traced it by hand. Post-change `urls_to_try` = `[release.binary_url_template, "{GITHUB_RELEASES_URL}/v{version}/doli-node-{platform}"]`. Index 1 is genuinely the GitHub-constant-derived entry, so `0 => "primary", _ => "GitHub"` is exactly right, and the `_` arm is unreachable for any index > 1 because no third push exists. The real labelling problem is the opposite one, and it is F5: on the standard tag shape those two strings are *identical*, so the log implies two origins where there is one.

### 3. Completeness — **the four deliberate non-changes: I agree with three, and partially disagree with one.**

| Non-change | My verdict |
|---|---|
| `docs/audits/security-audit-issue-174-2026-06-08.md:55` | **Agree, strongly.** A dated audit finding is a claim about the world *on that date*. Row NEW-4 recorded "install.sh uses `doli-network/doli`; updater uses `e-weil/doli` — different trust roots, P3". Rewriting it would erase the single strongest piece of evidence that this defect was *seen and under-prioritised* eight weeks before it detonated. The correct treatment of a stale audit row is a resolution note elsewhere, never an edit in place. |
| `docs/bugfixes/inc-i-157-installer-integrity-analysis.md` | **Agree.** It is this incident's own diagnosis. Its `Implementation Module` column holds defect *locations* (`constants.rs:120-126`), not developer fill-ins; post-fix line numbers there would make the record self-refuting. This matches the decision already recorded in memory.db. **But it must be committed — see F3.** |
| `testnet*/bin/*` | **Agree.** Compiled artifacts, local-testnet only, self-heal on the normal `cp target/release/...` workflow (QA OBS-002). Rebuilding a committed binary to fix a string is not a fix, it is churn. The right answer is the one QA already stated: stop version-controlling binaries. |
| `docs/legacy/**` | **Partially disagree — see F6.** "Historical record" is the correct rule for prose that *describes* the past. Lines 175-176 of both files are not prose; they are a copy-pasteable `docker pull` command with a `[x]` next to it. A shipped, executable instruction pointing at a re-registrable namespace is a hazard regardless of the folder it lives in. The fix that preserves the record *and* removes the hazard is a one-line `> SUPERSEDED — historical checklist, do not execute` banner at the top of each file. |

**What got missed:** F2 (SKILL.md), F3 (untracked pointer), F4 (guard scope). **What went too far:** nothing. I checked specifically for over-reach and found none — every edited line named the release origin or the removed mirror, and `Cargo.toml`'s `repository` field, `scripts/install.ps1`, `publish_release.sh` and `sign-release.sh` are all genuinely part of the origin surface, not collateral.

### 4. Test quality — **it pins the invariant it targets, and cannot be trivially defeated within its scope. Its scope is the problem.**

Strengths, and they are real: the value assertions (`P1`/`P2a`/`P3a`) are exact-match/contains, not regex-fuzzy; the negative assertions (`P2b`/`P3b`) are independent of the positive ones, so a string containing *both* namespaces fails; the `src/` walk is recursive, so a new module under `crates/updater/src/` is covered automatically; and the empty-file-list sanity check at `origin_pinning.rs:155-161` is the detail that stops the classic "the scan silently found nothing because the path was wrong" false pass. That last one is genuinely good engineering — most source-scan tests omit it.

The `src/`-only scoping is the correct answer to the *self-match* problem. It is the wrong answer to the *coverage* problem, and the gap is not hypothetical:

- The same milestone repointed **five other shipped origin definitions** — `scripts/install.ps1:7`, `scripts/publish_release.sh:22`, `scripts/sign-release.sh:26`, `docker/docker-compose{,.devnet,.testnet}.yml:6-7`, `Cargo.toml:15`. None is guarded. All 5 tests pass if any of them regresses.
- A reappearance in **another crate** (e.g. a future `bins/cli` hardcode) or in **`crates/updater/tests/`** itself is invisible.
- This is precisely the shape that already recurred: `scripts/install.sh` was fixed on 2026-04-01 (`48183c0`) and `constants.rs` then drifted for four months, undetected, until this incident (analysis doc line 518 calls it the "divergence marker"). A guard that covers one of the two divergent copies does not prevent divergence.

Fix (F4): replace the `CARGO_MANIFEST_DIR/src` walk with a repo-root walk (locate the root by walking up to the directory containing `Cargo.toml` + `.git`), scanning `crates/`, `bins/`, `scripts/`, `docker/`, `specs/`, `*.toml`, `*.yml`, `*.html`, with an **explicit, commented allowlist** of historical-record paths: `docs/legacy/`, `docs/audits/`, `docs/bugfixes/`, `docs/qa/`, `docs/reviews/`, `testnet/bin/`, `testnetlinux/bin/`, and the test file itself. An allowlist is auditable; a scope restriction is not — a reader of the current test cannot tell whether `scripts/` is uncovered by decision or by oversight.

### 5. The comment cannot name the namespace — **acceptable, and arguably better. Conditional on F3.**

The tradeoff is real but the loss is small and partly a gain:

- The comment at `constants.rs:118-131` states the *invariant* ("must name a namespace the project actually controls"), the *mechanism* ("a rename-redirect is NOT a security boundary; it lapses the instant the abandoned namespace is re-registered"), and the *prohibition* ("never rely on a redirect"). That is everything a future maintainer needs to avoid repeating the mistake. The specific string `e-weil` is forensics, not guidance.
- Naming a currently-unregistered, hijackable GitHub username in a public source comment is a mild attacker convenience — it publishes the exact account to squat. Omitting it is defensible on its own merits, independent of the test constraint.

**But the omission makes the doc pointer load-bearing**, and that pointer currently dangles (F3). Fix F3 and this is a clean tradeoff. Leave F3 unfixed and this is a genuine documentation regression, because the reader loses both the name *and* the path to the name.

If you later adopt F4's allowlist form, the constraint disappears (the needle can be assembled as `concat!("e-", "weil")` and the test file allowlisted), and naming it becomes a free choice rather than a forced one. I still recommend not naming it.

### 6. Specs/docs drift — **thorough everywhere except the one file that is a line-number index.**

Verified accurate after the change:

| File | Check |
|---|---|
| `docs/architecture.md:391,404` | Now says "GitHub repo/API/releases URLs (owned namespace, INC-I-157)" and "no fallback mirror (removed in INC-I-157)". Matches `constants.rs` exactly — the module table row no longer claims a `fallback mirror` field that does not exist. ✅ |
| `docs/auto_update_system.md:891,1244` | Origin corrected; the `FALLBACK_MIRROR` line deleted from the constants code block. The remaining block matches `constants.rs:135` byte-for-byte. ✅ |
| `specs/engine-parts.md:2612` | `fetch_latest_release()` description now "custom URL → GitHub API", matching `download.rs:104-146` (custom → GitHub → `Ok(None)`). ✅ |
| `specs/gui-architecture.md:1139` | Tauri updater endpoint repointed. ✅ |
| `docs/{buy_doli,docker,producer_node_quickstart,releases,running_a_node,testnet,troubleshooting}.md`, `scripts/README.md` | All `git clone`, `curl -LO`, `raw.githubusercontent.com` and `ghcr.io` references repointed; verified zero `e-weil` survivors outside the allowlisted historical set. ✅ |
| `docs/DOCS.md` / `specs/SPECS.md` | No index update needed — `docs/DOCS.md` does not index `docs/bugfixes/` (0 matches) and no doc/spec file was added or renamed. ✅ |

**`.claude/skills/updater/SKILL.md` is the exception — see F2.** The prompt asked me to check whether its cited line numbers survived the edits. They did not. Three were refreshed and nine were not, which is worse than refreshing none: a partially-refreshed index reads as verified.

### 7. Consensus safety — **CONFIRMED. No activation height. No synchronized deploy. Rolling deploy is safe.**

Applying CLAUDE.md's three-question checklist, with the evidence for each:

1. **Can a user-submittable tx reach this path?** **NO.** `crates/core/src/transaction/types.rs` contains no `Update`/`Upgrade` variant among its 24 `TxType`s (grep for `Update|Upgrade` → zero matches). No transaction validation, mempool, or `apply_block` path links against `crates/updater`.
2. **Can a producer-action or attestation pattern reach it?** **NO, in the consensus sense.** Update governance *does* have a producer-facing surface — `VoteMessage` is gossiped and handled at `bins/node/src/node/network_events.rs:506` and `bins/node/src/node/startup.rs:454`, tallied in `bins/node/src/updater/service.rs:266`. But votes are **network messages, never block content**: nothing from the updater is serialized into a header, a coinbase, a bitfield, `presence_root`, or any of the three states. And the URL constants are unreachable from the vote path regardless — they are consumed only by HTTP construction (the eight sites listed in §1).
3. **Is the new behavior bit-identical for all reachable inputs?** **YES for every consensus-visible output** (there are none). For *non*-consensus behavior it is not bit-identical, and correctly so: the fetch target changes and one fallback attempt disappears.

**(1) NO + (2) NO → no activation height.** Confirmed, refuting nothing.

Second deploy question (INC-I-062 / INV-8): does this change **block CONTENT**? **NO** — no bitfield, coinbase, tx ordering, `presence_root` or header field is touched. **No synchronized deploy required.**

One nuance the runner did not state, and should: mixed-fleet safety currently holds *because* the 301 is alive — old binaries (`e-weil`) and new binaries (`doli-network`) resolve to the same repository, so they see the same releases and converge on the same version. **If the `e-weil` namespace is claimed, that stops being true** and the fleet splits into two update channels. That is not a consensus fork, but it is a fleet-management hazard, and it is the second reason F1 is urgent.

---

## Findings

### F1 — CRITICAL: installed-base exposure survives M1 and has no assigned owner

- **Location:** `bins/cli/src/cmd_upgrade.rs:70-101`; every deployed binary ≤ v6.24.1
- **Category:** security / supply-chain
- **Evidence:**
  - `bins/cli/src/cmd_upgrade.rs:70` — `// Check maintainer signatures (informational — never blocks manual upgrade)`. Lines 82-101 print `Warning:` / `Note:` on `InsufficientSignatures`, on verification failure, and on a missing `SIGNATURES.json`, and **never return `Err`**. The only hard gate on that path is `verify_hash` at `:66`, whose expected hash comes from the *same release* as the artifact.
  - The origin constants are compile-time `&'static str` (`crates/updater/src/constants.rs:132,135,138`). No runtime override exists (`UpdateConfig::custom_url` is `None` at `crates/updater/src/types.rs:108` and `bins/node/src/main.rs:199`).
  - Incident record: `docs/bugfixes/inc-i-157-installer-integrity-analysis.md:627` already anticipates this — *"Old binaries keep using the redirect; new ones use the correct origin. No flag day."* That is true and it is exactly the problem: the old binaries never stop using it.
- **Impact:** For every host currently running DOLI, the update trust root is still `e-weil` — a GitHub username that measurement shows is **unregistered** (`github.com/e-weil` → 404, `api.github.com/users/e-weil` → 404). Anyone who registers that username and creates a repo named `doli` retires the rename-redirect and captures the update origin of the entire deployed fleet. On the `doli upgrade` path, signature verification does not stop them; `CHECKSUMS.txt` comes from their own release, so `verify_hash` passes. This diff cannot reach those binaries.
- **Suggested fix:** **Defensively register the GitHub account `e-weil` and hold it.** It is free, takes minutes, requires no code, no deploy and no consensus consideration, and it is the *only* control that protects hosts already in the field. Do it before this milestone closes. Secondarily, record it as an owned operational asset (alongside the domain registrations) so it is never released, and file the fleet-rebuild half (getting every host onto a new-origin binary) as its own tracked item.
- **Test Strategy:** NOT_TESTABLE in-repo — the property is "an external namespace is under our control", which no unit test can assert. The CI-level proxy is the F4 job: assert `https://api.github.com/repos/e-weil/doli` still returns `301` *and* that `github.com/e-weil` is owned by us; alert on either changing.
- **Blocking status:** does **not** block merging this diff (the diff strictly improves the situation). **Does** block closing INC-I-157.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — account registration, no code path)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed — no change to any runtime request)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: no code change can reach an already-deployed binary; namespace custody is the only control that covers the installed base
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F2 — MAJOR: `.claude/skills/updater/SKILL.md` refresh is partial — 1 dead symbol + 9 stale line offsets

- **Location:** `.claude/skills/updater/SKILL.md:32,95,133,135,139,141,143,145,147,491,568`
- **Category:** docs-drift
- **Evidence:** measured — `grep -n '^pub async fn \|^pub fn \|^async fn \|^fn ' crates/updater/src/download.rs` against the SKILL's cited offsets:

| SKILL says | Actual | Delta |
|---|---|---|
| `:32` lists `FALLBACK_MIRROR` as an exported constant | symbol deleted; `crates/updater/src/lib.rs:66` no longer re-exports it | dead symbol |
| `:95` `GithubReleaseInfo` (`download.rs:355`) | `download.rs:329` | −26 |
| `:133` `download_from_url` (`download.rs:72`) | `download.rs:64` | −8 |
| `:135` `verify_hash` (`download.rs:91`) | `download.rs:83` | −8 |
| `:139` `fetch_from_github` (`download.rs:205`) | `download.rs:179` | −26 |
| `:141` `fetch_github_release` (`download.rs:390`) | `download.rs:364` | −26 |
| `:141`,`:491` `platform_target_triple` (`download.rs:373`) | `download.rs:347` | −26 |
| `:143` `download_signatures_json` (`download.rs:547`) | `download.rs:521` | −26 |
| `:145` `download_checksums_txt` (`download.rs:570`) | `download.rs:544` | −26 |
| `:147` `parse_iso8601_timestamp` (`download.rs:496`) | `download.rs:470` | −26 |

  Correctly refreshed: `:131` (`download.rs:23` ✅), `:137` (`download.rs:104` and `:151` ✅), `:568` (`constants.rs:132` ✅). The deltas are exactly the −8 and −26 line removals this diff made, confirming every one of these was *accurate before* the change and was broken *by* it.

  Separately, `:568` compresses a causal claim: *"the dangling `releases.doli.network` fallback … fed `binary_url_template`, which `download_binary` tries FIRST."* This is true only via the two-step path (`fetch_latest_release` → mirror `latest.json` → poisoned `binary_url_template` → tried at index 0). In `download_binary` itself the mirror was **last** (index 2), not first. The mechanism is real and the removal is justified — but as written the sentence tells a future reader the mirror sat at position 0 of the download list, which it did not.
- **Impact:** `SKILL.md` exists specifically as a grep-first `keyword→file:line` index (per `CLAUDE.md`'s skills map). Stale offsets are the exact failure mode it is built to prevent, and a *partial* refresh is worse than none — three corrected entries signal that the section was verified. `:32` is worse still: it sends an agent looking for a symbol that no longer compiles.
- **Suggested fix:** delete `FALLBACK_MIRROR` from the `:32` constants list; recompute the nine `download.rs:` offsets (all are −8 or −26 from the current text); reword `:568` to *"the mirror was the last download fallback **and** the last metadata source — and a `Release` fetched from it supplies `binary_url_template`, which `download_binary` tries first, so the metadata position was the real hijack primitive."*
- **Test Strategy:** NOT_TESTABLE as a unit test in a reasonable budget. Cheap CI proxy: a script that extracts every `` `path:NNN` `` pair from `.claude/skills/**/SKILL.md` and asserts the cited line exists and contains the cited identifier. That is a general-purpose guard worth more than this one fix.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — markdown edit, no runtime path)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (observed — no network surface)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: strip line numbers from the SKILL entirely and rely on symbol-name grep
Why this proposal anyway: the line numbers measurably shorten agent lookups on a 588-line file; correctness of the index is what makes them worth keeping, and the CI check makes the correctness durable rather than manual
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F3 — MAJOR: shipped source comment and test cite an UNTRACKED file

- **Location:** `crates/updater/src/constants.rs:130`; `crates/updater/tests/origin_pinning.rs:4`
- **Category:** docs-drift / traceability
- **Evidence:**
  - `crates/updater/src/constants.rs:130` — `/// See docs/bugfixes/inc-i-157-installer-integrity-analysis.md.`
  - `crates/updater/tests/origin_pinning.rs:4` — `//! (measured, see docs/bugfixes/inc-i-157-installer-integrity-analysis.md …)`
  - `git status --short` → `?? docs/bugfixes/inc-i-157-installer-integrity-analysis.md` — **untracked.**
  - `git ls-files docs/bugfixes | wc -l` → `43`. The directory is tracked and 43 sibling analyses are committed, so this is an omission from the change set, not a `.gitignore` policy.
- **Impact:** Compounds F5-of-the-prompt (§5 above). Because the comment deliberately does not name the namespace, this path is the **only** route from the source file to the reason the invariant exists. If the commit lands without the doc, `constants.rs` ships a dangling reference and a future maintainer reads "an abandoned personal namespace" with no way to learn which one, why, or what was measured — and the natural failure mode is that they assume the comment is boilerplate and eventually relax it.
- **Suggested fix:** `git add docs/bugfixes/inc-i-157-installer-integrity-analysis.md` and include it in the same commit as the code. (Reviewer does not stage; this is the developer's action.) Verify by `git ls-files --error-unmatch` on the path before committing.
- **Test Strategy:** cheap and worth it — extend `origin_pinning.rs` with a test that asserts every `docs/…\.md` path cited in `crates/updater/src/**/*.rs` exists on disk relative to the repo root. That guards the whole class, not just this instance. Prove it fails today by running it before staging the doc.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one extra file in the commit; the optional test is compile-time/test-time only)
  Memory:   0 (observed)
  IO:       +O(cited paths) test-time `stat` calls (observed — single-digit count, test binary only)
  Network:  N-A (observed)
  Disk:     +~30KB repo size (observed — one markdown file)
  Latency:  0 (observed — no runtime path)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: a source comment that points at a file not in the repository is strictly worse than no comment; there is no cheaper way to make the pointer resolve than committing the file it points at
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F4 — MAJOR: origin-pinning guard covers one of six shipped origin definitions

- **Location:** `crates/updater/tests/origin_pinning.rs:128-130` (`updater_src_dir()`); uncovered: `scripts/install.ps1:7`, `scripts/publish_release.sh:22`, `scripts/sign-release.sh:26`, `docker/docker-compose.yml:7`, `docker/docker-compose.devnet.yml:7`, `docker/docker-compose.testnet.yml:6`, `Cargo.toml:15`, `scripts/install.sh:4`
- **Category:** tech-debt / regression-guard gap
- **Evidence:**
  - `origin_pinning.rs:128-130` — `Path::new(env!("CARGO_MANIFEST_DIR")).join("src")`. All five tests read only that subtree or the three `pub const` values.
  - The same milestone repointed the eight locations listed above; a regression in any of them passes the full suite.
  - Recurrence proof, not speculation: `docs/bugfixes/inc-i-157-installer-integrity-analysis.md:518` records `48183c0` (2026-04-01) fixing `install.sh`'s repo URL while `constants.rs` kept `e-weil` — and calls it the *"DIVERGENCE MARKER … proving the two copies were already being maintained independently."* Four months of undetected drift between exactly two of these files is what produced this incident. A guard on one copy does not prevent divergence between copies.
  - The `src/`-only scoping is itself sound *for its stated purpose* (self-match avoidance, documented at `:118-127`) — the defect is that the scope was chosen to solve the self-match problem and then silently accepted as the coverage boundary.
- **Impact:** The milestone's central claim is "the release origin is pinned to a controlled namespace." That claim is enforced for the Rust constants and unenforced for the Windows installer, both release-publishing scripts, all three compose files and the crate manifest. Any of those regressing reintroduces a split trust root with no signal.
- **Suggested fix:** widen the scan to a repo-root walk. Locate the root by ascending from `CARGO_MANIFEST_DIR` to the directory containing both `Cargo.toml` and `.git`; scan `crates/`, `bins/`, `scripts/`, `docker/`, `specs/`, plus root `*.toml`; use an explicit, commented allowlist for historical records (`docs/legacy/`, `docs/audits/`, `docs/bugfixes/`, `docs/qa/`, `docs/reviews/`, `testnet/bin/`, `testnetlinux/bin/`, and this test file). Build the needle as `concat!("e-", "weil")` so the allowlist is a policy statement rather than a self-match workaround. Optionally add the CI job from §1: assert `api.github.com/repos/doli-network/doli` returns 200 with **no** 301 hop, which catches "the origin became redirect-dependent again."
- **Test Strategy:** prove the gap first — temporarily set `scripts/install.ps1:7` back to the old namespace, run `cargo test -p updater`, observe **5 passed / 0 failed**. That is the FAIL evidence. Then land the widened walk and observe it fail on the same mutation, and pass once reverted.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(repo text files) once per `cargo test -p updater` run (observed — ~2k files, string `contains`; single-digit ms)
  Memory:   +peak file size per iteration, files read and dropped one at a time (observed — bounded by the largest scanned file, <1MB)
  IO:       +~2k `read_to_string` calls per test run (observed — test-time only, never in the node)
  Network:  0 (observed — the optional CI redirect probe is a CI job, not a test)
  Disk:     0 (observed)
  Latency:  0 (observed — no runtime path; test-suite wall clock +<100ms)
Inevitability: AVOIDABLE
Cheaper alternative: a `grep -rn` line in the pre-commit hook or CI instead of a Rust test
Why this proposal anyway: the cheaper form lives outside the crate and is skippable with `--no-verify`; the measured recurrence (4 months of undetected divergence between two copies of this same constant) is the concrete cost of a guard that can be bypassed or forgotten. Test-time cost is invisible against a workspace build.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F5 — MINOR: `download_binary` now retries a byte-identical URL and labels it as a second source

- **Location:** `crates/updater/src/download.rs:30-36` (construction), `:40-44` (labels), `:298-301` (template origin)
- **Category:** code-quality / observability
- **Evidence:**
  - `download.rs:298-301` — `fetch_from_github()` sets `binary_url_template: format!("{}/{}/doli-node-{{platform}}", GITHUB_RELEASES_URL, tag_name)`.
  - `download.rs:33-36` — pushes `format!("{}/v{}/doli-node-{}", GITHUB_RELEASES_URL, release.version, platform)`.
  - For the project's tag convention `tag_name == "v{version}"` (confirmed by `download.rs:204` `tag.strip_prefix('v')` and by `cmd_upgrade`/`download_signatures_json` both reconstructing `v{version}`), these two strings are **identical**. `urls_to_try` is therefore `[X, X]` on the dominant path.
  - The loop at `:40-57` logs the second attempt as `source = "GitHub"` while the first was `"primary"`.
- **Impact:** No correctness bug. Two effects, both small: a failed download costs two full attempts against the same dead URL (up to 2 × the 300 s timeout at `:66`, so a worst case that doubled from ~10 min to… no — it was already 3 entries, so this *reduced* it; the point is the remaining redundancy is nominal), and the log line asserts a fallback that does not exist, which will mislead exactly the person debugging an update failure. Note this interacts with QA OBS-003: both entries 404 against real release assets, so an operator debugging `doli-node update apply` sees "primary failed, GitHub failed" and reasonably concludes two independent origins are down.
- **Suggested fix:** skip the second push when it equals `urls_to_try[0]` (`if !urls_to_try.contains(&github_url)`), or drop the index→label `match` and log the URL's role derived from whether it came from the template or the constant. Either way the doc comment at `:20-22` should stop calling it an ordered *source* list.
- **Test Strategy:** unit test `download_binary`'s URL-list construction (extract it into a small pure `fn build_url_candidates(release, platform) -> Vec<String>`), assert `len() == 1` for a `Release` whose `binary_url_template` was produced by `fetch_from_github`, and `len() == 2` for one with a genuinely distinct template. This also closes QA OBS-005(a).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      −negligible (observed — one fewer `format!` + one `Vec` compare on a cold path)
  Memory:   −one `String` (~80 bytes) per download attempt (observed)
  IO:       0 (observed)
  Network:  −1 HTTP request per failed download (observed — the duplicate attempt is eliminated)
  Disk:     0 (observed)
  Latency:  −up to 300 s on the failure path (observed — one `download_from_url` timeout at `download.rs:66` removed)
Inevitability: AVOIDABLE
Cheaper alternative: leave the duplicate and only fix the log label
Why this proposal anyway: the label-only fix keeps a guaranteed-redundant 300 s timeout on the operator-visible failure path; deduplicating costs one comparison and removes it
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### F6 — MINOR: executable `docker pull ghcr.io/e-weil/doli-node` survives in `docs/legacy/`

- **Location:** `docs/legacy/implementation_distribution.md:175-176`; `docs/legacy/IMPLEMENTATION_PLAN_DISTRIBUTION.md:175-176`
- **Category:** security (low) / docs
- **Evidence:** `grep -rnI "e-weil" --exclude-dir=.git --exclude-dir=target .` — after excluding the test file, the audit record, the diagnosis doc, the QA report and committed binaries, these four lines are the only survivors. Both read `- [x] \`docker pull ghcr.io/e-weil/doli-node:latest\` works`.
- **Impact:** Low but non-zero, and asymmetric with the other "historical" exclusions: this is an instruction to *run something*, not a description of the past. Mitigating and measured: `ghcr.io/e-weil/doli-node` has never existed (anonymous GHCR token probe → 403 for both namespaces; control `astral-sh/uv` → 200; `.github/workflows/ci.yml:119-125` has `push: false`), and the `[x]` marks are therefore false records of a check that never passed. So the hazard is "namespace could be squatted later," not "malicious image today."
- **Suggested fix:** prepend one line to each file — `> **SUPERSEDED (INC-I-157).** Historical planning checklist. Do not execute these commands; the namespaces and image references are obsolete.` This preserves the record intact (no line is altered) while removing the executable hazard, and it is consistent with the correct decision to leave the dated audit row untouched.
- **Test Strategy:** covered by F4's widened scan if `docs/legacy/` is allowlisted *and* the allowlist entry carries the banner requirement as a comment; otherwise NOT_TESTABLE (banner presence is a stylistic property). A grep asserting each `docs/legacy/*.md` starts with a `> **SUPERSEDED` line is a reasonable, cheap CI check if the team wants it.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — markdown-only)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (observed)
  Disk:     0 (observed — two added lines)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: delete the two `docker pull` lines outright
Why this proposal anyway: deleting them edits a historical record, which is exactly the practice this milestone correctly refused for the audit row; a banner removes the hazard without falsifying the record
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Observations (deliberately NOT raised as findings)

- **`crates/updater/src/download.rs` is 587 lines, over the 500-line source budget (CLAUDE.md rule 19).** Raising this against a diff that *reduced* the file by 26 lines would be perverse. Noted so the budget is not silently forgotten; the natural split is `download.rs` (transport) / `github.rs` (API parsing), and it belongs to whoever next touches this file substantively.
- **QA's OBS-001 through OBS-009 all reproduce.** I re-derived OBS-003 (`download_binary` synthesizes `doli-node-{platform}` while published assets are `doli-v{ver}-{triple}.tar.gz`) and OBS-004 (`bins/node/src/commands/misc.rs:57` does not normalize the `v` prefix while `download.rs:544-549` does) from the code and agree both are pre-existing and correctly out of M1 scope. I did not duplicate them as findings — they are already recorded in `docs/qa/inc-i-157-M1-qa-report.md`. They should become their own incident.
- **The `testnetlinux/explorer/*.html` fix was correct and QA's overturn of the runner's "runtime fixture" classification was right.** A link labelled "Source" in a page served by `doli-explorer.service` is a social-engineering surface, and fixing it in the milestone that fixes exactly that class is the consistent call.
- **Intellectual-honesty check: no contradiction found.** The stated fix ("repoint the origin to an owned namespace, remove a dangling fallback") matches the code exactly — the diff touches nothing else, adds no retry loop, no flag, no workaround. The upstream diagnosis doc is internally consistent: it ranks the namespace reoccupation as *"CONDITIONAL — precondition not currently met"* and the fix does not claim otherwise. No `CONTRADICTION DETECTED`.
- **No injection surface.** Grepped the diff for f-string/format-into-SQL, `subprocess`/`Command` with interpolation, `eval`/`exec`: none. The only interpolation added or retained is `format!` into HTTP URL strings from compile-time constants and a version string that GitHub itself returned — no external, untrusted operand reaches a shell or a query.

---

## System Impact

This diff **removes** a protection-adjacent mechanism (a download fallback) rather than adding one, and adds no rate limit, backoff, blacklist, watchdog, cap, or circuit breaker. No `protection_mechanisms` registration is required. I queried `v_protection_surface` (25 active mechanisms): none shares a trigger surface with the updater's HTTP path — the registry is entirely consensus, sync, gossip, memory and installer-logrotate mechanisms. No interaction, no feedback loop, no starvation path. No numeric threshold was introduced, so the scale-sensitivity rule is vacuous here.

---

## Specs/Docs Drift

Only one file remains out of sync with the code after this change: `.claude/skills/updater/SKILL.md` (F2). Everything else in `docs/` and `specs/` was verified against the source and matches (see §6).

---

## Modules Not Reviewed

None within scope. The diff is 25 files / 74 insertions / 93 deletions and was reviewed in full, including the untracked test. Deliberately not reviewed: the concurrent-session artifacts the brief excluded, and `.claude/worktrees/**` (separate worktrees, not this change — they still carry the old constants and will pick up the fix on their next rebase).

---

## Final Verdict

**Requires one iteration — then approved.**

- **Fix inside this commit:** F2 (SKILL.md — dead symbol + 9 offsets) and F3 (`git add` the analysis doc). Both are mechanical, neither touches source logic, neither re-opens the gates.
- **This milestone or the immediate next:** F4 (widen the guard). It is the difference between "we fixed the constant" and "the constant cannot silently drift again," and the incident record proves that distinction is not academic.
- **Before INC-I-157 closes:** F1 — register and hold the `e-weil` GitHub account. No code milestone can substitute for it.
- **Opportunistic:** F5, F6.
- **Consensus:** no activation height, no synchronized deploy, rolling deploy safe. Confirmed against all three checklist questions with per-question evidence (§7).

The change itself is good work: minimal, correctly scoped, honest about what it does not fix, and it removed a metadata-source hijack primitive that the requirement text ("no non-resolving hostname") undersells.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — the diff changes `&'static str` values and deletes one const; no algorithm, no loop, no allocation changed)
  Memory:   −~30 bytes static rodata (observed — one deleted `&'static str` and its two format sites)
  IO:       0 (observed)
  Network:  −1 DNS resolution and −2 HTTP attempts per failed update cycle (observed — one NXDOMAIN lookup for `releases.doli.network` plus its `latest.json` fetch in `fetch_latest_release` and its binary fetch in `download_binary`)
  Disk:     0 (observed)
  Latency:  −up to one DNS timeout + one 300 s `download_from_url` timeout on the update-failure path (observed — `crates/updater/src/download.rs:66`); 0 on every success path
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: this is the aggregate cost of the change under review, not a proposed addition — repointing a compile-time constant and deleting a dead one is the minimum possible runtime footprint for correcting a trust root, and the deltas are all reductions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: (1) the change alters the software supply-chain trust root — `crates/updater/src/constants.rs:132,135,138` is the origin every auto-update and every `doli upgrade` resolves binaries from; (2) external data ingestion — `crates/updater/src/download.rs` deserializes attacker-reachable JSON (`Release`, `SignaturesFile`, `ReleaseMetadata`) fetched from that origin and derives the first download URL from it; (3) cryptographic verification path — the removed `FALLBACK_MIRROR` sat in the metadata chain that supplies `binary_sha256`, and `bins/cli/src/cmd_upgrade.rs:70-101` shows maintainer-signature checking is advisory, so origin custody is the effective sole control on that path; (4) deploy/enforcement surface — release-publishing scripts (`scripts/publish_release.sh`, `scripts/sign-release.sh`, `scripts/install.ps1`) and container image references were repointed in the same change; (5) an unresolved Critical residual (F1) affecting the entire installed base.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
