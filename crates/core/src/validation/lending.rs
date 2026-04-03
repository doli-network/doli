//! Validation rules for lending transactions.
//!
//! Structural validation only -- no UTXO state needed.
//! Interest accrual, pool balance, and price oracle checks happen in apply_block.

use crate::transaction::{
    OutputType, Transaction, COLLATERAL_MAX_INTEREST_BPS, COLLATERAL_MIN_LIQUIDATION_BPS,
};
use crate::validation::error::ValidationError;

/// Validate a CreateLoan transaction structure.
///
/// Rules:
/// - At least 1 input (collateral funding)
/// - At least 2 outputs: Collateral (locked collateral) + Normal (borrowed DOLI to borrower)
/// - Output[0] must be Collateral with valid metadata
/// - Output[1] must be Normal with amount == principal from collateral metadata
/// - Interest rate and liquidation ratio within bounds
/// - Principal > 0 and collateral amount > 0
pub(crate) fn validate_create_loan(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX045] CreateLoan requires at least one input".to_string(),
        ));
    }

    if tx.outputs.len() < 2 {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX046] CreateLoan requires at least 2 outputs (Collateral + Normal), got {}",
            tx.outputs.len()
        )));
    }

    // First output must be Collateral
    if tx.outputs[0].output_type != OutputType::Collateral {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX047] CreateLoan first output must be Collateral type, got {:?}",
            tx.outputs[0].output_type
        )));
    }

    // Validate collateral metadata is decodable
    let meta = tx.outputs[0].collateral_metadata().ok_or_else(|| {
        ValidationError::InvalidTransaction(format!(
            "[ERRTX048] CreateLoan: invalid collateral metadata in output 0 ({} bytes extra_data)",
            tx.outputs[0].extra_data.len()
        ))
    })?;

    // Principal must be positive
    if meta.principal == 0 {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX049] CreateLoan: principal must be positive".to_string(),
        ));
    }

    // Collateral amount must be positive
    if tx.outputs[0].amount == 0 {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX050] CreateLoan: collateral amount must be positive".to_string(),
        ));
    }

    // Interest rate must be within bounds
    if meta.interest_rate_bps > COLLATERAL_MAX_INTEREST_BPS {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX051] CreateLoan: interest rate {} bps exceeds maximum {} bps",
            meta.interest_rate_bps, COLLATERAL_MAX_INTEREST_BPS
        )));
    }

    // Liquidation ratio must be at least minimum
    if meta.liquidation_ratio_bps < COLLATERAL_MIN_LIQUIDATION_BPS {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX052] CreateLoan: liquidation ratio {} bps below minimum {} bps",
            meta.liquidation_ratio_bps, COLLATERAL_MIN_LIQUIDATION_BPS
        )));
    }

    // Second output must be Normal (borrowed DOLI to borrower)
    if tx.outputs[1].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX053] CreateLoan second output must be Normal type (borrowed DOLI), got {:?}",
            tx.outputs[1].output_type
        )));
    }

    // Second output amount must match principal
    if tx.outputs[1].amount != meta.principal {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX054] CreateLoan: output[1] amount {} must equal principal {}",
            tx.outputs[1].amount, meta.principal
        )));
    }

    // Remaining outputs (if any) must be Normal (change)
    for (i, output) in tx.outputs.iter().enumerate().skip(2) {
        if output.output_type != OutputType::Normal
            && output.output_type != OutputType::FungibleAsset
        {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX055] CreateLoan: output {} must be Normal or FungibleAsset type (change), got {:?}",
                i, output.output_type
            )));
        }
    }

    Ok(())
}

/// Validate a RepayLoan transaction structure.
///
/// Rules:
/// - At least 2 inputs (Collateral UTXO + DOLI repayment)
/// - At least 1 output (returned collateral or change)
pub(crate) fn validate_repay_loan(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.len() < 2 {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX056] RepayLoan requires at least 2 inputs (collateral + repayment), got {}",
            tx.inputs.len()
        )));
    }

    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX057] RepayLoan requires at least 1 output".to_string(),
        ));
    }

    Ok(())
}

/// Validate a LiquidateLoan transaction structure.
///
/// Rules:
/// - At least 1 input (Collateral UTXO)
/// - At least 1 output (liquidation proceeds)
pub(crate) fn validate_liquidate_loan(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX058] LiquidateLoan requires at least 1 input".to_string(),
        ));
    }

    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX059] LiquidateLoan requires at least 1 output".to_string(),
        ));
    }

    Ok(())
}

/// Validate a LendingDeposit transaction structure.
///
/// Rules:
/// - At least 1 input (DOLI to deposit)
/// - At least 1 output (LendingDeposit receipt)
/// - First output must be LendingDeposit with valid metadata
/// - Deposit amount > 0
pub(crate) fn validate_lending_deposit(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX060] LendingDeposit requires at least one input".to_string(),
        ));
    }

    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX061] LendingDeposit requires at least one output".to_string(),
        ));
    }

    // First output must be LendingDeposit type
    if tx.outputs[0].output_type != OutputType::LendingDeposit {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX062] LendingDeposit first output must be LendingDeposit type, got {:?}",
            tx.outputs[0].output_type
        )));
    }

    // Validate metadata is decodable
    tx.outputs[0].lending_deposit_metadata().ok_or_else(|| {
        ValidationError::InvalidTransaction(format!(
            "[ERRTX063] LendingDeposit: invalid metadata in output 0 ({} bytes extra_data)",
            tx.outputs[0].extra_data.len()
        ))
    })?;

    // Deposit amount must be positive
    if tx.outputs[0].amount == 0 {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX064] LendingDeposit: deposit amount must be positive".to_string(),
        ));
    }

    // Remaining outputs (if any) must be Normal (change)
    for (i, output) in tx.outputs.iter().enumerate().skip(1) {
        if output.output_type != OutputType::Normal {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX065] LendingDeposit: output {} must be Normal type (change), got {:?}",
                i, output.output_type
            )));
        }
    }

    Ok(())
}

/// Validate a LendingWithdraw transaction structure.
///
/// Rules:
/// - At least 1 input (LendingDeposit UTXO)
/// - At least 1 output (returned DOLI + interest)
pub(crate) fn validate_lending_withdraw(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX066] LendingWithdraw requires at least 1 input (LendingDeposit UTXO)"
                .to_string(),
        ));
    }

    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX067] LendingWithdraw requires at least 1 output".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{Input, Output, Transaction, TxType};
    use crypto::Hash;

    fn dummy_create_loan_tx() -> Transaction {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        let collateral_output =
            Output::collateral(500, pool_id, borrower, 100, 500, 42, 15000, asset_id);
        let borrow_output = Output::normal(100, borrower);

        Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![collateral_output, borrow_output],
            extra_data: vec![],
        }
    }

    fn dummy_lending_deposit_tx() -> Transaction {
        let pool_id = Hash::from_bytes([0xDD; 32]);
        let depositor = Hash::from_bytes([0xEE; 32]);
        let deposit_output = Output::lending_deposit(1000, pool_id, depositor, 50);

        Transaction {
            version: 1,
            tx_type: TxType::LendingDeposit,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![deposit_output],
            extra_data: vec![],
        }
    }

    #[test]
    fn test_create_loan_valid() {
        let tx = dummy_create_loan_tx();
        assert!(validate_create_loan(&tx).is_ok());
    }

    #[test]
    fn test_create_loan_no_inputs() {
        let mut tx = dummy_create_loan_tx();
        tx.inputs.clear();
        assert!(validate_create_loan(&tx).is_err());
    }

    #[test]
    fn test_create_loan_no_outputs() {
        let mut tx = dummy_create_loan_tx();
        tx.outputs.clear();
        assert!(validate_create_loan(&tx).is_err());
    }

    #[test]
    fn test_create_loan_wrong_first_output() {
        let mut tx = dummy_create_loan_tx();
        tx.outputs[0] = Output::normal(500, Hash::from_bytes([0xBB; 32]));
        assert!(validate_create_loan(&tx).is_err());
    }

    #[test]
    fn test_create_loan_principal_mismatch() {
        let mut tx = dummy_create_loan_tx();
        // Change borrow output amount to not match principal
        tx.outputs[1] = Output::normal(999, Hash::from_bytes([0xBB; 32]));
        assert!(validate_create_loan(&tx).is_err());
    }

    #[test]
    fn test_lending_deposit_valid() {
        let tx = dummy_lending_deposit_tx();
        assert!(validate_lending_deposit(&tx).is_ok());
    }

    #[test]
    fn test_lending_deposit_no_inputs() {
        let mut tx = dummy_lending_deposit_tx();
        tx.inputs.clear();
        assert!(validate_lending_deposit(&tx).is_err());
    }

    #[test]
    fn test_lending_deposit_wrong_output_type() {
        let mut tx = dummy_lending_deposit_tx();
        tx.outputs[0] = Output::normal(1000, Hash::from_bytes([0xEE; 32]));
        assert!(validate_lending_deposit(&tx).is_err());
    }

    #[test]
    fn test_repay_loan_valid() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RepayLoan,
            inputs: vec![
                Input::new(Hash::from_bytes([0xAA; 32]), 0),
                Input::new(Hash::from_bytes([0xBB; 32]), 0),
            ],
            outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
            extra_data: vec![],
        };
        assert!(validate_repay_loan(&tx).is_ok());
    }

    #[test]
    fn test_repay_loan_insufficient_inputs() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RepayLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
            outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
            extra_data: vec![],
        };
        assert!(validate_repay_loan(&tx).is_err());
    }

    #[test]
    fn test_liquidate_loan_valid() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::LiquidateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
            outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
            extra_data: vec![],
        };
        assert!(validate_liquidate_loan(&tx).is_ok());
    }

    /// Test that CreateLoan with excessive interest rate fails validation.
    #[test]
    fn test_create_loan_excessive_interest_fails() {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        // Use interest rate above max (5001 > 5000)
        let collateral_output = Output::collateral(
            500,
            pool_id,
            borrower,
            100,
            COLLATERAL_MAX_INTEREST_BPS + 1,
            42,
            15000,
            asset_id,
        );
        let borrow_output = Output::normal(100, borrower);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![collateral_output, borrow_output],
            extra_data: vec![],
        };
        let result = validate_create_loan(&tx);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("interest rate"));
    }

    /// Test that CreateLoan with liquidation ratio below minimum fails validation.
    #[test]
    fn test_create_loan_low_liquidation_ratio_fails() {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        // Use liquidation ratio below minimum (11999 < 12000)
        let collateral_output = Output::collateral(
            500,
            pool_id,
            borrower,
            100,
            500,
            42,
            COLLATERAL_MIN_LIQUIDATION_BPS - 1,
            asset_id,
        );
        let borrow_output = Output::normal(100, borrower);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![collateral_output, borrow_output],
            extra_data: vec![],
        };
        let result = validate_create_loan(&tx);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("liquidation ratio"));
    }

    /// Test that CreateLoan with zero principal fails validation.
    #[test]
    fn test_create_loan_zero_principal_fails() {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        let collateral_output =
            Output::collateral(500, pool_id, borrower, 0, 500, 42, 15000, asset_id);
        let borrow_output = Output::normal(0, borrower);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![collateral_output, borrow_output],
            extra_data: vec![],
        };
        let result = validate_create_loan(&tx);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("principal"));
    }

    /// Test that CreateLoan with zero collateral amount fails validation.
    #[test]
    fn test_create_loan_zero_collateral_fails() {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        let collateral_output =
            Output::collateral(0, pool_id, borrower, 100, 500, 42, 15000, asset_id);
        let borrow_output = Output::normal(100, borrower);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![collateral_output, borrow_output],
            extra_data: vec![],
        };
        let result = validate_create_loan(&tx);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("collateral amount"));
    }

    /// Test that LendingDeposit with zero amount fails validation.
    #[test]
    fn test_lending_deposit_zero_amount_fails() {
        let pool_id = Hash::from_bytes([0xDD; 32]);
        let depositor = Hash::from_bytes([0xEE; 32]);
        let deposit_output = Output::lending_deposit(0, pool_id, depositor, 50);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::LendingDeposit,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![deposit_output],
            extra_data: vec![],
        };
        let result = validate_lending_deposit(&tx);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("deposit amount"));
    }

    /// Test that LiquidateLoan with no inputs fails.
    #[test]
    fn test_liquidate_loan_no_inputs_fails() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::LiquidateLoan,
            inputs: vec![],
            outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
            extra_data: vec![],
        };
        assert!(validate_liquidate_loan(&tx).is_err());
    }

    /// Test that LiquidateLoan with no outputs fails.
    #[test]
    fn test_liquidate_loan_no_outputs_fails() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::LiquidateLoan,
            inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
            outputs: vec![],
            extra_data: vec![],
        };
        assert!(validate_liquidate_loan(&tx).is_err());
    }

    /// Test that LendingWithdraw with no inputs fails.
    #[test]
    fn test_lending_withdraw_no_inputs_fails() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::LendingWithdraw,
            inputs: vec![],
            outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
            extra_data: vec![],
        };
        assert!(validate_lending_withdraw(&tx).is_err());
    }

    /// Test that LendingWithdraw with no outputs fails.
    #[test]
    fn test_lending_withdraw_no_outputs_fails() {
        let tx = Transaction {
            version: 1,
            tx_type: TxType::LendingWithdraw,
            inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
            outputs: vec![],
            extra_data: vec![],
        };
        assert!(validate_lending_withdraw(&tx).is_err());
    }

    /// Test that CreateLoan with valid change outputs passes.
    #[test]
    fn test_create_loan_with_change_outputs() {
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let borrower = Hash::from_bytes([0xBB; 32]);
        let asset_id = Hash::from_bytes([0xCC; 32]);
        let collateral_output =
            Output::collateral(500, pool_id, borrower, 100, 500, 42, 15000, asset_id);
        let borrow_output = Output::normal(100, borrower);
        let change_output = Output::normal(50, borrower);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreateLoan,
            inputs: vec![
                Input::new(Hash::from_bytes([0xFF; 32]), 0),
                Input::new(Hash::from_bytes([0xFE; 32]), 0),
            ],
            outputs: vec![collateral_output, borrow_output, change_output],
            extra_data: vec![],
        };
        assert!(validate_create_loan(&tx).is_ok());
    }
}
