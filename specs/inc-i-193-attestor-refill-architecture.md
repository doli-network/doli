# INC-I-193 — Attestor refill of the capped active list (architecture)

Status: DESIGN COMMITTED by the owner (RUN 557, 2026-09-14). Worktree branch `fix/inc-i-193-attestor-refill`.
Verified against `fd14ca03`. Every `file:line` below was read at that commit.

## 1. Problem and decision

The capped active list (`ACTIVE_PRODUCERS_CAP = 50`) is re-derived at every epoch boundary from candidates that
attested `>= MIN_ATTESTATION_MINUTES` **and** produced `>= min_produced` blocks in the just-completed epoch
(`crates/core/src/epoch_state/mod.rs:242-246`). A producer outside the list cannot produce, so it can never satisfy
the second clause: the list is shrink-only until the `< producer_list.len()/3` fallback (mod.rs:262) re-admits
everyone. Measured on mainnet: 31 producers holding 5.5% of the stake made every block for 4.5 days.

**Decision (owner, closed):** at a NEW activation height delete the `produced >= min_produced` clause. After the
height the only per-epoch qualification is `attestation minutes >= MIN_ATTESTATION_MINUTES (30)`. Unchanged: sort by
`registered_at` asc with pubkey tiebreak, `ACTIVE_PRODUCERS_CAP = 50`, the `< len/3` fallback, the
`[EPOCH] Frozen producer list` log line, all wire formats and `EpochState` fields. Rationale: rewards are paid at the
boundary by bonded weight, so production frequency carries no economic weight; commit 3232ae1b contradicted the
design commit ab24182b.

## 2. Exact change list (M1)

| # | File:line | Change |
|---|-----------|--------|
| C1 | `crates/core/src/network_params/mod.rs:261` (after `inc_i_190_floor_bound_activation_height`) | Add `pub inc_i_193_attestor_refill_activation_height: u64` with the doc-comment shape of :250-261, including the three-question gate (see §2.1). |
| C2 | `crates/core/src/network_params/defaults.rs:114` (mainnet), `:458` (testnet), `:733` (devnet) | Add the field next to the `inc_i_190` line in each arm: mainnet `u64::MAX`, testnet `195_000`, devnet `u64::MAX`. Testnet comment: tip 191_020 measured 2026-09-14; first boundary `>= 195_000` is `h = 195_012` (epoch 5417, 36 blocks/epoch, defaults.rs:396). |
| C3 | `crates/core/src/network_params/env_loader.rs:316-323` | Twin arm: mainnet locked to defaults; non-mainnet `env_parse("DOLI_INC_I_193_ATTESTOR_REFILL_ACTIVATION_HEIGHT", ..)`. `chainspec_loader.rs` loads no activation height (grep: 0 hits) — no change there. |
| C4 | `crates/core/src/epoch_state/mod.rs:48-49` | Add `pub inc_i_193_attestor_refill_activation_height: u64` to `EpochDerivationInput` (last field, after the `inc_i_190` one). |
| C5 | `crates/core/src/epoch_state/mod.rs:242-246` | Gated retain: `let minutes_only = input.height >= input.inc_i_193_attestor_refill_activation_height; with_reg.retain(|(pk,_)| { let mins = ..; if minutes_only { mins >= MIN_ATTESTATION_MINUTES } else { let produced = ..; mins >= MIN_ATTESTATION_MINUTES && produced >= min_produced } });`. `min_produced` (:237-238) stays computed (it is also logged by the rebuild twin). |
| C6 | Construction sites forced by C4: `bins/node/src/node/apply_block/post_commit.rs:354-359`, `bins/node/src/node/fork_recovery.rs:702-706`, `bins/node/tests/it/inc_i_178_m6_replay_harness.rs:551` | Plumb `self.config.network.params().inc_i_193_attestor_refill_activation_height` (harness: `params.inc_i_193_attestor_refill_activation_height`). Existing test literals (`epoch_state/tests.rs` x10, `tests_m2.rs` x5, `tests_floor.rs:737`) get `u64::MAX` so legacy behaviour stays locked. |
| C7 | `bins/node/src/node/rewards.rs:1062-1083` (`rebuild_epoch_state_from_blocks`) | Identical gate: `let minutes_only = epoch_boundary_h >= self.config.network.params().inc_i_193_attestor_refill_activation_height;` then the same two-arm retain as C5. Height source is `epoch_boundary_h` (rewards.rs:583, `= epoch * blocks_per_epoch`, `epoch = target_height / blocks_per_epoch` at :578); params accessor is the one already used at :859-867. Extend the `[STARTUP][TIER] Demoted` line (:1078-1081) with `mode={legacy|minutes-only}`. Never key on `target_height`, `current_h`, or history availability (learning #416). |
| C8 | `crates/core/src/consensus/constants.rs:82-90` | Rewrite the `TIER_PROMOTION_ACTIVATION_HEIGHT` doc: "After: candidates below `MIN_ATTESTATION_MINUTES` in the just-completed epoch are removed; before `inc_i_193_attestor_refill_activation_height` a `>= 80% of expected blocks produced` clause also applied (INC-I-193 shrink-only defect, removed at that height). Survivors sorted by registered_at asc (pubkey tiebreak); first 50 form the list. Bond count is never an input." |
| C9 | `specs/security_model.md:457` (§5.2) | Replace `(plus a blocks-produced check, INC-I-193)` with `(before inc_i_193_attestor_refill_activation_height: also >= 80% of expected blocks produced — the INC-I-193 shrink-only defect; after: attestation minutes only)`. |
| C10 | `specs/engine-parts.md:748` | FLAGGED (found by grep, outside the owner's list): same sentence pattern as C9. Apply the same rewrite or leave — owner's call, one line. |
| — | `docs/architecture.md` | Does not describe the clause (grep `ACTIVE_PRODUCERS_CAP\|MIN_ATTESTATION_MINUTES\|promotion\|80%`: 0 relevant hits) — no change. |

No `CURRENT_PROTOCOL_VERSION` bump, no `EPOCH_STATE_FORMAT_VERSION` bump (the `EpochState` struct is untouched), no
`HardForkSchedule` entry, no Cargo version bump.

### 2.1 Three-question gate (INC-I-075 / INV-12) — text for the C1 doc comment
Q1: YES — a user-submittable `Register` tx adds a candidate that reaches the retain. Q2: YES — the attestation
pattern decides `mins`. Q3: NO — for the same inputs the post-change retain admits producers with `produced <
min_produced`, so `active_list` (the schedule) differs. Verdict: activation height REQUIRED. Own field, never bundled.

## 3. Gate semantics (truth table; `H` = boundary height, `AH` = the new field)

| Branch | Condition | `active_list` | Changed? |
|--------|-----------|---------------|----------|
| B0 | `H < TIER_SYSTEM_ACTIVATION_HEIGHT` (0) or `producer_list.len() <= 50` | `producer_list` (mod.rs:267-269, rewards.rs:1100-1102) | no |
| B1 | `len > 50` and `epoch <= 1` | no retain; seniority sort, take 50, `< len/3` fallback | no |
| B2 | `len > 50`, `epoch > 1`, `H < AH` | retain `mins >= 30 && produced >= min_produced` (legacy) | no |
| B3 | `len > 50`, `epoch > 1`, `H >= AH` | retain `mins >= 30` | **yes** |
| — | after B1-B3 | sort registered_at asc/pubkey, `take(50)`, `if len < producer_list.len()/3 || empty → producer_list` | no |

INV-193-01: `derive_at_boundary` (mod.rs) and `rebuild_epoch_state_from_blocks` (rewards.rs) evaluate the gate on the
same boundary height with the same `NetworkParams` accessor and produce a bit-identical `active_list` for every
reachable input. Pre-existing residual (not introduced here): the rebuild twin feeds `attestation_accum[0]` from a
local block scan (rewards.rs Fix #4B/#4C) — input equality is the INC-I-054 class, out of scope.

## 4. Test plan (RED first, then GREEN)

Location: new file `crates/core/src/epoch_state/tests_inc_i_193.rs`, registered with `#[cfg(test)] mod
tests_inc_i_193;` in mod.rs:16-21 (`tests.rs` is 1012 lines, over the 800-line test cap). Helper pattern to copy:
`tests_m2.rs:447-482` (`make_pubkey(seed)` :14, `reg` map `i*10`, `EpochState::genesis()` + `prev.epoch`,
`prev.attested_sets[0]`, `prev.attestation_accum[0].insert(pk, (0..30).collect())`, `prev.blocks_produced.insert`,
`EpochDerivationInput { height: 1800, epoch: 5, blocks_per_epoch: 360, .. }`). Sizes: 80 producers →
`min_produced = (360/80)*80/100 = 3`, `len/3 = 26`.

| ID | Setup | RED assertion (fails on fd14ca03) / GREEN |
|----|-------|-----------------|
| TEST-193-01 | AH=`u64::MAX`; 80 in `producer_list`; producer X has the earliest `registered_at`, 30 minutes, 0 blocks; 31 others have 30 min + 11 blocks | X NOT in `active_list` (legacy locked). GREEN already — a lock, not a RED. |
| TEST-193-02 | Same, AH=`height` | RED: X in `active_list` at index 0 (seniority). |
| TEST-193-03 | 80 producers, previous `active_list` = 31, all 80 attest >= 30 min, only the 31 produced; AH=`height` | RED: `active_list.len() == 50` and equals the 50 smallest `registered_at`. Extended (outcome metric §7): run 3 consecutive boundaries (prev = output, re-seed accumulators): at boundary 2, 19 of 50 go dark (0 min, 0 blocks); at boundary 3 they attest 30 min with 0 blocks. Print the sequence: AH=`u64::MAX` → `50 -> 31 -> 31`; AH=0 → `50 -> 31 -> 50`. |
| TEST-193-04 | 60 producers, every candidate has 30 min and 10 blocks (both predicates true) | `active_list` identical for AH=`height+1` and AH=`height` (bit-identity at the gate edge). |
| TEST-193-05 | derive/rebuild parity. No shared pure function exists (the rebuild twin is inline at rewards.rs:1041-1116) → node-level test `bins/node/tests/it/inc_i_193_rebuild_parity.rs` + `mod` line in `bins/node/tests/it/main.rs`. Reuse `inc_i_178_m6_replay_harness.rs` (`replay_epoch` :248, `healthy_epoch` :686, `degraded_epoch` :697, `demotion_survivors` :520 already builds the derive input from node state). | After replaying one epoch with >50 producers where a subset attests >= 30 min but produces 0 blocks: `node.rebuild_epoch_state_from_blocks(boundary_h)` (rewards.rs:562) yields `epoch_state.active_list == derive_at_boundary(..).active_list`, once with AH > boundary and once with AH = boundary. UNVERIFIED: how the AH reaches `Node::new_for_test` params (env var via `load_from_env` vs direct field set) and whether `replay_producer_count()` (:71) exceeds 50 — the test-writer must confirm both. |
| TEST-193-06 | AH=`height`; 80 producers; producer Y is in the previous list with 29 minutes and 11 blocks; 40 others have 30 min | Y NOT in `active_list` (attestation demotion survives). |
| TEST-193-07 | `crates/core/src/network_params/tests.rs` after :591, pattern :489-508 and :595-619 | defaults: mainnet `u64::MAX`, testnet `195_000`, devnet `u64::MAX`; testnet `> 191_020` (tip at pin), `!= 0`, not equal to `inc_i_190_floor_bound_activation_height` / `epoch_prune_activation_height`; env override honoured off mainnet only. Also add the field to the per-network asserts in `crates/core/tests/it/inc_i_204_m5_activation_height.rs:397/429/460`. |

Command: `cargo test -p doli-core --lib epoch_state::tests_inc_i_193` (RED before C5, GREEN after) and
`cargo test -p doli-node --test it inc_i_193_rebuild_parity` (RED before C7).

## 5. Failure-Modes matrix

| FM | Trigger | Blast radius | Detection signal | Mitigation |
|----|---------|--------------|------------------|------------|
| FM-1 derive/rebuild gate mismatch | C5 and C7 read different heights or params (e.g. `target_height` instead of `epoch_boundary_h`) | a node that rebuilds after rollback/reorg schedules a different `active_list` → it rejects valid blocks or produces off-schedule → fork | `[STARTUP][TIER] Active production list` count differs from peers' `[EPOCH] Frozen producer list ... active_list=` for the same epoch; `scripts/fork-monitor.sh` | TEST-193-05; both sites use `params()` + boundary height (C7) |
| FM-2 AH crossed on a mixed testnet fleet | any of the 18 local nodes still on the old binary at `h = 195_012` | old nodes keep B2, new nodes take B3 (only if `len > 50` — see FM-6) → fork at that boundary | tip-hash divergence on RPC 8500-8517; two distinct `Frozen producer list for epoch 5417` lines | deploy all 18 before 195_000 (~4_000 blocks ≈ 11 h at ~10 s/block); re-measure the tip with `getChainInfo` right before deploy |
| FM-3 attesting-but-non-producing member | a listed producer attests but cannot produce (VDF, key, config) | 1/50 of slots missed until it stops attesting (accepted by the owner) | per-producer missed-slot rate in seed log | none by design; the 30-minute demotion still removes silent nodes |
| FM-4 en-masse re-entry | first boundary `>= AH` on mainnet re-admits every benched attester (19+ nodes) | one epoch of missed slots if their producers cannot produce (BLS-mismatch nodes CAN produce, they only lose rewards) | `active_list` jumps to 50 then missed-slot rate | accepted; they were attesting, so they are online; VDF fallback covers misses |
| FM-5 snap-synced node rebuild | `rebuild_epoch_state_from_blocks` with `has_incomplete_history` (rewards.rs:592-613) | uses all active producers + Light mode; the gate keys on `epoch_boundary_h`, so predicate choice never depends on local history — only inputs can differ (pre-existing INC-I-054 class) | `[EPOCH_REBUILD] Incomplete block history` | unchanged; next boundary re-derives in post_commit |
| FM-6 testnet cannot exercise the gated path | local testnet has 5 producers `<= 50` → B0 at every boundary | a green testnet deploy proves NO-FORK / NO-REGRESSION only, not the refill | `Frozen producer list ... 5 producers, active_list=5` | TEST-193-03 (3-boundary simulation) + TEST-193-05; pin mainnet only after a code review of the two gated sites (C5, C7) |
| FM-7 mainnet pin | none: mainnet arm is `u64::MAX` and env-locked | none | — | separate HC-6 decision session; pattern of `inc_i_190` (defaults.rs:107-114) |

Protection-surface interactions (`v_protection_surface`): PM-190-01 (floor-fallback cap bound) and PM-190-02
(Light window) share the epoch-boundary trigger but act on `producer_list` before the tier stage; this change touches
only the `with_reg` retain after it. Scale assumption: calibrated for a registry of 80-100 with cap 50 (mainnet
2026-08/09); at `len <= 50` (testnet, devnet) the mechanism is dormant. Register as PM-193-01 (tuning of the tier
promotion filter): trigger = B3, action = minutes-only retain, thresholds unchanged (30/60 min, cap 50, 1/3 fallback).

## 6. Deploy answers

Q1 consensus rules change → YES → activation height (C1-C3). Q2 block content change → NO (no header, bitfield,
coinbase or tx-order change) → a rolling deploy BEFORE the AH is safe; every node must run the new binary before
`h = 195_012`. Testnet is LOCAL (`~/testnet/`, 18 nodes, `scripts/testnet.sh`; seed RPC needs ~120 s after a restart —
restart the seed last). Mainnet height: separate owner decision; not pinned here.

## 7. Outcome metric

Live testnet can only prove NO-FORK / NO-REGRESSION (FM-6). Probe, run after `h >= 195_012`:
`grep -h "Frozen producer list for epoch 5417:" ~/testnet/logs/*.log | sed 's/^.*\[EPOCH\]/[EPOCH]/' | sort | uniq -c`
→ exactly ONE distinct line, `... 5 producers, active_list=5 ...`, with a count equal to the number of nodes that
reached the boundary (log path: `LOG_DIR="$HOME/testnet/logs"`, `scripts/testnet.sh:21`; per-node files `seed.log`,
`n1.log` … `n17.log`), plus one tip hash across RPC 8500-8517 (`scripts/fork-monitor.sh`).

Behavioural metric (owner-visible, re-runnable): `cargo test -p doli-core --lib
epoch_state::tests_inc_i_193::test_193_03_three_boundary_sequence -- --nocapture` prints
`pre-AH active_list: 50 -> 31 -> 31 (stuck)` and `post-AH active_list: 50 -> 31 -> 50 (refilled)`. Before C5 the
post-AH line reads `50 -> 31 -> 31` (RED); after C5 it reads `50 -> 31 -> 50` (GREEN).

Mainnet is the only place the live metric exists; probe for the later decision session, on a seed
(`/var/log/doli/mainnet/seed.log`): `grep "Frozen producer list for epoch" seed.log | tail -20` — before the AH
`active_list=` only shrinks between `< 1/3` fallbacks; after the AH `active_list = min(50, attesters >= 30 min)`.

## 8. Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Attestor refill gate | network_params, epoch_state, node/rewards (+ constants/spec comment lines C8-C10) | C1, C2, C3, C4, C5, C6, C7, C8, C9, C10 | M | None |

Commit type `fix(consensus):`; commit body carries the three-question answers (§2.1) and this Failure-Modes block.
Deployment and the §7 probes are an ops step outside the milestone loop. Follow-up question (one line, owner):
should the cap become a NetworkParams field so testnet can exercise B3 live? Not proposed here.

## 9. Resource cost

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one predicate per candidate per epoch boundary, mod.rs:242-246 and rewards.rs:1071-1075; the post-AH arm evaluates one comparison fewer)
  Memory:   0 (observed — one `u64` added to `NetworkParams` and to the stack-only `EpochDerivationInput`; `EpochState` unchanged)
  IO:       0 (observed — no new reads; the rebuild scan at rewards.rs:984-1026 is unchanged)
  Network:  0 (observed — no wire format touched)
  Disk:     0 (observed — no persisted struct changes; one extra `mode=` token on an existing startup log line)
  Latency:  0 (observed — boundary derivation cost unchanged; no lock added, `producer_set.read()` at rewards.rs:1053 already existed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the change deletes a clause; the only addition is the activation-height field that consensus safety requires (§2.1).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
