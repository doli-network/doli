use std::path::Path;

use anyhow::Result;
use crypto::Hash;

use crate::commands::PoolCommands;
use crate::rpc_client::{coins_to_units, format_balance, RpcClient};
use crate::wallet::Wallet;

/// Select FungibleAsset UTXOs matching a specific asset ID.
/// Returns selected UTXOs and their total amount.
fn select_token_utxos(
    all_utxos: &[crate::rpc_client::Utxo],
    asset_id_hex: &str,
    required: u64,
) -> (Vec<crate::rpc_client::Utxo>, u64) {
    let mut selected = Vec::new();
    let mut total = 0u64;
    for utxo in all_utxos {
        if total >= required {
            break;
        }
        if utxo.output_type == "fungibleAsset" {
            if let Some(id) = utxo.asset_id() {
                if id == asset_id_hex {
                    selected.push(utxo.clone());
                    total += utxo.amount;
                }
            }
        }
    }
    (selected, total)
}

/// Create a FungibleAsset output for token change in pool operations.
/// Uses a signature condition owned by the recipient.
fn token_change_output(amount: u64, owner: Hash, asset_id: Hash) -> Result<doli_core::Output> {
    let cond = doli_core::Condition::signature(owner);
    // Use a placeholder ticker and supply=0 for change outputs.
    // The node validates asset_id consistency, not ticker/supply on transfers.
    doli_core::Output::fungible_asset(amount, owner, asset_id, 0, "XCHG", &cond)
        .map_err(|e| anyhow::anyhow!("Failed to create token change output: {:?}", e))
}

pub(crate) async fn cmd_pool(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: PoolCommands,
) -> Result<()> {
    match command {
        PoolCommands::Create {
            asset,
            doli,
            tokens,
            fee,
            yes,
        } => cmd_pool_create(wallet_path, rpc_endpoint, &asset, &doli, &tokens, fee, yes).await,
        PoolCommands::Swap {
            pool,
            amount,
            direction,
            min_out,
            yes,
        } => {
            cmd_pool_swap(
                wallet_path,
                rpc_endpoint,
                &pool,
                &amount,
                &direction,
                min_out.as_deref(),
                yes,
            )
            .await
        }
        PoolCommands::Add {
            pool,
            doli,
            tokens,
            yes,
        } => cmd_pool_add(wallet_path, rpc_endpoint, &pool, &doli, &tokens, yes).await,
        PoolCommands::Remove {
            pool,
            shares,
            min_doli,
            min_tokens,
            yes,
        } => {
            cmd_pool_remove(
                wallet_path,
                rpc_endpoint,
                &pool,
                &shares,
                min_doli.as_deref(),
                min_tokens.as_deref(),
                yes,
            )
            .await
        }
        PoolCommands::List => cmd_pool_list(rpc_endpoint).await,
        PoolCommands::Info { pool_id } => cmd_pool_info(rpc_endpoint, &pool_id).await,
    }
}

// =============================================================================
// pool list
// =============================================================================

async fn cmd_pool_list(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let pools: Vec<serde_json::Value> = rpc
        .call_raw("getPoolList", serde_json::json!({}))
        .await?
        .as_array()
        .cloned()
        .unwrap_or_default();

    if pools.is_empty() {
        println!("No pools found.");
        return Ok(());
    }

    println!(
        "{:<16}  {:>18}  {:>24}  {:>6}  {:>12}",
        "POOL ID", "RESERVE A (DOLI)", "RESERVE B (tokens)", "FEE", "PRICE"
    );
    println!("{}", "-".repeat(84));

    for pool in &pools {
        let pool_id = pool.get("poolId").and_then(|v| v.as_str()).unwrap_or("?");
        let reserve_a = pool.get("reserveA").and_then(|v| v.as_u64()).unwrap_or(0);
        let reserve_b = pool.get("reserveB").and_then(|v| v.as_u64()).unwrap_or(0);
        let fee_bps = pool.get("feeBps").and_then(|v| v.as_u64()).unwrap_or(0);
        let price = pool.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let fee_pct = format!("{:.2}%", fee_bps as f64 / 100.0);
        let pool_id_short = if pool_id.len() > 16 {
            &pool_id[..16]
        } else {
            pool_id
        };

        println!(
            "{:<16}  {:>18}  {:>24}  {:>6}  {:>12.8}",
            pool_id_short,
            format_balance(reserve_a),
            reserve_b,
            fee_pct,
            price
        );
    }

    println!("\n{} pool(s) found.", pools.len());
    Ok(())
}

// =============================================================================
// pool info
// =============================================================================

async fn cmd_pool_info(rpc_endpoint: &str, pool_id: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let info: serde_json::Value = rpc
        .call_raw("getPoolInfo", serde_json::json!({ "poolId": pool_id }))
        .await?;

    let pid = info.get("poolId").and_then(|v| v.as_str()).unwrap_or("?");
    let asset_a = info.get("assetA").and_then(|v| v.as_str()).unwrap_or("?");
    let asset_b = info.get("assetB").and_then(|v| v.as_str()).unwrap_or("?");
    let reserve_a = info.get("reserveA").and_then(|v| v.as_u64()).unwrap_or(0);
    let reserve_b = info.get("reserveB").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_shares = info
        .get("totalShares")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fee_bps = info.get("feeBps").and_then(|v| v.as_u64()).unwrap_or(0);
    let price = info.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let last_slot = info
        .get("lastUpdateSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let creation_slot = info
        .get("creationSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!("Pool Info:");
    println!("  Pool ID:        {}", pid);
    println!("  Asset A (DOLI): {}", asset_a);
    println!("  Asset B:        {}", asset_b);
    println!("  Reserve A:      {}", format_balance(reserve_a));
    println!("  Reserve B:      {} (raw token units)", reserve_b);
    println!("  Total LP Shares:{}", total_shares);
    println!(
        "  Fee:            {:.2}% ({} bps)",
        fee_bps as f64 / 100.0,
        fee_bps
    );
    println!("  Price (B/A):    {:.8}", price);
    println!("  Last Update:    slot {}", last_slot);
    println!("  Created:        slot {}", creation_slot);

    Ok(())
}

// =============================================================================
// pool create
// =============================================================================

#[allow(clippy::too_many_arguments)]
async fn cmd_pool_create(
    wallet_path: &Path,
    rpc_endpoint: &str,
    asset_hex: &str,
    doli_amount: &str,
    token_amount: &str,
    fee_bps: u16,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let asset_b_id = Hash::from_hex(asset_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid asset ID hex: {}", asset_hex))?;

    let doli_units =
        coins_to_units(doli_amount).map_err(|e| anyhow::anyhow!("Invalid DOLI amount: {}", e))?;
    let token_units: u64 = token_amount.parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid token amount: {} (use raw token units)",
            token_amount
        )
    })?;

    if doli_units == 0 || token_units == 0 {
        anyhow::bail!("Both DOLI and token amounts must be greater than zero");
    }

    if fee_bps > doli_core::transaction::POOL_MAX_FEE_BPS {
        anyhow::bail!(
            "Fee too high: {} bps (max {})",
            fee_bps,
            doli_core::transaction::POOL_MAX_FEE_BPS
        );
    }

    // Compute deterministic pool ID
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b_id);

    // Compute initial LP shares
    let lp_shares = doli_core::compute_initial_lp_shares(doli_units, token_units);
    if lp_shares == 0 {
        anyhow::bail!("Initial liquidity too small to create LP shares");
    }

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Calculate fee: base + per-byte for pool + LP share outputs extra_data
    // Pool output: POOL_METADATA_SIZE (116) bytes, LP share: 33 bytes
    let fee_units = {
        let extra_bytes: u64 = (doli_core::transaction::POOL_METADATA_SIZE
            + doli_core::transaction::LP_SHARE_METADATA_SIZE) as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    let required = doli_units + fee_units;
    let utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut selected = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= required {
            break;
        }
        selected.push(utxo.clone());
        total_input += utxo.amount;
    }
    if total_input < required {
        anyhow::bail!(
            "Insufficient DOLI balance. Available: {}, Required: {}",
            format_balance(total_input),
            format_balance(required)
        );
    }

    // Select FungibleAsset UTXOs matching the specific asset_b_id
    let all_utxos = rpc.get_utxos(&from_pubkey_hash, false).await?;
    let (token_selected, token_total) = select_token_utxos(&all_utxos, asset_hex, token_units);
    if token_total < token_units {
        anyhow::bail!(
            "Insufficient token balance for asset {}. Available: {}, Required: {}",
            &asset_hex[..16.min(asset_hex.len())],
            token_total,
            token_units
        );
    }

    // Build inputs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }
    for utxo in &token_selected {
        let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid token UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build outputs:
    // 1. Pool UTXO
    // 2. LP share for creator
    // 3. DOLI change (if any)
    // 4. Token change (if any)
    let pool_output = Output::pool(
        pool_id,
        asset_b_id,
        doli_units,
        token_units,
        lp_shares,
        0, // cumulative_price starts at 0
        0, // last_update_slot will be stamped by node
        fee_bps,
        0, // creation_slot will be stamped by node
    );

    let lp_output = Output::lp_share(lp_shares, pool_id, from_hash);

    let mut outputs = vec![pool_output, lp_output];

    let doli_change = total_input - required;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let token_change = token_total - token_units;
    if token_change > 0 {
        // Token change output: same asset type
        outputs.push(token_change_output(token_change, from_hash, asset_b_id)?);
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::CreatePool,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    let keypair = wallet.primary_keypair()?;

    // Sign all inputs + set covenant witnesses for conditioned inputs (FungibleAsset)
    let num_doli_inputs = selected.len();
    let mut witnesses: Vec<Vec<u8>> = Vec::new();
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        // FungibleAsset inputs (after DOLI inputs) need covenant witnesses
        if i >= num_doli_inputs {
            let witness_str = format!("sign({})", wallet_path.display());
            let witness_bytes = crate::parsers::parse_witness(&witness_str, &signing_hash)?;
            witnesses.push(witness_bytes);
        } else {
            witnesses.push(Vec::new()); // empty witness for Normal DOLI inputs
        }
    }
    tx.set_covenant_witnesses(&witnesses);

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Create Pool:");
    println!("  Pool ID:     {}", pool_id.to_hex());
    println!("  Asset B:     {}", asset_hex);
    println!("  DOLI:        {}", format_balance(doli_units));
    println!("  Tokens:      {}", token_units);
    println!("  LP Shares:   {}", lp_shares);
    println!(
        "  Fee:         {:.2}% ({} bps)",
        fee_bps as f64 / 100.0,
        fee_bps
    );
    println!("  TX Hash:     {}", tx_hash.to_hex());
    println!("  Size:        {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Pool created successfully!");
            println!("TX Hash: {}", result_hash);
            println!("Pool ID: {}", pool_id.to_hex());
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Pool creation failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// pool swap
// =============================================================================

#[allow(clippy::too_many_arguments)]
async fn cmd_pool_swap(
    wallet_path: &Path,
    rpc_endpoint: &str,
    pool_id_hex: &str,
    amount: &str,
    direction: &str,
    min_out: Option<&str>,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    if direction != "a2b" && direction != "b2a" {
        anyhow::bail!("Direction must be 'a2b' (DOLI->token) or 'b2a' (token->DOLI)");
    }

    let pool_id = Hash::from_hex(pool_id_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid pool ID hex: {}", pool_id_hex))?;

    let amount_in = if direction == "a2b" {
        // Swapping DOLI in — DOLI notation
        coins_to_units(amount).map_err(|e| anyhow::anyhow!("Invalid DOLI amount: {}", e))?
    } else {
        // Swapping tokens in — raw token units
        amount
            .parse::<u64>()
            .map_err(|_| anyhow::anyhow!("Invalid token amount: {} (use raw units)", amount))?
    };
    if amount_in == 0 {
        anyhow::bail!("Amount must be greater than zero");
    }

    let min_output = if let Some(m) = min_out {
        if direction == "a2b" {
            // Output is tokens — raw units
            m.parse::<u64>()
                .map_err(|_| anyhow::anyhow!("Invalid min_out: {} (raw token units)", m))?
        } else {
            // Output is DOLI
            coins_to_units(m).map_err(|e| anyhow::anyhow!("Invalid min_out: {}", e))?
        }
    } else {
        0
    };

    // Get pool info via RPC
    let pool_info: serde_json::Value = rpc
        .call_raw("getPoolInfo", serde_json::json!({ "poolId": pool_id_hex }))
        .await?;

    let reserve_a = pool_info
        .get("reserveA")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read pool reserveA"))?;
    let reserve_b = pool_info
        .get("reserveB")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read pool reserveB"))?;
    let fee_bps = pool_info
        .get("feeBps")
        .and_then(|v| v.as_u64())
        .unwrap_or(30) as u16;
    let asset_b_hex = pool_info
        .get("assetB")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot read pool assetB"))?;
    let asset_b_id = Hash::from_hex(asset_b_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid asset B ID in pool info"))?;
    let total_shares = pool_info
        .get("totalShares")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cumulative_str = pool_info
        .get("twapCumulativePrice")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let cumulative_price: u128 = cumulative_str.parse().unwrap_or(0);
    let last_update_slot = pool_info
        .get("lastUpdateSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let creation_slot = pool_info
        .get("creationSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // Compute swap
    let (reserve_in, reserve_out) = if direction == "a2b" {
        (reserve_a, reserve_b)
    } else {
        (reserve_b, reserve_a)
    };

    let (amount_out, new_reserve_in, new_reserve_out) =
        doli_core::compute_swap(reserve_in, reserve_out, amount_in, fee_bps)
            .ok_or_else(|| anyhow::anyhow!("Swap computation failed"))?;

    if amount_out < min_output {
        anyhow::bail!(
            "Slippage exceeded: output {} < minimum {}",
            amount_out,
            min_output
        );
    }

    let (new_reserve_a, new_reserve_b) = if direction == "a2b" {
        (new_reserve_in, new_reserve_out)
    } else {
        (new_reserve_out, new_reserve_in)
    };

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Get pool UTXO outpoint from getPoolInfo response
    let pool_tx_hash_str = pool_info
        .get("txHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Pool info missing txHash"))?;
    let pool_tx_hash =
        Hash::from_hex(pool_tx_hash_str).ok_or_else(|| anyhow::anyhow!("Invalid pool txHash"))?;
    let pool_output_index = pool_info
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut inputs = vec![Input::new(pool_tx_hash, pool_output_index)];

    // Calculate fee: base + per-byte for pool output extra_data (POOL_METADATA_SIZE)
    let fee_units = {
        let extra_bytes: u64 = doli_core::transaction::POOL_METADATA_SIZE as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    if direction == "a2b" {
        // Swapping DOLI in: need DOLI UTXOs
        let required = amount_in + fee_units;
        let utxos: Vec<_> = rpc
            .get_utxos(&from_pubkey_hash, true)
            .await?
            .into_iter()
            .filter(|u| u.output_type == "normal" && u.spendable)
            .collect();

        let mut total_doli = 0u64;
        for utxo in &utxos {
            if total_doli >= required {
                break;
            }
            let prev_hash = Hash::from_hex(&utxo.tx_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
            inputs.push(Input::new(prev_hash, utxo.output_index));
            total_doli += utxo.amount;
        }
        if total_doli < required {
            anyhow::bail!(
                "Insufficient DOLI. Available: {}, Required: {}",
                format_balance(total_doli),
                format_balance(required)
            );
        }

        // Outputs: updated pool, token output to user, DOLI change
        let pool_output = Output::pool(
            pool_id,
            asset_b_id,
            new_reserve_a,
            new_reserve_b,
            total_shares,
            cumulative_price,
            last_update_slot,
            fee_bps,
            creation_slot,
        );
        let token_output = token_change_output(amount_out, from_hash, asset_b_id)?;

        let mut outputs = vec![pool_output, token_output];
        let doli_change = total_doli - required;
        if doli_change > 0 {
            outputs.push(Output::normal(doli_change, from_hash));
        }

        let mut tx = Transaction {
            version: 1,
            tx_type: doli_core::transaction::TxType::Swap,
            inputs,
            outputs,
            extra_data: Vec::new(),
        };

        let keypair = wallet.primary_keypair()?;
        for i in 0..tx.inputs.len() {
            let signing_hash = tx.signing_message_for_input(i);
            tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        }

        broadcast_swap_tx(&rpc, &tx, amount_in, amount_out, direction, fee_bps, yes).await?;
    } else {
        // Swapping tokens in: need FungibleAsset UTXOs matching asset_b_id
        let all_utxos = rpc.get_utxos(&from_pubkey_hash, false).await?;
        let (token_utxos, token_total) = select_token_utxos(&all_utxos, asset_b_hex, amount_in);
        let num_token_inputs = token_utxos.len();
        for utxo in &token_utxos {
            let prev_hash = Hash::from_hex(&utxo.tx_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid token UTXO tx_hash"))?;
            inputs.push(Input::new(prev_hash, utxo.output_index));
        }
        if token_total < amount_in {
            anyhow::bail!(
                "Insufficient tokens for asset {}. Available: {}, Required: {}",
                &asset_b_hex[..16.min(asset_b_hex.len())],
                token_total,
                amount_in
            );
        }

        // Also need a DOLI UTXO for the fee
        let doli_utxos: Vec<_> = rpc
            .get_utxos(&from_pubkey_hash, true)
            .await?
            .into_iter()
            .filter(|u| u.output_type == "normal" && u.spendable)
            .collect();

        let mut fee_total = 0u64;
        for utxo in &doli_utxos {
            if fee_total >= fee_units {
                break;
            }
            let prev_hash = Hash::from_hex(&utxo.tx_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
            inputs.push(Input::new(prev_hash, utxo.output_index));
            fee_total += utxo.amount;
        }
        if fee_total < fee_units {
            anyhow::bail!("Insufficient DOLI for fee");
        }

        let pool_output = Output::pool(
            pool_id,
            asset_b_id,
            new_reserve_a,
            new_reserve_b,
            total_shares,
            cumulative_price,
            last_update_slot,
            fee_bps,
            creation_slot,
        );
        let doli_output = Output::normal(amount_out, from_hash);

        let mut outputs = vec![pool_output, doli_output];
        let token_change = token_total - amount_in;
        if token_change > 0 {
            outputs.push(token_change_output(token_change, from_hash, asset_b_id)?);
        }
        let doli_change = fee_total - fee_units;
        if doli_change > 0 {
            outputs.push(Output::normal(doli_change, from_hash));
        }

        let mut tx = Transaction {
            version: 1,
            tx_type: doli_core::transaction::TxType::Swap,
            inputs,
            outputs,
            extra_data: Vec::new(),
        };

        let keypair = wallet.primary_keypair()?;
        // inputs layout: [pool, token_0..token_N, doli_fee_0..doli_fee_M]
        // Token (FungibleAsset) inputs need covenant witnesses; pool and DOLI inputs don't.
        let token_start = 1usize; // pool is index 0
        let token_end = token_start + num_token_inputs;
        let mut witnesses: Vec<Vec<u8>> = Vec::new();
        for i in 0..tx.inputs.len() {
            let signing_hash = tx.signing_message_for_input(i);
            tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
            if i >= token_start && i < token_end {
                let witness_str = format!("sign({})", wallet_path.display());
                let witness_bytes = crate::parsers::parse_witness(&witness_str, &signing_hash)?;
                witnesses.push(witness_bytes);
            } else {
                witnesses.push(Vec::new());
            }
        }
        tx.set_covenant_witnesses(&witnesses);

        broadcast_swap_tx(&rpc, &tx, amount_in, amount_out, direction, fee_bps, yes).await?;
    }

    Ok(())
}

async fn broadcast_swap_tx(
    rpc: &RpcClient,
    tx: &doli_core::Transaction,
    amount_in: u64,
    amount_out: u64,
    direction: &str,
    fee_bps: u16,
    yes: bool,
) -> Result<()> {
    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    let (label_in, label_out) = if direction == "a2b" {
        ("DOLI", "tokens")
    } else {
        ("tokens", "DOLI")
    };

    println!("Swap:");
    println!("  Direction:  {} -> {}", label_in, label_out);
    println!("  Amount In:  {}", amount_in);
    println!("  Amount Out: {}", amount_out);
    println!(
        "  Fee:        {:.2}% ({} bps)",
        fee_bps as f64 / 100.0,
        fee_bps
    );
    println!("  TX Hash:    {}", tx_hash.to_hex());
    println!("  Size:       {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Swap executed successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Swap failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// pool add (add liquidity)
// =============================================================================

async fn cmd_pool_add(
    wallet_path: &Path,
    rpc_endpoint: &str,
    pool_id_hex: &str,
    doli_amount: &str,
    token_amount: &str,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let pool_id = Hash::from_hex(pool_id_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid pool ID hex: {}", pool_id_hex))?;

    let doli_units =
        coins_to_units(doli_amount).map_err(|e| anyhow::anyhow!("Invalid DOLI amount: {}", e))?;
    let token_units: u64 = token_amount.parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid token amount: {} (use raw token units)",
            token_amount
        )
    })?;

    if doli_units == 0 || token_units == 0 {
        anyhow::bail!("Both amounts must be greater than zero");
    }

    // Get pool info
    let pool_info: serde_json::Value = rpc
        .call_raw("getPoolInfo", serde_json::json!({ "poolId": pool_id_hex }))
        .await?;

    let reserve_a = pool_info
        .get("reserveA")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reserve_b = pool_info
        .get("reserveB")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_shares = pool_info
        .get("totalShares")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fee_bps = pool_info
        .get("feeBps")
        .and_then(|v| v.as_u64())
        .unwrap_or(30) as u16;
    let asset_b_hex = pool_info
        .get("assetB")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let asset_b_id =
        Hash::from_hex(asset_b_hex).ok_or_else(|| anyhow::anyhow!("Invalid asset B ID"))?;
    let cumulative_str = pool_info
        .get("twapCumulativePrice")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let cumulative_price: u128 = cumulative_str.parse().unwrap_or(0);
    let last_update_slot = pool_info
        .get("lastUpdateSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let creation_slot = pool_info
        .get("creationSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // Compute LP shares to mint
    let new_shares = doli_core::compute_lp_shares(
        doli_units,
        token_units,
        reserve_a,
        reserve_b,
        total_shares,
    )
    .ok_or_else(|| anyhow::anyhow!("Failed to compute LP shares (disproportional deposit?)"))?;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Get pool UTXO outpoint from getPoolInfo response
    let pool_tx_hash_str = pool_info
        .get("txHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Pool info missing txHash"))?;
    let pool_tx_hash =
        Hash::from_hex(pool_tx_hash_str).ok_or_else(|| anyhow::anyhow!("Invalid pool txHash"))?;
    let pool_output_index = pool_info
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut inputs = vec![Input::new(pool_tx_hash, pool_output_index)];

    // Calculate fee: base + per-byte for pool + LP share outputs extra_data
    let fee_units = {
        let extra_bytes: u64 = (doli_core::transaction::POOL_METADATA_SIZE
            + doli_core::transaction::LP_SHARE_METADATA_SIZE) as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    let required = doli_units + fee_units;
    let utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut total_doli = 0u64;
    for utxo in &utxos {
        if total_doli >= required {
            break;
        }
        let prev_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_hash, utxo.output_index));
        total_doli += utxo.amount;
    }
    if total_doli < required {
        anyhow::bail!(
            "Insufficient DOLI. Available: {}, Required: {}",
            format_balance(total_doli),
            format_balance(required)
        );
    }

    // Track where non-conditioned inputs end (pool + DOLI inputs)
    let num_non_token_inputs = inputs.len();

    // Fund tokens — filter by asset_id
    let all_utxos = rpc.get_utxos(&from_pubkey_hash, false).await?;
    let (token_utxos, token_total) = select_token_utxos(&all_utxos, asset_b_hex, token_units);
    for utxo in &token_utxos {
        let prev_hash = Hash::from_hex(&utxo.tx_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid token UTXO tx_hash"))?;
        inputs.push(Input::new(prev_hash, utxo.output_index));
    }
    if token_total < token_units {
        anyhow::bail!(
            "Insufficient tokens for asset {}. Available: {}, Required: {}",
            &asset_b_hex[..16.min(asset_b_hex.len())],
            token_total,
            token_units
        );
    }

    // Outputs: updated pool, new LP shares, change
    let new_reserve_a = reserve_a + doli_units;
    let new_reserve_b = reserve_b + token_units;
    let new_total_shares = total_shares + new_shares;

    let pool_output = Output::pool(
        pool_id,
        asset_b_id,
        new_reserve_a,
        new_reserve_b,
        new_total_shares,
        cumulative_price,
        last_update_slot,
        fee_bps,
        creation_slot,
    );
    let lp_output = Output::lp_share(new_shares, pool_id, from_hash);

    let mut outputs = vec![pool_output, lp_output];
    let doli_change = total_doli - required;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }
    let token_change = token_total - token_units;
    if token_change > 0 {
        outputs.push(token_change_output(token_change, from_hash, asset_b_id)?);
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::AddLiquidity,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    let keypair = wallet.primary_keypair()?;
    // inputs layout: [pool, doli_0..doli_N, token_0..token_M]
    // Token (FungibleAsset) inputs need covenant witnesses; pool and DOLI inputs don't.
    let mut witnesses: Vec<Vec<u8>> = Vec::new();
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        if i >= num_non_token_inputs {
            let witness_str = format!("sign({})", wallet_path.display());
            let witness_bytes = crate::parsers::parse_witness(&witness_str, &signing_hash)?;
            witnesses.push(witness_bytes);
        } else {
            witnesses.push(Vec::new());
        }
    }
    tx.set_covenant_witnesses(&witnesses);

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Add Liquidity:");
    println!("  Pool:        {}", pool_id_hex);
    println!("  DOLI:        {}", format_balance(doli_units));
    println!("  Tokens:      {}", token_units);
    println!("  LP Shares:   {}", new_shares);
    println!("  TX Hash:     {}", tx_hash.to_hex());
    println!("  Size:        {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Liquidity added successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Add liquidity failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// pool remove (remove liquidity)
// =============================================================================

#[allow(clippy::too_many_arguments)]
async fn cmd_pool_remove(
    wallet_path: &Path,
    rpc_endpoint: &str,
    pool_id_hex: &str,
    shares_str: &str,
    min_doli: Option<&str>,
    min_tokens: Option<&str>,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let pool_id = Hash::from_hex(pool_id_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid pool ID hex: {}", pool_id_hex))?;

    let shares_to_burn: u64 = shares_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid shares amount: {}", shares_str))?;
    if shares_to_burn == 0 {
        anyhow::bail!("Shares must be greater than zero");
    }

    let min_doli_units = if let Some(m) = min_doli {
        coins_to_units(m).map_err(|e| anyhow::anyhow!("Invalid min_doli: {}", e))?
    } else {
        0
    };
    let min_token_units: u64 = if let Some(m) = min_tokens {
        m.parse()
            .map_err(|_| anyhow::anyhow!("Invalid min_tokens: {} (raw token units)", m))?
    } else {
        0
    };

    // Get pool info
    let pool_info: serde_json::Value = rpc
        .call_raw("getPoolInfo", serde_json::json!({ "poolId": pool_id_hex }))
        .await?;

    let reserve_a = pool_info
        .get("reserveA")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reserve_b = pool_info
        .get("reserveB")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_shares = pool_info
        .get("totalShares")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fee_bps = pool_info
        .get("feeBps")
        .and_then(|v| v.as_u64())
        .unwrap_or(30) as u16;
    let asset_b_hex = pool_info
        .get("assetB")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let asset_b_id =
        Hash::from_hex(asset_b_hex).ok_or_else(|| anyhow::anyhow!("Invalid asset B ID"))?;
    let cumulative_str = pool_info
        .get("twapCumulativePrice")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let cumulative_price: u128 = cumulative_str.parse().unwrap_or(0);
    let last_update_slot = pool_info
        .get("lastUpdateSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let creation_slot = pool_info
        .get("creationSlot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    if shares_to_burn > total_shares {
        anyhow::bail!(
            "Cannot burn {} shares (pool only has {})",
            shares_to_burn,
            total_shares
        );
    }

    // Compute assets returned
    let (doli_out, token_out) =
        doli_core::compute_remove_liquidity(shares_to_burn, reserve_a, reserve_b, total_shares)
            .ok_or_else(|| anyhow::anyhow!("Failed to compute removal amounts"))?;

    if doli_out < min_doli_units {
        anyhow::bail!(
            "Slippage: DOLI output {} < minimum {}",
            format_balance(doli_out),
            format_balance(min_doli_units)
        );
    }
    if token_out < min_token_units {
        anyhow::bail!(
            "Slippage: token output {} < minimum {}",
            token_out,
            min_token_units
        );
    }

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Get pool UTXO outpoint from getPoolInfo response
    let pool_tx_hash_str = pool_info
        .get("txHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Pool info missing txHash"))?;
    let pool_tx_hash =
        Hash::from_hex(pool_tx_hash_str).ok_or_else(|| anyhow::anyhow!("Invalid pool txHash"))?;
    let pool_output_index = pool_info
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut inputs = vec![Input::new(pool_tx_hash, pool_output_index)];

    // Find LP share UTXOs to burn
    let all_utxos = rpc.get_utxos(&from_pubkey_hash, false).await?;
    let mut lp_total = 0u64;
    for utxo in &all_utxos {
        if utxo.output_type == "lpShare" && lp_total < shares_to_burn {
            let prev_hash = Hash::from_hex(&utxo.tx_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid LP UTXO tx_hash"))?;
            inputs.push(Input::new(prev_hash, utxo.output_index));
            lp_total += utxo.amount;
        }
    }
    if lp_total < shares_to_burn {
        anyhow::bail!(
            "Insufficient LP shares. Available: {}, Required: {}",
            lp_total,
            shares_to_burn
        );
    }

    // Calculate fee: base + per-byte for pool output extra_data
    let fee_units = {
        let extra_bytes: u64 = doli_core::transaction::POOL_METADATA_SIZE as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    let doli_utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut fee_total = 0u64;
    for utxo in &doli_utxos {
        if fee_total >= fee_units {
            break;
        }
        let prev_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_hash, utxo.output_index));
        fee_total += utxo.amount;
    }
    if fee_total < fee_units {
        anyhow::bail!("Insufficient DOLI for fee");
    }

    // Outputs: updated pool, DOLI to user, tokens to user, LP change, fee change
    let new_reserve_a = reserve_a - doli_out;
    let new_reserve_b = reserve_b - token_out;
    let new_total_shares = total_shares - shares_to_burn;

    let pool_output = Output::pool(
        pool_id,
        asset_b_id,
        new_reserve_a,
        new_reserve_b,
        new_total_shares,
        cumulative_price,
        last_update_slot,
        fee_bps,
        creation_slot,
    );

    let mut outputs = vec![pool_output];

    if doli_out > 0 {
        outputs.push(Output::normal(doli_out, from_hash));
    }
    if token_out > 0 {
        outputs.push(token_change_output(token_out, from_hash, asset_b_id)?);
    }

    let lp_change = lp_total - shares_to_burn;
    if lp_change > 0 {
        outputs.push(Output::lp_share(lp_change, pool_id, from_hash));
    }

    let fee_change = fee_total - fee_units;
    if fee_change > 0 {
        outputs.push(Output::normal(fee_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::RemoveLiquidity,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Remove Liquidity:");
    println!("  Pool:        {}", pool_id_hex);
    println!("  LP Shares:   {} (burning)", shares_to_burn);
    println!("  DOLI Out:    {}", format_balance(doli_out));
    println!("  Tokens Out:  {}", token_out);
    println!("  TX Hash:     {}", tx_hash.to_hex());
    println!("  Size:        {} bytes", tx_bytes.len());

    if !yes {
        println!();
        println!("Proceed? [y/N]");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Liquidity removed successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Remove liquidity failed: {}", e));
        }
    }

    Ok(())
}
