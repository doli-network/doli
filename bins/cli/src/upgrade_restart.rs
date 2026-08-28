//! Post-install service restart for `doli upgrade`.
//!
//! Split out of `cmd_upgrade.rs` to keep that file inside the 500-line module budget.
//! Nothing here makes a trust decision: `cmd_upgrade` has already verified maintainer
//! signatures and installed the binary before any of this runs.

use doli_cli::upgrade_systemd_plan::{systemd_restart_plan, SystemdStep};

/// Run a systemd plan under `sudo`. Steps with `required == false` run with their exit
/// status discarded; the return value is whether the required step succeeded.
fn run_systemd_plan(steps: &[SystemdStep]) -> bool {
    let mut ok = false;
    for step in steps {
        let status = std::process::Command::new("sudo").args(&step.args).status();
        if step.required {
            ok = matches!(status, Ok(s) if s.success());
        }
    }
    ok
}

/// Find the path to an installed doli-node binary
pub(crate) fn find_doli_node_path() -> Option<std::path::PathBuf> {
    // Tier 1: Check running doli-node process to find its binary path
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("pgrep")
            .args(["-a", "doli-node"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Take the first match's binary path (first token after PID)
                if let Some(line) = stdout.lines().next() {
                    if let Some(bin_path) = line.split_whitespace().nth(1) {
                        let p = std::path::PathBuf::from(bin_path);
                        if p.exists() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }

    // Tier 2: Check `which doli-node`
    if let Ok(output) = std::process::Command::new("which")
        .arg("doli-node")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(std::path::PathBuf::from(path));
            }
        }
    }

    // Tier 3: Check common install paths (most specific first)
    for path in &[
        "/mainnet/bin/doli-node",
        "/testnet/bin/doli-node",
        "/usr/local/bin/doli-node",
        "/opt/doli/target/release/doli-node",
    ] {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Restart a specific systemd service by name
pub(crate) fn restart_specific_service(service: &str) {
    println!("Restarting service: {}", service);
    if run_systemd_plan(&systemd_restart_plan(service)) {
        println!("  Restarted {}.", service);
    } else {
        println!(
            "  Failed to restart {}. Run: sudo systemctl reset-failed {} && sudo systemctl restart {}",
            service, service, service
        );
    }
}

/// Detect and restart the doli-node service that owns the installed binary.
/// If `installed_path` is provided, only restart the service whose ExecStart
/// references that path. Otherwise fall back to process detection.
pub(crate) fn restart_doli_service(
    #[allow(unused_variables)] installed_path: Option<&std::path::Path>,
) {
    // Tier 1: systemd (Linux) — find and restart only the owning service
    #[cfg(target_os = "linux")]
    {
        if try_restart_systemd(installed_path) {
            return;
        }
    }

    // Tier 2: launchd (macOS) — launchctl list shows all loaded agents
    #[cfg(target_os = "macos")]
    {
        if try_restart_launchd() {
            return;
        }
    }

    // Tier 3: Process fallback (all platforms)
    if try_restart_process() {
        return;
    }

    println!("No doli-node service or process found. Restart manually if needed.");
}

/// Tier 1: Detect and restart doli-node via systemd.
/// If `installed_path` is provided, only restart the service whose ExecStart
/// references that binary — prevents restarting unrelated services on multi-node servers.
#[cfg(target_os = "linux")]
fn try_restart_systemd(installed_path: Option<&std::path::Path>) -> bool {
    let output = match std::process::Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-pager"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all_units: Vec<String> = stdout
        .lines()
        .filter(|line| line.contains("doli"))
        .filter_map(|line| line.split_whitespace().next().map(|s| s.to_string()))
        .collect();

    if all_units.is_empty() {
        return false;
    }

    // Filter to only services whose ExecStart matches the installed binary path
    let units: Vec<String> = if let Some(bin_path) = installed_path {
        let bin_str = bin_path.to_string_lossy();
        all_units
            .into_iter()
            .filter(|unit| {
                // Read the service's ExecStart to check if it uses our binary
                if let Ok(cat_output) = std::process::Command::new("systemctl")
                    .args(["cat", unit, "--no-pager"])
                    .output()
                {
                    let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                    cat_str.contains(bin_str.as_ref())
                } else {
                    false
                }
            })
            .collect()
    } else {
        // No path context — only restart services that are currently running
        all_units
            .into_iter()
            .filter(|unit| {
                if let Ok(is_active) = std::process::Command::new("systemctl")
                    .args(["is-active", "--quiet", unit])
                    .status()
                {
                    is_active.success()
                } else {
                    false
                }
            })
            .collect()
    };

    if units.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for unit in &units {
        println!("Restarting service: {}", unit);
        if run_systemd_plan(&systemd_restart_plan(unit)) {
            println!("  Restarted {}.", unit);
            any_ok = true;
        } else {
            println!(
                "  Failed to restart {}. Run: sudo systemctl reset-failed {} && sudo systemctl restart {}",
                unit, unit, unit
            );
        }
    }
    any_ok
}

/// Tier 2: Detect and restart doli-node via launchd
#[cfg(target_os = "macos")]
fn try_restart_launchd() -> bool {
    let output = match std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        Ok(o) => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let services: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("doli"))
        .filter_map(|line| line.split_whitespace().last())
        .collect();

    if services.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for label in &services {
        println!("Restarting service: {}", label);
        let result = std::process::Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{}/{}", get_uid(), label)])
            .status();
        match result {
            Ok(s) if s.success() => {
                println!("  Restarted {}.", label);
                any_ok = true;
            }
            _ => {
                // Fallback: stop + start
                let _ = std::process::Command::new("launchctl")
                    .args(["stop", label])
                    .status();
                let _ = std::process::Command::new("launchctl")
                    .args(["start", label])
                    .status();
                println!("  Restarted {} (stop+start).", label);
                any_ok = true;
            }
        }
    }
    any_ok
}

/// Tier 3: Detect doli-node process via pgrep and restart it
fn try_restart_process() -> bool {
    // pgrep -a on Linux shows PID + full cmdline; pgrep -fl on macOS does the same
    let pgrep_args = if cfg!(target_os = "macos") {
        vec!["-fl", "doli-node"]
    } else {
        vec!["-a", "doli-node"]
    };

    let output = match std::process::Command::new("pgrep")
        .args(&pgrep_args)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for line in &lines {
        // Format: "<PID> <full command line>"
        let mut parts = line.splitn(2, char::is_whitespace);
        let pid_str = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        let cmdline = match parts.next() {
            Some(c) => c.trim(),
            None => continue,
        };

        println!("Found doli-node process (PID {}): {}", pid_str, cmdline);
        println!("  Sending SIGTERM...");

        // Kill the process gracefully
        let kill_ok = std::process::Command::new("kill")
            .arg(pid_str)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !kill_ok {
            println!("  Failed to kill PID {}. May need sudo.", pid_str);
            continue;
        }

        // Wait for clean shutdown
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Respawn with the same command line, detached
        println!("  Respawning: {}", cmdline);
        let spawn_result = std::process::Command::new("sh")
            .args(["-c", &format!("nohup {} > /dev/null 2>&1 &", cmdline)])
            .status();

        match spawn_result {
            Ok(s) if s.success() => {
                println!("  Respawned doli-node.");
                any_ok = true;
            }
            _ => println!("  Failed to respawn. Start manually: {}", cmdline),
        }
    }
    any_ok
}

/// Get the real user's UID (for launchctl kickstart), even under sudo.
#[cfg(target_os = "macos")]
fn get_uid() -> String {
    if let Ok(uid) = std::env::var("SUDO_UID") {
        if !uid.is_empty() && uid != "0" {
            return uid;
        }
    }
    if let Ok(user) = std::env::var("SUDO_USER") {
        if !user.is_empty() && user != "root" {
            if let Ok(output) = std::process::Command::new("id")
                .args(["-u", &user])
                .output()
            {
                if output.status.success() {
                    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !uid.is_empty() {
                        return uid;
                    }
                }
            }
        }
    }
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "501".to_string())
}
