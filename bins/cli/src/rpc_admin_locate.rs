//! INC-I-308: find the data dir of the local node that listens on a loopback RPC port.
//!
//! Security rule: when a listener on the port is visible, a token file is sent only if
//! that listener's uid owns it. Otherwise any local user who binds the port (forged
//! `--data-dir` or not) gets a root CLI's token.

use std::path::{Path, PathBuf};

use doli_cli::rpc_admin_auth::{self as admin, TOKEN_FILE_NAME};

/// A process that listens on a TCP port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    #[allow(dead_code)]
    pub pid: u32,
    pub uid: u32,
    pub args: Vec<String>,
}

/// Token dir for a loopback port. With a visible listener a dir counts only when
/// every visible listener's uid owns its `rpc-admin.token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenDir {
    /// The visible listener's uid owns `<dir>/rpc-admin.token`.
    Owned(PathBuf),
    /// No listener visible: `DOLI_DATA_DIR` or the default dir, not verified.
    Unidentified(PathBuf),
    /// A listener is visible but owns no candidate token file: send no token.
    Refused {
        uid: u32,
        file: PathBuf,
        owner: Option<u32>,
    },
}

impl TokenDir {
    pub fn dir(&self) -> Option<&Path> {
        match self {
            TokenDir::Owned(d) | TokenDir::Unidentified(d) => Some(d),
            TokenDir::Refused { .. } => None,
        }
    }
}

/// A visible listener: its uid and the data dir from its argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenListener {
    pub uid: u32,
    pub dir: PathBuf,
}

/// Candidates: `env_data_dir` alone, else the listeners' dirs then `default_dir`.
pub fn choose_token_dir(
    listeners: &[SeenListener],
    env_data_dir: Option<PathBuf>,
    default_dir: PathBuf,
    owner_of: impl Fn(&Path) -> Option<u32>,
) -> TokenDir {
    let Some(first) = listeners.first() else {
        return TokenDir::Unidentified(env_data_dir.unwrap_or(default_dir));
    };
    let candidates: Vec<PathBuf> = match env_data_dir {
        Some(d) => vec![d],
        None => listeners
            .iter()
            .map(|l| l.dir.clone())
            .chain([default_dir])
            .collect(),
    };
    let owned = |d: &Path| {
        let owner = owner_of(&d.join(TOKEN_FILE_NAME));
        owner.is_some() && listeners.iter().all(|l| Some(l.uid) == owner)
    };
    match candidates.iter().find(|d| owned(d)) {
        Some(d) => TokenDir::Owned(d.clone()),
        None => {
            let file = candidates[0].join(TOKEN_FILE_NAME);
            TokenDir::Refused {
                uid: first.uid,
                owner: owner_of(&file),
                file,
            }
        }
    }
}

/// Token dir for a loopback `endpoint` (`fallback` = the CLI's network default dir).
pub fn admin_token_dir(endpoint: &str, fallback: impl FnOnce() -> PathBuf) -> TokenDir {
    let env = std::env::var_os("DOLI_DATA_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let seen = endpoint_port(endpoint).map_or_else(Vec::new, port_listeners);
    choose_token_dir(&seen, env, fallback(), token_owner)
}

/// One-line -32008 hint for a loopback endpoint.
pub fn loopback_hint(endpoint: &str, env: Option<&str>, td: &TokenDir) -> String {
    let port = endpoint_port(endpoint).map_or_else(|| "?".to_string(), |p| p.to_string());
    let code = admin::UNAUTHORIZED_CODE;
    let short = admin::short_env_note(env);
    let fix = format!(
        "run with sudo, or set {}, or set DOLI_DATA_DIR to that node's data dir",
        admin::ENV_TOKEN
    );
    match td {
        TokenDir::Owned(d) => admin::unauthorized_hint_for(env, &d.join(TOKEN_FILE_NAME)),
        TokenDir::Unidentified(d) => format!(
            "admin RPC method refused ({code}): the CLI could not identify the node on port {port} (a non-root user cannot see a root node's sockets); it tried {}, which is not verified as that node's token file{short}; {fix}",
            d.join(TOKEN_FILE_NAME).display()
        ),
        TokenDir::Refused { uid, file, owner } => {
            let owner = owner.map_or_else(|| "file missing".to_string(), |o| format!("owner uid {o}"));
            format!(
                "admin RPC method refused ({code}): the process on port {port} (uid {uid}) is not owned by the owner of the token file {} ({owner}), so the CLI sent no token{short}; check that the node listens on port {port}, or {fix}",
                file.display()
            )
        }
    }
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    reqwest::Url::parse(endpoint).ok()?.port_or_known_default()
}

fn fallback_default(network: &str) -> PathBuf {
    crate::paths::resolve_base_dir(network, None)
}

/// Visible listeners on `port` with the data dir from their argv.
pub fn port_listeners(port: u16) -> Vec<SeenListener> {
    find_listeners(port)
        .into_iter()
        .map(|l| SeenListener {
            uid: l.uid,
            dir: data_dir_from_args(&l.args, &fallback_default),
        })
        .collect()
}

/// Owner uid of `file`, if it exists.
#[cfg(unix)]
pub fn token_owner(file: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(file).ok().map(|m| m.uid())
}

#[cfg(not(unix))]
pub fn token_owner(_file: &Path) -> Option<u32> {
    None
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
