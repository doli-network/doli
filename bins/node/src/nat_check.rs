//! P2P port reachability check for producer nodes.
//!
//! Verifies that the node's P2P port is reachable from the internet by:
//! 1. Discovering our public IP via HTTP services
//! 2. Attempting a TCP connect to our own public IP:port
//!
//! Blocks startup with a fatal error if the port is unreachable,
//! preventing producers from running with closed ports (which causes
//! orphaned blocks due to poor gossip mesh connectivity).

use std::time::Duration;

use anyhow::{anyhow, Result};
use tracing::info;

const IP_SERVICES: &[&str] = &[
    "https://api.ipify.org",
    "https://icanhazip.com",
    "https://ifconfig.me/ip",
];

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const TCP_TIMEOUT: Duration = Duration::from_secs(5);

/// Discover our public IP by querying external services.
async fn get_public_ip() -> Result<String> {
    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;

    for url in IP_SERVICES {
        match client.get(*url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    let ip = body.trim().to_string();
                    if !ip.is_empty() {
                        return Ok(ip);
                    }
                }
            }
            _ => continue,
        }
    }

    Err(anyhow!(
        "could not determine public IP (all services failed)"
    ))
}

/// Print a fatal box message to stderr explaining the port check failure.
fn print_fatal_message(port: u16, ip: &str, reason: &str) {
    let lines = [
        String::new(),
        "  FATAL: P2P port is NOT reachable from the internet".to_string(),
        format!("  Checked: {}:{}", ip, port),
        format!("  Reason:  {}", reason),
        String::new(),
        "  A producer with a closed P2P port cannot propagate blocks.".to_string(),
        "  Other nodes cannot connect to you, your blocks arrive late".to_string(),
        "  or get orphaned, and you waste your production slot.".to_string(),
        String::new(),
        format!(
            "  Fix: open TCP port {} (inbound) on your router/firewall.",
            port
        ),
        "  Override: --override-nat-check (only if you have custom NAT)".to_string(),
        String::new(),
    ];

    let width = lines.iter().map(|l| l.len()).max().unwrap_or(60).max(60);
    let border = "═".repeat(width + 2);

    eprintln!();
    eprintln!("╔{}╗", border);
    for line in &lines {
        eprintln!("║ {:<width$} ║", line, width = width);
    }
    eprintln!("╚{}╝", border);
    eprintln!();
}

/// Check that our P2P port is reachable from the internet.
///
/// Called only for mainnet + producer nodes. Exits the process on failure.
pub async fn check_port_reachability(port: u16) -> Result<()> {
    info!("Checking P2P port {} reachability...", port);

    let ip = match get_public_ip().await {
        Ok(ip) => {
            info!("Public IP: {}", ip);
            ip
        }
        Err(e) => {
            print_fatal_message(port, "unknown", &format!("cannot detect public IP: {}", e));
            return Err(anyhow!("P2P port reachability check failed: {}", e));
        }
    };

    let addr = format!("{}:{}", ip, port);
    match tokio::time::timeout(TCP_TIMEOUT, tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => {
            info!("P2P port {} is reachable at {}", port, addr);
            Ok(())
        }
        Ok(Err(e)) => {
            print_fatal_message(port, &ip, &format!("connection refused: {}", e));
            Err(anyhow!(
                "P2P port {} is not reachable at {}: {}",
                port,
                addr,
                e
            ))
        }
        Err(_) => {
            print_fatal_message(port, &ip, "connection timed out (5s)");
            Err(anyhow!(
                "P2P port {} is not reachable at {}: timed out",
                port,
                addr
            ))
        }
    }
}
