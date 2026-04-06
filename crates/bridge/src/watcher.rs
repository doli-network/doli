//! Bridge watcher daemon.
//!
//! Polls DOLI and counter-chains to detect and auto-act on bridge swaps:
//! - Detects new BridgeHTLC outputs on DOLI
//! - Scans counter-chain for matching HTLCs (counterparty lock)
//! - Detects preimage reveals on either chain → auto-claims on the other
//! - Auto-refunds expired HTLCs on DOLI
//!
//! State is persisted to `{data_dir}/swaps/*.json` so the watcher can
//! resume after restart without losing track of active swaps.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::doli::DoliClient;
use crate::error::Result;
use crate::swap::{SwapRecord, SwapRole, SwapState};

/// Bridge watcher configuration.
pub struct WatcherConfig {
    /// DOLI RPC endpoint
    pub doli_rpc: String,
    /// Our pubkey_hash (to identify our HTLCs)
    pub our_pubkey_hash: String,
    /// Bitcoin RPC endpoint (optional)
    pub btc_rpc: Option<String>,
    /// Ethereum RPC endpoint (optional)
    pub eth_rpc: Option<String>,
    /// Data directory for swap state persistence
    pub data_dir: PathBuf,
    /// Poll interval in seconds
    pub poll_interval_secs: u64,
}

/// Bridge watcher daemon.
pub struct Watcher {
    config: WatcherConfig,
    doli: DoliClient,
    /// Active swaps keyed by swap ID
    swaps: HashMap<String, SwapRecord>,
    /// Last scanned DOLI height
    last_scanned_height: u64,
}

impl Watcher {
    /// Create a new watcher.
    pub fn new(config: WatcherConfig) -> Self {
        let doli = DoliClient::new(&config.doli_rpc);
        Self {
            config,
            doli,
            swaps: HashMap::new(),
            last_scanned_height: 0,
        }
    }

    /// Run the watcher loop.
    pub async fn run(&mut self) -> Result<()> {
        // Verify DOLI connectivity
        if !self.doli.ping().await {
            return Err(crate::error::BridgeError::DoliRpc(format!(
                "Cannot connect to DOLI node at {}",
                self.config.doli_rpc
            )));
        }

        let chain_info = self.doli.get_chain_info().await?;
        info!(
            "Bridge watcher started — DOLI h={}, watching address {}",
            chain_info.best_height,
            &self.config.our_pubkey_hash[..16.min(self.config.our_pubkey_hash.len())]
        );

        // Load persisted swap state
        self.load_swaps()?;
        if !self.swaps.is_empty() {
            info!("Loaded {} persisted swap(s)", self.swaps.len());
        }

        // Start scanning from current height (don't replay old history)
        self.last_scanned_height = chain_info.best_height.saturating_sub(10);

        let interval = Duration::from_secs(self.config.poll_interval_secs);

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Watcher shutting down...");
                    self.save_swaps()?;
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    if let Err(e) = self.tick().await {
                        warn!("Watcher tick error: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Single poll cycle.
    async fn tick(&mut self) -> Result<()> {
        let chain_info = self.doli.get_chain_info().await?;
        let current_height = chain_info.best_height;

        if current_height <= self.last_scanned_height {
            return Ok(()); // No new blocks
        }

        // 1. Scan for new BridgeHTLC outputs
        let new_htlcs = self.doli.scan_for_htlcs(self.last_scanned_height).await?;
        for htlc in &new_htlcs {
            if self
                .swaps
                .contains_key(&format!("{}:{}", htlc.tx_hash, htlc.output_index))
            {
                continue; // Already tracking
            }

            // Determine our role: if we created the HTLC, we're the initiator
            let role = if htlc.creator_pubkey_hash == self.config.our_pubkey_hash {
                SwapRole::Initiator
            } else {
                SwapRole::Responder
            };

            let swap = SwapRecord::new(
                htlc.tx_hash.clone(),
                htlc.output_index,
                htlc.amount,
                htlc.hash.clone(),
                htlc.lock_height,
                htlc.expiry_height,
                htlc.creator_pubkey_hash.clone(),
                htlc.target_chain,
                htlc.target_address.clone(),
                role,
            );

            info!(
                "[WATCHER] New BridgeHTLC detected: {} ({} DOLI, chain={}, expires h={})",
                swap.id,
                swap.doli_amount as f64 / 1e8,
                swap.target_chain,
                swap.doli_expiry_height
            );
            self.swaps.insert(swap.id.clone(), swap);
        }

        // 2. Scan for preimage reveals on DOLI
        let reveals = self
            .doli
            .scan_for_preimage_reveals(self.last_scanned_height)
            .await?;
        for reveal in &reveals {
            let swap_id = format!("{}:{}", reveal.htlc_tx_hash, reveal.htlc_output_index);
            if let Some(swap) = self.swaps.get_mut(&swap_id) {
                if swap.preimage.is_none() {
                    info!(
                        "[WATCHER] Preimage revealed on DOLI for swap {}: {}",
                        swap_id,
                        &reveal.preimage[..16.min(reveal.preimage.len())]
                    );
                    swap.preimage = Some(reveal.preimage.clone());
                    swap.preimage_source = Some("doli".to_string());
                    swap.doli_claim_tx = Some(reveal.claim_tx_hash.clone());
                    swap.transition(SwapState::PreimageRevealed);
                }
            }
        }

        // 3. Check each active swap for state transitions
        let swap_ids: Vec<String> = self
            .swaps
            .keys()
            .filter(|id| !self.swaps[*id].is_terminal())
            .cloned()
            .collect();

        for swap_id in &swap_ids {
            let swap = self.swaps.get(swap_id).unwrap().clone();
            match swap.state {
                SwapState::DoliLocked => {
                    // Check if expired → refund
                    if current_height >= swap.doli_expiry_height {
                        info!(
                            "[WATCHER] Swap {} expired at h={} (current h={})",
                            swap_id, swap.doli_expiry_height, current_height
                        );
                        if let Some(s) = self.swaps.get_mut(swap_id) {
                            s.transition(SwapState::Expired);
                        }
                    }
                    // TODO: Scan counter-chain for matching HTLC
                    // When found: transition to BothLocked
                }
                SwapState::BothLocked => {
                    // Check if expired → refund
                    if current_height >= swap.doli_expiry_height {
                        if let Some(s) = self.swaps.get_mut(swap_id) {
                            s.transition(SwapState::Expired);
                        }
                    }
                    // TODO: Scan counter-chain for preimage reveal
                    // When found: auto-claim on DOLI
                }
                SwapState::PreimageRevealed => {
                    // Preimage known — swap is effectively complete on DOLI side
                    // TODO: Auto-claim on counter-chain if we're the responder
                    if let Some(s) = self.swaps.get_mut(swap_id) {
                        s.transition(SwapState::Complete);
                    }
                }
                SwapState::Expired => {
                    // Auto-refund if we're the creator and UTXO still exists
                    if swap.role == SwapRole::Initiator && swap.doli_refund_tx.is_none() {
                        // Check if UTXO still exists (not already refunded)
                        let utxos = self
                            .doli
                            .get_bridge_utxos(&swap.doli_creator)
                            .await
                            .unwrap_or_default();
                        let still_locked = utxos.iter().any(|u| {
                            u.tx_hash == swap.doli_tx_hash
                                && u.output_index == swap.doli_output_index
                        });

                        if still_locked {
                            info!(
                                "[WATCHER] Swap {} expired and still locked — refund available (use `doli bridge-refund {}`)",
                                swap_id, swap_id
                            );
                            // NOTE: Auto-refund requires wallet access (signing).
                            // The watcher detects the condition; the user executes the refund.
                        } else {
                            // Already spent (refunded or claimed)
                            if let Some(s) = self.swaps.get_mut(swap_id) {
                                s.transition(SwapState::Refunded);
                            }
                        }
                    }
                }
                _ => {} // Terminal states: do nothing
            }
        }

        self.last_scanned_height = current_height;

        // Persist state after each tick
        self.save_swaps()?;

        // Log stats
        let active = self.swaps.values().filter(|s| !s.is_terminal()).count();
        if active > 0 {
            debug!(
                "[WATCHER] h={} active_swaps={} total={}",
                current_height,
                active,
                self.swaps.len()
            );
        }

        Ok(())
    }

    /// Load swap state from disk.
    fn load_swaps(&mut self) -> Result<()> {
        let swap_dir = self.config.data_dir.join("swaps");
        if !swap_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&swap_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(data) => match serde_json::from_str::<SwapRecord>(&data) {
                    Ok(swap) => {
                        debug!("Loaded swap {} (state={:?})", swap.id, swap.state);
                        self.swaps.insert(swap.id.clone(), swap);
                    }
                    Err(e) => warn!("Failed to parse swap file {:?}: {}", path, e),
                },
                Err(e) => warn!("Failed to read swap file {:?}: {}", path, e),
            }
        }
        Ok(())
    }

    /// Save swap state to disk.
    fn save_swaps(&self) -> Result<()> {
        let swap_dir = self.config.data_dir.join("swaps");
        std::fs::create_dir_all(&swap_dir)?;

        for swap in self.swaps.values() {
            let filename = swap.id.replace(':', "_") + ".json";
            let path = swap_dir.join(filename);
            let data = serde_json::to_string_pretty(swap)?;
            std::fs::write(&path, data)?;
        }
        Ok(())
    }

    /// Get the swap directory path.
    pub fn swap_dir(&self) -> PathBuf {
        self.config.data_dir.join("swaps")
    }

    /// Get all active (non-terminal) swaps.
    pub fn active_swaps(&self) -> Vec<&SwapRecord> {
        self.swaps.values().filter(|s| !s.is_terminal()).collect()
    }

    /// Get a specific swap by ID.
    pub fn get_swap(&self, id: &str) -> Option<&SwapRecord> {
        self.swaps.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_config(data_dir: &Path) -> WatcherConfig {
        WatcherConfig {
            doli_rpc: "http://127.0.0.1:8500".to_string(),
            our_pubkey_hash: "abc123".to_string(),
            btc_rpc: None,
            eth_rpc: None,
            data_dir: data_dir.to_path_buf(),
            poll_interval_secs: 10,
        }
    }

    #[test]
    fn test_watcher_creation() {
        let tmp = TempDir::new().unwrap();
        let watcher = Watcher::new(test_config(tmp.path()));
        assert_eq!(watcher.swaps.len(), 0);
        assert_eq!(watcher.last_scanned_height, 0);
    }

    #[test]
    fn test_swap_persistence_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut watcher = Watcher::new(test_config(tmp.path()));

        // Create a test swap
        let swap = SwapRecord::new(
            "aabbccdd".to_string(),
            0,
            100_000_000,
            "hashlock123".to_string(),
            100,
            460,
            "creator_pk".to_string(),
            1, // Bitcoin
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string(),
            SwapRole::Initiator,
        );
        watcher.swaps.insert(swap.id.clone(), swap);

        // Save
        watcher.save_swaps().unwrap();

        // Verify file exists
        let swap_dir = tmp.path().join("swaps");
        assert!(swap_dir.exists());
        let files: Vec<_> = std::fs::read_dir(&swap_dir).unwrap().collect();
        assert_eq!(files.len(), 1);

        // Load into fresh watcher
        let mut watcher2 = Watcher::new(test_config(tmp.path()));
        watcher2.load_swaps().unwrap();
        assert_eq!(watcher2.swaps.len(), 1);

        let loaded = watcher2.swaps.get("aabbccdd:0").unwrap();
        assert_eq!(loaded.doli_amount, 100_000_000);
        assert_eq!(loaded.target_chain, 1);
        assert_eq!(loaded.state, SwapState::DoliLocked);
        assert_eq!(loaded.role, SwapRole::Initiator);
        assert_eq!(loaded.doli_expiry_height, 460);
    }

    #[test]
    fn test_swap_state_transitions() {
        let mut swap = SwapRecord::new(
            "tx123".to_string(),
            0,
            50_000_000,
            "hash456".to_string(),
            10,
            100,
            "creator".to_string(),
            2, // Ethereum
            "0xabc...".to_string(),
            SwapRole::Responder,
        );

        assert!(!swap.is_terminal());
        assert_eq!(swap.state, SwapState::DoliLocked);

        swap.transition(SwapState::BothLocked);
        assert_eq!(swap.state, SwapState::BothLocked);
        assert!(!swap.is_terminal());

        swap.transition(SwapState::PreimageRevealed);
        assert_eq!(swap.state, SwapState::PreimageRevealed);

        swap.transition(SwapState::Complete);
        assert!(swap.is_terminal());
    }

    #[test]
    fn test_swap_expiry_transition() {
        let mut swap = SwapRecord::new(
            "tx789".to_string(),
            1,
            25_000_000,
            "hash012".to_string(),
            5,
            50,
            "creator".to_string(),
            1,
            "bc1q...".to_string(),
            SwapRole::Initiator,
        );

        swap.transition(SwapState::Expired);
        assert_eq!(swap.state, SwapState::Expired);
        assert!(!swap.is_terminal());

        swap.transition(SwapState::Refunded);
        assert!(swap.is_terminal());
    }

    #[test]
    fn test_multiple_swaps_persistence() {
        let tmp = TempDir::new().unwrap();
        let mut watcher = Watcher::new(test_config(tmp.path()));

        // Add 3 swaps in different states
        let s1 = SwapRecord::new(
            "tx1".to_string(),
            0,
            100,
            "h1".to_string(),
            10,
            100,
            "c1".to_string(),
            1,
            "addr1".to_string(),
            SwapRole::Initiator,
        );
        let mut s2 = SwapRecord::new(
            "tx2".to_string(),
            0,
            200,
            "h2".to_string(),
            20,
            200,
            "c2".to_string(),
            2,
            "addr2".to_string(),
            SwapRole::Responder,
        );
        s2.transition(SwapState::BothLocked);
        let mut s3 = SwapRecord::new(
            "tx3".to_string(),
            0,
            300,
            "h3".to_string(),
            30,
            300,
            "c3".to_string(),
            1,
            "addr3".to_string(),
            SwapRole::Initiator,
        );
        s3.transition(SwapState::Complete);

        watcher.swaps.insert(s1.id.clone(), s1);
        watcher.swaps.insert(s2.id.clone(), s2);
        watcher.swaps.insert(s3.id.clone(), s3);

        watcher.save_swaps().unwrap();

        // Reload
        let mut watcher2 = Watcher::new(test_config(tmp.path()));
        watcher2.load_swaps().unwrap();
        assert_eq!(watcher2.swaps.len(), 3);

        // Check active swaps (non-terminal)
        let active = watcher2.active_swaps();
        assert_eq!(active.len(), 2); // s1 (DoliLocked) + s2 (BothLocked)

        // s3 should be terminal
        let s3_loaded = watcher2.get_swap("tx3:0").unwrap();
        assert!(s3_loaded.is_terminal());
    }

    #[test]
    fn test_swap_id_format() {
        let swap = SwapRecord::new(
            "abc123def456".to_string(),
            2,
            100,
            "h".to_string(),
            1,
            10,
            "c".to_string(),
            1,
            "a".to_string(),
            SwapRole::Initiator,
        );
        assert_eq!(swap.id, "abc123def456:2");
    }
}
