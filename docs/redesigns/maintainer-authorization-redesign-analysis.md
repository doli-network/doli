# INC-I-176 — Maintainer Authorization Redesign: Problem Scope

**Agent:** analyst · **Run:** 518 · **Date:** 2026-08-12
**Branch:** `bugfix/inc-i-173-state-only-fee-gate` @ `3f8bf185`
**Status:** SCOPE ONLY. No design is proposed here. No code was modified. Nothing was built or run.

> Method note. Every claim below carries a `file:line` or a command's output. Where a tool
> produced a claim, the tool is named. The code graph (`graphify-out/graph.json`, rebuilt this
> session) was queried first for structural questions; `blast.py` emitted its own
> lower-bound warning on receiver-method seeds (Rust `self.method()` blind spot, matches the
> banked `reference_graphify_rust_method_blind_spot` memory), so every caller set below was
> confirmed with a per-root grep and the tool is stated per claim.

---

## 1. Verified Capability Inventory (PRIOR-KNOWLEDGE-GATE)

Nothing in sections 3–7 asserts the system "lacks" anything that is not first enumerated here.

### 1.1 Transaction types — 24 live variants (claim of 24 in CLAUDE.md: CONFIRMED)

`crates/core/src/transaction/types.rs:7-137`. Discriminants 0–22 (23 variants) plus 31
(`ZKSettle`) = **24**. Discriminant 23 is absent entirely (no variant, no tombstone comment).
24–28 and 29–30 are explicitly tombstoned and `from_u32` maps them to `None`
(`types.rs:167-172`).

Relevant subset — the **5** types allowed to be 0-in/0-out and fee-exempt, from the ONE
exhaustive authority `TxType::allows_empty_io()` (`types.rs:180-208`, no `_` arm):
`Registration`, `DelegateBond`, `RevokeDelegation`, `AddMaintainer`, `RemoveMaintainer`.
`RemoveMaintainer = 11`, `AddMaintainer = 12` (`types.rs:38,44`).

### 1.2 Maintainer / governance data structures (`crates/core/src/maintainer/`, 1459 lines)

| Item | Location | Shape |
|---|---|---|
| `MaintainerChangeData` | `data.rs:10-17` | `{ target: PublicKey, signatures: Vec<MaintainerSignature>, reason: Option<String> }` |
| `MaintainerChangeData::signing_message(is_add)` | `data.rs:46-49` | `format!("{}:{}", "add"\|"remove", target.to_hex()).into_bytes()` |
| `ProtocolActivationData` | `data.rs:69-78` | `{ protocol_version, activation_epoch, description, signatures }` |
| `ProtocolActivationData::signing_message()` | `data.rs:99-105` | `format!("activate:{}:{}", version, epoch)` |
| `MaintainerSet` | `set.rs:17-24` | `{ members: Vec<PublicKey>, threshold: usize, last_updated: u64 }` |
| `MaintainerSignature` | `set.rs:368-373` | `{ pubkey, signature }`; `verify` = `crypto::signature::verify(message, sig, pubkey)` (`set.rs:382-384`) |
| `count_distinct_signers` | `set.rs:130-149` | outer loop over **current** `self.members`, inner over signatures, `break` on first match |
| `verify_multisig_at` / `verify_multisig_excluding_at` | `set.rs:262-290` | height-gated on `maintainer_derivation_activation_height` |
| `derive_maintainer_set` (replay) | `derivation.rs:148-221` | zero production callers today (banked as INC-I-172-M2-R1) |
| `maintainer_set_digest` | `digest.rs:77-89` | `BLAKE3(b"DOLI-MAINTAINER-SET-V1" ‖ genesis_hash ‖ threshold_le ‖ sorted members)` |
| `MaintainerState` (persisted) | `crates/storage/src/maintainer.rs:20,70-88` | file `maintainer_state.bin` in the **data dir** |

Constants (`mod.rs:83-92,181,190,198`): `INITIAL_MAINTAINER_COUNT=5`, `MAINTAINER_THRESHOLD=3`,
`MIN_MAINTAINERS=3`, `MAX_MAINTAINERS=5`, `MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES=1024`,
`MAX_MAINTAINER_CHANGE_SIGNATURES=5`, `MAX_MAINTAINER_CHANGE_REASON_BYTES=256`.
`calculate_threshold` (`set.rs:106-116`): 0→3, 1→1, 2→2, **3→2**, 4→3, 5→3.

### 1.3 Activation heights — 21 defined (`crates/core/src/network_params/mod.rs`)

Count and names from `grep -cE '^\s+pub [a-z0-9_]+_activation_height: u64,'` = **21**.
Values from `crates/core/src/network_params/defaults.rs` (mainnet / testnet / devnet):

| # | Field | mainnet | testnet | devnet |
|---|---|---|---|---|
| 1 | `inc_i_026_scheduler_activation_height` | 0 | 0 | 0 |
| 2 | `fork_id_activation_height` | 0 | 0 | 0 |
| 3 | `encrypted_content_activation_height` | 0 | 0 | 0 |
| 4 | `epoch_state_reorg_activation_height` | 0 | 0 | 0 |
| 5 | `security_audit_activation_height` | 0 | 0 | 0 |
| 6 | `encrypted_content_v2_activation_height` | 0 | 0 | 0 |
| 7 | `ghost_exclusion_activation_height` | 0 | 0 | 0 |
| 8 | `epoch_prune_activation_height` | 0 | 0 | 0 |
| 9 | `inc_i_068_weight_filter_activation_height` | 0 | 0 | 0 |
| 10 | `received_delegation_cap_activation_height` | 0 | 0 | `u64::MAX` |
| 11 | `delegation_auth_activation_height` | 0 | 0 | `u64::MAX` |
| 12 | `addbond_cap_enforcement_activation_height` | 0 | 0 | `u64::MAX` |
| 13 | `defi_activation_height` | 0 | `u64::MAX` | `u64::MAX` |
| 14 | `amm_activation_height` | 0 | 0 | 0 |
| 15 | `oracle_activation_height` | `u64::MAX` | `u64::MAX` | `u64::MAX` |
| 16 | `large_block_activation_height` | 0 | 0 | 0 |
| 17 | `inc_i_092_activation_height` | 0 | 0 | 0 |
| 18 | `inc_i_096_activation_height` | 0 | 0 | 0 |
| 19 | `inc_i_147_activation_height` | 129_500 | 80_700 | 0 |
| 20 | `maintainer_derivation_activation_height` | **172_000** | 127_200 | 0 |
| 21 | `inc_i_173_activation_height` | `u64::MAX` | **136_431** | 0 |

INC-I-176 must add **#22**. It may not ride #21 (user constraint) and may not ride #20 (crossed
on testnet at 127_200; INV-PARAMS-001).

### 1.4 Existing anti-replay / uniqueness primitives already in the chain

This is the subtraction inventory — what the redesign may be able to REUSE rather than invent.

| # | Primitive | Location | Scope | Applies to a maintainer tx? |
|---|---|---|---|---|
| P1 | UTXO double-spend (input consumed once) | `crates/core/src/validation/utxo.rs` spend path | chain-wide, permanent | **NO** — maintainer txs have 0 inputs (`tx_types.rs:763-767`) |
| P2 | `txid` determinism | `crates/core/src/transaction/core.rs:483-509` | hash covers version, tx_type, input outpoints, outputs, `extra_data` | txid is a pure function of `extra_data` here |
| P3 | `CF_TX_INDEX` (txid → height) | `crates/storage/src/block_store/queries.rs:105` | chain-wide index | **NO validation reader.** Per-root grep: `crates` → 3 readers, all RPC (`rpc/methods/transaction.rs:37,82`, `rpc/methods/history.rs:48`); `bins` → 0 readers |
| P4 | Mempool hash dedup | `crates/mempool/src/pool.rs:335,718-721` | **process-local**, evicted on mine | partial, and only until mined |
| P5 | Duplicate-registration rejection | `crates/core/src/validation/registration.rs:165-177` | state-precondition (`active_producers` / `pending_producer_keys`) | pattern applies; not wired for maintainers |
| P6 | PriceAttestation epoch binding **inside the signed message** | `crates/core/src/transaction/data.rs:778-785` — `BLAKE3(SIGNING_DOMAIN ‖ pair_id ‖ price ‖ epoch_number)` | per-epoch validity | **precedent** — an anti-replay term already exists in-tree |
| P7 | At most one PriceAttestation per (attester, epoch, pair_id) **per block** | `bins/node/src/node/validation_checks.rs:518-543`, `anyhow::bail!` = FATAL | per-block only | precedent, incl. a fatal reject path |
| P8 | Domain-separation tags on signed payloads | `DELEGATE_BOND_SIGNING_DOMAIN` (`data.rs:295`), `REVOKE_DELEGATION_SIGNING_DOMAIN` (`data.rs:401`), `PriceAttestationData::SIGNING_DOMAIN` (`data.rs:780`) | per-payload-type | maintainer payload has **none** |
| P9 | Genesis-hash network binding | `crates/core/src/maintainer/digest.rs:72-83` | maintainer module already binds `genesis_hash` — for the DIGEST, not for the SIGNATURE | precedent inside the very module |
| P10 | `UniqueIdIndex` (NFT token_id, pool_id) | `bins/node/src/node/apply_block/tx_processing.rs:134-150` | chain-wide, fatal | pattern applies |
| P11 | Output `lock_until` timelock | `crates/core/src/transaction` output model | per-UTXO | N/A (no outputs) |
| P12 | Gossip seen-cache on RAW BYTES | INV-NETWORK-004 | transport, TTL | not an authorization control |
| P13 | Maintainer state-precondition idempotence | `set.rs:298-338` — `add_maintainer` → `AlreadyMaintainer`; `remove_maintainer` → `NotMaintainer` / `MinMaintainersRequired` | state-scoped | **YES — this is the only replay control in force today** |

**Conclusion of the gate:** the chain already owns an in-signed-message anti-replay term (P6),
a domain tag idiom (P8), a network-identity binding inside the same module (P9) and a
state-precondition idempotence rule (P13). The maintainer authorization scheme uses **only**
P13. Nothing needs to be invented from first principles; the question is which existing
primitive to apply and where.

---

## 2. Current Architecture Map

### 2.1 Module boundaries and dependency direction

```
bins/cli, bins/node/src/commands/maintainer.rs  (SIGNERS — produce signature bytes)
        │  produces  MaintainerSignature over signing_message(is_add)
        ▼
crates/rpc/src/methods/governance.rs::submit_maintainer_change   (ADMISSION #1, unauthenticated)
crates/network gossip → bins/node/src/node/validation_checks.rs::handle_new_transaction (ADMISSION #2)
        │  both route 0-in/0-out txs to the FREE system lane
        ▼
crates/mempool/src/pool.rs::add_system_transaction  →  validate_transaction
        │
        ▼
crates/core/src/validation/{transaction.rs:171,174 → tx_types.rs:753} validate_maintainer_change_data
        │  STRUCTURAL ONLY.  Zero signature verification.
        ├──► bins/node/src/node/production/assembly.rs:242   (BLOCK BUILDER)
        └──► bins/node/src/node/apply_block/tx_processing.rs:106  (APPLY, FATAL on Err)
        ▼
bins/node/src/node/apply_block/governance.rs:17-102  process_transaction_governance
        │  THE ONLY signature verification site.  NON-FATAL (warn!, no Err).
        ▼
crates/storage/src/maintainer.rs  MaintainerState → maintainer_state.bin  (NODE-LOCAL FILE)
        ├──► crates/updater/src/trust_root.rs  (release verification — INC-I-172)
        ├──► crates/rpc/src/methods/governance.rs:127,204  (getMaintainerSet + digest)
        └──► bins/node/src/node/maintainer_rewind/{binding,commit}.rs  (INC-I-174 undo)
```

### 2.2 Data flow of one maintainer authorization

1. A maintainer signs the bytes `"add:<64hex>"` or `"remove:<64hex>"`. In the node CLI this is
   `crypto::signature::sign(&message, priv)` over the RAW bytes (`commands/maintainer.rs:158,225`)
   — note: **raw**, not `sign_hash`, unlike `DelegateBondData` (`bins/cli/src/cmd_producer/delegation.rs:124`).
2. Signatures are collected out of band (`commands/maintainer.rs:181-184`: "Share proposal with
   other maintainers → Collect 3/5 → Submit via RPC").
3. `submitMaintainerChange` (`rpc/methods/governance.rs:214-286`) parses hex, checks only
   `signatures.len() >= 3` (`:252`), builds the tx and calls `add_system_transaction`.
4. The tx gossips; every node admits it into the free lane (`validation_checks.rs:930-963`).
5. A producer includes it (`assembly.rs:242` accepts it above `inc_i_173_activation_height`).
6. Every node runs `process_transaction_governance` at apply. Signature check happens **here,
   once, per node**, and its failure is a log line.
7. The resulting `MaintainerSet` is written to `maintainer_state.bin` and consumed by the
   updater's `TrustRoot`.

### 2.3 The decisive structural fact

`compute_state_root` (`crates/storage/src/snapshot.rs:24-58`) hashes exactly three components:
`ChainState::serialize_canonical()`, `UtxoSet::serialize_canonical()`,
`ProducerSet::serialize_canonical()`. A per-root grep for `maintainer` in
`crates/storage/src/snapshot.rs` returns **zero** hits. The maintainer set is **not** in the
state root.

---

## 3. The Defect, Precisely

### Q1 — What bytes are signed today?

`crates/core/src/maintainer/data.rs:46-49`:

```rust
pub fn signing_message(&self, is_add: bool) -> Vec<u8> {
    let action = if is_add { "add" } else { "remove" };
    format!("{}:{}", action, self.target.to_hex()).into_bytes()
}
```

**Committed:** the action (`add`/`remove`, and the verifier derives `is_add` from `tx.tx_type`,
`governance.rs:32,38` / `:68,74`, so action *is* bound) and the 32-byte target public key.

**NOT committed (ride outside the signature):**
- `reason: Option<String>` — up to 256 bytes of attacker-chosen text (`mod.rs:198`) that lands in
  `extra_data` via `to_bytes()` (`data.rs:52-54`) and therefore in the txid (`core.rs:504-506`).
- The **composition and order of the `signatures` vector** itself. Each entry is verified
  individually (`set.rs:141-146`), so extra junk entries do not create authority — but they do
  change `extra_data`, hence the txid.
- Network / chain identity. No genesis hash, no chain id, no network tag.
- Height, slot, epoch, expiry, nonce.
- The maintainer-set version or member list the signers believed they were acting on.
- Any transaction-envelope binding (the signature is not over the tx).

Consequence of the two malleable fields: **an unlimited number of distinct txids can carry the
identical authorization.** Any dedup keyed on txid is defeated by construction. (This is the
"reason malleability defeats one anyway" clause of AUDIT-P1-004, verified.)

Contrast with the two nearest in-tree siblings, both of which already do better:
`DelegateBondData::signing_message` uses a domain tag (`data.rs:293-299`);
`PriceAttestationData::signing_message` uses a domain tag **and** binds `epoch_number`
(`data.rs:778-785`).

### Q2 — Who verifies, where, and what happens on failure?

Enumerated exhaustively. Tool provenance: the graph gave the module edges; the call-site list
below is from per-root grep of `crates` and `bins` for `.signing_message(true|false)` and
`verify_multisig`, because `blast.py` flagged its Rust receiver-method blind spot on these seeds.

| # | Site | File:line | Verifies signature? | Failure is |
|---|---|---|---|---|
| V1 | RPC `submitMaintainerChange` | `crates/rpc/src/methods/governance.rs:252` | **NO** — counts entries `>= 3` only | RPC error, no chain effect |
| V2 | Gossip admission | `bins/node/src/node/validation_checks.rs:930-963` | **NO** | tx not admitted |
| V3 | Mempool `add_system_transaction` → `validate_transaction` → `validate_maintainer_change_data` | `crates/mempool/src/pool.rs:792`; `crates/core/src/validation/tx_types.rs:753-836` | **NO** (structural + F5 bounds only; the doc comment at `:737-738` says so explicitly) | tx not admitted |
| V4 | Block builder | `bins/node/src/node/production/assembly.rs:242` (same validator as V3) | **NO** | tx skipped, block still built |
| V5 | Apply — UTXO/structural | `bins/node/src/node/apply_block/tx_processing.rs:106-131` | **NO** | **FATAL** — `return Err` → block rejected (except `ValidationMode::Replay`) |
| V6 | Apply — governance | `bins/node/src/node/apply_block/governance.rs:39-44` (add) and `:75-81` (remove) | **YES** — `verify_multisig_at` / `verify_multisig_excluding_at` | **NON-FATAL** — `warn!("[MAINTAINER] Rejected …")`, `process_transaction_governance` returns `Option`, never `Result`; the block applies |
| V7 | Replay derivation | `crates/core/src/maintainer/derivation.rs:191,204` | YES | `let _ =` — silently skipped. **Zero production callers today** (INC-I-172-M2-R1) |
| V8 | Updater `TrustRoot` | `crates/updater/src/trust_root.rs:98` `is_usable` | consumes the resulting SET; does not verify the authorization | fail-closed refusal to install |

**This is the crux, and it inverts one of the incoming assumptions.** The user's stated
constraint "keep the apply-path NON-FATAL" is **already the status quo** — V6 has been non-fatal
since it was written. The real hazard is the opposite one: the only *fatal* path for these
transactions is V5, which today performs **no** authorization check at all. Moving any part of
the authorization predicate into the shared validator (V3/V4/V5, one function, three call sites)
converts it into a block-reject rule and creates exactly the fleet-splitting path the
constraint forbids — unless it is gated.

INV-VALIDATION-001 sharpens this: V3 (mempool, `current_height` = tip) and V4 (builder, block
height) and V5 (apply, block height) evaluate the *same* function with *different* heights and a
strictly weaker builder context. A predicate that depends on height therefore has a built-in
one-block skew between admission and apply.

### Q3 — Is the maintainer set in the state root?

**No — and the transaction is, which is the whole asymmetry.**

- **Not in the state root:** `compute_state_root` (`snapshot.rs:24-58`) covers ChainState + UtxoSet
  + ProducerSet only; zero `maintainer` hits in that file. The set lives in `maintainer_state.bin`
  in the data dir (`crates/storage/src/maintainer.rs:20,233`) and is described in-tree as
  "unsigned and attacker-writable" (`crates/storage/src/lib.rs:184`,
  `crates/storage/src/maintainer_wellformed.rs:17`).
- **Also not state-root-visible indirectly:** the one downstream consumer that touches ChainState is
  `ProtocolActivation` → `active_protocol_version` (`apply_block/state_update.rs:64-83`), and
  `ChainState::serialize_canonical` is a fixed 140-byte buffer (`crates/storage/src/chain_state.rs:143-155`)
  that contains neither `active_protocol_version` nor `pending_protocol_activation`. This is
  already documented and was already corrected once
  (`network_params/mod.rs:571-583`, INC-I-172 M2 QA OBS-004).
- **In the state root / consensus-visible:** the **transaction** is. It is committed by the block's
  transaction list and its presence/absence decides block validity through V5.

So the precise split: **the authorization TRANSACTION is consensus data; the authorization EFFECT
is node-local.** A rule about *which transactions may appear in a block* is a consensus rule. A
rule about *which authorizations take effect* is, today, not.

The immediately relevant precedent is INV-AUTH-001 (active): *"An authorization rule that is
applied at every height, with no activation gate, may only remain ungated while its outcome is
provably invisible to consensus. The moment the outcome becomes state-root-visible or is read by
a consensus rule, the rule must be re-derived under an activation height."* And INC-I-172 already
took the conservative branch here — it gave the maintainer authorization change its own height
(#20) despite the outcome not being state-root-visible, on the explicit ground that "currently
unused is never a valid skip".

### Q4 — The exact replay attack

An archived authorization `A = (action, target, sigs)` — every byte of which is public, both in
the mempool and permanently in the block that carried it — is **effective again** at any future
height `H` iff both hold:

1. **Endorsement still counts.** `count_distinct_signers` (`set.rs:130-149`) iterates the
   **current** members `S_H`. So `A` needs `>= threshold(|S_H|)` of its original signers to still
   be members (for `remove`, excluding the target: `set.rs:176-186`).
2. **State precondition holds again.** For `add`: `|S_H| < 5` and `target ∉ S_H` (`set.rs:298-307`).
   For `remove`: `|S_H| > 3` and `target ∈ S_H` (`set.rs:322-331`).

Nothing else stands in the way. P3 (the txid index) has no validation reader; P4 (mempool dedup)
is process-local and evicts on mine; and even a hypothetical txid dedup is defeated by mutating
`reason`.

**Leg 1 — cross-effect / revocation reversal (the sharpest).** Governance adds maintainer `X`
(blob `add:X`, signed by M1,M2,M3). Later governance removes `X` because `X` is compromised.
`S` now has 4 members, `X ∉ S`, and M1,M2,M3 are still members. **Any** party — no key required —
replays the archived `add:X` payload through `submitMaintainerChange` (which never checks who is
calling) and `X` is a maintainer again. The revocation is undone by a stranger. Confirmed
reachable: V1 does not authenticate the caller, V3/V4/V5 do not check signatures, V6 accepts
because both conditions hold.

**Leg 2 — cross-time / reorg resurrection (armed by INC-I-174, verified this session).** INC-I-174
(`3f8bf185`) correctly rewinds the maintainer trust root when a block is rolled back
(`bins/node/src/node/rollback.rs:99-105,333-349`; `maintainer_rewind/`). The rewind restores the
member list — it cannot un-publish the authorization blob, which the attacker already captured
from gossip or from the orphaned block. So a rotation that a reorg undoes can be **re-applied at
any future height by anyone**. This is a direct interaction between the two incidents: INC-I-174
made the *effect* reversible while INC-I-176 leaves the *authorization* permanent.

**Leg 3 — cross-network (MEASURED, live today).** `crates/updater/src/constants.rs:53-64` vs
`:75-86`: `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` and `BOOTSTRAP_MAINTAINER_KEYS_TESTNET` are
byte-identical in all five entries. `signing_message` binds no chain identity. Therefore an
authorization produced on testnet — where the corresponding private keys are committed to a
public repository (INC-I-175) — is byte-valid on mainnet and vice versa. The maintainer module
already knows this and already fixed it in the *digest* (`digest.rs:72-76` cites exactly this
byte-identity as the reason `genesis_hash` is in the digest preimage) — but not in the signature.

**Leg 4 — the "3-of-5 → 2-of-3 ratchet": I must narrow the incident's claim.** Reading the code,
a replayed `remove:` can only fire when its target is a **current** member (`set.rs:330`), and
removals stop at `|S| = 3` (`set.rs:64-66,328`). After a normal rotation the removed keys are no
longer members, so their archived removals are inert. The ratchet is therefore **not**
unconditional; it requires a history in which a maintainer is removed and later re-added — which
Leg 1 supplies, since Leg 1 lets an attacker re-add a removed key at will. The composed attack is:
replay `add:X` (Leg 1) to put `X` back in `S`; the archived `remove:X` is now live again; replay
it. Repeat with a second such key and `|S|` reaches 3, where `calculate_threshold(3) = 2`
(`set.rs:110`) — a permanently weaker quorum, since the set can only climb back to 5 through
adds that themselves require the (now weaker) quorum.

⚠ **HONESTY FLAG.** I can construct the ratchet only as a *composition* of Leg 1 with an archived
removal, not as the standalone "replay two historical removals" the incident description states.
The end state (2-of-3) is reachable; the stated path is not sufficient on its own. AUDIT-P1-016
independently reached a compatible narrowing ("a replay dies on endorser turnover"). The
redesign's threat model should be written against Legs 1–3, with the ratchet as a derived
consequence, not as the primary claim.

**Arming asymmetry (verified, and it matters for INC-I-175).** The five bootstrap keys were
seeded through `MaintainerSet::with_members` (`set.rs:49-56`; the seed path in
`bins/node/src/node/periodic.rs`), **not** `add_maintainer`. No `add:` blob exists on-chain for
them. So the currently-exposed mainnet five are not re-addable by replay today. Exposure arms
with the **first governance-authorized add** — i.e. with the INC-I-175 rotation itself.

### Q5 — What already blocks a naive replay?

Only **P13**, the state-precondition idempotence in `set.rs:298-338`. A byte-identical
resubmission of an already-applied authorization is a no-op *while the state stays put*
(`AlreadyMaintainer` / `NotMaintainer` → `warn!`, `governance.rs:58,95`).

Everything else is absent, and I verified each absence with a positive control rather than
inferring it:
- **No txid uniqueness enforcement.** `CF_TX_INDEX` exists and is populated
  (`block_store/writes.rs:49`), but per-root grep finds its only readers in `crates` are three RPC
  methods (`rpc/methods/transaction.rs:37,82`, `rpc/methods/history.rs:48`) and **zero** in `bins`.
  Positive control that the grep works: the same scan finds the writer and the migrator.
- **No UTXO-based uniqueness.** The transaction has 0 inputs by rule (`tx_types.rs:763-767`), so
  P1 cannot apply. This is structural, not accidental.
- **No nonce, expiry or epoch on the payload.** `MaintainerChangeData` has exactly three fields
  (`data.rs:10-17`).

**Therefore the answer to "which replay is actually open" is: both.** The byte-identical replay is
open (nothing rejects it; only the state precondition mutes it), and the re-wrapped replay is also
open and strictly stronger — mutating `reason` yields a fresh txid, so even if a txid-uniqueness
rule were added it would be bypassed. **This sizes the fix: a txid/dedup-shaped remedy is
insufficient on its own; the malleable fields must be closed in the same change or the dedup is
theatre.**

---

## 4. Blast Radius (Q6)

Graph query (`python3 .claude/scripts/blast.py graphify-out/graph.json MaintainerChangeData --hops 2`):
28 dependents, 1 seed node matched exactly. Graph query on `verify_multisig_at` returned 1
dependent **with an explicit lower-bound warning** for Rust receiver-methods — so the tables below
are grep-confirmed per root.

### 4.1 Everything that produces or consumes the signed bytes

| Role | Location | Breaks if `signing_message` changes? |
|---|---|---|
| SIGNER — node CLI `maintainer remove` | `bins/node/src/commands/maintainer.rs:157-158` | **YES** |
| SIGNER — node CLI `maintainer add` | `bins/node/src/commands/maintainer.rs:224-225` | **YES** |
| SIGNER — out-of-tree Python harness | `…/scratchpad/sign_maintainer.py:25-26` (`msg = f"{action}:{target_hex}".encode()`) — two copies found under `/private/tmp/claude-501/**/scratchpad/` | **YES**, silently — it hardcodes the format |
| VERIFIER — apply | `bins/node/src/node/apply_block/governance.rs:38,74` | YES (must change in lockstep) |
| VERIFIER — replay derivation | `crates/core/src/maintainer/derivation.rs:190,203` | YES |
| DOMAIN-SEPARATION TEST | `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs:301,119` — asserts a release-signing message is NOT confusable with `signing_message(true)` | must be re-derived |
| TRANSPORT — RPC submit | `crates/rpc/src/methods/governance.rs:214-286` | payload shape only |
| TX CONSTRUCTORS | `crates/core/src/transaction/core.rs:741-783` (`new_remove_maintainer`, `new_add_maintainer`), `:801-806` (`maintainer_change_data`) | if the payload gains fields, both constructors change |

### 4.2 Everything that depends on the payload ENCODING (`MaintainerChangeData::to_bytes`)

Adding a field to the struct changes `extra_data` bytes, hence the txid, hence the 873-byte
maximal-payload figure pinned by
`req_173_014_maximal_legal_payload_fits_under_the_outer_cap`
(`crates/core/tests/inc_i_173_m3_payload_bounds.rs:238`; the intent is stated at
`crates/core/src/maintainer/mod.rs:166-171` — "Add a field to the payload and that test fails").
That test is a deliberate tripwire and **will** fire.

### 4.3 Tests that construct maintainer payloads (each must be re-derived)

Per-root grep for `MaintainerChangeData`. `bins` → `inc_i_172_m2_fail_close.rs:160`,
`inc_i_172_m2_maintainer_reset.rs:192`, `inc_i_173_m3_f4_routing.rs:444`,
`inc_i_173_state_only_fee_gate.rs:122`, `inc_i_174_maintainer_reorg.rs:184`,
`inc_i_174_maintainer_rewind_guards.rs:155`, `inc_i_174_maintainer_undo.rs:206`,
`inc_i_174_maintainer_undo_capture.rs:195`, `inc_i_174_snapshot_binding.rs:177`.
`crates` → `inc_i_172_m2_canonical_derivation.rs:466`,
`inc_i_172_m2_maintainer_governance.rs:394,422,430,475,483`,
`inc_i_173_common/mod.rs:267`, `inc_i_173_m3_payload_bounds.rs:158`,
`crates/core/src/maintainer/tests.rs:234,245`.
**Total: 9 test files in `bins`, 5 in `crates`.**

### 4.4 Indirect consumers of the RESULT (unchanged bytes, changed set)

`crates/updater/src/trust_root.rs` (release verification, fail-closed at `:98`);
`crates/rpc/src/methods/governance.rs:127,204` (`getMaintainerSet` + digest);
`bins/node/src/node/maintainer_rewind/{binding,commit}.rs` (INC-I-174 undo);
`crates/storage/src/{maintainer.rs,maintainer_journal.rs,maintainer_wellformed.rs}`;
`bins/cli/src/cmd_upgrade.rs` (AUDIT-P1-012, still pins `TrustRoot::bootstrap`).

### 4.5 Mixed-fleet hazard, stated plainly

If the payload encoding changes, a new-binary node builds a transaction an old-binary node
**cannot decode** (`from_bytes` → `None` → `governance.rs:34,70` silently skips the whole block).
Old nodes would not reject the block (V6 is non-fatal) — they would simply **not apply the
rotation**, producing a fleet where some nodes hold the new trust root and some hold the old.
That is the INC-I-172 failure shape (divergent install roots) reintroduced through the encoder.
This is a **block-content** consideration even though it is not a block-validity one.

---

## 5. Constraints & Invariants

### 5.1 Must not be regressed (upstream work on this branch)

| Incident | What must keep working | Where |
|---|---|---|
| INC-I-173 (M1 `32e0a650`, M3a `0d46959e`, M2 `7f917e7a`) | `AddMaintainer`/`RemoveMaintainer` remain **mineable** above `inc_i_173_activation_height`; F5 payload bounds still reject; testnet pin stays 136_431 | `validation/utxo.rs:239-249`, `tx_types.rs:783-826` |
| INC-I-174 (`3f8bf185`) | Maintainer undo still captured and replayed on rewind; `UndoData` bincode encoding still byte-unchanged (decision 68) | `apply_block/mod.rs:188,368-370`, `rollback.rs:99-105,333-349` |
| INC-I-172 (`b5f68bba`) | `TrustRoot` still fails **closed**; `verify_multisig_at` still reproduces pre-gate history bit-identically below `maintainer_derivation_activation_height` | `trust_root.rs:98`, `set.rs:262-290` |

### 5.2 Eliminated evidence — do not re-propose

**Binding the signed message to node-local `last_change_block` / `MaintainerSet::last_updated`
(INC-I-173 M3 Option E).** MEASURED fleet-divergent: RPC 8512 reported `last_change_block = 88289`
while 12 peers reported `1`, all at identical tip 134_682, all holding the same five members and
threshold (recorded at `crates/core/src/maintainer/digest.rs:47-62`, decision 67, finding
REV-173-M3a-002). Any binding to node-local, non-consensus state is dead. The distinguishing
test for a candidate binding: **can two honest nodes at the same tip compute different values for
it?** If yes, it is the same failure. `genesis_hash`, block height, and the transaction's own
bytes pass that test; anything derived from `maintainer_state.bin` does not.

### 5.3 INC-I-175 rotation — the hard non-foreclosure constraint (Q7)

From `docs/bugfixes/inc-i-175-remediation-plan.md:17-34`, the rotation shape is fixed by
`MAX_MAINTAINERS=5` / `MIN_MAINTAINERS=3`: fresh keys **cannot** be added beside the old ones, so
the sequence must alternate Remove → Add, one transaction at a time, and the outgoing (public,
exposed) keys hold **exactly equal on-chain authority** until removal #3 lands. The plan's own
conclusion: *"the whole sequence must be pre-staged, signed, and executed as one uninterrupted,
human-gated run — not spread over days."*

What that requires of any new authorization scheme:

1. **Pre-signing must remain possible.** Keys are to be air-gapped (`:46-47`). If the signed
   message binds a value that is unknown until broadcast time (an exact block height, or a
   set-version that the adversary can bump), each step must be re-signed on the air-gapped host
   mid-run, while an equally-authorized adversary is racing. That converts a hardening into an
   operational failure mode.
2. **An adversary with equal authority must not be able to invalidate the pre-staged bundle.**
   Until removal #3, the attacker can issue his own maintainer transactions. A strictly monotonic
   counter binding hands him a free, zero-cost griefing primitive: bump the counter, and every
   pre-signed step in the defender's bundle is void. A window/expiry binding has the mirror
   weakness (he can stall).
3. **Ordering must survive.** The steps are not commutative — `Add fresh #3` is only legal after
   `Remove exposed #3`.
4. **The rotation is what ARMS Leg 1.** It creates the first governance-authorized `add:` blobs on
   mainnet. Any scheme that leaves those blobs replayable makes every fresh key
   permanently re-addable by a stranger once it is ever removed.

This is a genuine tension between the security property and the operational requirement, and it
is the single most consequential thing the design must resolve. It is raised as Open Question 8.1.

### 5.4 Other binding constraints

- Own new forward-only activation height in `crates/core/src/network_params/` (#22). May not ride
  #20 (crossed on testnet at 127_200) or #21 (user constraint, and crossed on testnet at 136_431).
  INV-PARAMS-001: never move a crossed height forward.
- No genesis reset (CLAUDE.md rule #0). No version bump of any kind without explicit approval
  (MEMORY.md rule #0). `CURRENT_PROTOCOL_VERSION` in particular must not move (INV-EPOCH-001).
- No mainnet action, no `git push`, user gate before implementation.
- Testnet is LOCAL (`~/testnet/`, `scripts/testnet.sh`, 127.0.0.1). Never SSH the remote mainnet hosts.
- Module size budget: `crates/core/src/maintainer/` files are 89–443 lines; the 500-line ceiling
  applies (Rule 19).
- INV-VALIDATION-001: any new consensus rule reachable in `apply_block` must be reachable from
  the mempool AND the builder with an equivalently-populated context, and must be locked by tests
  driving all three paths to the same verdict.
- `.omega/gauntlet.conf` exists → Rule 29 (system-impact) applies: failure-mode matrix at
  briefing, `Failure-Modes:` commit block, gauntlet run before close.

---

## 6. INV-12 Classification

### The three questions (CLAUDE.md, INV-CONSENSUS-001)

**Q1. Can a user-submittable transaction reach this path? — YES.**
`submitMaintainerChange` (`crates/rpc/src/methods/governance.rs:214`) performs **no** caller
authentication and **no** signature verification; it checks `signatures.len() >= 3` (`:252`) and
calls `add_system_transaction`. Independently, any gossiped `AddMaintainer`/`RemoveMaintainer`
reaches the free system lane via `handle_new_transaction` (`bins/node/src/node/validation_checks.rs:948-957`).
Above `inc_i_173_activation_height` such a transaction is mineable (`validation/utxo.rs:239-249`).

**Q2. Can a producer-action or attestation pattern reach it? — YES.**
The block builder includes the transaction (`bins/node/src/node/production/assembly.rs:242`) and
every node executes `process_transaction_governance` for it at apply
(`bins/node/src/node/apply_block/mod.rs:224-230`).

**Q3. Is the new behavior bit-identical for ALL reachable inputs? — NO.**
The stated purpose is to make some currently-effective authorizations ineffective. Whatever the
binding, the accept/reject verdict changes for at least one reachable input; otherwise the
redesign does nothing.

**(Q1 | Q2) YES + Q3 NO ⇒ ACTIVATION HEIGHT REQUIRED.** No exception applies. "Currently unused"
is not available as a skip, and the INC-I-172 precedent for this exact module already took the
gated branch on weaker grounds (`crates/core/src/network_params/mod.rs:571-583`).

### The two deploy questions (CLAUDE.md "After Every Modification", INV-DEPLOY-001 / INV-8)

**D1. Does this change consensus RULES?** — **Conditional, and the design decides it.**
- If any part of the predicate lands in `validate_maintainer_change_data`
  (`crates/core/src/validation/tx_types.rs:753`), then **YES**: that function is reached from
  apply at `tx_processing.rs:106` where failure is `return Err` → the block is rejected. A block
  containing a non-conforming maintainer transaction becomes invalid. Fatal, fork-capable,
  activation height mandatory, and INV-VALIDATION-001 three-path parity mandatory.
- If the predicate lands only in `process_transaction_governance`
  (`bins/node/src/node/apply_block/governance.rs`), then **NO** in the state-root sense: block
  validity is untouched and V6 stays non-fatal. But INV-AUTH-001 still forces the height gate,
  because pre-gate replay must remain bit-identical for history and because the outcome is read
  by the release-install path.
- **This choice is the central design decision and I deliberately do not make it here.** See 8.2.

**D2. Does this change block CONTENT?** — **YES if the payload encoding changes; otherwise no.**
`MaintainerChangeData` is serialized into `extra_data` (`data.rs:52-54`), which is committed by
`Transaction::hash()` (`core.rs:504-506`). Adding a signed field changes the bytes of every future
maintainer transaction. Per INV-DEPLOY-001 / INC-I-062 that implies a **synchronized deploy**
(stop ALL, then start ALL) rather than a rolling restart — and §4.5's silent-skip divergence is
the concrete reason, not a formality. If the redesign changes only *which* bytes are signed while
leaving the wire struct alone (e.g. by binding values the verifier already has), D2 is NO and a
rolling deploy above the gate is sufficient. That is a real, load-bearing difference in
deployment cost.

**Mitigating fact, verified in-tree:** no block on any network currently contains an
`AddMaintainer` or `RemoveMaintainer`. The reasoning and the testnet scan are recorded at
`crates/core/src/maintainer/mod.rs:107-144`. I did **not** re-run that scan this session (see §9).

---

## 7. Redesign Acceptance Criteria

Traceability: `REQ-176-NNN`. Test-writer, architect, developer, QA and reviewer all key off these IDs.

### 7.1 Preservation (Must)

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-176-001 | Legitimate maintainer governance still works end to end | Must | - [ ] Given a 5-member set with threshold 3 and a validly signed `RemoveMaintainer` under the NEW rule, when submitted at height ≥ AH-176, then it is mined AND applied AND `getMaintainerSet` shows 4 members on every node<br>- [ ] Same for `AddMaintainer` from a 4-member set<br>- [ ] The equivalent of the INC-I-173 e2e (RemoveMaintainer mined + applied consensus-wide) passes on the local testnet |
| REQ-176-002 | INC-I-173 is not regressed | Must | - [ ] `AddMaintainer`/`RemoveMaintainer` remain fee/balance exempt above `inc_i_173_activation_height` via `is_zero_flow()`<br>- [ ] The below-gate expression in `validation/utxo.rs:242-248` stays CHARACTER-IDENTICAL<br>- [ ] F5 bounds (1024 / 5 / 256) still reject; `req_173_014_maximal_legal_payload_fits_under_the_outer_cap` is green (re-derived if the payload grew)<br>- [ ] `inc_i_173_activation_height` values are unchanged on all three networks |
| REQ-176-003 | INC-I-174 rollback undo is not regressed | Must | - [ ] `UndoData` bincode encoding remains byte-unchanged (decision 68)<br>- [ ] A reorg across a maintainer change still restores the prior set in both rewind paths<br>- [ ] All 5 `inc_i_174_*` test files in `bins/node/tests/` pass unmodified except for signing-shape updates |
| REQ-176-004 | INC-I-172 fail-closed TrustRoot still fails closed | Must | - [ ] `TrustRoot::is_usable()` still returns false for an empty or sub-threshold set<br>- [ ] `verify_multisig_at` below `maintainer_derivation_activation_height` is bit-identical to today for every reachable input<br>- [ ] No release becomes installable that is not installable today |

### 7.2 The security property (Must) — stated falsifiably

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-176-010 | **Single-use.** An authorization that has taken effect once can never take effect a second time, on any chain, at any height, regardless of intervening state changes | Must | - [ ] TEST: apply `add:X` (valid); remove `X` by a fresh valid authorization; re-submit the ORIGINAL `add:X` payload byte-for-byte at height ≥ AH-176 → `X` is NOT a maintainer afterwards, on every node<br>- [ ] TEST: the same with `reason` mutated to 1 byte, 256 bytes, and invalid UTF-8-adjacent content → still no effect (closes the txid-malleability bypass)<br>- [ ] TEST: the same with signature entries reordered and with duplicate entries appended → still no effect |
| REQ-176-011 | **Network-scoped.** An authorization valid on network A is invalid on network B | Must | - [ ] TEST: build a valid authorization against the testnet genesis hash; submit it on a devnet chain with a different genesis hash → no effect<br>- [ ] TEST: the byte-identity of `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` and `_TESTNET` (`constants.rs:53,75`) is no longer sufficient to make an authorization portable — asserted as a named test, not a comment |
| REQ-176-012 | **Effect-scoped.** A signature authorizing one concrete change cannot authorize a different change | Must | - [ ] TEST: a signature over an `add` for target X is rejected in a `RemoveMaintainer` tx (already true; must stay true — lock it)<br>- [ ] TEST: a signature over an authorization for target X cannot be transplanted to target Y<br>- [ ] TEST: every field that influences the applied effect is inside the signed bytes — asserted by a field-count/round-trip test that FAILS when a field is added to `MaintainerChangeData` without being signed |
| REQ-176-013 | **Reorg-durable.** An authorization whose block is reorged away cannot be re-applied later by a party that does not hold current maintainer keys | Must | - [ ] TEST: apply a maintainer change; roll back past it (INC-I-174 path); re-submit the identical payload at a later height → no effect |

### 7.3 Safety constraints (Must)

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-176-020 | **No new fleet-splitting reject path.** No node may reject a block that another honest node at the same height accepts, on account of this change | Must | - [ ] The enforcement point is named explicitly in the design, with its fatality stated (V3/V4/V5 shared validator = FATAL; V6 governance = NON-FATAL)<br>- [ ] If any part lands in the shared validator: mempool, builder and apply are driven with the SAME transaction in one test and asserted to reach the SAME verdict (INV-VALIDATION-001), including the one-block height skew between mempool (`current_height` = tip) and apply (block height)<br>- [ ] TEST: a mixed-height fleet crossing AH-176 produces no divergent block verdict |
| REQ-176-021 | **Own forward-only activation height** `inc_i_176_*_activation_height` in `crates/core/src/network_params/` | Must | - [ ] A new field is added (making 22); it is NOT `inc_i_173_activation_height` and NOT `maintainer_derivation_activation_height`<br>- [ ] Mainnet default `u64::MAX` until a separate pinning decision; testnet pinned strictly ABOVE the live tip measured at pin time; devnet `0`<br>- [ ] Below the gate, behavior is bit-identical to today for every reachable input, proven by a below-gate test pair<br>- [ ] A constant gate is used, never a `HardForkSchedule` entry |
| REQ-176-022 | **No genesis reset, no version bump** | Must | - [ ] `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION` and all `Cargo.toml` versions are unchanged in the diff<br>- [ ] No activation height that is already crossed on any network is moved |
| REQ-176-023 | **INV-12 + both deploy questions answered in the commit message** | Must | - [ ] The commit carries the three-question block with evidence<br>- [ ] It states whether consensus RULES changed and whether block CONTENT changed, and the required deploy shape (rolling vs synchronized) follows from the answer, not from convenience |
| REQ-176-024 | **The mixed-fleet silent-skip is addressed explicitly** | Must | - [ ] The design states what an OLD binary does with a NEW-format maintainer transaction, and what a NEW binary does with an OLD-format one<br>- [ ] If either silently skips, the divergence window is bounded by the activation height and the deploy order is specified |

### 7.4 Structural improvements (Should)

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-176-030 | Exactly ONE implementation of the maintainer-authorization predicate, reachable from all verification sites | Should | - [ ] After the change, a per-root grep finds one function computing the signed message and one computing the verdict<br>- [ ] The 3 production verification sites (`governance.rs:38`, `governance.rs:74`, `derivation.rs:190/203`) call it; none re-derives the format<br>- [ ] The out-of-tree `sign_maintainer.py` is either replaced by a supported in-repo signing command or documented as unsupported |
| REQ-176-031 | Reuse an existing in-tree primitive rather than inventing one, or record why not | Should | - [ ] The design names which of P1–P13 (§1.4) it reuses<br>- [ ] If it introduces a new mechanism, it states which existing primitive it rejected and why, in one sentence each |
| REQ-176-032 | **NON-FORECLOSURE.** The result must not recreate the dead end being escaped, and must not foreclose the INC-I-175 rotation | Should | - [ ] The design passes the §5.2 test: two honest nodes at the same tip must compute the same value for every bound term<br>- [ ] The design walks the exact 6-step Remove→Add rotation of `inc-i-175-remediation-plan.md:25-31` and states, per step, whether the authorization can be signed in advance on an air-gapped host<br>- [ ] The design states whether an equally-authorized adversary can invalidate a pre-staged bundle at zero cost, and if so, what the operator's counter is<br>- [ ] The known evolution path (per-authorization expiry, or set-version binding, or a proposal-commitment scheme) is NAMED and explicitly NOT built now |
| REQ-176-033 | The `reason` field is resolved one way or the other | Should | - [ ] Either `reason` is inside the signed bytes, or it is removed from the payload — it does not remain an unsigned, txid-mutating, fee-exempt free-text field<br>- [ ] Whichever is chosen, the F5 byte cap is re-derived against the real encoder, not restated in prose |
| REQ-176-034 | Observability | Should | - [ ] A rejected authorization emits a greppable token distinguishing "insufficient signatures" from "replay/binding failure"<br>- [ ] `MAINTAINER_SET_DIGEST` (`governance.rs:167-181`) still publishes after every applied rotation |

### 7.5 Could

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| REQ-176-040 | Align the maintainer signing primitive with its siblings (`sign_hash` + domain tag, as `DelegateBondData` and `PriceAttestationData` do) | Could | - [ ] If adopted, a cross-payload confusion test asserts no maintainer message is confusable with a delegation, attestation, or release-signing message |
| REQ-176-041 | Wire `derive_maintainer_set` (the replay-complete derivation, zero production callers) so the node's set is reconstructible from the chain | Could | - [ ] Explicitly deferred if not done; it is INC-I-172-M2-R1 / M3 scope, not this incident |

### 7.6 Won't (this iteration)

- The INC-I-175 key rotation itself. This redesign is a prerequisite for it, not part of it.
- Any mainnet action: no pinning of the mainnet AH-176 value, no deploy, no push.
- INC-I-171 (vesting penalty unenforced) — independent.
- INC-I-172-M2-R1 (the `rm maintainer_state.bin` re-seed hole) and M3 replay-derived state.
- AUDIT-P1-012 (`cmd_upgrade.rs` pinning `TrustRoot::bootstrap`) — separate finding.
- AUDIT-P1-015 (`producer_count_fn` `unwrap_or(0)` veto collapse) — separate finding.
- Moving the maintainer set into the state root. That is a far larger consensus change; if the
  design wants it, it is a separate incident with its own height.
- Any change to `ProtocolActivationData::signing_message`, which has the identical defect shape
  (`data.rs:99-105`: `"activate:{version}:{epoch}"`, no chain id, no expiry). **Flagged here so it
  is not lost** — it is out of scope but it is the same bug in the same file.

### 7.7 Traceability — Implementation Module (milestone M1a, 2026-08-12)

M1a is the **signing-message** milestone only. `MaintainerChangeData` moves **zero bytes**: the payload
work that §7.4 REQ-176-033 anticipated was moved to a new milestone **M2.5** after the architecture spec's
vacuity premise was falsified by measurement (testnet block 136_690 carries a real `add_maintainer`
payload; its decoder is consumed fatally and ungated in the shared validator). Test IDs are in
`docs/.workflow/inc-i-176-M1a-output-contract.md` §6.

| REQ | M1a status | Implementation Module @ file path |
|---|---|---|
| REQ-176-002 | MET (by non-regression) | *no change* — `crates/core/src/validation/tx_types.rs` and `crates/core/tests/inc_i_173_m3_payload_bounds.rs` are byte-identical to `3f8bf185`; all three F5 caps (1024 / 5 / 256) remain in force |
| REQ-176-003 | MET (stronger than written) | *no change* — the five `bins/node/tests/inc_i_174_*.rs` suites pass **fully unmodified**, no signing-shape update was needed (25 tests) |
| REQ-176-011 | MET (mechanism) | `signing_message_preimage` / `signing_message` @ `crates/core/src/maintainer/authmsg.rs` — `genesis_hash` is inside the preimage |
| REQ-176-012 | MET (mechanism) | `signing_message_preimage` @ `crates/core/src/maintainer/authmsg.rs` — `[is_add as u8]` and `target.as_bytes()` are both inside the preimage |
| REQ-176-020 | MET | *no new reject path* — `crates/core/src/validation/tx_types.rs` unchanged; the wire format is frozen, so no block that any binary accepts today becomes undecodable |
| REQ-176-021 | PARTIAL — mechanism only | `signing_message_at` @ `crates/core/src/maintainer/authmsg.rs` (the `>=` dispatch). **No height is pinned in M1a**; the `NetworkParams` field is M2's |
| REQ-176-022 | MET | enforced by the diff — no `crates/core/src/network_params/` edit, no version constant, no `HardForkSchedule` entry |
| REQ-176-024 | MET (stated) | the mixed-fleet answer is now trivial: M1a emits and accepts only the legacy format, so old and new binaries are indistinguishable on the wire. The non-trivial case belongs to M2/M2.5 |
| REQ-176-030 | **PARTIAL** — MET in M1a for the in-tree signing-message implementations (acceptance bullets 1 and 2 of §7.4); acceptance bullet 3 (`sign_maintainer.py` replaced by a supported in-repo signing command, or documented unsupported) is **M4 scope (REQ-176-032 sequence) and remains OPEN** — the script is absent from the tree but its references are not retired, so REQ-176-030 is **not fully closed** | `signing_message_legacy` @ `crates/core/src/maintainer/authmsg.rs` = the ONE owner; callers: `MaintainerChangeData::signing_message` (delegate) @ `crates/core/src/maintainer/data.rs`, and BOTH replay arms @ `crates/core/src/maintainer/derivation.rs`. Named exception: five `inc_i_174_*` test files still re-derive the format inline, held by REQ-176-003 |
| REQ-176-031 | MET | `Hasher::new()` (the `crates/core/src/maintainer/digest.rs` house idiom) @ `crates/core/src/maintainer/authmsg.rs`; leaf discipline — genesis as `&[u8]`, height and `valid_before` as plain `u64` |
| REQ-176-033 | DEFERRED to M2.5 | *not implemented* — `reason` is retained and FROZEN. See the CORRECTION box in `specs/maintainer-authorization-architecture.md` |
| REQ-176-040 | MET (primitive + golden vector) | `MAINTAINER_AUTH_DOMAIN` + `GOLDEN_AUTH_*` @ `crates/core/src/maintainer/authmsg.rs`; confusability vs the release-signing and set-digest families is test-proven |
| REQ-176-001, -004, -010, -013, -023, -032, -034, -041 | OUT OF M1a SCOPE | need production wiring (M2), the payload field (M2.5), or a pinned height (M4) |

---

## 8. Open Questions for the User

**8.1 (highest stakes) — pre-staging vs anti-replay.** The INC-I-175 rotation requires the whole
Remove→Add sequence to be signed in advance on air-gapped hosts and executed as one uninterrupted
run, while an adversary holds equal authority until removal #3
(`inc-i-175-remediation-plan.md:20-34`). Any binding to a value that is unknown at signing time,
or that an adversary can advance, makes pre-staging impossible or grief-able. **Which do you
prefer to give up: the ability to pre-sign the full rotation offline, or the strength of the
anti-replay binding for the duration of the rotation?** A third option exists — make the binding
predictable-but-adversary-controllable and accept that a griefed bundle must be re-signed — but
that trades a security property for an operational one and is your call, not mine.

**8.2 — where should the predicate be enforced?** Shared validator (V3/V4/V5) makes it a real
consensus rule with a fatal reject path and full mempool/builder/apply parity obligations; the
governance apply path (V6) keeps it non-fatal but leaves the outcome node-local, which is exactly
the property that made INC-I-172/174 hard. My reading of the evidence favours one of these, but
per the analyst's remit I record it as a question: **do you want maintainer authorization to
become block-validity-relevant, or to remain an effect-only rule?**

**8.3 — should `reason` be signed or deleted?** It is attacker-chosen free text on a fee-exempt
transaction, it is outside the signature, and it is the txid-malleability vector. Signing it
preserves the transparency feature; deleting it is the subtraction move and removes 256 bytes of
permanent free chain write. **Is `reason` load-bearing for you operationally?**

**8.4 — is a payload format change acceptable?** Adding any signed field changes `extra_data`,
which per INV-DEPLOY-001 implies a synchronized fleet deploy and re-derives the F5 caps. A binding
that uses only values the verifier already holds (genesis hash, block height, the transaction
itself) avoids that. **Do you want to pay for a synchronized deploy, or should the design be
constrained to add no bytes to the wire?**

**8.5 — mainnet AH-176 pinning.** Per INV-PARAMS-001 the mainnet value should stay `u64::MAX`
until a deliberate pinning session, as `inc_i_173_activation_height` does today. **Confirm that
this incident ships with mainnet at `u64::MAX` and testnet pinned above the live tip.**

**8.6 — `ProtocolActivationData` has the same defect** (`data.rs:99-105`). It is out of scope per
§7.6. **Do you want it opened as its own incident now, or left banked?**

---

## 9. What I Do NOT Know / Could Not Verify

Stated plainly, because gaps here become gaps in requirements.

1. **I did not re-run the chain scan** proving that no block on any network contains an
   `AddMaintainer`/`RemoveMaintainer`. I am relying on the in-tree record at
   `crates/core/src/maintainer/mod.rs:129-136` (testnet, 2026-08-11). Mainnet was never scanned by
   me. If any such transaction exists anywhere, several arguments in §6 weaken.
2. **I did not build or run any test.** Every behavioral claim is read from source. No FAIL→PASS
   evidence exists for anything in §7 — by design; that is the test-writer's job.
3. **I cannot reconstruct the incident's literal ratchet path.** §Q4 Leg 4 documents the
   narrowing. If there is a standalone two-removal path I did not find, my threat model
   understates the exposure.
4. **I do not know whether external tooling outside this repository signs these bytes.** I found
   two copies of `sign_maintainer.py` in session scratchpads. Anything on an operator's laptop is
   invisible to me and will break silently.
5. **I do not know the operational shape of the INC-I-175 signing ceremony** — whether keys will
   be hardware-backed, whether re-signing mid-run is minutes or hours, whether the signers are
   co-located. That determines whether 8.1 is a real constraint or a theoretical one.
6. **I did not verify the mempool/builder/apply height skew empirically.** I read the three
   context constructions (`pool.rs:766` uses `current_height`; `assembly.rs` and
   `tx_processing.rs` use the block height) and inferred the one-block skew. That is 1 inference,
   unverified by execution.
7. **I do not know whether `derive_maintainer_set` will be wired to production during or after
   this work.** It verifies the same message (`derivation.rs:190,203`), so it is in the blast
   radius either way, but its future is INC-I-172 M3 scope.
8. **I did not read `crates/updater/src/` in depth** beyond `trust_root.rs` signatures and
   `constants.rs`. The release-verification path consumes the maintainer SET, not the
   authorization, so I scoped it out; if the redesign changes the set's lifecycle, that scoping
   must be revisited.
9. **`blast.py` under-reports Rust receiver-method callers** (it said so itself for
   `verify_multisig_at`: 1 dependent, flagged as a lower bound). Every caller table in §4 was
   grep-confirmed per root, but a caller reachable only through a trait object or a re-export
   could still be missing.

### Brittleness Check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 3/5
Details:
  [1] Cross-module blast radius — YES. A correct fix spans crates/core/maintainer (signed bytes),
      crates/core/validation (enforcement point), bins/node/apply_block (verification),
      crates/rpc (admission), bins/node/commands + out-of-tree signers, and 14 test files
      across 2 roots. These modules share no direct dependency edge.
  [2] Invariant gaps — YES. No module owns "an authorization is used at most once". P13
      (state-precondition idempotence) is the only thing standing in for it, and it is a
      side effect of add/remove bounds checks, not a designed uniqueness rule.
  [5] Contract absence — YES. There is no explicit interface between the SIGNER
      (bins/node/src/commands/maintainer.rs, sign_maintainer.py) and the VERIFIER
      (apply_block/governance.rs). Both hardcode format!("{}:{}", action, hex) independently.
      The Python harness is not even in the repository.
  [3] Data flow reversal — NO. The fix flows with the existing direction.
  [4] Shared mutable state — NO. maintainer_state.bin has one writer
      (process_transaction_governance) plus the INC-I-174 rewind path, both under one RwLock.
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Consequence of BRITTLE:** this is an architectural problem, not a code bug. The architect's
feasibility assessment should treat "add a field to `signing_message`" as insufficient on its
own — the missing contract between signer and verifier (signal 5) and the missing uniqueness
invariant (signal 2) are the substance, and a new signed field that leaves both absent will be
the third patch on this surface after INC-I-172 M2 and INC-I-173 M3a.
