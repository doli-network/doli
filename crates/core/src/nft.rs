//! NFT content format detection via magic bytes.
//!
//! The UTXO stores raw bytes. What those bytes mean is determined by the
//! application layer (wallet, explorer, marketplace). This module provides
//! format detection so any client can identify and render content correctly
//! without external metadata.
//!
//! No protocol enforcement — detection is a hint for rendering.

/// Detected content format for an NFT or on-chain document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NftContentFormat {
    // ── Images ──────────────────────────────────────────────────────────
    /// PNG image (magic: 89 50 4E 47)
    Png,
    /// JPEG image (magic: FF D8 FF)
    Jpeg,
    /// GIF image (magic: 47 49 46 38)
    Gif,
    /// WebP image (magic: 52 49 46 46 ... 57 45 42 50)
    WebP,
    /// BMP image (magic: 42 4D)
    Bmp,
    /// ICO/favicon (magic: 00 00 01 00)
    Ico,
    /// TIFF image (magic: 49 49 2A 00 or 4D 4D 00 2A)
    Tiff,
    /// AVIF image (magic: ...66 74 79 70 61 76 69 66 at offset 4)
    Avif,
    /// SVG image (starts with "<svg" or "<?xml")
    Svg,
    /// DOLI pixel art (magic: 01 + width + height + palette_size)
    DoliPixelArt {
        /// Width in pixels
        width: u8,
        /// Height in pixels
        height: u8,
        /// Number of colors in palette
        palette_colors: u8,
    },

    // ── Documents ───────────────────────────────────────────────────────
    /// PDF document (magic: 25 50 44 46 = %PDF)
    Pdf,
    /// HTML document (starts with "<!DOCTYPE" or "<html")
    Html,
    /// JSON data (starts with { or [)
    Json,
    /// Markdown text (starts with # or ---)
    Markdown,
    /// CSV data (heuristic: lines with commas)
    Csv,

    // ── Audio ───────────────────────────────────────────────────────────
    /// MP3 audio (magic: FF FB or FF F3 or FF F2, or ID3 tag: 49 44 33)
    Mp3,
    /// OGG audio/video (magic: 4F 67 67 53)
    Ogg,
    /// WAV audio (magic: 52 49 46 46 ... 57 41 56 45)
    Wav,
    /// FLAC audio (magic: 66 4C 61 43)
    Flac,

    // ── Video ───────────────────────────────────────────────────────────
    /// MP4/M4V video (magic: ...66 74 79 70 at offset 4)
    Mp4,

    // ── Archives & Data ─────────────────────────────────────────────────
    /// ZIP archive (magic: 50 4B 03 04)
    Zip,
    /// GZIP compressed (magic: 1F 8B)
    Gzip,
    /// Protobuf / CBOR / MessagePack — binary serialization formats
    /// Detected as Binary (no unique magic), but listed for documentation.

    // ── Crypto & Proofs ─────────────────────────────────────────────────
    /// WASM bytecode (magic: 00 61 73 6D = \0asm)
    Wasm,

    // ── Generic ─────────────────────────────────────────────────────────
    /// Plain text (valid UTF-8, no structured format detected)
    Text,
    /// Raw binary data (unknown format)
    Binary,
    /// BLAKE3 hash reference (exactly 32 bytes — points to off-chain content)
    HashReference,
}

/// Detect the content format from raw bytes using magic byte signatures.
///
/// Returns the most specific format that matches. Falls back to `Text`
/// (if valid UTF-8) or `Binary` (otherwise).
///
/// # Examples
/// ```
/// use doli_core::nft::{detect_content_format, NftContentFormat};
///
/// // PNG image
/// let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
/// assert_eq!(detect_content_format(&png), NftContentFormat::Png);
///
/// // Plain text
/// assert_eq!(detect_content_format(b"hello"), NftContentFormat::Text);
/// ```
pub fn detect_content_format(data: &[u8]) -> NftContentFormat {
    if data.is_empty() {
        return NftContentFormat::Binary;
    }

    // Exactly 32 bytes = likely a BLAKE3/SHA256 hash reference
    if data.len() == 32 {
        return NftContentFormat::HashReference;
    }

    // ── Images ──────────────────────────────────────────────────────
    if data.len() >= 4 && data[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return NftContentFormat::Png;
    }
    if data.len() >= 3 && data[..3] == [0xFF, 0xD8, 0xFF] {
        return NftContentFormat::Jpeg;
    }
    if data.len() >= 4 && data[..4] == [0x47, 0x49, 0x46, 0x38] {
        return NftContentFormat::Gif;
    }
    if data.len() >= 12
        && data[..4] == [0x52, 0x49, 0x46, 0x46]
        && data[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return NftContentFormat::WebP;
    }
    if data.len() >= 2 && data[..2] == [0x42, 0x4D] {
        return NftContentFormat::Bmp;
    }
    if data.len() >= 4 && data[..4] == [0x00, 0x00, 0x01, 0x00] {
        return NftContentFormat::Ico;
    }
    if data.len() >= 4
        && ((data[..4] == [0x49, 0x49, 0x2A, 0x00]) || (data[..4] == [0x4D, 0x4D, 0x00, 0x2A]))
    {
        return NftContentFormat::Tiff;
    }
    // AVIF: ftyp box at offset 4 with "avif"
    if data.len() >= 12 && data[4..8] == [0x66, 0x74, 0x79, 0x70] {
        if data.len() >= 12 && data[8..12] == [0x61, 0x76, 0x69, 0x66] {
            return NftContentFormat::Avif;
        }
        // MP4/M4V: ftyp box with other brands
        return NftContentFormat::Mp4;
    }

    // ── Audio ───────────────────────────────────────────────────────
    // ID3 tag (MP3 with metadata)
    if data.len() >= 3 && data[..3] == [0x49, 0x44, 0x33] {
        return NftContentFormat::Mp3;
    }
    // MP3 frame sync
    if data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
        return NftContentFormat::Mp3;
    }
    if data.len() >= 4 && data[..4] == [0x4F, 0x67, 0x67, 0x53] {
        return NftContentFormat::Ogg;
    }
    if data.len() >= 12
        && data[..4] == [0x52, 0x49, 0x46, 0x46]
        && data[8..12] == [0x57, 0x41, 0x56, 0x45]
    {
        return NftContentFormat::Wav;
    }
    if data.len() >= 4 && data[..4] == [0x66, 0x4C, 0x61, 0x43] {
        return NftContentFormat::Flac;
    }

    // ── Documents ───────────────────────────────────────────────────
    if data.len() >= 4 && data[..4] == [0x25, 0x50, 0x44, 0x46] {
        return NftContentFormat::Pdf;
    }

    // ── Archives ────────────────────────────────────────────────────
    if data.len() >= 4 && data[..4] == [0x50, 0x4B, 0x03, 0x04] {
        return NftContentFormat::Zip;
    }
    if data.len() >= 2 && data[..2] == [0x1F, 0x8B] {
        return NftContentFormat::Gzip;
    }

    // ── WASM ────────────────────────────────────────────────────────
    if data.len() >= 4 && data[..4] == [0x00, 0x61, 0x73, 0x6D] {
        return NftContentFormat::Wasm;
    }

    // ── DOLI pixel art ──────────────────────────────────────────────
    // version=1, width>0, height>0, palette_size 1-64
    if data.len() >= 4
        && data[0] == 0x01
        && data[1] > 0
        && data[2] > 0
        && data[3] > 0
        && data[3] <= 64
    {
        return NftContentFormat::DoliPixelArt {
            width: data[1],
            height: data[2],
            palette_colors: data[3],
        };
    }

    // ── Text-based formats (must be valid UTF-8) ────────────────────
    if let Ok(text) = std::str::from_utf8(data) {
        let trimmed = text.trim();
        if trimmed.starts_with("<svg") || trimmed.starts_with("<?xml") {
            return NftContentFormat::Svg;
        }
        if trimmed.starts_with("<!DOCTYPE")
            || trimmed.starts_with("<html")
            || trimmed.starts_with("<HTML")
        {
            return NftContentFormat::Html;
        }
        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            return NftContentFormat::Json;
        }
        if trimmed.starts_with("# ") || trimmed.starts_with("---\n") || trimmed.starts_with("## ") {
            return NftContentFormat::Markdown;
        }
        // CSV heuristic: multiple lines, each with commas
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 && lines.iter().all(|l| l.contains(',')) {
            return NftContentFormat::Csv;
        }
        return NftContentFormat::Text;
    }

    NftContentFormat::Binary
}

/// Human-readable name for a content format.
pub fn format_name(format: &NftContentFormat) -> &'static str {
    match format {
        NftContentFormat::Png => "PNG image",
        NftContentFormat::Jpeg => "JPEG image",
        NftContentFormat::Gif => "GIF image",
        NftContentFormat::WebP => "WebP image",
        NftContentFormat::Bmp => "BMP image",
        NftContentFormat::Ico => "ICO icon",
        NftContentFormat::Tiff => "TIFF image",
        NftContentFormat::Avif => "AVIF image",
        NftContentFormat::Svg => "SVG image",
        NftContentFormat::DoliPixelArt { .. } => "DOLI pixel art",
        NftContentFormat::Pdf => "PDF document",
        NftContentFormat::Html => "HTML document",
        NftContentFormat::Json => "JSON data",
        NftContentFormat::Markdown => "Markdown document",
        NftContentFormat::Csv => "CSV data",
        NftContentFormat::Mp3 => "MP3 audio",
        NftContentFormat::Ogg => "OGG audio",
        NftContentFormat::Wav => "WAV audio",
        NftContentFormat::Flac => "FLAC audio",
        NftContentFormat::Mp4 => "MP4 video",
        NftContentFormat::Zip => "ZIP archive",
        NftContentFormat::Gzip => "GZIP compressed",
        NftContentFormat::Wasm => "WebAssembly",
        NftContentFormat::Text => "plain text",
        NftContentFormat::Binary => "binary data",
        NftContentFormat::HashReference => "hash reference (32 bytes)",
    }
}

/// MIME type for a content format (for HTTP serving, explorer display, etc.)
pub fn format_mime(format: &NftContentFormat) -> &'static str {
    match format {
        NftContentFormat::Png => "image/png",
        NftContentFormat::Jpeg => "image/jpeg",
        NftContentFormat::Gif => "image/gif",
        NftContentFormat::WebP => "image/webp",
        NftContentFormat::Bmp => "image/bmp",
        NftContentFormat::Ico => "image/x-icon",
        NftContentFormat::Tiff => "image/tiff",
        NftContentFormat::Avif => "image/avif",
        NftContentFormat::Svg => "image/svg+xml",
        NftContentFormat::DoliPixelArt { .. } => "image/x-doli-pixel",
        NftContentFormat::Pdf => "application/pdf",
        NftContentFormat::Html => "text/html",
        NftContentFormat::Json => "application/json",
        NftContentFormat::Markdown => "text/markdown",
        NftContentFormat::Csv => "text/csv",
        NftContentFormat::Mp3 => "audio/mpeg",
        NftContentFormat::Ogg => "audio/ogg",
        NftContentFormat::Wav => "audio/wav",
        NftContentFormat::Flac => "audio/flac",
        NftContentFormat::Mp4 => "video/mp4",
        NftContentFormat::Zip => "application/zip",
        NftContentFormat::Gzip => "application/gzip",
        NftContentFormat::Wasm => "application/wasm",
        NftContentFormat::Text => "text/plain",
        NftContentFormat::Binary => "application/octet-stream",
        NftContentFormat::HashReference => "application/x-hash-ref",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Images ──────────────────────────────────────────────────────

    #[test]
    fn test_detect_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_content_format(&data), NftContentFormat::Png);
    }

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_content_format(&data), NftContentFormat::Jpeg);
    }

    #[test]
    fn test_detect_gif() {
        let data = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_content_format(&data), NftContentFormat::Gif);
    }

    #[test]
    fn test_detect_webp() {
        let data = vec![
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
        ];
        assert_eq!(detect_content_format(&data), NftContentFormat::WebP);
    }

    #[test]
    fn test_detect_bmp() {
        let data = [0x42, 0x4D, 0x00, 0x00];
        assert_eq!(detect_content_format(&data), NftContentFormat::Bmp);
    }

    #[test]
    fn test_detect_tiff_le() {
        let data = [0x49, 0x49, 0x2A, 0x00];
        assert_eq!(detect_content_format(&data), NftContentFormat::Tiff);
    }

    #[test]
    fn test_detect_tiff_be() {
        let data = [0x4D, 0x4D, 0x00, 0x2A];
        assert_eq!(detect_content_format(&data), NftContentFormat::Tiff);
    }

    #[test]
    fn test_detect_avif() {
        let mut data = vec![0x00, 0x00, 0x00, 0x20]; // box size
        data.extend_from_slice(b"ftypavif"); // ftyp + brand
        assert_eq!(detect_content_format(&data), NftContentFormat::Avif);
    }

    #[test]
    fn test_detect_svg() {
        assert_eq!(
            detect_content_format(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            NftContentFormat::Svg
        );
    }

    #[test]
    fn test_detect_svg_xml() {
        assert_eq!(
            detect_content_format(b"<?xml version=\"1.0\"?><svg>"),
            NftContentFormat::Svg
        );
    }

    #[test]
    fn test_detect_doli_pixel_art() {
        let data = [0x01, 24, 24, 16, 0xFF, 0x00];
        assert_eq!(
            detect_content_format(&data),
            NftContentFormat::DoliPixelArt {
                width: 24,
                height: 24,
                palette_colors: 16
            }
        );
    }

    #[test]
    fn test_detect_doli_64x64() {
        let data = [0x01, 64, 64, 32, 0x00];
        assert_eq!(
            detect_content_format(&data),
            NftContentFormat::DoliPixelArt {
                width: 64,
                height: 64,
                palette_colors: 32
            }
        );
    }

    // ── Documents ───────────────────────────────────────────────────

    #[test]
    fn test_detect_pdf() {
        assert_eq!(detect_content_format(b"%PDF-1.7"), NftContentFormat::Pdf);
    }

    #[test]
    fn test_detect_html() {
        assert_eq!(
            detect_content_format(b"<!DOCTYPE html><html>"),
            NftContentFormat::Html
        );
    }

    #[test]
    fn test_detect_json_object() {
        assert_eq!(
            detect_content_format(b"{\"name\": \"Punk #42\"}"),
            NftContentFormat::Json
        );
    }

    #[test]
    fn test_detect_json_array() {
        assert_eq!(detect_content_format(b"[1, 2, 3]"), NftContentFormat::Json);
    }

    #[test]
    fn test_detect_markdown() {
        assert_eq!(
            detect_content_format(b"# Title\n\nSome content"),
            NftContentFormat::Markdown
        );
    }

    #[test]
    fn test_detect_csv() {
        assert_eq!(
            detect_content_format(b"name,age,city\nAlice,30,NYC\nBob,25,LA"),
            NftContentFormat::Csv
        );
    }

    // ── Audio ───────────────────────────────────────────────────────

    #[test]
    fn test_detect_mp3_id3() {
        assert_eq!(
            detect_content_format(&[0x49, 0x44, 0x33, 0x04]),
            NftContentFormat::Mp3
        );
    }

    #[test]
    fn test_detect_mp3_frame() {
        assert_eq!(
            detect_content_format(&[0xFF, 0xFB, 0x90, 0x00]),
            NftContentFormat::Mp3
        );
    }

    #[test]
    fn test_detect_ogg() {
        assert_eq!(
            detect_content_format(&[0x4F, 0x67, 0x67, 0x53]),
            NftContentFormat::Ogg
        );
    }

    #[test]
    fn test_detect_wav() {
        let data = vec![
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x41, 0x56, 0x45,
        ];
        assert_eq!(detect_content_format(&data), NftContentFormat::Wav);
    }

    #[test]
    fn test_detect_flac() {
        assert_eq!(
            detect_content_format(&[0x66, 0x4C, 0x61, 0x43]),
            NftContentFormat::Flac
        );
    }

    // ── Video ───────────────────────────────────────────────────────

    #[test]
    fn test_detect_mp4() {
        let mut data = vec![0x00, 0x00, 0x00, 0x18]; // box size
        data.extend_from_slice(b"ftypisom"); // ftyp + brand (not avif)
        assert_eq!(detect_content_format(&data), NftContentFormat::Mp4);
    }

    // ── Archives ────────────────────────────────────────────────────

    #[test]
    fn test_detect_zip() {
        assert_eq!(
            detect_content_format(&[0x50, 0x4B, 0x03, 0x04]),
            NftContentFormat::Zip
        );
    }

    #[test]
    fn test_detect_gzip() {
        assert_eq!(
            detect_content_format(&[0x1F, 0x8B, 0x08, 0x00]),
            NftContentFormat::Gzip
        );
    }

    // ── WASM ────────────────────────────────────────────────────────

    #[test]
    fn test_detect_wasm() {
        assert_eq!(
            detect_content_format(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]),
            NftContentFormat::Wasm
        );
    }

    // ── Generic ─────────────────────────────────────────────────────

    #[test]
    fn test_detect_text() {
        assert_eq!(
            detect_content_format(b"Hello, DOLI!"),
            NftContentFormat::Text
        );
    }

    #[test]
    fn test_detect_binary() {
        assert_eq!(
            detect_content_format(&[0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD]),
            NftContentFormat::Binary
        );
    }

    #[test]
    fn test_detect_empty() {
        assert_eq!(detect_content_format(&[]), NftContentFormat::Binary);
    }

    #[test]
    fn test_detect_hash_reference() {
        assert_eq!(
            detect_content_format(&[0xAA; 32]),
            NftContentFormat::HashReference
        );
    }

    // ── Full CryptoPunk simulation ──────────────────────────────────

    #[test]
    fn test_cryptopunk_24x24_detected() {
        let mut data = vec![0x01, 24, 24, 16];
        for i in 0..16u8 {
            data.extend_from_slice(&[i.wrapping_mul(16), i.wrapping_mul(8), i.wrapping_mul(4)]);
        }
        data.extend_from_slice(&[0u8; 576]);
        match detect_content_format(&data) {
            NftContentFormat::DoliPixelArt {
                width,
                height,
                palette_colors,
            } => {
                assert_eq!(width, 24);
                assert_eq!(height, 24);
                assert_eq!(palette_colors, 16);
            }
            other => panic!("Expected DoliPixelArt, got {:?}", other),
        }
    }

    // ── Helper functions ────────────────────────────────────────────

    #[test]
    fn test_format_name() {
        assert_eq!(format_name(&NftContentFormat::Png), "PNG image");
        assert_eq!(format_name(&NftContentFormat::Pdf), "PDF document");
        assert_eq!(format_name(&NftContentFormat::Wasm), "WebAssembly");
        assert_eq!(format_name(&NftContentFormat::Mp3), "MP3 audio");
    }

    #[test]
    fn test_format_mime() {
        assert_eq!(format_mime(&NftContentFormat::Png), "image/png");
        assert_eq!(format_mime(&NftContentFormat::Pdf), "application/pdf");
        assert_eq!(format_mime(&NftContentFormat::Json), "application/json");
        assert_eq!(format_mime(&NftContentFormat::Mp4), "video/mp4");
        assert_eq!(format_mime(&NftContentFormat::Wasm), "application/wasm");
    }

    // ── RLE decode tests ─────────────────────────────────────────────

    #[test]
    fn test_rle_decode_simple() {
        // RLE: 3 of color 0, 2 of color 1, 1 of color 5
        let rle = [3u8, 0, 2, 1, 1, 5];
        let decoded: Vec<u8> = rle
            .chunks(2)
            .flat_map(|pair| std::iter::repeat_n(pair[1], pair[0] as usize))
            .collect();
        assert_eq!(decoded, vec![0, 0, 0, 1, 1, 5]);
    }

    #[test]
    fn test_rle_decode_full_24x24_row() {
        // 24 pixels: 12 black + 12 white
        let rle = [12u8, 0, 12, 1];
        let decoded: Vec<u8> = rle
            .chunks(2)
            .flat_map(|pair| std::iter::repeat_n(pair[1], pair[0] as usize))
            .collect();
        assert_eq!(decoded.len(), 24);
        assert!(decoded[..12].iter().all(|&p| p == 0));
        assert!(decoded[12..].iter().all(|&p| p == 1));
    }

    #[test]
    fn test_rle_decode_single_pixel_runs() {
        // Each pixel is its own run
        let rle = [1u8, 0, 1, 1, 1, 2, 1, 3];
        let decoded: Vec<u8> = rle
            .chunks(2)
            .flat_map(|pair| std::iter::repeat_n(pair[1], pair[0] as usize))
            .collect();
        assert_eq!(decoded, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_rle_empty() {
        let rle: [u8; 0] = [];
        let decoded: Vec<u8> = rle
            .chunks(2)
            .flat_map(|pair| std::iter::repeat_n(pair[1], pair[0] as usize))
            .collect();
        assert!(decoded.is_empty());
    }
}
