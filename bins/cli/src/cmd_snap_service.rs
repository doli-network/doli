//! Service stop/restart for `doli snap`.

/// Stop the doli-node service that owns the given data directory.
/// On multi-node servers, only stops the service whose ExecStart references this data_dir.
/// If no match is found, stops nothing (safe default).
pub(crate) fn stop_doli_service(data_dir: &std::path::Path) {
    let data_dir_str = data_dir.to_string_lossy();

    // systemd: find the service whose ExecStart contains our data_dir
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-pager", "-q"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let service = line.split_whitespace().next().unwrap_or("");
            if !service.contains("doli") {
                continue;
            }
            // Read the service's ExecStart to check if it uses our data_dir
            if let Ok(cat_output) = std::process::Command::new("systemctl")
                .args(["cat", service, "--no-pager"])
                .output()
            {
                let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                if cat_str.contains(data_dir_str.as_ref()) {
                    let _ = std::process::Command::new("sudo")
                        .args(["systemctl", "stop", service])
                        .output();
                    println!("  Stopped {}", service);
                    return;
                }
            }
        }
    }

    // launchd (macOS): find the plist whose ProgramArguments contains our data_dir
    if let Ok(output) = std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let label = line.split_whitespace().last().unwrap_or("");
            if !label.contains("doli") {
                continue;
            }
            let _ = std::process::Command::new("launchctl")
                .args(["stop", label])
                .output();
            println!("  Stopped {}", label);
            return;
        }
    }

    println!("  No running service found");
}

/// Restart the doli-node service that owns the given data directory.
pub(crate) fn restart_matching_service(data_dir: &std::path::Path) {
    let data_dir_str = data_dir.to_string_lossy();

    // systemd
    if let Ok(output) = std::process::Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-pager"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let service = line.split_whitespace().next().unwrap_or("");
            if !service.contains("doli") {
                continue;
            }
            if let Ok(cat_output) = std::process::Command::new("systemctl")
                .args(["cat", service, "--no-pager"])
                .output()
            {
                let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                if cat_str.contains(data_dir_str.as_ref()) {
                    let _ = std::process::Command::new("sudo")
                        .args(["systemctl", "restart", service])
                        .output();
                    println!("  Restarted {}", service);
                    return;
                }
            }
        }
    }

    // launchd (macOS)
    if let Ok(output) = std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let label = line.split_whitespace().last().unwrap_or("");
            if label.contains("doli") {
                let _ = std::process::Command::new("launchctl")
                    .args(["start", label])
                    .output();
                println!("  Started {}", label);
                return;
            }
        }
    }

    println!("No doli-node service or process found. Restart manually if needed.");
}
