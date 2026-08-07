# Code Review: INC-I-161 / M1 — lazy `Wallet::load` in `cmd_balance`

━━━ FINDINGS — 4 total (Critical:0 Major:0 Minor:4) ━━━

  [F1] MINOR conf(0.90, observed) — bins/cli/src/cmd_wallet.rs:284 — the `if let (false, Some(wallet))` form turns a violation of the `!show_per_address ⇒ wallet.is_some()` invariant into a SILENT fall-through to the `else if` arm; the invariant is pinned by neither the compiler nor any test.
  [F2] MINOR conf(0.95, observed) — bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs:122-137 — the regression guard has two silent-skip paths (root, and `id -u` failure) and its SKIP notice is a `println!`, which cargo captures on a passing run, so a vacuous green is indistinguishable from a real green.
  [F3] MINOR conf(0.85, observed) — bins/cli/src/cmd_bridge.rs:824,895 and :1571-1572 — nearest analogues in the same operator-hazard class: read-only commands open the mode-600 signing key to obtain only a PUBLIC pubkey hash. Out of M1 scope, future milestone.
  [F4] MINOR conf(1.00, measured) — bins/cli/src/cmd_wallet.rs:1-1032 — file is 1032 lines against the 500-line module budget (CLAUDE.md rule 19); pre-existing, diff adds +10 net.

  Speculative: 0
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Summary

**Approved with observations.** No blocking findings. The change is the correct root-cause fix at the
correct layer, it introduces no new panic surface, and the totals-branch refactor is provably behavior-
preserving by construction across all four input combinations. The four findings are latent-fragility and
maintainability items; none blocks merge.

## Scope Reviewed

- `bins/cli/src/cmd_wallet.rs` — `cmd_balance` (`:137-332`), full function read; diff `+13/-3`
- `docs/cli.md` — 5 edited regions
- `bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs` — untracked, full read (532 lines)
- `bins/cli/src/main.rs:100-140`, `bins/cli/src/paths.rs:60-72` — upstream wallet-path resolution
- All 52 `Wallet::load` call sites under `bins/cli/src/` — mapped to enclosing function + signature
- Prior stage: `docs/qa/inc-i-161-M1-qa-report.md` (PASS, 7/7 AC, OBS-001..006) — read, not re-run

---

## 1. Root cause vs. superficial patch

**Position: this is the root-cause fix, at the correct layer. The "the real defect is upstream in `main.rs`"
hypothesis is factually wrong for this incident, and I can show why.**

`main.rs` does not load a wallet. It resolves a *path*:

```
Location: bins/cli/src/main.rs:103-106
Evidence: `let wallet = match &cli.wallet { Some(w) => expand_tilde(w),
           None => paths::resolve_wallet_path(&cli.network, None, None) };`
          — the binding named `wallet` is a `PathBuf`, passed as `&wallet` to ~40 subcommands.
Location: bins/cli/src/paths.rs:60-72
Evidence: `resolve_wallet_path` is pure path arithmetic — flag > `DOLI_WALLET_FILE` env >
          `resolve_base_dir(..).join("wallet.json")`. It never opens the file.
Confidence: conf(0.95, observed)
```

So the upstream is **already lazy**: it defers the read to whoever needs key material. There is no eager
upstream load to remove. Sharing one resolved path across ~40 subcommands is not the defect — it is the
reason the fix is possible at all, because the path is free and only the *read* is expensive.

Three further reasons the per-command gate is the right layer:

1. **Only the command knows whether key material is needed.** `balance -A` needs none; `send` needs the
   private key; `producer status --pubkey` needs none; `producer register` needs the key. Hoisting that
   decision into `main.rs` would duplicate per-subcommand policy in the dispatcher.
2. **The fix matches an idiom that already exists in-tree — `cmd_balance` was the outlier, not the
   pioneer.** Four sites already gate the load on an `Option` argument:
   ```
   Evidence: bins/cli/src/cmd_producer/status.rs:16-22   (handle_status)
             bins/cli/src/cmd_producer/status.rs:190-196 (handle_bonds)
             bins/cli/src/cmd_producer/status.rs:276-282 (handle_vesting_summary)
             bins/cli/src/cmd_producer/delegation.rs:249-255 (handle_delegation_status)
             all four: `let pk = match pubkey { Some(pk) => pk,
                        None => { let wallet = Wallet::load(wallet_path)?; ... } };`
   ```
   This is convergent evidence, not analogy: the same argument shape (`Option<String>` target) already
   produces the same gate elsewhere in the same crate.
3. **The rejected alternative is worse.** A lazy `WalletHandle` / `OnceCell<Wallet>` threaded from
   `main.rs` would touch 52 call sites to fix one defect, and would *reintroduce* the failure class it
   claims to solve: a handle that loads on first deref makes the read point invisible at the call site,
   so the next `cmd_*` that touches `.addresses()` for a display-only reason silently re-acquires the
   signing key with no reviewable diff. The explicit `Option<Wallet>` keeps the read point visible.

**One honest divergence.** The four convention sites load inside the `None` arm and immediately project to
a `String`, so no `Wallet` value escapes the match. `cmd_balance` instead keeps an `Option<Wallet>` alive
across the function because it has two consumers at different points (`:212`, `:286-287`). That divergence
is what creates [F1]. Matching the convention exactly would have required projecting both the query list
and the primary-display string inside the match — more code than a 13-line defect warrants. The choice is
proportionate (CLAUDE.md rule 18, SSF); the residual risk is [F1].

**No contradiction between stated fix and actual change.** The commit-intent ("load the wallet only when it
is actually read") is exactly what the diff does. No retry loop, no caught-and-ignored `io::Error`, no
`--no-wallet` escape flag, no permission advice. Verified independently of QA by reading the full function.

---

## 2. Completeness within scope — other commands with the same shape

**Answer: none with the exact shape. `cmd_balance` was the last one.** Method: every `Wallet::load` under
`bins/cli/src/` mapped to its enclosing function and signature by an AST-lite scan, then each candidate
read.

```
Evidence: cmd:$ python3 <scan> → 52 Wallet::load sites across 20 files, each attributed to its
          enclosing fn + signature. Control: the scan attributed cmd_wallet.rs:148 to `fn cmd_balance`
          with OPT_ARG=True, i.e. it does find the known-positive case.
Confidence: conf(0.90, observed)
```

Classification of all 52:

| Class | Sites | Verdict |
|---|---|---|
| Signing / mutating (send, spend, register, add-bond, exit, delegate, revoke, vote, mint, transfer, sell, buy, pool create/swap/add/remove, bridge lock/claim/refund/swap, token issue, channel open/close/close-finish, init, import, add-bls, sign, release-sign, protocol-sign) | 41 | Wallet is genuinely required on every path. Not the shape. |
| Already gated on an `Option` target (`producer status` / `bonds` / `vesting-summary` / `delegation-status`) | 4 | Already correct — the convention `cmd_balance` now joins. |
| Wallet-display commands with no argument that could avoid it (`addresses`, `address`, `info`, `export`, `history`, `nft list`) | 6 | The wallet file IS the subject of the command. Not the shape. |
| `cmd_balance` | 1 | **Fixed by M1.** |

Subcommand dispatchers were checked for read-only arms that load anyway — they do not:
`cmd_producer/dispatch.rs:28-91` loads only inside mutating arms (`List` and `Slash` load nothing;
`Status`/`Bonds`/`VestingSummary`/`DelegationStatus` delegate to the already-gated handlers);
`cmd_channel.rs:488` (`List`) and `:529` (`Info`) load nothing; `cmd_governance.rs:334` is inside
`UpdateCommands::Vote`, which signs.

**Nearest analogues, weaker class — [F3], future milestone, do NOT fix here:**

```
Location: bins/cli/src/cmd_bridge.rs:824 (load) → :895 (only use on the default path)
Evidence: `let wallet = Wallet::load(wallet_path)?;` … `let from_pubkey_hash = wallet.primary_pubkey_hash();`
          The private key is touched only at :1058 (`wallet.primary_keypair()?`), which sits inside
          `if auto_claim {` at :1034.
Location: bins/cli/src/cmd_bridge.rs:1571-1572 (cmd_bridge_watch)
Evidence: `let wallet = Wallet::load(wallet_path)?;` … `let our_pubkey_hash = wallet.primary_pubkey_hash();`
          — sole use in the function.
Severity: Minor
Confidence: conf(0.85, observed)
```

These are **not** the INC-I-161 shape — the wallet is genuinely read on the default path, so it is not
*provably unused*. But they are the same operator hazard one notch weaker: a read-only command on a
producer host opens the mode-600 signing key to obtain only a **public** pubkey hash. If M2+ ever addresses
"read-only commands should not need the private key file", these are the targets, and the remedy is
different (a public-identity source, e.g. `--pubkey`/cached public identity), not the M1 gate. Reporting
them as enumeration, not as a fix request.

---

## 3. Did the change go too far? (`docs/cli.md`)

**No. The doc edits are in scope, factually correct, and necessary — not scope creep.**

- **Necessary.** The pre-change doc contained a claim the fix makes *false*:
  `docs/cli.md:1035` (pre-diff) — "The `-w` flag is **always required** — even when using `--address` …
  Without `-w`, the CLI fails with `Error: No such file or directory (os error 2)`". Leaving that in place
  would be exactly the drift CLAUDE.md rule 7 and the post-modification checklist step 4 ("Documentation
  alignment (MANDATORY) — update specs/docs BEFORE committing") forbid. A source change that invalidates a
  documented claim and does not fix it is an incomplete change, not a smaller one.
- **Proportionate.** All 5 edited regions are the regions that referenced the now-false claim
  (`:37-41` quick-start, `:257-276` the `balance` reference entry, `:1038-1055` §7.3, `:1066` and
  `:1091-1096` §7.5 examples). No unrelated section was touched. `git diff --stat` → `docs/cli.md | 31 +++---`.
- **Correct.** QA verified all 7 edited claims empirically (`docs/qa/inc-i-161-M1-qa-report.md:338-350`,
  `[E9]`/`[E10]`); I did not re-run those. Structurally I confirm the two load-bearing claims follow from
  the code: "`--address` reads no wallet" follows from `cmd_wallet.rs:146-149`, and "without `--address` a
  readable wallet **is** required — including for `--all` on its own" follows from the gate keying on
  `address` and not on `show_per_address`.
- **One incompleteness, already logged by QA** (OBS-002, `docs/cli.md:261-262`): the paragraph enumerates
  "no wallet present, or … an unreadable one" but the gate now bypasses *every* `Wallet::load` failure
  class including unparseable JSON. Confirmed; wording nit, not an error. No new finding raised.

---

## 4. Unintended behavior changes — the totals-branch truth table

**None. The refactor is behavior-preserving by construction, not merely by measurement.**

Let `a = address.is_some()`, `s = show_all`.

- `wallet.is_some() ⟺ address.is_none() ⟺ ¬a` — `cmd_wallet.rs:146-149`
- `show_per_address = a ∨ s` — `cmd_wallet.rs:235`
- pre-fix guard: `¬show_per_address` = `¬(a ∨ s)` = `¬a ∧ ¬s`
- post-fix guard: `¬show_per_address ∧ wallet.is_some()` = `(¬a ∧ ¬s) ∧ ¬a` = `¬a ∧ ¬s`  **(identical)**

| a | s | pre `if !show_per_address` | post `if let (false, Some(w))` | `else if s ∧ len>1` reached? | pre/post arm |
|---|---|---|---|---|---|
| F | F | **true** → totals | `(false, Some)` → **true** → totals | no | same |
| F | T | false | `(true, Some)` → false | yes; `query_addresses` = wallet addrs, aggregate iff `len>1` | same |
| T | F | false | `(true, None)` → false | reached, but `s=false` → no arm | same |
| T | T | false | `(true, None)` → false | reached; `query_addresses.len()==1` (single CLI address, `:203-208`) → no arm | same |

The `else if` at `:316` is attached to a condition whose *value* is provably identical to the pre-fix
condition on all four inputs, so it is reached on exactly the same input set and its own predicate
(`show_all && query_addresses.len() > 1`) is untouched by the diff. The `(T,T)` cell is the one QA could
not reason about structurally: `--address --all` reaches the `else if`, but `query_addresses` is a
one-element vec by construction at `:203-208`, so `len() > 1` is false and no aggregate block prints —
matching pre-fix. Confirmed for inputs QA did not try.

Rust semantics check: `(show_per_address, &wallet)` is a `(bool, &Option<Wallet>)`; the pattern
`(false, Some(wallet))` binds `wallet: &Wallet` via default binding modes and shadows the outer
`Option<Wallet>` inside the block. `:286-287` then call `&self` methods. Sound.

### [F1] Latent silent-omission hazard

```
Location: bins/cli/src/cmd_wallet.rs:282-284 (and the coupled gate at :146-149)
Evidence: `if let (false, Some(wallet)) = (show_per_address, &wallet) { … } else if show_all && …`
          The `if let` has an `else` arm, so a `(false, None)` state does not fail loudly — it falls
          through to `else if show_all && …`, which with `show_all == false` prints NOTHING after the
          "Balances:" header. The correctness of the totals block therefore rests on the comment at
          :282-283, not on the compiler.
Severity: Minor
Confidence: conf(0.90, observed)
```

Unreachable today — the truth table above proves `(false, None)` cannot occur. The finding is about
*failure mode under future change*: if a later refactor keys the load gate on `show_per_address` instead of
on `address` (precisely the refactor QA's `[E4]`/OBS-005 gap leaves unguarded), `balance` and
`balance --all` would emit a header and no balances, with **no compile error, no panic, and no failing
test**. Silent output loss on a balance command is a worse failure mode than a hard error.

Two mitigations, either sufficient:
1. Add the `--all`-only case to `bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs` (QA OBS-005).
   This is the cheaper and better one — it pins the *behavior*, so it survives any fix shape.
2. Optionally make the impossible state loud rather than silent, e.g. compute the totals-block inputs
   inside the same `match &address` that decides the load, so the type system carries the pairing.

I recommend (1) only; (2) is a judgement call the developer may decline.

---

## 5. Test quality

**It is a real reproduction test, not a vacuous one.** The discriminating pair is O2/O3: P1a and P3a assert
the wallet-access markers are **ABSENT** *and* the `Cannot connect to node` line is **PRESENT**
(`test:…:375-409`). On the pre-fix binary both invert, so the assertions cannot pass by accident — the test
can only go green if execution actually reached `cmd_wallet.rs:153`. Anti-vacuity controls are explicit and
correct:

- `make_unreadable_wallet` (`:180-185`) asserts the fixture is genuinely unreadable and fails as
  "HARNESS FAILURE (not the defect)" otherwise — a positive control on the instrument itself.
- `query_address` (`:193-199`) resolves the literal through the exact resolver `cmd_balance` uses, so a bad
  address literal cannot masquerade as a fix.
- `WALLET_MISSING` is asserted ABSENT on P1a/P3a (`:96-99`, `:361-371`), so a wrong-path harness slip
  cannot be read as the defect being fixed.
- The fixture wallet contains real Ed25519 + BLS key material and valid JSON (`:156-169`), so the test
  cannot pass by the file being unparseable for some other reason.

**The `// OUTPUT CONTRACT:` block is accurate.** It declares `3 outputs x 3 paths x 1 partition = 9 cells
(every cell asserted below)` (`:27`). Verified by counting assertions: P1a → O1/O2/O3 via
`assert_wallet_not_required` (3), P3a → same (3), P2a → O1/O2/O3 inline at `:498-522` (3) = 9. The
single-partition justification at `:16-25` is sound and I agree with it: with a *readable* wallet, "loaded"
and "not loaded" are observationally identical through the CLI's outputs, so a readable-wallet partition is
provably blind to the defect and would add a cell that cannot fail.

**The test-writer's deliberate deviation (live stub for P2a) is sound reasoning, and stronger than stated.**

```
Location: bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs:66-70, :225-275, :485-493
Evidence: `ping()` is `match self.get_chain_info().await { Ok(_) => Ok(true), Err(_) => Ok(false) }`
          — bins/cli/src/rpc_client.rs:813-818. It swallows every error class, so a dead endpoint always
          yields the `Cannot connect` bail at cmd_wallet.rs:153-155 regardless of why the connection failed.
Confidence: conf(0.95, observed)
```

The writer's stated reason — a dead endpoint would make P2a fail spuriously against a "ping first, load
later" fix shape — is correct. The stronger reason, which the writer understates: with a dead endpoint,
P2a could not **discriminate** at all. A good reorder-fix and a bad fix that deleted the wallet requirement
entirely would *both* produce `Cannot connect to node` with no wallet marker, so the O2 assertion would
fail identically in both cases. The live stub forces execution past the ping, so the guard tests the
behavioral invariant ("a bare `balance` requires a readable wallet") rather than the current statement
order. That is the right instinct for a regression guard, and P2a's O3 assertion (`:517-522`, flagged as
"HARNESS FAILURE" if the unreachable line appears) closes the loop by proving the stub actually answered.
I endorse the deviation.

Incidental: the dead-endpoint idiom (`:203-208`, bind port 0 → read port → drop) has a theoretical port-
reassignment race, but it is benign here — a foreign responder would fail `get_chain_info` deserialization,
`ping` returns `Ok(false)` per the evidence above, and the test lands on the same expected bail. No finding.

### [F2] The guard can go green without testing anything

```
Location: bins/cli/tests/cmd_wallet_balance_address_no_wallet_read.rs:116-137
Evidence: `fn skip_if_root(...) -> bool { match effective_uid() { Some(0) => { println!("SKIP …"); true }
           Some(_) => false, None => { println!("SKIP …"); true } } }`
          — and each test's first statement is `if skip_if_root(..) { return; }` (:427, :456, :486).
          `effective_uid()` is `Command::new("id").arg("-u").output().ok()?` (:117), so a missing/failing
          `id` binary also returns None → skip.
Severity: Minor
Confidence: conf(0.95, observed)
```

QA logged the root case (OBS-006). Two things QA did not state, and they matter:

1. There are **two** silent-skip paths, not one — `Some(0)` and `None`. The `None` arm skips on any
   failure to determine the uid, which is a fail-open default in a security-relevant guard.
2. The skip notice is `println!`, and **cargo captures stdout on passing tests**. So a run that skipped all
   three tests prints `test result: ok. 3 passed` and nothing else — byte-indistinguishable from a real
   pass unless someone passes `--nocapture`. The notice exists but is invisible exactly when it matters.

Suggested fix: keep the skip for local root shells, but make it observable — either `panic!` with an
explicit opt-out env var (`DOLI_ALLOW_ROOT_TEST_SKIP=1`), or emit the notice on **stderr** (not captured
the same way) plus a `#[should_panic]`-free hard failure when a CI marker such as `CI=true` is set. Any of
these turns a silent green into a loud one.

---

## 6. Error-path panic surface

**No new panic is reachable, and the pre-existing one is reached under exactly the same conditions.**

```
Evidence: cmd:$ git diff -U0 -- bins/cli/src/cmd_wallet.rs | grep '^+' | grep "unwrap()\|expect(\|panic!\|\[0\]\|\[1\]"
          → exit 1 (no match). Positive control: `grep -c "unwrap_or\|\[0\]" bins/cli/src/cmd_wallet.rs`
          → 23, so the pattern does match this file and the empty result is a true zero.
Confidence: conf(0.95, measured)
```

Line by line on the added code:
- `match &address { Some(_) => None, None => Some(Wallet::load(wallet_path)?) }` (`:146-149`) — `?`
  propagates a `Result`; no panic.
- `wallet.iter().flat_map(|w| w.addresses())` (`:212-214`) — `Option::iter` yields 0 or 1 items; no
  indexing, no `unwrap`. Strictly safer than the pre-fix `wallet.addresses().iter()`.
- `if let (false, Some(wallet)) = …` (`:284`) — refutable pattern with an `else` arm; no panic. (Its
  silent-fall-through property is [F1], which is a correctness-under-change concern, not a panic.)

**Pre-existing panic (QA OBS-003) — reachability unchanged.**

```
Location: bins/cli/src/cmd_wallet.rs:287 → bins/cli/src/wallet.rs:204-208
Evidence: `let label = wallet.addresses()[0].label…` at cmd_wallet.rs:287, and
          `hex::decode(&self.addresses[0].public_key)` at wallet.rs:206 inside
          `primary_bech32_address` — both index `[0]` on a possibly-empty vec.
          `git diff --stat` → only `bins/cli/src/cmd_wallet.rs` and `docs/cli.md`; wallet.rs is untouched.
Confidence: conf(0.95, observed)
```

Both `[0]` sites sit inside the totals block, whose guard is proven in §4 to be the identical boolean
function pre and post. Therefore the panic is reached on exactly the input set `¬a ∧ ¬s` before and after —
the fix neither widened nor narrowed it. Pre-existing, out of M1 scope, correctly left alone. (Note for the
backlog: the same `wallet.addresses()[0]` hazard exists at the four convention sites in
`cmd_producer/status.rs` and `delegation.rs`; a zero-address-wallet fix should cover all five together.)

---

## 7. Specs/docs drift beyond `docs/cli.md`

**No residual drift. One incidental improvement.**

```
Evidence: cmd:$ grep -rn -i -- "-w is still required|always required|still needs -w|requires -w" docs/ specs/ README.md .claude/skills/
          → only docs/.workflow/milestone-progress.md:10 and docs/qa/inc-i-161-M1-qa-report.md:350,
            both of which *describe* the removal rather than assert the stale claim.
          Positive control: `grep -c -i "required" docs/cli.md` → 5, so the instrument matches this corpus.
Confidence: conf(0.90, measured)
```

```
Evidence: cmd:$ grep -rn -- "balance --address|balance -A" docs/ specs/ README.md .claude/skills/
          → outside docs/cli.md and the QA report, only:
            .claude/skills/doli-manager/doli-manager/SKILL.md:45
            .claude/skills/doli-manager/doli-manager/references/wallet.md:52-53
          All three show `doli balance --address …` with NO `-w`, and neither file asserts that `-w` is
          required (context read at SKILL.md:38-50, wallet.md:40-62).
Confidence: conf(0.90, measured)
```

Those skill snippets were *wrong before this change* — they documented an invocation that would have failed
on a host without a default-path wallet. The fix retro-validates them. No edit needed; noting it so nobody
"corrects" them back later.

`specs/` contains no description of CLI wallet-file access — nothing to reconcile (consistent with QA
`:377`). No `specs/protocol.md` or `specs/security_model.md` surface is touched: the change is CLI-only,
with no consensus, block-content, activation-height, or protocol-version implication, so neither of the two
CLAUDE.md deploy questions is triggered and a rolling deploy is safe.

### [F4] Module size

```
Location: bins/cli/src/cmd_wallet.rs:1-1032
Evidence: cmd:$ wc -l bins/cli/src/cmd_wallet.rs → 1032
Severity: Minor
Confidence: conf(1.00, measured)
```

1032 lines against the 500-line source budget (CLAUDE.md rule 19). Pre-existing — the file was already
1022 lines and the diff adds +10 net. Not introduced by M1 and not a merge blocker, but recorded because
the checklist requires it and because the file hosts 14 subcommands that split cleanly along the
query/sign boundary (`cmd_balance`/`cmd_history`/`cmd_info`/`cmd_addresses` vs `cmd_send`/`cmd_spend`/
`cmd_sign`). Recommend as a separate refactor milestone, never bundled with a behavioral fix.

---

## Verification of prior stage

QA's report is internally consistent and its evidence is discriminating. Two items I spot-checked rather
than trusted, because they are load-bearing for the verdict:

- QA `:242` claims panic reachability is unchanged via `¬show_per_address ⇒ address.is_none() ⇒
  wallet.is_some()`. Independently re-derived in §4 — correct.
- QA `:78` claims no code or doc suggests loosening permissions. Independently re-checked as part of §3 and
  §7: the diff touches neither `wallet.rs` (which still forces `0o640` on save) nor any remediation text,
  and adds no `chmod` advice. Confirmed. **No permission-loosening remedy appears anywhere in this change,
  and none should be accepted in a follow-up** — on `jorge`, `wallet.json` is the only copy of the producer
  signing key.

No contradiction found between the diagnosis, the architecture intent, the implementation, and the QA
result. Nothing to escalate to the Architect; the design is sound at the layer chosen.

---

## Modules Not Reviewed

None within scope. The milestone touches one function, one doc file, and one new test; all three were read
in full. The 51 non-`cmd_balance` `Wallet::load` sites were classified from signature + call-site context
rather than by reading every enclosing function in full — sufficient for the enumeration in §2, and the two
candidates that survived screening ([F3]) were read directly.

---

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 process spawn + 1 TCP bind per added test case (~50ms, test suite only); 0 in the shipped binary (observed)
  Memory:   0 — the recommended changes add no allocation to any runtime path (observed)
  IO:       +1 tempdir wallet fixture write + chmod per added test case; 0 in the shipped binary (observed)
  Network:  +1 loopback bind/connect per added test case; 0 in the shipped binary (observed)
  Disk:     +~1KB ephemeral tempfile per added test case, reclaimed on drop; 0 persistent (observed)
  Latency:  0 — no change to any user-facing command path (inferred, from the diff touching only test and skip-guard code)
Inevitability: AVOIDABLE
Cheaper alternative: rely on the manual verification QA already performed for `balance --all` ([E4]) and leave the guard's root-skip as-is — zero cost, zero new code.
Why this proposal anyway: the cheaper path leaves the only Must-level behavior without an automated guard, and per [F1] its failure mode is SILENT output omission rather than an error — a regression that manual verification catches only if someone happens to re-run it. [F2] compounds this: the existing guard can report green having tested nothing. ~50ms of test-suite time buys a loud failure for both.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Security Audit Verdict: AUDIT-REQUIRED

```
━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: Cryptography / key management — the change modifies the condition under which the CLI opens
         `wallet.json`, which on mainnet producer hosts (jorge) is the live Ed25519 + BLS signing key at
         mode 600 and the only copy on the host. The milestone's correctness rests on an invariant
         (`!show_per_address ⇒ wallet.is_some()`) enforced by neither the compiler nor any test ([F1]),
         and its regression guard can report green without executing ([F2]).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**I am deliberately not taking the cheap answer, and I want the reasoning on record because the cheap
answer here is `AUDIT-SKIP` and it is genuinely defensible.**

The case for SKIP is strong. The change is a strict *narrowing*: it never adds a credential read, it removes
one on a subset of paths. It introduces no new external input — `--address` was already parsed by
`crypto::address::resolve` on the same path before the fix, and the raw string still never reaches the
filesystem or the wire (only the resolved 32-byte hash does). It touches no crypto primitive, no auth, no
serialization, no network protocol, no consensus surface, and no enforcement/deploy surface (no hooks, no
installer, no `settings.json`, no agent/command/protocol instruction text — `docs/cli.md` is user
documentation, not executable instruction text). Net credential exposure after the change is provably
≤ exposure before it.

I tried to disprove SKIP three ways and two of the attempts failed:
1. *Can the change cause key material to be exposed rather than merely unread?* No — the diff adds no I/O,
   no printing of wallet content, no transmission; the only wallet fields touched are `public_key` and
   `label`, on the path where the wallet was already fully loaded pre-fix. Attempt failed.
2. *Does it make an authorization decision?* No — the filesystem mode bits are the authorization, and
   `wallet.rs` is untouched by the diff (`git diff --stat` shows two files, neither is `wallet.rs`), so the
   `0o640`-on-save invariant from AUDIT-KEY-001 still holds. Attempt failed.
3. *Is the invariant that makes it safe actually enforced?* **No.** This attempt succeeded. The safety of
   the change reduces to `¬show_per_address ⇒ wallet.is_some()`, which is currently guaranteed only by the
   coupling between `:146` and `:235` and documented only in a comment at `:282-283`. Neither the compiler
   nor any test pins it, and if it is ever broken the code fails *silently* ([F1]) — and the guard that
   would notice can itself pass without running ([F2]).

That third result is what decides it. The governing rule is signal-presence-based, not risk-based: deciding
when a private-key file is opened *is* key management, so a signal is present, and one signal is enough. On
top of that, an unenforced implicit invariant governing credential access is precisely the class of
assumption an independent multi-auditor sweep exists to attack — a single reviewer proving a truth table
is exactly the evidence that looks strongest right up until the premise is wrong.

I expect the sweep to find nothing, because the change's direction is a strict reduction in credential
exposure. I am recommending it anyway: a false positive costs one sweep, a false negative costs a mainnet
producer signing key.

---

## Final Verdict

**Approved for merge — with observations.** The root cause was removed at the correct layer and in the
codebase's own existing idiom, not patched over; the totals-branch refactor is provably behavior-preserving
on all four input combinations including the `(--address, --all)` cell QA reached only empirically; no new
panic surface is introduced and the pre-existing one is reached under identical conditions; the reproduction
test is genuine and its stub-vs-dead-endpoint deviation is well-reasoned; the doc edits are required by the
project's own sync rule, not scope creep; and no other command in `bins/cli/src/` carries the same
provably-unused-load shape.

None of [F1]–[F4] blocks the merge. [F1] and [F2] share one root — the milestone's safety invariant is
pinned by neither compiler nor test — and both are closed by adding the `balance --all` (no `-A`) case that
QA already recommended as OBS-005, plus making the root-skip loud. I recommend doing that before commit
because it is ~20 lines in a file that already exists; I do not consider it a gate. [F3] and [F4] are
future milestones and must not be bundled into this fix.

Deploy note: CLI-only, no consensus rules and no block content — neither CLAUDE.md deploy question is
triggered, no activation height, rolling deploy safe.
