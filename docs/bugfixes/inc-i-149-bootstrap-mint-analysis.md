# INC-I-149 — Analyst: Bootstrap mint on empty data-dir

**Agent:** analyst (step 1 of `/omega-doctor`)
**Date:** 2026-08-04
**Branch:** main
**Incident:** INC-I-149, domain `node/production+sync`, severity high, protection_level 3
**Status of root cause:** CONFIRMED by controlled experiment (upstream) — NOT re-investigated here.

---

## Scope

Read (full or targeted): `bins/node/src/node/production/{mod,scheduling,gates}.rs`,
`bins/node/src/node/{init,startup,event_loop,network_events}.rs`,
`crates/core/src/network/economics.rs`, `crates/core/src/network_params/defaults.rs`,
`crates/network/src/sync/manager/{mod,peers,production_gate}.rs`,
`crates/core/src/validation/{producer,registration}.rs`,
`bins/node/src/node/apply_block/state_update.rs`.
Forensic evidence: `~/testnet/logs/n13.log` (117 lines, preserved un-wiped).

Out of scope: the snap-sync layer itself (proven healthy by the controlled experiment's
`without --producer` arm), block validation, epoch/rewards.

---

## Summary (plain language)

A producer restarted on an empty data directory has no way to tell "the whole network is
brand new" from "my own disk is empty". Both look like *height 1*. Every guard that
would stop it from minting a block is switched off in the first case, so it mints one
block of its own, then snap-syncs onto the real chain and leaves that block behind as a
permanent fossil that no other node has.

Exactly one observable distinguishes the two situations: **what the peers say their
height is**. There is already a guard in the production path that reads exactly that
value — and it is written to cover exactly this case, in its own comment — but it is
excluded from running at height 1 by a single `height > 1` clause.

---

## A. Architecture comprehension — production decision path

### Module boundaries

| Module | Responsibility | Depends on | Depended on by |
|---|---|---|---|
| `bins/node/src/node/event_loop.rs` | Drives the 1 Hz production timer + event drain | `production/mod.rs` | — (top of loop) |
| `bins/node/src/node/production/mod.rs` | `try_produce_block()` — the whole mint decision + build + apply + broadcast | `production/gates.rs`, `production/scheduling.rs`, `production/assembly.rs`, `sync_manager` (read), `chain_state`, `producer_set`, `block_store` | `event_loop.rs` |
| `bins/node/src/node/production/gates.rs` | Thin adapter: `handle_production_authorization()` → `SyncManager::can_produce()` | `crates/network` `SyncManager` | `production/mod.rs` |
| `bins/node/src/node/production/scheduling.rs` | `resolve_bootstrap_eligibility()` (bootstrap round-robin + joining-node guards) and `resolve_epoch_eligibility()` | `sync_manager` (read), `producer_set`, `producer_gset`, `known_producers` | `production/mod.rs` |
| `crates/core/src/network/economics.rs` | `is_in_genesis(height)` — pure predicate over the *supplied* height | `network_params` | 13 non-test call sites in 9 files, **including consensus validation** |
| `crates/core/src/network_params/defaults.rs` | `genesis_blocks` per network | — | `economics.rs` |
| `crates/network/src/sync/manager/production_gate.rs` | `can_produce()` — the "single source of truth" gate (4 checks) | `peers.rs` state, `RecoveryPhase` | `production/gates.rs` |
| `crates/network/src/sync/manager/peers.rs` | Peer table; `add_peer` / `peer_count()` / `best_peer_height()` | — | production gate, sync engine, node layer |

**Direction:** `bins/node` (production) → `crates/network` (sync manager) → peer table.
Production never writes the peer table; it only reads it. There is **no** reverse edge.

### Data flow: timer fire → block built

```
event_loop.rs:14   production_timer = interval(1s)   [Devnet: 200ms]
event_loop.rs:101  tick → drain pending network events → try_produce_block()
  (also event_loop.rs:68-97 escape hatch: force a production check once per interval
   even under continuous event load)

production/mod.rs:31  try_produce_block()
  ├─ read producer_key                                     (local)
  ├─ gates.rs → SyncManager::can_produce(slot)             (peer/sync state)
  ├─ read chain_state → prev_hash, prev_slot, height=best_height+1   (LOCAL DISK)
  ├─ read sync_manager.best_peer_height()                  (PEER-REPORTED)
  ├─ read producer_set.active_producers_at_height(height)  (LOCAL DISK)
  ├─ is_in_genesis(height)                                 (LOCAL height only)
  ├─ bootstrap ? resolve_bootstrap_eligibility()           (scheduling.rs)
  │  : resolve_epoch_eligibility()
  ├─ eligibility window / propagation delay / signed-slots
  ├─ build_block_content() → VDF → apply_block(Light)      (MUTATES LOCAL STATE)
  └─ broadcast_header + broadcast_block                    (network)
```

The critical structural fact: **`apply_block()` happens before broadcast**
(`production/mod.rs:590` vs `:673`). The fossil block is written to the local block store
even if the broadcast is later suppressed. Any fix that only suppresses *broadcast*
(e.g. the `BEHIND_TIP_SUPPRESS` at `mod.rs:661`) does **not** prevent the fossil.

### Timer cadence (task B, question 4)

`bins/node/src/node/event_loop.rs:9-14`:

```rust
let production_interval = if self.config.network == Network::Devnet {
    Duration::from_millis(200)
} else {
    Duration::from_secs(1)
};
let mut production_timer = tokio::time::interval(production_interval);
```

**1 Hz on testnet/mainnet, 5 Hz on devnet.** `tokio::time::interval` completes its first
tick immediately, so `try_produce_block()` runs at event-loop entry, before any peer has
necessarily connected.

### Ordered guard map — every guard on the path, in order

Scenario column = producer with `bootstrap_nodes` configured, empty data dir, on an
84 k-block chain (the defect scenario, as measured on n13).

| # | Site | Keys on | Reachable when `in_genesis==true` / `height==1`? | Fired on n13? |
|---|---|---|---|---|
| G0 | `mod.rs:32` no producer key | local key | yes | no |
| G1 | `mod.rs:40` `is_production_allowed` | `pending_update.json` | yes | no |
| G2 | `mod.rs:62` hardfork stop-producing | local height + binary version | yes | no |
| G3 | `mod.rs:97` `last_produced_slot == current_slot` | in-process memory | yes | no (fresh process) |
| **G4** | `mod.rs:102` `handle_production_authorization` → `can_produce` | see sub-map | yes | **no — all 5 sub-checks passed** |
| G5 | `mod.rs:119` `has_block_for_slot \|\| seen_blocks_for_slot` | local store + gossip-seen set | yes | no (empty store) |
| G6 | `mod.rs:137` `current_slot <= prev_slot` | local `best_slot` (=0) | yes | no |
| **G7** | **`mod.rs:174` `if height > 1 && network_height_ahead`** | **peer-reported height** *and* local height | **NO — `height > 1` is false at height 1** | **would have fired: blocks_behind = 84 505 > max_behind 3** |
| G8 | `mod.rs:226` `height<10 && !in_genesis && active>5` | `in_genesis` + local height | **NO** (`!in_genesis`) | n/a |
| G9 | `mod.rs:245` `use_bootstrap = in_genesis \|\| active_with_weights.is_empty()` | `in_genesis` **OR** empty producer set | routes to BOOTSTRAP; **true on both counts** for a wiped disk | routed to bootstrap |
| G10 | `mod.rs:272` `should_defer_epoch_production()` (INC-I-053) | `first_peer_connected` elapsed | **NO — sits on the `else`/epoch branch only** | n/a |
| G11 | `scheduling.rs:37` producer-list stability (15 s testnet / 3 s devnet) | `last_producer_list_change` | yes | delayed, did not block |
| **G12** | **`scheduling.rs:56` `if has_bootstrap_nodes && !in_genesis {`** | **`in_genesis`** | **NO — gates G12a–G12d, closing at `:179`** | — |
| G12a | `scheduling.rs:62` `peer_count == 0` | peer table (status-derived) | NO (inside G12) | would NOT have fired (peers=1) |
| G12b | `scheduling.rs:129` `height<3 && best_peer_height>0 && !timeout` | peer height | NO (inside G12) | **would have fired** |
| G12c | `scheduling.rs:157` `height>0 && within_grace && slot_gap>1 && !timeout` | local `best_slot` vs wall clock | NO (inside G12) | would have fired |
| G12d | `scheduling.rs:172` `best_peer_height>0 && height+2 < best_peer_height` | peer height | NO (inside G12) | **would have fired** |
| G13 | `scheduling.rs:210/232` discovery grace (`known_count<=1 && grace_active`) | known producers, `first_peer_connected` | yes | no — n13 knew 12 producers |
| G14 | `scheduling.rs:334` `num_producers == 0` | known list | yes | no |
| G15 | `scheduling.rs:388` `!eligible.contains(our_pubkey)` | bootstrap round-robin over known producers | yes | passed in its rank-0 slot |
| G16 | `mod.rs:335-349` `is_producer_eligible_ms` window | slot offset | yes | passed |
| G17 | `mod.rs:377` rank-1 fallback guard | `if !use_bootstrap` | **NO — bootstrap path exempt** | n/a |
| G18 | `mod.rs:418` propagation delay (`min_offset_ms` 1000) | slot offset | yes | passed after 1 s |
| G19 | `mod.rs:497` signed-slots `check_and_mark` | local db | yes | passed (fresh db) |
| G20 | `mod.rs:529/540` post-VDF re-check | local store/state | yes | passed |
| G21 | `mod.rs:661` `net_tip >= height+2 && peer_count >= 3` `BEHIND_TIP_SUPPRESS` | peer height + peer count | yes — **but POST-`apply_block`** | did not fire (peer_count 1 < 3); would not have prevented the fossil anyway |

**`SyncManager::can_produce()` sub-map** (`crates/network/src/sync/manager/production_gate.rs`):

| # | Site | Keys on | Fired on n13? |
|---|---|---|---|
| G4a | `:52` explicit `production_blocked` | manual / invariant violation | no |
| G4b | `:59` `state.is_syncing() \|\| ResyncInProgress` | sync state | **no — state was `Idle` at boot** (n13.log 22:04:00 `state="Idle"`) |
| G4c | `:68` `RecoveryPhase::AwaitingCanonicalBlock` (INC-I-089 / INV-CONSENSUS-089) | post-restart lockout | **no — `engage_post_restart_lockout()` is skipped when `best_height == 0`**, `init.rs:748-754` |
| G4d | `:79` bootstrap-phase quorum | requires `first_peer_status_received.is_some()` **and** `!has_chain_activity` | **no — `has_chain_activity` was TRUE (best_peer_h 84 505), so the whole quorum branch is skipped** |
| G4e | `:130` minimum peers | `peers.len() < min_peers_for_production`, with `genesis_bypass = local_height==0 && min_peers<=1` | **no — `min_peers` was 1** (`init.rs:726-735`) and `peers.len()==1`; `genesis_bypass` would have waived it even at 0 peers |

### Architectural constraints / invariants

- **INV-CONSENSUS-089** (`.omega/memory.db`): "On producer restart with `state.best_height > 0`,
  the production gate MUST engage `RecoveryPhase::AwaitingCanonicalBlock` BEFORE the event loop
  starts." The invariant is written with `best_height > 0` in its own statement — the empty-disk
  case is *outside* its scope by construction. Not violated; **not covering the defect either**.
- **`is_in_genesis` is a consensus predicate.** `crates/core/src/validation/producer.rs:248`
  selects `validate_bootstrap_producer` vs the epoch round-robin; `crates/core/src/validation/registration.rs:37`
  selects the genesis registration rule set. Its value MUST be a deterministic function of
  chain data alone (INV-CONSENSUS-001 territory).
- **`apply_block` precedes broadcast.** Suppressing broadcast does not suppress the fossil.
- **Production never mutates the peer table.** Any new guard reading peer state is a pure read.

---

## B. Sync-manager semantics (the crux) — answered with quoted code

### B1. What does `best_peer_height()` return when no peer status has been received?

`crates/network/src/sync/manager/peers.rs:193-202`:

```rust
pub fn best_peer_height(&self) -> u64 {
    let peer_max = self
        .peers
        .values()
        .map(|p| p.best_height)
        .max()
        .unwrap_or(0);
    // Return the higher of peer data or network gossip tip
    peer_max.max(self.network.network_tip_height)
}
```

**Returns 0.** Empty `peers` map → `unwrap_or(0)`; `network.network_tip_height` starts at 0
(`SyncManager::new`, `mod.rs:212` → `NetworkState::new()`) and is only ever raised from peer
status (`peers.rs:47-53`, `:82-87`) or gossip. So `best_peer_height() == 0` is **ambiguous**:
it means *either* "the network is at genesis" *or* "I have heard from nobody".

### B2. What does `peer_count()` count?

`peers.rs:181-183`:

```rust
/// Get the number of connected peers with known status
pub fn peer_count(&self) -> usize {
    self.peers.len()
}
```

`self.peers` is inserted into **only** by `add_peer()` (`peers.rs:29`), and `add_peer` has
exactly **one** non-test call site: `bins/node/src/node/network_events.rs:226`, inside
`on_peer_status()` — i.e. on receipt of a `StatusResponse`.

**`peer_count()` counts peers whose STATUS RESPONSE has been received**, not transport-connected
peers. A node can be TCP-connected to 12 peers and still report `peer_count() == 0`.

### B3. When is `first_peer_connected` set on `Node`, relative to peer status arrival?

`bins/node/src/node/network_events.rs:22-27`, inside `on_peer_connected()`:

```rust
if self.first_peer_connected.is_none() {
    self.first_peer_connected = Some(Instant::now());
    info!("First peer connected - starting discovery grace period");
}
self.sync_manager.write().await.set_peer_connected();
```

`on_peer_connected` handles the transport-level `PeerConnected` event and *then* sends the
status request (`:43`). So **`first_peer_connected` is set STRICTLY BEFORE any status arrives.**
`SyncManager::set_peer_connected()` is a no-op (`production_gate.rs:372-376`, it only logs) —
the sync-manager-side bootstrap gate is driven by `first_peer_status_received`, set in
`note_peer_status_received()` (`production_gate.rs:382-390`), called from `on_peer_status`
(`network_events.rs:234`).

**Ordering guarantee:** `first_peer_connected` (Node) ≤ `first_peer_status_received` (SyncManager)
= first `peer_count() > 0` = first `best_peer_height() > 0` (for a live chain).
A guard keyed on `first_peer_connected` is therefore *weaker* evidence than one keyed on
`best_peer_height`.

### B4. Production-timer cadence

1 Hz (testnet/mainnet), 5 Hz (devnet) — `event_loop.rs:9-14`, quoted in §A. Plus the
escape-hatch invocation at `event_loop.rs:68-97`. First tick is immediate.

### B5. Is a `best_peer_height`-keyed guard reachable at mint time, or does it lose the race?

**Settled by measurement, not inference.** `~/testnet/logs/n13.log` (WARN-only capture,
117 lines, preserved):

```
22:03:50.946  --force-start specified: skipping duplicate key detection      [process start]
22:03:51.048  Failed to dial /ip4/127.0.0.1/tcp/30300: Dial error
22:04:00.048  [HEALTH] h=0 s=0 hash=f6cc888a… | peers=1 best_peer_h=84505 best_peer_s=884343
              net_tip_h=84505 net_tip_s=884343 | sync_fails=0 state="Idle"
22:04:21.117  Empty headers from 12D3KooWS6pt… (peer_h=84508, local_h=1, gap=84507, consecutive=1)
22:04:30.050  [HEALTH] h=1 s=884346 hash=c5efd7e0… | peers=1 best_peer_h=84508 …
```

At **22:04:00** the node was still at `h=0` and already had `peers=1`, `best_peer_h=84505`.
The mint happened between 22:04:00 and 22:04:21.

**Conclusion: at mint time `best_peer_height()` was ≥ 84 505 and `peer_count()` was 1.**
A guard keyed on `best_peer_height` is reachable at mint time and *would have fired*.
The pre-status race is **not** the observed mechanism.

*(Secondary, INFERRED:* the window before first status is additionally narrowed by G11 —
`last_producer_list_change` is set at startup when the node registers itself as a bootstrap
producer, `startup.rs:72`, and is reset by **every** producer discovered via peer status or
gossip, `network_events.rs:295`. Bootstrap eligibility therefore cannot fire until 15 s
(testnet) after the last producer discovery, and producer discovery is driven by the same
peer traffic that populates the peer table. This is a *correlation*, not a guarantee.)

---

## C. Blast radius

### Graph-first (Rule 28)

```
GRAPH=graphify-out/graph.json   (19.5 MB, built 2026-08-04 23:20)
python3 .claude/scripts/blast.py "$GRAPH" is_in_genesis --hops 2
  → 1 dependent, with the tool's own warning:
    "graphify does not resolve cross-file receiver-method calls for this language
     (Graphify-Labs/graphify#2234) — this count is a LOWER BOUND"
```

Same 1-dependent lower bound for `try_produce_block`, `resolve_bootstrap_eligibility`,
`best_peer_height`; 2 for `can_produce` (one of which — `crates/storage/src/producer/info.rs:47`
`ProducerInfo::can_produce` — is a **name collision, not a dependent**). The Rust
`self.method()` blind spot is the documented limitation in
`~/.claude/.../reference_graphify_rust_method_blind_spot.md`. Grep is the ground truth here,
and I am labelling it as such.

### Option 1 — change the FUNCTION `is_in_genesis` (network-relative predicate)

Grep, non-test call sites — **13 sites in 9 files**:

| File:line | What it gates | Consensus-visible? |
|---|---|---|
| `crates/core/src/validation/producer.rs:248` | selects `validate_bootstrap_producer` vs epoch round-robin for **block validation** | **YES** |
| `crates/core/src/validation/registration.rs:37` | selects genesis registration rule set (no bond, no chain validation) | **YES** |
| `bins/node/src/node/apply_block/tx_processing.rs:207` | genesis path inside `apply_block` | **YES** |
| `bins/node/src/node/apply_block/state_update.rs:6` | `update_known_producers` gating | indirect (round-robin denominator) |
| `bins/node/src/node/production/assembly.rs:36` | block-content assembly | **YES (block content)** |
| `bins/node/src/node/production/assembly.rs:112` | block-content assembly | **YES (block content)** |
| `bins/node/src/node/production/mod.rs:208` | `in_genesis` for the production decision | production only |
| `bins/node/src/node/validation_checks.rs:578` | producer eligibility check | **YES** |
| `bins/node/src/node/init.rs:728` | `min_peers_for_production` selection | production only |
| `bins/node/src/node/startup.rs:29`, `:101`, `:553` | startup + `recompute_active_status` | production only |
| `bins/node/src/node/network_events.rs:277`, `:398` | bootstrap producer discovery | production only |
| `bins/node/src/node/event_loop.rs:161` | periodic bookkeeping | production only |

**Verdict: STRUCTURALLY INVALID, not merely expensive.** `is_in_genesis` selects which
*validation rule set* applies to a block. Making it depend on `best_peer_height` — a per-node,
time-varying, non-deterministic value — makes block validation non-deterministic across nodes.
Two honest nodes would disagree on whether the same block is valid. That is a guaranteed fork,
and no activation height can rescue it because the divergent input is not chain data.
**Disqualified on correctness.**

### Option 2 — change only the PRODUCTION guard

Grep, callers of the affected function:

```
try_produce_block()  → bins/node/src/node/event_loop.rs:88, :123   (2 call sites, both in the event loop)
```

The guard at `production/mod.rs:167-189` is **inline code inside `try_produce_block`** — it has
no callers of its own, no public surface, and no other module reads it. `best_peer_height()` is
already read two lines above it (`mod.rs:167-170`); no new dependency edge is created.

- **Direct impact:** `bins/node/src/node/production/mod.rs` (1 file, 1 function).
- **Indirect impact:** none. Nothing downstream consumes the *decision*; the only consequence
  is that `try_produce_block` returns `Ok(())` earlier on one input class.
- **Consumers of the suppressed artifact:** none — the suppressed block is a local orphan the
  fleet already rejects (measured, n13.log 22:04:22: `header chain broken … Peer has different
  chain at our tip`).

**Quantified: Option 1 = 13 sites / 9 files / 2 crates / consensus-breaking. Option 2 = 1 site / 1 file / 1 crate / production-local.**

---

## D. Genesis-preservation analysis (hard constraint: INC-I-115 must not break)

### `genesis_blocks` per network — `crates/core/src/network_params/defaults.rs`

| Network | `genesis_blocks` | Line | `is_in_genesis(h)` true for |
|---|---|---|---|
| Mainnet | **360** | `defaults.rs:46` (block starting `Network::Mainnet =>` at `:18`) | h ≤ 360 |
| Testnet | **36** | `defaults.rs:294` (`Network::Testnet =>` at `:264`) | h ≤ 36 |
| Devnet | **40** | `defaults.rs:450` (`Network::Devnet =>` at `:426`) | h ≤ 40 |

Non-mainnet is env-overridable: `env_loader.rs:86-89`, `DOLI_GENESIS_BLOCKS`; mainnet is LOCKED.

**Consequence worth stating:** a wiped producer is `in_genesis` for its *first 36 (testnet) /
360 (mainnet) self-minted blocks*, not just one. The observed damage is capped at exactly one
block **only** because G7's `height > 1` clause becomes true at height 2 and the very same
guard then defers forever (`blocks_behind = 84 505 > max_behind = 5`). This mechanistically
explains why both n12 and n13 produced exactly **one** fossil block — a fact the incident record
notes but does not explain.

### What a genuinely-fresh-genesis node experiences

All nodes start together; nobody has blocks; every peer's `StatusResponse.best_height == 0`;
`network.network_tip_height` is initialised to 0 and only raised by peer status/gossip.
Therefore at real genesis, on every node: **`best_peer_height() == 0` and `network_tip_height == 0`.**

### Candidate-by-candidate genesis evaluation

| Candidate | Predicate | Genesis behaviour | Verdict |
|---|---|---|---|
| **A. `mod.rs:174` — drop `height > 1`** | `network_tip_height > height-1` | at h=1: `0 > 0` = **false** → guard cannot fire. Zero added delay, no timer, no new state. Once one genesis node produces block 1, a peer at h=0 sees `blocks_behind = 1 ≤ max_behind 3` → still produces. Only defers when the network is already ≥4 blocks ahead — where deferring is *correct* even at genesis. | **GENESIS-SAFE BY CONSTRUCTION** |
| B. `scheduling.rs:56` — drop `&& !in_genesis` | re-enables G12a–G12d | G12b/G12d require `best_peer_height > 0` → inert at genesis (safe by construction). **But G12c** (`slot_gap > 1` during `within_bootstrap_grace`) fires: at genesis `chain_tip_slot == 0` while `current_slot` is wall-clock-derived (INC-I-115 had a genesis_time ~54 days in the past → `current_slot` enormous), so `slot_gap > 1` is true and production is deferred for the whole 90 s bootstrap grace (15 s devnet). G12a (`peer_count == 0`) also blocks a genuinely solo bootstrap until first status. | **GENESIS-SAFE ONLY BY TIMEOUT** (adds up to 90 s) |
| C. network-relative `is_in_genesis` | peer height inside a consensus predicate | non-deterministic block validation → fork | **INVALID** (see §C) |
| D. `init.rs:754` — engage `AwaitingCanonicalBlock` at h=0 too | clears on first peer gossip block extending tip, else 60 s cleanup timeout | at real genesis **nobody can produce block 1**, so every node waits the full 60 s and unlocks *simultaneously* → synchronised competing block-1 race = exactly INC-I-115 | **ACTIVELY HARMFUL** |
| E. `production_gate.rs:124` — drop `genesis_bypass` | `local_height == 0 && min_peers <= 1` | same disk-relative flaw; forbids a genuinely solo genesis bootstrap (single-node devnet); and **would not have fired on n13** (`peers.len()==1 ≥ min_peers 1`) — it does not close the measured path at all | **DOES NOT CLOSE THE HOLE** |

**Genesis-safe BY CONSTRUCTION (keyed only on peer-reported height, which is 0 at real genesis):**
Candidate A, and the G12b/G12d sub-guards of Candidate B.
**Genesis-safe only BY TIMEOUT:** Candidate B as a whole (via G12c), Candidate D.

---

## Recommendation (SSF — one fix, presented alone)

**The simplest fix that resolves the root cause: delete the `height > 1 &&` clause at
`bins/node/src/node/production/mod.rs:174`, so the existing peer-aware behind-network check
also applies at height 1.** This works because the *only* observable that distinguishes
"network at genesis" from "my disk is empty" is peer-reported height, and that guard is the one
place on the production path that already reads it — it was written for exactly this case and
excludes it by accident.

Current code (`production/mod.rs:145-189`, comment abridged):

```rust
// DEFENSE-IN-DEPTH: Peer-Aware Behind-Network Check
//
// Prevents producing orphan blocks when we're significantly behind the
// network. A node at height 0 should never produce for slot 92 if peers
// are at height 90 — the block would be an orphan.
…
let network_tip_height = { let sync = self.sync_manager.read().await; sync.best_peer_height() };
let network_height_ahead = network_tip_height > height.saturating_sub(1);

if height > 1 && network_height_ahead {          // <-- `height > 1` excludes the case
    let blocks_behind = network_tip_height.saturating_sub(height.saturating_sub(1));
    let max_behind: u64 = if height < 10 { 3 } else { 5 };
    if blocks_behind > max_behind {
        debug!("Behind network by {} blocks …");  // <-- debug!, invisible on the fleet
        return Ok(());
    }
}
```

The guard's own comment says *"A node at height 0 should never produce for slot 92 if peers are
at height 90"* — and `height > 1` excludes precisely that node (empty disk ⇒ `height == 1`).
This is not a symptom patch bolted on top of a broken guard; it removes an incorrect exclusion
from the correct guard. The root cause — a disk-relative predicate standing in for a
network-relative fact — is resolved at the one decision point that has the network-relative
fact in hand.

**Why the others lose** (one line each, full analysis in §D):

- **B (`scheduling.rs:56`)** — costs up to 90 s of added delay at real genesis via the
  tip-freshness sub-guard; genesis-safe only by timeout; and it does not cover a wiped **seed**
  (no `bootstrap_nodes` ⇒ the whole block is skipped) nor the epoch branch.
- **C (`is_in_genesis`)** — makes consensus validation non-deterministic; guaranteed fork.
- **D (`AwaitingCanonicalBlock` at h=0)** — deadlocks real genesis for 60 s and then releases
  every node at once, reproducing INC-I-115.
- **E (`genesis_bypass`)** — would not have fired on the measured incident (`peers.len()==1 ≥ min_peers 1`).

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — the `best_peer_height()` read already happens unconditionally at production/mod.rs:167-170; only a boolean clause is removed)
  Memory:   0 (observed — no new state, no allocation)
  IO:       0 (observed — strictly FEWER block-store writes: one suppressed apply_block per wiped-producer boot)
  Network:  -1 block broadcast per wiped-producer boot (observed — the suppressed block is never gossiped)
  Disk:     -1 block + undo data per wiped-producer boot; +1 WARN log line per deferred slot (observed)
  Latency:  0 at real genesis (observed — the guard's predicate is `0 > 0` = false when all peers report height 0, so it cannot fire); +N slots for an empty-disk node on a live chain, which is the intended behaviour
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: it deletes a clause; there is no cheaper change than removing code, and every other candidate location costs strictly more (added genesis delay, consensus non-determinism, or no coverage of the measured path).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### Deploy-safety questions — ARGUED, not assumed

**(a) Does this change consensus RULES → activation height?** **NO.**
The change only makes a producer *decline to build* a block. It does not change what any node
*accepts*: `crates/core/src/validation/producer.rs`, `validation_checks.rs`, and `apply_block`
validation are untouched. Old-binary and new-binary nodes accept and reject exactly the same
set of blocks. Additionally, an activation height would be **meaningless** here — the guard
fires at *local height 1* on a node with an empty disk; there is no shared chain context at
that point in which the fleet could agree on an activation height.

CLAUDE.md three-question consensus-shape checklist:
1. Can any user-submittable transaction trigger this path? **NO.**
2. Can any producer-action or attestation pattern trigger it? **YES** — a producer restart on
   an empty data dir.
3. Is the new behaviour bit-identical for ALL reachable inputs? **NO** — it differs for
   `(height == 1, best_peer_height > max_behind)`.

(1)/(2) YES + (3) NO normally demands an activation height. The gate's purpose is computations
that change what goes **into** a block or what is **accepted from** one. This changes neither —
it changes only *whether a block that the whole fleet already rejects gets built locally*
(measured: n13.log 22:04:22 `header chain broken … Peer has different chain at our tip`).
**I am recording this as an argued exemption, not a silent skip; the architect must re-derive
it independently and the reviewer must challenge it.**

**(b) Does it change block CONTENT → synchronized deploy?** **NO.**
Header fields, coinbase shape, tx ordering, attestation bitfield, `presence_root`, `data_root`,
`fork_id` are all untouched — `production/assembly.rs` is not modified. During a rolling deploy,
an old-binary node and a new-binary node can differ only on whether an empty-disk node mints a
block that every peer rejects. No competing *valid* blocks are created. **Rolling deploy is
safe; no synchronized stop-all required.** (INC-I-062 discipline satisfied by argument.)

### Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details:
  1. Cross-module blast radius — NO. The fix is one clause in one function in one file.
     (The DEFECT class spans 5 sites in 3 crates, but the fix does not.)
  2. Invariant gaps — YES. No module owns "in_genesis must mean network-at-genesis, not
     disk-empty". INV-CONSENSUS-089 explicitly scopes itself to best_height > 0.
  3. Data flow reversal — NO. The fix reads a value already read two lines above.
  4. Shared mutable state without an owner — NO. is_in_genesis is a pure function derived
     independently at each call site.
  5. Contract absence — YES. production/mod.rs:270 asserts "Bootstrap mode already has its
     own guards (scheduling.rs:90-167)" — an unwritten, and false, cross-module contract.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## E. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|---|---|---|
| REQ-PROD-001 | A producer MUST NOT build a block at local height 1 when peer-reported network height exceeds `height-1` by more than the existing `max_behind` threshold | Must | see detail |
| REQ-PROD-002 | Real fresh-genesis bootstrap MUST be preserved with ZERO added delay | Must | see detail |
| REQ-PROD-003 | A producer SHOULD NOT build a block at local height 1 when it has bootstrap nodes configured and has received no peer status | Should | see detail |
| REQ-PROD-004 | The deferral MUST be observable at a log level the fleet actually captures | Must | see detail |
| REQ-PROD-005 | A reproduction test MUST exist and FAIL before any fix code | Must | see detail |
| REQ-PROD-006 | The change MUST NOT require an activation height or a synchronized deploy, and the argument MUST be recorded | Must | see detail |
| REQ-PROD-007 | Gauntlet GS-001 (`single-block1-hash`) SHOULD pass UN-WAIVED after the fix | Should | see detail |
| REQ-PROD-008 | The node COULD detect and report a fossil block-1 mismatch at boot | Could | see detail |
| REQ-PROD-009 | Rework `is_in_genesis` to be network-relative | Won't | N/A (deferred — structurally invalid, §C) |
| REQ-PROD-010 | Retroactively remove existing fossil block-1s from already-affected nodes | Won't | N/A (deferred — cosmetic; nodes agree with the fleet at the tip) |
| REQ-PROD-011 | Close the four sibling disk-relative holes (G4c, G4e, G9, G12) | Won't | N/A (deferred — SSF; file as follow-up findings) |

### Detailed acceptance criteria

**REQ-PROD-001 — no mint at height 1 when peers are known to be ahead** *(Must)*
- [ ] Given a node with `chain_state.best_height == 0` and `sync_manager.best_peer_height() == 84_505`, when `try_produce_block()` runs, then it returns without calling `apply_block` and without broadcasting.
- [ ] Given the same node, when `best_peer_height() == 3` (blocks_behind = 3, `max_behind` = 3 for `height < 10`), then production is **not** deferred by this guard (boundary: `blocks_behind > max_behind` is strict).
- [ ] Given the same node with `best_peer_height() == 4`, then production **is** deferred.
- [ ] Edge case: `best_peer_height()` transiently reports a value below local height — `saturating_sub` must not underflow (already guaranteed by `height.saturating_sub(1)` / `saturating_sub`).
- [ ] Traceability: the behaviour must hold on all three networks (mainnet/testnet/devnet) — the guard must not be network-conditional.

**REQ-PROD-002 — real fresh genesis preserved, zero added delay** *(Must)*
- [ ] Given every peer reports `best_height == 0` and `network_tip_height == 0`, when a node at `height == 1` runs `try_produce_block()`, then this guard does **not** defer (predicate `0 > 0` is false).
- [ ] Given one genesis node has produced block 1 (`network_tip_height == 1`) and this node is still at `best_height == 0`, then this guard does **not** defer (`blocks_behind = 1 ≤ 3`).
- [ ] No timer, no grace period, no new state field is introduced by the fix — verifiable by diff inspection: the change adds **zero** new struct fields and **zero** new `Instant`/timeout reads.
- [ ] Regression guard: a test named for INC-I-115 asserts a fresh-genesis node with all-zero peer heights is authorised to produce at height 1.

**REQ-PROD-003 — no mint at height 1 before any peer evidence** *(Should)*
- [ ] Given `bootstrap_nodes` is non-empty, `peer_count() == 0`, and `best_peer_height() == 0`, when a node at `height == 1` runs `try_produce_block()`, then production is deferred.
- [ ] Given `bootstrap_nodes` is **empty** (seed / solo devnet), the same condition MUST still authorise production — a genuinely solo node must be able to bootstrap a chain.
- [ ] **Priority rationale (honest):** this is `Should`, not `Must`, because it was **not** the measured mechanism. n13 had `best_peer_h = 84 505` at least 20 s before it minted (§B5). The window is real but unobserved, and closing it requires a *second* predicate, which SSF forbids bundling into this fix. Implement only if the reproduction test in REQ-PROD-005 shows the window is reachable in practice.

**REQ-PROD-004 — observable deferral** *(Must)*
- [ ] The deferral MUST log at `warn!` level, not `debug!`. **Evidence:** `~/testnet/logs/n13.log` contains **0 `INFO` lines and 117 `WARN` lines** — the fleet's log capture is WARN-only, so the existing `debug!` at `production/mod.rs:183` is invisible to operators. This is why three reproductions were diagnosed from block hashes rather than logs.
- [ ] The log line MUST include: local `height`, `network_tip_height`, `blocks_behind`, `peer_count`, and a stable grep token (e.g. `[BOOTSTRAP_MINT_BLOCKED]`).
- [ ] The line MUST be rate-limited or emitted at most once per slot to avoid the 1 Hz spam pattern seen in `[STUCK_FORK]` (n13.log emitted it every second for 10 consecutive seconds).

**REQ-PROD-005 — test before fix (Output Contract)** *(Must)*
- [ ] A test exists that constructs the mint decision inputs (`height == 1`, `best_peer_height == 84_505`, `peer_count == 1`, bootstrap nodes configured, empty producer set) and asserts production is refused.
- [ ] The test **FAILS** on the current `main` (pre-fix) and PASSES after. The FAIL→PASS transition MUST be captured as evidence.
- [ ] A companion test asserts the genesis case (`best_peer_height == 0`) is **authorised**, and it must PASS both before and after (proving the fix is narrow).
- [ ] The controlled experiment is the spec: same empty data dir, `--producer` vs no `--producer`.

**REQ-PROD-006 — deploy-safety argued** *(Must)*
- [ ] The commit message carries the CLAUDE.md three-question checklist with Q2 = YES, and the argued exemption from an activation height.
- [ ] The commit message carries a `Failure-Modes:` block (Rule 29 — `.omega/gauntlet.conf` lists `bins/node/src/node/` as a gated domain).
- [ ] The commit message carries a `Path-Coverage:` block (Rule 24 — the change alters an early-return guard in non-test Rust).
- [ ] No `CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION`, `HardForkSchedule`, or activation-height edit is made.

**REQ-PROD-007 — gauntlet GS-001 un-waived** *(Should)*
- [ ] `scripts/gauntlet.sh` GS-001 (`fresh-genesis-boot`, assertions `convergence,no-panic,single-block1-hash`) passes with **no waiver**. It was waived-with-evidence under INC-I-139 for this exact fingerprint.
- [ ] Note: existing fossil block-1s on already-affected nodes will keep GS-001 failing until those nodes are re-snapped. The acceptance is "no NEW divergent block-1 after a wipe+restart-with-`--producer`", verified by the GS-010-style targeted scenario, not by the historical fleet state.

**REQ-PROD-008 — boot-time fossil detection** *(Could)*
- [ ] At boot, if the local block store holds a block at height 1 whose hash differs from the fleet's, emit a WARN naming both hashes.

---

## Impact analysis

### Existing code affected

- `bins/node/src/node/production/mod.rs:174` — the guard condition. **Risk: low.** One boolean clause; the surrounding computation is unchanged and already runs unconditionally.
- `bins/node/src/node/production/mod.rs:183` — log level `debug!` → `warn!` (REQ-PROD-004). **Risk: low**, but log volume must be rate-limited.

### What breaks if this changes

- **A node genuinely behind by >3 blocks at height 1 stops producing.** That is the intent. Mitigation: none needed — it will sync and then produce.
- **A single-node network whose peer reports a stale high height** could be blocked. Mitigation: `best_peer_height()` is recomputed on `remove_peer` (`peers.rs:117-125`), which explicitly re-derives `network_tip_height` from remaining peers — the known phantom-height inflation path is already closed.
- **Nothing else.** No other module reads this decision.

### Regression risk areas

- **Fresh genesis (INC-I-115).** Argued genesis-safe by construction in §D; must be covered by the REQ-PROD-002 test.
- **Devnet single-node bootstrap.** At `height == 1` with no peers, `best_peer_height() == 0` → guard inert. Unaffected.
- **Gauntlet-gated domain.** `.omega/gauntlet.conf` gates `bins/node/src/node/` — a gauntlet pass is required before workflow close.

---

## Specs drift detected

- `bins/node/src/node/production/mod.rs:270` — comment asserts *"Bootstrap mode already has its own guards (scheduling.rs:90-167). This guard only applies to epoch mode."* **Stale/false:** those guards are gated off by `scheduling.rs:56` whenever `in_genesis` is true, which is exactly the empty-disk case. The comment must be corrected as part of the fix.
- `bins/node/src/node/init.rs:752-753` — comment asserts *"Skipped when starting from fresh genesis (height=0) because no race exists — the node has no prior tip to build on incorrectly."* **Stale/false:** an empty disk on a live chain is not a fresh genesis, and a race does exist.
- `crates/network/src/sync/manager/production_gate.rs:27-40` — doc-comment claims `can_produce` is *"the single source of truth for block production authorization"* with *"3 checks"*. It is not the single source of truth (G7/G12 live in `bins/node`), and it now has 4 numbered checks plus 2 removed ones. Low priority.

---

## Contradictions found (Intellectual Honesty — CONTRADICTION-STOP)

**⚠ CONTRADICTION 1 — the recorded SEVERE end state is falsified by the node's own log.**

The incident record and the task brief state: *"n13: peer_count stayed at 1, below SNAP_MIN_PEERS=3,
so the rescuing snap NEVER fired; node wedged at h=1 permanently."*

`~/testnet/logs/n13.log` shows otherwise:

```
22:04:21.117  Empty headers … local_h=1, gap=84507, consecutive=1        [wedge begins]
22:05:53.116  [SNAP_SYNC] Peer 12D3KooWAetAs3yt… failed, retrying with alternate
              peer 12D3KooWSuinNNdC… at height=84517 (3 remaining)        [snap DID fire]
22:06:00.049  [HEALTH] h=84518 … peers=13 … state="Synchronized"          [recovered]
22:06:30.050  [HEALTH] h=84521 … peers=13 … state="Synchronized"
```

**Resolution:** n13 was wedged for ~92 s, then snap-synced and reached full fleet agreement with
13 peers. Its end state was **BENIGN** (fossil orphan at block 1 below the snap horizon) —
the same class as n12, not a distinct severe class. The `peers=1` reading in the record is a
snapshot from the wedge window, not a steady state.

**What this changes:** the severity/urgency framing, and one argument. The claim *"the post-hoc
snap that hides the defect is itself peer-count-conditional, so a MINT-TIME gate is required"*
remains **true as a principle** (the snap is genuinely peer-count-conditional) but is **no longer
supported by the n13 observation** — n13 is evidence that the snap *does* fire. The mint-time
gate is still the right fix, justified by the fossil orphan itself (a permanent, silent, fleet-wide
block-1 disagreement that has already been misattributed once, under INC-I-139), not by a
permanent wedge that was not observed.

**What this does NOT change:** the root cause, the guard map, or the recommended fix.

**⚠ CONTRADICTION 2 (minor) — the incident's fix direction is under-specified.**

The recorded direction *"gate production on `is_in_genesis(local_height) AND best_peer_height == 0`"*
implies changing `is_in_genesis` or its use at `production/mod.rs:208`. But `use_bootstrap` at
`mod.rs:245` is `in_genesis || active_with_weights.is_empty()`, and a wiped disk has an **empty
producer set**, so the bootstrap path is entered on the second disjunct regardless of what
`in_genesis` evaluates to. Any fix that only changes the `in_genesis` value therefore does **not**
keep the node off the bootstrap path — it only re-enables the `scheduling.rs:56` guard block
(Candidate B), with the genesis-delay cost analysed in §D. Recorded so the fix session does not
implement the note verbatim.

---

## What I don't understand (mandatory, pre-recommendation)

1. **The exact mint timestamp on n13.** The WARN-only capture has no `[BLOCK_PRODUCED]` line
   (that log is `info!`, `production/mod.rs:580`). I bracketed the mint to 22:04:00–22:04:21 from
   `[HEALTH]` h=0 and the first `local_h=1`. Sufficient to prove `best_peer_height > 0` at mint,
   insufficient to measure the margin precisely.
2. **Whether the pre-peer-status window (REQ-PROD-003) is reachable in practice.** I can show it
   is reachable *in principle* (§B1/B2), and I can show a *correlation* that narrows it (G11
   producer-list stability is reset by the same peer traffic that fills the peer table), but I
   have no measurement of a mint with `peer_count() == 0`. All three recorded reproductions are
   consistent with peer evidence being present. Labelled INFERRED, priced as `Should`.
3. **What a wiped SEED node does.** Seeds have no `bootstrap_nodes`, so G12 never applies to them
   at all. The recommended fix (G7) covers them because it does not test `has_bootstrap_nodes` —
   but I have no observation of a wiped seed on a live chain, and the seed is also the node that
   must be able to bootstrap a genuinely new chain solo. Unverified.
4. **Whether `network_tip_height` can be inflated by a hostile or forked peer** to a value that
   would block a legitimate genesis node. `remove_peer` re-derives it (`peers.rs:117-125`), and
   `add_peer`/`update_peer` only ratchet it up. I did not audit every writer.
5. **The interaction with `snap.attempts` / INV-SYNC-011.** The fix removes one path into
   `local_height == 0` production but does not touch snap admission. I assume no interaction;
   unverified.

---

## Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| 1 | `network.network_tip_height` is 0 at process start and monotonically raised only from peer status/gossip | "Until someone tells us the network is higher, we think it's at zero" | Yes — `SyncManager::new` `mod.rs:212`, raised at `peers.rs:47,82` |
| 2 | `is_in_genesis` must remain a deterministic function of chain data | "Two honest nodes must always agree on whether a block is a genesis-phase block" | Yes — `validation/producer.rs:248`, `validation/registration.rs:37` |
| 3 | The suppressed block is never canonical | "The block we stop building is one every peer already throws away" | Yes — measured, n13.log 22:04:22 `header chain broken` |
| 4 | Fleet log capture is WARN-only | "Operators only see WARN and above" | Yes — n13.log: 0 INFO, 117 WARN |
| 5 | `max_behind` = 3 for `height < 10` is an acceptable genesis tolerance | "A brand-new node may still produce if the network is at most 3 blocks ahead" | Yes — existing shipped behaviour at `mod.rs:176-180`, unchanged by the fix |

---

## Identified risks

| Risk | Mitigation |
|---|---|
| The argued no-activation-height exemption is wrong | The architect must re-derive it independently; the reviewer must challenge it. Recorded explicitly in §Deploy-safety rather than skipped. |
| `warn!` logging at 1 Hz becomes spam on a legitimately-behind node | REQ-PROD-004 requires rate-limiting to at most once per slot. |
| The four sibling holes (G4c, G4e, G9, G12) stay open | REQ-PROD-011 (`Won't`) — filed as follow-up findings, not bundled (SSF). Each is independently reachable only through the same height-1 window this fix closes, so none is *currently* exploitable once G7 covers height 1. **This claim is INFERRED and should be re-checked by the architect.** |
| Fixing only the production side leaves the fossil blocks already on disk | REQ-PROD-010 (`Won't`) — cosmetic; nodes agree with the fleet at the tip. |

---

## Out of scope (Won't)

- Making `is_in_genesis` network-relative (REQ-PROD-009) — structurally invalid, §C.
- Retroactive fossil removal (REQ-PROD-010).
- Closing the four sibling disk-relative holes (REQ-PROD-011) — SSF: one fix at a time.
- Snap-sync admission / `SNAP_MIN_PEERS` tuning — proven healthy by the controlled experiment.

---

## Traceability matrix

All tests live in `bins/node/tests/inc_i_149_bootstrap_mint_gate.rs`
(run: `cargo test -p doli-node --test inc_i_149_bootstrap_mint_gate`).

| Requirement ID | Priority | Test IDs | Status pre-fix | Architecture Section | Implementation Module |
|---|---|---|---|---|---|
| REQ-PROD-001 | Must | `p1_empty_datadir_joining_live_chain_must_not_mint_block_1` (P1), `p4_local_height_above_one_with_network_far_ahead_must_defer` (P4), `p4c_control_local_height_above_one_with_quiet_network_does_mint` (P4c, non-vacuity control) | P1 **FAILS** (reproduces the bug); P4/P4c PASS | (architect) | `bins/node/src/node/production/mod.rs` |
| REQ-PROD-002 | Must | `p2_fresh_genesis_solo_bootstrap_must_still_mint_block_1` (P2), `p2b_fresh_genesis_fleet_all_peers_at_zero_must_still_mint_block_1` (P2b), `p3_fresh_network_two_blocks_ahead_must_still_mint_block_1` (P3) | All PASS — must keep passing after the fix | (architect) | `bins/node/src/node/production/mod.rs` |
| REQ-PROD-003 | Should | **NOT COVERED** — no test asserts "bootstrap_nodes non-empty + peer_count 0 ⇒ defer". Deliberate: the analyst prices it `Should` because it was never the measured mechanism, and the tests are written so none of them *contradicts* it (P2 is the seed/solo case the requirement explicitly exempts; P2b/P3 both register a peer). | n/a | (architect) | `bins/node/src/node/production/mod.rs` |
| REQ-PROD-004 | Must | **NOT COVERED** — log level/format is not observable through node state; needs a `tracing` capture test or manual verification. Flagged for QA. | n/a | (architect) | `bins/node/src/node/production/mod.rs` |
| REQ-PROD-005 | Must | the whole file; FAIL→PASS evidence is P1 | Satisfied (P1 FAILS pre-fix) | (architect) | `bins/node/tests/inc_i_149_bootstrap_mint_gate.rs` |
| REQ-PROD-006 | Must | N/A (commit gate) | n/a | (architect) | commit message |
| REQ-PROD-007 | Should | GS-001 | n/a | N/A | `scripts/gauntlet.sh` |
| REQ-PROD-008 | Could | not covered (deferred, `Could`) | n/a | (architect) | `bins/node/src/node/init.rs` |
| REQ-PROD-009 | Won't | N/A | n/a | N/A | N/A |
| REQ-PROD-010 | Won't | N/A | n/a | N/A | N/A |
| REQ-PROD-011 | Won't | N/A | n/a | N/A | N/A |

### Harness constraints discovered while writing the tests

Recorded here because they constrain any future test in `bins/node/tests/` that
drives block production, and because two of them are latent traps:

1. **`Node::new_for_test` (Devnet) can never mint.** `ConsensusParams::devnet()`
   sets `slot_duration = 1` s, but `production/mod.rs:418` enforces a 1000 ms
   propagation floor (`min_offset_ms = 1000` because `resolve_bootstrap_eligibility`
   always returns `our_bootstrap_rank = None`, `scheduling.rs:402`). `slot_offset_ms`
   is bounded by `slot_duration * 1000`, so on Devnet it is *always* `< 1000` and
   `try_produce_block` returns before building. Verified by direct probe.
2. **Production params and validation params are two different objects.**
   Production uses `self.params`; `validate_block_for_apply` rebuilds a
   `ValidationContext` from `ConsensusParams::for_network(self.config.network)`
   (`validation_checks.rs:285`). Overriding only `node.params.slot_duration`
   makes `apply_block` reject the node's own block with
   `invalid slot derivation: got=…, expected=…`. `config.network` and `params`
   must be switched together. The test harness therefore switches both to
   testnet shape (10 s slots).
3. **`add_peer` has a sync-state side effect.** Registering a peer that is ahead
   can start header-first sync, after which `can_produce` returns `BlockedSyncing`
   and any "did not mint" assertion passes for the wrong reason. Where a far-ahead
   network height is needed without that side effect, the tests use
   `SyncManager::update_network_tip_height` (peer-free) and assert
   `can_produce(slot) == Authorized` as an explicit non-vacuity precondition.
4. **The echo-chamber gate needs a peer above local height 0.** `genesis_bypass`
   in `production_gate.rs:124` is `local_height == 0 && min_peers_for_production <= 1`,
   so once the local chain advances a peer must be registered or production is
   blocked by `BlockedInsufficientPeers`.

---

## Triage verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.88, measured)
Reasoning: Root cause confirmed by controlled experiment with zero prior failed fixes; the fix is a one-clause deletion in one function whose only callers are the two event-loop ticks, and genesis-safety is provable by construction from a value the same function already reads two lines above.
━━━━━━━━━━━━━━━━━━━━━━
```

**Calibration.** DEEP exists to *find* an unknown root cause. That work is done and was done by
experiment, not inference. The remaining work is: delete one clause, raise one log level, write
a FAIL→PASS test, and argue two deploy-safety questions. Blast radius is 1 file / 1 crate
(§C). Brittleness is 2/5 = LOCALIZED. The fix does not span 3+ interacting components needing
independent investigation — it *removes* an exclusion from a component that already exists and
already reads the right input.

**Basis for 0.88, and the 0.12 residual:**
- `measured` — n13.log timestamps prove `best_peer_height ≥ 84 505` and `peer_count == 1` at mint
  time, so the recommended guard was reachable and would have fired.
- `measured` — the "exactly one fossil block" fact on two independent nodes is explained
  mechanistically by `height > 1` becoming true at height 2, corroborating that G7 is the
  operative hole.
- The 0.12 residual is REQ-PROD-003: the pre-peer-status window is reachable in principle and
  unclosed by this fix, and I have no measurement showing whether it ever occurs. If the
  reproduction harness shows it is reachable, the fix needs a second predicate and the verdict
  should be revisited — but not re-pathed to DEEP.
