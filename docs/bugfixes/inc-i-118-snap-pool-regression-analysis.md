# INC-I-118 — Snap-sync reward-pool regression — Analyst + Triage

## Symptom (from incident record + user report)
Fresh snap-synced testnet nodes (n6,n7,n8,n9,n11,n12) freeze at the first epoch
boundary after snap, rejecting the canonical EpochReward block:
`[ECON_EPOCH_OVERFLOW] EpochReward total 3600000000 exceeds pool balance 300000000`
(pool = 0.3 DOLI = 3 blocks; canonical pool ≈ mid-epoch accumulation). Continuous-history
nodes (seed,n1-n5,n10) are healthy. User reports snap-sync "was working as expected,
now it's not" and suspects regression starting at commit `cd4645a`.

## Regression archeology (MANDATED — cd4645a..HEAD)
`cd4645a` is **Phase 2** of an in-flight storage backend migration. The full series in range:
- `cd4645a5` Phase 2 — route UTXO **reads** through state_db (dual-write preserved)
- `6ea98f32` BUG-001 — BlockBatch must stamp Pool outputs like utxo_rocks
- `632045f2` Phase 3 — remove per-tx dual-write; **apply_block writes only to state_db**
- `28831590` Phase 4 — **delete utxo_store; state_db is the sole UTXO store**
- `8db0235f` Phase 5 — RocksDB BlobDB on cf_utxo + snap-sync size monitor
- `01bafc4b` / `e58526d6` / `3c4537f2` / `8f61b15b` — INC-I-112/INC-I-113 churn:
  restored then **reverted** in-memory UtxoSet writes during apply_block

`fork_recovery.rs` (the snap-install path) was **NOT** touched by any migration commit
in range (only `bf53ccf7` INC-I-116 M1 touched it, unrelated). → The snap-install code
predates the backend migration and still assumes the OLD model where `self.utxo_set`
was the authoritative in-memory store.

## Architecture context (post-migration)
`UtxoSet` is now an enum (`crates/storage/src/utxo/set.rs`):
- `InMemory(HashMap)` — produced by `deserialize_canonical()`, used for snap-sync + tests
- `RocksDb(Arc<StateDb>)` — production, **sole authoritative store since Phase 4**

Key sites:
- **Snapshot create (source):** `snapshot.rs:209` `utxo_set.serialize_canonical()` — root check
  passed on receivers, so the served snapshot bytes are complete/correct.
- **Snapshot install (receiver):** `fork_recovery.rs:~437` `*utxo = new_utxo_set` (InMemory),
  then `state_db.atomic_replace(&cs,&ps, utxo.iter_all())` persists the full set to state_db.
- **Pool balance read (validation):** `validation_checks.rs:676-685` reads `self.utxo_set`.
- **apply_block writes:** Phase 3 → writes go to state_db (batch overlay).
- **Restart rebuild:** `init.rs:305` `UtxoSet::from_state_db(...)` → RocksDb-backed.

## Probable cause (PRELIMINARY — code-confirmed neighborhood, mechanism ambiguous)
A **backend split** introduced by the Phase 2-4 migration: after snap-install, `self.utxo_set`
becomes an `InMemory` clone of the snapshot while `state_db` is the authority for apply-time
writes/reads and post-restart rebuild. The reward-pool read at the epoch boundary observes a
store that holds only post-snap coinbase credits (3 blocks = 0.3 DOLI), not the snapshot's
accumulated mid-epoch pool. Exactly which of three mechanisms produces "0.3" is unresolved by
static reading alone:
1. `self.utxo_set` (InMemory) goes stale — apply_block writes only hit state_db, so reads frozen/diverge.
2. A post-snap restart rebuilds `self.utxo_set` from a state_db that did not retain the snapshot pool.
3. `atomic_replace`/`iter_all`/`serialize_canonical` under-transfers the Pool UTXOs across the enum boundary.

This is the classic "rebuild/partial-state from incomplete local history is unsafe for
snap-synced nodes" class — here caused by the snap path not being migrated to the state_db-sole model.

## Impact / blast radius
Consensus-visible freeze of all snap-synced nodes at the next epoch boundary. NOT a fork
(nodes are BEHIND on canonical hash). Fix touches snap-install + possibly serialize/atomic_replace.
Because it changes how snap-synced nodes compute pool_balance (a validation input), any fix that
alters acceptance MUST be assessed for activation-height/sync-deploy needs (CLAUDE.md INC-I-075).

## Requirements for the fix (for later, post-investigate)
- REQ-118-001 (Must): A snap-synced node's reward-pool balance after install MUST equal canonical
  at the snapshot height. AC: snap to mid-epoch height h, assert pool_balance == canonical pool@h.
- REQ-118-002 (Must): A snap-synced node MUST accept the next canonical epoch-boundary EpochReward.
  AC: FAIL→PASS reproduction test across the epoch boundary.
- REQ-118-003 (Should): pool read path and snap-install write path MUST target the same authoritative store.

━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.9, basis=multi-module backend split across snap-install/serialize/read/restart; static read leaves exact mechanism ambiguous; resumed incident; consensus-critical)
Reasoning: 3+ interacting components (snap install, UtxoSet enum backends, state_db, validation read, restart rebuild); resumed incident; exact mechanism not determinable by reading alone — needs state reconstruction (getUtxoDiff) + parallel perspectives.
━━━━━━━━━━━━━━━━━━━━━━
