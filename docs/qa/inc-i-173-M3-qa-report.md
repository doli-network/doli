# QA Report: INC-I-173 M3 — Hardening before deploy

Incident **INC-I-173**, milestone **M3**, run **511**. Workflow `redesign`.
Branch `bugfix/inc-i-173-state-only-fee-gate`, base `32e0a650`, working tree DIRTY/UNCOMMITTED.
Authority: `docs/.workflow/inc-i-173-M3-design-contract.md` (+ both adopted amendments).

**Pass 1** 2026-08-10 — verdict FAIL (QA-BLOCK-1).
**Pass 2 (QA iteration 1 re-validation)** 2026-08-11 — verdict below. See
[§ QA iteration 1 re-validation](#qa-iteration-1-re-validation-2026-08-11).

---

## Final Verdict

**PASS** — all six contract items CLOSED, all seven hard prohibitions held, and the one blocking
defect from pass 1 (**QA-BLOCK-1**) is **CLOSED with independent evidence**. Nineteen from-scratch
adversarial probes — nine against the loader's bounds, ten against the new resolution cache —
reproduce every property the developer claims and find no way to defeat either. The byte ceiling is
genuinely DERIVED (the real bincode encoder emits exactly 890 / 911,380 / 911,388 bytes for the
maximal legal record / body / file), it is applied before `fs::read`, and the second `Read::take`
arm is demonstrably load-bearing — a FIFO defeats the metadata arm alone and is caught by `take`.
The resolution cache is provably CONTENT-keyed: a same-length content swap with the mtime restored
by `touch -r` still re-resolves, so a `(len, mtime)` stale-serve is not reachable. Fail-closed is
unchanged: an over-bound journal reaches byte-identical provenance / usability / key-count to a
rotated set with no journal, and an UNROTATED host (the entire fleet today) is unaffected.

Four non-blocking observations are added (OBS-9 … OBS-12); none of them is introduced by this fix
and none of them changes the trust decision. Approved for review.

---

## Per-Item Verdict Table

| # | Item / Finding | Verdict | Basis |
|---|---|---|---|
| 1 | **AUDIT-P1-001 / F5** — bound the maintainer payload | **CLOSED** | Size cap precedes `from_bytes`; maximal legal payload measured at 873 B and ACCEPTED; below-gate branch byte-identical |
| 2 | **AUDIT-P1-003 / F6** — publish a chain-derived digest | **CLOSED** | Digest sensitive to all four terms, insensitive to order; every pre-existing RPC field preserved. ~~`docs/rpc_reference.md` NOT updated~~ → documented in pass 2 (OBS-1 closed) |
| 3 | **AUDIT-P1-004 + AUDIT-P2-002 / Option E** — non-replayable authorizations | **CLOSED** | `bound_to` read BEFORE mutation at both arms; apply path still returns `Option`, never `Err` |
| 4 | **AUDIT-P1-002** — rotation journal + guard redesign | **CLOSED** | Legitimate rotation verifies; every tampering shape refused; below-AH refusal present AND load-bearing. ~~Resource bound NOT met~~ → **resource bound now met and independently re-probed in pass 2; QA-BLOCK-1 CLOSED** |
| 5 | **AUDIT-P3-002 / F4** — routing on `is_zero_flow()` | **CLOSED** | No live regression; oracle freeze VERIFIED not assumed. ~~delta understated~~ → `ClaimReward`/`ClaimBond` now stated as unconditional rejection in all three places (OBS-2 closed) |
| 6 | **AUDIT-P3-003** — the four `ValidationContext` sites | **CLOSED** | All 6 non-test sites wired from one source → `validate_block_for_apply` now AGREES with `apply_block`. No height VALUE moved |
| 7 | **F7** — cross-list total test | **CLOSED** | Behavioural probe, all 24 variants, anti-vacuity fires, L1/L2 character-identical |

---

## System Entrypoint

```
cargo build --release                                   -> Finished in 1m 53s, EXIT 0
cargo fmt --check                                       -> clean, EXIT 0
cargo clippy --workspace --all-targets -- -D warnings   -> clean, EXIT 0
```

Live testnet reached **READ-ONLY** over JSON-RPC on `127.0.0.1:8500/8501/8502` (`getMaintainerSet`).
No node was started, stopped, restarted or deployed. No SSH. `~/testnet/bin/doli-node` mtime is
`Aug 10 13:47`, unchanged by this session.

---

## Build Gate and Test Suite

### M3 targets — 94 passed, 0 failed *(pass 1, 2026-08-10; superseded by the pass-2 table below)*

| Target | Result |
|---|---|
| `doli-core inc_i_173_m3_payload_bounds` | **19 passed** |
| `doli-core inc_i_173_m3_maintainer_digest` | 10 passed |
| `doli-core inc_i_173_m3_option_e_binding` | 11 passed |
| `doli-core inc_i_173_m3_f7_cross_list` | 4 passed |
| `rpc inc_i_173_m3_maintainer_set_rpc` | 7 passed |
| `storage inc_i_173_m3_rotation_journal_store` | 8 passed |
| `updater inc_i_173_m3_rotation_journal_guard` | 11 passed |
| `updater inc_i_173_m3_rotation_replay` | 6 passed |
| `doli-node inc_i_173_m3_option_e_apply_path` | 10 passed |
| `doli-node inc_i_173_m3_f4_routing` | 8 passed |
| `updater trust_root_fail_closed` (pre-existing M1) | 13 passed |

> ⚠ **CONTRADICTION, named and resolved.** `docs/.workflow/inc-i-173-M3-implementation.md` §5.1
> declares `audit_p1_001_signature_flood_is_rejected_above_the_gate` **BLOCKED** by a fixture
> overflow (18/19). I measured **19/19**. The test plan §6 (written after the implementation report)
> records the fixture repair `sig_entry((i % 255) as u8 + 1)`, which is present in the tree. The
> implementation report is STALE on this point; the tree is correct. No defect.

### Full workspace — `cargo test --workspace --no-fail-fast`

3 failing TARGETS, **2 distinct tests, both declared pre-existing, both individually re-identified
by name and root cause. No new failure.**

| Failing target | Failing test | Classification |
|---|---|---|
| `-p doli-node --test test_network` | `test_network::test_cluster_10x100` | KNOWN environmental — `OPTIONS-000006.dbtmp: Too many open files` at cluster 8/10, node 69. fd exhaustion, not logic |
| `-p doli-node --test checkpoint_rotation` | `test_network::test_cluster_10x100` (same test — the target carries `mod test_network`) | Same instance, second target. This is the "counted twice" the developer described; it is not a third failure |
| `-p mempool --lib` | `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` | KNOWN pre-existing at base. The failing gate reads `inc_i_096_activation_height`, already wired before M3 |

### Pass 2 (2026-08-11) — re-run after the QA-BLOCK-1 fix

Build gate, run by QA on the developer's tree with QA's own probe files REMOVED first:

```
cargo build --release                                 -> EXIT 0
cargo clippy --workspace --all-targets -- -D warnings -> EXIT 0, no warnings
cargo fmt --check                                     -> EXIT 0
```

M3 targets — **103 passed, 0 failed** (was 94; +3 store, +6 new cache tests):

| Target | Pass 1 | Pass 2 |
|---|---:|---:|
| `doli-core inc_i_173_m3_payload_bounds` | 19 | **19** |
| `doli-core inc_i_173_m3_maintainer_digest` | 10 | **10** |
| `doli-core inc_i_173_m3_option_e_binding` | 11 | **11** |
| `doli-core inc_i_173_m3_f7_cross_list` | 4 | **4** |
| `rpc inc_i_173_m3_maintainer_set_rpc` | 7 | **7** |
| `storage inc_i_173_m3_rotation_journal_store` | 8 | **11** |
| `updater inc_i_173_m3_rotation_journal_guard` | 11 | **11** |
| `updater inc_i_173_m3_rotation_replay` | 6 | **6** |
| `doli-node inc_i_173_m3_option_e_apply_path` | 10 | **8** ¹ |
| `doli-node inc_i_173_m3_f4_routing` | 8 | **10** ¹ |
| `doli-node inc_i_173_m3_qa1_trust_root_cache` | — | **6** (new) |
| **M3 total** | 94 | **103** |
| `updater trust_root_fail_closed` (pre-existing M1) | 13 | **13** |

¹ ⚠ **CONTRADICTION, named and resolved.** Pass 1 recorded these two rows the other way round
(10 / 8). Counted directly in the tree: `inc_i_173_m3_option_e_apply_path.rs` declares
**8 `#[tokio::test]`** and 0 `#[test]`; `inc_i_173_m3_f4_routing.rs` declares **10 `#[test]`** and
0 async tests. The pass-2 values are correct and the pass-1 row order was a transcription error in
QA's own report, not a change in the tree. The pair total (18) and the M3 total are unaffected.

Full workspace, `cargo test --workspace --no-fail-fast`: **155 targets ok, 3 FAILED;
passed=3542 failed=3 ignored=43** (pass 1: 3532 / 4 instances). The 3 failing instances are the
**same 2 distinct KNOWN pre-existing tests**, each re-identified by its own panic message:

- `test_cluster_10x100` (in both `--test test_network` and `--test checkpoint_rotation`) —
  `Node 69 init failed: … OPTIONS-000006.dbtmp: Too many open files` (fd exhaustion, environmental);
- `mempool contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity` —
  `crates/mempool/src/contention_tests.rs:1108`.

**No new failure. Neither known failure masks one.** The pass-1 fourth instance
(`audit_p1_001_signature_flood_is_rejected_above_the_gate`) is gone: it now passes as part of
`payload_bounds` 19/19.

---

## Item 1 — AUDIT-P1-001 / F5: payload bounds — **CLOSED**

### Is the payload bounded above the gate, before `from_bytes`?

**YES.** `crates/core/src/validation/tx_types.rs`: the size cap is at `:783-792`, the
`MaintainerChangeData::from_bytes` call is at `:795`. bincode never sees an over-cap buffer.
Bounds 2 (signature count) and 3 (reason BYTE length) follow the decode, inside a single
`ctx.current_height >= ctx.inc_i_173_activation_height` block.

### Is the maximal LEGAL payload accepted? (the "does the cap re-create the bug" test)

Measured independently, not taken from the developer's report:

```
PROBE O1 maximal legal payload = 873 bytes (cap 1024)
PROBE O2 maximal legal payload ABOVE gate -> Ok(())
PROBE O2 maximal legal payload BELOW gate -> Ok(())
PROBE O2 emoji reason: 64 chars / 256 bytes, payload 873 bytes -> Ok(())
```

A legitimate **5-of-5 with a full 256-byte reason is ACCEPTED**. The cap does not re-create the
INC-I-173 class. The multi-byte case confirms the reason cap counts BYTES, not `char`s.

> **Non-blocking accuracy defect.** The design contract (Item 1) and
> `crates/core/src/maintainer/mod.rs` both state the maximal legal payload as
> *"32 target + 8 len + 5x96 sigs + 1 + 8 + 256 reason = 785"*. The MEASURED value is **873 bytes**.
> Real headroom under the 1024 cap is **151 bytes, not 239** — 15%, not 23%. The arithmetic is
> wrong by 88 bytes in the unsafe direction. The consistency test
> `req_173_014_maximal_legal_payload_fits_under_the_outer_cap` guards the real value, so this is a
> comment/spec error, not a code defect. It matters if anyone later adds a field to
> `MaintainerChangeData` and reasons from the written figure.

### Is below-gate behaviour genuinely unchanged (retroactive vacuity)?

**YES.**
- `git diff 32e0a650 -- crates/core/src/validation/utxo.rs` is **EMPTY** (the frozen branch).
- Every new check is inside `ctx.current_height >= ctx.inc_i_173_activation_height`. Below the gate
  control flow is the four historical checks then `from_bytes`, in their original order.
- Measured: the same hostile buffer is refused identically above and below the gate, with the
  DECODER's message below and the SIZE cap's message above — so the instrument discriminates.
- Five below-gate inertness partitions pass in `inc_i_173_m3_payload_bounds`.

### Adversarial probe — hostile length prefix inside a legal-size buffer

```
PROBE O3 hostile 1024-byte buffer, declared Vec len = u64::MAX, ABOVE gate
  -> Err(InvalidMaintainerChange("invalid maintainer change data format")) in 36.459µs
PROBE O3 same buffer BELOW gate -> Err(...) in 12.708µs
```

A 1024-byte buffer that passes the size cap and declares `Vec<MaintainerSignature>` length
`u64::MAX` does **not** cause an unbounded allocation — serde's cautious sizing refuses. The
defence is layered, not solely the cap. PASS.

---

## Item 2 — AUDIT-P1-003 / F6: maintainer-set digest — **CLOSED**

> *Pass 2: the documentation gap noted below is closed — see OBS-1 disposition.*

### Can an operator actually detect trust-root divergence from what is published?

**YES — and the live fleet proves the problem is real right now.** READ-ONLY `getMaintainerSet`
against three running local-testnet nodes:

| Node | `maintainers[]` order |
|---|---|
| `127.0.0.1:8500` | `5432…`, `effe…`, `2d27…`, `2020…`, `3047…` |
| `127.0.0.1:8501` | `5432…`, `effe…`, `2d27…`, `2020…`, `3047…` |
| **`127.0.0.1:8502`** | **`2d27…`, `3047…`, `2020…`, `5432…`, `effe…`** |

All three hold the SAME five keys, `source: "on-chain"`, `threshold: 3`, `last_change_block: 1`.
Node 8502 returns them in a **different array order**. An operator diffing the raw member lists
today sees a false mismatch. This is exactly the AUDIT-P3-014 insertion-order nondeterminism the
sorted digest is designed to neutralise — measured live, not argued. F6 closes a genuine operator
problem.

(These nodes run the pre-M3 binary, so they do not yet serve the new fields. No node was restarted.)

### Does the digest genuinely differ per term?

Measured one term at a time:

```
PROBE O4 base            = f99d3e792f6793d20344e0c9258a46fc1d77007b2de48501d9d5b985d364eeea
PROBE O4 member ORDER     differs? false  (must be NO)   <- order-insensitive, as designed
PROBE O4 MEMBER changed   differs? true   (must be YES)
PROBE O4 THRESHOLD 3->4   differs? true   (must be YES)
PROBE O4 LAST_UPDATED +1  differs? true   (must be YES)
PROBE O4 GENESIS changed  differs? true   (must be YES)
```

All four required sensitivities hold; order-independence holds. PASS.

### Does the response still carry every field it carried before?

**YES.** The live pre-M3 response carries exactly:
`maintainers[], threshold, member_count, max_maintainers, min_maintainers,
initial_maintainer_count, last_change_block, source, enforced,
maintainer_derivation_activation_height`.

All ten are present verbatim in the new on-chain branch
(`crates/rpc/src/methods/governance.rs:115-130`), plus the two additions. The consumer at
`bins/cli/src/cmd_governance.rs:466` reads `maintainers[].pubkey` and `threshold` — both intact.
The `derived` branch likewise preserves its full field set plus `advisory_note`. The `none` branch
correctly publishes `genesis_hash` and NO digest.

Apply-path grep anchor is present and fixed:
`[MAINTAINER] MAINTAINER_SET_DIGEST=<64 hex> members=… threshold=… last_updated=… height=…`
(`bins/node/src/node/apply_block/governance.rs`, `log_maintainer_set_digest`).

**GAP — see OBS-1.** `docs/rpc_reference.md` was not updated. `git diff 32e0a650 -- docs/` is EMPTY.

---

## Item 3 — AUDIT-P1-004 + AUDIT-P2-002 / Option E — **CLOSED**

### The ORDER dependency (the property that fails silently if reversed)

**HOLDS.** `bins/node/src/node/apply_block/governance.rs`, both arms:

```rust
let bound_to = ms.set.last_updated;                              // :38 (add) / :81 (remove)
let message  = data.signing_message(true, genesis_hash.as_bytes(), bound_to);
if ms.set.verify_multisig_at(...) { ... ms.set.add_maintainer(...) }
```

`bound_to` is read before the verify and before the mutation, on both arms. The guarding test
`audit_p1_004_apply_path_reads_bound_to_before_mutating_the_set` is genuinely **behavioural and
discriminating**, not a source-text scan: it seeds a set at `last_updated = 0`, signs bound to `0`,
applies at **height 1**, and asserts the removal **WAS applied** (`members.len() == 4`). If the
handler read `bound_to` after the stamp (or used `height`), the message would differ, verification
would fail, and the assertion would fire. The two values are deliberately distinct (0 vs 1), so the
test cannot pass by coincidence.

### Is the apply path still NON-FATAL on every rejection path?

**YES.** `process_transaction_governance` still returns `Option<(u32, u64)>`
(`apply_block/governance.rs:17-22`). Every new failure path is `warn!` + continue:
- `record_rotation` returns `()`; a journal LOAD failure `warn!`s and returns; a journal SAVE
  failure `warn!`s and falls through.
- No `?`, no `Err`, no panic added anywhere on the handler path.
- `prohibition_5_the_apply_path_is_non_fatal_on_every_rejection_shape` passes across 13 rejection
  shapes and then applies a legitimate change, so "non-fatal" is asserted as *skipped, not
  poisoned*. No fork risk. PASS.

### AUDIT-P2-002 (reason malleability)

`reason` is the last field of the signed message (`crates/core/src/maintainer/data.rs`), so a
relayer cannot rewrite it under a genuine quorum signature. `None` and `Some("")` render
identically (`as_deref().unwrap_or_default()`), so the offline signer's `--reason`-absent path and
the `Add` arm's `reason: None` cannot disagree. CLOSED.

### Offline signer

`doli-node maintainer add|remove` now take a **required** `--bound-to <u64>`, derive the genesis
hash from `--network`, and print a `SIGNED BINDING` block naming network, full genesis hash and
`bound_to`. Verified by inspection (declared unasserted by the test plan §4.4). See OBS-4 —
this is a breaking CLI change with no operator documentation.

---

## Item 4 — AUDIT-P1-002 / rotation journal — **CLOSED**

> *Pass 2: the resource bound is now met. QA-BLOCK-1 is CLOSED — see the re-validation section.*

### Does verification now SUCCEED for a legitimate quorum-verified rotation?

**YES.** `audit_p1_002_a_quorum_verified_rotation_replays_clean` and
`control_replay_applies_a_genuine_three_distinct_signer_rotation` pass. `TrustRoot::resolve`
consults `replay_onto` → `replay_rotation_journal`; the seam test
`audit_p1_002_resolve_delegates_to_the_replay_and_keeps_one_decision` pins the wiring so the M1
blanket refusal cannot survive with the predicate unused. The capability the M1 guard denied is
restored. Amendment 1's bootstrap injection is sound: `resolve` remains the sole decision.

### Does it STILL REFUSE for every tampering shape? Was fail-closed weakened anywhere?

**No weakening found.** Refusals verified present in `crates/core/src/maintainer/journal.rs:137-192`
and exercised end-to-end through `resolve` (11 guard tests + 6 replay tests, all pass):

| Shape | Refusal site |
|---|---|
| rotated set, EMPTY/absent journal | `replay_onto` set comparison → `None` |
| signatures that do not verify | `journal.rs:178` |
| `bound_to` that does not chain | `journal.rs:151` |
| `applied_height` that does not advance | `journal.rs:154` |
| record the set cannot apply | `journal.rs:187` |
| unknown journal version | `journal.rs:137` + loader `maintainer_journal.rs:85` |
| replay lands on different members than `maintainer_state.bin` | `trust_root.rs` `replay_onto` |
| threshold mismatch | `trust_root.rs` `replay_onto` |

Fail-closed is always `Self::on_chain(Vec::new(), threshold)` + `TRUST_ROOT_CONTAINED`. It never
falls back to the compiled bootstrap keys — confirmed by reading the branch and by the 13 passing
`trust_root_fail_closed` tests. The two branches every host in the fleet is on today
(empty set, unrotated bootstrap set) are driven with a GARBAGE journal and are unaffected.

### Is the `applied_height < maintainer_derivation_activation_height` refusal present and load-bearing?

**PRESENT** at `crates/core/src/maintainer/journal.rs:148`, and **LOAD-BEARING**, verified by
constructing the counterfactual: `inc_i_173_m3_rotation_replay.rs` case **IP-R7** signs a record
with a **genuine 3-of-5 quorum from the injected bootstrap set**, correctly bound
(`bound_to = 0`, chaining, advancing), at `applied_height = ah - 1`. Every other refusal passes for
that record. Delete the height check and it replays clean — the legacy entry-counting path clears
threshold 3 with three valid distinct entries. So IP-R7 flips only on this check. Genuinely
discriminating. (The guard-file counterpart at `:323-348` uses non-bootstrap signers and would
refuse anyway; the replay file carries the real discriminator.)

### Adversarial: can a duplicated member list forge a quorum through the guard?

**Probed and DISPROVEN — no defect.** Hypothesis: `replay_onto` compares `BTreeSet<String>`, which
collapses duplicates, so `maintainer_state.bin` could hold `[A,A,A,B,C,D]` while the replay lands on
`{A,B,C,D}`. `verify_release_with_trust_root` (`crates/updater/src/verification.rs:97`) loops over
`root.keys()`, so a duplicated slot would let ONE compromised key count three times and clear a
3-of-N. **Blocked upstream:** INC-I-172 M2 / AUDIT-P1-019 added
`crates/storage/src/maintainer_wellformed.rs::validate_persisted_set`, which refuses duplicate
members, over-`MAX_MAINTAINERS` lists and unreconciled thresholds before `MaintainerState::load`
returns. The tampered file never reaches `resolve`. PASS.

### QA-BLOCK-1 — the journal is unbounded in BYTES and in SIGNATURES per record

Contract Item 4: *"Cap records on load (`MAX_ROTATION_RECORDS = 1024`) so a hostile file cannot
force unbounded work."* The record cap is implemented. **The purpose it states is not achieved.**

`MaintainerRotationRecord.change.signatures` is a `Vec<MaintainerSignature>` with **no cap anywhere
on the journal path**. The F5 cap (5 signatures) guards the TRANSACTION validator; the journal is a
local file that never crosses it. Measured:

```
PROBE O5 sigs=   1000  journal=    112086 bytes  replay -> false  in   32.756ms
PROBE O5 sigs=  20000  journal=   2240086 bytes  replay -> false  in  593.778ms
PROBE O5 sigs= 200000  journal=  22400086 bytes  replay -> false  in    5.839s
```

~29 µs per entry: every entry bears a real member pubkey with a garbage signature, so
`count_distinct_signers` must run `verify()` on all of them (it only `break`s on verify SUCCESS).

The loader accepts it — the record cap does not fire, because there is only ONE record:

```
PROBE J1 records=1 (cap 1024), file=22400094 bytes, load -> Ok(1 records) in 552.422ms
PROBE J1 extrapolated worst case at the record cap: 1024 records x 22400094 bytes
         = 22.9 GB read into memory before the cap is evaluated
```

`load_rotation_journal` (`crates/storage/src/maintainer_journal.rs:67-119`) does
`std::fs::read(&path)` → magic → version → `bincode::deserialize(whole body)` → **then**
`records.len() > MAX_ROTATION_RECORDS`. Both the read and the decode are unbounded before the cap.
Worst case at the record cap: ~22.9 GB resident and ~100 minutes of Ed25519 verification.

Reachability and amplification:
- Threat actor is exactly the one the journal is designed against: a host-local attacker with
  data-dir write access.
- `resolve_trust_root` re-reads the journal on **every** call, deliberately and uncached
  (`maintainer_trust_root_fn`, `trust_root_wiring.rs`).
- `bins/node/src/node/startup.rs` wires the same call into the `getUpdateStatus` reporter, and
  `getUpdateStatus` is a dispatchable public RPC method
  (`crates/rpc/src/methods/dispatch.rs:33`). Once the file is planted, **any remote caller can
  trigger the full replay repeatedly** on a consensus node.

The asymmetry is the strongest evidence this is an oversight rather than a decision. The journal's
own module doc says it "mirrors `crates/storage/src/maintainer.rs` deliberately". That sibling file
has `validate_persisted_set`, whose comment states the exact principle:

> *"The size bound is checked FIRST so it also bounds the cost of everything below it. The duplicate
> scan is O(n²), the file is unauthenticated, and `bincode::deserialize` puts no ceiling on a vector
> length, so a hand-written member list of 100_000 keys would otherwise cost 10^10 comparisons at
> STARTUP."*

`maintainer_rotations.bin` has **no equivalent validator**
(`grep validate_persisted crates/storage/src/maintainer_journal.rs` → none). M3 re-creates in a new
file the exact class its own Item 1 exists to close.

**This does not weaken the trust decision.** A hostile journal cannot make a forged rotation
verify — it can only deny the node. The security property of AUDIT-P1-002 is CLOSED. What is open
is availability, and the surface is new in M3.

---

## Item 5 — AUDIT-P3-002 / F4 routing — **CLOSED**

> *Pass 2: the understated delta is corrected in all three documents — see OBS-2 disposition.*

`Transaction::is_state_only()` is deleted (`crates/core/src/transaction/core.rs`). Both production
callers route on `is_zero_flow()` (`crates/rpc/src/methods/transaction.rs:203`,
`bins/node/src/node/validation_checks.rs:930`).

### Measured behavioural deltas (empirical, via `Mempool::add_transaction`)

```
PROBE ClaimReward     is_zero_flow=false
PROBE ClaimReward     -> Err(InvalidTransaction("[MPTX008] insufficient funds: input=0 < output=500000000 (deficit=500000000)"))
PROBE ClaimBond       is_zero_flow=false
PROBE ClaimBond       -> Err(InvalidTransaction("[MPTX008] insufficient funds: input=0 < output=100000000000 (deficit=100000000000)"))
PROBE Exit                -> Err(Validation(InvalidTransaction("[ERRTX043] missing exit data in extra_data")))
PROBE SlashProducer       -> Err(Validation(InvalidSlash("missing slash data")))
PROBE RequestWithdrawal   -> Err(Validation(InvalidWithdrawalRequest("withdrawal request must have Bond UTXO inputs")))
PROBE AddMaintainer   is_zero_flow=true
PROBE AddMaintainer   add_system_transaction -> Ok        <- governance admission INTACT
```

**`Exit` / `SlashProducer` / `RequestWithdrawal`** — loud, type-specific mempool rejection, exactly
as documented. Improvement over silent limbo. **Not an operational regression.**

> **CORRECTED 2026-08-11 (M3 review iteration 1, REV-173-M3-007 / F7).** The probe RESULTS above are
> unchanged and remain valid. What was wrong is the grouping: **`RequestWithdrawal` is not an M3
> routing delta.** The deleted `is_state_only` list did not contain it (its nine arms were `Exit`,
> `ClaimReward`, `ClaimBond`, `SlashProducer`, `DelegateBond`, `RevokeDelegation`, `AddMaintainer`,
> `RemoveMaintainer`, `PriceAttestation`), so it was already on the normal lane at base and its
> routing is identical before and after. Only `Exit` and `SlashProducer` moved. The fourth actual
> delta, absent from this list, is **`Registration` 0-in/0-out, which GAINS the system lane** —
> reachable only inside the genesis window (REV-173-M3-004 / F4).

**`ClaimReward` / `ClaimBond` — the documented delta UNDERSTATES the change.** The contract and the
implementation report both say they *"stop being system-routed at `fee_rate=0`"*, which reads as
"they now pay a fee". They do not: both types are structurally required to have **0 inputs and
exactly 1 positive output** (`validate_claim_data` / `validate_claim_bond_data`,
`crates/core/src/validation/tx_types.rs:52-140`), so `total_input (0) < total_output` always holds
and the normal lane rejects them **unconditionally**.

**No operational regression, because both types are dead:**
- `Transaction::new_claim_reward` / `new_claim_bond` have **zero non-test callers**
  (repo-wide `grep`, `/target/` excluded).
- No CLI command constructs either; no RPC method constructs either.
- **No `apply_block` handler exists for either** (`grep 'ClaimReward\|ClaimBond'
  bins/node/src/node/apply_block/` → no matches), so they were never mineable in the first place.

Verdict: CLOSED. Recorded as **OBS-2** so whoever revives these types is not misled.

**`PriceAttestation` loses system routing — VERIFIED to have no live impact, not assumed.**
`oracle_activation_height = u64::MAX` on **all three** networks —
`crates/core/src/network_params/defaults.rs:195` (mainnet), `:416` (testnet), `:611` (devnet).

---

## Item 6 — AUDIT-P3-003 / `ValidationContext` wiring — **CLOSED**

All **six** non-test `ValidationContext::new` sites in the tree now carry
`.with_inc_i_173_activation_height(...)`:

| Site | Wired at | Class |
|---|---|---|
| `bins/node/src/node/validation_checks.rs:103` (`check_producer_eligibility`) | `:117` | header-only, gate unreachable |
| `bins/node/src/node/validation_checks.rs:289` (`validate_block_for_apply`) | `:319` | **CONSENSUS PATH** |
| `bins/node/src/node/apply_block/tx_processing.rs:61` | `:98` | pre-existing |
| `bins/node/src/node/production/assembly.rs:186` | `:223` | pre-existing |
| `crates/mempool/src/pool.rs:363` (`add_transaction`) | `:382` | node-local policy |
| `crates/mempool/src/pool.rs:747` (`add_system_transaction`) | `:766` | node-local policy |

**Does wiring `validate_block_for_apply` create AGREEMENT or a new disagreement?** **Agreement.**
Both `validate_block_for_apply` (`:319`) and `apply_block/tx_processing.rs` (`:98`) now read the
same expression, `self.config.network.params().inc_i_173_activation_height` — one source, one
value, no possibility of divergence. Before M3 the former held the `u64::MAX` default while the
latter was wired, so above the gate one path rejected a block the other accepted. This is a
divergence FIX.

**No activation-height VALUE moved.** `git diff 32e0a650 -- crates/core/src/network_params/` is
**EMPTY**. Pinned values re-read from code: `inc_i_173_activation_height` mainnet `u64::MAX`
(`:275`), testnet `133_000` (`:480`), devnet `0` (`:631`);
`maintainer_derivation_activation_height` mainnet `172_000` (`:264`), testnet `127_200` (`:450`),
devnet `0` (`:628`). Unchanged.

The unrelated `.with_sig_verification_height` drift at `:103` was correctly NOT repaired; the scope
guard test still passes.

---

## Item 7 — F7 cross-list total — **CLOSED**

**Behavioural, not a copy.** `membership()` in `crates/core/tests/inc_i_173_m3_f7_cross_list.rs`
calls `validate_transaction` and reads the `[ERRTX001]` / `[ERRTX002]` anchors. The L1/L2
expressions are never reproduced in the test.

**All 24 variants.** Driven over `ALL_TX_TYPES`, declared `[TxType; 24]` in
`crates/core/tests/inc_i_173_common/mod.rs:65`, and pinned by
`req_173_011_total_table_is_self_consistent` asserting `rows.len() == 24`.

**Amendment 2 honoured.** L1 and L2 are probed with **separate** transactions (L1: 0-in/**2**-out;
L2: 1-in/0-out), so the L1 short-circuit at `transaction.rs:59` cannot make the L2 half vacuous, and
the two-output shape avoids the shape-based `is_coinbase()` exclusion. Anti-vacuity test
`req_173_011_the_shape_probe_actually_fires` confirms both chains really fire, and both are also
driven above and below the gate to prove the probe reads the STRUCTURAL chains and not the fee gate.

**L1/L2 are CHARACTER-IDENTICAL to base.** `git diff 32e0a650 -- crates/core/src/validation/transaction.rs`
contains exactly ONE hunk — the two `validate_maintainer_change_data(tx, ctx)` call sites at
`:171,174`. Lines `39-88` are untouched.

---

## Hard Prohibitions — Compliance (independently verified)

| # | Prohibition | Status | Evidence I ran |
|---|---|---|---|
| 1 | Never change an activation-height VALUE | **HONOURED** | `git diff 32e0a650 -- crates/core/src/network_params/` EMPTY; values re-read from `defaults.rs` |
| 2 | Never bump a version constant | **HONOURED** | Diff scan for `VERSION =` finds only `MaintainerRotationJournal.version` (NEW constant, NEW file). `git diff --stat -- '*Cargo.toml'` EMPTY |
| 3 | Never edit L1/L2 | **HONOURED** | Only hunk in `transaction.rs` is `:171,174` |
| 4 | Never edit the below-gate frozen branch | **HONOURED** | `git diff 32e0a650 -- crates/core/src/validation/utxo.rs` EMPTY |
| 5 | Apply path stays NON-FATAL | **HONOURED** | Return type still `Option<(u32,u64)>`; all new failure paths `warn!`+continue |
| 6 | No deploy | **HONOURED** | `~/testnet/bin/doli-node` mtime `Aug 10 13:47` (pre-session). No `cp`, no `codesign`, no SSH, no restart. RPC READS only |
| 7 | No commit / no push | **HONOURED** | `git log --oneline -1` = `32e0a650`. Tree dirty |

---

## Module-Size Budget

**New source files — all well under 500. PASS.**

| File | Lines |
|---|---:|
| `crates/core/src/maintainer/digest.rs` | 67 |
| `crates/core/src/maintainer/journal.rs` | 195 |
| `crates/storage/src/maintainer_journal.rs` | 171 |

**New test files — all under 800. PASS**, but the largest is tight at **790/800**
(`bins/node/tests/inc_i_173_m3_option_e_apply_path.rs`). One more test case will breach it.

**Pre-existing violations — M3 made all three WORSE (did not create them):**

| File | base `32e0a650` | now | delta |
|---|---:|---:|---:|
| `crates/core/src/validation/tx_types.rs` | 1071 | 1127 | **+56** |
| `bins/node/src/node/validation_checks.rs` | 1276 | 1295 | **+19** |
| `crates/mempool/src/pool.rs` | 1695 | 1705 | **+10** |

Assessment: **+85 lines added to files already 2-3x over budget.** §5.2 of the implementation
report declares this honestly and the alternative (splitting frozen consensus validators) is
correctly out of M3 scope. Non-blocking, but the debt grew. `crates/updater/src/trust_root.rs`
(254→328), `apply_block/governance.rs` (176→270) and `bins/node/src/commands/maintainer.rs`
(329→371) all stay under budget.

---

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Hostile journal: 1 record, 200,000 signature entries, all bearing a real member pubkey with garbage sigs | Bounded by `MAX_ROTATION_RECORDS` | 22.4 MB file loads `Ok` in 552 ms; replay burns **5.84 s** of Ed25519. Cap never fires — it counts RECORDS | **high** |
| 2 | Extrapolate #1 to the record cap | Bounded work | 1024 x 22.4 MB = **22.9 GB read into memory** before the cap is evaluated; ~100 min CPU | **high** |
| 3 | `getUpdateStatus` reachability of the replay | Local-only | Public dispatchable RPC (`dispatch.rs:33`) re-reads and re-replays the journal on every call → remote amplification of #1 | **high** |
| 4 | Maximal legal payload: 5 sigs + 256-byte reason, above the gate | Accepted | `Ok(())` at **873 bytes** — but the contract/code comment says 785 | **medium** (doc) |
| 5 | 1024-byte payload declaring `Vec` len = `u64::MAX` | Possible unbounded alloc | Rejected in 36 µs, no allocation (serde cautious sizing) | none — PASS |
| 6 | 256-byte emoji reason (64 chars) above the gate | Accepted if BYTES counted | `Ok(())` — byte counting confirmed | none — PASS |
| 7 | Duplicated members in `maintainer_state.bin` to collapse the `BTreeSet` comparison in `replay_onto` and forge a quorum in `verify_release_with_trust_root` | Possible fail-open | **Blocked upstream** by `validate_persisted_set` (INC-I-172 M2 / AUDIT-P1-019) | none — PASS |
| 8 | `ClaimReward` / `ClaimBond` through the new normal lane | Fee-paying admission | **Unconditional rejection**, `[MPTX008] insufficient funds` | **medium** (doc; no live impact) |
| 9 | `AddMaintainer` 0-in/0-out through the system lane | Admitted at fee 0 | `Ok` — governance admission intact | none — PASS |
| 10 | Cross-node `getMaintainerSet` member-array comparison on the live testnet | Identical arrays | 8500/8501 agree; **8502 returns a different order** for the same five keys | none — this is the problem F6 solves |
| 11 | Reorg interaction: is the journal (or `maintainer_state.bin`) rolled back? | Rolled back with the block | Neither is — `bins/node/src/node/rollback.rs` has no maintainer handling | **low** (pre-existing, see OBS-3) |

---

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Rotated set, journal ABSENT | Yes | Yes | N/A | **Yes** | `replay_onto` set comparison fails → `TRUST_ROOT_CONTAINED`, empty `OnChain`. Never falls back to compiled keys |
| Rotated set, journal signatures forged | Yes | Yes | N/A | Yes | `journal.rs:178` |
| Rotated set, journal `bound_to` broken | Yes | Yes | N/A | Yes | `journal.rs:151` |
| Rotation applied below the distinct-signer height | Yes | Yes | N/A | Yes | `journal.rs:148`, discriminating test IP-R7 |
| Journal UNREADABLE / corrupt | Yes | Yes | N/A | **Yes** | `load_rotation_journal_or_empty` → `error!` + empty → rotated host fails CLOSED, unrotated host unaffected. Correctly non-fatal (INC-I-153 class avoided) |
| Journal HOSTILE-LARGE | **Yes** | **No** | **No** | **NO** | **QA-BLOCK-1.** Loads `Ok`, burns CPU/RAM, no bound |
| Journal save fails mid-apply | By inspection | Yes | N/A | Yes | `warn!` only; block never fails (Prohibition 5) |
| Maintainer state write fails mid-apply | By inspection | Yes | N/A | Yes | Pre-existing `warn!` path, unchanged |
| Oversized maintainer tx above the gate | Yes | Yes | N/A | Yes | Size cap names the bound before the decoder |
| Reorg after an applied rotation | **Not triggered** | — | — | Unknown | Untestable without driving a live reorg; see OBS-3 |

---

## Security Validation

| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Unbounded maintainer payload decode | 458 KB and 4096-signature floods; 1024 B buffer declaring `Vec` len `u64::MAX` | **PASS** | Size cap fires first and names the bound; no unbounded alloc |
| Reason-byte confusion (chars vs bytes) | 64-emoji / 256-byte reason at the cap | **PASS** | Bytes counted |
| Governance authorization replay (state) | Stale `bound_to` after a bump | **PASS** | Refused; control proves fresh binding is accepted |
| Governance authorization replay (chain) | Authorization signed against another genesis | **PASS** | Refused |
| Reason malleability rider (AUDIT-P2-002) | Mutate only `reason` post-signature | **PASS** | Signature invalidated — `reason` is inside the message |
| Release/governance message collision (AUDIT-P0-011) | Inverted M2 test asserting NO collision | **PASS** | Domain separation holds; anti-vacuity control proves the release sig is still valid over OLD bytes |
| Host-local trust-root forgery | 8 tampering shapes through `resolve` | **PASS** | All fail closed to an unusable `OnChain` root |
| Duplicate-member quorum collapse via the trust root | `[A,A,A,B,C,D]` state file vs `BTreeSet` comparison | **PASS** | Blocked upstream by `validate_persisted_set` |
| **Journal resource exhaustion** | 200k-signature record; extrapolation to the record cap; RPC reachability | **FAIL** | **QA-BLOCK-1** |
| Journal file permissions | `audit_p2_014_saved_journal_is_owner_only` | **PASS** | 0600 via the shared `create_owner_only`; atomic tmp→fsync→rename |
| Journal magic aliasing | Own `DMRJ` magic, distinct from `DMST` | **PASS** | A misplaced file cannot decode as the wrong type |

---

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `docs/rpc_reference.md` §`getMaintainerSet` (response block ~L1031-1049) | Response has 10 fields; no `maintainer_set_digest`, no `genesis_hash` | Both are now returned on the on-chain and derived branches; `genesis_hash` also on `none` | **medium** — this is the doc an operator uses to act on AUDIT-P1-003 |
| `docs/cli.md` §18.1 "Which keys authorise the install" | *"It resolves that root from `<data-dir>/maintainer_state.bin`"* | It now ALSO reads `<data-dir>/maintainer_rotations.bin`; a rotated set with no valid journal fails closed; `doli upgrade` prints a new journal WARNING | **medium** |
| `docs/troubleshooting.md` | No entry | New operator signals exist: `MAINTAINER_SET_DIGEST=` grep anchor, the reworded `TRUST_ROOT_CONTAINED`, and the new `maintainer_rotations.bin` file in the data dir | **low** |
| `docs/cli.md` §9 (`doli-node` commands) | `doli-node maintainer add\|remove` not documented at all | Now takes a **required** `--bound-to`; any existing runbook invocation fails with a clap error | **low** (pre-existing gap, newly breaking) |
| `crates/core/src/maintainer/mod.rs` comment + contract Item 1 | Maximal legal payload *"= 785"* | Measured **873** | **low** |
| `docs/.workflow/inc-i-173-M3-implementation.md` §5.1 / §6 / §7 | `audit_p1_001_signature_flood_...` BLOCKED, 18/19, "93 passed 1 blocked" | 19/19; the fixture was repaired (test plan §6). M3 total is **94** | **low** (stale artifact) |
| `crates/core/tests/inc_i_173_m3_f7_cross_list.rs` header, IP-S1 line | *"shape = 0 inputs, ONE output"* | Code and the surrounding prose use **TWO** outputs (load-bearing for `is_coinbase()`) | **low** (internal inconsistency) |

`git diff 32e0a650 -- docs/` is **EMPTY** — no operator-facing document was touched. The three
spec files the contract required (`specs/state-only-fee-gate-architecture.md`,
`specs/engine-parts.md`, `specs/maintainer-trust-root-architecture.md`) ARE updated and are of high
quality. The contract's Definition of Done listed only specs, so the developer complied with the
contract; the gap is in the contract.

---

## Blocking Issues

**None open.** The single blocking defect raised in pass 1 is CLOSED. The original write-up is kept
verbatim below as the record of what was found, with the closure evidence appended.

### QA-BLOCK-1 — `maintainer_rotations.bin` is unbounded in bytes and in signatures per record — **CLOSED 2026-08-11**

> **STATUS: CLOSED.** Re-probed independently in pass 2 with 19 from-scratch adversarial tests.
> Both required properties hold, neither weakens any fail-closed path, and the fleet's UNROTATED
> hosts are not bricked by the new bound. Evidence:
> [§ QA iteration 1 re-validation](#qa-iteration-1-re-validation-2026-08-11).
> The text below is the pass-1 finding, unchanged.

**Location:** `crates/storage/src/maintainer_journal.rs:67-119` (`load_rotation_journal`) and
`crates/core/src/maintainer/journal.rs:131-195` (`replay_rotation_journal`).

**Expected** (design contract, Item 4): *"Cap records on load (`MAX_ROTATION_RECORDS = 1024`) so a
hostile file cannot force unbounded work."*

**Actual:** the cap counts RECORDS only. `MaintainerRotationRecord.change.signatures` has no bound
anywhere on the journal path, and `load_rotation_journal` performs `std::fs::read` of the whole file
plus a full `bincode::deserialize` **before** the cap is evaluated. Measured: a single-record
22.4 MB journal loads `Ok` in 552 ms and replays in 5.84 s. At the record cap that extrapolates to
~22.9 GB resident and ~100 minutes of Ed25519 verification, on a path that
`bins/node/src/node/startup.rs` exposes to the public `getUpdateStatus` RPC and that
`maintainer_trust_root_fn` deliberately re-reads uncached on every update tick.

**Why blocking:** M3 is a hardening milestone whose Item 1 exists to close exactly this pattern
(bound the input before the decoder). It introduces the pattern in a NEW file, against exactly the
actor the journal is designed to defeat (host-local, data-dir write), and makes it remotely
amplifiable. The sibling file the module says it mirrors — `maintainer_state.bin` — already has
`validate_persisted_set`, whose own comment states the principle being violated here. The journal
has no equivalent validator.

**Not affected:** the AUDIT-P1-002 trust decision. A hostile journal cannot make a forged rotation
verify. The exposure is availability only.

**Suggested shape** (developer's call, not a QA prescription): a `validate_persisted_journal`
sibling to `validate_persisted_set`, applying (a) a file-SIZE ceiling checked before `fs::read`,
and (b) `signatures.len() <= MAX_MAINTAINER_CHANGE_SIGNATURES` per record, refused at load.
`MAX_MAINTAINER_CHANGE_SIGNATURES` already exists and is the principled bound — a real applied
rotation can never carry more.

---

## Non-Blocking Observations

### Pass-2 disposition (2026-08-11) — verified in the tree, not taken on the report's word

| OBS | Claimed | QA verification | Status |
|---|---|---|---|
| **OBS-1** | rpc_reference documents the digest | `docs/rpc_reference.md:1049-1086`: `maintainer_set_digest` + `genesis_hash` in the response block, a per-branch table, the digest preimage, the explicit "compare the DIGEST, not the `maintainers` array" instruction with QA's ordering measurement as the reason, and the `MAINTAINER_SET_DIGEST=` log anchor | **CLOSED** |
| **OBS-2** | stated as unconditional rejection in 3 places | design contract L303-311, impl report L190-200, `specs/state-only-fee-gate-architecture.md` L277-291 — all three now carry a dated CORRECTED block naming `[MPTX008] insufficient funds` and "no fee makes them admissible" | **CLOSED** |
| **OBS-3** | deferred, user registers an incident | Recorded in `specs/maintainer-trust-root-architecture.md:208,871` (F-6 rollback parity, "unimplemented"), `specs/state-only-fee-gate-architecture.md:342-343`, and impl report §10.2. **Not silently dropped.** However `.omega/memory.db` has **no `incidents` row** for it (latest is INC-I-173) — the registration the report assigns to the user has not happened yet | **DEFERRED, recorded** — see OBS-12 |
| **OBS-4** | cli.md §9.3 is new | `docs/cli.md:1343-1391` documents `maintainer list\|verify\|sign\|add\|remove`, states `--bound-to` is REQUIRED and BREAKING, points at `getMaintainerSet.last_change_block`, warns that all signers must share `--bound-to` and a byte-identical `--reason`. §18.1 (L1991-2021) covers the journal, the fail-closed consequence, the back-up instruction and the load bounds | **CLOSED** |
| **OBS-5** | fixed at the source, 873 derived | `crates/core/src/maintainer/mod.rs:159-186`: prose replaced by derived constants. QA re-derived independently AND against the real encoder — see [property (a)](#property-a--bounded-decode-re-probed) | **CLOSED** |
| **OBS-6** | recorded, not refactored | impl report §5.2 carries post-iteration counts. QA re-measured every one: all accurate (`maintainer/mod.rs` 186, `journal.rs` 226, `maintainer_journal.rs` 262, `maintainer_wellformed.rs` 200, `trust_root_wiring.rs` 455, `tx_types.rs` 1127, `validation_checks.rs` 1295, `pool.rs` 1705; tests 790 / 619 / 459). No new file over 500, no new test file over 800 | **DEFERRED, recorded** — see OBS-11 |
| **OBS-7** | §5.1/§6/§7 refreshed | §5.1 headed `~~BLOCKED~~ — RESOLVED` with a dated correction; §7 carries a REFRESHED pointer to §10.4. QA re-measured `payload_bounds` = 19/19 | **CLOSED** |
| **OBS-8** | header says TWO outputs | `crates/core/tests/inc_i_173_m3_f7_cross_list.rs:97` reads "0 inputs, TWO outputs"; L150 agrees | **CLOSED** |

### New in pass 2

- **OBS-9** *(medium)* — **A FIFO in place of `maintainer_rotations.bin` blocks the calling thread
  indefinitely.** `File::open` on a FIFO with no writer never returns (QA probe P9: no return after
  8 s). It is reachable from node startup, from the update tick, and — since M3 — from the public
  `getUpdateStatus` RPC, where each call would park one RPC worker permanently. **Not introduced by
  the QA-BLOCK-1 fix**: the pre-fix `std::fs::read` opens the same way, and `MaintainerState::load`
  has had the same property since INC-I-172. Same threat actor (host-local, data-dir write), who
  already holds the node's availability. The byte ceiling is NOT defeated by it — with a writer
  present the `Read::take` arm caught the stream at exactly `ceiling + 1` and refused (probe P8),
  which is direct evidence that the second arm is load-bearing. Cheap hardening if wanted:
  `metadata.file_type().is_file()` before the open, or `O_NONBLOCK` via `custom_flags`.
- **OBS-10** *(low)* — **The comment at `crates/storage/src/maintainer_journal.rs:225-227` is
  false.** It states *"the live apply path can never produce an over-cap journal"*.
  `record_rotation` (`bins/node/src/node/apply_block/governance.rs:203`) appends without bound and
  there is no compaction, so the **1025th applied rotation writes a journal the loader will then
  refuse permanently**, fail-closing that host's update channel with no automatic way back. There is
  also no operator warning as the count approaches the cap (`grep MAX_ROTATION_RECORDS` finds no
  near-cap check). 1024 governance rotations is not a near-term number — at one per month it is
  85 years — so this is low, but the stated assumption is wrong and would mislead a future
  maintainer. Pre-existing in M3; the QA-BLOCK-1 fix did not create it.
- **OBS-11** *(low)* — `bins/node/src/updater/trust_root_wiring.rs` grew to **455/500** in this
  iteration. Still inside budget, but it is now the tightest non-test source file in the change set;
  the next addition to it will breach.
- **OBS-12** *(low, process)* — OBS-3 is recorded in two specs and in the implementation report but
  has **no incident row** in `.omega/memory.db`. The impl report assigns the registration to the
  user; until that happens the reorg/rollback gap in the maintainer trust root exists only as prose.
- **OBS-13** *(low)* — `load_rotation_journal` returns `Ok` for a body whose **inner `version`
  field** is unknown; only the 4-byte header tag is checked at load. QA probe P3 planted an
  all-zero body exactly at the ceiling and the loader returned
  `Ok(MaintainerRotationJournal { version: 0, records: [] })`. This is **fail-CLOSED**, not
  fail-open: `replay_rotation_journal` refuses `version != 1` as its first act (probe P3 confirmed
  `None`), so a rotated host still refuses every release. Recorded because the loader and the
  replay disagree about which layer owns the version check.

### Pass-1 observations (original text, retained)

- **OBS-1** — `docs/rpc_reference.md`: `getMaintainerSet` response block does not list
  `maintainer_set_digest` or `genesis_hash`. AUDIT-P1-003's obligation is that an operator can
  DETECT divergence; an undocumented field is one they will not compare. Also add a line telling
  operators to compare the digest rather than the member array, with the live 8502-vs-8500 ordering
  difference as the reason.
- **OBS-2** — Design contract Item 5, implementation report Item 5 and
  `specs/state-only-fee-gate-architecture.md` L275 all describe the `ClaimReward`/`ClaimBond` delta
  as *"stop being system-routed"*. Measured behaviour is **unconditional mempool rejection**
  (`[MPTX008] insufficient funds`), because both types are structurally 0-in/1-positive-out. No
  live impact (zero production constructors, no apply handler), but state it plainly.
- **OBS-3** — `bins/node/src/node/rollback.rs` handles no maintainer state. A rotation that is
  reorged out leaves both `maintainer_state.bin` and the journal in place and mutually consistent,
  so the replay still succeeds and a reorged-out rotation stays install-authoritative. Pre-existing
  (INC-I-172), now extended to a second file. Worth an incident of its own; out of M3 scope.
- **OBS-4** — `--bound-to` is a REQUIRED argument on `doli-node maintainer add|remove`. This is a
  breaking CLI change with no entry in `docs/cli.md`. Any operator runbook invoking these commands
  will fail with a clap error and no pointer to `getMaintainerSet`'s `last_change_block`.
- **OBS-5** — Maximal-legal-payload arithmetic is wrong by 88 bytes in
  `crates/core/src/maintainer/mod.rs` and the contract (785 stated, 873 measured). Real headroom is
  151 B, not 239 B. The guarding test uses the real value, so this is comment-only.
- **OBS-6** — Module-size debt grew: +85 lines across three files already 2-3x over the 500-line
  budget. `bins/node/tests/inc_i_173_m3_option_e_apply_path.rs` sits at 790/800.
- **OBS-7** — `docs/.workflow/inc-i-173-M3-implementation.md` §5.1/§6/§7 are stale: they report the
  flood test BLOCKED and an M3 total of 93; the fixture was repaired (test plan §6) and the measured
  total is 94, 19/19 in that file. Refresh before the commit message quotes it.
- **OBS-8** — `inc_i_173_m3_f7_cross_list.rs` header line "IP-S1 shape = 0 inputs, ONE output"
  contradicts its own prose and code (TWO outputs, load-bearing for `is_coinbase()`).

---

## QA iteration 1 re-validation (2026-08-11)

Scope: the QA-BLOCK-1 fix only, plus regression. The six items already CLOSED were not re-argued;
only their gate and prohibition evidence was re-run.

**Method.** Two throwaway probe files were written from scratch — `crates/storage/tests/` (9 tests,
loader bounds) and `bins/node/tests/` (10 tests, resolution cache) — measured, then **deleted**. They
were written against the *properties*, not against the developer's tests, so they are an independent
reproduction rather than a re-run. Release profile, same machine as pass 1. The build gate below was
run **after** deleting them, so it reflects the developer's tree exactly. No implementation file and
no existing test file was modified at any point.

I did **not** re-run the developer's four claimed mutations, because doing so requires editing
implementation files, which this pass forbids. Independent from-scratch probes covering the same
partitions are the substitute, and they are the stronger check: they can fail for reasons the
developer's tests were never shaped to catch.

---

### Property (a) — bounded decode, re-probed

#### Is the 911,388-byte ceiling genuinely DERIVED, or a round number dressed up?

**DERIVED.** Verified three ways, the third being decisive.

1. *Arithmetic re-derived from scratch*, without reading the developer's comment:
   `PublicKey` 8+32 = 40; `Signature` 8+64 = 72; `MaintainerSignature` 112;
   `MaintainerChangeData` 40 + 8 + 5×112 + 1 + 8 + 256 = **873**;
   record 1 + 873 + 8 + 8 = **890**; body 4 + 8 + 8 + 1024×890 = **911,380**;
   file 8 + 911,380 = **911,388**. Every term matches.
2. *Constants pinned* — `MAX_ROTATION_RECORD_ENCODED_BYTES == 890` and
   `MAX_ROTATION_JOURNAL_ENCODED_BYTES == 4+8+8+1024*890`.
3. **Measured against the REAL bincode encoder**, which is what makes it derived rather than
   asserted:

   | Quantity | Real encoder | Constant |
   |---|---:|---:|
   | maximal legal record (5 sigs, 256-B reason) | **890 B** | 890 |
   | maximal legal journal body (1024 such records) | **911,380 B** | 911,380 |
   | the same journal ON DISK, through `save_rotation_journal` | **911,388 B** | 911,388 |

   The maximal legal journal is *exactly* the ceiling and **loads `Ok` with 1024 records in
   13.1 ms**. A ceiling built on the pass-1 figure of 785 would have been 90,112 B too small and
   would have refused it — OBS-5 was load-bearing, not cosmetic, and it was fixed first.

#### Is the ceiling applied BEFORE `fs::read`?

**YES, and the second arm is not decorative.** `read_rotation_journal_file` opens the file, checks
`metadata().len()` against the ceiling, and *then* reads through `Read::take(ceiling + 1)`.

| Probe | Input | Pass-1 result | Pass-2 result |
|---|---|---|---|
| P2 | QA's original 22,400,094-B plant, 1 record, 200,000 signature entries | `Ok` in **552 ms**, then a **5.84 s** replay | **`Err` in 30–49 µs**, `maxrss` delta **0 B**, message names the BYTE ceiling and the real file size |
| P2 | same file via `read_rotation_journal_file` (the resolver's entry point) | n/a | **`Err` in 13.5 µs** |
| P3 | ceiling **+1** = 911,389 B | n/a | **`Err`**, message `it is 911389 bytes, over the ceiling of 911388` |
| P3 | exactly 911,388 B | n/a | **NOT size-refused** — the bound is inclusive, as the maximal legal journal requires |
| P1 | maximal LEGAL journal, 911,388 B | n/a | **`Ok`, 1024 records, 13.1 ms** — the bound does not re-create the INC-I-173 class |

#### Attempts to defeat it

| # | Attack | Result |
|---|---|---|
| 1 | **File just under the ceiling with maximally expensive contents** — one record stuffed with 8,000 signature entries, 896,094 B (the most a legal-size file can carry) | **`Err` in 17.2 ms**, `maxrss` delta 574–590 KB, refused by the per-record F5 cap. **Zero Ed25519** — the replay is never reached. This is the residual worst case for a planted file, versus 552 ms + 22 MB before |
| 2 | **Hostile bincode length prefix inside a legal-size buffer** — records vector declaring `u64::MAX`, `2^40`, `1024`, `1025` inside a 1,024-byte file | All four **`Err` in 15–54 µs**, `maxrss` delta 0–32 KB. serde's cautious sizing plus the truncated buffer; no unbounded allocation |
| 3 | **Record count exactly at the cap** — 1024 *minimal* records, 67,612 B (under the byte ceiling, so the COUNT bound is what is being tested) | **`Ok`** — at the cap is legal (`>` not `>=`) |
| 4 | 1025 minimal records, 67,678 B | **`Err`**, message names `over the cap of 1024`. The record cap still bites for shapes the byte ceiling cannot catch |
| 5 | **Per-record F5 caps** — 5 sigs + 256-B reason / 6 sigs / 257-B reason | `Ok` / `Err` / `Err`. Both bounds are `>` not `>=`, and both messages name the specific record index and applied height |
| 6 | **Symlink** pointing at a 5 MB file | **`Err`** — `File::open` follows the link, so `metadata()` reports the TARGET's 5,242,880 B and the ceiling fires. `maxrss` delta 0 B |
| 7 | **Directory** in place of the file | **`Err`** — `io error: Is a directory (os error 21)` |
| 8 | **Unreadable file** (mode 000) | **`Err`** — `io error: Permission denied (os error 13)`. No panic, no partial read |
| 9 | **FIFO with a writer pumping 8 MB** — `metadata().len()` on a FIFO is **0**, so the metadata arm passes it | **`Err`** — the `Read::take` arm caught it at exactly `ceiling + 1` = 911,389 B and refused. `maxrss` delta 49–65 KB. **This is the empirical proof that the TOCTOU arm is load-bearing**; the metadata check alone would have admitted an unbounded stream |
| 10 | **FIFO with no writer** | **No return after 8 s** — `File::open` blocks. See **OBS-9**; not introduced by this fix |

#### Fail-closed preserved?

**YES, and byte-identically.** Probe Q5 drove an over-ceiling journal through the real
`TrustRootResolver` on a ROTATED set and compared it with a rotated set that has *no journal at all*:

```
oversized journal (911,389 B) -> provenance OnChain, usable false, keys 0, in 35.6 µs
rotated + NO journal          -> provenance OnChain, usable false, keys 0
```

Same provenance, same usability, same key count. No fail-closed path was softened to make the bounds
fit: the refusal is an `Err` that `load_rotation_journal_or_empty` / `JournalSource::into_journal`
turn into an EMPTY journal plus a loud `error!`, and `TrustRoot::resolve`'s rotated branch then
returns `on_chain(vec![])`. Nothing falls back to the compiled bootstrap keys on any path.

**And the fleet is not bricked.** Probe Q5b: an **UNROTATED** host (the chain-derived bootstrap
five — every host today) with the 22.4 MB plant in its data directory resolves
`OnChain, usable true, 5 keys, in 47 µs`. The journal is not consulted on that branch, and the new
ceiling makes the wasted read cheaper than it was before the fix.

---

### Property (b) — no remote amplification, and no stale trust root

The risk QA flagged for this half: *a cache that misses a change would let a tampered set stay
authoritative — strictly worse than the DoS it replaces.* Ten probes, all against that.

| # | Probe | Measurement |
|---|---|---|
| Q1 | 41 `getUpdateStatus`-shaped calls, largest legal journal (911,388 B), unchanged inputs | **`recomputations() == 1`.** First call (miss) **15.8–22.4 ms**; per subsequent call **438–528 µs** (bounded read + BLAKE3 only). Every hit still returned `usable false` — the fail-closed verdict survives reuse |
| Q2 | **200 consecutive SAME-LENGTH content edits** (`bootstrap_last_updated` is a fixed-width `u64`), one resolve after each | **201 recomputations — every single edit re-resolved.** Not one stale serve |
| Q2b | two same-length writes inside one filesystem mtime tick | **Could not be forced** on APFS (nanosecond mtime). Declared, not claimed — see Q2c |
| Q2c | **THE DECISIVE TEST.** Write A, snapshot `(len, mtime)`, write B of the same length, restore the original timestamp with `touch -r` | `len 918 == 918`, `mtime 1786414219377266262 ns == 1786414219377266262 ns`, **content different** → **re-resolved (`recomputations` 1 → 2)**. A `(len, mtime)` key would have served a STALE verdict here. The key is provably the CONTENT |
| Q3 | journal absent → appears → grows to maximal → deleted | **4 recomputations, one per transition**; the repeated "still absent" call was a hit |
| Q4 | in-memory `MaintainerState` bootstrap-five → rotated → bootstrap-five | **3 recomputations**, and the verdict FLIPS `usable true (5 keys)` → `usable false (0 keys)` → `usable true` |
| Q4 | **corrupt `maintainer_state.bin` ON DISK** between calls | **No recomputation, verdict unchanged — and that is CORRECT.** The resolver's state input is the in-memory `MaintainerState` passed by the caller; that file is read once at startup by `load_maintainer_state` and was never an input to `resolve_trust_root` before the fix either. The cache did not weaken this; it is unchanged |
| Q5 | oversized journal, repeated | 1 recomputation over 11 calls, every one `usable false` |
| Q6 | content **A → B → A** | **3 recomputations** — returning to a previously-seen content re-resolves (single-entry cache), so no historical verdict can be resurrected |
| Q7 | **8 threads × 300 resolves against a concurrently mutating journal** | 15–18 recomputations, **no panic, no non-fail-closed verdict**. Structurally safe: the key is computed from the bytes read *in that call* and the decode consumes the *same* buffer (`JournalSource` is moved into `into_journal`), so a hit requires byte-identity with the current read — a racing writer can only cause an EXTRA resolution, never a stale one |

**Amplification, before and after.** The residual an attacker can force per remote call on a rotated
host is now: bounded read (≤ 911,388 B) + BLAKE3, ≈ **438–528 µs**, unless they also mutate the file
between calls, in which case it is the decode, ≈ **13–18 ms**. Before the fix a single planted 22.4 MB
file bought **552 ms of load plus 5.84 s of Ed25519 per call**, with a ~22.9 GB extrapolation at the
record cap. The security property "a CHANGED journal or state is always re-resolved" is preserved,
which is the only thing that mattered.

---

### Regression

**Six items — all still CLOSED.** No item's evidence changed; the two pass-1 caveats
(Item 2 doc gap, Item 5 understated delta) are now closed too, so the table above upgrades both.

**Seven hard prohibitions — all still HELD**, re-verified against base `32e0a650` rather than taken
from the report:

| # | Prohibition | Evidence QA re-ran |
|---|---|---|
| 1 | No activation-height VALUE moved | `git diff 32e0a650 --stat -- crates/core/src/network_params/` → **empty** |
| 2 | No version constant bumped | Diff grep over `crates` + `bins` for `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION`, `MAINTAINER_STATE_VERSION`, `VERSION: u32/u64 =` → **no hits**. `git diff --stat -- '*Cargo.toml'` → **empty**. `MAINTAINER_ROTATION_JOURNAL_VERSION` is still 1 |
| 3 | L1/L2 untouched | `git diff -U0 -- crates/core/src/validation/transaction.rs` → exactly **two 1-line hunks, `@@ -171 +171 @@` and `@@ -174 +174 @@`**, both the `ctx` threading validated in pass 1. Character-identical otherwise |
| 4 | Below-gate frozen branch untouched | `git diff 32e0a650 --stat -- crates/core/src/validation/utxo.rs` → **empty** |
| 5 | Apply path NON-FATAL | `apply_maintainer_change` still returns `Option<(u32, u64)>`; `record_rotation` (`governance.rs:203`) returns `()` and both failure arms are `warn!` + continue. The new loader refusals arrive as an `Err` it already handles by warning and returning — verified by reading the body, not the doc comment |
| 6 | No deploy | `~/testnet/bin/doli-node` mtime **`Aug 10 13:47`**, unchanged. No `cp`, no `codesign`, no SSH, no node started/stopped/restarted. **No RPC call of any kind was made in pass 2** |
| 7 | No commit / no push | `git log --oneline -1` = **`32e0a650`**. Tree dirty and uncommitted |

**Build gate** (run after QA's probe files were deleted): `cargo build --release` EXIT 0;
`cargo clippy --workspace --all-targets -- -D warnings` EXIT 0, no warnings; `cargo fmt --check`
EXIT 0.

**Tests:** M3 targets **103 passed / 0 failed** (+ `updater trust_root_fail_closed` 13). Workspace
**3542 passed / 3 failed / 43 ignored**, 155 targets ok. The 3 failing instances are the **2 KNOWN
pre-existing tests** named in the brief and nothing else — recorded, not fixed.

---

## Scope Validated / Not Validated

**Validated:** all six contract items end to end; the seven hard prohibitions; the module-size
budget; the full build gate; the full workspace test suite; the M3 test targets plus the migrated
INC-I-172 suites; eleven exploratory probes (pass 1) plus nineteen adversarial probes (pass 2);
live READ-ONLY `getMaintainerSet` on three testnet nodes (pass 1 only); specs and docs drift.

**Not validated (declared):**
- **Reorg/rollback of an applied maintainer rotation** — requires driving a live reorg past a
  governance transaction; not constructible read-only. See OBS-3.
- **End-to-end mined rotation on a live network** — blocked by design: the mainnet/testnet
  bootstrap private keys are not in this repository (Amendment 1). The replay is covered by the pure
  predicate with an injected bootstrap set, which is the correct substitute.
- **`tracing` log RECORDS** on the apply path and in `resolve` — declared unasserted by the test
  plan §4.4. The `MAINTAINER_SET_DIGEST=` token and the `SIGNED BINDING` block were confirmed by
  inspection only.
- **`doli-node maintainer add|remove` interactive output** — no automated coverage; inspected.

**Not validated in pass 2 (declared):**
- **The developer's four claimed mutation tests** — reproducing them requires editing implementation
  files, which pass 2 forbids. Substituted by nineteen from-scratch probes over the same input
  partitions; they are an independent reproduction, not a re-run.
- **Live network of any kind** — no RPC call was made in pass 2. The pass-1 `getMaintainerSet`
  measurements stand as recorded.
- **An honest long-history journal's true replay cost** (~25,600 Ed25519) — not constructible in
  this repository, since the bootstrap private keys are absent and must stay absent (Amendment 1).
  A hostile journal costs at most one record's verification because the replay returns on the first
  refusal; probe P6 measured the deepest reachable hostile cost at 17.2 ms with zero Ed25519.

---

## Method Note

**Pass 1.** Three throwaway probe files under `crates/{mempool,core,storage}/tests/`, measured and
**deleted**.

**Pass 2.** Two throwaway probe files — `crates/storage/tests/zzz_qa2_journal_bounds_probe.rs`
(9 tests) and `bins/node/tests/zzz_qa2_trust_root_cache_probe.rs` (10 tests) — created, measured,
and **deleted**; `git status` confirms neither remains. The build gate was re-run afterwards so it
reflects the developer's tree.

Across both passes: **no implementation file and no existing test file was modified at any point.**
`git log` head is `32e0a650`; nothing was committed, pushed, deployed or restarted, no SSH was used,
and pass 2 made no RPC call at all.
