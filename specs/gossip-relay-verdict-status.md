# Gossip Relay Verdict — Implementation Status, Evidence, Open Items

> INC-I-237 · companion to `specs/gossip-relay-verdict-architecture.md` (design) and `specs/protocol.md` §7.2 (relay rules as shipped). Branch `refactor/inc-i-237-gossip-relay-verdict`. Updated 2026-10-12 (M4).

## Status: IMPLEMENTED

| Milestone | Commit | Content |
|---|---|---|
| M0 | `4277c300` | verdict-latency / block-recv / attestation-coverage metrics, probe, F-10 format corpus |
| M1 | `7c8dc3be` | D2 exhaustive relay table, D3 unsubscribe headers/heartbeats, D4 sync-only sink, D6 per-peer block bucket before the report |
| M1.5 | `b0051849` | R1 two bounded lanes, R3 reserve-before-admit (RPC Busy), byte-bounded control backlog |
| M2 | `cc90ddcd` | D1 verdict seam, D5 newly-admitted tx rule, R5 3.5 s deadline + late counter, R4 single-tx origin publish |
| M3 | `ec84959d` | R2 attestation / vote / producer verdicts, ATTEST_FETCH gate + per-tick cap, digest LocalOnly |
| M4 | `b4fa9ce9` | R6 app dedup cache deleted + const assert, INV-NETWORK-002 real lane budgets, lane depth / shed-by-topic metrics + probe `channel` lines, dead seam code removed, seam files ≤ 500 lines, specs/docs drift closed |

## Evidence

- **M0 latency gate — PASSED** (testnet, 2026-10-11): verdict p99 on producers — txs ≤ 0.16 s, blocks ≤ 0.68 s, attestations ≤ 0.42 s (gate ≤ 2 s); the seed's tail appeared only during restart catch-up.
- **M2 mixed-version check — PASSED** (testnet, 5 × M2 + 2 × 6.43.2): 275/275 txs mined, no fork, 0 errors, 0 peer penalties.
- **M3 devnet A-B-C-D line:** duplicate-publish WARNs 64 → 0; attestations at D = 40; all four nodes at the same height.
- **NOT run:** a mixed-version testnet soak with the M3/M4 build.

## Open items (owed; none is part of M4)

| ID | Item | Where it stands |
|---|---|---|
| F-12 / O12 | Control events (peer / status / sync) can still drop. The backlog holds at most `BACKLOG_CAP` = 8,192 events, and status/sync payload events at most `BACKLOG_BYTES` = 64 MiB (`service/event_sink.rs::admit_backlog`); past either bound the event is dropped and counted (`doli_event_lane_shed_total{lane="backlog"}`, probe `channel control_dropped=`). | O12 (on overflow drop only re-derivable events) is NOT approved and undesigned. |
| INC-I-238 (G1) | An attestation signs only `(block_hash, slot)`: changing `height`, `attester_weight` or `bls_signature` of a member-signed attestation mints a new message id. Votes: JSON field order / whitespace variants. | Relay half CLOSED for attestations (M5, AUDIT-P1-506): the verdict Accepts only a first-seen `(attester, slot)` or a newly pooled genuine BLS half. Wire half OPEN (the signature still excludes those fields). Vote variants OPEN (AUDIT-P1-507). |
| F2-bucket | Attestations have their own per-peer bucket (20/s, burst 200; M5 decision C), so a vote or producer flood no longer starves them. No global attestation bucket and no BLS pairing cap yet. One forwarder that relays more than 200 attesters per 10 s slot exceeds the per-peer bucket (same 20/s as before M5). | Owed: global attestation bucket + BLS pairing cap (`rate_limit.rs`). |
| M5-DEV-F1 | One member key could sign `(attester, slot)` for any future slot; each was first seen, so Accepted, forwarded and kept in `AttestSeen`. | CLOSED (M5 decision C): the swarm Ignores an attestation with slot > wall + 2 (`staleness.rs::ATTEST_FUTURE_SLOTS`). Risk: a relaying node whose clock lags ≥ 2 slots (20 s at 10 s slots) Ignores genuine current attestations. |
| G3 | The digest reply delta is a producer-topic publish to all peers, not a direct reply to the forwarder. | Owed: request-response reply. |
| AUDIT-P1-505 reach | A zero-flow tx (DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer) is admitted but always gets `Ignore` (M5), so a genuine one reaches only the origin's direct peers (one hop, flood publish). | Owed: AUDIT-P2-001 (verified zero-flow admission lane) lifts it. |
| F-15 | A transient `Ignore` can strand a valid chained or late received tx (no hop re-forwards it). Own-origin txs are covered by RPC Busy (R3). | Owed: O8 (origin re-publish of own unmined txs) or an RPC-visible status. |
| REQ-GRV-016 remainder | Not built: command-channel depth gauge, pending-verdict gauge, vendor `send_queue` drop metric (today counted inside the vendor handler and logged at WARN, not exported), 1/min INFO summary line. | Owed. |
