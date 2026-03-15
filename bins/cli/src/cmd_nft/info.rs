use anyhow::Result;

use crate::common::address_prefix;
use crate::rpc_client::{format_balance, RpcClient};

pub(crate) async fn cmd_nft_info(rpc_endpoint: &str, utxo_ref: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }

    let tx_info = rpc.get_transaction_json(parts[0]).await?;
    let output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(parts[1].parse::<usize>().unwrap_or(0)))
        .ok_or_else(|| anyhow::anyhow!("Cannot find output {}:{}", parts[0], parts[1]))?;

    let output_type = output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if output_type != "nft" {
        anyhow::bail!("Output is not an NFT (type: {})", output_type);
    }

    let nft = output
        .get("nft")
        .ok_or_else(|| anyhow::anyhow!("Missing NFT metadata"))?;

    let owner = output
        .get("pubkeyHash")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let amount = output.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);

    println!("NFT Info:");
    println!("{:-<60}", "");
    println!(
        "  Token ID:     {}",
        nft.get("tokenId").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "  Content Hash: {}",
        nft.get("contentHash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    // Try to decode content hash as UTF-8 (might be a URI)
    if let Some(ch) = nft.get("contentHash").and_then(|v| v.as_str()) {
        if let Ok(bytes) = hex::decode(ch) {
            if let Ok(uri) = std::str::from_utf8(&bytes) {
                if uri.starts_with("http") || uri.starts_with("ipfs") {
                    println!("  Content URI:  {}", uri);
                }
            }
        }
    }

    if let Some(owner_hash) = crypto::Hash::from_hex(owner) {
        let addr = crypto::address::encode(&owner_hash, address_prefix())
            .unwrap_or_else(|_| owner.to_string());
        println!("  Owner:        {}", addr);
    } else {
        println!("  Owner:        {}", owner);
    }

    if amount > 0 {
        println!("  Value:        {}", format_balance(amount));
    }

    if let Some(cond) = output.get("condition") {
        println!("  Condition:    {}", cond);
    }

    Ok(())
}
