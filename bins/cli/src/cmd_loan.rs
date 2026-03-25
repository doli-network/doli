use std::path::Path;

use anyhow::Result;

use crate::commands::LoanCommands;
use crate::rpc_client::RpcClient;

pub(crate) async fn cmd_loan(
    _wallet_path: &Path,
    rpc_endpoint: &str,
    command: LoanCommands,
) -> Result<()> {
    match command {
        LoanCommands::Deposit {
            pool,
            amount,
            yes: _,
        } => {
            println!("Lending deposit: {} DOLI into pool {}", amount, pool);
            println!("(Transaction building not yet wired to node apply_block)");
            Ok(())
        }
        LoanCommands::Withdraw {
            deposit_utxo,
            yes: _,
        } => {
            println!("Lending withdraw from deposit UTXO: {}", deposit_utxo);
            println!("(Transaction building not yet wired to node apply_block)");
            Ok(())
        }
        LoanCommands::Create {
            pool,
            collateral,
            borrow,
            interest_rate,
            yes: _,
        } => {
            println!(
                "Create loan: borrow {} DOLI against {} collateral in pool {} at {}bps",
                borrow, collateral, pool, interest_rate
            );
            println!("(Transaction building not yet wired to node apply_block)");
            Ok(())
        }
        LoanCommands::Repay { loan_utxo, yes: _ } => {
            println!("Repay loan: {}", loan_utxo);
            println!("(Transaction building not yet wired to node apply_block)");
            Ok(())
        }
        LoanCommands::Liquidate { loan_utxo, yes: _ } => {
            println!("Liquidate loan: {}", loan_utxo);
            println!("(Transaction building not yet wired to node apply_block)");
            Ok(())
        }
        LoanCommands::List { borrower } => cmd_loan_list(rpc_endpoint, borrower.as_deref()).await,
        LoanCommands::Info { loan_utxo } => cmd_loan_info(rpc_endpoint, &loan_utxo).await,
    }
}

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
