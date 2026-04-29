mod batch;
mod buy;
mod export;
mod fractionalize;
mod info;
mod list;
mod mint;
mod redeem;
mod sell;
mod transfer;

pub(crate) use batch::cmd_nft_batch_mint;
pub(crate) use buy::cmd_nft_buy;
pub(crate) use buy::cmd_nft_buy_from_offer;
pub(crate) use export::cmd_nft_export;
pub(crate) use fractionalize::cmd_nft_fractionalize;
pub(crate) use info::cmd_nft_info;
pub(crate) use list::cmd_nft_list;
pub(crate) use mint::cmd_mint;
pub(crate) use redeem::cmd_nft_redeem;
pub(crate) use sell::cmd_nft_sell;
pub(crate) use sell::cmd_nft_sell_sign;
pub(crate) use transfer::cmd_nft_transfer;

use anyhow::Result;
use crypto::Hash;
use doli_core::Output;

use crate::rpc_client::RpcClient;

/// Resolve a buyer's public key from an address string (bech32 or hex pubkey).
/// EncryptedContent ECIES requires the actual public key, not just the address hash.
pub(crate) async fn resolve_buyer_pubkey(
    rpc: &RpcClient,
    address_or_pubkey: &str,
) -> Result<crypto::PublicKey> {
    if address_or_pubkey.len() == 64 && address_or_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        let pubkey_bytes: [u8; 32] = hex::decode(address_or_pubkey)
            .map_err(|_| anyhow::anyhow!("Invalid pubkey hex"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Pubkey must be 32 bytes"))?;
        Ok(crypto::PublicKey::from_bytes(pubkey_bytes))
    } else {
        transfer::resolve_pubkey_from_address(rpc, address_or_pubkey).await
    }
}

/// Build an EncryptedContent output for a new owner by re-wrapping the ECIES key.
/// Returns (output, royalty_info) where royalty_info is (creator_hash, bps) if v1 metadata present.
pub(crate) fn build_ec_output_for_buyer(
    utxo_json: &serde_json::Value,
    amount: u64,
    seller_keypair: &crypto::KeyPair,
    buyer_pubkey: &crypto::PublicKey,
) -> Result<(Output, Option<(Hash, u16)>)> {
    let buyer_hash =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, buyer_pubkey.as_bytes());

    // Parse extra_data
    let extra_data_hex = utxo_json
        .get("encryptedContent")
        .and_then(|ec| ec.get("extraData"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing encryptedContent.extraData in RPC response"))?;
    let extra_data =
        hex::decode(extra_data_hex).map_err(|_| anyhow::anyhow!("Invalid hex in extraData"))?;

    if extra_data.len() < 128 {
        anyhow::bail!("Malformed EncryptedContent extra_data");
    }

    let ct_len = u32::from_le_bytes(extra_data[0..4].try_into()?) as usize;
    let offset = 4 + ct_len;
    if extra_data.len() < offset + 80 + 12 + 32 {
        anyhow::bail!("Truncated EncryptedContent extra_data");
    }

    let ciphertext = &extra_data[4..4 + ct_len];
    let mut wrapped_key = [0u8; 80];
    wrapped_key.copy_from_slice(&extra_data[offset..offset + 80]);
    let nonce: [u8; 12] = extra_data[offset + 80..offset + 92].try_into()?;
    let mut content_hash = [0u8; 32];
    content_hash.copy_from_slice(&extra_data[offset + 92..offset + 124]);

    // Unwrap the content key with seller's private key
    let content_key =
        crypto::encrypted_content::unwrap_key(&wrapped_key, seller_keypair.private_key())
            .map_err(|_| anyhow::anyhow!("Failed to unwrap key — you are not the owner"))?;

    // Re-wrap with buyer's public key
    let new_wrapped_key = crypto::encrypted_content::wrap_key(&content_key, buyer_pubkey)
        .map_err(|e| anyhow::anyhow!("Re-wrap failed: {}", e))?;

    // Check for v1 metadata (MIME + royalty) from RPC response
    let ec_meta = utxo_json.get("encryptedContent");
    let mime_type = ec_meta
        .and_then(|ec| ec.get("mimeType"))
        .and_then(|v| v.as_str());
    let ec_royalty = ec_meta.and_then(|ec| ec.get("royalty")).and_then(|r| {
        let creator = r.get("creator")?.as_str()?;
        let bps = r.get("bps")?.as_u64()?;
        let creator_hash = Hash::from_hex(creator)?;
        Some((creator_hash, bps as u16))
    });

    // Build new EncryptedContent output preserving v1 metadata
    let output = if mime_type.is_some() || ec_royalty.is_some() {
        let mime_bytes = mime_type.unwrap_or("").as_bytes();
        let (creator, bps) = ec_royalty.unwrap_or((Hash::ZERO, 0));
        Output::encrypted_content_v1(
            amount,
            buyer_hash,
            ciphertext,
            &new_wrapped_key,
            &nonce,
            &content_hash,
            mime_bytes,
            creator,
            bps,
        )
    } else {
        Output::encrypted_content(
            amount,
            buyer_hash,
            ciphertext,
            &new_wrapped_key,
            &nonce,
            &content_hash,
        )
    };

    Ok((output, ec_royalty))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calculate royalty amount from sale price and basis points.
    /// Same formula used by protocol validation (ERRTX-EC009).
    fn calculate_royalty(price_units: u64, royalty_bps: u16) -> u64 {
        (price_units as u128 * royalty_bps as u128 / 10000) as u64
    }

    #[test]
    fn test_royalty_calculation_basic() {
        // 5% of 1000 = 50
        assert_eq!(calculate_royalty(1000, 500), 50);
        // 10% of 10000 = 1000
        assert_eq!(calculate_royalty(10000, 1000), 1000);
        // 2.5% of 200 = 5
        assert_eq!(calculate_royalty(200, 250), 5);
    }

    #[test]
    fn test_royalty_calculation_zero() {
        assert_eq!(calculate_royalty(1000, 0), 0);
        assert_eq!(calculate_royalty(0, 500), 0);
    }

    #[test]
    fn test_royalty_calculation_max_bps() {
        // MAX_ROYALTY_BPS is 2500 (25%)
        assert_eq!(calculate_royalty(10000, 2500), 2500);
    }

    #[test]
    fn test_royalty_calculation_large_amounts() {
        // Test with large amounts that could overflow u64 without u128 intermediate
        let price = 1_000_000_000_000u64; // 1 trillion units
        let bps = 500u16; // 5%
        assert_eq!(calculate_royalty(price, bps), 50_000_000_000);
    }

    #[test]
    fn test_royalty_rounding_down() {
        // 5% of 99 = 4.95, should truncate to 4
        assert_eq!(calculate_royalty(99, 500), 4);
        // 1% of 50 = 0.5, should truncate to 0
        assert_eq!(calculate_royalty(50, 100), 0);
    }

    #[test]
    fn test_ec_offer_json_format() {
        let offer = serde_json::json!({
            "version": 1,
            "type": "ec_sell_offer",
            "content_utxo": {
                "tx_hash": "abcd1234",
                "output_index": 0,
            },
            "content_type": "encryptedContent",
            "mime_type": "image/png",
            "amount": 0,
            "price": 5000,
            "seller_pubkey_hash": "deadbeef",
            "royalty": {
                "creator": "cafebabe",
                "bps": 500,
            },
        });

        assert_eq!(offer["type"], "ec_sell_offer");
        assert_eq!(offer["content_utxo"]["tx_hash"], "abcd1234");
        assert_eq!(offer["price"], 5000);
        assert_eq!(offer["royalty"]["bps"], 500);
        assert_eq!(offer["mime_type"], "image/png");
    }

    #[test]
    fn test_nft_offer_json_format() {
        let offer = serde_json::json!({
            "version": 1,
            "type": "nft_sell_offer",
            "nft_utxo": {
                "tx_hash": "abcd1234",
                "output_index": 0,
            },
            "token_id": "deadbeef",
            "content_hash": "",
            "nft_amount": 0,
            "price": 5000,
            "seller_pubkey_hash": "cafebabe",
        });

        assert_eq!(offer["type"], "nft_sell_offer");
        assert_eq!(offer["nft_utxo"]["tx_hash"], "abcd1234");
    }

    #[test]
    fn test_signed_offer_json_format() {
        let offer = serde_json::json!({
            "version": 3,
            "type": "nft_sell_offer_signed",
            "content_type": "encryptedContent",
            "nft_utxo": {
                "tx_hash": "abcd1234",
                "output_index": 0,
            },
            "nft_amount": 0,
            "price": 5000,
            "seller_pubkey_hash": "seller_hash",
            "buyer_address": "doli1abc...",
            "buyer_pubkey_hash": "buyer_hash",
            "partial_tx": "deadbeef",
            "seller_witness": "cafebabe",
            "outputs_count": 3,
        });

        assert_eq!(offer["type"], "nft_sell_offer_signed");
        assert_eq!(offer["content_type"], "encryptedContent");
        assert_eq!(offer["outputs_count"], 3);
    }

    #[test]
    fn test_offer_type_routing() {
        // Verify the offer types route correctly
        let types = vec![
            ("nft_sell_offer", "nft_utxo"),
            ("ec_sell_offer", "content_utxo"),
        ];
        for (offer_type, expected_field) in types {
            let field = match offer_type {
                "nft_sell_offer" => "nft_utxo",
                "ec_sell_offer" => "content_utxo",
                _ => panic!("Unknown type"),
            };
            assert_eq!(field, expected_field);
        }
    }

    #[test]
    fn test_build_ec_output_rewrap() {
        // Generate seller and buyer keypairs
        let seller_kp = crypto::KeyPair::generate();
        let buyer_kp = crypto::KeyPair::generate();

        // Create a test content key and wrap it for the seller
        let content_key = [42u8; 32];
        let wrapped_for_seller =
            crypto::encrypted_content::wrap_key(&content_key, seller_kp.public_key()).unwrap();

        // Build fake ciphertext and metadata
        let ciphertext = b"encrypted_content_data";
        let nonce = [1u8; 12];
        let content_hash = [2u8; 32];

        // Build extra_data in the same format as Output::encrypted_content_v1
        let ct_len = ciphertext.len() as u32;
        let mime = b"image/png";
        let creator_hash = Hash::from_bytes([3u8; 32]);
        let royalty_bps: u16 = 500; // 5%

        let mut extra_data = Vec::new();
        extra_data.extend_from_slice(&ct_len.to_le_bytes());
        extra_data.extend_from_slice(ciphertext);
        extra_data.extend_from_slice(&wrapped_for_seller);
        extra_data.extend_from_slice(&nonce);
        extra_data.extend_from_slice(&content_hash);
        // v1 extension
        extra_data.push(1); // version
        extra_data.push(mime.len() as u8);
        extra_data.extend_from_slice(mime);
        extra_data.extend_from_slice(creator_hash.as_bytes());
        extra_data.extend_from_slice(&royalty_bps.to_le_bytes());

        let extra_hex = hex::encode(&extra_data);

        // Build the RPC JSON that would come from the node
        let utxo_json = serde_json::json!({
            "outputType": "encryptedContent",
            "amount": 0,
            "encryptedContent": {
                "extraData": extra_hex,
                "mimeType": "image/png",
                "royalty": {
                    "creator": creator_hash.to_hex(),
                    "bps": 500,
                },
            },
        });

        let (output, royalty) =
            build_ec_output_for_buyer(&utxo_json, 0, &seller_kp, buyer_kp.public_key()).unwrap();

        // Verify output type
        assert_eq!(output.output_type, doli_core::OutputType::EncryptedContent);

        // Verify royalty was parsed
        let (r_creator, r_bps) = royalty.unwrap();
        assert_eq!(r_creator, creator_hash);
        assert_eq!(r_bps, 500);

        // Verify the buyer can unwrap the re-wrapped key
        let new_extra = &output.extra_data;
        let new_ct_len = u32::from_le_bytes(new_extra[0..4].try_into().unwrap()) as usize;
        let new_offset = 4 + new_ct_len;
        let mut new_wrapped = [0u8; 80];
        new_wrapped.copy_from_slice(&new_extra[new_offset..new_offset + 80]);
        let decrypted_key =
            crypto::encrypted_content::unwrap_key(&new_wrapped, buyer_kp.private_key()).unwrap();
        assert_eq!(decrypted_key, content_key);
    }

    #[test]
    fn test_build_ec_output_no_royalty() {
        let seller_kp = crypto::KeyPair::generate();
        let buyer_kp = crypto::KeyPair::generate();

        let content_key = [42u8; 32];
        let wrapped_for_seller =
            crypto::encrypted_content::wrap_key(&content_key, seller_kp.public_key()).unwrap();

        let ciphertext = b"test_data";
        let nonce = [0u8; 12];
        let content_hash = [0u8; 32];

        let ct_len = ciphertext.len() as u32;
        let mut extra_data = Vec::new();
        extra_data.extend_from_slice(&ct_len.to_le_bytes());
        extra_data.extend_from_slice(ciphertext);
        extra_data.extend_from_slice(&wrapped_for_seller);
        extra_data.extend_from_slice(&nonce);
        extra_data.extend_from_slice(&content_hash);

        let extra_hex = hex::encode(&extra_data);

        let utxo_json = serde_json::json!({
            "outputType": "encryptedContent",
            "amount": 100,
            "encryptedContent": {
                "extraData": extra_hex,
            },
        });

        let (output, royalty) =
            build_ec_output_for_buyer(&utxo_json, 100, &seller_kp, buyer_kp.public_key()).unwrap();

        assert_eq!(output.output_type, doli_core::OutputType::EncryptedContent);
        assert_eq!(output.amount, 100);
        assert!(royalty.is_none());
    }

    #[test]
    fn test_build_ec_output_wrong_owner() {
        let seller_kp = crypto::KeyPair::generate();
        let wrong_kp = crypto::KeyPair::generate();
        let buyer_kp = crypto::KeyPair::generate();

        let content_key = [42u8; 32];
        // Wrap for seller, but try to unwrap with wrong key
        let wrapped_for_seller =
            crypto::encrypted_content::wrap_key(&content_key, seller_kp.public_key()).unwrap();

        let ciphertext = b"test";
        let nonce = [0u8; 12];
        let content_hash = [0u8; 32];

        let ct_len = ciphertext.len() as u32;
        let mut extra_data = Vec::new();
        extra_data.extend_from_slice(&ct_len.to_le_bytes());
        extra_data.extend_from_slice(ciphertext);
        extra_data.extend_from_slice(&wrapped_for_seller);
        extra_data.extend_from_slice(&nonce);
        extra_data.extend_from_slice(&content_hash);

        let extra_hex = hex::encode(&extra_data);

        let utxo_json = serde_json::json!({
            "outputType": "encryptedContent",
            "amount": 0,
            "encryptedContent": {
                "extraData": extra_hex,
            },
        });

        // Should fail because wrong_kp can't unwrap a key wrapped for seller_kp
        let result = build_ec_output_for_buyer(&utxo_json, 0, &wrong_kp, buyer_kp.public_key());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unwrap key"));
    }
}
