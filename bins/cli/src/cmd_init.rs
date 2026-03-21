use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::common::address_prefix;
use crate::wallet::Wallet;

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

    // Create the data directory if needed
    if let Some(parent) = wallet_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o750))?;
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
    let seed_path = wallet_path.with_extension("seed.txt");
    let mut seed_content = String::new();
    for (i, word) in words.iter().enumerate() {
        seed_content.push_str(&format!("{}. {}\n", i + 1, word));
    }
    std::fs::write(&seed_path, &seed_content)?;

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
