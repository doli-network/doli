use super::*;

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
        let block_store = Arc::new(BlockStore::open(&blocks_path)?);

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
                let old_cs = if state_path.exists() {
                    ChainState::load(&state_path)?
                } else {
                    let spec = match config.network {
                        Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                        Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                        Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
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

        // Load state from StateDb (or create fresh genesis)
        let canonical_spec = match config.network {
            Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
            Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
            Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
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

        // Load UTXOs from StateDb into in-memory working set
        let utxo_set = {
            let utxo_count = state_db.utxo_len();
            if utxo_count > 0 {
                info!(
                    "[STATE_DB] Loading {} UTXOs into in-memory working set...",
                    utxo_count
                );
                let mut mem = storage::InMemoryUtxoStore::new();
                for (outpoint, entry) in state_db.iter_utxos() {
                    mem.insert(outpoint, entry);
                }
                info!("[STATE_DB] Loaded {} UTXOs", mem.len());
                UtxoSet::InMemory(mem)
            } else {
                UtxoSet::new()
            }
        };
        let utxo_set = Arc::new(RwLock::new(utxo_set));

        // Validate genesis hash against embedded chainspec (detect state_db corruption).
        let canonical = {
            let spec = match config.network {
                Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
                Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
                Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
            };
            spec.genesis_hash()
        };
        if chain_state.genesis_hash != canonical {
            warn!(
                "Genesis hash mismatch: state_db={} chainspec={} — NEW CHAIN DETECTED. Wiping all state and block store.",
                &chain_state.genesis_hash.to_string()[..16],
                &canonical.to_string()[..16],
            );
            // Wipe block store — old-chain blocks poison sync via "already in store" guard
            if let Err(e) = block_store.clear() {
                warn!("Failed to clear block store during genesis reset: {}", e);
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
                return Err(anyhow::anyhow!(
                    "Block store genesis mismatch!\n\
                     Block 1 prev_hash: {}\n\
                     Chainspec genesis:  {}\n\
                     The block store contains blocks from a different chain.\n\
                     Fix: wipe data directory ({}) and restart.",
                    block_one.header.prev_hash,
                    genesis_hash,
                    config.data_dir.display()
                ));
            }
        }

        // Verify chain state consistency with block store
        if chain_state.best_height > 0 {
            match block_store.get_block(&chain_state.best_hash) {
                Ok(Some(_tip_block)) => {
                    // Tip hash exists in store — chain state is consistent
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
            // Startup verification: ensure genesis producers match block store.
            // Catches stale ProducerSet persisted from a pre-reorg chain.
            let genesis_blocks = config.network.genesis_blocks();
            let best_height = chain_state.read().await.best_height;
            if genesis_blocks > 0 && best_height > genesis_blocks && set.active_count() > 0 {
                // Check if genesis blocks exist (snap-synced nodes may be missing them)
                let has_genesis_blocks =
                    block_store.get_block_by_height(1).ok().flatten().is_some();
                if !has_genesis_blocks {
                    info!(
                        "[STARTUP] Snap sync detected (genesis blocks missing). \
                         Trusting StateDb producer set ({} producers).",
                        set.active_count()
                    );
                } else {
                    let mut seen = std::collections::HashSet::new();
                    let mut chain_genesis_count = 0usize;
                    for h in 1..=genesis_blocks {
                        if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                            for tx in &block.transactions {
                                if tx.tx_type == TxType::Registration {
                                    if let Some(reg_data) = tx.registration_data() {
                                        if reg_data.vdf_output.len() == 32
                                            && seen.insert(reg_data.public_key)
                                        {
                                            chain_genesis_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if chain_genesis_count > 0 && chain_genesis_count != set.active_count() {
                        warn!(
                            "[STARTUP] [INTEGRITY_RISK] Genesis producer mismatch: StateDb has {} producers \
                             but block store has {}. Block store may be incomplete (snap sync gap). \
                             If this persists across restarts, wipe data and full-sync from genesis.",
                            set.active_count(),
                            chain_genesis_count
                        );
                    }
                }
            }

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
        let mempool = Arc::new(RwLock::new(Mempool::new(
            mempool_policy,
            params.clone(),
            config.network,
        )));

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

            // Configure min peers for production based on network and genesis phase.
            // During genesis, allow single-peer production since the network is
            // still bootstrapping with very few nodes. After the first epoch boundary,
            // recompute_tier() overrides with tier-aware minimums (Tier1=10, Tier2=5, Tier3=2).
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

        Ok(Self {
            config,
            params,
            block_store,
            state_db,
            utxo_set,
            chain_state,
            producer_set,
            mempool,
            network: None,
            sync_manager,
            shutdown,
            producer_key,
            bls_key,
            last_produced_slot: None,
            _last_production_check: Instant::now(),
            known_producers: Arc::new(RwLock::new(Vec::new())),
            first_peer_connected: None,
            equivocation_detector,
            vdf_calibrator,
            fork_block_cache: Arc::new(RwLock::new(HashMap::new())),
            last_resync_time: None,
            last_producer_list_change: None,
            producer_gset,
            adaptive_gossip,
            our_announcement: Arc::new(RwLock::new(None)),
            announcement_sequence: Arc::new(AtomicU64::new(0)),
            signed_slots_db,
            consecutive_fork_blocks: 0,
            shallow_rollback_count: 0,
            cumulative_rollback_depth: 0,
            cached_scheduler: None,
            our_tier: 0, // Computed on first block application
            last_tier_epoch: None,
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
        })
    }
}
