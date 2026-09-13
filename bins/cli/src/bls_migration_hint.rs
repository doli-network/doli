//! INC-I-217 — the proactive-migration hint `doli info` prints for a pre-version-3
//! wallet, whose BLS attestation key the 24-word phrase cannot reproduce.

use std::str::FromStr;

use doli_core::{Network, NetworkParams};

/// Lines appended to the legacy-wallet Backup verdict in `cmd_info`.
///
/// `network` is the raw `--network` string; `None` is the clap default, mainnet.
/// An unparsable value also falls back to mainnet, matching the CLI's other
/// network defaults (`common::prefix_for_network`).
///
/// `u64::MAX` means the rotation rule is not live on this network, so no height
/// may be shown: rendered as a decimal it would read as a height no chain reaches.
pub(crate) fn migration_hint_lines(network: Option<&str>) -> Vec<String> {
    let params = NetworkParams::defaults(
        network
            .and_then(|n| Network::from_str(n).ok())
            .unwrap_or(Network::Mainnet),
    );
    let when = match params.bls_key_rotation_activation_height {
        u64::MAX => "not yet activated on this network".to_string(),
        height => format!("from height {}", height),
    };
    vec![
        "  RECOMMENDED — close this gap while you still have the file:".to_string(),
        "    1. doli -w <new-wallet.json> restore     (same 24 words, NEW file)".to_string(),
        format!("    2. doli -w <new-wallet.json> producer rotate-bls   ({when})"),
        "    3. at the height it prints, point the node at the new file and restart once."
            .to_string(),
        "  Full procedure: docs/bls-key-recovery.md".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_unparsable_networks_fall_back_to_mainnet() {
        let expected = migration_hint_lines(Some("mainnet"));
        assert_eq!(migration_hint_lines(None), expected);
        assert_eq!(migration_hint_lines(Some("not-a-network")), expected);
    }

    #[test]
    fn the_hint_stays_within_the_six_line_budget() {
        for network in [None, Some("mainnet"), Some("testnet"), Some("devnet")] {
            assert!(migration_hint_lines(network).len() <= 6);
        }
    }
}
