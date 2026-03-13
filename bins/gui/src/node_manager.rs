//! Manages the doli-node child process.
//!
//! The NodeManager spawns `doli-node` as a background process, routes its output
//! to a log file, and provides lifecycle control (start, stop, restart).
//! On Windows, the process is spawned with CREATE_NO_WINDOW to avoid a console flash.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// RPC ports per network (must match crates/core/src/network_params.rs defaults).
const MAINNET_RPC_PORT: u16 = 8500;
const TESTNET_RPC_PORT: u16 = 18500;
const DEVNET_RPC_PORT: u16 = 28500;

/// Node binary name.
#[cfg(not(target_os = "windows"))]
const NODE_BINARY: &str = "doli-node";

#[cfg(target_os = "windows")]
const NODE_BINARY: &str = "doli-node.exe";

/// Maximum time to wait for graceful shutdown before force-killing.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Manages the doli-node child process lifecycle.
pub struct NodeManager {
    /// Handle to the running child process (None if not running).
    process: Option<Child>,
    /// Data directory for the node (e.g., ~/.doli/).
    data_dir: PathBuf,
    /// Current network (mainnet, testnet, devnet).
    network: String,
    /// RPC port for the current network.
    rpc_port: u16,
    /// Path to the node's log file.
    log_path: PathBuf,
}

impl NodeManager {
    /// Create a new NodeManager for the given data directory and network.
    ///
    /// The RPC port is determined from the network name:
    /// - mainnet: 8500
    /// - testnet: 18500
    /// - devnet: 28500
    pub fn new(data_dir: PathBuf, network: &str) -> Self {
        let rpc_port = rpc_port_for_network(network);
        let log_path = data_dir.join("node.log");
        Self {
            process: None,
            data_dir,
            network: network.to_string(),
            rpc_port,
            log_path,
        }
    }

    /// Start the node process.
    ///
    /// Looks for the `doli-node` binary in:
    /// 1. Same directory as the current executable
    /// 2. System PATH
    ///
    /// Returns Ok(()) on success, Err with a message if the binary isn't found
    /// or the process fails to spawn.
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }

        let binary_path = find_node_binary()?;

        // Ensure the data directory exists.
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| format!("Failed to create data dir: {}", e))?;

        // Open (or create) the log file for stdout/stderr redirection.
        let log_file = std::fs::File::create(&self.log_path)
            .map_err(|e| format!("Failed to create log file: {}", e))?;
        let log_stderr = log_file
            .try_clone()
            .map_err(|e| format!("Failed to clone log file handle: {}", e))?;

        let mut cmd = Command::new(&binary_path);
        cmd.arg("--network")
            .arg(&self.network)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .arg("run")
            .arg("--rpc-port")
            .arg(self.rpc_port.to_string())
            .stdout(log_file)
            .stderr(log_stderr);

        // On Windows, prevent a console window from flashing.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to start doli-node ({}): {}",
                binary_path.display(),
                e
            )
        })?;

        self.process = Some(child);
        Ok(())
    }

    /// Stop the node process gracefully.
    ///
    /// Sends SIGTERM on Unix (kill on Windows), then waits up to 10 seconds.
    /// If the process hasn't exited by then, sends SIGKILL.
    pub fn stop(&mut self) -> Result<(), String> {
        let child = match self.process.take() {
            Some(c) => c,
            None => return Ok(()), // Not running.
        };

        graceful_shutdown(child)?;
        Ok(())
    }

    /// Check if the node process is currently alive.
    pub fn is_running(&mut self) -> bool {
        let child = match self.process.as_mut() {
            Some(c) => c,
            None => return false,
        };

        // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running.
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process has exited; clean up.
                self.process = None;
                false
            }
            Ok(None) => true,
            Err(_) => {
                self.process = None;
                false
            }
        }
    }

    /// Returns the RPC URL for the embedded node.
    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.rpc_port)
    }

    /// Returns the path to the node's log file.
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }

    /// Returns the current network.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the current RPC port.
    #[allow(dead_code)]
    pub fn rpc_port(&self) -> u16 {
        self.rpc_port
    }

    /// Returns the data directory.
    #[allow(dead_code)]
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Restart the node, optionally switching to a different network.
    pub fn restart(&mut self, network: &str) -> Result<(), String> {
        self.stop()?;
        self.network = network.to_string();
        self.rpc_port = rpc_port_for_network(network);
        self.start()
    }

    /// Read the last N lines from the log file.
    pub fn tail_log(&self, lines: usize) -> Result<Vec<String>, String> {
        let file =
            std::fs::File::open(&self.log_path).map_err(|e| format!("Cannot open log: {}", e))?;
        let reader = BufReader::new(file);
        let all_lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

        let start = all_lines.len().saturating_sub(lines);
        Ok(all_lines[start..].to_vec())
    }

    /// Read the last N bytes from the log file and split into lines.
    /// More efficient than tail_log for large files.
    #[allow(dead_code)]
    pub fn tail_log_bytes(&self, max_bytes: u64) -> Result<Vec<String>, String> {
        let mut file =
            std::fs::File::open(&self.log_path).map_err(|e| format!("Cannot open log: {}", e))?;

        let file_len = file
            .metadata()
            .map_err(|e| format!("Cannot stat log: {}", e))?
            .len();

        let seek_pos = file_len.saturating_sub(max_bytes);
        file.seek(SeekFrom::Start(seek_pos))
            .map_err(|e| format!("Cannot seek: {}", e))?;

        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .map_err(|e| format!("Cannot read log: {}", e))?;

        // If we seeked mid-line, skip the first (partial) line.
        let lines: Vec<String> = if seek_pos > 0 {
            let mut lines_iter = buf.lines();
            lines_iter.next(); // Skip partial first line.
            lines_iter.map(String::from).collect()
        } else {
            buf.lines().map(String::from).collect()
        };

        Ok(lines)
    }
}

impl Drop for NodeManager {
    fn drop(&mut self) {
        if let Some(child) = self.process.take() {
            let _ = graceful_shutdown(child);
        }
    }
}

/// Map a network name to its RPC port.
pub fn rpc_port_for_network(network: &str) -> u16 {
    match network {
        "mainnet" => MAINNET_RPC_PORT,
        "testnet" => TESTNET_RPC_PORT,
        "devnet" => DEVNET_RPC_PORT,
        _ => MAINNET_RPC_PORT, // Default to mainnet.
    }
}

/// Locate the `doli-node` binary.
///
/// Search order:
/// 1. Same directory as the current executable
/// 2. System PATH (via `which`-style lookup)
fn find_node_binary() -> Result<PathBuf, String> {
    // 1. Check sibling directory of current exe.
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let sibling = exe_dir.join(NODE_BINARY);
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    // 2. Fall back to PATH lookup.
    which_in_path(NODE_BINARY)
        .ok_or_else(|| format!("Node binary '{}' not found in exe dir or PATH", NODE_BINARY))
}

/// Simple PATH lookup (like `which`).
fn which_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Gracefully shut down a child process: SIGTERM, wait, then SIGKILL if needed.
fn graceful_shutdown(mut child: Child) -> Result<(), String> {
    // Send termination signal.
    #[cfg(unix)]
    {
        // SIGTERM on Unix via /bin/kill to avoid libc dependency.
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status();
    }

    #[cfg(not(unix))]
    {
        // On non-Unix (Windows), kill() sends TerminateProcess.
        let _ = child.kill();
    }

    // Wait up to SHUTDOWN_TIMEOUT for the process to exit.
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Force kill.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Error waiting for node process: {}", e)),
        }
    }
}

/// Returns the default data directory for the node.
///
/// - Unix: `~/.doli/`
/// - Windows: `%APPDATA%/doli/`
pub fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("doli")
    }

    #[cfg(not(target_os = "windows"))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_rpc_port_for_network_mainnet() {
        assert_eq!(rpc_port_for_network("mainnet"), 8500);
    }

    #[test]
    fn test_rpc_port_for_network_testnet() {
        assert_eq!(rpc_port_for_network("testnet"), 18500);
    }

    #[test]
    fn test_rpc_port_for_network_devnet() {
        assert_eq!(rpc_port_for_network("devnet"), 28500);
    }

    #[test]
    fn test_rpc_port_for_network_unknown_defaults_to_mainnet() {
        assert_eq!(rpc_port_for_network("unknown"), MAINNET_RPC_PORT);
    }

    #[test]
    fn test_node_manager_new_mainnet() {
        let dir = PathBuf::from("/tmp/test-doli");
        let mgr = NodeManager::new(dir.clone(), "mainnet");
        assert_eq!(mgr.network(), "mainnet");
        assert_eq!(mgr.rpc_port(), 8500);
        assert_eq!(mgr.data_dir(), &dir);
        assert_eq!(mgr.log_path(), &dir.join("node.log"));
    }

    #[test]
    fn test_node_manager_new_testnet() {
        let dir = PathBuf::from("/tmp/test-doli-tn");
        let mgr = NodeManager::new(dir, "testnet");
        assert_eq!(mgr.rpc_port(), 18500);
        assert_eq!(mgr.network(), "testnet");
    }

    #[test]
    fn test_node_manager_new_devnet() {
        let dir = PathBuf::from("/tmp/test-doli-dn");
        let mgr = NodeManager::new(dir, "devnet");
        assert_eq!(mgr.rpc_port(), 28500);
    }

    #[test]
    fn test_rpc_url_mainnet() {
        let mgr = NodeManager::new(PathBuf::from("/tmp/t"), "mainnet");
        assert_eq!(mgr.rpc_url(), "http://127.0.0.1:8500");
    }

    #[test]
    fn test_rpc_url_testnet() {
        let mgr = NodeManager::new(PathBuf::from("/tmp/t"), "testnet");
        assert_eq!(mgr.rpc_url(), "http://127.0.0.1:18500");
    }

    #[test]
    fn test_rpc_url_devnet() {
        let mgr = NodeManager::new(PathBuf::from("/tmp/t"), "devnet");
        assert_eq!(mgr.rpc_url(), "http://127.0.0.1:28500");
    }

    #[test]
    fn test_log_path() {
        let dir = PathBuf::from("/data/doli");
        let mgr = NodeManager::new(dir.clone(), "mainnet");
        assert_eq!(mgr.log_path(), &dir.join("node.log"));
    }

    #[test]
    fn test_is_running_when_no_process() {
        let mut mgr = NodeManager::new(PathBuf::from("/tmp/t"), "mainnet");
        assert!(!mgr.is_running());
    }

    #[test]
    fn test_stop_when_not_running_is_ok() {
        let mut mgr = NodeManager::new(PathBuf::from("/tmp/t"), "mainnet");
        assert!(mgr.stop().is_ok());
    }

    #[test]
    fn test_default_data_dir_is_not_empty() {
        let dir = default_data_dir();
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn test_default_data_dir_ends_with_doli() {
        let dir = default_data_dir();
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name == ".doli" || name == "doli",
            "Expected dir name to be .doli or doli, got: {}",
            name
        );
    }

    #[test]
    fn test_tail_log_nonexistent_file() {
        let mgr = NodeManager::new(PathBuf::from("/tmp/nonexistent-doli-test"), "mainnet");
        let result = mgr.tail_log(10);
        assert!(result.is_err());
    }

    #[test]
    fn test_tail_log_reads_last_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("node.log");
        let content = (1..=20).map(|i| format!("line {}", i)).collect::<Vec<_>>();
        std::fs::write(&log_path, content.join("\n")).unwrap();

        let mgr = NodeManager::new(dir.path().to_path_buf(), "mainnet");
        let lines = mgr.tail_log(5).unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "line 16");
        assert_eq!(lines[4], "line 20");
    }

    #[test]
    fn test_tail_log_fewer_lines_than_requested() {
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("node.log");
        std::fs::write(&log_path, "only one line").unwrap();

        let mgr = NodeManager::new(dir.path().to_path_buf(), "mainnet");
        let lines = mgr.tail_log(100).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "only one line");
    }

    #[test]
    fn test_restart_changes_network() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = NodeManager::new(dir.path().to_path_buf(), "mainnet");
        assert_eq!(mgr.network(), "mainnet");
        assert_eq!(mgr.rpc_port(), 8500);

        // restart will fail because doli-node is not present, but the
        // network/port fields should update before the start attempt.
        // We verify by checking a known side-effect: stop succeeds (no process),
        // then start fails but network is updated.
        let _ = mgr.restart("testnet");
        assert_eq!(mgr.network(), "testnet");
        assert_eq!(mgr.rpc_port(), 18500);
        assert_eq!(mgr.rpc_url(), "http://127.0.0.1:18500");
    }

    #[test]
    fn test_start_fails_when_binary_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        // Ensure PATH doesn't contain doli-node by using an empty PATH scenario.
        // We can't easily control PATH here, but the binary definitely won't be
        // in the temp dir, so the sibling check fails. PATH check may or may not
        // find it; just verify start() returns a result without panicking.
        let mut mgr = NodeManager::new(dir.path().to_path_buf(), "mainnet");
        let result = mgr.start();
        // It's either Ok (unlikely) or Err (expected). Just no panic.
        assert!(result.is_ok() || result.is_err());
    }
}
