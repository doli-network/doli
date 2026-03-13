//! Multi-hop payment coordination (Phase 2).
//!
//! Coordinates atomic multi-hop payments by managing the HTLC chain
//! across multiple channels along a route.

use doli_core::Amount;
use serde::{Deserialize, Serialize};

use crate::router::Route;

/// Payment status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    /// Payment is being set up (HTLCs being added along route).
    Pending,
    /// All HTLCs locked, waiting for preimage reveal.
    InFlight,
    /// Payment succeeded (preimage propagated back).
    Succeeded,
    /// Payment failed (HTLC failed or timed out).
    Failed(String),
}

/// A multi-hop payment attempt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Payment {
    /// Payment hash.
    pub payment_hash: [u8; 32],
    /// Payment preimage (known only to the receiver; set on success).
    pub preimage: Option<[u8; 32]>,
    /// Total amount (including fees).
    pub total_amount: Amount,
    /// Amount received by the destination.
    pub destination_amount: Amount,
    /// Total fees paid across all hops.
    pub total_fees: Amount,
    /// Current status.
    pub status: PaymentStatus,
    /// Number of hops.
    pub hop_count: usize,
    /// Creation timestamp.
    pub created_at: u64,
}

impl Payment {
    /// Create a new payment from a route.
    pub fn from_route(payment_hash: [u8; 32], route: &Route, destination_amount: Amount) -> Self {
        let now = chrono::Utc::now().timestamp() as u64;
        Self {
            payment_hash,
            preimage: None,
            total_amount: route.total_amount,
            destination_amount,
            total_fees: route.total_fee,
            status: PaymentStatus::Pending,
            hop_count: route.hops.len(),
            created_at: now,
        }
    }

    /// Mark the payment as succeeded with the revealed preimage.
    pub fn succeed(&mut self, preimage: [u8; 32]) {
        self.preimage = Some(preimage);
        self.status = PaymentStatus::Succeeded;
    }

    /// Mark the payment as failed.
    pub fn fail(&mut self, reason: &str) {
        self.status = PaymentStatus::Failed(reason.to_string());
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            PaymentStatus::Succeeded | PaymentStatus::Failed(_)
        )
    }
}
