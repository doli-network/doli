//! Output-creation authority (INC-I-234): pool placement, pool consumption,
//! per-key FA/LP conservation and input-anchored FA issuance.

use std::collections::BTreeMap;

use crypto::Hash;

use crate::transaction::{Output, OutputType, Transaction, TxType};

use super::{ValidationContext, ValidationError};

fn err(code: &str, msg: String) -> ValidationError {
    ValidationError::InvalidTransaction(format!("[ERRTX-{code}] {msg}"))
}

fn inactive(ctx: &ValidationContext) -> bool {
    ctx.current_height < ctx.inc_i_234_activation_height
}

fn is_amm(ty: TxType) -> bool {
    matches!(
        ty,
        TxType::CreatePool | TxType::Swap | TxType::AddLiquidity | TxType::RemoveLiquidity
    )
}

/// Stateless placement rule: Pool only at output[0] of an AMM tx (exactly one); coinbase is Normal-only.
pub fn check_output_placement(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if inactive(ctx) {
        return Ok(());
    }
    if tx.is_coinbase() {
        if let Some((i, o)) = tx
            .outputs
            .iter()
            .enumerate()
            .find(|(_, o)| o.output_type != OutputType::Normal)
        {
            return Err(err(
                "AUTH005",
                format!(
                    "coinbase output {i} has type {:?}, only Normal allowed",
                    o.output_type
                ),
            ));
        }
        return Ok(());
    }
    let pools: Vec<usize> = tx
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.output_type == OutputType::Pool)
        .map(|(i, _)| i)
        .collect();
    if is_amm(tx.tx_type) {
        if pools != [0] {
            return Err(err(
                "AUTH001",
                format!(
                    "{:?} must carry exactly one Pool output at index 0, found at {pools:?}",
                    tx.tx_type
                ),
            ));
        }
    } else if !pools.is_empty() {
        return Err(err(
            "AUTH001",
            format!(
                "{:?} may not create Pool outputs, found at {pools:?}",
                tx.tx_type
            ),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Fa(Hash),
    Lp(Hash),
}

#[derive(Default)]
struct Tally {
    input: u128,
    has_input: bool,
    output: u128,
    supplies: Vec<u64>,
}

fn key_of(o: &Output) -> Result<Option<(Key, u64)>, ValidationError> {
    match o.output_type {
        OutputType::FungibleAsset => match o.fungible_asset_metadata() {
            Some((id, supply, _)) => Ok(Some((Key::Fa(id), supply))),
            None => Err(err("AUTH003", "malformed FungibleAsset metadata".into())),
        },
        OutputType::LPShare => match o.lp_share_metadata() {
            Some(id) => Ok(Some((Key::Lp(id), 0))),
            None => Err(err("AUTH003", "malformed LPShare metadata".into())),
        },
        _ => Ok(None),
    }
}

fn check_pool_inputs(tx: &Transaction, consumed: &[Output]) -> Result<(), ValidationError> {
    let pools: Vec<usize> = consumed
        .iter()
        .enumerate()
        .filter(|(_, o)| o.output_type == OutputType::Pool)
        .map(|(i, _)| i)
        .collect();
    let needs_pool = matches!(
        tx.tx_type,
        TxType::Swap | TxType::AddLiquidity | TxType::RemoveLiquidity
    );
    if needs_pool && pools != [0] {
        return Err(err(
            "AUTH002",
            format!(
                "{:?} must consume exactly one Pool at input 0, found at {pools:?}",
                tx.tx_type
            ),
        ));
    }
    if !needs_pool && !pools.is_empty() {
        return Err(err(
            "AUTH002",
            format!(
                "{:?} may not consume Pool inputs, found at {pools:?}",
                tx.tx_type
            ),
        ));
    }
    Ok(())
}

/// Keys bound by amm.rs for this tx's own pool: (asset_b FA key, own LP key).
fn amm_exempt(
    tx: &Transaction,
    consumed: &[Output],
) -> Result<Option<(Key, Key)>, ValidationError> {
    let pool = match tx.tx_type {
        TxType::CreatePool => tx.outputs.first(),
        TxType::Swap | TxType::AddLiquidity | TxType::RemoveLiquidity => consumed.first(),
        _ => return Ok(None),
    };
    match pool.and_then(Output::pool_metadata) {
        Some(m) => Ok(Some((Key::Fa(m.asset_b_id), Key::Lp(m.pool_id)))),
        None => Err(err(
            "AUTH002",
            format!("{:?} pool metadata missing or malformed", tx.tx_type),
        )),
    }
}

/// UTXO-aware rule: pool consumption shape, Σout ≤ Σin per FA/LP key, and FA issuance.
pub fn check_value_authority(
    tx: &Transaction,
    consumed_outputs: &[Output],
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if inactive(ctx) {
        return Ok(());
    }
    check_pool_inputs(tx, consumed_outputs)?;
    let exempt = amm_exempt(tx, consumed_outputs)?;

    let mut tally: BTreeMap<Key, Tally> = BTreeMap::new();
    for o in consumed_outputs {
        if let Some((k, _)) = key_of(o)? {
            let t = tally.entry(k).or_default();
            t.input += u128::from(o.amount);
            t.has_input = true;
        }
    }
    for o in &tx.outputs {
        if let Some((k, supply)) = key_of(o)? {
            let t = tally.entry(k).or_default();
            t.output += u128::from(o.amount);
            t.supplies.push(supply);
        }
    }

    let mut fresh = 0usize;
    for (key, t) in &tally {
        if exempt.is_some_and(|(fa, lp)| *key == fa || *key == lp) {
            continue;
        }
        match key {
            Key::Fa(id) if !t.has_input => {
                fresh += 1;
                check_issuance(tx, id, t, fresh)?;
            }
            _ if t.output > t.input => {
                return Err(err(
                    "AUTH003",
                    format!("{key:?} outputs {} exceed inputs {}", t.output, t.input),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn check_issuance(
    tx: &Transaction,
    id: &Hash,
    t: &Tally,
    fresh: usize,
) -> Result<(), ValidationError> {
    if tx.tx_type != TxType::Transfer {
        return Err(err(
            "AUTH004",
            format!(
                "{:?} may not issue asset {id} (outputs {}, no inputs)",
                tx.tx_type, t.output
            ),
        ));
    }
    if fresh > 1 {
        return Err(err("AUTH004", format!("second fresh asset {id} in one tx")));
    }
    let Some(first) = tx.inputs.first() else {
        return Err(err("AUTH004", format!("issuance of {id} without inputs")));
    };
    let expected = Output::compute_asset_id(&first.prev_tx_hash, first.output_index);
    if *id != expected {
        return Err(err(
            "AUTH004",
            format!("asset {id} without inputs is not anchored on input 0 (expected {expected})"),
        ));
    }
    let supply = t.supplies[0];
    if t.supplies.iter().any(|s| *s != supply) {
        return Err(err(
            "AUTH004",
            format!("asset {id} issued with non-uniform total_supply"),
        ));
    }
    if t.output > u128::from(supply) {
        return Err(err(
            "AUTH004",
            format!("asset {id} issues {} above total_supply {supply}", t.output),
        ));
    }
    Ok(())
}
