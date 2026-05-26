use std::path::Path;

use anyhow::Result;

use crate::rpc_client::coins_to_units;
use crate::wallet::Wallet;

// =============================================================================
// COVENANT CONDITION PARSER
// =============================================================================

/// Parse a human-readable condition string into a Condition AST.
///
/// Supported formats:
///   multisig(threshold, addr1, addr2, ...)
///   hashlock(hex_hash)
///   htlc(hex_hash, lock_height, expiry_height)
///   timelock(min_height)
///   timelock_expiry(max_height)
///   vesting(addr, unlock_height)
///   threshold(n, cond1, cond2, ...)
///   amount_guard(min_amount, output_index)
///   output_type_guard(type_name, output_index)
///   recipient_guard(addr, output_index)
pub(crate) fn parse_condition(s: &str) -> Result<doli_core::Condition> {
    let s = s.trim();

    // Find the function name and arguments
    let open = s
        .find('(')
        .ok_or_else(|| anyhow::anyhow!("Expected condition format: name(args...)"))?;
    let close = s
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("Missing closing parenthesis"))?;
    if close <= open {
        anyhow::bail!("Invalid condition syntax");
    }

    let name = s[..open].trim().to_lowercase();
    let args_str = &s[open + 1..close];

    // For and/or/threshold, split at top-level commas (respecting nested parens)
    match name.as_str() {
        "and" => {
            let top_args = split_top_level(args_str);
            if top_args.len() != 2 {
                anyhow::bail!("and requires exactly 2 args: and(cond1, cond2)");
            }
            let left = parse_condition(top_args[0])?;
            let right = parse_condition(top_args[1])?;
            Ok(doli_core::Condition::And(Box::new(left), Box::new(right)))
        }
        "or" => {
            let top_args = split_top_level(args_str);
            if top_args.len() != 2 {
                anyhow::bail!("or requires exactly 2 args: or(cond1, cond2)");
            }
            let left = parse_condition(top_args[0])?;
            let right = parse_condition(top_args[1])?;
            Ok(doli_core::Condition::Or(Box::new(left), Box::new(right)))
        }
        // S1 CRITICAL: threshold MUST be here (top-level match using
        // split_top_level), NOT in parse_simple_condition where flat
        // args_str.split(',') would mangle nested sub-conditions.
        "threshold" => parse_threshold(args_str),
        _ => {
            // Simple comma split for non-nested conditions
            let args: Vec<&str> = args_str.split(',').map(|a| a.trim()).collect();
            parse_simple_condition(&name, &args)
        }
    }
}

/// Parse a threshold condition: threshold(n, cond1, cond2, ...)
///
/// Uses `split_top_level` to correctly handle nested sub-conditions.
fn parse_threshold(args_str: &str) -> Result<doli_core::Condition> {
    let top_args = split_top_level(args_str);
    if top_args.len() < 3 {
        anyhow::bail!("threshold requires at least 2 conditions: threshold(n, cond1, cond2, ...)");
    }

    let n: u8 = top_args[0]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid threshold: '{}' (must be u8)", top_args[0].trim()))?;

    let sub_conditions: Vec<&str> = top_args[1..].to_vec();
    let count = sub_conditions.len();

    if count < 2 {
        anyhow::bail!("threshold requires at least 2 conditions");
    }
    if count > doli_core::MAX_THRESHOLD_CONDITIONS {
        anyhow::bail!(
            "threshold has {} conditions, exceeds MAX_THRESHOLD_CONDITIONS ({})",
            count,
            doli_core::MAX_THRESHOLD_CONDITIONS
        );
    }
    if n == 0 {
        anyhow::bail!("threshold n must be >= 1");
    }
    if (n as usize) > count {
        anyhow::bail!("threshold n ({}) exceeds condition count ({})", n, count);
    }

    let conditions: Result<Vec<doli_core::Condition>> =
        sub_conditions.iter().map(|s| parse_condition(s)).collect();

    Ok(doli_core::Condition::Threshold {
        n,
        conditions: conditions?,
    })
}

/// Split a string at top-level commas, respecting nested parentheses.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

/// Parse a simple (non-compositional) condition from name + flat args.
fn parse_simple_condition(name: &str, args: &[&str]) -> Result<doli_core::Condition> {
    match name {
        "signature" => {
            if args.len() != 1 {
                anyhow::bail!("signature requires 1 arg: addr");
            }
            let pkh = resolve_to_hash(args[0])?;
            Ok(doli_core::Condition::Signature(pkh))
        }
        "multisig" => {
            if args.len() < 3 {
                anyhow::bail!("multisig requires at least 3 args: threshold, key1, key2");
            }
            let threshold: u8 = args[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid threshold: {}", args[0]))?;
            let keys: Result<Vec<crypto::Hash>> =
                args[1..].iter().map(|a| resolve_to_hash(a)).collect();
            Ok(doli_core::Condition::multisig(threshold, keys?))
        }
        "hashlock" => {
            if args.len() != 1 {
                anyhow::bail!("hashlock requires 1 arg: hex_hash");
            }
            let hash = crypto::Hash::from_hex(args[0])
                .ok_or_else(|| anyhow::anyhow!("Invalid hex hash: {}", args[0]))?;
            Ok(doli_core::Condition::hashlock(hash))
        }
        "htlc" => {
            if args.len() != 4 {
                anyhow::bail!(
                    "htlc requires 4 args: hex_hash, lock_height, expiry_height, refund_pubkey_hash"
                );
            }
            let hash = crypto::Hash::from_hex(args[0])
                .ok_or_else(|| anyhow::anyhow!("Invalid hex hash: {}", args[0]))?;
            let lock: u64 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid lock_height: {}", args[1]))?;
            let expiry: u64 = args[2]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid expiry_height: {}", args[2]))?;
            let refund_hash = crypto::Hash::from_hex(args[3])
                .ok_or_else(|| anyhow::anyhow!("Invalid refund pubkey hash: {}", args[3]))?;
            Ok(doli_core::Condition::htlc_signed_refund(
                hash,
                lock,
                expiry,
                refund_hash,
            ))
        }
        "timelock" => {
            if args.len() != 1 {
                anyhow::bail!("timelock requires 1 arg: min_height");
            }
            let height: u64 = args[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid height: {}", args[0]))?;
            Ok(doli_core::Condition::timelock(height))
        }
        "timelock_expiry" => {
            if args.len() != 1 {
                anyhow::bail!("timelock_expiry requires 1 arg: max_height");
            }
            let height: u64 = args[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid height: {}", args[0]))?;
            Ok(doli_core::Condition::timelock_expiry(height))
        }
        "vesting" => {
            if args.len() != 2 {
                anyhow::bail!("vesting requires 2 args: addr, unlock_height");
            }
            let pkh = resolve_to_hash(args[0])?;
            let height: u64 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid unlock_height: {}", args[1]))?;
            Ok(doli_core::Condition::vesting(pkh, height))
        }
        "amount_guard" => {
            if args.len() != 2 {
                anyhow::bail!(
                    "amount_guard requires 2 args: amount_guard(min_amount, output_index)"
                );
            }
            let min_amount =
                coins_to_units(args[0]).map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;
            if min_amount == 0 {
                anyhow::bail!("min_amount must be greater than zero");
            }
            let output_index: u8 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid output_index: {}", args[1]))?;
            Ok(doli_core::Condition::amount_guard(min_amount, output_index))
        }
        "output_type_guard" => {
            if args.len() != 2 {
                anyhow::bail!(
                    "output_type_guard requires 2 args: output_type_guard(type_name, output_index)"
                );
            }
            let expected_type = parse_output_type_name(args[0])?;
            let output_index: u8 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid output_index: {}", args[1]))?;
            Ok(doli_core::Condition::output_type_guard(
                expected_type,
                output_index,
            ))
        }
        "recipient_guard" => {
            if args.len() != 2 {
                anyhow::bail!(
                    "recipient_guard requires 2 args: recipient_guard(addr, output_index)"
                );
            }
            let pkh = resolve_to_hash(args[0])?;
            let output_index: u8 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid output_index: {}", args[1]))?;
            Ok(doli_core::Condition::recipient_guard(pkh, output_index))
        }
        _ => anyhow::bail!(
            "Unknown condition: '{}'. Supported: signature, multisig, hashlock, htlc, timelock, \
             timelock_expiry, vesting, threshold, amount_guard, output_type_guard, \
             recipient_guard, and, or",
            name
        ),
    }
}

/// Parse an OutputType name (case-insensitive) into an OutputType variant.
fn parse_output_type_name(name: &str) -> Result<doli_core::OutputType> {
    match name.to_lowercase().as_str() {
        "normal" => Ok(doli_core::OutputType::Normal),
        "bond" => Ok(doli_core::OutputType::Bond),
        "multisig" => Ok(doli_core::OutputType::Multisig),
        "hashlock" => Ok(doli_core::OutputType::Hashlock),
        "htlc" => Ok(doli_core::OutputType::HTLC),
        "vesting" => Ok(doli_core::OutputType::Vesting),
        "nft" => Ok(doli_core::OutputType::NFT),
        "fungibleasset" => Ok(doli_core::OutputType::FungibleAsset),
        "bridgehtlc" => Ok(doli_core::OutputType::BridgeHTLC),
        "pool" => Ok(doli_core::OutputType::Pool),
        "lpshare" => Ok(doli_core::OutputType::LPShare),
        "zkrollup" => Ok(doli_core::OutputType::ZKRollup),
        "encryptedcontent" => Ok(doli_core::OutputType::EncryptedContent),
        _ => anyhow::bail!(
            "Unknown output type '{}'. Valid: normal, bond, multisig, hashlock, htlc, \
             vesting, nft, fungibleasset, bridgehtlc, pool, lpshare, \
             zkrollup, encryptedcontent",
            name
        ),
    }
}

/// Resolve an address string (doli1... or hex) to a pubkey_hash.
pub(crate) fn resolve_to_hash(addr: &str) -> Result<crypto::Hash> {
    let addr = addr.trim();
    // Try as hex first
    if let Some(h) = crypto::Hash::from_hex(addr) {
        return Ok(h);
    }
    // Try as bech32 address
    crypto::address::resolve(addr, None)
        .map_err(|e| anyhow::anyhow!("Invalid address '{}': {}", addr, e))
}

/// Map a Condition to the appropriate OutputType.
///
/// NOTE: Guard conditions (AmountGuard, OutputTypeGuard, RecipientGuard) map to
/// OutputType::Multisig. This is a known lossy mapping — display-level only.
/// Validation reads extra_data, not output_type. A new OutputType::Guard would
/// require a consensus change.
pub(crate) fn condition_to_output_type(cond: &doli_core::Condition) -> doli_core::OutputType {
    match cond {
        doli_core::Condition::Multisig { .. } => doli_core::OutputType::Multisig,
        doli_core::Condition::Hashlock(_) => doli_core::OutputType::Hashlock,
        doli_core::Condition::Or(_, _) => {
            // HTLC is Or(And(Hashlock, Timelock), TimelockExpiry)
            doli_core::OutputType::HTLC
        }
        doli_core::Condition::And(_, _) => {
            // Vesting is And(Signature, Timelock)
            doli_core::OutputType::Vesting
        }
        doli_core::Condition::Timelock(_) | doli_core::Condition::TimelockExpiry(_) => {
            // Standalone timelock uses Vesting type
            doli_core::OutputType::Vesting
        }
        doli_core::Condition::Signature(_) => doli_core::OutputType::Normal,
        doli_core::Condition::Threshold { .. } => doli_core::OutputType::Multisig,
        doli_core::Condition::AmountGuard { .. }
        | doli_core::Condition::OutputTypeGuard { .. }
        | doli_core::Condition::RecipientGuard { .. }
        | doli_core::Condition::MaxDeltaGuard { .. }
        | doli_core::Condition::ReserveRatioGuard { .. } => doli_core::OutputType::Multisig,
    }
}

// =============================================================================
// WITNESS PARSER
// =============================================================================

/// Parse a human-readable witness string into encoded Witness bytes.
///
/// Supported formats:
///   preimage(hex_secret)
///   sign(wallet1.json, wallet2.json, ...)
///   branch(left|right)
pub(crate) fn parse_witness(s: &str, signing_hash: &crypto::Hash) -> Result<Vec<u8>> {
    let mut witness = doli_core::Witness::default();

    // Support compound witnesses: "branch(left)+preimage(hex)" for HTLC
    let parts: Vec<&str> = s.split('+').collect();
    for part in parts {
        let part = part.trim();
        let open = part
            .find('(')
            .ok_or_else(|| anyhow::anyhow!("Expected witness format: name(args...)"))?;
        let close = part
            .rfind(')')
            .ok_or_else(|| anyhow::anyhow!("Missing closing parenthesis"))?;

        let name = part[..open].trim().to_lowercase();
        let args_str = &part[open + 1..close];
        let args: Vec<&str> = args_str.split(',').map(|a| a.trim()).collect();

        match name.as_str() {
            "preimage" => {
                if args.len() != 1 {
                    anyhow::bail!("preimage requires 1 arg: hex_secret");
                }
                let bytes = hex::decode(args[0])
                    .map_err(|_| anyhow::anyhow!("Invalid hex preimage: {}", args[0]))?;
                if bytes.len() != 32 {
                    anyhow::bail!("Preimage must be exactly 32 bytes, got {}", bytes.len());
                }
                let mut preimage = [0u8; 32];
                preimage.copy_from_slice(&bytes);
                witness.preimage = Some(preimage);
            }
            "sign" => {
                for wallet_path in &args {
                    let w = Wallet::load(Path::new(wallet_path))?;
                    let kp = w.primary_keypair()?;
                    let sig = crypto::signature::sign_hash(signing_hash, kp.private_key());
                    witness
                        .signatures
                        .push(doli_core::ConditionWitnessSignature {
                            pubkey: *kp.public_key(),
                            signature: sig,
                        });
                }
            }
            "branch" => {
                for arg in &args {
                    match arg.to_lowercase().as_str() {
                        "left" | "false" | "0" => witness.or_branches.push(false),
                        "right" | "true" | "1" => witness.or_branches.push(true),
                        _ => anyhow::bail!("Invalid branch: '{}' (use left/right)", arg),
                    }
                }
            }
            "none" | "empty" => {}
            _ => anyhow::bail!(
                "Unknown witness type: '{}'. Supported: none, preimage, sign, branch",
                name
            ),
        }
    }

    Ok(witness.encode())
}

#[cfg(test)]
#[path = "parsers_tests.rs"]
mod tests;
