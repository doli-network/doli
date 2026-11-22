//! INC-I-308 send-back 1 — locate the data dir of the node listening on a loopback port.
//!
//! OUTPUT CONTRACT: data_dir_from_args(args, default) -> PathBuf; token_owner(file) -> Option<u32>;
//!   choose_token_dir(listeners, hidden, env dir, default, owner_of) -> TokenDir; admin_token_dir(endpoint,
//!   fallback) -> TokenDir; loopback_hint(endpoint, env, TokenDir) -> one line; /proc + ps parsers.
//! PATHS: --data-dir X | --data-dir=X | -d X | ~/X | no flag -> default(--network | -n | mainnet);
//!   listener visible: candidate {DOLI_DATA_DIR | own dir | default} owned by its uid -> Owned,
//!   other uid / absent -> Refused (no token); LISTEN socket whose owner process is unknown ->
//!   UnknownOwner (no token); no socket -> Unidentified(env | default).
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

/// Decision: a failure means the uid check reads the wrong owner, so a squatter's uid can
/// match a token file it does not own.
#[cfg(unix)]
#[test]
fn token_owner_reads_the_file_owner() {
    let dir = TempDir::new().unwrap();
    let uid = my_uid(dir.path());
    let file = dir.path().join("rpc-admin.token");
    assert_eq!(token_owner(&file), None, "no token file");
    std::fs::write(&file, "t\n").unwrap();
    assert_eq!(token_owner(&file), Some(uid));
}

fn seen(uid: u32, dir: &str) -> SeenListener {
    SeenListener {
        uid,
        dir: PathBuf::from(dir),
    }
}

/// owner_of over a fixed (token file -> uid) table.
fn owners(table: &'static [(&'static str, u32)]) -> impl Fn(&Path) -> Option<u32> {
    move |f: &Path| {
        let f = f.parent().unwrap().to_str().unwrap().to_string();
        table.iter().find(|(d, _)| *d == f).map(|(_, u)| *u)
    }
}

/// INC-I-308 run 737 F1 — Decision: a failure means a visible listener of uid X gets a token
/// file owned by uid Y (root's default-dir token reached an unprivileged fake listener on Linux).
#[test]
fn visible_listener_gets_only_a_token_file_its_uid_owns() {
    let def = || PathBuf::from("/var/lib/doli/mainnet");
    let env = || Some(PathBuf::from("/env/data"));
    let refused = |o: Option<u32>, f: &str| TokenDir::Refused {
        uid: 1000,
        file: PathBuf::from(f).join("rpc-admin.token"),
        owner: o,
    };
    // listener uid 1000, every candidate file owned by root (0): no token at all
    let root_owned = owners(&[("/var/lib/doli/mainnet", 0), ("/fake", 0), ("/env/data", 0)]);
    let got = choose_token_dir(&[seen(1000, "/fake")], false, None, def(), &root_owned);
    assert_eq!(got.dir(), None, "default dir must not be sent: {got:?}");
    assert_eq!(got, refused(Some(0), "/fake"));
    let got = choose_token_dir(&[seen(1000, "/fake")], false, env(), def(), &root_owned);
    assert_eq!(
        got,
        refused(Some(0), "/env/data"),
        "DOLI_DATA_DIR is uid-checked too"
    );
    let got = choose_token_dir(&[seen(1000, "/fake")], false, None, def(), owners(&[]));
    assert_eq!(got, refused(None, "/fake"), "no token file anywhere");
    // the listener's uid owns a candidate: that one is used
    let mine = owners(&[
        ("/n1", 1000),
        ("/var/lib/doli/mainnet", 1000),
        ("/env/data", 1000),
    ]);
    let got = choose_token_dir(&[seen(1000, "/n1")], false, None, def(), &mine);
    assert_eq!(got, TokenDir::Owned(PathBuf::from("/n1")));
    let got = choose_token_dir(&[seen(1000, "/n2")], false, None, def(), &mine);
    assert_eq!(
        got,
        TokenDir::Owned(def()),
        "default dir owned by the listener uid"
    );
    let got = choose_token_dir(&[seen(1000, "/n1")], false, env(), def(), &mine);
    assert_eq!(got, TokenDir::Owned(PathBuf::from("/env/data")));
    // two visible listeners of different uids: no file is owned by both
    let got = choose_token_dir(
        &[seen(1000, "/n1"), seen(1001, "/n1")],
        false,
        None,
        def(),
        &mine,
    );
    assert_eq!(got.dir(), None, "mixed-uid listeners: {got:?}");
    // no listener visible: unverified fallback, as before
    let got = choose_token_dir(&[], false, None, def(), owners(&[]));
    assert_eq!(got, TokenDir::Unidentified(def()));
    let got = choose_token_dir(&[], false, env(), def(), owners(&[]));
    assert_eq!(got, TokenDir::Unidentified(PathBuf::from("/env/data")));
}

/// INC-I-308 run 737 F4 — Decision: a failure means a root CLI without CAP_SYS_PTRACE, which sees
/// the LISTEN socket in /proc/net/tcp but cannot read its owner's /proc/<pid>/fd, sends the
/// unverified DOLI_DATA_DIR / default dir token to that unknown listener ("5d with-auth=4").
#[test]
fn visible_socket_with_unknown_owner_gets_no_token_dir() {
    let def = || PathBuf::from("/var/lib/doli/mainnet");
    let env = || Some(PathBuf::from("/env/data"));
    let root_owned = owners(&[("/var/lib/doli/mainnet", 0), ("/env/data", 0)]);
    let got = choose_token_dir(&[], true, None, def(), &root_owned);
    assert_eq!(got, TokenDir::UnknownOwner, "default dir sent: {got:?}");
    let got = choose_token_dir(&[], true, env(), def(), &root_owned);
    assert_eq!(got, TokenDir::UnknownOwner, "DOLI_DATA_DIR sent: {got:?}");
    // one identified listener and one socket with no known owner on the same port
    let got = choose_token_dir(
        &[seen(0, "/var/lib/doli/mainnet")],
        true,
        None,
        def(),
        &root_owned,
    );
    assert_eq!(got.dir(), None, "unowned second socket: {got:?}");
    // no socket at all: the unverified fallback stays
    let got = choose_token_dir(&[], false, None, def(), &root_owned);
    assert_eq!(got, TokenDir::Unidentified(def()));
}

/// INC-I-308 run 737 F1+F2 — Decision: a failure means a hint blames the wrong cause: it names
/// the default file as the node's when no listener is visible, or hides that the port's process
/// was refused.
#[test]
fn loopback_hint_names_the_cause_per_lookup_result() {
    let url = "http://127.0.0.1:18621";
    let file = PathBuf::from("/var/lib/doli/mainnet/rpc-admin.token");
    let h = loopback_hint(
        url,
        None,
        &TokenDir::Unidentified(file.parent().unwrap().into()),
    );
    assert!(
        h.contains("could not identify the node on port 18621"),
        "{h}"
    );
    assert!(h.contains("not verified as that node's token file"), "{h}");
    assert!(!h.contains("of the node listening"), "{h}");
    assert!(
        h.contains("run with sudo") && h.contains("DOLI_DATA_DIR"),
        "{h}"
    );
    let refused = TokenDir::Refused {
        uid: 1000,
        file: file.clone(),
        owner: Some(0),
    };
    let h = loopback_hint(url, None, &refused);
    assert!(
        h.contains("process on port 18621 (uid 1000) is not owned by the owner of the token file"),
        "{h}"
    );
    assert!(
        h.contains("owner uid 0") && h.contains("sent no token"),
        "{h}"
    );
    let h = loopback_hint(url, Some("short"), &refused);
    assert!(h.contains("shorter than 32"), "{h}");
    let h = loopback_hint(url, None, &TokenDir::Owned(file.parent().unwrap().into()));
    assert!(
        h.contains(&file.display().to_string()) && h.contains("run with sudo"),
        "{h}"
    );
    let h = loopback_hint(url, None, &TokenDir::UnknownOwner);
    assert!(
        h.contains("found a listener on port 18621 but could not identify its owner process"),
        "{h}"
    );
    assert!(
        h.contains("sent no token") && !h.contains("rpc-admin.token"),
        "{h}"
    );
    assert!(
        h.contains("run with sudo on the host") && h.contains("DOLI_RPC_ADMIN_TOKEN"),
        "{h}"
    );
    for td in [
        TokenDir::UnknownOwner,
        TokenDir::Owned("/a".into()),
        TokenDir::Unidentified("/a".into()),
        refused,
    ] {
        let h = loopback_hint(url, None, &td);
        assert_eq!(h.trim_end().lines().count(), 1, "{h:?}");
    }
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
    let (found, _) = find_listeners(port);
    let got = {
        let (seen, hidden) = port_listeners(port);
        choose_token_dir(&seen, hidden, None, net_default("x"), token_owner)
    };
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        found.iter().any(|l| l.pid == child.id()),
        "listener pid {} not in {found:?}",
        child.id()
    );
    assert_eq!(
        got.dir().map(|p| p.canonicalize().unwrap()),
        Some(dir.path().canonicalize().unwrap())
    );
}

/// INC-I-308 run 737 F1 — Decision: a failure means a real listener refused by the uid rule
/// still gets the fallback dir's token (red on 3508d861: `admin_data_dir` returned the fallback).
#[cfg(unix)]
#[test]
fn refused_real_listener_gets_no_fallback_dir() {
    use std::io::BufRead;
    let own = TempDir::new().unwrap();
    let fallback = TempDir::new().unwrap();
    let script = "import socket,time\ns=socket.socket()\ns.bind(('127.0.0.1',0))\ns.listen(1)\nprint(s.getsockname()[1],flush=True)\ntime.sleep(30)";
    let Ok(mut child) = std::process::Command::new("python3")
        .args(["-c", script, "--data-dir"])
        .arg(own.path())
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
    let fb = fallback.path().to_path_buf();
    let got = {
        let (seen, hidden) = port_listeners(port);
        choose_token_dir(&seen, hidden, None, fb, token_owner)
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        got.dir(),
        None,
        "refused listener on port {port} -> {got:?}"
    );
}
