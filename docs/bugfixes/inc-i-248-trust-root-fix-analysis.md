# INC-I-248 — Release trust root of snap-joined nodes (fix analysis)

Base: tag v6.44.0. Diagnosis: run 576 (verdict YES, conf 0.90). Fix run: 577.

## Root cause (from the diagnosis)

A node's maintainer trust root is a node-local file (`maintainer_state.bin`).
No sync path transfers it or checks it against the chain:

- The network snapshot payload carries ChainState, UtxoSet and ProducerSet only.
- `doli snap` and `doli wipe` delete the file and write no maintainer state.
- On the first applied block, `maybe_bootstrap_maintainer_set`
  (`bins/node/src/node/periodic.rs`) seeds the set from the CURRENT ProducerSet
  (the 5 earliest producers by `(registered_at, pubkey)`). It does not check
  whether the node replayed the chain.

The canonical set is a function of HISTORY (genesis seed plus the rotation
transactions). On mainnet the 5 earliest producers are the 5 retired genesis
keys, which are publicly compromised. So a snap-joined node trusts the retired
keys (threshold 3) and refuses every real release with
`Insufficient signatures: 0/3`. `TrustRoot::resolve` accepts any non-empty set.

## Requirements

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-248-001 (F-1) | In release verification only (not governance), a resolved root that contains any of the 5 retired genesis keys is not usable (`TrustRootUnavailable`). | Must | `inc_i_248_retired_keys_never_usable_root` fails on v6.44.0 and passes after. |
| REQ-248-002 (F-2) | The maintainer state travels with the verified state on every snapshot path (network snapshot and the `doli snap` payload). It is accepted only when the snap peer quorum agrees on its digest. `doli snap` and `doli wipe` keep or replace the file, never only delete it. A node that did not replay never seeds the set from producers. An absent set is stored EMPTY and fails closed. | Must | `inc_i_248_snap_join_root_equals_canonical` and a `doli snap`/wipe test fail on v6.44.0 and pass after. |
| REQ-248-003 (F-3) | A node that holds a wrong set (snap-synced, or digest differs from the peer quorum) replaces it with the quorum set at start. The wrong local set is never the multisig base for that. The updater then re-checks the pending release. | Must | `inc_i_248_wrong_root_repaired_on_start` fails on v6.44.0 and passes after: resolved root == canonical, verify(rotated signers) == Ok(3). |
| REQ-248-004 (F-4) | Replay-node invariance: for a node that replayed the chain, the maintainer set at every height and the acceptance of AddMaintainer/RemoveMaintainer/ProtocolActivation are identical before and after the change. | Must | A test replays a chain with a rotation through the old and the new code paths and asserts equal digests and equal accept/reject verdicts. INC-I-075 checklist in the commit. |
| REQ-248-005 (F-5) | Release validation: full `cargo test` (no test removed or weakened), build, clippy, fmt, gauntlet, Outcome-Metric. | Must | Commands and outputs in the run report. |
| REQ-248-006 (F-6) | Local testnet drill: rotate one maintainer, snap-join a new node, assert its `getMaintainerSet` digest equals the seeds'. | Should | Command and output. |

## Design constraints (operator decisions)

- No activation height for REQ-248-002/003. No node that replayed the chain
  changes any output (REQ-248-004 proves it). Only divergent snap nodes
  converge to the canonical set. If a change alters replay-node behavior,
  block validity, block content or the state root: STOP, report `blocked`.
- MaintainerState is NOT part of `compute_state_root`. The snapshot field is
  versioned. `CURRENT_PROTOCOL_VERSION` is not bumped.
- A snap from peers that do not send the field gets the EMPTY, fail-closed
  root (no updates, never the retired keys). Deploy order: seeds first.
- The retired-key list for REQ-248-001 is the 5 public genesis keys only,
  compiled as public keys.

## Milestones

| # | Scope | Requirements |
|---|---|---|
| M3 | `crates/updater` — retired keys never form a usable release root | REQ-248-001 |
| M1 | snapshot paths carry the maintainer state; no producer seed on a non-replay node | REQ-248-002, REQ-248-004 |
| M2 | repair a wrong root at start from the peer quorum; updater re-check | REQ-248-003, REQ-248-004 |

### M3 — retired keys are never a usable release root

- Add a compiled list of the 5 retired mainnet genesis PUBLIC keys
  (`20204725…`, `effe88fe…`, `54323cef…`, `2d27fdcc…`, `3047e96b…`; full
  values = `MAINNET_GENESIS_PRODUCERS` in `crates/core/src/genesis.rs` and
  `RETIRED_AND_PUBLICLY_COMPROMISED` in
  `crates/updater/tests/trust_root_fail_closed.rs`).
- In the release-verification trust-root resolver
  (`crates/updater/src/trust_root.rs`), a resolved root whose key set
  contains any retired key is not usable, so
  `verify_release_with_trust_root` returns `TrustRootUnavailable`.
- Governance (`bins/node/src/node/apply_block/governance.rs`) and the
  maintainer derivation are NOT changed.
- Test `inc_i_248_retired_keys_never_usable_root` (updater it):
  `TrustRoot::resolve(members = retired five, threshold 3, last_derived_height
  480_001, mainnet)` → `!is_usable()`, and verification returns
  `TrustRootUnavailable` (not `InsufficientSignatures`, not `Ok`). Also one
  retired key mixed into an otherwise valid set → not usable. The compiled
  Bootstrap roots of every network stay usable.

### M1 — the maintainer state travels with the snapshot (node and network)

Design: the fix-design document of run 577, sections 2, 3.1-3.4, 3.6 (node rows).

- Wire: append `SyncRequest::GetMaintainerState` and
  `SyncResponse::MaintainerStateAt { block_hash, block_height, digest, set_bytes,
  last_derived_height }`. Appending a FIELD to an existing sync message is not
  safe (bincode: an old peer's bytes end early, the new decoder fails); an
  appended VARIANT is safe both ways. An old peer does not answer.
- Pure rule `select_quorum_maintainer_state` in `crates/core/src/maintainer/`.
  Quorum = `snap_quorum()` (same rule as the state-root vote). The receiver
  recomputes the digest with its LOCAL genesis hash and drops a mismatch.
- Answer filter (orchestrator amendment, binding): an answer counts only if
  `answer.tip_height >= local_tip` AND `answer.set.last_updated <= local_tip`.
  Then the peer's set is the set at the local tip (no change happened between
  the local tip and the peer tip). This prevents two wrong adoptions: a lagging
  peer that has not applied a rotation the node already applied, and a peer
  ahead of a rotation the node has not applied yet (the node gets that change
  from the blocks).
- The digest excludes `last_updated` (`crates/core/src/maintainer/digest.rs`),
  so answers in one digest group can carry different `last_updated` values (a
  node that seeded at its snap height stamps that height). The adopted set
  takes the MINIMUM `last_updated` of the agreeing group (the history value).
- Adoption: a snap-synced node, an EMPTY root or an unusable root (M3) adopts
  the largest group when it reaches the quorum. A replay node with a usable
  root adopts only on a unanimous quorum (zero dissent among counted answers).
  The local set is never a multisig base: adoption replaces the file under the
  write lock.
  **Superseded by REL-M1 (below).** FM-6 in commit f3ebc311 said "one honest
  dissenting answer keeps the root". That was false (QA-248-R01): a replay node
  that applied a block in the window dropped the honest answers, so fake peers
  had zero dissent. Since REL-M1 a replay root or any non-empty root is never
  eligible, so peers cannot change it at all.
- Seed gate: `maybe_bootstrap_maintainer_set` seeds from producers only when
  the node is not snap-synced (`is_snap_synced()`, persisted). A snap-synced
  node without a set stores EMPTY with `last_derived_height` = snap height,
  which fails closed.
- Tests: `inc_i_248_snap_join_root_equals_canonical`,
  `inc_i_248_snap_join_without_answers_fails_closed`,
  `inc_i_248_sync_wire_append_only`,
  `inc_i_248_doli_wipe_then_fresh_node_never_seeds_from_producers`, and the
  guard `inc_i_248_replay_node_maintainer_set_invariant` (REQ-248-004; passes
  on v6.44.0 and after).

### M1.5 — `getStateSnapshot` and `doli snap` carry the maintainer state

Design: section 3.5.

- RPC `getStateSnapshot`: additive `maintainerState` key (old CLIs ignore it).
- `doli snap`: digest agreement from `getMaintainerSet` on the seeds under the
  same rule as the state-root agreement; after the install it ALWAYS writes a
  maintainer state file: the canonical set when the field is present and every
  check passes, else EMPTY at the snapshot height (fails closed).
- `doli wipe`: unchanged. Decision: keeping the file across a wipe breaks
  REQ-248-004 (a replaying node with the post-rotation set rejects the
  rotation transactions at their real heights). After a wipe the node either
  replays (correct seed from history) or snap-syncs (M1 path). The wipe test
  proves that a wiped node never seeds from producers.
- Test: `inc_i_248_doli_snap_keeps_canonical_root` (CLI, mock RPC seeds).

### M2 — repair a wrong root at start

Design: section 4.

- At start (at least 1 peer, 10 s uptime) the node runs the same probe and the
  same rule as M1. Up to 3 probes 60 s apart, then it keeps its state and logs
  one warning.
- After an adoption the updater re-checks releases at once (`Notify`).
- Tests: `inc_i_248_wrong_root_repaired_on_start`,
  `inc_i_248_correct_replay_root_keeps_on_dissent`,
  `inc_i_248_serve_maintainer_state_round_trip`,
  `inc_i_248_updater_recheck_on_notify`.

## Release-gate fix (REL-M1)

The release validation of the M1-M2 code found four P1 findings with two roots:

1. The peer quorum could replace a usable root. Honest answers carried the tip
   from the start of the 15 s window, so `peer tip >= local tip` dropped them
   after the node applied a new block. Peer-reported tips and `last_updated` are
   not authenticated, so 3 to 5 fake peers had zero dissent. A fresh replay
   node with an EMPTY root could also adopt the current set early, which breaks
   REQ-248-004.
2. Sets that hold retired keys were valid answers. A node could adopt the
   retired set, or keep it as "already canonical". On a node that holds the
   retired set, governance transactions signed with the leaked keys can rotate
   it to an attacker set that passes the M3 check.

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-248-007 | The peer quorum never changes a usable root and never changes a root that came from the node's own replay. It installs only into an EMPTY root stamped at a snap height. It never installs a set that holds a retired key on a network where the keys are retired. A node that holds the retired set past the retirement height fails closed and then repairs from the quorum. | Must | Tests below fail on b45e89fc and pass after. |

Rules:

- Eligibility: a node adopts from the quorum ONLY when its root is EMPTY with
  `last_derived_height > 0` (snap install, `doli snap` EMPTY write, or the
  retired-set sanitizer). An EMPTY root at height 0 (a fresh replay node that
  has not reached the gate) and any non-empty root never change. The probe
  arms only for an eligible root.
- Snapshot install (network path): the maintainer state ALWAYS becomes EMPTY at
  the snap height S. The node skipped the history before S, so a set it held
  before the snapshot (for example a producer seed at height 1) is not trusted.
- Retirement height: a new `NetworkParams` field gives the height after which
  the 5 retired genesis keys are no longer maintainers on that network:
  mainnet 331_457 (the last block of the rotation, public `last_change_block`);
  testnet and devnet `u64::MAX` (their on-chain sets still hold the keys).
- Sanitizer: when a node loads its maintainer state at start, and after a
  producer seed, a set that holds a retired key while the tip is above the
  retirement height becomes EMPTY at the tip (saved, one log line). Then the
  node is eligible and the probe arms. An EMPTY set verifies no governance
  signature, so the leaked keys cannot rotate it. Replay nodes are not
  affected: below the retirement height the genesis set is history, above it
  the canonical set holds no retired key.
- Vote: the filter uses the fixed snap height S of the EMPTY root (not the
  moving tip): an answer counts only if `peer tip >= S`. The `last_updated`
  filter is removed. Answers whose set holds a key retired at S are dropped.
  The decision is made only at the end of the window. Adopt the largest digest
  group only if it reaches `snap_quorum()` AND holds a strict majority of the
  counted answers. A tie keeps the EMPTY root. Every verdict logs one line.
- Routing: no response to the node's own `GetMaintainerState` requests,
  including an Error or busy reply, reaches the sync manager.
- `doli snap`: a set that holds a key retired at the snapshot height is
  written as EMPTY at that height. The digest agreement follows the state-root
  rule exactly, also when other seeds give no usable answer.

## Live repair fix (REL-M2)

The live drill on the local testnet showed a gap. A pre-fix snap node held a
stale set from a producer seed at height 1 (`last_change_block` 1). After the
owner installed the fixed binary, the node kept that set: REL-M1 lets the peer
quorum repair only an EMPTY root, and the retired-key sanitizer acts only where
the keys are retired (mainnet). So a stale set that is not the retired set is
never repaired.

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-248-008 | At start, a node whose non-empty maintainer set was not derived from history it replayed resets the set to EMPTY at its tip, so the peer quorum repairs it. "Not replayed" = with D = `last_derived_height` >= 1 and the tip above D, the node's own block store HAS the block at D and has NO block at D + 1 (the history right after the derivation was skipped by a snapshot). A node that replayed the history keeps its set (REQ-248-004). | Must | `inc_i_248_unreplayed_root_repaired_on_start` fails on 0b722d35 and passes after; the live drill step 4 shows the repaired digest equal to the fleet. |

Rules:

- The check reads only local data (the node's own block store). No peer
  answer can trigger it, so it does not reopen the release-gate P1 (a peer
  quorum replacing a usable root).
- `last_derived_height` changes only at a seed, a governance change, a quorum
  adoption, a reset or a rollback rewind. Nodes CAN prune blocks (admin RPC
  `pruneBlocks`, `BlockStore::prune_blocks_below`), but pruning deletes a
  contiguous prefix of heights from 1 upward (genesis kept). So on a pruned
  replay node, a missing block at D + 1 means the block at D is missing too,
  and the rule does not fire. A node that applied block D and then installed
  a snapshot has block D and lacks D + 1: the rule fires.
- D = 0 never fires (pruning keeps genesis and removes block 1).
- The check runs at start, next to the retired-key sanitizer, before the
  startup probe arms.
