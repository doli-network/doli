//! Root-side trigger for the staged upgrade (INC-I-215 M4).
//!
//! The node unit runs sandboxed and cannot write `/usr/bin`, so it only STAGES a verified
//! release. These two units are the privileged half: a `.path` watching the staged marker
//! and a root oneshot that runs `doli upgrade --from-staged`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// Directory systemd loads unit files from.
const UNIT_DIR: &str = "/etc/systemd/system";

/// Names of the `.path` watcher and the `.service` it triggers, derived from the resolved
/// node service name so two nodes on one host never share a unit.
pub fn helper_unit_names(service_name: &str) -> (String, String) {
    (
        format!("{service_name}-upgrade.path"),
        format!("{service_name}-upgrade.service"),
    )
}

/// On-disk path of the `.path` watcher unit.
pub fn helper_path_unit_path(service_name: &str) -> PathBuf {
    let (path_unit, _) = helper_unit_names(service_name);
    PathBuf::from(UNIT_DIR).join(path_unit)
}

/// On-disk path of the triggered oneshot unit.
pub fn helper_service_unit_path(service_name: &str) -> PathBuf {
    let (_, service_unit) = helper_unit_names(service_name);
    PathBuf::from(UNIT_DIR).join(service_unit)
}

/// Render the `.path` unit that watches the staged ready marker.
///
/// `PathExists=` is built from `updater::staging_dir` + `updater::READY_MARKER`, never from
/// a hand-typed literal: the node's M2 startup preflight scans `/etc/systemd/system/*.path`
/// for exactly that string, and any drift leaves the node warning forever.
pub fn helper_path_unit_content(service_name: &str, data_dir: &Path) -> String {
    let (_, service_unit) = helper_unit_names(service_name);
    let marker = updater::staging_dir(data_dir).join(updater::READY_MARKER);
    format!(
        "[Unit]
Description=DOLI staged-upgrade watcher for {service_name}

[Path]
PathExists={marker}
Unit={service_unit}

[Install]
WantedBy=multi-user.target
",
        service_name = service_name,
        marker = marker.display(),
        service_unit = service_unit,
    )
}

/// Render the root oneshot the `.path` triggers.
///
/// It carries NO `User=`, `NoNewPrivileges`, `ProtectSystem` or `ReadWritePaths`: the whole
/// point is that this half can write `/usr/bin`, which the sandboxed node unit cannot. It
/// also carries no `[Install]` — it is path-triggered and must never be boot-enabled.
pub fn helper_service_unit_content(
    service_name: &str,
    network: &str,
    data_dir: &Path,
    doli_cli: &Path,
) -> String {
    let staged = updater::staging_dir(data_dir);
    format!(
        "[Unit]
Description=DOLI staged-upgrade installer for {service_name}

[Service]
Type=oneshot
ExecStart={cli} --network {network} upgrade --from-staged {staged} --data-dir {data_dir} \
--service {service_name} --yes
",
        service_name = service_name,
        network = network,
        cli = doli_cli.display(),
        staged = staged.display(),
        data_dir = data_dir.display(),
    )
}

/// Absolute path of the `doli` CLI to bake into the oneshot's ExecStart.
pub fn which_doli_cli() -> PathBuf {
    if let Ok(output) = std::process::Command::new("which").arg("doli").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
    }
    for path in &["/usr/local/bin/doli", "/usr/bin/doli"] {
        if Path::new(path).exists() {
            return PathBuf::from(path);
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/doli"))
}

/// Write both units, reload systemd, then enable the watcher. Plain overwrite, so a
/// re-install is idempotent.
pub fn install_helper_units(
    service_name: &str,
    network: &str,
    data_dir: &Path,
    doli_cli: &Path,
) -> Result<()> {
    let (path_unit_name, _) = helper_unit_names(service_name);
    let path_unit = helper_path_unit_path(service_name);
    let service_unit = helper_service_unit_path(service_name);

    println!("Writing staged-upgrade unit: {}", path_unit.display());
    std::fs::write(&path_unit, helper_path_unit_content(service_name, data_dir))?;
    println!("Writing staged-upgrade unit: {}", service_unit.display());
    std::fs::write(
        &service_unit,
        helper_service_unit_content(service_name, network, data_dir, doli_cli),
    )?;

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", "--now", &path_unit_name])?;
    Ok(())
}

/// Disable and delete both helper units. An absent file or a failed disable is tolerated.
pub fn remove_helper_units(service_name: &str) -> Result<()> {
    let (path_unit_name, _) = helper_unit_names(service_name);
    let _ = std::process::Command::new("systemctl")
        .args(["disable", "--now", &path_unit_name])
        .status();

    for unit in [
        helper_path_unit_path(service_name),
        helper_service_unit_path(service_name),
    ] {
        if unit.exists() {
            println!("Removing {}...", unit.display());
            std::fs::remove_file(&unit)?;
        }
    }

    run_systemctl(&["daemon-reload"])?;
    Ok(())
}

/// Give an already-deployed host the helper units on its next `doli upgrade`.
///
/// There is no re-install step in the fleet's routine, so without this the fix would only
/// ever reach freshly installed hosts. Best-effort by construction: it returns `()` and can
/// never turn a working upgrade into a failure.
pub fn refresh_helper_units_if_root(network: &str, service: Option<&str>, data_dir: Option<&Path>) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let euid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if euid.trim() != "0" {
        return;
    }

    let service_name = service
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("doli-{network}"));
    let node_unit = format!("/etc/systemd/system/{service_name}.service");
    if !Path::new(&node_unit).exists() {
        return;
    }

    let dir = data_dir
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!("/var/lib/doli/{network}")));
    let cli = which_doli_cli();

    let mut wrote = false;
    for (unit, content) in [
        (
            helper_path_unit_path(&service_name),
            helper_path_unit_content(&service_name, &dir),
        ),
        (
            helper_service_unit_path(&service_name),
            helper_service_unit_content(&service_name, network, &dir, &cli),
        ),
    ] {
        if std::fs::read_to_string(&unit).ok().as_deref() != Some(content.as_str())
            && std::fs::write(&unit, &content).is_ok()
        {
            wrote = true;
        }
    }

    if wrote {
        let (path_unit_name, _) = helper_unit_names(&service_name);
        let _ = run_systemctl(&["daemon-reload"]);
        let _ = run_systemctl(&["enable", "--now", &path_unit_name]);
        println!("Refreshed staged-upgrade helper units for {service_name}.");
    }
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()?;
    if !status.success() {
        bail!("Command failed: systemctl {}", args.join(" "));
    }
    Ok(())
}
