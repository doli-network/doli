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
        MaintainerSignature, INITIAL_MAINTAINER_COUNT, MAINTAINER_THRESHOLD, MAX_MAINTAINERS,
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
                                "║  No producers registered yet.                              ║"
                            );
                        }
                    }
                    Err(_) => {
                        println!("║  Could not load producer data.                              ║");
                    }
                }
            } else {
                println!("║  No producer data found.                                      ║");
            }

            // Always show bootstrap keys as reference.
            //
            // They are NOT a fallback (INC-I-172 F1): they are the trust root ONLY for a
            // node that has never established an on-chain maintainer set. Once a set
            // exists, that set is authoritative, and a set that exists and is empty or
            // sub-threshold FAILS CLOSED — verification refuses rather than coming back
            // here. The old "fallback before sync" wording described the deleted
            // behaviour and is what made the compiled keys look permanently reachable.
            println!("║                                                                  ║");
            let net_label = match network {
                Network::Mainnet => "mainnet",
                Network::Testnet => "testnet",
                Network::Devnet => "devnet",
            };
            println!(
                "║  Bootstrap keys ({}, used only until an on-chain set exists): ║",
                net_label
            );
            println!("║  They are NOT a fallback: an on-chain set that is empty or   ║");
            println!("║  sub-threshold fails closed and does NOT return here.        ║");
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
            height,
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

            // The removal reason travels in the tx PAYLOAD, not in the signed
            // bytes (the signed message binds domain, genesis, action, target
            // and expiry only). It is echoed below so the operator carries it
            // to `submitMaintainerChange` unchanged.
            let reason = reason.unwrap_or_default();

            // Sign the proposal. INC-I-176 M4: the bytes come from
            // `maintainer_auth_message`, which reads the SAME message owner and
            // the SAME gate the apply path reads. `change_data.signing_message`
            // is the legacy-only constructor and must not be used here — above
            // #22 it produces a signature the chain rejects.
            let message = maintainer_auth_message(network, false, &target_pubkey, height);
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
            print_authorization(network, false, &target_pubkey, height, &sig);
            println!("reason:        {}", reason);
        }

        MaintainerCommands::Add {
            target,
            key,
            height,
        } => {
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

            // Sign the proposal. INC-I-176 M4: see the Remove arm — same owner,
            // same gate, same reason `change_data.signing_message` is not used.
            let message = maintainer_auth_message(network, true, &target_pubkey, height);
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
            print_authorization(network, true, &target_pubkey, height, &sig);
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

/// Print the authorization in full, machine-readable form.
///
/// The boxed summary above truncates every value to 16 hex characters, which is
/// enough to eyeball and not enough to submit. `submitMaintainerChange` needs
/// the whole signer pubkey and the whole signature, so they are printed here
/// verbatim.
///
/// The PREIMAGE is printed whenever the bound message is in force. That is the
/// AUDIT-P0-011 lesson stated as output: a signer shown only a 32-byte digest
/// cannot tell a maintainer authorization from a release approval, so the
/// operator is shown the domain tag, the genesis hash, the action byte, the
/// target and the expiry that went INTO the digest. Below the gate the signed
/// bytes are the legacy ASCII message and are already self-describing.
fn print_authorization(
    network: Network,
    is_add: bool,
    target: &PublicKey,
    height: u64,
    sig: &doli_core::maintainer::MaintainerSignature,
) {
    let gate = network.params().inc_i_176_auth_binding_activation_height;
    println!();
    println!("--- authorization (submitMaintainerChange) ---");
    println!("network:       {:?}", network);
    println!("action:        {}", if is_add { "add" } else { "remove" });
    println!("target_pubkey: {}", target.to_hex());
    println!("height:        {}  (#22 gate = {})", height, gate);
    if height >= gate {
        let genesis_hash = doli_core::consensus::ConsensusParams::for_network(network).genesis_hash;
        let preimage = doli_core::maintainer::signing_message_preimage(
            genesis_hash.as_bytes(),
            is_add,
            target,
            doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );
        println!("message:       BOUND (domain-tagged, genesis-bound)");
        println!("preimage:      {}", hex::encode(&preimage));
    } else {
        println!("message:       LEGACY (below #22)");
    }
    println!("pubkey:        {}", sig.pubkey.to_hex());
    println!("signature:     {}", sig.signature.to_hex());
    println!("----------------------------------------------");
}

/// The bytes a maintainer signs to authorize an `AddMaintainer` /
/// `RemoveMaintainer` change on `network`, for a change applied at `height`.
///
/// INC-I-176 M4 (REQ-176-030, "exactly ONE implementation of the signed
/// message"). The apply path
/// (`bins/node/src/node/apply_block/governance.rs:97,157`) derives the message
/// from `doli_core::maintainer::signing_message_at` fed by this chain's genesis
/// hash and `NetworkParams::inc_i_176_auth_binding_activation_height` (#22).
/// This signer MUST derive it from the same owner and the same two inputs, or
/// the signature it produces is not the signature the chain verifies.
///
/// `height` is the height the change is authorized FOR. It selects WHICH bytes
/// (legacy below #22, genesis-bound at or above it) and nothing else, so it is
/// only load-bearing across the gate boundary. It is an explicit operator input
/// rather than a read of local chain state because the running node holds the
/// RocksDB lock, and a silently defaulted height would silently produce an
/// invalid signature.
///
/// `valid_before` is the M2 sentinel `MAINTAINER_AUTH_VALID_BEFORE_UNSET`,
/// matching the apply path verbatim. The payload carries no expiry field until
/// INC-I-176 M2.5, so this signer must not invent one.
pub(crate) fn maintainer_auth_message(
    network: Network,
    is_add: bool,
    target: &PublicKey,
    height: u64,
) -> Vec<u8> {
    let genesis_hash = doli_core::consensus::ConsensusParams::for_network(network).genesis_hash;
    let auth_binding_activation_height = network.params().inc_i_176_auth_binding_activation_height;
    doli_core::maintainer::signing_message_at(
        genesis_hash.as_bytes(),
        is_add,
        target,
        doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        height,
        auth_binding_activation_height,
    )
}

#[cfg(test)]
mod tests {
    use super::maintainer_auth_message;
    use crypto::PublicKey;
    use doli_core::consensus::ConsensusParams;
    use doli_core::maintainer::{
        signing_message, signing_message_legacy, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
    };
    use doli_core::Network;

    fn target() -> PublicKey {
        PublicKey::from_hex("d13ae33891d55930e644935cdfd510922d673ec227425c14a98ca3744a1ec670")
            .expect("fixed 32-byte test target")
    }

    fn gate(network: Network) -> u64 {
        network.params().inc_i_176_auth_binding_activation_height
    }

    fn genesis(network: Network) -> crypto::Hash {
        ConsensusParams::for_network(network).genesis_hash
    }

    /// REQ-176-030 — at and above #22 the signer must emit the genesis-bound
    /// message the apply path verifies, NOT the legacy one.
    #[test]
    fn at_and_above_gate_the_signer_emits_the_bound_message() {
        for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
            let g = gate(network);
            if g == u64::MAX {
                continue; // frozen gate: no reachable height at or above it
            }
            for is_add in [true, false] {
                for height in [g, g + 1, g + 100_000] {
                    let expected = signing_message(
                        genesis(network).as_bytes(),
                        is_add,
                        &target(),
                        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                    );
                    assert_eq!(
                        maintainer_auth_message(network, is_add, &target(), height),
                        expected,
                        "{:?} is_add={} height={} must sign the bound message",
                        network,
                        is_add,
                        height
                    );
                    assert_ne!(
                        maintainer_auth_message(network, is_add, &target(), height),
                        signing_message_legacy(is_add, &target()),
                        "{:?} height={} must NOT sign the legacy message above #22",
                        network,
                        height
                    );
                }
            }
        }
    }

    /// Below #22 the legacy message is frozen consensus history and stays in
    /// force. Devnet reaches this arm on a live chain: #21 = 0 and #22 = 20, so
    /// a governance tx IS mineable below the binding gate there.
    #[test]
    fn below_gate_the_signer_stays_on_the_legacy_message() {
        for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
            let g = gate(network);
            if g == 0 {
                continue; // no height below the gate exists
            }
            let height = g - 1;
            for is_add in [true, false] {
                assert_eq!(
                    maintainer_auth_message(network, is_add, &target(), height),
                    signing_message_legacy(is_add, &target()),
                    "{:?} is_add={} height={} must stay legacy below #22",
                    network,
                    is_add,
                    height
                );
            }
        }
    }

    /// AUDIT-P1-016 — a signature produced for one network must not authorize
    /// the same change on another. Above the gate the messages must differ
    /// because the genesis hash is inside the signed bytes.
    #[test]
    fn above_gate_the_message_is_bound_to_this_chain() {
        let h = gate(Network::Testnet).max(gate(Network::Devnet)) + 1;
        assert_ne!(
            genesis(Network::Testnet),
            genesis(Network::Devnet),
            "precondition: the two chains must have distinct genesis hashes"
        );
        assert_ne!(
            maintainer_auth_message(Network::Testnet, true, &target(), h),
            maintainer_auth_message(Network::Devnet, true, &target(), h),
            "the signed bytes must differ per chain above #22"
        );
    }
}
