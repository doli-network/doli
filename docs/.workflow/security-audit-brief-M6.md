# Security Audit Brief — INC-I-139 M6 (RC-1 + RC-2)

## Feature / Change Description
Refactor of the DOLI sync coordinator's SnapSync **admission** surface (`crates/network/src/sync/manager/`). SnapSync is a history-wiping recovery: a node discards local chain state and rebuilds from a peer-served state snapshot. The recurrence class INC-I-005/033/138/139 is: minor forks/stalls (gap < SNAP_SYNC_GAP_MIN=500) reaching SnapSync through inconsistently-guarded entry paths. M6 completes the "demote `snap.threshold` to a pure enable/disable sentinel" step:

- **RC-1a**: `snap.threshold` keeps ONLY sentinel semantics — `< u64::MAX` = enabled, `== u64::MAX` = disabled (`--no-snap-sync`).
- **RC-1b**: all four gap-comparator reads (`gap > self.snap.threshold`) re-homed to the named constant `thresholds::SNAP_SYNC_GAP_MIN` (=500): `decision.rs:177` (fresh-node wait), `decision.rs:202` (discv5-grace), `production_gate.rs:813` (is_deep_fork_detected), `cleanup.rs:492` (snap retry). `dispatch.rs:261` peer-quality filter decoupled to literal `local_height + 10`.
- **RC-1c**: the discv5 peer-discovery grace wait is now gated on `local_height == 0` (an h>0 node must not park waiting for snap peers it will never use).
- **RC-2**: the emergency re-enable `snap.threshold = 10` (inside `request_genesis_resync`, reachable only via three evidence-guarded emergency RecoveryReasons) is replaced by `enable_snap_sync()` (sets 50). Claimed bit-for-bit because post-RC-1 no code reads the numeric value as a gap floor.

## Trust Boundary
Peer-served data drives sync decisions: peer-reported `best_height` (→ gap), peer count, empty-header responses, consensus target hash. The admission gate decides whether to trigger a state-wiping SnapSync. Attacker-reachable via: lying peers (inflated/deflated heights), withholding headers (empty responses), Sybil peer counts. Prior incidents: INC-I-120 (self-amplified sync request storm / DoS), INC-I-081 (snap onto fork chain), INC-I-139 (bare-gap snap admission at gap=51).

## Affected Code Paths (M6 diff, source only)
- crates/network/src/sync/manager/sync_engine/decision.rs
- crates/network/src/sync/manager/sync_engine/dispatch.rs
- crates/network/src/sync/manager/production_gate.rs
- crates/network/src/sync/manager/block_lifecycle.rs
- crates/network/src/sync/manager/cleanup.rs

## Key Security Question for This Refactor
Does the demotion of `snap.threshold` to a sentinel and the 10→50 emergency-enable change OPEN any new attacker-reachable path to trigger a history-wiping SnapSync (or suppress a needed guard), OR is behavior genuinely preserved? Specifically: can a peer-controllable input now push a minor-gap (or h>0) node into SnapSync that the pre-M6 code would have refused? Is the "bit-for-bit inert" claim on the 10→50 value change actually true across ALL reachable admission inputs?

## Relevant Invariants
- INV-SYNC-011: SnapSync unreachable for gaps < MINOR_FORK_GAP_MAX(50) except via corroborated deep-fork evidence, on ANY path.
- INV-SYNC-009: outbound sync requests must pass a rate governor (INC-I-120 kill mechanism).

## Tech Stack
Rust, libp2p-based P2P sync coordinator. No external network input parsing changed. Sync-coordinator-internal; no consensus rule or block-content change; rolling-safe; no activation height; no version bump.
