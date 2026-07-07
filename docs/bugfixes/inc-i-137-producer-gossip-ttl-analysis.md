# INC-I-137 — Producer-announcement gossip TTL filter (INC-I-120 Layer 3)

## Summary
Producer announcements on `PRODUCERS_TOPIC = "/doli/producers/1"` are re-forwarded
with **unconditional `MessageAcceptance::Accept`**. With `validate_messages=true`,
Accept = "forward to mesh peers". Only block-body topics get staleness
classification (`classify_block_gossip`); every other topic — including producers
— is Accepted before decode (`behaviour_events.rs:232-245`). Stale full-set
snapshots (announcements up to ~9 days old) therefore re-circulate forever,
dominating log/bandwidth volume during the 06/25-28 mainnet collapse
(~8.5M lines/day, n1.log.1 = 7.68 GB/day).

## Root cause (verified against code — not re-derived)
- `crates/network/src/service/behaviour_events.rs:75-77` — `is_block_body_topic`
  gate; only block topics reach `classify_block_gossip`.
- `behaviour_events.rs:232-245` — all non-block topics get hardcoded
  `report_message_validation_result(..., Accept)` **before** the per-topic decode
  at 248-419. So the forward decision is made with zero staleness awareness.
- INC-I-114 (`gossip/validation.rs`) added the block-topic staleness gate but
  never covered the producer topic. This is the third unhardened re-forward path
  (blocks = INC-I-114, sync = INC-I-120 L1/2, producers = INC-I-137).

## Architecture context — the CRDT trap and why the fix is safe
Producer announcements are a **grow-only CRDT (GSet)** used for producer
*discovery / bootstrap*, NOT consensus state. Verified data flow:

1. Gossip → `NetworkEvent::ProducerAnnouncementsReceived` → `on_producer_announcements`
   (`network_events.rs:428`) → `producer_gset.merge(anns)`.
2. `ProducerAnnouncement` (`discovery/announcement.rs:33`) carries a **signed
   `timestamp` (u64) and monotonic `sequence`**.
3. GSet **merge already rejects** any announcement older than
   `MAX_ANNOUNCEMENT_AGE_SECS = 3600` (`discovery/mod.rs:48`, error
   `StaleAnnouncement`).
4. Consensus only ever reads the GSet as a **genesis/bootstrap fallback**, and
   only when the on-chain `active_producers_at_height` set is empty:
   - `validation_checks.rs:27-40` — GSet consulted only if on-chain active set
     is empty / lacks the block producer (pre-registration).
   - `production/scheduling.rs:264-279` — GSet used only when on-chain set empty
     (genesis blocks 1-360).
   Both read via `gset.active_producers(7200)`, which serves only announcements
   the GSet **already merged while they were fresh** (<1h).

### Convergence-neutral rule (the key insight)
A producer message whose **newest** embedded timestamp is older than
`MAX_ANNOUNCEMENT_AGE_SECS` is a message **every node's merge would reject in
full** — it can change no node's GSet, so forwarding it accomplishes nothing.
Therefore:

> On `PRODUCERS_TOPIC`, `Ignore` (suppress re-forward, no peer penalty) a
> decoded non-empty `ProducerSet` iff **all** its announcements are older than
> `MAX_ANNOUNCEMENT_AGE_SECS`. Everything else Accepts+forwards.

- A message with **any** within-TTL announcement → newest ts is fresh → Accept
  → genuinely-new producers always propagate exactly once (CRDT convergence
  preserved). Mixed snapshots (some fresh) still forward.
- Timestamp-less formats (bloom **digest** delta-sync, legacy `Vec<PublicKey>`)
  and undecodable-as-set bytes → **fail-open Accept** (digest is the designed
  new-node convergence path; it is content-bounded so gossipsub's 60s dedup caps
  its volume).
- `now_unix == 0` (clock unavailable) → fail-open Accept (mirrors
  `classify_block_gossip` genesis_time=0 handling).

Because a suppressed message is one no node could merge, the filter cannot change
any node's `active_producers(7200)` view → **cannot change consensus** and is
**bit-safe in a mixed fleet** (a filtered node only drops re-forwards that gain
nothing; nothing mergeable is ever suppressed).

## Deploy safety (MEMORY.md #0b)
- **Q1 — consensus RULES changed? NO.** Producer-set membership for
  scheduling/validation derives from on-chain `Register` txs; the GSet is a
  discovery fallback that only serves already-merged (<1h) entries. The filter
  only suppresses forwarding of messages every node's merge rejects → no
  activation height required.
- **Q2 — block CONTENT changed? NO.** Inbound-gossip forwarding filter only;
  each node independently decides its own re-forwarding. Rolling-deploy safe,
  degrades gracefully in a mixed fleet.

No version bump (behavioral change → patch bump only, with explicit approval).

## Fix (SSF — single mechanism, ~10-line handler change + 1 pure fn)
1. `crates/network/src/gossip/validation.rs`: add pure
   `classify_producer_gossip(data: &[u8], now_unix: u64) -> MessageAcceptance`
   + `PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS` (aligned to core's
   `MAX_ANNOUNCEMENT_AGE_SECS = 3600`).
2. `behaviour_events.rs`: for non-block topics, compute `acceptance` —
   `classify_producer_gossip(&data, now_unix_secs())` for `PRODUCERS_TOPIC`,
   else `Accept` — report it, and early-return on non-Accept (mirrors the block
   path). No change to the per-topic decode/dispatch below.

## Invariant
INV-NETWORK-003 (new): "Gossip topics with unbounded re-forward risk MUST apply
staleness/dedup before `report_message_validation_result(Accept)`." Closes the
shape shared by INC-I-114 (blocks), INC-I-120 (sync), INC-I-137 (producers).

## Blast radius
- `crates/network/src/gossip/validation.rs` (+1 pure fn, +1 const, unit tests)
- `crates/network/src/service/behaviour_events.rs` (producer-topic branch only)
- New: `crates/network/tests/inc_i_137_producer_gossip_ttl.rs`
No cross-crate signature changes. No storage/serialization changes.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, verified against code — root cause confirmed at
behaviour_events.rs:232-245; consensus boundary confirmed at validation_checks.rs
+ scheduling.rs; convergence rule aligned to existing MAX_ANNOUNCEMENT_AGE_SECS)
Reasoning: Localized (1 pure fn + 1 handler branch), deterministic, pre-diagnosed
and code-verified; no architectural issues, no consensus rule change.
━━━━━━━━━━━━━━━━━━━━━━
