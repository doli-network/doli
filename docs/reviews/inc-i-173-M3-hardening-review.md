━━━ FINDINGS — 10 total (HIGH:3 MINOR:7) ━━━

  [F1] HIGH conf(0.95, measured) — **CLOSED** — bins/node/src/node/validation_checks.rs:319-334 — REV-173-M3-001: every "never-crossed"/"tip ~130_291" assertion corrected; I re-measured `bestHeight` 134,480 on 127.0.0.1:8500/8501/8502 (v6.24.1) vs gate 133,000, and `git diff 32e0a650 -- crates/core/src/network_params/` is still EMPTY, so no height VALUE was moved to "fix" it
  [F2] HIGH conf(0.92, measured) — **CLOSED** — crates/updater/src/trust_root.rs:181-265 — REV-173-M3-002: the short-circuit is now conditioned on `journal.records.is_empty()`; the 4-cell truth table proves a strict NARROWING of acceptance (no shape gained acceptance), and `rev_f2_*` (3 tests) + the anti-vacuity pair pass 14/14
  [F3] HIGH conf(0.95, measured) — **CLOSED** — .omega/memory.db protection_mechanisms — REV-173-M3-003: PM-173-01..PM-173-06 registered with `interacts_with` populated, PM-172-07 amended, OBS-3 filed as INC-I-174 (high, open); gauntlet run is owed at M2 deploy time, declared not worked around
  [F4] MINOR conf(0.95, measured) — **CLOSED** — crates/mempool/src/pool.rs:740-761 — REV-173-M3-004: stale comment rewritten with the genesis-window reachability bound; `Registration` added to the delta table at specs/state-only-fee-gate-architecture.md:308
  [F5] MINOR conf(0.85, observed) — **CLOSED** — crates/core/src/consensus/params.rs:130-132 + crates/updater/src/trust_root.rs:356 + bins/node/src/commands/maintainer.rs:18-41 — REV-173-M3-005: document-and-name-the-winner is DEFENSIBLE and leaves no live trap; I verified the detection path end-to-end (governance.rs:99 publishes `self.params.genesis_hash` — the named winner — on all three RPC branches, and `print_binding` prints the signer's)
  [F6] MINOR conf(0.95, measured) — **CLOSED** — bins/node/src/updater/trust_root_wiring.rs:238-244 — REV-173-M3-006: the `1u8`/`0u8` discriminating tag is implemented and the comment now describes the code
  [F7] MINOR conf(0.95, measured) — **CLOSED** — specs/state-only-fee-gate-architecture.md:309 — REV-173-M3-007: `RequestWithdrawal` reclassified `none` at the contract, the QA report and the spec; probe results retained as evidence
  [F8] MINOR conf(0.80, measured) — **NEW/OPEN** — crates/core/src/network_params/mod.rs:630-633 — the field doc still reads "`133_000` … 2_709 blocks ≈ 7.53 h of lead" with no crossed marker, and CLAUDE.md sends readers to `network_params/` as the SoT for activation heights
  [F9] MINOR conf(0.85, measured) — **NEW/OPEN** — specs/state-only-fee-gate-architecture.md:296-309 — the table headed "the COMPLETE list" has no row for `AddMaintainer`/`RemoveMaintainer`, though they are structurally identical to the `DelegateBond`/`RevokeDelegation` row it does carry
  [F10] MINOR conf(0.75, observed) — **NEW/OPEN** — .omega/memory.db PM-172-07 scale_assumptions — the amendment replaces "M3 with its own activation height" with "M3 rides the EXISTING inc_i_173_activation_height", but `MaintainerChangeData::signing_message` (data.rs:86) takes no height and is called unconditionally at governance.rs:43,83 — the domain tag is UNGATED

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-173 M3 — Hardening Before Deploy

Incident INC-I-173, milestone M3, run 511/514, workflow `redesign`.
Branch `bugfix/inc-i-173-state-only-fee-gate`, base `32e0a650`, working tree DIRTY and UNCOMMITTED.

**Security Audit Verdict: AUDIT-REQUIRED**

Signals unchanged from iteration 0, and iteration 1 REINFORCES them: the one code change in this
iteration edits `TrustRoot::resolve` — the single decision function that authorises every root binary
install on every host, including ~30 external auto-update producers. That is the enforcement surface
itself, not code behind it. Also in scope: Ed25519 quorum verification, a signed-message format with
domain separation and chain binding, an attacker-writable on-disk file parsed on a public RPC path,
consensus-visible transaction-payload bounds, and mempool admission routing. Multiple rows of the
trust-boundary taxonomy.

---

# PART A — RE-REVIEW (review iteration 1, 2026-08-11)

Scope of this pass: the remediation ONLY, plus regression. The iteration-0 review is retained verbatim
in PART B as the record of what was found.

Reviewed artifact: `docs/.workflow/inc-i-173-M3-implementation.md` §12 (lines 1051-1413) plus the
files it names. Interactions with the live network were **three read-only `getChainInfo` RPC calls**.
No commit, no push, no deploy, no restart, no SSH, no source file modified by this review.

## A.1 Summary

**Approved. All seven findings CLOSED.** Three new MINOR findings, all documentation or
institutional-memory accuracy, none blocking.

The one finding with real security consequence — F2 — is closed correctly and, importantly, closed in
the *safe direction*: the change is a provable narrowing of what `resolve` accepts, not a rewrite that
could have inverted the posture. I verified that by enumerating the branch predicate rather than
trusting the prose.

What is no longer a review blocker but remains a **deploy** obligation, now correctly recorded rather
than mis-stated: the testnet deploy is SYNCHRONIZED (stop-all then start-all), M2 must re-pin the
testnet height above the then-current tip and re-verify the tip immediately before pinning, and the
gauntlet run owed by the system-impact protocol is deferred to M2 deploy time because this milestone
forbids node restarts. Workflow run 514 is deliberately left `running` for that reason — declared, not
worked around, which is the correct handling.

## A.2 Build gate — re-run independently, all green

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --release` | **PASS** — exit 0 |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** — exit 0, `grep -c '^warning'` = 0 |
| Format | `cargo fmt --check` | **PASS** — exit 0, output 0 bytes |
| Tests | `cargo test --workspace --no-fail-fast` | 159 `test result` lines; **3550 passed / 4 failed** |

### The 4 failing instances are the 3 KNOWN failures. Zero new.

`test_cluster_10x100` is compiled into TWO binaries (`tests/test_network.rs` and
`tests/checkpoint_rotation.rs`, which carries `mod test_network`), so one known failure produces two
failing instances when both copies lose the fd race. The developer's §12.11 run saw the
`checkpoint_rotation` copy pass and reported 3; mine saw it fail and reports 4. Same test, same panic
site, opposite side of the same flake.

| Target | Test | Panic | Classification |
|---|---|---|---|
| `tests/test_network.rs` | `test_network::test_cluster_10x100` | `bins/node/tests/test_network.rs:55:37` — `Too many open files` | KNOWN environmental |
| `tests/test_network.rs` | `test_network::test_onchain_liveness_10k_nodes` | same site; `Node 973 init failed … OPTIONS-000006.dbtmp: Too many open files` | KNOWN environmental |
| `tests/checkpoint_rotation.rs` | `test_cluster_10x100` (second copy) | same site; `Node 69 / Node 95 init failed … Too many open files` | KNOWN environmental, same test |
| `mempool --lib` | `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` | `crates/mempool/src/contention_tests.rs:1108:9` | KNOWN pre-existing at base |

**Recorded, not fixed, per instruction.**

### All 12 M3 test targets pass, plus the M2 target the milestone inverted

```
inc_i_173_m3_rotation_journal_guard        ok. 14 passed; 0 failed
inc_i_173_m3_rotation_replay               ok.  7 passed; 0 failed
inc_i_173_m3_qa1_trust_root_cache          ok.  6 passed; 0 failed
inc_i_173_m3_option_e_apply_path           ok.  8 passed; 0 failed
inc_i_173_m3_option_e_binding              ok. 11 passed; 0 failed
inc_i_173_m3_payload_bounds                ok. 19 passed; 0 failed
inc_i_173_m3_f4_routing                    ok. 10 passed; 0 failed
inc_i_173_m3_f7_cross_list                 ok.  4 passed; 0 failed
inc_i_173_m3_maintainer_digest             ok. 10 passed; 0 failed
inc_i_173_m3_maintainer_set_rpc            ok.  7 passed; 0 failed
inc_i_173_m3_rotation_journal_store        ok. 11 passed; 0 failed
inc_i_173_m3_qa2_journal_path_and_version  ok.  5 passed; 0 failed
inc_i_172_m2_release_sign_arg_validation   ok.  9 passed; 0 failed   (AUDIT-P0-011, the INVERTED test)
```

## A.3 Hard prohibitions — re-verified mechanically against `32e0a650`

| # | Prohibition | Verification this pass |
|---|---|---|
| 1 | No activation-height VALUE moves | `git diff 32e0a650 -- crates/core/src/network_params/` → **EMPTY** (exit 0, no hunks). Values re-read from source: mainnet `u64::MAX` (`defaults.rs:275`), testnet `133_000` (`:480`), devnet `0` (`:631`) |
| 2 | No version constant bumped | `MAINTAINER_STATE_VERSION` still `1` (`crates/storage/src/maintainer.rs:54`). No `Cargo.toml` in the 38-file diff |
| 3 | L1/L2 character-identical | `diff <(git show 32e0a650:…/validation/transaction.rs \| sed -n '39,88p') <(sed -n '39,88p' …)` → **no output**. The file's only diff vs base is at `:168-178` (two `validate_maintainer_change_data(tx)` → `(tx, ctx)` call sites), outside L1/L2 |
| 4 | Below-gate frozen branch untouched | `git diff 32e0a650 -- crates/core/src/validation/utxo.rs` → **EMPTY**. Re-read `utxo.rs:239-248`: frozen branch is still `{Registration, DelegateBond, RevokeDelegation}` |
| 5 | Apply path stays NON-FATAL | Re-read `apply_block/governance.rs:1-110`: every rejection arm is `warn!`+skip (`:64`, `:67`, `:104`, `:107`); `record_rotation` is `warn!`-only on both load and save failure (`:212-219`, `:243-248`). `prohibition_5_the_apply_path_is_non_fatal_on_every_rejection_shape ... ok` |
| 6 | No deploy / SSH / restart; RPC READS only | Three `getChainInfo` POSTs to `127.0.0.1:8500/8501/8502`. No `cp`, `codesign`, `ssh`, `launchctl`, `systemctl` |
| 7 | No commit, no push | `git log` head still `32e0a650`; working tree still dirty |

## A.4 [F1] CLOSED — the "never-crossed" premise is corrected, and no height was moved

**Independent re-measurement, this session:**

```
$ curl -s -X POST http://127.0.0.1:{8500,8501,8502} -d '{"jsonrpc":"2.0","method":"getChainInfo",...}'
  → {'bestHeight': 134480, 'network': 'testnet', 'version': '6.24.1'}   (all three IDENTICAL)
crates/core/src/network_params/defaults.rs:480 → inc_i_173_activation_height: 133_000
```

134,480 > 133,000. **Crossed by 1,480 blocks**, up from the 1,159 the developer measured — the gap
widens at ~10 s/block, which is itself the argument for the M2 re-verify-immediately-before-pinning
duty.

The corrected text at `bins/node/src/node/validation_checks.rs:319-334` states all six things the
brief required, and I checked each against reality rather than against the report:

| Required assertion | Present at | Matches my measurement? |
|---|---|---|
| gate already crossed on testnet | `:321-323` ("the live testnet tip measured 134_159") | YES — my 134,480 is the same side of 133,000 |
| staged-activation safety gone until M2 re-pins | `:323-325` ("becomes active the moment the binary lands, not at a future scheduled height") | YES |
| SYNCHRONIZED stop-all/start-all, INV-8 / INC-I-062 | `:325-329` | YES, and the mechanism is stated correctly (a new-binary producer mines an `AddMaintainer` old-binary nodes reject) |
| history NOT invalidated — strictly more permissive | `:330-333` | YES, and independently true: `is_zero_flow` above the gate admits the frozen three PLUS `AddMaintainer`/`RemoveMaintainer` (`utxo.rs:239-248` vs `types.rs:184-188`), a superset |
| mainnet `u64::MAX`, devnet `0` unaffected | `:329-330` | YES — `defaults.rs:275` and `:631` re-read |
| M2 re-verifies the tip immediately before re-pinning | `:333-334` | YES |

**No activation-height VALUE was changed to "fix" this** — prohibition 1 above, empty diff. The
correction is entirely comment, spec and workflow text.

**Residual → new finding [F8]** (below): `crates/core/src/network_params/{defaults.rs,mod.rs}` were
deliberately left alone to keep prohibition 1's empty-diff proof intact. That trade is reasonable for
`defaults.rs` (its text is explicitly a "Re-pin history" block) but leaves `mod.rs:630-633` reading
"2_709 blocks ≈ 7.53 h of lead" as a present-tense property — at the exact location CLAUDE.md names
as the SoT for activation heights.

## A.5 [F2] CLOSED — probed properly, in all three directions the brief named

This is the finding with real security consequence, so I did not accept the report's table. I derived
the behaviour from the branch predicate.

**The code** (`crates/updater/src/trust_root.rs:181-265`):

```rust
if !keys.is_empty() {
    let is_bootstrap_set = is_chain_derived_bootstrap_set(&keys, network);
    if !is_bootstrap_set || !journal.records.is_empty() {
        if let Some(replayed) = replay_onto(&keys, threshold, network, journal) { ...
            return Self::on_chain(keys, threshold);          // ACCEPT
        }
        ... error!("TRUST_ROOT_CONTAINED: ...")
        return Self::on_chain(Vec::new(), threshold);        // FAIL CLOSED
    }
    ...
    Self::on_chain(keys, threshold)                          // short-circuit
```

**Direction 1 — the state-file-wipe attack now fails CLOSED.** With `keys` == the chain-derived five
and a surviving non-empty journal, `is_bootstrap_set` is `true` but `!journal.records.is_empty()` is
`true`, so the branch is entered. `replay_onto` cannot return `Some` for a journal that rotated AWAY
from the five (`claimed != derived` at `:408`), so the function reaches
`Self::on_chain(Vec::new(), threshold)` and `is_usable()` is false (`:102`, `keys.len() >= threshold`
fails at 0). The refusal carries a purpose-built message that names the actual trigger — the old
"is NOT the chain-derived bootstrap set" text would have been literally false on this branch and would
have sent an operator down the wrong runbook. Both messages keep the fixed `TRUST_ROOT_CONTAINED`
grep anchor.
Evidence: `rev_f2_bootstrap_five_with_a_contradicting_journal_fails_closed ... ok`, and its shape B
is a journal well-formed on every axis a host-local attacker CAN control (right genesis hash, above
the activation height, `bound_to` chained, heights advancing) and failing only on signatures — so the
result cannot be read as "only obviously-corrupt journals are refused."

**Direction 2 — a rotation that legitimately returns to the bootstrap five is still ACCEPTED.**
`replay_onto` compares membership AS A SET and threshold as a scalar; a remove-then-re-add sequence
lands on exactly the five with `calculate_threshold(5)`, so `claimed == derived` and the accept path
at `:191-205` is taken. Pinned by
`rev_f2_a_journal_that_returns_to_the_bootstrap_membership_replays_clean ... ok`, correctly declared
as a **lock, not a FAIL→PASS**.
I checked the honesty of the stated coverage gap rather than accepting it: `resolve` replays from
`bootstrap_maintainer_keys(network)`, and `constants.rs:108-113` maps Testnet AND Devnet to
`BOOTSTRAP_MAINTAINER_KEYS_TESTNET` — so there is no network whose bootstrap private keys an in-repo
test could hold. The claim is true, the seam test is the right substitute, and the residual is stated
rather than hidden.

**Direction 3 — no previously-refused shape became accepted.** Enumerated, not asserted:

| `is_bootstrap_set` | journal | BEFORE | AFTER | direction |
|---|---|---|---|---|
| true | empty | accept | accept (short-circuit) | unchanged |
| true | non-empty | **accept unconditionally** | accept iff `replay_onto` = `Some` | **narrowed** |
| false | empty | branch → `replay_onto(empty)` replays to the five ≠ keys → refuse | identical | unchanged |
| false | non-empty | branch → replay decides | identical | unchanged |

The branch predicate widened; the accept condition INSIDE the branch is untouched. Acceptance is
therefore a strict subset of what it was. **No shape gained acceptance. The fail-closed direction did
not invert anywhere** — the two empty-`keys` terminals (`:266-301`) are byte-for-byte the M1 posture,
and `is_usable()` is unchanged.

**Availability regressions checked, none found.**
- *Fleet-wide outage?* No. `rev_f2_the_journal_is_what_flips_the_verdict_not_the_keys ... ok` is a
  genuine anti-vacuity control (same keys, same network, same threshold, only the journal differs).
  Every host today holds an ABSENT journal → `MaintainerRotationJournal::new()` → empty → the
  unchanged short-circuit.
- *Could a rotated host be re-seeded and then bricked in a loop?* No. `maintainer_seed_is_done`
  (`periodic.rs:187-193`) is `!members.is_empty() || last_derived_height != 0` above the gate, so the
  seed is one-shot and a rotated set is never overwritten.
- *Crash-window ordering?* Safe. In `apply_block/governance.rs` the state `save` (`:56`, `:95`)
  precedes `record_rotation` (`:58`, `:97`), so the only reachable torn state is
  rotated-set + missing-record, which §12.8 already covers as fail-closed. The mirror (journal ahead
  of state), which F2 would newly turn into a refusal, cannot occur.
- *Same answer on both root-running binaries?* Yes — the AUDIT-P1-012 property holds.
  `bins/cli/src/cmd_upgrade.rs:41-58` loads the journal and passes it into the SAME
  `TrustRoot::resolve`, with the same degrade-to-empty posture as the node's
  `load_rotation_journal_or_empty`. No second copy of the decision was created.

## A.6 [F3] CLOSED — verified, though I was told I need not

Six mechanisms registered, each with `interacts_with` populated (not left null, which is the usual way
this obligation is discharged in name only):

```
PM-173-01 Maintainer change payload bounds (F5)            → ["PM-173-02","PM-173-04"]
PM-173-02 Maintainer rotation journal bounds               → ["PM-173-01","PM-173-03","PM-173-04"]
PM-173-03 Trust-root resolution cache                      → ["PM-173-02","PM-173-04"]
PM-173-04 Quorum-verified rotation replay (AUDIT-P1-002)   → ["PM-173-02","PM-173-03","PM-173-05"]
PM-173-05 Chain-bound maintainer authorization (Option E)  → ["PM-173-04"]
PM-173-06 Maintainer-set digest publication (F6)           → ["PM-173-04"]
```

PM-172-07 carries an appended `[CORRECTED 2026-08-11 by INC-I-173 M3: …]` block. OBS-3 is
`INC-I-174 | high | open`. The gauntlet run is owed at M2 deploy time and is declared, not fabricated
— which is the correct handling under `.claude/protocols/system-impact.md`; a hand-written
`gauntlet_runs` row would be worse than an open run.

One inaccuracy in the amendment's wording → new finding **[F10]**.

## A.7 [F4] [F6] [F7] CLOSED

- **F4** — `crates/mempool/src/pool.rs:740-761`: rewritten, not deleted, and the rewrite keeps the
  part a future reader needs. It states that its old justification cited a function this change set
  deleted, that a 0-in/0-out `Registration` DOES reach the path, and the reachability bound with
  citations (`registration.rs:37-63` genesis branch, `:67-71` post-genesis rejection,
  `network/economics.rs:56-59`). It ends with "Do NOT re-derive this as 'registrations cannot reach
  the system lane'" — the correct shape for a comment whose predecessor was wrong. The delta row is
  added at `specs/state-only-fee-gate-architecture.md:308`.
- **F6** — `bins/node/src/updater/trust_root_wiring.rs:238-244`: the tag exists.
  `match bincode::serialize(state) { Ok(bytes) => (1u8, bytes), Err(_) => (0u8, Vec::new()) }` then
  `hasher.update(&[state_tag])`, mirroring `JournalSource::preimage`'s `0/1/2`. The comment now
  describes the code ("this is a real discriminator, not a length prefix"). I re-checked that the
  QA-iteration-1 cache property survives: `JournalSource::Bytes(bytes)` is hashed and
  `into_journal` decodes **those same bytes** (`:294`), never a second read — no TOCTOU introduced by
  the extra tag byte. `inc_i_173_m3_qa1_trust_root_cache ... ok, 6 passed`.
- **F7** — `RequestWithdrawal` reclassified `none` with the reason at the contract origin
  (`design-contract.md:316-319`), at the QA report (`qa-report.md:448`) and in the spec's new table
  (`:309`). Probe results retained as evidence, correctly separated from the routing claim. The
  pre-existing `spec:267` line was correctly left alone — it is a true statement about the deleted
  function's doc comment, not a routing-delta claim.

## A.8 [F5] CLOSED — the decision is DEFENSIBLE, and I checked the escape hatch actually works

The brief asked me to judge whether document-the-divergence-and-name-the-winner leaves a real trap. It
does not, and the reasoning survives scrutiny:

1. **The un-reachability claim is true.** `--chainspec` is declared once, on the `Run` subcommand
   (`bins/node/src/cli.rs:146-149`). `bins/cli/src/cmd_upgrade.rs` takes only `data_dir` + `network`
   and constructs no `Node`. The offline `maintainer add|remove` command has no data directory at all.
   Threading `params.genesis_hash` into `TrustRoot::resolve` would force each of those sites to
   synthesize the value from the embedded spec — recreating the divergence while ADDING a
   wrong-value failure mode the current code cannot express.
2. **The failure is fail-CLOSED, not fail-open.** On a `--chainspec` devnet, `replay_onto` verifies
   against the embedded hash, `replay_rotation_journal` returns `None`, and the host refuses releases.
   The apply-path half is `warn!`+skip — silent, but the change simply never takes effect.
3. **The detection path is real, and this is the part I verified rather than took on trust.** The
   operator rule tells the signer to compare `getMaintainerSet.genesis_hash` against what
   `print_binding` prints. `crates/rpc/src/methods/governance.rs:99` reads `self.params.genesis_hash`
   — the **named winner**, not the embedded reconstruction — and publishes it on all three branches
   (`:129`, `:147`, `:206`). `print_binding` (`commands/maintainer.rs:56-69`) prints the hash the
   signature was actually built from. So the two values an operator is told to compare are exactly
   the two values that must agree. A documentation-only remedy that pointed at a field derived from
   the WRONG source would have been a trap; this one is not.

Documented at all three sites with the winner named: `params.rs:130-132` ("This site WINS"),
`trust_root.rs:356` (KNOWN DIVERGENCE), `commands/maintainer.rs:18-41` (KNOWN DIVERGENCE + OPERATOR
RULE). Doc comments only; zero behaviour change.

**One option the rejection analysis did not weigh** — see Improvement Suggestion 1. It does not
re-open the finding, because F5's own stated acceptable alternative was "document the limitation next
to `network_genesis_hash`", and that was done and then exceeded.

## A.9 Regression — the seven M1 findings are still closed

Iteration 1 touched exactly two source files (`trust_root.rs`, `trust_root_wiring.rs`); everything
else was comment, spec or workflow text. The blast radius is therefore the guard and the cache, and
both were re-verified above. Confirmed by test evidence, not by the file list:

| M1 finding | Still-closed evidence |
|---|---|
| AUDIT-P1-001 payload bounds | `inc_i_173_m3_payload_bounds ok, 19 passed`; `tx_types.rs` size cap still precedes `from_bytes` |
| AUDIT-P1-002 containment guard | `rotation_journal_guard ok, 14 passed` — includes `audit_p1_002_unrotated_bootstrap_set_with_no_journal_resolves_on_chain ... ok` and all seven original refusal tests; STRENGTHENED, see A.5 |
| AUDIT-P1-003 digest | `maintainer_digest ok, 10` + `maintainer_set_rpc ok, 7` |
| AUDIT-P1-004 Option E ordering | `option_e_apply_path ok, 8` + `option_e_binding ok, 11`; re-read `governance.rs:42,50,82,91` — `bound_to` still read BEFORE the mutation on both arms |
| AUDIT-P2-002 reason malleability | covered inside `option_e_apply_path` |
| AUDIT-P3-002 `is_state_only` deleted | `f4_routing ok, 10` + `f7_cross_list ok, 4` |
| AUDIT-P3-003 unwired contexts | `f4_routing ok, 10`; all six `with_inc_i_173_activation_height` sites still ride the one committed height |
| (bonus) AUDIT-P0-011 | `inc_i_172_m2_release_sign_arg_validation ok, 9 passed` — the inverted test still holds |

`replay_rotation_journal` (`crates/core/src/maintainer/journal.rs:162-224`) is unchanged: the
below-activation-height refusal is still FIRST inside the record loop (`:179-181`), followed by
`bound_to` chaining, strict height monotonicity, signature verification with
`verify_multisig_excluding_at` for removals, and `Err`-on-apply that STOPS rather than skipping
(`:216-220`). Nothing was softened to make F2's widened reach fit.

## A.10 Injection / debt scan on the iteration-1 delta

`crates/updater/src/trust_root.rs` and `bins/node/src/updater/trust_root_wiring.rs` re-scanned. Zero
`Command::new` / `std::process` / `exec` / `eval` / SQL sinks; zero `unsafe`; the only `unwrap` is
inside `#[cfg(test)]`. All interpolation feeds `format!` into `error!`/`debug!` messages and BLAKE3
preimages. No injection vector introduced.

---

## A.11 New findings from this pass

### [F8] the params field doc still frames the testnet gate as future

- **Severity:** MINOR
- **Location:** `crates/core/src/network_params/mod.rs:630-633`
- **Evidence:**
  - `mod.rs:630-633` → "testnet `133_000` (re-pinned 2026-08-10 at live tip 130_291, 2_709 blocks
    ≈ 7.53 h of lead at a measured 10.00 s/block …)". Nothing marks the lead as spent.
  - Measured this session: `bestHeight` 134,480 on three RPC ports. The "7.53 h of lead" is now
    −1,480 blocks.
  - `CLAUDE.md` → "**The pinned values are in `crates/core/src/network_params/` (code is SoT) — read
    them there, never from this file.**" A reader following the project's own instruction lands here.
  - Positive control that the correction reached other sites: the same grep finds the CORRECTED text
    at `validation_checks.rs:320-334` and `inc_i_173_m3_f4_routing.rs:88-89`, so the omission is
    specific to `network_params/`, not a broken search.
- **Confidence:** conf(0.80, measured)
- **Impact:** Bounded but pointed. The text is not a "never-crossed" claim, so F1 is genuinely
  closed; it is an invited false inference at the one location the project designates authoritative
  for activation heights. The developer's stated reason for leaving it — preserving the empty
  `network_params/` diff that proves prohibition 1 — is a real and good reason, which is why this is
  MINOR and not a re-open. `defaults.rs:460-480` is fine as-is: it is explicitly headed "Re-pin
  history", so its figures read as history.
- **Suggested fix:** At M2, when the height is legitimately re-pinned and `network_params/` is being
  edited anyway, replace the parenthetical in `mod.rs` with the new pin plus a one-line status marker
  ("crossed / not crossed as of <date>, tip <N>"). Do NOT edit it before M2 — the empty-diff proof is
  worth more than the comment until then.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — doc-comment text on a struct field, no code path)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (no runtime component)
  Disk:     0 (observed — a few characters in a file already on disk)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave it; the text is a historical record and asserts nothing false literally.
Why this proposal anyway: CLAUDE.md routes every reader of an activation height to exactly this file,
and the sentence invites the same false inference F1 was raised to kill — deferring the edit to M2
costs nothing because M2 must open the file regardless.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F9] the "COMPLETE list" delta table omits the two types the milestone is about

- **Severity:** MINOR
- **Location:** `specs/state-only-fee-gate-architecture.md:296-309`
- **Evidence:**
  - `:296` → the heading claims "the COMPLETE list".
  - The table carries an explicit `none` row for `DelegateBond`/`RevokeDelegation`, justified as
    INERT because `validate_delegate_bond_data` rejects any input or output.
  - `AddMaintainer`/`RemoveMaintainer` have no row, yet they are in BOTH set definitions and their
    non-empty-I/O shape is inert for the identical reason: `validate_maintainer_change_data`
    (`crates/core/src/validation/tx_types.rs:762-773`) returns
    `InvalidMaintainerChange("maintainer change transaction must have no inputs")` and
    `"… must have no outputs"`.
  - So the table answers 7 of the 9 old-lane types and omits precisely the two the milestone exists
    to make mineable.
- **Confidence:** conf(0.85, measured)
- **Impact:** Documentation only, and the omitted answer is "no delta" — so nothing is mis-stated,
  only unstated. It matters because this table is the third revision of a list that was previously
  both over-inclusive (F7) and under-inclusive (F4); a reader who checks whether the two governance
  types changed lane finds a blank in a table that promises completeness and re-derives it, which is
  what the table exists to prevent.
- **Suggested fix:** Add one row: `AddMaintainer`, `RemoveMaintainer` | **none** | in BOTH set
  definitions; the non-empty-I/O shape that would change lane is inert because
  `validate_maintainer_change_data` (`tx_types.rs:762-773`) rejects any input or output — same
  argument as the `DelegateBond` row.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one Markdown table row, no code path)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (no runtime component)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: drop the word "COMPLETE" from the heading instead of adding the row.
Why this proposal anyway: dropping the word removes the promise but not the reader's question, and
the answer is one line that is already derived above in this same review — the row is strictly cheaper
than the next agent re-deriving it, which is the cost F4 and F7 already charged once each.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F10] the PM-172-07 amendment replaces a false claim with an inaccurate one

- **Severity:** MINOR
- **Location:** `.omega/memory.db` → `protection_mechanisms.scale_assumptions` where
  `mechanism_id = 'PM-172-07'`
- **Evidence:**
  - Amendment text (queried this session): "…The cure (domain tags per family) changes SIGNED BYTES,
    is consensus-visible for the governance families, and is M3 with its own activation height.
    **[CORRECTED 2026-08-11 by INC-I-173 M3: the recorded cure text said M3 would carry ITS OWN
    activation height. It does not. M3 rides the EXISTING `inc_i_173_activation_height` pinned in M1;
    no new height was created and no existing height moved.]**"
  - `crates/core/src/maintainer/data.rs:86` →
    `pub fn signing_message(&self, is_add: bool, genesis_hash: &[u8], bound_to: u64) -> Vec<u8>` —
    **no height parameter**.
  - `bins/node/src/node/apply_block/governance.rs:43` and `:83` call it unconditionally; the only
    height-gated call in that handler is `verify_multisig_at(..., activation_height)`, whose gate is
    `maintainer_derivation_activation_height`, not `inc_i_173_activation_height`.
  - So the domain-tag / chain-binding change is **UNGATED**. The correction that iteration-0 F3
    actually asked for was "record that M3 shipped domain separation WITHOUT a height and why (zero
    mined maintainer transactions ⇒ zero outstanding authorizations ⇒ retroactively vacuous)."
  - **Mitigation, and it is substantial:** `PM-173-05.scale_assumptions` records exactly the right
    answer — "Zero outstanding authorizations existed when adopted (these tx types were unmineable),
    so the format change is retroactively vacuous and needs no activation height." A reader who
    follows `interacts_with` reaches it.
- **Confidence:** conf(0.75, observed)
- **Impact:** Low, and self-limiting because PM-173-05 carries the correct rationale. The residual is
  that the sentence sits directly after "The cure (domain tags per family)…", so in context it reads
  as a claim ABOUT the domain tag, and for the domain tag it is false. F3's original complaint was
  that a live registry row contradicting shipped code is worse than no row; the amendment removes the
  larger error and introduces a smaller one of the same species.
- **Suggested fix:** Amend PM-172-07 once more to say the cure shipped with NO activation height, and
  give the reason (retroactive vacuity — zero maintainer transactions have ever been mined, so no
  outstanding authorization exists to invalidate), cross-referencing PM-173-05. Keep the existing
  denial of the "own activation height" claim. Registry text only; no code change.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      N-A (institutional-memory row; no runtime code path)
  Memory:   N-A (no runtime code path)
  IO:       N-A (no runtime code path)
  Network:  N-A (no runtime code path)
  Disk:     +~0.5 KB one-time in `.omega/memory.db` (observed — one appended sentence, sized against
            the existing amendment block on the same row)
  Latency:  0 (observed — `v_protection_surface` is read at agent briefing, never at node runtime)
Inevitability: AVOIDABLE
Cheaper alternative: leave it and rely on PM-173-05, which states the correct rationale.
Why this proposal anyway: the reason F3 was HIGH is that agents brief off this table and a row that
misdescribes shipped code steers the next milestone; PM-173-05 only helps a reader who arrives via
`interacts_with`, and PM-172-07 is the row an agent reaches when asking about the SIGNING-BYTES
family, which is precisely the question the inaccurate sentence answers wrongly.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## A.12 Speculative (carried forward, still not actionable)

**[S1] Blocking filesystem I/O on the async RPC path.** Unchanged by iteration 1 and still
report-only. `TrustRootResolver::resolve` performs synchronous `std::fs::metadata` / `File::open` /
`read_to_end` from a closure invoked by the `getUpdateStatus` handler. Every host is on the
`Ok(None)` path today (one `metadata` syscall). The declared FIFO residual is accepted with a stated
reason. **conf(0.55, inferred)** — no worker-pool measurement was taken.

## A.13 Improvement suggestions (not findings)

1. **A `--genesis-hash` flag on the offline signer would close the half of F5 that is cheapest to
   close.** The rejection analysis correctly shows that four of five consumers cannot reach
   `NetworkParams` — but the offline signer does not need to: an explicit operator-supplied override
   (copied from `getMaintainerSet.genesis_hash`, which M3 already publishes from the winning source)
   requires no chain state, no data directory and no `Node`. That converts the documented OPERATOR
   RULE from "compare two printed values and remember to abort" into "paste the authoritative value
   in". Out of scope for M3; worth carrying into whichever milestone first rehearses a devnet
   rotation.
2. **Add a `Failure-Modes:` block to the M3 commit** covering: journal write fails after a successful
   set mutation; journal present but `maintainer_state.bin` wiped (F2, now a loud refusal); journal
   AND state both deleted (R1, still open); custom chainspec (F5); reorg-out of an applied rotation
   (INC-I-174). §12.8 already contains the material — it needs to reach the commit message, where the
   gate reads it.
3. **The gauntlet is owed.** Workflow run 514 is correctly left open. Whoever closes M3 must run
   `scripts/gauntlet.sh` at M2 deploy time, when node restarts are permitted. Do not close the run
   without it.

## A.14 Final verdict — re-review

**Approved. All seven findings CLOSED. Clear to commit; NOT yet clear to deploy — but for procedural
reasons that are now correctly recorded rather than mis-stated.**

The F2 fix is the one that mattered and it is right, including in the two ways it could most easily
have been wrong: it did not invert the fail-closed posture (acceptance is a provable subset of what it
was), and it did not brick the quorum-reverses-its-own-removal case. The remaining three findings are
documentation and institutional-memory accuracy, none of them blocking, and one of them ([F8]) should
deliberately wait for M2.

Three things must still happen before this reaches testnet, and none of them is a code change:
(a) the deploy is executed as a SYNCHRONIZED stop-all/start-all, (b) M2 re-pins the testnet height
above the then-current tip after re-verifying the tip immediately beforehand, and (c) the gauntlet
runs at M2 deploy time.

**Security Audit Verdict: AUDIT-REQUIRED**

---
---

# PART B — ORIGINAL REVIEW (iteration 0), retained as the record of what was found

> The verdict and findings-block below are SUPERSEDED by PART A. They are preserved unedited because
> the write-ups are the evidence for what each finding was.

Reviewed artifact = `git diff 32e0a650` (37 files, +1701/-194) plus 13 untracked source/test files.

**Original findings block (superseded):**

```
━━━ FINDINGS — 7 total (HIGH:3 MINOR:4) ━━━

  [F1] HIGH conf(0.95, measured) — crates/core/src/network_params/defaults.rs:480 + docs/.workflow/inc-i-173-M3-design-contract.md:333-334 — REV-173-M3-001: the "never-crossed" premise is false; live testnet tip 134,004 > testnet gate 133,000, so the consensus wiring lands ABOVE an already-crossed height
  [F2] HIGH conf(0.85, observed) — crates/updater/src/trust_root.rs:153-202 — REV-173-M3-002: `resolve` short-circuits on the bootstrap-five branch before consulting the journal, so the R1 state-file-wipe re-seed is accepted as usable while a non-empty journal proving the rotation sits unread on disk
  [F3] HIGH conf(0.90, measured) — .omega/memory.db `v_protection_surface` (32 rows) + PM-172-07 — REV-173-M3-003: no M3 protection mechanism is registered, and PM-172-07's recorded text ("the cure ... is M3 with its own activation height") contradicts what M3 shipped
  [F4] MINOR conf(0.85, observed) — crates/mempool/src/pool.rs:740-745 + crates/core/src/transaction/types.rs:184 — REV-173-M3-004: a fourth, undocumented F4 routing delta — `Registration` GAINS the 0-fee system lane — and the in-code comment denying it cites the deleted `is_state_only`
  [F5] MINOR conf(0.80, observed) — crates/core/src/consensus/params.rs:129-141 vs crates/updater/src/trust_root.rs:296-303 — REV-173-M3-005: three genesis-hash sources; `--chainspec` overrides the apply path's hash but not the updater's or the offline signer's, silently un-applying rotations on non-mainnet
  [F6] MINOR conf(0.90, observed) — bins/node/src/updater/trust_root_wiring.rs:216-224 — REV-173-M3-006: the cache-key comment claims a distinct tag for a bincode failure that is not implemented
  [F7] MINOR conf(0.90, observed) — docs/.workflow/inc-i-173-M3-design-contract.md:312-314 — REV-173-M3-007: `RequestWithdrawal` is listed as an F4 routing delta but was never in `is_state_only`, so its routing is unchanged

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Scope Reviewed

| Area | Files |
|---|---|
| F5 payload bounds | `crates/core/src/validation/tx_types.rs`, `crates/core/src/validation/transaction.rs`, `crates/core/src/maintainer/mod.rs` |
| F6 digest | `crates/core/src/maintainer/digest.rs`, `crates/rpc/src/methods/governance.rs`, `bins/node/src/node/apply_block/governance.rs` |
| Option E | `crates/core/src/maintainer/data.rs`, `derivation.rs`, `bins/node/src/node/apply_block/governance.rs`, `bins/node/src/commands/maintainer.rs`, `bins/node/src/cli.rs` |
| AUDIT-P1-002 guard | `crates/core/src/maintainer/journal.rs`, `crates/storage/src/maintainer_journal.rs`, `crates/storage/src/maintainer_wellformed.rs`, `crates/updater/src/trust_root.rs`, `bins/node/src/updater/trust_root_wiring.rs`, `bins/cli/src/cmd_upgrade.rs` |
| F4 routing | `crates/core/src/transaction/core.rs`, `crates/rpc/src/methods/transaction.rs`, `bins/node/src/node/validation_checks.rs`, `crates/mempool/src/pool.rs` |
| F7 | `crates/core/tests/inc_i_173_m3_f7_cross_list.rs` |
| Specs/docs | `specs/state-only-fee-gate-architecture.md`, `specs/maintainer-trust-root-architecture.md`, `specs/engine-parts.md`, `docs/rpc_reference.md`, `docs/cli.md` |

Nothing was skipped. All 13 untracked source/test files and all 37 tracked diffs were read.

## Summary (iteration 0 — superseded)

**⚠️ Approved with observations — NOT clear to deploy as planned.**

The engineering is strong. Every one of the seven M1 findings is closed at the ROOT, not patched at
the symptom, and the two places the prompt flagged as silently-failing are both correct. What blocks
the deploy is not the code: it is that the milestone's central deploy premise — that
`inc_i_173_activation_height` has never been crossed — was true when written and is false now (F1),
which changes the deploy procedure from "rolling restart" to "synchronized stop-all/start-all" on
testnet and invalidates the recorded M2 re-pin plan. F2 and F3 are gaps in reach, not defects in
what was built.

## Build Gate — all green (iteration 0)

| Gate | Result |
|---|---|
| `cargo build --release` | PASS (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0) |
| `cargo fmt --check` | PASS (clean; the pre-commit gate ran it on the same tree) |
| `cargo test --workspace --no-fail-fast` | 120 targets `ok`, **3 failures — all KNOWN, zero new** |

The three failures are exactly the recorded ones and nothing else:

```
test_network::test_onchain_liveness_10k_nodes ... FAILED   (fd exhaustion, environmental)
  panicked at bins/node/tests/test_network.rs:55:37: Too many open files
test_cluster_10x100 ... FAILED                             (fd exhaustion, environmental)
  panicked at bins/node/tests/test_network.rs:55:37
contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity ... FAILED  (pre-existing at base)
```

Recorded, not fixed, per instruction. No new failure hid behind them — the first run fail-fasted at
the first fd failure and was re-run with `--no-fail-fast` specifically so the remaining targets could
not be masked.

## Prohibition Compliance — verified mechanically, not asserted (iteration 0)

| # | Prohibition | Verification |
|---|---|---|
| 1 | No activation-height VALUE changes | `git diff 32e0a650 -- crates/core/src/network_params/` is **EMPTY**. Values still mainnet `u64::MAX`, testnet `133_000`, devnet `0` (`defaults.rs:275,480,631`) |
| 2 | No version-constant bump | `MAINTAINER_STATE_VERSION` still `1` (`crates/storage/src/maintainer.rs:54`); no `Cargo.toml` in the diff; the only `*_VERSION` addition is the NEW `MAINTAINER_ROTATION_JOURNAL_VERSION: u32 = 1` for a NEW file, which the contract explicitly permits |
| 3 | L1/L2 character-identical | `diff <(git show 32e0a650:…transaction.rs \| sed -n '39,88p') <(sed -n '39,88p' …)` → **no output**. CHARACTER-IDENTICAL |
| 4 | Below-gate frozen branch untouched | `git diff 32e0a650 -- crates/core/src/validation/utxo.rs` → **EMPTY**; range 240-250 diff → no output. Frozen branch is still `{Registration, DelegateBond, RevokeDelegation}` at `utxo.rs:242-247` |
| 5 | Apply path stays NON-FATAL | `process_transaction_governance` still returns `Option`, still `warn!`+skip on every rejection arm (`governance.rs:63,104`). Journal write failure is `warn!` only (`governance.rs:243-249`). Pinned by `prohibition_5_the_apply_path_is_non_fatal_on_every_rejection_shape` |
| 6 | No deploy | No `cp`/`codesign`/SSH/restart performed by this review. Local testnet RPC READ only |
| 7 | No push | Working tree still dirty and uncommitted |

## Injection / debt scan (iteration 0)

Instrument verified with a positive control (`grep -c "format!"` returns non-zero for 3 of the 8
files, so the grep family is reaching these paths).

| Pattern | Result |
|---|---|
| `Command::new`, `std::process`, `execute(`, `query(`, `eval(`, `exec(` | **ZERO-VERIFIED** (grep exit 1 with control satisfied) |
| `unsafe` | **ZERO-VERIFIED** (grep exit 1) |
| `.unwrap()` / `.expect(` / `panic!` in new production code | 1 hit, `trust_root_wiring.rs:399`, inside `mod tests` (module starts :386) — test code, acceptable |
| `TODO`/`FIXME`/`HACK`/`XXX` | 1 hit, `tx_types.rs:488`, a doc comment recording that a TODO was RESOLVED — not debt |

No injection vector. All string interpolation in the diff feeds `format!` into error/log messages and
BLAKE3 preimages, never into a shell, SQL or eval sink.

---

## Question 1 — Does it achieve its goal? Per-finding closure

### AUDIT-P1-001 (F5, payload bounds) — **CLOSED AT ROOT**

The size cap precedes `from_bytes` (`tx_types.rs:783-791` before `:794`), so bincode never sees an
attacker-sized buffer — that ordering is the actual root fix, and it is right. The three constants
are principled rather than picked: `MAX_MAINTAINER_CHANGE_SIGNATURES = MAX_MAINTAINERS` because
`count_distinct_signers` (`set.rs:130-149`) can never count entry 6.

Notably better than the contract asked: the contract's own worst-case arithmetic (785 bytes) was
**wrong by 88 bytes in the unsafe direction**, and M3 caught it, replaced the prose with a derived
constant (`MAX_MAINTAINER_CHANGE_ENCODED_BYTES` = 873, `mod.rs:180-186`), and pinned it against the
real encoder with a test. Both dropped terms (bincode's 8-byte length prefix on `PublicKey` and
`Signature`) are correct. This is the difference between closing a finding and closing it at the root.

### AUDIT-P1-002 (containment guard) — **CLOSED for the rotated-set case; see F2 for the reach gap**

The distinction is genuinely cryptographic, exactly as the contract required. Every refusal in the
contract's replay pseudocode is present in `replay_rotation_journal` (`journal.rs:168-225`) in the
specified order. **The `applied_height < maintainer_derivation_activation_height` refusal is intact
and load-bearing** — `journal.rs:179-181`, evaluated FIRST inside the record loop, before the binding
and monotonicity checks. It cannot be short-circuited: there is no early `continue`, no `if let Ok`
that swallows it, and the function returns `None` outright.

Nothing was softened to make the new bounds fit. The FAIL-CLOSED terminal is still
`Self::on_chain(Vec::new(), threshold)` plus `TRUST_ROOT_CONTAINED` (`trust_root.rs:171-186`) —
byte-identical posture to M1, only the *reachability* changed. The empty-journal case still fails
closed, achieved indirectly but soundly: `resolve` only calls `replay_onto` when
`keys != bootstrap five`, and an empty journal replays to exactly the bootstrap five, so the
set-equality check at `trust_root.rs:319-322` refuses.

Amendment 1's seam is the right call and does not weaken the decision. `resolve` remains the sole
decision function; `replay_rotation_journal` is a consulted predicate returning `Option`, structurally
identical to how `resolve` already consults `is_chain_derived_bootstrap_set`. The source-text seam
test prevents the M1 blanket refusal from surviving with the predicate dead.

### AUDIT-P1-003 (F6, digest) — **CLOSED at its stated minimum obligation**

Correct and leaf-preserving (genesis hash passed as `&[u8]`, `digest.rs:54`). Members sorted, so
honest insertion-order divergence below the gate does not produce a false mismatch; `last_updated`
included, so genuine history divergence does. Published on both the on-chain and derived RPC
branches, correctly omitted on the `none` branch (a digest over an absent set would invite a
comparison it cannot support) — that restraint is good judgment.

Scope honesty: this is **observability only**. It enforces nothing. AUDIT-P1-003's minimum obligation
is met; an operator still has to look.

### AUDIT-P1-004 (Option E) — **CLOSED AT ROOT. The order dependency is correct.**

This was the prompt's first silent-failure hotspot. Both arms read before mutating:

- add: `let bound_to = ms.set.last_updated;` at `governance.rs:42`, `add_maintainer` at `:50`
- remove: `let bound_to = ms.set.last_updated;` at `governance.rs:82`, `remove_maintainer` at `:91`

`add_maintainer`/`remove_maintainer` each stamp `self.last_updated = height` (`set.rs:312,336`), so
read-after-mutate would have produced `bound_to = height` and silently rejected every legitimate
authorization while still looking fail-closed.

The test that pins this is a genuine discriminator, not a tautology
(`inc_i_173_m3_option_e_apply_path.rs:231-291`): the seeded root has `last_updated == 0`, the change
applies at height 1, and the assertion is on **member count 5 → 4**. Read-after-mutate makes
`bound_to = 1`, the quorum fails, the count stays 5, the test fails. The suite also carries the
control the contract demanded (`audit_p1_004_control_a_freshly_rebound_authorization_is_applied`),
without which the refusal tests would prove only "everything fails".

The `derive_maintainer_set` replay (`derivation.rs:200-217`) reads `bound_to` before mutating too, so
the replay path and the apply path agree by construction.

### AUDIT-P2-002 (reason malleability) — **CLOSED AT ROOT**

`reason` is inside the signed bytes and LAST (`data.rs:86-96`), and the format is unambiguous by
construction as claimed: literal, `add|remove`, 64-hex, 64-hex, decimal `u64` are all newline-free, so
the first five newlines split deterministically and the remainder is exactly `reason`. Bounded by F5,
so it costs nothing. Verified by `audit_p2_002_reason_tampering_in_flight_refuses_the_change`.

### AUDIT-P0-011 (release-signing family collision) — **CLOSED as a bonus**

Not on the M3 list, but domain separation closed it. The M2 test that asserted the collision was
correctly **INVERTED** rather than deleted, exactly as its own M2 body instructed
(`crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs`). That is the right way to retire
a standing obligation. See F3 for the registry consequence.

### AUDIT-P3-002 (F4, `is_state_only`) — **CLOSED**

Deleted from `crates/core/src/transaction/core.rs`; both production callers route on `is_zero_flow()`.
Grep confirms zero production references remain (all surviving mentions are tests, doc comments and
one stale comment — F4). See Question 3 for the delta analysis.

### AUDIT-P3-003 (unwired contexts) — **CLOSED**

All four sites wired, only `.with_inc_i_173_activation_height(...)` added, and the unrelated
`.with_sig_verification_height` drift at `:103` correctly left alone. See Question 4 and F1.

---

## Question 2 — Did anything go too far, or not far enough?

**Not too far.** The guard redesign does exactly what was asked: it adds a path for a
quorum-verified rotation and weakens nothing else. I looked specifically for a softened refusal and
found none — the below-activation-height refusal, the `bound_to` chaining, the strict height
monotonicity, the `Err`-on-apply refusal (which STOPS rather than skipping, `journal.rs:218-220`, so
an attacker cannot append unappliable records to steer the final set), and the set/threshold equality
check are all present and all terminal.

**Not far enough in two places**, both about reach rather than correctness: F2 (the journal is not
consulted on the branch where it would catch the registered R1 residual) and F3 (the mechanism was
built but never registered, and it contradicts a registry entry that is still live).

The `TrustRootResolver` cache (QA iteration 1) is the prompt's second hotspot, and it is **correct —
it cannot miss a tampered file.** The key is a BLAKE3 over the actual bytes read
(`trust_root_wiring.rs:211-235`), length-prefixed, domain-separated, with the whole `MaintainerState`
folded in and `network` in the preimage. There is no mtime and no size shortcut. Critically there is
no TOCTOU between key and decode: `JournalSource::Bytes(bytes)` is hashed and then
`into_journal` decodes **those same bytes** (`:279`), never a second read. Cache-key input coverage is
complete — `resolve` consumes only `keys`, `threshold`, `last_derived_height` (all inside the
serialized state), `network`, and the journal, and every one is in the preimage. The file is still
read on every call, so the F7(a) property that a mid-flight rotation reaches an in-flight update is
preserved.

---

## Question 3 — Unintended behaviour changes (F4 routing)

Derived independently from the two set definitions rather than from the reports.

- OLD lane test: `tx_type ∈ is_state_only` = {Exit, ClaimReward, ClaimBond, SlashProducer,
  DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer, PriceAttestation} — 9 types, shape
  ignored.
- NEW lane test: `0-in ∧ 0-out ∧ tx_type ∈ allows_empty_io` = {Registration, DelegateBond,
  RevokeDelegation, AddMaintainer, RemoveMaintainer} (`types.rs:184-188`).

| Type | Delta | Documented? |
|---|---|---|
| `Exit`, `SlashProducer` | lose the system lane | YES |
| `ClaimReward`, `ClaimBond` | lose the system lane | YES, and correctly re-stated in QA iteration 1 (OBS-2) as *rejected unconditionally*, not *now pays a fee* |
| `PriceAttestation` | loses the system lane | YES |
| `DelegateBond`, `RevokeDelegation` with non-empty I/O | would lose the lane | **INERT** — `validate_delegate_bond_data` (`tx_types.rs:849-859`) rejects any input or output, so a valid one is always 0-in/0-out and never changes lane. Not a delta. |
| `Registration`, 0-in/0-out | **GAINS the system lane** | **NO — see F4** |
| `RequestWithdrawal` | none | Claimed as a delta; **is not one** — see F7 |

So the documented set is neither complete (misses `Registration`, the only delta that moves toward
*more* free relay) nor exactly accurate (`RequestWithdrawal`). Neither is an operational regression
on a live node: `Registration` is reachable only inside the genesis window, and the QA report did
probe `RequestWithdrawal` behaviourally and recorded its true type-specific rejection.

**Is any documented delta an understated operational regression?** No. The `Exit`/`SlashProducer`
change moves them from silent limbo to a loud mempool rejection, which is strictly better and is the
same defect class as INC-I-173 itself. `ClaimReward`/`ClaimBond` have zero production constructors and
no `apply_block` handler, so they were never mineable — QA measured this rather than inferring it, and
the corrected wording in the implementation report (§ OBS-2) is accurate.

---

## Question 4 — INV-12 / consensus classification, verified independently

Verified against the code, not the reports.

**Consensus-visible parts of this diff:**

1. **F5 payload bounds.** `validate_maintainer_change_data` is reached from `validate_transaction`,
   which block validation and apply both call. Above the gate it REJECTS transactions previously
   accepted. Consensus-visible. Q1 YES (user-submittable) / Q2 YES / Q3 NO → activation height
   REQUIRED, and it has one.
2. **`validate_block_for_apply` wiring** (`validation_checks.rs:319`). Confirmed a CONSENSUS path:
   it feeds `validate_block_with_mode`. Confirmed the wiring is a **divergence FIX** — at base this
   context held `u64::MAX` (frozen 3-type branch) while `apply_block/tx_processing.rs:98` and
   `production/assembly.rs:223` were already wired, so above the gate a producer could BUILD a block
   carrying a maintainer tx that every node's `validate_block_for_apply` would then reject. Above the
   gate the new branch is strictly MORE permissive (`is_zero_flow` adds AddMaintainer/RemoveMaintainer
   to the frozen `{Registration, DelegateBond, RevokeDelegation}`), so post-fix nodes accept blocks
   pre-fix nodes reject.

**Non-consensus:** F6 digest, Option E (justified — the maintainer set is node-local and outside the
state root, the apply handler is non-fatal, and zero maintainer transactions have ever been mined, so
there are no outstanding authorizations to invalidate), the journal and guard redesign (updater only),
the mempool wiring at `pool.rs:382,766` (node-local admission), and `validation_checks.rs:117`
(header-only, gate unreachable).

**Both ride the SAME already-committed height** — confirmed: only `inc_i_173_activation_height` is
referenced, at all six `with_inc_i_173_activation_height` sites.

**NO activation-height value moved and NO version constant was bumped** — proven by the empty
`network_params/` diff and the unchanged `MAINTAINER_STATE_VERSION` (see Prohibition table).

**L1/L2 and the below-gate frozen branch are CHARACTER-IDENTICAL** — proven by range diff, see
Prohibition table.

**But the height is no longer un-crossed.** That is F1, and it is what changes the deploy class.

---

## Question 5 — Do the tests bind the behaviour?

Yes, and the instrument hazards the test plan flagged were handled by running probes, not by reading
code — which is why they were caught.

- **F7 cross-list** correctly implements Amendment 2: L1 is probed with 0-in/**TWO**-out and L2 with
  ONE-in/0-out, using two SEPARATE transactions, because `validate_transaction` short-circuits on L1
  at `transaction.rs:59` and because `is_coinbase()` is shape-defined (`core.rs:122-124`) so a
  one-output 0-input `Transfer` returns `Ok(())`. Both hazards were established by observation. The
  file carries its own anti-vacuity control asserting `Transfer` is in NEITHER list — without it every
  answer could default to "in the list" and the whole test would be inert.
- **Option E ordering** is a real discriminator (analysed above), and the control test exists.
- **Nine migrated INC-I-172 files: migrated, not weakened.** I diffed them. Every change is
  call-site-only — an added `&empty_journal()` argument, or a `genesis_hash`/`bound_to` parameter
  threaded into a fixture that binds CORRECTLY so the property under test still decides the verdict.
  No assertion was relaxed and none was deleted. The single semantic change is the *deliberate
  inversion* of the release-signing collision test, which its own M2 body instructed and which is now
  correct.
- **Bounds tests are non-vacuous**: `req_173_014_maximal_legal_payload_fits_under_the_outer_cap`
  serialises a real maximal payload through the real encoder and asserts it fits — that is the test
  that would have caught the contract's 88-byte arithmetic error, and it did.
- **QA-derived tests measure what they claim**: `qa_block_1_a_maximal_legal_journal_still_loads`
  proves the new byte ceiling does not itself re-create an unmineable-legitimate-payload bug, which
  is the exact failure class of this incident applied to the new bound.

## Question 6 — Specs / docs accuracy

| File | Verdict |
|---|---|
| `specs/state-only-fee-gate-architecture.md` | Accurate, except it repeats the stale tip figure (`:633`, "Live testnet tip at re-pin time `130_291`") — F1 |
| `specs/maintainer-trust-root-architecture.md` | Journal, replay predicate and persistence documented (`:266-274`), including the OBS-13 body-version refusal (`:329`) and a cost line (`:442`). Stale `crates/core/src/maintainer.rs:NNN` pointers remain at `:5,:9,:466,:601` for a file that no longer exists, but `:29-32` explicitly declares that convention and maps it to `maintainer/set.rs` — pre-existing, acknowledged, not M3 drift |
| `specs/engine-parts.md` | Correct. `:476` records the `is_state_only` deletion with its reason; `:1804` carries the full new `signing_message` format and the superseded bearer-token form; `derive_maintainer_set` signature updated with the new `genesis_hash` parameter |
| `docs/rpc_reference.md` | Correct. Documents `maintainer_set_digest` with its exact preimage (`:1065`) and `genesis_hash` on all three branches including `none` (`:1066`), and states the operational rule "compare the digest, not the array" (`:1061`) |
| `docs/cli.md` | Correct. `--bound-to` documented as REQUIRED on both `add` and `remove` (`:1362`), sourced to the `getMaintainerSet` `last_change_block` field (`:1369`), with the two operational traps stated: all signers must use the same value (`:1391`) and the reason must be byte-identical across signers (`cli.rs:384-386`) |

No spec claims a behaviour the code does not have. The only drift is the tip figure, which is F1.

## Question 7 — Modular size budget

**Compliant.** No new source file over 500 lines, no test file over 800.

| New source | Lines | New tests (largest) | Lines |
|---|---|---|---|
| `crates/core/src/maintainer/digest.rs` | 67 | `inc_i_173_m3_option_e_apply_path.rs` | 790 |
| `crates/core/src/maintainer/journal.rs` | 226 | `inc_i_173_m3_payload_bounds.rs` | 736 |
| `crates/storage/src/maintainer_journal.rs` | 353 | `inc_i_173_m3_rotation_journal_store.rs` | 630 |

**Did M3 make the pre-existing debt materially worse? No.** The largest addition to an
already-oversized file is `tx_types.rs` at +56 lines (1071 → 1127, +5%); everything else is +3 to +19.
`crates/core/src/transaction/core.rs` got 22 lines SMALLER. Both files M3 could most easily have
bloated — `trust_root_wiring.rs` (455) and `trust_root.rs` (328) — are still under budget after
+281 and +146 lines respectively, because the bulk went into two new files instead. That is the right
call and OBS-6/OBS-11's decision not to refactor stands.

---

## Findings (iteration 0 write-ups — retained; closure status in PART A)

### [F1] REV-173-M3-001 — the "never-crossed activation height" premise is FALSE — **CLOSED, see A.4**

- **Severity:** HIGH
- **Location:** `crates/core/src/network_params/defaults.rs:480`; premise stated at
  `docs/.workflow/inc-i-173-M3-design-contract.md:333-334`,
  `docs/.workflow/inc-i-173-M3-implementation.md:207,453`,
  `specs/state-only-fee-gate-architecture.md:633`, and in the new code comment at
  `bins/node/src/node/validation_checks.rs:311-318`
- **Evidence:**
  - `cmd:$ curl -s -X POST http://127.0.0.1:8500 -d '{"method":"getChainInfo"}'` →
    `{"bestHeight":134004,"network":"testnet","version":"6.24.1"}`
  - `crates/core/src/network_params/defaults.rs:480` → `inc_i_173_activation_height: 133_000`
  - `docs/.workflow/inc-i-173-M3-design-contract.md:334` → "never-crossed … testnet `133_000`,
    tip ~130_291"
  - Positive control that the instrument discriminates: the same RPC returns
    `maintainer_derivation_activation_height: 127200` from `getMaintainerSet`, matching
    `defaults.rs:450` — the endpoint is live and reporting this chain.
- **Confidence:** conf(0.95, measured)
- **Impact:** Three consequences, in descending severity.
  1. **Deploy class changes.** The `validate_block_for_apply` wiring is consensus-visible above the
     gate, and testnet is now ~1,000 blocks above it. A post-fix node ACCEPTS a block carrying a
     0-in/0-out `AddMaintainer`/`RemoveMaintainer` that a pre-fix node REJECTS. On a rolling restart
     the fleet can split. This is CLAUDE.md deploy question 2 / INC-I-062 / INV-8: testnet needs a
     **synchronized stop-all-then-start-all**, not the rolling restart the "never crossed" wording
     implies. Mainnet is unaffected (`u64::MAX`).
  2. **F5's bounds go live immediately on testnet restart** rather than at a future height. Still
     safe — no maintainer transaction has ever been mined, so the bounds remain retroactively
     vacuous — but the risk profile is "active now", not "dormant".
  3. **The recorded M2 plan is now invalid.** Prohibition 1 keeps testnet at `133_000` because "M2
     re-pins it". Re-pinning it upward now means moving an activation height FORWARD after the chain
     crossed it — the INC-I-054 class that CLAUDE.md names explicitly. M2 needs a different plan
     (a NEW height for anything not yet live), not a re-pin.
- **Suggested fix:** Do not change code. (a) Re-measure the tip and correct the three documents and
  the `validation_checks.rs:311-318` comment to state the height IS crossed on testnet. (b) Re-classify
  the testnet deploy as synchronized. (c) Return the M2 re-pin decision to the Architect — it is a
  design question, not an implementation detail.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — documentation and deploy-procedure change only, no code path added)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed — a synchronized restart moves the same bytes as a rolling one, once)
  Disk:     0 (observed)
  Latency:  +one full-fleet restart window on testnet, ~minutes, one time (inferred from the
            existing stop-all/start-all procedure in scripts/testnet.sh)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the alternative is a rolling restart across an already-crossed consensus
gate, which is a chain split — the cost of the split exceeds the restart window by orders of magnitude.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F2] REV-173-M3-002 — a surviving journal is not consulted on the branch where it would catch the R1 wipe — **CLOSED, see A.5**

- **Severity:** HIGH
- **Location:** `crates/updater/src/trust_root.rs:153-202` (the short-circuit is `:154`, the
  unguarded acceptance is `:189-202`); interacts with the registered residual in `PM-172-02` /
  `PM-172-03` and with `bins/node/src/node/periodic.rs:122-125`
- **Evidence:**
  - `crates/updater/src/trust_root.rs:154` → `if !is_chain_derived_bootstrap_set(&keys, network) {`
    — the journal is consulted ONLY inside this branch; `:189-202` returns
    `Self::on_chain(keys, threshold)` with no journal reference.
  - The code's own doc table states it: `trust_root.rs:117` → "non-empty, == the chain-derived five |
    any | `OnChain`, authoritative; **the journal is NOT read**".
  - `bins/node/src/node/periodic.rs:122-125` → on re-seed,
    `MaintainerSet::with_members(bootstrap_keys.clone(), height)` — a wiped host re-seeds from LIVE
    producer state.
  - `crates/storage/src/maintainer_journal.rs:27` → the journal is a SEPARATE file
    (`maintainer_rotations.bin`), so `rm maintainer_state.bin` leaves it intact.
  - `.omega/memory.db` `v_protection_surface`, `PM-172-03` scale_assumptions → "RESIDUAL, OPEN AND
    MEASURED (INC-I-172 R1) … QA PROBE-1: after_wipe_len=5, removed_key_back=true."
- **Confidence:** conf(0.85, observed)
- **Impact:** A host that legitimately rotated (journal has ≥1 record, set ≠ bootstrap five) and then
  has `maintainer_state.bin` deleted re-seeds to the producer-derived five. On mainnet that set IS the
  compiled bootstrap five (N1–N5), so `is_chain_derived_bootstrap_set` is true, the journal is never
  read, and the trust root resolves USABLE on the PRE-ROTATION membership — re-arming a maintainer key
  governance had removed, for root binary installs. M3 does not make this worse than base (R1 is
  pre-existing and scoped out of M3), but M3 ships the exact evidence needed to detect it and then
  steps past it. The milestone's stated posture is "fail closed for everything that is not a
  quorum-verified rotation"; a re-seeded bootstrap five on a host with a non-empty journal is exactly
  such a thing, and it is accepted.
- **Suggested fix:** In `resolve`, before returning on the bootstrap-five branch, refuse when the
  journal contradicts it — `journal.records` non-empty (optionally also
  `set.last_updated != journal.bootstrap_last_updated`) means this host recorded rotations it no
  longer reflects. Fail closed with the existing `TRUST_ROOT_CONTAINED` message. This is a few lines
  inside the one decision function, needs no new file and no new dependency, and converts R1 from
  silent re-arm to loud refusal on every host that ever rotated. Hosts that never rotated (every host
  today) hold an empty journal and are completely unaffected.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +constant (observed — one `Vec::is_empty()` and one `u64` comparison on the
            bootstrap-five branch of `resolve`; no Ed25519 work is added, the replay is not invoked)
  Memory:   0 (observed — the journal is already loaded and passed in by every caller)
  IO:       0 (observed — the journal file is already read on this path by
            `TrustRootResolver::resolve` / `load_rotation_journal_or_empty` regardless of branch)
  Network:  N-A (local decision, no peer interaction)
  Disk:     0 (observed — read-only check, nothing persisted)
  Latency:  +negligible (inferred — two integer/pointer comparisons on a path that already performs
            a file read and a BLAKE3 hash; unmeasurable against that baseline)
Inevitability: AVOIDABLE
Cheaper alternative: leave R1 as a documented residual and rely on the PM-172-03 `error!` at
`periodic.rs:143-155` to make the re-seed loud.
Why this proposal anyway: the cheaper path is detection-by-log-reading on ~30 external auto-update
hosts nobody tails, against an attacker who chose `rm` precisely because it is quiet; the check
converts the same event into a fail-closed refusal that stops the malicious install itself, at a cost
indistinguishable from zero on a path that already does file I/O.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F3] REV-173-M3-003 — no M3 protection mechanism registered, and PM-172-07 now contradicts the code — **CLOSED, see A.6**

- **Severity:** HIGH (protocol blocker per `.claude/protocols/system-impact.md`, not a code defect)
- **Location:** `.omega/memory.db` → `protection_mechanisms` / `v_protection_surface`; the
  contradicting row is `PM-172-07`
- **Evidence:**
  - `cmd:$ sqlite3 .omega/memory.db "SELECT COUNT(*) FROM protection_mechanisms;"` → `32`
  - `cmd:$ sqlite3 .omega/memory.db "SELECT mechanism_id … FROM v_protection_surface;"` → returns
    PM-001…PM-025 and PM-172-01…PM-172-07. **No M3 mechanism appears.** Positive control: the query
    does return rows, including the PM-172-* family, so the instrument reaches this table.
  - `PM-172-07` scale_assumptions text → "The cure (domain tags per family) changes SIGNED BYTES, is
    consensus-visible for the governance families, and **is M3 with its own activation height**."
  - `crates/core/src/maintainer/data.rs:86-96` → M3 shipped the domain tag
    `DOLI-MAINTAINER-CHANGE-V1` with **no activation height**, justified by retroactive vacuity.
- **Confidence:** conf(0.90, measured)
- **Impact:** Two distinct problems. (a) M3 adds at least four unregistered protection mechanisms —
  the F5 payload caps, the journal-replay guard in `resolve`, the journal well-formedness gate
  (`validate_persisted_journal`, sibling of the registered PM-172-06 but with the OPPOSITE
  fail posture: PM-172-06 is FATAL at startup, the journal loader is deliberately non-fatal), and the
  `TrustRootResolver` cache. Unregistered mechanisms are invisible to the next milestone's
  `v_protection_surface` query, which is the specific way composite failures ship. (b) `PM-172-07`'s
  recorded prediction is now false, and a live registry row that contradicts shipped code is worse
  than no row — the next agent to query the surface will be told an activation height exists.
- **Suggested fix:** Register the four mechanisms with trigger condition, action, scale assumptions
  and interacts-with, and amend `PM-172-07` to record that M3 shipped domain separation WITHOUT a
  height and why (zero mined maintainer transactions ⇒ zero outstanding authorizations ⇒ retroactively
  vacuous). Record the PM-172-06 ↔ journal-loader posture asymmetry explicitly, since it is deliberate
  and a future reader will otherwise read it as a bug. No code change.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      N-A (institutional-memory rows; no runtime code path)
  Memory:   N-A (no runtime code path)
  IO:       N-A (no runtime code path)
  Network:  N-A (no runtime code path)
  Disk:     +~8 KB one-time in `.omega/memory.db` (observed — five INSERT/UPDATE rows of prose,
            sized against the existing PM-172-* rows)
  Latency:  0 (observed — `v_protection_surface` is queried at agent briefing, not at node runtime)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the protocol makes an unanswered protection interaction a blocker precisely
because the alternative — a mechanism nobody can find and a registry row that lies — is how two
individually-correct protections compose into a failure neither has alone.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F4] REV-173-M3-004 — a fourth, undocumented F4 routing delta, plus a stale in-code comment — **CLOSED, see A.7**

- **Severity:** MINOR
- **Location:** `crates/mempool/src/pool.rs:740-745`; delta derives from
  `crates/core/src/transaction/types.rs:184` vs the deleted list at
  `git:32e0a650:crates/core/src/transaction/core.rs:456-476`
- **Evidence:**
  - `crates/core/src/transaction/types.rs:184` → `Self::Registration => true` in `allows_empty_io`.
  - `git show 32e0a650:crates/core/src/transaction/core.rs` (the diff hunk) → the deleted
    `is_state_only` list has 9 arms and `Registration` is NOT among them; its doc comment even said
    "Registration and AddBond are NOT state-only".
  - Therefore a 0-in/0-out `Registration` moves normal-lane → system-lane. Direction is opposite to
    all three documented deltas.
  - `crates/mempool/src/pool.rs:740-745` → "`Registration` is NOT state-only (`Transaction::is_state_only`
    excludes it — it consumes UTXO inputs for its bond), so **no registration reaches this path
    today**" — a live comment whose entire justification is a function this diff DELETED.
  - Reachability bound: `crates/core/src/validation/registration.rs:67-71` → post-genesis,
    `registration must have inputs for bond` rejects it inside `add_system_transaction`'s
    `validate_transaction` call (`pool.rs:774`). The genesis branch (`registration.rs:37-63`) does NOT
    require inputs or outputs, and `is_in_genesis` is `height <= genesis_blocks`
    (`crates/core/src/network/economics.rs:56-59`).
  - Absence control: grepping the implementation report and QA report for a Registration routing
    delta returns nothing, while the same grep for `ClaimReward` returns the documented delta at
    `docs/.workflow/inc-i-173-M3-implementation.md:192,693` — so the blank is a real omission, not a
    broken search.
- **Confidence:** conf(0.85, observed)
- **Impact:** Low and bounded. On mainnet and testnet the genesis window is long past, so the delta is
  unreachable; on a fresh chain a VDF-bearing 0-in/0-out `Registration` would take the 0-fee lane, and
  the VDF cost bounds any amplification. The real problem is the stale comment: it asserts as fact
  something the same commit made false, in the file a future reader will consult to decide whether
  `Registration` needs handling on the system path.
- **Suggested fix:** Correct `pool.rs:740-745` to state that routing is now shape-based and that a
  0-in/0-out `Registration` DOES reach this path, surviving `validate_transaction` only inside the
  genesis window. Add the `Registration` row to the delta table in the implementation report and in
  `specs/state-only-fee-gate-architecture.md`. Comment and documentation only — no behaviour change is
  recommended, because the current behaviour is validated correctly on both branches.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — comment and documentation text only, no code path altered)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (no runtime component)
  Disk:     0 (observed — a few lines in files already on disk)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: delete the stale comment outright instead of rewriting it.
Why this proposal anyway: deletion loses the reachability analysis (genesis-window-only), which is the
part a future reader actually needs in order not to re-derive it; the rewrite costs the same and keeps
the measured bound.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F5] REV-173-M3-005 — three genesis-hash sources, and `--chainspec` moves only one of them — **CLOSED, see A.8**

- **Severity:** MINOR
- **Location:** `crates/core/src/consensus/params.rs:129-141` (`apply_chainspec`) vs
  `crates/updater/src/trust_root.rs:296-303` (`replay_onto`) vs
  `bins/node/src/commands/maintainer.rs:11-21` (`network_genesis_hash`)
- **Evidence:**
  - Apply path signs/verifies against `self.params.genesis_hash`
    (`bins/node/src/node/apply_block/governance.rs:29,43,83`).
  - `crates/core/src/consensus/params.rs:140` → `self.genesis_hash = spec.genesis_hash();` inside
    `apply_chainspec`, which is invoked whenever `--chainspec` is supplied
    (`bins/node/src/node/init.rs:173-174`) and early-returns for Mainnet only
    (`params.rs:130-132`).
  - `crates/updater/src/trust_root.rs:296-300` → `replay_onto` uses
    `ChainSpec::mainnet()/testnet()/devnet()`, the EMBEDDED spec, never `params`.
  - `bins/node/src/commands/maintainer.rs:14-19` → the offline signer likewise uses the embedded spec.
  - `crates/core/src/chainspec.rs:193-200` → `genesis_hash()` folds in `timestamp`, `network`,
    `slot_duration` and `message`, so a custom spec changes it.
  - `--chainspec` is an operator path: `scripts/deploy_producers.sh:484` and six `scripts/test_*.sh`.
  - Negative control confirming mainnet/testnet safety: the local testnet passes no `--chainspec`
    (no such flag in `~/Library/LaunchAgents/network.doli.testnet-n1.plist`, no chainspec file under
    `~/testnet`), so its `params.genesis_hash` equals the embedded one.
- **Confidence:** conf(0.80, observed)
- **Impact:** Mainnet is structurally safe (`apply_chainspec` returns early). On any non-mainnet node
  started with `--chainspec`, the operator's offline signature is bound to the EMBEDDED genesis hash
  while the apply path verifies against the CUSTOM one, so `verify_multisig_at` returns false and the
  rotation is `warn!`+skipped — silently never applied. That is the same silent-limbo class as
  INC-I-173 itself, in a new place. Devnet is where this bites (`inc_i_173_activation_height: 0`,
  `maintainer_derivation_activation_height: 0`, so devnet is the one network where rotations can
  actually be exercised), which also means the capability M3 restores is hardest to rehearse exactly
  where rehearsal is cheapest.
- **Suggested fix:** Make `params.genesis_hash` the single source. The offline signer should accept an
  explicit `--genesis-hash` (or read it from the new `getMaintainerSet` field, which M3 already
  publishes on every branch) instead of deriving it from `--network`; `replay_onto` should take the
  genesis hash as a parameter from the caller that holds `params`, matching the leaf-module idiom the
  rest of this milestone already uses for `activation_height: u64`. If that is out of scope, document
  the limitation next to `network_genesis_hash` so the devnet failure is expected rather than
  discovered.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — passing an existing `[u8; 32]` down a call chain instead of recomputing
            `ChainSpec::…::genesis_hash()`; if anything this REMOVES four BLAKE3 updates per resolve)
  Memory:   0 (observed — a borrowed slice, no new allocation)
  IO:       0 (observed)
  Network:  N-A (offline command and a local update path)
  Disk:     0 (observed)
  Latency:  0 (observed — the removed `ChainSpec` construction is not on any hot path)
Inevitability: AVOIDABLE
Cheaper alternative: document the limitation next to `network_genesis_hash` and leave the three
sources in place.
Why this proposal anyway: the documented-only path leaves a signature that is bound to one chain
identity and verified against another, failing silently through a `warn!` on the exact code path this
incident exists to de-silence; threading one parameter removes the divergence by construction.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F6] REV-173-M3-006 — the cache-key comment describes a mitigation that is not implemented — **CLOSED, see A.7**

- **Severity:** MINOR
- **Location:** `bins/node/src/updater/trust_root_wiring.rs:216-224`
- **Evidence:** The comment reads "An encoding failure is impossible for this type, but if it ever
  happened the empty vector below would pin the key to the journal alone — **so it is tagged
  distinctly instead**." The following statement is
  `let encoded = bincode::serialize(state).unwrap_or_default();` and the hashing is
  `hasher.update(&(encoded.len() as u64).to_le_bytes()); hasher.update(&encoded);` — a length prefix,
  which is not a distinguishing tag. Contrast `JournalSource::preimage()` at
  `trust_root_wiring.rs:266-272`, which DOES carry real discriminating tags (`0`/`1`/`2`) — so the
  file demonstrates the idiom it claims to have used here and did not.
- **Confidence:** conf(0.90, observed)
- **Impact:** Practically nil — `MaintainerState` is a `u32` plus a `MaintainerSet` plus a `u64` with
  no map or non-serialisable field, so bincode cannot fail, and a genuinely empty encoding is
  unreachable. The defect is the comment: it tells a future reader a safety property holds when it
  does not, on the one function whose whole job is to decide when a security verdict may be reused.
- **Suggested fix:** Either implement the tag (a leading `0u8`/`1u8` byte discriminating
  serialise-ok from serialise-failed, mirroring `JournalSource::preimage`), or correct the comment to
  say the length prefix plus the structural impossibility of an empty encoding is the argument. Both
  are one line; the tag is preferable because it survives a future field being added to
  `MaintainerState`.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +constant (observed — one additional 1-byte `Hasher::update` per resolution if the tag
            is implemented; zero if only the comment is corrected)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (local cache-key computation)
  Disk:     0 (observed)
  Latency:  0 (observed — one byte into a BLAKE3 that already absorbs up to 911 KB)
Inevitability: AVOIDABLE
Cheaper alternative: correct the comment only, adding no code.
Why this proposal anyway: the comment-only fix is genuinely acceptable here and is stated as the
alternative; the tag is preferred solely because it keeps the property true if `MaintainerState` later
gains a field that can fail to encode, which is exactly the drift the surrounding comment already
argues for guarding against.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F7] REV-173-M3-007 — `RequestWithdrawal` is documented as an F4 delta but is not one — **CLOSED, see A.7**

- **Severity:** MINOR
- **Location:** `docs/.workflow/inc-i-173-M3-design-contract.md:312-314`, repeated at
  `docs/qa/inc-i-173-M3-qa-report.md:444`
- **Evidence:** The contract states "`Exit` / `SlashProducer` / `RequestWithdrawal` with 0 inputs
  move from 'admitted at fee 0, gossiped, then silently never mined' to 'rejected at the mempool'."
  But the deleted `is_state_only` list (diff hunk on `crates/core/src/transaction/core.rs:456-476`)
  contains Exit, ClaimReward, ClaimBond, SlashProducer, DelegateBond, RevokeDelegation, AddMaintainer,
  RemoveMaintainer, PriceAttestation — **`RequestWithdrawal` is absent**. It was therefore already
  routed to the normal lane at base, and its routing is unchanged by M3.
- **Confidence:** conf(0.90, observed)
- **Impact:** Documentation only, and self-limiting: QA probed the type behaviourally and recorded its
  true rejection (`docs/qa/inc-i-173-M3-qa-report.md:439` →
  `Err(Validation(InvalidWithdrawalRequest("withdrawal request must have Bond UTXO inputs")))`), so no
  behaviour is mis-stated — only the attribution of that rejection to an M3 delta. It matters because
  the completeness of this delta list is the entire deliverable of contract Item 5, and a list that is
  simultaneously over-inclusive here and under-inclusive at F4 should not be trusted as-is by the next
  reader.
- **Suggested fix:** Remove `RequestWithdrawal` from the delta list (keep the QA probe result as
  evidence that it is correctly rejected either way) and add `Registration` per F4.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — documentation text only)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (no runtime component)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave the list as-is and rely on the QA probe output further down the report.
Why this proposal anyway: the probe output is 200 lines from the delta table and states a rejection
reason, not a routing verdict, so a reader who consults only the table — which is what the table is
for — carries away a false claim about which types M3 changed.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Speculative Findings (low-confidence, not actionable)

### [S1] Blocking filesystem I/O on the async RPC path

`TrustRootResolver::resolve` performs synchronous `std::fs::metadata`, `File::open` and
`read_to_end` (`crates/storage/src/maintainer_journal.rs:153-185`) from a closure invoked by the
`getUpdateStatus` RPC handler (`bins/node/src/node/startup.rs:479-489`). On a Tokio worker this is
blocking I/O, and it composes with the declared FIFO residual
(`maintainer_journal.rs:138-147`): the residual's own text says "one planted FIFO would park an RPC
worker permanently", which is a stronger statement on an async runtime than on a dedicated thread.
Today every host is on the `Ok(None)` path (one `metadata` syscall, no open), so there is nothing to
measure, and the residual is explicitly declared and accepted with a stated reason (`libc` is not a
non-dev dependency of `crates/storage`). Reported so the interaction is on record, not as an action.
**conf(0.55, inferred)** — I did not measure worker-pool behaviour under a planted FIFO.

---

## Improvement Suggestions (iteration 0)

1. **Add a `Failure-Modes:` block to the M3 commit** covering at minimum: journal write fails after a
   successful set mutation (→ fail closed, correct, and worth stating); journal present but
   `maintainer_state.bin` wiped (→ F2); custom chainspec (→ F5). The system-impact protocol requires
   the block to answer the recorded modes semantically, and F2/F5 are two of them.
2. **Consider a `maintainer_set_digest` mismatch alert** rather than leaving F6 purely pull-based.
   The digest is the right primitive; nothing currently consumes it automatically.
3. **The `getUpdateStatus` and update-service resolvers are two independent caches** for the same
   inputs (`startup.rs:478` and `trust_root_wiring.rs:358`). Correct, but they will report different
   `recomputations()` counters — worth a note before that counter is treated as an operator signal.

## Modules Not Reviewed

None. The full change set was reviewed in iteration 0; iteration 1 re-reviewed the remediation plus
regression, as scoped.

## Final Verdict (iteration 0 — SUPERSEDED by A.14)

**Requires iteration before deploy — but the iteration is documentation, registry and a deploy
decision, not a rewrite.**

The code itself I would approve: all seven M1 findings are closed at the root, both silent-failure
hotspots are correct, every hard prohibition is mechanically verified, the build gate is green, and
the only test failures are the three known ones. F2 is the one place I would ask for code, and it is
a few lines inside a function that is already correct.

What must not ship on the current plan is the deploy. F1 invalidates the premise that made a rolling
restart safe on testnet, and F3 leaves the milestone's new protections invisible to the next
milestone's briefing while a live registry row contradicts what shipped. Resolve F1 (re-measure,
re-classify the deploy as synchronized, return the M2 re-pin to the Architect), F3 (register), and
ideally F2, then this is ready.

**Security Audit Verdict: AUDIT-REQUIRED**
