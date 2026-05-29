//! Pure LP-share UTXO selection for `pool remove` (INC-I-095).
//!
//! Extracted as a dependency-free helper so it can be unit-tested directly.

use anyhow::Result;

/// A minimal view of a wallet UTXO for LP-share selection.
#[derive(Debug)]
pub struct LpCandidate<'a> {
    pub output_type: &'a str,
    /// Embedded pool id (hex) for `lpShare` outputs; `None` for other types.
    pub pool_id: Option<&'a str>,
    pub amount: u64,
    pub tx_hash: &'a str,
    pub output_index: u32,
}

/// Select `lpShare` UTXOs to burn for `pool remove`, restricted to the TARGET pool.
///
/// INC-I-095: a wallet may hold LP shares from several pools (each `(asset_pair, fee_bps)`
/// has its own pool_id and LP-UTXO series, per the D2 invariant). The LPShare covenant
/// requires the matching pool_id to be spent on input 0, so an LP UTXO from a *different*
/// pool placed on a later input is rejected by the node with `[MPTX007]`. Selection MUST
/// therefore filter by the embedded pool_id, never spend a foreign-pool LP UTXO.
pub fn select_lp_share_utxos<'a>(
    candidates: &'a [LpCandidate<'a>],
    target_pool_id: &str,
    shares_to_burn: u64,
) -> Result<Vec<&'a LpCandidate<'a>>> {
    let mut selected = Vec::new();
    let mut lp_total = 0u64;
    let mut foreign_pool_shares = false;
    for c in candidates {
        if c.output_type != "lpShare" {
            continue;
        }
        if c.pool_id != Some(target_pool_id) {
            foreign_pool_shares = true;
            continue;
        }
        if lp_total >= shares_to_burn {
            break;
        }
        selected.push(c);
        lp_total += c.amount;
    }
    if lp_total < shares_to_burn {
        if foreign_pool_shares {
            anyhow::bail!(
                "Insufficient LP shares for pool {target_pool_id}. \
                 Available for this pool: {lp_total}, Required: {shares_to_burn}. \
                 You hold LP shares from other pool(s) that cannot be spent here — \
                 remove liquidity from those pools separately."
            );
        }
        anyhow::bail!(
            "Insufficient LP shares. Available: {}, Required: {}",
            lp_total,
            shares_to_burn
        );
    }
    Ok(selected)
}
