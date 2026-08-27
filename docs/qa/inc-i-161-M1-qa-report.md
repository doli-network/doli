# QA Report: INC-I-161 / M1 — `doli balance --address` must not read the wallet

━━━ FINDINGS — 4 total (HIGH:0 MEDIUM:1 LOW:3) ━━━

  [F1] MEDIUM conf(0.95, measured) — bins/cli/src/wallet.rs:116 — the wallet-load error still ends in `Check file permissions.`, which is the exact nudge that led the jorge operator toward loosening a mode-600 signing key; it now fires only on paths that genuinely need the key, so it is non-blocking but unresolved (OBS-001).
  [F2] LOW conf(0.90, measured) — bins/cli/src/cmd_wallet.rs:146-149 — the `-A` path now bypasses *every* `Wallet::load` failure class, not just EACCES: a corrupt/unparseable `wallet.json` also succeeds where it previously aborted. Intended and desirable, but `docs/cli.md:261-265` only documents "no wallet present, or an unreadable one" (OBS-002).
  [F3] LOW conf(1.00, measured) — bins/cli/src/wallet.rs:206 — pre-existing panic (`index out of bounds: len is 0 but the index is 0`, exit 134) on bare `balance` with a zero-address wallet, byte-identical before and after the fix. Untouched by M1, reported for the backlog (OBS-003).
  [F4] LOW conf(0.85, observed) — graphify-out/2026-08-07/graph.json — the code graph reports 0 dependents for `cmd_balance`, while grep finds a real caller at bins/cli/src/main.rs:138; the known Rust intra-crate blind spot means blast radius here rests on grep, not the graph (OBS-004).

  Speculative: 0
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Verdict

**PASS** — all 7 acceptance criteria verified empirically against real binaries. AC5 output byte-identity holds on every `address.is_none()` path with a readable multi-address wallet against a live loopback node. Zero blocking issues. Four non-blocking observations, all pre-existing or documentation-nuance.

---

## Scope Validated

`bins/cli/src/cmd_balance` (`bins/cli/src/cmd_wallet.rs:137-331`) and its documentation projection `docs/cli.md`. Uncommitted working-tree change on `main`, RUN_ID=496, INC_ID=INC-I-161, milestone M1.

Tracked diff under test (`cmd:$ git diff --stat`):

```
 bins/cli/src/cmd_wallet.rs | 16 +++++++++++++---
 docs/cli.md                | 31 ++++++++++++++++++++-----------
 2 files changed, 33 insertions(+), 14 deletions(-)
```

Plus one untracked test: `bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs`.

## System Entrypoint

The system under test is a CLI binary, not a long-running service. Two binaries were built and compared:

| Binary | Provenance | md5 of `cmd_wallet.rs` at build time |
|---|---|---|
| `doli-postfix` | working tree as-is | `2493b3cf2e8b6cb9a2e52f207f6855f8` |
| `doli-prefix` | `git show HEAD:bins/cli/src/cmd_wallet.rs` temporarily swapped in, then restored | `fdd56c233d1158037a8147ec60603030` |

```
cmd:$ cargo build --release --bin doli
    Finished `release` profile [optimized] target(s) in 42.73s
```

The pre-fix build swapped one file in place and restored it in the same shell invocation. Restoration verified:

```
cmd:$ md5 -q bins/cli/src/cmd_wallet.rs
2493b3cf2e8b6cb9a2e52f207f6855f8      # identical to pre-experiment value
cmd:$ git status --porcelain | grep -v '^?? docs/'
 M bins/cli/src/cmd_wallet.rs
 M docs/cli.md
?? bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs
```

**Node substitute.** `cmd_balance` needs a JSON-RPC peer for `getChainInfo` (ping), `getProducers`, `getBalance`. A deterministic Python stub was run on `127.0.0.1:28991` answering exactly those three methods, with one `active` producer (bond 5000 DOLI) keyed to the test wallet's primary pubkey and one `pending` producer (1000 DOLI) keyed to the second address — so the `Bonded:` / `Activating:` branches of the totals block are actually exercised. `127.0.0.1:28777` was confirmed dead (`curl` exit 7 = connection refused) and used as the "unreachable node" endpoint.

**Fixtures** (scratchpad, all created with the real binary):

| Fixture | Shape |
|---|---|
| `readable.json` | 3-address wallet (`primary`, `second`, `third`), mode 640 |
| `unreadable.json` | byte-copy of the above, `chmod 000` — reproduces the mode-600 producer key case |
| `single.json` | 1-address wallet |
| `zeroaddr.json` | valid wallet JSON with `addresses: []` |
| `malformed.json` | `{ this is not json` |
| `does_not_exist.json` | absent path |

---

## Root-Cause Assessment (doctor workflow)

**The root cause was addressed, not patched over.** Three independent checks:

1. **The load is conditional on need, expressed in the type.** `bins/cli/src/cmd_wallet.rs:146-149` makes `wallet: Option<Wallet>`, `Some` only when `address.is_none()`. The compiler — not a comment — forces both consumers to handle absence (`:212-214` via `Option::iter().flat_map`, `:284` via `if let (false, Some(wallet))`). A symptom patch would have caught the `io::Error` and continued, or added a `--no-wallet` flag, or told the operator to chmod. None of those appear.
2. **No error swallowing, no permission mutation.** `cmd:$ grep -rn -i -E "chmod (a\+r|o\+r|644|755|666|777)|world-readable|loosen the permission|make the wallet readable" docs/cli.md bins/cli/src/` → exit 1 (no match). Positive control that the instrument works: `cmd:$ grep -rn "chmod" scripts/install.sh` → `scripts/install.sh:199: chmod 440 /etc/sudoers.d/doli-update`, exit 0. So the absence is `measured`, not merely unobserved.
3. **The fix matches an existing convention in the same codebase.** `bins/cli/src/cmd_producer/status.rs:16-22` and `bins/cli/src/cmd_producer/delegation.rs:250-255` already load the wallet only in the `None` arm of the explicit-key match. `cmd_balance` was the outlier; M1 brings it into line rather than inventing a new mechanism.

**Blast radius.** `cmd_balance` has exactly one caller: `bins/cli/src/main.rs:138`. Per-root scans: `cmd:$ grep -rn "cmd_balance" crates/` → exit 1 (zero references anywhere in `crates/`); `cmd:$ grep -rn "cmd_balance" bins/` → 7 hits (1 definition, 1 call site, 5 in the new test's prose). `main.rs` passes only a resolved `PathBuf`; `cmd:$ grep -n "Wallet::load" bins/cli/src/main.rs` → exit 1, so no wallet is read before dispatch (control: the same symbol matches 52 times across `bins/cli/src/`). Code graph query returned `0 dependent(s)` for both `cmd_balance` and the file — see [F4].

---

## Acceptance Criteria Results

| AC | Criterion | Result | Evidence |
|---|---|---|---|
| AC1 | `balance -A <addr>`, unreadable wallet → no wallet error, reaches RPC | **PASS** | [E1] |
| AC2 | `balance -A <addr> --all` behaves identically | **PASS** | [E2] |
| AC3 | bare `balance`, unreadable wallet → still fails on wallet | **PASS** | [E3] |
| AC4 | `balance --all` (no `-A`), unreadable wallet → still fails (no dedicated test; verified by hand) | **PASS** | [E4] |
| AC5 | output byte-identity vs pre-fix on all `address.is_none()` paths | **PASS** | [E5] |
| AC6 | `balance -A <addr>` with NO wallet file at all → reaches RPC | **PASS** | [E6] |
| AC7 | zero-address wallet on bare `balance` — behavior unchanged | **PASS** | [E7] |

### [E1] AC1 — address query with a mode-000 wallet (FAIL → PASS pair)

```
cmd:$ doli-prefix  -w <SC>/fix/unreadable.json -r http://127.0.0.1:28777 balance -A doli18kfzk0xx...wzvsqlyqk3
Error: cannot read wallet: <SC>/fix/unreadable.json
  Check file permissions.

Caused by:
    Permission denied (os error 13)
exit=1

cmd:$ doli-postfix -w <SC>/fix/unreadable.json -r http://127.0.0.1:28777 balance -A doli18kfzk0xx...wzvsqlyqk3
Error: Cannot connect to node at http://127.0.0.1:28777. Make sure a DOLI node is running and the RPC endpoint is correct.
exit=1
```

None of `cannot read wallet` / `wallet not found` / `Check file permissions` / `os error 13` appear post-fix; the `Cannot connect to node` line proves execution reached `cmd_wallet.rs:153` (the ping), i.e. past the former abort point.

The real-world happy path was also exercised against the **live** stub, which is the jorge scenario end to end:

```
cmd:$ doli-postfix -w <SC>/fix/unreadable.json -r http://127.0.0.1:28991 balance -A doli18kfzk0xx...wzvsqlyqk3
Balances:
------------------------------------------------------------
doli18kfzk0xxjvkf5rwr4tdw8e6s0njy48u29y3zt8st6pvqrhh2wzvsqlyqk3
  Spendable: 12.12345678 DOLI
  Bonded:    5000.00000000 DOLI  (producer bond)
  Pending:   1.50000000 DOLI
  Total:     5013.62345678 DOLI
exit=0
```

### [E2] AC2 — `--address` wins over `--all`

```
cmd:$ doli-prefix  -w <unreadable> -r <dead> balance -A doli18kfz... --all
Error: cannot read wallet: ... / Permission denied (os error 13)   exit=1
cmd:$ doli-postfix -w <unreadable> -r <dead> balance -A doli18kfz... --all
Error: Cannot connect to node at http://127.0.0.1:28777. ...        exit=1
```

Identical to AC1, confirming `address.is_some()` dominates. Structural basis: `show_per_address = address.is_some() || show_all` (`cmd_wallet.rs:235`), while the load gate keys on `address` alone (`:146`).

### [E3] AC3 — bare `balance` still requires the wallet

```
cmd:$ doli-postfix -w <SC>/fix/unreadable.json -r http://127.0.0.1:28777 balance
Error: cannot read wallet: <SC>/fix/unreadable.json
  Check file permissions.

Caused by:
    Permission denied (os error 13)
exit=1
```

The wallet requirement was not deleted globally.

### [E4] AC4 — `--all` alone still requires the wallet (no dedicated test; hand-verified)

```
cmd:$ doli-postfix -w <SC>/fix/unreadable.json -r http://127.0.0.1:28777 balance --all
Error: cannot read wallet: <SC>/fix/unreadable.json
  Check file permissions.

Caused by:
    Permission denied (os error 13)
exit=1
```

This is the one AC with no automated coverage. It is the branch where the fix could plausibly have over-reached (`--all` sets `show_per_address`, so the totals block is skipped and the wallet is only used by the address-list `else` arm) — it does not: the load gate is on `address`, not on `show_per_address`, so `--all` alone still loads. **Recommend adding a `--all`-only case to `bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs`** (non-blocking; the behavior is correct today, but nothing prevents a future refactor from keying the gate on `show_per_address` and silently deleting this requirement).

### [E5] AC5 — output byte-identity (highest-value check)

Method: run pre-fix and post-fix binaries with identical argv against the **live** stub node on `127.0.0.1:28991`, capture stdout and stderr to separate files, `diff` both plus the exit code. Readable **3-address** wallet used, so the `--all` per-address loop, the aggregate-totals branch (`cmd_wallet.rs:316-328`, requires `query_addresses.len() > 1`), and the consolidated-totals branch (`:284-315`) are all exercised, including the `Bonded:` and `Activating:` lines.

| Case | argv | pre exit | post exit | stdout+stderr |
|---|---|---|---|---|
| bare | `-w readable.json -r <live> balance` | 0 | 0 | **BYTE-IDENTICAL** (sha256 `3f8f1e5beceab016…`) |
| all | `-w readable.json -r <live> balance --all` | 0 | 0 | **BYTE-IDENTICAL** (sha256 `60cdae1cd219761d…`) |
| addr | `-w readable.json -r <live> balance -A doli18kfz…` | 0 | 0 | **BYTE-IDENTICAL** (sha256 `2d81af05a7493f22…`) |
| single-addr bare | `-w single.json -r <live> balance` | 0 | 0 | IDENTICAL |
| single-addr `--all` | `-w single.json -r <live> balance --all` | 0 | 0 | IDENTICAL |
| zero-addr `--all` | `-w zeroaddr.json -r <live> balance --all` | 0 | 0 | IDENTICAL |
| malformed wallet, bare | `-w malformed.json -r <live> balance` | 1 | 1 | IDENTICAL |

Actual post-fix output for the `all` case (pre-fix output is the same bytes):

```
Balances:
------------------------------------------------------------
doli18kfzk0xxjvkf5rwr4tdw8e6s0njy48u29y3zt8st6pvqrhh2wzvsqlyqk3 (primary)
  Spendable: 12.12345678 DOLI
  Bonded:    5000.00000000 DOLI  (producer bond)
  Pending:   1.50000000 DOLI
  Total:     5013.62345678 DOLI

doli1l4vcptg9k5shlh4wsdsyl6qaerfvwehfxy4g0drnz3pzm24e20psen2azu (second)
  Spendable: 53.12345678 DOLI
  Activating: 1000.00000000 DOLI  (pending epoch)
  Immature:  1.00000000 DOLI
  Pending:   2.50000000 DOLI
  Total:     1056.62345678 DOLI

doli1n233d6d4tlh2evqrlavnjkh9l9dzam829j8pggfnuh6drvv57x2shn3u6g (third)
  Spendable: 93.12345678 DOLI
  Immature:  0.75000000 DOLI
  Total:     93.87345678 DOLI

------------------------------------------------------------
Total Spendable: 158.37037034 DOLI
Total Bonded:    5000.00000000 DOLI
Total Activating: 1000.00000000 DOLI
Total:           6164.12037034 DOLI
```

The multi-address wallet requested in the AC brief **was** constructible cheaply (`doli new` + two `doli address` calls), so no part of AC5 is claimed on inference.

> Harness note, disclosed for honesty: the first AC5 run produced no `Bonded:`/`Activating:` lines. Cause was a **harness** bug, not a product bug — zsh does not word-split unquoted parameter expansions, so the three pubkeys reached the stub as one concatenated argv entry, `hex::decode` failed, and `filter_map` (`cmd_wallet.rs:174-179`) dropped every producer. Diagnosed from stub request logging (`RES getProducers [{"publicKey": "156df… c6f55… 9deff…"}]`), fixed by quoting, and AC5 was re-run from scratch. The byte-identity result above is from the corrected run and does cover the bond branches.

### [E6] AC6 — no wallet file at all

```
cmd:$ doli-postfix -w <SC>/fix/does_not_exist.json -r http://127.0.0.1:28777 balance -A doli18kfz…
Error: Cannot connect to node at http://127.0.0.1:28777. ...
exit=1
```

Reaches the RPC step. Against the live node with an empty `DOLI_DATA_DIR` and **no `-w` at all**, it returns a full balance, exit 0 (see [E9] under docs).

### [E7] AC7 — zero-address wallet, bare `balance`

```
cmd:$ doli-prefix  -w <SC>/fix/zeroaddr.json -r <live> balance   → exit 134
cmd:$ doli-postfix -w <SC>/fix/zeroaddr.json -r <live> balance   → exit 134

stdout (both):
Balances:
------------------------------------------------------------

stderr (both, modulo the thread id):
thread 'main' (14234084XX) panicked at bins/cli/src/wallet.rs:206:40:
index out of bounds: the len is 0 but the index is 0
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

`diff` reports exactly one differing line and the difference is the runtime thread identifier (`1423408444` vs `1423408465`) — not a behavioral difference. The panic originates at `wallet.rs:206` (inside `primary_bech32_address`), reached from the totals block. Reachability is unchanged: pre-fix the block ran on `!show_per_address`; post-fix on `!show_per_address && wallet.is_some()`, and `!show_per_address ⇒ address.is_none() ⇒ wallet.is_some()`, so the two conditions are equivalent. Logged as [F3]/OBS-003 — a pre-existing defect M1 neither introduced nor fixed.

---

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Operator on a locked-down producer host queries a third-party balance | mode-000 `wallet.json` → `balance -A <bech32>` → live node → formatted balance | **PASS** | Exit 0, full output incl. bond. See [E1]. |
| Same, with the pubkey-hash hex form | mode-000 wallet → `balance -A <64-hex>` | **PASS** | [E8] |
| Same, with no `-w` and no wallet anywhere | empty `DOLI_DATA_DIR` → `balance -A <64-hex>` | **PASS** | [E9] |
| Owner checks own wallet balance | readable 3-addr wallet → `balance` / `balance --all` | **PASS** | Byte-identical to pre-fix, [E5] |
| Owner with a locked wallet checks own balance | mode-000 wallet → `balance` | **PASS (correctly refuses)** | [E3] |

---

## Exploratory Testing Findings

All against the post-fix binary; the wallet was mode-000 unless stated.

| # | What was tried | Expected | Actual | Severity |
|---|---|---|---|---|
| X1 | `-A not-an-address` | clear parse error, no wallet read | `Error: unrecognized address format: use a bech32 address (doli1...), got 'not-an-address'`, exit 1 | none |
| X2 | `-A ""` (empty) | same | `...got ''`, exit 1 | none |
| X3 | `-A doli1…qk4` (bech32, bad checksum) | rejected | `Error: bech32: no valid bech32 or bech32m checksum` | none |
| X4 | `-A <64-char pubkey hash>` | accepted (documented form) | full balance, exit 0, bech32 echoed back | none |
| X5 | `-A tdoli1…` (wrong network prefix) | rejected | `Error: bech32: no valid bech32 or bech32m checksum` | none |
| X6 | `-A doli1` + 5000 `q` chars | no hang/crash | `Error: bech32: no valid bech32 or bech32m checksum` | none |
| X7 | `-A "doli1café🙂"` (unicode) | no panic | `Error: bech32: parsing failed` | none |
| X8 | `-A <63-char hex>` (one short) | rejected, input truncated in message | `...got '3d922b3cc6932c9a0dc3...'` | none |
| X9 | `-A` twice | clap rejects | `error: the argument '--address <ADDRESS>' cannot be used multiple times` | none |
| X10 | `-A "   "` (whitespace only) | rejected | `...got ''` (trimmed) | none |
| X11 | `-w <a DIRECTORY>` + `-A` | works (wallet never opened) | full balance, exit 0 | none |
| X12 | `-w <a DIRECTORY>`, bare `balance` | fails on wallet | `cannot read wallet: … / Is a directory (os error 21)`, exit 1 | none |
| X13 | `-w <path under a chmod-000 parent>` + `-A` | works | full balance, exit 0 | none |
| X14 | `-w <path under a chmod-000 parent>`, bare | fails on wallet | `wallet not found: … / Permission denied (os error 13)`, exit 1 | low — message says "not found" while the cause is EACCES; pre-existing (`wallet.rs:113` uses `path.exists()`, which is false on EACCES) |
| X15 | malformed (unparseable) wallet + `-A` | ? | succeeds, exit 0 — pre-fix it failed with a parse error | low, see [F2]/OBS-002 |
| X16 | malformed wallet, bare `balance` | fails | `failed to parse wallet file: … / key must be a string at line 1 column 3`, identical pre/post | none |
| X17 | other subcommands (`address`, `addresses`, `info`, `send --help`, `--help`, `balance --help`) | unchanged | all exit 0 with expected output; `balance --help` still documents `-A` and `--all` unchanged | none |

Note on ordering (unchanged by M1, verified explicitly): address parsing happens **after** `println!("Balances:")` and after the ping/`getProducers` round-trips (`cmd_wallet.rs:199-205`), so a bad `-A` prints the header and then the error. Pre-fix and post-fix output for that case with a readable wallet is byte-identical (`diff` clean, both exit 1).

---

## Failure Mode Validation

| Failure scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Node unreachable (dead TCP port) | Yes — `127.0.0.1:28777`, `curl` exit 7 | Yes | n/a | Yes | `Cannot connect to node at …`, exit 1. `RpcClient::ping` (`rpc_client.rs:813-818`) swallows the transport error and returns `Ok(false)`, so the bail is the designed message rather than a raw reqwest error. |
| Wallet unreadable (EACCES, mode 000) | Yes | Yes | n/a — by design | Yes | Address path proceeds; wallet-scoped paths refuse with a specific message. This is the incident itself. |
| Wallet absent | Yes | Yes | n/a | Yes | Address path proceeds; wallet-scoped paths emit `wallet not found` + the `-w` hint. |
| Wallet corrupt (unparseable JSON) | Yes | Yes | n/a | Yes | Wallet-scoped paths emit `failed to parse wallet file`; address path is unaffected ([F2]). |
| Wallet path is a directory (EISDIR) | Yes | Yes | n/a | Yes | `Is a directory (os error 21)`. |
| `getProducers` unavailable / errors | Partially — observed via the harness bug, where every producer failed to decode | Yes (silently) | Yes | Yes | `cmd_wallet.rs:193-197` maps `Err` to empty maps; balances still print, bond lines are simply omitted. Confirmed non-fatal in practice. |
| Zero-address wallet | Yes | **No** | No | **No** | Panics with an index-out-of-bounds instead of a diagnostic. Pre-existing, unchanged ([F3]). |
| Linux `sg doli` re-exec path (`main.rs:46-91`) | **Not triggered (untestable in this environment)** | — | — | — | macOS host; the function is `#[cfg(target_os = "linux")]`. It reads `/var/lib/doli` only, never `wallet.json`, and runs before `Cli::parse()`, so it cannot alter the `cmd_balance` gate. See scope limitation below. |

---

## Security Validation

No independent security-audit report exists for this milestone (`docs/.workflow/security-audit-report-M*.md` absent), so probing was done here.

| Attack surface | Test performed | Result | Notes |
|---|---|---|---|
| Privilege reduction (the point of the fix) | Read a balance with the signing key at mode 000 | **PASS** | Key material is never opened on the `-A` path. Net effect is *less* key exposure than before, not more. |
| Key exposure via remediation advice | `grep -rn -i -E "chmod (a\+r\|o\+r\|644\|755\|666\|777)\|world-readable\|loosen the permission\|make the wallet readable" docs/cli.md bins/cli/src/` | **PASS** (exit 1, with positive control `scripts/install.sh:199` matching `chmod`) | No code, comment, doc, or error text tells the operator to loosen a wallet's permissions. |
| Key exposure via error text | Read every error string emitted on the failing paths | **PASS with caveat** | Errors print the wallet **path** and the errno, never file contents. But `wallet.rs:116` ends with `Check file permissions.` — see [F1]/OBS-001. |
| Wallet-permission invariant | `grep -rn "permission" bins/cli/src/wallet.rs` | **PASS** | `wallet.rs:150-155` still forces `0o640` on save (AUDIT-KEY-001); the diff does not touch it. |
| Injection via `--address` | 10 hostile inputs: empty, whitespace, unicode, 5000 chars, wrong prefix, bad checksum, 63-char hex, repeated flag, `not-an-address`, path-like text | **PASS** | Every input is rejected by `crypto::address::resolve` (`cmd_wallet.rs:204-205`) with a bounded, truncated error message. No panic, no hang, no unbounded allocation, nothing reaches the filesystem. |
| Address value reaching RPC | `-A` value is hashed/decoded before use; only the resolved 32-byte hex hits `getBalance` | **PASS** | Observed on the wire: `REQ getBalance {"address": "3d922b3c…7099"}` — the raw CLI string is never forwarded. |
| Consensus / activation-height surface | `git diff \| grep -i -E "version\|activation_height\|CURRENT_PROTOCOL\|EPOCH_STATE_FORMAT\|MIN_PEER\|HardFork\|consensus"` | **PASS** (exit 1; control: `grep -c wallet` on the same diff = 32) | Zero consensus surface. CLI-only, no block content, no node behavior. Rolling deploy is safe; no activation height needed. |

---

## Traceability Matrix Status

| Requirement | Priority | Has test | Test passes | Acceptance met | Notes |
|---|---|---|---|---|---|
| `-A` must not read the wallet | Must | Yes — `cmd_wallet_balance_with_address_does_not_read_wallet` | Yes | Yes | AC1 |
| `-A --all` must not read the wallet | Must | Yes — `cmd_wallet_balance_with_address_and_all_does_not_read_wallet` | Yes | Yes | AC2 |
| bare `balance` must still require the wallet | Must | Yes — `cmd_wallet_balance_without_address_still_requires_wallet` | Yes | Yes | AC3 |
| `--all` alone must still require the wallet | Must | **No** | n/a | Yes (hand-verified, [E4]) | **Gap** — see below |
| No output change on wallet-scoped paths | Must | No (regression is behavioral, not asserted) | n/a | Yes (byte-diff, [E5]) | Gap is acceptable: byte-identity against a pre-fix binary is not automatable in-repo |
| Docs reflect reality | Should | No | n/a | Yes (7 claims spot-checked) | See docs section |

### Gaps found

- **`balance --all` with no `-A` has no automated test.** Verified by hand ([E4]); recommend a 4th case in the existing test file. Non-blocking.
- **No test pins the byte-identity of wallet-scoped output.** Inherent to the check (needs a pre-fix binary). Mitigated by the fact that the diff cannot alter those code paths structurally.
- Test file `bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs` self-skips when `euid == 0` (`skip_if_root`, line 122). Correct — mode 000 does not stop root — but it means CI running as root would report green without testing anything. Non-blocking; worth a CI note.

---

## Specs/Docs Drift

`docs/cli.md` — every edited claim was spot-checked empirically. **All 7 claims are true.**

| Location | Documented behavior | Actual behavior | Drift |
|---|---|---|---|
| `docs/cli.md:40` | `doli balance --address doli1abc...` (no `-w`) | works: [E9] below | none |
| `docs/cli.md:260-265` | `--address` reads no wallet; works with none present or unreadable | AC1 + AC6 | none |
| `docs/cli.md:263-265` | without `--address` a readable wallet **is** required, "including for `--all` on its own" | AC3 + AC4 | none |
| `docs/cli.md:270-276` | `balance` requires readable wallet; `-A a1b2c3…` no wallet read; `--all` requires readable wallet | AC3, X4, AC4 | none |
| `docs/cli.md:1041-1044` | wallet-scoped needs `-w` off-default-path, else `wallet not found` / `cannot read wallet` | [E10] below | none |
| `docs/cli.md:1053` | `doli balance --address <64-char-pubkey-hash-or-bech32>` — both forms, no `-w` | X4 (hex) + AC1 (bech32) + [E9] (no `-w`) | none |
| `docs/cli.md:1069`, `:1094-1096` | loops calling `balance --address` without `-w` | [E9] | none |

The removed claim (`the -w flag is always required … fails with Error: No such file or directory (os error 2)`) is gone and no residue remains: `cmd:$ grep -n -i "always required\|-w is still required\|still needs -w" docs/cli.md` → exit 1; positive control `grep -n -i "required" docs/cli.md` returns 5 unrelated hits, so the instrument works.

**[E9]** — doc claim "`-A` needs no `-w`":
```
cmd:$ DOLI_DATA_DIR=<empty-dir> doli-postfix -r http://127.0.0.1:28991 balance -A 3d922b3c…7099
Balances:
------------------------------------------------------------
doli18kfzk0xxjvkf5rwr4tdw8e6s0njy48u29y3zt8st6pvqrhh2wzvsqlyqk3
  Spendable: 12.12345678 DOLI
  Bonded:    5000.00000000 DOLI  (producer bond)
  Pending:   1.50000000 DOLI
  Total:     5013.62345678 DOLI
exit=0
```

**[E10]** — doc claim "wallet-scoped without `-w` fails with `wallet not found`" (run with an empty `DOLI_DATA_DIR` so the resolved default genuinely does not exist; the raw default path on this host *does* exist, which would have masked the check):
```
cmd:$ DOLI_DATA_DIR=<empty-dir> doli-postfix -r http://127.0.0.1:28991 balance
Error: wallet not found: <empty-dir>/wallet.json
  Use -w to specify the wallet path, e.g.: doli -w /path/to/wallet.json <command>

Caused by:
    No such file or directory (os error 2)
exit=1
```
Same for `balance --all`. Note the message text is `#[cfg]`-split (`wallet.rs:118-133`): Linux says `Create one: doli init` instead of the `-w` hint. The doc quotes only the generic prefix `wallet not found`, so it is accurate on both platforms.

No `specs/` file describes CLI wallet-file access; nothing to reconcile there.

---

## Quality Gate

```
cmd:$ cargo build --release
    Finished `release` profile [optimized] target(s) in 34.68s
cmd:$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [optimized + debuginfo] target(s) in 1.36s      # zero warnings
cmd:$ cargo fmt --check
    (no output)  exit=0
cmd:$ cargo test -p doli-cli
    192 passed (unit) + 3 + 4 + 5 + 2 + 3 + 3 = 212 passed; 0 failed
cmd:$ cargo test -p doli-cli --test cmd_wallet_balance_address_no_wallet_read
running 3 tests
test cmd_wallet_balance_without_address_still_requires_wallet ... ok
test cmd_wallet_balance_with_address_and_all_does_not_read_wallet ... ok
test cmd_wallet_balance_with_address_does_not_read_wallet ... ok
test result: ok. 3 passed; 0 failed
```

Gate: **GREEN**.

---

## Blocking Issues

**None.**

## Non-Blocking Observations

- **OBS-001** ([F1], `bins/cli/src/wallet.rs:116`) — the EACCES error ends with `Check file permissions.`. On a producer host the correct remedy is *never* `chmod`, and this line is the most likely thing an operator reads before reaching for one. Post-fix it only appears on paths that genuinely need the key, which bounds the damage. Suggested rewording for a future milestone: name the two safe options (run as the owning user / group, or use `--address` for a read-only query) instead of a bare "check permissions".
- **OBS-002** ([F2], `bins/cli/src/cmd_wallet.rs:146-149`) — the `-A` path now bypasses *all* wallet-load failures, including a corrupt `wallet.json` (X15). This is the right semantics (the query genuinely needs nothing from the file), but `docs/cli.md:261-262` enumerates only "no wallet present, or with an unreadable one". Consider "…does not open the wallet file at all".
- **OBS-003** ([F3], `bins/cli/src/wallet.rs:206`) — bare `balance` against a zero-address wallet panics (`index out of bounds`, exit 134) rather than erroring cleanly. Pre-existing and byte-identical pre/post; out of M1 scope.
- **OBS-004** ([F4]) — `blast.py` on `graphify-out/2026-08-07/graph.json` reports `0 dependent(s)` for both `cmd_balance` (exact-label match) and `bins/cli/src/cmd_wallet.rs`, while the real caller is `bins/cli/src/main.rs:138`. Consistent with the recorded graphify Rust blind spot; blast radius for this change rests on grep. No action for the developer.
- **OBS-005** — add a `balance --all` (no `-A`) case to the reproduction test; it is the only Must-level behavior with no automated guard.
- **OBS-006** — `skip_if_root` (test line 122) silently passes the whole file under a root CI runner. Consider failing loudly instead of skipping if CI ever runs as root.

## Scope Limitations

- **Platform.** All empirical evidence is from macOS (darwin 25.5.0). The incident host (jorge) is Linux. The changed logic (`match &address`) carries no `cfg` attributes, and the error branch the operator hit (`path.exists() == true` → `cannot read wallet … Check file permissions`, `wallet.rs:113-118`) is *not* `cfg`-gated, so the text and behavior are identical on Linux. The one Linux-only pre-dispatch step, `maybe_reexec_with_doli_group` (`main.rs:46-91`), touches only `/var/lib/doli` and never `wallet.json`, and runs before `Cli::parse()` — it cannot affect the gate. Not executed here.
- **Node.** A JSON-RPC stub was used rather than a real `doli-node`. It is sufficient because `cmd_balance` consumes exactly three RPC results and the fix does not touch RPC handling; the byte-identity comparison uses the same stub for both binaries, so any stub inaccuracy cancels out.

## Modules Not Validated

None within scope. The diff touches one function and one doc file; both were fully validated.

---

## Final Verdict

**PASS** — All Must acceptance criteria (AC1–AC7) are met, verified by running real binaries rather than by reading code. The root cause (an unconditional `Wallet::load` on a path with no key requirement) was removed structurally and type-enforced, matching an existing convention elsewhere in the CLI; it was not patched over, and nothing anywhere suggests loosening wallet permissions. Wallet-scoped output is byte-identical to pre-fix across every reachable branch, including the bond/activating and aggregate-totals paths. The full quality gate is green, and no consensus, version, or activation-height surface is touched. Approved for review, with six non-blocking observations for the backlog.
