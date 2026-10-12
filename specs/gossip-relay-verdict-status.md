# Gossip Relay Verdict — Implementation Status, Evidence, Open Items

> INC-I-237 · companion to `specs/gossip-relay-verdict-architecture.md` (design) and `specs/protocol.md` §7.2 (relay rules as shipped). Branch `refactor/inc-i-237-gossip-relay-verdict`. Updated 2026-10-12 (M4).

## Status: IMPLEMENTED

| Milestone | Commit | Content |
|---|---|---|
| M0 | `0706972b` | verdict-latency / block-recv / attestation-coverage metrics, probe, F-10 format corpus |
| M1 | `bb11338d` | D2 exhaustive relay table, D3 unsubscribe headers/heartbeats, D4 sync-only sink, D6 per-peer block bucket before the report |
| M1.5 | `c89b65b2` | R1 two bounded lanes, R3 reserve-before-admit (RPC Busy), byte-bounded control backlog |
| M2 | `21a37c7a` | D1 verdict seam, D5 newly-admitted tx rule, R5 3.5 s deadline + late counter, R4 single-tx origin publish |
| M3 | `c1ae6ad0` | R2 attestation / vote / producer verdicts, ATTEST_FETCH gate + per-tick cap, digest LocalOnly |
| M4 | this branch, commit after `c1ae6ad0` | R6 app dedup cache deleted + const assert, INV-NETWORK-002 real lane budgets, lane depth / shed-by-topic metrics + probe `channel` lines, dead seam code removed, seam files ≤ 500 lines, specs/docs drift closed |

## Evidence

- **M0 latency gate — PASSED** (testnet, 2026-10-11): verdict p99 on producers — txs ≤ 0.16 s, blocks ≤ 0.68 s, attestations ≤ 0.42 s (gate ≤ 2 s); the seed's tail appeared only during restart catch-up.
- **M2 mixed-version check — PASSED** (testnet, 5 × M2 + 2 × 6.43.2): 275/275 txs mined, no fork, 0 errors, 0 peer penalties.
- **M3 devnet A-B-C-D line:** duplicate-publish WARNs 64 → 0; attestations at D = 40; all four nodes at the same height.
- **NOT run:** a mixed-version testnet soak with the M3/M4 build.

## Open items (owed; none is part of M4)

| ID | Item | Where it stands |
|---|---|---|
| F-12 / O12 | Control events (peer / status / sync) can still drop. The backlog holds at most `BACKLOG_CAP` = 8,192 events, and status/sync payload events at most `BACKLOG_BYTES` = 64 MiB (`service/event_sink.rs::admit_backlog`); past either bound the event is dropped and counted (`doli_event_lane_shed_total{lane="backlog"}`, probe `channel control_dropped=`). | O12 (on overflow drop only re-derivable events) is NOT approved and undesigned. |
| INC-I-238 (G1) | An attestation signs only `(block_hash, slot)`: changing `height`, `attester_weight` or `bls_signature` of a member-signed attestation mints a new message id that passes the node verdict and is relayed. Votes: JSON field order / whitespace variants. | OPEN, pre-existing; user / architect decision. O2 seen-set deliberately not added. |
| F2-bucket | Attestations share the per-peer request bucket (20/s, burst 200) with votes and producer batches; a vote flood from one peer can starve that peer's attestations at this hop. It is also the first scale ceiling for thousands of producers. | Owed: own attestation bucket sized by producer count (`rate_limit.rs`). |
| G3 | The digest reply delta is a producer-topic publish to all peers, not a direct reply to the forwarder. | Owed: request-response reply. |
| F-15 | A transient `Ignore` can strand a valid chained or late received tx (no hop re-forwards it). Own-origin txs are covered by RPC Busy (R3). | Owed: O8 (origin re-publish of own unmined txs) or an RPC-visible status. |
| REQ-GRV-016 remainder | Not built: command-channel depth gauge, pending-verdict gauge, vendor `send_queue` drop metric (today counted inside the vendor handler and logged at WARN, not exported), 1/min INFO summary line. | Owed. |
