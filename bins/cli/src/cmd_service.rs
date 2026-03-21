use anyhow::{bail, Result};

use crate::commands::ServiceCommand;

/// Entry point for `doli service <subcommand>`.
pub(crate) fn cmd_service(network: &str, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install {
            network: net,
            name,
            data_dir,
            producer_key,
            p2p_port,
            rpc_port,
        } => {
            // Use the subcommand's --network if provided, otherwise fall back to the global flag
            let net = if net == "mainnet" && network != "mainnet" {
                network.to_string()
            } else {
                net
            };
            cmd_install(&net, name, data_dir, producer_key, p2p_port, rpc_port)
        }
        ServiceCommand::Uninstall { name } => cmd_uninstall(network, name),
        ServiceCommand::Start { name } => cmd_start(network, name),
        ServiceCommand::Stop { name } => cmd_stop(network, name),
        ServiceCommand::Restart { name } => cmd_restart(network, name),
        ServiceCommand::Status { name } => cmd_status(network, name),
        ServiceCommand::Logs {
            name,
            follow,
            lines,
        } => cmd_logs(network, name, follow, lines),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_service_name(network: &str, name: Option<String>) -> String {
    name.unwrap_or_else(|| format!("doli-{}", network))
}

fn is_linux() -> bool {
    cfg!(target_os = "linux")
}

fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

fn check_sudo() -> Result<()> {
    // On macOS user-scoped launchd doesn't require sudo
    if is_macos() {
        return Ok(());
    }
    // On Linux, check effective UID via `id -u`
    let uid = get_uid();
    if uid != "0" {
        bail!("This command requires root privileges.\n  Try: sudo doli service install ...");
    }
    Ok(())
}

fn get_uid() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "501".to_string())
}

fn launchd_label(network: &str, name: Option<&str>) -> String {
    name.map(|n| n.to_string())
        .unwrap_or_else(|| format!("network.doli.{}", network))
}

fn launchd_plist_path(label: &str) -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join("Library/LaunchAgents")
        .join(format!("{}.plist", label))
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

fn cmd_install(
    network: &str,
    name: Option<String>,
    data_dir: Option<String>,
    producer_key: Option<String>,
    p2p_port: Option<u16>,
    rpc_port: Option<u16>,
) -> Result<()> {
    check_sudo()?;

    if is_linux() {
        install_systemd(network, name, data_dir, producer_key, p2p_port, rpc_port)
    } else if is_macos() {
        install_launchd(network, name, data_dir, producer_key, p2p_port, rpc_port)
    } else {
        bail!("Unsupported platform. Only Linux (systemd) and macOS (launchd) are supported.");
    }
}

fn build_exec_args(
    network: &str,
    data_dir: &Option<String>,
    producer_key: &Option<String>,
    p2p_port: Option<u16>,
    rpc_port: Option<u16>,
) -> Vec<String> {
    let mut args = vec!["--network".to_string(), network.to_string()];
    if let Some(ref dir) = data_dir {
        args.push("--data-dir".to_string());
        args.push(dir.clone());
    }
    // "run" subcommand and producer flags
    args.push("run".to_string());
    args.push("--producer".to_string());
    args.push("--yes".to_string());
    args.push("--force-start".to_string());
    if let Some(ref key) = producer_key {
        args.push("--producer-key".to_string());
        args.push(key.clone());
    }
    if let Some(port) = p2p_port {
        args.push("--p2p-port".to_string());
        args.push(port.to_string());
    }
    if let Some(port) = rpc_port {
        args.push("--rpc-port".to_string());
        args.push(port.to_string());
    }
    args
}

fn install_systemd(
    network: &str,
    name: Option<String>,
    data_dir: Option<String>,
    producer_key: Option<String>,
    p2p_port: Option<u16>,
    rpc_port: Option<u16>,
) -> Result<()> {
    let service_name = resolve_service_name(network, name);
    let unit_path = format!("/etc/systemd/system/{}.service", service_name);

    let default_data_dir = format!("/var/lib/doli/{}", network);
    let actual_data_dir = data_dir.clone().unwrap_or_else(|| default_data_dir.clone());

    let exec_args = build_exec_args(network, &data_dir, &producer_key, p2p_port, rpc_port);
    let exec_start = format!("/usr/local/bin/doli-node {}", exec_args.join(" \\\n  "));

    let unit = format!(
        r#"[Unit]
Description=DOLI {network} Node
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=600
StartLimitBurst=5

[Service]
Type=simple
User=doli
Group=doli
ExecStart={exec_start}
Restart=always
RestartSec=10
StandardOutput=append:/var/log/doli/{network}.log
StandardError=append:/var/log/doli/{network}.log
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths={data_dir} /var/log/doli
PrivateTmp=true
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
"#,
        network = network,
        exec_start = exec_start,
        data_dir = actual_data_dir,
    );

    // Ensure log directory exists
    let _ = std::fs::create_dir_all("/var/log/doli");

    println!("Writing service file: {}", unit_path);
    std::fs::write(&unit_path, &unit)?;

    println!("Reloading systemd daemon...");
    run_cmd("systemctl", &["daemon-reload"])?;

    println!("Enabling {}...", service_name);
    run_cmd("systemctl", &["enable", &service_name])?;

    println!("Starting {}...", service_name);
    run_cmd("systemctl", &["start", &service_name])?;

    println!();
    println!("Service {} installed and started.", service_name);
    println!("  Unit file: {}", unit_path);
    println!("  Logs:      journalctl -u {} -f", service_name);
    println!("  Status:    doli service status --name {}", service_name);

    Ok(())
}

fn install_launchd(
    network: &str,
    name: Option<String>,
    data_dir: Option<String>,
    producer_key: Option<String>,
    p2p_port: Option<u16>,
    rpc_port: Option<u16>,
) -> Result<()> {
    let label = launchd_label(network, name.as_deref());
    let plist_path = launchd_plist_path(&label);

    let exec_args = build_exec_args(network, &data_dir, &producer_key, p2p_port, rpc_port);

    // Build ProgramArguments entries
    let mut program_args = vec!["    <string>/usr/local/bin/doli-node</string>".to_string()];
    for arg in &exec_args {
        program_args.push(format!("    <string>{}</string>", arg));
    }
    let program_args_str = program_args.join("\n");

    let home = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    let log_dir = format!("{}/Library/Logs/doli", home);
    let _ = std::fs::create_dir_all(&log_dir);

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program_args}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log_dir}/{network}.log</string>
  <key>StandardErrorPath</key>
  <string>{log_dir}/{network}.log</string>
  <key>SoftResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>65535</integer>
  </dict>
</dict>
</plist>
"#,
        label = label,
        program_args = program_args_str,
        log_dir = log_dir,
        network = network,
    );

    // Ensure LaunchAgents directory exists
    if let Some(parent) = plist_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    println!("Writing plist: {}", plist_path.display());
    std::fs::write(&plist_path, &plist)?;

    println!("Loading {}...", label);
    let uid = get_uid();
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{}/{}", uid, label)])
        .output(); // ignore if not loaded
    run_cmd(
        "launchctl",
        &[
            "bootstrap",
            &format!("gui/{}", uid),
            &plist_path.to_string_lossy(),
        ],
    )?;

    println!();
    println!("Service {} installed and started.", label);
    println!("  Plist: {}", plist_path.display());
    println!("  Logs:  tail -f {}/{}.log", log_dir, network);
    println!("  Status: doli service status");

    Ok(())
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(network: &str, name: Option<String>) -> Result<()> {
    if is_linux() {
        check_sudo()?;
        let service_name = resolve_service_name(network, name);
        let unit_path = format!("/etc/systemd/system/{}.service", service_name);

        println!("Stopping {}...", service_name);
        let _ = std::process::Command::new("systemctl")
            .args(["stop", &service_name])
            .status();

        println!("Disabling {}...", service_name);
        let _ = std::process::Command::new("systemctl")
            .args(["disable", &service_name])
            .status();

        if std::path::Path::new(&unit_path).exists() {
            println!("Removing {}...", unit_path);
            std::fs::remove_file(&unit_path)?;
        }

        println!("Reloading systemd daemon...");
        run_cmd("systemctl", &["daemon-reload"])?;

        println!("Service {} uninstalled.", service_name);
    } else if is_macos() {
        let label = launchd_label(network, name.as_deref());
        let plist_path = launchd_plist_path(&label);

        let uid = get_uid();
        println!("Unloading {}...", label);
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{}", uid, label)])
            .status();

        if plist_path.exists() {
            println!("Removing {}...", plist_path.display());
            std::fs::remove_file(&plist_path)?;
        }

        println!("Service {} uninstalled.", label);
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// start / stop / restart
// ---------------------------------------------------------------------------

fn cmd_start(network: &str, name: Option<String>) -> Result<()> {
    if is_linux() {
        let service_name = resolve_service_name(network, name);
        println!("Starting {}...", service_name);
        // Try without sudo first (polkit), fall back to sudo
        let status = std::process::Command::new("systemctl")
            .args(["start", &service_name])
            .status();
        match status {
            Ok(s) if s.success() => println!("Started {}.", service_name),
            _ => {
                println!("  Retrying with sudo...");
                run_cmd("sudo", &["systemctl", "start", &service_name])?;
                println!("Started {}.", service_name);
            }
        }
    } else if is_macos() {
        let label = launchd_label(network, name.as_deref());
        println!("Starting {}...", label);
        let uid = get_uid();
        let plist_path = launchd_plist_path(&label);
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args([
                    "bootstrap",
                    &format!("gui/{}", uid),
                    &plist_path.to_string_lossy(),
                ])
                .status();
        } else {
            let _ = std::process::Command::new("launchctl")
                .args(["start", &label])
                .status();
        }
        println!("Started {}.", label);
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

fn cmd_stop(network: &str, name: Option<String>) -> Result<()> {
    if is_linux() {
        let service_name = resolve_service_name(network, name);
        println!("Stopping {}...", service_name);
        let status = std::process::Command::new("systemctl")
            .args(["stop", &service_name])
            .status();
        match status {
            Ok(s) if s.success() => println!("Stopped {}.", service_name),
            _ => {
                println!("  Retrying with sudo...");
                run_cmd("sudo", &["systemctl", "stop", &service_name])?;
                println!("Stopped {}.", service_name);
            }
        }
    } else if is_macos() {
        let label = launchd_label(network, name.as_deref());
        println!("Stopping {}...", label);
        let uid = get_uid();
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{}", uid, label)])
            .status();
        println!("Stopped {}.", label);
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

fn cmd_restart(network: &str, name: Option<String>) -> Result<()> {
    if is_linux() {
        let service_name = resolve_service_name(network, name);
        println!("Restarting {}...", service_name);
        let status = std::process::Command::new("systemctl")
            .args(["restart", &service_name])
            .status();
        match status {
            Ok(s) if s.success() => println!("Restarted {}.", service_name),
            _ => {
                println!("  Retrying with sudo...");
                run_cmd("sudo", &["systemctl", "restart", &service_name])?;
                println!("Restarted {}.", service_name);
            }
        }
    } else if is_macos() {
        let label = launchd_label(network, name.as_deref());
        println!("Restarting {}...", label);
        let uid = get_uid();
        let _ = std::process::Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{}/{}", uid, label)])
            .status();
        println!("Restarted {}.", label);
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

fn cmd_status(network: &str, name: Option<String>) -> Result<()> {
    if is_linux() {
        let service_name = resolve_service_name(network, name);
        let output = std::process::Command::new("systemctl")
            .args(["status", &service_name, "--no-pager"])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.is_empty() {
            println!("{}", stdout);
        }
        if !stderr.is_empty() {
            eprintln!("{}", stderr);
        }
    } else if is_macos() {
        let label = launchd_label(network, name.as_deref());
        let output = std::process::Command::new("launchctl")
            .args(["list"])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut found = false;
        println!("{:<8} {:<8} Label", "PID", "Status");
        println!("{:-<8} {:-<8} {:-<40}", "", "", "");
        for line in stdout.lines() {
            if line.contains(&label) {
                println!("{}", line);
                found = true;
            }
        }
        if !found {
            println!("Service {} not found in launchctl list.", label);
        }

        // Also show if the plist file exists
        let plist_path = launchd_plist_path(&label);
        if plist_path.exists() {
            println!("\nPlist: {}", plist_path.display());
        } else {
            println!("\nPlist not found at {}", plist_path.display());
        }
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

fn cmd_logs(network: &str, name: Option<String>, follow: bool, lines: u32) -> Result<()> {
    if is_linux() {
        let service_name = resolve_service_name(network, name);

        // Check if there's a log file first (our service template uses StandardOutput=append)
        let log_file = format!("/var/log/doli/{}.log", network);
        if std::path::Path::new(&log_file).exists() {
            println!("Reading from {}:", log_file);
            let mut args = vec!["-n".to_string(), lines.to_string()];
            if follow {
                args.push("-f".to_string());
            }
            args.push(log_file);

            let status = std::process::Command::new("tail").args(&args).status()?;
            if !status.success() {
                bail!("Failed to read log file.");
            }
        } else {
            // Fall back to journalctl
            let mut args = vec![
                "-u".to_string(),
                service_name,
                "--no-pager".to_string(),
                "-n".to_string(),
                lines.to_string(),
            ];
            if follow {
                args.push("-f".to_string());
            }

            let status = std::process::Command::new("journalctl")
                .args(&args)
                .status()?;
            if !status.success() {
                bail!("Failed to read journalctl logs.");
            }
        }
    } else if is_macos() {
        let home = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        let log_file = format!("{}/Library/Logs/doli/{}.log", home, network);

        if !std::path::Path::new(&log_file).exists() {
            bail!(
                "Log file not found: {}\nIs the service installed? Run: doli service install --network {}",
                log_file,
                network
            );
        }

        println!("Reading from {}:", log_file);
        let mut args = vec!["-n".to_string(), lines.to_string()];
        if follow {
            args.push("-f".to_string());
        }
        args.push(log_file);

        let status = std::process::Command::new("tail").args(&args).status()?;
        if !status.success() {
            bail!("Failed to read log file.");
        }
    } else {
        bail!("Unsupported platform.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run_cmd helper
// ---------------------------------------------------------------------------

fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    if !status.success() {
        bail!("Command failed: {} {}", program, args.join(" "));
    }
    Ok(())
}
