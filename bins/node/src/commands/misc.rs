use std::path::Path;

use anyhow::{anyhow, Result};
use doli_core::Network;

use crate::cli::{DevnetCommands, ReleaseCommands};
use crate::keys::load_producer_key;
use crate::updater;

pub(crate) async fn handle_devnet_command(action: DevnetCommands) -> Result<()> {
    match action {
        DevnetCommands::Init { nodes } => {
            crate::devnet::init(nodes)?;
        }
        DevnetCommands::Start => {
            crate::devnet::start().await?;
        }
        DevnetCommands::Stop => {
            crate::devnet::stop().await?;
        }
        DevnetCommands::Status => {
            crate::devnet::status().await?;
        }
        DevnetCommands::Clean { keep_keys } => {
            crate::devnet::clean(keep_keys)?;
        }
        DevnetCommands::AddProducer {
            count,
            bonds,
            fund_amount,
        } => {
            crate::devnet::add_producer(count, bonds, fund_amount).await?;
        }
    }
    Ok(())
}

pub(crate) async fn handle_release_command(action: ReleaseCommands) -> Result<()> {
    match action {
        ReleaseCommands::Sign { key, version, hash } => {
            // INC-I-172 M2, AUDIT-P0-011. Validate BEFORE the key is loaded, before the
            // network is touched and — the part that matters — before the signing
            // message is interpolated. A release signature is raw bytes over
            // `"{version}:{hash}"`, and `"add:{pubkey_hex}"` / `"remove:{pubkey_hex}"` /
            // `"activate:{version}:{epoch}"` are the same interpolation with no domain
            // tag, so free-form arguments make one signing command able to mint a
            // governance authorization for a completely different intent. `version_str`
            // is the bare form this used to compute inline; the strip now happens inside
            // the validator so `--version vadd` cannot slip past it.
            let version_str = updater::validate_release_version(&version)?;

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let pubkey_hex = keypair.public_key().to_hex();
            eprintln!(
                "Signing release {} with key {}...{}",
                version,
                &pubkey_hex[..16],
                &pubkey_hex[pubkey_hex.len() - 8..]
            );

            // Get the binary hash
            let binary_sha256 = match hash {
                Some(h) => h,
                None => {
                    // Fetch CHECKSUMS.txt from GitHub release
                    let checksums_url =
                        format!("{}/{}/CHECKSUMS.txt", updater::GITHUB_RELEASES_URL, version);
                    eprintln!("Fetching checksums from {}...", checksums_url);

                    let client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(30))
                        .user_agent("doli-node")
                        .build()?;

                    let response = client
                        .get(&checksums_url)
                        .send()
                        .await
                        .map_err(|e| anyhow!("Failed to fetch CHECKSUMS.txt: {}", e))?;

                    if !response.status().is_success() {
                        return Err(anyhow!(
                            "Failed to fetch CHECKSUMS.txt: HTTP {}. \
                             Pass --hash manually if the release is not yet published.",
                            response.status()
                        ));
                    }

                    let body = response
                        .text()
                        .await
                        .map_err(|e| anyhow!("Failed to read CHECKSUMS.txt: {}", e))?;

                    // Parse CHECKSUMS.txt — format: "<hash>  <filename>"
                    // Look for the linux musl binary (canonical platform for signing)
                    let musl_hash = body
                        .lines()
                        .find(|line| line.contains("x86_64-unknown-linux-musl"))
                        .and_then(|line| line.split_whitespace().next())
                        .map(|h| h.to_string());

                    match musl_hash {
                        Some(h) => {
                            eprintln!("Using linux-x64-musl hash: {}...", &h[..16.min(h.len())]);
                            h
                        }
                        None => {
                            return Err(anyhow!(
                                "Could not find linux-x64-musl hash in CHECKSUMS.txt.\n\
                                 Contents:\n{}\n\n\
                                 Pass --hash manually to sign a specific hash.",
                                body
                            ));
                        }
                    }
                }
            };

            // The other operand of the same message. `--hash` is operator-supplied, and
            // the CHECKSUMS.txt branch is parsed from a file fetched over the network,
            // so BOTH sources are validated here rather than only the flag: an epoch
            // number (`--hash 1000`, the ProtocolActivation shape) and a malformed
            // checksums line are refused identically. A 64-hex Ed25519 key still passes —
            // it is indistinguishable from a digest — which is exactly why the version
            // check above is the one that closes the AddMaintainer leg.
            let binary_sha256 = updater::validate_release_hash(&binary_sha256)?;

            // Sign the release
            let sig = updater::sign_release_hash(&keypair, &version_str, &binary_sha256);

            // Output JSON to stdout (stderr had the progress messages)
            println!("{}", serde_json::to_string_pretty(&sig)?);
        }
    }
    Ok(())
}

/// `doli-node upgrade` — the fourth install path.
///
/// `network` selects the trust root the maintainer signatures are checked against, for
/// the same reason `doli upgrade` takes one: pinning it to a constant would check a
/// testnet or devnet operator's release against the mainnet keys (FM-12 cross-network
/// replay).
///
/// `data_dir` is what makes the root the ON-CHAIN one (INC-I-172 F3). This binary runs
/// on the host that holds `maintainer_state.bin`; resolving `TrustRoot::bootstrap` here
/// would leave the leaked compiled constants authoritative on every producer host
/// through this single command.
pub(crate) async fn handle_upgrade_command(
    version: Option<String>,
    yes: bool,
    network: Network,
    data_dir: &Path,
) -> Result<()> {
    println!("Checking for updates...");

    let release_info = updater::fetch_github_release(version.as_deref())
        .await
        .map_err(|e| anyhow!("Failed to fetch release: {}", e))?;

    let current = updater::current_version();
    if !updater::is_newer_version(&release_info.version, current) {
        println!("Already up to date (v{})", current);
        return Ok(());
    }

    println!();
    println!("  Current version:  v{}", current);
    println!("  Available:        v{}", release_info.version);
    if !release_info.changelog.is_empty() {
        println!();
        for line in release_info.changelog.lines().take(10) {
            println!("  {}", line);
        }
    }
    println!();

    if !yes {
        print!("Proceed with upgrade? [y/N] ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Upgrade cancelled.");
            return Ok(());
        }
    }

    // Download tarball
    println!("Downloading v{}...", release_info.version);
    let tarball = updater::download_from_url(&release_info.tarball_url)
        .await
        .map_err(|e| anyhow!("Download failed: {}", e))?;

    // Maintainer signatures GATE the install (INC-I-172 F6, QA ISSUE-001). A bare
    // tarball checksum is NOT an independent control: `expected_hash` is parsed from
    // CHECKSUMS.txt fetched from the same GitHub release as the tarball, so an origin
    // that can serve a malicious binary can serve its hash too. Only the maintainer
    // keys are independent of the origin. Every failure below aborts BEFORE anything
    // is extracted, backed up or written, and the shape matches
    // `bins/cli/src/cmd_upgrade.rs` so the two operator paths cannot drift:
    //   - SIGNATURES.json unreachable — "I could not check" is not "it is fine"; a
    //     network failure is indistinguishable from an attacker withholding the file;
    //   - SIGNATURES.json absent — an unsigned release is not a verified release;
    //   - below threshold, or any verification error — refuse.
    println!("Checking maintainer signatures...");
    let signatures = updater::download_signatures_json(&release_info.version)
        .await
        .map_err(|e| {
            anyhow!(
                "Could not retrieve SIGNATURES.json for v{}: {}. Refusing to install an \
                 unverified release.",
                release_info.version,
                e
            )
        })?;
    let sf = signatures.ok_or_else(|| {
        anyhow!(
            "Release v{} has no SIGNATURES.json. An unsigned release is not a verified \
             release; refusing to install.",
            release_info.version
        )
    })?;
    // INC-I-172 F3: the ON-CHAIN maintainer set held on this host, not the compiled
    // bootstrap keys. A load error is fatal here for the same reason it is fatal at
    // startup — guessing at the trust root is the one thing that must not happen.
    let root = updater::command_trust_root(data_dir, network)?;
    // `verify_release_artifact` binds the signatures to the artifact: sf.version to the
    // release tag, sf.checksums_sha256 to the bytes of the CHECKSUMS.txt actually
    // fetched, and the tarball to the per-platform hash parsed from THOSE bytes.
    // Checking signatures over `sf`'s own self-reported pair (what this code used to
    // do) accepts a verbatim replay of any past genuine SIGNATURES.json while
    // installing an arbitrary binary — INC-I-172 F1.
    let distinct_signers = updater::verify_release_artifact(&release_info, &tarball, &sf, &root)
        .map_err(|e| {
            anyhow!(
                "Maintainer verification FAILED for v{} on {}: {}. Refusing to install.",
                release_info.version,
                network,
                e
            )
        })?;
    println!(
        "Verified: {} distinct maintainer signature(s) bound to this v{} tarball \
         (threshold {}, {} trust root)",
        distinct_signers,
        release_info.version,
        root.threshold(),
        root.provenance()
    );

    // Extract binary
    println!("Extracting binary...");
    let binary = updater::extract_binary_from_tarball(&tarball)
        .map_err(|e| anyhow!("Extraction failed: {}", e))?;

    // Backup current
    println!("Backing up current binary...");
    updater::backup_current()
        .await
        .map_err(|e| anyhow!("Backup failed: {}", e))?;

    // Install
    println!("Installing v{}...", release_info.version);
    let target =
        updater::current_binary_path().map_err(|e| anyhow!("Failed to get binary path: {}", e))?;
    updater::install_binary(&binary, &target)
        .await
        .map_err(|e| anyhow!("Installation failed: {}", e))?;

    println!();
    println!(
        "Upgrade complete: v{} -> v{}",
        current, release_info.version
    );
    println!("Restart your node to use the new version.");

    Ok(())
}
