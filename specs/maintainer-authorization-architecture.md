━━━ FINDINGS — 4 total (DECISION:4) ━━━

  [F1] DECISION conf(0.82, converged) — crates/core/src/maintainer/data.rs:46-49 — rebind the authorization to a domain-separated, genesis-bound, expiry-bound BLAKE3 digest produced by one owned signing-message constructor
  [F2] DECISION conf(0.82, converged) — crates/core/src/maintainer/data.rs:10-17 — delete the write-only `reason` field (swap for `valid_before: u64`) and canonicalize the signature-vector order; net payload shrinks
  [F3] DECISION conf(0.80, converged) — bins/node/src/node/apply_block/governance.rs:32-102 — keep enforcement at the single existing NON-FATAL site behind a new activation height #22; put nothing new in the shared validator
  [F4] DECISION conf(0.72, converged) — bins/node/src/node/apply_block/governance.rs:58-98 — add the non-fatal `height >= AH && height >= valid_before` expiry check with a distinct AUTH_EXPIRED log token, update `derivation.rs` in lockstep, and lock the no-new-fatal-path property with a three-path test

  Speculative: 3 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Maintainer Authorization Architecture (INC-I-176)

RUN_ID=518 · INC_ID=INC-I-176 · repo `/Users/isudoajl/ownCloud/Projects/doli-network/doli`
Branch `bugfix/inc-i-173-state-only-fee-gate`, HEAD `3f8bf185`. **Local only — never pushed.**
Synthesized from 5 independent design evaluators (subtraction, restructure, patterns, failures, radical).

---

## Problem Statement

A maintainer governance signature (`AddMaintainer` / `RemoveMaintainer`) is a **context-free, replayable
bearer token**. The signed bytes are `"add:"|"remove:" ++ target_hex` and nothing else
(`crates/core/src/maintainer/data.rs:46-49`): no network identity, no effect coverage beyond the target,
no freshness term. `reason` and the order of the signature vector ride *outside* the signature yet mutate
the txid, so any txid-level dedup is theatre. Redesign the authorization scheme so a signature binds to its
network, its full effect, and a signer-chosen validity window — without introducing a fleet-splitting reject
path and without making the imminent INC-I-175 key rotation unperformable.

**The honest value statement — Era 0 vs Era 1 (Failure Analyst, conf 0.70 measured).**
This fix protects against **nothing today**. The five mainnet maintainer private keys are already public
(INC-I-175). An adversary who holds the keys signs fresh bytes under any binding scheme; no binding
constrains a key-holder. INC-I-176 is **not** a fix for INC-I-175 — it is a fix for the world the INC-I-175
rotation *creates*. Its entire value is Era 1 (post-rotation): without it, once the rotation runs, **no
maintainer key can ever be durably retired** — a stranger holding no keys can replay the archived `add:X`
blob after `X` is revoked and reinstate it (attack A2/R6). That standing defect the rotation itself cannot
fix, and it is the reason to build INC-I-176.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 BLAKE3 hash per maintainer-tx verification (observed)
  Memory:   -up to 256 bytes per decoded payload net (observed)
  IO:       0 (observed)
  Network:  -up to 248 bytes per gossiped maintainer tx (observed)
  Disk:     -up to 248 bytes per maintainer tx permanent (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: ship no authorization change and rely on the rotation to diverge the key arrays plus state-precondition idempotence
Why this proposal anyway: the cheaper path leaves revocation permanently undoable by a stranger after the rotation, a standing defect the rotation cannot fix
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Delete `reason`, canonicalize signature order; subtraction alone is NOT sufficient | conf(0.65, measured) | "The minimum irreducible addition is exactly one thing: content in `signing_message`." |
| Restructurer | boundaries | Factor predicate by **data dependency**: set-independent terms fatal in shared validator, set-dependent non-fatal at `governance.rs` | conf(0.65, observed) | D1: `ValidationContext` structurally cannot see the maintainer set (`grep -c maintainer`=0); the whole incident follows |
| Pattern Matcher | patterns | Domain tag + genesis + expiry + full-effect commit; "a Cosmos SignDoc minus the sequence number" | conf(0.65, observed) | The house already wrote the fix's tripwire test; **INC-I-176 IS the M3 that test names** |
| Failure Analyst | failures | 16 pass/fail filters (F1–F16); the quorum predicate can NEVER be block-fatal | conf(0.70, measured) | S1: the maintainer set is node-local, attacker-writable, MEASURED fleet-divergent → no fatal quorum check |
| Radical Simplifier | minimal | SSF: sign BLAKE3(domain‖genesis‖action‖target‖valid_before), delete `reason`, one non-fatal check behind AH | conf(0.70, measured) | R6 (revocation reversal) is the ONLY independent harmful primitive; exposure is ZERO today |

---

## Convergence Matrix

```
                                          Sub   Rest  Patt  Fail  Rad
Delete `reason` (write-only)               Y     Y     Y     Y*    Y     → 5/5  DELETE   conf(0.82,converged)
Domain-separation tag in signed bytes      Y     Y     Y     Y     Y     → 5/5  ADD      conf(0.82,converged)
Genesis/network binding in signed bytes    ~     Y     Y     Y     Y     → 4/5  ADD      conf(0.82,converged)
Signer-chosen expiry (`valid_before`)      -     Y     Y     ~     Y     → 3/5  ADD      conf(0.72,converged)
Canonicalize signature-vector order        Y     Y     Y     Y     ~     → 4/5  ADD      conf(0.80,converged)
Enforce at ONE non-fatal site (govern.)    ~      split Y     Y     Y     → 4/5  PLACE    conf(0.80,converged)
New AH #22, own field, u64::MAX mainnet     Y     Y     Y     Y     Y     → 5/5  GATE     conf(0.88,converged)
Reject monotonic nonce / set-version        Y     Y     Y     Y     Y     → 5/5  KILL     conf(0.85,converged)
Reject set-digest / CAS binding             -     -     Y     -     Y     → 2/5  KILL     conf(0.70,converged)
INC-I-176 BEFORE the INC-I-175 rotation     Y     Y     Y     Y     Y     → 5/5  SEQUENCE conf(0.85,converged)
REQ-176-010 (absolute single-use) NOT met   -     Y     Y     Y     Y     → 4/5  RELAX    conf(0.80,converged)
```
`*` Failure Analyst reaches "delete `reason`" through its subtraction note (F5 best satisfied by deletion).
`~` = concurs on the direction but did not lead with it. `split` = Restructurer prefers the fatal
deterministic-split variant (routed to Options for User Decision).

**Independence check (delete `reason`).** Three independent evidence sources: Subtractionist reader/writer
census (0 consuming readers outside 1 self-bounding cap check), Radical per-root grep **with a positive
control** (unrelated `reason` uses in `validation/producer.rs`, `rpc/methods/guardian.rs` confirm the scan
works), Restructurer blast + half-wiring analysis (D8: RPC cannot even set it on the add path). Genuine
convergence → confidence boost applies. **Residual (preserved, not laundered):** no evaluator could confirm
the *off-chain* value of `reason` (block explorer, governance audit trail). That question is routed to the
user gate; the security property does not depend on the answer.

---

## Defect, Consolidated

Four distinct defect classes wear one incident name. Each is tagged by the strongest evidence any evaluator
produced.

1. **Cross-protocol / release-signature collision — TEST-PROVEN.** The same 5 keys, same primitive, same
   `"{}:{}"` grammar sign both a maintainer change and a binary release. A live in-repo test,
   `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs::the_collision_still_exists_and_only_m3_closes_it`,
   asserts a **release signature verifies as an `AddMaintainer` authorization**, and its own comment says
   "when M3 lands domain separation, this test MUST flip to asserting NO collision." **INC-I-176 is that M3.**
   A single `doli sign`-style release-signing operation could mint a maintainer seat. This is more severe
   than the incident's recorded "replay only" framing and is treated as a first-class defect.
2. **Cross-network replay — MEASURED (arrays since diverged).** `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` and
   `_TESTNET` (`crates/updater/src/constants.rs`) were byte-identical when this was measured; the INC-I-196
   cutover made them disjoint. The defect is unchanged, because it never depended on the arrays matching:
   `signing_message` binds no chain id.
   A testnet-minted `add:` blob is byte-valid on mainnet. Note the Subtractionist correction: the *live
   authorization* verifier reads the on-chain `MaintainerSet` (seeded from the producer set), so the real
   Era-1 mechanism is **producer-key reuse across networks**, and "just change the keys" is NOT a zero-code
   substitute (it is circular — INC-I-176 is a prerequisite for the rotation that would diverge the keys).
3. **Cross-time replay (revocation reversal) — MEASURED.** An archived `add:X` blob is effective again
   whenever ≥threshold of its original signers are still members and the state precondition re-holds
   (`set.rs:130-149,303-308`). This is attack A2 / scenario **R6** — the Radical Simplifier's analysis
   proves it is the **only independent harmful primitive** (R1–R5 are inert by state precondition, R7 is
   derivative, R8 is cross-network). After the rotation, revocation becomes permanently undoable.
4. **Txid malleability — MEASURED.** `to_bytes` is `bincode` over the whole struct, so `reason` and the
   **order** of the signature vector land in the txid but not in `signing_message`. 3 sigs → 6 variants;
   the F5 cap of 5 → **120 distinct txids** per authorization before touching `reason`. This kills any
   remedy keyed on txid uniqueness (Failure Analyst F5).

**Arming state — MEASURED, and it inverts urgency.** Mainnet `inc_i_173_activation_height = u64::MAX`
(`crates/core/src/network_params/defaults.rs:275`) ⇒ `AddMaintainer`/`RemoveMaintainer` are **unmineable on
mainnet today**; the testnet scan (`maintainer/mod.rs:129-136`, 2026-08-11) found zero; the bootstrap five
were seeded via `MaintainerSet::with_members`, not `add_maintainer`, so **no `add:` blob exists on-chain**.
**There is not one replayable authorization blob in existence on any DOLI network today.** The exposure is
manufactured by the INC-I-175 rotation itself. The fix is cheap **because it is early** — and it gets more
expensive every week it is deferred (Radical A4). This fact must be re-verified immediately before
implementation, not inherited from this document (Radical C6).

> ### ⛔ CORRECTION — the paragraph above is FALSIFIED (measured 2026-08-12, INC-I-176 M1a)
>
> The re-verification the paragraph itself demanded was performed, read-only, against the LOCAL testnet
> (`127.0.0.1:8500`), and it **contradicts the claim**. Testnet block **136_690** carries an
> `add_maintainer` transaction, txid `62a3bfbd388a208d98d1b3ebb35757426358d1fb3730112297b12eb69bf8bc81`,
> size 417, with a **385-byte `extra_data`** that decodes as a `MaintainerChangeData` of the pre-INC-I-176
> shape (3 signatures, `reason = None`, final byte `0x00`). Mainnet is unaffected
> (`inc_i_173_activation_height = u64::MAX` there), but "no maintainer tx exists on any network" is false.
>
> Two consequences, both binding on every later milestone:
>
> 1. **The vacuity argument at §"Activation-height decision" below is void.** Below-gate bit-identity is
>    NOT vacuous — it has a real on-chain witness that every node re-validates on every sync from genesis.
> 2. **The payload swap may not ride ungated.** `MaintainerChangeData::from_bytes` is consumed **fatally and
>    without a height gate** in the shared validator, so a field add/remove/reorder makes block 136_690
>    undecodable in *both* deploy directions, and a synchronized deploy does not repair it.
>
> The payload work (delete `reason`, add the `valid_before` field, canonicalize the signature order) was
> therefore **moved out of M1 into a new milestone M2.5**, where an activation height **and an explicit
> format discriminator** carry it. M1a shipped the signing-message work only, with **zero** wire change.

---

## The Design (Recommended — the SSF, hardened)

**Bottom line:** change *what is signed*, not *where it is checked*. One owned signing-message constructor
produces a domain-separated, genesis-bound, expiry-bound BLAKE3 digest; delete the write-only `reason`;
canonicalize the signature order; enforce everything at the single existing **non-fatal** site behind a new
activation height. Zero new modules, zero new fatal paths, net-smaller payload.

### Exact bytes signed (at and above AH-176)

```
message = BLAKE3(
    b"DOLI-MAINTAINER-CHANGE-V1"   // domain tag — kills the cross-protocol/release-signature collision (defect 1)
  ‖ genesis_hash                   // 32 B, network scope — kills cross-network replay (defect 2). Already in scope at governance.rs:29
  ‖ [is_add as u8]                 // effect: action (also bound structurally via tx.tx_type)
  ‖ target.as_bytes()              // effect: target
  ‖ valid_before.to_le_bytes()     // 8 B, the ONE new payload term — signer-chosen expiry (defect 3, outside the window)
)
```

This matches the in-house digest idiom verbatim (`crates/core/src/maintainer/digest.rs:20,72-89`, which
already binds `b"DOLI-MAINTAINER-SET-V1" ‖ genesis_hash ‖ …`). It is the house pattern; it outranks any
industry pattern. Every non-signature field of the payload is now inside the signed bytes — effect coverage
holds **by construction**, because the payload has no unsigned fields left. A `signing_message_legacy`
(today's two lines) is retained solely so the below-gate branch is well-defined.

### Verification sites and fatality

| Site | Change | Fatality |
|---|---|---|
| `crates/core/src/validation/tx_types.rs:753` (shared validator; mempool+builder+apply) | **Only a deletion** — remove the `reason` length-cap branch (`:817-822`). Nothing added. | unchanged; no new reject path |
| `bins/node/src/node/apply_block/governance.rs:32-102` (the single authority site) | Read AH; select legacy-vs-new message by height; add non-fatal `height >= AH && height >= valid_before` → `warn!(AUTH_EXPIRED)`; genesis already in scope at `:29` | **NON-FATAL** (`Option` return, `warn!` only) — unchanged from today |
| `bins/node/src/node/apply_block/tx_processing.rs:106` (the one fatal path, `return Err`) | **No new rejection condition** | unchanged |

Because the predicate is never reachable from the mempool or the builder, **INV-VALIDATION-001's three-path
parity obligation does not attach as a fatal-path burden**, and the unbounded admission→apply height skew
(Failure Analyst S4) cannot become a block-poison — the expiry is evaluated exactly once, at the block's
own height, uniformly across every applying node. See "Contradiction 1 resolution" in the reasoning trace.

### Activation-height decision

New field **#22**: `inc_i_176_auth_binding_activation_height` in `crates/core/src/network_params/`.
Mainnet `u64::MAX` (frozen pre-activation), devnet `0`, testnet pinned strictly above the live tip measured
at pin time. **Constant gate — never a `HardForkSchedule` entry.** May NOT reuse #20
(`maintainer_derivation_activation_height`) or #21 (`inc_i_173_activation_height`). The gate is reached the
INC-I-172 dispatcher way — height passed in as a plain `u64` so `crates/core::maintainer` stays a leaf
module (`set.rs:259-261` idiom). ~~Below-gate bit-identity holds **vacuously** (no maintainer tx exists on
any network), the same argument INC-I-173 M3a used — and it carries the same expiry date.~~
**STRUCK 2026-08-12 (INC-I-176 M1a): FALSIFIED by measurement — testnet block 136_690 carries a real
`add_maintainer` payload. See the CORRECTION box above.** Below-gate bit-identity must now be proven
BEHAVIOURALLY, not assumed: the message constructor keeps a `signing_message_legacy` arm that is
bit-identical to the pre-INC-I-176 format (pinned by
`req_176_030_legacy_message_is_byte_identical_to_todays_format`), and the payload encoding is frozen
byte-for-byte (pinned by `crates/core/tests/inc_i_176_m1a_wire_freeze.rs`, which decodes block 136_690's
actual bytes).

### What is deleted / canonicalized — **ALL OF IT MOVED TO MILESTONE M2.5**

> **Scope correction (2026-08-12, INC-I-176 M1a).** Everything in this subsection is a **payload** change.
> Per the CORRECTION box above, a payload change is a **bincode wire-format break on frozen consensus
> history** (testnet block 136_690) and cannot ride ungated. It is therefore **deferred, not abandoned**, to
> a new milestone **M2.5**, which must carry its own activation height **and** an explicit format
> discriminator. `MaintainerChangeData` moves **zero bytes** in M1a — its bincode encoding is byte-identical
> to the pre-INC-I-176 shape for every input, pinned by `crates/core/tests/inc_i_176_m1a_wire_freeze.rs`.
> The byte figures below are the M2.5 target, not a shipped measurement.

- **[M2.5] Delete** `MaintainerChangeData::reason` (swap for `valid_before: u64`), the constant
  `MAX_MAINTAINER_CHANGE_REASON_BYTES`, its cap branch in `tx_types.rs`, the RPC param, the CLI arg.
  Target net payload: **873 B → 616 B**; headroom under the unchanged 1024-byte
  `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES` cap grows **151 B → 408 B**. Both figures must be re-measured
  through the real bincode encoder when M2.5 lands, never inherited from this prose. **In M1a the three
  caps of INC-I-173 M3a all remain in force, unchanged.**
- **[M2.5] Canonicalize** the signature-vector order (sorted ascending by pubkey). Deferred with the
  payload for two independent reasons: (a) `extra_data` feeds the txid, so sorting emits different BYTES
  for the same caller input — a behaviour change, which does not belong in a zero-wire-change milestone;
  and (b) per security audit **F3** it is **not** an adversarial control, because `sendTransaction` accepts
  any ordering off the wire, so M1a must not be described as closing txid malleability.

---

## Definite Changes (High Convergence)

- **ARCHITECTURAL: [F1] Rebind the authorization signature to a domain-separated, genesis-bound,
  expiry-bound BLAKE3 digest produced by one owned signing-message constructor.**
  Convergence: Radical (SSF, 0.70 measured) + Restructurer (P1/P3, 0.65 observed) + Pattern Matcher
  (composite, 0.65 observed) + Subtractionist ("content in `signing_message` is the irreducible addition",
  0.65 measured) + Failure Analyst (network-identity is the only binding surviving every Part-B attack).
  Evidence: `crates/core/src/maintainer/data.rs:46-49` (today's context-free message);
  `apply_block/governance.rs:29` (genesis already in scope); `maintainer/digest.rs:20,72-89` (the identical
  in-house idiom); `updater/tests/inc_i_172_m2_release_sign_arg_validation.rs` (the domain tag is what flips
  the collision test). Confidence: conf(0.82, converged).
  Seam eliminated: the context-free bearer token. The signature now names its protocol (domain tag closes
  the release-signature collision, defect 1), its network (genesis closes cross-network, defect 2), and its
  full effect. A new leaf constructor `crates/core/src/maintainer/authmsg.rs` (~80 lines) is the sole
  producer of these bytes, retiring the four duplicated `format!` sites and the out-of-repo
  `sign_maintainer.py`; a published golden vector pins the encoding (Full Bitfield Decode parity discipline).

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      +1 BLAKE3 hash per maintainer-tx verification (observed)
    Memory:   +57 bytes transient per signing-message construction (observed)
    IO:       0 (observed)
    Network:  0 (observed)
    Disk:     0 (observed)
    Latency:  0 (inferred)
  Inevitability: AVOIDABLE
  Cheaper alternative: keep signing_message on the payload and add terms as extra format args in place with no new file
  Why this proposal anyway: the cheaper path leaves four reimplementations one outside the repo with no golden vector, the encoder-decoder parity defect class the Full Bitfield Decode pillar warns about
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: [F2] Delete the write-only `reason` field (swap for `valid_before: u64`) and
  canonicalize the signature-vector order.**
  Convergence: Subtractionist P1 (0.65 measured) + Radical P2 (0.65 measured) + Restructurer P4 (0.60
  observed) + Failure Analyst subtraction note (F5 best satisfied by deletion). 5/5, independent evidence.
  Evidence: `data.rs:10-17` (struct); sole production reader is its own cap check `tx_types.rs:817-822`
  (Radical per-root grep with positive control; Subtractionist reader/writer census); RPC cannot set it on
  the add path (`rpc/methods/governance.rs:260-264`, Restructurer D8); `transaction/core.rs:504-506`
  (`extra_data` → txid). Confidence: conf(0.82, converged).
  Seam eliminated: txid malleability. Deleting `reason` removes 256 attacker-chosen, fee-exempt,
  permanently-stored, unsigned bytes and one malleability vector; ~~canonicalizing the signature order closes
  the second at zero signed bytes. Both are removed *by construction* rather than defended.~~ Net payload
  shrinks (873 B → ~625 B). **User-gate caveat:** the off-chain value of `reason` is unconfirmed (see
  Options); the fallback (sign `reason` in place) preserves the security property at +~250 signed bytes.

  > **AMENDED 2026-08-12 (INC-I-176 M1a, QA [F4]) — struck clause above.** Canonicalizing the
  > signature-vector order does **NOT** close a txid-malleability vector. Per security audit **F3** it is not
  > an adversarial control at all: `sendTransaction` accepts any signature ordering off the wire, so an
  > attacker simply submits the ordering he wants and a construction-time sort never sees him. The sort is a
  > **construction-time normalization only** — one canonical encoding per honest signer set, which is a
  > determinism/diffability property, not a security property. The reasoning is retained above because the
  > `reason` deletion half of it still holds (that one *is* removed by construction), but the whole bullet is
  > **DEFERRED to milestone M2.5**, not shipped: see the scope correction under "What is deleted /
  > canonicalized" and the amendment at the end of "Complexity Comparison". **M1a must never be described as
  > closing txid malleability.** The 873 B → ~625 B figure is an M2.5 target, never a measurement.

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      -O(n) bincode string decode removed per maintainer tx (observed)
    Memory:   -up to 256 bytes per decoded payload (observed)
    IO:       0 (observed)
    Network:  -up to 248 bytes per gossiped maintainer tx (observed)
    Disk:     -up to 248 bytes per maintainer tx permanent (observed)
    Latency:  0 (inferred)
  Inevitability: AVOIDABLE
  Cheaper alternative: keep reason and sign it, zero net wire bytes, preserves the transparency field
  Why this proposal anyway: signing reason preserves 256 permanent attacker-chosen fee-exempt bytes per tx forever for a field the RPC cannot even set on AddMaintainer
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: [F3] Keep enforcement at the single existing NON-FATAL site behind a new activation
  height #22; put nothing new in the shared validator.**
  Convergence: Radical P4 (0.70 measured) + Failure Analyst F1/F2 (0.70 measured) + Pattern Matcher house
  rule ("authority is effect: non-fatal, gated") + Subtractionist (safest design changes what is signed,
  not where it is checked). Restructurer offers a fatal deterministic-split variant → routed to Options.
  Evidence: `governance.rs:17-22,58,61,95,98` (`Option` return, `warn!`-only, confirmed this session);
  `tx_types.rs:753` reached from mempool+builder+apply → any predicate there is fatal at
  `tx_processing.rs:106,125`; `snapshot.rs:24-58` (`grep -c maintainer`=0 — the set is not in the state
  root); Failure Analyst S1 (the set is node-local, attacker-writable, MEASURED fleet-divergent 88289 vs 1).
  Confidence: conf(0.80, converged).
  Seam eliminated: the fork / block-poison vector. Because the authority predicate reads the node-local,
  attacker-writable maintainer set, it can **never** be block-fatal (F1). Keeping enforcement at the one
  non-fatal site means no fleet-splitting reject path is added — REQ-176-020 holds structurally, not by
  test. The AH bounds divergence among *new binaries*; fleet-uniform deploy before the first maintainer tx
  is the actual control (Radical C7), already an INC-I-175 prerequisite.

  ━━━ RESOURCE COST — NEGLIGIBLE ━━━
  Dimensions:
    CPU:      0 (observed)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  0 (observed)
    Disk:     0 (observed)
    Latency:  0 (observed)
  Inevitability: AVOIDABLE
  Cheaper alternative: NONE-NEEDED
  Why this proposal anyway: it avoids the fatal reject path, the three-path parity burden, and the mempool-apply skew at zero cost
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Recommended Changes (Medium Convergence)

- **ARCHITECTURAL: [F4] Add the non-fatal `height >= AH && height >= valid_before` expiry check with a
  distinct AUTH_EXPIRED log token, update `derivation.rs` in lockstep, and lock the no-new-fatal-path
  property with a three-path test.**
  Convergence: Radical P3 (0.60 inferred, the expiry mechanism) + Restructurer (expiry is the only
  rotation-safe freshness term) + Pattern Matcher (`nLockTime`/CLTV/`timeout_height`/JWT `exp` all survive
  the offline-pre-signing + equal-authority constraint) + Failure Analyst F13 (distinct log token) & F14
  (`derivation.rs` lockstep). Evidence: R6 is the only independent harmful replay (Radical §2); expiry is
  signer-chosen so an equally-authorized adversary cannot advance it (only stall, which needs majority
  production — and the defender runs the producers); `derivation.rs:186-218` and `governance.rs:32-102`
  already disagree about slashes (Failure Analyst V9), so lockstep fixes a live bug; `governance.rs:61,98`
  today emit the same token for binding and counting failures. Confidence: conf(0.72, converged).
  Seam eliminated: revocation-reversal outside a signer-declared window. The expiry closes R6 permanently
  after `valid_before`. It does **not** close R6 *inside* the window — that residual is the REQ-176-010
  relaxation (see Options and "What This Design Does NOT Fix").

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      +1 u64 comparison per maintainer-tx apply (observed)
    Memory:   +8 bytes per decoded payload (observed)
    IO:       0 (observed)
    Network:  +8 bytes per maintainer tx (observed)
    Disk:     +8 bytes per maintainer tx permanent (observed)
    Latency:  0 (inferred)
  Inevitability: AVOIDABLE
  Cheaper alternative: a permanent tombstone rule where a removed maintainer can never be re-added, closes R6 and R7 at zero wire bytes
  Why this proposal anyway: the tombstone hands the adversary a permanent per-key denial primitive mid-rotation whereas a signer-chosen expiry is untouchable by the adversary
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## USER GATE — RESOLVED 2026-08-12 (binding; the Options below are now CLOSED)

The design gate was presented and answered. These resolutions are binding on implementation. The
"Options for User Decision" section that follows is retained for its evidence and reasoning; its verdicts
are superseded by this block.

| # | Decision | Resolution | Consequence |
|---|----------|-----------|-------------|
| 1 | Fatal split (deterministic terms fatal in the shared validator) | **REJECTED** | Enforcement stays at the single existing NON-FATAL site (`apply_block/governance.rs`). Nothing new enters `validation/tx_types.rs`. The D3/P2 parity gap is therefore **NOT** a prerequisite milestone. |
| 2 | REQ-176-010 absolute single-use | **RELAXED to bounded-validity** | The seen-set (REQ-176-032 evolution path) is **DEFERRED**, not built. Residual accepted: R6 replay *inside* the signer-declared window. Must be stated in the commit message and in the release notes. |
| 3 | `reason` field | **DELETED** (swap for `valid_before: u64`) | Closes malleability vector 1. REQ-176-033 satisfied by removal, not by signing. Any off-chain audit trail for change rationale is an operator concern, out of scope. **AMENDED 2026-08-12 — see the SECOND GATE below: the deletion is DEFERRED to M2.5, not reversed.** |

### SECOND USER GATE — RESOLVED 2026-08-12 (after the M1 review)

**The arming premise of this spec was FALSIFIED.** Lines 123-130 claimed "There is not one replayable
authorization blob in existence on any DOLI network today", and the spec itself ordered that the fact be
re-verified immediately before implementation. It was not re-verified before M1 was dispatched.

**MEASURED (orchestrator, live RPC to `127.0.0.1:8500`, 2026-08-12):** testnet block **136690** carries an
`add_maintainer` transaction (txid `62a3bfbd…`); testnet tip 146,711. The transaction was mined by this
project's own INC-I-173 M2 end-to-end verification. **The premise is false.**

**Consequence — AUDIT-P0-001, converged across QA + reviewer + 5/5 security lenses.** Swapping
`MaintainerChangeData.reason: Option<String>` (1-byte bincode `None` tail) for `valid_before: u64` (8-byte
tail) is an **UNGATED wire-format break on consensus-visible frozen history**. `from_bytes` returns `None`
and the PRE-EXISTING, **height-ungated** fatal decode at `crates/core/src/validation/tx_types.rs:809`
(orchestrator-verified: the `Err` sits outside every activation-height branch) hard-rejects. An M1 binary
therefore cannot sync past testnet block 136690, in **both** deploy directions. A synchronized deploy does
NOT repair this — the block is history and is re-validated on every full sync from genesis.

**Second hazard — AUDIT-P1-001.** Some old payloads decode *successfully* with a **different meaning**:
`reason: Some("")` reads as `valid_before = 1`; attacker free text reads as an arbitrary expiry height. A
misparse yielding a different valid authorization that no signature covers. Therefore a naive
try-new-then-fallback decoder is **unsound** — any version discriminator must be **explicit**.

**RESOLUTION (user, 2026-08-12): SPLIT THE WIRE CHANGE OUT OF M1.**

| Aspect | Resolution |
|---|---|
| M1 | **No wire change at all.** `reason` STAYS. M1 delivers only the owned constructor, the golden vector, legacy bit-identity, and `derivation.rs` lockstep. Zero decode risk. Strictly fewer lines than the rejected M1. |
| Gate-3 status | **DEFERRED, NOT REVERSED.** `reason` is still deleted — in M2.5, where the activation height exists to make it safe. |
| New milestone **M2.5** | The payload change, done safely: an **explicit** version discriminator (not length-based, not try-then-fallback), emission of the new shape gated behind AH #22, old shape decodable forever. Also fixes AUDIT-P1-002. |
| Order | M1 → M2 (AH #22) → **M2.5** (payload) → M3 (enforcement) → M4 (locks). M3 cannot precede M2.5: enforcement needs the field to exist. |

**AUDIT-P1-002 (P1, INTRODUCED) — carried into M2.5.** `new_add_maintainer`
(`crates/core/src/transaction/core.rs:774-787`) takes no `valid_before`, and the RPC drops it on the add
branch. The **add** direction is the one that seats a key, and it is precisely the R6 replay the expiry
exists to close — so as built the accepted residual is **UNBOUNDED for `add`**, worse than gate decision 2
documents. M2.5 MUST thread `valid_before` through every add submission surface, and M4 must lock it.

**AUDIT-P0-011 is NOT closed by M1.** `signing_message_at` has zero production callers; M1 is a
precondition only. The release-signature collision closes at M2 when the constructor is wired in.

**Process correction adopted:** this spec's own re-verification instruction is now a HARD PRECONDITION —
re-run the live-chain scan for maintainer transactions on the target network immediately before any
milestone that touches the payload, and record the height and txid in the milestone report.

### M2 PREREQUISITE — REV-176-M1a-001 (P1, architecture) — raised by the M1a reviewer

**AH #22 must ALSO be ordered against AH #20, not only against AH #21.**

M2 pairs **two** activation heights on a single authorization decision: **#22**
(`inc_i_176_auth_binding_activation_height`) selects which *message* is signed, while **#20**
(`maintainer_derivation_activation_height`, INC-I-172) selects which *verifier* runs. The gate-added
constraint above orders #22 only against **#21** (`inc_i_173_activation_height`) and is **silent on #20**.

If **#22 < #20**, the new message form becomes active while the pre-INC-I-172 verifier is still selected,
**re-arming the AUDIT-P1-016 entry-counting verifier**. That is a security regression created purely by
height ordering.

**Binding constraint, to be honoured when M2 pins any height:**

```
AH #22  >=  AH #20   (maintainer_derivation_activation_height)   — verifier must already be the fail-closed one
AH #22  <=  AH #21   (inc_i_173_activation_height)               — binding live before governance txs are mineable
```

On mainnet all three are currently `u64::MAX`, so the ordering is free to establish. On testnet #21 is
already crossed at `136_431` — the M2 pin must therefore satisfy the #20 bound and be placed above the live
tip, and the residual (testnet already has an unbound authorization at h=136690) is accepted, testnet being
local-only and unreachable from the internet.

M2 MUST NOT pin a height until this ordering is checked against the live values of #20 and #21 on the
target network.

### GATE-ADDED HARD CONSTRAINT — activation-height ordering (mainnet)

**On mainnet, `inc_i_176_auth_binding_activation_height` MUST be pinned at or below
`inc_i_173_activation_height`.**

Rationale, measured: mainnet `inc_i_173_activation_height = u64::MAX`
(`crates/core/src/network_params/defaults.rs:275`) ⇒ `AddMaintainer`/`RemoveMaintainer` are unmineable on
mainnet today, so **no unbound authorization can exist on mainnet before the INC-I-173 gate opens**. Both
gates are unpinned. Pinning #22 at or below #21 makes the exploitable interval `[AH_173, AH_176)` **empty by
construction**, so an attacker's advance knowledge of the published heights yields nothing. Pinning #21
first and #22 later would manufacture exactly the window this incident is about.

This constraint binds the M4 / mainnet-rollout step, not M1–M3 (testnet pins #22 above the live tip; the
INC-I-173 testnet gate is already crossed at 136_431, which is accepted — testnet is local-only and
unreachable from the internet).

### GATE-ADDED GUIDANCE — `valid_before` window sizing

The window is signer-chosen per authorization; it is **not** a global parameter. `SLOT_DURATION = 10`
seconds (`crates/core/src/consensus/constants.rs:169`) ⇒ 360 blocks/hour, 8_640 blocks/day.

- **INC-I-175 rotation: ~2 days (~17_280 blocks).** The margin is nearly free in that era — the adversary
  holds the exposed private keys and can sign fresh, so a replayed blob grants them nothing they lack. The
  cost being hedged is the real one: an authorization expiring mid-ceremony forces a repeat of the 3-of-5
  offline signing while the compromised keys still hold authority.
- **Routine changes after rotation: ~1 hour (~360 blocks).** Post-rotation the adversary holds no keys, so
  replay becomes the live threat and the window should be minimal.

Document both figures in the operator runbook shipped with M4.

---

## Options for User Decision

Each is a genuine decision the synthesis cannot make for you. Presented with evidence and the residual each
buys or leaves.

### Option 1 — Fatal deterministic-split refinement (Restructurer P1). LOW-EVIDENCE for the "make it fatal" step; conf(0.65, observed) for the split itself.
**What:** move the *deterministic* terms (domain tag, genesis binding, effect coverage) into the shared
validator as **FATAL** block rules gated on AH-176, keeping only the quorum/threshold and `valid_before`
non-fatal at `governance.rs`.
**Argument for:** these terms are pure functions of consensus-visible data; two honest same-version nodes
compute the same verdict, so a fatal reject over them cannot make them *disagree* (a fork). The Restructurer
frames the user's constraint as "no NONDETERMINISTIC reject," and by that reading this is a **refinement of
the non-fatality constraint, not a violation** — and the synthesis agrees it is genuinely safe *for
domain+effect-coverage* against honest same-version nodes.
**Why it is an option, not a default:** (a) the user framed non-fatality as a **requirement**, so adopting
it must be an explicit user choice; (b) it creates a new fatal reject path, which arms **V7** — an
un-upgraded binary that cannot decode the new payload turns a silent skip into a **block reject during
rolling deploy** (Failure Analyst, CRITICAL-during-deploy); (c) genesis-fatal additionally requires the
**D3 prerequisite** (`ctx.params.genesis_hash` diverges between builder and apply on chainspec networks,
`consensus/params.rs:140`) to be fixed first, or it self-forks on the local testnet; (d) `valid_before`
**cannot** be fatal — it fails Failure Analyst **F3** (the unbounded ~10⁵-block admission→apply skew turns
a fatal window predicate into a free block-poison). **What it buys:** rejection at mempool admission, so a
replayed/expired auth never occupies block space — a marginal benefit for a rare, fee-exempt, 0-in/0-out tx
that fails harmlessly at apply anyway under the recommended design.
**Recommendation:** do not adopt by default. The benefit is small; the V7 deploy-split hazard and the D3
prerequisite are real. If you want admission-time rejection, adopt the *hybrid* (domain+effect fatal,
genesis+expiry+quorum non-fatal) and accept the F4 three-path test + F9/F10 synchronized-deploy discipline.

### Option 2 — REQ-176-010 must be RELAXED, or the seen-set (REQ-176-032) must be built. conf(0.80, converged) that no proposal meets it as written.
**The relaxation, stated explicitly:** REQ-176-010 ("an authorization can never take effect a second time,
at any height, regardless of intervening state changes") **must be RELAXED to:** *"an authorization is valid
only within the signer-declared window `[tip_at_signing, valid_before)` and is permanently inert
thereafter."* The recommended design converts "valid forever" into "valid for a period the signing quorum
itself declared." **Residual attack that survives (exactly one):** R6 *inside* the window — add F1 with
`valid_before=T`; quorum revokes F1 at H<T; a stranger replays the original `add:F1` at H'<T; F1 is
reinstated. The window is the maximum latency at which a revocation can be undone.
**Full closure needs** a bounded seen-set of applied authorization digests, pruned at `valid_before`, that
**must rewind with the INC-I-174 undo pipeline** (REQ-176-032). Cost: node-local persistent state that
plausibly collides with REQ-176-003 (UndoData bincode byte-frozen), + the divergence class this whole
incident chain has fought if the rewind is imperfect, + `maintainer_rewind/` (997 lines) re-entered.
**Recommendation:** RELAX REQ-176-010 to bounded-validity for this milestone and **defer** the seen-set
(REQ-176-032). The residual (R6-inside-window) is naturally anti-correlated with the danger window during
the rotation (replay buys the adversary nothing while he already holds equal authority; it only matters
after control passes, by which time the window should be closing). If you judge REQ-176-010 non-negotiable
as written, the seen-set is mandatory and this becomes a larger, state-touching change.

### Option 3 — Preserve `reason` for off-chain transparency. conf(0.60, observed).
No evaluator found a *code* reader; none could rule out a block-explorer or governance-audit-trail reader.
**If you confirm `reason` is load-bearing off-chain:** keep it and place it *inside* the signed preimage
(F1 already commits every field), at +~250 signed bytes and re-derivation of the 873-byte cap. Security is
unaffected either way. **Default recommendation:** delete (F2), pending your answer to analyst §8.3.

---

## Filter Matrix (every proposal × every MUST filter)

Failure Analyst filters F1–F12 (MUST), F13–F16 (SHOULD). Applied to the **Recommended design** (R) and the
**Fatal-split option** (Opt-1).

| Filter | Recommended (SSF+hardened) | Fatal-split (Option 1) |
|---|---|---|
| **F1** quorum not `Err`-capable from shared validator | **PASS** — quorum stays non-fatal at `governance.rs` | **PASS** — quorum stays non-fatal (only deterministic terms move) |
| **F2** per-term consensus-vs-node-local; fatal only on consensus data | **PASS** — no new fatal path; all new terms non-fatal | **PASS for domain+effect** (consensus data); **genesis needs D3/P2 fix** to be builder/apply-deterministic |
| **F3** no window predicate on `current_height` in a fork-capable path | **PASS** — `valid_before` non-fatal, evaluated once at block height | **FAIL for `valid_before`** if made fatal (unbounded skew → block-poison). Salvage: keep `valid_before` non-fatal |
| **F4** shared-validator check locked by 3-path parity test | **N-A** (no check added to shared validator) — parity lock-test still added to prove no new fatal path (M4) | **REQUIRED** — mandatory 3-path test burden |
| **F5** no txid-uniqueness reliance unless order+reason closed | **PASS** — no txid reliance; both vectors closed anyway (F2) | **PASS** — same |
| **F6** state fate of tx admitted below AH, mined above; not "block rejected" | **PASS** — non-fatal, so the block is never rejected; the apply-height branch selects legacy/new; expired auth silently skips | **RISK** — fatal placement risks "block rejected" for a straddling tx unless `valid_before` kept non-fatal |
| **F7** no adversary-advanceable term | **PASS** — `valid_before` signer-chosen; no nonce/set-version | **PASS** — same |
| **F8** new AH field #22, own field, mainnet `u64::MAX`, constant gate | **PASS** — `inc_i_176_auth_binding_activation_height` | **PASS** — same |
| **F9** deploy shape + upgrade-lag for ~30 external producers | **PASS** — synchronized deploy; below-AH old binaries silently skip; fleet upgraded before first maintainer tx | **HARDER** — fatal path makes lag a *block-reject* split, not a silent skip |
| **F10** no new-format tx below AH; encoding changed ⇒ synchronized deploy | **PASS** — stated as operational constraint (INV-8) | **PASS** — same, and more load-bearing |
| **F11** no dual-accept window | **PASS** — one rule per height (legacy below AH, new above); old-format rejected above AH | **PASS** — same |
| **F12** sequencing vs INC-I-175 rotation stated | **PASS** — AH-176 pinned+crossed BEFORE the rotation | **PASS** — same |
| **F13** distinct log token binding-vs-counting (SHOULD) | **PASS** — AUTH_EXPIRED token added | **PASS** |
| **F14** update `derivation.rs` in lockstep or state unwired (SHOULD) | **PASS** — call sites `:190,203` updated in M1 | **PASS** |
| **F15** state post-rotation testnet/mainnet key-array identity (SHOULD) | **PASS** — genesis binding closes cross-network regardless; rehearsal-mints-mainnet-blobs risk flagged | **PASS** |
| **F16** don't claim to fix front-running/T3/P-4 (SHOULD) | **PASS** — explicitly not claimed | **PASS** |

**Verdict:** the Recommended design passes every MUST filter. The Fatal-split option FAILS F3 for its
`valid_before` row and is HARDER on F4/F6/F9 — it enters consideration only in the salvaged hybrid form
(deterministic terms fatal, `valid_before` non-fatal) and only as an explicit user choice, because the user
framed non-fatality as a requirement.

---

## INV-12 Classification (mandatory — must appear in the commit message)

**Three-question consensus-shape checklist (INC-I-075, INV-12):**
1. **Can a user-submittable tx reach this path?** **YES.** `submitMaintainerChange`
   (`crates/rpc/src/methods/governance.rs:214-286`, unauthenticated) + gossip
   (`validation_checks.rs:944-956`) both admit these transactions.
2. **Can a producer-action or attestation pattern reach it?** **YES.** The builder includes it
   (`production/assembly.rs:242`); every node applies it (`apply_block/governance.rs:32,68`).
3. **Is the new behavior bit-identical for ALL reachable inputs?** **NO.** Above AH-176 the signed message
   and the payload encoding change; below the gate the legacy path is bit-identical (and the reachable
   below-gate input set is currently empty).

**(1|2) YES + (3) NO ⇒ activation height REQUIRED.** New field #22, forward-only, own height.

**Both deploy questions:**
- **Consensus RULES change?** The *authorization* rule changes (what a valid signature commits to). Enforced
  **non-fatally**, so it does not change block validity — but it changes trust-root outcomes and is gated by
  AH-176 (INV-AUTH-001: the moment an authorization outcome is read by a consensus-adjacent rule, gate it).
  **⇒ activation height required (satisfied by #22).**
- **Block CONTENT change?** **YES** — the `MaintainerChangeData` payload encoding changes (`reason` removed,
  `valid_before` added; signature order canonicalized). **⇒ synchronized deploy (INV-8): stop ALL, deploy
  ALL, start ALL, before any maintainer tx is broadcast.** Old binaries return `None` on `from_bytes` and
  silently skip (`data.rs:57-59` → `governance.rs:34,70`) — a divergent-effect, not a divergent-validity,
  failure, which is exactly why the synchronized deploy is mandatory.

**No version bumps.** `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, and crate versions are
untouched (no EpochState serialization change; no peer-handshake change). No `HardForkSchedule` entry.

---

## Migration & Compatibility

- **Legacy authorizations above AH-176:** **REJECTED** (they verify under the new message, which they do not
  match). **No dual-accept window** (Failure Analyst F11) — a window that accepts old *or* new above the AH
  leaves every archived blob valid for its duration, voiding the entire fix. The migration is: upgrade all
  signers → cross the AH → reject old.
- **New-format authorizations below AH-176:** must **not be broadcast** (Failure Analyst F10). Below the gate
  the legacy message is used; a new-format tx below the AH would be verified under the old rule (binding
  ignored) or fail to decode on old binaries (silent skip).
- **Straddle window:** a tx admitted below AH and mined above it is **not** rejected (the enforcement is
  non-fatal); the apply-height branch selects the message form, and an expired auth silently skips. No
  block-poison — the recommended design's decisive advantage over any fatal placement.
- **Existing signers that break** (all must migrate before the synchronized deploy):
  - Node CLI signer `bins/node/src/commands/maintainer.rs:157-158,224-225` — signs **raw** bytes today; must
    switch to the new BLAKE3 construction via the shared constructor (and resolve the raw-vs-hash mismatch,
    Radical Gap 7 — the CLI signs raw, `PriceAttestationData` signs a hash; standardize on the digest).
  - 14 test files across two roots that construct `MaintainerChangeData` (analyst §4.3) — re-derived.
  - The **out-of-repo** `sign_maintainer.py` harness — **breaks silently**. Mitigation: ship an in-repo
    `doli-node maintainer sign` command (REQ-176-030) + a published golden vector; retire the Python harness.
    This cannot be enforced on an operator laptop we cannot see — it is a deploy-communication risk.
- **Bridge fixes (transitional, labeled — belong here, not in Definite Changes):**
  - **BRIDGE: fleet-uniform deploy before the first maintainer tx.** The AH bounds divergence among *new
    binaries only*; the real control during the transition is that no maintainer tx exists yet and none is
    broadcast until the whole fleet runs the new binary. Removed as load-bearing once AH-176 is crossed.
- **Non-regression (verified against):** INC-I-173 (governance txs stay mineable — the fee gate is untouched;
  the tx stays 0-in/0-out and fee-exempt), INC-I-174 (maintainer state rewinds with the chain — the payload
  swap uses the same `cf_undo` key family; **largest unverified dependency**, see Test Plan M4), INC-I-172
  (fail-closed TrustRoot stays usable — a frozen set is still `is_usable`; governance death ≠ update death,
  Failure Analyst B7).

---

## Architecture Maps

### Current
```
SIGNERS (4 sites, 1 out-of-repo, no contract)
   │  "add:<hex>" | "remove:<hex>"   ← no domain, no genesis, no expiry
   ▼
RPC submitMaintainerChange (unauth) ─┐         gossip (unauth)
   ▼                                 ▼
tx_types.rs:753  STRUCTURAL ONLY (shape + reason-cap)  ── mempool/builder/apply; FATAL at apply
   ▼
governance.rs:39,75  verify_multisig_at  ── THE ONLY AUTHORITY CHECK; Option-return, warn!, NON-FATAL
   ▼
maintainer_state.bin  "unsigned and attacker-writable"; NODE-LOCAL, not in state root
```

### Proposed (Definite + Recommended)
```
SIGNERS → one constructor: crates/core/src/maintainer/authmsg.rs (golden-vectored)
   │  BLAKE3(DOMAIN ‖ genesis_hash ‖ action ‖ target ‖ valid_before)   ← every field signed
   ▼
RPC / gossip  (unchanged; RPC-spam flagged as a separate incident)
   ▼
tx_types.rs:753  STRUCTURAL ONLY, minus the reason-cap  ── unchanged fatality; nothing added
   ▼
governance.rs:32-102  [gate #22] legacy-or-new message · height>=valid_before → AUTH_EXPIRED · verify_multisig_at
   │     NON-FATAL (unchanged); genesis_hash already in scope at :29
   ▼
maintainer_state.bin  (unchanged lifecycle; INC-I-174 rewind intact)
```

---

## Milestone Plan

The design touches ~9 production files across 4 modules ⇒ milestones. Each is independently testable on the
**local** testnet (`~/testnet/`, `scripts/testnet.sh`, 127.0.0.1). **The D3 parity gap is NOT a prerequisite
for the recommended (non-fatal) design** — with a single apply-only evaluation site there is no builder/apply
divergence to reconcile; genesis is sourced from `governance.rs:29` where it already lives. D3/P2 becomes a
hard prerequisite milestone **only if the user adopts Option 1 (fatal split)**.

- **M1 — Signed-message construction + subtraction (crates/core).** New `maintainer/authmsg.rs` constructor
  (domain tag + genesis + action + target + valid_before) with a `_legacy` fallback; swap `reason` →
  `valid_before` in `MaintainerChangeData`; canonicalize signature order; delete the reason cap in
  `tx_types.rs:817-822` and `MAX_MAINTAINER_CHANGE_REASON_BYTES`; update `derivation.rs:190,203` in lockstep
  (F14). **Delivers:** the new bytes + the smaller payload, all behind the (not-yet-added) gate.
  **Verified:** golden-vector test pins the exact BLAKE3 preimage; payload-bound test re-derived in the
  *safe* direction (max payload shrinks); unit round-trip. No consensus behavior change yet.
- **M2 — Activation height #22 + gate wiring.** Add `inc_i_176_auth_binding_activation_height` to
  `network_params/` (mainnet `u64::MAX`, devnet `0`, testnet above tip at pin time); wire the legacy-vs-new
  selection at `governance.rs` via the `set.rs:262-274` dispatcher idiom. **Verified:** gate-selection test
  (below-gate → legacy bytes bit-identical; above-gate → new bytes) on a local devnet with AH=0.
- **M3 — Non-fatal expiry enforcement + observability.** Add `height >= AH && height >= valid_before` →
  `warn!("[MAINTAINER] Rejected …: AUTH_EXPIRED …")` at both arms (F13). **Verified:** end-to-end on the
  local testnet — an `add:X` replayed *after* `valid_before` fails to reinstate X (R6-outside-window closed);
  an in-window replay still reinstates X (residual documented, Option 2).
- **M4 — Regression locks + in-repo signer.** Three-path lock-test proving no new fatal reject path was added
  (mempool/builder/apply agree, no BLOCK_POISON); flip
  `updater/tests/inc_i_172_m2_release_sign_arg_validation.rs::the_collision_still_exists_and_only_m3_closes_it`
  to assert **NO collision**; effect-coverage lock-test (REQ-176-012); INC-I-173/174/172 non-regression
  suite incl. the maintainer-rewind encoding check (the load-bearing unverified dependency); ship
  `doli-node maintainer sign` + published golden vector, retire `sign_maintainer.py`. **Verified:** full
  `cargo test` + local testnet gauntlet (`scripts/gauntlet.sh`).

---

## Test Plan

- **Golden vector (M1):** pin the exact BLAKE3 preimage byte layout — the encoder/decoder-parity discipline
  the Full Bitfield Decode pillar mandates.
- **Below-gate bit-identity (M2):** assert the legacy message and payload are byte-identical below AH-176
  (holds vacuously today; the test makes the vacuity explicit and future-proof).
- **Three-path parity / no-new-fatal-path (M4, INV-VALIDATION-001):** drive the SAME maintainer tx through
  mempool admission, builder, and apply; assert none produces a fatal reject the others do not — locking the
  claim that the recommended design adds no fork-capable path.
- **Cross-protocol collision regression (M4):** the `updater` test must flip from
  "collision still exists" to "NO collision" — the executable proof that INC-I-176 is the M3 domain
  separation the house already scheduled.
- **Replay closure (M3):** R6-outside-window fails; R6-inside-window still succeeds (documents the residual).
- **Cross-network (M4):** a testnet-genesis authorization must FAIL on mainnet-genesis verification — the
  named test REQ-176-011 requires (byte-identity of key arrays is no longer sufficient).
- **INC-I-174 rewind (M4):** apply a maintainer change, reorg past it, assert the set rewinds and the
  `cf_undo` encoding is unchanged — the design's single largest unverified dependency (Radical Gap 3).

---

## What This Design Does NOT Fix (explicit)

1. **REQ-176-010 (absolute single-use) — RELAXED, not met.** R6 *inside* the signer-declared window
   survives. Full closure = the deferred seen-set (REQ-176-032, Option 2).
2. **Unauthenticated fee-exempt RPC/gossip spam.** `submitMaintainerChange` authenticates no caller and
   checks only `signatures.len() >= 3`; above `inc_i_173_activation_height` anyone can have arbitrary
   873-byte (soon ~625-byte) maintainer txs with garbage signatures mined for free and stored forever
   (Radical §9.1, Failure Analyst A5 — severity UNSCORED, mainnet RPC reachability unverified). **NEW
   INCIDENT CANDIDATE — possibly larger than INC-I-176 itself.**
3. **`ProtocolActivationData::signing_message`** (`data.rs:99-105`, `"activate:{version}:{epoch}"`) has the
   **identical defect** and is verified at the same site. Out of scope per analyst §7.6; P1+P3 port to it
   verbatim. **NEW INCIDENT CANDIDATE.**
4. **`Condition::TimelockExpiry` does not expire** (`crates/core/src/conditions/eval.rs:81` evaluates `>=`,
   identical to `Timelock`; two doc sites claim `<=`). DOLI has **no working expiry primitive to reuse**, and
   covenants are mainnet-live since h=9150. Latent, unrelated to INC-I-176. **NEW INCIDENT CANDIDATE — do not
   silently fix here** (CLAUDE.md rule 5).
5. **`maintainer_state.bin` node-local / attacker-writable (T3), front-running a removal, and the
   `rm maintainer_state.bin` re-seed hole (P-4).** All outside INC-I-176; claiming them hides real exposure
   (Failure Analyst F16).
6. **`Node::new_for_replay` applies zero governance** (`init.rs:1493` sets `maintainer_state: None`) —
   rebuild-from-genesis skips every Add/RemoveMaintainer; a standing divergence generator and a plausible
   mechanism for the n15/n17/n18 divergence. Independent of INC-I-176. **NEW INCIDENT CANDIDATE.**

---

## Complexity Comparison

| Metric | Current | Radical Minimum (SSF) | Recommended (SSF + hardened) | Option 1 (fatal split) |
|--------|---------|----------------------|------------------------------|------------------------|
| Modules touched | — | 4 (+2 edge) | 4 (+2 edge) | 6–8 |
| Production files | — | 9 | ~10 (+`authmsg.rs`) | 15–20 |
| New modules | — | 0 | 1 leaf (`authmsg.rs`) | 1–2 |
| New abstractions | — | 0 | 0 | 1–2 |
| Signed fields | 2 | 5 | 5 | 6–8 |
| Payload fields | 3 | 3 | 3 | 4–5 |
| Max payload | 873 B | 616 B (measured, M1) | 616 B (measured, M1) | ~900–950 B |
| Verification sites | 1 (non-fatal) | 1 (non-fatal) | 1 (non-fatal) | 3–4 (fatal) |
| New fatal reject paths | — | 0 | 0 | ≥1 |
| New activation heights | — | 1 (#22) | 1 (#22) | 1 (#22) |
| INV-VALIDATION-001 3-path parity | N/A | not triggered | not triggered (lock-test only) | mandatory |
| Production LoC | — | ≈130–160 | ≈210–240 | 400–700 |

**Is the recommendation within 0.1 confidence of the SSF candidate? YES.** The security-critical content is
identical — the domain tag already closes the cross-protocol collision, genesis closes cross-network, expiry
closes R6-outside-window, and deleting `reason` closes malleability vector 1. The delta (the owned
`authmsg.rs` constructor + golden vector, canonical signature order, the AUTH_EXPIRED log token) is
maintainability + ~~closing the *second* malleability vector at zero signed bytes~~ construction-time
encoding normalization + observability. Same
architecture, 1 leaf file, 0 new fatal paths. **The user gate presents the SSF ALONE first;** the delta buys
a durable signer/verifier contract (retiring the silent-drift Python harness), ~~the second malleability
vector~~ one canonical encoding per honest signer set, and binding-vs-counting log disambiguation during a
time-critical rotation.

> **AMENDED 2026-08-12 (INC-I-176 M1a, QA [F4]) — struck clauses above.** The canonical signature order was
> credited here with "closing the *second* malleability vector at zero signed bytes". That credit is
> **WITHDRAWN**: per security audit **F3** the sort is not an adversarial control, because `sendTransaction`
> accepts any signature ordering off the wire — a construction-time sort constrains honest builders only. It
> buys deterministic encoding, nothing adversarial. It is also **DEFERRED to milestone M2.5** together with
> the payload swap, so **no shipped M1a code makes or depends on this claim**; `MaintainerChangeData` moves
> zero bytes in M1a. Read the `Max payload` row of the table above (`616 B (measured, M1)`) the same way: it
> is an **M2.5 target**, not a shipped measurement — M1a's payload is still the unchanged 873 B shape.
> Likewise "deleting `reason` closes malleability vector 1" describes the M2.5 end state, not M1a.

---

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     1 (delete `reason`, 5/5 agreement)
Restructuring convergence:      2 (single non-fatal site 4/5; owned constructor 3/5)
Addition options presented:     3 (fatal-split, seen-set/REQ-176-010, reason-preservation)
Failure modes identified:       16 (F1–F16 from the Failure Analyst)
Failure modes applied as filters: 16/16 (12 MUST + 4 SHOULD), against 2 designs
Radical floor gap:              current → SSF (≈130–160 LoC) → recommended (≈210–240 LoC); within 0.1 conf
Contradictions found:           3 (expiry skew; fatality/placement; REQ-176-010 single-use)
Contradictions resolved:        3/3 (see architecture-reasoning.md)
Evidence independence verified: YES (delete-`reason` from 3 independent censuses w/ positive controls)
Recurrence flag:                ABSENT (no prior redesign; <2 prior incidents this domain) — prior-failure section omitted
Unverified (no evaluator ran a build or test): golden encoding size, INC-I-174 rewind interaction, mainnet RPC reachability, post-rotation testnet key plan, live re-scan for existing maintainer txs
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
