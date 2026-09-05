# INC-I-208 — Post-AH builders omit their OWN attendance bit

Run 545 · `/omega-doctor --fast` · analyst · 2026-09-05 · branch `feature/inc-i-178-attestation-bls`

**Scope**: `node/startup.rs` (egress), `node/attestation/{mod,ingress,commit,verify,keys}.rs`,
`node/apply_block/post_commit.rs`, `node/production/assembly.rs`, `crates/core/src/attestation/pool.rs`.
Read-only: `crates/storage/src/producer/`.

**Plain language**: a producer signs an attestation for the block it just applied and sends it to everyone
else, but never keeps a copy. Since AH 112,619 attendance bits are built only from kept copies, so every block
misses the bit of its own builder. Nothing is unsafe — one producer per block loses credit for one block. Fix:
keep the copy, but only after checking it against the key that producer published on-chain.

## 1. Deploy answer — CONFIRMED at file:line

**Consensus RULES change? NO. Block CONTENT change? YES. Rolling deploy? SAFE.**
The validator is a *predicate over the carried body*, never a recomputation of the builder's pool. No
"expected bitfield" comparison exists anywhere. The three checks a block's attestation body faces:

| Check | Site | Effect of one extra legitimately-signed bit |
|---|---|---|
| Root binds the body it arrived with | `attestation/verify.rs:51` — `if *presence_root != presence_commitment(bitfield, aggregate)` | Recomputed **from the carried bitfield+aggregate**; the builder set both consistently → matches |
| Aggregate covers exactly the set bits | `attestation/verify.rs:66-72` — `let keys = set_bit_keys.map_err(...)?;` … `crypto::bls_verify_aggregate(&bls_attest_msg(parent_hash), &sig, &keys)` | The extra key is the builder's on-chain `bls_pubkey`; its verified sig is one more component over the *same* message → verifies |
| No bit beyond universe width | `validation_checks.rs:441-442` — `width::bitfield_width_accepted_at(...) \|\| !doli_core::validate_attestation_bitfield_vec(bf, producer_count)` | The builder is by definition in `active_producers_at_height(h)` → index inside the universe → passes |

Set-bit keys come from the chain, not the sender: `attestation/keys.rs:22-26`
(`producers.get_by_pubkey(&pk).bls_pubkey`; `Err(pk)` if empty/invalid → `REASON_MISSING_BLS_KEY`).
Acceptance depends only on carried bytes + on-chain keys, so un-upgraded nodes accept upgraded builders'
blocks and vice versa. The post-AH bitfield is **already** builder-local and non-deterministic across nodes;
the fix does not introduce that, it makes the builder's own contribution truthful.

**Three-question checklist (INC-I-075 / INV-12)**
1. **User-submittable tx reaches this path? NO** — no `TxType` writes `parent_sig_pool`; it is fed only by
   `ingest_attestation` (`ingress.rs:88`) and, after the fix, by own signing.
2. **Producer-action / attestation pattern reaches it? YES** — attestations are producer-generated and the
   change alters which bit the local builder sets.
3. **Bit-identical for all reachable inputs?** Split precisely. **VALIDATION of any input: YES** —
   `decide_attestation`, `verify_block_attestation`, `set_bit_bls_pubkeys` and `validation_checks.rs:421-448`
   are untouched; same block bytes ⇒ same verdict before and after. **The builder's OUTPUT: NO** — an
   upgraded builder emits a superset bitfield and a different aggregate.

**Verdict: no activation height.** INV-12 gates *divergent validation of the same input*, which does not
occur. What changes is freely-chosen, already-node-local block content that every existing verifier accepts
bit-identically. Rolling deploy; no synchronized stop-all.

## 2. Architecture Context

**Module boundaries**
- **Egress — `startup.rs:591 create_and_broadcast_attestation(&self, ...)`**: derives weight
  (`derive_attester_weight`, INV-ATTEST-001), signs via `attestation/mod.rs:17 sign_attestation` (BLS half
  when `self.bls_key` is `Some`), adds finality weight, gossips, direct-sends to slot+1's producer.
  **Writes `sync_manager` only — never `parent_sig_pool`.** ← the defect.
- **Ingress — `attestation/ingress.rs:55 ingest_attestation(&mut self, att, height, source_peer)`**:
  membership → `minute_tracker` → `bls_verdict` → **the ONLY pool insert, `ingress.rs:87-90`**.
- **Verdict — `attestation/ingress.rs:129 fn bls_verdict(&self, att, onchain_bls_key)`**: private, `&self`,
  never mutates; `Valid`/`Empty`/`NoKey`/`Invalid`; flood bound at `ingress.rs:141-147`.
- **Commitment — `attestation/commit.rs:225 build_attestation_commitment_at`** → post-AH `pooled_commitment`
  (`commit.rs:156-188`): *"bit `i` is set iff `universe[i]` has a pooled signature over `parent`"*.
  **The own bit can ONLY come from the pool.**
- **Pool — `crates/core/src/attestation/pool.rs`**: `insert(parent, attester, sig) -> bool` (first-seen wins,
  `pool.rs:49-54`), `get`, `signatures_for`, `MAX_PARENTS = 8`. Node-local scratch, never serialized;
  `Node.parent_sig_pool` is `pub` (`node/mod.rs:227`); cleared at `post_commit.rs:421`.

**Data flow — peer vs own attestation**
```
PEER : peer signs -> gossip/direct -> ingest_attestation -> bls_verdict(on-chain key) -> POOL -> bit set
OWN  : self signs -> gossip out ......................................................... (no return path)
```
Gossipsub does not loop own messages back and the direct send targets slot+1's producer. At height h the
builder P_h attested block h-1 at `post_commit.rs:445`; building h with `parent = hash(h-1)`, its own sig is
absent from the pool, so its bit is clear. Violates **REQ-BLS-001 AC-3** ("Given an attester that attested
the parent, when the producer builds, then its bit is set" —
`docs/redesigns/attestation-bls-redesign-analysis.md:300`). Symptom: `[ATTEST_MISS]` at `post_commit.rs:117-124`.

**Blast radius** — graph query
(`python3 .claude/scripts/blast.py graphify-out/graph.json create_and_broadcast_attestation --hops 2`)
returned 1 dependent and self-flagged the known Rust receiver-method blind spot, so grep is authoritative
here (per-root: `bins/` = 5 hits, `crates/` = 0 hits):
- **Production, direct**: `apply_block/post_commit.rs:445`, `production/assembly.rs:639 attest_own_block` —
  both already `&mut self`, so a `&self → &mut self` change compiles unchanged.
- **Tests, direct**: `bins/node/tests/delegated_bond_attestation.rs:47` and `:94` bind `let node` /
  `let (node, producers, _tmp)` **immutably** → a `&mut self` change forces `let mut` at both. These carry
  INV-ATTEST-001 (`active_producer_with_bonds_attests`, `fully_delegated_producer_does_not_attest`);
  `tests/it/inc_i_178_m2_ingress.rs:656` asserts the same weight gate by marker.
- **Indirect**: `pooled_commitment` → every post-AH block this node builds → every peer's
  `verify_block_attestation`; metrics `ATTESTATION_BITFIELD_FILL_RATIO`, `ATTESTATION_BLS_VALID_*`;
  minute-union reward attribution (`rewards.rs`).
- **`bls_verdict` dependents**: exactly one (`ingest_attestation`) — widening its visibility is contained.

**Constraints / invariants**
- **INV-ATTEST-001** (protects `startup.rs`): active producers attest regardless of `selection_weight`; the
  `weight == 0 → None` early return at `startup.rs:605` must survive and stay *before* any pooling.
- **Only-verified-pools** (`ingress.rs:16-18`): *"Verified against the attester's on-chain key — the only arm
  that pools."* Any new pool writer must honour this or the aggregate becomes unverifiable.
- **Encoder/decoder index parity**: `pooled_commitment` encodes in universe order, `set_bit_bls_pubkeys`
  decodes with the paired LSB-first helper. Untouched — do not disturb.
- **Pool is epoch scratch**: cleared at `post_commit.rs:421` *before* the egress call at `:445` — ordering
  already correct at a boundary block. `MAX_PARENTS = 8` unaffected (no new parent keys).

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 1/5
Details: (5) contract absence — the egress<->pool contract was implicit, and assembly.rs:640
("the BLS half is pooled by the attestation ingress") encoded a FALSE version of it.
Not detected: (1) 2 production files in one module tree; (2) the verify-before-pool invariant
EXISTS (bls_verdict), it is only unreachable from egress; (3) data flows forward along the
existing egress->pool direction; (4) parent_sig_pool has one owner, epoch-scoped, one clear site.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

## 3. Risk — local BLS key that does not match the on-chain `bls_pubkey`

**Today (bit loss, contained)**: such a producer's attestation lands on `BlsAttestVerdict::Invalid`
(`ingress.rs:105-121`) or `NoKey` (`ingress.rs:133-135`, e.g. `bls_pubkey` never registered → empty) at every
peer. Nothing pools, no builder sets its bit, it loses attendance only. `ingress.rs:106-109` deliberately does
not act on the score, to avoid a fleet partition on one misconfigured producer.

**If the egress pooled the own sig WITHOUT verifying (slot loss, severe)**: that producer's own blocks would
carry a bit whose on-chain key either resolves to a non-matching key — `bls_verify_aggregate` fails →
`REASON_AGGREGATE_INVALID` (`verify.rs:70-72`) — or does not resolve at all — `set_bit_bls_pubkeys` returns
`Err(pk)` → `REASON_MISSING_BLS_KEY` (`verify.rs:66`). Either way **every peer rejects every block it
produces**: a silent 1/N attendance loss becomes a total, permanent slot loss. Hence REQ-208-001 is
conditional and REQ-208-002 exists.

**Recommended seam (ONE)**: reuse `bls_verdict` — widen `ingress.rs:129` from `fn` to `pub(crate) fn` and
call it from the egress after signing, pooling only on `Valid`. It is `&self`, never mutates, has one existing
caller, and its flood-bound branch (`ingress.rs:141-147`) makes a repeat call idempotent. The egress already
holds the `producer_set` read guard at `startup.rs:600-607`, so the on-chain `bls_pubkey` is captured in that
same scope, mirroring `ingress.rs:65-70`. **Rejected — `ingest_attestation`**: it demands a `source_peer:
PeerId` that does not exist for self, and on `Invalid` it calls
`bls_ingress_scorer.record_invalid_bls_attestation(&source_peer)` (`ingress.rs:110-111`) — scoring yourself
pollutes the peer scorer and logs a false `"relayed by <self>"`; it would also re-record `minute_tracker`,
which both egress call sites already do.

**Test-harness constraint (NEW this session)**: `Node::new_for_test` (`init.rs:1158`) sets
`bls_key = Some(BlsKeyPair::generate())` at `init.rs:1246` but **never registers it on-chain** —
`register_genesis_producer` writes `bls_pubkey: Vec::new()`
(`crates/storage/src/producer/set_registration.rs:210`; same default at `producer/info.rs:100,148,199`).
A test node therefore lands on `NoKey`, not `Valid`. Any reproduction test **must first write the node's local
BLS pubkey into the test ProducerSet** (`producer_set.write().await` → `get_by_pubkey_mut(...)` → set
`bls_pubkey`) or the verification path is unreachable and the test goes green for the wrong reason.

## 4. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-208-001 | After signing, the egress pools its OWN BLS half under `(block_hash, own_pubkey)` **iff** it verifies against the on-chain `bls_pubkey` | Must | - [ ] Verdict obtained by the same code the ingress uses<br>- [ ] Insert only on the `Valid` arm<br>- [ ] `parent_sig_pool.get(&block_hash, &own_pk)` is `Some` after the call |
| REQ-208-002 | A mismatching/unregistered local key is NOT pooled, is logged, does not panic, does not suppress the broadcast | Must | - [ ] `Invalid`/`NoKey`/`Empty` ⇒ pool unchanged<br>- [ ] At most one WARN per node run<br>- [ ] Gossip + direct send still happen<br>- [ ] Still returns `Some(attestation)` |
| REQ-208-003 | Existing egress contract preserved | Must | - [ ] `None` for weight 0 and non-producer<br>- [ ] INV-ATTEST-001 tests green<br>- [ ] Both production call sites compile unchanged |
| REQ-208-004 | The false comment at `assembly.rs:640` is corrected | Should | - [ ] No comment claims the ingress pools the own BLS half |

**Detailed AC — REQ-208-001**: given a test node whose local BLS pubkey **is** registered on-chain, when
`create_and_broadcast_attestation(block_hash, slot, height)` returns, then
`parent_sig_pool.get(&block_hash, own_pk)` is `Some` and the stored 96 bytes equal the returned
`Attestation.bls_signature`; called twice, the second insert returns `false` and the bytes are unchanged;
with `bls_key = None` the pool stays empty and the Ed25519-only attestation is still returned.
**REQ-208-002**: with an empty on-chain `bls_pubkey` (the `new_for_test` default) the pool stays empty, the
return is `Some`, and one WARN names the mismatch; with a valid-but-different 48-byte on-chain key the same
holds via `Invalid` and the peer scorer is **not** touched.
**REQ-208-003**: `active_producer_with_delegated_bonds_still_attests`, `active_producer_with_bonds_attests`,
`fully_delegated_producer_does_not_attest` pass unmodified except `let` → `let mut`.

**Expected FAIL→PASS reproduction (shape, words only)**: build a `new_for_test` node, register its local BLS
pubkey as its on-chain `bls_pubkey` in the test ProducerSet, call `create_and_broadcast_attestation` with a
fixed `block_hash`, assert `parent_sig_pool.get(&block_hash, &own_pk)` is `Some`. **Today this is `None` and
the test FAILS**; after M1 it is `Some` and equals the returned attestation's BLS half. A second test leaves
the on-chain `bls_pubkey` at its `Vec::new()` default and asserts the pool stays empty while the return is `Some`.

**Traceability**
| ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-208-001 | Must | `egress_pools_own_bls_half_when_onchain_key_matches` (RED) | §2 Egress/Pool, §3 seam | `node/startup.rs`, `node/attestation/ingress.rs` |
| REQ-208-002 | Must | `egress_does_not_pool_when_onchain_key_differs` | §3 risk | `node/startup.rs` |
| REQ-208-003 | Must | `egress_does_not_pool_when_onchain_key_is_unregistered`, `active_producer_with_bonds_attests`, `fully_delegated_producer_does_not_attest` | §2 INV-ATTEST-001 | `node/startup.rs`, `tests/delegated_bond_attestation.rs` |
| REQ-208-004 | Should | n/a (doc) | §2 contract absence | `node/production/assembly.rs` |

## 5. Impact, assumptions, gaps
**Affected**: `node/startup.rs` — verify+pool step, likely `&self → &mut self`, **risk medium** (INV-ATTEST-001
lives here). `node/attestation/ingress.rs` — visibility only, **low**. `tests/delegated_bond_attestation.rs` —
binding mutability, **low**. `node/production/assembly.rs` — comment, **low**.
**Regression risks**: (a) the `weight == 0` early return must stay before any pooling, or a fully delegated
producer starts pooling; (b) the `producer_set` read guard must drop before the `&mut self` insert (lock/borrow
ordering); (c) `MAX_PARENTS = 8` FIFO unaffected — one own entry per existing parent, no new eviction pressure.
**Assumptions**: (1) `pooled_commitment` is the only post-AH bit source — confirmed `commit.rs:233-234,156-188`;
(2) no validator recomputes an expected bitfield — confirmed `verify.rs:41-73`, `validation_checks.rs:421-448`;
(3) both production callers are already `&mut self` — confirmed `post_commit.rs:420-421,445`, `assembly.rs:638`;
(4) the fleet is uniformly ≥ v6.27.0 post-AH — **assumed, not probed this session**.
**What I don't understand**: whether any *mainnet* producer runs a local BLS key that mismatches its on-chain
key (mainnet AH is `u64::MAX`, so latent — but it sets how loud REQ-208-002's WARN must be before a mainnet
pin session); and whether the finality gadget (`sync.add_attestation_weight`, `startup.rs:619`) interacts with
pooling — I read no coupling, but I did not read `add_attestation_weight`.
**Out of scope (Won't)**: any change to bit semantics, `presence_root`, the aggregate scheme, or an activation
height; peer-scoring changes (the `Invalid` no-act policy at `ingress.rs:106-109` stands); backfilling bits of
blocks already produced since AH 112,619 (consensus history, immutable); mainnet activation of
`inc_i_178_attestation_bls_activation_height` (separate decision session, HC-6).

## Triage Verdict
```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.85, verified-prior-session)
Reasoning: Root cause is a single missing write on one code path, confirmed at file:line by a
prior session and re-confirmed here (the ONLY pool insert is ingress.rs:88; pooled_commitment at
commit.rs:156-188 sources bits exclusively from that pool; gossipsub does not loop back).
Brittleness 1/5 = LOCALIZED. The verification seam the fix needs already exists (bls_verdict,
ingress.rs:129, &self, one caller). No consensus rule change and no activation height: verify.rs:51
and :66-72 validate the carried body against on-chain keys, so an extra legitimately-signed bit is
accepted bit-identically by upgraded and un-upgraded nodes alike. Blast radius is 2 production files
plus 2 test bindings. Not 1.0: the WARN-dedup mechanism and the finality-weight coupling are unread,
and the reproduction test needs a ProducerSet mutation no existing test performs.
━━━━━━━━━━━━━━━━━━━━━━
```

## Milestones
**M1 — Pool the own attestation behind on-chain-key verification, and correct the comment** (only milestone).
Files: `bins/node/src/node/startup.rs` (verify+pool after signing; `&self` → `&mut self`);
`bins/node/src/node/attestation/ingress.rs` (`fn bls_verdict` → `pub(crate) fn`, body unchanged);
`bins/node/src/node/production/assembly.rs` (comment at :640);
`bins/node/tests/delegated_bond_attestation.rs` (binding mutability);
plus a new reproduction test module under `bins/node/tests/` covering REQ-208-001 and REQ-208-002.
Covers REQ-208-001 … REQ-208-004.

Tests: `bins/node/tests/it/inc_i_208_own_attestation_pooled.rs` (module of `bins/node/tests/it/main.rs` —
`test-binary-gate.sh` refuses new top-level `bins/node/tests/*.rs` targets). Run:
`cargo test -p doli-node --test it inc_i_208_own_attestation_pooled`.
