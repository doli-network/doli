use super::*;

/// Initialize the UTXO set from disk, reconciling `utxo_store/` against `state_db`.
///
/// Called by `Node::new` at startup. Kept as a free function so integration tests
/// can exercise the startup init logic in isolation (see
/// `bins/node/tests/inc_i_027_utxo_restore_selfheal.rs`).
///
/// ## Behavior
///
/// `state_db` is always authoritative — it holds the canonical UTXO set inside its
/// own column family and is carried in every guardian checkpoint. `utxo_store/` is
/// a separate RocksDB cache that must stay in lockstep with `state_db`.
///
/// Resolution matrix:
///
/// | `utxo_store` state | Action |
/// |---|---|
/// | Cannot open | Fall back to in-memory, migrate from `state_db` |
/// | Empty, `state_db` empty | Empty store (genesis / fresh install) |
/// | Empty, `state_db` non-empty | Migrate from `state_db` (first boot after upgrade) |
/// | Non-empty, `len` matches `state_db` | Use as-is (normal steady state) |
/// | Non-empty, `len` differs from `state_db` | **INC-I-027 self-heal**: clear and rebuild from `state_db` |
///
/// ## INC-I-027 — guardian-restore self-heal
///
/// Pre-fix, the non-empty branch used `utxo_store/` as-is with no comparison against
/// `state_db`. When an operator restored `state_db + blocks` from a guardian
/// checkpoint but left `utxo_store/` in place (the default, because the guardian
/// never snapshotted it), the node started with mismatched local state:
/// `state_db` at the restored height, `utxo_store/` at the pre-restore height.
/// The node reported the correct height on RPC but silently operated on stale
/// UTXOs, making it vulnerable to bad reorgs (2026-04-09 mainnet: ai1/ai2 reorged
/// forward within 15 seconds of restore).
///
/// The fix detects `store.len() != state_db.utxo_len()`, clears `utxo_store/`, and
/// re-migrates from `state_db.iter_utxos()` — the same authoritative loop already
/// used for empty stores. Triggers only when the two stores are already inconsistent;
/// zero cost in steady state.
pub fn init_utxo_set(data_dir: &std::path::Path, state_db: &StateDb) -> UtxoSet {
    let utxo_rocks_path = data_dir.join("utxo_store");
    match UtxoSet::open_rocksdb(&utxo_rocks_path) {
        Ok(mut store) => {
            let state_len = state_db.utxo_len();
            let store_len = store.len();

            if store.is_empty() && state_len > 0 {
                info!(
                    "[UTXO] Migrating {} UTXOs from StateDb to RocksDB...",
                    state_len
                );
                for (outpoint, entry) in state_db.iter_utxos() {
                    let _ = store.insert(outpoint, entry);
                }
                info!(
                    "[UTXO] Migration complete: {} UTXOs in RocksDB",
                    store.len()
                );
            } else if !store.is_empty() && store_len != state_len {
                // INC-I-027 self-heal: detected divergence between the two local
                // stores. This is the guardian-restore gap — operator restored
                // state_db from a checkpoint but left the stale utxo_store in place.
                // state_db is authoritative; rebuild utxo_store from it.
                warn!(
                    "[UTXO] INC-I-027: utxo_store mismatch with state_db \
                     (utxo_store={} state_db={}) — rebuilding from state_db (guardian-restore self-heal)",
                    store_len, state_len
                );
                store.clear();
                for (outpoint, entry) in state_db.iter_utxos() {
                    let _ = store.insert(outpoint, entry);
                }
                info!(
                    "[UTXO] INC-I-027: rebuild complete: {} UTXOs in RocksDB",
                    store.len()
                );
            } else if !store.is_empty() {
                info!("[UTXO] RocksDB store: {} UTXOs", store_len);
            }
            store
        }
        Err(e) => {
            warn!(
                "[UTXO] Failed to open RocksDB store: {}. Falling back to in-memory.",
                e
            );
            let mut mem = storage::InMemoryUtxoStore::new();
            for (outpoint, entry) in state_db.iter_utxos() {
                mem.insert(outpoint, entry);
            }
            UtxoSet::InMemory(mem)
        }
    }
}

/// Detect and recover from block body gaps in the recent chain.
///
/// Called during `Node::new` when the tip block exists in the block store.
/// Header-first sync can leave gaps (headers present, bodies missing).
/// If the node restarts with such gaps, rollback fails ("no block at height N").
pub fn recover_body_gaps(
    chain_state: &mut ChainState,
    block_store: &BlockStore,
    state_db: &StateDb,
    utxo_set: &mut UtxoSet,
) -> Result<(), anyhow::Error> {
    let check_depth = 100u64.min(chain_state.best_height);
    let mut first_gap = None;
    for h in (chain_state.best_height.saturating_sub(check_depth)..=chain_state.best_height).rev() {
        if h == 0 {
            continue;
        }
        if block_store.get_block_by_height(h)?.is_none() {
            first_gap = Some(h);
            break;
        }
    }

    let gap_height = match first_gap {
        Some(h) => h,
        None => return Ok(()),
    };

    let mut target_height = gap_height.saturating_sub(1);
    while target_height > 0 {
        if block_store.get_block_by_height(target_height)?.is_some() {
            break;
        }
        target_height -= 1;
    }

    let undo_count = chain_state.best_height - target_height;
    warn!(
        "[STARTUP] Body gap at h={} (tip={}). Undoing {} blocks to h={}.",
        gap_height, chain_state.best_height, undo_count, target_height
    );

    // Collect ALL undo data before mutating the UTXO set.
    // The UTXO set is RocksDB-backed — each remove/insert is immediately
    // persisted. If we mutate first and then discover a missing undo,
    // the partial mutations are already committed and cannot be rolled back.
    let mut undos = Vec::with_capacity(undo_count as usize);
    for h in (target_height + 1..=chain_state.best_height).rev() {
        match state_db.get_undo(h) {
            Some(undo) => undos.push(undo),
            None => {
                warn!(
                    "[STARTUP] No undo data at h={} — rebuilding UTXO set from state_db \
                     (avoiding partial mutation leak)",
                    h
                );
                utxo_set.clear();
                for (outpoint, entry) in state_db.iter_utxos() {
                    let _ = utxo_set.insert(outpoint, entry);
                }
                return Ok(());
            }
        }
    }

    for undo in &undos {
        for outpoint in &undo.created_utxos {
            let _ = utxo_set.remove(outpoint);
        }
        for (outpoint, entry) in &undo.spent_utxos {
            let _ = utxo_set.insert(*outpoint, entry.clone());
        }
    }

    if let Some(blk) = block_store.get_block_by_height(target_height)? {
        chain_state.best_height = target_height;
        chain_state.best_hash = blk.hash();
        chain_state.best_slot = blk.header.slot;
        state_db.put_chain_state(chain_state)?;
        info!(
            "[STARTUP] Recovered to h={} after undoing {} body-gap blocks. \
             Sync will fill the gaps.",
            target_height, undo_count
        );
    }

    Ok(())
}

impl Node {
    /// Create a new node
    ///
    /// If `producer_set` is Some, uses the provided ProducerSet (shared with update service).
    /// Otherwise, loads from disk or creates a new one.
    ///
    /// If `signed_slots_db` is Some, uses it to prevent double-signing after restart.
    ///
    /// If `shutdown_flag` is Some, uses the provided flag for graceful shutdown signaling.
    /// Otherwise, creates a new flag internally.
    pub async fn new(
        config: NodeConfig,
        producer_key: Option<KeyPair>,
        bls_key: Option<crypto::BlsKeyPair>,
        producer_set: Option<Arc<RwLock<ProducerSet>>>,
        signed_slots_db: Option<SignedSlotsDb>,
        shutdown_flag: Option<Arc<RwLock<bool>>>,
    ) -> Result<Self> {
        let mut params = ConsensusParams::for_network(config.network);

        // Apply chainspec overrides (authoritative for non-mainnet networks)
        if let Some(ref spec) = config.chainspec {
            params.apply_chainspec(spec);
            info!(
                "Applied chainspec overrides: slot_duration={}s, bond={}, reward={}",
                params.slot_duration, params.initial_bond, params.initial_reward
            );
        }

        // Open storage
        let blocks_path = config.data_dir.join("blocks");
        let mut block_store = Arc::new(BlockStore::open(&blocks_path)?);

        // Open unified StateDb (atomic WriteBatch per block)
        let state_db_path = config.data_dir.join("state_db");
        let state_db = Arc::new(StateDb::open(&state_db_path)?);

        // Clean up old RocksDB diagnostic logs (LOG.old.*) to reclaim disk space
        for db_dir in [&state_db_path, &blocks_path] {
            if let Ok(entries) = std::fs::read_dir(db_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("LOG.old.") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }

        // Migration: if state_db is empty but old files exist, migrate into it
        if !state_db.has_state() {
            let state_path = config.data_dir.join("chain_state.bin");
            let producers_path = config.data_dir.join("producers.bin");
            let utxo_rocks_path = config.data_dir.join("utxo_rocks");
            let utxo_path = config.data_dir.join("utxo.bin");

            if state_path.exists() || utxo_rocks_path.exists() || utxo_path.exists() {
                info!("[MIGRATION] Migrating to unified state_db...");

                // Load chain state
                // INC-I-084: honour chainspec file override here too — see lines 306-322.
                let old_cs = if state_path.exists() {
                    ChainState::load(&state_path)?
                } else {
                    let spec = match config.network {
                        Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                        Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                        Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
                    };
                    let spec = if let Some(ref override_spec) = config.chainspec {
                        override_spec.clone()
                    } else {
                        spec
                    };
                    ChainState::new(spec.genesis_hash())
                };

                // Load producer set
                let old_ps = if producers_path.exists() {
                    ProducerSet::load(&producers_path)?
                } else {
                    ProducerSet::new()
                };

                // Load UTXOs (priority: utxo_rocks > utxo.bin)
                if utxo_rocks_path.exists() {
                    let rocks = storage::RocksDbUtxoStore::open(&utxo_rocks_path)?;
                    let entries = rocks.iter_entries();
                    state_db.import_utxos(entries.iter().map(|(o, e)| (o, e)));
                    info!(
                        "[MIGRATION] Imported {} UTXOs from utxo_rocks",
                        state_db.utxo_len()
                    );
                } else if utxo_path.exists() {
                    let legacy = storage::InMemoryUtxoStore::load(&utxo_path)?;
                    state_db.import_utxos(legacy.iter());
                    info!(
                        "[MIGRATION] Imported {} UTXOs from utxo.bin",
                        state_db.utxo_len()
                    );
                }

                // Write chain state + producers atomically
                state_db.put_chain_state(&old_cs)?;
                state_db.write_producer_set(&old_ps)?;

                info!(
                    "[MIGRATION] Migrated to unified state_db (height={}, {} UTXOs, {} producers)",
                    old_cs.best_height,
                    state_db.utxo_len(),
                    old_ps.active_count(),
                );

                // Backup old files (safety net for rollback to old binary)
                for path in [&state_path, &producers_path] {
                    if path.exists() {
                        let backup = path.with_extension("bin.backup");
                        if let Err(e) = std::fs::rename(path, &backup) {
                            warn!("[MIGRATION] Failed to backup {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        // Load state from StateDb (or create fresh genesis).
        // INC-I-084: when a chainspec file is supplied, params.genesis_hash is updated
        // from the file via params.apply_chainspec() above. Use the same spec source
        // here so chain_state.genesis_hash agrees with params.genesis_hash; otherwise
        // the ExtendsTip guard in block_handling.rs drops every legitimate block when
        // file-derived header.genesis_hash != embedded-derived chain_state.genesis_hash.
        let canonical_spec = match config.network {
            Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
            Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
            Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
        };
        let canonical_spec = if let Some(ref spec) = config.chainspec {
            spec.clone()
        } else {
            canonical_spec
        };
        let canonical_genesis_hash = canonical_spec.genesis_hash();

        let mut chain_state = if let Some(cs) = state_db.get_chain_state() {
            // REQ-SYNC-003: Validate StateDb genesis hash against embedded chainspec.
            // A stale StateDb (from a different chain or pre-reset) causes consensus
            // divergence — producers compute different scheduling at the same height.
            if cs.genesis_hash != canonical_genesis_hash && cs.best_height > 0 {
                return Err(anyhow::anyhow!(
                    "StateDb genesis hash mismatch!\n\
                     StateDb has:    {}\n\
                     Chainspec has:  {}\n\
                     The state database belongs to a different chain (stale data from a prior reset or wrong network).\n\
                     Fix: wipe data directory ({}) and restart to re-sync from peers.",
                    cs.genesis_hash,
                    canonical_genesis_hash,
                    config.data_dir.display()
                ));
            }
            cs
        } else {
            let cs = ChainState::new(canonical_genesis_hash);
            state_db.put_chain_state(&cs)?;
            cs
        };

        // Load UTXOs: scales to millions of entries via RocksDB-backed store,
        // with startup self-heal against state_db (INC-I-027 guardian-restore fix).
        // See `init_utxo_set` doc comment for the full behavior matrix.
        let utxo_set = init_utxo_set(&config.data_dir, &state_db);
        let utxo_set = Arc::new(RwLock::new(utxo_set));

        // Validate genesis hash against embedded chainspec (detect state_db corruption).
        // INC-I-084: honour chainspec file override here too, otherwise the recheck
        // wipes storage on every restart of a chainspec-overridden network.
        let canonical = {
            let spec = match config.network {
                Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
            };
            let spec = if let Some(ref override_spec) = config.chainspec {
                override_spec.clone()
            } else {
                spec
            };
            spec.genesis_hash()
        };
        if chain_state.genesis_hash != canonical {
            warn!(
                "Genesis hash mismatch: state_db={} chainspec={} — NEW CHAIN DETECTED. Wiping all state and block store.",
                &chain_state.genesis_hash.to_string()[..16],
                &canonical.to_string()[..16],
            );
            // Wipe block store — old-chain blocks poison sync via "already in store" guard.
            // If the in-place clear fails, force-remove the directory and reopen.
            // Without this fallback, a transient FS error leaves the OLD block 1 in
            // place; the genesis-mismatch check below then refuses to start, and the
            // node enters a permanent restart loop that requires manual intervention
            // (the n6/n11/n12 incident on 2026-04-14).
            if let Err(e) = block_store.clear() {
                error!(
                    "[GENESIS_RESET] block_store.clear() failed: {} — force-wiping {}",
                    e,
                    blocks_path.display()
                );
                drop(block_store);
                if let Err(rm) = std::fs::remove_dir_all(&blocks_path) {
                    return Err(anyhow::anyhow!(
                        "Genesis reset failed: could not clear nor remove block store at {}: clear={} remove={}",
                        blocks_path.display(),
                        e,
                        rm
                    ));
                }
                block_store = Arc::new(BlockStore::open(&blocks_path)?);
                info!(
                    "[GENESIS_RESET] block store force-wiped and reopened at {}",
                    blocks_path.display()
                );
            }
            // Reset chain state and UTXOs to genesis
            chain_state = ChainState::new(canonical);
            state_db.clear_and_write_genesis(&chain_state);
            // Clear the in-memory UTXO set we already loaded (it's from the old chain)
            *utxo_set.write().await = UtxoSet::new();
            info!(
                "Chain reset complete. Node will sync from genesis on the new chain (genesis={})",
                &canonical.to_string()[..16]
            );
        }
        let genesis_hash = chain_state.genesis_hash;

        // REQ-SYNC-004: Validate block store genesis against chainspec.
        // StateDb may have the correct genesis hash (from a reset) but the block
        // store may still contain blocks from a previous chain. This causes fork
        // sync to find "mismatches" at every height because the blocks belong to
        // a different chain entirely — irrecoverable without manual wipe.
        if let Ok(Some(block_one)) = block_store.get_block_by_height(1) {
            if block_one.header.prev_hash != genesis_hash {
                warn!(
                    "Block 1 has wrong genesis (prev_hash={}, expected={}). \
                     Deleting stale block from previous chain.",
                    &block_one.header.prev_hash.to_string()[..16],
                    &genesis_hash.to_string()[..16]
                );
                if let Err(e) = block_store.delete_block_by_height(1) {
                    warn!("Failed to delete stale block 1: {}", e);
                }
            }
        }

        // Verify chain state consistency with block store
        if chain_state.best_height > 0 {
            match block_store.get_block(&chain_state.best_hash) {
                Ok(Some(_tip_block)) => {
                    // Tip hash exists — check for body gaps and recover if needed.
                    // Delegated to the extracted helper (INC-I-028).
                    let mut utxo = utxo_set.write().await;
                    recover_body_gaps(&mut chain_state, &block_store, &state_db, &mut utxo)?;
                }
                Ok(None) => {
                    if chain_state.is_snap_synced() {
                        // Block store is intentionally empty after snap sync —
                        // this is NOT corruption. Re-seed the canonical index so
                        // set_canonical_chain can exit cleanly on the first
                        // post-snap-sync block, then proceed normally.
                        info!(
                            "Chain state from snap sync at height {} — block store empty by design, re-seeding index.",
                            chain_state.best_height
                        );
                        if let Err(e) = block_store
                            .seed_canonical_index(chain_state.best_hash, chain_state.best_height)
                        {
                            warn!("Failed to re-seed canonical index after snap sync: {}", e);
                        }
                    } else {
                        warn!(
                            "Chain state tip {} at height {} not found in block store. Recovering...",
                            chain_state.best_hash, chain_state.best_height
                        );
                        // Walk backwards through height index to find highest valid block
                        let mut recovered = false;
                        for h in (1..chain_state.best_height).rev() {
                            if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                                info!(
                                    "Recovered chain state to height {} (hash {})",
                                    h,
                                    block.hash()
                                );
                                chain_state.best_hash = block.hash();
                                chain_state.best_height = h;
                                chain_state.best_slot = block.header.slot;
                                recovered = true;
                                break;
                            }
                        }
                        if !recovered {
                            // Block store is empty but chain state has valid data.
                            // This can happen after crash recovery or force-resync when
                            // state files survived but block history was lost.
                            // Treat as snap-synced state: preserve balances/producers,
                            // blocks will rebuild from peers.
                            warn!(
                                "No blocks in store but chain state at height {}. \
                                 Marking as snap-synced to preserve state.",
                                chain_state.best_height
                            );
                            chain_state.mark_snap_synced(chain_state.best_height);
                            if let Err(e) = block_store.seed_canonical_index(
                                chain_state.best_hash,
                                chain_state.best_height,
                            ) {
                                warn!("Failed to seed canonical index: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Block store error during integrity check: {}. Resetting to genesis.",
                        e
                    );
                    chain_state = ChainState::new(genesis_hash);
                }
            }
        } else {
            // Chain state at genesis — check if block store has blocks (e.g., chain_state.bin
            // was not saved before shutdown). Recover to highest stored block.
            let mut recovered_height = 0u64;
            let mut h = 1u64;
            while let Ok(Some(block)) = block_store.get_block_by_height(h) {
                chain_state.best_hash = block.hash();
                chain_state.best_height = h;
                chain_state.best_slot = block.header.slot;
                recovered_height = h;
                h += 1;
            }
            if recovered_height > 0 {
                warn!(
                    "Recovered chain state from block store: height {} (chain_state.bin was missing/stale)",
                    recovered_height
                );
            }
        }

        // Apply slot_duration from chainspec if available (consensus-critical)
        // This ensures all nodes compute the same slot numbers regardless of local .env
        if let Some(slot_duration) = config.slot_duration_override {
            params.slot_duration = slot_duration;
            info!("Slot duration from chainspec: {}s", slot_duration);
        }

        // For devnet, handle genesis_time from multiple sources (in priority order):
        // 1. Chainspec override (config.genesis_time_override) - ensures all nodes use same time
        // 2. Stored state (chain_state.genesis_timestamp) - for rejoining existing network
        // 3. Dynamic (current time) - for new isolated networks
        if config.network == Network::Devnet && params.genesis_time == 0 {
            use std::time::{SystemTime, UNIX_EPOCH};
            if let Some(override_time) = config.genesis_time_override {
                // Use chainspec genesis time for coordinated startup
                params.genesis_time = override_time;
                info!(
                    "Devnet genesis time from chainspec: {}",
                    params.genesis_time
                );
            } else if chain_state.genesis_timestamp != 0 {
                // Use stored genesis timestamp from previous run
                params.genesis_time = chain_state.genesis_timestamp;
                info!(
                    "Devnet genesis time loaded from state: {}",
                    params.genesis_time
                );
            } else {
                // New devnet without chainspec - set genesis time to current timestamp (rounded to slot)
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                params.genesis_time = now - (now % params.slot_duration);
                info!("Devnet genesis time initialized: {}", params.genesis_time);
            }
        }

        // INC-I-074 followup to INC-I-071: one-shot cleanup of stranded cf_undo
        // entries that were retained by the pre-fix code (UNDO_KEEP_DEPTH=2000).
        // The per-block prune_undo_before walks forward only, so it cannot reclaim
        // the historic tail left over when the retention window shrank to 360.
        // Runs once at startup, BEFORE network/event-loop/production. Idempotent.
        // The depth here must match UNDO_KEEP_DEPTH in apply_block/mod.rs.
        {
            const UNDO_KEEP_DEPTH: u64 = 360;
            let tip_height = chain_state.best_height;
            if tip_height > UNDO_KEEP_DEPTH {
                let horizon = tip_height - UNDO_KEEP_DEPTH;
                let deleted = state_db.prune_undo_below(horizon);
                if deleted > 0 {
                    info!(
                        "[STARTUP] Pruned {} stranded cf_undo entries below h={} \
                         (INC-I-071 followup, post-UNDO_KEEP_DEPTH-reduction cleanup)",
                        deleted, horizon
                    );
                }
            }
        }

        // Rebuild producer liveness map from recent blocks in block_store.
        // Scans the last LIVENESS_WINDOW_MIN blocks to determine which producers
        // have been active recently. This is deterministic (same chain = same map).
        let producer_liveness = {
            let mut liveness: HashMap<PublicKey, u64> = HashMap::new();
            let tip = chain_state.best_height;
            let window = consensus::LIVENESS_WINDOW_MIN;
            let start = tip.saturating_sub(window).max(1);
            for h in start..=tip {
                if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                    liveness.insert(block.header.producer, h);
                }
            }
            if !liveness.is_empty() {
                info!(
                    "Rebuilt producer liveness from blocks {}-{}: {} producers tracked",
                    start,
                    tip,
                    liveness.len()
                );
            }
            liveness
        };

        let chain_state = Arc::new(RwLock::new(chain_state));

        // Load or create producer set.
        // ALWAYS try StateDb first — it has the authoritative persisted state
        // (including post-genesis registrations). The provided set from main.rs
        // only has hardcoded genesis producers and would lose any runtime registrations.
        let producer_set = {
            let loaded = state_db.load_producer_set();
            let set = if loaded.active_count() > 0 {
                info!(
                    "[STATE_DB] Loaded {} producers from state_db",
                    loaded.active_count()
                );
                // If caller provided an Arc, replace its contents with the StateDb version
                // so the UpdateService (which shares this Arc) also sees the correct producers.
                if let Some(ref provided) = producer_set {
                    let mut guard = provided.write().await;
                    *guard = loaded.clone();
                    drop(guard);
                }
                loaded
            } else if let Some(ref provided) = producer_set {
                // StateDb is empty — use the provided set (hardcoded genesis producers)
                let guard = provided.read().await;
                let cloned = guard.clone();
                info!(
                    "StateDb empty, using provided producer set ({} producers)",
                    cloned.active_count()
                );
                drop(guard);
                cloned
            } else if config.network == Network::Testnet {
                // For testnet: initialize with genesis producers
                use doli_core::genesis::testnet_genesis_producers;
                let genesis_producers = testnet_genesis_producers();
                if !genesis_producers.is_empty() {
                    info!(
                        "Initializing testnet with {} genesis producers",
                        genesis_producers.len()
                    );
                    ProducerSet::with_genesis_producers(
                        genesis_producers,
                        config.network.bond_unit(),
                    )
                } else {
                    ProducerSet::new()
                }
            } else {
                ProducerSet::new()
            };
            // Reuse the caller's Arc if provided (so UpdateService shares the same reference).
            // We already updated its contents above if StateDb had data.
            if let Some(ref provided) = producer_set {
                // Contents already set (either StateDb version or kept as-is)
                provided.clone()
            } else {
                Arc::new(RwLock::new(set))
            }
        };

        // Create mempool
        let mempool_policy = match config.network {
            Network::Mainnet => MempoolPolicy::mainnet(),
            Network::Testnet | Network::Devnet => MempoolPolicy::testnet(),
        };
        // AUDIT-P2-003: restore oracle sunset flag from persisted state on
        // startup. Pre-fix this was hardcoded false, opening a window of
        // up to one epoch where a restarted node would accept
        // PriceAttestations that the rest of the fleet (with correct
        // sunset state) rejects — fork risk if the restarted node is a
        // producer. Read the persisted OracleSunsetState, compute current
        // health at the current_epoch, and seed the atomic before any
        // ValidationContext is built.
        let initial_sunset_triggered = {
            let blocks_per_epoch = config.network.blocks_per_reward_epoch();
            let best_height = chain_state.read().await.best_height;
            let current_epoch = if blocks_per_epoch > 0 {
                best_height / blocks_per_epoch
            } else {
                0
            };
            let sunset_state = state_db.get_oracle_sunset_state().unwrap_or_default();
            let triggered = sunset_state.health(current_epoch).is_sunset_triggered();
            if triggered {
                info!(
                    "[ORACLE] startup: restored sunset_triggered=true \
                     (current_epoch={}, halt_since_epoch={:?})",
                    current_epoch, sunset_state.halt_since_epoch
                );
            }
            triggered
        };
        let oracle_sunset_triggered = Arc::new(AtomicBool::new(initial_sunset_triggered));
        let mempool = Arc::new(RwLock::new(Mempool::new(
            mempool_policy,
            params.clone(),
            config.network,
        )));
        mempool
            .write()
            .await
            .share_oracle_sunset_flag(oracle_sunset_triggered.clone());

        // Create sync manager with default settings (2 slots/heights tolerance).
        // All networks use the same tolerance — recovery from forks is handled by
        // auto-resync (triggered by sync failure threshold), not by loose tolerances.
        let sync_config = SyncConfig::default();
        let sync_manager = Arc::new(RwLock::new(SyncManager::new(sync_config, genesis_hash)));

        // Configure bootstrap grace period from network params
        // This is the wait time at genesis before allowing production (chain evidence collection)
        {
            let mut sm = sync_manager.write().await;
            sm.set_bootstrap_grace_period_secs(params.bootstrap_grace_period_secs);

            // Disable snap sync if --no-snap-sync was passed
            if config.no_snap_sync {
                sm.disable_snap_sync();
                info!("Snap sync disabled via --no-snap-sync");
            }

            // Configure min peers for production based on network and genesis phase.
            // During genesis, allow single-peer production since the network is
            // still bootstrapping with very few nodes. After the first epoch boundary,
            // recompute_active_status() runs at epoch boundaries after sync completes.
            let in_genesis_at_start = {
                let state = chain_state.read().await;
                config.network.is_in_genesis(state.best_height + 1)
            };
            let min_peers = match config.network {
                Network::Devnet => 1,
                _ if in_genesis_at_start => 1,
                Network::Testnet | Network::Mainnet => 2,
            };
            sm.set_min_peers_for_production(min_peers);

            // Configure gossip timeout based on slot duration (P0 #3)
            // 18 slots = ~3 minutes on Mainnet (10s slots)
            // Scales down for Devnet/Testnet with faster slots
            let gossip_timeout = 18 * params.slot_duration;
            sm.set_gossip_activity_timeout_secs(gossip_timeout);

            // Initialize sync manager with current chain state (critical for restart correctness).
            // Without this, SyncManager starts at genesis and re-downloads the entire chain,
            // causing height double-counting (ISSUE-5).
            {
                let state = chain_state.read().await;
                if state.best_height > 0 {
                    sm.update_local_tip(state.best_height, state.best_hash, state.best_slot);
                    // INC-I-089: Engage post-restart production lockout. Blocks self-production
                    // until first canonical gossip block extends local tip OR safety timer expires.
                    // Skipped when starting from fresh genesis (height=0) because no race exists
                    // — the node has no prior tip to build on incorrectly.
                    sm.engage_post_restart_lockout();
                    info!(
                        "Sync manager initialized at height {} (hash {})",
                        state.best_height,
                        &state.best_hash.to_string()[..16]
                    );
                }
            }

            // Set block store floor so fork sync knows where our block coverage begins.
            // For snap-synced nodes missing block 1, find the lowest available height.
            // For full-sync nodes this stays at 1 (the default).
            if block_store.get_block_by_height(1).ok().flatten().is_none() {
                // Snap-synced: find lowest available block by scanning from chain state height downward.
                // The snap sync anchor is typically the only indexed block.
                let state = chain_state.read().await;
                let floor = if state.is_snap_synced() {
                    state.snap_sync_height().unwrap_or(state.best_height)
                } else {
                    state.best_height
                };
                sm.set_store_floor(floor);
                info!(
                    "[STARTUP] Block store floor set to {} (snap sync gap — block 1 missing)",
                    floor
                );
            }
        }

        if producer_key.is_some() {
            info!("Block production enabled");
        }

        // ── Epoch state format version check ──
        // INC-I-054: NEVER delete epoch_state based on version mismatch alone.
        // Deletion forces rebuild_epoch_state_from_blocks(), which is non-deterministic
        // on snap-synced nodes (incomplete block history) → guaranteed fork.
        //
        // Instead, trust the deserialization check below (init.rs:~747). If the format
        // actually changed and old data can't deserialize, it falls back gracefully.
        // If the format is compatible (which it almost always is), the data loads fine.
        //
        // EPOCH_STATE_FORMAT_VERSION is independent from CURRENT_PROTOCOL_VERSION.
        // Only bump it when the EpochState struct serialization actually changes.
        let persisted_version = state_db.get_epoch_state_version();
        if let Some(pv) = persisted_version {
            if pv != EPOCH_STATE_FORMAT_VERSION {
                // Log the version difference but DO NOT delete.
                // Legacy binaries stored CURRENT_PROTOCOL_VERSION (7, 8, 9...),
                // new binaries store EPOCH_STATE_FORMAT_VERSION (1).
                // The EpochState format has never changed between these versions.
                info!(
                    "[INIT] Epoch state version marker update ({} → {}): format compatible, keeping persisted state",
                    pv, EPOCH_STATE_FORMAT_VERSION
                );
                // Update the stored version to the new format version.
                state_db.put_epoch_state_version(EPOCH_STATE_FORMAT_VERSION);
            }
        }

        // Load complete EpochState from unified key (written by post_commit + snap sync).
        // Falls back to individual keys (pre-upgrade) then UTXO reconstruction.
        let loaded_epoch_state: Option<doli_core::EpochState> =
            state_db.get_epoch_state().and_then(|bytes| {
                match doli_core::EpochState::deserialize(&bytes) {
                    Ok(es) => {
                        info!(
                            "[INIT] Loaded persisted EpochState: epoch={} producers={} active={} bonds={}",
                            es.epoch, es.producer_list.len(), es.active_list.len(), es.bond_snapshot.len()
                        );
                        Some(es)
                    }
                    Err(e) => {
                        warn!("[INIT] Failed to deserialize persisted EpochState: {} — falling back to individual keys", e);
                        None
                    }
                }
            });

        // Legacy fallback: load bond snapshot from individual key
        let (initial_bond_snapshot, initial_bond_epoch) = if let Some(ref es) = loaded_epoch_state {
            (es.bond_snapshot.clone(), es.epoch)
        } else if let Some((snap, epoch)) = state_db.get_epoch_bond_snapshot() {
            let total: u64 = snap.values().sum();
            info!(
                "[INIT] Loaded persisted epoch_bond_snapshot: {} producers, total_bonds={}, epoch={}",
                snap.len(), total, epoch
            );
            (snap, epoch)
        } else {
            let ps = producer_set.read().await;
            let cs = chain_state.read().await;
            let h = cs.best_height;
            let bpe = config.network.blocks_per_reward_epoch();
            let active = ps.active_producers_at_height(h);
            let audit_activation = config.network.params().security_audit_activation_height;
            let mut snap = HashMap::new();
            for p in &active {
                let pkh =
                    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, p.public_key.as_bytes());
                // INC-I-068: Use delegation-aware weight; skip weight=0
                let count = p.selection_weight_at(h, audit_activation);
                if count == 0 {
                    continue;
                }
                snap.insert(pkh, count);
            }
            let total: u64 = snap.values().sum();
            let epoch = h.checked_div(bpe).unwrap_or(0);
            info!(
                "[INIT] No persisted bond snapshot — rebuilt from UTXO: {} producers, total_bonds={}, epoch={}",
                snap.len(), total, epoch
            );
            (snap, epoch)
        };

        // Create equivocation detector
        let equivocation_detector = Arc::new(RwLock::new(EquivocationDetector::new()));

        // Create VDF calibrator with network-specific target time
        // VDF is the BOTTLENECK in Proof of Time: ~80% of slot duration
        let vdf_target_ms = config.network.vdf_target_time_ms();
        let mut vdf_calibrator = VdfCalibrator::for_network(vdf_target_ms);
        if producer_key.is_some() {
            info!(
                "Calibrating VDF for {:?} (target: {}ms = {}s)...",
                config.network,
                vdf_target_ms,
                vdf_target_ms / 1000
            );
            let calibration_time = vdf_calibrator.calibrate_now();
            info!(
                "VDF calibrated: {} iterations (calibration took {:?})",
                vdf_calibrator.iterations(),
                calibration_time
            );
        }
        let vdf_calibrator = Arc::new(RwLock::new(vdf_calibrator));

        // Initialize producer discovery CRDT with persistence
        let gset_path = config.data_dir.join("producer_gset.bin");
        let network_id = config.network.id();
        let producer_gset = Arc::new(RwLock::new(ProducerGSet::new_with_persistence(
            network_id,
            genesis_hash,
            gset_path,
        )));

        // Initialize adaptive gossip controller
        let adaptive_gossip = Arc::new(RwLock::new(AdaptiveGossip::new()));

        // Use provided shutdown flag or create a new one
        let shutdown = shutdown_flag.unwrap_or_else(|| Arc::new(RwLock::new(false)));

        // Producer lists + attestation accumulators: prefer unified EpochState,
        // fall back to individual keys (pre-upgrade), then ProducerSet reconstruction.
        let best_h = chain_state.read().await.best_height;
        let (epoch_producer_list, active_production_list) = if let Some(ref es) = loaded_epoch_state
        {
            (es.producer_list.clone(), es.active_list.clone())
        } else {
            let persisted_epoch = state_db.get_epoch_producer_list();
            let persisted_active = state_db.get_active_production_list();

            if let Some(epoch_list) = persisted_epoch {
                let active_list = persisted_active.unwrap_or_else(|| epoch_list.clone());
                info!(
                    "[INIT] Loaded persisted epoch_producer_list ({} producers) and active_production_list ({} producers) from individual keys",
                    epoch_list.len(),
                    active_list.len()
                );
                (epoch_list, active_list)
            } else {
                let producers = producer_set.read().await;
                let audit_act = config.network.params().security_audit_activation_height;
                // INC-I-068: Filter fully-delegated producers (weight=0) from scheduling
                let mut pks: Vec<PublicKey> = producers
                    .active_producers_at_height(best_h)
                    .iter()
                    .filter(|p| p.selection_weight_at(best_h, audit_act) > 0)
                    .map(|p| p.public_key)
                    .collect();
                pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                if !pks.is_empty() {
                    info!(
                        "[INIT] No persisted producer lists — seeded with {} active producers at h={}",
                        pks.len(), best_h
                    );
                }
                let active = pks.clone();
                (pks, active)
            }
        };

        let (epoch_attested_set, epoch_attestation_accum, epoch_blocks_produced_accum) =
            if let Some(ref es) = loaded_epoch_state {
                (
                    es.attested_sets.clone(),
                    es.attestation_accum.clone(),
                    es.blocks_produced.clone(),
                )
            } else if let Some((attested, accum, produced)) =
                state_db.get_attestation_accumulators()
            {
                info!(
                    "[INIT] Loaded persisted attestation accumulators from individual keys: attested=[{},{},{}] produced={}",
                    attested[0].len(), attested[1].len(), attested[2].len(),
                    produced.len()
                );
                (attested, accum, produced)
            } else {
                info!("[INIT] No persisted attestation accumulators — starting fresh");
                (
                    [HashSet::new(), HashSet::new(), HashSet::new()],
                    [HashMap::new(), HashMap::new(), HashMap::new()],
                    HashMap::new(),
                )
            };

        // Recover announcement sequence from persisted GSet to avoid creating
        // stale announcements after restart. +1 so the next announcement is fresh.
        let initial_seq = {
            let gset = producer_gset.read().await;
            producer_key
                .as_ref()
                .map_or(0, |k| gset.sequence_for(k.public_key()) + 1)
        };

        // Capture the network before `config` is moved into Self below.
        let network_for_schedule = config.network;

        let mut node = Self {
            config,
            params,
            block_store,
            state_db,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None,
            seed_peer_ids: Vec::new(), // Populated in start_network()
            seeds_released: false,
            sync_manager,
            shutdown,
            producer_key,
            bls_key,
            last_produced_slot: None,
            known_producers: Arc::new(RwLock::new(Vec::new())),
            first_peer_connected: None,
            equivocation_detector,
            vdf_calibrator,
            fork_block_cache: Arc::new(RwLock::new(HashMap::new())),
            last_producer_list_change: None,
            producer_gset: producer_gset.clone(),
            adaptive_gossip,
            our_announcement: Arc::new(RwLock::new(None)),
            // Recover sequence from persisted GSet so we don't create stale
            // announcements after restart. Sequence is stable (no heartbeat
            // bumps) — only incremented on actual state changes.
            announcement_sequence: Arc::new(AtomicU64::new(initial_seq)),
            last_broadcast_gset_len: 0,
            signed_slots_db,
            shallow_rollback_count: 0,
            cumulative_rollback_depth: 0,
            seen_blocks_for_slot: std::collections::HashSet::new(),
            epoch_state: doli_core::EpochState {
                epoch: initial_bond_epoch,
                bond_snapshot: initial_bond_snapshot,
                producer_list: epoch_producer_list,
                active_list: active_production_list,
                attested_sets: epoch_attested_set,
                attestation_accum: epoch_attestation_accum,
                blocks_produced: epoch_blocks_produced_accum,
            },
            is_active_producer: false, // Computed on first block application
            last_active_status_epoch: None,
            vote_tx: None,
            pending_update: None,
            last_peer_redial: None,
            bootstrap_backoff: HashMap::new(),
            producer_liveness,
            genesis_vdf_output: None,
            cached_state_root: Arc::new(RwLock::new(None)),
            cached_genesis_producers: std::sync::OnceLock::new(),
            port_check_done: false,
            maintainer_state: None,
            archive_tx: None,
            pending_archive: std::collections::VecDeque::new(),
            archive_dir: None,
            archive_caught_up: false,
            ws_sender: Arc::new(RwLock::new(None)),
            minute_tracker: MinuteAttestationTracker::new(),
            rejected_fork_tips: HashSet::new(),
            snap_sync_height: None,
            sync_requests_this_interval: 0,
            last_checkpoint_height: 0,
            pending_tx_announcements: HashMap::new(),
            hardfork_schedule: updater::HardForkSchedule::for_network(network_for_schedule),
            peer_churn: HashMap::new(),
            last_integrity_check_tip: None,
            recovery_mode: Arc::new(AtomicBool::new(false)),
            oracle_sunset_triggered: oracle_sunset_triggered.clone(),
            health_window: std::collections::VecDeque::new(),
            attest_fetch_tracker: HashMap::new(),
            diagnostic_emitter: Arc::new(storage::diagnostic_ledger::emitter::NoOpEmitter)
                as Arc<dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter>,
            diagnostic_ledger: None,
            diagnostic_shutdown_tx: None,
            diagnostic_writer_stats: storage::diagnostic_ledger::DiagnosticWriterStats::new_shared(
            ),
            last_diagnostic_alerted: HashSet::new(),
        };

        // --- Diagnostic writer + pruner wiring (M2 follow-up) ---
        // Attempt to open the DiagnosticLedger. On success: create AsyncChannelEmitter,
        // spawn writer + pruner tasks. On failure: log warning, keep NoOpEmitter.
        match DiagnosticLedger::open(&node.config.data_dir) {
            Ok(ledger) => {
                let ledger = Arc::new(ledger);
                let (emitter, receiver) =
                    storage::diagnostic_ledger::emitter::AsyncChannelEmitter::new(1024);
                let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

                // Spawn writer task
                let writer_ledger = ledger.clone();
                let writer_stats = node.diagnostic_writer_stats.clone();
                tokio::spawn(super::diagnostic_writer::run_writer_task(
                    receiver,
                    writer_ledger,
                    writer_stats,
                    shutdown_rx.clone(),
                ));

                // Spawn pruner task
                let pruner_ledger = ledger.clone();
                tokio::spawn(super::diagnostics_pruner::run_pruner_task(
                    pruner_ledger,
                    shutdown_rx,
                ));

                node.diagnostic_emitter = Arc::new(emitter)
                    as Arc<dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter>;
                node.diagnostic_ledger = Some(ledger);
                node.diagnostic_shutdown_tx = Some(shutdown_tx);
                info!(
                    "[Diagnostics] Ledger opened at {:?}, writer + pruner spawned",
                    node.config.data_dir.join("diagnostics")
                );
            }
            Err(e) => {
                warn!(
                    "DiagnosticLedger failed to open ({:?}); diagnostics disabled",
                    e
                );
                // Node continues with NoOpEmitter (graceful degradation per REQ-FORKOBS-LEDGER-009)
            }
        }

        Ok(node)
    }

    /// Create a minimal Node for integration tests.
    ///
    /// Uses real RocksDB (tempdir), real ProducerSet, real SyncManager, real fork recovery
    /// state. No networking, no archiver, no updater.
    ///
    /// `producers`: list of KeyPairs to register as genesis producers (each gets 1 bond).
    /// The first producer in the list is set as `producer_key` (the node's own key).
    /// Create a minimal Node for integration tests.
    /// Uses real RocksDB, real ProducerSet, real SyncManager, real fork recovery state.
    /// No networking, no archiver, no updater.
    #[allow(dead_code)] // Used by integration tests in bins/node/tests/
    pub async fn new_for_test(
        data_dir: std::path::PathBuf,
        producers: Vec<KeyPair>,
    ) -> Result<Self> {
        let network = Network::Devnet;
        let mut params = ConsensusParams::devnet();

        // Use genesis_time = 0 for devnet tests.
        // This matches ConsensusParams::devnet() which validate_block_for_apply uses.
        params.genesis_time = 0;

        // Open real RocksDB storage
        std::fs::create_dir_all(&data_dir)?;
        let block_store = Arc::new(BlockStore::open(&data_dir.join("blocks"))?);
        let state_db = Arc::new(StateDb::open(&data_dir.join("state_db"))?);

        // Build real ProducerSet with genesis producers
        let mut ps = ProducerSet::new();
        let bond_unit = network.bond_unit();
        for kp in &producers {
            ps.register_genesis_producer(*kp.public_key(), 1, bond_unit)
                .expect("register_genesis_producer failed");
        }

        // Build real epoch bond snapshot from producer set
        let mut epoch_bond_snapshot: HashMap<Hash, u64> = HashMap::new();
        for kp in &producers {
            let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, kp.public_key().as_bytes());
            epoch_bond_snapshot.insert(pubkey_hash, 1); // 1 bond each
        }

        // Build real producer liveness (all producers alive at height 0)
        let mut producer_liveness: HashMap<PublicKey, u64> = HashMap::new();
        for kp in &producers {
            producer_liveness.insert(*kp.public_key(), 0);
        }

        // Real chain state at genesis
        let spec = doli_core::chainspec::ChainSpec::devnet();
        let genesis_hash = spec.genesis_hash();
        let chain_state = ChainState::new(genesis_hash);
        state_db.put_chain_state(&chain_state)?;
        state_db.write_producer_set(&ps)?;

        let chain_state = Arc::new(RwLock::new(chain_state));
        let producer_set = Arc::new(RwLock::new(ps));
        let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));

        // Real mempool
        let oracle_sunset_triggered = Arc::new(AtomicBool::new(false));
        let mempool = Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::testnet(),
            params.clone(),
            network,
        )));
        mempool
            .write()
            .await
            .share_oracle_sunset_flag(oracle_sunset_triggered.clone());

        // Real sync manager
        let sync_config = SyncConfig::default();
        let sync_manager = Arc::new(RwLock::new(SyncManager::new(sync_config, genesis_hash)));
        {
            let mut sm = sync_manager.write().await;
            sm.set_bootstrap_grace_period_secs(0); // No grace period in tests
            sm.set_min_peers_for_production(0); // No peers needed in tests
        }

        // Real VDF calibrator (minimal iterations for speed)
        let vdf_calibrator = Arc::new(RwLock::new(VdfCalibrator::new(100, 55)));

        // Producer key: first producer in list
        let producer_key = producers.first().cloned();
        let bls_key = Some(crypto::BlsKeyPair::generate());

        // Config
        let config = NodeConfig {
            network,
            data_dir,
            listen_addr: "0.0.0.0:0".to_string(),
            bootstrap_nodes: Vec::new(),
            max_peers: 0,
            rpc: crate::config::RpcConfig::for_network(network),
            producer: None,
            no_dht: true,
            relay_server: false,
            genesis_time_override: Some(params.genesis_time),
            chainspec: None,
            slot_duration_override: Some(params.slot_duration),
            external_address: None,
            no_snap_sync: false,
            seed_mode: false,
            auto_checkpoint_interval: None,
            bootnode_enrs: Vec::new(),
            no_discv5: true,
            discv5_port: None,
        };

        Ok(Self {
            config,
            params,
            block_store,
            state_db,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None, // No networking in tests
            seed_peer_ids: Vec::new(),
            seeds_released: false,
            sync_manager,
            shutdown: Arc::new(RwLock::new(false)),
            producer_key,
            bls_key,
            last_produced_slot: None,
            // Empty known_producers: validation accepts any producer during
            // bootstrap when the bootstrap list is empty (line 173 of
            // validation/producer.rs). This enables Full mode in tests
            // without requiring producers to match slot-specific rank ordering.
            known_producers: Arc::new(RwLock::new(Vec::new())),
            first_peer_connected: None,
            equivocation_detector: Arc::new(RwLock::new(EquivocationDetector::new())),
            vdf_calibrator,
            fork_block_cache: Arc::new(RwLock::new(HashMap::new())),
            last_producer_list_change: None,
            producer_gset: Arc::new(RwLock::new(ProducerGSet::new(network.id(), genesis_hash))),
            adaptive_gossip: Arc::new(RwLock::new(AdaptiveGossip::new())),
            our_announcement: Arc::new(RwLock::new(None)),
            announcement_sequence: Arc::new(AtomicU64::new(0)),
            last_broadcast_gset_len: 0,
            signed_slots_db: None, // No double-sign DB in tests
            // --- Fork recovery state: ALL REAL ---
            shallow_rollback_count: 0,
            cumulative_rollback_depth: 0,
            seen_blocks_for_slot: HashSet::new(),
            epoch_state: doli_core::EpochState {
                producer_list: {
                    let mut pks: Vec<_> = producers.iter().map(|kp| *kp.public_key()).collect();
                    pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    pks
                },
                active_list: Vec::new(), // Built at first epoch boundary
                bond_snapshot: epoch_bond_snapshot,
                epoch: 0,
                attested_sets: [HashSet::new(), HashSet::new(), HashSet::new()],
                attestation_accum: [HashMap::new(), HashMap::new(), HashMap::new()],
                blocks_produced: HashMap::new(),
            },
            is_active_producer: true, // Active in tests
            last_active_status_epoch: None,
            // --- Non-fork-recovery fields: safe defaults ---
            vote_tx: None,
            pending_update: None,
            last_peer_redial: None,
            bootstrap_backoff: HashMap::new(),
            producer_liveness,
            genesis_vdf_output: None,
            cached_state_root: Arc::new(RwLock::new(None)),
            cached_genesis_producers: std::sync::OnceLock::new(),
            port_check_done: true,
            maintainer_state: None,
            archive_tx: None,
            pending_archive: std::collections::VecDeque::new(),
            archive_dir: None,
            archive_caught_up: true,
            ws_sender: Arc::new(RwLock::new(None)),
            minute_tracker: MinuteAttestationTracker::new(),
            rejected_fork_tips: HashSet::new(),
            snap_sync_height: None,
            sync_requests_this_interval: 0,
            last_checkpoint_height: 0,
            pending_tx_announcements: HashMap::new(),
            hardfork_schedule: updater::HardForkSchedule::for_network(network),
            peer_churn: HashMap::new(),
            last_integrity_check_tip: None,
            recovery_mode: Arc::new(AtomicBool::new(false)),
            oracle_sunset_triggered: oracle_sunset_triggered.clone(),
            health_window: std::collections::VecDeque::new(),
            attest_fetch_tracker: HashMap::new(),
            diagnostic_emitter: Arc::new(storage::diagnostic_ledger::emitter::NoOpEmitter)
                as Arc<dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter>,
            diagnostic_ledger: None,
            diagnostic_shutdown_tx: None,
            diagnostic_writer_stats: storage::diagnostic_ledger::DiagnosticWriterStats::new_shared(
            ),
            last_diagnostic_alerted: HashSet::new(),
        })
    }

    /// Create a headless Node for disaster recovery replay.
    ///
    /// Uses the EXISTING block_store from `data_dir/blocks/` and a FRESH state_db.
    /// No networking, no archiver — only the state transition machinery.
    ///
    /// Genesis producers are registered from the chainspec so `maybe_complete_genesis`
    /// works correctly during replay.
    pub async fn new_for_replay(data_dir: std::path::PathBuf, network: Network) -> Result<Self> {
        let spec = match network {
            Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
            Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
            Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
        };
        let genesis_hash = spec.genesis_hash();
        let params = ConsensusParams::for_network(network);

        std::fs::create_dir_all(&data_dir)?;
        let block_store = Arc::new(BlockStore::open(&data_dir.join("blocks"))?);
        let state_db = Arc::new(StateDb::open(&data_dir.join("state_db"))?);

        // Fresh state — replay builds everything from genesis
        let chain_state = ChainState::new(genesis_hash);
        state_db.put_chain_state(&chain_state)?;

        // Register genesis producers so maybe_complete_genesis can find them
        let mut ps = ProducerSet::new();
        let bond_unit = network.bond_unit();
        if network == Network::Mainnet {
            let genesis_producers = doli_core::genesis::mainnet_genesis_producers();
            for (pk, bonds) in &genesis_producers {
                let _ = ps.register_genesis_producer(*pk, *bonds, bond_unit);
            }
        }
        state_db.write_producer_set(&ps)?;

        let epoch_bond_snapshot: HashMap<Hash, u64> = ps
            .all_producers()
            .iter()
            .map(|p| {
                let pkh = hash_with_domain(ADDRESS_DOMAIN, p.public_key.as_bytes());
                (pkh, p.bonds() as u64)
            })
            .collect();

        let producer_liveness: HashMap<PublicKey, u64> = ps
            .all_producers()
            .iter()
            .map(|p| (p.public_key, 0))
            .collect();

        let epoch_producer_list: Vec<PublicKey> = {
            let mut pks: Vec<_> = ps.all_producers().iter().map(|p| p.public_key).collect();
            pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            pks
        };

        let chain_state = Arc::new(RwLock::new(chain_state));
        let producer_set = Arc::new(RwLock::new(ps));
        let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));

        let oracle_sunset_triggered = Arc::new(AtomicBool::new(false));

        let mempool = Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::testnet(),
            params.clone(),
            network,
        )));

        mempool
            .write()
            .await
            .share_oracle_sunset_flag(oracle_sunset_triggered.clone());

        let sync_config = SyncConfig::default();
        let sync_manager = Arc::new(RwLock::new(SyncManager::new(sync_config, genesis_hash)));
        {
            let mut sm = sync_manager.write().await;
            sm.set_bootstrap_grace_period_secs(0);
            sm.set_min_peers_for_production(0);
        }

        let vdf_calibrator = Arc::new(RwLock::new(VdfCalibrator::new(100, 55)));

        let config = NodeConfig {
            network,
            data_dir,
            listen_addr: "0.0.0.0:0".to_string(),
            bootstrap_nodes: Vec::new(),
            max_peers: 0,
            rpc: crate::config::RpcConfig::for_network(network),
            producer: None,
            no_dht: true,
            relay_server: false,
            genesis_time_override: Some(params.genesis_time),
            chainspec: None,
            slot_duration_override: Some(params.slot_duration),
            external_address: None,
            no_snap_sync: false,
            seed_mode: false,
            auto_checkpoint_interval: None,
            bootnode_enrs: Vec::new(),
            no_discv5: true,
            discv5_port: None,
        };

        Ok(Self {
            config,
            params,
            block_store,
            state_db,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None, // No networking — headless replay
            seed_peer_ids: Vec::new(),
            seeds_released: false,
            sync_manager,
            shutdown: Arc::new(RwLock::new(false)),
            producer_key: None, // No production key — replay only
            bls_key: None,
            last_produced_slot: None,
            known_producers: Arc::new(RwLock::new(epoch_producer_list.clone())),
            first_peer_connected: None,
            equivocation_detector: Arc::new(RwLock::new(EquivocationDetector::new())),
            vdf_calibrator,
            fork_block_cache: Arc::new(RwLock::new(HashMap::new())),
            last_producer_list_change: None,
            producer_gset: Arc::new(RwLock::new(ProducerGSet::new(network.id(), genesis_hash))),
            adaptive_gossip: Arc::new(RwLock::new(AdaptiveGossip::new())),
            our_announcement: Arc::new(RwLock::new(None)),
            announcement_sequence: Arc::new(AtomicU64::new(0)),
            last_broadcast_gset_len: 0,
            signed_slots_db: None,
            shallow_rollback_count: 0,
            cumulative_rollback_depth: 0,
            seen_blocks_for_slot: HashSet::new(),
            epoch_state: doli_core::EpochState {
                producer_list: epoch_producer_list,
                active_list: Vec::new(),
                bond_snapshot: epoch_bond_snapshot,
                epoch: 0,
                attested_sets: [HashSet::new(), HashSet::new(), HashSet::new()],
                attestation_accum: [HashMap::new(), HashMap::new(), HashMap::new()],
                blocks_produced: HashMap::new(),
            },
            is_active_producer: false, // No production during replay
            last_active_status_epoch: None,
            vote_tx: None,
            pending_update: None,
            last_peer_redial: None,
            bootstrap_backoff: HashMap::new(),
            producer_liveness,
            genesis_vdf_output: None,
            cached_state_root: Arc::new(RwLock::new(None)),
            cached_genesis_producers: std::sync::OnceLock::new(),
            port_check_done: true,
            maintainer_state: None,
            archive_tx: None,
            pending_archive: std::collections::VecDeque::new(),
            archive_dir: None,
            archive_caught_up: true,
            ws_sender: Arc::new(RwLock::new(None)),
            minute_tracker: MinuteAttestationTracker::new(),
            rejected_fork_tips: HashSet::new(),
            snap_sync_height: None,
            sync_requests_this_interval: 0,
            last_checkpoint_height: 0,
            pending_tx_announcements: HashMap::new(),
            hardfork_schedule: updater::HardForkSchedule::for_network(network),
            peer_churn: HashMap::new(),
            last_integrity_check_tip: None,
            recovery_mode: Arc::new(AtomicBool::new(false)),
            oracle_sunset_triggered: oracle_sunset_triggered.clone(),
            health_window: std::collections::VecDeque::new(),
            attest_fetch_tracker: HashMap::new(),
            diagnostic_emitter: Arc::new(storage::diagnostic_ledger::emitter::NoOpEmitter)
                as Arc<dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter>,
            diagnostic_ledger: None,
            diagnostic_shutdown_tx: None,
            diagnostic_writer_stats: storage::diagnostic_ledger::DiagnosticWriterStats::new_shared(
            ),
            last_diagnostic_alerted: HashSet::new(),
        })
    }
}
