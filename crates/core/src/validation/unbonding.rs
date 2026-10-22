//! INC-I-257 unbonding lock: the one spend-lock verdict and the Unbonding output rules.

use crate::transaction::{Output, OutputType, Transaction, TxType, WithdrawalRequestData};
use crate::types::BlockHeight;

use super::{bond_owner, ValidationContext, ValidationError};

impl ValidationContext {
    /// Set the INC-I-257 gate and the unbonding period from `NetworkParams`.
    #[must_use]
    pub fn with_inc_i_257_unbonding(
        mut self,
        activation_height: u64,
        unbonding_period: u64,
    ) -> Self {
        self.inc_i_257_unbonding_activation_height = activation_height;
        self.unbonding_period = unbonding_period;
        self
    }
}

fn err(code: &str, msg: String) -> ValidationError {
    ValidationError::InvalidTransaction(format!("[ERRTX-UNBOND{code}] {msg}"))
}

/// Lock verdict for spending `spent` in a `spending` tx mined at `height`.
pub fn spend_lock_verdict(
    spending: TxType,
    spent: &Output,
    height: BlockHeight,
    unbonding_activation_height: BlockHeight,
) -> Result<(), ValidationError> {
    let is_bond = spent.output_type == OutputType::Bond;
    let bypass = if height < unbonding_activation_height {
        is_bond && matches!(spending, TxType::RequestWithdrawal | TxType::Exit)
    } else if is_bond {
        if spending != TxType::RequestWithdrawal {
            return Err(err(
                "001",
                format!("a Bond input may be spent only by RequestWithdrawal, got {spending:?}"),
            ));
        }
        true
    } else {
        false
    };
    if !bypass && !spent.is_spendable_at(height) {
        return Err(ValidationError::OutputLocked {
            lock_height: spent.lock_until,
            current_height: height,
        });
    }
    Ok(())
}

/// Shape of one Unbonding output: non-zero `lock_until`, empty `extra_data`.
pub(super) fn check_output_shape(index: usize, output: &Output) -> Result<(), ValidationError> {
    if output.lock_until == 0 || !output.extra_data.is_empty() {
        return Err(err(
            "007",
            format!("unbonding output {index} needs a non-zero lock_until and empty extra_data"),
        ));
    }
    Ok(())
}

/// Where Unbonding outputs may appear, and the gated RequestWithdrawal payout.
pub(super) fn check_unbonding_outputs(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    let has_unbonding = tx
        .outputs
        .iter()
        .any(|o| o.output_type == OutputType::Unbonding);
    if ctx.current_height < ctx.inc_i_257_unbonding_activation_height {
        if has_unbonding {
            return Err(err(
                "003",
                format!(
                    "Unbonding output below activation height {}",
                    ctx.inc_i_257_unbonding_activation_height
                ),
            ));
        }
        return Ok(());
    }
    if tx.tx_type != TxType::RequestWithdrawal {
        if has_unbonding {
            return Err(err(
                "002",
                format!("{:?} may not create Unbonding outputs", tx.tx_type),
            ));
        }
        return Ok(());
    }
    check_withdrawal_payout(tx, ctx.current_height, ctx.unbonding_period)
}

fn check_withdrawal_payout(
    tx: &Transaction,
    height: BlockHeight,
    period: u64,
) -> Result<(), ValidationError> {
    let [out] = tx.outputs.as_slice() else {
        return Err(err(
            "004",
            format!(
                "RequestWithdrawal must pay exactly one Unbonding output, found {}",
                tx.outputs.len()
            ),
        ));
    };
    if out.output_type != OutputType::Unbonding {
        return Err(err(
            "004",
            format!(
                "RequestWithdrawal payout must be Unbonding, got {:?}",
                out.output_type
            ),
        ));
    }
    let data = WithdrawalRequestData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidWithdrawalRequest("invalid withdrawal request data".to_string())
    })?;
    if out.pubkey_hash != bond_owner(&data.producer_pubkey) {
        return Err(err(
            "005",
            "Unbonding payout must be owned by the producer's own address".to_string(),
        ));
    }
    let floor = height.saturating_add(period);
    if out.lock_until < floor {
        return Err(err(
            "006",
            format!("Unbonding lock_until {} below {floor}", out.lock_until),
        ));
    }
    Ok(())
}
