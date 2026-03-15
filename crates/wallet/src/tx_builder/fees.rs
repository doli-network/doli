use anyhow::{anyhow, Result};

use crate::types::*;

/// Tiered fee multiplier (x100) matching `doli-core::consensus::fee_multiplier_x100()`.
///
/// | Pending     | Multiplier |
/// |-------------|------------|
/// | 0-4         | 100 (1.00x)|
/// | 5-9         | 150 (1.50x)|
/// | 10-19       | 200 (2.00x)|
/// | 20-49       | 300 (3.00x)|
/// | 50-99       | 450 (4.50x)|
/// | 100-199     | 650 (6.50x)|
/// | 200-299     | 850 (8.50x)|
/// | 300+        | 1000 (10x) |
const fn fee_multiplier_x100(pending_count: u32) -> u32 {
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

/// Calculate registration fee matching `doli-core::consensus::registration_fee()`.
///
/// Fee = BASE_REGISTRATION_FEE * multiplier / 100, capped at MAX_REGISTRATION_FEE.
fn registration_fee(pending_count: u32) -> u64 {
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (BASE_REGISTRATION_FEE as u128 * multiplier) / 100;
    (fee as u64).min(MAX_REGISTRATION_FEE)
}

/// Calculate the total cost for bond registration.
/// Returns (bond_cost, registration_fee, total).
pub fn calculate_registration_cost(
    bond_count: u32,
    pending_registrations: u32,
) -> Result<(u64, u64, u64)> {
    if bond_count == 0 {
        return Err(anyhow!("Bond count must be at least 1"));
    }
    if bond_count > MAX_BONDS_PER_PRODUCER {
        return Err(anyhow!(
            "Bond count exceeds maximum of {}",
            MAX_BONDS_PER_PRODUCER
        ));
    }

    let bond_cost = (bond_count as u64)
        .checked_mul(BOND_UNIT)
        .ok_or_else(|| anyhow!("Bond cost overflow"))?;

    // Registration fee uses the tiered multiplier table (matching node consensus)
    let reg_fee = registration_fee(pending_registrations);

    let total = bond_cost
        .checked_add(reg_fee)
        .ok_or_else(|| anyhow!("Total cost overflow"))?;

    Ok((bond_cost, reg_fee, total))
}

/// Calculate vesting penalty for a bond given its age in slots.
/// Returns penalty percentage (0, 25, 50, or 75).
pub fn vesting_penalty_pct(age_slots: u64) -> u8 {
    // Vesting schedule: Q1 (0-1yr) = 75%, Q2 (1-2yr) = 50%, Q3 (2-3yr) = 25%, Vested (3yr+) = 0%
    if age_slots >= VESTING_QUARTER_SLOTS * 3 {
        0 // Fully vested (3+ years)
    } else if age_slots >= VESTING_QUARTER_SLOTS * 2 {
        25 // Q3: 2-3 years
    } else if age_slots >= VESTING_QUARTER_SLOTS {
        50 // Q2: 1-2 years
    } else {
        75 // Q1: 0-1 year
    }
}

/// Calculate the net amount returned after withdrawal penalty.
pub fn calculate_withdrawal_net(bond_amount: u64, penalty_pct: u8) -> u64 {
    let penalty = bond_amount * penalty_pct as u64 / 100;
    bond_amount - penalty
}
