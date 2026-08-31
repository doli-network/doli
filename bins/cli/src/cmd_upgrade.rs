use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::upgrade_restart::{find_doli_node_path, restart_doli_service, restart_specific_service};

/// Operator advice for a failed `MaintainerState::load`, chosen by WHAT went wrong.
///
/// INC-I-199: this used to be one fixed string calling the file damaged and telling the
/// operator to restore or remove it. A plain `EACCES` from forgetting `sudo` got that
/// message too — and acting on it deletes a healthy trust root, which drops the host to
/// `Bootstrap` provenance and the compiled keys. On a binary predating the INC-I-196
/// cutover those are the publicly leaked keys, so the advice steered operators into the
/// exposure INC-I-175 rotated away from. Never advise deletion for an error that did not
/// come from reading the file's CONTENT.
fn trust_root_load_advice(err: &storage::StorageError, data_dir: &Path) -> String {
    let head = format!(
        "FATAL: cannot load the maintainer trust root from {}: {err}\n  \
         This file decides which keys may authorise a binary install, so `doli upgrade` \
         refuses rather than falling back to the compiled bootstrap keys.",
        data_dir.display()
    );

    let tail = match err {
        storage::StorageError::Io(io) => match io.kind() {
            std::io::ErrorKind::PermissionDenied => {
                "\n  This is a PERMISSION error, not a damaged file. The data directory is \
                 usually root-owned: re-run with `sudo doli upgrade`. Do NOT delete \
                 maintainer_state.bin — it is almost certainly intact."
            }
            std::io::ErrorKind::NotFound => {
                "\n  The path disappeared between the existence check and the read. Confirm \
                 --data-dir points at this host's node data directory, then retry. Do NOT \
                 delete anything."
            }
            _ => {
                "\n  This is an I/O error reading the file, not proof that its contents are \
                 bad. Check the device and the path, then retry before considering recovery."
            }
        },
        _ => {
            "\n  A file written by an older binary is migrated automatically, so this means \
             the file is damaged rather than merely old. Restore it from a backup, or remove \
             it deliberately and let the node re-derive the maintainer set from the chain."
        }
    };

    head + tail
}

/// Resolve the release trust root for `doli upgrade` from the node data directory on
/// THIS host (INC-I-172 M1, AUDIT-P1-012).
///
/// `doli upgrade` runs as root on producer hosts and is the path the docs call the
/// remediation path. It used to pin `TrustRoot::bootstrap` — the compiled, publicly
/// exposed keys — while the host's on-chain maintainer set sat one file read away. That
/// is a one-command revocation bypass: every fix on the `doli-node` path is undone by
/// typing the other binary's name.
///
/// The decision itself is [`updater::TrustRoot::resolve`], shared verbatim with
/// `bins/node/src/updater/trust_root_wiring.rs`, so both binaries reach the same answer
/// on the same host and the AUDIT-P0-010 containment applies to both.
///
/// Errors are FATAL, exactly as at node startup: a `maintainer_state.bin` that exists but
/// cannot be decoded must abort the upgrade, never degrade to the compiled keys.
fn resolve_upgrade_trust_root(
    data_dir: &Path,
    network: doli_core::Network,
) -> Result<updater::TrustRoot> {
    let state = storage::MaintainerState::load(data_dir)
        .map_err(|e| anyhow::anyhow!("{}", trust_root_load_advice(&e, data_dir)))?;
    let keys: Vec<String> = state.set.members.iter().map(|m| m.to_hex()).collect();
    Ok(updater::TrustRoot::resolve(
        keys,
        state.set.threshold,
        state.last_derived_height,
        network,
    ))
}

/// Install a released `doli` / `doli-node` pair.
///
/// `network` selects which compiled bootstrap array is used when — and only when — this
/// host has no on-chain maintainer set. It comes from the invoking `--network` flag.
///
/// WARNING (AUDIT-P2-012): `--network` is NOT a security boundary today. The signed
/// release message carries no network term (`"{version}:{sha256(CHECKSUMS.txt)}"`), so a
/// signature made for one network verifies on the other wherever that signer appears in
/// the resolved array. INC-I-196 made the two compiled arrays disjoint, which narrows who
/// can cross but does NOT close it. The flag selects a key array; it binds nothing.
/// Adding a network term to the signed bytes invalidates every already-published
/// `SIGNATURES.json`, so it is deferred with its own rollout.
pub(crate) async fn cmd_upgrade(
    version: Option<String>,
    yes: bool,
    doli_node_path: Option<std::path::PathBuf>,
    service: Option<String>,
    data_dir: Option<PathBuf>,
    network: doli_core::Network,
) -> Result<()> {
    // The node data directory on this host: explicit `--data-dir`, else the same
    // flag > env > platform-default > legacy chain every other `doli` command uses.
    let data_dir = match data_dir {
        Some(d) => d,
        None => crate::paths::resolve_base_dir(network.name(), None),
    };

    let current = updater::current_version();
    println!("Current version: v{}", current);
    println!("Checking for updates...");

    let release = updater::fetch_github_release(version.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch release: {}", e))?;

    if !updater::is_newer_version(&release.version, current) {
        if let Some(ref svc) = service {
            // Binary already updated (e.g. by a prior run on this server),
            // but the caller wants a specific service restarted.
            println!(
                "Binary already at v{}, restarting service: {}",
                current, svc
            );
            restart_specific_service(svc);
            return Ok(());
        }
        println!("Already up to date (v{}).", current);
        return Ok(());
    }

    println!();
    println!(
        "New version available: v{} -> v{}",
        current, release.version
    );
    if !release.changelog.is_empty() {
        println!();
        // Show first 20 lines of changelog
        for line in release.changelog.lines().take(20) {
            println!("  {}", line);
        }
        println!();
    }

    if !yes {
        print!("Proceed with upgrade? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Upgrade cancelled.");
            return Ok(());
        }
    }

    // Download tarball
    println!("Downloading v{}...", release.version);
    let tarball = updater::download_from_url(&release.tarball_url)
        .await
        .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    // Maintainer signatures GATE the install (INC-I-172 F6). `doli upgrade` runs as
    // root on producer hosts and is the documented remediation path, so every failure
    // below aborts before anything is extracted or written:
    //   - SIGNATURES.json unreachable — "I could not check" is not "it is fine"; a
    //     network failure is indistinguishable from an attacker withholding the file;
    //   - SIGNATURES.json absent — an unsigned release is not a verified release;
    //   - below threshold, or any verification error — refuse.
    println!("Checking maintainer signatures...");
    let signatures = updater::download_signatures_json(&release.version)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Could not retrieve SIGNATURES.json for v{}: {}. Refusing to install an \
                 unverified release.",
                release.version,
                e
            )
        })?;
    let sf = signatures.ok_or_else(|| {
        anyhow::anyhow!(
            "Release v{} has no SIGNATURES.json. An unsigned release is not a verified \
             release; refusing to install.",
            release.version
        )
    })?;
    // Resolve the trust root from the node data directory on THIS host (AUDIT-P1-012).
    // `doli upgrade` runs where the node runs — on a producer, `maintainer_state.bin` is
    // one file read away — so pinning the compiled bootstrap keys here made this command
    // a one-command bypass of every revocation the `doli-node` path honours. Bootstrap is
    // now reached ONLY through the genuinely-unbootstrapped branch inside
    // `TrustRoot::resolve`, and the provenance is printed below either way.
    let root = resolve_upgrade_trust_root(&data_dir, network)?;
    println!(
        "Trust root: {} ({} key(s), threshold {}, {}) from {}",
        root.provenance(),
        root.keys().len(),
        root.threshold(),
        network,
        data_dir.display()
    );
    // `verify_release_artifact` — not a bare signature check. It binds the signatures
    // to the artifact: sf.version to the release tag, sf.checksums_sha256 to the bytes
    // of the CHECKSUMS.txt actually fetched, and the tarball to the per-platform hash
    // parsed from THOSE bytes. Checking signatures over `sf`'s own self-reported pair
    // (what this code used to do) accepts a verbatim replay of any past genuine
    // SIGNATURES.json while installing an arbitrary binary — INC-I-172 F1.
    let distinct_signers = updater::verify_release_artifact(&release, &tarball, &sf, &root)
        .map_err(|e| {
            anyhow::anyhow!(
                "Maintainer verification FAILED for v{} on {}: {}. Refusing to install.",
                release.version,
                network,
                e
            )
        })?;
    // Print the count that was actually found, never the constant threshold: an
    // operator with 5 valid signatures used to be told "3" (QA OBS-001), which hides
    // exactly the signal — how much of the maintainer set stood behind this build.
    println!(
        "Verified: {} distinct maintainer signature(s) bound to this v{} tarball \
         (threshold {}, {} trust root)",
        distinct_signers,
        release.version,
        root.threshold(),
        root.provenance()
    );

    // Extract and install doli (CLI binary — ourselves)
    let cli_binary = updater::extract_named_binary_from_tarball(&tarball, "doli")
        .map_err(|e| anyhow::anyhow!("Failed to extract doli binary: {}", e))?;
    let cli_path = std::env::current_exe()?;
    println!("Installing doli to {:?}...", cli_path);
    if let Err(e) = updater::install_binary(&cli_binary, &cli_path).await {
        if e.to_string().contains("Permission denied") || e.to_string().contains("os error 13") {
            return Err(anyhow::anyhow!(
                "Permission denied writing to {:?}.\n  Try: sudo doli upgrade{}",
                cli_path,
                if yes { " --yes" } else { "" }
            ));
        }
        return Err(anyhow::anyhow!("Failed to install doli: {}", e));
    }

    // Extract and install doli-node (if found in tarball)
    let mut installed_node_path: Option<std::path::PathBuf> = None;
    match updater::extract_named_binary_from_tarball(&tarball, "doli-node") {
        Ok(node_binary) => {
            // Use custom path if provided, otherwise auto-detect
            let node_path = doli_node_path.or_else(find_doli_node_path);
            if let Some(path) = node_path {
                println!("Installing doli-node to {:?}...", path);
                if let Err(e) = updater::install_binary(&node_binary, &path).await {
                    if e.to_string().contains("Permission denied")
                        || e.to_string().contains("os error 13")
                    {
                        return Err(anyhow::anyhow!(
                            "Permission denied writing to {:?}.\n  Try: sudo doli upgrade{}",
                            path,
                            if yes { " --yes" } else { "" }
                        ));
                    }
                    return Err(anyhow::anyhow!("Failed to install doli-node: {}", e));
                }
                installed_node_path = Some(path);
            } else {
                println!("doli-node not found on system, skipping node binary install.");
                println!("  Hint: use --doli-node-path <PATH> to specify the doli-node location.");
            }
        }
        Err(_) => {
            println!("doli-node not in tarball, skipping node binary install.");
        }
    }

    // Update agent skills (best-effort)
    match updater::install_skills_from_tarball(&tarball) {
        Ok(count) if count > 0 => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_default();
            println!("Updated {} agent skills at {}/.doli/skills/", count, home);
        }
        Ok(_) => {}
        Err(e) => println!("Note: could not update agent skills: {}", e),
    }

    // Restart only the service that owns the installed binary
    if let Some(ref svc) = service {
        restart_specific_service(svc);
    } else {
        restart_doli_service(installed_node_path.as_deref());
    }

    println!();
    println!("Upgrade to v{} complete!", release.version);

    Ok(())
}

#[cfg(test)]
mod inc_i_199_trust_root_advice_tests {
    use super::*;

    // INC-I-199. Observed live on vm-server: a non-sudo `doli upgrade` reported
    // "io error: Permission denied (os error 13)" and then advised that the file was
    // damaged and should be restored or removed. Acting on that deletes a healthy
    // trust root and drops the host to the compiled keys — the leaked five on any
    // binary predating the INC-I-196 cutover.
    //
    // OUTPUT CONTRACT
    //   Under test : `trust_root_load_advice(&StorageError, &Path) -> String`
    //   Outputs    : O1 the advice text. No mutable params, no I/O, no side channel.
    //   Code paths : P1 Io/PermissionDenied, P2 Io/NotFound, P3 Io/other,
    //                P4 non-Io (decode, unsupported version, malformed value)
    //   Partitions : the error variant IS the partition.

    fn io(kind: std::io::ErrorKind) -> storage::StorageError {
        storage::StorageError::Io(std::io::Error::new(kind, "test"))
    }

    fn dir() -> &'static Path {
        Path::new("/var/lib/doli/mainnet")
    }

    /// REQ-199-001 (Must). The regression: EACCES must never be called damage.
    /// [P1 -> O1]
    #[test]
    fn permission_denied_says_sudo_and_never_suggests_deleting_the_file() {
        let msg = trust_root_load_advice(&io(std::io::ErrorKind::PermissionDenied), dir());

        assert!(
            msg.contains("sudo"),
            "EACCES must tell the operator to re-run with sudo. Got: {msg}"
        );
        // Assert on the harmful CLAIM, not the word: the corrected text legitimately
        // says "not a damaged file", so a bare contains("damaged") would fail on a
        // correct message.
        assert!(
            !msg.contains("damaged rather than merely old"),
            "a permission error must NOT be reported as a damaged file: {msg}"
        );
        assert!(
            !msg.contains("remove it deliberately") && !msg.contains("Restore it from a backup"),
            "advising deletion here destroys a healthy trust root and drops the host to \
             the compiled (pre-INC-I-196: leaked) keys: {msg}"
        );
    }

    /// REQ-199-002 (Must). No I/O failure implies bad content.
    /// [P2, P3 -> O1]
    #[test]
    fn other_io_errors_do_not_advise_deletion_either() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let msg = trust_root_load_advice(&io(kind), dir());
            assert!(
                !msg.contains("remove it deliberately"),
                "{kind:?}: an I/O failure is not evidence the contents are bad: {msg}"
            );
            assert!(
                !msg.contains("damaged rather than merely old"),
                "{kind:?}: must not claim damage from an I/O error: {msg}"
            );
        }
    }

    /// REQ-199-003 (Must). GREEN-lock: a genuine CONTENT failure must keep the original
    /// restore-or-remove guidance — the fix must not blunt real corruption reporting.
    /// [P4 -> O1]
    #[test]
    fn a_decode_failure_still_gets_the_restore_or_remove_guidance() {
        let msg = trust_root_load_advice(
            &storage::StorageError::Serialization("bad body".into()),
            dir(),
        );
        assert!(
            msg.contains("damaged") && msg.contains("remove it deliberately"),
            "a real decode failure must still tell the operator how to recover: {msg}"
        );
    }

    /// REQ-199-004 (Must). Every path names the file and keeps the fail-closed rationale,
    /// so the message is actionable regardless of which branch produced it.
    /// [P1, P2, P3, P4 -> O1]
    #[test]
    fn every_branch_names_the_path_and_states_why_it_refuses() {
        let errs = [
            io(std::io::ErrorKind::PermissionDenied),
            io(std::io::ErrorKind::NotFound),
            io(std::io::ErrorKind::Other),
            storage::StorageError::Serialization("bad".into()),
        ];
        for e in &errs {
            let msg = trust_root_load_advice(e, dir());
            assert!(
                msg.contains("/var/lib/doli/mainnet"),
                "must name the path: {msg}"
            );
            assert!(
                msg.contains("falling back to the compiled bootstrap keys"),
                "must keep the fail-closed rationale: {msg}"
            );
        }
    }
}
