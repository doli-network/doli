//! Core channel types.

use crypto::Hash;
use doli_core::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

/// Unique channel identifier derived from the funding transaction outpoint.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub [u8; 32]);

impl ChannelId {
    /// Derive channel ID from the funding tx hash and output index.
    /// H(funding_tx_hash || output_index)
    pub fn from_funding_outpoint(tx_hash: &Hash, output_index: u32) -> Self {
        let mut hasher = crypto::Hasher::new();
        hasher.update(tx_hash.as_bytes());
        hasher.update(&output_index.to_le_bytes());
        let hash = hasher.finalize();
        Self(*hash.as_bytes())
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn short(&self) -> String {
        hex::encode(&self.0[..8])
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short())
    }
}

/// Channel lifecycle states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelState {
    /// Initial negotiation (OpenChannel sent, waiting for AcceptChannel).
    Opening,
    /// Funding tx constructed but not yet broadcast.
    FundingSigned,
    /// Funding tx broadcast, waiting for confirmation.
    FundingBroadcast,
    /// Channel is active and ready for off-chain payments.
    Active,
    /// Cooperative close initiated.
    CooperativeClosing,
    /// Unilateral close broadcast (our commitment).
    ForceClosing,
    /// Detected counterparty's unilateral close on-chain.
    CounterpartyClosing,
    /// Dispute window expired, funds claimable.
    AwaitingClaim,
    /// All funds settled on-chain.
    Closed,
    /// Revoked commitment detected — penalty in flight.
    PenaltyInFlight,
}

impl ChannelState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed)
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn is_closing(&self) -> bool {
        matches!(
            self,
            Self::CooperativeClosing
                | Self::ForceClosing
                | Self::CounterpartyClosing
                | Self::AwaitingClaim
                | Self::PenaltyInFlight
        )
    }
}

impl std::fmt::Display for ChannelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opening => write!(f, "Opening"),
            Self::FundingSigned => write!(f, "FundingSigned"),
            Self::FundingBroadcast => write!(f, "FundingBroadcast"),
            Self::Active => write!(f, "Active"),
            Self::CooperativeClosing => write!(f, "CooperativeClosing"),
            Self::ForceClosing => write!(f, "ForceClosing"),
            Self::CounterpartyClosing => write!(f, "CounterpartyClosing"),
            Self::AwaitingClaim => write!(f, "AwaitingClaim"),
            Self::Closed => write!(f, "Closed"),
            Self::PenaltyInFlight => write!(f, "PenaltyInFlight"),
        }
    }
}

/// Monotonically increasing commitment number.
pub type CommitmentNumber = u64;

/// Balance distribution within a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelBalance {
    /// Our balance in base units.
    pub local: Amount,
    /// Counterparty's balance in base units.
    pub remote: Amount,
}

impl ChannelBalance {
    pub fn new(local: Amount, remote: Amount) -> Self {
        Self { local, remote }
    }

    pub fn total(&self) -> Amount {
        self.local + self.remote
    }

    /// Apply a payment from local to remote. Returns error string if insufficient.
    pub fn pay_local_to_remote(&self, amount: Amount) -> Option<Self> {
        if amount > self.local {
            return None;
        }
        Some(Self {
            local: self.local - amount,
            remote: self.remote + amount,
        })
    }

    /// Apply a payment from remote to local.
    pub fn pay_remote_to_local(&self, amount: Amount) -> Option<Self> {
        if amount > self.remote {
            return None;
        }
        Some(Self {
            local: self.local + amount,
            remote: self.remote - amount,
        })
    }
}

/// Direction of a payment within the channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentDirection {
    /// We are sending to the counterparty.
    Outgoing,
    /// The counterparty is sending to us.
    Incoming,
}

/// State of an in-flight HTLC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtlcState {
    /// HTLC has been offered but not yet settled.
    Pending,
    /// HTLC preimage has been revealed — ready to settle.
    Fulfilled,
    /// HTLC has timed out — ready to refund.
    Expired,
    /// HTLC has been removed from commitment (settled or refunded).
    Resolved,
}

/// An in-flight HTLC attached to a commitment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InFlightHtlc {
    /// Unique ID within this channel.
    pub htlc_id: u64,
    /// Payment hash (hashlock).
    pub payment_hash: [u8; 32],
    /// Amount locked in this HTLC.
    pub amount: Amount,
    /// Block height at which this HTLC expires.
    pub expiry_height: BlockHeight,
    /// Direction: offered by us or received from counterparty.
    pub direction: PaymentDirection,
    /// Current state.
    pub state: HtlcState,
    /// Preimage (once revealed).
    pub preimage: Option<[u8; 32]>,
}

/// Funding outpoint reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingOutpoint {
    pub tx_hash: [u8; 32],
    pub output_index: u32,
}

impl FundingOutpoint {
    pub fn tx_hash_as_crypto(&self) -> Hash {
        Hash::from_bytes(self.tx_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash;

    // ── ChannelId ─────────────────────────────────────────────────────

    #[test]
    fn channel_id_deterministic() {
        let tx = hash(b"funding_tx");
        let id1 = ChannelId::from_funding_outpoint(&tx, 0);
        let id2 = ChannelId::from_funding_outpoint(&tx, 0);
        assert_eq!(id1, id2);
    }

    #[test]
    fn channel_id_differs_by_output_index() {
        let tx = hash(b"funding_tx");
        let id1 = ChannelId::from_funding_outpoint(&tx, 0);
        let id2 = ChannelId::from_funding_outpoint(&tx, 1);
        assert_ne!(id1, id2);
    }

    #[test]
    fn channel_id_differs_by_tx_hash() {
        let tx1 = hash(b"funding_tx_1");
        let tx2 = hash(b"funding_tx_2");
        let id1 = ChannelId::from_funding_outpoint(&tx1, 0);
        let id2 = ChannelId::from_funding_outpoint(&tx2, 0);
        assert_ne!(id1, id2);
    }

    #[test]
    fn channel_id_display() {
        let tx = hash(b"test");
        let id = ChannelId::from_funding_outpoint(&tx, 0);
        let short = id.short();
        assert_eq!(short.len(), 16); // 8 bytes = 16 hex chars
        let full = id.to_hex();
        assert_eq!(full.len(), 64); // 32 bytes = 64 hex chars
        assert!(full.starts_with(&short));
    }

    #[test]
    fn channel_id_serde_roundtrip() {
        let tx = hash(b"test");
        let id = ChannelId::from_funding_outpoint(&tx, 0);
        let json = serde_json::to_string(&id).unwrap();
        let decoded: ChannelId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }

    // ── ChannelState ──────────────────────────────────────────────────

    #[test]
    fn channel_state_terminal() {
        assert!(ChannelState::Closed.is_terminal());
        assert!(!ChannelState::Active.is_terminal());
        assert!(!ChannelState::Opening.is_terminal());
        assert!(!ChannelState::ForceClosing.is_terminal());
    }

    #[test]
    fn channel_state_active() {
        assert!(ChannelState::Active.is_active());
        assert!(!ChannelState::Opening.is_active());
        assert!(!ChannelState::Closed.is_active());
    }

    #[test]
    fn channel_state_closing() {
        assert!(ChannelState::CooperativeClosing.is_closing());
        assert!(ChannelState::ForceClosing.is_closing());
        assert!(ChannelState::CounterpartyClosing.is_closing());
        assert!(ChannelState::AwaitingClaim.is_closing());
        assert!(ChannelState::PenaltyInFlight.is_closing());
        assert!(!ChannelState::Active.is_closing());
        assert!(!ChannelState::Closed.is_closing());
        assert!(!ChannelState::Opening.is_closing());
    }

    #[test]
    fn channel_state_display() {
        assert_eq!(ChannelState::Active.to_string(), "Active");
        assert_eq!(ChannelState::ForceClosing.to_string(), "ForceClosing");
        assert_eq!(ChannelState::PenaltyInFlight.to_string(), "PenaltyInFlight");
    }

    #[test]
    fn channel_state_serde_roundtrip() {
        let states = vec![
            ChannelState::Opening,
            ChannelState::FundingSigned,
            ChannelState::FundingBroadcast,
            ChannelState::Active,
            ChannelState::CooperativeClosing,
            ChannelState::ForceClosing,
            ChannelState::CounterpartyClosing,
            ChannelState::AwaitingClaim,
            ChannelState::Closed,
            ChannelState::PenaltyInFlight,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let decoded: ChannelState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, decoded);
        }
    }

    // ── ChannelBalance ────────────────────────────────────────────────

    #[test]
    fn channel_balance_total() {
        let b = ChannelBalance::new(700, 300);
        assert_eq!(b.total(), 1000);
    }

    #[test]
    fn channel_balance_pay_local_to_remote() {
        let b = ChannelBalance::new(700, 300);
        let b2 = b.pay_local_to_remote(200).unwrap();
        assert_eq!(b2.local, 500);
        assert_eq!(b2.remote, 500);
        assert_eq!(b2.total(), b.total());
    }

    #[test]
    fn channel_balance_pay_remote_to_local() {
        let b = ChannelBalance::new(700, 300);
        let b2 = b.pay_remote_to_local(100).unwrap();
        assert_eq!(b2.local, 800);
        assert_eq!(b2.remote, 200);
        assert_eq!(b2.total(), b.total());
    }

    #[test]
    fn channel_balance_pay_insufficient() {
        let b = ChannelBalance::new(700, 300);
        assert!(b.pay_local_to_remote(701).is_none());
        assert!(b.pay_remote_to_local(301).is_none());
    }

    #[test]
    fn channel_balance_pay_exact() {
        let b = ChannelBalance::new(700, 300);
        let b2 = b.pay_local_to_remote(700).unwrap();
        assert_eq!(b2.local, 0);
        assert_eq!(b2.remote, 1000);
    }

    #[test]
    fn channel_balance_pay_zero() {
        let b = ChannelBalance::new(700, 300);
        let b2 = b.pay_local_to_remote(0).unwrap();
        assert_eq!(b2, b);
    }

    #[test]
    fn channel_balance_serde_roundtrip() {
        let b = ChannelBalance::new(700, 300);
        let json = serde_json::to_string(&b).unwrap();
        let decoded: ChannelBalance = serde_json::from_str(&json).unwrap();
        assert_eq!(b, decoded);
    }

    // ── FundingOutpoint ───────────────────────────────────────────────

    #[test]
    fn funding_outpoint_to_crypto_hash() {
        let fp = FundingOutpoint {
            tx_hash: [42u8; 32],
            output_index: 0,
        };
        let h = fp.tx_hash_as_crypto();
        assert_eq!(*h.as_bytes(), [42u8; 32]);
    }

    #[test]
    fn funding_outpoint_serde_roundtrip() {
        let fp = FundingOutpoint {
            tx_hash: [42u8; 32],
            output_index: 7,
        };
        let json = serde_json::to_string(&fp).unwrap();
        let decoded: FundingOutpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(fp, decoded);
    }
}
