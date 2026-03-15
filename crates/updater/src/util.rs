//! Utility functions

/// Get current Unix timestamp
pub fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Get current version from the running binary
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare versions (simple semver comparison)
pub fn is_newer_version(new: &str, current: &str) -> bool {
    // Parse as semver and compare
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<&str> = v.trim_start_matches('v').split('.').collect();
        (
            parts.first().and_then(|s| s.parse().ok()).unwrap_or(0),
            parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
            parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
        )
    };

    parse(new) > parse(current)
}

/// Get platform identifier for binary download
pub fn platform_identifier() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x64";

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-arm64";

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x64";

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-arm64";

    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    return "unknown";
}
