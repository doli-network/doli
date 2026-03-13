//! Bridge watcher daemon.
//!
//! The main coordination loop that:
//! 1. Polls DOLI for new BridgeHTLC outputs
//! 2. Polls Bitcoin for matching HTLC locks
//! 3. Detects preimage reveals on either chain
//! 4. Auto-claims on the other chain when preimage is discovered
//! 5. Auto-refunds expired HTLCs

use std::path::Path;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::bitcoin::BitcoinClient;
use crate::config::WatcherConfig;
use crate::doli::DoliClient;
use crate::error::Result;
use crate::store::SwapStore;
use crate::swap::{SwapRole, SwapState};

/// The bridge watcher daemon.
pub struct BridgeWatcher {
    config: WatcherConfig,
    doli: DoliClient,
    bitcoin: BitcoinClient,
    store: SwapStore,
    last_doli_height: u64,
    last_btc_height: u64,
}

impl BridgeWatcher {
    /// Create a new bridge watcher.
    pub fn new(config: WatcherConfig) -> Result<Self> {
        let doli = DoliClient::new(&config.doli_rpc);
        let bitcoin = BitcoinClient::new(&config.bitcoin_rpc, &config.bitcoin_auth);
        let store = SwapStore::open(Path::new(&config.swap_db_path))?;

        Ok(Self {
            config,
            doli,
            bitcoin,
            store,
            last_doli_height: 0,
            last_btc_height: 0,
        })
    }

    /// Run the watcher loop.
    pub async fn run(&mut self) -> Result<()> {
        info!("Bridge watcher starting...");

        // Verify connectivity
        if !self.doli.ping().await {
            error!("Cannot connect to DOLI node at {}", self.config.doli_rpc);
            return Err(crate::error::BridgeError::DoliRpc(
                "connection failed".to_string(),
            ));
        }
        info!("Connected to DOLI node at {}", self.config.doli_rpc);

        let btc_ok = self.bitcoin.ping().await;
        if btc_ok {
            info!("Connected to Bitcoin Core at {}", self.config.bitcoin_rpc);
        } else {
            warn!(
                "Cannot connect to Bitcoin Core at {} — Bitcoin monitoring disabled",
                self.config.bitcoin_rpc
            );
        }

        // Initialize heights
        self.last_doli_height = self.doli.height().await.unwrap_or(0);
        if btc_ok {
            self.last_btc_height = self.bitcoin.get_block_count().await.unwrap_or(0);
        }

        info!(
            "Starting from DOLI height {}, BTC height {}",
            self.last_doli_height, self.last_btc_height
        );
        info!(
            "Active swaps: {}, poll interval: {}s",
            self.store.active_count(),
            self.config.poll_interval_secs
        );

        loop {
            if let Err(e) = self.tick().await {
                error!("Watcher tick error: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }
    }

    /// Single iteration of the watcher loop.
    async fn tick(&mut self) -> Result<()> {
        // ── Step 1: Scan DOLI for new BridgeHTLCs ────────────────────────
        match self.doli.scan_for_htlcs(self.last_doli_height).await {
            Ok(htlcs) => {
                for htlc in htlcs {
                    let id = format!("{}:{}", htlc.tx_hash, htlc.output_index);
                    if self.store.find(&id).is_some() {
                        continue; // Already tracked
                    }

                    info!(
                        "New BridgeHTLC: {} amount={} chain={} target={}",
                        id,
                        htlc.amount,
                        doli_core::transaction::Output::bridge_chain_name(htlc.target_chain),
                        htlc.target_address,
                    );

                    let swap = crate::swap::SwapRecord::new(
                        htlc.tx_hash,
                        htlc.output_index,
                        htlc.amount,
                        htlc.hash,
                        htlc.lock_height,
                        htlc.expiry_height,
                        htlc.creator_pubkey_hash,
                        htlc.target_chain,
                        htlc.target_address,
                        SwapRole::Initiator, // Assume we're watching our own swaps
                    );
                    self.store.add(swap);
                }
            }
            Err(e) => {
                warn!("Error scanning DOLI for HTLCs: {}", e);
            }
        }

        // ── Step 2: Scan DOLI for preimage reveals ───────────────────────
        match self
            .doli
            .scan_for_preimage_reveals(self.last_doli_height)
            .await
        {
            Ok(reveals) => {
                for reveal in reveals {
                    let htlc_id = format!("{}:{}", reveal.htlc_tx_hash, reveal.htlc_output_index);
                    if let Some(swap) = self.store.find_mut(&htlc_id) {
                        if swap.preimage.is_none() {
                            info!(
                                "Preimage revealed on DOLI for swap {}: {}",
                                htlc_id, reveal.preimage
                            );
                            swap.preimage = Some(reveal.preimage.clone());
                            swap.preimage_source = Some("doli".to_string());
                            swap.doli_claim_tx = Some(reveal.claim_tx_hash);
                            swap.transition(SwapState::PreimageRevealed);
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Error scanning DOLI for reveals: {}", e);
            }
        }

        // Update DOLI height
        if let Ok(h) = self.doli.height().await {
            self.last_doli_height = h;
        }

        // ── Step 3: Scan Bitcoin for matching HTLCs ──────────────────────
        if self.bitcoin.ping().await {
            let watch_hashes: Vec<(String, String)> = self
                .store
                .active_swaps()
                .iter()
                .filter(|s| {
                    s.state == SwapState::DoliLocked
                        && s.target_chain == doli_core::transaction::BRIDGE_CHAIN_BITCOIN
                })
                .filter_map(|s| {
                    // We need SHA256(preimage) to watch for on Bitcoin.
                    // But we only know the BLAKE3 hash. If we have the preimage, compute SHA256.
                    // For initiated swaps, we should have stored the SHA256 hash.
                    s.counter_hash.as_ref().map(|h| (h.clone(), s.id.clone()))
                })
                .collect();

            if !watch_hashes.is_empty() {
                match self
                    .bitcoin
                    .scan_for_htlcs(self.last_btc_height, &watch_hashes)
                    .await
                {
                    Ok(detected) => {
                        for (htlc, swap_id) in detected {
                            if htlc.confirmations >= self.config.btc_min_confirmations {
                                if let Some(swap) = self.store.find_mut(&swap_id) {
                                    if swap.state == SwapState::DoliLocked {
                                        info!(
                                            "Bitcoin HTLC confirmed for swap {}: txid={} amount={}sat",
                                            swap_id, htlc.txid, htlc.amount_sat
                                        );
                                        swap.counter_tx_hash = Some(htlc.txid);
                                        swap.counter_amount =
                                            Some(format!("{} sat", htlc.amount_sat));
                                        swap.transition(SwapState::BothLocked);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Error scanning Bitcoin for HTLCs: {}", e);
                    }
                }
            }

            // ── Step 4: Scan Bitcoin for preimage reveals ────────────────
            let watch_outpoints: Vec<(String, u32, String)> = self
                .store
                .active_swaps()
                .iter()
                .filter(|s| {
                    s.state == SwapState::BothLocked
                        && s.counter_tx_hash.is_some()
                        && s.target_chain == doli_core::transaction::BRIDGE_CHAIN_BITCOIN
                })
                .filter_map(|s| {
                    s.counter_tx_hash
                        .as_ref()
                        .map(|txid| (txid.clone(), 0u32, s.id.clone())) // TODO: track vout
                })
                .collect();

            if !watch_outpoints.is_empty() {
                match self
                    .bitcoin
                    .scan_for_preimage_reveals(self.last_btc_height, &watch_outpoints)
                    .await
                {
                    Ok(reveals) => {
                        for (reveal, swap_id) in reveals {
                            if let Some(swap) = self.store.find_mut(&swap_id) {
                                if swap.preimage.is_none() {
                                    info!(
                                        "Preimage revealed on Bitcoin for swap {}: {}",
                                        swap_id, reveal.preimage
                                    );
                                    swap.preimage = Some(reveal.preimage);
                                    swap.preimage_source = Some("bitcoin".to_string());
                                    swap.counter_claim_tx = Some(reveal.claim_txid);
                                    swap.transition(SwapState::PreimageRevealed);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Error scanning Bitcoin for reveals: {}", e);
                    }
                }
            }

            // Update Bitcoin height
            if let Ok(h) = self.bitcoin.get_block_count().await {
                self.last_btc_height = h;
            }
        }

        // ── Step 5: Auto-claim when preimage is known ────────────────────
        if self.config.auto_claim {
            let claimable: Vec<String> = self
                .store
                .active_swaps()
                .iter()
                .filter(|s| s.state == SwapState::PreimageRevealed && s.preimage.is_some())
                .map(|s| s.id.clone())
                .collect();

            for swap_id in claimable {
                if let Some(swap) = self.store.find_mut(&swap_id) {
                    // If preimage came from Bitcoin, we need to claim on DOLI
                    if swap.preimage_source.as_deref() == Some("bitcoin")
                        && swap.doli_claim_tx.is_none()
                    {
                        info!(
                            "Auto-claiming DOLI for swap {} (preimage from Bitcoin)",
                            swap_id
                        );
                        // TODO: Build and broadcast DOLI claim transaction
                        // This requires wallet access and transaction construction.
                        // For now, log the preimage so the user can claim manually.
                        warn!(
                            "Manual claim needed: doli bridge-claim {}:{} --preimage {}",
                            swap.doli_tx_hash,
                            swap.doli_output_index,
                            swap.preimage.as_deref().unwrap_or("?")
                        );
                    }

                    // If preimage came from DOLI, the counterparty chain claim
                    // is the counterparty's responsibility (they see the preimage on-chain)
                    if swap.preimage_source.as_deref() == Some("doli") {
                        info!(
                            "Swap {}: preimage revealed on DOLI. Counterparty can now claim on {}.",
                            swap_id,
                            doli_core::transaction::Output::bridge_chain_name(swap.target_chain)
                        );
                        swap.transition(SwapState::Complete);
                    }
                }
            }
        }

        // ── Step 6: Auto-refund expired HTLCs ────────────────────────────
        if self.config.auto_refund {
            let current_height = self.last_doli_height;
            let expired: Vec<String> = self
                .store
                .active_swaps()
                .iter()
                .filter(|s| {
                    matches!(s.state, SwapState::DoliLocked | SwapState::BothLocked)
                        && s.doli_expiry_height > 0
                        && current_height >= s.doli_expiry_height
                        && s.preimage.is_none()
                })
                .map(|s| s.id.clone())
                .collect();

            for swap_id in expired {
                if let Some(swap) = self.store.find_mut(&swap_id) {
                    warn!(
                        "Swap {} expired at height {} (current: {}). Refund available.",
                        swap_id, swap.doli_expiry_height, current_height
                    );
                    swap.transition(SwapState::Expired);
                    // TODO: Auto-broadcast refund transaction
                    warn!(
                        "Manual refund: doli bridge-refund {}:{}",
                        swap.doli_tx_hash, swap.doli_output_index
                    );
                }
            }
        }

        // ── Save state ───────────────────────────────────────────────────
        self.store.save()?;

        Ok(())
    }

    /// Get a summary of all tracked swaps.
    pub fn status_summary(&self) -> String {
        let all = self.store.all_swaps();
        let active = self.store.active_count();
        let complete = all
            .iter()
            .filter(|s| s.state == SwapState::Complete)
            .count();
        let expired = all
            .iter()
            .filter(|s| s.state == SwapState::Expired || s.state == SwapState::Refunded)
            .count();

        format!(
            "Bridge Watcher: {} active, {} complete, {} expired/refunded, {} total",
            active,
            complete,
            expired,
            all.len()
        )
    }
}
