# INC-I-182: authorization of producer-changing transactions (analysis)

Scope: every transaction type that changes a producer's bonds, registration, delegation, keys or exit
status. Areas: `crates/core/src/{transaction,validation,network_params}`, `crates/mempool/src`,
`bins/node/src/node/{production,validation_checks,apply_block}`, `bins/cli/src`.
Out of scope: activation height values for mainnet and testnet, whitepaper text, new live scenarios.

---

## 1. Bug statement and regression check

A `RequestWithdrawal` names its producer only as a public key in `extra_data`. The producer does not
sign it. After the INC-I-180 holdings gate, the only link between the fee payer and the named producer is
the ownership of the Bond UTXOs that the transaction spends. Some producers own no live Bond UTXO at
their derived address, but the ProducerSet still lists bonds for them (for example, a producer whose
bonds sit at a non-derived address). For such a producer, the full-exit check compares 0 with 0 and
passes. Any fee payer can then pay the base fee from its own coins and declare the producer's full
allowance. At the next epoch flush, the producer leaves the set and loses its delegation. The defect is
on the live rule path on every network that has crossed the holdings gate. It is not new.

```
$ git log ac186dbd..HEAD --oneline | wc -l
       0
```

Regression check: range ac186dbd..HEAD is empty (baseline == HEAD); defect pre-exists — finding 302 (AUDIT-P1-M2-002) recorded it before any commit on this branch.

---

## 2. Architecture Context

### 2.1 Module boundaries

Dependency direction: `core` <- `storage` <- `mempool` <- `node`. `cli` -> `core`.

| Module | What it does for producer-changing transactions | Depends on | Used by |
|---|---|---|---|
| `crates/core/src/transaction/` | Wire types. `TxType` has 25 variants (`types.rs:6-144`). Payload codecs are in `data.rs`. The input sighash is `signing_message` / `signing_message_for_input`, and it **does not include `extra_data`** (`core.rs:514-538`, `core.rs:548-575`) | crypto | everything |
| `crates/core/src/validation/` | `validate_transaction`: stateless checks (`transaction.rs:28-298`). The fast arm for Registration and Slash (`transaction.rs:306-336`). Per-type structure checks (`tx_types.rs`). UTXO-level checks (`utxo.rs:24+`). The spend lock (`unbonding.rs:47-75`). The C1 output rules (`unbonding.rs:38-149`). Block validation (`block.rs:139+`; the tx loop runs in Full mode at `:277/:281` and in Light/Replay mode at `:358`) | transaction, network_params | mempool, node |
| `crates/core/src/network_params/` | Gate values per network. Only devnet can override them from the environment | — | all |
| `crates/storage/src/producer/` | ProducerSet, pending updates, epoch flush. The resolver `withdrawal_inputs.rs` returns `owned_bonds`, `all_bonds` and `spent_bonds` | core | mempool, node |
| `crates/mempool/src/` | Admission. `pool.rs:536-586` builds the context (C1 rules judged at the next height, `:568-572`) and calls `validate_transaction` (`:586`). The holdings and vesting mirror runs at `:698-699` (`withdrawal_holdings.rs`, `vesting_bound.rs`). Zero-flow auth is in `zero_flow_auth.rs` | core, storage | node |
| `bins/node/src/node/production/` | Builder. It builds the genesis-window Registration (`assembly.rs:125-145`), runs `validate_transaction_with_utxos` (`:313`), and runs the builder mirror `withdrawal_holdings.rs` | core, mempool | — |
| `bins/node/src/node/validation_checks/` | `validate_block_for_apply` (core block validation at `mod.rs:513`). `validate_block_economics` (`mod.rs:544`) calls `check_withdrawal_economics` (`:676`, in `withdrawal_economics.rs`). It also refuses withdrawals for slashed producers (`slash_withdrawal.rs:6-34`) | core, storage | apply |
| `bins/node/src/node/apply_block/` | Economics run before mutation (`mod.rs:114`). Per-tx UTXO validation (`tx_processing.rs:134`; Replay only warns on failure, `:144-151`). Producer effects (`tx_processing.rs:230-622`) | all | — |
| `bins/node/src/node/rewards.rs` | `rebuild_producer_set_from_blocks` (`:1072`) replays the producer effects. It copies the DelegateBond skip (`:1409-1411`) | storage | recovery |
| `bins/node/src/node/rollback.rs` | Undo-based rollback (`:137-163`) | storage | reorg |
| `bins/cli/src/` | Builders: `cmd_producer/exit.rs`, `withdrawal.rs` (through `unbonding_tx.rs`), `bonds.rs`, `register.rs`, `delegation.rs`. They sign inputs only | core | users |

### 2.2 Data flow

1. The CLI builds the tx and signs each input over `signing_message_for_input`, which does not include `extra_data`.
2. The tx goes to RPC `sendTransaction`. Mempool admission runs: stateless `validate_transaction`, the inline UTXO and signature checks, the holdings and vesting mirror, slash admission and the AddBond verdict.
3. Gossip relays the tx.
4. The builder runs `validate_transaction_with_utxos` and `WithdrawalParity`, then puts the tx in a block.
5. Each peer runs `validate_block_for_apply`. The core `validate_block_with_mode` calls `validate_transaction` for each tx in every mode.
6. `apply_block` runs `validate_block_economics` (R1 allowance, R4 same-block input, R3 exclusivity, R2 full/partial, vesting bound). Then it runs the per-tx UTXO validation, the UTXO spend and add, and the producer effects. `RequestWithdrawal` increases `withdrawal_pending_count` at once and queues a `PendingProducerUpdate`.
7. The epoch flush clamps `bond_count`, runs the automatic exit and updates the state root.
8. Rebuild walks stored blocks again through the same effects. Rollback restores the undo snapshot.

### 2.3 Trust boundaries

| Entry point | Untrusted data | What uses it |
|---|---|---|
| RPC `sendTransaction`, P2P tx gossip | the whole tx | mempool admission, gossip relay |
| Blocks from peers, snap-sync gap blocks (Light mode), stored blocks (Replay mode) | every tx in the block | block validation, apply, rebuild |
| `extra_data` of Registration (bincode `RegistrationData`: public key, BLS key, PoP, bond count), AddBond (producer, bond count), RequestWithdrawal (producer, bond count, destination), Exit (public key) | **no input signature covers it** (`core.rs:514-517` states that `extra_data` is left out) | ProducerSet effects (`tx_processing.rs`), economics (`withdrawal_economics.rs`), mempool and builder mirrors |
| Inputs and outputs | covered by the signatures of the **input owners**, who are the fee payer and not always the named producer | UTXO set |

The producer identity in the payload is untrusted on every one of these paths. The security requirements
are REQ-PAUTH-002, REQ-PAUTH-003 and REQ-PAUTH-006, all Must.

### 2.4 Architectural constraints and invariants

- **Parity (INV-VALIDATION-001).** Each call site sets the gate fields with `with_*` builders (for example `with_inc_i_257_unbonding` at `pool.rs:568`, the apply `utxo_ctx` just above `tx_processing.rs:134`, and `slash_withdrawal.rs:48`). If one call site does not set the new gate, the rule is silently off on that path.
- **Mode independence.** The core block validator calls `validate_transaction` in Full, Light and Replay modes (`block.rs:277/281/358`). Apply's per-tx UTXO validation only warns in Replay (`tx_processing.rs:144-151`). A stateless rule inside `validate_transaction` gets the same verdict in every mode. A rule that lives only in the UTXO-level validator does not.
- **The Registration arm skips `validate_transaction`.** In Full mode, Registration takes the separate arm in `validate_transaction_skip_registration_vdf` (`transaction.rs:311-325`). A rule added only to `validate_transaction` does not reach Registration on that path.
- **Invalidate the block, do not skip the tx.** When the DelegateBond auth check fails, apply skips the effect and keeps the block valid (`tx_processing.rs:511-525`). Because of that, the rebuild path must repeat the check (`rewards.rs:1409-1411`). A rule that makes the block invalid needs no copy in rebuild, because rebuild only walks blocks that passed validation.
- **History below the gate.** Today all three decoders accept trailing bytes: `WithdrawalRequestData::from_bytes` (length >= 68, `data.rs:480-492`), `AddBondData::from_bytes` (length >= 36, `data.rs:201-211`), and the bincode decode of Registration (`core.rs:450-455`; bincode 1.3.3 `deserialize` calls `allow_trailing_bytes()`). Below the gate, the new binary must accept exactly the same sets.
- **The mempool judges at the next height.** The mempool judges the C1 rules at `current_height + 1` (`unbonding.rs:21-31`, `pool.rs:572`). The new gate needs the same treatment. If not, at the gate boundary the mempool admits a tx that the next block refuses (INV-BOND-002).
- **No wire change.** The `Transaction` struct (`core.rs:30-43`) stays the same, so `Vec<Block>` bincode stays the same.

### 2.5 Blast radius

Graph: graphify refreshed (`graphify-out/graph.json`). `blast.py validate_withdrawal_request_data --hops 2`
returned 138 dependents. Almost all of them are tests. The production dependents are the core block
validation (`block.rs:140`, through `validate_block_with_mode`) and the crate re-export. For the payload
accessor methods (`tx.withdrawal_request_data()`, `tx.add_bond_data()`, `tx.registration_data()`), the
graph returned only the file that defines them. This is the known blind spot for Rust method calls, so I
used grep for these (count per file, non-test code):

- `withdrawal_request_data()`: apply `tx_processing.rs` (1), builder `withdrawal_holdings.rs` (3), `rewards.rs` (1), `slash_withdrawal.rs` (1), `withdrawal_economics.rs` (2), core `slash_state.rs` (1), mempool `withdrawal_holdings.rs` (2).
- `add_bond_data()`: `tx_processing.rs`, builder `withdrawal_holdings.rs` (3), `rewards.rs`, `validation_checks/mod.rs`, `withdrawal_economics.rs`, core `unbonding.rs`, mempool `addbond_cap.rs`, `pool.rs`.
- `registration_data()`: `tx_processing.rs` (2), `genesis.rs` (2), `rewards.rs`, core `unbonding.rs`, mempool `pending_registrations.rs` (2).
- `validate_transaction_with_utxos(`: apply `tx_processing.rs:134`, builder `assembly.rs:313`.

Direct impact: core validation (new rule, plus the gate in `ValidationContext`), `network_params` (the
gate), CLI builders (they must sign), and the builder's genesis-window Registration on devnet (gate 0).
Indirect impact: none, as long as the decoders still accept trailing bytes. Every consumer above reads
only the first 68 bytes, the first 36 bytes, or the bincode prefix of the payload.

### 2.6 Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details: (2) invariant gap — no module makes sure that the producer's key authorized a change to its own
record; (5) contract absence — the payload depends on an unstated convention ("whoever owns the inputs
agreed") while the sighash leaves out extra_data. Not present: cross-module spread without a shared
dependency (every path depends on core validation), data-flow reversal, shared mutable state without an
owner.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 3. R-1 enumeration (REQ-PAUTH-001, report half)

Columns (a), (b) and (c) answer three questions. (a) Can a user-submitted tx reach the rule? (b) Can a
producer action reach it? (c) Is the behavior below the gate bit-identical?

| Variant | What it changes | Who can submit it today | How the signer is bound to the producer today | Vacuous in a reachable state? | Recommended rule after the new gate | (a) | (b) | (c) |
|---|---|---|---|---|---|---|---|---|
| Registration (1) | Creates the producer record, its bonds and its BLS key | Anyone who knows the public key, pays for the bonds and runs the public VDF. The VDF input is the public key plus the epoch (`registration.rs:232`) | **NONE** for the Ed25519 key. The BLS PoP proves only the BLS key (`registration.rs:259-268`). After C1, the Bond outputs must be owned by the producer's address (`unbonding.rs:132-147`). That controls where the funds go, but not who agreed | **Yes.** Anyone can register a key that is not registered or has exited. A relayer can also replace `bls_pubkey`+`bls_pop` in the producer's own registration, because the sighash leaves out `extra_data` | **Producer signature.** 64 bytes after the bincode payload. The builder signs its own genesis-window Registration | yes | yes | yes |
| Exit (2) | Queues a full withdrawal or an exit | Anyone. `ExitData` holds only a public key (`tx_types.rs:9-40`) | **NONE**, but it **cannot be mined at any height**. It has 0 inputs and 0 outputs, `allows_empty_io` is false (`types.rs:194-195`), and the fee gate refuses fee 0 < `BASE_FEE`=1 (`utxo.rs:236-294`, `constants.rs:648`) | No. Mempool, builder and apply all refuse it | **Already refused — no change.** The invariant must say that Exit stays unmineable unless it gets this rule | refused | refused | yes |
| SlashProducer (5) | Burns the accused's bonds and removes it | Anyone who has evidence | The evidence: two headers that the accused signed (`slash_evidence.rs:140`, `:194`), at or above the effective slashing gate. Nothing reads `reporter_signature` (`types.rs:196-197`) | No. The accused's own signatures are the authorization, by design | **Already bound — no change** | — | — | yes |
| AddBond (7) | Adds bonds and changes the allowance and the vesting age | Anyone who pays | **NONE** for the producer key (`tx_types.rs:251-299`). After C1, the Bond outputs are owned by the producer's address (`unbonding.rs:132-147`) | **Yes.** It works for any registered or pending producer. A third party can raise the producer's allowance in front of the producer's own full exit, and that exit then fails (`INCOMPLETE_DRAIN`/mismatch). It can also add bonds that carry a vesting penalty | **Producer signature.** 64 bytes after the 36-byte payload | yes | yes | yes |
| RequestWithdrawal (8) | Takes bonds off the producer. A full allowance is an exit. It also revokes the producer's delegation (`tx_processing.rs:436-447`) | Any fee payer | Only through ownership of the Bond inputs: R3 (`withdrawal_economics.rs:261-273`) and R2 (`:283-310`), from the holdings gate on | **Yes.** (i) If the producer owns 0 live Bond UTXOs, the full exit passes at 0 == 0. (ii) If the producer spends all the Bond UTXOs it owns while its allowance is larger, a relayer can rewrite `bond_count` to the allowance, because `extra_data` is not in the sighash | **Producer signature.** 64 bytes after the 68-byte payload | yes | yes | yes |
| DelegateBond (13) | Moves the delegator's weight to a delegate | Anyone. The delegator's signature is checked | Ed25519 signature of the delegator over BLAKE3(`"DELEGATE_BOND"` ‖ delegate ‖ bond_count) (`data.rs:295-301`). Apply checks it at `tx_processing.rs:511-517` (skip only). Admission checks it at `zero_flow_auth.rs:75-82`. Gate: `delegation_auth_activation_height` | Not for the delegator on mainnet and testnet (gate 0). Yes on devnet (gate MAX). The delegate does not consent, by design. The preimage has no genesis hash and no single-use anchor, so only admission refuses a replay (`zero_flow_mined.rs`) | **Already bound — no change.** Do not copy this preimage | — | — | yes |
| RevokeDelegation (14) | Removes a delegation | As DelegateBond | `data.rs:401-406`, `tx_processing.rs:583-589`, `zero_flow_auth.rs:83-85` | As DelegateBond | **Already bound — no change** | — | — | yes |
| RotateBlsKey (32) | Replaces the producer's BLS key | The producer | The key revealed by input 0 must equal the producer (`validation/rotate_bls.rs:62-72`). The signature covers `"DOLI-ROTATE-BLS-V1"` ‖ genesis ‖ new key ‖ outpoint (`validation/rotate_bls.rs:74-89`, preimage at `transaction/rotate_bls.rs:65-85`). The gate is frozen | No | **Already bound — no change.** Use it as the template | — | — | yes |

**Not in the table**, because these types do not change bonds, registration or exit:
- Transfer and every other type cannot spend a Bond (`unbonding.rs:47-75`).
- ClaimReward and ClaimBond have no producer effect (`tx_processing.rs:230-622`). Their zero-input output also fails the balance check.
- ClaimWithdrawal is a tombstone (`transaction.rs:157-162`).
- Coinbase and EpochReward are protocol transactions.
- AddMaintainer and RemoveMaintainer change the maintainer set and need a 3/5 multisig.
- PriceAttestation (gate MAX), ProtocolActivation, and the asset, AMM and ZK types.

**Recommended binding (rule, not code).** Digest = BLAKE3(`"DOLI-PRODUCER-AUTH-V1"` ‖ genesis_hash ‖
`tx.signing_message()` ‖ payload bytes without the signature). Verify it with the producer public key
from the payload.

What each part does:
- The domain tag makes sure that no other signed message can be used in its place: block header, `doli sign` prehash, rotation or delegation.
- The genesis hash prevents replay on another network.
- `signing_message()` commits to the tx type, every input outpoint and every output. Outpoints are spent only once, so the signature cannot be replayed into another tx. The outputs include the payout, the Unbonding lock and the Bond outputs.
- The payload commits to the producer, the bond count, the destination and the BLS key.

The fee payer's input signatures do not cover `extra_data`. So the producer can sign after the inputs
and outputs are final, and appending the signature does not break any input signature. A third party can
still pay the fee.

**Where the signature goes, with no new field on a wire type.** It goes in the last 64 bytes of the
existing `extra_data`, after the payload, which does not move. Old decoders read the same fields and
ignore the 64 extra bytes. Above the gate, the length must be exact: 68+64, 36+64, or canonical bincode+64.
Below the gate, the decoders keep accepting the same lengths as today. This follows the RotateBlsKey
preimage pattern (genesis plus a single-use anchor), placed in the same "longer `extra_data`" position
that DelegateBond uses.

---

## 4. Probable cause and evidence

The full-exit branch in consensus, `bins/node/src/node/validation_checks/withdrawal_economics.rs:283-297`:

```rust
                        if wd.bond_count == allowance && wd.bond_count > 0 {
                            let addr = hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
                            let owned = owned_live_bonds.get(&addr).copied().unwrap_or(0);
                            if bond_inputs != owned {
                                anyhow::bail!(
                                    "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN] RequestWithdrawal at \
                                     height={} producer={} declares its full allowance of {} \
                                     bonds but spends {} of the {} Bond UTXOs it owns",
                                    height,
                                    pk_hash,
                                    wd.bond_count,
                                    bond_inputs,
                                    owned
                                );
                            }
```

The same branch is copied in the mempool (`crates/mempool/src/withdrawal_holdings.rs:91-100`) and in the
builder (`bins/node/src/node/production/withdrawal_holdings.rs:218-227`). All three paths accept the tx.

**How a third party reaches it.** Fee payer T spends only its own Normal UTXOs and signs them. The
`extra_data` names producer P. P does not sign anything.
1. The resolver gives `owned_bonds = 0` and `all_bonds = 0`, so R3 (`:261`) passes at 0 == 0.
2. T declares P's full allowance, so the full-exit branch runs.
3. `owned_live_bonds(P) = 0`, so `bond_inputs != owned` is `0 != 0` and false. The check passes.
4. Above the vesting gate, the payout can be at most T's own non-bond input. T gets its coins back minus the base fee.
5. Apply queues `RequestWithdrawal(P, allowance)` (`tx_processing.rs:472-491`). The flush clamps `bond_count` to 0 and runs the automatic exit (comment at `withdrawal_economics.rs:274-282`).

A second way exists (malleation). P withdraws and spends every Bond UTXO it owns while its allowance is
larger. A relayer or a block producer then rewrites `bond_count` to the allowance, and the tx becomes a
full exit that P did not sign for.

**Root cause.** Nothing binds the producer identity in the payload to the producer's key. Input
signatures leave out `extra_data` (`core.rs:514-538`, `:548-575`). The only substitute, ownership of the
Bond inputs, is empty in a state that a third party can reach.

**Records.**
- Finding 302 / AUDIT-P1-M2-002 (open, P1) records that `WithdrawalRequestData` has no signature, unlike `DelegateBondData`. It also records that the full-exit check passes vacuously when the producer owns no Bond UTXO. Its recorded location (`validation_checks.rs:868-882`) is stale: that file is now a directory, and the check is at `withdrawal_economics.rs:283-297`.
- Outcome 2875 (auth auditor) says that the authorization is implicit in the ownership of the Bond UTXOs, so it is vacuous when nothing is owned. It also says that Exit is not authenticated but cannot be mined, because the fee gate refuses it.
- Outcome 2878 says that a third party can force another producer's full exit in this shape. The shape includes any producer whose bonds sit at a non-derived address.

---

## 5. SSF statement

The simplest fix for the root cause: at and above a new activation height, a Registration, AddBond or
RequestWithdrawal is valid only when its last 64 `extra_data` bytes are the named producer's Ed25519
signature over BLAKE3(`"DOLI-PRODUCER-AUTH-V1"` ‖ genesis hash ‖ the tx's existing signing message ‖ the
payload). One stateless core check, reached by mempool, builder, apply and replay, decides this, and a
failed check makes the block invalid.

A rule that requires an input owned by the producer is not enough. Input signatures leave out
`extra_data`, so they never commit to the producer name, the bond count or the BLS key that the payload
carries.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 Ed25519 verify per Registration/AddBond/RequestWithdrawal per path (admission, build, block validation) (inferred; these tx types are rare)
  Memory:   0 (observed; no new state, no caches)
  IO:       0 (observed; no new reads, the payload is already decoded)
  Network:  +64 bytes per affected tx (observed)
  Disk:     +64 bytes per affected tx in stored blocks (observed)
  Latency:  +constant, below 0.1 ms per affected tx (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: require one input owned by the producer's address (no new bytes, no extra verify)
Why this proposal anyway: the cheaper rule leaves bond_count, destination and the BLS key open to rewriting by a relayer, because the sighash leaves out extra_data
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 6. Impact analysis

| Path | What changes | Risk |
|---|---|---|
| Mempool admission (`pool.rs:536-586`, `:1023-1066`) | The context gets the new gate, judged at the next height. The rule runs inside `validate_transaction` | Medium. A call site that does not set the gate leaves the rule off |
| Builder (`assembly.rs:313`, genesis Registration `:125-145`) | Reaches the rule through `validate_transaction_with_utxos`. With gate 0 on devnet, the builder must sign its own genesis-window Registration | Medium (devnet genesis) |
| Block validation (`validate_block_for_apply` -> `block.rs:277/281/358`) | Same verdict in Full, Light and Replay. The Registration arm (`transaction.rs:311-325`) also needs the rule | Medium |
| Apply (`apply_block/mod.rs:114`, `tx_processing.rs:134`) | No new state. Effects do not change | Low |
| Rebuild (`rewards.rs:1072`) | No change, because the rule makes the block invalid instead of skipping the tx | Low |
| Rollback / reorg (`rollback.rs:137-163`) | No new state, so undo does not change. A reorg test across an authorized withdrawal is still required | Low |
| CLI (`exit.rs`, `withdrawal.rs`/`unbonding_tx.rs`, `bonds.rs`, `register.rs`) | Sign the payload after the inputs and outputs are final. `exit.rs` stops at the ledger-mismatch check (`:50-57`) and at the Bond-UTXO shortfall check (`:135-141`), so it cannot build the zero-Bond-input exit. It also builds a Normal payout with no C1 builder (`:192-198`), so above C1 (devnet gate 0) the payout must be an Unbonding output. `unbonding_tx.rs:65-70` refuses `total_net <= fee`, so a zero-Bond exit must pay for its payout from non-bond inputs | High for REQ-PAUTH-004 |
| RPC block/tx decode, snap sync, old-node decode | The `Transaction` type does not change. `extra_data` is 64 bytes longer. An `ac186dbd` node decodes and accepts the signed forms at every height | Low |
| Fee | `minimum_fee` counts only the `extra_data` of outputs (`core.rs:696-701`), so the fee does not change | None |

**Invariants at stake.**
- INV-VALIDATION-001: one shared stateless check gives parity by construction, provided that every context sets the gate.
- INV-BOND-002: refuse an unsigned tx at admission, judged at the next height.
- INV-BOND-003 and INV-UNBOND-001: no change. The signature covers the outputs, so the producer authorizes the Bond and Unbonding terms.
- INV-SLASH-001/002: no change. A slashed producer is still refused by `[ECON_WITHDRAWAL_SLASHED]` (`slash_withdrawal.rs:24-31`). Undo snapshots do not change.

**Three-question consensus-shape check (INC-I-075).**
1. Can a user-submitted tx reach this path? **Yes**: Registration, AddBond and RequestWithdrawal come in through RPC and gossip.
2. Can a producer action reach it? **Yes**: a producer can put an unsigned or rewritten version in its own block.
3. Is the behavior bit-identical for all inputs? **No**: above the gate, unsigned forms that are valid today become invalid.

Result: **an activation height is REQUIRED.**

**Gate.** `inc_i_182_producer_auth_activation_height`. Declare it next to
`inc_i_257_unbonding_activation_height` (`network_params/mod.rs:865-866`). Use the same per-network
values: `u64::MAX` on mainnet (`defaults.rs:285` block) and testnet (`defaults.rs:557` block), and 0 on
devnet (`defaults.rs:831` block). Add a devnet-only environment override,
`DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT`, the same as `env_loader.rs:593-600`. Mainnet and
testnet always use the compiled value. The `ValidationContext` field must default to "off" (`u64::MAX`),
so that existing test contexts do not change. No ordering rule against the other gates unless a test
shows one is needed.

**Deploy questions.**
- Consensus rules change, so an activation height is required (above).
- Block content does not change: no header, coinbase, bitfield or ordering change. Txs are 64 bytes longer, and old nodes accept that, so no synchronized deploy is needed for content.
- `CURRENT_PROTOCOL_VERSION`: **no bump**, because EpochState serialization does not change.
- `HardForkSchedule`: **no entry**.

---

## 7. Requirements

| ID | Requirement | Priority | Acceptance criteria |
|---|---|---|---|
| REQ-PAUTH-001 | List every tx type that changes a producer's bonds, registration or exit, and how each binds the signer to the producer (Section 3). Show the defect as failing tests on the baseline: a third party forces the full exit of a producer that owns no live Bond UTXO | Must | - [ ] Section 3 lists each type and its binding<br>- [ ] Named RED tests, with failure lines on `ac186dbd`, show that mempool, builder and apply each accept the third-party full exit<br>- [ ] The same tests PASS at HEAD |
| REQ-PAUTH-002 | Above the new gate, Registration, AddBond and RequestWithdrawal are valid only with the producer signature over the domain-separated digest in Section 3. One core function decides on mempool, builder, apply and replay. A rollback across such a tx restores the exact earlier state | Must | - [ ] On all four paths: authorized is accepted. Unsigned, wrong key, a signature over another tx's inputs, a signature from another genesis, and a truncated or extended tail are each refused with one error code<br>- [ ] The relayer rewrite of `bond_count` and of the BLS key is refused<br>- [ ] The mempool judges at the next height (a tx admitted at gate-1 is valid at the gate)<br>- [ ] A reorg test across an authorized withdrawal gives the earlier state root |
| REQ-PAUTH-003 | Gate and history: `u64::MAX` on mainnet and testnet, 0 on devnet. Below the gate, blocks and state are byte-identical to `ac186dbd`, and an `ac186dbd` node decodes every block that the new binary makes. No new field on any wire type. The environment override works only on devnet | Must | - [ ] Golden/replay test below the gate (same verdicts, same state root)<br>- [ ] Old-format decode test for the signed forms (68+64, 36+64, bincode+64) and the legacy forms<br>- [ ] Override test: the environment variable has no effect on mainnet and testnet |
| REQ-PAUTH-004 | The producer can still exit by itself. When the ProducerSet lists bonds but the producer owns no live Bond UTXO at its own address, the producer's own signed full-exit RequestWithdrawal (zero Bond inputs, fee and payout from non-bond funds) is valid, and the CLI builds it | Must | - [ ] The full exit is accepted on mempool, builder and apply, and the producer exits at the flush<br>- [ ] A CLI test shows that the built tx validates above this gate **and** above the C1 gate (Unbonding payout)<br>- [ ] The slash and full-withdrawal cases follow Q1/Q2 in Section 10 |
| REQ-PAUTH-005 | Records: INC-I-182 root cause, files and resolution. A decision row (rule per type, gate). An invariant with linked regression tests. Finding 302 updated, including its stale location | Should | - [ ] The rows exist, and the report gives their ids |
| REQ-PAUTH-006 | With the gate at 0 (devnet), every node-built and CLI-built Registration, AddBond and RequestWithdrawal carries the producer signature. A fresh devnet completes genesis and derives its genesis producers | Must | - [ ] Fresh devnet test: genesis completes and every genesis producer is derived<br>- [ ] CLI register, add-bond and request-withdrawal txs validate at gate 0<br>- [ ] GS-026 and GS-027 PASS at HEAD |

Why REQ-PAUTH-006 is added: with devnet gate 0, the builder's genesis-window Registration and every
CLI-built producer tx fall under the rule from block 1. Without this requirement, M1 breaks devnet and
the gauntlet regressions.

---

## 8. Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies | Status |
|----|------|-----------------|----------------------|--------------|--------|
| M1 | Producer signature rule behind its own gate on every path | crates/core/src/{transaction,validation,network_params}, crates/mempool/src/pool.rs, bins/node/src/node/{production,apply_block,validation_checks}, bins/cli/src/{cmd_producer,unbonding_tx.rs} | REQ-PAUTH-001, REQ-PAUTH-002, REQ-PAUTH-003, REQ-PAUTH-006 | - | PENDING |
| M2 | Producer self-exit without live Bond UTXOs, CLI exit form, records | bins/cli/src/{cmd_producer/exit.rs,unbonding_tx.rs}, crates/core/src/validation, docs/, specs/, .omega/memory.db | REQ-PAUTH-004, REQ-PAUTH-005 | M1 | PENDING |

---

## 9. Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.82, observed — the vacuous branch was read at its three mirrored sites, the sighash exclusion and Exit's fee refusal were read in code, and the RED tests are not yet run)
Reasoning: The cause is known and deterministic, there are no prior fix attempts, brittleness is LOCALIZED (2/5), and the fix is one gated stateless rule with the standard parity spread
━━━━━━━━━━━━━━━━━━━━━━
```

---

## 10. What I don't understand / open questions

- **Q1 (REQ-PAUTH-004, "after a slash").** A slashed producer is already out of the set: `slash_producer` sets its status to slashed (`set_lifecycle.rs:184-195`). Run B also refuses every RequestWithdrawal that names a slashed or slash-queued producer (`slash_withdrawal.rs:24-31`). So it has nothing to exit. Recommended reading: Run B stays as it is, and the test shows that the new rule adds no new stuck state. Not confirmed.
- **Q2 ("after a full withdrawal").** After its own full withdrawal, the producer's allowance is 0 until the flush, and the producer has exited after the flush. So no second exit is possible or needed. Recommended reading: the real case is "the ProducerSet lists bonds and the producer owns no live Bond UTXO at its own address". Not confirmed.
- I did not check whether any serde path (RPC JSON or storage) serializes `WithdrawalRequestData`, `AddBondData` or `RegistrationData` as structs. If the signature becomes a Rust field on a serde-derived struct, those encodings could change. The signature must stay out of their serde shape, or the developer must prove that no serde path uses them.
- I could not find the C1 follow-up findings AUDIT-P2-545..548 under those ids in `findings`. I cannot say if the Normal payout of `exit.rs` above C1 is already tracked.
- I do not know how many live producers are in the "no live Bond UTXO at their own address" shape. I have no access to the live systems, and that is out of scope.

## 11. Assumptions

| # | Technical | Plain language | Confirmed |
|---|---|---|---|
| 1 | The signature is the last 64 bytes of `extra_data`. Below the gate, the decoders keep today's lengths | The signature goes at the end of the data that already exists, so old software still reads it | No (architect) |
| 2 | Exit stays unmineable, so it gets no new rule | The old "exit" tx type can already never get into a block | Yes (code) |
| 3 | A third party may still pay the fee for a producer-signed tx | Someone else can pay the fee, but only the producer's key can approve the change | No |
| 4 | DelegateBond and RevokeDelegation keep their existing binding and preimage | The delegation signatures already work, so they do not change | No |

## 12. Specs drift detected

- `types.rs:11` says that Exit "starts unbonding period". In fact, Exit cannot be mined, and the CLI `exit` command sends a RequestWithdrawal.
- The project code map says that `TxType` has 24 variants. The enum has 25 (bincode ordinals 0-24).
- The location of finding 302 is stale (Section 4).

## 13. Identified risks

- A context call site does not set the gate, so the rule is off on that path. Mitigation: parity tests on all four paths.
- The Registration fast arm skips `validate_transaction`. Mitigation: a dedicated Full-mode Registration test.
- Devnet genesis at gate 0. Mitigation: REQ-PAUTH-006.
- Systemic, outside this incident: no input signature covers `extra_data` for **any** tx type. Payload fields that nothing else binds can be rewritten by a relayer. This needs its own finding.

## 14. Out of scope (Won't)

- Consent of the delegate to receiving a delegation.
- Replay protection for DelegateBond at consensus level.
- The `extra_data` malleability of tx types that do not change a producer's record.
- Mainnet and testnet height values, whitepaper text, push, release and deploy.

## 15. Traceability matrix

| Requirement | Priority | Test IDs | Architecture section | Implementation module |
|---|---|---|---|---|
| REQ-PAUTH-001 | Must | (test-writer) | (architect) | M1: `crates/core/src/validation/producer_auth.rs` (`check_producer_auth`, scoped to Registration, AddBond, RequestWithdrawal) |
| REQ-PAUTH-002 | Must | (test-writer) | (architect) | M1: `crates/core/src/transaction/producer_auth.rs`; `crates/core/src/validation/producer_auth.rs`; calls in `validation/transaction.rs`; gate at every context site (`crates/mempool/src/{pool,unbonding}.rs`, `bins/node/src/node/{production/assembly,apply_block/tx_processing,validation_checks/mod,validation_checks/slash_withdrawal}.rs`) |
| REQ-PAUTH-003 | Must | (test-writer) | (architect) | M1: `crates/core/src/network_params/{mod,defaults,env_loader}.rs`; `crates/core/src/validation/types.rs` (context default off) |
| REQ-PAUTH-004 | Must | (test-writer) | (architect) | M2: `bins/cli/src/unbonding_tx.rs` (`build_withdrawal_tx`, gated arm with no Bond inputs funded by fee UTXOs); `bins/cli/src/cmd_producer/exit.rs` (`handle_exit` through the lib builder, ProducerSet allowance, zero-Bond-input full exit). Node: no change (M1 already accepts the form) |
| REQ-PAUTH-005 | Should | (test-writer) | (architect) | M2: `specs/protocol.md` §3.7, §3.13.2; `docs/cli.md` §4.1, §4.5, §4.6, §4.8; `specs/security_model.md` §4.5. memory.db rows: at close-out |
| REQ-PAUTH-006 | Must | (test-writer) | (architect) | M1: `bins/node/src/node/production/assembly.rs` (genesis Registration); `bins/cli/src/producer_tx.rs`; `bins/cli/src/cmd_producer/{register,bonds,withdrawal,exit}.rs` |
