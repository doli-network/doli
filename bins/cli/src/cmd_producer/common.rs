use crate::rpc_client::{format_balance, BondDetailsInfo};
use doli_core::consensus::withdrawal_penalty_rate_with_quarter;
use doli_core::types::Slot;
use doli_core::validation::vesting::penalized_bond_net;

/// Format a slot duration as human-readable time (SLOT_DURATION = 10s)
pub(crate) fn format_slot_duration(slots: u64) -> String {
    let seconds = slots * 10;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("~{}h {}m", hours, minutes)
    } else {
        format!("~{}m", minutes)
    }
}

/// FIFO breakdown tier: (count, penalty_pct, gross_amount, net_amount)
pub(super) struct FifoBreakdown {
    pub(super) total_net: u64,
    pub(super) total_penalty: u64,
    pub(super) tiers: Vec<(u32, u8, u64, u64)>,
}

/// Compute FIFO breakdown for withdrawing `count` bonds (oldest first)
pub(super) fn compute_fifo_breakdown(details: &BondDetailsInfo, count: u32) -> FifoBreakdown {
    // The core ladder divides by the quarter unguarded; a zero or oversized node-supplied
    // quarter resolves to the worst tier instead of panicking.
    let quarter = Slot::try_from(details.vesting_quarter_slots)
        .ok()
        .filter(|q| *q > 0)
        .unwrap_or(Slot::MAX);

    let mut total_net: u64 = 0;
    let mut total_penalty: u64 = 0;
    let mut tiers: Vec<(u32, u8, u64, u64)> = Vec::new();

    let mut current_tier_pct: Option<u8> = None;
    let mut tier_count: u32 = 0;
    let mut tier_gross: u64 = 0;
    let mut tier_net: u64 = 0;

    for entry in details.bonds.iter().take(count as usize) {
        // The tier comes from the ladder, never from the node-supplied penalty_pct field.
        let reference_slot = entry
            .creation_slot
            .saturating_add(Slot::try_from(entry.age_slots).unwrap_or(Slot::MAX));
        let age = reference_slot.saturating_sub(entry.creation_slot);
        let pct = withdrawal_penalty_rate_with_quarter(age, quarter);
        let net = penalized_bond_net(entry.amount, entry.creation_slot, reference_slot, quarter);
        let penalty = entry.amount - net;
        total_net = total_net.saturating_add(net);
        total_penalty = total_penalty.saturating_add(penalty);

        if current_tier_pct == Some(pct) {
            tier_count += 1;
            tier_gross = tier_gross.saturating_add(entry.amount);
            tier_net = tier_net.saturating_add(net);
        } else {
            if let Some(prev_pct) = current_tier_pct {
                tiers.push((tier_count, prev_pct, tier_gross, tier_net));
            }
            current_tier_pct = Some(pct);
            tier_count = 1;
            tier_gross = entry.amount;
            tier_net = net;
        }
    }
    if let Some(pct) = current_tier_pct {
        tiers.push((tier_count, pct, tier_gross, tier_net));
    }

    FifoBreakdown {
        total_net,
        total_penalty,
        tiers,
    }
}

/// Display FIFO breakdown table
pub(super) fn display_fifo_breakdown(breakdown: &FifoBreakdown) {
    for (cnt, pct, gross, net) in &breakdown.tiers {
        let tier_label = match pct {
            0 => "vested (0% penalty)".to_string(),
            p => format!("Q{} ({}% penalty)", (4 - p / 25), p),
        };
        println!(
            "  {} x {}: {} -> {} ({} burned)",
            cnt,
            tier_label,
            format_balance(*gross),
            format_balance(*net),
            format_balance(gross - net)
        );
    }
    if breakdown.tiers.len() > 1 {
        let total_gross = breakdown.total_net + breakdown.total_penalty;
        println!("  {:-<50}", "");
        println!(
            "  Total: {} -> {} ({} burned)",
            format_balance(total_gross),
            format_balance(breakdown.total_net),
            format_balance(breakdown.total_penalty)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc_client::{BondEntryInfo, BondsSummaryInfo};

    const QUARTER: u64 = 2_160;
    const PERIOD: u64 = 4 * QUARTER;
    const CURRENT_SLOT: u64 = 10_000;

    fn entry(creation_slot: u32, amount: u64, penalty_pct: u8) -> BondEntryInfo {
        let age_slots = CURRENT_SLOT.saturating_sub(creation_slot as u64);
        BondEntryInfo {
            creation_slot,
            amount,
            age_slots,
            penalty_pct,
            vested: age_slots >= PERIOD,
            maturation_slot: creation_slot as u64 + PERIOD,
        }
    }

    fn details(bonds: Vec<BondEntryInfo>, quarter: u64) -> BondDetailsInfo {
        let total_staked = bonds.iter().map(|b| b.amount).sum();
        BondDetailsInfo {
            bond_count: bonds.len() as u32,
            total_staked,
            summary: BondsSummaryInfo {
                q1: 0,
                q2: 0,
                q3: 0,
                vested: 0,
            },
            bonds,
            withdrawal_pending_count: 0,
            vesting_quarter_slots: quarter,
            vesting_period_slots: 4 * quarter,
        }
    }

    /// Four tiers in one FIFO list, oldest first, at the testnet quarter.
    fn four_tier_fixture() -> BondDetailsInfo {
        details(
            vec![
                entry(1_000, 1_000, 0),
                entry(2_000, 2_000, 0),
                entry(4_000, 401, 25),
                entry(6_000, 303, 50),
                entry(8_000, 101, 75),
                entry(9_000, 200, 75),
            ],
            QUARTER,
        )
    }

    // REQ-VEST-002 (Must) — Decision: a change here reveals that the withdrawal preview the
    // operator signs off on no longer matches the payout the node will accept.
    #[test]
    fn req_vest_002_fifo_breakdown_four_tier_golden() {
        let b = compute_fifo_breakdown(&four_tier_fixture(), 6);
        assert_eq!(b.total_net, 3_529, "REQ-VEST-002: four-tier net");
        assert_eq!(b.total_penalty, 476, "REQ-VEST-002: four-tier burn");
        assert_eq!(
            b.tiers,
            vec![
                (2u32, 0u8, 3_000u64, 3_000u64),
                (1, 25, 401, 301),
                (1, 50, 303, 152),
                (2, 75, 301, 76),
            ],
            "REQ-VEST-002: consecutive equal tiers collapse and stay in FIFO order"
        );
        assert_eq!(b.total_net + b.total_penalty, 4_005);
    }

    // REQ-VEST-002 (Must) / INV-VEST-004 — Decision: a net of 25 here means the node rounds the
    // net instead of the penalty and rejects every honest Q1 withdrawal from the activation height.
    #[test]
    fn req_vest_002_penalty_first_truncation_101_at_q1_nets_26() {
        let b = compute_fifo_breakdown(&details(vec![entry(9_000, 101, 75)], QUARTER), 1);
        assert_eq!(b.total_net, 26, "INV-VEST-004: 101 at 75% nets 26, not 25");
        assert_eq!(b.total_penalty, 75);
        assert_eq!(b.tiers, vec![(1u32, 75u8, 101u64, 26u64)]);
    }

    // REQ-VEST-002 (Must) — Decision: reveals a FIFO order break, which would burn the operator's
    // newest bonds while reporting the oldest.
    #[test]
    fn req_vest_002_partial_count_takes_the_oldest_prefix() {
        let b = compute_fifo_breakdown(&four_tier_fixture(), 3);
        assert_eq!(b.total_net, 3_301);
        assert_eq!(b.total_penalty, 100);
        assert_eq!(
            b.tiers,
            vec![(2u32, 0u8, 3_000u64, 3_000u64), (1, 25, 401, 301)],
            "REQ-VEST-002: a partial withdrawal takes the OLDEST bonds"
        );
    }

    // REQ-VEST-002 (Must) — Decision: reveals a panic or a phantom tier on the empty preview the
    // CLI renders before any bond is selected.
    #[test]
    fn req_vest_002_zero_count_yields_no_tiers() {
        let b = compute_fifo_breakdown(&four_tier_fixture(), 0);
        assert_eq!(b.total_net, 0);
        assert_eq!(b.total_penalty, 0);
        assert!(b.tiers.is_empty());
    }

    // REQ-VEST-002 (Must) — Decision: reveals a silent under-count when the caller asks for more
    // bonds than the node reported.
    #[test]
    fn req_vest_002_count_beyond_the_bond_list_takes_every_bond() {
        let b = compute_fifo_breakdown(&four_tier_fixture(), 99);
        assert_eq!(b.total_net, 3_529);
        assert_eq!(b.total_penalty, 476);
        assert_eq!(b.tiers.len(), 4);
    }

    // REQ-VEST-002 (Must) — Decision: reveals that the CLI still multiplies in u64, which panics in
    // a debug build and wraps to a wrong payout in release on a large bond.
    #[test]
    fn inc_i_171_m7_large_amount_does_not_overflow_the_penalty_multiply() {
        let b = compute_fifo_breakdown(&details(vec![entry(10_000, u64::MAX, 75)], QUARTER), 1);
        assert_eq!(
            b.total_net, 4_611_686_018_427_387_904,
            "REQ-VEST-002: the widened core ladder must not wrap on a u64::MAX bond"
        );
        assert_eq!(b.total_penalty, 13_835_058_055_282_163_711);
        assert_eq!(
            b.tiers,
            vec![(1u32, 75u8, u64::MAX, 4_611_686_018_427_387_904u64)]
        );
    }

    // REQ-VEST-002 (Must) — Decision: reveals that a node-supplied percentage still drives CLI
    // arithmetic, so a hostile or buggy RPC can panic the wallet or misreport the burn.
    #[test]
    fn inc_i_171_m7_node_supplied_percentage_above_the_ladder_is_ignored() {
        let mut hostile = entry(10_000, 1_000, 200);
        hostile.age_slots = 0;
        let b = compute_fifo_breakdown(&details(vec![hostile], QUARTER), 1);
        assert_eq!(
            b.total_net, 250,
            "REQ-VEST-002: the tier comes from the core ladder, never from the RPC field"
        );
        assert_eq!(b.total_penalty, 750);
        assert_eq!(b.tiers, vec![(1u32, 75u8, 1_000u64, 250u64)]);
    }

    // REQ-VEST-002 (Must) — Decision: reveals that consuming the core ladder introduced a
    // divide-by-zero panic on a node-supplied zero quarter.
    #[test]
    fn inc_i_171_m7_zero_vesting_quarter_from_the_node_does_not_panic() {
        let b = compute_fifo_breakdown(&details(vec![entry(9_000, 1_000, 75)], 0), 1);
        assert_eq!(
            b.total_net + b.total_penalty,
            1_000,
            "REQ-VEST-002: gross is conserved whatever tier a zero quarter resolves to"
        );
        assert!(b.total_net <= 1_000);
        assert_eq!(b.tiers.len(), 1);
    }
}
