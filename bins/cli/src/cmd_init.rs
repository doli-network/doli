use std::io::Write;
use std::path::Path;

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
