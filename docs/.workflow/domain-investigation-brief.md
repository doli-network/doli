# Domain Investigation Brief — INC-I-083 deep root cause

**INC_ID:** INC-I-083
**RUN_ID:** 345 (this investigation; earlier run 342 was the validation that uncovered the regression)
**Branch:** `main` @ `479711b5`
**Date:** 2026-05-19
**Workspace:** `/Users/isudoajl/ownCloud/Projects/doli-network/doli`
**Memory DB:** `.omega/memory.db`

---

## Problem statement (refined)

A 14-commit fix batch (`ce1a72dc..HEAD` = INC-I-078/079/080/081/082 + AH re-pin) was
deployed to the LOCAL testnet at `~/testnet/`. Code gate, regression suites, and
short-window live tests (M1/M2/M3/M5) all PASSED. **But after ~2 h of natural
load, the post-snap fork-recovery deadlock family the bundle was designed to
prevent is REPRODUCING on 5/18 nodes**, and the broader fleet has split into
two advancing chain clusters.

The user wants the **root cause** identified — which domain (fork, connectivity,
parameters, code) owns it, what the actual mechanism is, and whether a
cross-domain causal chain exists.

---

## Chain context

- **Chain:** DOLI (PoS, 10s slots, 18-node testnet: seed + n1–n17, 14 producers)
- **Consensus:** scheduled-producer + pooled epoch rewards (no VDF in production)
- **Client version (binary string):** `doli-node 6.21.20 (3faeccc0)` — **stale**;
  actual code = HEAD `479711b5` (verified by md5 + per-file `git diff HEAD = empty`
  for the INC-I-082 files + 8/8 PASS of `inc_i_082_rebuild_safety` against the
  working tree)
- **Deployed binary:** `~/testnet/bin/doli-node` md5 `15e0d6c7e847f0ac37ea10a2e76c291e`
  (codesigned), deployed 2026-05-19 20:16 via synchronized stop-all → cp + codesign
  → start-all
- **Activation heights active on testnet:** INC-I-078/080 AH=109,559 crossed
  (current ~110,388 >> 109,559) → caps + Ed25519 auth active in production

---

## Live state @ 2026-05-19 22:51:36 — locked snapshot

```
Node  Height    Tip hash         csHash         utxoHash       Status
seed  110,388   63ea535511a3     57105b7280     e2843944e6     advancing (seed-cluster)
n1    110,379   0cba4d6e0b53     5286043386     eefd2b73e5     FROZEN / forked
n2    110,361   f38bd99a912d     44d09bc912     f4f550800b     FROZEN sync_fails=360 gap=31
n3    110,367   0b2750dcb31e     d0677a6d0b     4bdbb561d2     FROZEN sync_fails=259 gap=25 (== n10)
n4    110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
n5    110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
n6    110,387   63ea535511a3     a8ca5689f7     c14de65310     seed-cluster (lag)
n7    110,383   8de645dcdb0a     0f0f8b4c2f     6d14c4e1ff     FROZEN forked tip
n8    110,388   63ea535511a3     57105b7280     e2843944e6     seed-cluster
n9    110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
n10   110,367   0b2750dcb31e     d0677a6d0b     4bdbb561d2     FROZEN sync_fails=259 gap=25 (== n3)
n11   110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
n12   110,385   90f3c8320b70     458f077316     c15acc0fbc     forked tip
n13   110,385   8fff0db05fdf     ee6c500c78     713be81c0f     forked tip
n14   110,358   c72a3052e55b     d55b50ebd3     6c18469289     FROZEN sync_fails=233 gap=34
n15   110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
n16   110,388   63ea535511a3     57105b7280     e2843944e6     seed-cluster
n17   110,396   2974c44ed119     e3b1c35363     ea2fddb9ba     advancing (cluster A)
```

**Key observations:**
- `psHash = 6eb003ff40` on EVERY node — ProducerSet is consistent fleet-wide
- Divergence lives in **ChainState (csHash) + UtxoSet (utxoHash)** — i.e., the
  blocks themselves disagree
- **Two advancing clusters:** cluster A (n4/n5/n9/n11/n15/n17, 6 nodes @
  h=110,396 hash `2974c44ed119`); seed-cluster (seed/n6/n8/n16, 4 nodes @
  h=110,387–388 hash `63ea535511a3`). 8 blocks apart — could be lag or could be
  a fork; needs forensics
- **5 frozen nodes** in post-snap fork-recovery deadlock: n2, n3, n10, n14, n7
  (and possibly n1 and the forked-tip nodes n12/n13 — confirm which class they
  belong to)
- **n3 + n10 are stuck on the EXACT same fork** (same height, hash, cs, utxo) —
  they agree with each other but are isolated from the majority

---

## Deadlock signature (every frozen node shows it)

```
[SYNC] Using GetHeadersByHeight(height=X) — post-snap hash fallback     (repeating)
[HEADER_DEBUG] Chain break: header.prev_hash=… expected=… valid_so_far=0
Empty headers from <peer> — local hash not recognized
state="Syncing:Headers"
sync_fails climbing into 200s–360s
```

T0→T1 (25 s window): zero height advance on n2/n3/n10/n14/n7.

---

## FINALITY_GUARD evidence (from cbaa3963)

**One in-vivo activation @ 20:57:01.125339Z:**
```
WARN network::sync::manager::recovery:
  [FINALITY_GUARD] refusing ShallowRollback target_h=110356 (finality=110357, local_tip=110357)
```

The guard works for its specific branch (ShallowRollback past finality, in the
`classify()` Rule 1 path that requires `recently_synced()`). But the frozen
nodes are in a different code path (Syncing:Headers HeaderFirstSync loop) that
the guard cannot reach.

**Why the guard cannot save the frozen nodes:**
`recently_synced()` (recovery.rs:180-184) = `last_applied_secs < 60`. Frozen
nodes haven't applied in minutes → `recently_synced()` returns false → Rule 1
is never evaluated → FINALITY_GUARD branch is never reached.

---

## Session-level disruptions to consider as candidate triggers

1. **20:16 synchronized stop-all → cp + codesign → start-all**: thundering-herd
   dial contention noted; n2/n3/n4/n5 momentarily peerless; recovered via
   staggered restart, but n2/n3 currently among the frozen
2. **20:16 n2 wipe + snap-sync** (`rm -rf n2/data/{blocks,state_db,utxo_store,
   producer_gset.bin,peers.cache,producer.lock}` — preserved signed_slots.db,
   maintainer_state.bin, external node_key + producer_2.json). n2 went h=7 →
   109,819 in ~30 s and converged; degraded back into deadlock ~2 h later
3. **n8 SIGSTOP/SIGCONT cycles** (6 cycles, all SIGCONT'd at end) — n8 currently
   in seed-cluster, not frozen
4. **n6/n7/n8 partition+heal @ ~21:30** — 75 s stop, then restart. n7 currently
   frozen with forked tip
5. **Production fork (natural)** — 14 producers @ 10 s slots, tip races are routine

---

## Commit-range under deployment

| Commit | Incident | Effect |
|---|---|---|
| `1e07876a` | INC-I-079 | economic_sim S2 test (test-only, no consensus impact) |
| `8562f3d7` | INC-I-078 M1 | per-producer `received_delegation_cap` (consensus, height-gated) |
| `882585bc` | INC-I-078 M2 | DelegateBond/RevokeDelegation Ed25519 auth (consensus, height-gated) |
| `c46e9f62` | INC-I-078 | docs alignment |
| `3a58dc20` | INC-I-081 Bundle 1 | abort slot on incomplete epoch store |
| `cbaa3963` | INC-I-081 Bug 1 | ShallowRollback FINALITY_GUARD |
| `e25a9a97` | INC-I-081 Bug 2 | plan_reorg ancestor height fallback |
| `52116b64` | INC-I-081 Bug 3 | direct-apply fallback in fork recovery |
| `4349403a` | INC-I-081 Bug 4 | clear last_finality_height on rollback below finality |
| `d885a449` | INC-I-078 AH | mainnet AH pin 231,830 |
| `2ed43260` | INC-I-080 | AddBond cap: reject post-AH, clip pre-AH |
| `0f5a841e` | INC-I-080 AH | mainnet AH pin 231,830 |
| `3faeccc0` | merge | INC-I-078 + INC-I-080 → main |
| `641c00f6` | INC-I-082 | rebuild_epoch_state_from_blocks bit-identity + explicit target_height |
| `479711b5` | re-pin | mainnet 231,830→240,138, testnet 0→109,559 |

---

## Critical code locations (for forensics — DO NOT EDIT)

- `crates/network/src/sync/manager/recovery.rs:252-363` — `classify()` (Rule 1
  contains FINALITY_GUARD @ 307-319)
- `crates/network/src/sync/manager/recovery.rs:180-184` — `recently_synced()`
  = `last_applied_secs < 60`
- `crates/network/src/sync/manager/recovery.rs:190-211` — thresholds:
  `MIN_MINOR_FORK_EVIDENCE=3`, `MINOR_FORK_GAP_MAX=50`, `SHALLOW_ROLLBACK_MAX=10`,
  `STALE_TIP_SECS=300`, `EVIDENCE_TTL=120s`, `ACTION_COOLDOWN=30s`
- `crates/network/src/sync/manager/block_lifecycle.rs:549, 593` — where
  `last_finality_height` is wired into `RecoveryContext`
- `crates/network/src/sync/manager/` — Syncing:Headers / HeaderFirstSync path
  (this is where the deadlock loops)
- `bins/node/src/node/block_handling.rs`, `rewards.rs`, `rollback.rs` —
  INC-I-082 rebuild path
- `crates/core/src/network_params/defaults.rs:117–137, 239-245` — activation
  heights (mainnet 240,138 / testnet 109,559 for INC-I-078/080)
- `bins/node/build.rs:5` — stale-embedded-commit bug (missing
  `cargo:rerun-if-changed=.git/HEAD`); cosmetic, not consensus

---

## Diagnostic resources available

- **Log files (huge):** `~/testnet/logs/<node>.log` (n2.log ≈ 1.9 GB).
  Use `tail -n NNN`, `grep`, `awk`, never load whole files
- **RPC ports:** seed=8500, nX = 8500+X (e.g. `curl -s http://127.0.0.1:8510`
  for n10). Methods: `getStateRootDebug`, `getEpochInfo`, `getProducers`,
  `getChainInfo`, `getBlockByHeight`, `getBlock`
- **Status script:** `scripts/testnet.sh status [seed|nX|all]`
- **memory.db:** `.omega/memory.db` (SQL via heredoc only)
- **Constants:** `~/testnet/` is the testnet root. Producer keys in
  `~/testnet/keys/`. launchd labels `network.doli.testnet-{seed,n1…n17}`

---

## Constraints (binding)

- ⚠️ **testnet = LOCAL.** `~/testnet/`, launchd, 127.0.0.1. **NEVER SSH ai1–ai5**
  (those are MAINNET, off limits).
- ⚠️ **Read-only investigation.** No source edits. No `rm -rf data/*`. No
  `pkill`/`kill` on launchd-managed nodes (they will respawn → split-brain).
- ⚠️ **Cosmetic version string is misleading.** Binary reports `(3faeccc0)` but
  contains HEAD/479711b5. Investigate against HEAD code, NOT 3faeccc0.
- ⚠️ Use `<<'EOF' ... EOF` for any sqlite3 — never inline single-quotes (breaks
  `datetime('now')`).

---

## Anchors to avoid

- The handoff doc hypothesizes the root cause is in the `recently_synced()` gap.
  Treat this as **ONE hypothesis** — investigate it, but also consider:
  - The fork itself (cluster A vs seed-cluster) may have a producer-level cause
  - Connectivity events at 20:16 deploy or 21:30 partition test may be
    upstream triggers
  - Parameters (`MINOR_FORK_GAP_MAX=50`, `SHALLOW_ROLLBACK_MAX=10`, etc.) may
    be wrong for this fleet size
  - INC-I-082 rebuild logic may produce different results on a snap-synced n2
    vs a full-history node
  - INC-I-081's plan_reorg ancestor fallback may itself create the deadlock by
    accepting headers that don't chain back

---

## Output requirements (per investigator)

- Use the role-specific output filename in your prompt
- Cite **specific evidence** (log line, file:line, RPC output, config value).
  No narrative-only claims.
- State confidence as `conf(0.NN, [measured|inferred|speculative])`
- List hypotheses ranked by likelihood with evidence for/against
- Identify cross-domain causation if observed (e.g., "param X triggers code path
  Y which produces fork Z")
- **Anti-hedging rule:** do NOT recommend "add more logging" as a finding —
  work with what exists
