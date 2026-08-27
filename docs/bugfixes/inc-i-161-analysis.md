# INC-I-161 — Analysis: `doli balance` resolves to `/var/lib/doli/mainnet/wallet.json` and fails ENOENT

**Agent:** Analyst · **RUN_ID:** 496 · **Branch:** main · **Date:** 2026-08-07
**Constraint honored:** no SSH, no host mutation. Code-only analysis. Host facts are listed as open questions.

---

## Scope

`bins/cli/src/` (path resolution + wallet load + init), `scripts/install.sh`, `docs/cli.md`,
`docs/producer-ux-proposal.md`. No node/consensus code is in scope — nothing here touches
`apply_block`, activation heights, or block content. **No consensus risk. No activation height. No
synchronized deploy required.**

## Summary (plain language)

`doli balance` looks for the wallet in one place and one place only: whatever directory the resolver
picks, plus `/wallet.json`. On Linux it picks `/var/lib/doli/mainnet` **as soon as that folder exists**
— and the installer creates that folder, empty, on every install. So the resolver's own "fall back to
the old `~/.doli/mainnet` location" branch can never run on an installed Linux host. If the user's
wallet is in the old location, the CLI will insist it doesn't exist. If the user simply never created a
wallet on this host, the error is correct and there is no bug in the path logic at all — only in the
wording of the hint.

The error number tells us which of those two it is, and I can narrow it further with one read-only
`ls` on the host (see Open Questions).

---

## 1. Complete wallet-path resolution chain for `doli balance`

Every input, in precedence order. `doli balance` loads the wallet as its very first statement
(`bins/cli/src/cmd_wallet.rs:143` — `let wallet = Wallet::load(wallet_path)?;`), so this chain fully
determines the failure.

| # | Input | Site | Notes |
|---|-------|------|-------|
| 1 | `-w/--wallet <PATH>` (global) | `bins/cli/src/commands.rs:10-12` → `bins/cli/src/main.rs:104-107` | Tilde-expanded via `expand_tilde` (`bins/cli/src/common.rs:48-58`). **Wins outright.** |
| 2 | `DOLI_WALLET_FILE` env | `bins/cli/src/paths.rs:68-70` | Read manually, **not** via clap `#[arg(env=…)]`. **Not tilde-expanded** (asymmetry vs. #1). |
| 3 | `DOLI_DATA_DIR` env | `bins/cli/src/paths.rs:21-23` | Base dir only; `wallet.json` appended at `paths.rs:71`. Not tilde-expanded. |
| 4 | Platform default **if the DIRECTORY exists** | `bins/cli/src/paths.rs:26-29` + `paths.rs:75-106` | Linux → `/var/lib/doli/{network}`. **← this is the link that fires.** |
| 5 | Legacy `~/.doli/{network}` **if the DIRECTORY exists** | `bins/cli/src/paths.rs:32-52` | Prints a migration hint to stderr. **Unreachable on Linux — see §2.** |
| 6 | Platform default unconditionally | `bins/cli/src/paths.rs:55` | Same value as #4, returned even when absent. |

Then: `resolve_base_dir(...).join("wallet.json")` (`bins/cli/src/paths.rs:71`).

Network selection: `--network/-n`, default `"mainnet"`, env `DOLI_NETWORK`
(`bins/cli/src/commands.rs:18-20`). This is what makes the resolved path `…/mainnet/…`.

**Inputs that do NOT exist (verified absent, not assumed):**
- **No global `--data-dir` flag.** The `Cli` struct (`bins/cli/src/commands.rs:9-24`) has exactly three
  global options: `-w/--wallet`, `-r/--rpc`, `-n/--network`. The `data_dir` fields at
  `commands.rs:549`, `:573`, `:1047` are **subcommand-local** (snap/service/chain) and are never
  plumbed into wallet resolution — `main.rs:106` hard-codes `None` for that parameter.
- **No config file** participates in wallet resolution. (`config.toml` at `cmd_snap.rs:122` and
  `cmd_chain.rs:298` are entries in file-copy manifests, not config inputs.)
- **No effective-UID / root-vs-non-root branch** exists anywhere in `paths.rs`. Resolution is
  identical for root and for `isudoajl`. There is one *access*-related branch —
  `maybe_reexec_with_doli_group()` (`bins/cli/src/main.rs:45-91`) — but it re-execs the process under
  `sg doli`; it never changes the resolved path.

## 2. Is `/var/lib/doli/mainnet/` intended for an unprivileged user? — **YES, intended.**

Evidence of design intent, all from code/scripts (not narrative docs):

- `scripts/install.sh:148` creates a `doli` system user with `--home-dir /var/lib/doli`.
- `scripts/install.sh:156` runs `usermod -aG doli "$REAL_USER"` — the human operator is deliberately
  put in the `doli` group.
- `scripts/install.sh:163-166`, comment verbatim: *"Mode 2770: setgid + group-writable so doli group
  members can run `doli init` without sudo"*, then
  `install -d -o doli -g doli -m 2770 /var/lib/doli/mainnet`.
- `bins/cli/src/main.rs:45-91` (`maybe_reexec_with_doli_group`) exists solely to make the system dir
  usable by a non-root user whose `doli` group membership isn't yet active in the session.
- `bins/cli/src/cmd_init.rs:147-160` — on write failure the remedy printed is
  `sudo usermod -aG doli $USER && newgrp doli`, i.e. "join the group", never "use a different path".

So the *destination* is correct by design. The defect, if any, is in the **precedence rule that gets
there**, not in the destination.

### The installer/resolver contract violation (latent defect, provable without host access)

`scripts/install.sh:165` creates `/var/lib/doli/mainnet` **unconditionally at install time, empty**.
`bins/cli/src/paths.rs:26-29` returns the platform default **the moment that directory exists**.

Therefore **on every Linux host that ran `install.sh`, step 5 (the legacy `~/.doli/{network}`
fallback, `paths.rs:32-52) is dead code.** The back-compat contract stated in
`docs/producer-ux-proposal.md:47` — *"if `/var/lib/doli/{network}` does not exist but `~/.doli/{network}`
does, use the legacy path"* — and restated at `:577`, is structurally unsatisfiable. The resolver
depends on an implicit convention ("the platform dir exists only if it is in use") that the installer
violates by construction.

Note the **read/write asymmetry**: the *write* path already implements wallet-file-aware legacy
detection — `find_legacy_wallet()` at `bins/cli/src/cmd_init.rs:26-38` checks for
`~/.doli/{network}/wallet.json` **as a file** and `migrate_legacy_wallet()` (`cmd_init.rs:41-66`)
copies it forward. The *read* path (`resolve_wallet_path`) has no equivalent. `doli init` can find a
legacy wallet that `doli balance` cannot.

## 3. Where the error is emitted, and what it knows vs. reports

`bins/cli/src/wallet.rs:112-135`:

```rust
let contents = std::fs::read_to_string(path).with_context(|| {
    if path.exists() {
        format!("cannot read wallet: {}\n  Check file permissions.", path.display())
    } else {
        #[cfg(target_os = "linux")]
        { format!("wallet not found: {}\n  Create one: doli init", path.display()) }
        #[cfg(not(target_os = "linux"))]
        { format!("wallet not found: {}\n  Use -w to specify the wallet path, e.g.: doli -w /path/to/wallet.json <command>", path.display()) }
    }
})?;
```

- The `Caused by:` line is the raw `io::Error` from `read_to_string` on the **wallet file itself** —
  not on the parent directory. There is no separate parent-directory `stat` in this path.
- What the error path **knows but does not report**: the full precedence chain that produced `path`,
  which link fired, whether `DOLI_WALLET_FILE`/`DOLI_DATA_DIR` were set, and — critically — whether a
  legacy wallet exists at `~/.doli/{network}/wallet.json` (the code to answer that already exists
  three files away at `cmd_init.rs:26-38`).

### ENOENT vs. EACCES — the permission hypothesis is DISPROVEN

If the cause were a permission block on the `2770 doli:doli` ancestor `/var/lib/doli`, then:
`read_to_string("/var/lib/doli/mainnet/wallet.json")` returns **EACCES (os error 13)** (POSIX: search
permission denied on a path-prefix component → `EACCES`), and `Path::exists()` at `wallet.rs:114`
returns `false` (it swallows *any* error, including EACCES), so the output would read
`wallet not found: …` / `Caused by: Permission denied (os error 13)`.

Observed: **`os error 2` (ENOENT).** Therefore the process **could traverse** `/var/lib/doli/mainnet`,
and `wallet.json` is **genuinely absent there**. The permission/EACCES cause is eliminated, and by
implication the user's effective credentials did have group access (natively, or via the `sg doli`
re-exec at `main.rs:84-90`).

This also means the resolver took **step 4**, not step 6 — the directory existed and was stat-able.
So step 5 (legacy) was short-circuited, exactly as §2 predicts.

## 4. Regression check — **NO behavioral change** (this is not a regression)

Baseline `v6.21.12` = `5a9414cf` (2026-05-08), confirmed ancestor of HEAD. Installed build
`dev-e830a35f` = `e830a35f` (2026-08-06).

| File | `v6.21.12..HEAD` | Verdict |
|------|------------------|---------|
| `bins/cli/src/wallet.rs` | **empty diff** | Error text + `read_to_string` path byte-identical |
| `bins/cli/src/cmd_init.rs` | **empty diff** | Legacy migration byte-identical |
| `bins/cli/src/paths.rs` | +14/−2, one commit: `666ce6c5` (2026-05-09, "add Windows platform support") | **Purely additive `#[cfg(target_os = "windows")]` branch + widened `not(any(…))` guard. ZERO change to the Linux branch or to `resolve_base_dir`'s precedence.** |
| `scripts/install.sh` | +80 lines, all sudoers/staging-path (ISSUE-174 #7) | The `install -d … /var/lib/doli/{mainnet,testnet}` lines are **unchanged** |
| `bins/cli/src/main.rs` | wallet resolution at `:104-107` **unchanged**; `maybe_reexec_with_doli_group` modified | Re-exec now tests `read_dir("/var/lib/doli")` instead of returning early when `…/mainnet` is absent → fires in *more* cases. Affects **access**, never the resolved path. |

`resolve_base_dir`'s existence-based precedence has been unchanged since `ede960bd` (2026-03-21,
"zero-config producer UX", ≈v4.4) — verified with `git log -S "platform_default.exists()"`, which
returns exactly that one commit. `git diff e830a35f..HEAD -- paths.rs wallet.rs main.rs` is **empty**,
so the installed build behaves exactly as the code read above.

**Conclusion: the resolved path and the resolution logic are identical in v6.21.12 and in the
currently installed build. The INC-I-153 upgrade did not cause this.**

## 5. Is the error message itself a defect? — **Yes, independently (Should-fix).**

Assessed separately from the path question:

1. **The Linux branch is strictly less actionable than the non-Linux branch.** macOS/Windows users are
   told `Use -w to specify the wallet path` (`wallet.rs:129-132`). Linux users are told only
   `Create one: doli init` (`wallet.rs:122-125`). The `-w` escape hatch — the *only* way to reach a
   wallet that the precedence chain shadows — is hidden from exactly the platform where shadowing is
   possible (§2).
2. **"Create one" is factually wrong when a legacy wallet exists.** `doli init` would *migrate*, not
   create (`cmd_init.rs:108-126`). The wording misdescribes its own remedy.
3. **Bounded, not catastrophic.** I tried to disprove the "creates a second wallet" fear and it does
   not hold up: `cmd_init.rs:127-137` bails out if a wallet already exists at the target without
   `--force`, and `cmd_init.rs:108-126` migrates a legacy wallet before creating anything. The residual
   risk is narrower but real: if the user's wallet is somewhere the two known locations don't cover
   (a custom `-w` path, another user's `$HOME`), `doli init` silently succeeds, prints a fresh 24-word
   seed phrase, and the operator reasonably concludes *that* is their wallet — while the funded one
   becomes invisible to every subsequent CLI call.
4. **The message discards information it holds.** It names the resolved path but not *why* that path
   won, and does not surface a legacy wallet even though the detection function already exists.

## 6. How the node service finds its data dir

There is **no systemd unit template in the repo** (`ls scripts/*.service` → none; no `.service` file
with `ExecStart` anywhere). Units are generated by `doli service install`
(`bins/cli/src/cmd_service.rs`), which resolves the wallet with
`crate::paths::resolve_wallet_path(network, None, data_dir.as_deref())` at `cmd_service.rs:306` —
**the same resolver**, but with the subcommand's `--data-dir` actually plumbed through (unlike
`main.rs:106`). Per `docs/producer-ux-proposal.md:393`, the unit passes **no `--data-dir`**, so the
node also lands on the platform default `/var/lib/doli/mainnet/`.

**So the CLI and the service agree on the directory.** They disagree on nothing here — which further
supports "the wallet is simply not in that directory" over "the CLI is pointed at the wrong place".

---

## Architecture Context

### Module Boundaries
- **`bins/cli/src/paths.rs`** — sole owner of path resolution. Depends on: `dirs`, env, filesystem
  probes. Depended on by: `main.rs`, `cmd_service.rs`, `cmd_snap.rs`, `cmd_chain.rs`. Owns no state.
- **`bins/cli/src/main.rs`** — composition root. Resolves the wallet path *once* (`:104-107`) and
  passes an already-resolved `&Path` to all ~40 subcommands. Depends on: `paths`, `commands`, `common`.
- **`bins/cli/src/wallet.rs`** — serialization + load/save. Depends on: filesystem, `crypto`. **Has no
  knowledge of the resolution chain** — it receives a `&Path` and can only report what it was handed.
  This is the structural reason the error message cannot explain itself.
- **`bins/cli/src/cmd_init.rs`** — the *write* path. Independently reimplements legacy-location
  awareness (`find_legacy_wallet`, `:26-38`) that `paths.rs` lacks.
- **`scripts/install.sh`** — provisions `/var/lib/doli/{network}`. **No compile-time or test-time
  coupling to `paths.rs`**, yet `paths.rs`'s correctness depends on its behavior. This is the contract
  gap.

### Data Flow Through the Affected Area
```
argv (-w/-n)  ─┐
env DOLI_*    ─┼─→ paths::resolve_wallet_path ──→ PathBuf ──→ main.rs (single resolution point)
filesystem    ─┘        (paths.rs:60-72)                          │
 probes .exists()                                                 ├─→ cmd_wallet::cmd_balance
 on /var/lib/doli/{net}  ← created by install.sh:165              ├─→ cmd_init::cmd_init  (write)
 and ~/.doli/{net}                                                └─→ ~38 other subcommands
                                                                        │
                                                          Wallet::load (wallet.rs:112-139)
                                                          → io::Error surfaces as `Caused by:`
```
Direction is strictly one-way (inputs → path → consumers). No feedback edge, no shared mutable state.

### Architectural Constraints & Invariants
- **INV-A: single resolution point.** `main.rs:104-107` resolves once for every subcommand. A change in
  `resolve_wallet_path` changes *all* of them uniformly — good for consistency, wide for blast radius.
- **INV-B: `resolve_base_dir` is wallet-agnostic.** It also serves snapshots (`cmd_snap.rs:20`) and
  chain data (`cmd_chain.rs:165`), which care about `state_db/`/`block_store/`, not `wallet.json`.
  **Making `resolve_base_dir` gate on `wallet.json` would break both** — a node with a valid data dir
  and no wallet would have `doli snap`/`doli chain` silently retarget to `~/.doli`. Any fix must live
  in `resolve_wallet_path`, not `resolve_base_dir`.
- **INV-C: CLI and node service must agree on the base dir** (§6). A wallet-only override preserves
  this; a base-dir override would violate it.
- **INV-D: never write to a wallet path during diagnosis.** `Wallet::save` (`wallet.rs:142-158`) and
  `cmd_init` are the only writers; both must stay out of any read/diagnostic path.

### Blast Radius (graph-derived, Rule 28)

`python3 .claude/scripts/blast.py graphify-out/graph.json resolve_base_dir --hops 1` → 2 dependents
(`resolve_wallet_path` at `paths.rs:60`, plus one unit test).
`… resolve_wallet_path --hops 1` → 2 dependents, **both unit tests**.

**⚠ The graph under-reports here.** Per project memory (`reference_graphify_rust_method_blind_spot.md`)
graphify misses certain Rust call forms; here it missed all three **fully path-qualified**
`crate::paths::…` call sites. Corroborated by grep, the true dependent set is:

- **Direct — `resolve_wallet_path`:** `bins/cli/src/main.rs:106`, `bins/cli/src/cmd_service.rs:306`.
- **Direct — `resolve_base_dir`:** `bins/cli/src/paths.rs:71`, `bins/cli/src/cmd_snap.rs:20`,
  `bins/cli/src/cmd_chain.rs:165`.
- **Indirect (via `main.rs:106`):** every `Commands::*` arm in `main.rs:118-260+` — ~40 subcommands
  (`balance`, `send`, `spend`, `init`, `producer`, `pool`, `nft`, `token`, `governance`, `channel`,
  `bridge`, `guardian`, …). All receive the same resolved `&Path`.
- **Not affected:** anything outside `bins/cli/`. No node, consensus, storage, network, or RPC code
  depends on these functions.

**Practical containment:** a change confined to `resolve_wallet_path` touches the wallet path for ~40
CLI subcommands but leaves `cmd_snap`/`cmd_chain` (which call `resolve_base_dir` directly) untouched —
which is exactly what INV-B requires.

### Brittleness Check
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details:
  ✗ 1. Cross-module blast radius — NO. Change is one function in one file, one crate.
  ✓ 2. Invariant gaps — YES. No module enforces "the chosen base dir can actually contain the wallet".
       cmd_init enforces it on the write path; nothing enforces it on the read path.
  ✗ 3. Data flow reversal — NO. Resolution is strictly one-directional.
  ✗ 4. Shared mutable state — NO. Filesystem is read-only in this path.
  ✓ 5. Contract absence — YES. install.sh and paths.rs have no explicit contract; paths.rs relies on
       the implicit convention "platform dir exists only if in use", which install.sh violates.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Impact Analysis

### Existing Code Affected (if the fix is taken)
- `bins/cli/src/paths.rs` — `resolve_wallet_path` only. **Risk: low.** Pure function of env +
  filesystem probes; four existing unit tests (`paths.rs:112-137`) pin flag and explicit-dir behavior
  and would continue to pass.
- `bins/cli/src/main.rs:106` — no edit needed; inherits new behavior. **Risk: medium** (breadth: ~40
  subcommands change which file they open when, and only when, the system dir has no `wallet.json`).
- `bins/cli/src/cmd_service.rs:306` — inherits new behavior; a generated systemd unit could reference a
  wallet under `$HOME`. **Risk: medium** — the `doli` service user cannot read another user's `$HOME`.
  Must be excluded or explicitly handled.

### What Breaks If This Changes
- **`doli init` migration semantics** (`cmd_init.rs:108-126`): if `resolve_wallet_path` starts
  returning the *legacy* path, `wallet_path.exists()` becomes true and init would report
  "Wallet already exists at ~/.doli/…" instead of migrating it forward. **Mitigation:** the fix must
  not apply to the `Init` arm, or `cmd_init` must keep resolving the platform target itself.
- **`doli snap` / `doli chain`**: unaffected — they call `resolve_base_dir` directly (INV-B holds).
- **Service unit generation**: see above; mitigate by keeping `cmd_service.rs:306` on the platform
  default.

### Regression Risk Areas
- **Multi-user hosts**: a root/sudo invocation would newly see `~root/.doli/{net}/wallet.json`.
- **Silent target change**: a user who *intends* the system wallet but has a stale `~/.doli/mainnet/`
  would be redirected. Mitigation: the legacy branch already prints a stderr note (`paths.rs:40-49`);
  the new branch must print one too.
- **`doli init` after the fix**: must still create/migrate into the platform dir, never the legacy dir.

---

## Probable Cause

**The resolver is pointed at the right directory, and that directory genuinely has no `wallet.json`
(proven by ENOENT, §3).** Two mutually exclusive causes remain, distinguished by exactly one
read-only host fact:

- **C1 — pure UX/ops, no code defect.** `vm-server` is a non-producer test node. `install.sh` created
  `/var/lib/doli/mainnet/` empty and printed `doli init  # create wallet + keys`
  (`install.sh:214`, `:230`). The user never ran it. The error is **correct**; only the message
  wording (§5) is at fault.
- **C2 — the latent defect fires.** A wallet exists at `~/.doli/mainnet/wallet.json`, and
  `paths.rs:26-29` short-circuits to the (empty) system dir because `install.sh:165` pre-created it,
  so the legacy branch at `paths.rs:32-52` never runs.

**Discriminator (read-only, safe, single command):** `ls -la ~/.doli/mainnet/ /var/lib/doli/mainnet/`
on `vm-server`. `wallet.json` present under `~/.doli/mainnet/` → **C2**. Absent from both → **C1**.

Either way, the §2 contract violation and the §5 message defect are **independently true and provable
from code alone**, and are worth fixing on their own merits.

---

## Stupid Simple First — ONE recommendation

> **The simplest fix that addresses the root cause: in `resolve_wallet_path` (`bins/cli/src/paths.rs:71`),
> if the resolved base dir has no `wallet.json` but `~/.doli/{network}/wallet.json` does exist, return
> the legacy wallet path (with the same stderr migration note already used at `paths.rs:40-49`).**
>
> This works because it makes the *read* path use the exact rule the *write* path already uses
> (`find_legacy_wallet`, `cmd_init.rs:26-38`) — checking for the wallet **file** rather than the
> **directory** — which is precisely the check that `install.sh:165` invalidated, while leaving
> `resolve_base_dir` untouched so `doli snap`/`doli chain`/the service unit keep their current
> directory semantics (INV-B, INV-C).

**Gate:** this code change is warranted **only under C2**. Under C1 the correct outcome is
documentation + message wording (REQ-I161-004/005/006) and **no change to path resolution** — a
legitimate "no code change" result, not a failure.

---

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|--------------------|
| REQ-I161-001 | Determine C1 vs C2 via read-only host enumeration before any code edit | Must | - [ ] `ls -la ~/.doli/mainnet/ /var/lib/doli/mainnet/` captured on vm-server<br>- [ ] No file created/moved/deleted; `doli init` NOT run<br>- [ ] Verdict recorded as C1 or C2 in the incident log |
| REQ-I161-002 | Wallet **read** path must prefer an existing legacy `wallet.json` over a wallet-less platform dir | Must (if C2) / Won't (if C1) | - [ ] Failing test first: platform dir exists + empty, legacy `wallet.json` exists → asserts legacy path; FAILS on HEAD<br>- [ ] After fix, test PASSES<br>- [ ] `resolve_base_dir` signature/behavior unchanged<br>- [ ] stderr migration note emitted exactly once |
| REQ-I161-003 | The fix must not regress `doli init` migration or `snap`/`chain`/`service` base-dir semantics | Must (if C2) | - [ ] `doli init` with a legacy wallet still migrates into `/var/lib/doli/{net}/` (not "already exists")<br>- [ ] `cmd_snap.rs:20` / `cmd_chain.rs:165` resolve unchanged (test-pinned)<br>- [ ] `cmd_service.rs:306` still generates a unit pointing at the platform default |
| REQ-I161-004 | Linux "wallet not found" message must offer the `-w` escape hatch, at parity with non-Linux | Must | - [ ] `wallet.rs:122-125` includes a `-w` example<br>- [ ] Test asserts both cfg branches mention `-w` |
| REQ-I161-005 | The error must disclose a detected legacy wallet instead of saying "Create one" | Should | - [ ] When `~/.doli/{net}/wallet.json` exists, message names it and recommends `doli init` **as a migration**<br>- [ ] When no legacy wallet exists, current "Create one: doli init" text is retained |
| REQ-I161-006 | Fix `docs/cli.md` wallet-location drift (see below) | Should | - [ ] `docs/cli.md:107-108` shows the real Linux default `/var/lib/doli/{network}/wallet.json`<br>- [ ] `docs/cli.md:1987` no longer labels `DOLI_DATA_DIR` node-only<br>- [ ] Documented precedence matches `paths.rs:14-72` link-for-link |
| REQ-I161-007 | Close the installer↔resolver contract gap with a test that fails if it reopens | Should | - [ ] Test asserts: platform dir existing-but-wallet-less does not make the legacy wallet unreachable<br>- [ ] Comment in `paths.rs` cites `scripts/install.sh:165` as the reason the dir-existence probe is insufficient |
| REQ-I161-008 | Tilde-expand `DOLI_WALLET_FILE` / `DOLI_DATA_DIR` for parity with `-w` | Could | - [ ] `DOLI_WALLET_FILE="~/x/wallet.json"` resolves like `-w ~/x/wallet.json`<br>- [ ] Unit test covers both env vars |
| REQ-I161-009 | Add a `doli where` / path-provenance diagnostic printing the full chain and which link won | Could | - [ ] Prints all 6 links + the winner + whether each probed path exists<br>- [ ] Read-only; never writes |
| REQ-I161-010 | Change `resolve_base_dir` itself to be wallet-aware | Won't | N/A — violates INV-B; would silently retarget `doli snap`/`doli chain` |
| REQ-I161-011 | Change the installer to stop pre-creating `/var/lib/doli/{network}` | Won't | N/A — the 2770 setgid dir is the mechanism that lets non-root run `doli init` (`install.sh:163`); removing it breaks the documented onboarding flow |

### Detailed acceptance criteria

**REQ-I161-002**
- [ ] Given `/var/lib/doli/mainnet/` exists and contains **no** `wallet.json`, and
      `~/.doli/mainnet/wallet.json` exists, when `resolve_wallet_path("mainnet", None, None)` is called
      with no env vars set, then it returns `~/.doli/mainnet/wallet.json`.
- [ ] Given both locations contain `wallet.json`, then the **platform** path wins (no silent demotion
      of the canonical location).
- [ ] Given `-w` is supplied, then it wins over both (existing `paths.rs:119-123` test still passes).
- [ ] Given `DOLI_WALLET_FILE` is set, then it wins over both.
- [ ] Given neither location has a wallet, then the platform path is returned (error text unchanged) —
      **this is the C1 case and must not change**.
- [ ] The reproduction test **fails on HEAD before the fix** and passes after (Rule 21, Output Contract).

**REQ-I161-004**
- [ ] Given a Linux build and a missing wallet, when `Wallet::load` fails, then stderr contains both
      `doli init` and a `-w /path/to/wallet.json` example.

## Specs / Docs Drift Detected

- **`docs/cli.md:107-108`** — claims `Wallet saved to: "/home/user/.doli/wallet.json"` and
  `.../wallet.seed.txt`. **Wrong on Linux** (real: `/var/lib/doli/{network}/wallet.json`) and wrong in
  shape — `~/.doli/wallet.json` has no `{network}` segment and is **not** a path any link in
  `paths.rs` can produce. Same shape recurs at `docs/cli.md:41`, `:348`, `:1102`.
- **`docs/cli.md:1987`** — `DOLI_DATA_DIR | Override data directory (node)`. The **CLI** honors it too
  (`paths.rs:21-23`). The "(node)" qualifier is wrong.
- **`docs/cli.md:58`** — lists `DOLI_WALLET_FILE` as the env for `-w`, implying clap `env` wiring.
  `commands.rs:10-12` has **no** `#[arg(env=…)]`; it is read manually at `paths.rs:68`. Net behavior
  matches, but `doli --help` will not show it, unlike `-r`/`-n`.
- **`docs/producer-ux-proposal.md:47` and `:577`** — state the legacy fallback contract that
  `install.sh:165` makes unreachable (§2). Either the doc or the code is wrong; per CLAUDE.md the code
  is SoT, so the **doc describes an intent the code no longer delivers**.
- **`docs/producer-ux-proposal.md:77, 116`** — say mode `0750` for `/var/lib/doli/{net}`;
  `install.sh:164-166` uses `2770`. `docs/producer-ux-proposal.md:579` also promises a
  `doli migrate` command that does not exist in `commands.rs`.

## Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| 1 | `vm-server` is Linux, so `#[cfg(target_os = "linux")]` branches at `paths.rs:76-79` and `wallet.rs:120-126` are the live ones | The output text matches the Linux-only message, so it's a Linux box | Yes — the observed message is emitted only under `cfg(linux)` |
| 2 | Network was left at the default `mainnet` (`commands.rs:19`) | User didn't pass `-n` and `DOLI_NETWORK` wasn't set | Yes — inferred from the `/mainnet/` segment in the error |
| 3 | Neither `DOLI_WALLET_FILE` nor `DOLI_DATA_DIR` was set | No env override was in play | **No** — open question Q2; if either were set the path would still *look* like this only by coincidence |
| 4 | `/var/lib/doli/mainnet` exists on the host | The folder is there (installer makes it) | **No** — inferred from ENOENT-on-file + `install.sh:165`; not directly observed |
| 5 | The installed binary matches HEAD for these files | The code I read is the code that ran | Yes — `git diff e830a35f..HEAD` on `paths.rs`/`wallet.rs`/`main.rs` is empty |
| 6 | No `sg doli` re-exec loop distorted the reported path | The re-exec didn't change the answer | Yes — `main.rs:84-90` re-execs the same argv; resolution is identical either way |

## What I Don't Understand (mandatory)

1. **Whether a wallet exists at `~/.doli/mainnet/wallet.json` on vm-server.** This single fact decides
   C1 vs C2, and I am constrained from checking it. Everything downstream of that branch is
   conditional.
2. **Whether this host ever had a wallet at all.** It is described as a non-producer test node; a
   non-producer may legitimately have none, in which case `doli balance` has never worked there and
   the user's expectation, not the code, is the thing to correct.
3. **Whether `DOLI_WALLET_FILE`/`DOLI_DATA_DIR` are exported in the user's shell profile.** If
   `DOLI_DATA_DIR=/var/lib/doli/mainnet` were set, link #3 (not #4) fired and the §2 defect is
   irrelevant to this incident — the resulting path string is identical, so the message cannot
   distinguish them. This is a genuine observational blind spot (REQ-I161-009 would close it).
4. **What the user expected `doli balance` to show.** If they expected the *node's* balance, note that
   `doli balance` is purely wallet-scoped (`cmd_wallet.rs:143`) — a running non-producer node has no
   wallet-independent "balance", which would make this an expectation mismatch rather than any defect.
5. **Whether `/var/lib/doli/mainnet` on this host is 2770 as installed**, or was altered by the
   INC-I-153 non-root `doli upgrade` incident (project memory records that installing as a non-root
   sudoer produced 0750 binaries). I found no code path in which `doli upgrade` chmods the data dir,
   but I did not audit `crates/updater/` for this incident.

## Identified Risks

- **R1 — Running `doli init` as the diagnostic step destroys the evidence** that distinguishes C1 from
  C2, and on a mainnet-connected host prints a new seed phrase the operator may mistake for their real
  wallet. Mitigation: REQ-I161-001 gates all action behind read-only enumeration
  (already a `⚠️ CONSTRAINT` in the refined prompt).
- **R2 — Fixing the read path without excluding `cmd_init`** would convert migration into a false
  "wallet already exists" report, permanently stranding the wallet in the legacy dir. Mitigation:
  REQ-I161-003.
- **R3 — Fixing the read path without excluding `cmd_service.rs:306`** could emit a systemd unit
  pointing at a wallet under a human `$HOME` that the `doli` service user cannot read. Mitigation:
  REQ-I161-003.
- **R4 — Treating this as a regression** would send the team hunting a diff that provably does not
  exist (§4), burning time. Mitigation: the regression verdict above is explicit and evidence-backed.
- **R5 — Over-fixing.** If the host turns out to be C1, changing `paths.rs` alters wallet resolution
  for ~40 subcommands on every Linux host to solve a problem this host does not have.

## Out of Scope (Won't)

- Changing `resolve_base_dir` semantics (REQ-I161-010) — violates INV-B.
- Changing `scripts/install.sh` directory pre-creation (REQ-I161-011) — it is load-bearing for
  non-root `doli init`.
- Any node, consensus, storage, or RPC change. Nothing in this incident touches block content or
  consensus rules; **no activation height and no synchronized deploy are implicated.**
- Implementing the `doli migrate` command promised at `docs/producer-ux-proposal.md:579`.
- Any host mutation, deploy, restart, or `doli init` on vm-server.

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-I161-001 | Must | (ops step — no test) | Probable Cause | n/a (read-only host enumeration) |
| REQ-I161-002 | Must (C2) | (test-writer) | Data Flow / INV-B | `bins/cli/src/paths.rs` |
| REQ-I161-003 | Must (C2) | (test-writer) | Blast Radius / INV-B, INV-C | `paths.rs`, `cmd_init.rs`, `cmd_service.rs` |
| REQ-I161-004 | Must | (test-writer) | §3 error path | `bins/cli/src/wallet.rs` |
| REQ-I161-005 | Should | (test-writer) | §3 error path | `bins/cli/src/wallet.rs` |
| REQ-I161-006 | Should | (docs check) | Specs/Docs Drift | `docs/cli.md` |
| REQ-I161-007 | Should | (test-writer) | §2 contract gap | `bins/cli/src/paths.rs` |
| REQ-I161-008 | Could | (test-writer) | Resolution chain #2/#3 | `bins/cli/src/paths.rs` |
| REQ-I161-009 | Could | (test-writer) | Resolution chain | `bins/cli/src/` (new subcommand) |

## Open Questions for the Orchestrator (host evidence needed — all read-only)

1. **Q1 (decisive):** `ls -la ~/.doli/mainnet/ /var/lib/doli/mainnet/ 2>&1` → C1 or C2.
2. **Q2:** `env | grep -i '^DOLI_'` → was link #2 or #3 active instead of #4?
3. **Q3:** `id isudoajl` and `stat -c '%a %U:%G' /var/lib/doli /var/lib/doli/mainnet` → confirms the
   group/mode model matches `install.sh:164-166` (and cross-checks the ENOENT deduction in §3).
4. **Q4:** `ls -la /var/lib/doli/mainnet/` — does it contain `state_db/`/`block_store/` (node is
   running there, wallet just absent) or is it empty (never initialized)?
5. **Q5:** `sudo ls -la ~doli/` — is there a wallet under the `doli` service user's home?

---

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.82, verified — full resolution chain read at file:line, git archaeology across v6.21.12..HEAD, graph+grep blast radius; ENOENT/EACCES deduction eliminates the permission branch; one read-only host fact outstanding)
Reasoning: Deterministic, single-crate, one-directional path resolution with a probable cause identified and a latent installer/resolver contract violation proven from code alone; brittleness 2/5 LOCALIZED, no consensus surface, and C1-vs-C2 resolves with a single read-only `ls`.
━━━━━━━━━━━━━━━━━━━━━━
```
