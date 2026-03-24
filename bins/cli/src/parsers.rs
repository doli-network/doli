use std::path::Path;

use anyhow::Result;

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

    // For and/or, split at top-level commas (respecting nested parentheses)
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
        _ => {
            // Simple comma split for non-nested conditions
            let args: Vec<&str> = args_str.split(',').map(|a| a.trim()).collect();
            parse_simple_condition(&name, &args)
        }
    }
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
        "multisig" => {
            if args.len() < 3 {
                anyhow::bail!("multisig requires at least 3 args: threshold, key1, key2");
            }
            let threshold: u8 = args[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid threshold: {}", args[0]))?;
            let keys: Result<Vec<crypto::Hash>> = args[1..]
                .iter()
                .map(|a| resolve_to_hash(a))
                .collect();
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
            if args.len() != 3 {
                anyhow::bail!("htlc requires 3 args: hex_hash, lock_height, expiry_height");
            }
            let hash = crypto::Hash::from_hex(args[0])
                .ok_or_else(|| anyhow::anyhow!("Invalid hex hash: {}", args[0]))?;
            let lock: u64 = args[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid lock_height: {}", args[1]))?;
            let expiry: u64 = args[2]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid expiry_height: {}", args[2]))?;
            Ok(doli_core::Condition::htlc(hash, lock, expiry))
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
        _ => anyhow::bail!(
            "Unknown condition: '{}'. Supported: multisig, hashlock, htlc, timelock, timelock_expiry, vesting, and, or",
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
