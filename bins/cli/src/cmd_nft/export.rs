use anyhow::Result;

use crate::rpc_client::RpcClient;

pub(crate) async fn cmd_nft_export(
    rpc_endpoint: &str,
    utxo_ref: &str,
    out_path: &str,
) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse UTXO ref
    let parts: Vec<&str> = utxo_ref.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("UTXO format: txhash:output_index");
    }
    let tx_hash = parts[0];
    let output_index: usize = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid output index"))?;

    // Fetch transaction
    let tx_info = rpc.get_transaction_json(tx_hash).await?;
    let output = tx_info
        .get("outputs")
        .and_then(|o| o.as_array())
        .and_then(|arr| arr.get(output_index))
        .ok_or_else(|| anyhow::anyhow!("Output not found"))?;

    // Verify it's an NFT
    let output_type = output
        .get("outputType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if output_type != "nft" {
        anyhow::bail!("Output is not an NFT (type: {})", output_type);
    }

    // Extract content bytes from NFT metadata
    let nft = output
        .get("nft")
        .ok_or_else(|| anyhow::anyhow!("Missing NFT metadata"))?;
    let content_hex = nft
        .get("contentHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing contentHash in NFT"))?;

    if content_hex.is_empty() {
        anyhow::bail!("NFT has empty content");
    }

    let content_bytes =
        hex::decode(content_hex).map_err(|_| anyhow::anyhow!("Invalid hex in contentHash"))?;

    // Detect format
    let format = doli_core::detect_content_format(&content_bytes);
    let format_name = doli_core::format_name(&format);

    // Handle DOLI pixel art -- decode RLE to PPM
    match &format {
        doli_core::NftContentFormat::DoliPixelArt {
            width,
            height,
            palette_colors,
        } => {
            let data = &content_bytes;
            let w = *width as u32;
            let h = *height as u32;
            let pal_size = *palette_colors as usize;

            // Header: version(1) + width(1) + height(1) + palette_size(1)
            let header_len = 4;

            if data.len() < header_len + pal_size * 3 {
                // Not enough data for palette -- write raw
                std::fs::write(out_path, &content_bytes)?;
                println!(
                    "Exported NFT to {} ({}, {} bytes)",
                    out_path,
                    format_name,
                    content_bytes.len()
                );
                return Ok(());
            }

            let palette = &data[header_len..header_len + pal_size * 3];
            let pixel_data = &data[header_len + pal_size * 3..];

            // RLE decode: pairs of (count, palette_index)
            let mut pixels = Vec::new();
            for chunk in pixel_data.chunks(2) {
                if chunk.len() == 2 {
                    for _ in 0..chunk[0] {
                        pixels.push(chunk[1]);
                    }
                }
            }

            // Build PPM image (P6 binary format)
            let header = format!("P6\n{} {}\n255\n", w, h);
            let total_pixels = (w * h) as usize;
            let mut rgb = Vec::with_capacity(total_pixels * 3);
            for i in 0..total_pixels {
                let idx = if i < pixels.len() {
                    pixels[i] as usize
                } else {
                    0
                };
                let pi = idx * 3;
                if pi + 2 < palette.len() {
                    rgb.push(palette[pi]);
                    rgb.push(palette[pi + 1]);
                    rgb.push(palette[pi + 2]);
                } else {
                    rgb.extend_from_slice(&[0, 0, 0]);
                }
            }

            let mut file_data = header.into_bytes();
            file_data.extend_from_slice(&rgb);
            std::fs::write(out_path, &file_data)?;

            println!(
                "Exported NFT to {} ({}x{} DOLI pixel art -> PPM, {} bytes)",
                out_path,
                w,
                h,
                file_data.len()
            );
        }
        _ => {
            // All other formats: write raw bytes
            std::fs::write(out_path, &content_bytes)?;
            println!(
                "Exported NFT to {} ({}, {} bytes)",
                out_path,
                format_name,
                content_bytes.len()
            );
        }
    }

    Ok(())
}
