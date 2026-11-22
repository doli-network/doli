//! INC-I-308 send-back 1 — locate the data dir of the node listening on a loopback port.
//!
//! OUTPUT CONTRACT: data_dir_from_args(args, default) -> PathBuf; accept_listener(uid, dir) -> bool;
//!   listener_data_dir(port, default) -> Option<PathBuf>; parsers for /proc and ps text.
//! PATHS: --data-dir X | --data-dir=X | -d X | ~/X | no flag -> default(--network | -n | mainnet);
//!   token file owned by the listener uid -> accept | other uid -> reject | absent -> reject.
//! INPUT PARTITIONS: argv in {long flag, =form, short flag, tilde, network long/=/short, none,
//!   flag as argv[0]}; token file in {absent, own uid, other uid}; ps line in {spaced existing
//!   dir, non-existing dir}; /proc rows in {LISTEN on port, connected on port, LISTEN other port};
//!   listener in {real child process with --data-dir}.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

fn args(s: &[&str]) -> Vec<String> {
    s.iter().map(|a| a.to_string()).collect()
}

fn net_default(net: &str) -> PathBuf {
    PathBuf::from("/default").join(net)
}

#[cfg(unix)]
fn my_uid(dir: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    let probe = dir.join(".uid-probe");
    std::fs::write(&probe, "x").unwrap();
    std::fs::metadata(&probe).unwrap().uid()
}

/// Decision: a failure means the CLI reads the token file of a different node than the
/// one on the port (multi-node host) or of no node at all.
#[test]
fn data_dir_from_args_follows_the_node_rule() {
    let d = |a: &[&str]| data_dir_from_args(&args(a), &net_default);
    assert_eq!(
        d(&["doli-node", "--data-dir", "/m/n1/data", "run"]),
        PathBuf::from("/m/n1/data")
    );
    assert_eq!(
        d(&["doli-node", "--data-dir=/m/n2/data", "run"]),
        PathBuf::from("/m/n2/data")
    );
    assert_eq!(
        d(&["doli-node", "-n", "testnet", "-d", "/m/n3"]),
        PathBuf::from("/m/n3")
    );
    assert_eq!(
        d(&["doli-node", "--network", "testnet", "run"]),
        net_default("testnet")
    );
    assert_eq!(
        d(&["doli-node", "--network=Devnet", "run"]),
        net_default("devnet")
    );
    assert_eq!(d(&["doli-node", "-n", "testnet"]), net_default("testnet"));
    assert_eq!(d(&["doli-node", "run"]), net_default("mainnet"));
    let home = dirs::home_dir().unwrap();
    assert_eq!(d(&["doli-node", "--data-dir", "~/x"]), home.join("x"));
    assert_eq!(d(&["--data-dir"]), net_default("mainnet"));
}

/// Decision: a failure means a local user who forges `--data-dir` on a free port gets
/// another node's token from a root CLI (uid mismatch must reject).
#[cfg(unix)]
#[test]
fn listener_accepted_only_when_its_uid_owns_the_token_file() {
    let dir = TempDir::new().unwrap();
    let uid = my_uid(dir.path());
    assert!(!accept_listener(uid, dir.path()), "no token file -> reject");
    std::fs::write(dir.path().join("rpc-admin.token"), "t\n").unwrap();
    assert!(accept_listener(uid, dir.path()));
    assert!(
        !accept_listener(uid.wrapping_add(1), dir.path()),
        "uid mismatch"
    );
}

/// Decision: a failure means a ps line with a data dir that has spaces (macOS default
/// `Library/Application Support`) resolves to the wrong dir.
#[test]
fn ps_command_keeps_a_data_dir_with_spaces() {
    let t = TempDir::new().unwrap();
    let spaced = t.path().join("Application Support").join("doli");
    std::fs::create_dir_all(&spaced).unwrap();
    let cmd = format!("doli-node --data-dir {} run --rpc-port 1", spaced.display());
    let a = args_from_ps_command(&cmd);
    assert_eq!(a[2], spaced.display().to_string());
    assert_eq!(a[3..], args(&["run", "--rpc-port", "1"])[..]);
    let a = args_from_ps_command("doli-node --data-dir /no/such dir run");
    assert_eq!(
        a,
        args(&["doli-node", "--data-dir", "/no/such", "dir", "run"])
    );
}

/// Decision: a failure means the Linux /proc parser misses a LISTEN socket or takes a
/// connected socket on the same port.
#[test]
fn proc_parsers() {
    let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:2135 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4242 1 0\n\
   1: 0100007F:2135 0100007F:C350 01 00000000:00000000 00:00000000 00000000     0        0 4343 1 0\n\
   2: 0100007F:2136 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4444 1 0\n";
    assert_eq!(listen_inodes(table, 0x2135), vec![4242]);
    assert_eq!(
        status_uid("Name:\tdoli-node\nUid:\t1001\t1001\t1001\t1001\n"),
        Some(1001)
    );
    assert_eq!(
        split_cmdline(b"doli-node\0--data-dir\0/a b\0run\0"),
        args(&["doli-node", "--data-dir", "/a b", "run"])
    );
}

/// Decision: a failure means the port lookup cannot find a real local listener, so the
/// CLI falls back to the network default dir on a multi-node host.
#[cfg(unix)]
#[test]
fn finds_a_real_listener_and_its_data_dir() {
    use std::io::BufRead;
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("rpc-admin.token"), "t\n").unwrap();
    let script = "import socket,time\ns=socket.socket()\ns.bind(('127.0.0.1',0))\ns.listen(1)\nprint(s.getsockname()[1],flush=True)\ntime.sleep(30)";
    let Ok(mut child) = std::process::Command::new("python3")
        .args(["-c", script, "--data-dir"])
        .arg(dir.path())
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        eprintln!("python3 not available; skipped");
        return;
    };
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().parse().unwrap();
    let found = find_listeners(port);
    let got = listener_data_dir(port, &net_default);
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        found.iter().any(|l| l.pid == child.id()),
        "listener pid {} not in {found:?}",
        child.id()
    );
    assert_eq!(
        got.map(|p| p.canonicalize().unwrap()),
        Some(dir.path().canonicalize().unwrap())
    );
}
