# Skeptic Analysis — disk-guardian feature (VERIFIED)

## The 3 load-bearing challenges (all code-verified by orchestrator)

1. **Halting production does NOT stop the crash.** The signal-6 ABRT fires on RocksDB
   writes that continue during sync/apply of peer blocks, not just production.
   - VERIFIED: `crates/storage/src/state_db/writes.rs:43,66,322,329,404` use
     `.expect("RocksDB write batch")` → panic/abort on ENOSPC.
   - VERIFIED: `apply_block(.., ValidationMode::Light)` runs on the sync path at
     `bins/node/src/node/periodic.rs:353`.
   - Consequence: a node that pauses production but keeps syncing STILL aborts on a
     full disk. Production-only halt converts a visible crash-loop into a silent one.

2. **It targets the wrong writer.** The disk was filled by an unrotated STDOUT log the
   node cannot see or throttle; emitting more structured logs on low disk makes it worse.
   - VERIFIED: `bins/node/src/main.rs:54` = `FmtSubscriber` to stdout; NO
     tracing_appender / rolling appender anywhere in `bins/node/src`.
   - The "poll data_dir not the log mount" guidance is self-defeating: shared volume →
     triggers but production-halt doesn't stop the log/compaction; separate mounts →
     never triggers for the log-driven fill.

3. **Stated "true defect" (mid-write corruption) is already mitigated; auto-resume is
   dead weight.**
   - VERIFIED: `crates/storage/src/state_db/batch.rs:480` commits atomically
     ("All-or-nothing"); `atomic_replace` documents "never an empty DB". A full disk
     fails the WriteBatch atomically → no half-applied state.
   - Nothing in the node reclaims the log, so on nano the disk stays full → a
     production-halt-with-auto-resume node self-halts forever while compaction still aborts.

## Conceded (skeptic agrees these are correct)
- Pausing == missed slot is consensus-safe (`production/mod.rs:280-296`); no active-set mutation.
- No activation height needed (local resource gate).
- Machinery reuse is real (`BlockedExplicit` gate `gates.rs:59`, `fs2::available_space`,
  `utxo_size_monitor.rs` TTL-poll precedent).

## Reframed requirement (higher-leverage, lower-risk)
The load-bearing fix for the nano incident is NOT a production-halt watchdog. It is:
- **(A) Fail-safe writes**: propagate `StorageError` from the `.expect("RocksDB write
  batch")` sites (prod AND sync paths) so a full disk = clean refuse-to-advance, not ABRT.
- **(B) Bound log growth**: in-node rolling appender (tracing-appender) or shipped
  logrotate — prevents the fill in the first place (this is the earlier-deferred "#1").
- **(C) Optional**: a pre-emptive disk-space watchdog that triggers a clean read-only /
  halt-ALL-writes state (not just production) with a non-amplifying log, polling the
  data_dir volume — early-warning polish on top of A+B, not the core fix.
