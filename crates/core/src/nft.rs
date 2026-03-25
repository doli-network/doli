//! NFT content format detection via magic bytes.

/// Detected content format for an NFT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NftContentFormat {
    /// PNG image (magic: 89 50 4E 47)
    Png,
    /// JPEG image (magic: FF D8 FF)
    Jpeg,
    /// SVG image (starts with "<svg" or "<?xml")
    Svg,
    /// GIF image (magic: 47 49 46 38)
    Gif,
    /// WebP image (magic: 52 49 46 46 ... 57 45 42 50)
    WebP,
    /// DOLI pixel art (magic: 01 + width + height + palette_size)
    DoliPixelArt {
        width: u8,
        height: u8,
        palette_colors: u8,
    },
    /// Plain text (UTF-8 valid)
    Text,
    /// Raw binary data (unknown format)
    Binary,
    /// BLAKE3 hash reference (exactly 32 bytes)
    HashReference,
}

/// Detect the content format from the raw bytes.
pub fn detect_content_format(data: &[u8]) -> NftContentFormat {
    if data.is_empty() {
        return NftContentFormat::Binary;
    }

    // Exactly 32 bytes = likely a hash reference
    if data.len() == 32 {
        return NftContentFormat::HashReference;
    }

    // Check magic bytes
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

    // DOLI pixel art: version=1, then width, height, palette_size
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

    // Check if valid UTF-8 text
    if let Ok(text) = std::str::from_utf8(data) {
        let trimmed = text.trim();
        if trimmed.starts_with("<svg") || trimmed.starts_with("<?xml") {
            return NftContentFormat::Svg;
        }
        return NftContentFormat::Text;
    }

    NftContentFormat::Binary
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_detect_svg() {
        let data = b"<svg xmlns=\"http://www.w3.org/2000/svg\">";
        assert_eq!(detect_content_format(data), NftContentFormat::Svg);
    }

    #[test]
    fn test_detect_doli_pixel_art() {
        // version=1, width=24, height=24, 16 colors
        let data = [0x01, 24, 24, 16, 0xFF, 0x00]; // header + some pixel data
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
    fn test_detect_hash_reference() {
        let data = [0xAA; 32]; // exactly 32 bytes
        assert_eq!(
            detect_content_format(&data),
            NftContentFormat::HashReference
        );
    }

    #[test]
    fn test_detect_text() {
        let data = b"Hello, DOLI!";
        assert_eq!(detect_content_format(data), NftContentFormat::Text);
    }

    #[test]
    fn test_detect_binary() {
        let data = [0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
        assert_eq!(detect_content_format(&data), NftContentFormat::Binary);
    }

    #[test]
    fn test_detect_empty() {
        assert_eq!(detect_content_format(&[]), NftContentFormat::Binary);
    }

    #[test]
    fn test_cryptopunk_24x24_detected() {
        // Simulate a CryptoPunk: version=1, 24x24, 16 colors, then palette + pixel data
        let mut data = vec![0x01, 24, 24, 16]; // header
                                               // Palette: 16 colors x 3 bytes RGB
        for i in 0..16u8 {
            data.extend_from_slice(&[i.wrapping_mul(16), i.wrapping_mul(8), i.wrapping_mul(4)]);
        }
        // Pixels: 576 bytes
        data.extend_from_slice(&vec![0u8; 576]);

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
}
