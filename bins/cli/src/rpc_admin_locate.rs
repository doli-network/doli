//! INC-I-308: find the data dir of the local node that listens on a loopback RPC port.
//!
//! Security rule: a listener is accepted only when its uid owns `<dir>/rpc-admin.token`.
//! Without it, any local user can bind a free port with a forged `--data-dir` and make
//! a root CLI send another node's token to it.

use std::path::{Path, PathBuf};

use doli_cli::rpc_admin_auth::TOKEN_FILE_NAME;

/// A process that listens on a TCP port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    #[allow(dead_code)]
    pub pid: u32,
    pub uid: u32,
    pub args: Vec<String>,
}

/// Data dir whose token file the CLI reads for a loopback `endpoint`:
/// `DOLI_DATA_DIR`, else the accepted listener's data dir, else `fallback()`.
pub fn admin_data_dir(endpoint: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    if let Some(dir) = std::env::var_os("DOLI_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    endpoint_port(endpoint)
        .and_then(|port| listener_data_dir(port, &fallback_default))
        .unwrap_or_else(fallback)
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    reqwest::Url::parse(endpoint).ok()?.port_or_known_default()
}

fn fallback_default(network: &str) -> PathBuf {
    crate::paths::resolve_base_dir(network, None)
}

/// Data dir of the accepted listener on `port`, if any.
pub fn listener_data_dir(port: u16, default_dir: &dyn Fn(&str) -> PathBuf) -> Option<PathBuf> {
    find_listeners(port).into_iter().find_map(|l| {
        let dir = data_dir_from_args(&l.args, default_dir);
        accept_listener(l.uid, &dir).then_some(dir)
    })
}

/// True iff `<dir>/rpc-admin.token` exists and is owned by `listener_uid`.
#[cfg(unix)]
pub fn accept_listener(listener_uid: u32, dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(dir.join(TOKEN_FILE_NAME)).is_ok_and(|m| m.uid() == listener_uid)
}

#[cfg(not(unix))]
pub fn accept_listener(_listener_uid: u32, _dir: &Path) -> bool {
    false
}

/// Node data dir from its argv, by the rule of `bins/node/src/main.rs`:
/// `--data-dir X` / `--data-dir=X` / `-d X`, else the default dir of `--network`.
pub fn data_dir_from_args(args: &[String], default_dir: &dyn Fn(&str) -> PathBuf) -> PathBuf {
    if let Some(dir) = flag_value(args, "--data-dir", Some("-d")) {
        return expand_tilde(&dir);
    }
    let net = flag_value(args, "--network", Some("-n"))
        .unwrap_or_else(|| "mainnet".to_string())
        .to_ascii_lowercase();
    default_dir(&net)
}

fn flag_value(args: &[String], long: &str, short: Option<&str>) -> Option<String> {
    let eq = format!("{long}=");
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == long || Some(a.as_str()) == short {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_string());
        }
    }
    None
}

fn expand_tilde(dir: &str) -> PathBuf {
    match (dir.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(dir),
    }
}

/// Argv from a `ps -o command=` line. ps prints argv unquoted, so a value after
/// `--data-dir`/`-d` runs to the next flag; the longest existing-dir prefix wins.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn args_from_ps_command(cmd: &str) -> Vec<String> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        out.push(w.to_string());
        i += 1;
        if w != "--data-dir" && w != "-d" {
            continue;
        }
        let end = (i..words.len())
            .find(|&j| words[j].starts_with('-'))
            .unwrap_or(words.len());
        let take = (i + 1..=end)
            .rev()
            .find(|&j| Path::new(&words[i..j].join(" ")).is_dir())
            .unwrap_or((i + 1).min(end));
        if take > i {
            out.push(words[i..take].join(" "));
        }
        i = take;
    }
    out
}

#[cfg(target_os = "linux")]
pub fn find_listeners(port: u16) -> Vec<Listener> {
    let mut inodes = Vec::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            inodes.extend(listen_inodes(&text, port));
        }
    }
    if inodes.is_empty() {
        return Vec::new();
    }
    let wanted: Vec<String> = inodes.iter().map(|i| format!("socket:[{i}]")).collect();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in procs.flatten() {
        let Some(pid) = p.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(p.path().join("fd")) else {
            continue;
        };
        let owns = fds.flatten().any(|fd| {
            std::fs::read_link(fd.path())
                .is_ok_and(|t| wanted.iter().any(|w| t.as_os_str() == w.as_str()))
        });
        if !owns {
            continue;
        }
        let uid = std::fs::read_to_string(p.path().join("status"))
            .ok()
            .and_then(|s| status_uid(&s));
        let args = std::fs::read(p.path().join("cmdline")).map(|b| split_cmdline(&b));
        if let (Some(uid), Ok(args)) = (uid, args) {
            out.push(Listener { pid, uid, args });
        }
    }
    out
}

/// Socket inodes in LISTEN state (`st` 0A) on `port` from a /proc/net/tcp{,6} table.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn listen_inodes(table: &str, port: u16) -> Vec<u64> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let local_port = f.get(1)?.rsplit(':').next()?;
            let ok = u16::from_str_radix(local_port, 16).ok()? == port && *f.get(3)? == "0A";
            ok.then(|| f.get(9)?.parse().ok()).flatten()
        })
        .collect()
}

/// Real uid from a /proc/<pid>/status text.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn status_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// NUL-separated /proc/<pid>/cmdline into argv (exact).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn split_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub fn find_listeners(port: u16) -> Vec<Listener> {
    let Some(out) = run(&[
        "lsof",
        "-nP",
        &format!("-iTCP:{port}"),
        "-sTCP:LISTEN",
        "-t",
    ]) else {
        return Vec::new();
    };
    out.split_whitespace()
        .filter_map(|p| p.parse::<u32>().ok())
        .filter_map(|pid| {
            let pid_s = pid.to_string();
            let uid = run(&["ps", "-o", "uid=", "-p", &pid_s])?
                .trim()
                .parse()
                .ok()?;
            let cmd = run(&["ps", "-o", "command=", "-p", &pid_s])?;
            Some(Listener {
                pid,
                uid,
                args: args_from_ps_command(cmd.trim()),
            })
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn run(argv: &[&str]) -> Option<String> {
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
#[path = "rpc_admin_locate_tests.rs"]
mod tests;
