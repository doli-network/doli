use std::path::Path;

use anyhow::Result;

use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(crate) async fn cmd_nft_list(wallet_path: &Path, rpc_endpoint: &str) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    let pubkey_hash = wallet.primary_pubkey_hash();
    let response = rpc.get_utxos_json(&pubkey_hash, false).await?;
    let utxos = response
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid UTXO response"))?;

    let nfts: Vec<_> = utxos
        .iter()
        .filter(|u| u.get("outputType").and_then(|v| v.as_str()) == Some("nft"))
        .collect();

    if nfts.is_empty() {
        println!("No NFTs found in wallet.");
        return Ok(());
    }

    println!("NFTs ({}):", nfts.len());
    println!("{:-<80}", "");
    for nft in &nfts {
        let tx_hash = nft.get("txHash").and_then(|v| v.as_str()).unwrap_or("?");
        let idx = nft.get("outputIndex").and_then(|v| v.as_u64()).unwrap_or(0);
        let amount = nft.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);

        if let Some(meta) = nft.get("nft") {
            let token = meta.get("tokenId").and_then(|v| v.as_str()).unwrap_or("?");
            let content = meta
                .get("contentHash")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content_display = if let Ok(bytes) = hex::decode(content) {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    if s.starts_with("http") || s.starts_with("ipfs") || s.is_ascii() {
                        s.to_string()
                    } else {
                        format!("{}...", &content[..std::cmp::min(32, content.len())])
                    }
                } else {
                    format!("{}...", &content[..std::cmp::min(32, content.len())])
                }
            } else {
                content.to_string()
            };

            let tx_prefix = if tx_hash.len() >= 16 {
                &tx_hash[..16]
            } else {
                tx_hash
            };
            println!("  UTXO:    {}:{}", tx_prefix, idx);
            if token.len() >= 16 {
                println!("  Token:   {}...{}", &token[..8], &token[token.len() - 8..]);
            } else {
                println!("  Token:   {}", token);
            }
            if !content_display.is_empty() {
                println!("  Content: {}", content_display);
            }
            if amount > 1 {
                println!("  Value:   {}", format_balance(amount));
            }
            println!();
        }
    }

    Ok(())
}
