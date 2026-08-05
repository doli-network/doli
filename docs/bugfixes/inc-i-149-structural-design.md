# INC-I-149 — Structural Fix Design

## The root, in one sentence

`Network::is_in_genesis(height)` serves **two incompatible meanings**, and the
codebase never separated them:

| Meaning | Question it answers | Correct basis | Consumers |
|---|---|---|---|
| **Consensus** | "At this height, do genesis RULES apply?" (bond-free coinbase, bootstrap scheduler, bootstrap validation) | **Pure function of height.** Must be identical on every node or validation diverges. | `validation/producer.rs:248`, `validation/registration.rs:37`, `production/assembly.rs:36,112`, `use_bootstrap` selection |
| **Operational** | "Is it SAFE for me to produce right now without syncing first?" | **Peer-reported evidence.** Local height is exactly what a wiped disk destroys. | `production/mod.rs:174,217`, `scheduling.rs:56`, `production_gate.rs:124`, `init.rs:728,748` |

A wiped producer joining an 84k-block chain has `best_height=0 → height=1`, so
the consensus answer ("genesis rules apply at height 1") is **correct**, while the
operational answer inferred from it ("safe to produce without syncing") is
**catastrophically wrong**. One function cannot serve both.

## What must NOT change (and why)

- `Network::is_in_genesis()` itself — it selects the validation rule set. Making it
  peer-relative makes block validation depend on local peer state → non-deterministic
  validation → fork.
- `production/assembly.rs:36,112` — these determine BLOCK CONTENT (epoch distribution
  branch; genesis VDF Registration TX inclusion). Changing them requires synchronized
  deploy and consensus review.
- `use_bootstrap` selection at `production/mod.rs:245` — production must pick the same
  scheduler validation expects. `scheduling.rs:16-19` states this explicitly:
  *"Production switches to bond-weighted scheduler at genesis_blocks + 1, so validation
  must use the same threshold."*

**Scope rule: this change may alter WHETHER this node produces. It may never alter
WHAT a produced block contains, or WHICH rules validate it.**

## The missing concept

```rust
/// What we actually KNOW about the network's age, from evidence that
/// survives a data-directory wipe.
pub enum NetworkEvidence {
    /// No peer status received yet. We know nothing.
    /// Absence of evidence is NOT evidence of genesis.
    Unknown,
    /// Peers are connected and NONE reports any blocks — genuine fresh genesis.
    AtGenesis,
    /// At least one peer reports height > 0 — the network has history,
    /// so an empty local disk means WE are behind, not that the chain is new.
    HasHistory,
}
```

Computed in `SyncManager` (it owns the peer table):

```rust
pub fn network_evidence(&self) -> NetworkEvidence {
    if self.peer_count() == 0        { NetworkEvidence::Unknown }
    else if self.best_peer_height() > 0 { NetworkEvidence::HasHistory }
    else                             { NetworkEvidence::AtGenesis }
}
```

`peer_count()` counts peers whose STATUS has arrived (`peers.rs:181`; `add_peer` is
called only from `on_peer_status`), so `Unknown` genuinely means "nobody has told us
anything yet" — exactly the pre-status window that the one-clause fix left open.

## The permission gate — full truth table

The second input is `has_bootstrap_nodes = !config.bootstrap_nodes.is_empty()`.
This is **durable configuration**: it survives a disk wipe and states operator intent
("there is a network out there — go find it"). It is the only durable signal available
to a node whose disk was just erased.

> **Spec correction (found by the truth-table tests, 2026-08-05).** An earlier draft of
> this table said `HasHistory ⇒ NO` unconditionally. That is **wrong**, and partition P3
> falsifies it: a node at height 0 whose peers are at height 2 is formally `HasHistory`
> and **must still mint** (`blocks_behind = 2 ≤ max_behind = 3`). `HasHistory` is bounded
> by **magnitude**, not by evidence class, and that bound is *already* enforced by the
> existing behind-network guard once `height > 1` stops excluding the first block.
> Consequence: the only genuinely NEW rule is the `Unknown` row. Do not implement an
> unconditional `HasHistory ⇒ defer` before the bootstrap branch — it would break P3.

| `has_bootstrap_nodes` | evidence | may produce? | why |
|---|---|---|---|
| any | `HasHistory`, **materially** behind (`blocks_behind > max_behind`) | **NO** | The chain has history and we are far behind. Sync first. **Closes the observed defect** — enforced by the existing behind-network guard, which needed only the `height > 1` exclusion removed. |
| any | `HasHistory`, within tolerance | **YES** | Normal operation and early-chain catch-up (P3). Unchanged. |
| `true` | `Unknown` | **NO** | We were told peers exist and have heard from none. Absence of evidence is not evidence of genesis. **Closes the pre-status window (REQ-PROD-003).** |
| `true` | `AtGenesis` | **YES** | Peers connected, nobody has blocks → genuine fresh-genesis fleet. This is the INC-I-115 shape; unchanged. |
| `false` | `Unknown` | **YES** | Origin/seed node with no bootstrap configured. It is definitionally the chain's starting point and has nobody to wait for. **This is what the 2026-02-12 commit legitimately protects.** |
| `false` | `AtGenesis` | **YES** | Origin node, peers confirm genesis. Unchanged. |

Every row is reachable and justified. Note that genesis liveness is preserved on the
`false`/`Unknown` row *by configuration*, not by a timeout — a fresh network's origin
node still produces block 1 instantly with zero peers.

## Known residual, stated not hidden

A **wiped node with NO bootstrap nodes configured** (a wiped seed) rejoining a live
chain lands on row 4 and may produce before its first peer status. This is genuinely
undecidable locally: with no peers and no disk, no evidence distinguishes it from a
genuine origin node. It is bounded — the first peer status flips it to `HasHistory`,
and the behind-network guard then defers it — and the operator remedy is to configure
bootstrap nodes on seeds. Recorded rather than silently accepted.

A second residual (review F-P1-003): a **wiped producer whose FIRST peer status
arrives from a height-0 peer** classifies `AtGenesis`, the no-evidence gate opens,
and with a best known height of 0 the behind-network guard is inert — the fossil
mint is again possible until a status carrying real height arrives. Reachable
during a documented full-fleet-wipe recovery if wiped nodes exchange status with
each other before any synced node. Operational remedy: bring seeds/synced nodes
up FIRST so the first status a wiped producer sees carries the real tip. Not
closed in code by this change; recorded here and in `docs/troubleshooting.md`.

## Change set (minimal for the concept)

1. **NEW** `NetworkEvidence` + `SyncManager::network_evidence()` — `crates/network/src/sync/manager/peers.rs`.
2. **NEW** single permission gate in `try_produce_block`, placed before the
   bootstrap/epoch branch — `bins/node/src/node/production/mod.rs`.
3. **FIX** `production_gate.rs:124` — `genesis_bypass = local_height == 0 && min_peers <= 1`
   waives the echo-chamber peer minimum on any empty disk. Must key on evidence, not height.
4. **KEEP** the `height > 1` removal at `production/mod.rs:174` — correct independently
   as the general behind-network guard, and already regression-tested.
5. **UNTOUCHED** `is_in_genesis`, `assembly.rs`, `validation/*`, `use_bootstrap` selection.

Sites 3 and the gate remove the two waivers that existed *only* because the concept was
missing; they are not new guards.

## Deploy shape (unchanged from the one-clause fix)

No consensus RULE change (validation untouched), no block CONTENT change (assembly
untouched). Alters only whether this node builds. No activation height — and an
activation height would be actively wrong, since it is evaluated against local height
(`ctx.current_height >= ...`) and this logic operates at local height 1, where any real
activation value disables it permanently.
