# Redesign Analysis: Gossip Relay Verdict + Swarm/Node Channel Contract (INC-I-237)

> Workflow: `/omega-redesign --scope="tx gossip relay + swarm/node channels" --incident=INC-I-237` — PROPOSAL-ONLY, Step 0 (problem scoping).
> Code revision read: origin/main = v6.43.3 = `d0e15686` (worktree `wt-main`). Pre-hotfix comparison: `git diff v6.43.0 v6.43.3`.
> All `file:line` citations are against `d0e15686` unless they say `v6.43.0`.
> Analyst date: 2026-10-11. Requirement prefix: `REQ-GRV-NNN` (Gossip Relay Verdict), `REQ-GRV-SEC-NNN` (security).

## SSF line (Rule 18 — subtraction first)

**Make the node the only relay authority for every gossip topic: the swarm may decide only Ignore or Reject on its own cheap checks, a message is forwarded (Accept) only after the node's own handler has judged it, nothing received is re-published under a new message id, a topic the node does not consume is not relayed, and the swarm reaches the node only through a sink that cannot await.**

Why this removes the recurring root: all five occurrences (INC-I-114 blocks, INC-I-120 sync, INC-I-137 producers, INC-I-142 five topics, INC-I-237 txs) have one shape — the swarm says Accept before the node has a verdict, so gossipsub forwards content that no peer can absorb. Each fix so far added a per-topic swarm-side filter. The line above removes the decision from the wrong layer instead of adding a sixth filter. It deletes code (the tx Ignore override, the node's re-publish of received txs, the relay of two consumer-less topics, the awaitable sender in the swarm) more than it adds (one verdict path from node to swarm).

---

## 1. Current architecture of the seam

### 1.1 Per-topic relay map (who decides, and when)

All gossip messages arrive in `handle_behaviour_event` (`crates/network/src/service/behaviour_events.rs:54-58`) inside the **swarm task**. gossipsub holds each message un-forwarded until `report_message_validation_result` (`validate_messages()` at `crates/network/src/gossip/config.rs:215`). There are exactly **2 report call sites**: `behaviour_events.rs:104-107` (block-body topics) and `:265-269` (all others). The node task's verdict **never** flows back to gossipsub for any topic: there is no `NetworkCommand` that carries a verdict (`crates/network/src/service/types.rs:103-160`).

| Topic | Subscribed? | Swarm-side gate before verdict | Verdict reported | Verdict values | Post-Accept local drops (already forwarded) | Node consumer / node verdict | Relay authority today |
|---|---|---|---|---|---|---|---|
| `/doli/blocks/1` | yes `config.rs:406` | `Block::deserialize` + stale cutoff 6 slots (`validation.rs:35`, `classify_block_gossip`); **no** seen cache, **no** future-slot bound, **no** producer/VDF check | immediately in swarm, `behaviour_events.rs:91-107` | Accept / Ignore (stale) / Reject (undecodable) | memory-watchdog shed `:130-138`; per-peer/global rate limit `:157-170`; `enqueue_or_shed` try_send drop `:188-192` | `on_new_block_event` → full validation in node (`event_loop.rs:282-283`) | **swarm** (stateless classifier) |
| `/doli/t1/blocks/1` | **no** (not in `subscribe_to_topics`, `config.rs:398-431`) | same branch `:194-218` | — | — | — | — | dormant |
| `/doli/r{N}/blocks/1` | **no** (`region_topic` only used by `publish_to_region`, `publish.rs:96`, no production caller) | same branch `:219-229` | — | — | — | — | dormant |
| `/doli/txs/1` | yes `:409` | raw-bytes SeenCache 180 s (`staleness.rs:393-401`) | immediately, `:241-269` | first sight Accept → **overridden to Ignore** `:260-264` (v6.43.3 (b)); dup → Ignore | rate limit `:290-297`; EventSink shed (tx is sheddable, `event_sink.rs:40-52`) | `handle_new_transaction` → mempool (`validation_checks/mod.rs:977-1012`); on admit → `broadcast_transaction` (`:1025-1030`) → swarm batcher (`swarm_loop.rs:191-238`) → **new publish** | **node via re-publish** — refused as `Duplicate` when the batch bytes equal the received bytes (vendor `behaviour.rs:607-616`, content-only id `config.rs:199-202`) → one hop [E8] |
| `/doli/producers/1` | yes `:412` | `classify_producer_gossip`: all announcements older than 3600 s → Ignore; undecodable → **Accept** (fail-open) (`validation.rs` `PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS`) ; no seen cache | immediately | Accept / Ignore | rate limit `:319-327`; EventSink shed | GSet merge in node | **swarm** |
| `/doli/votes/1` | yes `:415` | SeenCache + timestamp age 7 days; undecodable → Accept (`staleness.rs` `classify_vote_gossip`) | immediately | Accept / Ignore | rate limit `:397-401`; EventSink shed | `on_new_vote` (`event_loop.rs:356-358`) | **swarm** |
| `/doli/heartbeats/1` | yes `:418` | SeenCache + slot age 6; undecodable → Accept | immediately | Accept / Ignore | rate limit `:408-412`; EventSink shed | **NONE** — `event_loop.rs:359-361` is a no-op; our nodes never publish (`NetworkService::broadcast_heartbeat` is a no-op, `service/mod.rs:535-538`) | **swarm**, relay with no consumer |
| `/doli/headers/1` | yes `:421-424` | SeenCache + slot age 6; undecodable → Accept | immediately | Accept / Ignore | rate limit `:433-437`; EventSink shed | **NONE** — `event_loop.rs:285-287` logs at debug only; published by every producer (`production/mod.rs:653`) | **swarm**, relay with no consumer |
| `/doli/attestations/1` | yes `:426-429` | SeenCache + slot age 12; undecodable → Accept | immediately | Accept / Ignore | rate limit `:419-423`; EventSink shed | `on_new_attestation` verifies the signature **after** the relay (`network_events.rs:542-582`, `attestation.verify()` at `:544`) | **swarm** |
| any other string | n/a | `GossipTopic::from_topic_str` → `None` (`staleness.rs:172`) | immediately | **Accept** (`behaviour_events.rs:252`) | — | none | **swarm**, accept-by-default |

Supporting registries of "already seen / relayable" (5, in 2 tasks):

| Registry | Owner | Key | Lifetime / bound | Cite |
|---|---|---|---|---|
| gossipsub `duplicate_cache` | swarm (vendor) | message id = BLAKE3(data)[..20], no source/seqno | 300 s | `config.rs:67,234`, id fn `:199-202` |
| gossipsub `mcache` (pending-validation + IHAVE window) | swarm (vendor) | message id | `history_length 5` × heartbeat 1 s ≈ 5 s; late Accept → "Message not in cache. Ignoring forwarding" | `config.rs:207,229`; vendor `behaviour.rs:747-787` |
| app `SeenCache` | swarm | BLAKE3(topic_u8 ‖ raw bytes) | 180 s TTL, 16,384 entries drop-oldest | `staleness.rs:99,110`, created `swarm_loop.rs:87` |
| mempool | node | `tx.hash()` (signature-excluding) | until mined/evicted; `max_count`/`max_size` policy | `crates/mempool/src/pool.rs:475,872,1055` |
| producer GSet | node | producer pubkey | permanent CRDT | node network_events |

### 1.2 Message-id and vendored gossipsub facts

- Message id = `BLAKE3(message.data)[..20]` (`config.rs:199-202`) although `MessageAuthenticity::Signed` (`config.rs:248`) carries source+seqno. Consequences: (i) two publishers of byte-identical data share one id (good dedup); (ii) a node that Ignores a received batch and later publishes the same bytes is refused `PublishError::Duplicate` for 300 s (vendor `behaviour.rs:607-616`) → the [E8] one-hop regression; (iii) any re-composition of txs into a new batch is a new id → the [E4] amplifier.
- `flood_publish(true)` (`config.rs:238`): an origin sends to all topic peers, not only mesh peers. This is why txs still reach the origin's direct neighbours (and get mined) under (b).
- Two gossipsub builders exist: `new_gossipsub` (`config.rs:193`, production, `service/mod.rs:130`) and `new_gossipsub_with_cache_time` (`config.rs:343`, test-only). Both hard-code the same id function.
- `assert_gossip_hardening_invariant(&config, true)` (`config.rs:246,376`) passes a **literal `true`** for `has_bounded_queue`: the INV-NETWORK-002 queue half is asserted, not checked.
- Vendor patch (`vendor/libp2p-gossipsub/DOLI-PATCH.md`): per-connection `send_queue` capped at 4 MiB (`handler.rs:46`), drop of publish/IHAVE/IWANT over cap. Removal path: libp2p-gossipsub ≥ 0.48.

### 1.3 Swarm ↔ node channels

| Channel | Type / capacity | Producer(s) | Consumer | Blocking behaviour | Cite |
|---|---|---|---|---|---|
| event channel | `mpsc`, cap `clamp(max_peers*16, 4096, 32768)` | swarm task via `EventSink::emit` (try_send) and `enqueue_or_shed` (try_send, NewBlock) | node event loop, `biased` select, events first; drain up to `max_peers*3` before production | swarm never awaits since 6.43.3; full ⇒ gossip events **shed**, control events **backlogged** | `service/mod.rs:88-90`; `event_sink.rs:71-97`; `backpressure.rs:73-78`; `event_loop.rs:47-60,71-74,104-107` |
| control backlog | `Mutex<VecDeque>`, **8,192 events (count, not bytes)** | `EventSink::emit` for non-sheddable events (PeerConnected/Disconnected, Status*, Sync*, TxFetch*, mismatches) | swarm select arm `reserve()` → `flush_one` | over cap ⇒ control events **dropped**; `SyncResponse` can carry up to `MAX_SYNC_SIZE` 16 MiB | `event_sink.rs:29,88-96,107-111`; `swarm_loop.rs:183-188`; `protocols/sync.rs:22` |
| command channel | `mpsc`, cap `clamp(max_peers*4, 2048, 32768)` | node task (31 `.await` sites, below); RPC closures via `tokio::spawn` per tx/vote | swarm select arm; `BroadcastTransaction` intercepted into `tx_batch`, others `handle_command` (no inner await) | producers `.send().await` ⇒ node loop parks when full | `service/mod.rs:89-91,449-692`; `swarm_loop.rs:191-219`; `command_handling.rs` |
| RPC broadcast | **unbounded** spawned tasks, each `cmd_tx.send(...).await` | RPC `sendTransaction` / governance | command channel | one parked task + one `Transaction` per RPC call while the command channel is full | `bins/node/src/node/startup.rs:428-436,439-452` |
| swarm tx batcher | `Vec<Transaction>` flushed at ≥50 or every 100 ms | command arm | `gossipsub.publish` | bounded by flush | `swarm_loop.rs:205-215,221-238` |
| peers table | `Arc<RwLock<HashMap>>` | swarm writes (`swarm_events.rs:112,242`; `behaviour_events.rs:669`) | node `peer_count()/get_peers()` short reads; RPC `try_read` | no lock is held across a channel await (verified) | `service/mod.rs:656-668`; `startup.rs:405-412` |
| gossipsub send_queue | per connection, 4 MiB | gossipsub | connection handler | drop over cap | vendor `handler.rs:46` |

Node-side `.await` on a network command (all go through `command_tx.send().await`): `event_loop.rs:211,223,321,330,344`; `network_events.rs:43,269,314,659,688`; `validation_checks/mod.rs:1029,1082,1207,1214`; `periodic.rs:418,702,734,755,821,888,921,928,990,1310`; `production/mod.rs:653,654`; `block_handling.rs:285,454`; `startup.rs:90,652,679` — **31 sites** in the node event loop's call graph.

### 1.4 The await-cycle

- **C1 (v6.43.0, the [E6a] park):** node `[ATTEST_FETCH]` → `request_sync(...).await` (`periodic.rs:417-422`, v6.43.0 `:411-423`) → `command_tx.send().await` on a full 2,048 channel (`service/mod.rs:570-581`) **while** the swarm, the only consumer of `command_rx`, was parked in `event_tx.send(...).await` on a full 4,096 channel (v6.43.0 `behaviour_events.rs:290`, and 15 other `send().await` sites removed by `git diff v6.43.0 v6.43.3 -- crates/network/src/service/`). Each task waits for the other to drain → permanent mutual park.
- **v6.43.3 state:** the swarm side no longer awaits event delivery. The only remaining swarm-side awaits are the peers `RwLock` (`swarm_events.rs:112,242`; `behaviour_events.rs:669`; reads in `swarm_loop.rs:251,346,464,510,527-529`), and no node path holds that lock across an await. **Await-cycles today: 0 — by convention only.** `EventSink::sender()` (`event_sink.rs:66`) still hands the swarm an awaitable `&mpsc::Sender<NetworkEvent>` (used at `behaviour_events.rs:189,214,225`), so one future `.send().await` in swarm code re-creates C1 with no compile-time or test guard beyond `inc_i_237_loop_deadlock.rs`.
- **Residual non-cycle stall:** the node loop still parks on a full command channel (31 sites). Meanwhile the swarm sheds every gossip event (tx, attestation, header, vote, producer) into the same shed bucket, and drops control events past 8,192.

---

## 2. Capability inventory (PRIOR-KNOWLEDGE-GATE)

| Primitive | Count | Members / cite |
|---|---|---|
| Gossip topics defined | **9** (7 static + Tier1 + region family) | `gossip/mod.rs:18-49` |
| Gossip topics subscribed | **7** | blocks, txs, producers, votes, heartbeats, headers, attestations (`config.rs:398-431`) |
| Topics with a local consumer | **5** | blocks, txs, producers, votes, attestations |
| Topics relayed with **no** local consumer | **2** | headers, heartbeats |
| `GossipTopic` enum variants | **8** | `staleness.rs:137-158` (Blocks, Tier1Blocks, Producers, Attestations, Heartbeats, Headers, Votes, Transactions) |
| Gossip wire message types | **10** | Block, BlockHeader, tx batch `0xBA`, legacy single tx, tx announce, ProducerSet (protobuf), producer bloom digest, legacy bincode producer list, Vote (JSON), Heartbeat, Attestation (`behaviour_events.rs:290-440`; `publish.rs:111-216`) — 11 if legacy bincode counted separately |
| `MessageAcceptance` outcomes | **3** | Accept, Ignore, Reject |
| `report_message_validation_result` call sites | **2** | `behaviour_events.rs:104-107`, `:265-269` |
| Relay-decision sites in code | **5** | block classify `:91`; unified `classify_gossip` `:241-251`; `None → Accept` `:252`; tx override `:260-264`; node re-publish `validation_checks/mod.rs:1029` |
| Classifier functions | **9** | `classify_block`, `classify_block_gossip`, `classify_producer_gossip`, `classify_gossip`, + attestation, heartbeat, header, vote, transaction classifiers (`validation.rs`, `staleness.rs`) |
| Post-Accept local drop paths (forwarded but not processed) | **4** | rate limiter, memory-watchdog shed, `EventSink` shed, `enqueue_or_shed` block shed |
| `NetworkEvent` variants (swarm→node) | **21** | `types.rs:18-100`; sheddable 9 (`event_sink.rs:40-52`), control 11, NewBlock 1 (own shed path) |
| `NetworkCommand` variants (node→swarm) | **19** | `types.rs:103-160`; **0** carry a gossip verdict |
| Channels between swarm and node | **2** mpsc + 1 backlog + 1 shared RwLock | §1.3 |
| Node-side awaited command sends | **31** | §1.3 |
| Mempool admission entry points | **2** | `Mempool::add_transaction` (`pool.rs:475`), `add_system_transaction` (`pool.rs:872`, for zero-flow types: Registration, DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer — `types.rs:187-193`) |
| Admission callers | **4** | gossip/fetch `handle_new_transaction` (`validation_checks/mod.rs:1003-1010`, via `event_loop.rs:289`, `network_events.rs:710`); RPC `transaction.rs:223`; RPC `governance.rs:285` |
| "Relayable" registries | **5** | §1.1 second table |
| Dormant relay capabilities | **3** | tx announce/fetch mode (`config.rs:132` default false, never set by the node); Tier1 topic; region topics |
| Dead code in the seam | **3** | `command_handling.rs:66` `BroadcastTransaction` arm (always intercepted at `swarm_loop.rs:193`); `classify_gossip` Blocks arm (block topics return at `behaviour_events.rs:232` first); `BroadcastHeartbeat` publish path (no sender) |

---

## 3. What is tangled or misplaced

1. **The relay decision is in the wrong layer.** The swarm can only judge bytes (decode, age, seen). Validity — signature, mempool admission, producer eligibility — is known only in the node, and it never returns to gossipsub. Every non-tx topic is forwarded on a swarm guess. Attestations are forwarded before `attestation.verify()` (`network_events.rs:544`); a forger can make each forged attestation distinct bytes, so the raw-bytes SeenCache does not bound it. This is the INC-I-237 tx shape on another topic.
2. **Drops happen after the forward.** The rate limiter, memory watchdog, and event shed all run after `report_message_validation_result`. A message the node refuses to process for load reasons is still forwarded to the mesh. The comment at `behaviour_events.rs:123-129` states this is intended ("gossip propagation to other peers is preserved").
3. **Relay without a consumer.** Headers and heartbeats are relayed network-wide, but no local code consumes them (§1.1). This is pure amplification surface with zero function.
4. **Two relay mechanisms for txs.** The 6.43.3 tx path relays by node re-publish (new message), not by gossipsub forward. Because the message id is content-only, re-publish of the same bytes is refused, and re-publish of other compositions creates new ids. One mechanism (forward the original message after the node verdict) would keep the id stable and needs no batcher on the relay path.
5. **Five "seen/relayable" registries with different keys and lifetimes** (300 s msg-id, ~5 s mcache, 180 s raw-bytes SeenCache, mempool by txid, GSet). They disagree by design: the SeenCache TTL (180 s) is shorter than the duplicate cache (300 s) since 6.43.1; F-1 in `staleness.rs:112-122` still reasons against the old 120 s value.
6. **Back-pressure direction.** Events flow swarm→node and commands node→swarm on two bounded channels with `.await` producers. Since 6.43.3 only the node side awaits, which is acyclic. But the guarantee is not in the type system, and the control backlog is count-bound, not byte-bound.
7. **One shed bucket for all gossip.** A tx flood fills the same event channel as attestations and producer announcements, and `EventSink` sheds them all together (`event_sink.rs:40-52`). The node can lose attestations (presence → rewards) because of tx volume.
8. **Invariant violated by the hotfix.** INV-NETWORK-003 says "a genuinely-new-to-this-node message must still Accept+forward exactly once". v6.43.3 (b) Ignores every first-sight tx batch (`behaviour_events.rs:260-264`). The invariant record and the code disagree today.

---

## 4. Requirements (acceptance criteria for the redesign)

| ID | Requirement | Priority | Acceptance criterion (testable) |
|---|---|---|---|
| REQ-GRV-001 | Valid transactions propagate multi-hop (INV-NETWORK-003) | Must | On a line topology A–B–C–D (no other links, real `NetworkService` + real mempool admission on B and C), a valid tx submitted at A is admitted at D within 5 s. Run two variants: (i) a single-tx batch (the [E8] `Duplicate` case) and (ii) a 50-tx batch. Mainnet probe: log count of `Not publishing a message that has already been published` on `/doli/txs/1` = 0 over 24 h. |
| REQ-GRV-002 | No forward before the node's own verdict — transactions | Must | On F–B–C where B's node rejects every tx, C receives 0 F-originated txs (the existing `inc_i_237_tx_relay` O1 stays green). Add the missing partition: B admits → C receives (O4 > 0). |
| REQ-GRV-003 | No forward before the node's own verdict — every topic, or a written per-topic exception | Must | For each subscribed topic, a 3-node line test: a message that B's node handler rejects (bad signature, unadmittable, unknown) is received 0 times at C. A topic may keep a pre-verdict Accept only with a written exception that gives (a) the latency need and (b) a volume bound per slot that holds for forged input. The exception list is in code (exhaustive match, no wildcard), and the `None → Accept` default (`behaviour_events.rs:252`) is gone or has an exception entry. |
| REQ-GRV-004 | Re-batching cannot amplify unabsorbable content | Must | A publisher that wraps the same K unadmittable txs into M distinct batches (new ids) causes M deliveries at its direct neighbours and **0** deliveries at 2 hops. Bytes a compliant node forwards for that content = 0. Re-use the `inc_i_237_flood.rs` harness with a re-batching publisher. |
| REQ-GRV-005 | Each tx goes on the wire at most once per node, under one identity | Must | Per node, per tx hash, per mempool residence: forwards + publishes ≤ 1, and the node never emits received content under a second message id. Test: count publish+forward events per tx on B in the REQ-GRV-001 topology. |
| REQ-GRV-006 | No await-cycle between the swarm task and the node task | Must | The swarm task holds no handle that can `.await` on the event channel (type-level: the sink type has no async send and no accessor that returns an awaitable `Sender`). Test: stop the node consumer for 60 s while it floods 10× the command capacity. The swarm loop keeps polling (its 1 s ticks still fire). After the node resumes, every control event that fits the bound is delivered and the loop does not hang (extends `inc_i_237_loop_deadlock.rs`). Metric: await-cycles = 0, enforced by type. |
| REQ-GRV-007 | Bounded memory for every queue in the path | Must | Each queue has a documented ceiling in bytes (or count × max item size): event channel, control backlog (**byte** bound — `SyncResponse` ≤ 16 MiB), command channel, RPC broadcast path (no unbounded `tokio::spawn` per tx, `startup.rs:431-436`), tx batcher, pending-verdict set, mcache, send_queue, SeenCache. Flood test at 10× the [E4] peak rate for 10 min: retained heap < 64 MiB above baseline (the `inc_i_237_flood.rs` cap). |
| REQ-GRV-008 | Verdict latency fits the gossipsub validation window; late means not relayed | Must | p99 time from receive to verdict < 2 s under the REQ-GRV-007 flood (window ≈ 5 s: `history_length 5` × 1 s heartbeat). A verdict that misses the window, or a node that is in catch-up, results in no forward (fail-closed for relay), never a stalled swarm, never a missing report (INV-NET-002). Pending-verdict state is bounded and drained on shutdown. |
| REQ-GRV-009 | Block propagation latency is not worse | Must | Measure the baseline first on testnet (`[GOSSIP_BLOCK] recv_ts_ms − block_ts`, p50/p99, 4-node line and full fleet). After the change: p50 and p99 ≤ baseline + 50 ms. No new await on the block path. |
| REQ-GRV-010 | Attestation delivery is not worse | Must | Epoch attestation coverage (attesting producers / active producers) on testnet over ≥ 3 epochs ≥ the baseline. Under a tx flood (REQ-GRV-007 load) attestation coverage does not drop. |
| REQ-GRV-011 | Node-local: no consensus change, no activation height | Must | Diff touches no file under `crates/core/src/validation/`, `bins/node/src/node/apply_block/`, `block.rs`, `transaction/`, `network_params/`; no change to the message-id function, topic names, gossip wire encodings, `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION` or `HardForkSchedule`. The state root converges on a mixed-version testnet. The three-question checklist (§7) is answered in the commit message. |
| REQ-GRV-012 | Rolling deploy with mixed versions | Must | Mixed testnet (new + 6.43.3 + 6.43.0-equivalent + an old-version external, `MIN_PEER_PROTOCOL_VERSION = 1`): (a) txs from an old node reach every new node ≥ 3 hops away; (b) txs from a new node reach old 6.43.3 nodes; (c) no Reject or peer-score penalty for a valid message in an old format; (d) no partition. No synchronized stop-all needed. |
| REQ-GRV-013 | Gossip invariants preserved and the invariant record corrected | Must | INV-NET-002 (exactly one report per message, on every path including deferred, timeout and shutdown); INV-NETWORK-002 (the assertion checks the real queue, not the literal `true` at `config.rs:246`); INV-NETWORK-003 amended to "a new message that the node judged relayable is forwarded exactly once"; INV-NETWORK-004 (raw-bytes seen key); INV-SYNC-009 untouched; INV-ATTEST-002. Each has a linked regression test that passes. |
| REQ-GRV-SEC-001 | All gossip input is untrusted; relay depends only on local checks and the node's own verdict | Must | No relay decision reads an unauthenticated field as authority (INV-AUTH-002). Test: forged attestation, forged header, and forged block with valid encoding but an invalid signature or eligibility → not forwarded past the first honest node (or within the documented exception bound). |
| REQ-GRV-SEC-002 | Forged variants cannot suppress genuine messages | Must | A forged message with a victim's semantic fields does not cause the genuine message to be Ignored or not forwarded (INV-NETWORK-004 test set extended to the verdict path). |
| REQ-GRV-SEC-003 | Per-peer resource bound decided before the forward | Must | A message dropped for rate, memory or queue reasons is reported Ignore (not forwarded). One peer cannot hold more than a fixed byte share of the pending-verdict state. Test: a single peer at 100× its rate limit → forwards attributable to it ≤ the limit. |
| REQ-GRV-SEC-004 | Reject only for provably bad bytes | Must | `Reject` (score penalty) only for undecodable or invalid-by-construction content. State-dependent failures (unknown parent, not in mempool, stale) → Ignore. Mixed-version formats are never Rejected. Test: a valid-but-unknown-format message from an old peer → 0 score penalty. |
| REQ-GRV-SEC-005 | Bounded decode before allocation | Must | Tx batch decode caps the tx count and total bytes before it allocates (`max_transmit_size`); a 1 MiB batch of minimum-size txs cannot allocate more than 2× its wire size. |
| REQ-GRV-014 | One relay authority | Should | Relay-decision sites: 5 → 1 (one function or one table of per-topic rules). Registries that can authorize a forward: 5 → 1 (the node verdict), plus swarm pre-filters that can only Ignore or Reject. |
| REQ-GRV-015 | Do not relay topics that have no local consumer | Should | Headers and heartbeats are not forwarded unless a consumer exists (currently `event_loop.rs:285-287,359-361`). Relay-without-consumer topics: 2 → 0. Precondition: Q3 answered. |
| REQ-GRV-016 | Channel depth and shed metrics with real readers | Should | Prometheus gauges and counters for event depth, command depth, gossip shed by topic, control drops, pending verdicts, late verdicts, and send_queue drops, plus one INFO summary line per minute. A dashboard or alert reads each one (readers > 0; today the `event_sink.rs` counters have 0 readers [M1]). |
| REQ-GRV-017 | A tx flood cannot starve blocks, attestations or sync | Should | Under the REQ-GRV-007 flood: 0 shed blocks, 0 shed attestations, 0 dropped control events. Txs and other bulk gossip are shed first. Two lanes at most — do not build a priority framework. |
| REQ-GRV-018 | Non-foreclosure (MANDATORY) | Should | The design document gives the evolution path and shows that each item stays possible without rework of the seam: (1) thousands of producers in 10 s slots — attestation volume grows linearly, so the verdict path must not do per-message work that scales with the producer count beyond one signature check, and the pending-verdict bound scales with peers, not producers; (2) more topics — Tier1/region blocks and tx announce/fetch are dormant today; adding a topic = one exhaustive match arm + one node verdict, with no wildcard; (3) libp2p-gossipsub ≥ 0.48 — the design must not depend on the vendored `handler.rs` patch or on 0.46-only internals, and the vendor directory must be removable; (4) announce-then-fetch for blocks > 2 MB (INV-NETWORK-001 Era 1+) must still fit the same verdict seam. Build none of these now. |
| REQ-GRV-019 | Specs and docs aligned | Should | `specs/protocol.md` (~2085-2100, 2227), `staleness.rs:112-122` doc comment, `specs/security_model.md:620`, `docs/architecture.md` describe the new relay rule. The drift in §6 is closed. |
| REQ-GRV-020 | Remove dead seam code | Could | Delete `command_handling.rs:66` arm, the unreachable `classify_gossip` Blocks path (or route blocks through it), the heartbeat publish path, and `new_gossipsub_with_cache_time` duplication. Net lines in `crates/network/src/service/` ≤ today. |
| REQ-GRV-021 | Shrink oversize seam files | Could | `behaviour_events.rs` (811), `service/mod.rs` (693), `swarm_loop.rs` (581) each ≤ 500 lines after the change (module budget). At minimum, none grows. |
| REQ-GRV-022 | One tx per gossip message if batching has no measured benefit | Could | Only if the design evaluators show that batching gives no benefit at mainnet tx rates. Wire-compatible because every decoder accepts a legacy single tx (`publish.rs:134-160`). |
| REQ-GRV-W01 | `handle_equivocation` payload, `pending_proofs` cap, `add_system_transaction` type guard ([E2]) | Won't | Owed via `/omega-doctor --incident INC-I-237`. **Dependency:** after this redesign, mempool admission becomes the tx relay authority. The doctor fix must keep a test that a tx type the block validator can never include (SlashProducer) is not admitted from gossip. Today it is refused (`FeeTooLow(0,1)`). |
| REQ-GRV-W02 | apply_block skip-guard parent-hash check; Rule-2 bare-gap snap ([E7], INC-I-235) | Won't | Owed separately (fork domain). |
| REQ-GRV-W03 | `[INTEGRITY]` scan cadence/concurrency ([E9]) | Won't | Owed separately (`periodic.rs:248-262,1464-1491`). |
| REQ-GRV-W04 | INC-I-236 signed headers | Won't | Owed separately with its own activation height. **Dependency:** while headers are unsigned, the block-topic exception (Q1) has no volume bound for forged blocks. Record this in the exception text. |
| REQ-GRV-W05 | production host density, swap and MemoryMax ([E6b]) | Won't | Ops item. |
| REQ-GRV-W06 | libp2p upgrade to ≥ 0.48 itself | Won't | Kept possible by REQ-GRV-018. |
| REQ-GRV-W07 | Sync request/response governance (INV-SYNC-009) | Won't | Not gossip. Unchanged. |
| REQ-GRV-W08 | Change of message-id function, topic names or wire encodings | Won't | Wire-compat risk with `MIN_PEER_PROTOCOL_VERSION = 1` peers (mismatched ids break IHAVE/IWANT and dedup across versions). Needs its own analysis. |

None of the Won't items must move inside this redesign. W01 and W04 are dependencies: their tests guard the relay authority. They are not part of the relay change.

### Metrics that matter

| Metric | Today (v6.43.3) | Target |
|---|---|---|
| Places that decide relayability | 5 sites, 2 tasks | 1 authority (node verdict) + Ignore/Reject-only pre-filters |
| Topics forwarded before the node's verdict | 6 of 7 subscribed (txs now never forwarded) | 0, or the documented exceptions only (candidate: blocks) |
| Topics relayed with no local consumer | 2 (headers, heartbeats) | 0 |
| Post-Accept drop paths (forwarded, not processed) | 4 | 0 |
| Max hops for a valid received tx | 1 (byte-identical batch) [E8] | ≥ 3 on a line; full mesh diameter in practice |
| Swarm↔node await-cycles | 0 by convention (1 in v6.43.0) | 0, enforced by type |
| Queues bounded by count only or unbounded | 2 (control backlog; RPC spawn-per-tx) | 0 |
| Channel/shed metrics with readers | 0 | all REQ-GRV-016 series |

---

## 5. Constraint table — what already failed and what must not be repeated

| Attempt | What it did | Result | Rule for the redesign |
|---|---|---|---|
| 6.43.1 `duplicate_cache_time` 60 → 300 s (`4e7e3f97`) | Longer msg-id dedup | Removed the re-delivery signature only; RSS still grew ~2 MB/s (re-batching makes new ids) | Dedup by id cannot stop new-id amplification. Do not tune TTLs as the fix. |
| 6.43.2 4 MiB send_queue cap (`63a3f67e`) | Bounded per-connection bytes | Fixed retained memory; did not stop the flood or the freezes | Bounds are a retainer fix, not a relay fix. Keep them, but do not count them as the design. |
| 6.43.3 (b) tx first sight → Ignore (`83f9e7b1`) | No pre-check tx forward; relies on node re-publish | Stopped the flood; valid txs travel one hop (re-publish refused `Duplicate`) [E8]; violates INV-NETWORK-003 | Relay of judged content must keep its original identity (or the id rule must change). No relay by re-publish of received content. |
| 6.43.3 (c) EventSink (`83f9e7b1`) | Swarm never awaits event delivery | Removed the swarm half of C1; [E11] 0 loop silences | Keep. Make it a type contract. Byte-bound the backlog. |
| INC-I-114 / INC-I-137 / INC-I-142 per-topic classifiers | Swarm-side stale/seen filters per topic | Each closed one topic; the shape recurred on the next | Do not add a sixth swarm-side filter as the fix. Filters may only Ignore or Reject. |
| `HardForkSchedule` for rolling changes | — | `current_fork_id(u64::MAX)` activates all entries at once | Not needed (node-local). Never use it here. |

---

## 6. Specs drift detected

- `specs/protocol.md` (~2093) and `crates/network/src/gossip/staleness.rs:112-122`: both state that each node re-publishes the txs its mempool admits, which implies multi-hop. Reality: a re-publish of byte-identical data is refused `Duplicate` (vendor `behaviour.rs:607-616`), so propagation is one hop for the common single-tx batch [E8]. The F-1 TTL reasoning cites a 120 s libp2p cache; the configured value is 300 s (`config.rs:67`).
- `specs/protocol.md` (~2096): "an unrecognized or dynamic topic … never silently Accepted". The code Accepts `None` topics (`behaviour_events.rs:252`).
- INV-NETWORK-003 (memory.db) vs `behaviour_events.rs:260-264`: the invariant requires that a new message is forwarded once; the code never forwards txs.
- INV-NETWORK-002 assertion is tautological for the queue half (`config.rs:246`, literal `true`).
- `crates/network/tests/it/inc_i_237_tx_relay.rs` covers only "rejected → not relayed" and "B's own tx reaches C". It has no partition "B admits F's tx → C receives it", which is why [E8] passed CI.

---

## 7. Consensus and deploy answers

Three-question checklist (INC-I-075, INV-12), for a design that meets REQ-GRV-011:
1. Can a user-submittable tx reach this path? **YES** — every gossiped tx goes through the relay verdict.
2. Can a producer action or attestation pattern reach it? **YES** — blocks, headers and attestations go through the same seam.
3. Is the new behaviour bit-identical for all reachable inputs? **For every consensus-visible computation: YES.** No validation, apply, scheduler, bitfield, coinbase or state-root code changes. Only which node forwards which gossip message changes, and that is not consensus-visible. ⇒ **(1|2) YES + (3) YES → no activation height.**

Deploy questions:
- Does it change consensus **RULES**? **NO.**
- Does it change block **CONTENT** (bitfield, coinbase, tx ordering, presence_root, header fields)? **NO** by format. It can change *which* txs and attestations reach a producer in time, which is ordinary network variance. REQ-GRV-010 guards the attestation side. ⇒ **rolling restart is allowed** (no synchronized stop-all), testnet first.
- Genesis reset (#0 rule): **not needed and not allowed** for this change.

Mixed-version reality: nodes on v6.43.0–6.43.2 and old externals still forward tx batches before a verdict. The redesign cannot change them. It guarantees only that an upgraded node is a firewall: unabsorbable content from an old node reaches that node's direct peers and stops there (REQ-GRV-004).

---

## 8. Trust boundaries

| Entry point | Data | Operations that consume it | Requirement |
|---|---|---|---|
| gossipsub message on any topic (`behaviour_events.rs:54`) | raw bytes from any peer (untrusted) | decode (Block, BlockHeader, tx batch, attestation, heartbeat, vote JSON, producer protobuf/bincode/bloom), seen-cache hashing, relay verdict, node handlers | REQ-GRV-SEC-001..005 |
| tx batch decode (`publish.rs:147-216`) | count/length fields | allocation of `Vec<Transaction>` | REQ-GRV-SEC-005 |
| node verdict → swarm (new path) | message id + propagation source + verdict | `report_message_validation_result` | REQ-GRV-008, SEC-003 |
| RPC `sendTransaction` → broadcast (`startup.rs:428-436`) | user tx | command channel | REQ-GRV-007 |
| request/response (sync, status, txfetch) | responses up to 16 MiB | control backlog | REQ-GRV-007 (byte bound) |

Trust model default: all external gossip input is untrusted (unchanged from today).

---

## 9. Architecture context

### Module boundaries
- `crates/network/src/service/` (swarm task): owns gossipsub, the peers table, rate limiter, SeenCache, EventSink. Depends on `crates/network/src/gossip/`. Depended on by `bins/node` through `NetworkService` (events out, commands in). It must not depend on mempool, UTXO or chain state (crate layering).
- `crates/network/src/gossip/`: topics, config, classifiers, encoders. Pure functions + config.
- `bins/node/src/node/` (node task): event loop (`event_loop.rs`), handlers (`network_events.rs`, `validation_checks/mod.rs`, `block_handling.rs`), periodic tasks (`periodic.rs`) — the only holder of validity knowledge.
- `crates/mempool/`: admission (`add_transaction`, `add_system_transaction`).
- `crates/rpc/`: submits txs/votes through closures wired in `startup.rs:428-452`.
- `vendor/libp2p-gossipsub/` (0.46.1 + 4 MiB patch).

### Data flows through the seam
- Gossip in: peer → gossipsub (dup cache, mcache hold) → `handle_behaviour_event` → classifier → **report verdict (forward decision)** → rate limit / shed → `EventSink` / `enqueue_or_shed` → event channel → node loop → handler (validity) → [txs only] `broadcast_transaction` → command channel → swarm batcher → `publish` (new id).
- Commands out: node handlers / periodic / production / RPC → command channel → swarm → publish or request/response.

### Constraints and invariants to keep
- INV-NET-002: exactly one report per message, every topic, every path.
- INV-NETWORK-002: `validate_messages` + bounded queues while `flood_publish = true`.
- INV-NETWORK-003 (to amend): forward once, after the node's judgement.
- INV-NETWORK-004: raw-bytes seen key.
- INV-SYNC-009: all outbound sync requests through `RequestSync` with the governor.
- INV-ATTEST-002: own attestations recorded locally, never via gossip echo.
- INV-NETWORK-001: max transmit size ≥ largest block; Era 1+ uses announce-then-fetch.
- Crate layering: the network crate cannot call the mempool. A verdict must come back as data (a command) or as a pure swarm-side function.
- gossipsub 0.46.1: a late Accept after the mcache window is a silent no-forward (vendor `behaviour.rs:755-765`).

### Blast radius
- Direct: `service/behaviour_events.rs`, `service/swarm_loop.rs`, `service/event_sink.rs`, `service/backpressure.rs`, `service/types.rs`, `service/command_handling.rs`, `service/mod.rs`, `gossip/staleness.rs`, `gossip/config.rs` (assertion only), `bins/node/src/node/event_loop.rs`, `network_events.rs`, `validation_checks/mod.rs` (`handle_new_transaction`), `startup.rs` (RPC broadcast closures).
- Indirect: `periodic.rs` (31 awaited command sends), `production/mod.rs` (block/header publish), sync manager (control events), RPC `transaction.rs` / `governance.rs`, tests `crates/network/tests/it/inc_i_237_*.rs`, `gossip/staleness_tests.rs`, `service/tests.rs`.
- Graph note: `blast.py` returned 0 dependents for `behaviour_events.rs`. That is the known graphify blind spot for Rust `self.method()` edges, and the graph was built from the old root checkout. The map above comes from grep on `d0e15686`.

### Brittleness check
```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
Details: (1) cross-module blast radius — network service, gossip, node loop, mempool, rpc;
         (2) invariant gap — nothing enforces "forward only after own verdict";
         (3) data-flow reversal — the verdict must flow node→swarm, today only swarm→node;
         (5) contract absence — swarm/node no-await rule is a convention (EventSink::sender() exposes an awaitable Sender).
         (4) shared mutable state — NOT present (peers RwLock has no cross-await holder).
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 10. Open questions (I cannot ask the user; defaults used so the work is not blocked)

| # | Question | Default assumption (technical) | Plain language | Confirmed |
|---|---|---|---|---|
| Q1 | Do blocks keep a pre-verdict Accept for latency? | Yes, as the single written exception: deserialize + stale gate + raw-bytes seen dedup + rate limit moved **before** the verdict. The exception records that forged-block volume has no bound until INC-I-236. | Blocks keep moving fast. We write down why, and what still needs fixing. | No |
| Q2 | Verdict for a tx batch with mixed valid/invalid txs? | All or nothing per message: forward only if the node holds every tx in it as valid (admitted now or already resident). Otherwise Ignore. No node re-publishes received txs; only origins publish. | A bundle is passed on only if every tx in it checks out. | No |
| Q3 | Can headers and heartbeats stop being relayed? | Yes. No code in v6.43.3 consumes them and our nodes do not publish heartbeats. Assumed: no external version ≥ `MIN_PEER_PROTOCOL_VERSION` needs them relayed by us. Verify by archaeology before the change. | Stop forwarding two message kinds that nobody uses. | No |
| Q4 | Relay valid content that came from old-version (re-batching) publishers? | Yes. The verdict is content-based, not version-based. | Good txs are passed on, whoever sent them. | No |
| Q5 | What if the node verdict misses the ~5 s window? | Fail-closed: no forward, report Ignore, and count it. Do not raise `history_length` unless the measured p99 needs it. | Late means not forwarded. It does not mean stuck. | No |
| Q6 | Scale target for the bounds? | `max_peers` = 50, 1000s of producers, 10 s slots, sustained tx peak 100 tx/s, attestation peak = producer count per slot. | We size the limits for the network we plan, not only today's. | No |
| Q7 | Do attestations go through the node verdict (signature check) before relay? | Yes, the same rule as txs, provided REQ-GRV-010 holds. If it fails, attestations become a written exception. | Check an attestation before passing it on, if that stays fast enough. | No |
| Q8 | Keep 6.43.3 (b) until the redesign ships? | Yes. It is the safe interim. The one-hop regression is tolerated because origins' `flood_publish` reaches their neighbours and txs are still mined [E8]. | Keep the current hotfix until the real fix is ready. | No |
| Q9 | Is changing the message-id function allowed? | No (REQ-GRV-W08). | Do not change how messages are named on the wire. | No |

---

## 11. What I do not understand (mandatory)

1. Onset mechanics of the re-batcher ([M2]): who published the 0xBA batches at 13:20Z and why. The design does not depend on it, but REQ-GRV-004's test models it as "any publisher, any composition".
2. Whether any external binary consumes `/doli/headers/1` or `/doli/heartbeats/1` (Q3). I read only `d0e15686`.
3. Real attestation volume per slot on mainnet now and at 1000s of producers. The node already verifies each one (`network_events.rs:544`), but I did not measure the per-message cost on the event loop.
4. gossipsub 0.46.1 internals that I did not read: whether IHAVE/IWANT serving can leak not-yet-validated messages, and how the pending-validation set is bounded apart from mcache shifting. The design evaluators should read vendor `behaviour.rs` around `handle_received_message` and `mcache.rs`.
5. Block-path cost split: how long the node takes to validate a gossip block before a verdict would exist. Q1 assumes it is too slow for a pre-relay verdict; I did not measure it (block VDF iterations are 1,000 on mainnet per `network_params/defaults.rs:49`, so VDF is cheap; the rest is not measured).
6. Whether a zero-flow tx type admitted by `add_system_transaction` can be admitted again after it is mined (no inputs to spend). Under "admission = relay authority" that would allow replay relays. Owed to W01.

## 12. Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| A1 | The mainnet fleet runs `d0e15686` (v6.43.3) on all structural units | The code I read is what runs | No (per brief) |
| A2 | `tx_announce_enabled` is false on every mainnet node (`config.rs:132`, never set in `bins/node`) | The tx announce mode is not in use | Yes (code) |
| A3 | Tier1 and region topics are not subscribed on any current binary | Those topics carry no traffic | Yes (code, our binary only) |
| A4 | The swarm and node run as separate tokio tasks with the two channels as their only queues (plus the peers lock) | The two halves talk only through these pipes | Yes (code) |
| A5 | Redesign is node-local and ships in a normal rolling release | No chain reset, no stop-all | Yes, if REQ-GRV-011 holds |

## 13. Contradictions with the RUN 570 report

- RUN 570's SSF says "ONE registry of relayable instead of **two** (gossipsub first-sight vs mempool)". There are **five** registries (§1.1) and **four** post-Accept drop paths. The tx-only fix leaves the same shape live on attestations: they are forwarded before `attestation.verify()` (`network_events.rs:544`), and forged bytes defeat the raw-bytes SeenCache.
- RUN 570 defense-in-depth asks to "give attestations/**heartbeats** a budget a tx flood cannot shed". Heartbeats have no consumer (`event_loop.rs:359-361`) and our nodes never publish them (`service/mod.rs:535-538`). They should be subtracted, not prioritized.
- RUN 570 [E6a] and (c): the park is "removed on the swarm side". Confirmed, but only by convention: `EventSink::sender()` (`event_sink.rs:66`) still exposes an awaitable `Sender`. Also, the control backlog is count-bound (8,192), not byte-bound, and `SyncResponse` can be 16 MiB (`protocols/sync.rs:22`).
- RUN 570 [E8] "valid txs travel only ONE hop". Precise form: refusal happens when the re-published batch is **byte-identical** to the received message (common: one tx per 100 ms window). A re-composed batch gets a new id and propagates. Hop count therefore depends on traffic shape, and that is itself the [E4] amplifier.
- Line numbers: RUN 570 cites `service/mod.rs:569-578`. At `d0e15686` it is `:570-581` (shifted by `mod event_sink`). No substantive contradiction.

## Traceability matrix

| Requirement ID | Priority | Test IDs | Architecture section | Implementation module |
|---|---|---|---|---|
| REQ-GRV-008 (M0) | Must | network `it::inc_i_237_m0_api` (handle, topic map, labels, batch stats), `it::inc_i_237_m0_swarm`; doli-node `it::inc_i_237_m0_metrics` (observe_gossip_handled x2), `it::inc_i_237_m0_node_hook` | Migration Path M0, F-8 | GossipHandle + topic map @ crates/network/src/service/gossip_handle.rs; stamp @ crates/network/src/service/behaviour_events.rs; tx_batch_stats @ crates/network/src/gossip/tx_batch_stats.rs; metrics_gossip @ bins/node/src/metrics_gossip.rs; hook @ bins/node/src/node/event_loop.rs |
| REQ-GRV-009 (M0) | Must | doli-node `it::inc_i_237_m0_metrics::req_grv_009_*` | M0 block recv baseline | observe_block_recv_latency @ bins/node/src/metrics_gossip.rs; call @ bins/node/src/node/event_loop.rs |
| REQ-GRV-010 (M0) | Must | doli-node `it::inc_i_237_m0_metrics::req_grv_010_*` | M0 attestation coverage baseline | observe_attestation_coverage @ bins/node/src/metrics_gossip.rs; call @ bins/node/src/node/apply_block/post_commit.rs |
| REQ-GRV-012 (M0, F-10) | Must | network `it::inc_i_237_m0_corpus` (4 tests, 38 rows; PASS on v6.43.3) | F-10 golden-vector corpus | tests only (C8) — no implementation module |
| REQ-GRV-016 (M0 readers) | Should | doli-node `it::inc_i_237_m0_metrics::req_grv_016_*`, `it::inc_i_237_m0_probe` (5) | M0 reader | scrape_tx_batch_stats @ bins/node/src/metrics_gossip.rs (via /metrics handler @ bins/node/src/metrics.rs); probe @ scripts/inc-i-237-m0-latency-probe.sh |
| REQ-GRV-SEC-005 (M0) | Must | network `it::inc_i_237_m0_sec005` (4), `inc_i_237_m0_sec005_alloc` (2, own binary) | C7 | decode_tx_batch pre-allocation @ crates/network/src/gossip/publish.rs |
| REQ-GRV-003 (M1) | Must | network `it::inc_i_237_m1_relay_table` (from_topic_str + tx/attestation/vote/producer rows), `it::inc_i_237_m0_corpus`, `gossip_reforward_storm`, lib `gossip::staleness::tests` | D2 relay table | decide / relay_decision @ crates/network/src/gossip/relay_decision.rs; handle_gossip_message @ crates/network/src/service/gossip_intake.rs |
| REQ-GRV-006 (M1) | Must | network lib `service::tests::inc_i_237_m1_*` (4), `it::inc_i_237_m1_contract::req_grv_006_*` (2), `it::inc_i_237_loop_deadlock` (3) | D4 sync-only sink, F-11 | EventSink::{emit, emit_block, try_flush} @ crates/network/src/service/event_sink.rs; run_swarm @ crates/network/src/service/swarm_loop.rs |
| REQ-GRV-009 (M1) | Must | network `it::inc_i_237_m1_relay_table::req_grv_009_*` (2), `it::inc_i_237_m1_block_gate::f_1_*` | D6 / C1 / F-1 | decide (Blocks arm) @ crates/network/src/gossip/relay_decision.rs; block arm @ crates/network/src/service/gossip_intake.rs; NetworkService::memory_shed_flag @ crates/network/src/service/mod.rs |
| REQ-GRV-015 (M1) | Should | network `it::inc_i_237_m1_contract::req_grv_015_*` (2), `it::inc_i_237_m1_relay_table::req_grv_015_*`; doli-node `it::inc_i_237_m0_metrics::req_grv_015_*` | D3 unsubscribe | subscribe_to_topics @ crates/network/src/gossip/config.rs; EVENT_TOPICS @ bins/node/src/metrics_gossip.rs |
| REQ-GRV-020 (M1) | Could | network `it::inc_i_237_m1_contract::req_grv_015_network_command_has_no_header_or_heartbeat_publish` | D3 subtraction | deletions @ crates/network/src/service/{types,command_handling,mod}.rs, crates/network/src/gossip/publish.rs, bins/node/src/node/{event_loop,production/mod}.rs |
| REQ-GRV-SEC-003 (M1) | Must | network `it::inc_i_237_m1_block_gate::req_grv_sec_003_*` | D6 per-peer bucket before the report | RateLimiter::check_block_peer_only @ crates/network/src/rate_limit.rs; call @ crates/network/src/service/gossip_intake.rs |
| REQ-GRV-SEC-004 (M1) | Must | network `it::inc_i_237_m1_relay_table::{req_grv_sec_004_*, f_16_*, req_grv_009_block_rows_*}` | D2 (Reject only for undecodable blocks) | RelayDecision::acceptance @ crates/network/src/gossip/relay_decision.rs |
| REQ-GRV-006 (M1.5) | Must | network lib `service::tests::inc_i_237_m15_lanes::req_grv_006_f11_*`, `service::tests::inc_i_237_m15_budgets::inc_i_237_m1_*` (4, migrated), `it::inc_i_237_m15_contract::req_grv_006_*` (4), `service::tests::inc_i_237_m15_dev_paths::req_grv_006_*` (1), `it::inc_i_237_loop_deadlock` (non-vacuity adapted to R1) | R1 sync sink + EventLanes, F-6, F-11 | EventSink / event_lanes @ crates/network/src/service/event_sink.rs; EventLanes @ crates/network/src/service/event_lanes.rs; NetworkService (lanes, lane_stats, lane_budgets) @ crates/network/src/service/mod.rs; run_swarm(EventSink) @ crates/network/src/service/swarm_loop.rs |
| REQ-GRV-007 (M1.5) | Must | network lib `service::tests::inc_i_237_m15_{lanes,budgets}::req_grv_007_*` (6), `it::inc_i_237_m15_contract::req_grv_007_*` (2); rpc lib `methods::tests_inc_i237_busy` (5); doli-node lib `node::inc_i_237_m15_tests` (6), `node::producer_merge::tests` (1), `it::inc_i_237_m15_lane_metrics` (1); network lib `service::tests::inc_i_237_m15_dev_paths::req_grv_007_*` (1), `service::backpressure::tests` (9, migrated) | R1 byte budgets, R3 reserve-before-admit, F-13, F-15 | LaneBudgets / event_bytes @ crates/network/src/service/event_sink.rs; TxBroadcastPermit @ crates/rpc/src/methods/context.rs; RpcError::broadcast_busy @ crates/rpc/src/error.rs; reserve-before-admit @ crates/rpc/src/methods/{transaction,governance}.rs; reserve_tx_broadcast / broadcast_vote_now @ bins/node/src/node/rpc_broadcast.rs (wired @ bins/node/src/node/startup.rs); on_producer_digest / on_producers_announced / on_producer_announcements @ bins/node/src/node/network_events.rs; MergeSlot @ bins/node/src/node/producer_merge.rs; observe_event_lanes / watch_event_lanes @ bins/node/src/metrics_gossip.rs |
| REQ-GRV-017 (M1.5) | Should | network lib `service::tests::inc_i_237_m15_lanes::req_grv_017_*` (4) | R1 two lanes, C3, F-7 | lane_of / EventSink::emit @ crates/network/src/service/event_sink.rs; EventLanes::recv (biased) @ crates/network/src/service/event_lanes.rs |
| REQ-GRV-SEC-003 (M1.5) | Must | network lib `service::tests::inc_i_237_m15_budgets::req_grv_007_bulk_bytes_stay_within_budget_under_10x_flood_and_release_on_dequeue` | R1 bulk byte budget | EventSink::emit @ crates/network/src/service/event_sink.rs |
| REQ-GRV-001 (M2) | Must | network `it::inc_i_237_m2_line::req_grv_001_*` (2); doli-node `it::inc_i_237_m2_node_relay::req_grv_001_*` | D1 verdict seam | Defer @ crates/network/src/gossip/relay_decision.rs; defer @ crates/network/src/service/gossip_intake.rs; GossipVerdict arm @ crates/network/src/service/command_handling.rs; dispatch @ bins/node/src/node/event_loop.rs |
| REQ-GRV-002 / 004 (M2) | Must | network `it::inc_i_237_m2_line::{req_grv_002_*, req_grv_004_*}` | D1 / D5 | as REQ-GRV-001; Node::handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs |
| REQ-GRV-005 / F-5 (M2) | Must | doli-node `it::inc_i_237_m2_node_relay::req_grv_005_*`, `it::inc_i_237_m2_node_verdict::{d5_*, f_5_*}` | D5 strict rule | Node::handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs; handle_new_transaction (-> bool, no re-publish) @ bins/node/src/node/validation_checks/mod.rs |
| REQ-GRV-008 / F-3 / F-11 / F-13 (M2) | Must | network lib `service::tests::inc_i_237_m2_verdict` (7), `it::inc_i_237_m2_verdict_path` (4); doli-node `it::inc_i_237_m2_node_verdict::req_grv_008_*` | D1 / R5 / C6 | apply_gossip_verdict, send_gossip_verdict, LATE_VERDICTS, VERDICT_DEADLINE @ crates/network/src/service/verdict.rs; GossipHandle origin @ crates/network/src/service/gossip_handle.rs |
| REQ-GRV-011 / 012 (M2) | Must | doli-node `it::inc_i_237_m2_pins`; network `it::inc_i_237_m2_origin`, `it::inc_i_237_m0_corpus` | no wire/id/version change | decoders kept @ crates/network/src/gossip/publish.rs |
| REQ-GRV-016 (M2 reader) | Should | doli-node `it::inc_i_237_m2_late_metric` (2) | R5 late counter | doli_gossip_verdict_late_total @ bins/node/src/metrics_gossip.rs; verdict_late line @ scripts/inc-i-237-m0-latency-probe.sh |
| REQ-GRV-022 (M2) | Could | network `it::inc_i_237_m2_origin` (3) | R4 single-tx origin publish | BroadcastTransaction arm @ crates/network/src/service/command_handling.rs; batcher deleted @ crates/network/src/service/swarm_loop.rs |
| REQ-GRV-003 (M3) | Must | network `it::inc_i_237_m3_relay_table::req_grv_003_*` (2), `it::inc_i_237_m3_line` (4: attestation, vote, producer set, digest), `it::inc_i_237_m1_relay_table` (attestation/vote/producer rows), `it::inc_i_237_m0_corpus` (6 rows); doli-node `it::inc_i_237_m3_node_vote_producers` (4) | R2, per-topic table | Defer arms @ crates/network/src/gossip/relay_decision.rs; defer @ crates/network/src/service/gossip_intake.rs; node verdicts (on_new_vote, on_producer_announcements, relay_verdict) @ bins/node/src/node/gossip_verdicts.rs; dispatch @ bins/node/src/node/event_loop.rs |
| REQ-GRV-010 / 017 / F-7 (M3) | Must | network `it::inc_i_237_m3_flood` (2) | R1 + R2, F-7 | consensus-lane attestation Defer, within_peer_budget (request bucket, never tx) @ crates/network/src/service/gossip_intake.rs |
| REQ-GRV-014 (M3) | Should | network `it::inc_i_237_m3_relay_table::req_grv_014_*` | one relay authority | RelayDecision (no AcceptTransitional) @ crates/network/src/gossip/relay_decision.rs |
| REQ-GRV-SEC-001 / 002 / F-9 (M3) | Must | network `it::inc_i_237_m3_relay_table::req_grv_sec_002_*`; doli-node `it::inc_i_237_m3_node_attest` (4) | R2 membership verdict, ATTEST_FETCH gate + per-tick cap | on_new_attestation @ bins/node/src/node/gossip_verdicts.rs; maybe_fetch_attested_block @ bins/node/src/node/network_events.rs; ATTEST_FETCH tick (FETCH_CAP_PER_TICK) @ bins/node/src/node/periodic.rs |
| REQ-GRV-005 (M3 re-publish) | Must | doli-node `it::inc_i_237_m3_node_republish` (2) | one identity per message | gossip-timer digest (digest_due) @ bins/node/src/node/event_loop.rs; DirectAttestation arm @ bins/node/src/node/validation_checks/mod.rs; own-block attestation (attest_own_block removed) @ bins/node/src/node/production/mod.rs |
| REQ-GRV-SEC-001 / 002 (M2) | Must | doli-node `it::inc_i_237_m2_node_verdict::req_grv_sec_*`, `it::inc_i_237_m2_node_relay::req_grv_sec_001_002_*` | D5 / F-5 | Node::handle_gossip_txs @ bins/node/src/node/validation_checks/gossip_txs.rs |
| W01 dependency (M2) | Must | doli-node `it::inc_i_237_m2_node_verdict::w01_*`, lib `node::inc_i_237_slash_tests::test_handle_new_transaction_does_not_admit_gossiped_slash` | D5 | existing mempool refusal, verdict Ignore @ bins/node/src/node/validation_checks/gossip_txs.rs |
| REQ-GRV-013 (M4: R6, INV-NETWORK-002/004) | Must | network `it::inc_i_237_m4_source_pins::req_grv_013_*` (3), `it::inc_i_237_m4_vendor_dedup` (6: constants, in-window dup once, 4 `inv_network_004_*`), `it::inc_i_237_m4_lane_bounds` (2) | R6 SeenCache deletion; queue half from `LaneBudgets::is_bounded` | R6 deletion @ crates/network/src/gossip/{staleness,relay_decision}.rs; const assert @ crates/network/src/gossip/config.rs; LaneBudgets::is_bounded @ crates/network/src/service/event_sink.rs; assert_production_hardening @ crates/network/src/gossip/config.rs (caller NetworkService::new @ crates/network/src/service/mod.rs) |
| REQ-GRV-016 (M4 channel metrics) | Should | network lib `service::tests::inc_i_237_m4_lane_depth` (3); doli-node `it::inc_i_237_m4_metric_readers` (4), `it::inc_i_237_m4_lane_depth_api` (2) | lane depth, shed by topic, probe `channel` section | LaneStats depth + shed_by_topic, TopicCounts @ crates/network/src/service/event_sink.rs; EventLanes::take @ crates/network/src/service/event_lanes.rs; observe_event_lanes (doli_event_lane_depth, doli_event_shed_by_topic_total) @ bins/node/src/metrics_gossip.rs; channel lines @ scripts/inc-i-237-m0-latency-probe.sh |
| REQ-GRV-018 / 019 (M4 docs) | Should | network `it::inc_i_237_m4_doc_drift` (8), `it::inc_i_237_m4_source_pins::req_grv_018_*` (1) | Non-Foreclosure Check; protocol/security_model/troubleshooting/architecture drift | Non-Foreclosure Check @ specs/gossip-relay-verdict-architecture.md; status @ specs/gossip-relay-verdict-status.md; drift @ specs/protocol.md §7.2/§7.5, specs/security_model.md §6.1.3a, docs/architecture.md, docs/troubleshooting.md §4.5/§4.7, docs/rpc_reference.md |
| REQ-GRV-020 / 021 (M4 cleanup) | Could | network `it::inc_i_237_m4_source_pins::req_grv_020_*` (2), `req_grv_021_*` (1) | caller-less seam fns, one gossipsub builder, 500-line budget | deletions @ crates/network/src/service/{mod,types,command_handling}.rs, crates/network/src/gossip/publish.rs; gossipsub_config + build_gossipsub @ crates/network/src/gossip/config.rs; split → crates/network/src/service/{command_api,event_size}.rs |
| REQ-GRV-001 … 013 | Must | (test-writer) | (architect) | (developer) |
| REQ-GRV-SEC-001 … 005 | Must | (test-writer) | (architect) | (developer) |
| REQ-GRV-014 … 019 | Should | (test-writer) | (architect) | (developer) |
| REQ-GRV-020 … 022 | Could | (test-writer) | (architect) | (developer) |
| REQ-GRV-W01 … W08 | Won't | n/a | n/a | n/a |
