//! Condition → CLI string serializer.
//!
//! Converts a `Condition` AST back to the parser-compatible string format
//! accepted by `doli send --condition "..."`. This enables `--dry-run` output
//! that can be copy-pasted directly.
//!
//! Round-trip property: `parse_condition(condition_to_cli_string(&c)) == c`
//! for all well-formed conditions produced by the template functions.

use doli_core::conditions::Condition;
use doli_core::OutputType;

use crate::rpc_client::units_to_coins;

/// Convert a Condition AST to the CLI parser's string format.
///
/// The output is valid input for `parse_condition()` and can be used with
/// `doli send --condition "<output>"`.
pub(crate) fn condition_to_cli_string(cond: &Condition) -> String {
    match cond {
        Condition::Signature(hash) => {
            format!("signature({})", hash.to_hex())
        }
        Condition::Multisig { threshold, keys } => {
            let key_strs: Vec<String> = keys.iter().map(|k| k.to_hex()).collect();
            format!("multisig({}, {})", threshold, key_strs.join(", "))
        }
        Condition::Hashlock(hash) => {
            format!("hashlock({})", hash.to_hex())
        }
        Condition::Timelock(height) => {
            format!("timelock({})", height)
        }
        Condition::TimelockExpiry(height) => {
            format!("timelock_expiry({})", height)
        }
        Condition::And(a, b) => {
            format!(
                "and({}, {})",
                condition_to_cli_string(a),
                condition_to_cli_string(b)
            )
        }
        Condition::Or(a, b) => {
            format!(
                "or({}, {})",
                condition_to_cli_string(a),
                condition_to_cli_string(b)
            )
        }
        Condition::Threshold { n, conditions } => {
            let cond_strs: Vec<String> = conditions.iter().map(condition_to_cli_string).collect();
            format!("threshold({}, {})", n, cond_strs.join(", "))
        }
        Condition::AmountGuard {
            min_amount,
            output_index,
        } => {
            format!(
                "amount_guard({}, {})",
                units_to_coins(*min_amount),
                output_index
            )
        }
        Condition::OutputTypeGuard {
            expected_type,
            output_index,
        } => {
            let type_name = output_type_to_name(*expected_type);
            format!("output_type_guard({}, {})", type_name, output_index)
        }
        Condition::RecipientGuard {
            expected_pubkey_hash,
            output_index,
        } => {
            format!(
                "recipient_guard({}, {})",
                expected_pubkey_hash.to_hex(),
                output_index
            )
        }
        Condition::MaxDeltaGuard {
            max_change_bps,
            reference_amount,
            output_index,
        } => {
            format!(
                "max_delta_guard({}, {}, {})",
                max_change_bps,
                units_to_coins(*reference_amount),
                output_index
            )
        }
        Condition::ReserveRatioGuard {
            min_ratio_bps,
            reserve_output_index,
            debt_output_index,
        } => {
            format!(
                "reserve_ratio_guard({}, {}, {})",
                min_ratio_bps, reserve_output_index, debt_output_index
            )
        }
    }
}

/// Map an OutputType variant to its CLI parser name.
fn output_type_to_name(ot: OutputType) -> &'static str {
    match ot {
        OutputType::Normal => "normal",
        OutputType::Bond => "bond",
        OutputType::Multisig => "multisig",
        OutputType::Hashlock => "hashlock",
        OutputType::HTLC => "htlc",
        OutputType::Vesting => "vesting",
        OutputType::NFT => "nft",
        OutputType::FungibleAsset => "fungibleasset",
        OutputType::BridgeHTLC => "bridgehtlc",
        OutputType::Pool => "pool",
        OutputType::LPShare => "lpshare",
        OutputType::Collateral => "collateral",
        OutputType::LendingDeposit => "lendingdeposit",
        OutputType::ZKRollup => "zkrollup",
        OutputType::EncryptedContent => "encryptedcontent",
        OutputType::OraclePrice => "oracleprice",
    }
}
