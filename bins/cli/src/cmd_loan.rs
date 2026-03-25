use std::path::Path;

use anyhow::Result;
use crypto::Hash;

use crate::commands::LoanCommands;
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

/// Create a FungibleAsset output for returning collateral tokens.
/// Uses a signature condition owned by the recipient.
fn token_output(amount: u64, owner: Hash, asset_id: Hash) -> Result<doli_core::Output> {
    let cond = doli_core::Condition::signature(owner);
    doli_core::Output::fungible_asset(amount, owner, asset_id, 0, "XCHG", &cond)
        .map_err(|e| anyhow::anyhow!("Failed to create token output: {:?}", e))
}

pub(crate) async fn cmd_loan(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: LoanCommands,
) -> Result<()> {
    match command {
        LoanCommands::Deposit { pool, amount, yes } => {
            cmd_loan_deposit(wallet_path, rpc_endpoint, &pool, &amount, yes).await
        }
        LoanCommands::Withdraw { deposit_utxo, yes } => {
            cmd_loan_withdraw(wallet_path, rpc_endpoint, &deposit_utxo, yes).await
        }
        LoanCommands::Create {
            pool,
            collateral,
            borrow,
            interest_rate,
            yes,
        } => {
            cmd_loan_create(
                wallet_path,
                rpc_endpoint,
                &pool,
                &collateral,
                &borrow,
                interest_rate,
                yes,
            )
            .await
        }
        LoanCommands::Repay { loan_utxo, yes } => {
            cmd_loan_repay(wallet_path, rpc_endpoint, &loan_utxo, yes).await
        }
        LoanCommands::Liquidate { loan_utxo, yes } => {
            cmd_loan_liquidate(wallet_path, rpc_endpoint, &loan_utxo, yes).await
        }
        LoanCommands::List { borrower } => cmd_loan_list(rpc_endpoint, borrower.as_deref()).await,
        LoanCommands::Info { loan_utxo } => cmd_loan_info(rpc_endpoint, &loan_utxo).await,
    }
}

// =============================================================================
// loan deposit
// =============================================================================

async fn cmd_loan_deposit(
    wallet_path: &Path,
    rpc_endpoint: &str,
    pool_id_hex: &str,
    amount_str: &str,
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

    let deposit_amount =
        coins_to_units(amount_str).map_err(|e| anyhow::anyhow!("Invalid DOLI amount: {}", e))?;
    if deposit_amount == 0 {
        anyhow::bail!("Deposit amount must be greater than zero");
    }

    // Compute lending pool ID from AMM pool ID
    let lending_pool_id = Output::compute_lending_pool_id(&pool_id);

    // Get current slot from chain info
    let chain_info = rpc.get_chain_info().await?;
    let current_slot = chain_info.best_slot as u32;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build LendingDeposit output
    let deposit_output =
        Output::lending_deposit(deposit_amount, lending_pool_id, from_hash, current_slot);

    // Calculate fee: base + per-byte for deposit output extra_data
    let fee_units = {
        let extra_bytes: u64 = deposit_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    let required = deposit_amount + fee_units;
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

    // Build inputs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build outputs
    let mut outputs = vec![deposit_output];
    let doli_change = total_input - required;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::LendingDeposit,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign all inputs
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Lending Deposit:");
    println!("  Pool ID:        {}", pool_id_hex);
    println!("  Lending Pool:   {}", lending_pool_id.to_hex());
    println!("  Amount:         {}", format_balance(deposit_amount));
    println!("  TX Hash:        {}", tx_hash.to_hex());
    println!("  Size:           {} bytes", tx_bytes.len());

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
            println!("Lending deposit successful!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Lending deposit failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// loan withdraw
// =============================================================================

async fn cmd_loan_withdraw(
    wallet_path: &Path,
    rpc_endpoint: &str,
    deposit_utxo_str: &str,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let parts: Vec<&str> = deposit_utxo_str.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid UTXO format. Expected txhash:output_index");
    }
    let utxo_tx_hash_hex = parts[0];
    let utxo_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let utxo_tx_hash =
        Hash::from_hex(utxo_tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash hex"))?;

    // Get the LendingDeposit UTXO details via getTransaction
    let tx_json = rpc.get_transaction_json(utxo_tx_hash_hex).await?;
    let outputs_arr = tx_json
        .get("outputs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Cannot read transaction outputs"))?;

    let deposit_output = outputs_arr.get(utxo_output_index as usize).ok_or_else(|| {
        anyhow::anyhow!(
            "Output index {} not found in transaction",
            utxo_output_index
        )
    })?;

    let output_type = deposit_output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "lendingDeposit" {
        anyhow::bail!(
            "UTXO is not a LendingDeposit output (got '{}')",
            output_type
        );
    }

    let deposit_amount = deposit_output
        .get("amount")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read deposit amount"))?;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build inputs: LendingDeposit UTXO + DOLI for fee
    let mut inputs = vec![Input::new(utxo_tx_hash, utxo_output_index)];

    let fee_units = 1u64;
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

    // Build outputs: returned DOLI (deposit + any accrued interest handled by node)
    let mut outputs = vec![Output::normal(deposit_amount, from_hash)];
    let doli_change = fee_total - fee_units;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::LendingWithdraw,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign all inputs
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Lending Withdraw:");
    println!(
        "  Deposit UTXO:   {}:{}",
        utxo_tx_hash_hex, utxo_output_index
    );
    println!("  Amount:         {}", format_balance(deposit_amount));
    println!("  TX Hash:        {}", tx_hash.to_hex());
    println!("  Size:           {} bytes", tx_bytes.len());

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
            println!("Lending withdrawal successful!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Lending withdrawal failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// loan create
// =============================================================================

#[allow(clippy::too_many_arguments)]
async fn cmd_loan_create(
    wallet_path: &Path,
    rpc_endpoint: &str,
    pool_id_hex: &str,
    collateral_str: &str,
    borrow_str: &str,
    interest_rate: u16,
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

    let collateral_amount: u64 = collateral_str.parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid collateral amount: {} (use raw token units)",
            collateral_str
        )
    })?;
    let borrow_amount = coins_to_units(borrow_str)
        .map_err(|e| anyhow::anyhow!("Invalid DOLI borrow amount: {}", e))?;

    if collateral_amount == 0 {
        anyhow::bail!("Collateral amount must be greater than zero");
    }
    if borrow_amount == 0 {
        anyhow::bail!("Borrow amount must be greater than zero");
    }

    // Validate interest rate
    if interest_rate > doli_core::transaction::COLLATERAL_MAX_INTEREST_BPS {
        anyhow::bail!(
            "Interest rate {} bps exceeds maximum {} bps",
            interest_rate,
            doli_core::transaction::COLLATERAL_MAX_INTEREST_BPS
        );
    }

    // Get pool info via RPC to get asset_b_id
    let pool_info: serde_json::Value = rpc
        .call_raw("getPoolInfo", serde_json::json!({ "poolId": pool_id_hex }))
        .await?;

    let asset_b_hex = pool_info
        .get("assetB")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot read pool assetB"))?;
    let asset_b_id = Hash::from_hex(asset_b_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid asset B ID in pool info"))?;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build Collateral output (creation_slot=0, node stamps real slot at apply)
    let collateral_output = Output::collateral(
        collateral_amount,
        pool_id,
        from_hash,
        borrow_amount,
        interest_rate,
        0, // creation_slot stamped by node
        doli_core::transaction::COLLATERAL_DEFAULT_LIQUIDATION_BPS,
        asset_b_id,
    );

    // Build Normal DOLI output for the borrowed amount (minted DOLI to borrower)
    let borrow_output = Output::normal(borrow_amount, from_hash);

    // Select FungibleAsset UTXOs for collateral (filter by asset_id)
    let all_utxos = rpc.get_utxos(&from_pubkey_hash, false).await?;
    let (token_selected, token_total) =
        select_token_utxos(&all_utxos, asset_b_hex, collateral_amount);
    if token_total < collateral_amount {
        anyhow::bail!(
            "Insufficient token balance for asset {}. Available: {}, Required: {}",
            &asset_b_hex[..16.min(asset_b_hex.len())],
            token_total,
            collateral_amount
        );
    }

    // Calculate fee: base + per-byte for collateral output extra_data
    let fee_units = {
        let extra_bytes: u64 = collateral_output.extra_data.len() as u64;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    let doli_utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut fee_total = 0u64;
    let mut fee_selected = Vec::new();
    for utxo in &doli_utxos {
        if fee_total >= fee_units {
            break;
        }
        fee_selected.push(utxo.clone());
        fee_total += utxo.amount;
    }
    if fee_total < fee_units {
        anyhow::bail!("Insufficient DOLI for fee");
    }

    // Build inputs: token UTXOs first, then DOLI fee UTXOs
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &token_selected {
        let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid token UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }
    let num_token_inputs = inputs.len();

    for utxo in &fee_selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build outputs
    let mut outputs = vec![collateral_output, borrow_output];

    // Token change (if any)
    let token_change = token_total - collateral_amount;
    if token_change > 0 {
        outputs.push(token_output(token_change, from_hash, asset_b_id)?);
    }

    // DOLI fee change (if any)
    let doli_change = fee_total - fee_units;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::CreateLoan,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign all inputs + set covenant witnesses for FungibleAsset inputs
    let keypair = wallet.primary_keypair()?;
    let mut witnesses: Vec<Vec<u8>> = Vec::new();
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
        // FungibleAsset inputs (token inputs) need covenant witnesses
        if i < num_token_inputs {
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

    println!("Create Loan:");
    println!("  Pool ID:        {}", pool_id_hex);
    println!("  Collateral:     {} tokens", collateral_amount);
    println!("  Borrow:         {}", format_balance(borrow_amount));
    println!("  Interest Rate:  {} bps", interest_rate);
    println!("  TX Hash:        {}", tx_hash.to_hex());
    println!("  Size:           {} bytes", tx_bytes.len());

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
            println!("Loan created successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Loan creation failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// loan repay
// =============================================================================

async fn cmd_loan_repay(
    wallet_path: &Path,
    rpc_endpoint: &str,
    loan_utxo_str: &str,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let parts: Vec<&str> = loan_utxo_str.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid UTXO format. Expected txhash:output_index");
    }
    let loan_tx_hash_hex = parts[0];
    let loan_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let loan_tx_hash =
        Hash::from_hex(loan_tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash hex"))?;

    // Get the Collateral UTXO via getLoanInfo RPC
    let loan_info: serde_json::Value = rpc
        .call_raw(
            "getLoanInfo",
            serde_json::json!({
                "txHash": loan_tx_hash_hex,
                "outputIndex": loan_output_index,
            }),
        )
        .await?;

    let collateral_amount = loan_info
        .get("collateralAmount")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read collateral amount"))?;

    let total_debt = loan_info
        .get("totalDebt")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read total debt"))?;

    let collateral_asset_hex = loan_info
        .get("collateralAssetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot read collateral asset ID"))?;
    let collateral_asset_id = Hash::from_hex(collateral_asset_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid collateral asset ID"))?;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build inputs: Collateral UTXO + DOLI UTXOs (to cover debt + fee)
    let mut inputs = vec![Input::new(loan_tx_hash, loan_output_index)];

    let fee_units = 1u64;
    let required_doli = total_debt + fee_units;
    let doli_utxos: Vec<_> = rpc
        .get_utxos(&from_pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();

    let mut total_doli = 0u64;
    for utxo in &doli_utxos {
        if total_doli >= required_doli {
            break;
        }
        let prev_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_hash, utxo.output_index));
        total_doli += utxo.amount;
    }
    if total_doli < required_doli {
        anyhow::bail!(
            "Insufficient DOLI to repay loan. Available: {}, Required: {} (debt) + {} (fee)",
            format_balance(total_doli),
            format_balance(total_debt),
            format_balance(fee_units)
        );
    }

    // Build outputs: FungibleAsset returning collateral to borrower, DOLI change
    let mut outputs = vec![token_output(
        collateral_amount,
        from_hash,
        collateral_asset_id,
    )?];

    let doli_change = total_doli - required_doli;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::RepayLoan,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign all inputs
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Repay Loan:");
    println!(
        "  Loan UTXO:      {}:{}",
        loan_tx_hash_hex, loan_output_index
    );
    println!("  Total Debt:     {}", format_balance(total_debt));
    println!("  Collateral:     {} tokens returned", collateral_amount);
    println!("  TX Hash:        {}", tx_hash.to_hex());
    println!("  Size:           {} bytes", tx_bytes.len());

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
            println!("Loan repaid successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Loan repayment failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// loan liquidate
// =============================================================================

async fn cmd_loan_liquidate(
    wallet_path: &Path,
    rpc_endpoint: &str,
    loan_utxo_str: &str,
    yes: bool,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let parts: Vec<&str> = loan_utxo_str.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid UTXO format. Expected txhash:output_index");
    }
    let loan_tx_hash_hex = parts[0];
    let loan_output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let loan_tx_hash =
        Hash::from_hex(loan_tx_hash_hex).ok_or_else(|| anyhow::anyhow!("Invalid tx hash hex"))?;

    // Get loan info to check if liquidatable
    let loan_info: serde_json::Value = rpc
        .call_raw(
            "getLoanInfo",
            serde_json::json!({
                "txHash": loan_tx_hash_hex,
                "outputIndex": loan_output_index,
            }),
        )
        .await?;

    let liquidatable = loan_info
        .get("liquidatable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !liquidatable {
        let ltv = loan_info
            .get("ltvBps")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let liq_ratio = loan_info
            .get("liquidationRatioBps")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        anyhow::bail!(
            "Loan is not liquidatable. Current LTV: {} bps, Liquidation ratio: {} bps",
            ltv,
            liq_ratio
        );
    }

    let collateral_amount = loan_info
        .get("collateralAmount")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read collateral amount"))?;

    let collateral_asset_hex = loan_info
        .get("collateralAssetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot read collateral asset ID"))?;
    let collateral_asset_id = Hash::from_hex(collateral_asset_hex)
        .ok_or_else(|| anyhow::anyhow!("Invalid collateral asset ID"))?;

    let total_debt = loan_info
        .get("totalDebt")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("Cannot read total debt"))?;

    let from_pubkey_hash = wallet.primary_pubkey_hash();
    let from_hash =
        Hash::from_hex(&from_pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid pubkey hash"))?;

    // Build inputs: Collateral UTXO + DOLI for fee
    let mut inputs = vec![Input::new(loan_tx_hash, loan_output_index)];

    let fee_units = 1u64;
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

    // Build outputs: FungibleAsset collateral to liquidator
    let mut outputs = vec![token_output(
        collateral_amount,
        from_hash,
        collateral_asset_id,
    )?];

    let doli_change = fee_total - fee_units;
    if doli_change > 0 {
        outputs.push(Output::normal(doli_change, from_hash));
    }

    let mut tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::LiquidateLoan,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };

    // Sign all inputs
    let keypair = wallet.primary_keypair()?;
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
    }

    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!("Liquidate Loan:");
    println!(
        "  Loan UTXO:      {}:{}",
        loan_tx_hash_hex, loan_output_index
    );
    println!("  Total Debt:     {}", format_balance(total_debt));
    println!("  Collateral:     {} tokens seized", collateral_amount);
    println!("  TX Hash:        {}", tx_hash.to_hex());
    println!("  Size:           {} bytes", tx_bytes.len());

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
            println!("Loan liquidated successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Loan liquidation failed: {}", e));
        }
    }

    Ok(())
}

// =============================================================================
// loan list
// =============================================================================

async fn cmd_loan_list(rpc_endpoint: &str, borrower: Option<&str>) -> Result<()> {
    let client = RpcClient::new(rpc_endpoint);

    let mut params = serde_json::json!({});
    if let Some(b) = borrower {
        params["borrower"] = serde_json::json!(b);
    }

    let result: serde_json::Value = client.call_raw("getLoanList", params).await?;
    let empty = vec![];
    let loans = result.as_array().unwrap_or(&empty);

    if loans.is_empty() {
        println!("No active loans found.");
        return Ok(());
    }

    println!(
        "{:<20} {:<15} {:<15} {:<15} {:<10}",
        "OUTPOINT", "PRINCIPAL", "TOTAL_DEBT", "COLLATERAL", "STATUS"
    );
    println!("{}", "-".repeat(75));

    for loan in loans {
        let outpoint = loan.get("outpoint").unwrap_or(&serde_json::Value::Null);
        let tx_hash = outpoint
            .get("txHash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let idx = outpoint
            .get("outputIndex")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let principal = loan.get("principal").and_then(|v| v.as_u64()).unwrap_or(0);
        let total_debt = loan.get("totalDebt").and_then(|v| v.as_u64()).unwrap_or(0);
        let collateral = loan
            .get("collateralAmount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let liquidatable = loan
            .get("liquidatable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let status = if liquidatable {
            "LIQUIDATABLE"
        } else {
            "HEALTHY"
        };
        let short_hash = if tx_hash.len() > 12 {
            &tx_hash[..12]
        } else {
            tx_hash
        };

        println!(
            "{}..:{:<4} {:<15} {:<15} {:<15} {:<10}",
            short_hash, idx, principal, total_debt, collateral, status
        );
    }

    Ok(())
}

// =============================================================================
// loan info
// =============================================================================

async fn cmd_loan_info(rpc_endpoint: &str, loan_utxo: &str) -> Result<()> {
    let parts: Vec<&str> = loan_utxo.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid UTXO format. Expected txhash:output_index");
    }
    let tx_hash = parts[0];
    let output_index: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    let client = RpcClient::new(rpc_endpoint);
    let params = serde_json::json!({
        "txHash": tx_hash,
        "outputIndex": output_index,
    });

    let result: serde_json::Value = client.call_raw("getLoanInfo", params).await?;

    println!("Loan Info:");
    println!(
        "  Pool ID:              {}",
        result.get("poolId").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  Borrower:             {}",
        result
            .get("borrowerHash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    println!(
        "  Collateral Amount:    {}",
        result
            .get("collateralAmount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Collateral Asset ID:  {}",
        result
            .get("collateralAssetId")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    println!(
        "  Principal:            {}",
        result
            .get("principal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Interest Rate:        {} bps",
        result
            .get("interestRateBps")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Accrued Interest:     {}",
        result
            .get("accruedInterest")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Total Debt:           {}",
        result
            .get("totalDebt")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Elapsed Slots:        {}",
        result
            .get("elapsedSlots")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  LTV:                  {} bps",
        result.get("ltvBps").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!(
        "  Liquidation Ratio:    {} bps",
        result
            .get("liquidationRatioBps")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );
    println!(
        "  Liquidatable:         {}",
        result
            .get("liquidatable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    );

    Ok(())
}
