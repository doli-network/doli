//! # channels
//!
//! LN-Penalty payment channels for the DOLI protocol.
//!
//! Builds a full payment channel system on top of DOLI's existing L1 primitives:
//! 2-of-2 multisig, hashlocks, timelocks, composable And/Or conditions,
//! per-input signing (AnyoneCanPay), and the witness system.
//!
//! No consensus changes required — all constructs use existing output types.
//!
//! ## Architecture
//!
//! LN-Penalty (same design as Bitcoin's Lightning Network): each party stores
//! revocation preimages for all prior states. Broadcasting a revoked commitment
//! lets the counterparty sweep the full channel balance via the penalty path.
//!
//! ## Phases
//!
//! - **Phase 1**: Single channel lifecycle (open, pay, close, penalty)
//! - **Phase 2**: Multi-hop routing, invoices, HTLC forwarding
//! - **Phase 3**: Watchtower delegation, splice-in/out

pub mod channel;
pub mod close;
pub mod commitment;
pub mod conditions;
pub mod config;
pub mod error;
pub mod funding;
pub mod htlc;
pub mod invoice;
pub mod manager;
pub mod monitor;
pub mod payment;
pub mod protocol;
pub mod router;
pub mod rpc;
pub mod state_machine;
pub mod store;
pub mod types;
pub mod watchtower;

pub use channel::ChannelRecord;
pub use config::ChannelConfig;
pub use error::{ChannelError, Result};
pub use manager::ChannelManager;
pub use types::{ChannelBalance, ChannelId, ChannelState};
