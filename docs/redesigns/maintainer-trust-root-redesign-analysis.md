# INC-I-172 — Maintainer / Update-Signing Trust Root: Problem-Scoping Analysis

> **Status**: problem scoping only. This document is NOT a design. It is the verified input for
> parallel design evaluation. Every factual claim below carries `file:line` evidence.
> **Analyst**: OMEGA analyst agent, `/omega-redesign` chain.
> **Date**: 2026-08-09
> **Incident**: INC-I-172 (open, severity high, domain `updater/governance`)
> **Related**: INC-I-170 (key exposure), INC-I-171 (unenforced vesting penalty), INC-I-157 (release origin)

---

## 1. SSF candidate (Rule 18 baseline — evaluators must beat or confirm this)

> **The simplest mechanism that resolves the root cause**: change `maintainer_keys_fn`
> (`bins/node/src/run.rs:461`) to return the hex-encoded members of the already-maintained on-chain
> `MaintainerSet` instead of `Vec::new()`, so release-signature verification reads a trust root that
> `AddMaintainer` / `RemoveMaintainer` transactions can already rotate.
>
> **Why it works**: the rotation machinery already exists and is already exercised
> (`bins/node/src/node/apply_block/governance.rs:17-77`), the keys it holds are already raw Ed25519
> (`crates/core/src/maintainer.rs:64`), and the updater already accepts a dynamic key list
> (`crates/updater/src/verification.rs:57-79`). Only the wire between them is missing. It is a
> node-local binary change: no consensus rule change, no block-content change, no activation height,
> no genesis reset.

**The SSF candidate is necessary but demonstrably NOT sufficient.** Three verified facts constrain it,
and the evaluators must design against all three:

1. **It changes nothing about *who* is trusted today.** The on-chain set is bootstrapped from the
   first 5 registered producers (`bins/node/src/node/periodic.rs:52-75`), which are exactly N1–N5 —
   the keys whose private halves are in the public repo (§6, F1). Wiring makes the set *rotatable*;
   it does not make it *uncompromised*.
2. **Authorization for rotation is the compromised quorum itself.** `AddMaintainer` needs 3 valid
   signatures from current maintainers (`maintainer.rs:145-159`, threshold 3 of 5 at
   `maintainer.rs:125-135`). An attacker holding the leaked keys can front-run, or add their own key
   and then remove the honest ones.
3. **First delivery rides the compromised channel.** Any node-side change ships as a binary through
   the same GitHub release path the attacker can sign for (`crates/updater/src/constants.rs:132-138`,
   `crates/updater/src/download.rs:238-304`).

**Root-cause discipline check**: the SSF candidate addresses the *stated* root cause ("trust root is a
compile-time constant"). It does **not** address the second half of the incident's root cause record
("producer and maintainer roles share a single key") and it does not address the live compromise. A
design that only wires the updater is a partial fix, not a symptom patch — but it must be shipped
together with an answer to (2) and (3) or it is inert.

---

## 2. Verified-claims table

Every claim in `docs/bugfixes/inc-i-172-maintainer-key-rotation-architect-prompt.md` was re-checked
against code. **5 CORRECTIONS. Corrections C12, C13, C14 materially change the problem.**

| # | Claim as stated in the brief | Verdict | Evidence |
|---|---|---|---|
| C1 | `run.rs:461` builds `maintainer_keys_fn = move \|\| Vec::new()` (always empty, intentional; comment at 456-459) | **CONFIRMED** | `bins/node/src/run.rs:456-461` — comment lines 457-459, closure line 461, verbatim `let maintainer_keys_fn = move \|\| -> Vec<String> { Vec::new() };` |
| C2 | `updater/service.rs:221-222` passes that empty list to `verify_release_signatures_with_keys` | **CONFIRMED** | `bins/node/src/updater/service.rs:221-222` |
| C3 | `verification.rs:66-78` — empty list falls back to `bootstrap_maintainer_keys(network)` | **CONFIRMED** | `crates/updater/src/verification.rs:66-79` |
| C4 | `REQUIRED_SIGNATURES = 3` | **CONFIRMED** | `crates/updater/src/constants.rs:29` |
| C5 | `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` exists; comment states N1–N5 are dual-role producer+maintainer | **CONFIRMED** | `crates/updater/src/constants.rs:31-48` (comment 33-36, keys 37-48) |
| C6 | `AddMaintainer`/`RemoveMaintainer` "verify the 3-of-5 multisig and **reject** insufficient signatures" | **CORRECTED (partial)** | Signature check exists — `governance.rs:24` (`verify_multisig`) and `governance.rs:56` (`verify_multisig_excluding`). But a failed check only emits `warn!` and skips the state mutation (`governance.rs:39-41`, `72-74`); the **transaction is not rejected and the block is still valid**. Consensus-level validation is **structural only** — no inputs, no outputs, deserializable `extra_data` (`crates/core/src/validation/tx_types.rs:739-778`), which itself documents at 770-776 that signature and set-state checks are deferred "to the node level". |
| C7 | `MaintainerState` applied immediately, persisted to the data dir, **not** part of the state root | **CONFIRMED** | `crates/storage/src/maintainer.rs:18` (`maintainer_state.bin`), `:38-56` (load/save to `data_dir`). No state-root inclusion found. |
| C8 | The on-chain set is consumed for `ProtocolActivation` verification, self-governance, and RPC — not for release verification | **CONFIRMED** | `governance.rs:80-106` (ProtocolActivation), `governance.rs:17-77` (self-governance), `crates/rpc/src/methods/governance.rs:74` + `crates/rpc/src/methods/context.rs:78` (RPC). No updater consumer — C1/C2 prove the gap. |
| C9 | `MaintainerSet.members: Vec<PublicKey>` — raw Ed25519 | **CONFIRMED** | `crates/core/src/maintainer.rs:62-69` |
| C10 | The set is "derived deterministically by replaying the blockchain" | **CORRECTED** | `derive_maintainer_set()` (`crates/core/src/maintainer.rs:490-531`) and the `BlockchainReader` trait (`:461-470`) have **zero production callers** — the only non-test reference is the re-export at `crates/core/src/lib.rs:258`. The live path is `maybe_bootstrap_maintainer_set()` (`bins/node/src/node/periodic.rs:35-92`), which reads the **in-memory `ProducerSet`**, takes the first 5 by `registered_at`, and caches to disk. Incremental `Add`/`Remove` mutate the cached set in place (`governance.rs:25`, `:58`). **There is no replay-from-genesis derivation in production.** |
| C11 | `VETO_THRESHOLD_PERCENT = 40` | **CONFIRMED** | `crates/updater/src/constants.rs:26` |
| C12 | The malicious release is "gated by a **7-day** veto" | **CORRECTED — the veto window is 5 MINUTES** | `crates/updater/src/constants.rs:12-13`: `pub const VETO_PERIOD: Duration = Duration::from_secs(5 * 60);` with the comment "5 minutes for ALL updates (early network; target: 7 days)". Mainnet params agree: `crates/updater/src/params.rs:28-29` doctest asserts `UpdateParams::for_network(Network::Mainnet).veto_period_secs == 5 * 60`. The "7 days" appears only as hardcoded prose: `bins/node/src/updater/service.rs:227` (log literal), `crates/updater/src/vote.rs:3` (doc), `crates/updater/src/apply.rs:69` (doc), `crates/network/src/gossip/staleness.rs:85` (comment). **All four are false.** |
| C13 | The veto is "**seniority-weighted**" | **CORRECTED — the live path is an UNWEIGHTED HEAD COUNT** | Live tally: `bins/node/src/updater/service.rs:321` calls `calculate_veto_result(p.vote_tracker.veto_count(), total)` → `crates/updater/src/verification.rs:149-164`, which is `veto_count * 100 / total_producers`, approved iff `< 40`. The weighted functions `VoteTracker::should_reject_weighted` / `veto_weight` / `with_weights` / `set_weights` (`crates/updater/src/vote.rs:126-233`) have callers **only in test modules** (`crates/updater/src/lib.rs` test mod at :224-231, `vote.rs` test mod at :287-357). The storage-side `ProducerSet::has_weighted_veto` / `has_weighted_veto_for_network` (`crates/storage/src/producer/set_governance.rs:112,128`) likewise has callers **only** in `crates/storage/src/producer/tests.rs:330,333,693,696,905,908,911,915`. `UpdateParams::calculate_vote_weight` (`params.rs:95-101`) is used only by tests. **Seniority weighting is dead code.** |
| C14 | The `run.rs:457-459` TODO's stated blocker: "on-chain `ProducerInfo` stores BLAKE3 pubkey hashes, not raw Ed25519 keys, so `MaintainerState` members can't be used for signature verification" | **CORRECTED — THE STATED BLOCKER IS FALSE** | `ProducerInfo.public_key: PublicKey` is a **raw Ed25519 public key** (`crates/storage/src/producer/types.rs:72-74`). The BLAKE3 hash is only the `HashMap` **index key**, not the stored value (`crates/storage/src/producer/set_core.rs:159`, `:282-287`). Two production call sites already read raw keys straight out of the producer set: `bins/node/src/node/periodic.rs:63-68` (`.map(\|p\| p.public_key)`) and `bins/node/src/node/apply_block/governance.rs:118-122` (`.map(\|p\| p.public_key)`). **The reason the updater is left unwired is obsolete.** |
| C15 | `AddMaintainer` targets already carry raw `PublicKey`s ⇒ raw keys recoverable from block history | **CONFIRMED, and moot** | `MaintainerChangeData.target: PublicKey` (`crates/core/src/maintainer.rs:287-294`). Moot because C14 shows raw keys were never unavailable in the first place. |

---

## 3. Architecture comprehension

### 3.1 Data flow A — update verification (release manifest → verify → veto → install)

```
GitHub release  (crates/updater/src/constants.rs:132-138 — GITHUB_REPO / API / RELEASES origin;
                 INC-I-157 invariant: origin must be a namespace the project controls)
   │
   ▼  fetch_latest_release()                       crates/updater/src/download.rs:178-304
   │    • downloads CHECKSUMS.txt → checksums_sha256              download.rs:231-236
   │    • downloads SIGNATURES.json → Vec<MaintainerSignature>    download.rs:238-251
   │    • Release.binary_sha256 := checksums_sha256               download.rs:297
   ▼
   is_newer_version() gate                          service.rs:204-207
   ▼
   maintainer_keys_fn()  ──►  Vec::new()  ◄── THE GAP (run.rs:461)
   ▼
   verify_release_signatures_with_keys(release, &[], network)      service.rs:221-222
   │    • on_chain_keys empty ⇒ allowed_keys = bootstrap_maintainer_keys(network)
   │                                                verification.rs:66-79
   │    • message = "{version}:{binary_sha256}"     verification.rs:62
   │    • count valid Ed25519 sigs whose pubkey ∈ allowed_keys
   │                                                verification.rs:83-121
   │    • require valid_count >= REQUIRED_SIGNATURES (3)  verification.rs:123
   ▼
   PendingUpdate created, VoteTracker::new()        service.rs:233-241
   │    persisted to data_dir (pending_update.json) service.rs:247-249
   ▼
   VETO WINDOW = release.published_at + config.veto_period_secs   service.rs:311
   │    mainnet value = 300 s                       constants.rs:13 / params.rs:28-29
   │    votes arrive via gossip → on_new_vote()     bins/node/src/node/network_events.rs:503-527
   │      • Ed25519-verified at ingress             network_events.rs:508
   │      • forwarded to UpdateService via vote_tx  network_events.rs:525
   │      • handle_vote(): only checks is_producer_fn, then records
   │                                                service.rs:266-292
   ▼
   calculate_veto_result(veto_count, active_count)  service.rs:321  ← UNWEIGHTED COUNT
   │    approved iff veto_percent < 40              verification.rs:149-164
   ▼
   Approved → VersionEnforcement + GRACE_PERIOD (3600 s, constants.rs:16)  service.rs:344-356
   ▼
   auto_apply() → auto_apply_from_github(version, signed_checksums_sha256)  apply.rs:411
   │    • re-fetch and re-compare CHECKSUMS.txt hash vs signed hash (AUDIT-UPDATE-002 TOCTOU fix)
   │                                                apply.rs:418-436
   │    • verify_hash(tarball, expected_hash)       apply.rs:446
   ▼
   backup_current() → install → chmod → restart_node()
   ▼
   check_production_allowed() gates block production   enforcement.rs:170-205
        (does NOT block if !binary_ready, or after ENFORCEMENT_TIMEOUT_SECS = 30 min)
```

**Trust chain root** = the 5 hardcoded strings at `crates/updater/src/constants.rs:37-48`.
Everything downstream (CHECKSUMS.txt → per-platform hash → tarball → installed binary) is
cryptographically anchored to those 5 constants and nothing else.

### 3.2 Data flow B — maintainer governance (tx → apply_block → MaintainerState)

```
Bootstrap (once):  maybe_bootstrap_maintainer_set(height)      periodic.rs:35-92
   • guard: already fully bootstrapped (>= 5 members)?         periodic.rs:44-49
   • read ProducerSet, require >= 5 producers                  periodic.rs:52-56
   • sort by registered_at, take 5, map to p.public_key        periodic.rs:58-64
   • MaintainerSet::with_members(keys, height)                 periodic.rs:70-71
   • persist maintainer_state.bin                              periodic.rs:76-79

Mutation:  Transaction (TxType::AddMaintainer = 12 | RemoveMaintainer = 11)
   ├─ consensus validation (STRUCTURAL ONLY)
   │     validate_maintainer_change_data()      crates/core/src/validation/tx_types.rs:739-778
   │     no inputs, no outputs, extra_data deserializes as MaintainerChangeData
   │     ⇒ a tx with ZERO valid signatures is a STRUCTURALLY VALID transaction
   └─ node-local apply       bins/node/src/node/apply_block/governance.rs:17-77
         • MaintainerChangeData::from_bytes(tx.extra_data)     governance.rs:20, :50
         • message = "add:{hex}" | "remove:{hex}"              maintainer.rs:323-326
         • Add:    set.verify_multisig(sigs, msg)              governance.rs:24
         • Remove: set.verify_multisig_excluding(sigs, msg, target)   governance.rs:56
         • on success: set.add_maintainer / remove_maintainer, persist to data_dir
         • on failure: warn! and continue — NO tx rejection     governance.rs:39-41, :72-74

Consumers of the resulting set:
   • ProtocolActivation verification (3-of-5)                  governance.rs:80-106
       fallback when not fully bootstrapped: derive_ad_hoc_maintainer_set(producers, height)
                                                               governance.rs:112-124
   • RPC exposure                                              crates/rpc/src/methods/governance.rs:74
   • Slashing removal path                                     maintainer.rs:242-251 (force_remove)
   • ✗ NOT the updater — this is INC-I-172
```

### 3.3 Consensus state vs node-local state (critical distinction for any design)

| State | Consensus (state root)? | Where | Consequence |
|---|---|---|---|
| `ChainState`, `UtxoSet`, `ProducerSet` | **YES** | `crates/storage/src/snapshot.rs` | Divergence = fork |
| `MaintainerSet` / `MaintainerState` | **NO** | `maintainer_state.bin` in the data dir (`crates/storage/src/maintainer.rs:18,38-56`) | Divergence = silent, per-node, undetected by state-root comparison |
| `PendingUpdate` / `VoteTracker` | **NO** | `pending_update.json` in the data dir (`service.rs:247`) | Each node tallies only the votes it personally received |
| Bootstrap maintainer keys | **NO — compile-time** | `crates/updater/src/constants.rs:37-67` | Rotation requires a new binary on every node |

**Consequence for design**: the maintainer set today is *node-local cached state derived from
consensus inputs*, not consensus state. It is not covered by the state root, so two nodes disagreeing
about who the maintainers are will **not** fork and **will not be detected**. Promoting it into the
state root would be a consensus change (activation height required); leaving it out means the design
must supply its own convergence argument.

### 3.4 Dependency map (graph-verified, grep-complemented)

`bootstrap_maintainer_keys` — 7 dependents (graph), all confirmed by read:
- `crates/updater/src/verification.rs:78` — the fallback that makes the constants the effective trust root
- `crates/updater/src/constants.rs:82` (`is_using_placeholder_keys`), `:92` (`assert_production_keys`), `:111` (`get_maintainer_keys`)
- `bins/node/src/node/startup.rs` (via `.run`), `bins/node/src/updater/service.rs:182`

`MaintainerSet` — 11 dependents (graph), all confirmed:
- `bins/node/src/node/apply_block/governance.rs:10,112` · `bins/node/src/node/mod.rs:192,468` ·
  `bins/node/src/node/periodic.rs:38-71` (graph-blind `self.method()` call — found by grep) ·
  `bins/node/src/node/startup.rs:514` · `crates/core/src/maintainer.rs:490` (dead) ·
  `crates/rpc/src/methods/context.rs:78` · `crates/rpc/src/methods/governance.rs:74` ·
  `crates/storage/src/maintainer.rs` · `bins/node/src/run.rs:453,460,516`

### 3.5 Blast radius

- **Direct** (any design touching the trust root): `crates/updater/{constants,verification,vote,apply,download,enforcement,params,types}.rs`, `bins/node/src/run.rs`, `bins/node/src/updater/service.rs`.
- **Direct** (any design touching the maintainer set): `crates/core/src/maintainer.rs`, `crates/storage/src/maintainer.rs`, `bins/node/src/node/{periodic,mod,startup}.rs`, `bins/node/src/node/apply_block/governance.rs`, `crates/rpc/src/methods/{governance,context}.rs`.
- **Indirect**: `ProtocolActivation` verification shares the same set — changing set semantics changes who can schedule consensus upgrades (`governance.rs:80-106`). Slashing force-removes maintainers (`maintainer.rs:242-251`), so a slash event mutates the update trust root as a side effect.
- **Indirect**: `bins/cli/src/cmd_chain.rs:406-609` treats `maintainer_state.bin` as a wipe-sensitive artifact; any format change interacts with the chain-reset path.
- **Operational**: ~20–30 external auto-update producers + N1–N12 + 3 seeds. No synchronized stop is possible.

---

## 4. Capability inventory (PRIOR-KNOWLEDGE-GATE)

Built from code before any "the system lacks X" claim.

**Transaction primitives** — `crates/core/src/transaction/types.rs`
- **24 declared `TxType` variants**: discriminants 0–22 contiguous, plus `ZKSettle = 31`.
- **7 permanently tombstoned discriminants**: 24–28 (native lending, `types.rs:91-108`), 29–30 (NFT fractionalization, `types.rs:109-124`). Discriminant 9 (`ClaimWithdrawal`) is a live-but-tombstoned wire-compat variant (`types.rs:26-27`). Discriminant 23 is unassigned.
- **3 variants touch maintainer governance**: `RemoveMaintainer = 11` (`types.rs:39`), `AddMaintainer = 12` (`types.rs:45`), `ProtocolActivation = 15` (`types.rs:58`). A 4th touches it indirectly: `SlashProducer = 5` (via `force_remove_maintainer`).

**Maintainer primitives** — `crates/core/src/maintainer.rs`
- **4 constants**: `INITIAL_MAINTAINER_COUNT = 5` (:38), `MAINTAINER_THRESHOLD = 3` (:41), `MIN_MAINTAINERS = 3` (:44), `MAX_MAINTAINERS = 5` (:47).
- **`MaintainerSet` — 14 public methods**: `new`, `with_members`, `is_maintainer`, `can_remove`, `can_add`, `member_count`, `calculate_threshold`, `verify_multisig`, `verify_multisig_excluding`, `add_maintainer`, `remove_maintainer`, `force_remove_maintainer`, `is_fully_bootstrapped`, `needs_bootstrap_member` (:79-261).
- **3 payload types**: `MaintainerSignature` (:266), `MaintainerChangeData` (:287), `ProtocolActivationData` (:346).
- **1 derivation trait + 1 derivation fn**, both with **zero production callers**: `BlockchainReader` (:461), `derive_maintainer_set` (:490).
- **7 error variants**: `MaintainerError` (:397-412).

**Updater primitives** — `crates/updater/src/`
- **4 public verification fns**: `sign_release_hash` (`verification.rs:27`), `verify_release_signatures` (:48), `verify_release_signatures_with_keys` (:57), `calculate_veto_result` (:149). Plus 1 private `verify_ed25519` (:171).
- **Vote layer**: `VoteMessage` (4 methods, `vote.rs:39-81`), `VoteTracker` (**14 methods**, `vote.rs:110-244`) — of which **6 weighted-voting methods are dead in production** (§C13).
- **Timing/threshold constants (6)**: `VETO_PERIOD` (constants.rs:13), `GRACE_PERIOD` (:16), `VETO_THRESHOLD_PERCENT` (:26), `REQUIRED_SIGNATURES` (:29), `CHECK_INTERVAL` (:116), `ENFORCEMENT_TIMEOUT_SECS` (enforcement.rs:64).
- **Key constants (2 arrays × 5 keys) + 4 accessor fns**: `constants.rs:37-113`.
- **Origin constants (3)**: `GITHUB_REPO`, `GITHUB_API_URL`, `GITHUB_RELEASES_URL` (`constants.rs:132-138`) — carry the INC-I-157 invariant.
- **`UpdateParams` — 11 network-aware fields + 10 methods** (`params.rs:32-136`), of which `calculate_vote_weight`, `seniority_multiplier`, `is_eligible_to_vote` have **zero production callers**.

**Forward-only activation primitives already available (2)**
1. `ProtocolActivation` tx — 3-of-5 maintainer multisig schedules a protocol version at a future epoch (`types.rs:54-58`, `governance.rs:80-106`, `maintainer.rs:346-393`).
2. Activation-height fields in `NetworkParams` — **67 `_activation_height` occurrences in `crates/core/src/network_params/defaults.rs`**, 33 in `mod.rs`, 59 in `env_loader.rs`.

**Conclusion of the inventory**: the system does **not** lack a rotation transaction, a multisig
verifier, a raw-key store, or a forward-only activation mechanism. It lacks exactly one wire
(`run.rs:461`) plus an authorization model that survives a compromised quorum.

---

## 5. Veto-math trace and verdict

**Question**: can a compromised most-senior quorum push a malicious release AND block or outweigh an
honest veto?

### 5.1 Who can veto
Any node whose local `is_producer_fn(pubkey_hex)` returns true — i.e. any producer with
`status == Active` in the local `ProducerSet` (`bins/node/src/run.rs:434-448`). Votes arrive by
gossip and **are** Ed25519-authenticated at ingress (`bins/node/src/node/network_events.rs:508`
calls `VoteMessage::verify`, which checks a signature over `"{version}:{vote}:{timestamp}"`,
`crates/updater/src/vote.rs:52-80`). `handle_vote` then applies only the active-producer filter and
records the vote (`service.rs:266-292`). **No minimum-age gate is applied**: `is_eligible_to_vote` /
`min_voting_age_blocks` (`params.rs:110-112`) have zero production callers.

### 5.2 How weights are computed
**They are not.** The live tally is `calculate_veto_result(veto_count, active_count)`
(`service.rs:321`) → `veto_percent = veto_count * 100 / total_producers`; approved iff
`veto_percent < 40` (`verification.rs:149-164`). Every seniority/bond-weighting implementation is
test-only (§C13). One producer = one vote, regardless of age, bond, or seniority.

### 5.3 The attacker's position
The compromised keys are 5 identities (N1–N5) out of ~45 live mainnet producers ≈ **11 % of the head
count**. They therefore **cannot** reach the 40 % veto threshold themselves, and — because weighting
is dead code — their seniority buys them **exactly nothing** in the tally.

### 5.4 Verdict

```
━━━ VETO-MATH VERDICT ━━━
Can a compromised senior quorum defeat the veto?   YES
  — but NOT via seniority weighting, which is dead code (C13).
Mechanism of defeat (3 independent, each sufficient):
  V1. WINDOW. The veto window is 300 seconds, not 7 days (C12; constants.rs:13,
      params.rs:28-29). Assembling >= 40% of ~45 independent operators (>= 18 human
      operators) into signed veto gossip within 5 minutes of an unannounced release is
      not operationally achievable. Absent pre-armed automation the veto never fires.
  V2. NON-CANONICAL TALLY. Each node counts only the votes its own gossip delivered
      (service.rs:266-292; VoteTracker is per-node, persisted to that node's data dir).
      The result is not consensus state and is never reconciled. A veto that succeeds on
      some nodes does not stop the others from installing.
  V3. SYMMETRY. The same unweighted, age-ungated vote path lets any 40% head count block
      the HONEST recovery release. The attacker does not need to defeat a veto — a
      defender's veto and an attacker's veto cost the same.
Evidence: crates/updater/src/constants.rs:13,26; crates/updater/src/params.rs:28-29;
          crates/updater/src/verification.rs:149-164; bins/node/src/updater/service.rs:321;
          bins/node/src/updater/service.rs:266-292; crates/updater/src/vote.rs:126-233
          (weighted fns, test-only callers).
Corollary: the veto CANNOT be treated as a mitigating control in any design. Any design
          that relies on "the 7-day veto gives humans time to react" is built on a false
          premise and must be rejected.
━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 6. New findings not present in the brief

**F1 — The maintainer quorum is not "at risk of compromise"; it is already fully public.**
All 5 mainnet bootstrap maintainer public keys are byte-identical to the `public_key` fields of
`testnetlinux/keys/producer_{1..5}.json`, files whose `private_key` fields are committed in the
public repository:

| Bootstrap key (constants.rs) | Matching repo file |
|---|---|
| `202047…d3df` (`constants.rs:39`) | `testnetlinux/keys/producer_1.json:7` (has `private_key` at :8) |
| `effe88…272b` (`constants.rs:41`) | `testnetlinux/keys/producer_2.json:7` (has `private_key` at :8) |
| `54323c…8c2b` (`constants.rs:43`) | `testnetlinux/keys/producer_3.json:7` (has `private_key` at :8) |
| `2d27fd…4116` (`constants.rs:45`) | `testnetlinux/keys/producer_4.json:7` (has `private_key` at :8) |
| `3047e9…7602` (`constants.rs:47`) | `testnetlinux/keys/producer_5.json:7` (has `private_key` at :8) |

INC-I-170 cryptographically proved (12/12) that these stored private keys derive their live public
keys. **5 of 5 maintainer private keys are public; the threshold is 3.** Anyone with a clone of the
repository can sign a release that every auto-update node on mainnet will accept and install. This is
not a hypothetical future hack — it is the current state, and it has been for ~138 days.

**F1b — mainnet and testnet bootstrap key arrays are byte-identical.** `constants.rs:37-48` vs
`:56-67` list the same 5 hex strings. There is no key separation between networks, so testnet key
hygiene *is* mainnet key hygiene.

**F2 — `assert_production_keys` cannot detect this.** It only rejects keys whose hex starts with
`"00000000"` (`constants.rs:81-101`). A real-but-leaked key passes the check.

**F3 — Governance signing messages have no replay domain.**
`MaintainerChangeData::signing_message` produces `"add:{target_hex}"` / `"remove:{target_hex}"`
(`maintainer.rs:323-326`); `ProtocolActivationData::signing_message` produces
`"activate:{version}:{epoch}"` (`maintainer.rs:376-382`). Neither binds a chain id, network, height,
or nonce. Combined with F1b (identical key sets across networks), a signature produced on testnet is
valid on mainnet, and any signature is replayable indefinitely.

**F4 — The threshold is dynamic and can be driven down.** `calculate_threshold` maps 5→3, 4→3, 3→2,
2→2 (`maintainer.rs:125-135`). `force_remove_maintainer` bypasses `MIN_MAINTAINERS` entirely
(`maintainer.rs:242-251`) and is reachable from slashing. Driving the set to 3 members lowers the
attacker's bar from 3 signatures to 2.

**F5 — Bond-side retirement does NOT revoke maintainer power.** `all_producers()` returns
`self.producers.values()` unfiltered by status (`crates/storage/src/producer/set_core.rs:398-400`),
and the bootstrap takes the first 5 by `registered_at` from that unfiltered list
(`periodic.rs:58-64`). I found no removal of a producer's map entry on `Exit` (only `exit_history`
retention pruning at `set_registration.rs:143,152`). **Therefore executing the `producer-retirement`
remediation for INC-I-170 removes the leaked identities' bond/finality power but leaves them as the
bootstrap-derived maintainers.** This is the single most important coupling between INC-I-170 and
INC-I-172 and must be re-verified on the target network before any remediation sequencing decision.

**F6 — Enforcement fails open.** `check_production_allowed` returns `Ok` if `binary_ready == false`
or if more than `ENFORCEMENT_TIMEOUT_SECS` (30 min) have elapsed (`enforcement.rs:170-205`). Relevant
to any design that plans to use enforcement as a forcing function for a recovery release.

**F7 — There is no consensus-level signature check on maintainer-change transactions.** See C6.
A structurally valid `AddMaintainer` with zero valid signatures is accepted into a block; only the
node-local apply step declines to mutate state. Nodes that reconstruct `maintainer_state.bin`
differently (fresh sync, data-dir wipe, differing gossip history) can therefore reach different
maintainer sets with no fork and no detection.

**F8 — Contradiction I caught and corrected in this analysis.**
⚠ CONTRADICTION: I first claimed `VoteMessage::verify()` had zero callers, based on a grep scoped to
`bins/node/src/updater/` and `crates/updater/src/vote.rs`. It **does** have a caller — at
`bins/node/src/node/network_events.rs:508`, outside both roots. Resolved: veto votes **are**
Ed25519-authenticated at gossip ingress. My scope was too narrow. The surviving authentication gap is
the missing *age* gate (§5.1), not a missing signature check.

---

## 7. Constraints the design must satisfy

**Hard (non-negotiable)**
1. **NO genesis reset.** CLAUDE.md #0 RULE. Features activate forward-only at a future height.
2. **Consensus-rule change ⇒ activation height. Block-content change ⇒ synchronized deploy.** Both
   questions must be answered explicitly for every consensus-visible element (INV-12, INC-I-062/INV-8).
3. **No synchronized fleet stop.** ~20–30 external auto-update producers cannot be stopped in unison.
4. **Late-upgrading and not-yet-synced nodes must not break.** Nodes that have not taken the new
   binary must keep verifying releases correctly.
5. **Assume the current maintainer keys ARE compromised, not may be** (F1). Any authorization path
   that requires only the current quorum's consent is assumed already available to the attacker.
6. **Convergence.** Every node must derive the same trust root from the same chain, including a node
   that fresh-syncs after a data-dir wipe (F7 shows this is currently not guaranteed).
7. **Encoder/decoder index parity** and `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION`
   discipline if any serialized format is touched.

**Soft (structural quality)**
8. **Non-foreclosure**: the result must not re-create the dead end being escaped — i.e. must not
   introduce a new hardcoded constant that again requires a fleet redeploy to rotate — and must not
   foreclose threshold/hardware signing or a later move of the maintainer set into the state root.
9. **Role separation**: producer key ≠ maintainer key, so a hot-node compromise does not yield
   software-signing power.
10. **The veto is not a control** (§5.4). Do not lean on it.

---

## 8. Requirements (MoSCoW)

### 8.1 Requirements table

| ID | Requirement | Priority | Acceptance criteria (summary) |
|---|---|---|---|
| REQ-172-001 | The update-signing trust root MUST be rotatable by on-chain action, with no new binary required to change *which* keys are trusted | Must | See §8.2 |
| REQ-172-002 | Rotation MUST remain possible when the current maintainer quorum is hostile (leak OR server hack) | Must | See §8.2 |
| REQ-172-003 | The design MUST activate forward-only and MUST NOT require a genesis reset | Must | See §8.2 |
| REQ-172-004 | The design MUST NOT require a synchronized fleet stop | Must | See §8.2 |
| REQ-172-005 | Behavior preservation: nodes that have not yet upgraded MUST continue to verify legitimate releases; fresh-sync and late-syncing nodes MUST converge on the same trust root | Must | See §8.2 |
| REQ-172-006 | The first-delivery bootstrap problem MUST have an explicit answer: how the first trust-root-fixing binary reaches operators without riding the compromised channel | Must | See §8.2 |
| REQ-172-007 | The design MUST state, per element, whether it is a consensus-rule change (⇒ activation height) and whether it is a block-content change (⇒ synchronized deploy) | Must | See §8.2 |
| REQ-172-008 | The design MUST NOT depend on the veto as a security control, and MUST state what it replaces the veto's assumed function with | Must | See §8.2 |
| REQ-172-009 | Producer and maintainer roles SHOULD be separated into distinct keys, with maintainer keys held offline | Should | See §8.2 |
| REQ-172-010 | The trust root SHOULD be replayable on-chain state that every node derives identically from block history | Should | See §8.2 |
| REQ-172-011 | Non-foreclosure: the design SHOULD NOT introduce a new non-rotatable constant, and SHOULD leave threshold/hardware signing and a future move into the state root reachable | Should | See §8.2 |
| REQ-172-012 | Governance authorization messages SHOULD carry a replay domain (network/chain id + height or nonce) | Should | See §8.2 |
| REQ-172-013 | Release verification COULD require signatures from keys that are provably *not* producer keys (role-separation enforcement at verify time) | Could | Verifier rejects a release signed only by keys present in `ProducerSet` |
| REQ-172-014 | Veto parameters COULD be corrected (window length, weighting, age gate) as a separate work item | Could | `VETO_PERIOD`, weighting wiring, and `is_eligible_to_vote` are addressed in their own change with their own tests |
| REQ-172-015 | Documentation/comment drift COULD be corrected: the four "7 days" claims and the `run.rs:457-459` obsolete TODO | Could | Each of the 5 cited locations updated to match code |
| REQ-172-016 | Implementing bond-side producer retirement | **Won't** | Already covered by `.claude/skills/producer-retirement/SKILL.md`. Out of scope. |
| REQ-172-017 | Fixing INC-I-171 (unenforced vesting penalty) | **Won't** | Independent incident; the retirement path currently *relies* on it being unfixed. Out of scope. |
| REQ-172-018 | Any DeFi, oracle, or ZK-settlement scope | **Won't** | `oracle_activation_height` / `defi_activation_height` are `u64::MAX`; unrelated to the trust root. Out of scope. |
| REQ-172-019 | Changing `CURRENT_PROTOCOL_VERSION` or `EPOCH_STATE_FORMAT_VERSION` | **Won't** | INV-4 / INC-I-054. Not required by this problem; forbidden without explicit user approval. |

### 8.2 Detailed acceptance criteria

**REQ-172-001 — On-chain rotatable trust root (Must)**
- [ ] Given a node running the new binary, when the on-chain maintainer set changes via an authorized action, then release verification uses the new set **without any binary replacement**.
- [ ] Given a release signed by a key that was removed from the set, when the node verifies it, then verification fails.
- [ ] Given a release signed by 3 keys added after genesis, when the node verifies it, then verification succeeds.
- [ ] The design names the exact replacement for `bins/node/src/run.rs:461` and states what it returns.
- [ ] Edge case: the on-chain set is empty or below threshold — the design states the defined behavior (it must not silently fall back to the leaked constants).

**REQ-172-002 — Rotation survives a hostile current quorum (Must)**
- [ ] The design names at least one authorization path that does **not** require ≥3 signatures from the currently-listed maintainers.
- [ ] Given an attacker holding 3 of 5 current maintainer keys, when honest operators execute the rotation path, then the attacker cannot veto, front-run, or reverse it — and the design shows why, step by step.
- [ ] Given an attacker attempts the same path first (front-running), the design states which side wins and why (time lock, producer-weight gate, social recovery, or an explicit accepted risk).
- [ ] The design states the trust anchor for that alternative path and why that anchor is not also compromised (F1: producer keys of N1–N12 are also public; ~89.85 % of selection weight per INC-I-170 — a producer-weight-gated override built on the current weighted set is **not** automatically safe).

**REQ-172-003 — Forward-only, no genesis reset (Must)**
- [ ] The design contains an explicit statement: "genesis reset required: NO", with reasoning.
- [ ] No existing block's state root changes.
- [ ] Any consensus-visible element names a specific future activation height in `crates/core/src/network_params/` (never 0, never a height already crossed).
- [ ] The three-question INV-12 checklist is answered in writing for every consensus-visible computation touched.

**REQ-172-004 — No synchronized fleet stop (Must)**
- [ ] The rollout plan works with ~20–30 external producers upgrading at arbitrary times.
- [ ] Given a mixed fleet (some upgraded, some not), when a legitimate release is published, then both populations accept it.
- [ ] Given a mixed fleet, no partition, fork, or production halt occurs at any point in the rollout.

**REQ-172-005 — Behavior preservation and convergence (Must)**
- [ ] Given a node still on the old binary, when a legitimate release signed by the current constants is published, then it verifies and installs as today.
- [ ] Given a node that fresh-syncs from genesis after the change, when it reaches tip, then its trust root is byte-identical to a node that was online throughout.
- [ ] Given a node whose data directory (including `maintainer_state.bin`) was wiped, when it re-syncs, then it derives the same trust root — closing F7.
- [ ] Given a node that syncs via snapshot rather than full replay, the design states how it obtains the trust root.

**REQ-172-006 — First-delivery bootstrap (Must)**
- [ ] The design states how the first trust-root-fixing binary reaches operators, given the attacker can sign releases on the normal channel.
- [ ] The plan specifies an out-of-band verification method operators can perform (independent of the compromised keys) and who can perform it.
- [ ] The plan distinguishes the ~15 operator-controlled nodes (N1–N12 + 3 seeds) from the ~20–30 external auto-update nodes and gives a path for each.
- [ ] The plan states what happens to external nodes that never take the fixing binary.
- [ ] The plan addresses the INC-I-157 origin invariant (`crates/updater/src/constants.rs:118-138`) and the unversioned served installer (`doli.network/install.sh`).

**REQ-172-007 — Deploy classification (Must)**
- [ ] For every element: "consensus rule change: YES/NO" and "block content change: YES/NO", each with reasoning.
- [ ] Any YES on consensus rules names its activation height.
- [ ] Any YES on block content states the synchronized-deploy requirement and how it is reconciled with REQ-172-004.
- [ ] The design confirms no `HardForkSchedule` entry is added for a rolling deploy (CLAUDE.md).

**REQ-172-008 — Do not rely on the veto (Must)**
- [ ] The design contains no security argument of the form "the veto period gives time to react".
- [ ] If the design assigns any function to the veto, it cites the corrected facts (5-minute window, unweighted count, node-local tally) and shows the function still holds.
- [ ] The design states what supplies the "humans can stop a bad release" property, if anything.

**REQ-172-009 — Role separation (Should)**
- [ ] Post-change, a maintainer key is not required to be a registered producer key.
- [ ] Given an attacker who fully compromises a producer node's host, then they obtain producer/bond power but **not** release-signing power.
- [ ] The operational cost is stated: key ceremony, signing latency, quorum availability, and what happens if an offline key holder is unreachable during an emergency.
- [ ] The design states whether `MaintainerError::NotRegisteredProducer` (`maintainer.rs:408-409`) and the `AddMaintainer` doc contract "Target must be a registered producer" (`types.rs:43`) must be relaxed, and whether that is a consensus-visible change.

**REQ-172-010 — Replayable on-chain derivation (Should)**
- [ ] The trust root is derivable by any node from block history alone, without trusting `maintainer_state.bin`.
- [ ] The design states whether `derive_maintainer_set` (`maintainer.rs:490`, currently dead) is revived, replaced, or deleted.
- [ ] Two nodes with identical block history derive identical trust roots — demonstrated by a test, not asserted.

**REQ-172-011 — Non-foreclosure (Should)**
- [ ] The design introduces no new compile-time key constant whose rotation requires a fleet redeploy. If a genesis-seed constant remains, the design states the precise conditions under which it is consulted and shows those conditions cannot be attacker-induced.
- [ ] The design states how it would later accommodate threshold signing (e.g. FROST) or hardware-backed keys without a second redesign.
- [ ] The design states how the maintainer set could later be promoted into the consensus state root, and what that would cost then.

**REQ-172-012 — Replay domain on authorization messages (Should)**
- [ ] Signing messages bind at minimum a network identifier and a height or nonce (closing F3).
- [ ] Given a signature produced for testnet, when replayed on mainnet, then it is rejected.
- [ ] The design states whether changing the signing message is a consensus-visible change and how old-format signatures are handled during the transition.

---

## 9. What I do not understand (mandatory disclosure)

These gaps are real and the evaluators should treat them as unresolved, not as settled facts.

1. **Is the mainnet maintainer set actually bootstrapped today?** `maybe_bootstrap_maintainer_set` runs at an epoch boundary once ≥5 producers exist (`periodic.rs:35-56`). Whether mainnet nodes currently hold a 5-member `maintainer_state.bin`, and whether all of them hold the *same* one, is a live-network question I cannot answer from source. It determines whether the SSF candidate has anything to read.
2. **Have any `AddMaintainer` / `RemoveMaintainer` transactions ever been included on mainnet?** If none, the entire governance path is untested in production.
3. **Do exited/retired producers remain in `ProducerSet::producers`?** I verified `all_producers()` is unfiltered (`set_core.rs:398-400`) and found no map removal on `Exit`, but I did not exhaustively trace every mutation path. F5 depends on this and must be re-verified on the target network.
4. **Snap-sync interaction.** How a snapshot-syncing node obtains the maintainer set — whether it replays, inherits, or re-bootstraps — I did not trace.
5. **`crates/storage/src/producer/set_governance.rs` weighted-veto machinery.** It is fully implemented, threshold-correct, and has zero production callers. I do not know whether it is abandoned, pre-wired for a planned feature, or wired somewhere I failed to find.
6. **Where the 45 live mainnet producer count comes from.** I used the figure from the INC-I-170 record (`getProducers`, 45 producers, 12 leaked, 89.85 % weight). I did not re-measure it this session. §5.3's "≈11 % head count" inherits that uncertainty.
7. **Whether N1–N5's *live mainnet* producer keys equal the bootstrap constants.** I proved the constants match `testnetlinux/keys/producer_{1..5}.json` in this repo. INC-I-170 proved 12 repo key files match live mainnet producers, but I did not personally re-run that derivation against the live set this session.

---

## 10. Open questions for the user (non-blocking)

1. **Sequencing vs INC-I-170.** If F5 holds, retiring the leaked producers does not revoke their maintainer power. Should the update-side rotation land **before**, **with**, or **after** the bond-side retirement? This changes the whole rollout shape.
2. **Is an out-of-band distribution channel available** for the first-delivery binary (a signing key held by a person rather than a server, a second independent origin, a published fingerprint operators already trust)? REQ-172-006 has no good answer without one.
3. **Where should the new maintainer keys live?** Hardware tokens, an offline machine, a threshold scheme across separate operators — this is an operational-cost decision only you can make, and it bounds REQ-172-009.
4. **Is `VETO_PERIOD = 5 minutes` intentional at the current network stage?** The comment says "early network; target: 7 days". If it should be longer, that is a separate small change — but four documents and one log line currently assert 7 days, which is drift that must be corrected regardless.
5. **Is a producer-weight-gated override acceptable**, given that INC-I-170 reports 89.85 % of selection weight is held by leaked keys? Such an override may be handing the attacker a second door rather than giving defenders a first one.
6. **Should `MaintainerSet` eventually move into the consensus state root?** That would make divergence fork-detectable (closing F7) at the cost of an activation height and a block-content discussion. Knowing your appetite shapes REQ-172-011.

---

## 11. Traceability matrix

Implementation column filled after **M1** (Layer 1, node-local, no activation height)
and **M2** (Layer 2, gated on `maintainer_derivation_activation_height`: mainnet
`172_000`, testnet `127_200`, devnet `0`). M3 (replay-domain binding) is NOT
implemented; rows it owns say so explicitly.

| Requirement ID | Priority | Design section | Test IDs | Implementation module |
|---|---|---|---|---|
| REQ-172-001 | Must | F1, F6 | trust_root_fail_closed A1,A2,A9,A11; upgrade_verify_blocks C2; update_cmd C7 | **M1 partial** — `trust_root` @ `crates/updater/src/trust_root.rs`; `verification` @ `crates/updater/src/verification.rs`; `trust_root_wiring` @ `bins/node/src/updater/trust_root_wiring.rs`. Rotation BY on-chain action is wired (the on-chain set is now authoritative); making it durable needs F2 (M2, AH). |
| REQ-172-002 | Must | Option A, F4 | inc_i_172_m2_fail_close IP-F1,IP-F3,IP-F4 | **M2 partial** — the hostile-quorum BACK-DOOR is closed: `ProtocolActivation` no longer falls back to producer-key authority above the gate. `process_transaction_governance` @ `bins/node/src/node/apply_block/governance.rs`; `MaintainerSet::is_authorizable` @ `crates/core/src/maintainer/set.rs`. The positive answer (rotation despite a hostile quorum) still needs the Option A weight hatch, hard-sequenced after INC-I-170. |
| REQ-172-003 | Must | all layers | — | **M1 satisfied** — no activation height, no genesis reset, no consensus change (INV-12 Q1 NO / Q2 NO / Q3 N/A) |
| REQ-172-004 | Must | all layers | — | **M1 satisfied** — node-local only; rolling deploy safe, no synchronized stop |
| REQ-172-005 | Must | F1, F5, F2 | trust_root_fail_closed A5,A6,A12; maintainer_state_versioned B1,B5; inc_i_172_m2_canonical_derivation IP-B1..IP-B6; inc_i_172_m2_maintainer_reset IP-4,IP-5 | **M2 — behavior-preservation half SATISFIED, convergence half PARTIAL.** Preservation: `TrustRoot::bootstrap` @ `crates/updater/src/trust_root.rs`; `resolve_trust_root` (members empty AND `last_derived_height == 0` → bootstrap) @ `bins/node/src/updater/trust_root_wiring.rs`; `MaintainerState` @ `crates/storage/src/maintainer.rs`; below the gate `maybe_bootstrap_maintainer_set` reproduces pre-M2 behavior verbatim, so old and new binaries agree at every height < AH. Convergence: `derive_canonical_maintainer_set` (total order `(registered_at, pubkey_bytes)`) @ `crates/core/src/maintainer/derivation.rs`, consumed by `maybe_bootstrap_maintainer_set` @ `bins/node/src/node/periodic.rs`. **Criterion-by-criterion (analysis §8.2):** old binary verifies legitimate releases — PASS; fresh-sync ≡ always-online — PASS on membership; wiped data dir + FULL resync — PASS (every block re-applied, governance re-executed); wiped `maintainer_state.bin` only, chain intact — **FAIL**, residual **R1**; snapshot-sync path stated — **the design STATED "snap-only nodes fail closed", the code does NOT** (`periodic.rs:55-136` never reads `ChainState::is_snap_synced()`), spec amended 2026-08-10, residual **R3**. Also unimplemented: reorg undo for maintainer state (`bins/node/src/node/rollback.rs`), residual **R2**. All three residuals are scoped in `docs/.workflow/inc-i-172-M3-scope.md`; none is a regression against pre-M2 behavior. |
| REQ-172-006 | Must | F6 | upgrade_verify_blocks C1,C3,C4,C5; update_cmd C6,C8 | `cmd_upgrade` @ `bins/cli/src/cmd_upgrade.rs`; `handle_update_command` @ `bins/node/src/commands/update.rs`. NOTE: this covers the *enforcement* half of the operator path. The out-of-band FIRST-DELIVERY answer is still M2+. |
| REQ-172-007 | Must | §Consensus classification | — | **M1 satisfied** — classification stated per change in `docs/.workflow/inc-i-172-M1-dev-notes.md` and in the milestone commit |
| REQ-172-008 | Must | F8 | — (doc-only; no code path) | Weighted-veto machinery DELETED @ `crates/updater/src/vote.rs`, `crates/updater/src/params.rs`; false claims corrected in `docs/auto_update_system.md`, `docs/architecture.md`, `docs/attack_analysis.md`, `docs/cli.md`, `.claude/skills/{updater,auto-update}/SKILL.md` |
| REQ-172-009 | Should | Option C | — | **NOT IMPLEMENTED** (M2+; cold-key role separation) |
| REQ-172-010 | Should | F2, F8 | inc_i_172_m2_canonical_derivation (replay-identical); inc_i_172_m2_maintainer_governance IP-12,IP-13,IP-14 | **M2 partial — function landed, NODE wiring deferred to M3 (R1).** NODE path: `maybe_bootstrap_maintainer_set` @ `bins/node/src/node/periodic.rs` calls `derive_canonical_maintainer_set` over the **live `ProducerSet`**, not block history, and its one-shot guard `maintainer_seed_is_done` is a function of `maintainer_state.bin` alone. So criterion 1 ("derivable from block history alone, **without trusting `maintainer_state.bin`**") is **NOT met**: deleting that file on a node with an intact chain re-seeds from live producer state and re-arms a governance-removed key (M2 QA PROBE-1, `removed_key_back=true`). Criterion 3 (two nodes, identical history → identical root) IS met, but only as a property of the pure function. Criterion 2 is met: `derive_maintainer_set` @ `crates/core/src/maintainer/derivation.rs` is revived and replay-complete (genesis seed via `derive_canonical_maintainer_set` + every governance action ≤ H) — **but it has ZERO production callers**, so `wiped_node_replay_converges_with_a_node_that_was_online_throughout` (`crates/core/tests/inc_i_172_m2_canonical_derivation.rs:512`) proves a property of code no node executes. Wiring it needs a real `BlockchainReader` over the block store: **R1** in `docs/.workflow/inc-i-172-M3-scope.md`. What M2 *did* buy: above the gate the root is no longer re-derived on every applied block @ `bins/node/src/node/periodic.rs::maintainer_seed_is_done`. |
| REQ-172-011 | Should | F1, F5 | trust_root_fail_closed A3,A4,A11; maintainer_state_versioned B2,B3,B4 | `TrustRoot::is_usable` @ `crates/updater/src/trust_root.rs`; `MAINTAINER_STATE_VERSION` + fail-closed `load()` @ `crates/storage/src/maintainer.rs`; `StorageError::UnsupportedFormatVersion` @ `crates/storage/src/lib.rs`. No new non-rotatable constant was introduced. |
| REQ-172-012 | Should | F3, F7 | trust_root_fail_closed A7,A8,A9,A10; service_timing D1–D5 | Distinct-signer counter @ `crates/updater/src/verification.rs`; node-local timing @ `crates/updater/src/enforcement.rs`, `crates/updater/src/params.rs`, `bins/node/src/updater/mod.rs`, `bins/node/src/updater/service.rs`; re-verify-at-install @ `bins/node/src/updater/service.rs::auto_apply`. **M2** extends the distinct-signer counter to the GOVERNANCE path (`AddMaintainer` / `RemoveMaintainer` / `ProtocolActivation`), gated: `verify_multisig{,_excluding}` (distinct) / `_legacy` (frozen entry-counting) / `_at` (dispatcher) @ `crates/core/src/maintainer/set.rs`, call sites @ `bins/node/src/node/apply_block/governance.rs`. **Replay-DOMAIN binding on governance messages is NOT implemented** (M3, AH required) — neither M1 nor M2 changed the signed message format. |
| REQ-172-013 | Could | — | — | **NOT IMPLEMENTED** |
| REQ-172-014 | Could | F8/G4 | — | **M1 partial** — the false 7-day/weighted claims are corrected and `is_eligible_to_vote` is deleted; `VETO_PERIOD` itself is UNCHANGED (still 300 s) and window-length correction remains its own work item |
| REQ-172-015 | Could | F8/G3/G4 | — | `constants.rs` fallback + 7-day comments, `run.rs` false BLAKE3 TODO (deleted), `service.rs` 7-day log literal, `updater/mod.rs:13`. `WHITEPAPER.md` §§ deliberately NOT edited — see dev notes |
| REQ-172-016..019 | Won't | N/A (excluded) | N/A | N/A — confirmed for M1 **and M2**: no `Cargo.toml`, `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION` or `MAINTAINER_STATE_VERSION` change. M2's one-shot seed guard reuses the existing `last_derived_height` field precisely to avoid a `MAINTAINER_STATE_VERSION` bump. |

### M3 scope — residuals carried out of M2

Full detail (evidence, fix, why-not-in-M2, consensus classification):
**`docs/.workflow/inc-i-172-M3-scope.md`**. Summary, so the table above is not read as
"everything is closed":

| # | Sev | Residual | Where | Blocks the M2 AH? |
|---|---|---|---|---|
| **R1** | high | Deleting `maintainer_state.bin` above the AH re-seeds from live producer state and **re-arms a governance-removed maintainer key** (measured: QA PROBE-1). `rm maintainer_state.bin` is the strongest attacker capability against the trust root. Fix = wire the replay derivation when the file is absent and `best_height >= AH` | `bins/node/src/node/periodic.rs` | **No** — pre-M2 behavior was equal or worse (re-derived every block) |
| **R2** | med | No maintainer-state undo on reorg, so above the gate a governance mutation from a reorged-out block persists. Fix = record Add/Remove in the undo batch, or re-derive on reorg | `bins/node/src/node/rollback.rs` | **No** — masked below the gate by the per-block re-seed |
| **R3** | med | Snap-synced nodes never replay governance history and **silently diverge rather than failing closed**; the spec's F-7 claim was amended to match the code | `bins/node/src/node/periodic.rs` | **No** — deliberate: a snap-only fail-close is a NEW divergence axis needing its own review |
| **R4** | note | **RELEASE NOTE**: the first legitimate rotation above the AH disables auto-update fleet-wide until the M1 containment at `crates/updater/src/trust_root.rs:155` is lifted. Fail-closed, so not a hole — an operational trap on the exact action this incident enables | `crates/updater/src/trust_root.rs` | **No** |
| **R5** | action | **PRE-RELEASE**: re-verify the live mainnet tip against `172_000`. Pinned 2026-08-10 at tip 162_727 = ~25.8 h of lead **from pin time**, thin for ~30 external auto-updaters. Re-pin BEFORE first crossing if it has eroded — after crossing it is immutable (INC-I-054) | `crates/core/src/network_params/defaults.rs` | **No** — but must be done before release |

---

## 12. Specs / docs drift detected

| Location | Drift | Severity |
|---|---|---|
| `bins/node/src/run.rs:457-459` | TODO states raw Ed25519 keys are not on-chain. **False** — `ProducerInfo.public_key` is raw Ed25519 (`crates/storage/src/producer/types.rs:73-74`) and is already read at `periodic.rs:63` and `governance.rs:121`. | High — this comment is the stated reason the fix was never made |
| `bins/node/src/updater/service.rs:227` | Log literal "(veto period: 7 days)" — actual window is 300 s | High — operators read this |
| `crates/updater/src/vote.rs:3` | Doc comment "7-day period" | Medium |
| `crates/updater/src/apply.rs:69` | Doc comment "The 7-day veto period has ended" | Medium |
| `crates/network/src/gossip/staleness.rs:85` | Comment "= 7 days: the auto-update GOVERNANCE VOTING WINDOW is `VETO_PERIOD`" — used to justify a staleness constant | High — a real constant may be sized from a false premise |
| `crates/updater/src/constants.rs:18` | `VETO_THRESHOLD_PERCENT` doc says "40% of active producers (weighted by seniority)" — the live path is unweighted | High |
| `crates/core/src/maintainer.rs:11,55-60` | Module doc claims the set is derived by replaying the blockchain; `derive_maintainer_set` has zero production callers | High — **PARTLY resolved in M2.** Resolved: the module was split into `crates/core/src/maintainer/` and its doc now states what is true (distinct-signer threshold, canonical total order, the ungated empty-set refusal); `derive_maintainer_set` is revived, replay-complete, and shares `seated_registrations` with the node's genesis seed. **Still true and NOT resolved: `derive_maintainer_set` STILL has zero production callers** (verified 2026-08-10 — `grep -rn derive_maintainer_set bins crates --include="*.rs"` returns only the `crates/core/src/lib.rs` re-export and test files). No doc may claim the node replays governance history. Residual **R1**. |
| `crates/updater/src/constants.rs:34-36`, `:53-55` | "Once synced, on-chain keys take precedence" — they never do (`run.rs:461`) | High |

---

## 13. Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
  [x] 1. Cross-module blast radius — a fix spans crates/updater, crates/core, crates/storage,
         bins/node/{run,updater,node/periodic,node/apply_block}, crates/rpc, with no single
         shared dependency owning the trust root.
  [x] 2. Invariant gaps — no module enforces "every node agrees on the maintainer set".
         MaintainerState is outside the state root (§3.3) and consensus validation of
         maintainer-change txs is structural only (C6/F7), so divergence is silent.
  [ ] 3. Data flow reversal — NOT detected. The needed flow (chain state -> updater) runs in
         the same direction as existing flows; only the wire is absent.
  [x] 4. Shared mutable state without a clear owner — maintainer_state.bin is written by
         periodic.rs (bootstrap) and by apply_block/governance.rs (mutation), read by the
         updater, RPC, and ProtocolActivation, with no single owning module.
  [x] 5. Contract absence — updater <-> governance have no explicit interface. The connection
         point is an untyped `impl Fn() -> Vec<String>` closure stubbed to Vec::new().
Verdict: BRITTLE
Implication: this is an architectural problem, not a code bug. A one-line wiring change
             (the SSF candidate) is necessary but insufficient; the design must also supply
             the missing ownership, the missing convergence invariant, and the missing
             authorization model. This supports a DEEP treatment.
━━━━━━━━━━━━━━━━━━━━━━━━
```
