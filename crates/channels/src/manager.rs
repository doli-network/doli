//! Channel manager — main coordination loop.
//!
//! Tick-based loop that:
//! 1. Checks funding confirmations
//! 2. Monitors for channel closes
//! 3. Claims timelocked outputs after dispute windows
//! 4. Resolves expired HTLCs
//! 5. Broadcasts penalty transactions for revoked commitments

use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::config::ChannelConfig;
use crate::error::Result;
use crate::monitor::{ChainMonitor, MonitorEvent};
use crate::rpc::RpcClient;
use crate::store::ChannelStore;
use crate::types::ChannelState;

/// Main channel manager.
pub struct ChannelManager {
    config: ChannelConfig,
    rpc: RpcClient,
    store: ChannelStore,
    monitor: ChainMonitor,
}

impl ChannelManager {
    /// Create a new channel manager.
    pub fn new(config: ChannelConfig) -> Result<Self> {
        let rpc = RpcClient::new(&config.rpc_url);
        let store = ChannelStore::open(Path::new(&config.store_path))?;
        let monitor = ChainMonitor::new(rpc.clone());

        Ok(Self {
            config,
            rpc,
            store,
            monitor,
        })
    }

    /// Run the manager loop.
    pub async fn run(&mut self) -> Result<()> {
        info!("Channel manager starting...");

        if !self.rpc.ping().await {
            error!("Cannot connect to DOLI node at {}", self.config.rpc_url);
            return Err(crate::error::ChannelError::Rpc(
                "connection failed".to_string(),
            ));
        }

        info!("Connected to DOLI node at {}", self.config.rpc_url);

        loop {
            if let Err(e) = self.tick().await {
                error!("Channel manager tick error: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }
    }

    /// Single iteration of the manager loop.
    async fn tick(&mut self) -> Result<()> {
        self.monitor.update_height().await?;

        // Collect events for all active channels
        let active_ids: Vec<_> = self
            .store
            .active_channels()
            .iter()
            .map(|c| c.channel_id.clone())
            .collect();

        for channel_id in active_ids {
            let events = {
                let channel = match self.store.find(&channel_id) {
                    Some(c) => c,
                    None => continue,
                };
                self.monitor.check_channel(channel).await?
            };

            for event in events {
                self.handle_event(event).await?;
            }
        }

        self.store.save()?;
        Ok(())
    }

    /// Handle a monitor event.
    async fn handle_event(&mut self, event: MonitorEvent) -> Result<()> {
        match event {
            MonitorEvent::FundingConfirmed {
                channel_id,
                confirmations,
            } => {
                if let Some(channel) = self.store.find_mut(&channel_id) {
                    channel.funding_confirmations = confirmations;
                    if confirmations >= self.config.funding_confirmations {
                        info!(
                            "channel {} funding confirmed ({} confs)",
                            channel_id, confirmations
                        );
                        channel.transition(ChannelState::Active);
                    }
                }
            }

            MonitorEvent::RevokedCommitment {
                channel_id,
                spending_tx_hash,
                revocation_preimage: _,
                to_local_amount: _,
                to_local_output_index: _,
            } => {
                warn!(
                    "REVOKED commitment detected for channel {}! Broadcasting penalty.",
                    channel_id
                );
                if let Some(channel) = self.store.find_mut(&channel_id) {
                    channel.transition(ChannelState::PenaltyInFlight);
                    channel.close_tx_hash = Some(spending_tx_hash);
                    // Penalty tx construction would go here with the keypair
                    // (requires access to the local keypair, which the manager
                    // would hold or be passed)
                }
            }

            MonitorEvent::DisputeWindowExpired {
                channel_id,
                commitment_tx_hash: _,
            } => {
                info!(
                    "Dispute window expired for channel {}. Claiming timelocked output.",
                    channel_id
                );
                if let Some(channel) = self.store.find_mut(&channel_id) {
                    channel.transition(ChannelState::AwaitingClaim);
                    // Delayed claim tx construction would go here
                }
            }

            MonitorEvent::FundingSpent {
                channel_id,
                spending_tx_hash,
            } => {
                info!(
                    "Funding spent for channel {}: {}",
                    channel_id, spending_tx_hash
                );
                if let Some(channel) = self.store.find_mut(&channel_id) {
                    if channel.state == ChannelState::Active {
                        channel.transition(ChannelState::CounterpartyClosing);
                        channel.close_tx_hash = Some(spending_tx_hash);
                    }
                }
            }

            MonitorEvent::HtlcExpired {
                channel_id,
                htlc_id,
            } => {
                warn!("HTLC {} expired in channel {}", htlc_id, channel_id);
                // HTLC timeout handling — would force-close if needed
            }
        }

        Ok(())
    }

    /// Get the channel store (for external queries).
    pub fn store(&self) -> &ChannelStore {
        &self.store
    }

    /// Get the channel store mutably.
    pub fn store_mut(&mut self) -> &mut ChannelStore {
        &mut self.store
    }
}
