use crate::types::{Amount, Slot};

use super::constants::{
    withdrawal_penalty_rate_with_quarter, BOND_UNIT, MAX_BONDS_PER_PRODUCER, VESTING_QUARTER_SLOTS,
};

// ==================== Bond Types ====================

/// A single bond entry with its creation time.
///
/// Each bond tracks when it was created for vesting calculation.
/// Bonds vest over 4 years with decreasing withdrawal penalties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondEntry {
    /// Slot when this bond was created
    pub creation_slot: Slot,
    /// Amount staked (always BOND_UNIT = 10 DOLI)
    pub amount: Amount,
}

impl BondEntry {
    /// Create a new bond entry
    pub fn new(creation_slot: Slot) -> Self {
        Self {
            creation_slot,
            amount: BOND_UNIT,
        }
    }

    /// Calculate the age of this bond in slots
    pub fn age(&self, current_slot: Slot) -> Slot {
        current_slot.saturating_sub(self.creation_slot)
    }

    /// Calculate withdrawal penalty for this bond.
    /// Uses mainnet quarter duration. For network-aware code, use `penalty_rate_with_quarter`.
    pub fn penalty_rate(&self, current_slot: Slot) -> u8 {
        self.penalty_rate_with_quarter(current_slot, VESTING_QUARTER_SLOTS)
    }

    /// Calculate withdrawal penalty with custom quarter duration (network-aware).
    pub fn penalty_rate_with_quarter(&self, current_slot: Slot, quarter_slots: Slot) -> u8 {
        withdrawal_penalty_rate_with_quarter(self.age(current_slot), quarter_slots)
    }

    /// Calculate net amount if withdrawn now.
    /// Uses mainnet quarter duration. For network-aware code, use `withdrawal_amount_with_quarter`.
    pub fn withdrawal_amount(&self, current_slot: Slot) -> (Amount, Amount) {
        self.withdrawal_amount_with_quarter(current_slot, VESTING_QUARTER_SLOTS)
    }

    /// Calculate net amount with custom quarter duration (network-aware).
    pub fn withdrawal_amount_with_quarter(
        &self,
        current_slot: Slot,
        quarter_slots: Slot,
    ) -> (Amount, Amount) {
        let penalty_rate = self.penalty_rate_with_quarter(current_slot, quarter_slots);
        let penalty = (self.amount * penalty_rate as u64) / 100;
        let net = self.amount - penalty;
        (net, penalty)
    }

    /// Check if this bond is fully vested (4 quarters old).
    /// Uses mainnet quarter duration. For network-aware code, use `is_vested_with_quarter`.
    pub fn is_vested(&self, current_slot: Slot) -> bool {
        self.is_vested_with_quarter(current_slot, VESTING_QUARTER_SLOTS)
    }

    /// Check if fully vested with custom quarter duration (network-aware).
    pub fn is_vested_with_quarter(&self, current_slot: Slot, quarter_slots: Slot) -> bool {
        self.age(current_slot) >= 4 * quarter_slots
    }
}

/// Result of a bond withdrawal (FIFO, instant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithdrawalResult {
    /// Number of bonds withdrawn
    pub bond_count: u32,
    /// Net amount after penalty
    pub net_amount: Amount,
    /// Penalty amount (burned)
    pub penalty_amount: Amount,
    /// Destination pubkey hash
    pub destination: crypto::Hash,
}

/// Producer's bond holdings.
///
/// Tracks all bonds staked by a producer.
/// Selection weight is proportional to total bonds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProducerBonds {
    /// List of bonds (sorted by creation_slot, oldest first for FIFO)
    pub bonds: Vec<BondEntry>,
}

impl ProducerBonds {
    /// Create empty bond holdings
    pub fn new() -> Self {
        Self { bonds: Vec::new() }
    }

    /// Total number of active bonds
    pub fn bond_count(&self) -> u32 {
        self.bonds.len() as u32
    }

    /// Total staked value
    pub fn total_staked(&self) -> Amount {
        self.bonds.len() as Amount * BOND_UNIT
    }

    /// Selection weight (same as bond count)
    pub fn selection_weight(&self) -> u32 {
        self.bond_count()
    }

    /// Add new bonds
    pub fn add_bonds(&mut self, count: u32, creation_slot: Slot) -> Result<(), BondError> {
        let new_total = self.bond_count() + count;
        if new_total > MAX_BONDS_PER_PRODUCER {
            return Err(BondError::MaxBondsExceeded {
                current: self.bond_count(),
                requested: count,
                max: MAX_BONDS_PER_PRODUCER,
            });
        }

        for _ in 0..count {
            self.bonds.push(BondEntry::new(creation_slot));
        }

        Ok(())
    }

    /// Request withdrawal of bonds (FIFO - oldest first)
    ///
    /// Returns the withdrawal result with calculated amounts.
    /// Penalty is 100% burned (not sent anywhere).
    /// Withdrawal is instant — no delay.
    /// Uses mainnet quarter duration. For network-aware code, use `request_withdrawal_with_quarter`.
    pub fn request_withdrawal(
        &mut self,
        count: u32,
        current_slot: Slot,
        destination: crypto::Hash,
    ) -> Result<WithdrawalResult, BondError> {
        self.request_withdrawal_with_quarter(
            count,
            current_slot,
            destination,
            VESTING_QUARTER_SLOTS,
        )
    }

    /// Request withdrawal with custom quarter duration (network-aware).
    pub fn request_withdrawal_with_quarter(
        &mut self,
        count: u32,
        current_slot: Slot,
        destination: crypto::Hash,
        quarter_slots: Slot,
    ) -> Result<WithdrawalResult, BondError> {
        if count == 0 {
            return Err(BondError::ZeroWithdrawal);
        }

        if count > self.bond_count() {
            return Err(BondError::InsufficientBonds {
                requested: count,
                available: self.bond_count(),
            });
        }

        // Calculate amounts for FIFO withdrawal (oldest bonds first)
        let mut total_net: Amount = 0;
        let mut total_penalty: Amount = 0;

        // Remove oldest bonds first (FIFO)
        for _ in 0..count {
            if let Some(bond) = self.bonds.first() {
                let (net, penalty) =
                    bond.withdrawal_amount_with_quarter(current_slot, quarter_slots);
                total_net += net;
                total_penalty += penalty;
                self.bonds.remove(0);
            }
        }

        Ok(WithdrawalResult {
            bond_count: count,
            net_amount: total_net,
            penalty_amount: total_penalty,
            destination,
        })
    }

    /// Get summary of bonds by vesting quarter.
    /// Uses mainnet quarter duration. For network-aware code, use `maturity_summary_with_quarter`.
    pub fn maturity_summary(&self, current_slot: Slot) -> BondsMaturitySummary {
        self.maturity_summary_with_quarter(current_slot, VESTING_QUARTER_SLOTS)
    }

    /// Get summary with custom quarter duration (network-aware).
    pub fn maturity_summary_with_quarter(
        &self,
        current_slot: Slot,
        quarter_slots: Slot,
    ) -> BondsMaturitySummary {
        let mut summary = BondsMaturitySummary::default();

        for bond in &self.bonds {
            let quarters = bond.age(current_slot) / quarter_slots;
            match quarters {
                0 => summary.q1 += 1,
                1 => summary.q2 += 1,
                2 => summary.q3 += 1,
                _ => summary.vested += 1,
            }
        }

        summary
    }

    /// Calculate total penalty if all bonds withdrawn now.
    /// Uses mainnet quarter duration. For network-aware code, use `total_withdrawal_penalty_with_quarter`.
    pub fn total_withdrawal_penalty(&self, current_slot: Slot) -> Amount {
        self.total_withdrawal_penalty_with_quarter(current_slot, VESTING_QUARTER_SLOTS)
    }

    /// Calculate total penalty with custom quarter duration (network-aware).
    pub fn total_withdrawal_penalty_with_quarter(
        &self,
        current_slot: Slot,
        quarter_slots: Slot,
    ) -> Amount {
        self.bonds
            .iter()
            .map(|b| {
                b.withdrawal_amount_with_quarter(current_slot, quarter_slots)
                    .1
            })
            .sum()
    }
}

/// Summary of bonds by vesting quarter
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BondsMaturitySummary {
    /// Bonds in Q1 (0-6h, 75% penalty)
    pub q1: u32,
    /// Bonds in Q2 (6-12h, 50% penalty)
    pub q2: u32,
    /// Bonds in Q3 (12-18h, 25% penalty)
    pub q3: u32,
    /// Bonds fully vested (18h+, 0% penalty)
    pub vested: u32,
}

/// Errors related to bond operations
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BondError {
    /// Attempted to add bonds exceeding the maximum
    MaxBondsExceeded {
        current: u32,
        requested: u32,
        max: u32,
    },
    /// Attempted to withdraw more bonds than available
    InsufficientBonds { requested: u32, available: u32 },
    /// Attempted to withdraw zero bonds
    ZeroWithdrawal,
    /// No withdrawal is ready to claim
    NoClaimableWithdrawal,
    /// Amount not a multiple of BOND_UNIT
    InvalidAmount { amount: Amount },
}

impl std::fmt::Display for BondError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxBondsExceeded {
                current,
                requested,
                max,
            } => write!(
                f,
                "max bonds exceeded: have {}, requested {}, max {}",
                current, requested, max
            ),
            Self::InsufficientBonds {
                requested,
                available,
            } => write!(
                f,
                "insufficient bonds: requested {}, available {}",
                requested, available
            ),
            Self::ZeroWithdrawal => write!(f, "cannot withdraw zero bonds"),
            Self::NoClaimableWithdrawal => write!(f, "no withdrawal ready to claim"),
            Self::InvalidAmount { amount } => write!(
                f,
                "amount {} is not a multiple of bond unit {}",
                amount, BOND_UNIT
            ),
        }
    }
}

impl std::error::Error for BondError {}
