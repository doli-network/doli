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
pub(crate) fn resolve_upgrade_trust_root(
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

/// `doli release verify` — judge a release manifest and install nothing (INC-I-202 M2).
///
/// The exit code IS the verdict, so `scripts/publish-release.sh` can gate promotion on
/// it. The trust root is resolved exactly as `doli upgrade` resolves it, so the publish
/// gate answers the same question the fleet will ask at install time.
pub(crate) async fn cmd_release_verify(
    version: String,
    dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    network: doli_core::Network,
    trust_root: Option<String>,
) -> Result<()> {
    let data_dir = match data_dir {
        Some(d) => d,
        None => crate::paths::resolve_base_dir(network.name(), None),
    };
    // Best-effort: the on-chain height is reported for staleness, never used to judge.
    // The `None` arm below re-reads the same file fatally, so this cannot mask a failure.
    let on_chain_height = match storage::MaintainerState::load(&data_dir) {
        Ok(state) => state.last_derived_height.to_string(),
        Err(_) => "unavailable".to_string(),
    };
    let root = match trust_root.as_deref() {
        None => resolve_upgrade_trust_root(&data_dir, network)?,
        Some("bootstrap") => {
            println!(
                "BYPASS: --trust-root bootstrap ignores this host's on-chain maintainer root \
                 and judges the manifest against the compiled bootstrap keys."
            );
            updater::TrustRoot::bootstrap(network)
        }
        Some(other) => anyhow::bail!(
            "release verify: --trust-root accepts only `bootstrap`, got `{}`. Omit the flag \
             to use this host's on-chain maintainer root.",
            other
        ),
    };
    println!(
        "Trust root: {} ({} key(s), threshold {}, {}) from {} (on-chain last_derived_height {})",
        root.provenance(),
        root.keys().len(),
        root.threshold(),
        network,
        data_dir.display(),
        on_chain_height
    );

    let distinct_signers = match dir {
        // A DRAFT release is invisible to the unauthenticated GitHub API, so the publish
        // gate hands over the directory it already downloaded.
        Some(d) => doli_cli::cmd_release_verify::verify_manifest_dir(&d, &version, &root)?,
        None => {
            let release = updater::fetch_github_release(Some(&version))
                .await
                .map_err(|e| anyhow::anyhow!("Failed to fetch release v{}: {}", version, e))?;
            // "I could not check" is not "it is fine": an unreachable or absent
            // SIGNATURES.json is a hard refusal, never a pass.
            let sf = updater::download_signatures_json(&release.version)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Could not retrieve SIGNATURES.json for v{}: {}. Refusing to report an \
                         unverified release as verified.",
                        release.version,
                        e
                    )
                })?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Release v{} has no SIGNATURES.json. An unsigned release is not a \
                         verified release.",
                        release.version
                    )
                })?;
            updater::verify_release_manifest(&release.version, &release.checksums_body, &sf, &root)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Maintainer verification FAILED for v{} on {}: {}.",
                        release.version,
                        network,
                        e
                    )
                })?
        }
    };

    // The count actually found, never the threshold constant (QA OBS-001).
    println!(
        "Verified: {} distinct maintainer signature(s) for v{} (threshold {}, {} trust root)",
        distinct_signers,
        version.trim_start_matches('v'),
        root.threshold(),
        root.provenance()
    );
    Ok(())
}

#[cfg(test)]
#[path = "cmd_upgrade_tests.rs"]
mod inc_i_199_trust_root_advice_tests;
