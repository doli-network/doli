# Gossip Relay Verdict — Implementation Status, Evidence, Open Items

> INC-I-237 · companion to `specs/gossip-relay-verdict-architecture.md` (design) and `specs/protocol.md` §7.2 (relay rules as shipped). Branch `refactor/inc-i-237-gossip-relay-verdict`. Updated 2026-10-12 (M8).

## Status: IMPLEMENTED

| Milestone | Commit | Content |
|---|---|---|
| M0 | `4277c300` | verdict-latency / block-recv / attestation-coverage metrics, probe, F-10 format corpus |
| M1 | `7c8dc3be` | D2 exhaustive relay table, D3 unsubscribe headers/heartbeats, D4 sync-only sink, D6 per-peer block bucket before the report |
| M1.5 | `b0051849` | R1 two bounded lanes, R3 reserve-before-admit (RPC Busy), byte-bounded control backlog |
| M2 | `cc90ddcd` | D1 verdict seam, D5 newly-admitted tx rule, R5 3.5 s deadline + late counter, R4 single-tx origin publish |
| M3 | `ec84959d` | R2 attestation / vote / producer verdicts, ATTEST_FETCH gate + per-tick cap, digest LocalOnly |
| M4 | `b4fa9ce9` | R6 app dedup cache deleted + const assert, INV-NETWORK-002 real lane budgets, lane depth / shed-by-topic metrics + probe `channel` lines, dead seam code removed, seam files ≤ 500 lines, specs/docs drift closed |
| M5 | `7fb9d2b1` | zero-flow never forwards, `(attester, slot)` attestation newness, own attestation bucket, future-slot bound, per-tx bucket charge, block headroom, F-13 soak |
| M6 | `bf6c8efd` | attestation relay authority from a per-`(attester, slot)` BLS-forwarded bit (not pool residency), unknown-block relay-only BLS check (F7), wall-clock `AttestSeen` prune |
| M7.1 | `80724ccf` | soak retained-heap attribution per (swarm, structure); vendor size readouts; duplicate-cache gauge |
| M7.2 | `5076336b` | count caps with oldest-first eviction on the vendor duplicate cache (218,000) and peer-score `deliveries` cache (50,000); table pinned at the cap's size (F-13) |
| M8 | this branch, after `5076336b` | zero-flow admission verifies DelegateBond / RevokeDelegation signatures and AddMaintainer / RemoveMaintainer current-set multisig as apply does (all 3 admission sites, `mempool::zero_flow_auth`); those 4 types relay multi-hop again; Registration stays never-forward |

## Evidence

- **M0 latency gate — PASSED** (testnet, 2026-10-11): verdict p99 on producers — txs ≤ 0.16 s, blocks ≤ 0.68 s, attestations ≤ 0.42 s (gate ≤ 2 s); the seed's tail appeared only during restart catch-up.
- **M2 mixed-version check — PASSED** (testnet, 5 × M2 + 2 × 6.43.2): 275/275 txs mined, no fork, 0 errors, 0 peer penalties.
- **M3 devnet A-B-C-D line:** duplicate-publish WARNs 64 → 0; attestations at D = 40; all four nodes at the same height.
- **NOT run:** a mixed-version testnet soak with the M3/M4 build.

## Open items (owed; none is part of M7)

| ID | Item | Where it stands |
|---|---|---|
| F-12 / O12 | Control events (peer / status / sync) can still drop. The backlog holds at most `BACKLOG_CAP` = 8,192 events, and status/sync payload events at most `BACKLOG_BYTES` = 64 MiB (`service/event_sink.rs::admit_backlog`); past either bound the event is dropped and counted (`doli_event_lane_shed_total{lane="backlog"}`, probe `channel control_dropped=`). | O12 (on overflow drop only re-derivable events) is NOT approved and undesigned. |
| INC-I-238 (G1) | An attestation signs only `(block_hash, slot)`: changing `height`, `attester_weight` or `bls_signature` of a member-signed attestation mints a new message id. Votes: JSON field order / whitespace variants. | Relay half CLOSED for attestations (M6, AUDIT-P1-506 residual): the verdict Accepts only a first-seen `(attester, slot)` or the first copy with a valid BLS half for that `(attester, slot)`, recorded in `AttestSeen` (never read from pool residency); ≤ 2 Accepts per `(attester, slot)`. Wire half OPEN (the signature still excludes those fields). Vote variants OPEN (AUDIT-P1-507). |
| F2-bucket | Attestations have their own per-peer bucket (20/s, burst 200; M5 decision C), so a vote or producer flood no longer starves them. No global attestation bucket and no BLS pairing cap yet. One forwarder that relays more than 100 attesters per 10 s slot can exceed the per-peer bucket (up to 2 forwards per `(attester, slot)` × attesters per 10 s slot vs 20/s; same 20/s as before M5). | Owed: global attestation bucket + BLS pairing cap (`rate_limit.rs`). |
| M5-DEV-F1 | One member key could sign `(attester, slot)` for any future slot; each was first seen, so Accepted, forwarded and kept in `AttestSeen`. | CLOSED (M5 decision C): the swarm Ignores an attestation with slot > wall + 2 (`staleness.rs::ATTEST_FUTURE_SLOTS`). Risk: a relaying node whose clock lags ≥ 2 slots (20 s at 10 s slots) Ignores genuine current attestations. |
| G3 | The digest reply delta is a producer-topic publish to all peers, not a direct reply to the forwarder. | Owed: request-response reply. |
| AUDIT-P1-505 reach | M5 gave every zero-flow tx `Ignore`, so a genuine DelegateBond / RevokeDelegation / AddMaintainer / RemoveMaintainer reached only the origin's direct peers. | CLOSED (M8): admission refuses a forged one (signature or current-set multisig, as apply), so a genuine one relays multi-hop again; a zero-flow Registration stays never-forward. Residual: replay of a captured genuine tx (INC-I-246). |
| F-15 | A transient `Ignore` can strand a valid chained or late received tx (no hop re-forwards it). Own-origin txs are covered by RPC Busy (R3). | Owed: O8 (origin re-publish of own unmined txs) or an RPC-visible status. |
| REQ-GRV-016 remainder | Not built: command-channel depth gauge, pending-verdict gauge, vendor `send_queue` drop metric (today counted inside the vendor handler and logged at WARN, not exported), 1/min INFO summary line. | Owed. |
