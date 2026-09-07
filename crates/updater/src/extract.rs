//! Tarball entry extraction for the install paths.

use tracing::info;

use crate::types::{Result, UpdateError};

/// Extract a named binary from a .tar.gz tarball
///
/// CI produces tarballs like `doli-node-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
/// containing entries like `doli-node-v0.1.0-x86_64-unknown-linux-gnu/doli-node`
/// and `doli-node-v0.1.0-x86_64-unknown-linux-gnu/doli`.
/// This function decompresses and finds the entry matching `name`.
pub fn extract_named_binary_from_tarball(tarball: &[u8], name: &str) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let decoder = GzDecoder::new(tarball);
    let mut archive = Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|e| UpdateError::InstallFailed(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;

        if path.file_name().map(|n| n == name).unwrap_or(false) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
            info!("Extracted {} binary ({} bytes)", name, bytes.len());
            return Ok(bytes);
        }
    }

    Err(UpdateError::InstallFailed(format!(
        "{} binary not found in tarball",
        name
    )))
}

/// Extract the doli-node binary from a .tar.gz tarball
///
/// Convenience wrapper around `extract_named_binary_from_tarball` for "doli-node".
pub fn extract_binary_from_tarball(tarball: &[u8]) -> Result<Vec<u8>> {
    extract_named_binary_from_tarball(tarball, "doli-node")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tarball_contains_both_binaries() {
        // Build a minimal tarball with both doli-node and doli entries
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);

            let node_content = b"fake-doli-node-binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(node_content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    "doli-v1.0.0-x86_64-unknown-linux-gnu/doli-node",
                    &node_content[..],
                )
                .unwrap();

            let cli_content = b"fake-doli-cli-binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(cli_content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    "doli-v1.0.0-x86_64-unknown-linux-gnu/doli",
                    &cli_content[..],
                )
                .unwrap();

            builder.finish().unwrap();
        }
        let tarball = encoder.finish().unwrap();

        // Both binaries must be extractable
        let node = extract_named_binary_from_tarball(&tarball, "doli-node");
        assert!(node.is_ok(), "doli-node must be in tarball");
        assert_eq!(node.unwrap(), b"fake-doli-node-binary");

        let cli = extract_named_binary_from_tarball(&tarball, "doli");
        assert!(cli.is_ok(), "doli CLI must be in tarball");
        assert_eq!(cli.unwrap(), b"fake-doli-cli-binary");
    }
}
