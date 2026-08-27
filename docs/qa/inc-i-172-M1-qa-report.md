# QA Report: INC-I-172 M1 — Maintainer trust root (Layer 1, node-local)

Run 508 · branch `bugfix/inc-i-172-maintainer-trust-root` · QA date 2026-08-10
Baseline: last commit on branch (`f2b66c19`) + `git diff` + untracked files.

## Scope Validated

Criteria A–H from the QA brief, against the binding contract
`docs/.workflow/inc-i-172-M1-api-contract.md` (including §8 runner decisions G1–G5 and
§9 runner correction), the design `specs/maintainer-trust-root-architecture.md`
(F1, F3 release path, F5, F6, F7 node-local, F8 delete part), and
`docs/redesigns/maintainer-trust-root-redesign-analysis.md`.

## Summary

**PASS.** Every defect the milestone targeted is verifiably gone, and the highest-risk
item — the legacy `maintainer_state.bin` migration that an earlier implementation turned
into a fleet-wide startup refusal — is confirmed fixed **against the real 232-byte file
from a live testnet node**, not a synthetic fixture. The full gate is green with 1,994
tests passing and zero failures. No forbidden change was made: no activation height, no
protocol/format-version bump, no `Cargo.toml` version bump, `WHITEPAPER.md` untouched,
`crates/core/` untouched.

One **HIGH** finding is recorded that is *outside* the binding contract's scope and is
**not a regression**: `doli-node upgrade` is a third operator-facing install path that
performs no maintainer signature verification at all. It was invisible to the design's
"three verification call sites" survey precisely because it never calls a verifier. It
does not block this milestone, but it defeats the incident's headline property and should
be scheduled before INC-I-172 closes.

## System Entrypoint

This milestone is node-local library + CLI code with no runnable end-to-end network
change, so validation was done by (a) running the real code paths under `cargo test`
harnesses driven by QA-authored probes, and (b) exercising the storage load path against
production bytes copied out of a live node.

```bash
cargo build --release                                  # exit 0
cargo clippy --workspace --all-targets -- -D warnings  # exit 0
cargo fmt --check                                      # exit 0
```

Real-bytes probe fixture (source never modified — md5 verified identical before and
after the entire QA run):

```
~/testnet/seed/data/maintainer_state.bin  232 bytes  md5 6a7848443c0d47dd5d63a333a9e1f135
first 8 bytes: 05 00 00 00 00 00 00 00   (bincode u64 length prefix = 5 members, NO magic)
```

## Per-Criterion Results

| # | Criterion | Result | Evidence |
|---|---|---|---|
| A | REQ-172-001 / F1 fail-closed on-chain trust root | **PASS** | see A below |
| B | REQ-172-005 preserved behavior (legacy migration) | **PASS** | see B below |
| C | REQ-172-012 / F3 distinct-signer counter | **PASS** | see C below |
| D | REQ-172-006 / F6 install blocked on verification failure | **PASS** | see D below |
| E | F7 re-verify at install; no `published_at` timing | **PASS** | see E below |
| F | F8 dead veto machinery gone; `derive_maintainer_set` kept | **PASS** | see F below |
| G | No forbidden changes | **PASS** | see G below |
| H | Adjacent breakage / docs truth / exploratory | **PASS** (3 findings) | see H below |

---

### A — REQ-172-001 / F1: fail-closed on-chain trust root — PASS

`crates/updater/src/verification.rs` contains **no** `bootstrap_maintainer_keys` call and
**no** `is_empty()` fallback. The only occurrence of the name is a doc comment at
`crates/updater/src/verification.rs:59` stating the fallback does not exist.

Fail-closed behaviour proven empirically by a QA-authored probe (independent of the
developer's assertions), run against `verify_release_with_trust_root`:

| Input | Result observed |
|---|---|
| `TrustRoot::on_chain(vec![], 3)` + 3 valid signatures | `TrustRootUnavailable { provenance: "OnChain", keys: 0, threshold: 3 }` |
| `TrustRoot::on_chain(2 keys, 3)` + 2 valid signatures | `TrustRootUnavailable { provenance: "OnChain", keys: 2, threshold: 3 }` |
| `TrustRoot::on_chain(5 keys, 0)` + zero signatures | `TrustRootUnavailable { provenance: "OnChain", keys: 5, threshold: 0 }` |

The third row matters on its own: a defaulted set has `threshold == 0`, and a bare
`valid >= threshold` test would have vacuously accepted a zero-signature release (FM-02).
`TrustRoot::is_usable` guards it at `crates/updater/src/trust_root.rs:96-98`.

**Every remaining caller of the compiled keys, accounted for:**

| Site | Verdict |
|---|---|
| `crates/updater/src/constants.rs:89` | definition |
| `crates/updater/src/constants.rs:101` (`is_using_placeholder_keys`) | placeholder self-check, not verification |
| `crates/updater/src/constants.rs:126-130` (`get_maintainer_keys`) | **dead** — no caller repo-wide except the re-export; cannot reach verification |
| `crates/updater/src/trust_root.rs:18,56` | `TrustRoot::bootstrap` — the ONLY permitted reader (contract §1) |
| `crates/updater/src/lib.rs:68`, `bins/node/src/updater/mod.rs:26` | re-exports |
| `bins/node/src/commands/maintainer.rs:88` | `println!` display only (see OBS-002) |
| `crates/updater/tests/trust_root_fail_closed.rs` | test fixture |

No path reaches the compiled keys when an on-chain set exists: the single decision point is
`resolve_trust_root` (`bins/node/src/updater/trust_root_wiring.rs:58-96`), and it selects
`Bootstrap` **only** when `members.is_empty() && last_derived_height == 0`. `members` empty
with `last_derived_height > 0` returns `TrustRoot::on_chain(vec![], …)` and fails closed
(`:88-94`). `verify_release_signatures_with_keys` — the empty-slice sentinel — is **deleted**
(grep repo-wide: zero hits).

`bins/node/src/run.rs`: the `move || -> Vec<String> { Vec::new() }` closure and the false
BLAKE3 TODO are both gone (`git diff bins/node/src/run.rs` shows them as `-` lines);
replaced by `crate::updater::load_maintainer_state(&data_dir)?` +
`maintainer_trust_root_fn(...)`.

Lock-contention case is also fail-closed: `trust_root_wiring.rs:106-119` returns an
unusable `OnChain` root when `try_read()` fails, rather than degrading to Bootstrap.

### B — REQ-172-005: preserved behavior — PASS (verified on production bytes)

This was the highest-risk item. Verified with a QA probe pointed at a copy of the **real**
`~/testnet/seed/data/maintainer_state.bin`:

```
QA: raw legacy len=232 first8=[05, 00, 00, 00, 00, 00, 00, 00]
QA load#1: version=1 members=5 threshold=3 height=1
QA   member[0] = 54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b
QA   member[1] = effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b
QA   member[2] = 2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116
QA   member[3] = 202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df
QA   member[4] = 3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602
QA: raw after  len=240 first8=[44, 4d, 53, 54, 01, 00, 00, 00]     ("DMST" + version 1)
QA load#2: version=1 members=5 threshold=3 height=1
```

Asserted and passing:
- the real legacy file **loads** — it does not brick the node;
- recovered set is **5 members / threshold 3**, `last_derived_height` preserved;
- `raw_after[8..] == raw_before[..]` — the body is preserved **bit-for-bit**; only the
  8-byte header is added;
- the second `load()` takes the current-format path and yields identical members
  (migration is persisted and idempotent).

Fresh node (no file) → `Ok(default())`, `bins/node/src/updater/trust_root_wiring.rs:74-83`
then resolves `TrustRoot::bootstrap(network)`, so an un-upgraded/unbootstrapped node keeps
verifying exactly as before.

The §9 aliasing hazard is genuinely closed: the format is `MAGIC "DMST" || u32 LE version ||
bincode(body)` (`crates/storage/src/maintainer.rs:29-35, 201-221`), so a bincode length
prefix can never be mistaken for a version tag.

The stale deploy-day warning required to be removed by contract §9 **is** removed —
`docs/.workflow/inc-i-172-M1-dev-notes.md:9-22` now states "Upgrade day: NO operator action".

### C — REQ-172-012 / F3: distinct-signer counter — PASS

Loop shape at `crates/updater/src/verification.rs:89-104` is structurally identical to the
mainnet-live covenant k-of-n shape at `crates/core/src/conditions/eval.rs:51-68`: outer loop
over expected keys, inner loop over witnesses, `break` on the first valid match.

QA probe result (with a working positive control, so the negative result is meaningful):

| Input to a 3-of-5 root | Observed |
|---|---|
| 3 valid signature **entries, all from ONE key** | `InsufficientSignatures { found: 1, required: 3 }` |
| 3 valid signatures from **3 distinct keys** (positive control) | `Ok(())` |
| zero signature entries (empty `SIGNATURES.json`) | `InsufficientSignatures { found: 0, required: 3 }` |

The GOVERNANCE counter is untouched, as required (it needs an AH and is M2):
`git diff --stat -- crates/core/` is **empty**; `MaintainerSet::verify_multisig`
(`crates/core/src/maintainer.rs:145-159`) still uses the entry-counting
`.filter(...).count()`.

### D — REQ-172-006 / F6: install blocked on verification failure — PASS

**`bins/cli/src/cmd_upgrade.rs`** — control flow traced line by line. Three distinct failure
modes all `return Err` *before* the first `install_binary` at `:132`:

| Condition | Line | Outcome |
|---|---|---|
| `SIGNATURES.json` unreachable | `:87-96` | `?` on `map_err` → `Err`, aborts |
| `Ok(None)` — no `SIGNATURES.json` | `:97-103` | `ok_or_else` → `Err`, aborts |
| verification fails / below threshold | `:113-120` | `map_err(...)?` → `Err`, aborts |
| first install | `:132` | unreachable unless all three passed |

Network is threaded from the CLI argument (`network` parameter used at `:113` and `:117`);
the hardcoded `doli_core::Network::Mainnet` is gone.

**`bins/node/src/commands/update.rs`** — `UpdateCommands::Verify` at `:391-416`: the `Err`
arm now `return Err(anyhow!(...))` instead of printing and exiting 0, so
`doli-node update verify vX && doli upgrade` can no longer be chained on a false success.

No path from a *failed or absent* signature check reaches `install_binary` in either file.

> Separately, a third command that never performs a signature check at all was found —
> see **ISSUE-001** under H. It is outside this criterion's two named files and outside the
> binding contract, and is not a regression.

### E — F7 node-local: re-verify at install; no `published_at` timing — PASS

**F7(a) re-verify.** `bins/node/src/updater/service.rs:344-385` (`auto_apply`) re-resolves
the trust root via `maintainer_keys_fn()` at `:368` and re-verifies the staged release at
`:369`, *immediately before* `auto_apply_from_github` at `:388`. On failure it logs at
`error!`, sets `*pending = None`, calls `PendingUpdate::remove(...)` and `return`s — the
pending update is dropped, not merely skipped. This is the revocation-reaches-in-flight-update
property, and it is correct.

**F7(b)/G1 timing.** Every deadline function now takes an explicit node-local reference
timestamp — no function reads `release.published_at`:

```
crates/updater/src/enforcement.rs:29  veto_deadline(first_notified_at: u64)
crates/updater/src/enforcement.rs:34  veto_period_ended(first_notified_at: u64)
crates/updater/src/enforcement.rs:41  grace_period_deadline(first_notified_at: u64)
crates/updater/src/enforcement.rs:54  in_grace_period(first_notified_at: u64)
crates/updater/src/enforcement.rs:98  VersionEnforcement::from_approved_release(release, first_notified_at)
crates/updater/src/apply.rs:78        apply_update(release, first_notified_at, approved, veto_percent)
```

**Every remaining `published_at` use, accounted for** (`grep -rn "published_at" --include="*.rs"`):

| Category | Sites | Deadline input? |
|---|---|---|
| struct field | `crates/updater/src/types.rs:38`, `crates/storage/src/update.rs:36` | no |
| GitHub JSON parse → metadata | `crates/updater/src/download.rs:175,206,213,303` | no |
| doc comments explaining it is *not* used | `enforcement.rs:19,24`, `apply.rs:67`, `lib.rs:8`, `bins/node/src/updater/mod.rs:115-116` | no |
| operator display | `bins/node/src/updater/cli.rs:27`, `bins/cli/src/cmd_governance.rs:268` | no |
| `getUpdateStatus` RPC field | `bins/node/src/node/startup.rs:500` | no |
| fixture value `published_at: 0` | `cmd_upgrade.rs:110`, tests | no |

No deadline, grace window, or install gate reads it. Display-only uses are acceptable per
contract §8 G1.

### F — F8: dead veto machinery deleted, `derive_maintainer_set` preserved — PASS

Repo-wide reference counts after the change:

```
with_weights            refs=0     approval_weight         refs=0
set_weights             refs=0     veto_percent_weighted   refs=0
should_reject_weighted  refs=0     calculate_vote_weight   refs=0
seniority_multiplier    refs=0     is_eligible_to_vote     refs=0
veto_weight             refs=4  -> all four are unrelated LOCAL variables in
                                   crates/storage/src/producer/set_governance.rs:114,122,135,143
```

`crates/updater/src/vote.rs` −161 lines, `crates/updater/src/params.rs` −50 lines.

`derive_maintainer_set` **still exists** at `crates/core/src/maintainer.rs:490`
(`pub fn derive_maintainer_set<R: BlockchainReader>(reader: &R) -> MaintainerSet`) — its
deletion would have been a defect since M2 revives it. `crates/core/` is untouched entirely.

### G — No forbidden changes — PASS (verified by diff, not assertion)

| Check | Command | Result |
|---|---|---|
| Cargo version bump | `git diff -- '**/Cargo.toml' Cargo.toml` | `version = "6.24.1"` is an unchanged **context** line. The only hunk is `repository = e-weil/doli` → `doli-network/doli`, which belongs to INC-I-157, not a version bump. |
| `CURRENT_PROTOCOL_VERSION` (=8) | `crates/network/src/protocols/status.rs:49` | file not in `git status` → unchanged |
| `EPOCH_STATE_FORMAT_VERSION` (=1) | `crates/network/src/protocols/status.rs:68` | unchanged |
| `MIN_PEER_PROTOCOL_VERSION` (=1) | `crates/network/src/protocols/status.rs:83` | unchanged |
| Activation height added | `git status` on `network_params/`, `consensus/` | both untouched; no `activation_height` in any new source file |
| Consensus-visible change | `git diff --stat -- crates/core/` | empty |
| `WHITEPAPER.md` | `git status` | untouched |

**Module size budget.** No new violation was introduced; the milestone *improved* the worst
offender it touched.

| File | baseline | now | delta |
|---|---|---|---|
| `bins/node/src/updater/service.rs` | 537 | **450** | −87 (brought **under** budget via `service_checks.rs`) |
| `bins/node/src/run.rs` | 773 | 769 | −4 (pre-existing >500) |
| `crates/updater/src/download.rs` | 613 | 587 | −26 (pre-existing >500) |
| `crates/updater/src/apply.rs` | 753 | 757 | +4 (pre-existing >500) |
| `bins/node/src/node/startup.rs` | 653 | 675 | +22 (pre-existing >500) |
| `bins/cli/src/main.rs` | 566 | 571 | +5 (pre-existing >500) |

All **new** source files are well under 500 (`trust_root.rs` 99, `service_checks.rs` 164,
`trust_root_wiring.rs` 174, `upgrade_restart.rs` 338, `maintainer.rs` 333). All test files
are under 800 (largest: `maintainer_state_versioned_test.rs` 682).

### H — Adjacent breakage, docs truth, exploratory — PASS (3 findings)

**Exploratory probes** (each a real execution, not a reading):

| # | What was tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Load real 232-byte legacy `maintainer_state.bin` | migrates, set preserved | `Ok`, 5 members / threshold 3, body bit-identical | — (pass) |
| 2 | Load the same file twice | 2nd load takes current-format path | identical values, file now `DMST`-prefixed | — (pass) |
| 3 | Legacy file in a **read-only data dir** (file mode 0444 + dir 0555) | in-memory set correct, node not bricked | `Ok`, 5 members / threshold 3; on-disk file still 232 bytes/unmigrated (**positive control confirmed the write was genuinely blocked**) | — (pass) |
| 4 | **Zero-length** `maintainer_state.bin` | `Err`, never a silent empty default | `Err(Serialization)`, explicitly not `default()` | — (pass) |
| 5 | Truncated (half-length) real file | `Err`, never a default | `Err(Serialization)` | — (pass) |
| 6 | `SIGNATURES.json` present but with an **empty** signature list | refused | `InsufficientSignatures { found: 0, required: 3 }` → CLI aborts before install | — (pass) |
| 7 | Maintainer set drops below threshold while an update is pending | pending dropped, no install | `auto_apply` `service.rs:369-385` re-verifies, logs `error!`, clears pending + removes `pending_update.json` | — (pass) |
| 8 | `getUpdateStatus` when the on-chain root is empty | empty condition surfaced | `startup.rs:472-486,513,521` emits `trust_root {provenance, keys, threshold, usable}` in both the pending and no-pending shapes → `{"provenance":"OnChain","keys":0,"usable":false}` | — (pass) |
| 9 | Enumerate **all** paths to `install_binary` (not just verified ones) | all gated | `doli-node upgrade` reaches `install_binary` with no signature check | **HIGH** (ISSUE-001) |

**Note on probe 3.** My first attempt made only the *directory* read-only and passed —
falsely. A read-only directory does not stop `fs::write` from truncating an existing
writable file, so that run never reached the save-failure branch. I added an explicit
positive control asserting the on-disk bytes are still unmigrated; only then is the `Ok`
result evidence. Recorded because the original form would have been a false pass.

**Verified EMFILE claim (not accepted on assertion).** The brief stated
`test_cluster_10x100` fails with EMFILE in this environment at HEAD too.

- Run **in isolation** on the current tree: `test test_network::test_cluster_10x100 ... ok`
  (12.23s). It does **not** fail on its own; `ulimit -n` is 1,048,576.
- Run as part of the **full** `cargo test -p doli-node`: `FAILED`, with
  `Node 69 init failed: database error: IO error: DB::Open() failed … OPTIONS-000006.dbtmp: Too many open files`.

So the claim holds *in the full-suite context only*, and the mechanism is fd exhaustion from
~70 concurrent RocksDB instances — not the milestone diff. The milestone touches no RocksDB
open path (`crates/storage/src/maintainer.rs` uses plain `fs::read`/`fs::write`), and the
isolation pass demonstrates the test itself is healthy on this tree. Skipping it is
justified; the claim is now substantiated rather than assumed.

**Docs verified true against the code (code is SoT).** The `.claude/skills/auto-update/SKILL.md`
correction banner, the `constants.rs:12-34` veto/threshold comments, and the
`docs/DOCS.md` / `docs/attack_analysis.md` entries all match the code. Remaining "7 days"
strings in `docs/` are correctly phrased as "5 minutes (early network; target 7 days)" —
they state the configured value first, so they are truthful.

Two false doc lines were found and **fixed by QA** (trivial corrections, permitted by the
brief):

- `docs/auto_update_system.md:806` claimed `doli upgrade` requires signatures: "No (warning
  only)" — directly contradicting `:1319` in the same file and the code at
  `cmd_upgrade.rs:113-120`. Corrected, and the table now also lists the third path with its
  true (absent) verification status.
- `.claude/skills/updater/SKILL.md:441` claimed `doli upgrade` "shows signature/veto status
  via `download_signatures_json`/`calculate_veto_result`". `calculate_veto_result` has **no**
  CLI caller (only `bins/node/src/updater/service.rs:207`), and the command now *gates*
  rather than shows. Corrected.

## Specs/Docs Drift

| File | Documented behavior | Actual behavior | Severity | Status |
|---|---|---|---|---|
| `docs/auto_update_system.md:806` | `doli upgrade` signatures "No (warning only)" | aborts install on failure (`cmd_upgrade.rs:113-120`) | medium | **fixed by QA** |
| `.claude/skills/updater/SKILL.md:441` | `doli upgrade` "shows signature/veto status"; implies `calculate_veto_result` is a CLI path | it gates the install; `calculate_veto_result` has no CLI caller | low | **fixed by QA** |
| `bins/node/src/commands/maintainer.rs:85` | prints "Bootstrap keys (…, **fallback before sync**)" and "Using bootstrap keys" | bootstrap keys are no longer a fallback (F1); an existing-but-empty on-chain set fails closed | low | **open** (OBS-002) |

## Test Results — real per-crate counts, verbatim from `cargo test`

Re-run cleanly with all QA probe files removed from the tree.

| Crate | Command | passed | failed | ignored |
|---|---|---|---|---|
| storage | `cargo test -p storage` | **343** | 0 | 0 |
| updater | `cargo test -p updater` | **51** | 0 | 2 |
| doli-cli | `cargo test -p doli-cli` | **222** | 0 | 0 |
| doli-core | `cargo test -p doli-core` | **1043** | 0 | 6 |
| doli-node | `cargo test -p doli-node -- --skip test_cluster_10x100` | **335** | 0 | 32 |
| **Total** | | **1994** | **0** | **42** |

Representative verbatim lines:

```
storage   test result: ok. 251 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.80s
updater   test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
updater   test result: ok. 1 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 112.06s
doli-cli  test result: ok. 195 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
doli-core test result: ok. 972 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s
doli-node test result: ok. 12 passed; 0 failed; 11 ignored; 0 measured; 2 filtered out; finished in 200.14s
```

Gate:

```
cargo build --release                                   exit 0
cargo clippy --workspace --all-targets -- -D warnings   exit 0
cargo fmt --check                                       exit 0
```

## Findings

### ISSUE-001 — HIGH — `doli-node upgrade` installs a binary with NO signature verification

**Evidence:** `bins/node/src/commands/misc.rs:118` (`handle_upgrade_command`) → downloads at
`:156`, verifies **only** the tarball checksum at `:162` against
`release_info.expected_hash`, and calls `updater::install_binary(&binary, &target)` at
**`bins/node/src/commands/misc.rs:180`**. Reachable from `bins/node/src/main.rs:303`
(`Commands::Upgrade`), documented at `docs/running_a_node.md:693`.

Verified there is no verification anywhere on that path: `handle_upgrade_command` contains
no call to `verify_release_signatures`, `verify_release_with_trust_root`, or
`download_signatures_json`; and `fetch_github_release` (`crates/updater/src/download.rs:364`)
performs no signature check internally. `expected_hash` is parsed from `CHECKSUMS.txt`
fetched from the same GitHub release, so it is self-consistent with — not independent of —
a compromised origin.

**Why it was missed:** the design's F6 survey enumerated the *three sites that call a
verifier*. A path that never calls one is invisible to that survey. The binding contract
(§5 + §8 "Path correction") therefore scoped F6 to `cmd_upgrade.rs` and
`commands/update.rs:384` only.

**Classification:** pre-existing (file is unmodified in this diff), **not a regression**, and
**outside the binding contract**. It does not block M1. It does defeat REQ-172-006's stated
property at the incident level and should be closed before INC-I-172 is closed — the fix is
the same six lines already present in `cmd_upgrade.rs:87-120`.

### OBS-001 — LOW — `doli upgrade` success message prints the threshold as if it were a count

`bins/cli/src/cmd_upgrade.rs:121-125` prints
`"Verified: {} distinct maintainer signatures"` with `updater::REQUIRED_SIGNATURES` (the
constant 3), not the number of distinct signers actually found. If 5 valid signatures are
present the operator is told "3". Cosmetic; the gate itself is correct.

### OBS-002 — LOW — operator-facing text still calls the bootstrap keys a "fallback"

`bins/node/src/commands/maintainer.rs:70,74,85` print "Using bootstrap keys" and
"Bootstrap keys (…, fallback before sync)". F1 removed the fallback; contract §8 G3
required exactly this wording corrected in `constants.rs` (done), but the same framing
survives in this command's output.

### OBS-003 — LOW — `get_maintainer_keys` is now dead code

`crates/updater/src/constants.rs:126-132` has no caller repo-wide other than the `lib.rs:68`
re-export. Harmless today, but it is a ready-made second reader of the compiled keys that a
future edit could wire back into a verification path. Consider deleting it in M2.

### OBS-004 — LOW — the working tree mixes two incidents

`git status` shows INC-I-157 work (release-origin de-pinning: `crates/updater/src/download.rs`,
`constants.rs` origin constants, `scripts/`, `docker/`, `crates/updater/tests/origin_pinning.rs`,
`docs/qa/inc-i-157-M1-qa-report.md`) uncommitted alongside INC-I-172 M1. The M1 diff is not
isolated; splitting the commits will make review and any future bisect meaningful.

## Blocking Issues

**None.** All Must acceptance criteria within the binding contract's scope are met, all
tests pass, and no forbidden change was made. ISSUE-001 is HIGH but pre-existing,
non-regressive and out of contract scope — tracked for a follow-up milestone, not a blocker
for M1 merge.

## Modules Not Validated

- Governance multisig counter (`crates/core/src/maintainer.rs:145-188`) — deliberately
  deferred to M2 (needs an activation height, contract §8 G5). Confirmed untouched.
- Replay-domain binding on governance messages — deliberately not implemented in M1
  (contract §2: the signed message format must not change).
- Live network behaviour (mainnet/testnet deploy) — out of scope for a node-local library
  milestone; no node was started or restarted during this QA run.

## Final Verdict

All Must and Should acceptance criteria for the contracted M1 scope are met. The defects the
milestone targeted are verifiably gone, the highest-risk regression path is proven safe on
production bytes, and the full gate is green.

**QA VERDICT: PASS**
