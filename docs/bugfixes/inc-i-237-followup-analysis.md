# INC-I-237 follow-up — bugfix analysis (RUN 573, resumed incident)

> Analyst, 2026-10-12. Branch `refactor/inc-i-237-gossip-relay-verdict`, V = `ba55599a` (M5). Base origin/main `d0e15686` (v6.43.3).
> Inputs: `docs/bugfixes/inc-i-237-followup-session-prompt.md` (main checkout), release pass 2 reports (QA FAIL, review PASS, audit FAIL), memory.db incident entries 3005-3033.
> Code is the source of truth. Every `file:line` below was read at V unless marked otherwise.
> Graph: graphify (`graphify-out/graph.json`, refreshed at V) + `blast.py`. graphify does not resolve Rust `self.method()` edges, so every caller list was cross-checked with grep.

━━━ TRIAGE VERDICT ━━━
Path: FAST (milestone loop M6-M10, with ONE design gate between M7.a and M7.b)
Confidence: 0.75
Reasoning: The root causes are diagnosed and the open items are bounded residuals at verified locations (all prompt pointers confirmed, see "Pointer check"). DEEP trigger 4 applies only weakly: AUDIT-P1-506 had ONE incomplete fix (M5). The release audit traced the residual flow step by step and named the failing test (>= 9 cycled known parents), so the gap is a missing test partition, not an unknown cause. More investigators cannot add evidence that the failing-first test does not add. REQ-GRV-007 has an INFERRED retainer. Code reading cannot settle it: only an instrumented run can. So M7 starts with a measurement half (M7.a), not with a 5-investigator diagnosis. M7.b (the cap) is a system-dynamics design with a known failure mode (INC-I-114-E2, stale re-forward after dedup expiry). It needs the architect and a failure-mode matrix at the gate, not the deep path. Brittleness 2/5 = LOCALIZED.
━━━━━━━━━━━━━━━━━━━━━━

## Scope

| Item (user order) | Milestone | Files (production) |
|---|---|---|
| 1. AUDIT-P1-506 residual + F7 + AttestSeen wall-clock prune | **M6** | `bins/node/src/node/gossip_verdicts.rs`, `bins/node/src/node/attestation/ingress.rs` (`AttestSeen`); prune call site (see M6) |
| 2. REQ-GRV-007 `soak_10min` retained heap | **M7.a** measure, design gate, **M7.b** cap | `vendor/libp2p-gossipsub/src/{behaviour,time_cache}.rs` (accessor; cap only if the gate picks it), `crates/network/src/gossip/config.rs`, `crates/network/src/service/` (gauge), `bins/node/tests/inc_i_237_m5_soak.rs` |
| 3. Zero-fee admission signature check (AUDIT-P2-001), multi-hop relay again | **M8** | admission predicate on the 3 `add_system_transaction` call sites; `bins/node/src/node/validation_checks/gossip_txs.rs` |
| also-should: spec / docs / registry drift not folded into M6-M8 | **M9** | `specs/`, `docs/`, `.claude/skills/network/SKILL.md`, memory.db protection rows |
| 4. Push blockers: message rewords + 0-hit disclosure scan | **M10** (last history operation) | commit messages M1-M5 (+ any later), `crates/network/tests/it/inc_i_237_m5_disclosure.rs:100-102` |

Ordering note: items 1-4 keep the user's order. M10 (item 4) runs after M9 because a reword rewrites every later SHA, so it must be the final history operation before the release pass. This is a dependency, not a change of priority.

Out of scope (separate incidents, already open): INC-I-238 (unsigned-field re-mint), INC-I-239 (vote hex-case veto, AUDIT-P1-507), INC-I-240 to INC-I-244, INC-I-245 / AUDIT-P1-508 (BLS pool eviction empties a producer's commitment: the pool-recency fix belongs there, NOT in M6), SEC-FIXVERIFY-R572-02 (0xAA announce fetch path), R572-03 (origin token refund).

## Summary (plain language)

The branch rebuilt how a DOLI node decides to pass gossip on. Four gaps remain before it can ship:
1. A peer can still make a node pass on unlimited copies of real attestations. It does this by cycling more than 8 known blocks through a small BLS cache. Each pass-on uses up the node's allowance at its neighbours, and then the neighbours drop the node's real attestations.
2. Under a 10-minute flood the test process keeps about 300 MiB. The limit is 64 MiB. The holder of that memory is not yet measured. We measure it first, and then we cap it.
3. Four free transaction types (delegate, revoke, add/remove maintainer) never travel more than one hop today. This is a safety stop-gap. We add a signature check at admission, so genuine ones travel again and forged ones still never do.
4. Six commit messages hold a local path with the workstation account name. We reword them and scan every commit before the first push.

## Pointer check (follow-up prompt vs code at V)

| Prompt pointer | Verdict | Evidence at V |
|---|---|---|
| `gossip_verdicts.rs:45-51,76` BLS newly-pooled exception | CONFIRMED | `:42-44` `first = attest_seen.insert((attester, slot))`; `:45-51` `relay = \|newly_pooled\| first \|\| newly_pooled`; `:59`, `:66` `relay(false)`; `:76` `relay(newly_pooled)` |
| `pool.rs:28,58-62,68` 8-parent FIFO | CONFIRMED | `:28` `MAX_PARENTS = 8`; `:58-62` evicts the oldest parent and all its signatures; `:64-68` a re-insert returns `true` |
| `inc_i_237_m5_soak.rs:616` heap assert | CONFIRMED | `:616-617` `assert!(r.retained_mib < 64.0, ...)` |
| `config.rs:70` TTL 300 s, `:67-68` "30 MB worst case", const assert | CONFIRMED | `:70`; `:67-68`; `:74-78` (>= 180 s and >= `ATTEST_STALE_SLOTS` x `SLOT_DURATION`) |
| Push blocker "reword the M5 commit message" | **INCOMPLETE** | The same absolute session path (with the workstation account name) is in **6** messages: `07a8b122` (M1, line 63), `0ac1123e` (M1.5, :153), `a5d7d4e1` (M2, :57), `0e69eb5d` (M3, :65), `19ce3987` (M4, :34), `ba55599a` (M5, :34). M0 is clean. |
| "4 zero-fee tx types" | **INCOMPLETE** | `TxType::allows_empty_io` (`crates/core/src/transaction/types.rs:187-193`) also returns `true` for `Registration`. So `is_zero_flow()` (`crates/core/src/transaction/core.rs:479-481`) covers 5 types. A zero-flow Registration passes validation only in the genesis window (`crates/mempool/src/pool.rs` comment at `add_system_transaction`, citing `validation/registration.rs:37-71`). |
| `AttestSeen` "pruned only on block apply" (review F1) | CONFIRMED | only call: `bins/node/src/node/apply_block/post_commit.rs:450` `self.attest_seen.prune(block.header.slot)` |

Offending M5 line (shape only; the path itself is not repeated here so this file passes the M10 scan):
```
  probe: <absolute temp-dir session path that embeds the workstation account name>/scratchpad/m5-node/probe.sh <doli-node binary> <doli cli binary> <tag>
```
The five other messages carry the same shape with `m1-node`, `m15-node`, `m2-node`, `m3-node`, `m4-node`. The probe script is not in the repo, so the Outcome-Metric probe is also not re-runnable by anyone else.

## Trust Boundaries

Trust model: inherited from RUN 572 REQ-GRV-SEC-001 (user-approved). All gossip and RPC input is untrusted. A producer-set member key may be malicious (M5-DEV-F1 precedent). No new trust boundary is added; M8 changes what is admitted at an existing boundary.

| # | Entry point | Untrusted fields | Consumers |
|---|---|---|---|
| TB1 | gossip `/doli/attestations/1` | every field; Ed25519 covers only `DOMAIN \|\| block_hash \|\| slot` (`crates/core/src/attestation/message.rs:103-111`); `height`, `attester_weight`, `bls_signature` are unsigned | swarm prefilter (`relay_decision.rs:134-140`, `staleness.rs:15-33`), per-peer attestation bucket (`rate_limit.rs:111-114`), node `on_new_attestation` -> `AttestSeen`, `ParentSignaturePool`, `minute_tracker`, finality weight, relay verdict |
| TB2 | gossip `/doli/txs/1` (0xBA batch / legacy single) | whole tx, incl. zero-flow payload + signatures | swarm per-tx bucket (`gossip_intake.rs:201,237`), node `handle_gossip_txs` -> `handle_new_transaction` -> mempool, relay verdict, own block assembly |
| TB3 | RPC `sendTransaction`, `submitMaintainerChange` | whole tx; maintainer signatures are parsed and COUNTED only (`crates/rpc/src/methods/governance.rs:245-262`), never verified | origin token, `add_system_transaction` (`transaction.rs:222-224`, `governance.rs:286`), origin publish |
| TB4 | raw gossip ingress (any topic, any peer) | message bytes | vendor `handle_received_message` inserts the id into the 300 s duplicate cache (`vendor/libp2p-gossipsub/src/behaviour.rs:1718`) BEFORE any DOLI per-peer budget runs. Memory grows with raw distinct-id ingress. |

## Architecture Context

### Module Boundaries
- `vendor/libp2p-gossipsub` (0.46.1 + DOLI send_queue patch, 7 `DOLI INC-I-237` hunks): duplicate cache (`behaviour.rs:255`, TTL from config), `published_message_ids` (`:323`, default 10 s), mcache (`:288`, history 5 x 1 s), send_queue (4 MiB cap). Peer scoring is OFF (no `with_peer_score`). Depended on by: `crates/network`.
- `crates/network` — gossip config (`gossip/config.rs`), swarm relay table (`gossip/relay_decision.rs`), per-peer buckets (`rate_limit.rs`), intake (`service/gossip_intake.rs`), EventSink lanes, verdict seam (`service/verdict.rs`). Depends on: vendor, core. Depended on by: node.
- `bins/node` — event loop dispatch (`event_loop.rs:309` txs, `:402` attestations), attestation verdict (`gossip_verdicts.rs`), `AttestSeen` + BLS ingest (`attestation/ingress.rs`), tx admission (`validation_checks/mod.rs:979-1030`), tx verdict (`validation_checks/gossip_txs.rs`), relay-state prune (`apply_block/post_commit.rs:450`). Depends on: network, mempool, core, storage.
- `crates/mempool` — `add_system_transaction` (`pool.rs:872`): residency, size, `validate_transaction` (structure only for the zero-flow types). No access to the maintainer set. Depended on by: node, rpc.
- `crates/core` — `ParentSignaturePool` (`attestation/pool.rs`), tx types, block/tx validation. `crates/core` does NOT depend on mempool (`crates/core/Cargo.toml` has no mempool edge).

### Data Flows Through Affected Area
- Relay verdict path (non-block topics): receive -> vendor dup-cache insert (`behaviour.rs:1718`) -> DOLI `relay_decision` (Defer / LocalOnly / Ignore) -> per-peer bucket (`gossip_intake.rs`; drop => Ignore at the drop site) -> EventSink lane -> node handler -> `Verdict` -> `send_gossip_verdict` (command channel, `try_send`, drop counted) -> swarm `report_message_validation_result` on the ORIGINAL id within the 3.5 s deadline -> vendor forward on Accept. Blocks are the single pre-verdict Accept exception (`AcceptBlock`).
- Attestation: Ed25519 + membership -> `AttestSeen` first-seen -> known block? -> `ingest_attestation` (attendance, BLS verdict, `ParentSignaturePool::insert` -> `newly_pooled`) -> finality weight -> verdict. Unknown block -> member-only fetch (`attest_fetch_tracker`, 30 s retain, `network_events.rs:467-485`) -> `relay(false)`.
- Zero-flow tx: `handle_new_transaction` (mempool residency only, `validation_checks/mod.rs:982-988`) -> `is_zero_flow()` -> `add_system_transaction` (no signature check) -> admitted -> M5 rule `all_new &= admitted && !zero_flow` (`gossip_txs.rs:11-12`) -> Ignore (never forwarded). The signature / multisig check runs only at APPLY: DelegateBond `apply_block/tx_processing.rs:489-504` (gated by `delegation_auth_activation_height`: 0 on mainnet and testnet, `u64::MAX` on devnet, `network_params/defaults.rs:153,483,766`), RevokeDelegation `:556-580`, AddMaintainer `apply_block/governance.rs:105-127` (`verify_multisig_at`), RemoveMaintainer `:165-188` (`verify_multisig_excluding_at`). A failure there logs `warn!`, refuses the state change, and LEAVES the tx in the block (AUDIT-P2-001). Block validation is shape-only: `crates/core/src/validation/tx_types.rs:765` (maintainer), `:860` (delegate), `:965` (revoke).

### Architectural Constraints & Invariants
- INV-NET-002: at most ONE report per message, on the original id. M6/M8 change only WHICH verdict is sent, never the count.
- INV-NETWORK-003: a node-relayable message is forwarded once per hop, by the vendor forward after Accept.
- INV-NETWORK-004: a suppressing dedup keys on full raw bytes or authenticated content. `(attester, slot)` is Ed25519-authenticated; keep any new M6 key authenticated.
- INV-NETWORK-002: an aggressive dedup setting needs `validate_messages` + bounded lanes. A count cap that evicts early under flood is "aggressive" in effect; both mitigations already hold.
- INV-ATTEST-001: attendance is recorded on Ed25519 regardless of BLS; M6 must not move attendance behind the relay rule.
- INV-RPC-002: an RPC that admits a tx must hand it to broadcast. Its premise (the tx actually reaches a producer) is false at V for zero-flow txs on endpoints without a producer direct peer (review F2). M8 restores it.
- Relay rules are node-local policy. No relay change may alter `apply_block`, block validation, rewards decode, or the attestation commitment math.
- Genuine attestations with an INVALID BLS half exist on mainnet (pre-v3 restored wallets, INC-I-162 / INC-I-217). Ed25519 is valid, so attendance counts. M6 must still forward their first copy per `(attester, slot)`: the first-seen Accept must not depend on BLS validity.
- Module size budget: `crates/network/src/rate_limit.rs` is 608 lines, `bins/node/src/node/periodic.rs` 2,149, `crates/mempool/src/pool.rs` 1,957, `validation_checks/mod.rs` 1,364 (all already over 500). New logic goes into a new focused module, not into these files.

### Blast Radius
- M6 — `on_new_attestation`: one production caller, `event_loop.rs:402` (graph: 5 dependents, all dispatch). `AttestSeen`: `gossip_verdicts.rs:43`, `post_commit.rs:450`, 3 constructors `init.rs:1115,1368,1588`, lib test `inc_i_237_m5_attest_seen_tests.rs`. Graph 2-hop gives 473 "dependents", all noise through the `Node` struct. `ParentSignaturePool` is read by production (`production/assembly.rs:437-450`) and startup (`startup.rs:612`). M6 must NOT change its insert or eviction (that is INC-I-245).
- M7 — `GOSSIP_DUPLICATE_CACHE_TIME` consumers: `config.rs:207,226` (builders), `event_loop.rs:410-417` (producer digest republish waits TTL + margin), `staleness.rs:4` (doc), `gossip/tests.rs:475-485` (TTL pin). `new_gossipsub` production caller: `service/mod.rs:131`. Any cap changes behaviour for every topic in every node.
- M8 — `add_system_transaction` production callers: `validation_checks/mod.rs:1006`, `crates/rpc/src/methods/transaction.rs:224`, `crates/rpc/src/methods/governance.rs:286` (graph: 30 dependents, mostly mempool tests). `handle_gossip_txs`: one caller, `event_loop.rs:309`. Tests whose expectation changes: `bins/node/tests/it/inc_i_237_m2_node_verdict.rs::m5_audit_p1_505_forged_zero_flow_tx_never_authorizes_a_forward` (the genuine half flips to Accept), `inc_i_237_m2_node_relay.rs:157`.
- Indirect: the block builder selects from the mempool. A new producer stops including forged zero-flow txs in its own blocks. That is producer-discretionary content, not validated content (see the consensus checklist).

### Brittleness Check
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details: (2) invariant gap — relay authority depended on residency in an evictable cache (M6), and no admission layer enforced zero-flow authentication (M8); (5) contract absence — the vendor duplicate cache has a TTL but no count or byte contract, while votes (no node newness rule, INC-I-239), block re-forward and the digest republish timer rely on its retention implicitly (M7). Not present: (1) cross-module spread (each milestone stays in one module plus its callers), (3) data-flow reversal, (4) shared state without an owner (`AttestSeen` and the pool are owned by the node loop, `&mut self`).
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━

## Requirements

New IDs continue the RUN 572 numbering (last: REQ-GRV-022, REQ-GRV-SEC-005).

| ID | Requirement | Priority | Milestone |
|---|---|---|---|
| REQ-GRV-SEC-002 | (existing, RE-OPENED) Forged variants cannot suppress genuine messages | Must | M6 |
| REQ-GRV-SEC-006 | Attestation relay authority does not depend on any cache a peer can evict | Must | M6 |
| REQ-GRV-023 | The genuine BLS half is forwarded even when a junk-BLS copy arrived first (audit F7) | Should | M6 |
| REQ-GRV-024 | `AttestSeen` is bounded by the wall clock, not only by block apply (review F1) | Should | M6 |
| REQ-GRV-025 | Per-peer attestation bucket sized for > 100 producer-set members (audit F4) | Won't (this round) | follow-up, measured before mainnet |
| REQ-GRV-007 | (existing) bounded memory: 10-minute flood retains < 64 MiB | Must | M7 |
| REQ-GRV-026 | Retained heap is attributed per candidate structure before any cap | Must | M7.a |
| REQ-GRV-027 | The node's duplicate-cache memory is bounded under a distinct-id flood without re-opening stale re-forwarding | Must | M7.b |
| REQ-GRV-028 | A CI-run test fails if the duplicate-cache bound is removed | Should | M7.b |
| REQ-GRV-SEC-007 | Zero-flow admission authenticates DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer on every admission path | Must | M8 |
| REQ-GRV-SEC-008 | (AUDIT-P1-505 property) A forged or unsigned zero-flow tx is never admitted and never forwarded | Must | M8 |
| REQ-GRV-029 | A genuine zero-flow tx of those 4 types relays multi-hop (INC-I-195 gap closed) | Must | M8 |
| REQ-GRV-030 | No consensus rule and no validated block content changes | Must | M8 (and M6) |
| REQ-GRV-031 | A zero-flow `Registration` stays never-forward | Should | M8 |
| REQ-GRV-SEC-009 | Replay of a captured genuine zero-flow tx: residual recorded or spun off (user decision Q2) | Should | M8 |
| REQ-GRV-032 | Specs, docs, skill and protection registry match the code at the new V | Should | M9 (+ folded rows in M6-M8) |
| REQ-GRV-SEC-010 | 0 disclosure hits in every branch commit message, diff and file version before the first push | Must | M10 |
| REQ-GRV-033 | (AUDIT-P1-509) Maintainer-change newness is keyed on signed content: at most one resident AddMaintainer / RemoveMaintainer per `(action, target)`; a second one is `AlreadyExists` (not admitted, not forwarded) | Must | M11 |
| REQ-GRV-034 | (AUDIT-P1-509) Admission refuses a maintainer change apply would turn into a no-op against the same current set: Add of a member or into a full set, Remove of a non-member or at the minimum size (`MPTX011`) | Must | M11 |
| REQ-GRV-035 | (AUDIT-P1-509) Admission refuses a non-canonical maintainer-change payload (trailing bytes) or a repeated signature entry (`MPTX012`); `reason` is never gated | Must | M11 |
| REQ-GRV-036 | (INC-I-246 stop-gap) Admission refuses a RevokeDelegation naming a delegate other than the effective one, a DelegateBond apply refuses (cap) or that repeats the effective delegation (`MPTX013`), and a DelegateBond / maintainer change whose txid a canonical block carries (`MPTX014`) | Must | M11 |
| REQ-GRV-SEC-011 | Maintainer-change residents are bounded: `MAX_MAINTAINERS` per action, `2 × MAX_MAINTAINERS` total (`MPTX015`); one captured change costs one resident and no forward | Must | M11 |

## Milestones

### M6 — attestation relay authority decoupled from BLS-pool residency (item 1)
- SSF: Record "BLS half already forwarded" per authenticated `(attester, slot)` in `AttestSeen` and authorise the BLS exception from that record, not from `ParentSignaturePool::insert`. This works because the record lives as long as the relay window and no peer can evict it.
- Same milestone (same struct, same rule): F7 (REQ-GRV-023) and the review F1 wall-clock prune (REQ-GRV-024). Separate: F4 bucket sizing (REQ-GRV-025, in `rate_limit.rs`, a different struct; it depends on the forwards-per-`(attester, slot)` constant that M6 fixes at <= 2).
- Open design point for the architect (NOT a fix plan): F7 is on the unknown-block path, where no BLS verdict runs today (`gossip_verdicts.rs:54-60`). The genuine copy's message id is already in the vendor duplicate cache when the block arrives later, so "check the bit once the block is known" (audit wording) does not by itself re-forward it. Closing F7 needs a relay-only BLS verdict on that path (no pooling: D4 needs the local height). Its cost is in the hardware line.
- Scope files: `bins/node/src/node/gossip_verdicts.rs`, `bins/node/src/node/attestation/ingress.rs` (`AttestSeen`, 190 lines); prune site: a new small module or an existing per-slot tick, NOT `periodic.rs` (2,149 lines).
- Acceptance criteria (tests from symptoms; all FAIL at V unless noted):
  - [ ] REQ-GRV-SEC-006 / SEC-002: a real node (`Node::new_for_test`) with >= 9 known blocks in the slot window receives height-mutated copies of captured genuine attestations, round-robin over the parents, for >= 3 full cycles. Accepts per `(attester, slot)` <= 2. At V this is unbounded (audit F1 trace, steps 1-7).
  - [ ] Keyed variant: one member key signs `(B_old_i, current slot)` for >= 9 distinct known old blocks. Accepts for that `(attester, slot)` <= 2.
  - [ ] The existing M5 guards stay green: `cargo nextest run -p doli-node --test it -E 'test(/m5_audit_p1_506|m5_dev_f1/)'` and lib `inc_i_237_m5_attest_seen_tests`.
  - [ ] REQ-GRV-023: unknown-block path, a junk-BLS copy (valid Ed25519, member) arrives first, then the genuine copy: the node forwards the genuine copy (Accept). At V: Ignore.
  - [ ] The first copy of a genuine attestation with an INVALID BLS half (mismatched-key producer) is still Accepted once per `(attester, slot)`, and attendance is recorded (INV-ATTEST-001).
  - [ ] Edge case pinned by a test: an honest same-slot attestation for a DIFFERENT block (fork switch, review F7). The chosen behaviour is written in the test name and in the spec (user question Q1).
  - [ ] REQ-GRV-024: with the tip frozen and the wall clock advancing 100 slots, `AttestSeen` holds <= members x (`ATTEST_STALE_SLOTS` + `ATTEST_FUTURE_SLOTS` + 1) entries (= members x 15).
  - [ ] REQ-GRV-030: `apply_block`, pooling, attendance and commitment code paths are byte-identical (diff touches none of `pool.rs`, `production/assembly.rs`, `attestation/commit.rs`).
  - [ ] Gate: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check && cargo nextest run --workspace`.
- Folded drift rows: `specs/gossip-relay-verdict-architecture.md:92` ("<= 2 Accepts" becomes true), `specs/gossip-relay-verdict-status.md:28` ("relay half CLOSED" becomes true), status `:29` threshold "> 200 attesters" -> "> 100" (audit F4 arithmetic: 2 forwards x N per 10 s slot vs 20/s).
- Hardware: CPU 0 on the known-block path (one extra map lookup per verified message); if F7 adds a relay-only BLS verdict on the unknown-block path: + 1 BLS pairing per non-first member copy, bounded by the per-peer attestation bucket (20/s, burst 200 per peer). Memory: `AttestSeen` grows from a set to a map (about +8 B per entry, inferred) and becomes bounded at members x 15 entries by the wall-clock prune. IO 0. Locks: none new (`&mut self` on the node loop).
- System impact: yes (relay dynamics). Needs a `Failure-Modes:` commit block per `.claude/protocols/system-impact.md`.

### M7 — REQ-GRV-007 retained heap: measure, then cap (item 2)

#### M7.a — attribution (measurement half; test-writer + developer)
- SSF: Print, at the end of the soak, the entry count and the attributed bytes of every structure that lives for the soak's lifetime, per swarm, and compare the sum with the retained heap.
- Hypothesis H1 (INFERRED, not measured; logged as incident entry): the soak's counting allocator is process-wide (`inc_i_237_m5_soak.rs:53-97`). The 3 attacker swarms, H, C and B run in ONE process, and the attackers use the PRODUCTION builder `new_gossipsub` (`:120-131`). `publish()` inserts every published id into the publisher's own 300 s duplicate cache (`vendor/libp2p-gossipsub/src/behaviour.rs:702`). Rates: about 600 ids/s published per attacker (249 + 249 + 49.8 + 49.8, QA table) and about 1,800 ids/s into B. Retained = end - warm (`WARM_SECS` = 10, `DRAIN` = 7 s), so about 290 s of growth: 301.5 MiB / (290 s x 3,600 ids/s) = about 300 B per id if all six duplicate caches count, about 600 B per id if only B's counts. The 30 s run (about 0.97 MiB/s, QA) gives about 280 B per id at 3,600 ids/s. This fits H1: roughly half of the 301.5 MiB may be harness-owned. Even so, B's own share (about 150 MiB at soak rates) is still above 64 MiB.
- Candidate retainers (soak lifetime) and growth drivers:

| # | Structure | Owner | Growth driver | Bound today |
|---|---|---|---|---|
| C1 | vendor duplicate cache | B | distinct ids received (inserted before any DOLI budget, TB4) + ids B publishes | TTL 300 s only, no count cap |
| C2 | vendor duplicate cache | A1-A3 (harness) | ids each attacker publishes (`behaviour.rs:702`) + ids received from B | TTL 300 s |
| C3 | vendor duplicate cache | C, H (harness) | ids forwarded by B (votes 10,284 at C in 600 s) + honest ids | TTL 300 s |
| C4 | `published_message_ids` | every swarm | published ids | 10 s |
| C5 | mcache (full messages) | every swarm | history 5 x 1 s; 1.6 MB tx batches at the attackers = the 1,986.8 MiB peak driver (inferred) | 5 s; drained before `end` |
| C6 | send_queue + connection / yamux buffers | every connection, both ends | burst traffic; capacity is kept after drain | 4 MiB per connection (patch) |
| C7 | `AttestSeen` | B | distinct `(member, slot)`; never pruned in the soak (no block applied, QA fidelity limit 1) | none in the soak |
| C8 | `ParentSignaturePool` | B | known parents | 8 parents |
| C9 | `attest_fetch_tracker` | B | distinct unknown block hashes from members | 30 s retain (`network_events.rs:478-479`) |
| C10 | per-peer rate-limit map | B | peers | `max_peers` 8 |
| C11 | EventSink lanes | B | backlog | measured: 22 KB / 5 KB / 0 B peaks |
| C12 | mempool | B | admitted txs (1 in the soak) | policy |
| C13 | metrics label vectors | B | per-attester labels (members only), per-topic | members |
| C14 | updater vote store | B | not wired: test nodes have `vote_tx: None` (`init.rs:1098,1351,1571`) | n/a in the soak |
| C15 | harness state | test | `at_c.honest` (<= 600), counters, prebuilt payloads (allocated before `pre`) | small |

- Measurement method (both parts are required):
  1. Entry counts. Add a `// DOLI INC-I-237:` accessor in the vendor crate for the duplicate cache, `published_message_ids` and mcache lengths (and capacities). Expose B's value as a production gauge (for example `doli_gossip_duplicate_cache_entries`), so the new `pub fn` has a production reader (wiring census, rule 31) and mainnet gains the signal that RUN 570 lacked. The soak reads the harness swarms directly. Make `AttestSeen::len` and the other B-side lengths (C7-C12) readable from the soak binary (today `AttestSeen::len` is `#[cfg(test)]`, `ingress.rs:44-47`, so an integration binary cannot call it).
  2. Bytes per entry. A calibration test under a counting allocator inserts N distinct 20-byte `MessageId`s (N = 100k and 500k) into the vendor duplicate cache and reports bytes per entry, hashbrown and `VecDeque` growth steps included.
  3. Report. At the soak end, print one row per (swarm, structure): entries, bytes per entry, attributed MiB, and `unattributed = retained - sum`. Cross-check with one differential run where only the attackers use `new_gossipsub_with_cache_time` (`config.rs:213`) with a short TTL: the drop in retained heap must match the C2 rows within 15%.
  4. If unattributed > 10% of retained, run one heap profile (for example `dhat` as a dev-dependency in a separate soak binary) at `SOAK_SECS` >= 320 and name the top 5 retaining call sites.
- Acceptance criteria:
  - [ ] REQ-GRV-026: `SOAK_SECS=600 cargo nextest run -p doli-node --test inc_i_237_m5_soak --run-ignored only -E 'test(soak_10min)' --no-capture` prints the attribution table; the attributed rows cover >= 90% of the retained bytes, or the remainder is named by the profile.
  - [ ] The table separates node-under-test (B) bytes from harness bytes (A1-A3, H, C).
  - [ ] The evidence (table + calibration) is saved with the milestone and quoted in the design gate.
- Hardware: test-only instrumentation + 1 gauge read per metrics scrape. CPU 0, memory 0, IO 0, locks 0 in production.

#### Design gate (between M7.a and M7.b) — open question, not an answer
What may bound C1 without re-opening stale re-forwarding? A count or byte cap on the duplicate cache would evict ids YOUNGER than 300 s under a flood. The consequences, per topic, must be answered from the M7.a numbers:
- Blocks (the single pre-verdict Accept exception): an evicted block id that re-arrives is Accepted and re-forwarded again. Only the stale gate (`STALE_BLOCK_SLOT_THRESHOLD` = 6 slots, `gossip/validation.rs:35`) and the per-peer block bucket then bound it. This is the INC-I-114-E2 class (open P0 finding). The gate must state the minimum retention for block ids under flood (>= 6 slots + clock skew).
- Votes: no node newness rule exists (INC-I-239), so the duplicate cache is the ONLY dedup for votes. An early eviction re-Accepts and re-forwards a vote.
- Attestations: node newness (`AttestSeen`, M6) Ignores re-arrivals, but each re-arrival then costs an Ed25519 verify (and maybe a BLS pairing) instead of a free drop. Window: wall - 12 slots.
- Txs: mempool residency + the newly-admitted rule Ignore a re-arrival; a mined zero-flow tx is the exception after M8 (see Q2).
- Producer digest republish (`event_loop.rs:410-417`) assumes the TTL; an early eviction lets a republish through sooner.
- The F-13 row says "no TTL tuning as the fix" (`specs/gossip-relay-verdict-architecture.md:401`), and the const assert (`config.rs:74-78`) pins TTL >= 180 s.
- The 64 MiB bound's subject (user question Q3): the node under test only, with harness retention printed but not counted.
- Growth driver to weigh at the gate: the vendor inserts at receive (`behaviour.rs:1718`) before any DOLI per-peer budget, so C1 grows with raw ingress bandwidth x TTL, not with the admitted rate.

#### M7.b — bound (after the gate)
- SSF: Bound the node's duplicate-cache memory by the mechanism the gate picks from the M7.a numbers, with block-id retention under flood kept above the block stale window.
- Acceptance criteria:
  - [ ] REQ-GRV-007: `soak_10min` (command above) passes: node-under-test retained < 64 MiB; every other soak bound still holds (delivery >= 0.99 with vote load, 0 forged zero-flow at C, <= 2 crossings per family, verdict p100 < 2 s, 0 late, lanes inside budgets).
  - [ ] REQ-GRV-027: under honest load (no flood) every id stays deduplicated for the full 300 s (no behaviour change); under the soak flood a re-sent copy of a block younger than 6 slots is NOT forwarded again by B.
  - [ ] REQ-GRV-028: a test that runs in CI (`cargo nextest run --workspace`) fails if the bound is removed (for example an entry-count assertion against the bound at the 30 s soak, or a calibrated static check).
  - [ ] `cargo nextest run -p network --test it -E 'test(inc_i_237)'` (vendor patch judge) stays green; `vendor/libp2p-gossipsub/DOLI-PATCH.md` lists any new hunk.
- Folded drift rows: `crates/network/src/gossip/config.rs:67-68` ("30 MB worst case" is not a bound), `specs/gossip-relay-verdict-architecture.md:401` (F-13 "PASS"), `docs/architecture.md:328` and spec `:401` ("inbound sync requests are limited to 10/s per peer": 10/s is the OUTBOUND governor, `rate_limit.rs:163`; inbound serving is a global 24 per production interval, QA drift row).
- Hardware: Memory: minus B's C1 share (UNKNOWN until M7.a; inferred about 150 MiB at soak rates; at higher ingress the unbounded share grows with bandwidth x 300 s). CPU: O(1) amortised eviction. Network: depends on the gate (possible early re-forward under flood, bounded by the per-topic guards above). IO 0. Locks 0.
- System impact: yes (dedup dynamics, all topics, all nodes). Needs the failure-mode matrix and a gauntlet run.

### M8 — zero-fee admission authenticates; genuine zero-flow txs relay again (item 3)
- SSF: Before `add_system_transaction`, verify the same signature predicates (and activation gates) that apply time uses for DelegateBond, RevokeDelegation, AddMaintainer and RemoveMaintainer, and then let a newly admitted genuine tx of those types authorise a forward. This works because admission then rejects exactly what apply would refuse, so the never-forward stop-gap is no longer needed for those 4 types.
- Constraint: the check lives in the ADMISSION layer only. It must NOT go into `crates/core/src/validation/tx_types.rs` (shared with block validation). Putting it there changes a consensus rule and needs an activation height.
- Constraint: one predicate serves all 3 admission call sites (`validation_checks/mod.rs:1006`, `rpc/methods/transaction.rs:224`, `rpc/methods/governance.rs:286`). The maintainer check needs the on-chain maintainer set, which the mempool crate does not hold today.
- Constraint: mirror `delegation_auth_activation_height` (0 mainnet and testnet, `u64::MAX` devnet), so admission is never stricter than apply.
- Acceptance criteria:
  - [ ] REQ-GRV-SEC-007: for each of the 4 types, a tx with an absent, malformed or wrong signature (DelegateBond/RevokeDelegation at heights >= the auth AH), or a maintainer tx below the multisig threshold of the CURRENT set, is rejected by gossip admission, by RPC `sendTransaction`, and by RPC `submitMaintainerChange`.
  - [ ] REQ-GRV-SEC-008: `m5_audit_p1_505_forged_zero_flow_tx_never_authorizes_a_forward` and `m5_audit_p1_505_forged_zero_flow_batch_stops_at_the_first_node` stay green for the forged half (`cargo nextest run -p doli-node --test it -E 'test(m5_audit_p1_505)'`); forged txs are also not admitted (mempool count unchanged).
  - [ ] REQ-GRV-029: a genuine signed tx of each type, submitted at a non-producer node, reaches a producer that is >= 2 hops away and is mined (testnet or a 4-node line test). At V it stops at hop 1.
  - [ ] A batch that mixes one forged and one genuine zero-flow tx is Ignored (D5 AND-fold unchanged).
  - [ ] REQ-GRV-031: a zero-flow `Registration` is still never forwarded.
  - [ ] REQ-GRV-030: a block built by an OLD producer that contains a forged zero-flow tx validates and applies on the new binary with the same state root as on the old binary.
  - [ ] Devnet (`delegation_auth_activation_height = u64::MAX`): an unsigned legacy DelegateBond is still admitted (admission not stricter than apply).
  - [ ] Gate as in M6.
- Folded drift rows: `specs/gossip-relay-verdict-status.md:32` (AUDIT-P1-505 reach row), `crates/rpc/src/methods/governance.rs:298-305` comment (cites the old `validation_checks.rs:1253-1259`; the "relaying is safe ... no new admission rule" premise), INV-RPC-002 premise (review F2).
- Hardware: CPU + 1 Ed25519 verify per DelegateBond/RevokeDelegation admission and <= 5 per maintainer tx (`MAX_MAINTAINERS` = 5, `crates/core/src/maintainer/mod.rs:98`), bounded by the per-peer tx bucket (500/s, burst 2,000) and the global bucket (1,000/s). Lock: + one maintainer-state read per maintainer tx admission. Network: + forwards of genuine zero-flow txs (user-initiated, rare) and of replays (Q2). Memory 0. IO 0.

### M9 — docs, specs, registry (also-should, rows not folded above)
- SSF: Update every stale text so that it states the rules in the code at the new V.
- Acceptance criteria (each item = one row of the drift list below, marked fixed):
  - [ ] Drift rows 3, 4, 8, 9, 13, 14, 15, 16 fixed; protection registry: new `protection_mechanisms` rows for the 5 M5 mechanisms + M6/M7/M8 bounds, PM-237-02 updated (per-tx charge, 500/s, burst 2,000).
  - [ ] REQ-GRV-025 recorded as a follow-up with its measurement (current producer-set size vs the 100-member threshold) before any mainnet deploy.
- Hardware: NEGLIGIBLE (docs).

### M10 — push hygiene (item 4; the final history operation)
- SSF: Reword every branch commit message that holds a local path, encode the planted guard fixtures like the other tokens, and run one disclosure scan over every branch commit before the release pass.
- Acceptance criteria:
  - [ ] The probe lines in `07a8b122`, `0ac1123e`, `a5d7d4e1`, `0e69eb5d`, `19ce3987`, `ba55599a` (and any M6-M9 message) name an in-repo script or a neutral command; no absolute path, no account name.
  - [ ] `crates/network/tests/it/inc_i_237_m5_disclosure.rs:100-102` stores its 3 planted fixtures (an external-producer count phrase, a host alias, a host log file name) in the same encoded form as the rest; the guard still detects them.
  - [ ] Scan = 0 hits for: workstation paths and the account name, host aliases, non-loopback IPv4, operator names, host log file names, unit or producer counts, deploy dates. Scope: every message in `d0e15686..HEAD`, every added line in `git log -p d0e15686..HEAD`, and every file version the branch adds or changes. (The base tree already holds some host tokens, for example `docs/infrastructure.md:303`; that is pre-existing on origin/main and out of this scope.)
  - [ ] The trees after the reword are identical to the trees before it (`git diff <old-head> <new-head>` empty).
  - [ ] `backup/inc-i-237-pre-rewrite-20261012` is untouched and never pushed.
- Hardware: NEGLIGIBLE (history only).

### M11 — keyless maintainer-change variants and the INC-I-246 stop-gap (AUDIT-P1-509, admission only)
- SSF: newness on the signed content `(action, target)` + the apply precondition against the same snapshot + canonical bytes. Admission layer only; `crates/core`, `apply_block` and `crates/storage` are unchanged; no activation height.
- Acceptance criteria:
  - [x] REQ-GRV-033: after a genuine change T is resident, a different-reason / reordered-entries copy is `AlreadyExists` on gossip (`Ignore`) and on `sendTransaction` / `submitMaintainerChange` (`TX_ALREADY_EXISTS`); the probe line reads `variant_forwards=0 middle_residents=1`.
  - [x] REQ-GRV-034: Add of a member, Add into a full set, Remove of a non-member and Remove at `MIN_MAINTAINERS` are refused with `MPTX011` (not `MPTX010`); after T applies, a variant and a byte-identical copy are refused.
  - [x] REQ-GRV-035: one trailing byte or a repeated signature entry is refused (`MPTX012`); a genuine change with a `reason` is admitted and broadcast once.
  - [x] REQ-GRV-036: a revoke naming P while D is effectively delegated to Q, a repeat of the effective delegation and an over-cap DelegateBond are refused (`MPTX013`); a DelegateBond whose txid a canonical block carries is refused (`MPTX014`); first-time delegation, a matching revoke, same-epoch queued sequences and a txid seen only on a side block are admitted. RevokeDelegation is exempt from the mined check (a second honest revoke of the same delegate is byte-identical).
  - [x] REQ-GRV-SEC-011: 10,000 variants of T leave 1 resident and 0 Accepts; 20 Adds + 4 Removes for distinct targets peak at <= 10 residents and admit the first of each.
- Residual (owed to the INC-I-246 consensus fix, activation height required): nonce / valid-before / chain binding in the signed preimage, `RevokeDelegation.delegate == delegated_to` at apply, and the replay of a mined revoke while D is again delegated to the same P.

## Post-milestone validation (pipeline steps, not milestones)
1. Release validation pass at the post-M10 V: QA (incl. `soak_10min` 600 s and the 30 s soak), reviewer, security fix-verification of AUDIT-P1-506, SEC-FIXVERIFY-R572-01/-04, AUDIT-P2-001, REL-572-R2-01/-02.
2. Local testnet deploy (`scripts/testnet.sh`, seed + n1-n6, copy + `codesign`, never pkill), mixed-version check (review F5: no mixed-version run exists with the M3+ rules), then `scripts/gauntlet.sh` (run 572 baseline: 7/7).
3. Ask the user about push / PR (`main` is protected: PR only). Nothing goes to mainnet.

## Consensus-shape checklist (INV-12) and deploy classification

| Question | M6 | M8 |
|---|---|---|
| (1) Can a user-submittable tx reach this path? | NO. Only `/doli/attestations/1` gossip reaches `on_new_attestation` / `AttestSeen` (`event_loop.rs:402`). | YES. Zero-flow txs are submittable by RPC and gossip (TB2, TB3). |
| (2) Can a producer action or attestation pattern reach it? | YES. Any peer, keyless, can send attestation copies; members can sign. | NO. |
| (3) Bit-identical for ALL reachable inputs on every consensus-visible computation? | YES. The change is the relay verdict and relay state only. `ingest_attestation` pooling, attendance, finality weight, `ParentSignaturePool`, `apply_block`, block validation and rewards decode are untouched. Which attestations reach a peer already varies with gossip loss; the attestation commitment is producer-local and is not validated against a peer's pool (audit "consensus shape" check at V). | YES, IF the check stays out of `crates/core/src/validation/` (constraint above). Block validation for these types is shape-only (`tx_types.rs:765,860,965`) and apply already refuses bad signatures with no state change (`tx_processing.rs:489-504,556-580`; `governance.rs:105-127,165-188`). An old producer's block with a forged zero-flow tx stays valid and applies identically. |
| Activation height? | Not required ((2) YES but (3) YES). | Not required ((1) YES but (3) YES). |
| Consensus RULES change? | No | No |
| Block CONTENT change (INV-8: bitfield, coinbase, tx ordering, presence_root, header)? | No | No. A new producer stops selecting forged zero-flow txs; tx selection is producer-discretionary and no validator recomputes it. |
| Deploy | Rolling | Rolling |

## Specs Drift Detected (16 rows)

| # | Location | Says | Code at V |
|---|---|---|---|
| 1 | `specs/gossip-relay-verdict-architecture.md:92` | "<= 2 Accepts per `(attester, slot)` on a known block" | unbounded after pool cycling (M6) |
| 2 | `specs/gossip-relay-verdict-status.md:28` | "Relay half CLOSED for attestations (M5, AUDIT-P1-506)" | not closed (M6) |
| 3 | `specs/gossip-relay-verdict-architecture.md:23` | header stops at "M4: this branch, commit after `0e69eb5d`" | M4 = `19ce3987`, M5 = `ba55599a` |
| 4 | `specs/gossip-relay-verdict-status.md:3` | "Updated 2026-10-12 (M4)" | M5 shipped |
| 5 | `specs/gossip-relay-verdict-architecture.md:401` | F-13 "PASS" | `soak_10min` fails (M7) |
| 6 | `crates/network/src/gossip/config.rs:67-68` | "1 000 msg/s x 300 s ~= 30 MB worst case" | nothing caps distinct-id ingress (M7) |
| 7 | `docs/architecture.md:328` + architecture spec `:401` | inbound sync requests 10/s per peer | 10/s is the outbound governor (`rate_limit.rs:163`); inbound is global 24 per interval (M7) |
| 8 | `docs/redesigns/gossip-relay-verdict-redesign-analysis.md` matrix | M5 rows | 17 M5 tests missing (QA/review count, not recounted here) |
| 9 | `specs/protocol.md:2085-2099`, `specs/security_model.md:637-640` | M4 relay rules | M5 rules (review F8) |
| 10 | `specs/gossip-relay-verdict-status.md:32` | zero-flow tx "always gets Ignore" | true at V; stale after M8 |
| 11 | `crates/rpc/src/methods/governance.rs:298-305` | cites `validation_checks.rs:1253-1259`; "relaying is safe ... no new admission rule" | file is now `validation_checks/mod.rs:1000-1006`; M5 never forwards (M8) |
| 12 | memory.db INV-RPC-002 | admitted RPC tx reaches producers | false at V for zero-flow txs (review F2; M8) |
| 13 | memory.db protection registry | PM-237-02 "50/s per peer, one token per message"; 5 M5 mechanisms missing | per-tx charge 500/s, burst 2,000 (review F3; M9) |
| 14 | `specs/gossip-relay-verdict-status.md:29` | "more than 200 attesters per 10 s slot" | 2 forwards x N > 200 per slot at N > 100 (audit F4; M6/M9) |
| 15 | `.claude/skills/network/SKILL.md:27,70,93,112` | removed `enqueue_or_shed`, 2-field `NewBlock`, `broadcast_transaction` | removed in M1-M4 (review F13; M9) |
| 16 | M5 commit message / incident 3029 | disclosure guard "stores its tokens hex-encoded so it carries none" | 3 literal fixtures at `inc_i_237_m5_disclosure.rs:100-102` (M10) |

## Impact Analysis

### Existing Code Affected
- `gossip_verdicts.rs` + `attestation/ingress.rs` (M6): relay rule and its state. Risk: medium (one incomplete fix already; adversarial partitions must be complete).
- vendor gossipsub + `gossip/config.rs` (M7): every topic, every node. Risk: high (INC-I-114 / INC-I-237 OOM history).
- admission call sites in node + RPC (M8): Risk: medium (3 sites must stay in parity; INV-VALIDATION-001 class).

### What Breaks If This Changes
- M6: a key that is not authenticated, or a rule that makes the first Accept depend on BLS, would suppress genuine attestations of mismatched-key producers. Mitigation: the INVALID-BLS acceptance row.
- M7: a cap that evicts young block or vote ids re-opens re-forward loops. Mitigation: the design gate and the REQ-GRV-027 block re-send row.
- M8: a check placed in core validation forks old and new nodes. Mitigation: REQ-GRV-030 old-producer-block row. A check stricter than apply on devnet blocks valid delegations. Mitigation: the devnet row.

### Regression Risk Areas
- Attestation coverage and rewards (54 of 60 minutes rule, `node/rewards.rs:37-44`) if M6 forwards fewer genuine halves.
- Vote propagation if M7 shortens effective dedup (votes have no other dedup).
- RPC `submitMaintainerChange` success path (INC-I-175 rotation) after M8.

## Traceability Matrix
| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-GRV-SEC-002 | Must | (test-writer) | (architect) | AttestSeen::admit @ bins/node/src/node/attestation/ingress.rs; on_new_attestation @ bins/node/src/node/gossip_verdicts.rs |
| REQ-GRV-SEC-006 | Must | (test-writer) | (architect) | AttestSeen::admit + ingest_attestation (bls_valid) @ bins/node/src/node/attestation/ingress.rs; on_new_attestation @ bins/node/src/node/gossip_verdicts.rs |
| REQ-GRV-023 | Should | (test-writer) | (architect) | Node::relay_bls_valid + AttestSeen::awaits_bls @ bins/node/src/node/attestation/ingress.rs; on_new_attestation @ bins/node/src/node/gossip_verdicts.rs |
| REQ-GRV-024 | Should | (test-writer) | (architect) | AttestSeen::prune_wall @ bins/node/src/node/attestation/ingress.rs; on_new_attestation @ bins/node/src/node/gossip_verdicts.rs |
| REQ-GRV-025 | Won't | n/a | n/a | n/a |
| REQ-GRV-007 | Must | `inc_i_237_m5_soak::{soak, soak_10min}` + (test-writer) | (architect) | M7.2 count caps @ `vendor/libp2p-gossipsub/src/time_cache.rs` (`TimeCache::new_capped`, `make_room`), wired @ `crates/network/src/gossip/config.rs::gossipsub_config` |
| REQ-GRV-026 | Must | (test-writer) | (architect) | `Behaviour::retention_stats` @ vendor/libp2p-gossipsub/src/{behaviour,time_cache,mcache,peer_score}.rs; `Retention` @ crates/network/src/service/retention.rs (swarm_loop.rs flush_tick); `watch_retention` + `doli_gossip_duplicate_cache_entries` @ bins/node/src/metrics_gossip.rs (startup.rs); `Node::attest_seen_len` @ bins/node/src/node/attestation/ingress.rs; probe line @ scripts/inc-i-237-m0-latency-probe.sh |
| REQ-GRV-027 | Must | (test-writer) | (architect) | `GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES` + const asserts @ `crates/network/src/gossip/config.rs`; oldest-first eviction @ `vendor/libp2p-gossipsub/src/time_cache.rs::make_room` |
| REQ-GRV-028 | Should | (test-writer) | (architect) | caps @ `vendor/libp2p-gossipsub/src/{time_cache,config,behaviour,peer_score}.rs`; constants @ `crates/network/src/gossip/config.rs` |
| REQ-GRV-SEC-007 | Must | `inc_i_237_m8_admission::req_grv_sec_007_gossip_refuses_*` (4), `admission_matches_apply_*` (2), `devnet_unsigned_legacy_delegate_bond_is_still_admitted`; rpc `tests_inc_i237_m8_admission::req_grv_sec_007_*` (2), `inv_rpc_002_*` (2) | (architect) | zero_flow_auth::authorized @ crates/mempool/src/zero_flow_auth.rs; Mempool::add_system_transaction @ crates/mempool/src/pool.rs; snapshot_for callers (M11; was current_set_for) @ bins/node/src/node/validation_checks/mod.rs, crates/rpc/src/methods/{transaction,governance}.rs |
| REQ-GRV-SEC-008 | Must | `m5_audit_p1_505_*` (2, flipped in M8), `inc_i_237_m8_admission::d5_mixed_forged_and_genuine_zero_flow_message_is_ignored` | (architect) | zero_flow_auth::authorized @ crates/mempool/src/zero_flow_auth.rs; handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs |
| REQ-GRV-029 | Must | `inc_i_237_m8_admission::req_grv_029_genuine_zero_flow_of_each_type_authorizes_a_forward`, `devnet_unsigned_legacy_delegate_bond_authorizes_a_forward`, `inc_i_237_m8_relay::req_grv_029_genuine_zero_flow_txs_reach_a_node_two_hops_away` | (architect) | handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs; zero_flow_auth::is_authenticated @ crates/mempool/src/zero_flow_auth.rs |
| REQ-GRV-030 | Must | `inc_i_237_m8_block::req_grv_030_old_block_with_forged_*` (2) | (architect) | none (admission-only; crates/core/src/validation/ and bins/node/src/node/apply_block/ unchanged) |
| REQ-GRV-031 | Should | `inc_i_237_m8_admission::req_grv_031_zero_flow_registration_never_forwards` | (architect) | handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs (Registration is not in zero_flow_auth::is_authenticated) |
| REQ-GRV-SEC-009 | Should | (decision) | (architect) | not fixed: residual recorded as INC-I-246 (memory.db incident_entries) |
| REQ-GRV-032 | Should | network `it::inc_i_237_m9_spec_sync::m9_req_grv_032_*` (4) | n/a (docs) | specs/protocol.md §7.2, specs/security_model.md, specs/gossip-relay-verdict-status.md, redesign traceability matrix, .claude/skills/network/SKILL.md, SKILLS-INDEX.md; protection_mechanisms PM-237-* (memory.db) |
| REQ-GRV-SEC-010 | Must | scan command (M10) | n/a | (orchestrator) |
| REQ-GRV-033 | Must | `inc_i_237_m11_variants::m11_gossip_variants_of_a_resident_{add,remove}_maintainer_are_ignored` (2), `inc_i_237_m11_probe::m11_audit_p1_509_variant_flood_probe`, rpc `tests_inc_i237_m11_admission::m11_send_transaction_refuses_keyless_variants_of_a_resident_change`, `m11_submit_maintainer_change_second_call_for_the_same_change_errors` | M11 (audit remediation steps 1-3) | `ChangeIndex::admit` @ crates/mempool/src/zero_flow_newness.rs |
| REQ-GRV-034 | Must | `inc_i_237_m11_preconditions::m11_*` (5), `inc_i_237_m11_variants::m11_after_{add,remove}_maintainer_applies_its_variant_and_copy_are_refused` (2), rpc `m11_send_transaction_precondition_refusal_carries_its_own_code` | M11 (audit remediation steps 1-3) | `zero_flow_policy::pre_auth` @ crates/mempool/src/zero_flow_policy.rs |
| REQ-GRV-035 | Must | variants rows `one trailing byte` / `duplicated signature entry`, `inc_i_237_m11_variants::m11_genuine_change_with_a_reason_still_admits_and_relays`, rpc `m11_send_transaction_admits_a_genuine_change_with_a_reason` | M11 (audit remediation steps 1-3) | `zero_flow_policy::pre_auth` @ crates/mempool/src/zero_flow_policy.rs |
| REQ-GRV-036 | Must | `inc_i_237_m11_delegation::m11_*` (9) | M11 (audit remediation steps 1-3) | `zero_flow_policy::{delegation_view, post_auth, mined_in_chain}` @ crates/mempool/src/zero_flow_policy.rs; `zero_flow_auth::snapshot_for` @ crates/mempool/src/zero_flow_auth.rs |
| REQ-GRV-SEC-011 | Must | `inc_i_237_m11_bound::m11_*` (2), probe | M11 (audit remediation steps 1-3) | `ChangeIndex::admit` cap @ crates/mempool/src/zero_flow_newness.rs |

## Assumptions
| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| 1 | Trust model = REQ-GRV-SEC-001: all gossip and RPC input untrusted; member keys may be malicious | Any peer may lie, even a registered producer | Yes (RUN 572) |
| 2 | Relay verdicts are node-local and need no activation height | Each node decides alone what to pass on; the chain rules do not change | Yes (audit consensus-shape check at V) |
| 3 | H1: about half of the 301.5 MiB is the harness's own duplicate caches | The test may count the attackers' memory, not only the node's | No (M7.a measures it) |
| 4 | The 64 MiB bound applies to the node under test, not the harness | We judge the node, not the test's fake attackers | No (Q3) |
| 5 | F4 (bucket sizing above 100 members) can wait until before mainnet | The per-neighbour attestation allowance may be too small for a large network; we measure first | No (needs the live producer-set size) |

## What I Don't Understand
1. The bytes per duplicate-cache entry and the B-versus-harness split of the 301.5 MiB. Only M7.a can answer this.
2. How often an honest attester sends a second attestation for the same slot with a different block (review F7, inferred). This decides how much Q1 costs.
3. Whether any layer rejects a replay of an already-mined genuine zero-flow tx. I found only a mempool-residency check (`validation_checks/mod.rs:982-988`). The DelegateBond signing message has no nonce (`crates/core/src/transaction/tests_delegation_auth.rs:9`). I did not trace what apply does with a replayed RevokeDelegation.
4. The current mainnet producer-set size (members incl. pending) against the 100-member F4 threshold. Live state is UNKNOWN; I did not probe it.
5. Whether yamux / connection buffers (C6) keep a material share after the drain.

## Open questions for the user (via the orchestrator)
- Q1 (M6): With the bit keyed on `(attester, slot)`, a second honest attestation for a different block in the same slot (fork switch) gets no second BLS forward at a hop that already forwarded one. Recommendation: accept this. It is rare, other mesh paths repair it, and attendance does not depend on it.
- Q2 (M8): Once genuine zero-flow txs relay again, a captured genuine one can be replayed with no key and is newly admitted each time it is not in the mempool. Recommendation: open a separate incident for chain-level replay (it may need an activation height), and record the relay residual in M8 as bounded by the finite set of past signed txs and the per-peer tx bucket.
- Q3 (M7): Does the 64 MiB bound apply to the node under test only (harness memory printed but not counted)? Recommendation: yes.

## Identified Risks
- M6 repeats M5's failure mode (a test partition left out). Mitigation: the acceptance rows list every partition the audit traced (cycling >= 9 parents, keyed old-block variant, unknown-block junk-first, invalid-BLS genuine, fork switch).
- M7.b picks a cap before the numbers exist. Mitigation: the gate is blocking; M7.b starts only from the M7.a table.
- M8 opens network-wide relay of replayed genuine zero-flow txs (Q2). Mitigation: decision Q2; per-peer tx bucket.
- The M10 reword runs after the release reports are written. Mitigation: run M10 BEFORE the release pass, so V is the push SHA.

## Out of Scope (Won't)
- REQ-GRV-025 global attestation bucket / BLS pairing cap (F2-bucket follow-up).
- AUDIT-P1-508 / INC-I-245 pool recency (the pool eviction fix itself).
- AUDIT-P1-507 / INC-I-239 vote newness.
- SEC-FIXVERIFY-R572-02 (0xAA fetch path), R572-03 / review F6 (origin token refund), review F4 (RPC permit across lock waits), F11, F12, F14 (file size), F15.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 Ed25519 per DelegateBond/RevokeDelegation admission, <= 5 per maintainer tx (observed: apply predicates, MAX_MAINTAINERS = 5); + up to 1 BLS pairing per non-first member attestation copy on the unknown-block path if F7 is closed that way, <= 20/s per peer (inferred: per-peer attestation bucket)
  Memory:   -large on B's duplicate cache (inferred: about 150 MiB at soak rates, UNKNOWN until M7.a); AttestSeen about +8 B per entry but bounded at members x 15 (inferred)
  IO:       0 (no disk or syscall path changes; observed)
  Network:  + forwards of genuine and replayed zero-flow txs (inferred: user-initiated + finite replay corpus, bucket-bounded); - attacker-driven attestation forwards (observed: audit F1 flow closed)
  Disk:     0 (no persisted state; observed)
  Latency:  about +50 us per zero-flow admission (inferred: one Ed25519 verify); attestation verdict unchanged on the common path (observed)
Inevitability: AVOIDABLE
Cheaper alternative: record AUDIT-P1-506 and REQ-GRV-007 as accepted residuals and ship M5 as is (zero code)
Why this proposal anyway: AUDIT-P1-506 is a keyless P1 that suppresses genuine attestations, and REQ-GRV-007 is a Must with an unbounded memory sink that grows with ingress bandwidth; the release gate cannot pass without both.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Follow-up 3 (RUN 574): M12.1 and M12.2

> Analyst, RUN 574, code at V = `8cc50d33`. Input: the M12 session prompt (`docs/bugfixes/inc-i-237-m12-session-prompt.md`) and the round-2 release reports at V (QA; review REL-573-R2-01..03; audit AUDIT-P1-515/516 and SEC-FIXVERIFY-R573-01..03). Every pointer below was re-read at V. The test-writer runs first in each milestone. This section names behaviours and seams. It is not a fix plan.

### Pointer check (M12 brief vs code at V)

| # | Brief premise | At V | Verdict |
|---|---|---|---|
| P1 | `mined_in_chain` = tx-index get, then `get_block_by_height`, then a hash scan (`zero_flow_policy.rs:59-70`) | `:60-70`. `get_tx_block_height` (`block_store/queries.rs:105-121`) → `get_block_by_height` = `get_hash_by_height` + `get_block` (`queries.rs:171-177`; full decode `:26-48`) → `t.hash()` per tx | holds |
| P2 | `snapshot_for` runs the mined check for DelegateBond / Add / RemoveMaintainer before auth, newness and no-effect; RevokeDelegation exempt (`zero_flow_auth.rs:40-70`) | `:66-67`, before the lock. Inside `add_system_transaction` the order is residency `pool.rs:889`, size `:894`, `validate_transaction` `:967`, `pre_auth` `:970`, `authorized` `:972`, `post_auth` `:978`, `ChangeIndex::admit` `:979-981` | holds |
| P3 | Node site = residency, `snapshot_for`, `add_system_transaction` per tx, inline on the event loop | `validation_checks/mod.rs:983-989`, `:1000-1006`, `:1007-1019`; `gossip_txs.rs:11-15`; `event_loop.rs:308-311`; 0xAA fetch `network_events.rs:569-576` | holds |
| P4 | The two RPC sites behave like the node site | They do NOT check residency first: `transaction.rs:212-231` and `governance.rs:282-293` call `snapshot_for` before the write lock, and residency is tested only inside `add_system_transaction` (`pool.rs:889`). On RPC, a resident duplicate also pays the block read | stronger than the brief |
| P5 | A batch carries up to 10,000 txs; per-peer bucket = `max_txs_per_second` x `TX_BURST_SECONDS` | decode cap `publish.rs:88`; bucket `rate_limit.rs:71,104` = 500 x 4 = 2,000 (`:158`). Node-visible maximum = 2,000 copies per message (PM-237-08 derived bound) | holds |
| P6 | The tx index is written for every stored block, side blocks included (`writes.rs:55`) | `put_block` `writes.rs:20`; tx index `:49-56` | holds |
| P7 | Reorg safety of a negative cache needs a hook or a storage change | Not needed. `get_hash_by_height` (`queries.rs:84-102`) is one `CF_HEIGHT_INDEX` point get with no decode. `set_canonical_chain` rewrites that index and purges above the tip (`writes.rs:107-130`). `remove_canonical_entry` clears it on rollback (`writes.rs:203-219`) | design input |
| P8 | `delegation_stop_gap` admits an unknown delegator (`zero_flow_policy.rs:140-170`); `authorized` checks only the signature | `:140-171`; `zero_flow_auth.rs:86-91` | holds |
| P9 | `delegate_bonds_capped` (`set_delegation.rs:46-95`) requires the delegator to exist, be active and hold `bond_count` available bonds | Incomplete. It also refuses self-delegation (`:58-60`), an existing delegation (`:82-84`), a missing or inactive delegate (`:86-93`) and the cap (`:102-115`). Shape validation refuses only `bond_count == 0` (`crates/core/src/validation/tx_types.rs:885`). Apply queues on signature + cap only (`apply_block/tx_processing.rs:484-553`). The boundary runs `delegate_bonds_capped` / `revoke_delegation` (`set_core.rs:138-153`, `set_delegation.rs:182-211`) | **does not survive as written**: the M12.1 SSF uses all the preconditions |
| P10 | SSF: delegator standing alone closes AUDIT-P1-515 | No. `delegation_view` (`zero_flow_policy.rs:25-57`) reads applied state + queued updates, never mempool residents. Without a delegate check, one registered key still mints unlimited DelegateBonds to random delegates. With it, one key still mints (active delegates x available bonds) distinct admissible txs per view. The round-2 audit remediation adds a per-delegator newness key (AUDIT-P1-515, Remediation bullet 2), and its test strategy expects at most 1 Accept for 50 DelegateBonds from one registered delegator | **does not survive** |
| P11 | AC7: refused "at all three call sites" | `submitMaintainerChange` builds only maintainer txs (`governance.rs:268-270`). Delegation txs reach two sites: node `handle_new_transaction` (0xBA and 0xAA) and RPC `sendTransaction` | narrowed to 2 sites |
| P12 | AC11: a delegator whose registration is still pending "would also fail at apply today" | No. The boundary applies queued updates in queue order (`set_core.rs:106-153`), so a Register queued before the DelegateBond applies first. Refusing a queued-registration delegator would be stricter than apply. `pending_updates_for` returns Register and AddBond (`set_core.rs:219-233`) | **does not survive**: M12.1 counts queued Register / AddBond |
| P13 | PM-237-12 scale assumption: "one block read only for indexed txids" | False per copy (REL-573-R2-01): a refused copy is never resident, so every copy reads again | registry drift (fixed by M12.2) |

### Architecture Context (M12)

**Module boundaries**
- `crates/mempool`: `zero_flow_auth` (snapshot, signature predicate), `zero_flow_policy` (pre/post-auth policy, delegation view, mined check), `zero_flow_newness` (`ChangeIndex`), `pool` (`add_system_transaction`). Depends on `storage` (read only: `ProducerSet`, `MaintainerState`, `BlockStore`) and `doli-core`. Depended on by `bins/node` (validation_checks, gossip_txs, network_events) and `crates/rpc` (transaction, governance).
- `bins/node`: `handle_gossip_txs` → `handle_new_transaction` → `snapshot_for` → `add_system_transaction`. The verdict is Accept iff the message is non-empty and every tx is newly admitted (D5). Relay is the swarm's job (`gossip_verdicts.rs:12-14`).
- `crates/storage`: ProducerSet and BlockStore are read only. Nothing in storage changes.

**Data flows**
- Gossip 0xBA: swarm → per-peer bucket (PM-237-02, 2,000) → `NetworkEvent::NewTransaction` → `handle_gossip_txs` (inline on the event loop) → `handle_new_transaction` per tx → verdict → vendor forward or drop.
- 0xAA fetch response: `network_events.rs:569-576` → `handle_new_transaction` per tx, no verdict.
- RPC: `sendTransaction` (`transaction.rs:212-231`) and `submitMaintainerChange` (`governance.rs:282-293`) → `snapshot_for` → `add_system_transaction` → broadcast permit.
- Apply (unchanged): DelegateBond / RevokeDelegation queue a `PendingProducerUpdate` (`tx_processing.rs:539,571`). The epoch boundary drains the queue in order (`set_core.rs:106-153`).

**Architectural constraints and invariants**
- INV-VALIDATION-001: one admission rule set for every call site. The new ordering and the standing check must reach the node site and both RPC sites through one shared code path, not per-site copies.
- INV-NETWORK-004 / C-PAT-1: a key that can suppress a relay must be full-content. For zero-flow txs (no inputs) `tx.hash()` covers version, type, outputs and all of `extra_data` (`crates/core/src/transaction/core.rs`, `hash()`), so the txid is full-content for the negative cache. For a tx with inputs the txid excludes the input signatures, so the per-message dedupe (which sees every tx type) must key on the full serialized bytes.
- D5 verdict: a message that repeats a tx was never "every tx newly admitted" at V, so it is Ignore today. The dedupe must keep it Ignore.
- The block-body read must not run while the mempool write lock is held (block production `select_for_block` and the RPC readers share that lock).
- Source-scan pins: `bins/node/tests/inc_i_173_m3_f4_routing.rs:163-216` require `is_zero_flow()` in the RPC `sendTransaction` source and in `validation_checks/mod.rs`. The routing branch stays at each site.
- Admission only: `crates/core/src/validation/`, `bins/node/src/node/apply_block/` and `crates/storage/` stay unchanged; no activation height (INV-12 table below).

**Blast radius.** Source: the code graph (graphify, refreshed this run) with `blast.py --hops 2` and `graphify explain`. The graph misses Rust `self.method()` and cross-crate free-function edges, so the call sites are grep-confirmed.

| Symbol | Graph | Grep-confirmed production call sites |
|---|---|---|
| `delegation_stop_gap` | 1 dependent: `post_auth` (`zero_flow_policy.rs:132`) | same |
| `snapshot_for` | 0 (cross-crate edges missed); `explain` callees: `delegation_view`, `mined_in_chain` | 3: `validation_checks/mod.rs:1000`, `rpc/methods/transaction.rs:212`, `rpc/methods/governance.rs:282` |
| `mined_in_chain` | 2: the `zero_flow_auth.rs` import and `snapshot_for` (`:67`) | 1 (`zero_flow_auth.rs:67`) |
| `add_system_transaction` | 30 (lower bound; tests in mempool, node `it`, `testing/`, `rpc/context.rs`) | 3: `validation_checks/mod.rs:1011`, `transaction.rs:231`, `governance.rs:293`; 11 files reference it |
| `handle_gossip_txs` | 4 (lower bound): `event_loop.rs:309` `dispatch_network_event` | 1 production caller; tests `inc_i_237_slash_tests.rs:146` and the M8/M11 `it` suites |
| `handle_new_transaction` | method edge `gossip_txs.rs:14` | 2: `gossip_txs.rs:14`, `network_events.rs:576` |

- Direct impact: `zero_flow_{auth,policy,newness}.rs`, `pool.rs`, `validation_checks/{mod,gossip_txs}.rs`, `rpc/methods/{transaction,governance}.rs` (call order only).
- Indirect impact: relay verdicts on the tx topic (fewer Accepts); block selection at M12 producers (they stop selecting no-op delegations); RPC error codes of `sendTransaction`.
- Test fixtures that assert the defect: `bins/node/tests/it/inc_i_237_m8_common.rs:112-137` (`delegate_bond` / `revoke` build a throwaway delegator and delegate), used by `inc_i_237_m8_relay.rs` (2), `inc_i_237_m8_admission.rs` (12), `inc_i_237_m8_block.rs` (2) and `inc_i_237_m2_node_relay.rs` (1).

**Brittleness check**
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details: (2) invariant gap: no module bounds resident delegation txs per signer (ChangeIndex covers maintainer changes only); (5) contract absence: the three admission sites repeat the snapshot -> add sequence by hand, and the RPC sites already drifted (no residency pre-check, P4)
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

### M12.1 — delegator standing at admission (AUDIT-P1-515)

- **Scope files:** `crates/mempool/src/zero_flow_policy.rs`, `crates/mempool/src/zero_flow_auth.rs`, `crates/mempool/src/zero_flow_newness.rs`, `crates/mempool/src/pool.rs`; tests in `bins/node/tests/it/inc_i_237_m8_*.rs`, `inc_i_237_m2_node_relay.rs` and a new `inc_i_237_m12_*` suite; the docs listed below.
- **Root cause (verified at V):** admission proves possession of the delegator key, not standing (`zero_flow_auth.rs:86-91`). The stop-gap admits when the delegator is unknown (`effective` None) and skips the cap when the delegate is unknown (`zero_flow_policy.rs:146-170`). `delegation_view` ignores residents, so every distinct tx is "new". At the boundary `delegate_bonds_capped` turns each one into a no-op (`set_delegation.rs:65-68`). Result: one keygen + one signature = one fee-free Accept at every hop.
- **SSF:** at admission, evaluate a DelegateBond / RevokeDelegation against exactly the preconditions the epoch boundary applies (`delegate_bonds_capped` / `revoke_delegation`), on the ProducerSet view `snapshot_for` already builds (applied state plus the delegator's queued updates), and keep at most one resident delegation tx per delegator (the M11 `ChangeIndex` pattern). This works because admission then refuses exactly what the boundary would turn into a no-op, and residents can no longer multiply one signer's view.
  - This is wider than the brief's sentence (P9, P10). It is still "nothing stricter than apply" against queued state. The CLI already enforces the same preconditions on applied state before it submits (`bins/cli/src/cmd_producer/delegation.rs:43-86`), so honest CLI users see no new refusal.

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-GRV-SEC-012 | DelegateBond boundary parity: delegator registered (applied, or a queued Register) and active; delegate registered and active; delegator != delegate; `bond_count` <= available (applied + queued Register / AddBond bonds - `withdrawal_pending_count`); no effective delegation (the existing fold). Refusal code `MPTX016` | Must | A DelegateBond that fails any one condition is refused with `MPTX016` on gossip (Ignore, not resident) and on `sendTransaction` (RPC error) |
| REQ-GRV-SEC-013 | RevokeDelegation requires an effective delegation; "no delegation at all" is refused with `MPTX016`; the "names another delegate" arm stays `MPTX013` | Must | A throwaway-key revoke and a revoke by a producer with no effective delegation are refused on both sites |
| REQ-GRV-SEC-014 | At most one resident DelegateBond / RevokeDelegation per delegator (`AlreadyExists`) | Must | 50 DelegateBonds from one registered delegator to 50 registered delegates give 1 resident and at most 1 Accept |
| REQ-GRV-SEC-015 | Fail closed: a delegation tx whose snapshot carries no producer-set view is refused (mirrors `zero_flow_auth.rs:120-121`) | Should | `add_system_transaction(DelegateBond, h, None)` and a snapshot built with `producers = None` both refuse |
| REQ-GRV-037 | Honest flows stay admissible | Must | Every row of the honest guards below is Accept + resident |
| REQ-GRV-038 | M8 fixtures use registered delegators; throwaway rows become refusal rows | Must | `req_grv_029_*` passes with a registered, active delegator and delegate at B and C, and a throwaway-key row is absent at C |
| REQ-GRV-039 | Operator docs for the honest edge flows refused until the INC-I-246 consensus fix (folds REL-573-R2-02, SEC-FIXVERIFY-R573-03 b/c/e and the corrected M11 FM3 text) | Should | Each refused flow below has a cause and a workaround in `docs/error-codes.md` and `docs/rpc_reference.md` |

**Acceptance criteria (detailed)**
- [ ] REQ-GRV-SEC-012: given the 2-hop line A → B → C and a `KeyPair::generate()` delegator, when A publishes a validly signed DelegateBond, then B returns Ignore, B and C hold 0 copies, and `sendTransaction` at B returns `MPTX016`.
- [ ] REQ-GRV-SEC-012: one row per condition (unregistered delegator, inactive delegator, unregistered delegate, inactive delegate, self, `bond_count` = available + 1, an effective delegation to another producer with no queued revoke) is refused with `MPTX016`.
- [ ] REQ-GRV-SEC-013: a RevokeDelegation from a throwaway key, and one from a registered producer with no applied or queued delegation, are refused with `MPTX016`. `m11_revoke_naming_a_delegate_other_than_the_current_one_is_refused` stays green (code stays `MPTX013`).
- [ ] REQ-GRV-SEC-014: 50 DelegateBonds (one delegator, 50 delegates) through `handle_gossip_txs`, one per message: exactly 1 Accept and 1 resident. After that resident is mined or evicted, the next admissible one is accepted again.
- [ ] REQ-GRV-SEC-015: both None-view paths refuse. Maintainer and non-zero-flow callers that pass `None` are unchanged (the fork_recovery and inc_i_180 / 204 / 234 suites stay green).
- [ ] REQ-GRV-037 (honest guards): first-time delegation by an active producer with free bonds to another active producer; a revoke naming the effective delegate; the three `m11_same_epoch_sequences_*` rows; register-then-delegate in one epoch (queued Register); add-bond-then-delegate within the new bonds (queued AddBond); `m11_tx_indexed_only_on_a_side_block_is_still_admitted`. All Accept + resident.
- [ ] REQ-GRV-038: the "genuine" rows of `inc_i_237_m8_common::delegate_bond` / `revoke` build a delegator and delegate that are registered and active on every node under test (reuse the `World::delegator` pattern, `inc_i_237_m11_delegation.rs:75-85`). The M8 admission, M8 block and M2 relay rows that used throwaway keys as "genuine" are rewritten or re-labelled as refusals.
- [ ] REQ-GRV-039: doc rows: (a) a byte-identical re-delegation after a revoke → `MPTX014`; workaround: a different `bond_count`. (b) A CLI revoke while a same-epoch switch is queued → `MPTX013`; workaround: wait for the epoch boundary. (c) A dependent Add / Remove at the set bounds → `MPTX011` until the predecessor is mined (corrects M11 FM3 "for at most that block"). (d) Revoke then delegate back to back → the DelegateBond gets `MPTX016` until the revoke is mined; workaround: wait one block. (e) A pending delegation tx holds the delegator's one resident slot until it is mined, evicted or expires.

**Test strategy (RED at V; the test-writer writes these first)**
- New `bins/node/tests/it/inc_i_237_m12_standing.rs`: one row per condition against `Node::handle_gossip_txs` and the RPC `sendTransaction` handler. RED at V: all are admitted.
- The REQ-GRV-SEC-014 burst row. RED at V: 50 Accepts.
- The rewritten `req_grv_029_*` plus a throwaway refusal row on the line. RED at V: the throwaway row reaches C.
- The REQ-GRV-037 guards: GREEN at V and after, except the queued-Register / AddBond rows, which pin parity.
- Seam: ProducerSet fixtures through `register_genesis_producer` / `queue_update` (already used at `inc_i_237_m11_delegation.rs:60-89`). No new production seam.

**Outcome metric (AC10)**
- metric: the middle node B's forwards of 100 DelegateBonds from 100 fresh keys published at A, plus B's residents.
- probe: a `--nocapture` `it` test on the A → B → C line. It counts NewTransaction arrivals at C that carry one of the 100 txids (C's only peer is B). It does NOT count C's mempool: per REL-573-R2-03, C's own M12 admission refuses them, so C's residency reads 0 whatever B does. It prints `M12.1-PROBE middle_forwards=<n> middle_residents=<m>`.
- before: 100 / 100 expected by derivation (every tx is admitted at B and Accepted). The milestone loop MUST run the probe at V and record the measured value.
- after: 0 / 0.

**Failure modes**
- FM1 mixed-version window: old nodes still Accept throwaway delegations. M12 nodes return Ignore, which carries no peer-score penalty, so there is no mesh partition. Old producers may still mine them (a no-op at the boundary, as at V).
- FM2 epoch boundary: the view is applied state plus queued updates, so admission matches the boundary outcome for the delegator's own updates. Updates by other producers (for example, the delegate exits in the same epoch) can still turn an admitted tx into a no-op. That error is in the looser direction only.
- FM3 lagging node (snapshot at its own `best_height`): it can refuse a delegation from a delegator registered in a block it has not applied yet. The verdict is Ignore with no penalty, and other peers relay the tx.
- FM4 stale resident: a resident delegation whose delegate exits stays resident and holds the delegator's one slot (`revalidate` skips zero-flow txs; SEC-FIXVERIFY-R573-02 shape). It is bounded to one per delegator, and only the delegator can fill that slot.
- FM5 fresh genesis / small network: genesis producers are registered and active (`register_genesis_producer` sets Active), so delegation between them is admitted. Unaffected.
- FM6 restart: no new persisted state; the view reads the reloaded ProducerSet. Unaffected.

### M12.2 — the mined check costs one lookup per copy (AUDIT-P1-516 = REL-573-R2-01)

- **Scope files:** `crates/mempool/src/zero_flow_auth.rs`, `crates/mempool/src/zero_flow_policy.rs`, `crates/mempool/src/pool.rs`, `bins/node/src/node/validation_checks/{mod,gossip_txs}.rs`, `crates/rpc/src/methods/{transaction,governance}.rs` (call order only), `bins/node/tests/it/inc_i_237_m11_probe.rs` and a new `inc_i_237_m12_*` suite; the docs listed below.
- **Root cause (verified at V):** M11 put a full canonical-block read in front of every other check (P2), on the event loop (P3). A refused copy never becomes resident, so the residency short-circuit never fires for the next copy, and nothing remembers the refusal. One message can repeat a txid up to 2,000 times (P5). The RPC sites pay the read even for a resident duplicate (P4).
- **SSF:** move the mined check behind every cheaper refusal (structural, signature, newness, no-effect, standing) so only a tx that would otherwise be admitted pays it; remember txids refused as mined in a small bounded negative cache, each entry pinned to the `(height, block_hash)` that carried it and re-checked on a hit with one `get_hash_by_height` point get; and dedupe byte-identical txs per gossip message. This works because the full read then runs once per distinct mined txid that passes everything else, a reorg shows up as a pinned-hash mismatch without any hook, and copies inside one message cost nothing.

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-GRV-SEC-016 | The mined check runs only after residency, size, `validate_transaction`, `pre_auth`, `authorized`, the delegation stop-gap / M12.1 standing and newness all pass, on every admission site, and never while the mempool write lock is held | Must | 0 block-body reads for each cheaper refusal on gossip and on both RPC sites |
| REQ-GRV-SEC-017 | Bounded negative cache of txids refused as mined: entry = txid → `(height, block_hash)`. A hit re-checks `get_hash_by_height(height) == block_hash` (no body read) and refuses with `MPTX014`. A mismatch or a missing height drops the entry and runs the full check. Count cap 10,000, oldest first. One instance per node, shared by every admission site | Must | Copies 2..n of one mined txid cost 0 body reads; a tx whose carrying block is reorged out is admitted on the next copy |
| REQ-GRV-SEC-018 | Per-message dedupe in `handle_gossip_txs`: one admission attempt per distinct tx, keyed on the full serialized tx bytes (INV-NETWORK-004); a message that repeats a tx stays Ignore | Must | A message of N copies of tx T gives 1 attempt, and the verdict is Ignore for any N >= 2, whether T is new, resident or refused |
| REQ-GRV-SEC-019 | SEC-FIXVERIFY-R573-01 fold: the mined check covers DelegateBond only. Drop the maintainer arm: any `reason` / order variant already bypasses it, and MPTX011 already refuses copies of an applied change | Should | AddMaintainer / RemoveMaintainer admission performs 0 block-body reads; `m11_after_add_maintainer_applies_*` and `m11_after_remove_maintainer_applies_*` stay green |
| REQ-GRV-040 | REL-573-R2-03: the M11 probe counts B's forwards (NewTransaction arrivals at C that carry a variant) and asserts `middle_residents`, not C's residency | Should | A mutation that makes B Accept every variant makes the probe print 20 and fail |
| REQ-GRV-041 | Registry and specs state the real cost and limits | Should | PM-237-12 names the per-peer and global tx buckets (PM-237-02), the 2,000 message bound (PM-237-08) and the negative-cache cap as the limits; the error code stays `MPTX014` |

**Acceptance criteria (detailed)**
- [ ] REQ-GRV-SEC-016: a DelegateBond or maintainer change with a bad signature, a non-canonical payload, a resident duplicate (node, `sendTransaction`, `submitMaintainerChange`), a ChangeIndex variant, a no-effect change, and a throwaway delegator (M12.1): each performs 0 block-body reads.
- [ ] REQ-GRV-SEC-016: no block-body read happens while the mempool write lock is held (shown by a test that holds a mempool read guard during admission and still sees the read complete, or by an equivalent seam).
- [ ] REQ-GRV-SEC-017: after one `MPTX014` refusal, 1,000 further copies (one per message) cost 0 body reads and 1,000 height-index point gets.
- [ ] REQ-GRV-SEC-017: T is mined at height h in canonical block X. A copy is refused and cached. The canonical index at h is then re-pointed to a block X' without T (the `m11::store_block(.., canonical = true)` helper that `m11_tx_indexed_only_on_a_side_block_is_still_admitted` uses, `inc_i_237_m11_delegation.rs:314-320`). The next copy is admitted. The existing side-block test stays green.
- [ ] REQ-GRV-SEC-017: after 10,001 distinct txids are cached, 10,000 entries remain, and the missing one is the oldest.
- [ ] REQ-GRV-SEC-018: a message `[T, T]` with T new: T is admitted once and the verdict is Ignore (as at V). A message of 1,000 copies of a mined T: 1 attempt. A message of two txs with inputs that share a txid but differ in an input signature: both are attempted (no suppression by txid).
- [ ] REQ-GRV-SEC-019: as in the table.
- [ ] REQ-GRV-040: as in the table.
- [ ] REQ-GRV-041: the PM-237-12 row text, the `specs/gossip-relay-verdict-status.md` M12 row, the `specs/protocol.md` §7.2 and `specs/security_model.md` admission order, and the `docs/error-codes.md` `MPTX014` row all state "checked after every other refusal; repeated copies hit a bounded cache".

**Test strategy (RED at V; the test-writer writes these first)**
- Seam: a per-instance, `#[doc(hidden)] pub`, read-only counter of block-body reads made by the mined check, on the owner of the negative cache, in the `fault_inject_*` style (`crates/storage/src/state_db/promote.rs:49-55`). Do NOT use a process-global static: `cargo test` runs tests in parallel threads. A `#[cfg(test)]` item is not visible to `bins/node/tests/it`. Tests that read the counter are RED at V because the counter does not exist yet; the RED evidence must say so.
- New `bins/node/tests/it/inc_i_237_m12_mined_cost.rs`: the ordering rows (REQ-GRV-SEC-016; at V each costs 1 read), the cache rows, the reorg row, and the dedupe rows. The input-signature dedupe row is GREEN at V and guards INV-NETWORK-004.
- The REL-573-R2-03 probe rewrite inside `inc_i_237_m11_probe.rs`, with the mutation check.

**Outcome metric (AC4)**
- metric (primary): event-loop wall time of one `handle_gossip_txs` call that carries 1,000 byte-identical copies of a mined DelegateBond T. T sits in a canonical block filled to the builder budget, and T passes every cheaper check: its delegator is registered and its delegation was revoked, so only `MPTX014` refuses it (this is the REL-573-R2-02 input).
- metric (secondary): block-body reads made by that call.
- probe: a `--nocapture` `it` test that prints `M12.2-PROBE handle_ms=<t> body_reads=<n>`.
- before: `handle_ms` MUST be measured by running the probe at V (it needs no seam). `body_reads` = 1,000 by derivation (one read per copy, P2-P4); it can be measured only once the seam exists.
- after: `body_reads` = 1, and `handle_ms` drops by about 999 x the per-copy decode time. That magnitude is the one inferred element of AUDIT-P1-516; this probe settles it.

**Failure modes**
- FM1 reorg / rollback: each hit is re-checked against the canonical height index, so a tx mined only on an orphaned branch is admissible again. There is no reorg hook to forget.
- FM2 flood (2,000-copy message, 0xAA fetch copies, RPC 500/s): each copy costs at most one hash lookup + one point get, and 0 within one message. The first copy of each distinct mined txid pays one full read, and only if it passes every cheaper check.
- FM3 cache churn: cycling more than 10,000 distinct mined txids that each pass the cheaper checks needs that many mined DelegateBonds whose delegators currently have standing and no effective delegation. The corpus is finite; each eviction cycle pays one read per txid again.
- FM4 restart: the cache is memory only, so the first copy after a restart pays one read again. No persisted state.
- FM5 lock: the body read stays outside the mempool write lock, so block production and RPC readers do not wait on it.
- FM6 mixed-version window: old nodes still pay per copy. The verdict shape is unchanged (Ignore for refusals and repeats), so peers see no difference.
- FM7 snap-synced node: it has no bodies below the snap height, so `get_tx_block_height` misses and an old mined DelegateBond is admitted. This is unchanged from M11 (pre-existing; noted, not widened).

### SEC-FIXVERIFY-R573-01..03 dispositions
- **SEC-FIXVERIFY-R573-01 (P3): FOLD** into M12.2 as REQ-GRV-SEC-019. It is one condition at `zero_flow_auth.rs:66`, in a file M12.2 already changes. It also removes the honest byte-identical maintainer re-add refusal (REL-573-R2-02 maintainer half; R573-03 a). It removes no protection that a `reason` variant does not already bypass. The residual (an old carrier revives after a reversal) goes to the INC-I-246 residual list.
- **SEC-FIXVERIFY-R573-02 (P3): DEFER.** The fix is a fee-0 eviction-policy change (`pool.rs:984-988`, `:1380-1399`) plus `revalidate` pruning that needs a maintainer-set input. That is not a few lines. It changes behaviour for every zero-fee system tx in a full pool (the FM-10 history) and needs its own failure-mode analysis. M12.1 removes the free junk that the audit names as the churn source while the pool is not honestly full.
- **SEC-FIXVERIFY-R573-03 (P3):** (a) is closed by REQ-GRV-SEC-019. (b), (c), (e) and the new (d) are documented by REQ-GRV-039 (M12.1). No admission change for these until the INC-I-246 consensus fix.

### Shared constraint (both milestones)

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-GRV-042 | Admission only (AC5, AC11) | Must | `git diff 8cc50d33.. -- crates/core/src/validation bins/node/src/node/apply_block crates/storage` is empty; no activation-height change in `crates/core/src/network_params/`; the `inc_i_173_m3_f4_routing` source scans stay green |

### Consensus-shape checklist (INV-12) and deploy classification

| Question | M12.1 | M12.2 |
|---|---|---|
| (1) Can a user-submittable tx reach this path? | YES (gossip, `sendTransaction`) | YES (gossip, 0xAA, both RPC sites) |
| (2) Can a producer action or attestation pattern reach it? | NO | NO |
| (3) Bit-identical for all reachable inputs on every consensus-visible computation? | YES. Block validation for these types stays shape-only (`tx_types.rs:860-892,965`); apply and the boundary are untouched; a block from an old producer that carries a throwaway delegation still validates and applies identically | YES. Only the order and the cost of an admission refusal change |
| Activation height? | Not required ((1) YES, (3) YES) | Not required |
| Consensus RULES change? | No | No |
| Block CONTENT change (INV-8)? | No. M12 producers stop selecting no-op delegations; tx selection is producer-discretionary and no validator recomputes it | No |
| Deploy | Rolling: admission and relay verdicts only; Ignore carries no peer penalty, so mixed versions do not partition | Rolling, for the same reason |

No genesis reset, no `CURRENT_PROTOCOL_VERSION` change and no `HardForkSchedule` entry is involved (CLAUDE.md #0, INV-4).

### Docs, specs, registry rows

| Target | Change | Milestone |
|---|---|---|
| `docs/error-codes.md` | New `MPTX016` row (boundary-parity refusal for delegations, with the condition list); `MPTX013` row narrowed to "names a delegate other than the effective one / over the cap"; `MPTX014` row: DelegateBond only, checked last, with the honest re-delegation workaround | M12.1, M12.2 |
| `docs/rpc_reference.md` (`sendTransaction`, near the current `MPTX012` / `MPTX014` text) | `MPTX016`; the honest edge flows and their workarounds (REQ-GRV-039) | M12.1 |
| CLI delegation docs (`docs/cli.md`, delegate / revoke) | Revoke then delegate: wait one block; a pending delegation holds the slot | M12.1 |
| `specs/gossip-relay-verdict-status.md` | M12 row; corrected M11 FM3 text; the INC-I-246 residual row names the maintainer revival after a reversal (R573-01) | M12.1, M12.2 |
| `specs/protocol.md` §7.2, `specs/security_model.md` | Delegation admission = boundary preconditions + one resident per delegator; mined check last, bounded cache, per-message dedupe | M12.1, M12.2 |
| memory.db `protection_mechanisms` PM-237-12 | Rewrite the scale assumption (P13); limits = PM-237-02 buckets, PM-237-08 bound, negative-cache cap 10,000; add the per-delegator resident key | M12.2 |

### Traceability (M12)

| Requirement | Priority | Test IDs | Architecture section | Implementation module |
|---|---|---|---|---|
| REQ-GRV-SEC-012 | Must | `inc_i_237_m12_standing.rs` (`m12_sec_012_*`), `inc_i_237_m12_probe.rs`, M8 throwaway rows | Architecture Context (M12) | `Standing` @ `crates/mempool/src/zero_flow_standing.rs`; `post_auth` @ `crates/mempool/src/zero_flow_policy.rs` |
| REQ-GRV-SEC-013 | Must | `inc_i_237_m12_standing.rs` (`m12_sec_013_*`) | Architecture Context (M12) | `Standing::no_op` @ `zero_flow_standing.rs`; `post_auth` @ `zero_flow_policy.rs` |
| REQ-GRV-SEC-014 | Must | `inc_i_237_m12_bound.rs` (`m12_sec_014_*`) | Architecture Context (M12) | `ChangeIndex` (`ChangeKey::Delegator`) @ `crates/mempool/src/zero_flow_newness.rs` |
| REQ-GRV-SEC-015 | Should | `inc_i_237_m12_bound.rs` (`m12_sec_015_*`) | Architecture Context (M12) | `post_auth` @ `zero_flow_policy.rs` |
| REQ-GRV-037 | Must | `inc_i_237_m12_guards.rs` (`m12_req_grv_037_*`), `m11_same_epoch_sequences_*` | Architecture Context (M12) | `Standing::of` @ `zero_flow_standing.rs` |
| REQ-GRV-038 | Must | `inc_i_237_m8_*.rs`, `inc_i_237_m2_node_{relay,verdict}.rs`, `tests_inc_i237_m8_admission.rs` | Architecture Context (M12) | test fixtures only |
| REQ-GRV-039 | Should | n/a (docs) | Architecture Context (M12) | `docs/error-codes.md`, `docs/rpc_reference.md`, `docs/cli.md` §4.11, `specs/gossip-relay-verdict-status.md` |
| REQ-GRV-SEC-016 | Must | `inc_i_237_m12_mined_cost.rs` (`m12_2_sec_016_*`) | Architecture Context (M12) | `admit_zero_flow` @ `crates/mempool/src/zero_flow_mined.rs`; `check_system_transaction` @ `crates/mempool/src/pool.rs`; `zero_flow_policy::check` @ `zero_flow_policy.rs`; `ChangeIndex::probe` @ `zero_flow_newness.rs` |
| REQ-GRV-SEC-017 | Must | `inc_i_237_m12_mined_cache.rs` (`m12_2_sec_017_*`), `inc_i_237_m12_mined_probe.rs` | Architecture Context (M12) | `MinedCheck` + `Mempool::mined_check` @ `crates/mempool/src/zero_flow_mined.rs` |
| REQ-GRV-SEC-018 | Must | `inc_i_237_m12_mined_dedupe.rs` (`m12_2_sec_018_*`) | Architecture Context (M12) | `handle_gossip_txs` @ `bins/node/src/node/validation_checks/gossip_txs.rs` |
| REQ-GRV-SEC-019 | Should | `inc_i_237_m12_mined_cache.rs` (`m12_2_sec_019_*`), `m11_after_{add,remove}_maintainer_applies_*` | Architecture Context (M12) | `admit_zero_flow` (DelegateBond only) @ `crates/mempool/src/zero_flow_mined.rs` |
| REQ-GRV-040 | Should | `inc_i_237_m11_probe.rs` (`m11_audit_p1_509_variant_flood_probe`) | Architecture Context (M12) | test only |
| REQ-GRV-041 | Should | n/a (docs) | Architecture Context (M12) | `docs/error-codes.md`, `docs/rpc_reference.md`, `specs/gossip-relay-verdict-status.md`, `specs/protocol.md` §7.2, `specs/security_model.md`; registry PM-237-12 |
| REQ-GRV-042 | Must | scope diff + `inc_i_173_m3_f4_routing` scans | Architecture Context (M12) | admission only (no validation / apply / storage / activation-height change) |

### What I did not verify
- The status that the normal `register()` path gives a queued Register. I read only that `register_genesis_producer` sets Active (`set_registration.rs:195`), not the `ProducerInfo` that a Register tx builds. The REQ-GRV-037 queued-Register guard settles it.
- The real per-copy cost of decoding a builder-budget block (the auditor inferred 1-10 ms). The M12.2 probe measures it.
- That C's only peer in `line()` stays B (no discovery link between A and C). The M12.1 and REL-573-R2-03 probes should assert C's peer set.
- The mempool expiry for zero-fee residents. It bounds the M12.1 FM4 stale slot.

### Open questions for the orchestrator
1. Accept the wider M12.1 SSF (all boundary preconditions + one resident per delegator) instead of the brief's standing-only sentence (P9, P10)? Its only honest cost is friction (d): revoke then delegate back to back waits one block.
2. Code choice: a new `MPTX016` for the boundary-parity refusals? They are permanent parity rules, not part of the INC-I-246 stop-gap that `MPTX013` documents.
3. REQ-GRV-SEC-019 widens admission for a byte-identical replay of a mined maintainer change after a reversal (equal to today's exposure to a variant). Acceptable?
4. The AUDIT-P1-505 findings row still says `fixed`, but its delegation half is AUDIT-P1-515. Reopen it, or keep the link? (Linked in an incident entry; status unchanged.)
5. Q1/Q2/Q3 are still unconfirmed. REQ-GRV-039 (a) rests on Q2.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -one full block decode + per-tx hash scan per refused copy of a mined zero-flow tx (observed path; magnitude inferred, measured by the M12.2 probe); +one ProducerSet field read per delegation admission under the read lock snapshot_for already takes (observed); +one hash of the serialized tx per gossip tx for the per-message dedupe (inferred)
  Memory:   +negative cache <= 10,000 x (32 B txid + 8 B height + 32 B hash + map overhead), about 1 MB worst case (inferred); +one map entry per resident delegation tx, bounded by registered producers (inferred); +transient per-message dedupe set <= 2,000 entries (inferred)
  IO:       -3 RocksDB reads incl. one block body per refused replay; +1 height-index point get per cache hit (observed: queries.rs:84-102)
  Network:  -fee-free relays of throwaway-key delegations (observed: the AUDIT-P1-515 flow is closed); verdict shape unchanged (observed)
  Disk:     0 (no persisted state; observed)
  Latency:  -event-loop stall of up to 2,000 decodes per message (inferred); +microseconds per delegation admission (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: the per-message dedupe alone for M12.2 (leaves one decode per copy across messages at the global bucket) and standing-only for M12.1 (leaves the registered-delegator variant sink that the fix-verifier tests for)
Why this proposal anyway: both cheaper variants leave open a P1 path that the round-2 auditor's own test strategy checks; the full SSF closes the cause with no storage, consensus or activation-height change
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.80, observed)
Reasoning: both root causes re-verified at V; the fixes are deterministic, admission-only and local to crates/mempool plus the admission call sites; brittleness 2/5 LOCALIZED; the reorg-safe cache uses an existing point get, so there is no storage or consensus change
━━━━━━━━━━━━━━━━━━━━━━
```
