# INC-I-214 — Release signing cannot follow the runbook (Analyst, run 548)

## Scope
`bins/cli/src/{commands.rs,cmd_governance.rs,cmd_upgrade.rs,main.rs}`; `crates/updater/src/{download.rs,trust_root.rs,install_gate.rs}` (read-only);
`scripts/{sign-release.sh,publish-release.sh,monitor-release-signed.sh}`; `.claude/skills/release/SKILL.md`.
Out of scope: `doli-node`, consensus, `install_gate.rs` (11 dependents — must not change).

## Summary (plain language)
Three independent breaks stop the documented draft→sign→verify→promote order. (1) `doli release sign` insists on fetching CHECKSUMS.txt from the PUBLIC download URL, which 404s for a draft — even though the calling script already downloads that exact file with an authenticated `gh` 40 lines later. (2) `doli release verify` on the dev Mac resolves a STALE local on-chain maintainer snapshot (pre-2026-08-30 rotation), reports 0/3, and offers no way to say "use the compiled keys". (3) `publish-release.sh` never clears the prerelease flag, so promotion 422s. Fix: a local-file input for `sign`, an explicit bootstrap trust-root switch for `verify`, one flag on the promote call.

## Anchor detection (no skeptic report present)
FIRST READ: "the CLI cannot reach a draft; it needs an authenticated GitHub fetch." Set aside. SECOND: "the CLI should not be fetching at all — the caller already holds the authenticated bytes on disk; the gap is an INPUT seam, not a network capability." Chosen: the second. Evidence — `release verify` already has `--dir`, commented *"Required for a DRAFT release: the unauthenticated GitHub API cannot see one"* (`commands.rs:980-983`), and `sign-release.sh:99-101` already runs
`gh release download --pattern CHECKSUMS.txt`. The codebase already picked the local-input seam on the verify side.

## Architecture Context

### Module boundaries
- `sign-release.sh` — orchestrator; owns `gh` auth. Depends on `gh`, `jq`, `doli release sign` (argv + **stdout scrape**).
- `publish-release.sh` — the only promoter. Depends on `gh`, `jq`, `doli release verify --dir`.
- `monitor-release-signed.sh` — read-only health check. Depends on `gh`, `doli release verify` (**no `--dir`**).
- `cmd_release_sign` (`cmd_governance.rs:20-77`) — validates version, loads key, **fetches** CHECKSUMS.txt, signs, prints JSON.
- `cmd_release_verify` (`cmd_upgrade.rs:307+`) — resolves trust root, then `verify_manifest_dir(dir)` or fetch-from-GitHub.
- `updater::download.rs` — `download_checksums_txt()` → `download_from_url()`: bare reqwest, **no Authorization header**.
- `updater::install_gate::verify_release_manifest` — L1 version bind, L2 `sha256(bytes)` bind, L3 k-of-n distinct signers.
- `updater::trust_root` — `TrustRoot::{bootstrap,on_chain,resolve}`, provenance-carrying, `is_usable()` fail-closed.

### Data flow (sign → manifest → verify → promote)
CI creates DRAFT + CHECKSUMS.txt → `sign-release.sh` → per key `doli release sign` → *fetch* CHECKSUMS.txt → sha256 → sign
`"{version_bare}:{sha256}"` (`verification.rs:33`) → JSON on stdout → script scrapes, assembles SIGNATURES.json, uploads to
the draft → `publish-release.sh` downloads both assets → count ≥ THRESHOLD → `verify --dir` → `verify_release_manifest`
(L1/L2/L3) → `gh release edit --draft=false --latest`.

### Constraints and invariants (must survive the fix)
- **The signed operand is `"{version}:{sha256(CHECKSUMS.txt BYTES)}"`, hashed from bytes, never read from a field.** `install_gate` L2 exists because a hash trusted from a field is an unbound operand (INC-I-172 F1).
- **`TrustRoot` never fails open.** Non-empty on-chain set is authoritative; empty + height>0 fails closed; `Bootstrap` is reachable only via the never-bootstrapped branch (`trust_root.rs:119-160`).
- **Nothing leaves DRAFT before signatures verify** (INC-I-202).
- **`release sign` must keep `validate_release_version()`** — a free-form `--version` can mint a governance authorization (`add:`/`remove:`/`activate:` share the interpolation, AUDIT-P0-011).

### Blast radius (code graph `graphify-out/graph.json`, blast.py --hops 2)
- `resolve_upgrade_trust_root` → **2 dependents: `cmd_release_verify` AND `cmd_upgrade`**. **Hard constraint:** a trust-root override goes at the `cmd_release_verify` call site only; changing the shared resolver would hand it to the INSTALL path.
- `verify_release_manifest` → 11 dependents (`verify_release_artifact` + 10 INC-I-172 binding tests). Do not touch.
- `sign_release_hash` → 8 dependents (all tests). Signature construction is unchanged.
- `cmd_release_sign` → 1 dependent (`cmd_release`). Isolated.
- `download_checksums_txt` → graph reports 0 dependents; **grep cross-check finds the real caller** `cmd_governance.rs:45` (known graphify Rust cross-crate blind spot). One call site only.
- Not affected: `bins/node` upgrade/`update verify`, auto-update, `updater/src/apply.rs`.

### Brittleness check
Signals detected: **1/5** — signal 5 (contract absence: `sign-release.sh` scrapes CLI stdout with `sed -n '/^{/,/^}/p'`).
Signals 1-4 do not apply. Verdict: **LOCALIZED**.

## Answers

**(a) Draft assets over REST, and the CLI's auth story.** The REST API *does* serve drafts and their assets, but only to an
authenticated caller with push access: `GET /repos/{o}/{r}/releases/tags/{tag}` returns the draft, and the asset must be
pulled from the API-side `assets[].url` with `Accept: application/octet-stream` — **not** `browser_download_url`, the public
path that 404s. That is what `gh release download` does, and it is proven here: both scripts already retrieve draft assets
with it. The CLI has **no** authenticated path — `grep -rn "GITHUB_TOKEN|GH_TOKEN|Authorization|bearer_auth"` over `crates/`
and `bins/` returns zero hits in updater/CLI; `download_from_url` (`download.rs:64-80`) builds a bare client.
**SSF seam: add `--checksums <file>` to `doli release sign`** — read the file, sha256 it locally, skip the network. ~10 lines
of Rust, no credential inside a signing binary, reusing the seam verify already has. Rejected: an authenticated GitHub client
in the updater — it adds asset-id resolution and a token to fix a problem the caller has already solved.

**(b) Verify staleness.** The snapshot is `storage::MaintainerState` (`crates/storage/src/maintainer.rs:70-80`): `version`,
`set` (members + threshold), **`last_derived_height`**. No timestamp. The height IS read by `resolve_upgrade_trust_root`
(`cmd_upgrade.rs:66-79`) and passed to `TrustRoot::resolve`, but `TrustRoot::on_chain()` **discards it** — so nothing prints
it and staleness is invisible in the CLI output. **SSF seam: add `--trust-root bootstrap` to `doli release verify` ONLY, and
make the banner name `last_derived_height`.** Applied at the `cmd_release_verify` call site (`TrustRoot::bootstrap(network)`),
never inside the shared resolver. The compiled mainnet array (`constants.rs:59-63`) holds the three actual signers
`d07ec4ec`/`25c24110`/`2559a47e`, which is why ai2's `Bootstrap` root verifies 3/3. Rejected: `--trust-root-rpc <url>` — it
makes an RPC endpoint a new trust boundary for the verification root and needs new client code.

**(c) publish-release.sh.** Both `gh release edit` calls (~L117, ~L119) become
`gh release edit "$TAG" --repo "$REPO" [--notes-file ...] --draft=false --prerelease=false --latest`. This also un-sticks the
current v6.28.0, a published non-Latest prerelease from the manual workaround. **`sign-release.sh` must NOT publish anything:**
once `--checksums` lands, signing works directly on the draft, so the "publish as non-Latest prerelease first" workaround is
unnecessary — and baking a publish step into the signer would permanently invert the INC-I-202 invariant.

## Security check — no weakening
Trust boundaries: T1 release assets from GitHub; T2 the local CHECKSUMS.txt path the operator supplies; T3 the maintainer
trust root (on disk / compiled in).
- **T2 is not new authority.** `--checksums` changes only *how the bytes arrive*. The hash is still computed locally from those bytes, `validate_release_hash` still runs, and the message is still `"{version}:{sha256}"` — byte-identical to the fetch path for the same file (REQ-214-001's acceptance criterion).
- **A wrong local file cannot be promoted.** `install_gate` L2 recomputes sha256 over the CHECKSUMS.txt that `publish-release.sh` downloads *from the release*; a mismatch refuses promotion. Worst case is a refusal, not a bad accept.
- **Threshold enforcement untouched.** `verify_release_manifest` / `verify_release_with_trust_root` are not modified; `is_usable()` (threshold ≥ 1, keys ≥ threshold) still gates every verification.
- **`--trust-root bootstrap` is verify-only** and must print a loud bypass line. It never reaches `cmd_upgrade`, `verify_release_artifact`, or any node install path.
- **An attacker with repo write access can do nothing new.** Before and after they can replace CHECKSUMS.txt on the release; in both cases the maintainer signs the bytes in hand and the authority is the private key in `~/.ssh/doli`, which repo write does not grant. No signature is accepted that was not accepted before; no threshold lowered; no key set widened.
- Residual, pre-existing: the signed message still carries no network term (AUDIT-P2-012).

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-214-001 | `doli release sign --checksums <file>` signs a draft from a local CHECKSUMS.txt | Must | - [ ] With `--checksums`, no HTTP request is made<br>- [ ] Signature bytes IDENTICAL to the fetch path for the same file<br>- [ ] `validate_release_version` + `validate_release_hash` still run<br>- [ ] Missing/unreadable file → non-zero exit, no signature printed |
| REQ-214-002 | `doli release verify --dir <d> --trust-root bootstrap` verifies v6.28.0 as 3/3 | Must | - [ ] Reports `Verified: 3 distinct maintainer signature(s)`<br>- [ ] Banner prints provenance AND `last_derived_height`<br>- [ ] Banner states the on-chain root was bypassed<br>- [ ] Flag absent from `doli upgrade`; `resolve_upgrade_trust_root` unchanged |
| REQ-214-003 | `publish-release.sh` clears the prerelease flag when promoting | Must | - [ ] Recorded `gh` argv has `--draft=false --prerelease=false --latest`<br>- [ ] Absent SIGNATURES.json → REFUSING, no `gh release edit`<br>- [ ] `signatures` length < THRESHOLD → REFUSING, no edit<br>- [ ] `doli release verify` non-zero → REFUSING, no edit |
| REQ-214-004 | `sign-release.sh` no longer swallows the signer's stderr | Must | - [ ] `2>/dev/null` removed from the `release sign` call<br>- [ ] A failing signer's own error text is printed<br>- [ ] Exit non-zero and names which maintainer key failed |
| REQ-214-005 | The release skill's step table matches the mechanical order | Should | - [ ] Step 4 shows `--dir` (+ `--trust-root bootstrap` on the dev Mac)<br>- [ ] No step publishes before verification<br>- [ ] The prerelease workaround is removed, not normalised |
| REQ-214-006 | `monitor-release-signed.sh` uses the same verify inputs as `publish-release.sh` | Should | - [ ] Downloads the assets and calls `verify --dir`<br>- [ ] A stale local trust root gives no false UNHEALTHY<br>- [ ] Still refuses on DRAFT and on missing release |

REQ-214-001 also requires `sign-release.sh` to move its existing `gh release download` **above** the signing loop and pass
`--checksums "$TMPDIR/CHECKSUMS.txt"` — the script half of the same seam.

## Rust vs script split (for the test-writer)
- **Rust (cargo, FAIL-first)** — REQ-214-001/002. Home: `bins/cli/tests/inc_i_214_*.rs`, beside `inc_i_172_cli_trust_root_resolution_test.rs` and `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs`. RED today is a compile/arg-parse failure (the flags do not exist) — **that counts as RED only if the test asserts the flag is accepted AND the signature equals a fixture from `sign_release_hash(key, version, sha256(file))`**. A test that merely fails to compile proves nothing about behaviour.
- **Shell (stubbed `gh`/`doli` on PATH, argv recorder)** — REQ-214-003/004/006. Home: a new `scripts/test_release_scripts_inc_i_214.sh` modelled on `scripts/test_gauntlet_gs015.sh` (OUTPUT CONTRACT header, gh/doli stub log, 16 partitions). No `--dry-run` needed: the stub records argv.
- **Docs** — REQ-214-005, verified by review.

## Milestones
**M1 (Rust, first — the scripts depend on the new flags):** `commands.rs` (`--checksums` on `Sign`, `--trust-root` on
`Verify`), `cmd_governance.rs` (`cmd_release_sign` local-file branch), `main.rs` (pass the flag through), `cmd_upgrade.rs`
(`cmd_release_verify` only: honour the flag at the call site, print `last_derived_height`); tests `bins/cli/tests/inc_i_214_*.rs`.
**M2 (scripts + skill):** `sign-release.sh`, `publish-release.sh`, `monitor-release-signed.sh`,
`.claude/skills/release/SKILL.md`, new `scripts/test_release_scripts_inc_i_214.sh`.

## Deploy questions
Consensus RULES: **NO**. Block CONTENT: **NO**. Activation height: **N/A**. Rolling-safe: **YES** (CLI + scripts only; no
`doli-node` change, no fleet deploy). The mainnet AH crossing at 409,000 (~2026-09-08) is untouched. The dev Mac uses
`target/release/doli` immediately after `cargo build --release`; operators get the CLI in the next release. Old CLIs keep
working — both new flags are optional.

## Triage Verdict
```
━━━ TRIAGE VERDICT ━━━
Verdict: FAST
Rationale: 1/5 brittleness signals (LOCALIZED). Three independent, additive seams at
           leaf call sites; the shared verification core (install_gate, trust_root,
           sign_release_hash) is untouched. Blast radius is 4 CLI files + 3 scripts +
           1 skill, with one graph-derived constraint (do not touch the shared
           resolver). No consensus or node surface.
Milestones: 2 (M1 Rust, M2 scripts+skill)
━━━━━━━━━━━━━━━━━━━━━━
```

## Assumptions
| # | Assumption (technical) | Plain language | Confirmed |
|---|------------------------|----------------|-----------|
| 1 | `gh release edit` sends `prerelease` only when the flag is given, and accepts `prerelease:false` + `make_latest:true` in one PATCH | One extra flag is enough; no second command | No — argv is stub-testable; the live 200-vs-422 needs one real promotion |
| 2 | The dev Mac's stale `maintainer_state.bin` will keep drifting (no local mainnet node is resyncing) | Deleting the file once would not stop the recurrence | No |
| 3 | `bins/node`'s own `release sign` copy is unused in this runbook and stays unchanged | Server-side signing keeps old behaviour on purpose | No — flag as deliberate drift in the commit |

## What I do not understand
- Whether any host runs `release verify` where the on-chain root has legitimately REVOKED a compiled bootstrap key; there `--trust-root bootstrap` could mask a real revocation. Hence verify-only + a printed bypass line. Not enumerated.
- Why the dev Mac holds a mainnet `MaintainerState` at all (which local node wrote it, at what height).
- Whether CI's `prerelease:` expression (`release.yml:593`) has ever evaluated true for a non-rc tag.

## Out of scope (Won't)
- An authenticated GitHub client in `crates/updater` (credential in the signing path; the caller already has `gh`).
- A network term in the signed message (invalidates every published SIGNATURES.json — AUDIT-P2-012, own rollout).
- Reconciling `MaintainerSet::calculate_threshold` with `REQUIRED_SIGNATURES` (governance multisig, needs an AH).
- Replacing the stdout scrape with a machine contract (brittleness signal 5) — real, but not what is broken today.

## Traceability Matrix
| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|----------------|----------|----------|----------------------|-----------------------|
| REQ-214-001 | Must | (test-writer) | Data flow / T2 | `bins/cli/src/{commands.rs,cmd_governance.rs}` + `scripts/sign-release.sh` |
| REQ-214-002 | Must | (test-writer) | Blast radius / T3 | `bins/cli/src/{commands.rs,main.rs,cmd_upgrade.rs}` |
| REQ-214-003 | Must | (test-writer) | Data flow (promote) | `scripts/publish-release.sh` |
| REQ-214-004 | Must | (test-writer) | Module boundaries | `scripts/sign-release.sh` |
| REQ-214-005 | Should | (review) | Constraints | `.claude/skills/release/SKILL.md` |
| REQ-214-006 | Should | (test-writer) | Module boundaries | `scripts/monitor-release-signed.sh` |
