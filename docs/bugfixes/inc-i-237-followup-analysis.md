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
| REQ-GRV-SEC-007 | Must | `inc_i_237_m8_admission::req_grv_sec_007_gossip_refuses_*` (4), `admission_matches_apply_*` (2), `devnet_unsigned_legacy_delegate_bond_is_still_admitted`; rpc `tests_inc_i237_m8_admission::req_grv_sec_007_*` (2), `inv_rpc_002_*` (2) | (architect) | zero_flow_auth::authorized @ crates/mempool/src/zero_flow_auth.rs; Mempool::add_system_transaction @ crates/mempool/src/pool.rs; current_set_for callers @ bins/node/src/node/validation_checks/mod.rs, crates/rpc/src/methods/{transaction,governance}.rs |
| REQ-GRV-SEC-008 | Must | `m5_audit_p1_505_*` (2, flipped in M8), `inc_i_237_m8_admission::d5_mixed_forged_and_genuine_zero_flow_message_is_ignored` | (architect) | zero_flow_auth::authorized @ crates/mempool/src/zero_flow_auth.rs; handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs |
| REQ-GRV-029 | Must | `inc_i_237_m8_admission::req_grv_029_genuine_zero_flow_of_each_type_authorizes_a_forward`, `devnet_unsigned_legacy_delegate_bond_authorizes_a_forward`, `inc_i_237_m8_relay::req_grv_029_genuine_zero_flow_txs_reach_a_node_two_hops_away` | (architect) | handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs; zero_flow_auth::is_authenticated @ crates/mempool/src/zero_flow_auth.rs |
| REQ-GRV-030 | Must | `inc_i_237_m8_block::req_grv_030_old_block_with_forged_*` (2) | (architect) | none (admission-only; crates/core/src/validation/ and bins/node/src/node/apply_block/ unchanged) |
| REQ-GRV-031 | Should | `inc_i_237_m8_admission::req_grv_031_zero_flow_registration_never_forwards` | (architect) | handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs (Registration is not in zero_flow_auth::is_authenticated) |
| REQ-GRV-SEC-009 | Should | (decision) | (architect) | not fixed: residual recorded as INC-I-246 (memory.db incident_entries) |
| REQ-GRV-032 | Should | network `it::inc_i_237_m9_spec_sync::m9_req_grv_032_*` (4) | n/a (docs) | specs/protocol.md §7.2, specs/security_model.md, specs/gossip-relay-verdict-status.md, redesign traceability matrix, .claude/skills/network/SKILL.md, SKILLS-INDEX.md; protection_mechanisms PM-237-* (memory.db) |
| REQ-GRV-SEC-010 | Must | scan command (M10) | n/a | (orchestrator) |

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
