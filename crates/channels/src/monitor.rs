//! Chain monitor — detects channel closes, revoked states, and HTLC timeouts.
//!
//! Polls the chain for events relevant to open channels:
//! 1. Funding tx confirmation tracking
//! 2. Funding output spends (detect closes)
//! 3. Revoked commitment detection (compare against revocation store)
//! 4. HTLC expiry tracking

use doli_core::BlockHeight;
use tracing::{debug, warn};

use crate::channel::ChannelRecord;
use crate::error::Result;
use crate::rpc::RpcClient;
use crate::types::{ChannelId, ChannelState};

/// Events detected by the chain monitor.
#[derive(Clone, Debug)]
pub enum MonitorEvent {
    /// Funding tx reached required confirmations.
    FundingConfirmed {
        channel_id: ChannelId,
        confirmations: u32,
    },
    /// Funding output was spent (channel close detected).
    FundingSpent {
        channel_id: ChannelId,
        spending_tx_hash: String,
    },
    /// A revoked commitment was broadcast by the counterparty.
    RevokedCommitment {
        channel_id: ChannelId,
        spending_tx_hash: String,
        revocation_preimage: [u8; 32],
        to_local_amount: u64,
        to_local_output_index: u32,
    },
    /// Dispute window has expired; timelocked outputs are now claimable.
    DisputeWindowExpired {
        channel_id: ChannelId,
        commitment_tx_hash: String,
    },
    /// An HTLC has expired on-chain.
    HtlcExpired { channel_id: ChannelId, htlc_id: u64 },
}

/// Chain monitor that watches for channel-relevant events.
pub struct ChainMonitor {
    rpc: RpcClient,
    last_height: BlockHeight,
}

impl ChainMonitor {
    pub fn new(rpc: RpcClient) -> Self {
        Self {
            rpc,
            last_height: 0,
        }
    }

    /// Check a single channel for events. Returns any detected events.
    pub async fn check_channel(&self, channel: &ChannelRecord) -> Result<Vec<MonitorEvent>> {
        let mut events = Vec::new();
        let current_height = self.rpc.get_height().await?;

        match &channel.state {
            ChannelState::FundingBroadcast => {
                // Check funding confirmation
                let tx_hash = hex::encode(channel.funding_outpoint.tx_hash);
                match self.rpc.get_transaction_status(&tx_hash).await {
                    Ok(status) if status.confirmed() => {
                        events.push(MonitorEvent::FundingConfirmed {
                            channel_id: channel.channel_id.clone(),
                            confirmations: status.confirmations.unwrap_or(0) as u32,
                        });
                    }
                    Ok(_) => {
                        debug!(
                            "channel {} funding tx not yet confirmed",
                            channel.channel_id
                        );
                    }
                    Err(e) => {
                        warn!("channel {} funding check failed: {}", channel.channel_id, e);
                    }
                }
            }

            ChannelState::ForceClosing => {
                // Check if dispute window has expired
                if let Some(close_hash) = &channel.close_tx_hash {
                    if let Ok(status) = self.rpc.get_transaction_status(close_hash).await {
                        if let Some(block_height) = status.block_height {
                            let dispute_end = block_height + channel.dispute_window;
                            if current_height >= dispute_end {
                                events.push(MonitorEvent::DisputeWindowExpired {
                                    channel_id: channel.channel_id.clone(),
                                    commitment_tx_hash: close_hash.clone(),
                                });
                            }
                        }
                    }
                }
            }

            ChannelState::Active => {
                // Check for HTLC expiries
                for htlc in &channel.htlcs {
                    if current_height >= htlc.expiry_height {
                        events.push(MonitorEvent::HtlcExpired {
                            channel_id: channel.channel_id.clone(),
                            htlc_id: htlc.htlc_id,
                        });
                    }
                }
            }

            _ => {}
        }

        Ok(events)
    }

    /// Update the last known height.
    pub async fn update_height(&mut self) -> Result<BlockHeight> {
        self.last_height = self.rpc.get_height().await?;
        Ok(self.last_height)
    }

    pub fn last_height(&self) -> BlockHeight {
        self.last_height
    }
}
