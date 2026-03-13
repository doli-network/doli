//! Swap state machine for cross-chain atomic swaps.
//!
//! Each swap goes through a defined lifecycle:
//!
//! ```text
//! Detected → Locked(DOLI) → Locked(Both) → Claimed → Complete
//!                                        → Expired → Refunded
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The state of a cross-chain atomic swap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwapState {
    /// BridgeHTLC detected on DOLI chain, waiting for counterparty lock on target chain.
    DoliLocked,

    /// Both sides locked — waiting for one party to reveal preimage.
    BothLocked,

    /// Preimage discovered (via claim on either chain).
    /// The watcher should auto-claim on the other chain.
    PreimageRevealed,

    /// Successfully claimed on both chains.
    Complete,

    /// HTLC expired without claim. Refund path available.
    Expired,

    /// Refunded on DOLI chain.
    Refunded,

    /// Failed (error during processing).
    Failed(String),
}

/// Which side of the swap initiated (who locked first on DOLI).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwapRole {
    /// We initiated: locked DOLI, expecting counterparty to lock BTC.
    Initiator,
    /// Counterparty initiated: they locked DOLI, we need to lock BTC.
    Responder,
}

/// Record of a single cross-chain swap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwapRecord {
    /// Unique swap ID (derived from DOLI HTLC tx hash + output index)
    pub id: String,

    /// Current state of the swap
    pub state: SwapState,

    /// Our role in this swap
    pub role: SwapRole,

    /// Target chain (1=Bitcoin, 2=Ethereum, etc.)
    pub target_chain: u8,

    /// Target address on the counterparty chain
    pub target_address: String,

    // ── DOLI side ──────────────────────────────────────────────────────
    /// DOLI BridgeHTLC transaction hash
    pub doli_tx_hash: String,

    /// DOLI BridgeHTLC output index
    pub doli_output_index: u32,

    /// Amount locked on DOLI (in base units)
    pub doli_amount: u64,

    /// DOLI hashlock hash (BLAKE3 domain-separated)
    pub doli_hash: String,

    /// DOLI lock height (claim available after)
    pub doli_lock_height: u64,

    /// DOLI expiry height (refund available after)
    pub doli_expiry_height: u64,

    /// DOLI pubkey_hash of the HTLC creator
    pub doli_creator: String,

    // ── Counterparty chain side ────────────────────────────────────────
    /// Counterparty HTLC transaction hash (once detected)
    pub counter_tx_hash: Option<String>,

    /// Expected amount on counterparty chain (informational)
    pub counter_amount: Option<String>,

    /// SHA256 hash used on the counterparty chain (for Bitcoin HTLCs)
    pub counter_hash: Option<String>,

    // ── Preimage ───────────────────────────────────────────────────────
    /// The preimage (once discovered from a claim on either chain)
    pub preimage: Option<String>,

    /// Which chain revealed the preimage
    pub preimage_source: Option<String>,

    // ── Claim/refund results ───────────────────────────────────────────
    /// DOLI claim transaction hash (if we auto-claimed)
    pub doli_claim_tx: Option<String>,

    /// Counterparty claim transaction hash
    pub counter_claim_tx: Option<String>,

    /// DOLI refund transaction hash (if refunded)
    pub doli_refund_tx: Option<String>,

    // ── Timestamps ─────────────────────────────────────────────────────
    /// When the swap was first detected
    pub created_at: DateTime<Utc>,

    /// Last state update
    pub updated_at: DateTime<Utc>,
}

impl SwapRecord {
    /// Create a new swap record from a detected DOLI BridgeHTLC.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        doli_tx_hash: String,
        doli_output_index: u32,
        doli_amount: u64,
        doli_hash: String,
        doli_lock_height: u64,
        doli_expiry_height: u64,
        doli_creator: String,
        target_chain: u8,
        target_address: String,
        role: SwapRole,
    ) -> Self {
        let id = format!("{}:{}", doli_tx_hash, doli_output_index);
        let now = Utc::now();
        Self {
            id,
            state: SwapState::DoliLocked,
            role,
            target_chain,
            target_address,
            doli_tx_hash,
            doli_output_index,
            doli_amount,
            doli_hash,
            doli_lock_height,
            doli_expiry_height,
            doli_creator,
            counter_tx_hash: None,
            counter_amount: None,
            counter_hash: None,
            preimage: None,
            preimage_source: None,
            doli_claim_tx: None,
            counter_claim_tx: None,
            doli_refund_tx: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Transition to a new state.
    pub fn transition(&mut self, new_state: SwapState) {
        self.state = new_state;
        self.updated_at = Utc::now();
    }

    /// Check if this swap is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            SwapState::Complete | SwapState::Refunded | SwapState::Failed(_)
        )
    }
}
