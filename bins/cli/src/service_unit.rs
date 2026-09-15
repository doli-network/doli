//! The systemd unit template for `doli service install`.
//!
//! Lifted out of `cmd_service.rs` so the unit text is a pure, testable value: the binary
//! writes `/etc/systemd/system` as root, which no test can execute.

use std::path::Path;

/// Render the node service unit.
///
/// The skills path appears twice on purpose (REQ-SKILLS-002): `Environment=` tells the
/// node where to install, and `ReadWritePaths=` is what makes the path writable under
/// `ProtectSystem=full`. It is quoted, because a home directory may contain a space and
/// systemd splits that line on whitespace.
pub fn render_service_unit(
    network: &str,
    exec_start: &str,
    data_dir: &str,
    user: &str,
    group: &str,
    skills_dir: &Path,
) -> String {
    let skills = skills_dir.display().to_string();
    format!(
        r#"[Unit]
Description=DOLI {network} Node
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=600
StartLimitBurst=5

[Service]
Type=simple
User={user}
Group={group}
Environment=DOLI_SKILLS_DIR={skills}
ExecStart={exec_start}
Restart=always
RestartSec=10
StandardOutput=append:/var/log/doli/{network}.log
StandardError=append:/var/log/doli/{network}.log
NoNewPrivileges=true
ProtectSystem=full
ReadWritePaths={data_dir} /var/log/doli "{skills}"
PrivateTmp=true
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
"#
    )
}
