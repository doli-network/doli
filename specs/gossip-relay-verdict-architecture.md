━━━ FINDINGS — 12 total (DECISION:12) ━━━

  [F1] DECISION conf(0.85, converged) — crates/network/src/service/behaviour_events.rs:255-269 + bins/node/src/node/validation_checks/mod.rs:1027-1030 — replace both tx relay mechanisms (6.43.3 (b) Ignore override + node re-publish) with ONE node verdict try-sent back on the ORIGINAL message id; the vendor mcache is the pending store
  [F2] DECISION conf(0.90, converged) — crates/network/src/service/behaviour_events.rs:252 — the swarm can never Accept a non-block topic: one exhaustive per-topic table (Reject | Ignore | AcceptNow(blocks) | Defer); `None`, undecodable non-block bytes, 0xAA announce, legacy bincode, Tier1/region → Ignore
  [F3] DECISION conf(0.85, converged) — crates/network/src/gossip/config.rs:418-424 + bins/node/src/node/production/mod.rs:653 — unsubscribe `/doli/headers/1` and `/doli/heartbeats/1`, delete the header publish and both topics' plumbing (precondition: F-10 tag sweep)
  [F4] DECISION conf(0.90, converged) — crates/network/src/service/event_sink.rs:66 — delete `EventSink::sender()` and the `reserve()` select arm; the swarm→node sink is synchronous only (no awaitable handle, by type)
  [F5] DECISION conf(0.85, converged) — crates/mempool/src/pool.rs:483-487 + crates/core/src/transaction/core.rs:491-497 — tx verdict = Accept iff EVERY tx in the message was NEWLY admitted by this message; `AlreadyExists`/resident never authorizes a forward; state-dependent failure → Ignore, never Reject
  [F6] DECISION conf(0.85, converged) — crates/network/src/service/behaviour_events.rs:104-170 + crates/network/src/rate_limit.rs:209-257 — blocks keep the ONE written pre-verdict Accept; only the per-peer bucket moves before the report; the global bucket and the memory-watchdog shed stay after it (F-1); no (slot, producer) dedup before INC-I-236
  [F7] DECISION conf(0.75, converged) — crates/network/src/service/event_sink.rs:40-52 — two bounded lanes (consensus: blocks + attestations + control; bulk: txs/votes/producers), biased select, per-lane bytes-in-flight bound; ships BEFORE any attestation verdict
  [F8] DECISION conf(0.75, converged) — crates/core/src/attestation/message.rs:103-107 + bins/node/src/node/network_events.rs:542-557 — attestation verdict = decode + Ed25519 + producer-set membership (incl. pending activation), independent of block knowledge and BLS; ATTEST_FETCH gated by the same check and capped per tick
  [F9] DECISION conf(0.72, converged) — bins/node/src/node/startup.rs:431-452 + bins/node/src/node/network_events.rs:477-500 — node→swarm: must-deliver command sends keep their awaits; verdicts are try_send; RPC tx/vote reserve a command slot BEFORE mempool admission (Busy on Full, no spawn-per-item); digest reply try_send, digest relay Ignore
  [F10] DECISION conf(0.70, converged) — crates/network/src/service/swarm_loop.rs:191-238 + crates/network/src/gossip/publish.rs:131-135 — own-origin txs publish one tx per message in the EXISTING legacy single-tx encoding; delete the 100 ms / 50-tx batcher and the announce flush (scale ceiling recorded)
  [F11] DECISION conf(0.80, converged) — vendor/libp2p-gossipsub/src/behaviour.rs:755-768,805-807 — late-verdict contract: 3.5 s deadline at the swarm verdict arm, late = counted and NOT reported; every swarm-side drop reports Ignore at its drop site; INV-NET-002 amended to "at most one report; mcache expiry ≡ Ignore"
  [F12] DECISION conf(0.75, converged) — crates/network/src/gossip/staleness.rs:93-122 + vendor/libp2p-gossipsub/src/behaviour.rs:1718-1725 — delete the app SeenCache (dominated by the 300 s duplicate cache, inserted before app delivery); const-assert `GOSSIP_DUPLICATE_CACHE_TIME >=` the largest slot-bound age window

  Speculative: 12 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Gossip Relay Verdict + Swarm/Node Channel Contract Architecture

> INC-I-237 · PROPOSAL-ONLY (no source change) · 2026-10-11 · design-synthesizer over 5 evaluators (subtraction, restructure, patterns, failures, radical) · code read = wt-main v6.43.3 `d0e15686` (what mainnet runs). Every conflicting claim in this spec was re-read in that tree; `file:line` cites are against it. Reasoning trace: `docs/.workflow/architecture-reasoning.md`. Analyst input: `docs/redesigns/gossip-relay-verdict-redesign-analysis.md` (REQ-GRV-001..022, SEC-001..005, W01..W08).

## Problem Statement

The app-layer gossip handler in the **swarm task** reports `Accept` (= forward) to libp2p-gossipsub BEFORE the **node task** has judged the bytes. gossipsub then re-forwards content no peer can absorb; with a content-only message id (`BLAKE3(data)[..20]`, `gossip/config.rs:199-202`) every re-batch is a new id and is relayed again; bytes pile up in count-bounded queues → OOM. Co-root: swarm and node are joined by two bounded channels on which each side could `.send().await` the other → mutual park (8 permanent freezes on mainnet, v6.43.0).

**Recurrence record (same shape, 5 occurrences):** INC-I-114 (blocks: `validate_messages=false` + 60 s dedup → forever re-forward), INC-I-120 (sync: zero outbound governance), INC-I-137 (producers: unconditional Accept before decode), INC-I-142 (attestations/heartbeats/headers/votes/txs: Accept-by-default), INC-I-237 (txs: re-batch amplifier). Each fix added one more **swarm-side** filter; the next topic recurred. The v6.43.x hotfixes are mitigations: 6.43.1 (300 s dedup) FAILED, 6.43.2 (4 MiB send_queue) fixed memory only, 6.43.3 (b) stopped the flood but valid txs now travel ONE hop (re-publish of identical bytes is refused `Duplicate`, vendor `behaviour.rs:607-616`; violates INV-NETWORK-003), 6.43.3 (c) removed the swarm-side await **by convention only** (`event_sink.rs:66` still hands out an awaitable `Sender`).

**Goal:** relayability becomes ONE node-verdict decision for every topic (blocks = the single written exception), and swarm↔node back-pressure becomes acyclic **by type**. Node-local, no consensus change, no activation height, rolling-deploy safe with external producers on mixed versions, block propagation not worse, designed for 1000s of producers in 10 s slots.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 try_send + 1 report per deferred gossip message (~us); -1 encode+sign+publish per relayed admitted tx; -1 BLAKE3 per non-block message (SeenCache); +1 envelope verify per received single-tx message instead of per batch (<= ~4% of one core at 100 tx/s x 8 mesh copies, inferred) (inferred)
  Memory:   +mcache retention of to-be-dropped non-block messages until the verdict, <= inbound rate x 5 s (~10 MB at a 2 MB/s flood, per-peer rate-limited); +~80 B handle per in-flight deferred event; -SeenCache (<= 2 MB); control backlog <= byte budget (64 MiB) instead of 8,192 x 16 MiB; RPC path <= command capacity instead of unbounded spawned tasks (inferred)
  IO:       0 (observed)
  Network:  valid txs relayed multi-hop again at mesh D (vs 0-or-~50 re-publish); unabsorbable content 0 forwards past the first upgraded hop; -1 header publish per produced block; +~130-150 B envelope per own-origin tx only when a burst would have batched (inferred)
  Disk:     -WARN streams ("Not publishing a message that has already been published"; vendor late-report WARNs avoided by the deadline gate) (observed)
  Latency:  tx/attestation/vote/producer hop latency = RTT + node-loop time per hop (UNMEASURED, hard cap 3.5 s, M0 gate); blocks 0 (no new await on the block path; block publish no longer queued behind a header publish) (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: keep 6.43.3 (b) forever — zero code, but valid txs stay at one hop (INV-NETWORK-003 violated) and five topics still forward forged or undecodable bytes before any verdict
Why this proposal anyway: it is the only composition the five lenses found that restores multi-hop relay of valid content AND stops unabsorbable content at hop 1 with ONE authority, using the vendor mechanism already configured (`validate_messages`), with no id, topic or wire change
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## SSF Gate (Step 1.5) — PRESENT RADICAL ALONE FIRST

| Candidate | Confidence | Basis |
|---|---|---|
| **Radical minimum** (evaluator #5 as written: gossipsub-native deferred verdict for every non-block topic, blocks = exception, headers/heartbeats never relayed, sync-only sink, strict tx rule, single-tx origin publish, SeenCache deleted; 0 new modules/tasks/channels) | conf(0.65, converged) | 5/5 on the shape; as written it FAILS F-1 (moves the global bucket and watchdog before the block report), F-7 (half-channel shed gives no dequeue priority), F-12 (drops control events) and F-15 (RPC `try_send` strands an admitted tx) |
| **Proposed** (= the radical minimum amended ONLY by the filter-forced fixes: per-peer-only reorder on blocks, two bounded lanes, RPC reserve-before-admit, byte-bounded backlog) | conf(0.74, converged) | the same 5/5 shape + each filter failure closed by the smallest fix any evaluator proposed (each <= ~30 lines) |

Gap = 0.09 <= 0.1 → **PRESENT RADICAL ALONE FIRST.** The Definite + Recommended sections below ARE the radical minimum plus the four filter-forced amendments. Every other item from any evaluator is an OPTION (Section "Options for User Decision"), not part of the proposal.

## Evaluation Summary

| Evaluator | Lens | Top proposal | Confidence | Key finding |
|---|---|---|---|---|
| Subtractionist | removal | delete the swarm's power to Accept non-blocks; verdict on the original id; unsubscribe 2 topics; delete SeenCache + `sender()`; ~-900 lines | conf(0.6, observed) | vendor dup cache inserts BEFORE app delivery (`behaviour.rs:1718`) → SeenCache hit rate 0; batcher removal killed (K1: envelope verify per received copy) |
| Restructurer | boundaries | opaque `GossipRef` out, `RelayVerdict` back, one node Accept site; two byte-budgeted lanes; swarm = sink (out-degree 0) | conf(0.6, inferred) | 35 awaited sends (not 31) + a 3rd spawn-per-item producer (`on_producer_digest`); keep watchdog shed AFTER the block report |
| Pattern Matcher | patterns | Lighthouse extended-validator (`NetworkMessage::ValidationResult` → `report_message_validation_result`); one object per message; wtxid rule | conf(0.65, observed) | Q2 default "already resident" (keyed on signature-excluding `tx.hash()`) is an unbounded malleation amplifier |
| Failure Analyst | failures | F-1..F-16 hard filters; blocks must NEVER fail closed; attestations only behind a lane | conf(0.65, observed) | headers drain the GLOBAL block bucket (C-3); `verify()` is self-certifying; every verdict delay is fleet-synchronous |
| Radical Simplifier | minimal | gossipsub-native deferred validation, 0 new modules/tasks/channels/registries; mcache IS the pending store | conf(0.6, observed) | any re-publish of received content (same, semantic or envelope id) is DEAD (V1); R-a..R-e killed on vendor code |

## Convergence Matrix (from the 5 independent conclusions, verified against the full reports)

| Change | Sub | Restr | Patt | Fail | Rad | Score | Independence |
|---|---|---|---|---|---|---|---|
| Node verdict on the ORIGINAL id via try_send; mcache = pending store | Y | Y | Y | Y (cond.) | Y | 5/5 | YES — 4 read vendor `mcache.rs`/`behaviour.rs` independently; Patterns from Lighthouse source |
| Delete (b) override + node re-publish (same commit) | Y | Y | Y | Y (Q8) | Y | 5/5 | YES — [E8] log + vendor `:607-616` + `validation_checks/mod.rs:1029` |
| Blocks = the one written pre-verdict exception | Y | Y | Y (temporary) | Y | Y | 5/5 | YES — REQ-GRV-009 latency (Rad R-e), orphan chase (Fail i-1) |
| `None -> Accept` → Ignore; exhaustive table, no wildcard | Y | Y | Y | Y | Y | 5/5 | YES |
| Unsubscribe headers + heartbeats; stop header publish | Y | Y (publish 1 release) | Y | Y | Y | 5/5 | YES — git archaeology (Sub, Rad), C-3 bucket drain (Fail), AP-10 (Patt) |
| Delete `EventSink::sender()` / `reserve()` arm | Y | Y | Y | Y (F-6) | Y | 5/5 | YES |
| Tx verdict: every tx NEWLY admitted by this message | Y | partial (≥1 new) | byte-equal variant | Y (F-5) | Y | 4/5 strict | YES — `core.rs:491-497`, `pool.rs:483-487`, REQ-GRV-005 |
| Attestation verdict needs producer-set membership | Y | Y | Y | Y | Y | 5/5 | YES — `message.rs:103-107` |
| Two lanes (tx flood cannot shed/delay attestations) | gap noted | Y | Y | Y (F-7) | Y (half-shed) | 4/5 | YES |
| Keep node-side awaited command sends; verdict try_send | Y (K5) | Y | Y | Y (F-6) | Y | 5/5 | YES |
| One tx per message (own origin, legacy encoding) | NO (K1) | conditional | Y | prefer | Y | 3/5 | conflict C5 |
| Delete SeenCache | Y | keeps | — | — | Y | 2/5 | YES — dominance code-verified |
| Rate limit before block report | per-peer | per-peer only | Y | NO (F-1) | Y | conflict C1 | resolved by code (below) |
| Late verdict: skip report vs explicit Ignore | skip (4 s) | skip (3.5 s) | count expiry | explicit (F-3) | skip (4 s) | conflict C6 | resolved by vendor code (below) |

## Target Architecture

### Per-topic relay authority table (exhaustive match, no wildcard — REQ-GRV-003/014)

| Topic | Subscribed | Swarm pre-filter (may ONLY Ignore/Reject) | Relay authority | Pending store |
|---|---|---|---|---|
| `/doli/blocks/1` | yes | `Block::deserialize` (Reject), stale > 6 slots (Ignore), **per-peer** block bucket (Ignore) — all BEFORE the report | **swarm — the ONE written exception (AcceptNow)**; global bucket + memory-watchdog shed stay AFTER the report (local drops only, F-1); forged-block volume unbounded until INC-I-236 (W04) | none |
| `/doli/txs/1` | yes | decode bound (SEC-005), per-peer tx bucket (drop ⇒ Ignore at the drop site); 0xAA announce → Ignore | **node**: Accept iff every tx NEWLY admitted by this message (D5) | vendor mcache ≤ 5 s |
| `/doli/attestations/1` | yes | slot age 12 (Ignore), per-peer request bucket (never the tx bucket; drop ⇒ Ignore at the drop site) | **node** (M3): decode + Ed25519 + `producer_set.get_by_pubkey(attester).is_some()` (incl. pending activation, the set `ingest_attestation` admits); NOT block-known; NOT BLS; consensus lane | vendor mcache ≤ 5 s |
| `/doli/votes/1` | yes | timestamp age (Ignore), per-peer request bucket | **node** (M3): decode + `vote_msg.verify(&vote_msg.producer_id)` (self-certifying) + `producer_set.get_by_pubkey(pk).is_some_and(is_active)` — the set the updater tally counts (`run.rs` `is_producer_fn`, X6 resolved); bulk lane | vendor mcache ≤ 5 s |
| `/doli/producers/1` | yes | age 3600 s (Ignore), per-peer bucket; legacy bincode → Ignore | **node** (M3): announcement batch → Accept iff GSet merge added ≥ 1 verified entry (verdict sent from the spawned merge; single-flight drop ⇒ Ignore); bloom digest → swarm **LocalOnly** (Ignore, still processed; relaying it multi-hop sends the delta to the wrong peer, `behaviour_events.rs:328` `peer_id: propagation_source`). The reply is a `SendProducerDelta` try_send whose delta is computed for the forwarder's bloom but is PUBLISHED to all producer-topic peers (`command_handling.rs` `SendProducerDelta` arm) — "to the forwarder" holds by content only (G3) | vendor mcache ≤ 5 s |
| headers, heartbeats, Tier1, region, unknown | **no** | never delivered (vendor dispatches only `mesh.contains_key(topic)`, `behaviour.rs:1747-1761`); handler fallback = Ignore | none | — |

### Verdict seam (D1)

```
swarm (one fn, exhaustive):  relay_decision(topic, bytes, ctx) -> Reject | Ignore | AcceptNow(block) | Defer(decoded)
  Defer: per-peer rate check -> emit(Event{payload, GossipHandle{topic, msg_id, source, recv_at}}) ; any swarm-side drop -> report Ignore NOW
node:  existing handlers return Verdict{Accept|Ignore} instead of (); command_tx.try_send(GossipVerdict{handle, verdict})  (never awaits)
swarm command arm:  if handle.recv_at.elapsed() > 3.5 s -> late_counter[topic]++ (NO report) else report_message_validation_result(id, src, verdict)
```
No new module, task, channel or registry: the vendor mcache already holds unvalidated messages un-forwarded (`mcache.rs:123-124,166-167`), evicts them after `history_length(5)` heartbeats of 1 s (`mcache.rs:186-206`), and treats a late report as a logged no-op (`behaviour.rs:755-768,805-807`). Node→swarm verdict = data on the existing command channel.

### Channel contract — wait-for graph

```
v6.43.0:  node --cmd full (await)--> swarm --event full (send.await)--> node            CYCLE C1 (8 freezes)
v6.43.3:  node --cmd full (await)--> swarm --reserve() in select arm--> non-blocking    acyclic BY CONVENTION (sender() leaks)
proposed: node --cmd slot (await, must-deliver)--> swarm   [swarm drains unconditionally; handle_command has 0 inner awaits]
          node --GossipVerdict try_send--> swarm           [never waits]
          RPC/digest --try_reserve / try_send--> swarm     [Busy surfaced; 0 spawned tasks]
          swarm --EventSink::emit (sync) / emit_block / try_flush--> node   [no async method, no Sender accessor: out-degree 0]
          ⇒ a cycle is unrepresentable; the only shared awaitable left is the peers tokio RwLock (option O6)
```

## Resolved Conflicts C1–C7 (evidence re-read in wt-main)

- **C1 — block-path rate limit / watchdog placement.** Code: the block arm reports first (`behaviour_events.rs:104-107`), then the watchdog shed (`:130-138`), then rate limits (`:157-170`); candidate-next blocks (slot ≥ best) are gated by the GLOBAL bucket only (`check_block_global_only`, `rate_limit.rs:232-243`), non-candidate blocks by per-peer + global (`check_block`, `:209-230`); the HEADERS arm calls the same `check_block`/`record_block` (`behaviour_events.rs:434-438`), so headers DO drain the global 100/min bucket (`rate_limit.rs:184`). **Decision:** move ONLY the per-peer bucket before the report (attacker-attributable, applies only to non-candidate blocks); the global bucket and the watchdog shed stay AFTER the report as the two written post-Accept local drops on the block exception (F-1: a shared budget or fleet-wide memory pressure must never turn into a network-wide no-forward of the canonical block). C-3 is fixed by D3 (no header can reach the block bucket), not by reordering. Departs from REQ-GRV-SEC-003 for blocks only, written into the exception text. conf(0.85, converged: Restr + Fail explicit; Sub per-peer; Rad/Patt overruled on the global/watchdog half by F-1).
- **C2 — Q2 semantics.** `handle_new_transaction` early-returns on `mempool.contains(tx.hash())` (`validation_checks/mod.rs:982-986`), `add_transaction` returns `AlreadyExists` before any check (`pool.rs:483-487`), `tx.hash()` excludes input signatures (`core.rs:491-497`). **Decision:** strict rule — Accept iff every tx was NEWLY admitted by THIS message (4/5 + F-5). The Patterns byte-equal variant is rejected as the authorizing rule: an all-resident byte-equal re-batch of K valid txs under M compositions would be forwarded M times (REQ-GRV-005 / [E4] with valid content); byte-equality survives only as the node-side twin of INV-NETWORK-004 (C-PAT-1: residency keyed on `tx.hash()` never authorizes anything). Restructurer's "all valid AND ≥ 1 new" is rejected: one fee-paying fresh tx would carry up to 1 MiB of resident txs per forward. F-15 interplay: a chained child arriving before its parent is Ignored at that hop; mitigation = origin-side (options O8) or documented/RPC-visible, never a relay-side retry. conf(0.85, converged).
- **C3 — channel byte bound.** F-7 needs dequeue priority, which a single channel with "shed bulk at half full" (Rad) cannot give; F-12 forbids silent control drops, which Restr's 64 MiB byte-permit lanes and Rad's byte backlog both do (counted). **Decision:** two bounded `mpsc` lanes (consensus = blocks + attestations + control; bulk = txs, votes, producers) with a `biased` node select and a per-lane `AtomicUsize` bytes-in-flight bound (Rad's ~25-line fix applied per lane) — the simplest mechanism that passes F-7; Restr's permit-in-event semaphore (O10) is the heavier alternative. F-12 remains the one filter NO evaluator's proposal passes as written → recorded as a constraint with the smallest fix found (O12); control drops must at least stay counted and readable. conf(0.75, converged).
- **C4 — node→swarm direction.** `handle_command` has 0 inner awaits and the swarm drains `command_rx` in an unguarded arm (Restr, grep-verified), so the 35 node-side awaits are back-pressure edges, not cycle edges once the swarm cannot await the node (D4). **Decision:** keep all 35 (K5, v-1: `try_send` would drop own-block publish and `RequestSync`); only `GossipVerdict` is `try_send`. Spawn-per-item: RPC tx/vote (`startup.rs:431-452`, verified) → `command_tx.try_reserve_owned()` BEFORE mempool admission, `Busy` on Full (F-15: no admitted tx is ever stranded without a client-visible error), zero spawned tasks, no forwarder task/queue (Restr's extra queue rejected as not needed once the permit is taken synchronously); gossip-driven `on_producer_digest` (`network_events.rs:477-500`, verified spawn + awaited send) → `try_send`, drop on Full (the peer re-sends its digest). Shutdown: closing the swarm closes its receivers (F-11). conf(0.72, converged; Busy client-compat unverified).
- **C5 — own-origin single-tx publish.** `decode_tx_gossip` falls through to `Transaction::deserialize` for any non-0xAA/0xBA first byte (`publish.rs:131-135`) — the legacy single-tx encoding is the OLDEST format, decodable by every version, so the F-10 census is a precondition only for the decode direction (old formats into new nodes), not for this encode change. Subtractionist K1 is correct that the vendor codec verifies the envelope signature per received copy (`vendor/protocol.rs:299`, `ValidationMode::Strict`, `config.rs:209`): single-tx raises verify count by the batch factor. At the Q6 target (100 tx/s, ~8 copies/node) that is ≤ ~4% of one core — a recorded scale ceiling, not a kill. **Decision:** one tx per message for own-origin publishes (3/5 + Fail "prefer"); correctness wins at today's rates (a resident sibling inside an own batch would Ignore the whole batch at receivers under D5; a 6.43.3 neighbour re-publishes legacy bytes X as a 0xBA batch Y ≠ X, so the tx keeps moving through old nodes); re-batching above ~1,000 tx/s is the announce/fetch evolution path (REQ-GRV-018(4)), never a relay-side composition. M0 measures the origin rate (Restr's condition). conf(0.70, converged).
- **C6 — late verdict.** Vendor: a late Accept → `warn!("Message not in cache")` + `Ok(false)` (`behaviour.rs:759-768`); a late Ignore/Reject → `warn!("Rejected message not in cache")` + `Ok(false)` (`:805-807`); un-reported expiry → `debug!` only (`mcache.rs:189-196`). So an explicit late Ignore (F-3 literal) is a no-op that WARNs per message (a WARN per message is unbounded log growth at flood rates). **Decision:** deadline 3.5 s at the swarm verdict arm (the swarm owns `recv_at`; min mcache lifetime ≈ 4 s), late = per-topic counter, NO report; every swarm-side drop site (rate, shed, pre-filter Ignore) reports Ignore immediately — this is where F-3 bites and it is preserved; INV-NET-002 amended to "at most one report per message; a report is never issued after the window; mcache expiry ≡ Ignore and is counted". conf(0.80, converged).
- **C7 — block exception lifetime.** The exception's SHAPE is permanent: a node verdict for blocks needs `apply_block` and is dead on REQ-GRV-009 (Rad R-e) and kills orphan chase / fork relay (Fail i-1). Its BOUND is temporary: until INC-I-236 signs headers there is no forgery bound (W04); after it, the swarm pre-filter gains a signature check and may adopt "first valid-signature block per (slot, producer)" (Patt). Until then NO (slot, producer) dedup (F-16, REQ-GRV-SEC-002: a forged variant would suppress the genuine block). Future-slot Ignore = option O1 (wall-clock state can veto a candidate block under skew; F-1 caution). conf(0.85, converged).

## Definite Changes (High Convergence)

- ARCHITECTURAL: D1 — the node becomes the relay authority: every non-block gossip message is Deferred (no swarm report) and carried to the node as ONE event per gossip message with a `GossipHandle{topic, msg_id, source, recv_at}`; the node's existing handler returns a verdict that a `try_send` `GossipVerdict` command carries back to `report_message_validation_result` on the ORIGINAL id; the 6.43.3 (b) override (`behaviour_events.rs:255-264`), the node re-publish (`validation_checks/mod.rs:1027-1030`) and the per-tx fan-out (`:299-302`) are deleted in the SAME commit.
    Convergence: 5/5 (Sub P3, Restr P1, Patt P1, Fail (i)+P5, Rad P1)
    Evidence: vendor `mcache.rs:123-124,166-167,186-206`; `behaviour.rs:607-616,755-768`; `types.rs:103-160` (0 verdict commands); [E8] production node log (RUN 570 evidence)
    Confidence: conf(0.85, converged)
    Seam eliminated: 5 relay-decision sites → 2 (block exception arm + verdict arm); 2 tx relay mechanisms → 1; 0 new modules/tasks/channels/registries.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 try_send + 1 report call per deferred message; -1 encode+ed25519 sign+publish per relayed admitted tx (inferred)
      Memory:   +mcache retention of to-be-dropped messages until the verdict, <= inbound x 5 s (~10 MB at 2 MB/s, per-peer rate-limited); +~80 B per in-flight handle (inferred)
      IO:       0 (observed)
      Network:  +multi-hop forwarding of valid txs restored at mesh D; -all re-batch amplification through upgraded nodes (inferred)
      Disk:     -24 h stream of duplicate-publish WARNs (observed)
      Latency:  +node-loop time per hop on tx relay (UNMEASURED, M0 gate, cap 3.5 s); blocks 0 (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep 6.43.3 (b) with no verdict path — zero new code, valid txs stay at one hop
    Why this proposal anyway: it is the only mechanism that gives multi-hop relay with no amplification and no id or wire change, using vendor code already configured
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: D2 — the swarm loses the power to Accept any non-block topic: ONE exhaustive per-topic table (`Reject | Ignore | AcceptNow(blocks) | Defer`) replaces the two report sites and `classify_gossip`; `None → Accept` (`behaviour_events.rs:252`) becomes Ignore; undecodable non-block bytes → Ignore (not Reject) during mixed versions (F-4, F-10); 0xAA announce, legacy bincode producer list, Tier1/region block branches → Ignore.
    Convergence: 5/5 (Sub R2/R3/R8/R9, Restr prefilter, Patt table, Fail (ii)-as-prefilter, Rad relay_decision)
    Evidence: `behaviour_events.rs:252`; `staleness.rs:264,295,328,366,374` (fail-open Accept); vendor `behaviour.rs:1747-1761` (unsubscribed never delivered)
    Confidence: conf(0.90, converged)
    Abstraction removed: 9 classifiers → 7 Ignore/Reject/Defer-only pre-filters; "Accept by default" unrepresentable.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: change only the `None` arm to Ignore and leave the two report sites and the fail-open classifiers in place
    Why this proposal anyway: every remaining swarm-side Accept path is a case a sixth topic can reuse; the exhaustive table makes a forgotten verdict a compile error, not a silent fail-open
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: D3 — unsubscribe `/doli/headers/1` and `/doli/heartbeats/1`, delete the header publish before the block (`production/mod.rs:653`), `BroadcastHeader`/`BroadcastHeartbeat`, `NewHeader`/`NewHeartbeat`, their classifiers, dispatch and rate-limit arms. Precondition: the F-10 tag sweep (`git show <tag>:event_loop.rs` for every release tag since genesis 2026-07-22; v6.25.0 already a no-op).
    Convergence: 5/5 (Sub P1, Restr Q3, Patt P5, Fail P2, Rad P3)
    Evidence: `event_loop.rs:285-287,359-361` (debug/no-op consumers); `service/mod.rs:528-538` (no heartbeat publisher); `behaviour_events.rs:434-438` + `rate_limit.rs:209-257` (headers drain the global block bucket, C-3); last consumer change `ced27b92` 2026-03-14 < genesis
    Confidence: conf(0.85, converged)
    Seam eliminated: 2 relay-without-consumer topics → 0; the header↔block shared budget coupling is gone.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -1 decode+classify per header/heartbeat message (observed)
      Memory:   -header/heartbeat events in the event channel; -2 topics' mcache/dup-cache entries (observed)
      IO:       0 (observed)
      Network:  -1 header publish per produced block x fan-out; -all relayed header/heartbeat bytes from upgraded nodes (observed)
      Disk:     0 (observed)
      Latency:  -one command queued ahead of BroadcastBlock on own-block publish (observed, production/mod.rs:653-654)
    Inevitability: AVOIDABLE
    Cheaper alternative: stay subscribed and report Ignore for both topics (two classifier lines) — saves the relay bytes but leaves headers consuming the global block bucket from direct peers
    Why this proposal anyway: only unsubscribing removes the C-3 chain-level block-drop vector and the inbound bytes; Ignore-only does neither
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: D4 — the swarm→node sink becomes synchronous by type: delete `EventSink::sender()` (`event_sink.rs:66`), the `reserve()` select arm (`swarm_loop.rs:184`) and `backpressure::enqueue_or_shed`'s raw-sender use; public API = `emit(ev) -> Emitted|Shed`, `emit_block(..)`, `try_flush()` (sync `try_reserve` drain at the top of each loop iteration + the 100 ms tick); a unit test asserts the type has no `async fn` and no `Sender` getter.
    Convergence: 5/5 (Sub P4, Restr P2 type part, Patt P3, Fail F-6, Rad P4)
    Evidence: `event_sink.rs:66` (awaitable accessor), callers `behaviour_events.rs:189,214,225`, `swarm_loop.rs:184`; [E6a] 8 freezes from exactly one such await
    Confidence: conf(0.90, converged)
    Contract made structural: await-cycles 0 by convention → 0 by type (REQ-GRV-006).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 try_reserve attempt per swarm loop iteration only while a backlog exists (inferred)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  +<= 100 ms on backlogged control events while the swarm is idle (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep `sender()` and add a CI grep forbidding `.send(`/`.reserve(` outside the select arm
    Why this proposal anyway: a grep is a convention one edit bypasses; removing the accessor makes the mutual park unrepresentable
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: D5 — tx relay verdict = Accept iff EVERY tx in the message was NEWLY admitted by this message (`Admission::New` for all); any `Resident`/`AlreadyExists` or `Refused` → Ignore; undecodable → Reject only if provably no known format; state-dependent failure never Reject. Zero-flow types that the block validator can never include (SlashProducer) must stay refused at admission (W01 dependency).
    Convergence: 4/5 strict (Sub Q2, Fail P3/F-5, Rad P2, Restr partial) + F-5 hard filter; Patt byte-equal variant subsumed as C-PAT-1
    Evidence: `validation_checks/mod.rs:982-986`; `pool.rs:483-487`; `core.rs:491-497`; REQ-GRV-005
    Confidence: conf(0.85, converged)
    Rule change: relay authority = admission of the received bytes, never mempool membership by a forgeable key (INV-NETWORK-004 extended to the node side).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      0 (the admit result is already computed) (observed)
      Memory:   0 — batch join state is eliminated because the message arrives as one event (observed)
      IO:       0 (observed)
      Network:  -all forwards of resident-only recompositions and malleated-signature variants (inferred)
      Disk:     0 (observed)
      Latency:  0 beyond D1 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: the analyst's Q2 default "admitted now or already resident" — avoids no work and forwards forged-signature variants and all-resident re-batches
    Why this proposal anyway: it is the only verdict rule found under which no forgeable or already-paid content authorizes a forward
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: D6 — blocks keep the ONE written pre-verdict Accept (`classify_block_gossip` decode + stale ≤ 6 slots), with the per-peer block bucket moved BEFORE the report (Ignore) and the global bucket + memory-watchdog shed kept AFTER it (F-1); no `(slot, producer)` dedup before INC-I-236 (F-16); the exception text records the INC-I-236 dependency (W04) and the SEC-003 departure for blocks.
    Convergence: 5/5 on the exception; placement resolved by C1 (Restr + Fail explicit, code-verified)
    Evidence: `behaviour_events.rs:104-170`; `rate_limit.rs:184,209-257`; INC-I-114 M2 rationale (`:123-129`)
    Confidence: conf(0.85, converged)
    Boundary: block relay never depends on node state, a shared budget or fleet-wide memory pressure.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave the per-peer check after the report too (today's order) and accept that non-candidate blocks from one abusive peer are forwarded before they are dropped
    Why this proposal anyway: the per-peer bound is attacker-attributable and cheap to move, while every shared or load-dependent gate must stay local to satisfy F-1
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: R1 — two bounded lanes replace the single event channel + single shed bucket: consensus lane (blocks, attestations, all control events) and bulk lane (txs, votes, producer gossip); node select is `biased` consensus-first; each lane carries an `AtomicUsize` bytes-in-flight bound (added at `emit`, subtracted when the node drops the event); bulk sheds first and every shed reports Ignore; `NewBlock` becomes a sheddable class inside `emit_block`. Must ship BEFORE M3 (F-2/F-7 ordering).
    Convergence: 4/5 on the need (Restr P2, Patt P3, Fail F-7, Rad P4 lane rule); mechanism = simplest that passes F-7
    Evidence: `event_sink.rs:40-52` (attestations shed with txs); [E11] epochs 1835-1850 d=9-36 vs 1860-1861 d=1-2; REQ-GRV-017 "two lanes at most"
    Confidence: conf(0.75, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 select-arm poll per node loop iteration; +2 atomic ops per event (inferred)
      Memory:   worst-case event-path retention falls from count x max item (GiB-scale) to the two lane budgets (inferred)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  -attestation/block queueing behind tx floods; +tx latency under consensus load (intended) (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: one lane plus Rad's "shed bulk when less than half the channel is free" — bounds admission but gives attestations no dequeue priority, so a flood can still push their verdicts past the window
    Why this proposal anyway: under D1 attestation relay latency equals queue position; only a separate, first-served lane keeps REQ-GRV-010 under a tx flood (F-7)
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: R2 — attestations, votes and producer announcements go through the node verdict (M3): attestation = decode + Ed25519 + attester ∈ producer set (incl. pending activation), independent of block knowledge and BLS (F-9); `[ATTEST_FETCH]` gated by the same membership check and capped per tick; votes = decode + `verify` + membership; producer announcements = GSet merge added ≥ 1 verified entry; digests → Ignore. Gated on the M0 REQ-GRV-010 baseline; pre-planned fallback = option O9.
    Convergence: 5/5 membership (Sub K2/C7, Restr, Patt P4, Fail P4, Rad); F-9 overrules Patt's block-known clause
    Evidence: `attestation/message.rs:103-107` (self-certifying); pre-M3 `network_events.rs:541-549` (unknown block → fetch + return BEFORE membership, no verdict — drifted from the cited `:548-557`, G2; M3 moved the handlers to `gossip_verdicts.rs`); `:514` (vote self-certifying); `network_events.rs:595-631` + `sync.rs:309` (ungoverned fetch per forged hash)
    Confidence: conf(0.75, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 hash-set lookup per attestation/vote; the Ed25519 verify moves before relay, it is not added (observed)
      Memory:   0 (inferred)
      IO:       0 (observed)
      Network:  -forged attestation relay past hop 1; -ungoverned GetBlockByHash per forged hash (inferred)
      Disk:     0 (observed)
      Latency:  +node-loop time per attestation hop, bounded by the R1 lane (UNMEASURED, M0) (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep pre-verdict Accept for attestations as a second written exception and only gate ATTEST_FETCH on membership
    Why this proposal anyway: the cheaper path still relays forged attestations fleet-wide (the INC-I-237 shape with distinct bytes) and leaves the C-2 request storm reachable
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: R3 — node→swarm contract: the 35 awaited must-deliver command sends are kept (the swarm drains unconditionally); `GossipVerdict` is `try_send` only; RPC tx/vote broadcast closures take `try_reserve_owned()` BEFORE mempool admission and return `Busy` on Full (no `tokio::spawn` per item, no admitted-but-unbroadcast tx — F-15); `on_producer_digest` replies with `try_send` (drop on Full); the control backlog is byte-bounded (64 MiB, `SyncResponse` counts its payload).
    Convergence: 4/5 keep awaits (Sub K5, Restr P3, Fail v-1, Rad P4); RPC: Restr two-phase, Patt busy-error, Rad try_send → reserve-before-admit is the smallest F-15-safe form
    Evidence: `startup.rs:431-452`; `network_events.rs:477-500`; `command_handling.rs` 0 inner awaits; `event_sink.rs:29` (count bound) vs `protocols/sync.rs:22` (16 MiB)
    Confidence: conf(0.72, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -1 task spawn per RPC tx/vote and per received digest (observed)
      Memory:   -unbounded spawned-task growth under overload; backlog <= 64 MiB instead of 8,192 x 16 MiB (inferred)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  RPC sendTransaction returns Busy immediately under saturation instead of parking (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: Rad's plain `try_send` in the RPC closure — bounds memory but can admit a tx to the local mempool and silently never broadcast it (F-15)
    Why this proposal anyway: reserving the slot before admission is the only form in which an overload is both bounded and client-visible, at zero extra tasks
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: R4 — own-origin (RPC/system) txs publish one tx per gossip message in the EXISTING legacy single-tx encoding; delete the 100 ms / 50-tx batcher and the dormant announce flush (`swarm_loop.rs:191-238`); the dead `BroadcastTransaction` command arm (`command_handling.rs:66`) becomes the live path. Scale ceiling recorded: envelope verify is per received copy (`vendor/protocol.rs:299`), so above ~1,000 tx/s re-batching must come from the announce/fetch evolution path, never from relay-side composition.
    Convergence: 3/5 (Patt P2, Rad P2, Fail P3 "prefer"); Sub K1 opposed (CPU), Restr conditional on M0 origin rate
    Evidence: `publish.rs:131-135` (legacy fallthrough, oldest format); `vendor/protocol.rs:299`; commit `23d11422` (batching, no measured benefit)
    Confidence: conf(0.70, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +~70 us per own-origin tx (sign at origin + envelope verify per received copy), <= ~4% of one core at 100 tx/s x 8 copies; -batch encode/decode (inferred)
      Memory:   -tx_batch and tx_announce_batch Vecs; +1 dup-cache entry per tx instead of per batch (<= ~1.8 MiB at 100 tx/s x 300 s) (inferred)
      IO:       0 (observed)
      Network:  +~130-150 B envelope per tx only when a burst would have batched (~0 at today's typical batch of 1) (inferred)
      Disk:     0 (observed)
      Latency:  -up to 100 ms per own-origin publish (no flush wait) (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep the batcher for origin txs (they are all new, so D5 holds for them) and accept that one resident sibling inside an own batch stops its siblings at one hop
    Why this proposal anyway: one object per message makes the content id the object id (no id-function change), survives 6.43.3 neighbours (X != their re-published Y), and deletes code
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: R5 — late-verdict contract: the swarm verdict arm skips the report when `recv_at.elapsed() > 3.5 s` and increments a per-topic late counter; every swarm-side drop site (per-peer rate, lane shed, pre-filter Ignore) reports Ignore at the drop; INV-NET-002 is amended to "at most one report per message; never after the window; mcache expiry ≡ Ignore, counted"; `history_length` is NOT raised as the fix (only re-sized from the measured p99, option O11).
    Convergence: 4/5 skip (Sub, Restr, Rad, Patt C-PAT-3) vs Fail F-3 literal; resolved by vendor code (both late paths WARN and are no-ops)
    Evidence: `behaviour.rs:759-768,805-807`; `mcache.rs:189-196`; `config.rs:304-314` (Ignore carries no P3 penalty on the attestation topic)
    Confidence: conf(0.80, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 Instant comparison per verdict (observed)
      Memory:   +O(topics) counters (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     -vendor "not in cache" WARN per late message under a backlog (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: report every late verdict anyway (F-3 letter) — identical relay outcome plus one WARN line per late message
    Why this proposal anyway: a late report changes no state in gossipsub 0.46.1, so the counter is the only observable the letter would add, without the log flood
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: R6 — delete the app `SeenCache` + `seen_key` + `classify_transaction_gossip` + its param threading; add `const_assert!(GOSSIP_DUPLICATE_CACHE_TIME >= 12 slots x slot_duration)` (largest slot-bound age window); INV-NETWORK-004's raw-bytes key is then carried by the vendor id `BLAKE3(data)` and its regression test points there.
    Convergence: 2/5 explicit (Sub P2, Rad P3); dominance code-verified (dup cache insert precedes app dispatch, id excludes topic, 300 s > 180 s)
    Evidence: `behaviour.rs:1718-1725`; `config.rs:67,199-202`; `staleness.rs:93-99` (doc still reasons against a 120 s cache)
    Confidence: conf(0.75, converged)

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -1 BLAKE3 per non-block gossip message in the single-threaded swarm task (observed)
      Memory:   -up to ~2 MB (16,384 entries) (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave the SeenCache in place unchanged
    Why this proposal anyway: two registries with the same key and different TTLs have already drifted once; one asserted registry removes a future relay-bug source
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision (divergent additions; all low-evidence, conf < 0.7)

| # | Option | Source | What / cost | Filter note | Confidence |
|---|---|---|---|---|---|
| O1 | Future-slot Ignore on blocks (slot > wall-clock + tolerance) | Patt | stateless, uses the existing `best_slot` atomic; +1 compare | F-1 caution: a wall-clock gate can veto a candidate block under skew → tolerance ≥ 2 slots | conf(0.50, inferred) |
| O2 | First-per-(attester, slot) seen set AFTER authentication | Patt | ~480 KB at 1,000 producers; bounds member-signed re-attestations | safe only after membership (C-PAT-2); not required by F-16 | conf(0.55, observed) |
| O3 | Delete the dormant tx announce/fetch subsystem (~400 lines, 3 events, 2 commands, 1 protocol) | Sub P6 | default already in D2 (0xAA → Ignore, 2 lines); full deletion is REQ-GRV-020 | evolution path kept in git history (REQ-GRV-018) | conf(0.45, observed) |
| O4 | Dead seam code sweep: Tier1/region branches, 9 unused `publish_*` helpers, legacy bincode producer path + its per-message spawn, duplicate test builder, tautological `assert_gossip_hardening_invariant(.., true)` fixed to read the real lane bounds | Sub P5 | ≈ -215 lines, 0 callers (grep-measured) | none | conf(0.65, measured) |
| O5 | Module moves for the 500-line budget: `gossip_ingress.rs`, `prefilter.rs`, `command_api.rs`, `swarm_maintenance.rs`, `relay_dispatch.rs` | Restr | ~800 moved lines; REQ-GRV-021 Could | none | conf(0.50, inferred) |
| O6 | Peers table tokio `RwLock` → `std::sync::RwLock` + `clippy::await_holding_lock` | Restr P3 | closes latent cycle C2 by lint; µs sections | toolchain lint default unverified | conf(0.50, inferred) |
| O7 | Tx-topic `mesh_message_deliveries_weight` → 0 during the (b) interim | Patt | `config.rs:271-281` uses defaults; every Ignored tx counts as a mesh deficit today | measure gossipsub scores in M0 first | conf(0.45, observed) |
| O8 | Origin re-publish of OWN unmined txs after `duplicate_cache_time` (F-15 alternative to documented loss) | Fail F-15 (no evaluator mechanism) | periodic tick over own-origin mempool entries; not a relay of received content | must stay origin-only (REQ-GRV-005) | conf(0.50, inferred) |
| O9 | Contingent attestation exception: swarm checks age + Ed25519 + attester ∈ node-published producer-set snapshot, then AcceptNow | Rad P5 | +1 shared snapshot, signature verified twice (~50 µs); use ONLY if M3 measurement lowers coverage | F-14 (swarm-side crypto) — bounded by producer count | conf(0.40, inferred) |
| O10 | Byte-permit semaphore lanes (permits travel inside events, unbounded mpsc) instead of the R1 atomic counter | Restr P2 full form | exact byte accounting across handler residence; ~130 lines | none | conf(0.55, inferred) |
| O11 | Re-size `history_length` 5 → 8-10 from the measured verdict p99 | Patt Q5 | local non-wire knob (Lighthouse uses 12); raises mcache bytes | F-13: allowed only as window sizing from measurement, never as the relay fix | conf(0.50, observed) |
| O12 | Control-event coalescing design (F-12): on backlog overflow, drop ONLY re-derivable events (Peer*/Status* re-synced from the shared peers table the node already reads, `service/mod.rs:656-668`); `SyncResponse` count is request-bounded by `max_concurrent_requests` (`sync/bodies.rs:34`) | synthesizer, from Fail F-12 | no evaluator proposal passes F-12 as written; this is the smallest fix found, UNDESIGNED | must keep control drops counted and readable (REQ-GRV-016) | conf(0.60, inferred) |

## Q1–Q8 Resolutions

| Q | Analyst default | Resolution | Why |
|---|---|---|---|
| Q1 | blocks keep pre-verdict Accept, rate limit + watchdog moved before | **KEPT, amended**: per-peer bucket before; global bucket + watchdog AFTER | C1 (F-1, code: global bucket is shared, drainable; candidate-next blocks use it only) |
| Q2 | all-or-nothing, "admitted now or already resident" | **FLIPPED**: all NEWLY admitted by this message | C2 (F-5; `tx.hash()` excludes signatures; REQ-GRV-005) |
| Q3 | stop relaying headers + heartbeats | **KEPT, stronger**: unsubscribe + delete header publish, after the F-10 tag sweep | 5/5; headers drain the global block bucket |
| Q4 | relay valid content from old publishers | **KEPT**: content-based verdict; "valid" = verified-and-new | F-5 |
| Q5 | late ⇒ no forward, report Ignore, count | **KEPT, amended**: no report after the 3.5 s deadline, counted; swarm-side drops report at the drop | C6 (vendor late paths are logged no-ops) |
| Q6 | max_peers 50, 100 tx/s, attestations = producers/slot | **KEPT, extended**: bounds list adds the vendor dup cache (300 s × distinct-id rate, no count cap) and the attestation intake budget scaling with producer count | Sub, Fail |
| Q7 | attestations through the node verdict | **KEPT, conditional**: membership required; NOT block-known; ships only after R1 lanes and the M0 baseline; fallback O9 | 5/5 + F-7/F-9 |
| Q8 | keep 6.43.3 (b) until this ships | **KEPT**: removed only in the M2 commit that adds the verdict and deletes the re-publish | Rad C3, Restr |

## Constraints (from the Failure Analyst) — every chosen path must pass

| ID | Constraint | Proposal status | Proving test |
|---|---|---|---|
| F-1 | No load/state/shared-budget condition suppresses a decodable non-stale candidate-next block | PASS via D6 (global + watchdog after report) | A-B-C-D line, B loop paused 30 s + watchdog set + global bucket drained → block at A reaches D ≤ baseline + 50 ms |
| F-2 | No traffic class consumes a budget gating block/attestation intake | PASS via D3 (unsubscribe) + R1 | 1,000 forged headers then a candidate block passes; 1,000 heartbeats/votes then 10 attestations delivered |
| F-3 | One report per message on every path | PASS as amended (R5): every swarm-side drop reports; expiry counted, never double-reported | counting fake gossipsub: on-time/late/never/duplicate/shutdown → ≤ 1 report, late counter = 1 |
| F-4 | Node verdict never maps to Reject | PASS (D2/D5) | mixed testnet 1 h: honest scores > -10 |
| F-5 | Only verified NEW content authorizes a forward | PASS (D5) | F-B-C: M garbage-sig variants + M resident recompositions → C receives 0 |
| F-6 | Swarm never awaits the node; must-deliver node→swarm keeps back-pressure | PASS (D4 + R3) | extend `inc_i_237_loop_deadlock.rs`: node stopped 60 s under 10× flood, swarm ticks continue, own block publish delivered after resume; compile-fail test for `sender()` |
| F-7 | Tx flood cannot delay attestation verdicts past the window nor shed blocks/attestations/control; ≤ 2 lanes | PASS (R1) | REQ-GRV-007 flood + 1,000 attestations/slot: 0 attestation sheds, verdict p99 < 2 s |
| F-8 | Verdict latency measured INCLUDING fleet-synchronous stalls | **GATE** = milestone M0 | ≥ 3 epochs incl. boundaries + [INTEGRITY] windows + one rolling restart; per-topic histogram; late-counter readers > 0 |
| F-9 | Attestation relay independent of block knowledge/BLS; ATTEST_FETCH gated + bounded | PASS (R2) | 1,000 forged attestations/s → C receives 0, ≤ k fetches/tick; genuine attestation preceding its block still relayed |
| F-10 | Every old wire format on a relayed topic decodes, or has a written fail-open exception | **PRECONDITION** for D2/D3 (tag sweep + golden vectors) | golden-vector corpus per release tag since genesis → Accept |
| F-11 | Shutdown never waits on a verdict or command send | PASS (R3: closing the swarm closes receivers) | SIGTERM under flood × 20 → exit < 5 s |
| F-12 | Control-state events never dropped silently; coalesce, do not drop | **OPEN** — no proposal passes as written; smallest fix = O12 | 16 MiB SyncResponses + peer churn → node peer count == swarm table every 1 s |
| F-13 | Relay memory bounded by mcache window + per-peer share; no TTL tuning as the fix | PASS (per-peer bucket before Defer; D1) | 10× [E4] flood 10 min: heap delta < 64 MiB |
| F-14 | Swarm-side per-message work stays cheap | PASS (no swarm crypto; O9 is the only exception and is contingent) | block recv p50/p99 under tx flood ≤ baseline + 50 ms |
| F-15 | Transient Ignore never strands a valid tx forever | PASS for own-origin (R3 Busy); OPEN for chained/late received txs → O8 or documented RPC-visible status | A-B-C-D with B paused 10 s + parent/child split → both at D within 330 s, or documented status |
| F-16 | Both equivocating variants stay visible (raw-bytes dedup only) | PASS (D6: no (slot, producer) key) | two variants at A → D receives both |

Binding order: F-2 + F-7 (D3 + R1) ship before any attestation verdict (R2/M3); the F-10 sweep ships before any undecodable → Ignore change (D2).

## Invariants to Amend and Spec Drift to Fix (REQ-GRV-013/019)

- **INV-NET-002** → "at most one `report_message_validation_result` per message; every swarm-side drop reports Ignore at its drop site; a report is never issued after the mcache window; mcache expiry ≡ Ignore and is counted per topic. Only swarm-provable undecodable bytes may Reject." (code today fails open to Accept on non-block topics — the record and the code already disagree).
- **INV-NETWORK-003** → "a message that the NODE judged relayable is forwarded exactly once, under its original id; topics without a local consumer are not subscribed" (today literally violated by 6.43.3 (b) and by the two consumer-less topics).
- **INV-NETWORK-002** → the queue half must read the real lane bounds, not the literal `true` at `config.rs:246`.
- **INV-NETWORK-004** → extended to the node side: no residency/rejection key that can authorize or suppress a relay may be a signature-excluding `tx.hash()` (C-PAT-1).
- **Drift:** `specs/protocol.md` ~2089-2094 (claims "never silently Accepted" for `None` topics — code Accepts; documents fail-open Accept as a feature; describes re-publish as the multi-hop mechanism — refused `Duplicate`); `staleness.rs:93-122` doc (120 s cache rationale vs 300 s); `specs/security_model.md:620`; `docs/architecture.md`.

## Architecture Maps

**Current (v6.43.3):** peer → gossipsub(dup 300 s → mcache) → `handle_behaviour_event` → classifier → REPORT (swarm decides 6/7 topics; txs always Ignore) → watchdog / rate limit / `EventSink` shed (after the forward) → one event channel (count-bound) + control backlog (8,192 count) → node loop → handler → [txs] `broadcast_transaction().await` → command channel → batcher → NEW publish (refused if byte-identical). Node→swarm: 35 awaits + 3 spawn-per-item tasks. Swarm→node: `EventSink` + raw `sender()`.

**Proposed (Definite + Recommended):** peer → gossipsub(dup 300 s → mcache = pending store) → exhaustive `relay_decision` → {blocks: AcceptNow after per-peer bucket; others: Defer} → consensus lane | bulk lane (byte-bounded, biased) → node handler returns Verdict → `GossipVerdict` try_send → swarm arm (3.5 s deadline) → `report_message_validation_result` on the ORIGINAL id → mesh forward. No re-publish of received content; own-origin txs = one legacy single-tx message each. Node→swarm: 35 awaits kept + verdict/RPC/digest non-awaiting. Swarm→node: sync-only sink (out-degree 0).

## Migration Path (every milestone rolling-deploy safe; testnet first)

- **M0 — Measure (no relay-path change).** Per-topic verdict-latency histogram receive→handler-done (p50/p99) over ≥ 3 epochs incl. boundaries, the fleet-synchronous `[INTEGRITY]` windows (W03 is a dependency of the SLO) and one rolling restart (F-8); attestation coverage baseline (REQ-GRV-010); block recv latency baseline (REQ-GRV-009); origin tx rate + batch-size distribution (R4 condition); gossipsub score metrics (O7); `sizeof(Transaction)` (SEC-005); F-10 tag sweep + golden-vector corpus. Deploy: rolling. Gate: no M2/M3 ships with p99 > 2 s at the tip without O11 re-sizing.
  - BRIDGE: keep 6.43.3 (b) (tx first-sight → Ignore + node re-publish) until M2 lands; watch metric = count of `Not publishing a message that has already been published` on `/doli/txs/1`. Temporary — removed in the M2 commit.
- **M1 — Subtraction + contract (keeps (b)).** D3 unsubscribe + stop header publish (after F-10 sweep); D4 sync-only sink; D2 exhaustive table with `None`/0xAA/legacy-bincode/Tier1/region → Ignore and undecodable non-block → Ignore; D6 per-peer reorder on blocks; R3 RPC `try_reserve`/Busy + digest `try_send` + byte-bounded backlog; R1 two lanes. Tests: F-1, F-2, F-6, F-11; gauntlet. Deploy: rolling.
- **M2 — Tx verdict (ONE atomic commit).** D1 verdict seam + D5 strict rule + R5 deadline/counters + R4 single-tx origin publish + batcher deletion; remove (b) AND the node re-publish in the same commit (never one without the other: one alone re-opens either the flood or the one-hop gap). Tests: REQ-GRV-001 line A-B-C-D (single-tx and 50-tx variants — the missing partition "B admits F's tx → C receives it"), 002, 004, 005; F-3, F-5, F-13; mixed testnet with 6.43.3 + 6.43.0-equivalent + an old external (REQ-GRV-012). Deploy: rolling.
- **M3 — Attestations / votes / producers via the verdict.** R2 (membership, NOT block-known, ATTEST_FETCH gate + per-tick cap of 8 `GetBlockByHash`, excess kept queued oldest-first inside the tracker's 30 s TTL, digest Ignore). One identity per message: no `DirectAttestation` re-publish (the attester's own gossip publish carries it), no second own-block attestation publish, no byte-identical digest publish inside `GOSSIP_DUPLICATE_CACHE_TIME`. Precondition: M0 baseline + R1 shipped (F-2/F-7). Tests: F-7, F-9, REQ-GRV-010 coverage ≥ baseline under flood. If coverage drops → O9. Deploy: rolling.
- **M4 — Cleanup + invariants.** R6 SeenCache deletion + const assert; INV-NET-002 / INV-NETWORK-002/003/004 amendments with linked regression tests; `config.rs:246` literal fixed; specs/docs drift closed (REQ-GRV-019); REQ-GRV-016 metrics with readers (lane depth, late verdicts per topic, control drops, shed per topic); chosen options O3/O4/O5. Deploy: rolling.

## Deploy Answers (CLAUDE.md three-question checklist, INC-I-075 / INV-12)

1. Can a user-submittable tx reach this path? **YES** (every gossiped tx is judged). 2. Can a producer action or attestation pattern reach it? **YES** (attestations, blocks). 3. Is the new behaviour bit-identical for ALL reachable inputs on every consensus-visible computation? **YES** — no validation, apply, scheduler, bitfield, coinbase, state-root, message-id, topic, wire-encoding, `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION` or `HardForkSchedule` change; only WHICH node forwards WHICH gossip message changes. ⇒ **(1|2) YES + (3) YES → NO activation height.**
- Consensus RULES change? **NO.** Block CONTENT change (bitfield, coinbase, tx ordering, presence_root, header fields)? **NO** by format — which txs/attestations reach a producer in time is ordinary network variance (REQ-GRV-010 guards the attestation side). ⇒ **rolling restart permitted** (no synchronized stop-all); testnet first; mainnet only with explicit confirmation.
- Genesis reset (#0 rule): **NOT needed, NOT allowed.** `HardForkSchedule`: never (node-local).
- Mixed versions: nodes ≤ 6.43.2 and old externals still forward before any verdict; an upgraded node is a one-hop firewall (REQ-GRV-004). New single-tx messages decode on every version; old 0xBA batches are judged all-new-or-Ignore; no Reject for any old valid format (SEC-004).

## Won't (owed separately — not part of this redesign)

| ID | Item | Note |
|---|---|---|
| W01 | [E2] `handle_equivocation` payload, `pending_proofs` cap, `add_system_transaction` type guard | `/omega-doctor --incident INC-I-237`; **dependency**: admission becomes the relay authority, so SlashProducer must stay refused from gossip (test owed) and re-admittable zero-flow types must not become replay relays |
| W02 | [E7] apply_block skip-guard parent-hash check; Rule-2 bare-gap snap (INC-I-235) | fork domain |
| W03 | [E9] `[INTEGRITY]` scan cadence/concurrency | separate fix, but a **dependency of the latency SLO** — M0 measures inside scan windows |
| W04 | INC-I-236 signed headers | own activation height; bounds the block exception (D6/C7) |
| W05 | production host density, swap and MemoryMax ([E6b]) | ops |
| W06 | libp2p-gossipsub ≥ 0.48 upgrade | kept possible: no gossipsub type crosses into `bins/node`; no dependence on the vendored send_queue cap |
| W07 | Sync request/response governance (INV-SYNC-009) | unchanged; R2 gates ATTEST_FETCH node-locally only |
| W08 | Message-id function, topic names, wire encodings | unchanged by design (R4 uses an EXISTING encoding) |

## Hardware Impact (CPU / memory / IO / lock)

CPU: swarm task −1 BLAKE3 per message, −header/heartbeat decode, +1 report per deferred message; node task unchanged work (it already admits/verifies every tx/attestation) + 1 try_send per verdict; +per-copy envelope verify for single-tx origin publishes (≤ ~4% of one core at the Q6 target). Memory: pending-verdict retention = vendor mcache, inbound × 5 s, per-peer rate-limited (unchanged in kind — entries live 5 heartbeats validated or not); −SeenCache ≤ 2 MB; event-path worst case falls from count × max item to the two lane budgets; backlog ≤ 64 MiB; RPC path ≤ command capacity. IO: 0; disk −WARN floods. Locks: no new lock; no lock held across any send; the backlog mutex stays push/pop-only; the peers tokio `RwLock` is the only remaining shared awaitable (O6).

## Complexity Comparison

| Metric | Current v6.43.3 | Radical minimum (#5 as written) | Proposed (radical + filter fixes) |
|---|---|---|---|
| Modules (seam files; new) | 16 (—) | 16 (0 new) | 16 (0 new; O5 moves optional) |
| Interfaces: `NetworkEvent` / `NetworkCommand` / `EventSink` pub fns | 21 / 19 / 6 | 19 / 18 / 4 | 19 / 18 / 4 |
| Channels (swarm↔node conduits) | 2 mpsc + 1 count backlog + 1 RwLock | 2 mpsc + 1 byte backlog + 1 RwLock | 3 mpsc (2 lanes + command) + 1 byte backlog + 1 RwLock |
| Registries (seen / relayable) | 5 | 4 | 4 |
| Relay-decision sites | 5 | 2 | 2 |
| Post-Accept drop paths | 4 | 0 (fails F-1) | 2 (block-only, written) |
| Topics forwarded before the node verdict | 6 of 7 | 1 (blocks) | 1 (blocks) |
| Topics relayed with no consumer | 2 | 0 | 0 |
| Node→swarm: awaited sends / spawn-per-item / try_send | 35 / 3 / 0 | 30 / 0 / 3 | 35 / 0 / 3 |
| Await-cycles | 0 by convention | 0 by type | 0 by type |
| New types | — | 3 | 4 (+ lane class) |
| Production lines `service/` + `gossip/` | ~5,257 (measured) | ~5,100 (−150 ± 100, inferred) | ~5,050 ± 150 (≈ 4,450 with O3 + O4) |

## Risks and Unmeasured Items (block implementation confidence > 0.7)

1. **Node-loop verdict latency at the tip** (p50/p99, incl. epoch-boundary apply, `[INTEGRITY]` windows, catch-up after a rolling restart) — every evaluator flags it; cap 3.5 s; M0.
2. **Attestation presence deadline vs hops × node-loop latency** (REQ-GRV-010) — unknown whether one missed slot costs presence; M0 + M3 gate.
3. **Mainnet origin tx rate and batch-size distribution** (R4 cost; K1 ceiling).
4. **External/third-party consumers of `/doli/headers/1`, `/doli/heartbeats/1`** and the full F-10 format census (only v6.25.0 checked).
5. **RPC `Busy` client compatibility** (wallet, faucet, CLI not searched).
6. **F-12** has no passing proposal (O12 undesigned); **F-15** open for chained/late received txs (O8 or documented status).
7. `sizeof(Transaction)` for the SEC-005 2× decode bound; per-topic `TopicScoreParams` interaction with held-then-Ignored messages (tx-topic P3 deficit, O7); GSet merge signature verification (producer verdict). Vote-path membership source RESOLVED in M3 (X6): `producer_set` `get_by_pubkey(pk).is_some_and(|p| p.is_active())`, the same `Arc<RwLock<ProducerSet>>` the updater tally reads (`run.rs:440-452`).
8. Fleet-synchronous stalls turn "forwarded late" into "not forwarded" on every node at once (Fail i-6); the only floor is the origin's `flood_publish` hop.
9. During the mixed-version window, old re-batchers still amplify to their direct peers; new nodes stop it at one hop only.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     6 (3+/5 agreement: (b)+re-publish, None→Accept, headers/heartbeats, sender(), per-tx fan-out, fail-open Accept)
Restructuring convergence:      5 (verdict seam, block exception, strict tx rule, lanes, membership verdict)
Addition options presented:     12
Failure modes identified:       16 (F-1..F-16) + 10 current-state (C-1..C-10)
Failure modes applied as filters: 16/16 (14 PASS, F-8 = M0 gate, F-12 OPEN, F-15 partial)
Radical floor gap:              5 relay sites / 6 pre-verdict topics / 4 post-Accept drops → 2 / 1 / 0 (radical) → 2 / 1 / 2 (proposed; +2 lanes)
Contradictions found:           13 (C1-C7 + X1 batcher, X2 block-known, X3 announce/fetch, X4 header publish timing, X5 digest relay, X6 vote authority)
Contradictions resolved:        13/13 (X6 vote membership source resolved in M3: active producer set, same as the updater tally)
Evidence independence verified: YES (vendor code read by 4 evaluators independently; Lighthouse source by the 5th)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
