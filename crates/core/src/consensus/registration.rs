use crate::network::Network;
use crate::types::{Amount, BlockHeight};

// ==================== Registration Queue Anti-DoS ====================

/// Maximum registrations allowed per block (prevents spam)
pub const MAX_REGISTRATIONS_PER_BLOCK: u32 = 5;

/// Base registration fee (0.001 DOLI = 100,000 base units)
/// This is the minimum fee when no registrations are pending.
pub const BASE_REGISTRATION_FEE: Amount = 100_000;

/// Maximum fee multiplier (10x = 1000/100)
/// Cap prevents plutocratic barriers - when fee hits cap, system enters "queue mode"
/// where wait time increases instead of price. Time is the scarce resource, not money.
pub const MAX_FEE_MULTIPLIER_X100: u32 = 1000;

/// Maximum registration fee (BASE * 10 = 0.01 DOLI)
/// This is the absolute cap regardless of queue size.
pub const MAX_REGISTRATION_FEE: Amount = BASE_REGISTRATION_FEE * 10;

/// Get fee multiplier x100 for a given pending count (deterministic table lookup)
///
/// Returns value in range [100, 1000] representing 1.00x to 10.00x multiplier.
/// Uses integer arithmetic only - no floats in consensus code.
///
/// Table:
/// - 0-4 pending:     100  (1.00x)
/// - 5-9 pending:     150  (1.50x)
/// - 10-19 pending:   200  (2.00x)
/// - 20-49 pending:   300  (3.00x)
/// - 50-99 pending:   450  (4.50x)
/// - 100-199 pending: 650  (6.50x)
/// - 200-299 pending: 850  (8.50x)
/// - 300+ pending:    1000 (10.00x cap)
///
/// Design: When fee reaches cap, the system enters "queue mode" where
/// wait time increases, not price. Time is the scarce resource, not money.
#[must_use]
pub const fn fee_multiplier_x100(pending_count: u32) -> u32 {
    if pending_count >= 300 {
        return 1000;
    }
    if pending_count >= 200 {
        return 850;
    }
    if pending_count >= 100 {
        return 650;
    }
    if pending_count >= 50 {
        return 450;
    }
    if pending_count >= 20 {
        return 300;
    }
    if pending_count >= 10 {
        return 200;
    }
    if pending_count >= 5 {
        return 150;
    }
    100
}

/// Calculate the registration fee based on pending queue size
///
/// Uses deterministic table lookup with 10x cap. No floats.
/// Fee = BASE_REGISTRATION_FEE * multiplier / 100
///
/// Examples:
/// - 0 pending:   0.001 DOLI (1.00x)
/// - 5 pending:   0.0015 DOLI (1.50x)
/// - 20 pending:  0.003 DOLI (3.00x)
/// - 100 pending: 0.0065 DOLI (6.50x)
/// - 300+ pending: 0.01 DOLI (10.00x cap)
///
/// # Arguments
/// - `pending_count`: Number of registrations currently pending
///
/// # Returns
/// The fee required for a new registration (capped at 10x base)
#[must_use]
pub fn registration_fee(pending_count: u32) -> Amount {
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (BASE_REGISTRATION_FEE as u128 * multiplier) / 100;
    // Double-check cap (should be redundant given table, but defensive)
    (fee as Amount).min(MAX_REGISTRATION_FEE)
}

/// Calculate the registration fee for a specific network
///
/// Uses network-specific base fee with same 10x cap multiplier.
/// Deterministic table lookup, no floats.
#[must_use]
pub fn registration_fee_for_network(pending_count: u32, network: Network) -> Amount {
    let base_fee = network.registration_base_fee();
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (base_fee as u128 * multiplier) / 100;
    // Cap at 10x base for this network
    let max_fee = base_fee * 10;
    (fee as Amount).min(max_fee)
}

/// A pending registration in the queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRegistration {
    /// Producer public key
    pub public_key: crypto::PublicKey,
    /// Bond amount committed
    pub bond_amount: Amount,
    /// Fee paid (burned on invalid, refunded on valid)
    pub fee_paid: Amount,
    /// Block height when submitted
    pub submitted_at: BlockHeight,
    /// Previous registration hash (for chaining)
    pub prev_registration_hash: crypto::Hash,
    /// Sequence number (for ordering)
    pub sequence_number: u64,
}

/// Registration queue with anti-DoS fee mechanism
///
/// Limits registrations per block and applies escalating fees
/// to prevent spam attacks while maintaining accessibility during
/// normal operation.
#[derive(Clone, Debug, Default)]
pub struct RegistrationQueue {
    /// Pending registrations awaiting inclusion
    pending: Vec<PendingRegistration>,
    /// Registrations processed in current block
    current_block_count: u32,
    /// Current block height
    current_block_height: BlockHeight,
}

impl RegistrationQueue {
    /// Create a new empty registration queue
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            current_block_count: 0,
            current_block_height: 0,
        }
    }

    /// Get the current required fee for a new registration
    pub fn current_fee(&self) -> Amount {
        registration_fee(self.pending.len() as u32)
    }

    /// Get the current required fee for a specific network
    pub fn current_fee_for_network(&self, network: Network) -> Amount {
        registration_fee_for_network(self.pending.len() as u32, network)
    }

    /// Get the number of pending registrations
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check if a new registration can be added to the current block
    pub fn can_add_to_block(&self) -> bool {
        self.current_block_count < MAX_REGISTRATIONS_PER_BLOCK
    }

    /// Check if a new registration can be added for a specific network
    pub fn can_add_to_block_for_network(&self, network: Network) -> bool {
        self.current_block_count < network.max_registrations_per_block()
    }

    /// Submit a registration to the queue
    ///
    /// # Arguments
    /// - `registration`: The pending registration to submit
    /// - `current_height`: Current block height
    ///
    /// # Returns
    /// - `Ok(())` if submitted successfully
    /// - `Err(reason)` if submission fails (insufficient fee, etc.)
    ///
    /// # Fee Handling
    /// The fee is deducted at submission time. If the registration is later
    /// found invalid during processing, the fee is burned. If valid, the
    /// fee is refunded (or applied as transaction fee).
    pub fn submit(
        &mut self,
        registration: PendingRegistration,
        _current_height: BlockHeight,
    ) -> Result<(), &'static str> {
        let required_fee = self.current_fee();

        if registration.fee_paid < required_fee {
            return Err("Insufficient registration fee");
        }

        self.pending.push(registration);
        Ok(())
    }

    /// Submit a registration with network-specific fee requirements
    pub fn submit_for_network(
        &mut self,
        registration: PendingRegistration,
        _current_height: BlockHeight,
        network: Network,
    ) -> Result<(), &'static str> {
        let required_fee = self.current_fee_for_network(network);

        if registration.fee_paid < required_fee {
            return Err("Insufficient registration fee");
        }

        self.pending.push(registration);
        Ok(())
    }

    /// Start processing a new block
    ///
    /// Resets the per-block registration counter.
    pub fn begin_block(&mut self, height: BlockHeight) {
        self.current_block_height = height;
        self.current_block_count = 0;
    }

    /// Process the next registration from the queue
    ///
    /// Returns the next registration to be processed, or None if:
    /// - Queue is empty
    /// - Block limit reached
    ///
    /// The caller is responsible for validating the registration
    /// and calling `mark_processed` with the result.
    pub fn next_registration(&mut self) -> Option<PendingRegistration> {
        if self.current_block_count >= MAX_REGISTRATIONS_PER_BLOCK {
            return None;
        }

        if self.pending.is_empty() {
            return None;
        }

        // FIFO order - take from front
        let reg = self.pending.remove(0);
        self.current_block_count += 1;
        Some(reg)
    }

    /// Process the next registration with network-specific block limit
    pub fn next_registration_for_network(
        &mut self,
        network: Network,
    ) -> Option<PendingRegistration> {
        if self.current_block_count >= network.max_registrations_per_block() {
            return None;
        }

        if self.pending.is_empty() {
            return None;
        }

        // FIFO order - take from front
        let reg = self.pending.remove(0);
        self.current_block_count += 1;
        Some(reg)
    }

    /// Mark a registration as processed
    ///
    /// # Arguments
    /// - `valid`: Whether the registration was valid
    ///
    /// # Returns
    /// The fee amount. If invalid, this should be burned.
    /// If valid, this can be refunded or kept as tx fee.
    pub fn mark_processed(&self, fee_paid: Amount, valid: bool) -> Amount {
        if valid {
            // Valid registration: fee can be refunded or kept as tx fee
            fee_paid
        } else {
            // Invalid registration: fee is burned
            fee_paid
        }
    }

    /// Get all pending registrations (for inspection)
    pub fn pending_registrations(&self) -> &[PendingRegistration] {
        &self.pending
    }

    /// Remove expired registrations (optional cleanup)
    ///
    /// Registrations that have been pending for too long may be removed.
    /// Their fees would be returned to them.
    pub fn prune_expired(
        &mut self,
        current_height: BlockHeight,
        max_age: BlockHeight,
    ) -> Vec<PendingRegistration> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|r| current_height.saturating_sub(r.submitted_at) > max_age)
            .cloned()
            .collect();

        self.pending
            .retain(|r| current_height.saturating_sub(r.submitted_at) <= max_age);

        expired
    }

    /// Clear the queue (for testing or emergency)
    pub fn clear(&mut self) {
        self.pending.clear();
        self.current_block_count = 0;
    }
}
