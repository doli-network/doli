use std::path::Path;

use anyhow::Result;
use crypto::PublicKey;
use doli_core::Network;

use crate::cli::MaintainerCommands;
use crate::keys::load_producer_key;
use crate::updater;

pub(crate) async fn handle_maintainer_command(
    action: MaintainerCommands,
    data_dir: &Path,
    network: Network,
) -> Result<()> {
    use doli_core::maintainer::{
        MaintainerChangeData, MaintainerSignature, INITIAL_MAINTAINER_COUNT, MAINTAINER_THRESHOLD,
        MAX_MAINTAINERS,
    };

    match action {
        MaintainerCommands::List => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    MAINTAINER SET                                ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");
            println!("║                                                                  ║");
            println!(
                "║  Threshold: {} of {} signatures required                        ║",
                MAINTAINER_THRESHOLD, MAX_MAINTAINERS
            );
            println!("║                                                                  ║");

            // Try to derive on-chain maintainers from producer set
            let producers_path = data_dir.join("producers.bin");
            if producers_path.exists() {
                match storage::ProducerSet::load(&producers_path) {
                    Ok(set) => {
                        let mut sorted: Vec<_> = set.all_producers().into_iter().cloned().collect();
                        sorted.sort_by_key(|p| p.registered_at);
                        let maintainers: Vec<_> =
                            sorted.into_iter().take(INITIAL_MAINTAINER_COUNT).collect();

                        if !maintainers.is_empty() {
                            println!(
                                "║  On-chain maintainers (from first {} registrations):      ║",
                                INITIAL_MAINTAINER_COUNT
                            );
                            println!(
                                "║  These keys are used for release signature verification.   ║"
                            );
                            println!("║                                                                  ║");
                            for (i, p) in maintainers.iter().enumerate() {
                                let hex = p.public_key.to_hex();
                                println!(
                                    "║  {}. {}...{} (reg height: {})      ║",
                                    i + 1,
                                    &hex[..16],
                                    &hex[hex.len() - 8..],
                                    p.registered_at
                                );
                            }
                        } else {
                            println!(
                                "║  No producers registered yet. Using bootstrap keys.        ║"
                            );
                        }
                    }
                    Err(_) => {
                        println!("║  Could not load producer data. Using bootstrap keys.        ║");
                    }
                }
            } else {
                println!("║  No producer data found. Using bootstrap keys.                ║");
            }

            // Always show bootstrap keys as reference
            println!("║                                                                  ║");
            let net_label = match network {
                Network::Mainnet => "mainnet",
                Network::Testnet => "testnet",
                Network::Devnet => "devnet",
            };
            println!(
                "║  Bootstrap keys ({}, fallback before sync):              ║",
                net_label
            );
            for (i, key) in updater::bootstrap_maintainer_keys(network)
                .iter()
                .enumerate()
            {
                println!(
                    "║  {}. {}...{}                          ║",
                    i + 1,
                    &key[..16],
                    &key[key.len() - 8..]
                );
            }
            println!("║                                                                  ║");
            println!("║  Use RPC 'getMaintainerSet' for live on-chain status.          ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Remove {
            target,
            key,
            reason,
        } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    PROPOSE MAINTAINER REMOVAL                    ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = *keypair.public_key();

            // Parse target public key
            let target_pubkey = match PublicKey::from_hex(&target) {
                Ok(pk) => pk,
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid target public key: {}                               ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    return Ok(());
                }
            };

            // Create the change data
            let change_data = MaintainerChangeData::with_reason(
                target_pubkey,
                vec![], // Signatures will be collected separately
                reason.unwrap_or_default(),
            );

            // Sign the proposal
            let message = change_data.signing_message(false); // false = removal
            let signature = crypto::signature::sign(&message, keypair.private_key());

            let sig = MaintainerSignature {
                pubkey: signer_pubkey,
                signature,
            };

            println!("║                                                                  ║");
            println!("║  Proposal created:                                               ║");
            println!(
                "║  Target:  {}...                                                  ║",
                &target[..16.min(target.len())]
            );
            println!(
                "║  Signer:  {}...                                                  ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!(
                "║  Signature: {}...                                                ║",
                &sig.signature.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Next steps:                                                     ║");
            println!("║  1. Share proposal with other maintainers                        ║");
            println!("║  2. Collect 3/5 signatures                                       ║");
            println!("║  3. Submit via RPC 'submitMaintainerChange'                      ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Add { target, key } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    PROPOSE MAINTAINER ADDITION                   ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = *keypair.public_key();

            // Parse target public key
            let target_pubkey = match PublicKey::from_hex(&target) {
                Ok(pk) => pk,
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid target public key: {}                               ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    return Ok(());
                }
            };

            // Create the change data
            let change_data = MaintainerChangeData::new(target_pubkey, vec![]);

            // Sign the proposal
            let message = change_data.signing_message(true); // true = addition
            let signature = crypto::signature::sign(&message, keypair.private_key());

            let sig = MaintainerSignature {
                pubkey: signer_pubkey,
                signature,
            };

            println!("║                                                                  ║");
            println!("║  Proposal created:                                               ║");
            println!(
                "║  Target:  {}...                                                  ║",
                &target[..16.min(target.len())]
            );
            println!(
                "║  Signer:  {}...                                                  ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!(
                "║  Signature: {}...                                                ║",
                &sig.signature.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Next steps:                                                     ║");
            println!("║  1. Share proposal with other maintainers                        ║");
            println!("║  2. Collect 3/5 signatures                                       ║");
            println!("║  3. Submit via RPC 'submitMaintainerChange'                      ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Sign { proposal_id, key } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    SIGN MAINTAINER PROPOSAL                      ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = *keypair.public_key();

            println!("║                                                                  ║");
            println!(
                "║  Proposal ID: {}                                                 ║",
                proposal_id
            );
            println!(
                "║  Signer:      {}...                                              ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Note: Proposal signing requires the full proposal data.         ║");
            println!("║  Use RPC to fetch proposal and sign interactively.               ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Verify { pubkey } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    VERIFY MAINTAINER STATUS                      ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Parse public key
            match PublicKey::from_hex(&pubkey) {
                Ok(_pk) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Public key: {}...                                               ║",
                        &pubkey[..16.min(pubkey.len())]
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Note: Full verification requires blockchain access.             ║"
                    );
                    println!(
                        "║  Use RPC 'getMaintainerSet' to check current maintainers.        ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid public key: {}                                       ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
            }
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
    }

    Ok(())
}
