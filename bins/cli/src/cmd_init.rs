use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::common::address_prefix;
use crate::wallet::Wallet;

/// Check if the data directory is writable by the current process.
fn dir_is_writable(dir: &Path) -> bool {
    if !dir.exists() {
        return dir.parent().map(dir_is_writable).unwrap_or(false);
    }
    let test_path = dir.join(".doli-write-test");
    match std::fs::write(&test_path, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_path);
            true
        }
        Err(_) => false,
    }
}

/// On Linux: check if user is in 'doli' group (assigned but not yet active in this session).
#[cfg(target_os = "linux")]
fn user_in_doli_group_but_inactive() -> bool {
    // Check /etc/group for membership (assigned) vs `id -Gn` (active in session)
    let real_user = std::env::var("USER").unwrap_or_default();
    if real_user.is_empty() {
        return false;
    }
    // Check if assigned to doli group
    let assigned = std::process::Command::new("id")
        .args(["-Gn", &real_user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.split_whitespace().any(|g| g == "doli"))
        .unwrap_or(false);
    if !assigned {
        return false;
    }
    // Check if active in current session (groups of current process)
    let active = std::process::Command::new("id")
        .arg("-Gn")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.split_whitespace().any(|g| g == "doli"))
        .unwrap_or(false);
    !active
}

/// Re-exec `doli init` under `sg doli` to activate group membership.
#[cfg(target_os = "linux")]
fn reexec_with_doli_group(force: bool, non_producer: bool) -> Result<()> {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("doli"));
    let mut cmd = format!("{} init", exe.display());
    if force {
        cmd.push_str(" --force");
    }
    if non_producer {
        cmd.push_str(" --non-producer");
    }
    println!("  Activating 'doli' group membership...");
    println!();
    let status = std::process::Command::new("sg")
        .args(["doli", "-c", &cmd])
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Find a legacy wallet if the current wallet path doesn't exist.
/// Returns the legacy directory containing wallet.json, or None.
fn find_legacy_wallet(network: &str, current_wallet: &Path) -> Option<PathBuf> {
    if current_wallet.exists() {
        return None;
    }
    let home = dirs::home_dir()?;
    let legacy_dir = home.join(".doli").join(network);
    let legacy_wallet = legacy_dir.join("wallet.json");
    if legacy_wallet.exists() {
        Some(legacy_dir)
    } else {
        None
    }
}

/// Migrate wallet (and seed file) from legacy dir to the new platform dir.
fn migrate_legacy_wallet(legacy_dir: &Path, new_dir: &Path) -> Result<()> {
    // Ensure target directory exists
    if !new_dir.exists() {
        std::fs::create_dir_all(new_dir)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(new_dir, std::fs::Permissions::from_mode(0o2770))?;
        }
    }

    let mut migrated = Vec::new();
    // Copy wallet.json and any seed/key files
    for name in &["wallet.json", "wallet.seed.txt"] {
        let src = legacy_dir.join(name);
        let dst = new_dir.join(name);
        if src.exists() && !dst.exists() {
            std::fs::copy(&src, &dst)?;
            // Preserve appropriate permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if *name == "wallet.json" { 0o640 } else { 0o600 };
                std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(mode))?;
            }
            migrated.push(*name);
        }
    }

    if !migrated.is_empty() {
        println!("  Migrated from {}:", legacy_dir.display());
        for name in &migrated {
            println!("    {} -> {}", name, new_dir.join(name).display());
        }
        println!();
        println!(
            "  The legacy directory is still at {}",
            legacy_dir.display()
        );
        println!(
            "  You can remove it after verifying: rm -rf {}",
            legacy_dir.display()
        );
        println!();
    }

    Ok(())
}

pub(crate) fn cmd_init(
    network: &str,
    wallet_path: &Path,
    force: bool,
    non_producer: bool,
) -> Result<()> {
    println!();
    println!("  DOLI Producer Setup");
    println!("  ====================");
    println!();
    println!("  Network:  {}", network);
    println!(
        "  Data dir: {}",
        wallet_path.parent().unwrap_or(wallet_path).display()
    );
    println!();

    // Check for legacy wallet and migrate if found
    if !wallet_path.exists() && !force {
        let new_dir = wallet_path.parent().unwrap_or(wallet_path);
        if let Some(legacy_dir) = find_legacy_wallet(network, wallet_path) {
            println!("  Found legacy wallet at {}", legacy_dir.display());
            println!();
            migrate_legacy_wallet(&legacy_dir, new_dir)?;

            // Verify migration succeeded
            if wallet_path.exists() {
                let wallet = Wallet::load(wallet_path)?;
                let bech32_addr = wallet.primary_bech32_address(address_prefix());
                println!("  Wallet migrated successfully!");
                println!("    Address: {}", bech32_addr);
                println!();
                return Ok(());
            }
        }
    }

    // Check if wallet already exists
    if wallet_path.exists() && !force {
        let wallet = Wallet::load(wallet_path)?;
        let bech32_addr = wallet.primary_bech32_address(address_prefix());
        println!("  Wallet already exists at {}", wallet_path.display());
        println!("    Address: {}", bech32_addr);
        println!();
        println!("  Run 'doli info' to see details.");
        println!("  Run 'doli init --force' to overwrite (DANGEROUS: destroys existing keys).");
        return Ok(());
    }

    if force && wallet_path.exists() {
        println!("  WARNING: --force specified. Overwriting existing wallet.");
        println!();
    }

    // Check write access to data directory
    let data_dir = wallet_path.parent().unwrap_or(wallet_path);
    if !dir_is_writable(data_dir) {
        #[cfg(target_os = "linux")]
        {
            // If user is in 'doli' group but hasn't re-logged, activate via `sg`
            if user_in_doli_group_but_inactive() {
                return reexec_with_doli_group(force, non_producer);
            }
            bail!(
                "Cannot write to {}\n\n  \
                 Fix: add yourself to the 'doli' group and re-login:\n  \
                   sudo usermod -aG doli $USER && newgrp doli\n  \
                 Then retry: doli init\n",
                data_dir.display()
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!(
                "Cannot write to {}\n  Check directory permissions.",
                data_dir.display()
            );
        }
    }

    // Create the data directory if needed
    if let Some(parent) = wallet_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o2770))?;
            }
        }
    }

    // Generate a new wallet (same logic as `doli new`)
    println!("  Creating wallet...");
    println!();

    let (mut wallet, phrase) = Wallet::new("default");

    // Display the 24-word seed phrase
    let words: Vec<&str> = phrase.split_whitespace().collect();
    println!("  YOUR 24-WORD SEED PHRASE:");
    println!("  +------------------------------------------------------+");
    for (chunk_idx, chunk) in words.chunks(6).enumerate() {
        let numbered: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(j, w)| {
                let word_num = chunk_idx * 6 + j + 1;
                format!("{:>2}. {:<12}", word_num, w)
            })
            .collect();
        println!("  |  {}|", numbered.join(""));
    }
    println!("  |                                                      |");
    println!("  |  WRITE THIS DOWN AND STORE IT SAFELY.                |");
    println!("  |  This is the ONLY way to recover your wallet.        |");
    println!("  |  It will NOT be shown again.                         |");
    println!("  +------------------------------------------------------+");
    println!();

    // Prompt user to confirm they saved it
    print!("  Have you saved your seed phrase? [y/N]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    if input != "y" && input != "yes" {
        println!();
        println!("  Aborted. Please save your seed phrase and try again.");
        return Ok(());
    }

    // Generate BLS attestation key if producer mode
    if !non_producer {
        println!();
        println!("  Generating BLS attestation key...");
        // Wallet::new() already generates BLS keys in v2, but verify
        if !wallet.has_bls_key() {
            wallet.add_bls_key()?;
        }
    }

    // Save wallet
    wallet.save(wallet_path)?;

    // Also write seed phrase to a separate file (same as cmd_new)
    // Permissions: owner-only (0600) to prevent other users from reading the seed
    let seed_path = wallet_path.with_extension("seed.txt");
    let mut seed_content = String::new();
    for (i, word) in words.iter().enumerate() {
        seed_content.push_str(&format!("{}. {}\n", i + 1, word));
    }
    std::fs::write(&seed_path, &seed_content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&seed_path, std::fs::Permissions::from_mode(0o600))?;
    }

    // Print the producer identity
    let bech32_addr = wallet.primary_bech32_address(address_prefix());
    let pubkey = wallet.primary_public_key();

    println!();
    println!("  Your producer identity:");
    println!("    Address:    {}", bech32_addr);
    println!("    Public key: {}", pubkey);
    if let Some(bls_pub) = wallet.primary_bls_public_key() {
        println!("    BLS key:    {}", bls_pub);
    }

    println!();
    println!("  Wallet saved to: {}", wallet_path.display());
    println!("  Seed phrase saved to: {}", seed_path.display());
    println!();
    println!("  WARNING: Delete the seed file after you have written down the phrase:");
    println!("    rm {}", seed_path.display());
    println!();

    // Print next steps
    println!("  NEXT STEPS:");
    println!("  1. Fund your address with at least 10 DOLI (1 bond)");
    println!("  2. Register: doli producer register --bonds 1");
    println!("  3. Start:    sudo doli service install");
    println!();

    Ok(())
}
