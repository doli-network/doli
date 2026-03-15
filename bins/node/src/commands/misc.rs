use anyhow::{anyhow, Result};

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
            // Strip 'v' prefix for internal version string
            let version_str = version.strip_prefix('v').unwrap_or(&version);

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

            // Sign the release
            let sig = updater::sign_release_hash(&keypair, version_str, &binary_sha256);

            // Output JSON to stdout (stderr had the progress messages)
            println!("{}", serde_json::to_string_pretty(&sig)?);
        }
    }
    Ok(())
}

pub(crate) async fn handle_upgrade_command(version: Option<String>, yes: bool) -> Result<()> {
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

    // Verify hash
    println!("Verifying checksum...");
    updater::verify_hash(&tarball, &release_info.expected_hash)
        .map_err(|e| anyhow!("Checksum verification failed: {}", e))?;

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
