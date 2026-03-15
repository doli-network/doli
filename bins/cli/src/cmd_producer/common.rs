use crate::rpc_client::{format_balance, BondDetailsInfo};

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
    let mut total_net: u64 = 0;
    let mut total_penalty: u64 = 0;
    let mut tiers: Vec<(u32, u8, u64, u64)> = Vec::new();

    let mut current_tier_pct: Option<u8> = None;
    let mut tier_count: u32 = 0;
    let mut tier_gross: u64 = 0;
    let mut tier_net: u64 = 0;

    for entry in details.bonds.iter().take(count as usize) {
        let pct = entry.penalty_pct;
        let penalty = (entry.amount * pct as u64) / 100;
        let net = entry.amount - penalty;
        total_net += net;
        total_penalty += penalty;

        if current_tier_pct == Some(pct) {
            tier_count += 1;
            tier_gross += entry.amount;
            tier_net += net;
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
