# INC-I-255 Analysis: Make equivocation slashing work end to end

Run 601 · Analyst · Status: ANALYSIS · Base `c8afea35` (INC-I-236 signed blocks)

## Scope
- Related incidents: INC-I-185 (a rollback keeps a queued Slash), INC-I-244 (a slashed producer stays a member), INC-I-237 (earlier flood of slash txs that no block could hold), INC-I-181 / AUDIT-P1-001 / SEC-CRYPTO-M2-001 (pending queue installed by snap sync).
- Invariants that apply: INV-AUTH-003, INV-BOND-002, INV-NETWORK-003, INV-VALIDATION-001, INV-VALIDATION-008.
- Modules:
  - `crates/core/src/{validation/{slash_evidence,utxo,transaction,block,types}.rs, transaction/{types,core,data}.rs, network_params/}`
  - `crates/mempool/src/{pool,zero_flow_auth,zero_flow_mined}.rs`
  - `crates/network/src/sync/equivocation.rs`
  - `crates/storage/src/producer/{set_lifecycle,set_core,info,set_persistence}.rs`
  - `bins/node/src/node/{block_handling,rewards,rollback,rpc_broadcast}.rs`, `bins/node/src/node/{apply_block,production,validation_checks,attestation}/`
  - `crates/rpc/src/methods/transaction.rs`
  - `scripts/gauntlet-gs027.sh` (new), `bins/node/examples/` (injector)

## Summary (plain language)
When a producer signs two different blocks for the same slot, every node can see it, but nothing happens: the node only writes a log line. A slash transaction was built in the past, but no block could include it because it pays no fee. Even if it could be included, it would not burn the coins: the bond coins stay spendable, a second slash for the same offence would also be accepted, and a rollback would keep a slash that should have been undone. This work makes one slash per offence, after a new activation height, that every node checks the same way, that burns the whole bond and removes the producer, and that a rollback undoes exactly.

## Regression check (regression-detect hook)
The baseline is `c8afea35`, which is HEAD. `git log c8afea35..HEAD` is empty (0 commits). Nothing in this range can have caused the defect, so it is pre-existing. The last commits on the slash path (`git log --oneline -8` over `rewards.rs`, `slash_evidence.rs`, `utxo.rs`, `apply_block/{helpers,tx_processing}.rs`, `equivocation.rs`, `set_lifecycle.rs`, `transaction/types.rs`):
`b7464002` M4 no header side effects before auth · `56642802` M3 slash evidence hardening · `0e9eab52` M2 producer signs + ctx threading · `83f9e7b1` INC-I-237 handler made log-only · `97b3fe08` INC-I-233 · `92a41c9b` INC-I-234 · `d5bd9c35` INC-I-193 · `df1b50da` docs.
No commit ever made SlashProducer mineable. `83f9e7b1` removed tx creation on purpose. The INC-I-236 commits added header authentication to the evidence but kept the tx unmineable.

## Anchor check (no skeptic report present)
- FIRST READ: the fee gate is the blocker; exempt SlashProducer above a gate and it works. Setting aside.
- SECOND READ: the fee gate is the only thing that keeps the slash path from causing harm. With the gate removed, the path has no dedup and no membership check, the burn does not burn, and the rollback does not undo.
- Choice: the second read. Evidence: `set_lifecycle.rs:165-193` changes only a status flag. `helpers.rs:15-28` omits the type. `core.rs:484-509` hashes the reporter signature into the tx id. The fee fix alone would turn INC-I-237 (a flood of txs that could not be mined) into a flood of txs that can be mined.

## Trust Boundaries
| # | Entry point | External data | Consumer |
|---|---|---|---|
| TB-1 | Gossip blocks topic → `handle_new_block` (`block_handling.rs:170-177`) | Block headers | `EquivocationDetector::check_block` → handler |
| TB-2 | Gossip tx topic → `handle_gossip_txs` (`validation_checks/gossip_txs.rs:11`) | `SlashData` bytes in `extra_data` | mempool admission, relay verdict |
| TB-3 | RPC send-transaction (`rpc/methods/transaction.rs:220`) | Same as TB-2 | mempool admission, publish (`rpc_broadcast.rs:33`) |
| TB-4 | Blocks from sync, reorg, backfill, replay | Mined slash txs | `validate_block_with_mode`, `apply_block` |
| TB-5 | Snap sync | `ProducerSet.pending_updates` (may hold `Slash`) | epoch flush |

All five carry untrusted bytes. A node must reject unauthenticated evidence at the first point it touches it, so this analysis treats every field as untrusted (see Assumption A1).

## User Stories
- As a node operator, I want a producer that signs two blocks for one slot to lose its whole bond automatically, so that double production is never profitable.
- As an honest producer, I want nobody to be able to slash me without two headers that carry my own signature, so that a false accusation is impossible.
- As a node operator, I want a single equivocation to cause at most one mined slash and bounded mempool growth, so that the network does not flood.

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-SLASH-001 | List each blocker and defect between detection and an applied slash. Open INC-I-255 and cross-link INC-I-185 and INC-I-244. | Must | - [ ] Each blocker in the Blockers section has a named test that is RED on `c8afea35` and GREEN at HEAD |
| REQ-SLASH-002 | Above the slashing gate, one shared consensus check decides whether a slash with authenticated evidence may pay zero fee. Mempool (both lanes), builder, apply and replay all call it. | Must | - [ ] Valid evidence accepted on all four paths<br>- [ ] SEC-1..SEC-7 rejected on all four paths with the same error variant |
| REQ-SLASH-003 | The slash burns 100% of the accused's own bond and removes the producer from selection and membership. A reorg across the slash block restores the exact previous state. | Must | - [ ] Bond UTXOs spent and supply lower by exactly the bond<br>- [ ] Absent from selection from the stated exclusion point<br>- [ ] After a reorg past the slash block, the state root matches a node that followed only the winning branch |
| REQ-SLASH-004 | Each offence (producer, slot) leads to at most one mined slash, and at most one slash per accused is in effect. All reporters derive the same tx id. A node refuses duplicate or invalid evidence on first contact and does not forward it. A per-block cap limits the verification cost. Below the gate no node builds or forwards a slash. | Must | - [ ] N detecting nodes: one shared id, one mined<br>- [ ] Block over the cap rejected in every mode<br>- [ ] Invalid evidence never forwarded<br>- [ ] Below the gate: no tx, no relay |
| REQ-SLASH-005 | Add gauntlet scenario GS-027. It starts several local devnet nodes with finite gates and makes one producer sign two blocks for one slot. | Must | - [ ] Old build (`c8afea35`): no slash happens, so the scenario FAILS<br>- [ ] HEAD: the scenario PASSES when exactly one slash is mined, each node shows the bond burned once and supply lower by the bond, all tips match, blocks keep coming and mempool slash counts stay bounded<br>- [ ] Both runs recorded in `gauntlet_runs` |
| REQ-SLASH-006 | A snapshot install must not bring in a queued Slash that no applied block created. | Should | - [ ] Test, or "not reachable" with a reason (see R-6 disposition) |
| REQ-SLASH-007 | Write the records: update the incident; add one decision row that fixes the burn rule, the exclusion point, the gate and the cap; add an invariant with its regression tests and a monitoring signal; update INC-I-185 and INC-I-244. | Should | - [ ] Row ids in the final report |

### Security sub-criteria (all Must; part of REQ-SLASH-002 and REQ-SLASH-004)
| Sub-id | Rule (fails closed) |
|---|---|
| SEC-1 | Each header must have a valid strict-Ed25519 signature by `header.producer` over `signing_message()`. Reuse `verify_block_signature` (`validation/block.rs:106-116`). |
| SEC-2 | `h1.slot == h2.slot`, `h1.producer == h2.producer == accused`, `h1.hash() != h2.hash()`. |
| SEC-3 | Both headers have `genesis_hash == ctx.params.genesis_hash`. |
| SEC-4 | Encoded `SlashData` is at most 64 KiB and the decode is bounded. This already holds above the INC-I-236 gate (`slash_evidence.rs:17-51`). |
| SEC-5 | The accused is a registered producer with a bond, not slashed, and has no slash queued, in the block, or earlier in the same block. |
| SEC-6 | The reporter field is either verified or forced to one canonical value. Header order is canonical, so a third party cannot make a second id for the same evidence. |
| SEC-7 | The slash-tx count per block is at most the cap, enforced before the mode split (`block.rs:155-159`), so Full, Light and Replay give one verdict. |

## Acceptance Criteria (detailed)
### REQ-SLASH-001
- [ ] Given `c8afea35`, when each Blockers-section test runs, then it fails with the stated assertion. Given HEAD, then it passes.
- [ ] Given memory.db, then INC-I-255 has one discovery entry per blocker, and INC-I-185 and INC-I-244 reference INC-I-255.
### REQ-SLASH-002
- [ ] Given h ≥ gate and two headers for one slot signed by a registered producer, when the tx goes through `handle_new_transaction`, the builder loop, `validate_block_with_mode` (Full/Light/Replay) and `apply_block` (Full/Replay), then each path accepts with fee 0.
- [ ] Given each variant (one bad signature, the same header twice, different slots, wrong genesis, more than 64 KiB, unknown producer, already slashed or slash queued), then every path rejects it with the same `ValidationError` variant.
- [ ] Given h < gate, then the verdict is byte-identical to `c8afea35` (InsufficientFee on the UTXO path, FeeTooLow in the mempool).
### REQ-SLASH-003
- [ ] Given a mined slash, then the accused's Bond UTXOs are spent, `bond_count` drops to 0 at the stated point, and total supply drops by exactly the self-bond amount. Delegators' principal is not touched (security model §4.2.1).
- [ ] Given the exclusion point (recommended: the next epoch boundary, the same as every other producer mutation), then the accused is not in `active_producers_at_height`, has no attester weight (`attestation/ingress.rs:102`), and cannot submit `RequestWithdrawal`.
- [ ] Given a reorg that removes the slash block, then the ProducerSet, UTXO set and state root equal those of a node that never applied it.
### REQ-SLASH-004
- [ ] Given N nodes that detect the same offence, then each submits at most one tx, all txs have one id, and one is mined. Later copies are refused as already slashed or queued and are not relayed.
- [ ] Given a block with cap + 1 valid slashes, or two slashes for one producer, then every mode rejects it.
- [ ] Given evidence that fails SEC-1..SEC-6, then the gossip verdict is not Accept and it is never re-published.
- [ ] Given h < gate, then the handler creates no tx and the mempool refuses a gossiped slash.
### REQ-SLASH-005
- [ ] Runner `scripts/gauntlet-gs027.sh` and test `scripts/test_gauntlet_gs027.sh` follow the GS-026 pattern (control binary, HEAD binary, env-var ports). Equivocation is injected by signing two different headers for one slot with the scheduled producer's throwaway devnet key.
### REQ-SLASH-006 / REQ-SLASH-007
- [ ] See the R-6 disposition. Rows exist and the report lists them.

## Architecture Context

### Module Boundaries
- **network/sync/equivocation.rs**: detection. Keeps a map (producer, slot) → first header (10k entries, 360-slot window). Depends on core. Used by node `block_handling.rs:174`.
- **node/rewards.rs `handle_equivocation`** (`:449-458`): reaction. Log only today. It reaches the mempool through `admit_zero_flow`, and the network through `rpc_broadcast.rs:33`.
- **core/transaction**: wire. `SlashData {producer_pubkey, evidence, reporter_signature}` (`data.rs:139-148`). `new_slash_producer` is 0-in/0-out (`core.rs:334-344`). The tx id hashes all of `extra_data` (`core.rs:484-509`).
- **core/validation**: the verdict. `validate_transaction` → `validate_slash_data` (VDF on). `validate_transaction_skip_registration_vdf` → `validate_slash_data_skip_vdf` (VDF off). `validate_transaction_with_utxos` = skip-VDF structural (`utxo.rs:34`), then the fee gate (`utxo.rs:241-297`). `validate_block_with_mode` = rotation rules and signature check before the mode split (`block.rs:149-157`), parallel VDF pre-pass in Full (`block.rs:187-266`), per-tx structural check (`:273-281` Full, `:355-357` Light/Replay).
- **mempool**: two lanes. The zero-flow lane `admit_zero_flow` → `check_system_transaction` (`validate_transaction` with VDF, under the write lock) → `zero_flow_policy::check` → `add_system_transaction`. The normal lane `add_transaction` → `validate_transaction` → its own fee floor (`pool.rs:751-757`). The mempool never calls `validate_transaction_with_utxos`.
- **node/production/assembly.rs**: builder. Per tx `rotation_admissible` → `validate_transaction_with_utxos` (`:291`) → uniqueness, byte budget, holdings gate.
- **node/apply_block**: `validate_block_for_apply` in every mode (`mod.rs:110`). Per tx `validate_transaction_with_utxos` (`tx_processing.rs:121`), tolerated in Replay (`:131-139`). Producer effects are queued (`tx_processing.rs:315-326` → `PendingProducerUpdate::Slash`). Snapshot decision at `mod.rs:173-176`.
- **storage/producer**: flush `apply_pending_updates_with_cap` (`set_core.rs:106-126`) → `slash_producer` (`set_lifecycle.rs:165-193`: status Slashed, delegations cleaned, exit history). Since the INC-I-181 gate the pending queue is root-bound (`set_persistence.rs:122-129`).
- **node/rollback.rs**: restores the ProducerSet snapshot unless the sentinel is empty (`:178-199`). The fallback `rebuild_producer_set_from_blocks` queues Slash at `rewards.rs:1290-1297`.

### Data Flows Through Affected Area
(a) Detection: gossip block → `gossip_block_authenticated` (`block_handling.rs:170`, `header_auth.rs:43-54`, gated at best+1) → `check_block` (`equivocation.rs:125-166`) → `handle_equivocation` (log). Nothing is created.
(b) Wire and evidence: `EquivocationProof::to_slash_transaction` (`equivocation.rs:45-66`) signs the evidence with the reporter key. No code verifies that signature. `validate_slash` (`slash_evidence.rs:70-107`) → `validate_double_production` (`:109-159`): same producer and slot, different hash, accused match. Above the INC-I-236 gate it also checks genesis and both signatures (`:135-148`). VDF check only if `verify_vdf`. No membership check.
(c) Admission: gossip `handle_gossip_txs` → `handle_new_transaction` (`validation_checks/mod.rs:991-1050`) routes by `is_zero_flow()` (`:1011`). SlashProducer is not zero-flow (`types.rs:197`), so it goes to `add_transaction` → FeeTooLow(0,1). The relay verdict is Accept only if newly admitted and, for zero-flow txs, the type is in `is_authenticated` (`gossip_txs.rs:19-21`, `zero_flow_auth.rs:21-28`).
(d) Builder: `assembly.rs:274-298` skips the tx with "UTXO validation failed: InsufficientFee".
(e) Apply: `validate_block_for_apply` passes the structure. Then `tx_processing.rs:121` fails the fee, so the block is rejected in Full and Light. Replay tolerates it, so the Slash is queued. The epoch flush calls `slash_producer`.
(f) Rollback: the block has no snapshot (helpers omit the type), so the queued Slash survives.
(g) Replay or backfill: Replay tolerates every UTXO-stage error. Since INC-I-236, backfill verifies the block signature before it stores a block.
(h) Gates: the INC-I-236 field goes NetworkParams (`mod.rs:862`, defaults `defaults.rs:283/553/825`, devnet-only env `env_loader.rs:576-584`) → `ValidationContext` (`types.rs:260/304/394`) → 6 production construction sites (`tx_processing.rs:61`, `validation_checks/mod.rs:107` and `:306`, `assembly.rs:211`, `pool.rs:522` and `:978`). The apply site and the two validation_checks sites use `ConsensusParams::for_network`. The builder and the mempool use chainspec-applied `self.params`.
(i) INC-I-236 helper: `verify_block_signature` (`validation/block.rs:106-116`) checks the key `header.producer` and the message `signing_message()`. `slash_evidence.rs:142` already reuses it.
(j) Existing tests: `bins/node/src/node/inc_i_237_slash_tests.rs` (3 tests on devnet that assert no slash tx), `crates/core/tests/inc_i_173_fee_gate.rs:594` (slash rejected "at EVERY height", mainnet ctx), `inc_i_173_zero_flow_predicate.rs:215-258`, `crates/core/tests/it/inc_i_236_slash_evidence_authn.rs`, `crates/core/src/validation/tests_oracle_slash.rs`, `crates/network/src/sync/{equivocation.rs tests, adversarial_tests.rs}`, `bins/node/tests/it/inc_i_236_m4_*`, `bins/node/tests/inc_i_173_m3_f4_routing.rs`.

### Architectural Constraints & Invariants
- **INV-VALIDATION-001**: the four paths do not share one verdict today. The mempool uses `validate_transaction` and its own floor. The builder and apply use `validate_transaction_with_utxos`. Block validation uses a VDF-on or VDF-off variant by network and mode. The slash rule must be one function, called from each.
- **INV-VALIDATION-008 and INC-I-173 C1**: `allows_empty_io` is a height-free `const fn` with no `_` arm. SlashProducer must stay `false` there. The exemption must be height-gated AND conditioned on the evidence the same call just verified. It must not be a shape-based exemption (entry 1891).
- **INV-BOND-002 and INV-NETWORK-003**: a tx that cannot be mined must not be admitted or relayed. A relay happens once, under the original id, after a fresh admission.
- **INV-AUTH-003**: evidence is authentic only through header signatures, so the slashing gate must be ≥ the INC-I-236 gate on every network.
- **Epoch deferral (project rule)**: producer mutations apply at the epoch boundary. The scheduler list is frozen per epoch. If the exclusion happened mid-epoch, the scheduler inputs would change and nodes would fork.
- **INC-I-181**: the pending queue is in the state root above its gate. The slashing gate must be ≥ `inc_i_181_pending_root_activation_height`.
- **Replay parity**: a rule that must hold on replay must live in `validate_block_with_mode` (all modes) or in apply outside the Replay-tolerant branch, never only inside `validate_transaction_with_utxos`.
- **Module budget**: `utxo.rs` (1114), `tx_processing.rs` (619), `assembly.rs` (665), `validation_checks/mod.rs` (1380), `rewards.rs` (1546) are already over 500 lines. New logic goes into `slash_evidence.rs` (159) or new focused modules. The call sites get one line each.

### Blast Radius
- Direct: `slash_evidence.rs`, `utxo.rs` (exemption branch), `validation/block.rs` (cap before the mode split), `validation/types.rs` (+gate, +slashable-set field), `network_params/{mod,defaults,env_loader}.rs`, `mempool/{pool,zero_flow_auth}.rs`, `validation_checks/{mod,gossip_txs}.rs`, `rewards.rs` (handler + rebuild mirror), `apply_block/{helpers,tx_processing}.rs`, `storage/producer/{set_lifecycle,set_core}.rs`, `equivocation.rs`, `rpc/methods/transaction.rs`.
- Graph (`blast.py --hops 1`): `validate_transaction_with_utxos` has 26 dependents, `validate_block_with_mode` has 22. They are mostly tests and re-exports. The graph misses the method-internal production callers; grep confirmed them (`assembly.rs:291`, `tx_processing.rs:121`, `validation_checks/mod.rs:479`). `slash_producer`: the graph found 2 (a lower bound for methods); grep found 1 production caller (`set_core.rs:124`). `block_mutates_producer_set`: the graph found 0 callers; grep found `apply_block/mod.rs:173`.
- Indirect: the state root (ProducerSet + UTXO) of every block with a slash, snap sync, `rebuild_producer_set_from_blocks`, the attestation weight (`ingress.rs:102`), the withdrawal allowance (`withdrawal_economics.rs:162-190`), RPC producer status (`rpc/methods/producer.rs:111,221`), supply reporting.

### Brittleness Check
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
Details: (1) cross-module blast: core validation, mempool, network, storage and node apply/rollback do not share a direct dependency for this rule; (2) invariant gap: "one slash per producer" and "a slash spends the bond" are enforced by no module; (4) shared mutable state: the ProducerSet pending queue is written by apply, read by mempool snapshot, flush and snap with no single owner of slash state; (5) contract absence: the mempool and builder reach the slash rule through different validators (no shared contract)
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```
Signal 3 (data-flow reversal) does not apply.

## Blockers (detected → applied), with RED tests
RED style: tests that call a new API (gate field, ctx builder) fail to compile on `c8afea35`. Tests that use `Node::new_for_test` (devnet) and existing APIs fail at runtime there. Each blocker has at least one test that fails at runtime. Where `c8afea35` cannot mine a slash, the test applies it through `ValidationMode::Replay`, which tolerates the fee (B12).

| # | file:line | Class | RED test (name → exact assertion) |
|---|---|---|---|
| B1 | `bins/node/src/node/rewards.rs:449-458` (called `block_handling.rs:174-177`) | height-wiring gap (log-only, no gate to switch on) | `inc_i_255_b1_equivocation_above_gate_queues_one_slash` → `assert_eq!(slash_count(&node).await, 1)` after two validly signed blocks for one slot. Base: 0 |
| B2 | `crates/core/src/validation/utxo.rs:241-251` + `:284-297`; `transaction/types.rs:197` | fee gate | `inc_i_255_b2_signed_evidence_is_fee_exempt_above_gate` → `assert_eq!(validate_transaction_with_utxos(&slash, &ctx_above, &no_utxos), Ok(()))`. Base: `Err(InsufficientFee{actual:0,minimum:1,..})` |
| B3 | `bins/node/src/node/validation_checks/mod.rs:1011`, `crates/rpc/src/methods/transaction.rs:220` → `crates/mempool/src/pool.rs:751-757` | fee gate | `inc_i_255_b3_gossiped_signed_slash_is_admitted` → `assert!(node.handle_new_transaction(slash.clone()).await && node.mempool.read().await.contains(&slash.hash()))`. Base: false (FeeTooLow(0,1)) |
| B4 | `bins/node/src/node/validation_checks/gossip_txs.rs:19-21`; `crates/mempool/src/zero_flow_auth.rs:21-28`, `:118` | authentication gap (relay allow-list) | `inc_i_255_b4_signed_slash_gossip_verdict_is_accept` → `assert_eq!(node.handle_gossip_txs(vec![slash]).await, network::Verdict::Accept)`. Base: Ignore |
| B5 | `crates/core/src/validation/slash_evidence.rs:25-27`, `:135-148`; `transaction/data.rs:147`; `network/src/sync/equivocation.rs:51-53` | authentication gap (signatures only above INC-I-236; reporter field never verified) | `inc_i_255_b5_non_canonical_reporter_field_rejected` → `assert!(matches!(validate_transaction(&slash_with_random_reporter_sig, &ctx_above), Err(ValidationError::InvalidSlash(_))))`. Base: Ok. Also `inc_i_255_b5_slash_gate_not_below_auth_gates` → `assert!(p.<gate> >= p.inc_i_236_block_signature_activation_height && p.<gate> >= p.inc_i_181_pending_root_activation_height)` for all 3 networks (compile-RED) |
| B6 | `crates/core/src/validation/transaction.rs:326-331`; `validation/block.rs:187-261`, `:355-357` | authentication gap (builder and validator verify different things; no cap, one thread per tx) | `inc_i_255_b6_block_over_slash_cap_rejected_every_mode` → for `m in [Full, Light, Replay]`: `assert!(validate_block_with_mode(&b_cap_plus_1, &ctx_above, m).is_err())`. Base: Ok in Light/Replay |
| B7 | `slash_evidence.rs:109-159`; `bins/node/src/node/apply_block/tx_processing.rs:315-326`; `crates/storage/src/producer/set_lifecycle.rs:165-193`; `set_core.rs:123-126` | membership gap | `inc_i_255_b7_slash_of_unregistered_producer_rejected` → `assert!(matches!(err, ValidationError::InvalidSlash(_)))` on the mempool, builder and apply paths. Base: InsufficientFee / FeeTooLow. Also `..._already_slashed_or_queued_rejected`, same assertion |
| B8 | `crates/core/src/transaction/core.rs:484-509`; `validation/utxo.rs:1096-1111`; `network/src/sync/equivocation.rs:136-158`, `:155` | dedup gap | `inc_i_255_b8_detector_reports_each_offence_once` → check h1, h2, h2 again, h3 → `assert_eq!(proofs.iter().flatten().count(), 1)`. Base: 3. Also `inc_i_255_b8_two_slashes_same_producer_in_block_rejected` → `assert!(validate_block_with_mode(&b, &ctx_above, Light).is_err())`. Base: Ok |
| B9 | `bins/node/src/node/apply_block/helpers.rs:15-28` → `apply_block/mod.rs:173-176` → `rollback.rs:178-183` | rollback gap (INC-I-185) | `inc_i_255_b9_slash_block_mutates_producer_set` → `assert!(block_mutates_producer_set(&block_with_slash))`. Base: false. Integration: apply (Replay) then `rollback_one_block` → `assert!(producers.pending_updates_for(&accused).is_empty())` |
| B10 | `set_lifecycle.rs:165-193`, `crates/storage/src/producer/info.rs:223-227`; `tx_processing.rs:391-420`; `validation_checks/withdrawal_economics.rs:162-190`; `validation/tx_types.rs:375` | none of the seven classes: effect gap (the "burn" spends no UTXO) | `inc_i_255_b10_slash_spends_bond_and_lowers_supply` → `assert!(bond_utxos(&utxo, &accused).is_empty() && supply_after == supply_before - self_bond)`. Base: bonds unspent. Also `..._slashed_producer_cannot_withdraw` → RequestWithdrawal after the slash is `Err` |
| B11 | `set_lifecycle.rs:233-240` (no production caller); `bins/node/src/node/attestation/ingress.rs:102` | membership gap (INC-I-244) | `inc_i_255_b11_slashed_producer_has_no_attester_weight` → after the flush `assert_eq!(node.derive_attester_weight(&producers, &accused, h), None)`. Base: `Some(_)` |
| B12 | `bins/node/src/node/apply_block/tx_processing.rs:131-139` (used by `bins/node/src/operations/chain.rs:354`) | replay tolerance | `inc_i_255_b12_replay_rejects_slash_like_full` → `assert_eq!(apply(Full).is_err(), apply(Replay).is_err())` for a block with an unregistered-producer slash. Base: Full Err, Replay Ok |
| B13 | `crates/core/src/network_params/{mod.rs,defaults.rs,env_loader.rs}`; `validation/types.rs:260,304,394`; 6 ctx sites (see flow h) | height-wiring gap | `inc_i_255_b13_gate_defaults_and_env_scope` → mainnet/testnet `u64::MAX`, devnet 0, env honoured on devnet only. Plus a census test that every production `ValidationContext::new` site sets the gate (compile-RED) |

Not a blocker, but a constraint on the fix: `rewards.rs:1290-1297` (rebuild from blocks) must mirror every new apply effect (B10 burn, B7/B8 refusal). If it does not, the rollback fallback diverges. Test: `inc_i_255_rebuild_matches_apply_after_slash` → equal ProducerSet and UTXO roots.

Checked and not a blocker: no gossip-layer dedup keyed on (producer, slot) exists outside the detector (finding 76's key is not present in `crates/network/src/gossip`/`service`), so the second header reaches `check_block`.

## R-6 disposition (REQ-SLASH-006)
Not reachable at or above `inc_i_181_pending_root_activation_height`. Above that gate a non-empty `pending_updates` queue is hashed into the ProducerSet root component (`set_persistence.rs:122-129`). Snap install derives the root again from the installed state and refuses a mismatch (`fork_recovery.rs:314-338`). A made-up queued `Slash` therefore changes the root and is refused. A slash can exist only above the new gate, so the condition gate ≥ INC-I-181 gate (B5 test) closes it on every network. Proposed test: `inc_i_255_snap_with_fabricated_slash_is_refused`: install a snapshot whose ProducerSet has an extra `PendingProducerUpdate::Slash` → `assert!(computed_root != snapshot.state_root)` and memory stays on the old state.

## SSF (one recommendation)
The simplest fix that addresses the root cause: above a new `inc_i_255_slashing_activation_height`, make SlashProducer one canonical, zero-fee transaction. One shared core function (in `slash_evidence.rs`) verifies it: SEC-1..SEC-6 against a ctx-carried set of slashable producers, with the per-block cap SEC-7 checked before the mode split. Its apply effect spends the accused's Bond UTXOs with undo and takes a ProducerSet snapshot. The node submits it once per offence. This works because every path (mempool, builder, block validation, apply, replay) already calls into `validate_slash`. The bug is that this verdict is missing its state checks, its fee exemption and its effect, not that the paths differ.

Recommended anchors for the architect (to confirm or reject):
- Gate: `inc_i_255_slashing_activation_height`. Mainnet and testnet `u64::MAX`, devnet 0. Env `DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT` honoured on devnet only.
- Reporter field: fixed to the all-zero signature above the gate, and any other value rejected (removed from consensus, no key needed). Headers ordered by `hash()` ascending.
- Exclusion point: the next epoch boundary (the existing deferred `PendingProducerUpdate::Slash`). Bond UTXOs spent in the slash block itself (normal UTXO undo), so no RequestWithdrawal can drain them before the flush.
- Cap: 16 slash txs per block (32 Ed25519 verifies). Above the gate, evidence VDFs are not verified (the signature is the proof; this ends the thread-per-tx pre-pass cost for slashes).
  - As built (M2): `MAX_SLASH_TXS_PER_BLOCK` = 8. The full validator (mempool admission, Full block validation) still checks the evidence VDFs.

## Impact Analysis
### Existing Code Affected
- `validation/utxo.rs` fee branch: high risk. Every tx type passes through it. The new branch must be keyed on `tx_type == SlashProducer && h >= gate`, with the evidence already verified at `:34`.
- `mempool/pool.rs` routing and floor: medium. Two ctx sites. The zero-flow lane runs VDF-on evidence under the write lock (entry 1890). Above the gate, skip it.
- `apply_block/{helpers,tx_processing}.rs` and `rollback.rs`: high. The state root and undo must stay symmetric.
- `storage/producer/set_lifecycle.rs`: medium. `slash_producer` gets a status guard and keeps `bond_count` consistent with the spent UTXOs.
- `equivocation.rs`: low. A reported-offence set and a bounded `pending_proofs` (or the field removed, since it has no production reader).
### What Breaks If This Changes
- `inc_i_237_slash_tests.rs` (devnet, gate would be 0): the 3 "no slash" tests flip. Re-scope them to below the gate with a per-node gate override that is safe in parallel (not process env), and add counterparts above the gate.
- `inc_i_173_fee_gate.rs:594` and `inc_i_173_zero_flow_predicate.rs:215-258` stay GREEN only because the new ctx field defaults to `u64::MAX` and `allows_empty_io` is unchanged. Record that C1 is superseded above the slashing gate for authenticated slashes only.
- `docs/architecture.md:384` becomes false at HEAD (sync-docs step).
### Regression Risk Areas
- Pre-gate byte-identity of every verdict (mixed fleet). Needs a golden test over the slash tx at h = gate − 1.
- Self-fork if the builder and apply ctx disagree on the gate or the slashable set (INC-I-173 F2 lesson: both or neither).
- Devnet `genesis_hash()` reads the wall clock in tests (INC-I-252). Evidence tests must pin a genesis time.

## Specs Drift Detected (report only, no edits)
- `WHITEPAPER.md` §11.3 (lines 855-865): "100% of bond burned permanently", no reporter reward. The fix keeps both. It also says "Immediate exclusion from producer set". The code and the recommended design exclude at the next epoch boundary. This analysis records the difference and leaves the whitepaper as is.
- `WHITEPAPER.md` §6 (line 602): an unbonding period "retains slashing capability". The code pays withdrawals instantly (known open hotfix). The escape window is a risk (R3).
- `specs/protocol.md` §3.10 (341-366): says "Full header with VDF proof" and that validators verify VDF proofs. The authorship proof is the INC-I-236 signature. `reporter_signature` is listed but never verified. "Burns 100%" is not true in code (B10).
- `specs/security_model.md` §4.2 (294-327): check 4 "valid VDF outputs (proving actual computation)" is drift; the VDF is public. "Deflationary: reduces total supply" is false today (B10). "Burned, not redistributed" matches no reporter reward. §4.2.1 (338-339) cites `set_lifecycle.rs:165-178`; the function is now 165-193.
- `docs/architecture.md:161` ("100% bond burn") is false today. `:384` is true at `c8afea35` and false after the fix.

## Traceability Matrix
| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-SLASH-001 | Must | B1-B13 tests above; node: `inc_i_255_probe_{handler_builds_tx,mempool_admits,builder_includes,apply_full_accepts}`, `inc_i_255_parity_{reporter_signature,unknown_producer,already_slashed,pending_slash}_*`, `inc_i_255_n1_*`, `inc_i_255_b13_every_ctx_site_sets_slash_gate`, `inc_i_255_n4_*` (`bins/node/tests/it/inc_i_255_m1_*.rs`) | (architect) | M1: `crates/core/src/network_params/ordering.rs` (`effective_slashing_activation_height`), `crates/core/src/validation/slash_state.rs`, `crates/mempool/src/slash_admission.rs`, `bins/node/src/node/slash_submit.rs` (handler), gate wired at the 6 ctx sites (`pool.rs` x2, `validation_checks/mod.rs` x2, `tx_processing.rs`, `assembly.rs`) |
| REQ-SLASH-002 | Must | node: `inc_i_255_parity_*_rejected_on_every_path` (11 variants), `inc_i_255_b2_apply_light_and_replay_match_full`, `inc_i_255_b8_one_canonical_id_per_evidence_pair`, `inc_i_255_b3_gossip_relay_verdict_follows_admission`, `inc_i_255_below_gate_every_path_keeps_head_verdict`; M4 node `it`: `inc_i_255_m4_probe_{3_mode_parity,4_builder_packs_slash_only}` (`bins/node/tests/it/inc_i_255_m4_{block,builder}.rs`); M6 node `it`: `inc_i_255_m6_probe_{3_mode_parity,4_builder_packs_slash_only}` (`bins/node/tests/it/inc_i_255_m6_{block,builder}.rs`) | (architect) | M1: `crates/core/src/validation/slash_evidence.rs` (`slash_fee_exempt`), `crates/core/src/transaction/slash_canonical.rs`, fee consult at `crates/core/src/validation/utxo.rs` and `crates/mempool/src/pool.rs`; builder pre-check in `bins/node/src/node/production/assembly.rs`; apply ctx on chainspec params in `bins/node/src/node/apply_block/tx_processing.rs` |
| REQ-SLASH-003 | Must | node `it`: `inc_i_255_m2_probe_{burn,rollback,exclusion,attester,withdrawal}`, `inc_i_255_m2_b10_{epoch0_slash_block_burns_every_owned_bond,zero_bond_accused_slashable_only_with_locked_collateral,below_gate_slash_burns_nothing,both_backends_agree_on_burned_set_and_root}`, `inc_i_255_m2_b9_{epoch0_rollback_restores_active_status_and_bonds,across_boundary_rollback_matches_nodes_that_stopped_earlier,reorg_to_sibling_matches_node_that_never_saw_the_orphan}`, `inc_i_255_m2_exclusion_{epoch_gt0_from_next_epoch_boundary,epoch0_from_next_block}`, `inc_i_255_m2_second_slash_of_slashed_producer_is_refused`, `inc_i_255_m2_withdrawal_{admission_refuses_slashed_and_queued,resident_with_burned_inputs_is_evicted}`; M4 node `it`: `inc_i_255_m4_probe_{1_block_slash_first_rejected,2_block_wd_first_rejected,5_admit_slash_evicts_wd,6_refuse_wd_while_slash_resident,7_revalidate_evicts_wd}`, `inc_i_255_m4_below_gate_pair_block_verdict_unchanged`, `inc_i_255_m4_other_producer_withdrawal_with_slash_accepted` (`bins/node/tests/it/inc_i_255_m4_{block,mempool}.rs`); M6 node `it`: `inc_i_255_m6_probe_{1_block_slash_first_rejected,2_block_spend_first_rejected,5_admit_slash_evicts_spend,6_refuse_spend_while_slash_resident,7_revalidate_evicts_spend,8_in_block_bond_spend_rejected}`, `inc_i_255_m6_guard_{other_producer_bond_spend,accused_non_bond_spend,below_gate_verdict_unchanged}` (`bins/node/tests/it/inc_i_255_m6_{block,mempool,guards}.rs`) | (architect) | M2: `bins/node/src/node/apply_block/slash_burn.rs` (burn); M4: `crates/core/src/validation/slash_state.rs` (`bond_withdrawal_producer`, `validate_block_slashes`), `bins/node/src/node/validation_checks/slash_withdrawal.rs` (`check_slashed_withdrawals`); M6: `crates/core/src/validation/slash_state.rs` (`validate_slash_bond_spends`, `spends_owned_bond`, `SlashBlockBudget::spends_accused_bond`), `bins/node/src/node/validation_checks/slash_withdrawal.rs` (`check_slash_bond_spend`), `bins/node/src/node/production/assembly.rs`, `crates/mempool/src/slash_admission.rs` (`slash_bond_owners`, `bond_spend_refusal`, `withdrawals_for_slash`), `crates/mempool/src/pool.rs` |
| REQ-SLASH-004 | Must | node `it`: `inc_i_255_m2_probe_{cap,duplicate,handler}`, `inc_i_255_m2_cap_{over_cap_refused_on_every_path,exactly_cap_accepted,builder_never_exceeds_cap}`, `inc_i_255_m2_dup_{same_block_refused_on_every_path,builder_one_per_accused,second_admissible_once_first_leaves,refused_after_mined,revalidate_evicts_when_accused_queued,revalidate_evicts_when_accused_slashed}`, `inc_i_255_m2_handler_{new_evidence_after_report_creates_nothing,repeat_proofs_leave_one_tx,marks_per_accused,gate_crossing_creates_after_refusal,redelivered_sibling_bounded}`, `inc_i_255_m2_determinism_{one_id_one_mined,two_offences_one_mined}`, `inc_i_255_m2_relay_{invalid_evidence_never_forwarded,duplicate_accused_not_forwarded,queued_or_resident_not_forwarded}`; core `it`: `inc_i_255_m2_block_{over_cap_rejected_in_every_mode,at_cap_not_refused,duplicate_accused_rejected_in_every_mode,cap_judged_before_evidence_vdf,rules_inert_below_gate}`, `inc_i_255_m2_cap_constant_named_in_consensus`; mempool `it`: `inc_i_255_m2_mempool_{one_resident_slash_per_accused,revalidate_evicts_queued_accused,revalidate_evicts_slashed_accused}`; network `it`: `inc_i_255_m2_detector_{redelivered_sibling_one_pending,third_header_same_offence_one_pending,distinct_slots_kept,pending_bounded}`; M5 (AUDIT-P1-517): node `it`: `inc_i_255_m5_{redelivery_never_adds_a_slash,below_gate_warns_without_tx}`, network `it`: `inc_i_255_m5_detector_*` (7), core `it`: `inc_i_255_m5_{admissible,fits}_*` (4) | (architect) | (developer) |
| REQ-SLASH-005 | Must | GS-027; M5 (AUDIT-P1-517): node `it`: `inc_i_255_m5_probe_{decoy_genesis,decoy_vdf,decoy_oversize,many_bad_vdf}`, `inc_i_255_m5_valid_decoy_slashes_first_pair` | (architect) | M5: `crates/core/src/validation/slash_evidence.rs` (`evidence_header_admissible`, `double_production_evidence_fits`), `crates/network/src/sync/equivocation.rs` (`check_block_with`), `bins/node/src/node/header_auth.rs` (`detector_admits`), `bins/node/src/node/block_handling.rs` |
| REQ-SLASH-006 | Should | `inc_i_255_snap_with_fabricated_slash_is_refused` | (architect) | (developer) |
| REQ-SLASH-007 | Should | n/a (records) | n/a | memory.db |

## Assumptions
| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| A1 | All slash evidence from gossip, RPC or peers' blocks is untrusted | We never believe an accusation until we check the producer's own signatures | Yes |
| A2 | "Whole bond" = the accused's own Bond UTXOs; delegators' principal is not burned (security model §4.2.1) | Only the cheater's own deposit is destroyed | No |
| A3 | `ConsensusParams::for_network(net).genesis_hash` equals the live chain genesis on a running node, because block validation (`validation_checks/mod.rs:107/306`) already depends on it | The chain-identity check in evidence uses the same source as block checks | No (inferred, not measured) |
| A4 | Evidence for any past slot after the gates is slashable (no expiry) | An old double-sign can still be punished later | No |
| A5 | Supply is observable as the UTXO total (so spent bonds lower it) | Burning coins shows up in the supply number | No |
| A6 | Ordering inside a block: a RequestWithdrawal that comes before the slash in the same block wins; the slash then burns what is left | The order of transactions in a block decides the result | No |

## What I Don't Understand
- How RPC/explorer supply is computed (UTXO sum or an emission formula). This decides whether R-3's supply check needs a burn counter.
- Whether delegated weight should be removed from delegators at once or at the flush (`cleanup_all_delegations` runs at the flush today).
- Whether a producer with a pending Exit or RequestWithdrawal (status Unbonding) is still slashable. `is_active` includes Unbonding.
- How the GS-027 injector gets the scheduled producer's throwaway key without reading private key material. I assume the runner generates the devnet keys itself.

## Identified Risks
- R1 Flood regression (INC-I-237 class): mitigation = canonical id + detector reported-set + refusal of queued or slashed producers at admission + a relay only on fresh admission.
- R2 Consensus DoS by evidence cost: mitigation = cap before the mode split + no evidence VDF above the gate.
- R3 Escape by instant withdrawal between the double-sign and the slash mining: not closable here (the unbonding delay is separate follow-up work). Record it in the decision row.
- R4 Self-fork from ctx drift across the 6 sites: mitigation = census test + both-or-neither wiring.
- R5 Tests flipping on devnet (gate 0): mitigation = re-scope the existing "no slash" tests with an explicit gate override.

## Out of Scope (Won't)
- Follow-up work, not this incident: the unbonding delay and binding the signer to the producer.
- No whitepaper edits and no reporter reward.
- No mainnet activation height, push, release or deploy.
- Not covered: the gossip message id (INC-I-240) and the remaining AUDIT-P1-203 gap.

## Follow-up INC-I-259: a queued slash lost to a whole-bond withdrawal

- **Symptom (live GS-028):** the accused withdraws its whole bond at W and equivocates; the slash is mined at S. The economics hold (the locked Unbonding output is burned at S, the whole bond leaves supply), but after the next reward-epoch boundary the accused is `exited`, not `slashed`, on every node, and each node logs `Deferred slash failed` with `[STOR041]`.
- **Root cause:** the boundary applies the queue in FIFO order (`set_core.rs`, `apply_pending_updates_with_cap`). `ProducerInfo::apply_withdrawal` (`info.rs`) sets `Exited` at bond count 0; `ProducerSet::slash_producer` (`set_lifecycle.rs`) then refused every status that is not Active or Unbonding. The same happens when a boundary falls between W and S. `register` accepts an `Exited` key, so the lifetime bar was lost.
- **Fix (DC-5):** `slash_producer` refuses only an already `Slashed` producer (`[STOR041]` kept for that case) and slashes Active, Unbonding and Exited alike. `apply_withdrawal` never moves a `Slashed` producer to `Exited`. `register` already refuses a `Slashed` key with `[STOR011]`. ProducerSet only; the UTXO paths and the state-root encoding are unchanged.
- **Parity:** block apply (`state_update.rs`) and rebuild from blocks (`rewards.rs`) both call `apply_pending_updates_with_cap`; rollback restores a whole ProducerSet snapshot or rebuilds through the same function. No rollback path holds its own status logic, so none changed.
- **Consensus shape (INV-12):** (1) a user transaction (RequestWithdrawal) and a SlashProducer reach this path; (2) producer equivocation evidence reaches it; (3) the new behaviour differs only when a Slash update is applied, and a SlashProducer is mineable only at or above the effective slashing activation height (below it the fee exemption is off and the input-less transaction fails the base-fee gate); `inc_i_255_slashing_activation_height` is `u64::MAX` on mainnet and testnet. Below that gate every reachable input is bit-identical, so no new activation height.
- **Tests:** `crates/storage/tests/it/inc_i_259_slash_race.rs` (both queue orders, the boundary split, no downgrade of `Slashed`, `[STOR011]` on re-register, `[STOR041]` on a second slash, the Unbonding and unknown-producer arms).

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies | Status |
|----|------|-----------------|----------------------|--------------|--------|
| M1 | RED for every blocker + mineable authenticated slash on all paths | crates/core/src/{validation,transaction,network_params}, crates/mempool/src/{pool,zero_flow_auth}.rs, bins/node/src/node/{validation_checks,production,rewards.rs}, crates/rpc/src/methods/transaction.rs, crates/network/src/sync/equivocation.rs | REQ-SLASH-001, REQ-SLASH-002 | none | PENDING |
| M2 | Burn effect, exclusion, rollback, flood control | bins/node/src/node/{apply_block,rollback.rs,rewards.rs,attestation}, crates/storage/src/producer, crates/core/src/validation/block.rs, crates/network/src/sync/equivocation.rs, crates/mempool/src | REQ-SLASH-003, REQ-SLASH-004 | M1 | PENDING |
| M3 | GS-027 gauntlet, snap disposition, records | scripts/gauntlet-gs027.sh, scripts/test_gauntlet_gs027.sh, bins/node/examples, bins/node/src/node/fork_recovery.rs (test only), .omega/memory.db | REQ-SLASH-005, REQ-SLASH-006, REQ-SLASH-007 | M2 | PENDING |

━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.92, measured — 13 blockers each confirmed by reading the cited lines at c8afea35)
Reasoning: the fix spans core validation, mempool, network detection, builder, apply, rollback and storage, which interact through one shared verdict and the state root, and the brittleness check reads 4/5.
━━━━━━━━━━━━━━━━━━━━━━
